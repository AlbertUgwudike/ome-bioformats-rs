use std::io::{self};

use super::FormatReader;
use crate::common::{Loc, Metadata};
use crate::format::lof::LofDecoder;

pub struct LofReader {
    metadata: Metadata,
    decoder: LofDecoder,
}

impl LofReader {
    pub fn new(file: String) -> io::Result<Self> {
        let decoder = LofDecoder::new(file)?;
        let metadata = decoder.metadata()?;
        Ok(Self { metadata, decoder })
    }
}

impl FormatReader for LofReader {
    fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    fn open_bytes(&mut self, origin: Loc, h: u64, w: u64) -> io::Result<Vec<u8>> {
        let bits_per_pixel = self.metadata.bits_per_pixel(origin.s as usize).unwrap();
        let dims = self.metadata.dimensions(origin.s as usize).unwrap();
        let bytes_per_pixel = bits_per_pixel.iter().map(|&n| n as u64).sum::<u64>() / 8;

        let pixels_per_plane = dims.h * dims.w;

        let bytes_to_skip =
            bytes_per_pixel * (origin.s * pixels_per_plane + origin.y * dims.w + origin.x);

        let n_bytes_to_read = bytes_per_pixel * h * w;

        let mut bytes = vec![0u8; n_bytes_to_read as usize];
        self.decoder.read_pixel_bytes(bytes_to_skip, &mut bytes)?;

        let bytes = bytes
            .into_iter()
            .skip(origin.c as usize)
            .step_by(bytes_per_pixel as usize)
            .map(|a| a.to_owned())
            .collect();

        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fmt::Display,
        time::{SystemTime, UNIX_EPOCH},
    };

    use crate::format_in::PixelSlice;

    use super::*;
}
