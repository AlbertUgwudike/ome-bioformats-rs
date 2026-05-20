use std::{
    io,
    iter::{Enumerate, Peekable},
    path::Path,
};

use crate::{
    common::{Loc, Metadata},
    format_in::{FormatReader, lof_reader::LofReader, tiff_reader::TiffReader},
    format_out::{FormatWriter, tiff_writer::TiffWriter},
};

pub enum Reader {
    Tiff(TiffReader),
    Lof(LofReader),
}

impl Reader {
    pub fn new(input_file: &String) -> io::Result<Self> {
        let ext_err = io::Error::other("No Extension");
        let uni_err = io::Error::other("Invalid Unicode");

        let ext = Path::new(input_file)
            .extension()
            .ok_or(ext_err)?
            .to_str()
            .ok_or(uni_err)?;

        match ext {
            "tiff" | "tif" => {
                let reader = TiffReader::new(input_file.clone())?;
                Ok(Reader::Tiff(reader))
            }
            "lof" => {
                let reader = LofReader::new(input_file.clone())?;
                Ok(Reader::Lof(reader))
            }
            other => Err(io::Error::other(format!(
                "Unsupported Reader Format: {}",
                other
            ))),
        }
    }
}

impl FormatReader for Reader {
    fn metadata(&self) -> &Metadata {
        match self {
            Reader::Tiff(reader) => reader.metadata(),
            Reader::Lof(reader) => reader.metadata(),
        }
    }

    fn open_bytes(&mut self, loc: Loc, df: u64) -> io::Result<Vec<u8>> {
        match self {
            Reader::Tiff(reader) => reader.open_bytes(loc, df),
            Reader::Lof(reader) => reader.open_bytes(loc, df),
        }
    }
}

pub enum Writer {
    Tiff(TiffWriter),
}

impl Writer {
    pub fn new(output_file: &String, metadata: &Metadata) -> io::Result<Self> {
        let ext_err = io::Error::other("No Extension");
        let uni_err = io::Error::other("Invalid Unicode");

        let ext = Path::new(output_file)
            .extension()
            .ok_or(ext_err)?
            .to_str()
            .ok_or(uni_err)?;

        match ext {
            "tiff" | "tif" => {
                let writer = TiffWriter::new(output_file.clone())
                    .create()
                    .set_metadata(metadata.clone())
                    .big_tiff(true)
                    .build()?;

                Ok(Writer::Tiff(writer))
            }
            other => Err(io::Error::other(format!(
                "Unsupported Reader Format: {}",
                other
            ))),
        }
    }
}

impl FormatWriter for Writer {
    fn write_bytes(&mut self, bytes: Vec<u8>, loc: Loc) -> io::Result<()> {
        match self {
            Writer::Tiff(writer) => writer.write_bytes(bytes, loc),
        }
    }
}

pub struct FormatConverter {
    reader: Reader,
    writer: Writer,
    permutations: Peekable<Enumerate<std::vec::IntoIter<Loc>>>,
    permutation_count: usize,
}

impl FormatConverter {
    pub fn new(input_file: String, output_file: String, chunk_length: usize) -> io::Result<Self> {
        let reader = Reader::new(&input_file)?;
        let metadata = reader.metadata();
        let writer = Writer::new(&output_file, metadata)?;
        println!("Metadata: {:?}", metadata);

        let permutations = metadata
            .permutations(chunk_length)
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

    pub fn step(&mut self) -> io::Result<ConverterProgress> {
        if self.permutations.peek().is_none() {
            return Ok(ConverterProgress::Finished);
        }

        let (i, loc) = self.permutations.next().unwrap();
        println!("LOC: {:?}", loc);
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
