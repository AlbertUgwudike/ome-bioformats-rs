use std::path::PathBuf;

/// Errors raised while reading or writing images.
///
/// Mirrors Bio-Formats' split between `FormatException` (the file is not what
/// we expected) and `IOException` (the operating system failed us).
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// The file is malformed or uses a feature this reader does not handle.
    #[error("format error: {0}")]
    Format(String),

    /// The file is valid but uses a feature that has not been ported yet.
    #[error("unsupported: {0}")]
    Unsupported(String),

    #[error("no reader found for {0}")]
    UnknownFormat(PathBuf),

    #[error("no writer found for {0}")]
    UnknownOutputFormat(PathBuf),

    /// A caller passed an out-of-range series, plane or region.
    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    #[error("XML error: {0}")]
    Xml(#[from] roxmltree::Error),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Shorthand for building an [`Error::Format`].
macro_rules! format_err {
    ($($arg:tt)*) => { $crate::error::Error::Format(format!($($arg)*)) };
}
pub(crate) use format_err;

macro_rules! unsupported {
    ($($arg:tt)*) => { $crate::error::Error::Unsupported(format!($($arg)*)) };
}
pub(crate) use unsupported;
