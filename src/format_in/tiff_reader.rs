use std::io::{self};

use super::FormatReader;
use crate::common::{Loc, Metadata};
use crate::format::tiff::TiffDecoder;

pub struct TiffReader {
    metadata: Metadata,
    decoder: TiffDecoder,
}

impl TiffReader {
    pub fn new(file: String) -> io::Result<Self> {
        let mut decoder = TiffDecoder::new(file)?;
        let metadata = decoder.metadata()?;
        Ok(Self { metadata, decoder })
    }
}

impl FormatReader for TiffReader {
    fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    fn open_bytes(&mut self, origin: Loc, h: u64, w: u64) -> io::Result<Vec<u8>> {
        let mut out = vec![];

        self.decoder
            .apply_to_roi_strips(origin, h, w, |strip: &mut [u8], sd| {
                let mut rows = strip
                    .chunks_exact(sd.bytes_per_row)
                    .skip(sd.lower_idx)
                    .take(sd.upper_idx - sd.lower_idx)
                    .map(|row| &row[sd.lower_col..sd.upper_col])
                    .flatten()
                    .map(|a| a.to_owned())
                    .collect::<Vec<u8>>();

                if sd.is_chunky {
                    rows = rows
                        .chunks_exact(sd.bytes_per_sample)
                        .skip(origin.c as usize)
                        .step_by(sd.samples_per_pixel)
                        .flatten()
                        .map(|a| a.to_owned())
                        .collect();
                }

                Ok(out.extend_from_slice(&rows))
            })?;

        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fmt::Display,
        time::{SystemTime, UNIX_EPOCH},
    };

    use crate::format_in::PixelSlice;

    use super::*;

    fn print_2d<T: Display>(v: &Vec<T>, h: usize, w: usize) {
        for i in 0..h {
            print!("[");
            for j in 0..w {
                print!(" {:5} ", v[i * w + j]);
            }
            println!("]");
        }
    }

    #[test]
    fn open_pixels_normal_tiff() {
        let f_name = "assets/example_valid.tiff".into();
        let mut tr = TiffReader::new(f_name).unwrap();

        let (x, y, z, c, t, s, h, w) = (0, 0, 0, 1, 0, 0, 1979, 1979);
        let origin = Loc::new(x, y, z, c, t, s);

        let pxs = tr.open_pixels(origin, h, w).unwrap();

        let data = match pxs {
            PixelSlice::U16(v) => v,
            _ => vec![],
        };

        let check_sum = data.into_iter().map(|a| a as u64).sum::<u64>();

        assert_eq!(check_sum, 184163095);
    }

    #[test]
    fn open_pixels_big_tiff() {
        let f_name = "/Users/albert/Downloads/example_ws/ws_converted/24_3_21_7.1_conv.tiff".into();
        let mut tr = TiffReader::new(f_name).unwrap();

        let (x, y, z, c, t, s, h, w) = (0, 0, 0, 0, 0, 0, 10000, 10000);
        let origin = Loc::new(x, y, z, c, t, s);

        let start = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        let pxs = tr.open_pixels(origin, h, w).unwrap();
        let end = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        println!("Duration {:?}", end - start);

        let data = match pxs {
            PixelSlice::U16(v) => v,
            _ => vec![],
        };

        let check_sum = data.into_iter().map(|a| a as u64).sum::<u64>();

        assert_eq!(check_sum, 3343488639);
        assert_eq!(1, 2)
    }

    #[test]
    fn open_pixels_example_tiff() {
        let f_name = "assets/two.tiff".into();
        let mut tr = TiffReader::new(f_name).unwrap();

        let (x, y, z, c, t, s, h, w) = (0, 0, 0, 0, 0, 0, 200, 200);
        let origin = Loc::new(x, y, z, c, t, s);

        let pxs = tr.open_pixels(origin, h, w).unwrap();

        let data = match pxs {
            PixelSlice::U16(v) => v,
            _ => vec![],
        };

        let check_sum = data.into_iter().map(|a| a as u64).sum::<u64>();

        assert_eq!(check_sum, 1);
    }
}
