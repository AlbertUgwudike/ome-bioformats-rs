//! IFD parsing. Port of `loci.formats.tiff.TiffParser` and `IFD`.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use super::{field, tag};
use crate::error::{Result, format_err, unsupported};
use crate::io::RandomAccessFile;
use crate::pixel::PixelType;

/// A decoded tag value.
#[derive(Debug, Clone, PartialEq)]
pub enum TagValue {
    /// BYTE, SHORT, LONG, LONG8 and IFD types.
    Unsigned(Vec<u64>),
    /// SBYTE, SSHORT, SLONG, SLONG8.
    Signed(Vec<i64>),
    Ascii(String),
    /// RATIONAL / SRATIONAL as (numerator, denominator).
    Rational(Vec<(i64, i64)>),
    Float(Vec<f64>),
    Undefined(Vec<u8>),
}

impl TagValue {
    pub fn as_u64s(&self) -> Option<&[u64]> {
        match self {
            TagValue::Unsigned(v) => Some(v),
            _ => None,
        }
    }

    pub fn first_u64(&self) -> Option<u64> {
        match self {
            TagValue::Unsigned(v) => v.first().copied(),
            TagValue::Signed(v) => v.first().map(|&x| x as u64),
            _ => None,
        }
    }
}

/// One image file directory.
#[derive(Debug, Clone, Default)]
pub struct Ifd {
    pub offset: u64,
    pub tags: BTreeMap<u16, TagValue>,
}

impl Ifd {
    pub fn get(&self, tag: u16) -> Option<&TagValue> {
        self.tags.get(&tag)
    }

    pub fn u64(&self, tag: u16) -> Option<u64> {
        self.get(tag).and_then(TagValue::first_u64)
    }

    fn required(&self, tag: u16) -> Result<u64> {
        self.u64(tag)
            .ok_or_else(|| format_err!("IFD at {} is missing required tag {tag}", self.offset))
    }

    pub fn ascii(&self, tag: u16) -> Option<&str> {
        match self.get(tag) {
            Some(TagValue::Ascii(s)) => Some(s),
            _ => None,
        }
    }

    pub fn width(&self) -> Result<u32> {
        Ok(self.required(tag::IMAGE_WIDTH)? as u32)
    }

    pub fn height(&self) -> Result<u32> {
        Ok(self.required(tag::IMAGE_LENGTH)? as u32)
    }

    pub fn samples_per_pixel(&self) -> u32 {
        self.u64(tag::SAMPLES_PER_PIXEL).unwrap_or(1).max(1) as u32
    }

    pub fn bits_per_sample(&self) -> u32 {
        self.u64(tag::BITS_PER_SAMPLE).unwrap_or(1) as u32
    }

    pub fn compression(&self) -> u16 {
        self.u64(tag::COMPRESSION).unwrap_or(1) as u16
    }

    pub fn planar(&self) -> bool {
        self.u64(tag::PLANAR_CONFIGURATION) == Some(2) && self.samples_per_pixel() > 1
    }

    pub fn predictor(&self) -> u16 {
        self.u64(tag::PREDICTOR).unwrap_or(1) as u16
    }

    pub fn is_tiled(&self) -> bool {
        self.get(tag::TILE_WIDTH).is_some() && self.get(tag::TILE_OFFSETS).is_some()
    }

    /// True for thumbnails and pyramid levels (NewSubfileType bit 0).
    pub fn is_reduced(&self) -> bool {
        self.u64(tag::NEW_SUBFILE_TYPE).unwrap_or(0) & 1 == 1
    }

    /// Port of `IFD.getPixelType`.
    pub fn pixel_type(&self) -> Result<PixelType> {
        let bits = self.bits_per_sample();
        let format = self.u64(tag::SAMPLE_FORMAT).unwrap_or(1);
        let bytes = match bits {
            8 => 1,
            16 => 2,
            32 => 4,
            64 => 8,
            other => return Err(unsupported!("{other}-bit TIFF samples")),
        };
        match format {
            1 | 4 => PixelType::from_bytes(bytes, false, false),
            2 => PixelType::from_bytes(bytes, true, false),
            3 if bytes >= 4 => PixelType::from_bytes(bytes, true, true),
            other => Err(unsupported!("TIFF SampleFormat {other} with {bits} bits")),
        }
    }

    /// `(chunk width, chunk height)`: the tile size, or full-width strips.
    pub fn chunk_size(&self) -> Result<(u32, u32)> {
        if self.is_tiled() {
            Ok((
                self.required(tag::TILE_WIDTH)? as u32,
                self.required(tag::TILE_LENGTH)? as u32,
            ))
        } else {
            let h = self.height()?;
            let rps = self
                .u64(tag::ROWS_PER_STRIP)
                .unwrap_or(h as u64)
                .clamp(1, h.max(1) as u64);
            Ok((self.width()?, rps as u32))
        }
    }

    pub fn chunk_offsets(&self) -> Result<&[u64]> {
        let t = if self.is_tiled() {
            tag::TILE_OFFSETS
        } else {
            tag::STRIP_OFFSETS
        };
        self.get(t)
            .and_then(TagValue::as_u64s)
            .ok_or_else(|| format_err!("IFD at {} has no data offsets", self.offset))
    }

    pub fn chunk_byte_counts(&self) -> Result<&[u64]> {
        let t = if self.is_tiled() {
            tag::TILE_BYTE_COUNTS
        } else {
            tag::STRIP_BYTE_COUNTS
        };
        self.get(t)
            .and_then(TagValue::as_u64s)
            .ok_or_else(|| format_err!("IFD at {} has no byte counts", self.offset))
    }
}

/// An open TIFF file and its main IFD chain.
#[derive(Debug)]
pub struct TiffFile {
    pub path: PathBuf,
    pub file: RandomAccessFile,
    pub little_endian: bool,
    pub big_tiff: bool,
    pub ifds: Vec<Ifd>,
}

/// Returns `Some((little_endian, big_tiff))` if `header` starts a TIFF file.
pub fn sniff(header: &[u8]) -> Option<(bool, bool)> {
    let le = match header.get(..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let magic = [header[2], *header.get(3)?];
    let magic = if le {
        u16::from_le_bytes(magic)
    } else {
        u16::from_be_bytes(magic)
    };
    match magic {
        42 => Some((le, false)),
        43 => Some((le, true)),
        _ => None,
    }
}

impl TiffFile {
    pub fn open(path: &Path) -> Result<Self> {
        let file = RandomAccessFile::open(path)?;
        let header = file.read_prefix(0, 16)?;
        let (little_endian, big_tiff) =
            sniff(&header).ok_or_else(|| format_err!("{} is not a TIFF file", path.display()))?;
        let mut cur = file.cursor(4, little_endian);
        let first = if big_tiff {
            let (size, zero) = (cur.read_u16()?, cur.read_u16()?);
            if size != 8 || zero != 0 {
                return Err(format_err!("unsupported BigTIFF offset size {size}"));
            }
            cur.read_u64()?
        } else {
            cur.read_u32()? as u64
        };
        let mut tiff = TiffFile {
            path: path.to_owned(),
            file,
            little_endian,
            big_tiff,
            ifds: Vec::new(),
        };
        let mut seen = HashSet::new();
        let mut next = first;
        while next != 0 && seen.insert(next) {
            if next >= tiff.file.len() {
                break; // truncated chain; keep what we have, as Bio-Formats does
            }
            let (ifd, following) = tiff.read_ifd(next)?;
            tiff.ifds.push(ifd);
            next = following;
        }
        if tiff.ifds.is_empty() {
            return Err(format_err!("{} contains no IFDs", path.display()));
        }
        Ok(tiff)
    }

    /// Reads the IFD at `offset`, returning it and the offset of the next one.
    pub fn read_ifd(&self, offset: u64) -> Result<(Ifd, u64)> {
        let mut cur = self.file.cursor(offset, self.little_endian);
        let count = if self.big_tiff {
            cur.read_u64()?
        } else {
            cur.read_u16()? as u64
        };
        let entry_len = if self.big_tiff { 20 } else { 12 };
        let table = cur.read_vec((count * entry_len) as usize)?;
        let next = if self.big_tiff {
            cur.read_u64()?
        } else {
            cur.read_u32()? as u64
        };

        let mut ifd = Ifd {
            offset,
            tags: BTreeMap::new(),
        };
        for entry in table.chunks_exact(entry_len as usize) {
            let u16_at = |i: usize| self.u16(&entry[i..i + 2]);
            let (tag, ty) = (u16_at(0), u16_at(2));
            let (count, value_bytes) = if self.big_tiff {
                (self.u64(&entry[4..12]), &entry[12..20])
            } else {
                (self.u32(&entry[4..8]) as u64, &entry[8..12])
            };
            let Some(size) = field::size(ty) else {
                continue;
            };
            let total = size as u64 * count;
            let raw = if total <= value_bytes.len() as u64 {
                value_bytes[..total as usize].to_vec()
            } else {
                let at = if self.big_tiff {
                    self.u64(value_bytes)
                } else {
                    self.u32(value_bytes) as u64
                };
                match self.file.read_vec_at(at, total as usize) {
                    Ok(v) => v,
                    Err(_) => continue, // dangling pointer; skip the tag rather than the file
                }
            };
            ifd.tags.insert(tag, self.decode(ty, &raw));
        }
        Ok((ifd, next))
    }

    fn u16(&self, b: &[u8]) -> u16 {
        let a = [b[0], b[1]];
        if self.little_endian {
            u16::from_le_bytes(a)
        } else {
            u16::from_be_bytes(a)
        }
    }

    fn u32(&self, b: &[u8]) -> u32 {
        let a = b[..4].try_into().unwrap();
        if self.little_endian {
            u32::from_le_bytes(a)
        } else {
            u32::from_be_bytes(a)
        }
    }

    fn u64(&self, b: &[u8]) -> u64 {
        let a = b[..8].try_into().unwrap();
        if self.little_endian {
            u64::from_le_bytes(a)
        } else {
            u64::from_be_bytes(a)
        }
    }

    fn decode(&self, ty: u16, raw: &[u8]) -> TagValue {
        match ty {
            field::BYTE => TagValue::Unsigned(raw.iter().map(|&b| b as u64).collect()),
            field::SBYTE => TagValue::Signed(raw.iter().map(|&b| b as i8 as i64).collect()),
            field::ASCII => {
                let end = raw.iter().rposition(|&b| b != 0).map_or(0, |p| p + 1);
                TagValue::Ascii(String::from_utf8_lossy(&raw[..end]).into_owned())
            }
            field::UNDEFINED => TagValue::Undefined(raw.to_vec()),
            field::SHORT => {
                TagValue::Unsigned(raw.chunks_exact(2).map(|c| self.u16(c) as u64).collect())
            }
            field::SSHORT => TagValue::Signed(
                raw.chunks_exact(2)
                    .map(|c| self.u16(c) as i16 as i64)
                    .collect(),
            ),
            field::LONG | field::IFD => {
                TagValue::Unsigned(raw.chunks_exact(4).map(|c| self.u32(c) as u64).collect())
            }
            field::SLONG => TagValue::Signed(
                raw.chunks_exact(4)
                    .map(|c| self.u32(c) as i32 as i64)
                    .collect(),
            ),
            field::LONG8 | field::IFD8 => {
                TagValue::Unsigned(raw.chunks_exact(8).map(|c| self.u64(c)).collect())
            }
            field::SLONG8 => {
                TagValue::Signed(raw.chunks_exact(8).map(|c| self.u64(c) as i64).collect())
            }
            field::RATIONAL => TagValue::Rational(
                raw.chunks_exact(8)
                    .map(|c| (self.u32(c) as i64, self.u32(&c[4..]) as i64))
                    .collect(),
            ),
            field::SRATIONAL => TagValue::Rational(
                raw.chunks_exact(8)
                    .map(|c| (self.u32(c) as i32 as i64, self.u32(&c[4..]) as i32 as i64))
                    .collect(),
            ),
            field::FLOAT => TagValue::Float(
                raw.chunks_exact(4)
                    .map(|c| f32::from_bits(self.u32(c)) as f64)
                    .collect(),
            ),
            field::DOUBLE => TagValue::Float(
                raw.chunks_exact(8)
                    .map(|c| f64::from_bits(self.u64(c)))
                    .collect(),
            ),
            _ => TagValue::Undefined(raw.to_vec()),
        }
    }
}
