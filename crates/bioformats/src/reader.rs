//! The reader interface. Port of `loci.formats.IFormatReader`.
//!
//! Unlike Bio-Formats, a reader has no "current series" cursor: every call
//! names the series it wants, and readers are `Sync` so that tiles can be
//! read from several threads at once.

use std::path::{Path, PathBuf};

use crate::core::CoreMetadata;
use crate::error::{Error, Result};
use crate::io::RandomAccessFile;
use crate::meta::OmeMetadata;

/// A rectangle within a plane, in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl Region {
    pub const fn new(x: u32, y: u32, w: u32, h: u32) -> Self {
        Region { x, y, w, h }
    }

    pub const fn full(core: &CoreMetadata) -> Self {
        Region {
            x: 0,
            y: 0,
            w: core.size_x,
            h: core.size_y,
        }
    }

    /// Returns the part of `self` that lies within `other`, if any.
    pub fn intersect(&self, other: &Region) -> Option<Region> {
        let x0 = self.x.max(other.x);
        let y0 = self.y.max(other.y);
        let x1 = (self.x + self.w).min(other.x + other.w);
        let y1 = (self.y + self.h).min(other.y + other.h);
        (x0 < x1 && y0 < y1).then(|| Region::new(x0, y0, x1 - x0, y1 - y0))
    }
}

/// Reads pixels and metadata from one dataset (which may span several files).
pub trait FormatReader: Send + Sync {
    /// Human-readable format name, e.g. `"Leica Object Format"`.
    fn format_name(&self) -> &'static str;

    /// Core metadata for every series.
    fn series(&self) -> &[CoreMetadata];

    /// Full OME metadata, one image per series.
    fn metadata(&self) -> &OmeMetadata;

    /// Reads `region` of plane `plane` of `series` into `buf`.
    ///
    /// `buf` must be exactly `core.region_bytes(region.w, region.h)` long. The
    /// bytes are laid out as described by the series' `CoreMetadata`
    /// (`interleaved`, `little_endian`), with rows tightly packed.
    fn open_bytes(&self, series: usize, plane: u32, region: Region, buf: &mut [u8]) -> Result<()>;

    /// The block size this reader is fastest at handing out.
    /// Defaults to full-width strips of roughly 4 MiB.
    fn optimal_tile_size(&self, series: usize) -> (u32, u32) {
        let core = &self.series()[series];
        let row = core.region_bytes(core.size_x, 1).max(1);
        let rows = ((4 << 20) / row).clamp(1, core.size_y as usize) as u32;
        (core.size_x, rows)
    }

    /// Every file that makes up this dataset.
    fn used_files(&self) -> Vec<PathBuf>;
}

impl dyn FormatReader + '_ {
    /// Reads a whole plane into a newly allocated buffer.
    pub fn read_plane(&self, series: usize, plane: u32) -> Result<Vec<u8>> {
        let core = self.core(series)?;
        let mut buf = vec![0; core.plane_bytes()];
        self.open_bytes(series, plane, Region::full(core), &mut buf)?;
        Ok(buf)
    }

    pub fn core(&self, series: usize) -> Result<&CoreMetadata> {
        self.series()
            .get(series)
            .ok_or_else(|| Error::InvalidArgument(format!("series {series} out of range")))
    }
}

/// Validates `open_bytes` arguments. Port of `FormatTools.checkPlaneParameters`.
pub fn check_plane_args(
    cores: &[CoreMetadata],
    series: usize,
    plane: u32,
    region: Region,
    buf_len: usize,
) -> Result<&CoreMetadata> {
    let core = cores.get(series).ok_or_else(|| {
        Error::InvalidArgument(format!(
            "series {series} out of range ({} series)",
            cores.len()
        ))
    })?;
    if plane >= core.image_count {
        return Err(Error::InvalidArgument(format!(
            "plane {plane} out of range (image count {})",
            core.image_count
        )));
    }
    if region.w == 0
        || region.h == 0
        || region.x as u64 + region.w as u64 > core.size_x as u64
        || region.y as u64 + region.h as u64 > core.size_y as u64
    {
        return Err(Error::InvalidArgument(format!(
            "region {region:?} outside {}x{} plane",
            core.size_x, core.size_y
        )));
    }
    let need = core.region_bytes(region.w, region.h);
    if buf_len != need {
        return Err(Error::InvalidArgument(format!(
            "buffer is {buf_len} bytes, region needs {need}"
        )));
    }
    Ok(core)
}

/// Describes a readable format: how to recognise it and how to open it.
pub struct ReaderFormat {
    pub name: &'static str,
    /// Lower-case suffixes without the leading dot.
    pub suffixes: &'static [&'static str],
    /// Returns true if the file's contents look like this format.
    pub is_this_type: fn(&Path, &RandomAccessFile) -> bool,
    pub open: fn(&Path) -> Result<Box<dyn FormatReader>>,
}

/// True if `path` ends with `.suffix` for any of `suffixes` (case-insensitive).
pub fn has_suffix(path: &Path, suffixes: &[&str]) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    suffixes.iter().any(|s| name.ends_with(&format!(".{s}")))
}
