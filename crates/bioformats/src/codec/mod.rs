//! Compression codecs. Port of `loci.formats.codec`.

mod packbits;
pub mod predictor;

use std::fmt;
use std::io::{Read, Write};
use std::str::FromStr;

use crate::error::{Error, Result, unsupported};

/// Compression schemes this crate can write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Compression {
    #[default]
    None,
    Lzw,
    /// zlib-wrapped deflate (TIFF compression 8). Level 0-9.
    Deflate {
        level: u32,
    },
}

impl Compression {
    pub const fn tiff_code(self) -> u16 {
        match self {
            Compression::None => 1,
            Compression::Lzw => 5,
            Compression::Deflate { .. } => 8,
        }
    }

    pub fn encode(self, data: &[u8]) -> Result<Vec<u8>> {
        match self {
            Compression::None => Ok(data.to_vec()),
            Compression::Lzw => {
                weezl::encode::Encoder::with_tiff_size_switch(weezl::BitOrder::Msb, 8)
                    .encode(data)
                    .map_err(|e| Error::Format(format!("LZW encode: {e}")))
            }
            Compression::Deflate { level } => {
                let mut enc = flate2::write::ZlibEncoder::new(
                    Vec::with_capacity(data.len() / 2),
                    flate2::Compression::new(level.min(9)),
                );
                enc.write_all(data)?;
                Ok(enc.finish()?)
            }
        }
    }
}

impl fmt::Display for Compression {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Compression::None => f.write_str("Uncompressed"),
            Compression::Lzw => f.write_str("LZW"),
            Compression::Deflate { level } => write!(f, "zlib (level {level})"),
        }
    }
}

impl FromStr for Compression {
    type Err = Error;

    /// Accepts the names `bfconvert -compression` uses, case-insensitively.
    fn from_str(s: &str) -> Result<Self> {
        match s.to_ascii_lowercase().as_str() {
            "uncompressed" | "none" => Ok(Compression::None),
            "lzw" => Ok(Compression::Lzw),
            "zlib" | "deflate" => Ok(Compression::Deflate { level: 6 }),
            other => Err(unsupported!(
                "compression {other:?} (expected uncompressed, lzw or zlib)"
            )),
        }
    }
}

/// Decompresses a TIFF strip or tile. `expected` is the decoded size; the
/// result is truncated or zero-padded to exactly that length, as libtiff does
/// for short strips.
pub fn decode_tiff(code: u16, data: &[u8], expected: usize) -> Result<Vec<u8>> {
    let mut out = match code {
        1 => data.to_vec(),
        5 => {
            let mut dec = weezl::decode::Decoder::with_tiff_size_switch(weezl::BitOrder::Msb, 8);
            let mut out = Vec::with_capacity(expected);
            let result = dec.into_vec(&mut out).decode(data);
            // Many writers omit the end-of-information code; keep whatever decoded.
            if let Err(e) = result.status
                && out.len() < expected
            {
                return Err(Error::Format(format!("LZW decode: {e}")));
            }
            out
        }
        8 | 32946 => {
            let mut out = Vec::with_capacity(expected);
            flate2::read::ZlibDecoder::new(data)
                .read_to_end(&mut out)
                .map_err(|e| Error::Format(format!("zlib decode: {e}")))?;
            out
        }
        32773 => packbits::decode(data, expected),
        6 | 7 => return Err(unsupported!("JPEG-compressed TIFF")),
        33003 | 33004 | 33005 | 34712 => return Err(unsupported!("JPEG 2000-compressed TIFF")),
        other => return Err(unsupported!("TIFF compression {other}")),
    };
    out.resize(expected, 0);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let data: Vec<u8> = (0..10_000u32).map(|i| (i % 251) as u8).collect();
        for c in [
            Compression::None,
            Compression::Lzw,
            Compression::Deflate { level: 6 },
        ] {
            let enc = c.encode(&data).unwrap();
            assert_eq!(
                decode_tiff(c.tiff_code(), &enc, data.len()).unwrap(),
                data,
                "{c}"
            );
        }
    }
}
