use std::collections::HashMap;
use std::io::{self, Error};

use crate::format_in::{Dim, Loc, Metadata};

use super::FormatReader;
use super::tiff::TiffParser;

pub struct TiffReader {
    parser: TiffParser,
}

impl TiffReader {
    pub fn new(file: String) -> io::Result<Self> {
        Ok(Self {
            parser: TiffParser::new(file)?,
        })
    }
}

impl FormatReader for TiffReader {
    fn metadata(&mut self) -> io::Result<Metadata> {
        let mut bpp = HashMap::new();
        let mut dim = HashMap::new();

        let be = self.parser.byte_order();
        let ifd_count = self.parser.n_ifds()?;

        for i in 0..ifd_count {
            let ifd = self.parser.nth_ifd(i)?;
            let w = self.parser.image_width(&ifd)? as i32;
            let h = self.parser.image_length(&ifd)? as i32;
            let c = self.parser.samples_per_pixel(&ifd)? as i32;

            dim.insert(i, Dim::from_whc(w, h, c));

            let bpps = self.parser.bits_per_sample(&ifd)?;

            for (j, v) in bpps.iter().enumerate() {
                bpp.insert((j as i32, i), *v);
            }
        }

        Ok(Metadata {
            dimensions: dim,
            bits_per_pixel: bpp,
            byte_order: be,
        })
    }

    fn open_bytes(&mut self, origin: Loc, h: i32, w: i32) -> io::Result<Vec<u8>> {
        let Loc { x, y, z, c, t, s } = origin;

        let ifd = self.parser.nth_ifd(s)?;
        let iw = self.parser.image_width(&ifd)?;
        let bits_per_sample = self.parser.bits_per_sample(&ifd)?;
        let samples_per_pixel = bits_per_sample.len();
        let bytes_per_sample = (bits_per_sample[c as usize] / 8) as usize;
        let is_chunky = self.parser.planar_configuration(&ifd)? == 1;
        let rows_per_strip = self.parser.rows_per_strip(&ifd)?;

        let bytes_per_pixel = if is_chunky {
            // Chunky configuration, 'c' samples per pixel
            bits_per_sample.into_iter().map(|a| a as i32).sum::<i32>() / 8
        } else {
            // Planar configuration, one sample per pixel
            *bits_per_sample
                .get(c as usize)
                .ok_or(Error::other("Invalid c"))? as i32
                / 8
        };

        let start_idx = y / rows_per_strip as i32;
        let end_idx = (y + h) / rows_per_strip as i32;

        let mut out = Vec::with_capacity((h * w * bytes_per_pixel) as usize);

        for strip_idx in start_idx..end_idx + 1 {
            // Calculate start/end indexes into image rows
            let s_idx = (strip_idx * rows_per_strip as i32) as usize;
            let e_idx = ((strip_idx + 1) * rows_per_strip as i32) as usize;

            // Calculate start/end indices into a vector of strip rows
            let lower_idx = std::cmp::max(s_idx, y as usize) - s_idx;
            let upper_idx = std::cmp::min(e_idx, (y + h) as usize) - s_idx;

            // chunk and change
            let bytes_per_row = bytes_per_pixel * iw as i32;
            let lower_col = (bytes_per_pixel * x) as usize;
            let upper_col = lower_col + (bytes_per_pixel * w) as usize;

            // println!("{:?} {:?} {:?} ", bytes_per_row, lower_col, upper_col);

            let strip = self.parser.read_strip(&ifd, strip_idx, bytes_per_pixel)?;

            let rows = strip
                .chunks_exact(bytes_per_row as usize)
                .skip(lower_idx)
                .take(upper_idx - lower_idx)
                .map(|row| &row[lower_col..upper_col])
                .flatten()
                .map(|a| a.to_owned())
                .collect::<Vec<u8>>();

            let bytes: Vec<u8> = if is_chunky {
                rows.chunks_exact(bytes_per_sample)
                    .skip(c as usize)
                    .step_by(samples_per_pixel)
                    .flatten()
                    .map(|a| a.to_owned())
                    .collect()
            } else {
                rows.chunks_exact(bytes_per_sample)
                    .skip((c * h * w) as usize)
                    .take((h * w) as usize)
                    .flatten()
                    .map(|a| a.to_owned())
                    .collect()
            };

            out.extend_from_slice(&bytes);
        }

        Ok(out)
    }
}

mod tests {
    use std::{
        fmt::{Debug, Display},
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
    fn test_open_pixels() {
        let mut tr = TiffReader::new("assets/example_valid.tiff".into()).unwrap();
        let origin = Loc::new(0, 10, 0, 1, 0, 0);
        let (h, w) = (30, 15);

        let start = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        let pxs = tr.open_pixels(origin, h, w).unwrap();
        let end = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();

        println!("{:?}", end - start);

        let data = match pxs {
            PixelSlice::U16(v) => v,
            _ => vec![],
        };

        println!("Length = {:?}", data.len());

        print_2d(&data, h as usize, w as usize);

        assert_eq!(data.len(), (h * w) as usize);
        assert!(1 == 2);
    }
}
