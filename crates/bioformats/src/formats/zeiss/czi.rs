//! Zeiss CZI reader. Port of `loci.formats.in.ZeissCZIReader`.
//!
//! A CZI file is a sequence of 32-byte-aligned segments (`ZISRAWFILE`,
//! `ZISRAWDIRECTORY`, `ZISRAWSUBBLOCK`, `ZISRAWMETADATA`, ...). Pixels live in
//! subblocks, each tagged with its position along X, Y and the other
//! dimensions (C, Z, T, S scene, M mosaic tile, B, H phase, R rotation,
//! I illumination, V view). Pyramid levels are subblocks whose stored size is
//! smaller than their logical size.
//!
//! How subblocks become series follows Bio-Formats' defaults: mosaic tiles
//! are stitched per scene ("autostitching") and each pyramid level becomes
//! its own series ("flattened resolutions"). Bio-Formats has further
//! heuristics (PALM data, scan lines, multi-file datasets, mixed pixel
//! types) that are not ported yet.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::czi_meta::{self, CziMetadata};
use crate::core::CoreMetadata;
use crate::dims::{DimensionOrder, Zct};
use crate::error::{Result, format_err, unsupported};
use crate::io::RandomAccessFile;
use crate::meta::{ChannelMetadata, ImageMetadata, OmeMetadata, Quantity};
use crate::pixel::PixelType;
use crate::reader::{FormatReader, ReaderFormat, Region, check_plane_args};

pub const FORMAT: ReaderFormat = ReaderFormat {
    name: "Zeiss CZI",
    suffixes: &["czi"],
    is_this_type: |_, file| file.read_prefix(0, 10).is_ok_and(|h| h == b"ZISRAWFILE"),
    open: |path| Ok(Box::new(CziReader::open(path)?)),
};

const HEADER_SIZE: u64 = 32;

// Compression schemes.
const UNCOMPRESSED: i32 = 0;
const ZSTD_0: i32 = 5;
const ZSTD_1: i32 = 6;

/// Bytes of decoded subblocks kept in memory, so strip-wise reads of a
/// compressed tile do not decode it once per strip.
const CACHE_BYTES: usize = 256 << 20;

/// Port of `convertPixelType`: (sample type, samples per pixel).
fn pixel_type(code: i32) -> Result<(PixelType, u32)> {
    Ok(match code {
        0 => (PixelType::Uint8, 1),
        1 => (PixelType::Uint16, 1),
        2 => (PixelType::Float, 1),
        3 => (PixelType::Uint8, 3),
        4 => (PixelType::Uint16, 3),
        8 => (PixelType::Float, 3),
        9 => (PixelType::Uint8, 4),
        12 => (PixelType::Uint32, 1),
        13 => (PixelType::Double, 1),
        10 | 11 => return Err(unsupported!("complex CZI pixel data")),
        other => return Err(format_err!("unknown CZI pixel type {other}")),
    })
}

#[derive(Debug, Clone, Copy, Default)]
struct Dim {
    start: i32,
    size: i32,
    stored: i32,
}

#[derive(Debug, Clone)]
struct SubBlock {
    pixel_type: i32,
    compression: i32,
    pyramid_type: u8,
    segment: u64,
    x: Dim,
    y: Dim,
    /// Every other dimension, keyed by its letter.
    other: BTreeMap<char, Dim>,
    data_offset: u64,
    data_size: u64,
    level: u32,
}

impl SubBlock {
    fn get(&self, d: char) -> Option<Dim> {
        self.other.get(&d).copied()
    }

    fn start(&self, d: char) -> i32 {
        self.get(d).map_or(0, |d| d.start)
    }
}

/// Reads a directory entry (`DV` schema). Port of `DirectoryEntry`.
fn read_entry(cur: &mut crate::io::Cursor) -> Result<(SubBlock, i32)> {
    let mut schema = [0; 2];
    cur.read_exact(&mut schema)?;
    let pixel_type = cur.read_i32()?;
    let segment = cur.read_i64()? as u64;
    let file_part = cur.read_i32()?;
    let compression = cur.read_i32()?;
    let pyramid_type = cur.read_u8()?;
    cur.skip(5);
    let count = cur.read_i32()?;
    if !(0..=64).contains(&count) {
        return Err(format_err!("CZI directory entry has {count} dimensions"));
    }
    let mut sb = SubBlock {
        pixel_type,
        compression,
        pyramid_type,
        segment,
        x: Dim::default(),
        y: Dim::default(),
        other: BTreeMap::new(),
        data_offset: 0,
        data_size: 0,
        level: 0,
    };
    let mut have_x = false;
    let mut have_y = false;
    for _ in 0..count {
        let mut name = [0; 4];
        cur.read_exact(&mut name)?;
        let start = cur.read_i32()?;
        let size = cur.read_i32()?;
        cur.skip(4); // start coordinate (float)
        let stored = cur.read_i32()?;
        let name = String::from_utf8_lossy(&name)
            .trim_matches(['\0', ' '])
            .to_owned();
        let dim = Dim {
            start,
            size,
            stored,
        };
        let mut chars = name.chars();
        match (chars.next(), chars.next()) {
            (Some('X'), None) => {
                sb.x = dim;
                have_x = true;
            }
            (Some('Y'), None) => {
                sb.y = dim;
                have_y = true;
            }
            (Some(c), None) => {
                sb.other.insert(c, dim);
            }
            // Bio-Formats ignores multi-character dimensions.
            _ => {}
        }
    }
    if !have_x || !have_y {
        return Err(format_err!("CZI subblock at {segment} lacks X or Y"));
    }
    Ok((sb, file_part))
}

/// Where one subblock is drawn in a plane of a series.
#[derive(Debug, Clone, Copy)]
struct Placement {
    block: usize,
    /// Position in the series' (scaled) coordinates.
    x: i64,
    y: i64,
}

pub struct CziReader {
    path: PathBuf,
    file: RandomAccessFile,
    blocks: Vec<SubBlock>,
    cores: Vec<CoreMetadata>,
    meta: OmeMetadata,
    /// `(core, plane)` -> subblocks to draw, in file order.
    placements: HashMap<(usize, u32), Vec<Placement>>,
    /// Bytes per pixel including samples.
    bpp: usize,
    fill: u8,
    tile_size: Vec<(u32, u32)>,
    cache: Mutex<VecDeque<(usize, Arc<Vec<u8>>)>>,
}

impl CziReader {
    pub fn open(path: &Path) -> Result<Self> {
        let file = RandomAccessFile::open(path)?;
        if file.read_prefix(0, 10)? != b"ZISRAWFILE" {
            return Err(format_err!("{} is not a CZI file", path.display()));
        }
        // File header: version, GUIDs, file part, directory and metadata positions.
        let mut cur = file.cursor(HEADER_SIZE + 16 + 32, true);
        let file_part = cur.read_i32()?;
        let directory = cur.read_i64()? as u64;
        let metadata = cur.read_i64()? as u64;
        if file_part != 0 {
            return Err(unsupported!(
                "multi-file CZI datasets (this is part {file_part})"
            ));
        }

        // Use the directory when it is intact; otherwise (e.g. a truncated
        // file) scan the segments, which is all Bio-Formats ever does.
        let mut blocks = match read_directory(&file, directory) {
            Ok(b) => b,
            Err(_) => scan_segments(&file)?,
        };
        // Draw order is file order, as Bio-Formats scans segments sequentially.
        blocks.sort_by_key(|b| b.segment);
        for b in &mut blocks {
            // Subblock segment: header, then sizes, a copy of the directory
            // entry padded to 256 bytes, metadata, then pixel data.
            if b.segment + HEADER_SIZE + 16 > file.len() {
                b.data_size = 0;
                continue;
            }
            let mut c = file.cursor(b.segment + HEADER_SIZE, true);
            let metadata_size = c.read_i32()? as u64;
            c.read_i32()?; // attachment size
            b.data_size = c.read_i64()? as u64;
            let entry_len = 32 + 20 * (b.other.len() as u64 + 2);
            b.data_offset = b.segment + HEADER_SIZE + (16 + entry_len).max(256) + metadata_size;
        }

        let xml = read_metadata(&file, metadata)?;
        let cmeta = czi_meta::parse(&xml);

        let first = blocks.first().ok_or_else(|| {
            format_err!("pixel data could not be found; this file may be corrupted")
        })?;
        if blocks.iter().any(|b| b.pixel_type != first.pixel_type) {
            return Err(unsupported!("CZI files mixing pixel types"));
        }
        let (ty, samples) = pixel_type(first.pixel_type)?;
        let bpp = ty.bytes() * samples as usize;

        // -- pyramid levels (the "remove any invalid SubBlocks" pass) --
        let mut scale_factor = 0i64;
        let mut keep = Vec::with_capacity(blocks.len());
        for mut b in blocks {
            let stored = (b.x.stored.max(1) as i64, b.y.stored.max(1) as i64);
            if b.compression == UNCOMPRESSED
                && b.data_size < (stored.0 * stored.1) as u64 * bpp as u64
            {
                continue; // truncated block
            }
            let mut scale = (b.x.size as f64 / stored.0 as f64 + 0.5) as i64;
            if scale > 1 {
                // Reduced blocks must be pyramid blocks (types 1 and 2) and
                // a multiple of 2 or 3 smaller, or Bio-Formats drops them.
                let pyramid = matches!(b.pyramid_type, 1 | 2);
                if (b.compression == UNCOMPRESSED && !pyramid) || (scale % 2 != 0 && scale % 3 != 0)
                {
                    continue;
                }
                if scale_factor == 0 {
                    scale_factor = if scale % 2 == 0 { 2 } else { 3 };
                }
                while scale > 1 {
                    scale /= scale_factor;
                    b.level += 1;
                }
            }
            keep.push(b);
        }
        let blocks = keep;
        if blocks.is_empty() {
            return Err(format_err!(
                "pixel data could not be found; this file may be corrupted"
            ));
        }
        let max_level = blocks.iter().map(|b| b.level).max().unwrap_or(0);

        // -- dimension sizes (calculateDimensions) --
        let mut size_c = 0i32;
        let mut size_z = 0i32;
        let mut ts = BTreeSet::new();
        let range = |d: char| {
            let starts: Vec<(i32, i32)> = blocks
                .iter()
                .filter_map(|b| b.get(d))
                .map(|d| (d.start, d.start + d.size))
                .collect();
            let min = starts.iter().map(|s| s.0).min();
            let max = starts.iter().map(|s| s.1).max();
            match (min, max) {
                (Some(a), Some(b)) => (b - a).max(1),
                _ => 1,
            }
        };
        let (phases, rotations, illuminations) = (range('H'), range('R'), range('I'));
        for b in &blocks {
            if let Some(c) = b.get('C') {
                size_c = size_c.max(c.start + 1);
            }
            if let Some(z) = b.get('Z') {
                size_z = if z.start > 0 && z.start >= size_z {
                    z.start + 1
                } else {
                    size_z.max(z.size)
                };
            }
            if let Some(t) = b.get('T') {
                ts.extend(t.start..t.start + t.size.max(1));
            }
        }
        let size_c = size_c.max(1) as u32;
        let size_z = size_z.max(1) as u32;
        let size_t = (ts.len() as u32).max(1);

        let mut proto = CoreMetadata::new(1, 1, ty);
        proto.size_z = size_z * rotations as u32;
        proto.size_c = size_c * illuminations as u32 * samples;
        proto.size_t = size_t * phases as u32;
        proto.rgb_channel_count = samples;
        proto.interleaved = samples > 1;
        proto.little_endian = true;
        proto.dimension_order = DimensionOrder::XYCZT;
        if let Some(bits) = cmeta.bits_per_pixel
            && bits > 0
            && bits <= ty.bytes() as u32 * 8
        {
            proto.bits_per_pixel = bits;
        }
        proto.update_image_count();

        // -- series (assignPlaneIndices) --
        // Scenes, acquisitions and views split series; mosaic tiles are
        // stitched within them.
        let min_s = blocks
            .iter()
            .filter_map(|b| b.get('S'))
            .map(|d| d.start)
            .min()
            .unwrap_or(0);
        let key = |b: &SubBlock| (b.start('S') - min_s, b.start('B'), b.start('V'));
        let keys: BTreeSet<_> = blocks.iter().map(key).collect();
        let series_of: HashMap<_, usize> = keys.iter().enumerate().map(|(i, k)| (*k, i)).collect();

        // Levels actually present per series.
        let mut levels: Vec<BTreeSet<u32>> = vec![BTreeSet::new(); keys.len()];
        for b in &blocks {
            levels[series_of[&key(b)]].insert(b.level);
        }
        // Missing reduced levels are dropped, but if the full-resolution
        // level is missing (e.g. a truncated file) Bio-Formats keeps every
        // level, blank ones included ("keepMissingPyramid").
        for lv in &mut levels {
            if !lv.contains(&0) {
                *lv = (0..=max_level).collect();
            }
        }

        let sf = scale_factor.max(1);
        let mut cores = Vec::new();
        let mut core_of: HashMap<(usize, u32), usize> = HashMap::new();
        let mut origin = Vec::new(); // per series: (min x, min y)
        let mut tile_size = Vec::new();
        for (s, lv) in levels.iter().enumerate() {
            // Level-0 bounding box gives the stitched size.
            let base: Vec<&SubBlock> = blocks
                .iter()
                .filter(|b| series_of[&key(b)] == s && b.level == 0)
                .collect();
            let all: Vec<&SubBlock> = blocks.iter().filter(|b| series_of[&key(b)] == s).collect();
            let src = if base.is_empty() { &all } else { &base };
            // Tiles are placed relative to the full-resolution origin; the
            // size covers every level's logical extent, as in Bio-Formats
            // (coarse pyramid blocks can extend slightly past the tiles).
            let x0 = src.iter().map(|b| b.x.start as i64).min().unwrap();
            let y0 = src.iter().map(|b| b.y.start as i64).min().unwrap();
            let x1 = all
                .iter()
                .map(|b| b.x.start as i64 + b.x.size as i64)
                .max()
                .unwrap();
            let y1 = all
                .iter()
                .map(|b| b.y.start as i64 + b.y.size as i64)
                .max()
                .unwrap();
            let tw = src.iter().map(|b| b.x.size).max().unwrap_or(1).max(1) as u32;
            let th = src.iter().map(|b| b.y.size).max().unwrap_or(1).max(1) as u32;
            origin.push((x0, y0));
            for &level in lv {
                let div = sf.pow(level);
                let mut core = proto.clone();
                core.size_x = (((x1 - x0) / div).max(1)) as u32;
                core.size_y = (((y1 - y0) / div).max(1)) as u32;
                core_of.insert((s, level), cores.len());
                cores.push(core);
                tile_size.push((
                    (tw as i64 / div).max(1) as u32,
                    (th as i64 / div).max(1) as u32,
                ));
            }
        }

        // -- plane indices and placements --
        let mut placements: HashMap<(usize, u32), Vec<Placement>> = HashMap::new();
        for (i, b) in blocks.iter().enumerate() {
            let s = series_of[&key(b)];
            let core_index = core_of[&(s, b.level)];
            let core = &cores[core_index];
            let mut z = b.start('Z').clamp(0, size_z as i32 - 1) as u32;
            let mut c = b.start('C').max(0) as u32;
            let mut t = b.start('T').clamp(0, size_t as i32 - 1) as u32;
            if rotations > 1 {
                z += b.start('R') as u32 * size_z;
            }
            if illuminations > 1 {
                c += b.start('I') as u32 * size_c;
            }
            if phases > 1 {
                t += b.start('H') as u32 * size_t;
            }
            let zct = Zct {
                z,
                c: c.min(core.effective_size_c() - 1),
                t,
            };
            let plane = core.plane_index(zct)?;
            let div = sf.pow(b.level);
            let (x0, y0) = origin[s];
            placements
                .entry((core_index, plane))
                .or_default()
                .push(Placement {
                    block: i,
                    x: (b.x.start as i64 - x0) / div,
                    y: (b.y.start as i64 - y0) / div,
                });
        }

        let meta = build_metadata(path, &cmeta, &cores, &core_of, &levels, sf);
        Ok(CziReader {
            path: path.to_owned(),
            file,
            blocks,
            cores,
            meta,
            placements,
            bpp,
            // Bio-Formats fills gaps in pyramidal RGB mosaics with white.
            fill: if samples > 1 && max_level > 0 { 255 } else { 0 },
            tile_size,
            cache: Mutex::new(VecDeque::new()),
        })
    }

    /// The decoded pixels of a compressed subblock (cached).
    fn decoded(&self, index: usize) -> Result<Arc<Vec<u8>>> {
        if let Some((_, data)) = self.cache.lock().unwrap().iter().find(|(i, _)| *i == index) {
            return Ok(data.clone());
        }
        let b = &self.blocks[index];
        let expected = b.x.stored.max(0) as usize * b.y.stored.max(0) as usize * self.bpp;
        let raw = self.file.read_vec_at(b.data_offset, b.data_size as usize)?;
        let data = Arc::new(decode(b.compression, &raw, expected)?);
        let mut cache = self.cache.lock().unwrap();
        cache.push_back((index, data.clone()));
        let mut total: usize = cache.iter().map(|(_, d)| d.len()).sum();
        while total > CACHE_BYTES && cache.len() > 1 {
            total -= cache.pop_front().unwrap().1.len();
        }
        Ok(data)
    }
}

fn read_directory(file: &RandomAccessFile, position: u64) -> Result<Vec<SubBlock>> {
    let id = file.read_prefix(position, 16)?;
    if !id.starts_with(b"ZISRAWDIRECTORY") {
        return Err(unsupported!("CZI files without a subblock directory"));
    }
    let mut cur = file.cursor(position + HEADER_SIZE, true);
    let count = cur.read_i32()?;
    cur.skip(124);
    let mut blocks = Vec::with_capacity(count.max(0) as usize);
    for _ in 0..count {
        let (sb, part) = read_entry(&mut cur)?;
        if part != 0 {
            return Err(unsupported!("multi-file CZI datasets"));
        }
        blocks.push(sb);
    }
    Ok(blocks)
}

/// Walks the 32-byte-aligned segments from the start of the file, collecting
/// subblocks. Port of `readSegments`.
fn scan_segments(file: &RandomAccessFile) -> Result<Vec<SubBlock>> {
    let mut blocks = Vec::new();
    let mut pos = 0u64;
    while pos + HEADER_SIZE <= file.len() {
        let header = file.read_vec_at(pos, HEADER_SIZE as usize)?;
        let id = String::from_utf8_lossy(&header[..16])
            .trim_matches(['\0', ' '])
            .to_owned();
        let allocated = u64::from_le_bytes(header[16..24].try_into().unwrap());
        if id.is_empty() {
            break;
        }
        if id == "ZISRAWSUBBLOCK" && pos + HEADER_SIZE + 16 + 32 <= file.len() {
            let mut cur = file.cursor(pos + HEADER_SIZE + 16, true);
            if let Ok((mut sb, part)) = read_entry(&mut cur) {
                if part != 0 {
                    return Err(unsupported!("multi-file CZI datasets"));
                }
                sb.segment = pos;
                blocks.push(sb);
            }
        }
        let next = pos + HEADER_SIZE + allocated;
        pos = next.div_ceil(32) * 32;
    }
    Ok(blocks)
}

fn read_metadata(file: &RandomAccessFile, position: u64) -> Result<String> {
    if position == 0
        || !file
            .read_prefix(position, 16)?
            .starts_with(b"ZISRAWMETADATA")
    {
        return Ok(String::new());
    }
    let mut cur = file.cursor(position + HEADER_SIZE, true);
    let size = cur.read_i32()?.max(0) as usize;
    cur.skip(4 + 248);
    let bytes = cur.read_vec(size)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Port of `readVarint` (at most three bytes).
fn read_varint(data: &[u8], pos: &mut usize) -> Result<usize> {
    let mut value = 0usize;
    for i in 0..3 {
        let byte = *data
            .get(*pos)
            .ok_or_else(|| format_err!("truncated zstd header"))?;
        *pos += 1;
        value |= ((byte & 0x7f) as usize) << (7 * i);
        if byte & 0x80 == 0 || i == 2 {
            if i == 2 {
                value |= ((byte & 0x80) as usize) << 14;
            }
            break;
        }
    }
    Ok(value)
}

/// Decodes a compressed subblock to exactly `expected` bytes.
fn decode(compression: i32, raw: &[u8], expected: usize) -> Result<Vec<u8>> {
    let mut out = match compression {
        ZSTD_0 => zstd::bulk::decompress(raw, expected).map_err(|e| format_err!("zstd: {e}"))?,
        ZSTD_1 => {
            let mut pos = 0;
            let header = read_varint(raw, &mut pos)?;
            let mut hi_lo = false;
            while pos < header {
                match read_varint(raw, &mut pos)? {
                    1 => {
                        hi_lo = raw.get(pos).is_some_and(|p| p & 1 == 1);
                        pos += 1;
                    }
                    other => return Err(format_err!("invalid CZI zstd chunk ID {other}")),
                }
            }
            let decoded = zstd::bulk::decompress(&raw[pos.min(raw.len())..], expected)
                .map_err(|e| format_err!("zstd: {e}"))?;
            if hi_lo {
                // Low bytes first, then high bytes; interleave them back.
                let half = decoded.len() / 2;
                let mut out = vec![0; decoded.len()];
                for i in 0..decoded.len() {
                    out[i] = if i % 2 == 0 {
                        decoded[i / 2]
                    } else {
                        decoded[half + i / 2]
                    };
                }
                out
            } else {
                decoded
            }
        }
        4 => return Err(unsupported!("JPEG-XR-compressed CZI")),
        1 => return Err(unsupported!("JPEG-compressed CZI")),
        2 => return Err(unsupported!("LZW-compressed CZI")),
        other => return Err(unsupported!("CZI compression {other}")),
    };
    out.resize(expected, 0);
    Ok(out)
}

fn build_metadata(
    path: &Path,
    cmeta: &CziMetadata,
    cores: &[CoreMetadata],
    core_of: &HashMap<(usize, u32), usize>,
    levels: &[BTreeSet<u32>],
    sf: i64,
) -> OmeMetadata {
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = cmeta
        .name
        .clone()
        .filter(|n| !n.trim().is_empty())
        .unwrap_or(file_name);
    let width = cores.len().to_string().len();
    let mut images = vec![ImageMetadata::default(); cores.len()];
    for (s, lv) in levels.iter().enumerate() {
        for &level in lv {
            let i = core_of[&(s, level)];
            let core = &cores[i];
            // Pixel size at this level. Bio-Formats reports the full-resolution
            // size on every level; scaling it is what the pixels mean.
            let scale = sf.pow(level) as f64;
            let image = &mut images[i];
            image.name = Some(format!("{name} #{:0width$}", i + 1));
            image.description = cmeta.description.clone();
            image.acquisition_date = cmeta.acquisition_date.clone();
            image.physical_size_x = cmeta.physical_x.map(|v| Quantity::micrometers(v * scale));
            image.physical_size_y = cmeta.physical_y.map(|v| Quantity::micrometers(v * scale));
            image.physical_size_z = cmeta.physical_z.map(Quantity::micrometers);
            image.channels = (0..core.effective_size_c() as usize)
                .map(|c| {
                    let ch = cmeta.channels.get(c).cloned().unwrap_or_default();
                    ChannelMetadata {
                        name: ch.name,
                        samples_per_pixel: core.rgb_channel_count,
                        color: if core.is_rgb() { None } else { ch.color },
                        emission_wavelength: ch.emission,
                        excitation_wavelength: ch.excitation,
                        fluor: ch.fluor,
                    }
                })
                .collect();
        }
    }
    OmeMetadata {
        images,
        global_metadata: cmeta.original.clone(),
    }
}

impl FormatReader for CziReader {
    fn format_name(&self) -> &'static str {
        FORMAT.name
    }

    fn series(&self) -> &[CoreMetadata] {
        &self.cores
    }

    fn metadata(&self) -> &OmeMetadata {
        &self.meta
    }

    fn open_bytes(&self, series: usize, plane: u32, region: Region, buf: &mut [u8]) -> Result<()> {
        let core = check_plane_args(&self.cores, series, plane, region, buf.len())?;
        buf.fill(self.fill);
        let bpp = self.bpp;
        let out_row = region.w as usize * bpp;
        let Some(list) = self.placements.get(&(series, plane)) else {
            return Ok(());
        };
        let rx = (
            region.x as i64,
            region.y as i64,
            region.x as i64 + region.w as i64,
            region.y as i64 + region.h as i64,
        );
        for p in list {
            let b = &self.blocks[p.block];
            let (tw, th) = (b.x.stored.max(0) as i64, b.y.stored.max(0) as i64);
            let (x0, y0) = (p.x.max(rx.0), p.y.max(rx.1));
            let (x1, y1) = ((p.x + tw).min(rx.2), (p.y + th).min(rx.3));
            if x0 >= x1 || y0 >= y1 {
                continue;
            }
            let copy = (x1 - x0) as usize * bpp;
            let tile_row = tw as usize * bpp;
            let dst_col = (x0 - rx.0) as usize * bpp;
            let src_col = (x0 - p.x) as usize * bpp;
            if b.compression == UNCOMPRESSED {
                for y in y0..y1 {
                    let dst = (y - rx.1) as usize * out_row + dst_col;
                    let src = b.data_offset + ((y - p.y) as usize * tile_row + src_col) as u64;
                    if src + copy as u64 > self.file.len() {
                        // Truncated file: keep whatever bytes exist and leave
                        // the fill value after them, as Bio-Formats does.
                        let have = self.file.len().saturating_sub(src) as usize;
                        if have > 0 {
                            self.file.read_exact_at(&mut buf[dst..dst + have], src)?;
                        }
                        break;
                    }
                    self.file.read_exact_at(&mut buf[dst..dst + copy], src)?;
                }
            } else {
                let data = self.decoded(p.block)?;
                for y in y0..y1 {
                    let dst = (y - rx.1) as usize * out_row + dst_col;
                    let src = (y - p.y) as usize * tile_row + src_col;
                    buf[dst..dst + copy].copy_from_slice(&data[src..src + copy]);
                }
            }
        }
        if core.is_rgb() {
            // Stored as BGR(A); swap red and blue.
            let bytes = core.pixel_type.bytes();
            for px in buf.chunks_exact_mut(bpp) {
                for i in 0..bytes {
                    px.swap(i, 2 * bytes + i);
                }
            }
        }
        Ok(())
    }

    fn optimal_tile_size(&self, series: usize) -> (u32, u32) {
        let core = &self.cores[series];
        let (tw, th) = self.tile_size[series];
        if tw >= core.size_x {
            // Single-block planes: strips are as good as anything.
            let row = core.region_bytes(core.size_x, 1).max(1);
            (
                core.size_x,
                ((4 << 20) / row).clamp(1, core.size_y as usize) as u32,
            )
        } else {
            (tw.min(core.size_x), th.min(core.size_y))
        }
    }

    fn used_files(&self) -> Vec<PathBuf> {
        vec![self.path.clone()]
    }
}
