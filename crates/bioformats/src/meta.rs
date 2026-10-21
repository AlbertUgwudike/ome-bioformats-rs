//! A subset of the OME data model, enough to carry what readers know about
//! images through a conversion.
//!
//! Bio-Formats routes this through `MetadataStore`/`MetadataRetrieve` and the
//! generated `ome.xml.model` classes; here it is a plain tree of structs that
//! [`crate::ome_xml`] serializes.

use std::collections::BTreeMap;

use crate::core::CoreMetadata;
use crate::dims::Zct;

/// A value with an OME unit symbol, e.g. `1.5 µm` or `0.2 s`.
#[derive(Debug, Clone, PartialEq)]
pub struct Quantity {
    pub value: f64,
    pub unit: String,
}

impl Quantity {
    pub fn new(value: f64, unit: impl Into<String>) -> Self {
        Quantity {
            value,
            unit: unit.into(),
        }
    }

    pub fn micrometers(value: f64) -> Self {
        Self::new(value, "µm")
    }

    pub fn seconds(value: f64) -> Self {
        Self::new(value, "s")
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct OmeMetadata {
    /// One entry per series, in series order.
    pub images: Vec<ImageMetadata>,
    /// Format-specific key/value pairs shared by every series.
    pub global_metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImageMetadata {
    pub name: Option<String>,
    pub description: Option<String>,
    /// ISO 8601 timestamp.
    pub acquisition_date: Option<String>,
    pub physical_size_x: Option<Quantity>,
    pub physical_size_y: Option<Quantity>,
    pub physical_size_z: Option<Quantity>,
    pub time_increment: Option<Quantity>,
    /// One entry per effective channel.
    pub channels: Vec<ChannelMetadata>,
    /// Optional per-plane details; may be empty or sparse.
    pub planes: Vec<PlaneMetadata>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChannelMetadata {
    pub name: Option<String>,
    pub samples_per_pixel: u32,
    /// RGBA packed as a signed 32-bit integer, as in OME-XML.
    pub color: Option<i32>,
    pub excitation_wavelength: Option<Quantity>,
    pub emission_wavelength: Option<Quantity>,
    pub fluor: Option<String>,
}

impl Default for ChannelMetadata {
    fn default() -> Self {
        ChannelMetadata {
            name: None,
            samples_per_pixel: 1,
            color: None,
            excitation_wavelength: None,
            emission_wavelength: None,
            fluor: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlaneMetadata {
    pub zct: Zct,
    pub delta_t: Option<Quantity>,
    pub exposure_time: Option<Quantity>,
    pub position_x: Option<Quantity>,
    pub position_y: Option<Quantity>,
    pub position_z: Option<Quantity>,
}

/// Packs an RGBA colour the way OME-XML stores `Channel/@Color`.
pub fn pack_color(r: u8, g: u8, b: u8, a: u8) -> i32 {
    i32::from_be_bytes([r, g, b, a])
}

impl OmeMetadata {
    /// Makes sure there is an image per series and a channel per effective
    /// channel, so writers can rely on the shape matching the core metadata.
    /// Port of `MetadataTools.populatePixels`' defaulting behaviour.
    pub fn conform_to(&mut self, cores: &[CoreMetadata]) {
        self.images.resize_with(cores.len(), ImageMetadata::default);
        for (image, core) in self.images.iter_mut().zip(cores) {
            let channels = core.effective_size_c() as usize;
            image
                .channels
                .resize_with(channels, ChannelMetadata::default);
            for channel in &mut image.channels {
                channel.samples_per_pixel = core.rgb_channel_count;
            }
        }
    }
}
