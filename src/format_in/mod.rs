pub mod tiff_reader;

use std::io;

use crate::common::{ByteOrder, Loc, Metadata, PixelSlice};

pub trait FormatReader {
    // ----------------- Required -------------------

    fn metadata(&self) -> &Metadata;

    // Read rectangular portion of image data at given location
    // returns bytes, image metadata should be used to decode bytes
    fn open_bytes(&mut self, origin: Loc, h: u64, w: u64) -> io::Result<Vec<u8>>;

    // ----------------- Derived -------------------

    // Read rectangular portion of image data at given location
    // returns PixelSlice
    fn open_pixels(&mut self, origin: Loc, h: u64, w: u64) -> io::Result<PixelSlice> {
        let bytes = self.open_bytes(origin, h, w)?;
        let md = self.metadata();

        let bbp = md
            .bits_per_pixel(origin.s as usize)
            .ok_or(io::Error::other("Error reading bpp"))?
            .get(origin.c as usize)
            .ok_or(io::Error::other("Error reading bpp"))?;

        match bbp {
            8 => Ok(PixelSlice::U8(bytes)),
            16 => Ok(PixelSlice::U16(
                bytes
                    .chunks_exact(2)
                    .map(|a| match md.byte_order() {
                        ByteOrder::LE => u16::from_le_bytes([a[0], a[1]]),
                        ByteOrder::BE => u16::from_be_bytes([a[0], a[1]]),
                    })
                    .collect(),
            )),
            _ => Err(io::Error::other("Unsupported PixelSlice Format")),
        }
    }
}
