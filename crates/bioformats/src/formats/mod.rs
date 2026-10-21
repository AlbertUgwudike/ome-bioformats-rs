//! Format implementations and the registry that picks one for a file.
//! Port of `loci.formats.ImageReader` / `ImageWriter` and `readers.txt`.

pub mod leica;
pub mod tiff;

use std::path::Path;

use crate::core::CoreMetadata;
use crate::error::{Error, Result};
use crate::io::RandomAccessFile;
use crate::meta::OmeMetadata;
use crate::reader::{FormatReader, ReaderFormat, has_suffix};
use crate::writer::{FormatWriter, WriterOptions};

/// Readable formats, in the order they are tried (most specific first, as
/// in Bio-Formats' `readers.txt`).
pub const READERS: &[ReaderFormat] = &[leica::LIF_FORMAT, leica::LOF_FORMAT, tiff::READER_FORMAT];

/// Suffixes the TIFF writer handles.
const TIFF_SUFFIXES: &[&str] = &["tif", "tiff", "tf2", "tf8", "btf"];

/// Finds the format of `path` by content, preferring formats whose suffix matches.
pub fn detect(path: &Path) -> Result<&'static ReaderFormat> {
    let file = RandomAccessFile::open(path)?;
    let by_suffix = READERS.iter().filter(|f| has_suffix(path, f.suffixes));
    let others = READERS.iter().filter(|f| !has_suffix(path, f.suffixes));
    by_suffix
        .chain(others)
        .find(|f| (f.is_this_type)(path, &file))
        .ok_or_else(|| Error::UnknownFormat(path.to_owned()))
}

/// Opens `path` with whichever reader recognises it.
pub fn open(path: impl AsRef<Path>) -> Result<Box<dyn FormatReader>> {
    let path = path.as_ref();
    (detect(path)?.open)(path)
}

/// Creates a writer for `path`, chosen by its suffix.
pub fn create_writer(
    path: &Path,
    cores: &[CoreMetadata],
    meta: &OmeMetadata,
    options: &WriterOptions,
) -> Result<Box<dyn FormatWriter>> {
    if has_suffix(path, TIFF_SUFFIXES) {
        return Ok(Box::new(tiff::TiffWriter::create(
            path, cores, meta, options,
        )?));
    }
    Err(Error::UnknownOutputFormat(path.to_owned()))
}
