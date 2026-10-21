//! Per-series core metadata. Port of `loci.formats.CoreMetadata`.

use std::collections::BTreeMap;

use crate::dims::{DimensionOrder, Zct};
use crate::error::Result;
use crate::pixel::PixelType;

/// The minimum a reader must know about a series to hand out its pixels.
///
/// As in Bio-Formats, `size_c` counts every channel *including* the samples
/// of RGB pixels, while planes are indexed by the *effective* channel count
/// (`size_c / rgb_channel_count`).
#[derive(Debug, Clone, PartialEq)]
pub struct CoreMetadata {
    pub size_x: u32,
    pub size_y: u32,
    pub size_z: u32,
    pub size_c: u32,
    pub size_t: u32,
    pub pixel_type: PixelType,
    /// Number of significant bits per sample (e.g. 12 for 12-bit data in uint16).
    pub bits_per_pixel: u32,
    pub image_count: u32,
    pub dimension_order: DimensionOrder,
    /// Samples stored together in one plane (3 for RGB, 1 otherwise).
    pub rgb_channel_count: u32,
    /// Whether multi-sample pixels are stored `RGBRGB...` (true) or `RRR...GGG...` (false).
    pub interleaved: bool,
    pub little_endian: bool,
    pub indexed: bool,
    pub false_color: bool,
    pub order_certain: bool,
    pub metadata_complete: bool,
    /// Format-specific key/value pairs ("original metadata") for this series.
    pub series_metadata: BTreeMap<String, String>,
}

impl CoreMetadata {
    /// A single-plane, single-channel series with sensible defaults.
    pub fn new(size_x: u32, size_y: u32, pixel_type: PixelType) -> Self {
        CoreMetadata {
            size_x,
            size_y,
            size_z: 1,
            size_c: 1,
            size_t: 1,
            pixel_type,
            bits_per_pixel: pixel_type.bytes() as u32 * 8,
            image_count: 1,
            dimension_order: DimensionOrder::XYCZT,
            rgb_channel_count: 1,
            interleaved: false,
            little_endian: true,
            indexed: false,
            false_color: false,
            order_certain: true,
            metadata_complete: true,
            series_metadata: BTreeMap::new(),
        }
    }

    pub fn is_rgb(&self) -> bool {
        self.rgb_channel_count > 1
    }

    pub fn effective_size_c(&self) -> u32 {
        self.size_c / self.rgb_channel_count.max(1)
    }

    /// `(sizeZ, effectiveSizeC, sizeT)`, the shape used for plane indexing.
    pub fn zct_sizes(&self) -> (u32, u32, u32) {
        (self.size_z, self.effective_size_c(), self.size_t)
    }

    pub fn plane_index(&self, zct: Zct) -> Result<u32> {
        self.dimension_order.index(self.zct_sizes(), zct)
    }

    pub fn zct(&self, plane: u32) -> Result<Zct> {
        self.dimension_order.zct(self.zct_sizes(), plane)
    }

    /// Bytes in one pixel of a plane, counting all RGB samples.
    pub fn bytes_per_pixel(&self) -> usize {
        self.pixel_type.bytes() * self.rgb_channel_count as usize
    }

    /// Bytes needed to hold a `w` x `h` region of one plane.
    pub fn region_bytes(&self, w: u32, h: u32) -> usize {
        w as usize * h as usize * self.bytes_per_pixel()
    }

    pub fn plane_bytes(&self) -> usize {
        self.region_bytes(self.size_x, self.size_y)
    }

    /// Fills in `image_count` from the dimension sizes.
    pub fn update_image_count(&mut self) {
        self.image_count = self.size_z * self.effective_size_c() * self.size_t;
    }
}
