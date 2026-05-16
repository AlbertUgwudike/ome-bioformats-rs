use std::io::{self};

use super::FormatReader;
use crate::common::{ByteOrder, Loc, Metadata, PixelSlice};
use crate::format::tiff::TiffDecoder;
use crate::format::tiff::tiff_decoder::{DecoderContext, StripDesc, TiffRegionDecoder};

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

    pub fn open_pixels_chunky<'a>(
        &'a mut self,
        loc: Loc,
        df: u64,
    ) -> io::Result<ChunkyTiffReader<'a>> {
        let byte_order = self.metadata().byte_order().clone();
        let decoder = self.decoder.region_decoder(loc, df)?;

        Ok(ChunkyTiffReader {
            region_decoder: decoder,
            curr_strip_idx: 0,
            chunks: vec![],
            byte_order,
        })
    }

    // Given a tiff strip and its metadata, this method extracts the relevant bytes
    // e.g. crops each row, downsamples, and extracts desired channel
    fn interpret_strip(strip: &[u8], sd: StripDesc, ctx: &DecoderContext) -> Vec<u8> {
        let mut rows = strip
            .chunks_exact(ctx.bytes_per_row as usize)
            .skip(sd.lower_row)
            .take(sd.upper_row - sd.lower_row)
            .enumerate()
            .filter_map(|(i, r)| {
                if (i + sd.first_row_idx) % ctx.df as usize == 0 {
                    Some(r)
                } else {
                    None
                }
            })
            .map(|row| &row[sd.lower_col..sd.upper_col])
            .map(|row| {
                row.chunks_exact((ctx.bytes_per_sample * ctx.samples_per_pixel) as usize)
                    .step_by(ctx.df as usize)
                    .flatten()
            })
            .flatten()
            .map(|a| a.to_owned())
            .collect::<Vec<u8>>();

        if ctx.is_chunky {
            rows = rows
                .chunks_exact(ctx.bytes_per_sample as usize)
                .skip(ctx.loc.c as usize)
                .step_by(ctx.samples_per_pixel as usize)
                .flatten()
                .map(|a| a.to_owned())
                .collect();
        }

        rows
    }
}

impl FormatReader for TiffReader {
    fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    fn open_bytes(&mut self, loc: Loc, df: u64) -> io::Result<Vec<u8>> {
        let mut out = vec![];

        let mut decoder = self.decoder.region_decoder(loc, df)?;
        let mut buff = vec![0; decoder.nominal_strip_byte_count() as usize];

        // println!("{:?}", decoder.context());

        for strip_idx in decoder.region_strip_idx_iter() {
            let sd = decoder.read_region_strip(strip_idx, &mut buff)?;

            // strip can be skipped
            if sd.is_none() {
                continue;
            }

            let rows = TiffReader::interpret_strip(&buff, sd.unwrap(), decoder.context());

            out.extend_from_slice(&rows)
        }

        Ok(out)
    }
}

pub struct ChunkyTiffReader<'a> {
    region_decoder: TiffRegionDecoder<'a>,
    curr_strip_idx: usize,
    byte_order: ByteOrder,
    chunks: Vec<u8>,
}

impl<'a> ChunkyTiffReader<'a> {
    pub fn step(&mut self) -> io::Result<Option<PixelSlice>> {
        let mut strip = vec![0; self.region_decoder.nominal_strip_byte_count() as usize];
        let idxs = self.region_decoder.region_strip_idx_iter();
        let idx = self.curr_strip_idx as u64 + idxs.start;
        let sd = self.region_decoder.read_region_strip(idx, &mut strip)?;
        let ctx = self.region_decoder.context();

        if let Some(sd) = sd {
            let rows = TiffReader::interpret_strip(&strip, sd, ctx);
            self.chunks.extend_from_slice(&rows);
        }

        self.curr_strip_idx += 1;

        if idx as usize + 1 == idxs.end as usize {
            let bpp = 8 * ctx.bytes_per_pixel as u16;
            let byte_order = self.byte_order.clone();
            let out = PixelSlice::interpret_bytes(bpp, byte_order, &self.chunks)?;
            Ok(Some(out))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use itertools::{Itertools, izip};

    use crate::format_in::PixelSlice;

    use super::*;

    #[test]
    fn open_pixels_normal_tiff() {
        let f_name = "assets/example_valid.tiff".into();
        let mut tr = TiffReader::new(f_name).unwrap();

        let origin = Loc::new(0, 0, 0, 1, 0, 0, 1979, 1979);

        let pxs = tr.open_pixels(origin, 1).unwrap();

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

        let loc = Loc::new(0, 0, 0, 0, 0, 0, 10000, 10000);

        let start = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        let pxs = tr.open_pixels(loc, 1).unwrap();
        let end = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        println!("Duration {:?}", end - start);

        let data = match pxs {
            PixelSlice::U16(v) => v,
            _ => vec![],
        };

        let check_sum = data.into_iter().map(|a| a as u64).sum::<u64>();

        assert_eq!(check_sum, 3343488639);
        // assert_eq!(1, 2)
    }

    #[test]
    fn open_pixels_example_tiff() {
        let f_name = "assets/two.tiff".into();
        let mut tr = TiffReader::new(f_name).unwrap();

        let loc = Loc::new(400, 400, 0, 1, 0, 2, 200, 200);

        let pxs = tr.open_pixels(loc, 1).unwrap();

        let data = match pxs {
            PixelSlice::U8(v) => v,
            _ => vec![],
        };

        println!("{:?}", data.len());

        let check_sum = data.into_iter().map(|a| a as u64).sum::<u64>();

        assert_eq!(check_sum, 200 * 200 * 255);
    }

    #[test]
    fn read_large_region_downsampled() {
        let file_name = "/Users/albert/Downloads/example_ws/ws_converted/24_3_21_7.3_conv.tiff";
        let mut tr = TiffReader::new(file_name.into()).unwrap();
        let md = tr.metadata().clone();
        let d = md.dimensions(0).unwrap();
        let df = 25;

        let mut pxs_vec = vec![];

        if md.series_count() > 1 {
            for s in 0..md.series_count() {
                let loc = Loc::new(0, 0, 0, 0, 0, s as u64, d.h, d.w);
                pxs_vec.push(tr.open_pixels(loc, df).unwrap());
            }
        } else {
            for c in 0..md.dimensions(0).unwrap().c {
                let loc = Loc::new(0, 0, 0, c as u64, 0, 0, d.h, d.w);
                pxs_vec.push(tr.open_pixels(loc, df).unwrap());
            }
        }

        let flat = izip!(
            pxs_vec[0].to_u16vec(),
            pxs_vec[1].to_u16vec(),
            pxs_vec[2].to_u16vec()
        )
        .map(|(a, b, c)| [a, b, c])
        .flatten()
        .chunks(d.h.div_ceil(df) as usize * d.w.div_ceil(df) as usize);

        let mut out: Vec<Vec<u16>> = vec![];

        for arr in &flat {
            out.push(arr.collect());
        }

        println!("Out: {}", out.len())
    }
}
