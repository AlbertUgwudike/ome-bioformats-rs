use std::io;

use crate::tools::{FormatConverter, Progress};

pub mod common;
pub mod format;
pub mod format_in;
pub mod format_out;
pub mod tools;

pub fn convert(input_file: String, output_file: String) -> io::Result<()> {
    let mut converter = FormatConverter::new(&input_file, &output_file, 100)?;

    let mut progress = converter.step()?;
    while progress != Progress::Finished {
        if let Progress::Running(num, den) = progress {
            let perc = 100.0 * num as f64 / den as f64;
            println!("Progress: {:?}%", perc)
        }
        progress = converter.step()?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::{
        common::Loc,
        convert,
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
        let input_file = "/Users/albert/Downloads/olig_ws/ws_converted/Iba1_Arg1_2_conv.tiff";
        // let input_file = "/Users/albert/Downloads/Saline3 brain 2.lof";
        let output_file = "assets/dab.tiff";

        convert(input_file.into(), output_file.into()).unwrap();

        assert!(1 == 2)
        // clean up
        // std::fs::remove_file(output_file).unwrap();
    }
}
