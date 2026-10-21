//! The writer interface. Port of `loci.formats.IFormatWriter`, reshaped so
//! that encoding (CPU-bound) can run in parallel while file output stays
//! sequential.
//!
//! A plane is written as a sequence of chunks (strips or tiles) chosen by the
//! writer. For each chunk the caller hands raw pixels to [`FormatWriter::encode_chunk`]
//! (which takes `&self` and may run on any thread), then passes the result to
//! [`FormatWriter::write_chunk`] in order.

use crate::codec::Compression;
use crate::error::Result;
use crate::reader::Region;

/// Settings shared by all writers. Writers ignore options they cannot honour.
#[derive(Debug, Clone, PartialEq)]
pub struct WriterOptions {
    pub compression: Compression,
    /// Write tiles of this size instead of strips. Must be multiples of 16 for TIFF.
    pub tile_size: Option<(u32, u32)>,
    /// `Some(true)` forces BigTIFF, `Some(false)` forbids it, `None` picks
    /// BigTIFF when the uncompressed data would not fit in a classic TIFF.
    pub big_tiff: Option<bool>,
    /// Apply horizontal differencing before compression (integer types only).
    pub predictor: bool,
    /// Embed format-specific "original metadata" as OME annotations.
    pub original_metadata: bool,
}

impl Default for WriterOptions {
    fn default() -> Self {
        WriterOptions {
            compression: Compression::None,
            tile_size: None,
            big_tiff: None,
            predictor: false,
            original_metadata: true,
        }
    }
}

pub trait FormatWriter: Send + Sync {
    /// The chunks that make up one plane of `series`, in the order they must
    /// be written. Chunks never extend past the plane.
    fn plane_chunks(&self, series: usize) -> Vec<Region>;

    /// Encodes one chunk. `data` holds `region` with samples interleaved
    /// (`RGBRGB...`), little-endian, rows tightly packed.
    fn encode_chunk(&self, series: usize, region: Region, data: Vec<u8>) -> Result<Vec<u8>>;

    /// Appends an encoded chunk. Every chunk of a plane must be written, in
    /// [`plane_chunks`](Self::plane_chunks) order, before the next plane starts.
    fn write_chunk(
        &mut self,
        series: usize,
        plane: u32,
        chunk: usize,
        encoded: &[u8],
    ) -> Result<()>;

    /// Writes trailing metadata and closes the file.
    fn finish(self: Box<Self>) -> Result<()>;
}
