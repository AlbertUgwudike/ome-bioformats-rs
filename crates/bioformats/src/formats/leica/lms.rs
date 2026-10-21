//! Translation of Leica LMS image XML into core and OME metadata.
//!
//! Port of `loci.formats.in.LeicaMicrosystemsMetadata.{LMSMetadataExtractor,
//! MetadataTempBuffer, Dimension, Channel, MetadataStoreInitializer}`, limited
//! to what affects pixel layout plus image-level OME metadata. Instrument
//! settings (lasers, detectors, filters, objectives) and ROIs are not ported yet.

use std::collections::BTreeMap;

use roxmltree::Node;

use crate::core::CoreMetadata;
use crate::dims::DimensionOrder;
use crate::error::{Result, format_err};
use crate::meta::{ChannelMetadata, ImageMetadata, PlaneMetadata, Quantity, pack_color};
use crate::pixel::PixelType;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DimKey {
    X,
    Y,
    Z,
    T,
    C,
    /// Stage position: tiles of a mosaic.
    S,
}

impl DimKey {
    fn from_id(id: i64) -> Option<Self> {
        Some(match id {
            1 => DimKey::X,
            2 => DimKey::Y,
            3 => DimKey::Z,
            4 => DimKey::T,
            5 => DimKey::C,
            10 => DimKey::S,
            _ => return None,
        })
    }

    fn token(self) -> Option<char> {
        Some(match self {
            DimKey::X => 'X',
            DimKey::Y => 'Y',
            DimKey::Z => 'Z',
            DimKey::T => 'T',
            DimKey::C => 'C',
            DimKey::S => return None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Dimension {
    pub key: DimKey,
    pub size: u32,
    pub bytes_inc: u64,
    /// Per-pixel step in micrometres (or seconds for T).
    pub step: f64,
}

impl Dimension {
    /// Port of `Dimension.setLength`: converts the total extent into a
    /// per-element step, normalising metres to micrometres and Ks to s.
    fn new(key: DimKey, size: u32, bytes_inc: u64, unit: &str, length: f64) -> Self {
        let mut step = if size > 1 {
            length / (size - 1) as f64
        } else {
            0.0
        };
        match unit {
            "Ks" => step /= 1000.0,
            "m" => step *= 1e6,
            _ => {}
        }
        Dimension {
            key,
            size,
            bytes_inc,
            step,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Channel {
    pub channel_tag: i64,
    pub resolution: u32,
    pub lut_name: String,
    pub bytes_inc: u64,
}

/// Which reader's dimension rules to apply. Bio-Formats' LIF reader predates
/// the shared `LeicaMicrosystemsMetadata` package and keeps its own logic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavor {
    /// `LMSMetadataExtractor` (LOF, XLIF).
    Lms,
    /// `LIFReader.translateImageNodes`.
    Lif,
}

/// Everything the readers need to know about one LMS image.
#[derive(Debug, Clone)]
pub struct LmsImage {
    pub core: CoreMetadata,
    pub image: ImageMetadata,
    /// BytesInc of the axis stored as rows; larger than the row size when
    /// rows are padded.
    pub row_inc: u64,
    /// True if RGB samples are stored blue-first.
    pub bgr: bool,
    pub tile_count: u32,
    pub tile_bytes_inc: u64,
}

/// Pixel layout produced by a flavor's dimension rules.
struct Layout {
    core: CoreMetadata,
    step_x: f64,
    step_y: f64,
    step_z: f64,
    step_t: f64,
    row_inc: u64,
    tile_count: u32,
    tile_bytes_inc: u64,
}

fn attr<'a>(node: Node<'a, '_>, name: &str) -> &'a str {
    node.attribute(name).unwrap_or("").trim()
}

fn parse_i64(node: Node, name: &str) -> Result<i64> {
    let v = attr(node, name);
    if v.is_empty() {
        return Ok(0);
    }
    v.parse()
        .map_err(|_| format_err!("Leica XML: bad {name}={v:?}"))
}

fn parse_f64(node: Node, name: &str) -> f64 {
    attr(node, name).parse().unwrap_or(0.0)
}

/// Finds the descendant elements named `name` (document order).
fn descendants<'a, 'i>(
    node: Node<'a, 'i>,
    name: &'a str,
) -> impl Iterator<Item = Node<'a, 'i>> + 'a {
    node.descendants().filter(move |n| n.has_tag_name(name))
}

fn first<'a, 'i>(node: Node<'a, 'i>, name: &'a str) -> Option<Node<'a, 'i>> {
    descendants(node, name).next()
}

fn dimension_nodes<'a, 'i>(description: Node<'a, 'i>) -> impl Iterator<Item = Node<'a, 'i>> {
    first(description, "Dimensions")
        .into_iter()
        .flat_map(|d| descendants(d, "DimensionDescription"))
}

/// Sets the fields both flavors fill in the same way.
fn finish_core(core: &mut CoreMetadata, channels: &[Channel], rgb: bool, extras: u32) {
    if extras > 1 {
        if core.size_z <= 1 {
            core.size_z = extras;
        } else {
            core.size_t = core.size_t.max(1) * extras;
        }
    }
    core.size_x = core.size_x.max(1);
    core.size_y = core.size_y.max(1);
    core.size_z = core.size_z.max(1);
    core.size_t = core.size_t.max(1);
    core.size_c = (channels.len() as u32).max(1);
    core.rgb_channel_count = if rgb && core.size_c.is_multiple_of(3) {
        3
    } else {
        1
    };
    core.interleaved = core.is_rgb();
    core.indexed = !core.is_rgb();
    core.false_color = true;
    core.little_endian = true;
    if let Some(bits) = channels.first().map(|c| c.resolution)
        && bits > 0
        && bits <= core.pixel_type.bytes() as u32 * 8
    {
        core.bits_per_pixel = bits;
    }
    core.update_image_count();
}

/// Port of `LMSMetadataExtractor.translateDimensionDescriptions` and the
/// `MetadataTempBuffer` dimension sorting.
fn lms_layout(description: Node, channels: &[Channel]) -> Result<Layout> {
    let mut dims = Vec::new();
    let mut extras = 1u32;
    for d in dimension_nodes(description) {
        let size = parse_i64(d, "NumberOfElements")? as u32;
        let bytes_inc = parse_i64(d, "BytesInc")? as u64;
        match DimKey::from_id(parse_i64(d, "DimID")?) {
            Some(key) => dims.push(Dimension::new(
                key,
                size,
                bytes_inc,
                attr(d, "Unit"),
                parse_f64(d, "Length"),
            )),
            None => extras *= size.max(1),
        }
    }
    let find = |dims: &[Dimension], k| dims.iter().position(|d: &Dimension| d.key == k);
    let (Some(xi), Some(yi)) = (find(&dims, DimKey::X), find(&dims, DimKey::Y)) else {
        return Err(format_err!("Leica XML image lacks an X or Y dimension"));
    };

    // addChannelDimension
    let rgb = dims[xi].bytes_inc.is_multiple_of(3);
    let size_c_dim = if rgb {
        channels.len() / 3
    } else {
        channels.len()
    } as u32;
    let mut channel_inc = channels
        .iter()
        .filter(|c| !rgb || c.channel_tag == 3)
        .map(|c| c.bytes_inc)
        .max()
        .unwrap_or(0);
    if channel_inc == 0 {
        channel_inc = dims[yi].bytes_inc * dims[yi].size as u64;
    }
    dims.push(Dimension {
        key: DimKey::C,
        size: size_c_dim,
        bytes_inc: channel_inc,
        step: 0.0,
    });

    // addMissingDimensions
    let last_inc = dims.iter().map(|d| d.bytes_inc).max().unwrap_or(0);
    for key in [DimKey::Z, DimKey::T, DimKey::S] {
        if find(&dims, key).is_none() {
            dims.push(Dimension {
                key,
                size: 1,
                bytes_inc: last_inc,
                step: 0.0,
            });
        }
    }
    let tile_count: u32 = dims
        .iter()
        .filter(|d| d.key == DimKey::S)
        .map(|d| d.size.max(1))
        .product();
    let tile_bytes_inc = dims
        .iter()
        .rfind(|d| d.key == DimKey::S)
        .map_or(0, |d| d.bytes_inc);

    // setCoreDimensionSizes / setPixelType
    let get = |k| dims.iter().find(|d: &&Dimension| d.key == k).unwrap();
    let mut x_inc = get(DimKey::X).bytes_inc;
    if rgb {
        x_inc /= 3;
    }
    let pixel_type = PixelType::from_bytes(x_inc as usize, false, true)?;
    let mut core = CoreMetadata::new(get(DimKey::X).size, get(DimKey::Y).size, pixel_type);
    core.size_z = get(DimKey::Z).size;
    core.size_t = get(DimKey::T).size;
    finish_core(&mut core, channels, rgb, extras);

    // sortDimensions / getDimensionOrder
    dims.sort_by_key(|d| d.bytes_inc);
    let take =
        |dims: &mut Vec<Dimension>, k| dims.iter().position(|d| d.key == k).map(|i| dims.remove(i));
    let dx = take(&mut dims, DimKey::X).unwrap();
    let dy = take(&mut dims, DimKey::Y).unwrap();
    let ds = take(&mut dims, DimKey::S).unwrap();
    let mut sorted = if dx.bytes_inc < dy.bytes_inc {
        vec![dx, dy]
    } else {
        vec![dy, dx]
    };
    sorted.append(&mut dims);
    sorted.push(ds);
    let order: String = sorted.iter().filter_map(|d| d.key.token()).collect();
    core.dimension_order = order.parse().unwrap_or(DimensionOrder::XYCZT);

    let step = |k| {
        sorted
            .iter()
            .find(|d: &&Dimension| d.key == k)
            .map_or(0.0, |d| d.step)
    };
    Ok(Layout {
        step_x: step(DimKey::X),
        step_y: step(DimKey::Y),
        step_z: step(DimKey::Z).abs(),
        step_t: step(DimKey::T),
        row_inc: sorted
            .iter()
            .find(|d| d.key == DimKey::Y)
            .map_or(0, |d| d.bytes_inc),
        core,
        tile_count: tile_count.max(1),
        tile_bytes_inc,
    })
}

/// Port of `LIFReader.translateImageNodes`, including its handling of XZ
/// and XT line scans (where Z or T is stored as rows).
fn lif_layout(description: Node, channels: &[Channel]) -> Result<Layout> {
    // Axis tokens keyed by BytesInc, which orders them in the file.
    let mut axes: BTreeMap<u64, char> = BTreeMap::new();
    for c in channels.iter().filter(|c| c.bytes_inc > 0) {
        axes.insert(c.bytes_inc, 'C');
    }
    // As in Java, sizes start at 0 ("unset").
    let (mut sx, mut sy, mut sz, mut st) = (0u32, 0u32, 0u32, 0u32);
    let mut pixel_type = None;
    let mut rgb = false;
    let (mut step_x, mut step_y, mut step_z, mut step_t) = (0.0, 0.0, 0.0, 0.0);
    let mut row_inc = 0;
    let mut tile_count = 1u32;
    let mut tile_bytes_inc = 0;
    let mut extras = 1u32;
    for d in dimension_nodes(description) {
        let id = parse_i64(d, "DimID")?;
        let len = parse_i64(d, "NumberOfElements")? as u32;
        let mut n_bytes = parse_i64(d, "BytesInc")? as u64;
        let step = Dimension::new(
            DimKey::X,
            len,
            n_bytes,
            attr(d, "Unit"),
            parse_f64(d, "Length"),
        )
        .step;
        match id {
            1 => {
                sx = len;
                rgb = n_bytes.is_multiple_of(3);
                if rgb {
                    n_bytes /= 3;
                }
                pixel_type = Some(PixelType::from_bytes(n_bytes as usize, false, true)?);
                step_x = step;
            }
            2 if sy != 0 => {
                if sz == 1 {
                    sz = len;
                    axes.insert(n_bytes, 'Z');
                    step_z = step;
                } else if st == 1 {
                    st = len;
                    axes.insert(n_bytes, 'T');
                }
            }
            2 => {
                sy = len;
                step_y = step;
                row_inc = n_bytes;
            }
            3 | 4 if sy == 0 => {
                // XZ or XT scan: the axis is stored as rows.
                sy = len;
                if id == 3 {
                    sz = 1;
                } else {
                    st = 1;
                }
                axes.insert(n_bytes, 'Y');
                step_y = step;
                row_inc = n_bytes;
            }
            3 => {
                sz = len;
                axes.insert(n_bytes, 'Z');
                step_z = step;
            }
            4 => {
                st = len;
                axes.insert(n_bytes, 'T');
                step_t = step;
            }
            10 => {
                tile_count *= len.max(1);
                tile_bytes_inc = n_bytes;
            }
            _ => extras *= len.max(1),
        }
    }
    let pixel_type =
        pixel_type.ok_or_else(|| format_err!("Leica XML image lacks an X dimension"))?;
    let mut core = CoreMetadata::new(sx, sy, pixel_type);
    core.size_z = sz;
    core.size_t = st;
    finish_core(&mut core, channels, rgb, extras);

    let mut order = String::from("XY");
    let samples = core.rgb_channel_count;
    if samples == 1 || samples == core.size_c {
        if samples > 1 {
            order.push('C');
        }
        for axis in axes.values() {
            if !order.contains(*axis) {
                order.push(*axis);
            }
        }
    }
    for axis in ['Z', 'C', 'T'] {
        if !order.contains(axis) {
            order.push(axis);
        }
    }
    core.dimension_order = order.parse()?;
    Ok(Layout {
        core,
        step_x,
        step_y,
        step_z: step_z.abs(),
        step_t,
        row_inc,
        tile_count,
        tile_bytes_inc,
    })
}

/// Translates one image node. `name` becomes the OME image name.
/// Port of `LMSMetadataExtractor.translateImage` (and the image-level parts
/// of `LIFReader.translateMetadata` for [`Flavor::Lif`]).
pub fn translate_image(image: Node, name: &str, flavor: Flavor) -> Result<LmsImage> {
    let description = first(image, "ImageDescription")
        .ok_or_else(|| format_err!("Leica XML has no ImageDescription"))?;

    let channels: Vec<Channel> = first(description, "Channels")
        .into_iter()
        .flat_map(|c| descendants(c, "ChannelDescription"))
        .map(|c| {
            Ok(Channel {
                channel_tag: parse_i64(c, "ChannelTag")?,
                resolution: parse_i64(c, "Resolution")? as u32,
                lut_name: attr(c, "LUTName").to_owned(),
                bytes_inc: parse_i64(c, "BytesInc")? as u64,
            })
        })
        .collect::<Result<_>>()?;

    let layout = match flavor {
        Flavor::Lms => lms_layout(description, &channels)?,
        Flavor::Lif => lif_layout(description, &channels)?,
    };
    let mut core = layout.core;
    let bgr = core.rgb_channel_count == 3
        && match flavor {
            // LIF RGB data is always BGR.
            Flavor::Lif => true,
            // BGR unless the LUTs explicitly say R, G, B.
            Flavor::Lms => {
                !(channels[0].lut_name == "Red"
                    && channels[1].lut_name == "Green"
                    && channels[2].lut_name == "Blue")
            }
        };

    // -- OME image metadata (MetadataStoreInitializer.initImageDetails) --
    let positive = |v: f64| (v.is_finite() && v > 0.0).then_some(v);
    let mut image_meta = ImageMetadata {
        name: Some(name.trim().to_owned()),
        physical_size_x: positive(layout.step_x).map(Quantity::micrometers),
        physical_size_y: positive(layout.step_y).map(Quantity::micrometers),
        physical_size_z: positive(layout.step_z).map(Quantity::micrometers),
        time_increment: positive(layout.step_t).map(Quantity::seconds),
        ..Default::default()
    };

    for attachment in descendants(image, "Attachment") {
        if attr(attachment, "Name") == "ContextDescription" {
            image_meta.description = attachment.attribute("Content").map(str::to_owned);
        }
    }

    image_meta.channels = (0..core.effective_size_c() as usize)
        .map(|c| ChannelMetadata {
            samples_per_pixel: core.rgb_channel_count,
            color: (!core.is_rgb())
                .then(|| channels.get(c).map(|ch| lut_color(&ch.lut_name)))
                .flatten(),
            ..Default::default()
        })
        .collect();

    let timestamps = translate_timestamps(image, core.image_count as usize);
    if let Some(&first_ts) = timestamps.first() {
        image_meta.acquisition_date = Some(cobol_seconds_to_iso(first_ts));
        for (plane, &ts) in timestamps.iter().enumerate() {
            image_meta.planes.push(PlaneMetadata {
                zct: core.zct(plane as u32)?,
                delta_t: Some(Quantity::seconds(ts - first_ts)),
                ..Default::default()
            });
        }
    }

    populate_original_metadata(image, &mut Vec::new(), &mut core.series_metadata);

    Ok(LmsImage {
        core,
        image: image_meta,
        row_inc: layout.row_inc,
        bgr,
        tile_count: layout.tile_count,
        tile_bytes_inc: layout.tile_bytes_inc,
    })
}

/// Port of `LMSMetadataExtractor.translateLut`.
fn lut_color(lut: &str) -> i32 {
    let lut: String = lut.chars().filter(|c| !c.is_whitespace()).collect();
    let lower = lut.to_ascii_lowercase();
    if let Some(rest) = lower
        .strip_prefix("gradient(")
        .and_then(|r| r.strip_suffix(')'))
    {
        let parts: Vec<u8> = rest.split(',').filter_map(|p| p.parse().ok()).collect();
        if let [b, g, r] = parts[..] {
            // Stored as BGR, as Bio-Formats reads it.
            return pack_color(r, g, b, 255);
        }
    }
    match lower.as_str() {
        "red" => pack_color(255, 0, 0, 255),
        "green" => pack_color(0, 255, 0, 255),
        "blue" => pack_color(0, 0, 255, 255),
        "cyan" => pack_color(0, 255, 255, 255),
        "magenta" => pack_color(255, 0, 255, 255),
        "yellow" => pack_color(255, 255, 0, 255),
        _ => pack_color(255, 255, 255, 255),
    }
}

/// Port of `translateTimestamps`: seconds since 1601-01-01, one per plane.
fn translate_timestamps(image: Node, image_count: usize) -> Vec<f64> {
    let Some(list) = first(image, "TimeStampList") else {
        return Vec::new();
    };
    if list
        .attribute("NumberOfTimeStamps")
        .is_some_and(|n| !n.is_empty())
    {
        // LAS AF 3.1+: space-separated hex FILETIMEs.
        list.text()
            .unwrap_or("")
            .split_whitespace()
            .take(image_count)
            .map(|hex| u64::from_str_radix(hex, 16).map_or(0.0, filetime_seconds))
            .collect()
    } else {
        // LAS AF 3.0 and older: <TimeStamp HighInteger=".." LowInteger=".."/>.
        descendants(image, "TimeStamp")
            .take(image_count)
            .map(|t| {
                let hi: u64 = attr(t, "HighInteger").parse().unwrap_or(0);
                let lo: u64 = attr(t, "LowInteger").parse().unwrap_or(0);
                filetime_seconds((hi << 32) | lo)
            })
            .collect()
    }
}

/// FILETIME ticks (100 ns) to whole-millisecond seconds, as `DateTools.getMillisFromTicks`.
fn filetime_seconds(ticks: u64) -> f64 {
    (ticks / 10_000) as f64 / 1000.0
}

/// Formats seconds since 1601-01-01T00:00:00Z as ISO 8601.
fn cobol_seconds_to_iso(seconds: f64) -> String {
    const DAYS_1601_TO_1970: i64 = 134_774;
    let secs = seconds.floor() as i64;
    let days = secs.div_euclid(86_400) - DAYS_1601_TO_1970;
    let tod = secs.rem_euclid(86_400);
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}",
        tod / 3600,
        tod / 60 % 60,
        tod % 60
    )
}

/// Port of `populateOriginalMetadata`: flattens element attributes into
/// `A|B|C|Attr` keys.
fn populate_original_metadata(
    node: Node,
    stack: &mut Vec<String>,
    out: &mut BTreeMap<String, String>,
) {
    let name = node.tag_name().name();
    let tracked = node.attributes().len() > 0
        && !matches!(name, "Element" | "Attachment" | "LMSDataContainerHeader");
    if tracked {
        stack.push(name.to_owned());
        let prefix: String = stack.iter().map(|s| format!("{s}|")).collect();
        let mut suffix = attr(node, "Identifier");
        if suffix.is_empty() {
            suffix = attr(node, "Description");
        }
        let value = attr(node, "Variant");
        if !suffix.is_empty()
            && !value.is_empty()
            && suffix != "HighInteger"
            && suffix != "LowInteger"
        {
            insert_list(out, format!("{prefix}{suffix}"), value);
        } else {
            for a in node
                .attributes()
                .filter(|a| a.name() != "HighInteger" && a.name() != "LowInteger")
            {
                out.insert(format!("{prefix}{}", a.name()), a.value().to_owned());
            }
        }
    }
    for child in node.children().filter(Node::is_element) {
        populate_original_metadata(child, stack, out);
    }
    if tracked {
        stack.pop();
    }
}

/// Port of `addMetaList`: repeated keys get ` #2`, ` #3`, ... suffixes.
fn insert_list(out: &mut BTreeMap<String, String>, key: String, value: &str) {
    let n = (2..)
        .find(|n| !out.contains_key(&format!("{key} #{n}")))
        .unwrap();
    if let std::collections::btree_map::Entry::Vacant(e) = out.entry(key.clone()) {
        e.insert(value.to_owned());
    } else {
        out.insert(format!("{key} #{n}"), value.to_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cobol_dates() {
        assert_eq!(cobol_seconds_to_iso(0.0), "1601-01-01T00:00:00");
        // 2024-07-08T12:34:56Z
        let unix = 1_720_442_096.0;
        assert_eq!(
            cobol_seconds_to_iso(unix + 11_644_473_600.0),
            "2024-07-08T12:34:56"
        );
    }

    #[test]
    fn planar_three_channel_image() {
        let xml = r#"<LMSDataContainerHeader><Element Name="e"><Data><Image><ImageDescription>
            <Channels>
              <ChannelDescription ChannelTag="0" Resolution="8" LUTName="Red" BytesInc="0"/>
              <ChannelDescription ChannelTag="0" Resolution="8" LUTName="Green" BytesInc="200"/>
              <ChannelDescription ChannelTag="0" Resolution="8" LUTName="Blue" BytesInc="400"/>
            </Channels>
            <Dimensions>
              <DimensionDescription DimID="1" NumberOfElements="20" Length="1.9e-05" Unit="m" BytesInc="1"/>
              <DimensionDescription DimID="2" NumberOfElements="10" Length="9e-06" Unit="m" BytesInc="20"/>
            </Dimensions>
            </ImageDescription></Image></Data></Element></LMSDataContainerHeader>"#;
        let doc = roxmltree::Document::parse(xml).unwrap();
        let image = doc.descendants().find(|n| n.has_tag_name("Image")).unwrap();
        let lms = translate_image(image, "test", Flavor::Lms).unwrap();
        let c = &lms.core;
        assert_eq!(
            (c.size_x, c.size_y, c.size_c, c.size_z, c.size_t),
            (20, 10, 3, 1, 1)
        );
        assert_eq!(c.pixel_type, PixelType::Uint8);
        assert_eq!(c.image_count, 3);
        assert!(!c.is_rgb());
        assert_eq!(c.dimension_order, DimensionOrder::XYCZT);
        assert!((lms.image.physical_size_x.unwrap().value - 1.0).abs() < 1e-9);
        assert_eq!(
            lms.image.channels[0].color,
            Some(pack_color(255, 0, 0, 255))
        );
    }
}
