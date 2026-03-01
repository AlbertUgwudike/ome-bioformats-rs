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
        let Loc { x, y, z, c, t, s } = origin;

        let ifd = self.decoder.nth_ifd(s)?;
        let iw = self.decoder.image_width(&ifd)?;
        let bits_per_sample = self.decoder.bits_per_sample(&ifd)?;
        let samples_per_pixel = bits_per_sample.len();
        let bytes_per_sample = (bits_per_sample[c as usize] / 8) as usize;
        let is_chunky = self.decoder.planar_configuration(&ifd)? == 1;
        let rows_per_strip = self.decoder.rows_per_strip(&ifd)? as u64;
        let n_strips = self.decoder.strip_offsets(&ifd)?.len() as u64;

        let bytes_per_pixel = if is_chunky {
            // Chunky configuration, 'c' samples per pixel
            bits_per_sample.into_iter().map(|a| a as u64).sum::<u64>() / 8
        } else {
            // Planar configuration, one sample per pixel
            *bits_per_sample
                .get(c as usize)
                .ok_or(Error::other("Invalid c"))? as u64
                / 8
        };

        println!("BPPS: {:?}", bytes_per_pixel);
        let bytes_per_pixel = 2;

        let start_idx = y / rows_per_strip;
        let end_idx = (y + h) / rows_per_strip;

        let mut buff = vec![0; (bytes_per_pixel * iw * rows_per_strip) as usize];
        // let mut out = Vec::with_capacity((h * w * bytes_per_pixel) as usize);
        let mut src_row_idx = 0;

        for strip_idx in start_idx..end_idx + 1 {
            // Calculate start/end indexes into image rows
            let s_idx = (strip_idx * rows_per_strip) as usize;
            let e_idx = ((strip_idx + 1) * rows_per_strip) as usize;

            // Calculate start/end indices into a vector of strip rows
            let lower_idx = std::cmp::max(s_idx, y as usize) - s_idx;
            let upper_idx = std::cmp::min(e_idx, (y + h) as usize) - s_idx;

            // Chunk and change
            let bytes_per_row = bytes_per_pixel * iw;
            let lower_col = (bytes_per_pixel * x) as usize;
            let upper_col = lower_col + (bytes_per_pixel * w) as usize;

            let expected_bytes = if strip_idx + 1 == n_strips {
                bytes_per_pixel * iw * ((y + h) % rows_per_strip)
            } else {
                bytes_per_pixel * iw * rows_per_strip
            };

            self.decoder
                .read_strip(&ifd, strip_idx, &mut buff, expected_bytes)?;

            buff.chunks_exact_mut(bytes_per_row as usize)
                .skip(lower_idx)
                .take(upper_idx - lower_idx)
                .for_each(|row| {
                    let bytes_per_src_row = bytes_per_pixel * w;
                    let lower_src_col = src_row_idx * bytes_per_src_row as usize;
                    let upper_src_col = (src_row_idx + 1) * bytes_per_src_row as usize;
                    let src_row = &bytes[lower_src_col..upper_src_col];
                    // row[lower_col..upper_col].copy_from_slice(src_row);
                    row[lower_col..upper_col].fill(100u8);
                    src_row_idx += 1;
                });

            let strip_offsets = self.decoder.strip_offsets(&ifd)?;
            let offset = strip_offsets
                .get(strip_idx as usize)
                .ok_or(Error::other("Strip offset index out of range"))?;

            self.encoder.write_strip(*offset, &buff)?;
        }

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
        let mut writer = TiffWriter::new("assets/two.tiff".into())
            .create()
            .dimensions(0, 10, 10)
            .dimensions(1, 500, 500)
            .dimensions(2, 1000, 1000)
            .dimensions(3, 10000, 10000)
            .dimensions(4, 50000, 50000)
            .bits_per_pixel(0, [8, 8, 8, 8].to_vec())
            .bits_per_pixel(1, [8, 8, 8, 8].to_vec())
            .bits_per_pixel(2, [8, 8, 8, 8].to_vec())
            .bits_per_pixel(3, [8, 8, 8, 8].to_vec())
            .bits_per_pixel(4, [8, 8, 8, 8].to_vec())
            .build()
            .unwrap();

        let bytes = vec![255u8; 10000 * 4];
        let origin = Loc::new(200, 200, 0, 0, 0, 1);

        writer.write_bytes(bytes, origin, 100, 100).unwrap();

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
