use std::io::{self, Error};

use crate::{
    common::{ChannelSeries, Loc, Metadata},
    format::tiff::{TiffDecoder, TiffEncoder},
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
        let encoder = TiffEncoder::new(self.file_name, *decoder.is_big_tiff())?;

        Ok(TiffWriter {
            metadata: decoder.metadata()?,
            encoder,
            decoder,
        })
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

    fn bits_per_pixel(mut self, cs: ChannelSeries, value: u16) -> Self {
        self.metadata.set_bits_per_pixel(cs, value);
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
        todo!()
    }

    fn build(self) -> io::Result<TiffWriter> {
        self.validate()?;

        let encoder = TiffEncoder::new(self.file_name.clone(), self.is_big_tiff)?;

        let n_ifds = self.metadata.series_count();
        for _ in 0..n_ifds {}

        let decoder = TiffDecoder::new(self.file_name)?;

        Ok(TiffWriter {
            metadata: self.metadata,
            encoder,
            decoder,
        })
    }
}

pub struct TiffWriter {
    metadata: Metadata,
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
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn construct_writer() {
        // Example of modifying existing tiff (ONLY writing pixel data)
        let writer = TiffWriter::new("file".into()).modify().unwrap();

        // Example of creating brand new tiff from scratch
        let writer = TiffWriter::new("file".into())
            .create()
            .dimensions(0, 10, 10)
            .bits_per_pixel((0, 0), 8)
            .bits_per_pixel((0, 1), 8)
            .bits_per_pixel((0, 2), 8)
            .build()
            .unwrap();

        // Example for use during format conversion
        // 'metadat' could come from some other format reader
        let metadata = Metadata::default();
        let writer = TiffWriter::new("file".into())
            .create()
            .set_metadata(metadata)
            .build()
            .unwrap();
    }
}
