use roxmltree::Document;
use std::{
    fs::File,
    io::{self, Error},
};

use ome_common_rs::ios::RandomAccessInputStream;

use crate::common::{Dim, Metadata};

pub struct LofDecoder {
    istream: RandomAccessInputStream<File>,
    data_offset: u64,
    xml_str: String,
}

impl LofDecoder {
    pub fn new(file: String) -> io::Result<Self> {
        let mut istream = RandomAccessInputStream::from_file(file)?;
        let (data_offset, xml_str) = Self::init_stream(&mut istream)?;

        Ok(Self {
            istream,
            data_offset,
            xml_str,
        })
    }

    fn init_stream(istream: &mut RandomAccessInputStream<File>) -> io::Result<(u64, String)> {
        istream.seek_abs(0)?;
        istream.order(true);

        LofDecoder::read_magic_byte(istream, 1)?;
        istream.skip_bytes(4)?;
        LofDecoder::read_memory_byte(istream, 1)?;

        let char_count = istream.read_u32()?;
        let type_name: String = istream
            .read_string(2 * char_count as usize)?
            .chars()
            .step_by(2)
            .collect();

        if type_name != "LMS_Object_File" {
            return Err(Error::other(format!("Incorrect Type Name {:?}", type_name)));
        }

        LofDecoder::read_memory_byte(istream, 2)?;
        istream.skip_bytes(4)?;
        LofDecoder::read_memory_byte(istream, 3)?;
        istream.skip_bytes(4)?;
        LofDecoder::read_memory_byte(istream, 4)?;

        let memory_size = istream.read_u64()?;
        let memory_offset = istream.get_file_pointer()?;
        istream.skip_bytes(memory_size)?;

        LofDecoder::read_magic_byte(istream, 2)?;
        istream.skip_bytes(4)?;
        LofDecoder::read_memory_byte(istream, 4)?;

        let xml_length = istream.read_u32()?;
        let xml_str: String = istream
            .read_string(2 * xml_length as usize)?
            .chars()
            .step_by(2)
            .collect();

        Ok((memory_offset, xml_str))
    }

    pub fn metadata(&self) -> io::Result<Metadata> {
        let mut bpp = Vec::new();
        let mut dim = Vec::new();

        let doc = Document::parse(&self.xml_str).map_err(|_| Error::other("error"))?;

        let image_desc = doc
            .descendants()
            .find(|n| n.has_tag_name("ImageDescription"))
            .ok_or(Error::other("Cannot find 'ImageDescription' element"))?;

        let channel_descs = image_desc
            .descendants()
            .filter(|n| n.has_tag_name("ChannelDescription"));

        let mut channle_incs = vec![];
        for channel_desc in channel_descs {
            channle_incs.push(
                channel_desc
                    .attribute("BytesInc")
                    .ok_or(Error::other("No BytesInc Attribute"))?
                    .parse::<u32>()
                    .unwrap(),
            );
        }

        let series_count = if channle_incs.len() <= 1 {
            1
        } else if let &[1, 2] = &channle_incs[..2] {
            1
        } else {
            channle_incs.len()
        };

        let mut descriptor = image_desc
            .descendants()
            .find(|n| n.has_tag_name("Dimensions"))
            .map(|des| {
                des.descendants()
                    .filter(|n| n.has_tag_name("DimensionDescription"))
            })
            .ok_or(Error::other("Cannot find 'Dimensions' element"))?;

        let x_dim = descriptor
            .find(|d| d.attribute("DimID") == Some("1"))
            .ok_or(Error::other("Cannot read X Dim"))?;

        let y_dim = descriptor
            .find(|d| d.attribute("DimID") == Some("2"))
            .ok_or(Error::other("Cannot read Y Dim"))?;

        let bytes_inc = x_dim
            .attribute("BytesInc")
            .ok_or(Error::other("No BytesInc Attribute"))?
            .parse::<u16>()
            .map_err(|e| Error::other(e.to_string()))?;

        let bytes_per_pixel = if bytes_inc % 3 == 0 {
            bytes_inc / 3
        } else {
            bytes_inc
        };

        let channel_count = if bytes_inc % 3 == 0 { 3 } else { 1 };

        let width = x_dim
            .attribute("NumberOfElements")
            .ok_or(Error::other("No NumberOfElements Attribute"))?
            .parse::<u64>()
            .map_err(|e| Error::other(e.to_string()))?;

        let height = y_dim
            .attribute("NumberOfElements")
            .ok_or(Error::other("No NumberOfElements Attribute"))?
            .parse::<u64>()
            .map_err(|e| Error::other(e.to_string()))?;

        for _ in 0..series_count {
            dim.push(Dim::from_whd(width, height, channel_count));
            bpp.push([bytes_per_pixel * 8].repeat(channel_count as usize));
        }

        Ok(Metadata::new(dim, bpp, crate::common::ByteOrder::LE))
    }

    fn read_memory_byte(istream: &mut RandomAccessInputStream<File>, n: usize) -> io::Result<()> {
        if istream.read_byte()? != LofDecoder::LOF_MEMORY_BYTE {
            return Err(Error::other(format!("Incorrect Memory Byte (No: {})", n)));
        }
        Ok(())
    }

    fn read_magic_byte(istream: &mut RandomAccessInputStream<File>, n: usize) -> io::Result<()> {
        if istream.read_u32()? != LofDecoder::LOF_MAGIC_BYTE as u32 {
            return Err(Error::other(format!("Incorrect Magic Byte (No: {})", n)));
        }
        Ok(())
    }

    pub fn read_pixel_bytes(&mut self, bytes_to_skip: u64, buff: &mut [u8]) -> io::Result<usize> {
        self.istream.read(buff, self.data_offset + bytes_to_skip)
    }

    const LOF_MAGIC_BYTE: u8 = 112;
    const LOF_MEMORY_BYTE: u8 = 42;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intialise_decoder() {
        LofDecoder::new("/Users/albert/Downloads/DAB test brain 1.lof".into()).unwrap();
    }
}
