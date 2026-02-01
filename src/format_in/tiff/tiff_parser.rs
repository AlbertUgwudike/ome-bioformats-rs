use std::{
    fs::File,
    io::{self, Error},
};

use either::Either::{Left, Right};
use ome_common_rs::ios::RandomAccessInputStream;

use crate::format_in::{
    ByteOrder,
    tiff::{
        Datum,
        compression::Compression,
        ifd::{Entry, IFD, Tag, Type},
    },
};

pub struct TiffParser {
    istream: RandomAccessInputStream<File>,
    is_big_tiff: bool,
    first_ifd_offset: u64,
}

impl TiffParser {
    pub fn new(file: String) -> io::Result<Self> {
        let mut istream = RandomAccessInputStream::from_file(file)?;
        let (is_big_tiff, first_ifd_offset) = Self::init_stream(&mut istream)?;
        // let bytes_per_entry = if is_big_tiff { 20 } else { 12 };

        Ok(Self {
            istream,
            is_big_tiff,
            first_ifd_offset,
        })
    }

    fn init_stream(istream: &mut RandomAccessInputStream<File>) -> io::Result<(bool, u64)> {
        istream.seek_abs(0)?;

        let first_two_chars = (istream.read_char()?, istream.read_char()?);
        let is_le = match first_two_chars {
            ('I', 'I') => Ok(true),
            ('M', 'M') => Ok(false),
            _ => Err(Error::other(format!("First two bytes incorrect"))),
        }?;

        istream.order(is_le);

        let is_bt = match istream.read_u16()? {
            43 => Ok(true),
            42 => Ok(false),
            _ => Err(Error::other(format!("Invalid magic number"))),
        }?;

        let first_offset = if is_bt {
            istream.skip_bytes(4)?;
            istream.read_u64()?
        } else {
            istream.read_u32()? as u64
        };

        Ok((is_bt, first_offset))
    }

    fn read_ifd(&mut self) -> io::Result<IFD> {
        let n_entries = if self.is_big_tiff {
            self.istream.read_u64()?
        } else {
            self.istream.read_u16()? as u64
        };

        let mut entry_vec = Vec::with_capacity(n_entries as usize);

        for _ in 0..n_entries {
            let tag_short = self.istream.read_u16()?;
            let tag = Tag::from_short(tag_short)
                .ok_or(Error::other(format!("Failed Parse Tag: {tag_short}")))?;

            let kind_short = self.istream.read_u16()?;
            let kind = Type::from_short(kind_short)
                .ok_or(Error::other(format!("Failed Parse Type: {kind_short}")))?;

            let count = self.read_offset()?;

            let n_bytes = IFD::size_of(kind, count);

            // println!(
            //     "TAG: {:<25}  | KIND: {:10}  | COUNT: {:6}  | BYTES: {:4}",
            //     tag.to_str(),
            //     kind.to_str(),
            //     count,
            //     n_bytes
            // );

            let offset;
            let threshold = if self.is_big_tiff { 8 } else { 4 };

            if n_bytes > threshold {
                offset = Left(self.read_offset()?);
            } else {
                offset = Right(self.read_datum(kind, count)?);
                self.istream.skip_bytes(threshold - n_bytes)?;
            };

            entry_vec.push(Entry::new(tag, kind, count, offset))
        }

        let next_ifd_offset = self.read_offset()?;
        let new_ifd = IFD::new(entry_vec, next_ifd_offset);

        Ok(new_ifd)
    }

    // The number of IFDs
    pub fn n_ifds(&mut self) -> io::Result<i32> {
        let mut count = 1;
        self.istream.seek_abs(self.first_ifd_offset)?;
        let mut curr_ifd = self.read_ifd()?;

        while *curr_ifd.next_ifd_offset() != 0 {
            count += 1;
            self.istream.seek_abs(*curr_ifd.next_ifd_offset())?;
            curr_ifd = self.read_ifd()?;
        }

        Ok(count)
    }

    fn read_offset(&mut self) -> io::Result<u64> {
        if self.is_big_tiff {
            self.istream.read_u64()
        } else {
            self.istream.read_u32().map(|v| v as u64)
        }
    }

    pub fn nth_ifd(&mut self, i: u64) -> io::Result<IFD> {
        self.istream.seek_abs(self.first_ifd_offset)?;
        let mut curr_ifd = self.read_ifd()?;

        for j in 1..i + 1 {
            let next_offset = curr_ifd.next_ifd_offset();
            if *next_offset == 0 {
                return Err(Error::other(format!("IFD idx out of bounds: {i}/{j}")));
            }
            self.istream.seek_abs(*next_offset)?;
            curr_ifd = self.read_ifd()?;
        }

        Ok(curr_ifd)
    }

    pub fn read_entry(&mut self, ifd: &IFD, tag: Tag) -> io::Result<Datum> {
        let entry = ifd
            .get_entry(tag)
            .ok_or(Error::other(format!("Tag parse error: {:?}", tag)))?;

        match &entry.offset_or_datum {
            Left(offset) => {
                self.istream.seek_abs(*offset)?;
                self.read_datum(entry.kind, entry.count)
            }
            // Cloning here is inexpensive as datum is limited to 4 bytes
            Right(datum) => Ok(datum.clone()),
        }
    }

    fn read_datum(&mut self, kind: Type, count: u64) -> io::Result<Datum> {
        let byte_count = IFD::size_of(kind, count) as usize;
        let offset = self.istream.get_file_pointer()?;

        let mut buff = vec![0; byte_count];

        let is_le = self.istream.is_little_endian();
        let n = self.istream.read(&mut buff, offset)?;

        if n < byte_count {
            return Err(Error::other("Insufficient byes read"));
        }

        Ok(match kind {
            Type::BYTE | Type::UNDEFINED => Datum::U8(buff),
            Type::SHORT => Datum::from_bytes_u16(&buff, is_le),
            Type::LONG => Datum::from_bytes_u32(&buff, is_le),
            Type::DOUBLE => Datum::from_bytes_u64(&buff, is_le),
            Type::ASCII => Datum::STR(String::from_utf8(buff).map_err(|_| Error::other("ASCII"))?),
            Type::RATIONAL => Datum::from_bytes_rational(&buff, is_le),
        })
    }

    pub fn byte_order(&mut self) -> ByteOrder {
        if self.istream.is_little_endian() {
            ByteOrder::LE
        } else {
            ByteOrder::BE
        }
    }

    pub fn strip_byte_counts(&mut self, ifd: &IFD) -> io::Result<Vec<u64>> {
        // Array of SHORT OR LONG in tiff spec, use most permissive
        self.read_entry(ifd, Tag::StripByteCounts)?
            .to_vec_u64()
            .ok_or(Error::other("Failed parse strip byte counts"))
    }

    pub fn image_length(&mut self, ifd: &IFD) -> io::Result<u64> {
        self.read_entry(ifd, Tag::ImageLength)?
            .to_u64()
            .ok_or(Error::other("Failed parse ImageLength"))
    }

    pub fn image_width(&mut self, ifd: &IFD) -> io::Result<u64> {
        self.read_entry(ifd, Tag::ImageWidth)?
            .to_u64()
            .ok_or(Error::other("Failed parse ImageWidth"))
    }

    pub fn rows_per_strip(&mut self, ifd: &IFD) -> io::Result<u64> {
        self.read_entry(ifd, Tag::RowsPerStrip)?
            .to_u64()
            .ok_or(Error::other("Failed parse RowsPerStrip"))
    }

    pub fn strip_offsets(&mut self, ifd: &IFD) -> io::Result<Vec<u64>> {
        // Array of SHORT OR LONG in tiff spec, use most permissive
        self.read_entry(ifd, Tag::StripOffsets)?
            .to_vec_u64()
            .ok_or(Error::other("Failed parse strip offsets"))
    }

    pub fn bits_per_sample(&mut self, ifd: &IFD) -> io::Result<Vec<u16>> {
        // Array of SHORT OR LONG in tiff spec, use most permissive
        self.read_entry(ifd, Tag::BitsPerSample)?
            .to_vec_u16()
            .ok_or(Error::other("Failed parse bits per sample"))
    }

    pub fn samples_per_pixel(&mut self, ifd: &IFD) -> io::Result<u16> {
        self.read_entry(ifd, Tag::SamplesPerPixel)?
            .to_u16()
            .ok_or(Error::other("Failed parse samples per pixel"))
    }

    pub fn planar_configuration(&mut self, ifd: &IFD) -> io::Result<u16> {
        self.read_entry(ifd, Tag::PlanarConfiguration)?
            .to_u16()
            .ok_or(Error::other("Failed parse planar configuratoin"))
    }

    pub fn compression(&mut self, ifd: &IFD) -> io::Result<Compression> {
        self.read_entry(ifd, Tag::Compression)?
            .to_u16()
            .ok_or(Error::other("Failed parse compression"))
            .map(|a| Compression::from_short(a).ok_or(Error::other("Failed parse compression")))
            .flatten()
    }

    pub fn fill_order(&mut self, ifd: &IFD) -> io::Result<u16> {
        self.read_entry(ifd, Tag::FillOrder)?
            .to_u16()
            .ok_or(Error::other("Failed parse fill order"))
    }

    pub fn orientation(&mut self, ifd: &IFD) -> io::Result<u16> {
        self.read_entry(ifd, Tag::FillOrder)?
            .to_u16()
            .ok_or(Error::other("Failed parse orientation"))
    }

    pub fn read_strip(
        &mut self,
        ifd: &IFD,
        strip_idx: u64,
        buff: &mut [u8],
        expected_bytes: u64,
    ) -> io::Result<()> {
        let strip_offsets = self.strip_offsets(ifd)?;
        let offset = strip_offsets
            .get(strip_idx as usize)
            .ok_or(Error::other("Strip offset index out of range"))?;
        self.istream.seek_abs(*offset)?;

        match self.compression(&ifd)? {
            Compression::PackBits => {
                Compression::unpackbits(&mut self.istream, buff, expected_bytes)?;
            }
            Compression::CCITT => todo!(),
            Compression::None => {
                self.istream.read(buff, *offset as u64)?;
            }
        };

        Ok(())
    }

    pub fn is_big_tiff(&self) -> &bool {
        &self.is_big_tiff
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intialise_parser() {
        let tp = TiffParser::new("assets/example_valid.tiff".into()).unwrap();

        assert!(!tp.is_big_tiff);
        assert!(!tp.istream.is_little_endian());
    }
}
