//! Shared test helpers: a synthetic in-memory reader and a LOF file generator.

#![allow(dead_code, clippy::too_many_arguments)]

use std::path::{Path, PathBuf};

use bioformats::meta::{ImageMetadata, OmeMetadata, Quantity};
use bioformats::reader::check_plane_args;
use bioformats::{CoreMetadata, DimensionOrder, FormatReader, PixelType, Region};

/// A unique scratch directory under the target dir.
pub fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Deterministic pseudo-random byte for a position, so every plane, sample
/// and pixel differs.
pub fn pattern(series: usize, plane: u32, offset: usize) -> u8 {
    let mut x = (series as u64) << 48 ^ (plane as u64) << 32 ^ offset as u64;
    x = x.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    // Mix in smooth structure so predictors and compressors have work to do.
    ((x >> 59) as u8).wrapping_add((offset / 7) as u8)
}

/// An in-memory reader whose pixels are `pattern(series, plane, byte offset)`
/// in its own layout.
pub struct SyntheticReader {
    pub cores: Vec<CoreMetadata>,
    pub meta: OmeMetadata,
}

impl SyntheticReader {
    pub fn new(cores: Vec<CoreMetadata>) -> Self {
        let mut meta = OmeMetadata {
            images: cores
                .iter()
                .enumerate()
                .map(|(i, _)| ImageMetadata {
                    name: Some(format!("synthetic {i}")),
                    physical_size_x: Some(Quantity::micrometers(0.25)),
                    physical_size_y: Some(Quantity::micrometers(0.25)),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        meta.conform_to(&cores);
        SyntheticReader { cores, meta }
    }

    /// The full plane in the reader's own layout.
    pub fn plane(&self, series: usize, plane: u32) -> Vec<u8> {
        let n = self.cores[series].plane_bytes();
        (0..n).map(|i| pattern(series, plane, i)).collect()
    }
}

impl FormatReader for SyntheticReader {
    fn format_name(&self) -> &'static str {
        "synthetic"
    }

    fn series(&self) -> &[CoreMetadata] {
        &self.cores
    }

    fn metadata(&self) -> &OmeMetadata {
        &self.meta
    }

    fn open_bytes(
        &self,
        series: usize,
        plane: u32,
        region: Region,
        buf: &mut [u8],
    ) -> bioformats::Result<()> {
        let core = check_plane_args(&self.cores, series, plane, region, buf.len())?;
        let full = self.plane(series, plane);
        let samples = core.rgb_channel_count as usize;
        let bytes = core.pixel_type.bytes();
        if core.is_rgb() && !core.interleaved {
            // Planar: each sample plane is cropped separately.
            let plane_len = core.size_x as usize * core.size_y as usize * bytes;
            let out_len = region.w as usize * region.h as usize * bytes;
            for s in 0..samples {
                crop(
                    &full[s * plane_len..],
                    core.size_x as usize * bytes,
                    region,
                    bytes,
                    &mut buf[s * out_len..(s + 1) * out_len],
                );
            }
        } else {
            crop(
                &full,
                core.size_x as usize * bytes * samples,
                region,
                bytes * samples,
                buf,
            );
        }
        Ok(())
    }

    fn used_files(&self) -> Vec<PathBuf> {
        Vec::new()
    }
}

fn crop(src: &[u8], src_row: usize, r: Region, px: usize, out: &mut [u8]) {
    let row = r.w as usize * px;
    for y in 0..r.h as usize {
        let s = (r.y as usize + y) * src_row + r.x as usize * px;
        out[y * row..(y + 1) * row].copy_from_slice(&src[s..s + row]);
    }
}

pub fn core(
    w: u32,
    h: u32,
    ty: PixelType,
    z: u32,
    c: u32,
    t: u32,
    rgb: u32,
    interleaved: bool,
) -> CoreMetadata {
    let mut core = CoreMetadata::new(w, h, ty);
    core.size_z = z;
    core.size_c = c * rgb;
    core.size_t = t;
    core.rgb_channel_count = rgb;
    core.interleaved = interleaved && rgb > 1;
    core.dimension_order = DimensionOrder::XYZCT;
    core.update_image_count();
    core
}

/// Description of a synthetic Leica LOF file.
pub struct ImageSpec {
    pub width: u32,
    pub height: u32,
    /// Bytes per sample (1 or 2).
    pub bytes: u32,
    /// LUT names, one per channel. Three channels with `rgb` set are stored
    /// as interleaved RGB.
    pub luts: Vec<&'static str>,
    pub rgb: bool,
    pub z: u32,
    pub tiles: u32,
    /// Extra bytes at the end of each row.
    pub row_padding: u32,
    /// Store rows along DimID 3 (an XZ line scan) instead of DimID 2.
    pub xz: bool,
    /// Varies the pixel data between images.
    pub seed: usize,
}

pub fn utf16(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
}

impl ImageSpec {
    pub fn simple(luts: &[&'static str]) -> Self {
        ImageSpec {
            width: 64,
            height: 40,
            bytes: 1,
            luts: luts.to_vec(),
            rgb: false,
            z: 1,
            tiles: 1,
            row_padding: 0,
            xz: false,
            seed: 9,
        }
    }

    pub fn channels(&self) -> u32 {
        self.luts.len() as u32 / if self.rgb { 3 } else { 1 }
    }

    fn strides(&self) -> (u64, u64, u64, u64, u64) {
        let samples = if self.rgb { 3 } else { 1 };
        let px = (self.bytes * samples) as u64;
        let row = self.width as u64 * px + self.row_padding as u64;
        let plane = row * self.height as u64;
        let z_inc = plane * self.channels() as u64;
        let s_inc = z_inc * self.z as u64;
        (px, row, plane, z_inc, s_inc)
    }

    /// The raw memory block.
    pub fn data(&self) -> Vec<u8> {
        let (.., s_inc) = self.strides();
        (0..(s_inc * self.tiles as u64) as usize)
            .map(|i| pattern(self.seed, 9, i))
            .collect()
    }

    /// The `<Image>` element describing this block.
    pub fn image_xml(&self) -> String {
        let (px, row, plane, z_inc, s_inc) = self.strides();
        let mut chans = String::new();
        for (i, lut) in self.luts.iter().enumerate() {
            let (tag, inc) = if self.rgb {
                (3, i as u64 * self.bytes as u64)
            } else {
                (0, i as u64 * plane)
            };
            chans += &format!(
                r#"<ChannelDescription DataType="0" ChannelTag="{tag}" Resolution="{}" Min="0" Max="255" Unit="" LUTName="{lut}" IsLUTInverted="0" BytesInc="{inc}" BitInc="0"/>"#,
                self.bytes * 8
            );
        }
        let row_dim = if self.xz { 3 } else { 2 };
        let mut dims = format!(
            r#"<DimensionDescription DimID="1" NumberOfElements="{}" Origin="0" Length="{}" Unit="m" BitInc="0" BytesInc="{px}"/><DimensionDescription DimID="{row_dim}" NumberOfElements="{}" Origin="0" Length="{}" Unit="m" BitInc="0" BytesInc="{row}"/>"#,
            self.width,
            (self.width - 1) as f64 * 0.5e-6,
            self.height,
            (self.height - 1) as f64 * 0.5e-6,
        );
        if self.z > 1 {
            dims += &format!(
                r#"<DimensionDescription DimID="3" NumberOfElements="{}" Origin="0" Length="{}" Unit="m" BitInc="0" BytesInc="{z_inc}"/>"#,
                self.z,
                (self.z - 1) as f64 * 2e-6
            );
        }
        if self.tiles > 1 {
            dims += &format!(
                r#"<DimensionDescription DimID="10" NumberOfElements="{}" Origin="0" Length="0" Unit="" BitInc="0" BytesInc="{s_inc}"/>"#,
                self.tiles
            );
        }
        format!(
            r#"<Image TextDescription=""><ImageDescription><Channels>{chans}</Channels><Dimensions>{dims}</Dimensions></ImageDescription><TimeStampList NumberOfTimeStamps="1">1d9d0f5a7b1c2e0</TimeStampList></Image>"#
        )
    }

    /// Writes a LOF file and returns the raw pixel block.
    pub fn write_lof(&self, path: &Path) -> Vec<u8> {
        let data = self.data();
        let xml = format!(
            r#"<LMSDataContainerHeader Version="2"><Element Name="synthetic" Visibility="1" CopyOption="1" UniqueID="x"><Data>{}</Data></Element></LMSDataContainerHeader>"#,
            self.image_xml()
        );
        let mut f = Vec::new();
        let type_name = utf16("LMS_Object_File");
        let block1_len = 1 + 4 + type_name.len() + 5 + 5 + 1 + 8;
        f.extend(0x70i32.to_le_bytes());
        f.extend((block1_len as i32).to_le_bytes());
        f.push(0x2a);
        f.extend(15i32.to_le_bytes());
        f.extend(&type_name);
        f.push(0x2a);
        f.extend(2i32.to_le_bytes());
        f.push(0x2a);
        f.extend(0i32.to_le_bytes());
        f.push(0x2a);
        f.extend((data.len() as i64).to_le_bytes());
        f.extend(&data);
        let xml16 = utf16(&xml);
        f.extend(0x70i32.to_le_bytes());
        f.extend(((1 + 4 + xml16.len()) as i32).to_le_bytes());
        f.push(0x2a);
        f.extend((xml.encode_utf16().count() as i32).to_le_bytes());
        f.extend(&xml16);
        std::fs::write(path, f).unwrap();
        data
    }
}

/// A synthetic LIF project. Entries are `(folder path, element name, spec)`;
/// `None` specs are folders/elements without pixels.
pub struct LifSpec<'a> {
    pub images: Vec<(&'a str, &'a str, Option<&'a ImageSpec>)>,
    /// 1: 32-bit block sizes; 2: 64-bit.
    pub version: u32,
    /// Write memory blocks in reverse XML order.
    pub reverse_blocks: bool,
    /// Pad the end of the file with zeros, as newer LAS X versions do.
    pub zero_tail: bool,
    /// Add an image under ProcessingHistory, which readers must skip.
    pub processing_history: bool,
    /// Give each image a `FrameProperties` child with its own small,
    /// non-image memory block, as LAS X does. This breaks Bio-Formats'
    /// positional block matching.
    pub frame_properties: bool,
}

impl LifSpec<'_> {
    /// Writes the file and returns each pixel image's raw block, in XML order.
    pub fn write(&self, path: &Path) -> Vec<Vec<u8>> {
        use std::collections::BTreeMap;
        // Build a folder tree: path -> list of child element XML.
        let mut blocks: Vec<(String, Vec<u8>)> = vec![("MemBlock_root".into(), Vec::new())];
        let mut children: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut datas = Vec::new();
        for (n, (folder, name, spec)) in self.images.iter().enumerate() {
            let id = format!("MemBlock_{}", n + 1);
            let (data_xml, data) = match spec {
                Some(s) => (s.image_xml(), s.data()),
                None => (
                    r#"<Collection ChildTypeTest="AcceptAll"/>"#.to_string(),
                    Vec::new(),
                ),
            };
            let history = if self.processing_history && spec.is_some() {
                format!(r#"<ProcessingHistory><Data>{data_xml}</Data></ProcessingHistory>"#)
            } else {
                String::new()
            };
            children.entry(folder.to_string()).or_default().push(format!(
                r#"<Element Name="{name}" Visibility="1" UniqueID="u{n}"><Data>{data_xml}</Data>{history}<Memory Size="{}" MemoryBlockID="{id}"/><Children>{{children:{folder}/{name}}}</Children></Element>"#,
                data.len()
            ));
            if spec.is_some() {
                datas.push(data.clone());
            }
            blocks.push((id, data));
            if self.frame_properties && spec.is_some() {
                let id = format!("MemBlock_fp{}", n + 1);
                children.entry(format!("{folder}/{name}")).or_default().push(format!(
                    r#"<Element Name="FrameProperties" Visibility="0" UniqueID="fp{n}"><Data><SimpleListInMemory/></Data><Memory Size="26" MemoryBlockID="{id}"/><Children>{{children:{folder}/{name}/FrameProperties}}</Children></Element>"#
                ));
                blocks.push((id, vec![7; 26]));
            }
        }
        fn expand(
            path: &str,
            children: &std::collections::BTreeMap<String, Vec<String>>,
        ) -> String {
            children
                .get(path)
                .map(|items| {
                    items
                        .iter()
                        .map(|xml| {
                            let marker_start = xml.find("{children:").unwrap();
                            let marker_end = marker_start + xml[marker_start..].find('}').unwrap();
                            let child_path = &xml[marker_start + 10..marker_end];
                            format!(
                                "{}{}{}",
                                &xml[..marker_start],
                                expand(child_path, children),
                                &xml[marker_end + 1..]
                            )
                        })
                        .collect()
                })
                .unwrap_or_default()
        }
        let xml = format!(
            r#"<LMSDataContainerHeader Version="2"><Element Name="project.lif" Visibility="1" UniqueID="root"><Data><Experiment IsSavedFlag="1"/></Data><Memory Size="0" MemoryBlockID="MemBlock_root"/><Children>{}</Children></Element></LMSDataContainerHeader>"#,
            expand("", &children)
        );

        let mut f = Vec::new();
        let xml16 = utf16(&xml);
        f.extend(0x70i32.to_le_bytes());
        f.extend(((1 + 4 + xml16.len()) as i32).to_le_bytes());
        f.push(0x2a);
        f.extend((xml.encode_utf16().count() as i32).to_le_bytes());
        f.extend(&xml16);
        if self.reverse_blocks {
            blocks[1..].reverse();
        }
        for (id, data) in &blocks {
            let id16 = utf16(id);
            let size_len = if self.version == 1 { 4 } else { 8 };
            f.extend(0x70i32.to_le_bytes());
            f.extend(((1 + size_len + 1 + 4 + id16.len()) as i32).to_le_bytes());
            f.push(0x2a);
            if self.version == 1 {
                f.extend((data.len() as i32).to_le_bytes());
            } else {
                f.extend((data.len() as i64).to_le_bytes());
            }
            f.push(0x2a);
            f.extend((id.encode_utf16().count() as i32).to_le_bytes());
            f.extend(&id16);
            f.extend(data);
        }
        if self.zero_tail {
            f.extend(std::iter::repeat_n(0u8, 4096));
        }
        std::fs::write(path, f).unwrap();
        datas
    }
}

/// The bytes of `tile`, plane (`z`, `c`) as `spec` laid them out, with
/// padding removed and BGR swapped to RGB if `bgr`.
pub fn expected_plane(
    spec: &ImageSpec,
    data: &[u8],
    tile: u32,
    z: u32,
    c: u32,
    bgr: bool,
) -> Vec<u8> {
    let samples = if spec.rgb { 3 } else { 1 };
    let channels = spec.channels();
    let px = (spec.bytes * samples) as usize;
    let row = spec.width as usize * px + spec.row_padding as usize;
    let plane = row * spec.height as usize;
    let base =
        tile as usize * plane * (channels * spec.z) as usize + (z * channels + c) as usize * plane;
    let mut out = Vec::new();
    for y in 0..spec.height as usize {
        out.extend_from_slice(&data[base + y * row..base + y * row + spec.width as usize * px]);
    }
    if bgr {
        bioformats::pixel::bgr_to_rgb(&mut out, true, spec.bytes as usize, 3);
    }
    out
}
