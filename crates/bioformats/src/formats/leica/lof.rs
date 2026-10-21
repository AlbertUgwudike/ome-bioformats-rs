//! Leica Object Format (`.lof`) reader. Port of `loci.formats.in.LOFReader`.
//!
//! Layout (all integers little-endian, strings UTF-16LE):
//!
//! ```text
//! i32 0x70 | i32 len | u8 0x2A | i32 n | n chars "LMS_Object_File"
//! u8 0x2A | i32 major | u8 0x2A | i32 minor | u8 0x2A | i64 memory size
//! <memory size bytes of pixels>
//! i32 0x70 | i32 len | u8 0x2A | i32 n | n chars of XML
//! ```

use std::path::{Path, PathBuf};

use super::lms::{self, Flavor};
use super::pixels::{ImageData, MemoryBlock};
use crate::core::CoreMetadata;
use crate::error::{Result, format_err};
use crate::io::{Cursor, RandomAccessFile};
use crate::meta::OmeMetadata;
use crate::reader::{FormatReader, ReaderFormat, Region, check_plane_args};

const MAGIC: i32 = 0x70;
const MEMORY_BYTE: u8 = 0x2a;
const TYPE_NAME: &str = "LMS_Object_File";

pub const FORMAT: ReaderFormat = ReaderFormat {
    name: "Leica Object Format",
    suffixes: &["lof"],
    is_this_type: |_, file| is_lof(file),
    open: |path| Ok(Box::new(LofReader::open(path)?)),
};

/// Fields of the LOF header.
struct Header {
    /// Offset of the pixel block.
    data_offset: u64,
    memory_size: u64,
    /// Offset of the XML character count.
    xml_offset: u64,
}

fn expect_byte(cur: &mut Cursor, what: &str) -> Result<()> {
    if cur.read_u8()? != MEMORY_BYTE {
        return Err(format_err!("not a valid Leica LOF file (error at {what})"));
    }
    Ok(())
}

/// Port of `checkForLofLayout`.
fn read_header(file: &RandomAccessFile) -> Result<Header> {
    let mut cur = file.cursor(0, true);
    if cur.read_i32()? != MAGIC {
        return Err(format_err!(
            "not a valid Leica LOF file (error at header section)"
        ));
    }
    cur.read_i32()?;
    expect_byte(&mut cur, "header section")?;
    let n = cur.read_i32()?;
    if !(0..=256).contains(&n) {
        return Err(format_err!(
            "not a valid Leica LOF file (type name length {n})"
        ));
    }
    let type_name = cur.read_utf16(n as usize)?;
    if type_name != TYPE_NAME {
        return Err(format_err!(
            "not a valid Leica LOF file (typename={type_name})"
        ));
    }
    for what in ["major version", "minor version"] {
        expect_byte(&mut cur, what)?;
        cur.read_i32()?;
    }
    expect_byte(&mut cur, "memory size")?;
    let memory_size =
        u64::try_from(cur.read_i64()?).map_err(|_| format_err!("negative LOF memory size"))?;
    let data_offset = cur.pos;
    cur.skip(memory_size);
    if cur.remaining() == 0 {
        return Err(format_err!(
            "not a valid Leica LOF file (xml section not found)"
        ));
    }
    if cur.read_i32()? != MAGIC {
        return Err(format_err!(
            "not a valid Leica LOF file (error at xml section)"
        ));
    }
    cur.read_i32()?;
    expect_byte(&mut cur, "xml section")?;
    Ok(Header {
        data_offset,
        memory_size,
        xml_offset: cur.pos,
    })
}

fn read_xml(file: &RandomAccessFile, header: &Header) -> Result<String> {
    let mut cur = file.cursor(header.xml_offset, true);
    let n = cur.read_i32()?;
    if n < 0 || n as u64 * 2 > cur.remaining() {
        return Err(format_err!("LOF XML length {n} is out of range"));
    }
    cur.read_utf16(n as usize)
}

/// Port of `LofXmlDocument.getImageNode`: `/Image`, `/Element/Image` or
/// `/Element/Data/Image` below the root.
fn image_node<'a, 'i>(doc: &'a roxmltree::Document<'i>) -> Option<roxmltree::Node<'a, 'i>> {
    let child =
        |n: roxmltree::Node<'a, 'i>, name: &str| n.children().find(|c| c.has_tag_name(name));
    let root = doc.root_element();
    child(root, "Image").or_else(|| {
        let element = child(root, "Element")?;
        child(element, "Image").or_else(|| child(child(element, "Data")?, "Image"))
    })
}

/// Port of `LOFReader.isThisType`.
fn is_lof(file: &RandomAccessFile) -> bool {
    let Ok(header) = read_header(file) else {
        return false;
    };
    let Ok(xml) = read_xml(file, &header) else {
        return false;
    };
    roxmltree::Document::parse(&xml).is_ok_and(|doc| image_node(&doc).is_some())
}

pub struct LofReader {
    path: PathBuf,
    file: RandomAccessFile,
    data: ImageData,
    /// One per tile; tiles share the image's core metadata.
    cores: Vec<CoreMetadata>,
    meta: OmeMetadata,
}

impl LofReader {
    pub fn open(path: &Path) -> Result<Self> {
        let file = RandomAccessFile::open(path)?;
        let header = read_header(&file)?;
        let xml = read_xml(&file, &header)?;
        let doc = roxmltree::Document::parse(&xml)?;
        let node =
            image_node(&doc).ok_or_else(|| format_err!("this LOF does not contain image data"))?;
        // As in Bio-Formats, the image is named after the file.
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let name = file_name.strip_suffix(".lof").unwrap_or(&file_name);
        let image = lms::translate_image(node, name, Flavor::Lms)?;

        let tiles = image.tile_count as usize;
        let cores = vec![image.core.clone(); tiles];
        // As in Bio-Formats, tiles share the image's name.
        let mut meta = OmeMetadata {
            images: vec![image.image.clone(); tiles],
            ..Default::default()
        };
        meta.conform_to(&cores);
        let block = MemoryBlock {
            offset: header.data_offset,
            len: header.memory_size,
        };
        Ok(LofReader {
            path: path.to_owned(),
            file,
            data: ImageData::new(image, Some(block)),
            cores,
            meta,
        })
    }
}

impl FormatReader for LofReader {
    fn format_name(&self) -> &'static str {
        FORMAT.name
    }

    fn series(&self) -> &[CoreMetadata] {
        &self.cores
    }

    fn metadata(&self) -> &OmeMetadata {
        &self.meta
    }

    fn open_bytes(&self, series: usize, plane: u32, region: Region, buf: &mut [u8]) -> Result<()> {
        check_plane_args(&self.cores, series, plane, region, buf.len())?;
        self.data
            .read(&self.file, series as u32, plane, region, buf)
    }

    fn used_files(&self) -> Vec<PathBuf> {
        vec![self.path.clone()]
    }
}
