use crate::{common::Loc, format_in::FormatReader, format_out::FormatWriter};

pub mod common;
pub mod format;
pub mod format_in;
pub mod format_out;

pub fn convert(mut reader: impl FormatReader, mut writer: impl FormatWriter) {
    let metadata = reader.metadata().clone();
    let series_count = metadata.series_count();

    for series in 0..series_count {
        let dims = metadata.dimensions(series).unwrap().clone();

        for chn in 0..dims.d {
            let mut f_write = |sr, rc| {
                let loc = Loc::new(0, sr, 0, chn, 0, series as u64, rc, dims.w);
                let pixels = reader.open_pixels(loc, 1).unwrap();
                writer.write_pixels(pixels, loc).unwrap();
            };

            for r in 0..dims.h / 100 {
                f_write(r * 100, 100);
            }

            f_write(dims.h - dims.h % 100, dims.h % 100);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        common::Loc,
        convert,
        format_in::{FormatReader, lof_reader::LofReader, tiff_reader::TiffReader},
        format_out::{FormatWriter, tiff_writer::TiffWriter},
    };

    #[test]
    fn duplicate_tiff() {
        let input_file = "assets/example_valid.tiff";
        let output_file = "assets/example_valid_copy.tiff";

        let mut reader = TiffReader::new(input_file.into()).unwrap();
        let metadata = reader.metadata();

        let mut writer = TiffWriter::new(output_file.into())
            .create()
            .set_metadata(metadata.clone())
            .build()
            .unwrap();

        let dims = metadata.dimensions(0).unwrap().clone();

        for chn in 0..dims.d {
            for r in 0..dims.h {
                let loc = Loc::new(0, r, 0, chn, 0, 0, 1, dims.w);
                let pixels = reader.open_pixels(loc, 1).unwrap();
                writer.write_pixels(pixels, loc).unwrap();
            }
        }

        let mut reader_output = TiffReader::new(output_file.into()).unwrap();

        for chn in 0..dims.d {
            for r in 0..dims.h {
                let origin = Loc::new(0, r, 0, chn, 0, 0, 1, dims.w);
                let pixels_input = reader.open_pixels(origin, 1).unwrap();
                let pixels_output = reader_output.open_pixels(origin, 1).unwrap();

                assert_eq!(pixels_input, pixels_output)
            }
        }

        // clean up
        std::fs::remove_file(output_file).unwrap();
    }

    #[test]
    fn lof_to_tiff() {
        // let input_file = "/Users/albert/Downloads/DAB test brain 1.lof";
        let input_file = "/Users/albert/Downloads/Saline3 brain 2.lof";
        let output_file = "assets/dab.tiff";

        let reader = LofReader::new(input_file.into()).unwrap();
        let writer = TiffWriter::new(output_file.into())
            .create()
            .set_metadata(reader.metadata().clone())
            .build()
            .unwrap();

        convert(reader, writer);

        // clean up
        std::fs::remove_file(output_file).unwrap();
    }
}
