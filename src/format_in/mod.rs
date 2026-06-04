pub mod lof_reader;
pub mod tiff_reader;

use std::{io, path::Path};

use crate::{
    common::{Loc, Metadata, PixelSlice},
    format_in::{lof_reader::LofReader, tiff_reader::TiffReader},
};

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
pub enum Reader {
    Tiff(TiffReader),
    Lof(LofReader),
}

impl Reader {
    pub fn new(input_file: &String) -> io::Result<Self> {
        let ext_err = io::Error::other("No Extension");
        let uni_err = io::Error::other("Invalid Unicode");

        let ext = Path::new(input_file)
            .extension()
            .ok_or(ext_err)?
            .to_str()
            .ok_or(uni_err)?;

        match ext {
            "tiff" | "tif" => {
                let reader = TiffReader::new(input_file.clone())?;
                Ok(Reader::Tiff(reader))
            }
            "lof" => {
                let reader = LofReader::new(input_file.clone())?;
                Ok(Reader::Lof(reader))
            }
            other => Err(io::Error::other(format!(
                "Unsupported Reader Format: {}",
                other
            ))),
        }
    }
}

impl FormatReader for Reader {
    fn metadata(&self) -> &Metadata {
        match self {
            Reader::Tiff(reader) => reader.metadata(),
            Reader::Lof(reader) => reader.metadata(),
        }
    }

    fn open_bytes(&mut self, loc: Loc, df: u64) -> io::Result<Vec<u8>> {
        match self {
            Reader::Tiff(reader) => reader.open_bytes(loc, df),
            Reader::Lof(reader) => reader.open_bytes(loc, df),
        }
    }
}
