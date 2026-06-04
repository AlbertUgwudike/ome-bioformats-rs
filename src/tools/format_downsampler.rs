use std::{
    io,
    iter::{Enumerate, Peekable},
};

use crate::{
    common::{Dim, Loc},
    format_in::{FormatReader, Reader, tiff_reader::TiffReader},
    format_out::{FormatWriter, Writer},
    tools::Progress,
};

pub struct FormatDownsampler;

impl FormatDownsampler {
    pub fn downsample(input_file: &String, output_file: &String, df: u64) -> io::Result<()> {
        let mut reader = TiffReader::new(input_file.clone())?;
        println!("Yeetek");

        let mut new_metadata = reader.metadata().clone();
        for series in 0..new_metadata.series_count() {
            let Dim { w, h, d, t, c } = new_metadata.dimensions(series).unwrap();
            new_metadata.set_dimensions(series, h / df, w.div_ceil(df), *d);
        }

        let mut writer = Writer::new(output_file, &new_metadata)?;

        for mut loc in reader.metadata().plane_locs() {
            let pixels = reader.open_pixels(loc, df)?;
            loc.h /= df;
            loc.w = loc.w.div_ceil(df);
            writer.write_pixels(pixels, loc)?;
        }

        Ok(())
    }
}
