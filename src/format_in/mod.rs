pub mod lof_reader;
pub mod tiff_reader;

use std::io;

use crate::common::{ByteOrder, Loc, Metadata, PixelSlice};

pub trait FormatReader {
    // ----------------- Required -------------------

    fn metadata(&self) -> &Metadata;

    // Read rectangular portion of image data at given location
    // returns bytes, image metadata should be used to decode bytes
    fn open_bytes(&mut self, loc: Loc, df: u64) -> io::Result<Vec<u8>>;

    // ----------------- Derived -------------------

    // Read rectangular portion of image data at given location
    // returns PixelSlice
    fn open_pixels(&mut self, loc: Loc, df: u64) -> io::Result<PixelSlice> {
        let bytes = self.open_bytes(loc, df)?;
        let md = self.metadata();
        let byte_order = md.byte_order().clone();
        let bpp = md
            .bits_per_pixel(loc.s as usize)
            .ok_or(io::Error::other("Error reading bpp"))?
            .get(loc.c as usize)
            .ok_or(io::Error::other("Error reading bpp"))?
            .clone();

        PixelSlice::interpret_bytes(bpp, byte_order, &bytes)
    }
}
