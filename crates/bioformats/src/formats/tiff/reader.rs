//! TIFF / OME-TIFF reader. Port of `MinimalTiffReader`, the ImageJ handling
//! in `TiffReader`, and the TiffData plane mapping in `OMETiffReader`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::ifd::{Ifd, TiffFile, sniff};
use super::tag;
use crate::codec::{decode_tiff, predictor};
use crate::core::CoreMetadata;
use crate::dims::DimensionOrder;
use crate::error::{Result, format_err, unsupported};
use crate::meta::{ImageMetadata, OmeMetadata, Quantity};
use crate::ome_xml;
use crate::reader::{FormatReader, ReaderFormat, Region, check_plane_args};

pub const FORMAT: ReaderFormat = ReaderFormat {
    name: "Tagged Image File Format",
    suffixes: &["tif", "tiff", "tf2", "tf8", "btf"],
    is_this_type: |_, file| file.read_prefix(0, 4).is_ok_and(|h| sniff(&h).is_some()),
    open: |path| Ok(Box::new(TiffReader::open(path)?)),
};

/// Where a plane's pixels live.
#[derive(Debug, Clone, Copy)]
struct PlaneRef {
    file: usize,
    ifd: usize,
}

pub struct TiffReader {
    files: Vec<TiffFile>,
    cores: Vec<CoreMetadata>,
    meta: OmeMetadata,
    /// `planes[series][plane]`; `None` for planes the file does not contain.
    planes: Vec<Vec<Option<PlaneRef>>>,
    format_name: &'static str,
}

impl TiffReader {
    pub fn open(path: &Path) -> Result<Self> {
        let tiff = TiffFile::open(path)?;
        let description = tiff.ifds[0]
            .ascii(tag::IMAGE_DESCRIPTION)
            .map(str::to_owned);
        let mut reader = TiffReader {
            files: vec![tiff],
            cores: Vec::new(),
            meta: OmeMetadata::default(),
            planes: Vec::new(),
            format_name: "Tagged Image File Format",
        };
        match description.as_deref() {
            Some(d) if d.contains("<OME") => {
                reader.init_ome(d)?;
                reader.format_name = "OME-TIFF";
            }
            Some(d) if d.starts_with("ImageJ=") && reader.init_imagej(d)? => {}
            _ => reader.init_plain()?,
        }
        if let Some(d) = description {
            reader
                .meta
                .global_metadata
                .insert("ImageDescription".into(), d);
        }
        reader.meta.conform_to(&reader.cores);
        Ok(reader)
    }

    /// Builds core metadata for an IFD the way `MinimalTiffReader` does.
    fn core_from_ifd(&self, ifd: &Ifd) -> Result<CoreMetadata> {
        let mut core = CoreMetadata::new(ifd.width()?, ifd.height()?, ifd.pixel_type()?);
        let samples = ifd.samples_per_pixel();
        core.bits_per_pixel = ifd.bits_per_sample();
        core.little_endian = self.files[0].little_endian;
        core.rgb_channel_count = samples;
        core.size_c = samples;
        core.interleaved = !ifd.planar();
        core.indexed = ifd.u64(tag::PHOTOMETRIC_INTERPRETATION) == Some(3);
        Ok(core)
    }

    fn init_plain(&mut self) -> Result<()> {
        let main: Vec<usize> = (0..self.files[0].ifds.len())
            .filter(|&i| !self.files[0].ifds[i].is_reduced())
            .collect();
        let main = if main.is_empty() { vec![0] } else { main };
        // Consecutive IFDs with the same shape form one series of timepoints.
        let mut groups: Vec<(CoreMetadata, Vec<usize>)> = Vec::new();
        for i in main {
            let core = self.core_from_ifd(&self.files[0].ifds[i])?;
            match groups.last_mut() {
                Some((c, members)) if same_shape(c, &core) => members.push(i),
                _ => groups.push((core, vec![i])),
            }
        }
        for (mut core, members) in groups {
            core.size_t = members.len() as u32;
            core.update_image_count();
            self.planes.push(
                members
                    .into_iter()
                    .map(|ifd| Some(PlaneRef { file: 0, ifd }))
                    .collect(),
            );
            self.cores.push(core);
        }
        self.meta.images = vec![ImageMetadata::default(); self.cores.len()];
        Ok(())
    }

    /// Port of the ImageJ hyperstack handling in `TiffReader.initStandardMetadata`.
    /// Returns false if the description does not describe the IFDs.
    fn init_imagej(&mut self, description: &str) -> Result<bool> {
        let props: HashMap<&str, &str> = description
            .lines()
            .filter_map(|l| l.split_once('='))
            .collect();
        let get = |k: &str| {
            props
                .get(k)
                .and_then(|v| v.trim().parse::<u32>().ok())
                .unwrap_or(1)
                .max(1)
        };
        let (channels, slices, frames) = (get("channels"), get("slices"), get("frames"));
        let ifds: Vec<usize> = (0..self.files[0].ifds.len())
            .filter(|&i| !self.files[0].ifds[i].is_reduced())
            .collect();
        let mut core = self.core_from_ifd(&self.files[0].ifds[ifds[0]])?;
        if (channels * slices * frames) as usize != ifds.len()
            || ifds.iter().any(|&i| {
                self.core_from_ifd(&self.files[0].ifds[i])
                    .map_or(true, |c| !same_shape(&c, &core))
            })
        {
            return Ok(false);
        }
        core.size_c = channels * core.rgb_channel_count;
        core.size_z = slices;
        core.size_t = frames;
        core.dimension_order = DimensionOrder::XYCZT;
        core.update_image_count();
        let mut image = ImageMetadata::default();
        if let Some(spacing) = props.get("spacing").and_then(|v| v.trim().parse().ok()) {
            image.physical_size_z = Some(Quantity::new(
                spacing,
                props.get("unit").map_or("µm", |u| ome_unit(u.trim())),
            ));
        }
        image.physical_size_x = resolution_size(&self.files[0].ifds[ifds[0]], tag::X_RESOLUTION);
        image.physical_size_y = resolution_size(&self.files[0].ifds[ifds[0]], tag::Y_RESOLUTION);
        self.planes.push(
            ifds.into_iter()
                .map(|ifd| Some(PlaneRef { file: 0, ifd }))
                .collect(),
        );
        self.cores.push(core);
        self.meta.images.push(image);
        Ok(true)
    }

    fn init_ome(&mut self, xml: &str) -> Result<()> {
        let parsed = ome_xml::parse(xml)?;
        let dir = self.files[0]
            .path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let own_name = self.files[0]
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned());
        let mut file_index: HashMap<PathBuf, usize> =
            HashMap::from([(self.files[0].path.clone(), 0)]);

        for (s, (mut core, tiff_data)) in
            parsed.cores.into_iter().zip(&parsed.tiff_data).enumerate()
        {
            let count = core.image_count as usize;
            let mut planes: Vec<Option<PlaneRef>> = vec![None; count];
            let mut certain = vec![false; count];
            for td in tiff_data {
                let file = match &td.uuid {
                    Some((name, uuid))
                        if Some(name) != own_name.as_ref()
                            && Some(uuid) != parsed.uuid.as_ref() =>
                    {
                        self.resolve_file(&dir, name, &mut file_index)?
                    }
                    _ => 0,
                };
                let Ok(start) = core.plane_index(td.zct) else {
                    break;
                };
                let n = if td.plane_count == 0 {
                    1
                } else {
                    td.plane_count as usize
                };
                for q in 0..n {
                    if let Some(p) = planes.get_mut(start as usize + q) {
                        *p = Some(PlaneRef {
                            file,
                            ifd: td.ifd as usize + q,
                        });
                        certain[start as usize + q] = true;
                    }
                }
                if td.plane_count == 0 {
                    // Unknown number of planes: fill down until a known plane.
                    for no in start as usize + 1..count {
                        if certain[no] {
                            break;
                        }
                        let prev = planes[no - 1].unwrap();
                        planes[no] = Some(PlaneRef {
                            file,
                            ifd: prev.ifd + 1,
                        });
                    }
                }
            }
            if tiff_data.is_empty() {
                // No TiffData at all: planes are the IFDs in order.
                for (no, p) in planes.iter_mut().enumerate() {
                    *p = Some(PlaneRef { file: 0, ifd: no });
                }
            }
            let first = planes
                .iter()
                .flatten()
                .next()
                .copied()
                .ok_or_else(|| format_err!("OME-TIFF series {s} has no planes"))?;
            let ifd = self.files[first.file].ifds.get(first.ifd).ok_or_else(|| {
                format_err!("OME-TIFF series {s} references missing IFD {}", first.ifd)
            })?;
            let from_ifd = self.core_from_ifd(ifd)?;
            core.little_endian = self.files[first.file].little_endian;
            core.interleaved = from_ifd.interleaved;
            core.indexed = from_ifd.indexed;
            if from_ifd.rgb_channel_count != core.rgb_channel_count
                && core.size_c % from_ifd.rgb_channel_count == 0
            {
                core.rgb_channel_count = from_ifd.rgb_channel_count;
                core.update_image_count();
                planes.resize(core.image_count as usize, None);
            }
            if (from_ifd.size_x, from_ifd.size_y, from_ifd.pixel_type)
                != (core.size_x, core.size_y, core.pixel_type)
            {
                return Err(format_err!(
                    "OME-XML for series {s} does not match its first IFD"
                ));
            }
            self.cores.push(core);
            self.planes.push(planes);
        }
        self.meta = parsed.meta;
        Ok(())
    }

    fn resolve_file(
        &mut self,
        dir: &Path,
        name: &str,
        index: &mut HashMap<PathBuf, usize>,
    ) -> Result<usize> {
        let direct = dir.join(name);
        let base = Path::new(name).file_name().map(|b| dir.join(b));
        let path = [Some(direct), base]
            .into_iter()
            .flatten()
            .find(|p| p.exists());
        let Some(path) = path else {
            // Bio-Formats assumes a missing file is the current one.
            return Ok(0);
        };
        if let Some(&i) = index.get(&path) {
            return Ok(i);
        }
        self.files.push(TiffFile::open(&path)?);
        index.insert(path, self.files.len() - 1);
        Ok(self.files.len() - 1)
    }

    /// Reads `region` of one IFD into `buf`, in the series' layout.
    fn read_ifd_region(
        &self,
        tiff: &TiffFile,
        ifd: &Ifd,
        core: &CoreMetadata,
        region: Region,
        buf: &mut [u8],
    ) -> Result<()> {
        let (cw, ch) = ifd.chunk_size()?;
        let tiled = ifd.is_tiled();
        let planar = ifd.planar();
        let bytes = core.pixel_type.bytes();
        let samples = core.rgb_channel_count as usize;
        let chunk_samples = if planar { 1 } else { samples };
        let px = bytes * chunk_samples;
        let across = core.size_x.div_ceil(cw) as usize;
        let down = core.size_y.div_ceil(ch) as usize;
        let offsets = ifd.chunk_offsets()?;
        let counts = ifd.chunk_byte_counts()?;
        let compression = ifd.compression();
        let pred = ifd.predictor();
        if pred == 3 {
            return Err(unsupported!("floating-point predictor"));
        }

        let out_row = region.w as usize * px;
        let sample_plane = out_row * region.h as usize;
        for sample in 0..if planar { samples } else { 1 } {
            for ty in region.y / ch..=(region.y + region.h - 1) / ch {
                for tx in region.x / cw..=(region.x + region.w - 1) / cw {
                    let idx = sample * across * down + ty as usize * across + tx as usize;
                    let chunk = Region::new(tx * cw, ty * ch, cw, ch);
                    // Strips (not tiles) are truncated at the bottom of the image.
                    let rows = if tiled {
                        ch
                    } else {
                        ch.min(core.size_y - chunk.y)
                    };
                    let chunk_row = cw as usize * px;
                    let (Some(&off), Some(&len)) = (offsets.get(idx), counts.get(idx)) else {
                        return Err(format_err!(
                            "chunk {idx} missing from IFD at {}",
                            ifd.offset
                        ));
                    };
                    let decoded = if len == 0 {
                        vec![0; chunk_row * rows as usize]
                    } else {
                        let raw = tiff.file.read_vec_at(off, len as usize)?;
                        let mut d = decode_tiff(compression, &raw, chunk_row * rows as usize)?;
                        if pred == 2 {
                            predictor::undo(
                                &mut d,
                                chunk_row,
                                chunk_samples,
                                bytes,
                                tiff.little_endian,
                            );
                        }
                        d
                    };
                    let Some(inter) = region.intersect(&Region::new(chunk.x, chunk.y, cw, rows))
                    else {
                        continue;
                    };
                    let copy = inter.w as usize * px;
                    for row in 0..inter.h {
                        let src = (inter.y - chunk.y + row) as usize * chunk_row
                            + (inter.x - chunk.x) as usize * px;
                        let dst = sample * sample_plane
                            + (inter.y - region.y + row) as usize * out_row
                            + (inter.x - region.x) as usize * px;
                        buf[dst..dst + copy].copy_from_slice(&decoded[src..src + copy]);
                    }
                }
            }
        }
        Ok(())
    }
}

fn same_shape(a: &CoreMetadata, b: &CoreMetadata) -> bool {
    (
        a.size_x,
        a.size_y,
        a.pixel_type,
        a.rgb_channel_count,
        a.interleaved,
    ) == (
        b.size_x,
        b.size_y,
        b.pixel_type,
        b.rgb_channel_count,
        b.interleaved,
    )
}

fn ome_unit(imagej: &str) -> &'static str {
    match imagej {
        "nm" => "nm",
        "mm" => "mm",
        "cm" => "cm",
        "m" | "meter" => "m",
        _ => "µm",
    }
}

/// Physical pixel size from X/YResolution, in micrometres.
fn resolution_size(ifd: &Ifd, tag: u16) -> Option<Quantity> {
    let (n, d) = match ifd.get(tag)? {
        super::TagValue::Rational(v) => *v.first()?,
        _ => return None,
    };
    if n <= 0 || d <= 0 {
        return None;
    }
    let per_unit = n as f64 / d as f64;
    let unit_um = match ifd.u64(tag::RESOLUTION_UNIT).unwrap_or(2) {
        3 => 10_000.0,
        2 => 25_400.0,
        _ => return None,
    };
    Some(Quantity::micrometers(unit_um / per_unit))
}

impl FormatReader for TiffReader {
    fn format_name(&self) -> &'static str {
        self.format_name
    }

    fn series(&self) -> &[CoreMetadata] {
        &self.cores
    }

    fn metadata(&self) -> &OmeMetadata {
        &self.meta
    }

    fn open_bytes(&self, series: usize, plane: u32, region: Region, buf: &mut [u8]) -> Result<()> {
        let core = check_plane_args(&self.cores, series, plane, region, buf.len())?;
        let Some(PlaneRef { file, ifd }) = self.planes[series][plane as usize] else {
            buf.fill(0);
            return Ok(());
        };
        let tiff = &self.files[file];
        let ifd = tiff
            .ifds
            .get(ifd)
            .ok_or_else(|| format_err!("IFD {ifd} not found in {}", tiff.path.display()))?;
        self.read_ifd_region(tiff, ifd, core, region, buf)
    }

    fn optimal_tile_size(&self, series: usize) -> (u32, u32) {
        let core = &self.cores[series];
        self.planes[series]
            .iter()
            .flatten()
            .next()
            .and_then(|p| self.files[p.file].ifds[p.ifd].chunk_size().ok())
            .map(|(w, h)| {
                if w >= core.size_x {
                    (core.size_x, h.max(1))
                } else {
                    (w, h)
                }
            })
            .unwrap_or((core.size_x, core.size_y))
    }

    fn used_files(&self) -> Vec<PathBuf> {
        self.files.iter().map(|f| f.path.clone()).collect()
    }
}
