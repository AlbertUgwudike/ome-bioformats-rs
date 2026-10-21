//! Format conversion. The engine behind `bfconvert`; port of the core loop
//! of `loci.formats.tools.ImageConverter`.
//!
//! Each plane is processed in batches of writer chunks: a batch is read,
//! normalised and compressed in parallel, then appended to the output in
//! order. Memory use is bounded by the batch size, not the plane size.

use std::path::Path;

use rayon::prelude::*;

use crate::core::CoreMetadata;
use crate::error::{Error, Result};
use crate::formats;
use crate::meta::OmeMetadata;
use crate::pixel::{interleave, swap_endianness};
use crate::reader::FormatReader;
use crate::writer::{FormatWriter, WriterOptions};

/// Target amount of raw pixel data in flight per batch.
const BATCH_BYTES: usize = 128 << 20;

#[derive(Debug, Clone, Default)]
pub struct ConvertOptions {
    /// Series to convert (input indices); `None` converts all.
    pub series: Option<Vec<usize>>,
    pub writer: WriterOptions,
}

/// Reported after each plane is written.
#[derive(Debug, Clone, Copy)]
pub struct Progress {
    pub series: usize,
    pub plane: u32,
    pub planes_done: u64,
    pub planes_total: u64,
}

#[derive(Debug, Clone)]
pub struct Summary {
    pub reader_format: &'static str,
    pub series: usize,
    pub planes: u64,
    pub bytes_read: u64,
}

/// The core metadata a writer receives for a reader's series: same shape,
/// but interleaved and little-endian, which is what the chunk pipeline produces.
pub fn output_core(core: &CoreMetadata) -> CoreMetadata {
    let mut out = core.clone();
    out.interleaved = core.is_rgb();
    out.little_endian = true;
    out
}

/// Converts `input` to `output`, choosing formats from the files.
pub fn convert(
    input: &Path,
    output: &Path,
    options: &ConvertOptions,
    progress: &(dyn Fn(Progress) + Sync),
) -> Result<Summary> {
    let reader = formats::open(input)?;
    let series = selected_series(reader.as_ref(), options)?;
    let cores: Vec<CoreMetadata> = series
        .iter()
        .map(|&s| output_core(&reader.series()[s]))
        .collect();
    let meta = OmeMetadata {
        images: series
            .iter()
            .map(|&s| reader.metadata().images.get(s).cloned().unwrap_or_default())
            .collect(),
        global_metadata: reader.metadata().global_metadata.clone(),
    };
    let mut writer = formats::create_writer(output, &cores, &meta, &options.writer)?;
    let summary = copy_pixels(reader.as_ref(), writer.as_mut(), &series, progress)?;
    writer.finish()?;
    Ok(summary)
}

fn selected_series(reader: &dyn FormatReader, options: &ConvertOptions) -> Result<Vec<usize>> {
    let count = reader.series().len();
    match &options.series {
        None => Ok((0..count).collect()),
        Some(list) => {
            if let Some(bad) = list.iter().find(|&&s| s >= count) {
                return Err(Error::InvalidArgument(format!(
                    "series {bad} out of range (file has {count})"
                )));
            }
            Ok(list.clone())
        }
    }
}

/// Streams every plane of `series` from `reader` into `writer`. Writer
/// series `i` receives reader series `series[i]`.
pub fn copy_pixels(
    reader: &dyn FormatReader,
    writer: &mut dyn FormatWriter,
    series: &[usize],
    progress: &(dyn Fn(Progress) + Sync),
) -> Result<Summary> {
    let planes_total: u64 = series
        .iter()
        .map(|&s| reader.series()[s].image_count as u64)
        .sum();
    let mut planes_done = 0;
    let mut bytes_read = 0;
    for (out_s, &in_s) in series.iter().enumerate() {
        let core = &reader.series()[in_s];
        let chunks = writer.plane_chunks(out_s);
        let chunk_bytes = chunks
            .first()
            .map_or(1, |r| core.region_bytes(r.w, r.h))
            .max(1);
        let batch = (BATCH_BYTES / chunk_bytes)
            .max(rayon::current_num_threads() * 2)
            .max(1);
        for plane in 0..core.image_count {
            for (b, group) in chunks.chunks(batch).enumerate() {
                let encoded: Vec<Vec<u8>> = group
                    .par_iter()
                    .map(|&region| {
                        let raw = read_normalized(reader, in_s, plane, region)?;
                        writer.encode_chunk(out_s, region, raw)
                    })
                    .collect::<Result<_>>()?;
                for (i, bytes) in encoded.iter().enumerate() {
                    writer.write_chunk(out_s, plane, b * batch + i, bytes)?;
                }
                bytes_read += group
                    .iter()
                    .map(|r| core.region_bytes(r.w, r.h) as u64)
                    .sum::<u64>();
            }
            planes_done += 1;
            progress(Progress {
                series: in_s,
                plane,
                planes_done,
                planes_total,
            });
        }
    }
    Ok(Summary {
        reader_format: reader.format_name(),
        series: series.len(),
        planes: planes_done,
        bytes_read,
    })
}

/// Reads a region and converts it to interleaved little-endian samples.
fn read_normalized(
    reader: &dyn FormatReader,
    series: usize,
    plane: u32,
    region: crate::reader::Region,
) -> Result<Vec<u8>> {
    let core = &reader.series()[series];
    let mut buf = vec![0; core.region_bytes(region.w, region.h)];
    reader.open_bytes(series, plane, region, &mut buf)?;
    let bytes = core.pixel_type.bytes();
    if !core.little_endian {
        swap_endianness(&mut buf, bytes);
    }
    if core.is_rgb() && !core.interleaved {
        let mut out = vec![0; buf.len()];
        interleave(&buf, &mut out, core.rgb_channel_count as usize, bytes);
        buf = out;
    }
    Ok(buf)
}
