//! PNG and animated PNG. Port of `loci.formats.in.APNGReader` and
//! `loci.formats.out.APNGWriter`.

mod reader;
mod writer;

pub use reader::{FORMAT as READER_FORMAT, PngReader};
pub use writer::PngWriter;
