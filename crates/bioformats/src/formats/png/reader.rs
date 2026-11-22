//! PNG / APNG reader. Port of `loci.formats.in.APNGReader`.
//!
//! Samples come out as PNG stores them: 16-bit values big-endian, palette
//! images as their indices (with the palette as the series' LUT), and depths
//! below 8 bits unpacked to one value per byte without rescaling. Animation
//! frames become timepoints, composited as the APNG specification says (each
//! frame drawn onto the previous canvas with its blend operation, then
//! disposed). Bio-Formats instead pastes every frame onto frame 0, which
//! leaves stale pixels once sub-frames move.

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use png::{BitDepth, BlendOp, ColorType, DisposeOp, Transformations, Unit};

use crate::core::CoreMetadata;
use crate::dims::DimensionOrder;
use crate::error::{Error, Result, format_err};
use crate::meta::{ImageMetadata, OmeMetadata, Quantity};
use crate::pixel::PixelType;
use crate::reader::{FormatReader, ReaderFormat, Region, check_plane_args};

const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

pub const FORMAT: ReaderFormat = ReaderFormat {
    name: "Animated PNG",
    suffixes: &["png"],
    is_this_type: |_, file| file.read_prefix(0, 8).is_ok_and(|h| h == SIGNATURE),
    open: |path| Ok(Box::new(PngReader::open(path)?)),
};

type Decoder = png::Reader<BufReader<File>>;

fn png_err(e: png::DecodingError) -> Error {
    match e {
        png::DecodingError::IoError(e) => Error::Io(e),
        other => Error::Format(format!("PNG: {other}")),
    }
}

fn decoder(path: &Path) -> Result<Decoder> {
    let mut d = png::Decoder::new_with_limits(
        BufReader::new(File::open(path)?),
        png::Limits { bytes: usize::MAX },
    );
    d.set_transformations(Transformations::IDENTITY);
    d.read_info().map_err(png_err)
}

/// Decoding state: the decoder advances through frames in order, so it is
/// kept between calls and only restarted when an earlier frame is wanted.
struct State {
    reader: Decoder,
    /// Index of the frame the decoder will produce next.
    next: u32,
    /// The canvas the next frame is drawn onto (after the previous frame's
    /// dispose operation).
    canvas: Vec<u8>,
    /// The most recently composited frames: frame 0 is kept because
    /// conversion often returns to it.
    first: Option<Arc<Vec<u8>>>,
    last: Option<(u32, Arc<Vec<u8>>)>,
}

impl State {
    fn new(reader: Decoder, plane_bytes: usize) -> Self {
        State {
            reader,
            next: 0,
            canvas: vec![0; plane_bytes],
            first: None,
            last: None,
        }
    }
}

pub struct PngReader {
    path: PathBuf,
    cores: Vec<CoreMetadata>,
    meta: OmeMetadata,
    bit_depth: u8,
    samples: usize,
    state: Mutex<State>,
}

impl PngReader {
    pub fn open(path: &Path) -> Result<Self> {
        let reader = decoder(path)?;
        let info = reader.info();
        let samples = match info.color_type {
            ColorType::Grayscale | ColorType::Indexed => 1,
            ColorType::GrayscaleAlpha => 2,
            ColorType::Rgb => 3,
            ColorType::Rgba => 4,
        };
        let bit_depth = info.bit_depth as u8;
        let pixel_type = if info.bit_depth == BitDepth::Sixteen {
            PixelType::Uint16
        } else {
            PixelType::Uint8
        };
        let mut core = CoreMetadata::new(info.width, info.height, pixel_type);
        core.size_c = samples as u32;
        core.rgb_channel_count = samples as u32;
        core.size_t = info.animation_control.map_or(1, |a| a.num_frames).max(1);
        core.dimension_order = DimensionOrder::XYCTZ;
        core.interleaved = samples > 1;
        core.little_endian = false;
        core.bits_per_pixel = bit_depth as u32;
        if info.color_type == ColorType::Indexed {
            core.indexed = true;
            core.lut = info
                .palette
                .as_ref()
                .map(|p| p.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect());
        }
        core.update_image_count();

        let mut image = ImageMetadata {
            name: path.file_name().map(|n| n.to_string_lossy().into_owned()),
            ..Default::default()
        };
        if let Some(dims) = info.pixel_dims
            && dims.unit == Unit::Meter
            && dims.xppu > 0
            && dims.yppu > 0
        {
            image.physical_size_x = Some(Quantity::micrometers(1e6 / dims.xppu as f64));
            image.physical_size_y = Some(Quantity::micrometers(1e6 / dims.yppu as f64));
        }
        let mut meta = OmeMetadata {
            images: vec![image],
            ..Default::default()
        };
        for t in &info.uncompressed_latin1_text {
            meta.global_metadata
                .insert(t.keyword.clone(), t.text.clone());
        }
        for t in &info.compressed_latin1_text {
            if let Ok(text) = t.get_text() {
                meta.global_metadata.insert(t.keyword.clone(), text);
            }
        }
        for t in &info.utf8_text {
            if let Ok(text) = t.get_text() {
                meta.global_metadata.insert(t.keyword.clone(), text);
            }
        }
        if let Some(a) = info.animation_control {
            meta.global_metadata
                .insert("Loop count".into(), a.num_plays.to_string());
        }
        let plane_bytes = core.plane_bytes();
        let cores = vec![core];
        meta.conform_to(&cores);
        Ok(PngReader {
            path: path.to_owned(),
            cores,
            meta,
            bit_depth,
            samples,
            state: Mutex::new(State::new(reader, plane_bytes)),
        })
    }

    /// Decodes one frame from the decoder: `(pixels, w, h, frame control)`.
    fn decode_raw(
        &self,
        state: &mut State,
    ) -> Result<(Vec<u8>, usize, usize, Option<png::FrameControl>)> {
        let size = state
            .reader
            .output_buffer_size()
            .ok_or_else(|| format_err!("PNG frame too large"))?;
        let mut raw = vec![0; size];
        let out = state.reader.next_frame(&mut raw).map_err(png_err)?;
        let (fw, fh) = (out.width as usize, out.height as usize);
        Ok((
            self.unpack(&raw, fw, fh, out.line_size),
            fw,
            fh,
            state.reader.info().frame_control,
        ))
    }

    /// Decodes the next animation frame and composites it onto the canvas.
    fn decode_next(&self, state: &mut State) -> Result<Arc<Vec<u8>>> {
        let (mut frame, mut fw, mut fh, mut fc) = self.decode_raw(state)?;
        let animated =
            self.cores[0].image_count > 1 || state.reader.info().animation_control.is_some();
        if state.next == 0 && animated && fc.is_none() {
            // The default image is not part of the animation (no fcTL before
            // IDAT): skip it, the first frame is the first fdAT.
            (frame, fw, fh, fc) = self.decode_raw(state)?;
        }
        let (x, y, blend, dispose) = match fc {
            Some(f) => (
                f.x_offset as usize,
                f.y_offset as usize,
                f.blend_op,
                f.dispose_op,
            ),
            None => (0, 0, BlendOp::Source, DisposeOp::None),
        };
        // "Previous" on the first frame means "background" (APNG spec).
        let dispose = if state.next == 0 && dispose == DisposeOp::Previous {
            DisposeOp::Background
        } else {
            dispose
        };
        let saved = (dispose == DisposeOp::Previous).then(|| state.canvas.clone());
        self.draw(&mut state.canvas, &frame, x, y, fw, fh, blend);
        let output = Arc::new(state.canvas.clone());
        match dispose {
            DisposeOp::None => {}
            DisposeOp::Background => self.clear(&mut state.canvas, x, y, fw, fh),
            DisposeOp::Previous => state.canvas = saved.unwrap(),
        }
        state.next += 1;
        Ok(output)
    }

    /// The canvas rows and columns a `fw` x `fh` frame at (`x`, `y`) covers.
    fn clip(&self, x: usize, y: usize, fw: usize, fh: usize) -> (usize, usize) {
        let core = &self.cores[0];
        (
            fw.min((core.size_x as usize).saturating_sub(x)),
            fh.min((core.size_y as usize).saturating_sub(y)),
        )
    }

    fn clear(&self, canvas: &mut [u8], x: usize, y: usize, fw: usize, fh: usize) {
        let px = self.cores[0].bytes_per_pixel();
        let w = self.cores[0].size_x as usize;
        let (cols, rows) = self.clip(x, y, fw, fh);
        for row in 0..rows {
            let dst = ((y + row) * w + x) * px;
            canvas[dst..dst + cols * px].fill(0);
        }
    }

    /// Draws a frame onto the canvas with the APNG blend operation.
    #[allow(clippy::too_many_arguments)]
    fn draw(
        &self,
        canvas: &mut [u8],
        frame: &[u8],
        x: usize,
        y: usize,
        fw: usize,
        fh: usize,
        blend: BlendOp,
    ) {
        let core = &self.cores[0];
        let px = core.bytes_per_pixel();
        let w = core.size_x as usize;
        let (cols, rows) = self.clip(x, y, fw, fh);
        // Alpha blending needs an alpha channel (gray+alpha or RGBA).
        let alpha = matches!(self.samples, 2 | 4) && blend == BlendOp::Over;
        for row in 0..rows {
            let dst = ((y + row) * w + x) * px;
            let src = row * fw * px;
            if !alpha {
                canvas[dst..dst + cols * px].copy_from_slice(&frame[src..src + cols * px]);
                continue;
            }
            for c in 0..cols {
                let d = &mut canvas[dst + c * px..dst + (c + 1) * px];
                let s = &frame[src + c * px..src + (c + 1) * px];
                blend_over(d, s, self.samples, self.bit_depth);
            }
        }
    }

    /// Strips row padding and expands depths below 8 bits to one byte per sample.
    fn unpack(&self, raw: &[u8], w: usize, h: usize, line: usize) -> Vec<u8> {
        let per_row = w * self.samples;
        if self.bit_depth >= 8 {
            let row = per_row * (self.bit_depth as usize / 8);
            if row == line {
                return raw[..row * h].to_vec();
            }
            return raw
                .chunks(line)
                .take(h)
                .flat_map(|r| &r[..row])
                .copied()
                .collect();
        }
        let bits = self.bit_depth as usize;
        let mask = (1u8 << bits) - 1;
        let mut out = Vec::with_capacity(per_row * h);
        for r in raw.chunks(line).take(h) {
            for i in 0..per_row {
                let bit = i * bits;
                out.push((r[bit / 8] >> (8 - bits - bit % 8)) & mask);
            }
        }
        out
    }

    fn frame(&self, no: u32) -> Result<Arc<Vec<u8>>> {
        let mut state = self.state.lock().unwrap();
        if let Some((n, data)) = &state.last
            && *n == no
        {
            return Ok(data.clone());
        }
        if no == 0
            && let Some(first) = &state.first
        {
            return Ok(first.clone());
        }
        if no < state.next {
            // The decoder only moves forwards; start again from the top.
            let first = state.first.take();
            *state = State::new(decoder(&self.path)?, self.cores[0].plane_bytes());
            state.first = first;
        }
        let mut frame = None;
        while state.next <= no {
            let f = self.decode_next(&mut state)?;
            if state.next == 1 {
                state.first = Some(f.clone());
            }
            frame = Some(f);
        }
        let frame = frame.expect("loop runs at least once");
        state.last = Some((no, frame.clone()));
        Ok(frame)
    }
}

/// APNG `APNG_BLEND_OP_OVER` for one pixel with straight (non-premultiplied)
/// alpha in the last sample. `bits` is 8 or 16 (big-endian).
fn blend_over(dst: &mut [u8], src: &[u8], samples: usize, bits: u8) {
    let bytes = if bits == 16 { 2 } else { 1 };
    let max = if bits == 16 { 65535.0 } else { 255.0 };
    let get = |p: &[u8], i: usize| -> f64 {
        if bytes == 2 {
            u16::from_be_bytes([p[2 * i], p[2 * i + 1]]) as f64
        } else {
            p[i] as f64
        }
    };
    let a_s = get(src, samples - 1);
    if a_s >= max {
        dst.copy_from_slice(src);
        return;
    }
    if a_s == 0.0 {
        return;
    }
    let a_d = get(dst, samples - 1);
    let (fs, fd) = (a_s / max, a_d / max * (1.0 - a_s / max));
    let a_out = fs + fd;
    let mut out = [0f64; 4];
    for (c, o) in out.iter_mut().enumerate().take(samples - 1) {
        *o = (get(src, c) * fs + get(dst, c) * fd) / a_out;
    }
    out[samples - 1] = a_out * max;
    for (c, &v) in out.iter().take(samples).enumerate() {
        let v = v.round().clamp(0.0, max) as u16;
        if bytes == 2 {
            dst[2 * c..2 * c + 2].copy_from_slice(&v.to_be_bytes());
        } else {
            dst[c] = v as u8;
        }
    }
}

impl FormatReader for PngReader {
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
        let frame = self.frame(plane)?;
        let px = core.bytes_per_pixel();
        let src_row = core.size_x as usize * px;
        let row = region.w as usize * px;
        for (r, dst) in buf.chunks_exact_mut(row).enumerate() {
            let src = (region.y as usize + r) * src_row + region.x as usize * px;
            dst.copy_from_slice(&frame[src..src + row]);
        }
        Ok(())
    }

    fn optimal_tile_size(&self, series: usize) -> (u32, u32) {
        // Whole frames are decoded at once; hand them out in one piece.
        let core = &self.cores[series];
        (core.size_x, core.size_y)
    }

    fn used_files(&self) -> Vec<PathBuf> {
        vec![self.path.clone()]
    }
}
