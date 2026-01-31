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
    bytes_per_entry: u64,
    first_ifd_offset: u64,
}

impl TiffParser {
    pub fn new(file: String) -> io::Result<Self> {
        let mut istream = RandomAccessInputStream::new(file)?;
        let (is_big_tiff, first_ifd_offset) = Self::init_stream(&mut istream)?;
        let bytes_per_entry = if is_big_tiff { 16 } else { 12 };

        Ok(Self {
            istream,
            is_big_tiff,
            first_ifd_offset,
            bytes_per_entry,
        })
    }

    fn init_stream(istream: &mut RandomAccessInputStream<File>) -> io::Result<(bool, u64)> {
        istream.mark()?;
        istream.seek_abs(0)?;

        let first_two_chars = (istream.read_char()?, istream.read_char()?);
        let is_le = match first_two_chars {
            ('I', 'I') => Ok(true),
            ('M', 'M') => Ok(false),
            _ => Err(Error::other(format!("First two bytes incorrect"))),
        }?;

        istream.order(is_le);

        let is_bt = match istream.read_short()? {
            43 => Ok(true),
            42 => Ok(false),
            _ => Err(Error::other(format!("Invalid magic number"))),
        }?;

        let first_offset = istream.read_u32()? as u64;

        istream.reset()?;
        Ok((is_bt, first_offset))
    }

    fn sequence<T: Clone>(v: Vec<io::Result<T>>) -> io::Result<Vec<T>> {
        let mut res = vec![];
        for i in v {
            match &i {
                Ok(j) => res.push(j.clone()),
                Err(_) => return Err(Error::other("")),
            }
        }
        Ok(res)
    }

    fn read_ifd(&mut self) -> io::Result<IFD> {
        let n_entries = self.istream.read_short()? as u64;
        let mut entry_vec = Vec::with_capacity(n_entries as usize);

        for _ in 0..n_entries {
            let tag_short = self.istream.read_short()?;
            let tag = Tag::from_short(tag_short)
                .ok_or(Error::other(format!("Failed Parse Tag: {tag_short}")))?;

            let kind_short = self.istream.read_short()?;
            let kind = Type::from_short(kind_short)
                .ok_or(Error::other(format!("Failed Parse Type: {kind_short}")))?;

            let count = self.istream.read_u32()?;
            let n_bytes = IFD::size_of(kind, count);

            // println!(
            //     "TAG: {:<25}  | KIND: {:10}  | COUNT: {:4}  | BYTES: {:4}",
            //     tag.to_str(),
            //     kind.to_str(),
            //     count,
            //     n_bytes
            // );

            let offset;
            if n_bytes > 4 {
                offset = Left(self.istream.read_u32()? as u64);
            } else {
                offset = Right(self.read_datum(kind, count)?);
                self.istream.skip_bytes(4 - n_bytes)?;
            };

            entry_vec.push(Entry::new(tag, kind, count, offset))
        }

        let next_ifd_offset = self.istream.read_u32()? as u64;
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

    pub fn nth_ifd(&mut self, i: i32) -> io::Result<IFD> {
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
        let e = ifd.get_entry(tag).ok_or(Error::other("error"))?;
        match &e.offset_or_datum {
            Left(offset) => {
                self.istream.seek_abs(*offset)?;
                self.read_datum(e.kind, e.count)
            }
            // Cloning here is inexpensive as datum is limited to 4 bytes
            Right(datum) => Ok(datum.clone()),
        }
    }

    fn read_datum(&mut self, kind: Type, count: u32) -> io::Result<Datum> {
        Ok(match kind {
            Type::BYTE => Datum::U8(Self::sequence(
                (0..count).map(|_| self.istream.read_byte()).collect(),
            )?),
            Type::SHORT => Datum::U16(Self::sequence(
                (0..count).map(|_| self.istream.read_short()).collect(),
            )?),
            Type::LONG => Datum::U32(Self::sequence(
                (0..count).map(|_| self.istream.read_u32()).collect(),
            )?),
            Type::ASCII => Datum::STR(
                Self::sequence((0..count).map(|_| self.istream.read_char()).collect())?
                    .iter()
                    .fold(String::new(), |a, b| a + &b.to_string()),
            ),
            Type::RATIONAL => Datum::RAT(Self::sequence(
                (0..count)
                    .map(|_| Ok((self.istream.read_u32()?, self.istream.read_u32()?)))
                    .collect(),
            )?),
        })
    }

    pub fn byte_order(&mut self) -> ByteOrder {
        if self.istream.is_little_endian() {
            ByteOrder::LE
        } else {
            ByteOrder::BE
        }
    }

    pub fn strip_byte_counts(&mut self, ifd: &IFD) -> io::Result<Vec<u32>> {
        // Array of SHORT OR LONG in tiff spec, use most permissive
        self.read_entry(ifd, Tag::StripByteCounts)?
            .to_vec_u32()
            .ok_or(Error::other("Failed parse strip byte counts"))
    }

    pub fn image_length(&mut self, ifd: &IFD) -> io::Result<u16> {
        self.read_entry(ifd, Tag::ImageLength)?
            .to_u16()
            .ok_or(Error::other("Failed parse ImageLength"))
    }

    pub fn image_width(&mut self, ifd: &IFD) -> io::Result<u16> {
        self.read_entry(ifd, Tag::ImageWidth)?
            .to_u16()
            .ok_or(Error::other("Failed parse ImageWidth"))
    }

    pub fn rows_per_strip(&mut self, ifd: &IFD) -> io::Result<u16> {
        self.read_entry(ifd, Tag::RowsPerStrip)?
            .to_u16()
            .ok_or(Error::other("Failed parse RowsPerStrip"))
    }

    pub fn strip_offsets(&mut self, ifd: &IFD) -> io::Result<Vec<u32>> {
        // Array of SHORT OR LONG in tiff spec, use most permissive
        self.read_entry(ifd, Tag::StripOffsets)?
            .to_vec_u32()
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
        strip_idx: i32,
        bytes_per_pixel: i32,
    ) -> io::Result<Vec<u8>> {
        let strip_offsets = self.strip_offsets(ifd)?;
        let offset = strip_offsets
            .get(strip_idx as usize)
            .ok_or(Error::other("Strip offset index out of range"))?;

        let strip_byte_counts = self.strip_byte_counts(ifd)?;
        let strip_byte_count = strip_byte_counts
            .get(strip_idx as usize)
            .ok_or(Error::other("Strip byte count index out of range"))?;

        let rows_per_strip = self.rows_per_strip(ifd)?;
        let strip_count = strip_byte_counts.len();
        let row_count = if strip_idx as usize == strip_count - 1 {
            self.image_length(ifd)? % rows_per_strip
        } else {
            rows_per_strip
        } as i32;

        let expected_byte_count = row_count * self.image_width(ifd)? as i32 * bytes_per_pixel;

        let mut bytes = vec![0].repeat(*strip_byte_count as usize);
        self.istream.read(&mut bytes, *offset as u64)?;

        match self.compression(&ifd)? {
            Compression::PackBits => Compression::unpackbits(bytes, expected_byte_count),
            Compression::CCITT => todo!(),
            Compression::None => Ok(bytes),
        }
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
