//! A Rust port of [OME Bio-Formats](https://github.com/ome/bioformats).
//!
//! ```no_run
//! let reader = bioformats::open("image.lof")?;
//! let core = &reader.series()[0];
//! println!("{}x{} {:?}, {} planes", core.size_x, core.size_y, core.pixel_type, core.image_count);
//! let plane0 = reader.read_plane(0, 0)?;
//! # Ok::<(), bioformats::Error>(())
//! ```
//!
//! Module map (Bio-Formats equivalents in brackets):
//! - [`reader`] / [`writer`]: the format traits [`IFormatReader`, `IFormatWriter`]
//! - [`core`]: per-series dimensions and layout [`CoreMetadata`]
//! - [`meta`], [`ome_xml`]: OME metadata model and XML [`ome.xml.model`, `OMEXMLService`]
//! - [`formats`]: readers and writers [`loci.formats.in`, `loci.formats.out`]
//! - [`codec`]: compression [`loci.formats.codec`]
//! - [`convert`]: the conversion engine [`ImageConverter` / `bfconvert`]

pub mod codec;
pub mod convert;
pub mod core;
pub mod dims;
pub mod error;
pub mod formats;
pub mod io;
pub mod meta;
pub mod ome_xml;
pub mod pixel;
pub mod reader;
pub mod writer;

pub use crate::codec::Compression;
pub use crate::core::CoreMetadata;
pub use crate::dims::{DimensionOrder, Zct};
pub use crate::error::{Error, Result};
pub use crate::formats::{create_writer, open};
pub use crate::meta::OmeMetadata;
pub use crate::pixel::PixelType;
pub use crate::reader::{FormatReader, Region};
pub use crate::writer::{FormatWriter, WriterOptions};
