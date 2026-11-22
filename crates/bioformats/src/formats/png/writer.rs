//! PNG / APNG writer. Port of `loci.formats.out.APNGWriter`.
//!
//! Every plane of the (single) series becomes an animation frame, as in
//! Bio-Formats. Single images stream rows into the encoder, so the plane is
//! never held in memory. Animated output buffers one frame at a time: the
//! `png` crate's streaming writer (0.18) produces corrupt chunks across APNG
//! frames (sequence numbers inside `IDAT`, a stray trailing `fdAT`).
//! PNG's single deflate stream means encoding is sequential either way.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

use png::{BitDepth, ColorType, PixelDimensions, Unit};

use crate::core::CoreMetadata;
use crate::error::{Error, Result, format_err, unsupported};
use crate::meta::OmeMetadata;
use crate::pixel::{PixelType, swap_endianness};
use crate::reader::Region;
use crate::writer::{FormatWriter, WriterOptions};

/// Target uncompressed size of a chunk handed to the encoder.
const CHUNK_BYTES: usize = 1 << 20;

/// A file shared between the encoder and the writer, so the writer can
/// flush and report errors after the encoder (which writes `IEND` when it
/// is dropped) is gone.
#[derive(Clone)]
struct SharedFile(Arc<Mutex<BufWriter<File>>>);

impl Write for SharedFile {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.lock().unwrap().flush()
    }
}

fn png_err(e: png::EncodingError) -> Error {
    match e {
        png::EncodingError::IoError(e) => Error::Io(e),
        other => Error::Format(format!("PNG: {other}")),
    }
}

enum Output {
    Stream(Box<png::StreamWriter<'static, SharedFile>>),
    Frames {
        writer: png::Writer<SharedFile>,
        frame: Vec<u8>,
    },
}

pub struct PngWriter {
    file: SharedFile,
    output: Mutex<Option<Output>>,
    core: CoreMetadata,
    /// Bits per sample in the file (1, 2, 4, 8 or 16).
    bits: usize,
    chunks: Vec<Region>,
    /// The (plane, chunk) the next `write_chunk` must be.
    next: (u32, usize),
}

impl PngWriter {
    pub fn create(
        path: &Path,
        cores: &[CoreMetadata],
        meta: &OmeMetadata,
        _options: &WriterOptions,
    ) -> Result<Self> {
        let [core] = cores else {
            return Err(Error::InvalidArgument(format!(
                "PNG holds a single series but {} were given; pick one with --series",
                cores.len()
            )));
        };
        let depth = match core.pixel_type {
            PixelType::Uint8 | PixelType::Int8 => BitDepth::Eight,
            PixelType::Uint16 | PixelType::Int16 => BitDepth::Sixteen,
            other => {
                return Err(unsupported!(
                    "{} pixels in PNG (8- and 16-bit only)",
                    other.ome_name()
                ));
            }
        };
        // Keep 1/2/4-bit gray and palette images (masks, small palettes) at
        // their depth rather than inflating them to 8 bits.
        let depth = match (depth, core.rgb_channel_count, core.bits_per_pixel) {
            (BitDepth::Eight, 1, 1) => BitDepth::One,
            (BitDepth::Eight, 1, 2) => BitDepth::Two,
            (BitDepth::Eight, 1, 4) => BitDepth::Four,
            (d, ..) => d,
        };
        let palette = core
            .lut
            .as_ref()
            .filter(|_| core.rgb_channel_count == 1 && depth != BitDepth::Sixteen);
        let color = match (core.rgb_channel_count, palette.is_some()) {
            (1, true) => ColorType::Indexed,
            (1, false) => ColorType::Grayscale,
            (2, _) => ColorType::GrayscaleAlpha,
            (3, _) => ColorType::Rgb,
            (4, _) => ColorType::Rgba,
            (n, _) => return Err(unsupported!("{n} samples per pixel in PNG")),
        };

        let file = SharedFile(Arc::new(Mutex::new(BufWriter::with_capacity(
            4 << 20,
            File::create(path)?,
        ))));
        let mut enc = png::Encoder::new(file.clone(), core.size_x, core.size_y);
        enc.set_color(color);
        enc.set_depth(depth);
        if let Some(lut) = palette {
            let mut p: Vec<u8> = lut.iter().take(256).flatten().copied().collect();
            p.resize(p.len().max(3), 0);
            enc.set_palette(p);
        }
        if core.image_count > 1 {
            enc.set_animated(core.image_count, 0).map_err(png_err)?;
            enc.set_frame_delay(1, 10).map_err(png_err)?;
        }
        let image = meta.images.first();
        let ppm = |q: Option<&crate::meta::Quantity>| {
            let q = q?;
            let um = match q.unit.as_str() {
                "µm" | "um" => q.value,
                "nm" => q.value / 1e3,
                "mm" => q.value * 1e3,
                "m" => q.value * 1e6,
                _ => return None,
            };
            (um > 0.0)
                .then(|| (1e6 / um).round() as u32)
                .filter(|&p| p > 0)
        };
        if let (Some(x), Some(y)) = (
            ppm(image.and_then(|i| i.physical_size_x.as_ref())),
            ppm(image.and_then(|i| i.physical_size_y.as_ref())),
        ) {
            enc.set_pixel_dims(Some(PixelDimensions {
                xppu: x,
                yppu: y,
                unit: Unit::Meter,
            }));
        }
        enc.add_text_chunk("Software".into(), crate::ome_xml::CREATOR.into())
            .map_err(png_err)?;
        let writer = enc.write_header().map_err(png_err)?;
        let output = if core.image_count > 1 {
            Output::Frames {
                writer,
                frame: Vec::with_capacity(core.plane_bytes()),
            }
        } else {
            Output::Stream(Box::new(
                writer
                    .into_stream_writer_with_size(CHUNK_BYTES)
                    .map_err(png_err)?,
            ))
        };

        let row = core.region_bytes(core.size_x, 1).max(1);
        let rows = (CHUNK_BYTES / row).clamp(1, core.size_y.max(1) as usize) as u32;
        let chunks = (0..core.size_y)
            .step_by(rows as usize)
            .map(|y| Region::new(0, y, core.size_x, rows.min(core.size_y - y)))
            .collect();
        Ok(PngWriter {
            file,
            output: Mutex::new(Some(output)),
            core: core.clone(),
            bits: depth as usize,
            chunks,
            next: (0, 0),
        })
    }
}

impl FormatWriter for PngWriter {
    fn plane_chunks(&self, _series: usize) -> Vec<Region> {
        self.chunks.clone()
    }

    fn encode_chunk(&self, _series: usize, region: Region, mut data: Vec<u8>) -> Result<Vec<u8>> {
        if data.len() != self.core.region_bytes(region.w, region.h) {
            return Err(Error::InvalidArgument(format!(
                "chunk {region:?} has {} bytes",
                data.len()
            )));
        }
        if self.bits < 8 {
            return pack_rows(&data, region.w as usize, self.bits);
        }
        // PNG stores 16-bit samples big-endian.
        swap_endianness(&mut data, self.core.pixel_type.bytes());
        Ok(data)
    }

    fn write_chunk(
        &mut self,
        series: usize,
        plane: u32,
        chunk: usize,
        encoded: &[u8],
    ) -> Result<()> {
        if series != 0 || (plane, chunk) != self.next {
            return Err(Error::InvalidArgument(format!(
                "PNG frames must be written in order: expected plane {} chunk {}, got series {series} plane {plane} chunk {chunk}",
                self.next.0, self.next.1
            )));
        }
        let last_chunk = chunk + 1 == self.chunks.len();
        let mut output = self.output.lock().unwrap();
        match output
            .as_mut()
            .ok_or_else(|| format_err!("PNG writer already finished"))?
        {
            Output::Stream(stream) => stream.write_all(encoded)?,
            Output::Frames { writer, frame } => {
                frame.extend_from_slice(encoded);
                if last_chunk {
                    writer.write_image_data(frame).map_err(png_err)?;
                    frame.clear();
                }
            }
        }
        self.next = if last_chunk {
            (plane + 1, 0)
        } else {
            (plane, chunk + 1)
        };
        Ok(())
    }

    fn finish(self: Box<Self>) -> Result<()> {
        if self.next != (self.core.image_count, 0) {
            return Err(format_err!(
                "PNG incomplete: {} of {} frames written",
                self.next.0,
                self.core.image_count
            ));
        }
        match self
            .output
            .lock()
            .unwrap()
            .take()
            .ok_or_else(|| format_err!("PNG writer already finished"))?
        {
            // Finishing drops the encoder, which writes IEND into the shared file.
            Output::Stream(stream) => stream.finish().map_err(png_err)?,
            Output::Frames { writer, .. } => writer.finish().map_err(png_err)?,
        }
        let mut file = self.file.0.lock().unwrap();
        file.flush()?;
        file.get_ref().sync_all()?;
        Ok(())
    }
}

/// Packs one-byte samples into `bits`-wide fields, MSB first, each row
/// padded to a whole byte.
fn pack_rows(data: &[u8], width: usize, bits: usize) -> Result<Vec<u8>> {
    let row_bytes = (width * bits).div_ceil(8);
    let mut out = vec![0u8; row_bytes * data.len() / width.max(1)];
    for (r, row) in data.chunks_exact(width).enumerate() {
        for (i, &v) in row.iter().enumerate() {
            if v as u16 >= (1 << bits) {
                return Err(format_err!("sample value {v} does not fit in {bits} bits"));
            }
            let bit = i * bits;
            out[r * row_bytes + bit / 8] |= v << (8 - bits - bit % 8);
        }
    }
    Ok(out)
}
