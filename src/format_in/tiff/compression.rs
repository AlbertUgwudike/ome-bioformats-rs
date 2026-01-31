use std::io::{self, Read, Seek};

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

    pub fn unpackbits<T: Read + Seek>(
        istream: &mut RandomAccessInputStream<T>,
        buff: &mut [u8],
        expected_byte_count: u64,
    ) -> io::Result<()> {
        let mut curr_byte_idx: usize = 0;

        if istream.available()? == 0 {
            return Ok(());
        }

        while (curr_byte_idx as u64) < expected_byte_count {
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

        let mut istream = RandomAccessInputStream::from_byte_array(input);

        let expected_output: Vec<u8> = vec![
            0xAA, 0xAA, 0xAA, 0x80, 0x00, 0x2A, 0xAA, 0xAA, 0xAA, 0xAA, 0x80, 0x00, 0x2A, 0x22,
            0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA,
        ];

        let mut output_buff = vec![0; 24];

        Compression::unpackbits(&mut istream, &mut output_buff, 24).unwrap();

        assert_eq!(output_buff, expected_output);
    }
}
