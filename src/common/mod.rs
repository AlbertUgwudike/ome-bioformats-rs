pub mod compression;

use std::io;

pub use compression::Compression;

#[derive(Clone, Copy, Default, Debug)]
pub struct Loc {
    pub x: u64,
    pub y: u64,
    pub z: u64,
    pub c: u64,
    pub t: u64,
    pub s: u64,
    pub h: u64,
    pub w: u64,
}

impl Loc {
    pub fn new(x: u64, y: u64, z: u64, c: u64, t: u64, s: u64, h: u64, w: u64) -> Self {
        Loc {
            x,
            y,
            z,
            c,
            t,
            s,
            h,
            w,
        }
    }

    pub fn origin() -> Self {
        Loc::new(0, 0, 0, 0, 0, 0, 0, 0)
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
    pub fn new(w: u64, h: u64, d: u64, c: u64, t: u64) -> Self {
        Self { w, h, d, t, c }
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
                self.dimensions.push(Dim::new(1, 1, 1, 1, 1));
            }
            self.dimensions.push(Dim::new(w, h, d, 1, 1))
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

    pub fn locs(&self, row_count: usize) -> Vec<Loc> {
        let mut perms = vec![];
        for series_idx in 0..self.series_count() {
            let dim = self.dimensions(series_idx).unwrap();
            for time_idx in 0..dim.t {
                for channel_idx in 0..dim.c {
                    for z_idx in 0..dim.d {
                        for r in 0..(dim.h as usize / row_count) {
                            perms.push(Loc::new(
                                0,
                                (r * row_count) as u64,
                                z_idx,
                                channel_idx,
                                time_idx,
                                series_idx as u64,
                                row_count as u64,
                                dim.w,
                            ));
                        }

                        if dim.h % row_count as u64 != 0 {
                            perms.push(Loc::new(
                                0,
                                dim.h - (dim.h % row_count as u64),
                                z_idx,
                                channel_idx,
                                time_idx,
                                series_idx as u64,
                                dim.h % row_count as u64,
                                dim.w,
                            ));
                        }
                    }
                }
            }
        }
        perms
    }

    pub fn plane_locs(&self) -> Vec<Loc> {
        let mut perms = vec![];
        for series_idx in 0..self.series_count() {
            let dim = self.dimensions(series_idx).unwrap();
            for time_idx in 0..dim.t {
                for channel_idx in 0..dim.c {
                    for z_idx in 0..dim.d {
                        perms.push(Loc::new(
                            0,
                            0,
                            z_idx,
                            channel_idx,
                            time_idx,
                            series_idx as u64,
                            dim.h,
                            dim.w,
                        ));
                    }
                }
            }
        }
        perms
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum PixelSlice {
    U8(Vec<u8>),
    U16(Vec<u16>),
    U32(Vec<u32>),
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
            PixelSlice::U32(v) => &v
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
            PixelSlice::U32(v) => v.len(),
        }
    }

    pub fn bytes_len(&self) -> usize {
        match self {
            PixelSlice::U8(v) => v.len(),
            PixelSlice::U16(v) => 2 * v.len(),
            PixelSlice::U32(v) => 4 * v.len(),
        }
    }

    pub fn to_u16vec(&self) -> Vec<u16> {
        match self {
            PixelSlice::U8(v) => v.iter().map(|a| *a as u16).collect(),
            PixelSlice::U16(v) => v.to_vec(),
            PixelSlice::U32(v) => v
                .iter()
                .map(|a| std::cmp::min(u16::MAX as u32, *a) as u16)
                .collect(),
        }
    }

    pub fn interpret_bytes(
        bbp: u16,
        byte_order: ByteOrder,
        bytes: &[u8],
    ) -> io::Result<PixelSlice> {
        match bbp {
            8 => Ok(PixelSlice::U8(bytes.to_vec())),
            16 => Ok(PixelSlice::U16(
                bytes
                    .chunks_exact(2)
                    .map(|a| match byte_order {
                        ByteOrder::LE => u16::from_le_bytes([a[0], a[1]]),
                        ByteOrder::BE => u16::from_be_bytes([a[0], a[1]]),
                    })
                    .collect(),
            )),
            32 => Ok(PixelSlice::U32(
                bytes
                    .chunks_exact(4)
                    .map(|a| match byte_order {
                        ByteOrder::LE => u32::from_le_bytes([a[0], a[1], a[2], a[3]]),
                        ByteOrder::BE => u32::from_be_bytes([a[0], a[1], a[2], a[3]]),
                    })
                    .collect(),
            )),
            n => Err(io::Error::other(format!(
                "Unsupported PixelSlice Format {n}"
            ))),
        }
    }
}
