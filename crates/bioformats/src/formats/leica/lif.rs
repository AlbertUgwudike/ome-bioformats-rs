//! Leica Image File (`.lif`) reader. Port of `loci.formats.in.LIFReader`.
//!
//! Layout (all integers little-endian, strings UTF-16LE):
//!
//! ```text
//! i32 0x70 | i32 len | u8 0x2A | i32 n | n chars of XML
//! then memory blocks until EOF (or a run of zeros in newer files):
//! i32 0x70 | i32 len | u8 0x2A | (i32 | i64) size | u8 0x2A | i32 n | n chars ID
//! <size bytes of pixels>
//! ```
//!
//! Version 1 files store the block size as i32, version 2 as i64; Bio-Formats
//! tells them apart by where the next 0x2A appears. Each `Image` element in
//! the XML names its block through a sibling `Memory/@MemoryBlockID`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::lms::{self, Flavor};
use super::pixels::{ImageData, MemoryBlock};
use crate::core::CoreMetadata;
use crate::error::{Result, format_err};
use crate::io::RandomAccessFile;
use crate::meta::OmeMetadata;
use crate::reader::{FormatReader, ReaderFormat, Region, check_plane_args};

const MAGIC: i32 = 0x70;
const MEMORY_BYTE: u8 = 0x2a;

pub const FORMAT: ReaderFormat = ReaderFormat {
    name: "Leica Image File Format",
    suffixes: &["lif"],
    is_this_type: |_, file| is_lif(file),
    open: |path| Ok(Box::new(LifReader::open(path)?)),
};

/// Port of `LIFReader.isThisType`, plus a check that the description is XML
/// (Bio-Formats accepts anything that is not a LOF).
fn is_lif(file: &RandomAccessFile) -> bool {
    let check = || -> Result<bool> {
        let mut cur = file.cursor(0, true);
        if cur.read_u8()? != MAGIC as u8 {
            return Ok(false);
        }
        cur.skip(7);
        if cur.read_u8()? != MEMORY_BYTE {
            return Ok(false);
        }
        let n = cur.read_i32()?;
        if n <= 0 {
            return Ok(false);
        }
        let head = cur.read_utf16((n as usize).min(32))?;
        Ok(head != "LMS_Object_File"
            && head
                .trim_start_matches('\u{feff}')
                .trim_start()
                .starts_with('<'))
    };
    check().unwrap_or(false)
}

#[derive(Debug)]
struct Block {
    id: String,
    block: MemoryBlock,
}

/// Reads the XML description and the memory block index. Port of the first
/// half of `LIFReader.initFile`.
fn read_container(file: &RandomAccessFile) -> Result<(String, Vec<Block>)> {
    let mut cur = file.cursor(0, true);
    let first = cur.read_u8()?;
    cur.skip(2);
    if first != MAGIC as u8 && cur.read_u8()? != MAGIC as u8 {
        return Err(format_err!("not a valid Leica LIF file"));
    }
    cur.pos = 8;
    if cur.read_u8()? != MEMORY_BYTE {
        return Err(format_err!("invalid LIF XML description"));
    }
    let n = cur.read_i32()?;
    if n < 0 || n as u64 * 2 > cur.remaining() {
        return Err(format_err!("LIF XML length {n} is out of range"));
    }
    let xml = cur.read_utf16(n as usize)?;

    let mut blocks = Vec::new();
    while cur.remaining() > 0 {
        let at = cur.pos;
        let check = if cur.remaining() >= 4 {
            cur.read_i32()?
        } else {
            0
        };
        if check != MAGIC {
            if check == 0 && !blocks.is_empty() {
                // Newer files pad the remainder with zeros.
                break;
            }
            return Err(format_err!(
                "invalid LIF memory block at {at}: magic {check:#x}"
            ));
        }
        cur.skip(4);
        if cur.read_u8()? != MEMORY_BYTE {
            return Err(format_err!("invalid LIF memory description at {at}"));
        }
        let size_at = cur.pos;
        let mut len = cur.read_i32()? as i64;
        if cur.read_u8()? != MEMORY_BYTE {
            // Version 2: the size is 64-bit.
            cur.pos = size_at;
            len = cur.read_i64()?;
            if cur.read_u8()? != MEMORY_BYTE {
                return Err(format_err!("invalid LIF memory description at {at}"));
            }
        }
        let id_len = cur.read_i32()?;
        if id_len < 0 || id_len as u64 * 2 > cur.remaining() {
            return Err(format_err!("invalid LIF memory block ID length at {at}"));
        }
        let id = cur
            .read_utf16(id_len as usize)?
            .trim_matches('\0')
            .to_owned();
        let len = u64::try_from(len).map_err(|_| format_err!("negative LIF block size at {at}"))?;
        if len > 0 {
            blocks.push(Block {
                id,
                block: MemoryBlock {
                    offset: cur.pos,
                    len,
                },
            });
        }
        cur.skip(len);
    }
    Ok((xml, blocks))
}

/// Removes characters XML 1.0 forbids, as `XMLTools.sanitizeXML` does.
fn sanitize(xml: &str) -> String {
    xml.chars()
        .filter(|&c| !((c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r')) && c != '\u{feff}')
        .collect()
}

/// Port of `translateImageNames`: the names of enclosing `Element`s, outermost
/// first, without the outermost one (the project itself).
fn image_name(image: roxmltree::Node) -> String {
    let mut names: Vec<&str> = image
        .ancestors()
        .filter(|n| n.has_tag_name("Element"))
        .map(|n| n.attribute("Name").unwrap_or(""))
        .collect();
    names.pop();
    names.reverse();
    names.join("/")
}

/// Images worth exposing, with their memory block IDs. Port of the image
/// selection in `LIFReader.translateMetadata`.
fn image_nodes<'a, 'i>(
    doc: &'a roxmltree::Document<'i>,
) -> Vec<(roxmltree::Node<'a, 'i>, Option<&'a str>)> {
    doc.descendants()
        .filter(|n| n.has_tag_name("Image"))
        // Bio-Formats prunes sequential-acquisition templates first.
        .filter(|n| {
            !n.ancestors()
                .any(|a| a.has_tag_name("LDM_Block_Sequential_Master"))
        })
        .filter_map(|image| {
            let grandparent = image.parent_element()?.parent_element()?;
            if grandparent.has_tag_name("ProcessingHistory") {
                return None;
            }
            let id = grandparent
                .children()
                .rfind(|c| c.has_tag_name("Memory"))
                .and_then(|m| m.attribute("MemoryBlockID"));
            Some((image, id))
        })
        .collect()
}

pub struct LifReader {
    path: PathBuf,
    file: RandomAccessFile,
    images: Vec<ImageData>,
    /// `(image index, tile)` for each series.
    series_map: Vec<(usize, u32)>,
    cores: Vec<CoreMetadata>,
    meta: OmeMetadata,
}

impl LifReader {
    pub fn open(path: &Path) -> Result<Self> {
        let file = RandomAccessFile::open(path)?;
        let (xml, blocks) = read_container(&file)?;
        // LIF descriptions may lack a single root element.
        let xml = format!("<LEICA>{}</LEICA>", sanitize(&xml));
        let doc = roxmltree::Document::parse_with_options(
            &xml,
            roxmltree::ParsingOptions {
                allow_dtd: true,
                ..Default::default()
            },
        )?;
        let nodes = image_nodes(&doc);
        if nodes.is_empty() {
            return Err(format_err!("no images found; this LIF file is not valid"));
        }

        // Prefer matching blocks by ID. Bio-Formats matches by position
        // (with a size-based fix-up); fall back to that if no IDs match.
        let by_id: HashMap<&str, MemoryBlock> =
            blocks.iter().map(|b| (b.id.as_str(), b.block)).collect();
        let any_id = nodes
            .iter()
            .any(|(_, id)| id.is_some_and(|id| by_id.contains_key(id)));

        let mut images = Vec::new();
        let mut series_map = Vec::new();
        let mut cores = Vec::new();
        let mut meta = OmeMetadata::default();
        for (i, (node, id)) in nodes.iter().enumerate() {
            let name = image_name(*node);
            let image = lms::translate_image(*node, &name, Flavor::Lif)?;
            let block = if any_id {
                id.and_then(|id| by_id.get(id).copied())
            } else {
                blocks.get(i).map(|b| b.block)
            };
            for tile in 0..image.tile_count {
                series_map.push((images.len(), tile));
                let mut core = image.core.clone();
                core.series_metadata
                    .insert("Image name".into(), name.clone());
                cores.push(core);
                // As in Bio-Formats, tiles share the image's name.
                meta.images.push(image.image.clone());
            }
            images.push(ImageData::new(image, block));
        }
        meta.conform_to(&cores);
        Ok(LifReader {
            path: path.to_owned(),
            file,
            images,
            series_map,
            cores,
            meta,
        })
    }
}

impl FormatReader for LifReader {
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
        let (image, tile) = self.series_map[series];
        self.images[image].read(&self.file, tile, plane, region, buf)
    }

    fn used_files(&self) -> Vec<PathBuf> {
        vec![self.path.clone()]
    }
}
