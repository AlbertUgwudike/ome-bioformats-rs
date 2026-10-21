//! TIFF / BigTIFF / OME-TIFF writer. Port of `loci.formats.out.TiffWriter`,
//! `OMETiffWriter` and `loci.formats.tiff.TiffSaver`.
//!
//! Pixel data is streamed to disk as it arrives; the IFDs (and the OME-XML,
//! which needs to know where every plane went) are written at the end, and
//! the header is patched to point at them.

use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use super::{field, is_ome_tiff_name, tag};
use crate::codec::{Compression, predictor};
use crate::core::CoreMetadata;
use crate::dims::DimensionOrder;
use crate::error::{Error, Result, format_err};
use crate::meta::{OmeMetadata, Quantity};
use crate::ome_xml::{self, TiffData, XmlOptions};
use crate::reader::{Region, has_suffix};
use crate::writer::{FormatWriter, WriterOptions};

/// Target uncompressed size of a strip.
const STRIP_BYTES: usize = 1 << 20;
/// Switch to BigTIFF when uncompressed pixels exceed this (Bio-Formats' cutoff
/// leaves similar headroom for IFDs and metadata).
const BIG_TIFF_CUTOFF: u64 = (1 << 32) - (1 << 27);

struct Layout {
    tiled: bool,
    chunk_w: u32,
    chunk_h: u32,
    chunks: Vec<Region>,
}

struct IfdRecord {
    series: usize,
    plane: u32,
    offsets: Vec<u64>,
    counts: Vec<u64>,
}

pub struct TiffWriter {
    path: PathBuf,
    out: BufWriter<File>,
    pos: u64,
    big_tiff: bool,
    ome: bool,
    cores: Vec<CoreMetadata>,
    meta: OmeMetadata,
    options: WriterOptions,
    layouts: Vec<Layout>,
    ifds: Vec<IfdRecord>,
}

impl TiffWriter {
    /// Creates `path`. Names ending in `.ome.tif(f)` get OME-XML; `.btf`,
    /// `.tf2` and `.tf8` force BigTIFF.
    ///
    /// `cores` must describe interleaved, little-endian data, which is what
    /// [`FormatWriter::encode_chunk`] receives.
    pub fn create(
        path: &Path,
        cores: &[CoreMetadata],
        meta: &OmeMetadata,
        options: &WriterOptions,
    ) -> Result<Self> {
        if cores.is_empty() {
            return Err(Error::InvalidArgument("nothing to write: no series".into()));
        }
        if let Some((w, h)) = options.tile_size
            && (w == 0 || h == 0 || w % 16 != 0 || h % 16 != 0)
        {
            return Err(Error::InvalidArgument(format!(
                "TIFF tile size {w}x{h} must be a non-zero multiple of 16"
            )));
        }
        let total: u64 = cores
            .iter()
            .map(|c| c.plane_bytes() as u64 * c.image_count as u64)
            .sum();
        let big_tiff = match options.big_tiff {
            Some(b) => b,
            None => has_suffix(path, &["btf", "tf2", "tf8"]) || total >= BIG_TIFF_CUTOFF,
        };
        let layouts = cores.iter().map(|c| layout(c, options.tile_size)).collect();

        let mut out = BufWriter::with_capacity(4 << 20, File::create(path)?);
        let header: &[u8] = if big_tiff {
            b"II\x2b\x00\x08\x00\x00\x00"
        } else {
            b"II\x2a\x00"
        };
        out.write_all(header)?;
        let placeholder = if big_tiff { 8 } else { 4 };
        out.write_all(&vec![0; placeholder])?;
        let mut meta = meta.clone();
        meta.conform_to(cores);
        Ok(TiffWriter {
            path: path.to_owned(),
            out,
            pos: (header.len() + placeholder) as u64,
            big_tiff,
            ome: is_ome_tiff_name(path),
            cores: cores.to_vec(),
            meta,
            options: options.clone(),
            layouts,
            ifds: Vec::new(),
        })
    }

    pub fn is_big_tiff(&self) -> bool {
        self.big_tiff
    }

    fn uses_predictor(&self, series: usize) -> bool {
        self.options.predictor
            && self.options.compression != Compression::None
            && !self.cores[series].pixel_type.is_float()
    }

    fn check_offset(&self, end: u64) -> Result<()> {
        if !self.big_tiff && end > u32::MAX as u64 {
            return Err(format_err!(
                "{} would exceed 4 GiB; write BigTIFF instead (e.g. a .btf suffix or --bigtiff)",
                self.path.display()
            ));
        }
        Ok(())
    }

    fn description(&self) -> String {
        if self.ome {
            let file_name = self
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let uuid = format!("urn:uuid:{}", uuid::Uuid::new_v4());
            let mut tiff_data: Vec<Vec<TiffData>> = vec![Vec::new(); self.cores.len()];
            for (i, rec) in self.ifds.iter().enumerate() {
                let zct = self.cores[rec.series].zct(rec.plane).unwrap_or_default();
                tiff_data[rec.series].push(TiffData {
                    ifd: i as u32,
                    zct,
                    plane_count: 1,
                    uuid: Some((file_name.clone(), uuid.clone())),
                });
            }
            ome_xml::write(
                &self.cores,
                &self.meta,
                &XmlOptions {
                    uuid: Some(&uuid),
                    tiff_data: &tiff_data,
                    original_metadata: self.options.original_metadata,
                },
            )
        } else {
            imagej_description(&self.cores, &self.meta)
        }
    }

    fn ifd_entries(&self, index: usize, description: Option<&str>) -> Vec<Entry> {
        let rec = &self.ifds[index];
        let core = &self.cores[rec.series];
        let layout = &self.layouts[rec.series];
        let spp = core.rgb_channel_count as u16;
        let bits = (core.pixel_type.bytes() * 8) as u16;
        let mut e = vec![
            Entry::longs(tag::NEW_SUBFILE_TYPE, &[0]),
            Entry::longs(tag::IMAGE_WIDTH, &[core.size_x]),
            Entry::longs(tag::IMAGE_LENGTH, &[core.size_y]),
            Entry::shorts(tag::BITS_PER_SAMPLE, &vec![bits; spp as usize]),
            Entry::shorts(tag::COMPRESSION, &[self.options.compression.tiff_code()]),
            Entry::shorts(
                tag::PHOTOMETRIC_INTERPRETATION,
                &[if spp == 3 || spp == 4 { 2 } else { 1 }],
            ),
            Entry::shorts(tag::SAMPLES_PER_PIXEL, &[spp]),
            Entry::shorts(tag::PLANAR_CONFIGURATION, &[1]),
            Entry::ascii(tag::SOFTWARE, ome_xml::CREATOR),
        ];
        if let Some(d) = description {
            e.push(Entry::ascii(tag::IMAGE_DESCRIPTION, d));
        }
        let offsets = Entry::offsets(self.big_tiff, &rec.offsets);
        let counts = Entry::offsets(self.big_tiff, &rec.counts);
        if layout.tiled {
            e.push(Entry::longs(tag::TILE_WIDTH, &[layout.chunk_w]));
            e.push(Entry::longs(tag::TILE_LENGTH, &[layout.chunk_h]));
            e.push(Entry {
                tag: tag::TILE_OFFSETS,
                ..offsets
            });
            e.push(Entry {
                tag: tag::TILE_BYTE_COUNTS,
                ..counts
            });
        } else {
            e.push(Entry::longs(tag::ROWS_PER_STRIP, &[layout.chunk_h]));
            e.push(Entry {
                tag: tag::STRIP_OFFSETS,
                ..offsets
            });
            e.push(Entry {
                tag: tag::STRIP_BYTE_COUNTS,
                ..counts
            });
        }
        if self.uses_predictor(rec.series) {
            e.push(Entry::shorts(tag::PREDICTOR, &[2]));
        }
        let extra = match spp {
            2 => 1,
            s if s > 3 => s - 3,
            _ => 0,
        };
        if extra > 0 {
            e.push(Entry::shorts(tag::EXTRA_SAMPLES, &vec![0; extra as usize]));
        }
        let format = if core.pixel_type.is_float() {
            3
        } else if core.pixel_type.is_signed() {
            2
        } else {
            1
        };
        e.push(Entry::shorts(
            tag::SAMPLE_FORMAT,
            &vec![format; spp as usize],
        ));
        let image = &self.meta.images[rec.series];
        if let (Some(x), Some(y)) = (
            pixels_per_cm(image.physical_size_x.as_ref()),
            pixels_per_cm(image.physical_size_y.as_ref()),
        ) {
            e.push(Entry::rational(tag::X_RESOLUTION, x));
            e.push(Entry::rational(tag::Y_RESOLUTION, y));
            e.push(Entry::shorts(tag::RESOLUTION_UNIT, &[3]));
        }
        e.sort_by_key(|e| e.tag);
        e
    }

    /// Serializes one IFD (entries, next pointer, out-of-line values) placed
    /// at `at`. Returns the bytes.
    fn encode_ifd(&self, entries: &[Entry], at: u64, next: u64) -> Vec<u8> {
        let (count_len, entry_len, ptr_len) = if self.big_tiff {
            (8, 20, 8)
        } else {
            (2, 12, 4)
        };
        let table_len = count_len + entries.len() * entry_len + ptr_len;
        let mut table = Vec::with_capacity(table_len);
        let mut extra: Vec<u8> = Vec::new();
        push_uint(&mut table, entries.len() as u64, count_len);
        for e in entries {
            table.extend_from_slice(&e.tag.to_le_bytes());
            table.extend_from_slice(&e.ty.to_le_bytes());
            push_uint(&mut table, e.count, ptr_len);
            if e.data.len() <= ptr_len {
                table.extend_from_slice(&e.data);
                table.resize(table.len() + ptr_len - e.data.len(), 0);
            } else {
                push_uint(
                    &mut table,
                    at + table_len as u64 + extra.len() as u64,
                    ptr_len,
                );
                extra.extend_from_slice(&e.data);
                if extra.len() % 2 == 1 {
                    extra.push(0);
                }
            }
        }
        push_uint(&mut table, next, ptr_len);
        table.extend_from_slice(&extra);
        table
    }
}

fn layout(core: &CoreMetadata, tile: Option<(u32, u32)>) -> Layout {
    let (tiled, cw, ch) = match tile {
        Some((w, h)) => (true, w, h),
        None => {
            let row = core.region_bytes(core.size_x, 1).max(1);
            let rows = (STRIP_BYTES / row).clamp(1, core.size_y.max(1) as usize) as u32;
            (false, core.size_x, rows)
        }
    };
    let mut chunks = Vec::new();
    for y in (0..core.size_y).step_by(ch as usize) {
        for x in (0..core.size_x).step_by(cw as usize) {
            chunks.push(Region::new(
                x,
                y,
                cw.min(core.size_x - x),
                ch.min(core.size_y - y),
            ));
        }
    }
    Layout {
        tiled,
        chunk_w: cw,
        chunk_h: ch,
        chunks,
    }
}

fn push_uint(out: &mut Vec<u8>, v: u64, len: usize) {
    out.extend_from_slice(&v.to_le_bytes()[..len]);
}

/// Converts a physical pixel size into a TIFF resolution in pixels per cm.
fn pixels_per_cm(size: Option<&Quantity>) -> Option<(u32, u32)> {
    let q = size?;
    let um = q.value
        * match q.unit.as_str() {
            "µm" | "um" | "micron" => 1.0,
            "nm" => 1e-3,
            "mm" => 1e3,
            "cm" => 1e4,
            "m" => 1e6,
            _ => return None,
        };
    if !(um.is_finite() && um > 0.0) {
        return None;
    }
    let per_cm = 1e4 / um;
    [1_000_000u32, 10_000, 100, 1]
        .into_iter()
        .find(|&den| per_cm * den as f64 <= u32::MAX as f64)
        .map(|den| ((per_cm * den as f64).round().max(1.0) as u32, den))
}

/// The hyperstack description ImageJ (and Bio-Formats' `TiffWriter`) use.
fn imagej_description(cores: &[CoreMetadata], meta: &OmeMetadata) -> String {
    let core = &cores[0];
    let mut d = format!("ImageJ=\nimages={}\n", core.image_count);
    if cores.len() == 1 && core.dimension_order == DimensionOrder::XYCZT {
        let c = core.effective_size_c();
        d += &format!(
            "channels={c}\nslices={}\nframes={}\n",
            core.size_z, core.size_t
        );
        if core.image_count > 1 {
            d += "hyperstack=true\n";
        }
        if let Some(z) = meta.images.first().and_then(|i| i.physical_size_z.as_ref()) {
            d += &format!("spacing={}\n", z.value);
        }
    }
    d
}

struct Entry {
    tag: u16,
    ty: u16,
    count: u64,
    data: Vec<u8>,
}

impl Entry {
    fn shorts(tag: u16, v: &[u16]) -> Self {
        Entry {
            tag,
            ty: field::SHORT,
            count: v.len() as u64,
            data: v.iter().flat_map(|x| x.to_le_bytes()).collect(),
        }
    }

    fn longs(tag: u16, v: &[u32]) -> Self {
        Entry {
            tag,
            ty: field::LONG,
            count: v.len() as u64,
            data: v.iter().flat_map(|x| x.to_le_bytes()).collect(),
        }
    }

    fn offsets(big: bool, v: &[u64]) -> Self {
        if big {
            Entry {
                tag: 0,
                ty: field::LONG8,
                count: v.len() as u64,
                data: v.iter().flat_map(|x| x.to_le_bytes()).collect(),
            }
        } else {
            Entry {
                tag: 0,
                ty: field::LONG,
                count: v.len() as u64,
                data: v.iter().flat_map(|&x| (x as u32).to_le_bytes()).collect(),
            }
        }
    }

    fn ascii(tag: u16, s: &str) -> Self {
        let mut data = s.as_bytes().to_vec();
        data.push(0);
        Entry {
            tag,
            ty: field::ASCII,
            count: data.len() as u64,
            data,
        }
    }

    fn rational(tag: u16, (n, d): (u32, u32)) -> Self {
        let mut data = n.to_le_bytes().to_vec();
        data.extend_from_slice(&d.to_le_bytes());
        Entry {
            tag,
            ty: field::RATIONAL,
            count: 1,
            data,
        }
    }
}

impl FormatWriter for TiffWriter {
    fn plane_chunks(&self, series: usize) -> Vec<Region> {
        self.layouts[series].chunks.clone()
    }

    fn encode_chunk(&self, series: usize, region: Region, data: Vec<u8>) -> Result<Vec<u8>> {
        let core = &self.cores[series];
        let layout = &self.layouts[series];
        let px = core.bytes_per_pixel();
        if data.len() != core.region_bytes(region.w, region.h) {
            return Err(Error::InvalidArgument(format!(
                "chunk {region:?} has {} bytes, expected {}",
                data.len(),
                core.region_bytes(region.w, region.h)
            )));
        }
        // Edge tiles are padded to the full tile size.
        let (mut data, row_len) =
            if layout.tiled && (region.w < layout.chunk_w || region.h < layout.chunk_h) {
                let full_row = layout.chunk_w as usize * px;
                let mut padded = vec![0; full_row * layout.chunk_h as usize];
                let row = region.w as usize * px;
                for (r, src) in data.chunks_exact(row).enumerate() {
                    padded[r * full_row..r * full_row + row].copy_from_slice(src);
                }
                (padded, full_row)
            } else {
                (data, region.w as usize * px)
            };
        if self.uses_predictor(series) {
            predictor::apply(
                &mut data,
                row_len,
                core.rgb_channel_count as usize,
                core.pixel_type.bytes(),
            );
        }
        match self.options.compression {
            Compression::None => Ok(data),
            c => c.encode(&data),
        }
    }

    fn write_chunk(
        &mut self,
        series: usize,
        plane: u32,
        chunk: usize,
        encoded: &[u8],
    ) -> Result<()> {
        let expected_chunks = self
            .layouts
            .get(series)
            .map(|l| l.chunks.len())
            .unwrap_or(0);
        if chunk >= expected_chunks {
            return Err(Error::InvalidArgument(format!(
                "chunk {chunk} out of range for series {series}"
            )));
        }
        let starts_plane = match self.ifds.last() {
            Some(rec)
                if rec.series == series
                    && rec.plane == plane
                    && rec.offsets.len() < expected_chunks =>
            {
                if rec.offsets.len() != chunk {
                    return Err(Error::InvalidArgument(format!(
                        "expected chunk {}, got {chunk}",
                        rec.offsets.len()
                    )));
                }
                false
            }
            Some(rec) if rec.offsets.len() < self.layouts[rec.series].chunks.len() => {
                return Err(Error::InvalidArgument(format!(
                    "series {} plane {} is incomplete; finish it before starting another",
                    rec.series, rec.plane
                )));
            }
            _ => true,
        };
        if starts_plane {
            if chunk != 0 {
                return Err(Error::InvalidArgument(format!(
                    "plane must start at chunk 0, got {chunk}"
                )));
            }
            self.ifds.push(IfdRecord {
                series,
                plane,
                offsets: Vec::with_capacity(expected_chunks),
                counts: Vec::new(),
            });
        }
        self.check_offset(self.pos + encoded.len() as u64)?;
        self.out.write_all(encoded)?;
        let rec = self.ifds.last_mut().unwrap();
        rec.offsets.push(self.pos);
        rec.counts.push(encoded.len() as u64);
        self.pos += encoded.len() as u64;
        Ok(())
    }

    fn finish(mut self: Box<Self>) -> Result<()> {
        if self.ifds.is_empty() {
            return Err(Error::InvalidArgument("no planes were written".into()));
        }
        if let Some(rec) = self.ifds.last()
            && rec.offsets.len() < self.layouts[rec.series].chunks.len()
        {
            return Err(format_err!(
                "series {} plane {} is incomplete",
                rec.series,
                rec.plane
            ));
        }
        if self.pos % 2 == 1 {
            self.out.write_all(&[0])?;
            self.pos += 1;
        }
        let first_ifd = self.pos;
        let description = self.description();
        for i in 0..self.ifds.len() {
            let entries = self.ifd_entries(i, (i == 0).then_some(description.as_str()));
            // Encode once to learn the size, then again with the real next pointer.
            let size = self.encode_ifd(&entries, self.pos, 0).len() as u64;
            let size = size + size % 2;
            let next = if i + 1 < self.ifds.len() {
                self.pos + size
            } else {
                0
            };
            let mut bytes = self.encode_ifd(&entries, self.pos, next);
            bytes.resize(size as usize, 0);
            self.check_offset(self.pos + size)?;
            self.out.write_all(&bytes)?;
            self.pos += size;
        }
        let mut file = self.out.into_inner().map_err(|e| e.into_error())?;
        file.seek(SeekFrom::Start(if self.big_tiff { 8 } else { 4 }))?;
        if self.big_tiff {
            file.write_all(&first_ifd.to_le_bytes())?;
        } else {
            file.write_all(&(first_ifd as u32).to_le_bytes())?;
        }
        file.sync_all()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolution_round_trip() {
        let (n, d) = pixels_per_cm(Some(&Quantity::micrometers(0.575))).unwrap();
        let um = 1e4 / (n as f64 / d as f64);
        assert!((um - 0.575).abs() < 1e-6);
    }

    #[test]
    fn strip_layout_covers_plane() {
        let core = CoreMetadata::new(1000, 3001, crate::pixel::PixelType::Uint16);
        let l = layout(&core, None);
        assert_eq!(
            l.chunks
                .iter()
                .map(|r| r.h as u64 * r.w as u64)
                .sum::<u64>(),
            1000 * 3001
        );
        let t = layout(&core, Some((256, 256)));
        assert_eq!(t.chunks.len(), 4 * 12);
    }
}
