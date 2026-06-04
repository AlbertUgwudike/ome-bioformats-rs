use std::{
    io,
    iter::{Enumerate, Peekable},
};

use crate::{
    common::Loc,
    format_in::{FormatReader, Reader},
    format_out::{FormatWriter, Writer},
    tools::Progress,
};

pub struct FormatConverter {
    reader: Reader,
    writer: Writer,
    permutations: Peekable<Enumerate<std::vec::IntoIter<Loc>>>,
    permutation_count: usize,
}

impl FormatConverter {
    pub fn new(input_file: &String, output_file: &String, chunk_length: usize) -> io::Result<Self> {
        let reader = Reader::new(input_file)?;
        let metadata = reader.metadata();
        let writer = Writer::new(output_file, metadata)?;

        println!("{:?}", metadata);

        let permutations = metadata
            .locs(chunk_length)
            .into_iter()
            .enumerate()
            .peekable();

        let permutation_count = permutations.len();

        Ok(Self {
            reader,
            writer,
            permutations,
            permutation_count,
        })
    }

    pub fn step(&mut self) -> io::Result<Progress> {
        if self.permutations.peek().is_none() {
            return Ok(Progress::Finished);
        }

        let (i, loc) = self.permutations.next().unwrap();
        let pixels = self.reader.open_pixels(loc.clone(), 1)?;
        self.writer.write_pixels(pixels, loc.clone())?;

        Ok(Progress::Running(i, self.permutation_count))
    }
}

#[cfg(test)]
mod tests {
    use crate::tools::{FormatConverter, Progress};

    #[test]
    fn duplicate_small_wierd_tiff() {
        // Noticed issue with rote copy of single series tiff with ?two channels
        // When duplicated using format converter, no image data
        // I think its due to assumptions made by tiff writer, ?need more metadata ...
        // Plot twist: I was wrong, bug was channel being read as depth

        let input_file = "assets/example.tiff".to_string();
        let output_file = "assets/example_out.tiff".to_string();

        let mut converter = FormatConverter::new(&input_file, &output_file, 100).unwrap();
        while let Progress::Running(_, _) = converter.step().unwrap() {}

        // clean up
        std::fs::remove_file(output_file).unwrap();
    }

    #[test]
    fn duplicate_multi_series_rgb() {
        // Noticed issue with rote copy of single series tiff with ?two channels
        // When duplicated using format converter, no image data
        // I think its due to assumptions made by tiff writer, ?need more metadata ...
        // Plot twist: I was wrong, bug was channel being read as depth

        // let input_file = "/Users/albert/Downloads/astro_dab.lof".to_string();
        let input_file =
            "/Users/albert/Downloads/Microcount example images-selected/24_3_21_7.1.lof"
                .to_string();
        // let output_file = "assets/astro_dab.tiff".to_string();
        let output_file = "assets/24_3_21_7.1.tiff".to_string();

        let mut converter = FormatConverter::new(&input_file, &output_file, 100).unwrap();
        while let Progress::Running(_, _) = converter.step().unwrap() {}

        // clean up
        std::fs::remove_file(output_file).unwrap();

        assert!(false);
    }
}
