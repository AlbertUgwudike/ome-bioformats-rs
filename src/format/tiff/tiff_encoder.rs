use std::{
    fs::File,
    io::{self, Error},
};

use ome_common_rs::ios::RandomAccessOutputStream;

use crate::common::{ByteOrder, Compression, Metadata};

use crate::format::tiff::{
    Datum,
    ifd::{Entry, IFD, Tag, Type},
};

pub struct TiffEncoder {
    ostream: RandomAccessOutputStream<File>,
    is_big_tiff: bool,
}

impl TiffEncoder {
    pub fn new(file: String, is_big_tiff: bool) -> io::Result<Self> {
        let mut ostream = RandomAccessOutputStream::new(file)?;
        Self::init_stream(&mut ostream, is_big_tiff)?;

        Ok(Self {
            ostream,
            is_big_tiff,
        })
    }

    fn init_stream(ostream: &mut RandomAccessOutputStream<File>, is_bt: bool) -> io::Result<()> {
        ostream.write_bytes(&['M' as u8, 'M' as u8])?;
        ostream.write_u16(if is_bt { 43 } else { 42 })?;

        let first_offset = if is_bt { 16 } else { 8 };

        if is_bt {
            ostream.write_u32(0)?;
            ostream.write_u64(first_offset as u64)?;
        } else {
            ostream.write_u32(first_offset)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intialise_encoder() {}
}
