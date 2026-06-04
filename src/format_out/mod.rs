use std::{
    io::{self, Error},
    path::Path,
};

use crate::{
    common::{Loc, Metadata, PixelSlice},
    format_out::tiff_writer::TiffWriter,
};

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

pub enum Writer {
    Tiff(TiffWriter),
}

impl Writer {
    pub fn new(output_file: &String, metadata: &Metadata) -> io::Result<Self> {
        let ext_err = io::Error::other("No Extension");
        let uni_err = io::Error::other("Invalid Unicode");

        let ext = Path::new(output_file)
            .extension()
            .ok_or(ext_err)?
            .to_str()
            .ok_or(uni_err)?;

        match ext {
            "tiff" | "tif" => {
                let writer = TiffWriter::new(output_file.clone())
                    .create()
                    .set_metadata(metadata.clone())
                    .big_tiff(true)
                    .build()?;

                Ok(Writer::Tiff(writer))
            }
            other => Err(io::Error::other(format!(
                "Unsupported Reader Format: {}",
                other
            ))),
        }
    }
}

impl FormatWriter for Writer {
    fn write_bytes(&mut self, bytes: Vec<u8>, loc: Loc) -> io::Result<()> {
        match self {
            Writer::Tiff(writer) => writer.write_bytes(bytes, loc),
        }
    }
}
