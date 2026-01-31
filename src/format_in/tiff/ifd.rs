use std::collections::HashMap;

use either::Either;

#[derive(Debug)]
pub struct IFD {
    next_ifd_offset: u64,
    entries: HashMap<Tag, Entry>,
}

impl IFD {
    pub fn new(entry_vec: Vec<Entry>, next_ifd_offset: u64) -> Self {
        let mut entries = HashMap::new();

        entry_vec.into_iter().for_each(|a| {
            entries.insert(a.tag, a);
        });

        IFD {
            next_ifd_offset,
            entries,
        }
    }
    pub fn next_ifd_offset(&self) -> &u64 {
        &self.next_ifd_offset
    }

    pub fn n_entries(&self) -> usize {
        self.entries.len()
    }

    pub fn insert_entry(&mut self, entry: Entry) {
        self.entries.insert(entry.tag, entry);
    }

    pub fn get_entry(&self, tag: Tag) -> Option<&Entry> {
        self.entries.get(&tag)
    }

    pub fn size_of(kind: Type, count: u64) -> u64 {
        match kind {
            Type::ASCII | Type::BYTE | Type::UNDEFINED => 1 * count as u64,
            Type::SHORT => 2 * count as u64,
            Type::LONG => 4 * count as u64,
            Type::RATIONAL | Type::DOUBLE => 8 * count as u64,
        }
    }
}

#[derive(Debug)]
pub struct Entry {
    tag: Tag,
    pub kind: Type,
    pub count: u64,
    pub offset_or_datum: Either<u64, Datum>,
}

impl Entry {
    pub fn new(tag: Tag, kind: Type, count: u64, offset: Either<u64, Datum>) -> Self {
        Entry {
            tag,
            kind,
            count,
            offset_or_datum: offset,
        }
    }
}

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub enum Tag {
    ImageWidth = 256,
    ImageLength = 257,
    BitsPerSample = 258,
    Compression = 259,
    PhotometricInterpretation = 262,
    FillOrder = 266,
    StripOffsets = 273,
    Orientation = 274,
    SamplesPerPixel = 277,
    RowsPerStrip = 278,
    StripByteCounts = 279,
    XResolution = 282,
    YResolution = 283,
    PlanarConfiguration = 284,
    ResolutionUnit = 296,
    ExtraSamples = 338,
    SampleFormat = 339,
    Other = 0,
}

impl Tag {
    pub fn from_short(val: u16) -> Option<Self> {
        match val {
            256 => Some(Self::ImageWidth),
            257 => Some(Self::ImageLength),
            258 => Some(Self::BitsPerSample),
            259 => Some(Self::Compression),
            262 => Some(Self::PhotometricInterpretation),
            266 => Some(Self::FillOrder),
            273 => Some(Self::StripOffsets),
            274 => Some(Self::Orientation),
            277 => Some(Self::SamplesPerPixel),
            278 => Some(Self::RowsPerStrip),
            279 => Some(Self::StripByteCounts),
            282 => Some(Self::XResolution),
            283 => Some(Self::YResolution),
            284 => Some(Self::PlanarConfiguration),
            296 => Some(Self::ResolutionUnit),
            338 => Some(Self::ExtraSamples),
            339 => Some(Self::SampleFormat),
            _ => Some(Self::Other),
        }
    }

    pub fn to_str(&self) -> String {
        format!("{:?}", self)
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Type {
    BYTE = 1,
    ASCII,
    SHORT,
    LONG,
    RATIONAL,
    UNDEFINED = 7,
    DOUBLE = 16,
}

impl Type {
    pub fn from_short(val: u16) -> Option<Self> {
        match val {
            1 => Some(Type::BYTE),
            2 => Some(Type::ASCII),
            3 => Some(Type::SHORT),
            4 => Some(Type::LONG),
            5 => Some(Type::RATIONAL),
            7 => Some(Type::UNDEFINED),
            16 => Some(Type::DOUBLE),
            _ => None,
        }
    }

    pub fn to_str(&self) -> String {
        format!("{:?}", self)
    }
}

#[derive(Debug, Clone)]
pub enum Datum {
    // All tiff values are arrays!
    U8(Vec<u8>),          // Type::BYTE
    STR(String),          // Type::ASCII
    U16(Vec<u16>),        // Type::SHORT
    U32(Vec<u32>),        // Type::LONG
    U64(Vec<u64>),        // Type::DOUBLE
    RAT(Vec<(u32, u32)>), // Type::RATIONAL
}

impl Datum {
    pub fn to_vec_u64(&self) -> Option<Vec<u64>> {
        match self {
            Self::U8(v) => Some(v.into_iter().map(|a| *a as u64).collect()),
            Self::U16(v) => Some(v.into_iter().map(|a| *a as u64).collect()),
            Self::U32(v) => Some(v.into_iter().map(|a| *a as u64).collect()),
            Self::U64(v) => Some(v.to_vec()),
            _ => None,
        }
    }

    pub fn to_vec_u32(&self) -> Option<Vec<u32>> {
        match self {
            Self::U8(v) => Some(v.into_iter().map(|a| *a as u32).collect()),
            Self::U16(v) => Some(v.into_iter().map(|a| *a as u32).collect()),
            Self::U32(v) => Some(v.to_vec()),
            _ => None,
        }
    }

    pub fn to_vec_u16(&self) -> Option<Vec<u16>> {
        match self {
            Self::U8(v) => Some(v.into_iter().map(|a| *a as u16).collect()),
            Self::U16(v) => Some(v.to_vec()),
            _ => None,
        }
    }

    pub fn to_vec_u8(&self) -> Option<Vec<u8>> {
        match self {
            Self::U8(v) => Some(v.to_vec()),
            _ => None,
        }
    }

    pub fn to_u64(&self) -> Option<u64> {
        match self {
            Self::U8(v) => Some(v.get(0).map(|a| a.to_owned() as u64)).flatten(),
            Self::U16(v) => Some(v.get(0).map(|a| a.to_owned() as u64)).flatten(),
            Self::U32(v) => Some(v.get(0).map(|a| a.to_owned() as u64)).flatten(),
            Self::U64(v) => Some(v.get(0).map(|a| a.to_owned())).flatten(),
            _ => None,
        }
    }

    pub fn to_u16(&self) -> Option<u16> {
        match self {
            Self::U8(v) => Some(v.get(0).map(|a| a.to_owned() as u16)).flatten(),
            Self::U16(v) => Some(v.get(0).map(|a| a.to_owned())).flatten(),
            _ => None,
        }
    }

    pub fn to_u8(&self) -> Option<u8> {
        match self {
            Self::U8(v) => Some(v.get(0).map(|a| a.to_owned())).flatten(),
            _ => None,
        }
    }
}
