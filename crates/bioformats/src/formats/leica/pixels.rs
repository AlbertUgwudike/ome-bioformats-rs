//! Pixel access shared by the LOF and LIF readers: both store an image as one
//! contiguous memory block of planes, with mosaic tiles as an outer dimension.
//! Port of `openBytes` / `seekStartOfPlane` from `LOFReader` and `LIFReader`.

use super::lms::LmsImage;
use crate::error::Result;
use crate::io::RandomAccessFile;
use crate::pixel::bgr_to_rgb;
use crate::reader::Region;

/// Where an image's pixels live in the file.
#[derive(Debug, Clone, Copy)]
pub struct MemoryBlock {
    pub offset: u64,
    pub len: u64,
}

#[derive(Debug)]
pub struct ImageData {
    pub image: LmsImage,
    /// `None` if the file holds no pixels for this image.
    block: Option<MemoryBlock>,
    /// Bytes from one row to the next (includes any row padding).
    row_stride: u64,
}

impl ImageData {
    pub fn new(image: LmsImage, block: Option<MemoryBlock>) -> Self {
        let core = &image.core;
        let row_bytes = core.region_bytes(core.size_x, 1) as u64;
        // Leica pads rows when the width is not a multiple of 4; the row
        // axis' BytesInc gives the padded stride. Bio-Formats infers padding
        // from offsets instead (the file length, for LOF, which includes the
        // XML trailer, so it misses it there). Only trust the stride if the
        // block is big enough to hold the padded planes.
        let planes = core.image_count as u64 * image.tile_count as u64;
        let padded_fits =
            block.is_some_and(|b| image.row_inc * core.size_y as u64 * planes <= b.len);
        let row_stride =
            if image.row_inc > row_bytes && !core.size_x.is_multiple_of(4) && padded_fits {
                image.row_inc
            } else {
                row_bytes
            };
        ImageData {
            image,
            block,
            row_stride,
        }
    }

    fn plane_stride(&self) -> u64 {
        self.row_stride * self.image.core.size_y as u64
    }

    /// Port of `seekStartOfPlane`: the offset of plane `no` of `tile`,
    /// relative to the start of the memory block.
    fn plane_offset(&self, tile: u32, no: u32) -> u64 {
        let plane = self.plane_stride();
        if self.image.tile_count > 1 {
            // LAS AF treats tiles as just another dimension; Bio-Formats
            // exposes them as series.
            let per_tile = self.image.tile_bytes_inc;
            let frames_per_tile = (per_tile / plane).max(1);
            let outside = no as u64 / frames_per_tile;
            let inside = no as u64 % frames_per_tile;
            outside * per_tile * self.image.tile_count as u64
                + tile as u64 * per_tile
                + inside * plane
        } else {
            no as u64 * plane
        }
    }

    /// Reads `region` of plane `no` of `tile` into `buf` (already validated
    /// against the core metadata).
    pub fn read(
        &self,
        file: &RandomAccessFile,
        tile: u32,
        no: u32,
        region: Region,
        buf: &mut [u8],
    ) -> Result<()> {
        let core = &self.image.core;
        let Some(block) = self.block else {
            buf.fill(0);
            return Ok(());
        };
        let start = block.offset + self.plane_offset(tile, no);
        let end = block.offset + block.len;
        if start + self.plane_stride() > end.min(file.len()) {
            // Truncated file: imitate LAS AF and return blank planes.
            buf.fill(0);
            return Ok(());
        }
        let bpp = core.bytes_per_pixel() as u64;
        let row = region.w as usize * bpp as usize;
        let first = start + region.y as u64 * self.row_stride + region.x as u64 * bpp;
        if region.w == core.size_x && self.row_stride == row as u64 {
            // Whole rows with no padding: one contiguous read.
            file.read_exact_at(buf, first)?;
        } else {
            for (r, dst) in buf.chunks_exact_mut(row).enumerate() {
                file.read_exact_at(dst, first + r as u64 * self.row_stride)?;
            }
        }
        if self.image.bgr {
            bgr_to_rgb(buf, core.interleaved, core.pixel_type.bytes(), 3);
        }
        Ok(())
    }
}
