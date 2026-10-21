//! TIFF, BigTIFF and OME-TIFF support. Port of `loci.formats.tiff`,
//! `loci.formats.in.{MinimalTiffReader,TiffReader,OMETiffReader}` and
//! `loci.formats.out.{TiffWriter,OMETiffWriter}`.

mod ifd;
mod reader;
mod writer;

pub use ifd::{Ifd, TagValue, TiffFile};
pub use reader::{FORMAT as READER_FORMAT, TiffReader};
pub use writer::TiffWriter;

/// Tag numbers used by this crate.
pub mod tag {
    pub const NEW_SUBFILE_TYPE: u16 = 254;
    pub const IMAGE_WIDTH: u16 = 256;
    pub const IMAGE_LENGTH: u16 = 257;
    pub const BITS_PER_SAMPLE: u16 = 258;
    pub const COMPRESSION: u16 = 259;
    pub const PHOTOMETRIC_INTERPRETATION: u16 = 262;
    pub const IMAGE_DESCRIPTION: u16 = 270;
    pub const STRIP_OFFSETS: u16 = 273;
    pub const SAMPLES_PER_PIXEL: u16 = 277;
    pub const ROWS_PER_STRIP: u16 = 278;
    pub const STRIP_BYTE_COUNTS: u16 = 279;
    pub const X_RESOLUTION: u16 = 282;
    pub const Y_RESOLUTION: u16 = 283;
    pub const PLANAR_CONFIGURATION: u16 = 284;
    pub const RESOLUTION_UNIT: u16 = 296;
    pub const SOFTWARE: u16 = 305;
    pub const DATE_TIME: u16 = 306;
    pub const PREDICTOR: u16 = 317;
    pub const COLOR_MAP: u16 = 320;
    pub const TILE_WIDTH: u16 = 322;
    pub const TILE_LENGTH: u16 = 323;
    pub const TILE_OFFSETS: u16 = 324;
    pub const TILE_BYTE_COUNTS: u16 = 325;
    pub const SUB_IFDS: u16 = 330;
    pub const EXTRA_SAMPLES: u16 = 338;
    pub const SAMPLE_FORMAT: u16 = 339;
}

/// TIFF field types.
pub mod field {
    pub const BYTE: u16 = 1;
    pub const ASCII: u16 = 2;
    pub const SHORT: u16 = 3;
    pub const LONG: u16 = 4;
    pub const RATIONAL: u16 = 5;
    pub const SBYTE: u16 = 6;
    pub const UNDEFINED: u16 = 7;
    pub const SSHORT: u16 = 8;
    pub const SLONG: u16 = 9;
    pub const SRATIONAL: u16 = 10;
    pub const FLOAT: u16 = 11;
    pub const DOUBLE: u16 = 12;
    pub const IFD: u16 = 13;
    pub const LONG8: u16 = 16;
    pub const SLONG8: u16 = 17;
    pub const IFD8: u16 = 18;

    /// Size in bytes of one value of `ty`, or `None` for unknown types.
    pub const fn size(ty: u16) -> Option<usize> {
        Some(match ty {
            BYTE | ASCII | SBYTE | UNDEFINED => 1,
            SHORT | SSHORT => 2,
            LONG | SLONG | FLOAT | IFD => 4,
            RATIONAL | SRATIONAL | DOUBLE | LONG8 | SLONG8 | IFD8 => 8,
            _ => return None,
        })
    }
}

/// True if a filename should be written as OME-TIFF rather than plain TIFF.
pub fn is_ome_tiff_name(path: &std::path::Path) -> bool {
    crate::reader::has_suffix(
        path,
        &["ome.tif", "ome.tiff", "ome.tf2", "ome.tf8", "ome.btf"],
    )
}
