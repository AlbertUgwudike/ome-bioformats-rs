//! Carl Zeiss formats. Port of `loci.formats.in.ZeissCZIReader`.

mod czi;
mod czi_meta;

pub use czi::{CziReader, FORMAT as CZI_FORMAT};
