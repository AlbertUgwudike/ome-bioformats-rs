pub mod compression;

pub use compression::Compression;

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

    pub fn dimensions(&self, series: usize) -> Option<&Dim> {
        self.dimensions.get(series)
    }

    // We allow the bit depth to vary between channels/series
    pub fn bits_per_pixel(&self, series: usize) -> Option<&Vec<u16>> {
        self.bits_per_pixel.get(series)
    }

    pub fn set_bits_per_pixel(&mut self, series: usize, v: Vec<u16>) {
        if series < self.bits_per_pixel.len() {
            self.bits_per_pixel[series] = v
        } else {
            for _ in 0..series - self.bits_per_pixel.len() {
                self.bits_per_pixel.push(vec![]);
            }
            self.bits_per_pixel.push(v)
        }
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

impl PixelSlice {
    pub fn to_be_bytes(&self, buff: &mut [u8]) {
        let bytes = match self {
            PixelSlice::U8(v) => v,
            PixelSlice::U16(v) => &v
                .iter()
                .map(|u| u.to_be_bytes())
                .flatten()
                .collect::<Vec<u8>>(),
        };
        buff.copy_from_slice(&bytes);
    }

    pub fn len(&self) -> usize {
        match self {
            PixelSlice::U8(v) => v.len(),
            PixelSlice::U16(v) => v.len(),
        }
    }

    pub fn bytes_len(&self) -> usize {
        match self {
            PixelSlice::U8(v) => v.len(),
            PixelSlice::U16(v) => 2 * v.len(),
        }
    }
}
