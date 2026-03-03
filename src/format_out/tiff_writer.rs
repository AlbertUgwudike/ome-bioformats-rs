use std::io::{self, Error};

use crate::{
    common::{Loc, Metadata},
    format::tiff::{TiffDecoder, TiffEncoder},
};

use super::FormatWriter;

pub struct TiffInit {
    file_name: String,
}

impl TiffInit {
    pub fn modify(self) -> io::Result<TiffWriter> {
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

    pub fn create(self) -> TiffBuilder {
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

    pub fn bits_per_pixel(mut self, series: usize, value: Vec<u16>) -> Self {
        self.metadata.set_bits_per_pixel(series, value);
        self
    }

    pub fn dimensions(mut self, series: usize, h: u64, w: u64) -> Self {
        self.metadata.set_dimensions(series, h, w, 1);
        self
    }

    pub fn big_tiff(mut self, is_big_tiff: bool) -> Self {
        self.is_big_tiff = is_big_tiff;
        self
    }

    // Will be useful for rosetta
    pub fn set_metadata(mut self, metadata: Metadata) -> Self {
        self.metadata = metadata;
        self
    }

    fn validate(&self) -> io::Result<()> {
        // Runtime check that metadata is valid
        //
        // todo!()
        Ok(())
    }

    pub fn build(self) -> io::Result<TiffWriter> {
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

fn strided_copy(dest: &mut [u8], src: &[u8], chunk_size: usize, stride: usize) {
    let mut k = 0;
    dest.chunks_exact_mut(chunk_size)
        .step_by(stride)
        .for_each(|s| {
            s.copy_from_slice(&src[k..k + chunk_size]);
            k += chunk_size;
        });
}

impl FormatWriter for TiffWriter {
    fn write_bytes(&mut self, bytes: Vec<u8>, origin: Loc, h: u64, w: u64) -> std::io::Result<()> {
        let ifd = self.decoder.nth_ifd(origin.s)?;
        let strip_offsets = self.decoder.strip_offsets(&ifd)?;

        // declared here as we need overall row_idx (i.e. increments between strips)
        let mut src_row_idx = 0;

        self.decoder.apply_roi_strips(origin, h, w, |strip, sd| {
            let bytes_per_src_row = sd.bytes_per_sample * w as usize;

            strip
                .chunks_exact_mut(sd.bytes_per_row as usize)
                .skip(sd.lower_row)
                .take(sd.upper_row - sd.lower_row)
                .for_each(|row| {
                    let lower_src_col = src_row_idx * bytes_per_src_row as usize;
                    let upper_src_col = (src_row_idx + 1) * bytes_per_src_row as usize;
                    let src_row = &bytes[lower_src_col..upper_src_col];

                    if sd.is_chunky {
                        let skip = sd.bytes_per_sample * origin.c as usize;
                        let dest = &mut row[sd.lower_col + skip..sd.upper_col];
                        let chunk_size = sd.bytes_per_sample;
                        let stride = sd.samples_per_pixel;
                        strided_copy(dest, src_row, chunk_size, stride);
                    } else {
                        row[sd.lower_col..sd.upper_col].copy_from_slice(src_row);
                    }

                    src_row_idx += 1;
                });

            let offset = strip_offsets
                .get(sd.strip_idx)
                .ok_or(Error::other("Strip offset index out of range"))?;

            self.encoder.write_strip(*offset, &strip)
        })?;

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
        let mut writer = TiffWriter::new("assets/three.tiff".into())
            .create()
            .dimensions(0, 10, 10)
            .dimensions(1, 500, 500)
            .dimensions(2, 1000, 1000)
            .dimensions(3, 10000, 10000)
            .dimensions(4, 50000, 50000)
            .bits_per_pixel(0, [8, 8, 8].to_vec())
            .bits_per_pixel(1, [8, 8, 8].to_vec())
            .bits_per_pixel(2, [8, 8, 8].to_vec())
            .bits_per_pixel(3, [8, 8, 8].to_vec())
            .bits_per_pixel(4, [8, 8, 8].to_vec())
            .build()
            .unwrap();

        let mut g = |r: u64, s: u64| {
            let bytes = vec![255; ((r * r) / 25) as usize];
            let origin = Loc::new((r * 2) / 5, (r * 2) / 5, 0, 1, 0, s);
            writer.write_bytes(bytes, origin, r / 5, r / 5).unwrap();
        };

        g(10, 0);
        g(500, 1);
        g(1000, 2);
        g(10000, 3);
        g(50000, 4);

        // assert!(2 == 1)

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
