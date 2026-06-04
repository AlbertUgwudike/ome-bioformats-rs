pub mod format_converter;
pub mod format_downsampler;

pub use format_converter::FormatConverter;
pub use format_downsampler::FormatDownsampler;

#[derive(PartialEq, Eq)]
pub enum Progress {
    Running(usize, usize),
    Finished,
}
