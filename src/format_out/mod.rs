use std::io::{self, Error};

use crate::common::{Loc, PixelSlice};

pub mod tiff_writer;

pub trait FormatWriter {
    // ----------------- Required -------------------

    // Write rectangular portion of image data to given location
    fn write_bytes(&mut self, bytes: Vec<u8>, origin: Loc, h: u64, w: u64) -> io::Result<()>;

    // ----------------- Derived -------------------

    // Write rectangular portion of image data to given location
    fn write_pixels(&mut self, pixels: PixelSlice, origin: Loc, h: u64, w: u64) -> io::Result<()> {
        if pixels.len() != (h * w) as usize {
            return Err(Error::other("Invalid write dimensions for pixel slice"));
        }

        let mut bytes = vec![0u8; pixels.bytes_len()];
        pixels.to_be_bytes(&mut bytes);

        self.write_bytes(bytes, origin, h, w)?;

        Ok(())
    }
}
