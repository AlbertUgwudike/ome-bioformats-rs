use std::io::{self, Error};

use crate::common::{Loc, PixelSlice};

pub mod tiff_writer;

pub trait FormatWriter {
    // ----------------- Required -------------------

    // Write rectangular portion of image data to given location
    fn write_bytes(&mut self, bytes: Vec<u8>, loc: Loc) -> io::Result<()>;

    // ----------------- Derived -------------------

    // Write rectangular portion of image data to given location
    fn write_pixels(&mut self, pixels: PixelSlice, loc: Loc) -> io::Result<()> {
        if pixels.len() != (loc.h * loc.w) as usize {
            return Err(Error::other(format!(
                "Invalid write dimensions for pixel slice {} != {}, {}",
                pixels.len(),
                loc.h,
                loc.w
            )));
        }

        let mut bytes = vec![0u8; pixels.bytes_len()];
        pixels.to_be_bytes(&mut bytes);

        self.write_bytes(bytes, loc)?;

        Ok(())
    }
}
