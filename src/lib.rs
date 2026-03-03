pub mod common;
pub mod format;
pub mod format_in;
pub mod format_out;

#[cfg(test)]
mod tests {
    use crate::{
        common::Loc,
        format_in::{FormatReader, tiff_reader::TiffReader},
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
                let origin = Loc::new(0, r, 0, chn, 0, 0);
                let pixels = reader.open_pixels(origin, 1, dims.w).unwrap();
                writer.write_pixels(pixels, origin, 1, dims.w).unwrap();
            }
        }

        let mut reader_output = TiffReader::new(output_file.into()).unwrap();

        for chn in 0..dims.d {
            for r in 0..dims.h {
                let origin = Loc::new(0, r, 0, chn, 0, 0);
                let pixels_input = reader.open_pixels(origin, 1, dims.w).unwrap();
                let pixels_output = reader_output.open_pixels(origin, 1, dims.w).unwrap();

                assert_eq!(pixels_input, pixels_output)
            }
        }

        // clean up
        std::fs::remove_file(output_file).unwrap();
    }
}
