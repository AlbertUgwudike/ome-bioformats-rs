use std::io;

use crate::common::{Loc, Metadata, PixelSlice};

pub mod tiff_writer;

pub trait FormatWriter {
    // ----------------- Required -------------------

    // Write rectangular portion of image data to given location
    fn write_bytes(&mut self, bytes: Vec<u8>, origin: Loc, h: u64, w: u64) -> io::Result<()>;

    // ----------------- Derived -------------------

    // Write rectangular portion of image data to given location
    fn write_pixels(&mut self, pixels: PixelSlice, origin: Loc, h: u64, w: u64) -> io::Result<()> {
        todo!()
    }
}
