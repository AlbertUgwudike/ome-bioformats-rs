use std::io::{self, Error};

use crate::{
    common::{ChannelSeries, Loc, Metadata},
    format::tiff::{TiffDecoder, TiffEncoder, ifd::IFD},
};

use super::FormatWriter;

pub struct TiffInit {
    file_name: String,
}

impl TiffInit {
    fn modify(self) -> io::Result<TiffWriter> {
        // 1. Check file exists
        if !std::fs::exists(&self.file_name)? {
            return Err(Error::other("File not found"));
        }

        // 2. Check file is tiff and get handle to parser
        let mut decoder = TiffDecoder::new(self.file_name.clone())?;
        let encoder =
            TiffEncoder::modify(self.file_name, decoder.metadata()?, decoder.is_big_tiff())?;

        Ok(TiffWriter { encoder, decoder })
    }

    fn create(self) -> TiffBuilder {
        TiffBuilder::new(self.file_name)
    }
}

pub struct TiffBuilder {
    file_name: String,
    metadata: Metadata,
    is_big_tiff: bool,
}

impl TiffBuilder {
    fn new(file_name: String) -> Self {
        Self {
            file_name,
            metadata: Metadata::default(),
            is_big_tiff: false,
        }
    }

    fn bits_per_pixel(mut self, series: usize, value: Vec<u16>) -> Self {
        self.metadata.set_bits_per_pixel(series, value);
        self
    }

    fn dimensions(mut self, series: usize, h: u64, w: u64) -> Self {
        self.metadata.set_dimensions(series, h, w, 1);
        self
    }

    fn big_tiff(mut self, is_big_tiff: bool) -> Self {
        self.is_big_tiff = is_big_tiff;
        self
    }

    // Will be useful for rosetta
    fn set_metadata(mut self, metadata: Metadata) -> Self {
        self.metadata = metadata;
        self
    }

    fn validate(&self) -> io::Result<()> {
        // Runtime check that metadata is valid
        //
        // todo!()
        Ok(())
    }

    fn build(self) -> io::Result<TiffWriter> {
        self.validate()?;

        let encoder = TiffEncoder::create(self.file_name.clone(), self.metadata, self.is_big_tiff)?;
        let decoder = TiffDecoder::new(self.file_name)?;

        Ok(TiffWriter { encoder, decoder })
    }
}

pub struct TiffWriter {
    encoder: TiffEncoder,
    decoder: TiffDecoder,
}

impl TiffWriter {
    pub fn new(file_name: String) -> TiffInit {
        TiffInit { file_name }
    }
}

impl FormatWriter for TiffWriter {
    fn write_bytes(&mut self, bytes: Vec<u8>, origin: Loc, h: u64, w: u64) -> std::io::Result<()> {
        // todo!()
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn construct_writer() {
        // Example of modifying existing tiff (ONLY writing pixel data)
        // let writer = TiffWriter::new("assets/one.tiff".into()).modify().unwrap();

        // Example of creating brand new tiff from scratch
        let writer = TiffWriter::new("assets/two.tiff".into())
            .create()
            .dimensions(0, 1000, 1000)
            .bits_per_pixel(0, [8, 8, 8, 8].to_vec())
            .build()
            .unwrap();

        assert!(2 == 1)

        // Example for use during format conversion
        // 'metadata' could come from some other format reader
        // let metadata = Metadata::default();
        // let writer = TiffWriter::new("assets/three.tiff".into())
        //     .create()
        //     .set_metadata(metadata)
        //     .build()
        //     .unwrap();
    }
}
