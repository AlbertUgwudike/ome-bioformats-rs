//! Pixel types and helpers for rearranging pixel buffers.
//!
//! Port of the pixel-type portion of `loci.formats.FormatTools` and the
//! byte-shuffling helpers from `loci.formats.ImageTools`.

use crate::error::{Error, Result};

/// The storage type of a single sample, matching the OME `PixelType` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PixelType {
    Int8,
    Uint8,
    Int16,
    Uint16,
    Int32,
    Uint32,
    Float,
    Double,
    Bit,
}

impl PixelType {
    /// Bytes used to store one sample. `Bit` is stored one sample per byte,
    /// as Bio-Formats does.
    pub const fn bytes(self) -> usize {
        match self {
            PixelType::Int8 | PixelType::Uint8 | PixelType::Bit => 1,
            PixelType::Int16 | PixelType::Uint16 => 2,
            PixelType::Int32 | PixelType::Uint32 | PixelType::Float => 4,
            PixelType::Double => 8,
        }
    }

    pub const fn is_signed(self) -> bool {
        matches!(
            self,
            PixelType::Int8
                | PixelType::Int16
                | PixelType::Int32
                | PixelType::Float
                | PixelType::Double
        )
    }

    pub const fn is_float(self) -> bool {
        matches!(self, PixelType::Float | PixelType::Double)
    }

    /// Port of `FormatTools.pixelTypeFromBytes`.
    pub fn from_bytes(bytes: usize, signed: bool, float: bool) -> Result<Self> {
        Ok(match (bytes, signed, float) {
            (1, false, _) => PixelType::Uint8,
            (1, true, _) => PixelType::Int8,
            (2, false, _) => PixelType::Uint16,
            (2, true, _) => PixelType::Int16,
            (4, _, true) => PixelType::Float,
            (4, false, false) => PixelType::Uint32,
            (4, true, false) => PixelType::Int32,
            (8, _, _) => PixelType::Double,
            _ => {
                return Err(Error::Format(format!(
                    "no pixel type with {bytes} bytes (signed={signed}, float={float})"
                )));
            }
        })
    }

    /// The value used in OME-XML `Pixels/@Type`.
    pub const fn ome_name(self) -> &'static str {
        match self {
            PixelType::Int8 => "int8",
            PixelType::Uint8 => "uint8",
            PixelType::Int16 => "int16",
            PixelType::Uint16 => "uint16",
            PixelType::Int32 => "int32",
            PixelType::Uint32 => "uint32",
            PixelType::Float => "float",
            PixelType::Double => "double",
            PixelType::Bit => "bit",
        }
    }

    pub fn from_ome_name(name: &str) -> Option<Self> {
        Some(match name {
            "int8" => PixelType::Int8,
            "uint8" => PixelType::Uint8,
            "int16" => PixelType::Int16,
            "uint16" => PixelType::Uint16,
            "int32" => PixelType::Int32,
            "uint32" => PixelType::Uint32,
            "float" => PixelType::Float,
            "double" => PixelType::Double,
            "bit" => PixelType::Bit,
            _ => return None,
        })
    }
}

/// Reverses the byte order of every `bytes`-wide sample in `buf`, in place.
pub fn swap_endianness(buf: &mut [u8], bytes: usize) {
    if bytes > 1 {
        for sample in buf.chunks_exact_mut(bytes) {
            sample.reverse();
        }
    }
}

/// Converts a planar (`RRR...GGG...BBB...`) block into interleaved
/// (`RGBRGB...`) order. `out` must be the same length as `planar`.
pub fn interleave(planar: &[u8], out: &mut [u8], samples: usize, bytes: usize) {
    debug_assert_eq!(planar.len(), out.len());
    let plane_len = planar.len() / samples;
    let pixels = plane_len / bytes;
    for s in 0..samples {
        let src = &planar[s * plane_len..(s + 1) * plane_len];
        for p in 0..pixels {
            let dst = (p * samples + s) * bytes;
            out[dst..dst + bytes].copy_from_slice(&src[p * bytes..(p + 1) * bytes]);
        }
    }
}

/// Converts an interleaved block into planar order; the inverse of [`interleave`].
pub fn deinterleave(interleaved: &[u8], out: &mut [u8], samples: usize, bytes: usize) {
    debug_assert_eq!(interleaved.len(), out.len());
    let plane_len = interleaved.len() / samples;
    let pixels = plane_len / bytes;
    for s in 0..samples {
        let dst = &mut out[s * plane_len..(s + 1) * plane_len];
        for p in 0..pixels {
            let src = (p * samples + s) * bytes;
            dst[p * bytes..(p + 1) * bytes].copy_from_slice(&interleaved[src..src + bytes]);
        }
    }
}

/// Swaps the first and third sample of every pixel, converting BGR to RGB.
/// Port of `ImageTools.bgrToRgb`.
pub fn bgr_to_rgb(buf: &mut [u8], interleaved: bool, bytes: usize, samples: usize) {
    if samples < 3 {
        return;
    }
    if interleaved {
        let px = bytes * samples;
        for pixel in buf.chunks_exact_mut(px) {
            for b in 0..bytes {
                pixel.swap(b, 2 * bytes + b);
            }
        }
    } else {
        let plane_len = buf.len() / samples;
        let (first, rest) = buf.split_at_mut(plane_len);
        first.swap_with_slice(&mut rest[plane_len..2 * plane_len]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interleave_round_trip() {
        let planar = [1u8, 2, 3, 4, 10, 20, 30, 40, 100, 101, 102, 103];
        let mut inter = [0u8; 12];
        interleave(&planar, &mut inter, 3, 2);
        assert_eq!(inter, [1, 2, 10, 20, 100, 101, 3, 4, 30, 40, 102, 103]);
        let mut back = [0u8; 12];
        deinterleave(&inter, &mut back, 3, 2);
        assert_eq!(back, planar);
    }

    #[test]
    fn bgr_swap() {
        let mut px = [1u8, 2, 3, 4, 5, 6];
        bgr_to_rgb(&mut px, true, 1, 3);
        assert_eq!(px, [3, 2, 1, 6, 5, 4]);
        let mut planar = [1u8, 1, 2, 2, 3, 3];
        bgr_to_rgb(&mut planar, false, 1, 3);
        assert_eq!(planar, [3, 3, 2, 2, 1, 1]);
    }
}
