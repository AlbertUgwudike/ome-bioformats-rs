use std::io::{self, Error};

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

    pub fn unpackbits(bytes: Vec<u8>, expected_byte_count: i32) -> io::Result<Vec<u8>> {
        let mut out: Vec<u8> = Vec::with_capacity(expected_byte_count as usize);
        let mut curr_byte_idx = 0;

        if bytes.len() == 0 {
            return Ok(out);
        }

        while curr_byte_idx < bytes.len() && out.len() < expected_byte_count as usize {
            let byte = bytes[curr_byte_idx];
            let count = byte as usize;

            if byte == 128 {
                curr_byte_idx += 1;
            } else if byte > 128 {
                let next_byte = bytes
                    .get(curr_byte_idx + 1)
                    .ok_or(Error::other(format!("Idx error")))?
                    .to_owned();

                out.extend_from_slice(&[next_byte].repeat(256 - count + 1));
                curr_byte_idx += 2;
            } else {
                let start = curr_byte_idx + 1;
                let end = start + count + 1;
                let next_bytes = bytes
                    .get(start..end)
                    .ok_or(Error::other(format!("Idx error")))?
                    .to_owned();

                out.extend_from_slice(&next_bytes);
                curr_byte_idx += count + 2;
            }
        }

        Ok(out)
    }
}

mod tests {
    use crate::format_in::tiff::compression::Compression;

    #[test]
    fn test_unpackbits() {
        let input: Vec<u8> = vec![
            0xFE, 0xAA, 0x02, 0x80, 0x00, 0x2A, 0xFD, 0xAA, 0x03, 0x80, 0x00, 0x2A, 0x22, 0xF7,
            0xAA,
        ];

        let expected_output: Vec<u8> = vec![
            0xAA, 0xAA, 0xAA, 0x80, 0x00, 0x2A, 0xAA, 0xAA, 0xAA, 0xAA, 0x80, 0x00, 0x2A, 0x22,
            0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA,
        ];

        let actual_out = Compression::unpackbits(input, 24).unwrap();

        assert_eq!(expected_output, actual_out);
    }
}
