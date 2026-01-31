use std::{
    collections::HashMap,
    io::{self},
};

pub mod tiff;
pub mod tiff_reader;

type ChannelSeries = (u64, u64);
type ChannelSeriesMap<T> = HashMap<ChannelSeries, T>;

#[derive(Clone, Copy, Default)]
pub struct Loc {
    x: u64,
    y: u64,
    z: u64,
    c: u64,
    t: u64,
    s: u64,
}

impl Loc {
    fn new(x: u64, y: u64, z: u64, c: u64, t: u64, s: u64) -> Self {
        Loc { x, y, z, c, t, s }
    }

    fn channel_series(&self) -> ChannelSeries {
        (self.c, self.s)
    }
}

#[derive(Debug)]
pub struct Dim {
    w: u64,
    h: u64,
    d: u64,
    t: u64,
    c: u64,
}

impl Dim {
    fn from_whc(w: u64, h: u64, d: u64) -> Self {
        Self {
            w,
            h,
            d,
            t: 1,
            c: 1,
        }
    }
}

#[derive(Debug)]
pub enum ByteOrder {
    BE,
    LE,
}

#[derive(Debug)]
pub struct Metadata {
    dimensions: HashMap<u64, Dim>,
    bits_per_pixel: ChannelSeriesMap<u16>,
    byte_order: ByteOrder,
}

impl Metadata {
    // We allow the bit depth to vary between channels/series
    fn bits_per_pixel(&self, cs: ChannelSeries) -> Option<&u16> {
        self.bits_per_pixel.get(&cs)
    }

    fn byte_order(&self) -> &ByteOrder {
        &self.byte_order
    }
}

#[derive(Debug)]
pub enum PixelSlice {
    U8(Vec<u8>),
    U16(Vec<u16>),
    // and so on ...
}

pub trait FormatReader {
    // ----------------- Required -------------------

    fn metadata(&mut self) -> io::Result<Metadata>;

    // Read rectangular portion of image data at given location
    // returns bytes, image metadata should be used to decode bytes
    fn open_bytes(&mut self, origin: Loc, h: u64, w: u64) -> io::Result<Vec<u8>>;

    // ----------------- Derived -------------------

    // Read rectangular portion of image data at given location
    // returns PixelSlice
    fn open_pixels(&mut self, origin: Loc, h: u64, w: u64) -> io::Result<PixelSlice> {
        let bytes = self.open_bytes(origin, h, w)?;
        let md = self.metadata()?;

        let bbp = md
            .bits_per_pixel(origin.channel_series())
            .ok_or(io::Error::other("Error reading bpp"))?;

        match bbp {
            8 => Ok(PixelSlice::U8(bytes)),
            16 => Ok(PixelSlice::U16(
                bytes
                    .chunks_exact(2)
                    .map(|a| match md.byte_order {
                        ByteOrder::LE => u16::from_le_bytes([a[0], a[1]]),
                        ByteOrder::BE => u16::from_be_bytes([a[0], a[1]]),
                    })
                    .collect(),
            )),
            _ => Err(io::Error::other("Unsupported PixelSlice Format")),
        }
    }
}
