pub mod compression;

pub use compression::Compression;

use std::collections::HashMap;

pub type ChannelSeries = (usize, usize);
pub type ChannelSeriesMap<T> = HashMap<ChannelSeries, T>;

#[derive(Clone, Copy, Default)]
pub struct Loc {
    pub x: u64,
    pub y: u64,
    pub z: u64,
    pub c: u64,
    pub t: u64,
    pub s: u64,
}

impl Loc {
    pub fn new(x: u64, y: u64, z: u64, c: u64, t: u64, s: u64) -> Self {
        Loc { x, y, z, c, t, s }
    }

    pub fn channel_series(&self) -> ChannelSeries {
        (self.c as usize, self.s as usize)
    }
}

#[derive(Debug, Copy, Clone)]
pub struct Dim {
    pub w: u64,
    pub h: u64,
    pub d: u64,
    pub t: u64,
    pub c: u64,
}

impl Dim {
    pub fn from_whd(w: u64, h: u64, d: u64) -> Self {
        Self {
            w,
            h,
            d,
            t: 1,
            c: 1,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub enum ByteOrder {
    #[default]
    BE,
    LE,
}

#[derive(Clone, Debug, Default)]
pub struct Metadata {
    dimensions: Vec<Dim>,
    bits_per_pixel: Vec<Vec<u16>>,
    byte_order: ByteOrder,
}

impl Metadata {
    pub fn new(dimensions: Vec<Dim>, bits_per_pixel: Vec<Vec<u16>>, byte_order: ByteOrder) -> Self {
        Metadata {
            dimensions,
            bits_per_pixel,
            byte_order,
        }
    }

    pub fn set_dimensions(&mut self, series: usize, h: u64, w: u64, d: u64) {
        if series < self.dimensions.len() {
            self.dimensions[series].h = h;
            self.dimensions[series].w = w;
            self.dimensions[series].d = d;
        } else {
            for _ in 0..series - self.dimensions.len() {
                self.dimensions.push(Dim::from_whd(0, 0, 0));
            }
            self.dimensions.push(Dim::from_whd(w, h, d))
        }
    }

    // We allow the bit depth to vary between channels/series
    pub fn bits_per_pixel(&self, cs: ChannelSeries) -> Option<&u16> {
        self.bits_per_pixel.get(cs.0).map(|v| v.get(cs.1)).flatten()
    }

    pub fn set_bits_per_pixel(&mut self, cs: ChannelSeries, v: u16) {
        self.bits_per_pixel
            .get_mut(cs.0)
            .map(|n| n.get_mut(cs.1).map(|t| *t = v));
    }

    pub fn byte_order(&self) -> &ByteOrder {
        &self.byte_order
    }

    pub fn series_count(&self) -> usize {
        self.dimensions.len()
    }
}

#[derive(Debug)]
pub enum PixelSlice {
    U8(Vec<u8>),
    U16(Vec<u16>),
    // and so on ...
}
