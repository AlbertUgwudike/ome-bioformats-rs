use std::{
    io::{self, Read, Seek},
    usize,
};

use ome_common_rs::ios::RandomAccessInputStream;

#[derive(Debug)]
pub enum Compression {
    None = 1,
    CCITT = 2,
    PackBits = 32773,
}

impl Compression {
    pub fn from_short(val: u16) -> Option<Self> {
        match val {
            1 => Some(Self::None),
            2 => Some(Self::CCITT),
            32773 => Some(Self::PackBits),
            _ => None,
        }
    }

    pub fn unpackbits_stream<T: Read + Seek>(
        istream: &mut RandomAccessInputStream<T>,
        buff: &mut [u8],
        expected_byte_count: u64,
    ) -> io::Result<()> {
        let mut curr_byte_idx: usize = 0;

        while (curr_byte_idx as u64) < expected_byte_count && istream.available()? != 0 {
            let byte = istream.read_byte()?;
            let count = byte as usize;

            if byte == 128 {
                continue;
            } else if byte > 128 {
                let next_byte = istream.read_byte()?;

                buff.get_mut(curr_byte_idx..(curr_byte_idx + 256 - count + 1))
                    .map(|a| a.fill(next_byte));

                curr_byte_idx += 256 - count + 1;
            } else {
                let offset = istream.get_file_pointer()?;

                buff.get_mut(curr_byte_idx..(curr_byte_idx + count + 1))
                    .map(|a| istream.read(a, offset));

                curr_byte_idx += count + 1;
            }
        }

        Ok(())
    }

    pub fn unpackbits(
        in_buff: &mut [u8],
        input_len: u64,
        out_buff: &mut [u8],
        output_len: u64,
    ) -> io::Result<()> {
        let mut in_idx = 0;
        let mut out_idx = 0;

        while in_idx < input_len as usize && out_idx < output_len as usize {
            let byte = in_buff[in_idx];
            let count = byte as usize;

            if byte == 128 {
                in_idx += 1;
                continue;
            } else if byte > 128 {
                let next_byte = in_buff[in_idx + 1];
                out_buff[out_idx..(out_idx + 256 - count + 1)].fill(next_byte);

                out_idx += 256 - count + 1;
                in_idx += 2;
            } else {
                let bytes = &in_buff[in_idx + 1..in_idx + count + 2];
                out_buff[out_idx..(out_idx + count + 1)].copy_from_slice(bytes);

                out_idx += count + 1;
                in_idx += count + 2;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::format_in::tiff::compression::Compression;
    use ome_common_rs::ios::RandomAccessInputStream;

    #[test]
    fn test_unpackbits() {
        let input: Vec<u8> = vec![
            0xFE, 0xAA, 0x02, 0x80, 0x00, 0x2A, 0xFD, 0xAA, 0x03, 0x80, 0x00, 0x2A, 0x22, 0xF7,
            0xAA,
        ];

        let mut istream = RandomAccessInputStream::from_byte_array(&input);

        let expected_output: Vec<u8> = vec![
            0xAA, 0xAA, 0xAA, 0x80, 0x00, 0x2A, 0xAA, 0xAA, 0xAA, 0xAA, 0x80, 0x00, 0x2A, 0x22,
            0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA,
        ];

        let mut output_buff = vec![0; 24];

        Compression::unpackbits_stream(&mut istream, &mut output_buff, 24).unwrap();

        assert_eq!(output_buff, expected_output);
    }
}
