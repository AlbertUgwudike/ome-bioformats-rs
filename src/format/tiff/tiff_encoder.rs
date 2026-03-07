use std::{
    fs::File,
    io::{self, Error},
};

use ome_common_rs::ios::RandomAccessOutputStream;

use crate::common::Metadata;

use crate::format::tiff::ifd::{Tag, Type};

pub struct TiffEncoder {
    ostream: RandomAccessOutputStream<File>,
    is_big_tiff: bool,
    metadata: Metadata,
}

impl TiffEncoder {
    pub fn create(file: String, metadata: Metadata, is_big_tiff: bool) -> io::Result<Self> {
        let ostream = RandomAccessOutputStream::new(file)?;

        let mut encoder = Self {
            ostream,
            is_big_tiff,
            metadata,
        };

        encoder.write_header()?;
        encoder.write_empty_data()?;

        Ok(encoder)
    }

    pub fn modify(file: String, metadata: Metadata, is_big_tiff: bool) -> io::Result<Self> {
        Ok(Self {
            ostream: RandomAccessOutputStream::modify(file)?,
            is_big_tiff,
            metadata,
        })
    }

    fn write_header(&mut self) -> io::Result<()> {
        self.ostream.write_bytes(&['M' as u8, 'M' as u8])?;
        self.ostream.write_u16(self.magic_number())?;

        let first_ifd_offset = self.first_ifd_offset();

        if self.is_big_tiff {
            self.ostream.write_u32(0)?;
            self.ostream.write_u64(first_ifd_offset)?;
        } else {
            self.ostream.write_u32(first_ifd_offset as u32)?;
        }

        Ok(())
    }

    fn write_empty_data(&mut self) -> io::Result<()> {
        let ifd_size_bytes = TiffEncoder::ifd_size(self.is_big_tiff);
        // let mut curr_ifd_offset = self.first_ifd_offset();

        let n_ifds = self.metadata.series_count();
        for i in 0..n_ifds {
            let next_ifd_pos = self.ostream.length()?;
            self.ostream.seek_abs(next_ifd_pos)?;

            // HERE: Allocate (write) the space for the IFD (i.e. initialise data pointer)
            self.ostream
                .write_bytes_exact(&vec![0; ifd_size_bytes as usize], next_ifd_pos)?;

            // THEN: we need to initlaise the data pointer, data written imediately after IFD
            // let mut data_pos = (curr_ifd_offset + ifd_size_bytes) as usize;

            if self.is_big_tiff {
                self.ostream.write_u64(TiffEncoder::ENTRY_COUNT)?;
            } else {
                self.ostream.write_u16(TiffEncoder::ENTRY_COUNT as u16)?;
            }

            let dims = self
                .metadata
                .dimensions(i)
                .ok_or(Error::other("Bad index"))?;

            let length = (dims.h as u16).to_be_bytes();
            let width = (dims.w as u16).to_be_bytes();

            let bpp = self
                .metadata
                .bits_per_pixel(i)
                .ok_or(io::Error::other("Error reading bpp"))?;

            let bpp_bytes = bpp
                .iter()
                .map(|a| a.to_be_bytes())
                .flatten()
                .collect::<Vec<u8>>();

            let spp = bpp.len() as u16;
            let bytes_per_pixel = bpp.iter().sum::<u16>() as u64 / 8;
            let bytes_per_row = dims.w * bytes_per_pixel;
            let pi = if spp > 1 { 2 } else { 1 };

            let rows_per_strip = TiffEncoder::MAX_STRIP_BYTE_COUNT / bytes_per_row as usize;
            let rows_per_strip = std::cmp::max(rows_per_strip, 1);

            let bytes_per_strip = rows_per_strip * bytes_per_row as usize;
            let rows_in_last = dims.h % rows_per_strip as u64;
            let strip_count = dims.h.div_ceil(rows_per_strip as u64) as usize;

            println!(
                "SC: {:?}, BIL: {:?}, BPS: {:?}",
                strip_count, rows_in_last, bytes_per_strip
            );

            println!("W: {:?}, BPP: {:?}", dims.w, bytes_per_pixel);

            let mut byte_counts = vec![bytes_per_strip as u32; strip_count];

            if rows_in_last != 0 {
                byte_counts[strip_count - 1] = (rows_in_last * bytes_per_row) as u32;
            }

            // HERE: write in IFDs + data sequentially
            self.write_entry(Tag::ImageLength, Type::SHORT, 1, &length)?;
            self.write_entry(Tag::ImageWidth, Type::SHORT, 1, &width)?;
            self.write_entry(Tag::BitsPerSample, Type::SHORT, spp as u64, &bpp_bytes)?;
            self.write_entry(Tag::Compression, Type::SHORT, 1, &[0, 1])?;
            self.write_entry(Tag::PhotometricInterpretation, Type::SHORT, 1, &[0, pi])?;
            self.write_entry(Tag::PlanarConfiguration, Type::SHORT, 1, &[0, 1])?;

            self.write_entry(
                Tag::StripByteCounts,
                Type::LONG,
                strip_count as u64,
                &byte_counts
                    .iter()
                    .map(|v| v.to_be_bytes())
                    .flatten()
                    .collect::<Vec<u8>>(),
            )?;

            self.write_entry(
                Tag::RowsPerStrip,
                Type::SHORT,
                1,
                &(rows_per_strip as u16).to_be_bytes(),
            )?;

            self.write_entry(Tag::SamplesPerPixel, Type::SHORT, 1, &spp.to_be_bytes())?;

            // TODO: Change so user can configure Res units through metadata
            self.write_entry(Tag::XResolution, Type::RATIONAL, 1, &0u64.to_be_bytes())?;
            self.write_entry(Tag::YResolution, Type::RATIONAL, 1, &0u64.to_be_bytes())?;
            self.write_entry(Tag::ResolutionUnit, Type::SHORT, 1, &[0, 1])?;

            let data_pos = self.ostream.length()? as usize;
            let strip_offsets = (0..strip_count)
                .map(|j| data_pos + strip_count * 4 + j * byte_counts[0] as usize)
                .collect::<Vec<_>>();

            self.write_entry(
                Tag::StripOffsets,
                Type::LONG,
                strip_count as u64,
                &strip_offsets
                    .iter()
                    .map(|v| (*v as u32).to_be_bytes())
                    .flatten()
                    .collect::<Vec<u8>>(),
            )?;

            // Write zero pixels
            let mut start = self.ostream.length()?;
            for j in 0..strip_count {
                let bytes = vec![0u8; byte_counts[j] as usize];
                self.ostream.write_bytes_exact(&bytes, start)?;
                start += byte_counts[j] as u64;
            }

            // Write next_ifd_offset
            let next_offset = if i == n_ifds - 1 { 0 } else { start };
            self.write_offset(next_offset)?;
        }

        Ok(())
    }

    fn write_entry(&mut self, tag: Tag, kind: Type, count: u64, data: &[u8]) -> io::Result<()> {
        self.ostream.write_u16(tag as u16)?;
        self.ostream.write_u16(kind as u16)?;
        self.write_offset(count)?;

        let data_pos = self.ostream.length()?;
        let threshold = if self.is_big_tiff { 8 } else { 4 };

        if data.len() > threshold {
            self.write_offset(data_pos)?;
            self.ostream.write_bytes_exact(data, data_pos)?;
        } else {
            let n_pad_bytes = (threshold - data.len()) as usize;
            self.ostream.write_bytes(data)?;
            self.ostream.skip_bytes(n_pad_bytes as u64)?;
        }

        Ok(())
    }

    fn write_offset(&mut self, offset: u64) -> io::Result<usize> {
        if self.is_big_tiff {
            self.ostream.write_u64(offset)
        } else {
            self.ostream.write_u32(offset as u32)
        }
    }

    fn first_ifd_offset(&self) -> u64 {
        if self.is_big_tiff {
            TiffEncoder::BIG_HEADR_SIZE
        } else {
            TiffEncoder::HEADER_SIZE
        }
    }

    fn magic_number(&self) -> u16 {
        if self.is_big_tiff { 43 } else { 42 }
    }

    pub fn write_strip(&mut self, offset: u64, buff: &[u8]) -> io::Result<()> {
        self.ostream.write_bytes_exact(buff, offset as u64)?;
        Ok(())
    }

    // -------------- Associated Items ---------------

    const HEADER_SIZE: u64 = 8;
    const BIG_HEADR_SIZE: u64 = 16;
    const ENTRY_COUNT: u64 = 13; // <- See ifd.rs, TODO: add support for custom tags?
    const MAX_STRIP_BYTE_COUNT: usize = 500000;

    fn ifd_size(is_bt: bool) -> u64 {
        let entry_count_bytes = if is_bt { 8 } else { 2 };
        let total_entry_bytes = TiffEncoder::entry_size(is_bt) * TiffEncoder::ENTRY_COUNT;
        let next_ifd_bytes = if is_bt { 8 } else { 4 };
        entry_count_bytes + total_entry_bytes + next_ifd_bytes
    }

    fn entry_size(is_bt: bool) -> u64 {
        if is_bt { 28 } else { 12 }
    }
}

#[cfg(test)]
mod tests {
    // use super::*;

    #[test]
    fn intialise_encoder() {}
}
