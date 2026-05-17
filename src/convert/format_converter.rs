use std::{
    io,
    iter::{Enumerate, Peekable},
};

use crate::{
    common::Loc,
    format_in::{FormatReader, lof_reader::LofReader},
    format_out::{FormatWriter, tiff_writer::TiffWriter},
};

pub struct FormatConverter {
    reader: Box<dyn FormatReader>,
    writer: Box<dyn FormatWriter>,
    permutations: Peekable<Enumerate<std::vec::IntoIter<Loc>>>,
    permutation_count: usize,
}

impl FormatConverter {
    pub fn new(input_file: String, output_file: String, chunk_length: usize) -> io::Result<Self> {
        let reader = LofReader::new(input_file)?; // <----- TODO: Make generic
        let metadata = reader.metadata();

        let permutations = metadata
            .permutations(chunk_length)
            .into_iter()
            .enumerate()
            .peekable();

        let permutation_count = permutations.len();

        let writer = TiffWriter::new(output_file.into()) // <----- TODO: Make generic
            .create()
            .set_metadata(metadata.clone())
            .build()?;

        Ok(Self {
            reader: Box::new(reader),
            writer: Box::new(writer),
            permutations,
            permutation_count,
        })
    }

    pub fn step(&mut self) -> io::Result<ConverterProgress> {
        if self.permutations.peek().is_none() {
            return Ok(ConverterProgress::Finished);
        }

        let (i, loc) = self.permutations.next().unwrap();
        let pixels = self.reader.open_pixels(loc.clone(), 1)?;
        self.writer.write_pixels(pixels, loc.clone())?;

        Ok(ConverterProgress::Converting(i, self.permutation_count))
    }
}

#[derive(PartialEq, Eq)]
pub enum ConverterProgress {
    Converting(usize, usize),
    Finished,
}
