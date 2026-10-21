# ome-bioformats-rs

A Rust port of [OME Bio-Formats](https://github.com/ome/bioformats), aimed at
fast, low-memory conversion between microscopy image formats.

Bio-Formats is ~245k lines of Java with ~190 format readers. This port is
starting with the pipeline that matters first, **Leica LOF/LIF → OME-TIFF**,
and is built so the rest can be added one format at a time. See
[ROADMAP.md](ROADMAP.md).

## Status

| Component | State |
|---|---|
| Core model (`CoreMetadata`, pixel types, dimension orders, plane indexing) | ported |
| OME-XML 2016-06 write/parse (images, pixels, channels, planes, TiffData, annotations) | subset ported |
| **Leica LOF reader** (RGB/BGR, 8/16-bit, Z/T, mosaic tiles → series, padded rows, timestamps, LUT colours) | ported |
| **Leica LIF reader** (multi-image projects with folders, v1/v2 block headers, XZ/XT line scans, plus everything the LOF reader handles) | ported |
| **TIFF / BigTIFF / OME-TIFF reader** (strips, tiles, planar/chunky, LZW, zlib, PackBits, predictor, multi-file OME-TIFF, ImageJ hyperstacks) | ported |
| **TIFF / BigTIFF / OME-TIFF writer** (strips or tiles, LZW, zlib, predictor, auto BigTIFF, physical size → resolution tags) | ported |
| `bfconvert`, `showinf` command-line tools | ported (core options) |
| Every other format, pyramids, JPEG/JPEG 2000 codecs, instrument metadata | not yet; see roadmap |

## Usage

```bash
cargo build --release
```

```bash
./target/release/bfconvert input.lof output.ome.tif
```

```bash
./target/release/bfconvert --compression zlib --predictor --tile-size 512x512 input.lof output.ome.tif
```

```bash
./target/release/showinf input.lof --omexml
```

Output format is chosen by suffix: `.ome.tif`/`.ome.tiff` write OME-TIFF,
`.tif` writes plain TIFF with an ImageJ hyperstack description, and
`.btf`/`.tf2`/`.tf8` force BigTIFF. Files whose pixels exceed ~3.9 GiB switch
to BigTIFF automatically.

As a library:

```rust
let reader = bioformats::open("image.lof")?;
let core = &reader.series()[0];
let plane = reader.read_plane(0, 0)?;            // whole plane
let mut tile = vec![0; core.region_bytes(256, 256)];
reader.open_bytes(0, 0, bioformats::Region::new(0, 0, 256, 256), &mut tile)?;
```

## Design

Differences from the Java implementation, chosen for conversion throughput:

- **Readers are stateless and `Sync`.** There is no "current series"; every
  call names the series. Files are read with positional I/O (`pread`), so many
  threads can read tiles from one open reader at once.
- **Writers split encoding from output.** `encode_chunk(&self)` (compression,
  predictor, tile padding) runs in parallel; `write_chunk(&mut self)` appends
  bytes in order. The TIFF writer streams pixel data and writes all IFDs plus the
  OME-XML at the end, so it never holds a plane in memory.
- **Conversion streams by chunk, not by plane.** `convert::copy_pixels` reads,
  normalises and compresses batches of strips/tiles across all cores. Memory is
  bounded by the batch size (~128 MiB), whatever the plane size.

Layout (Bio-Formats equivalents in brackets):

```
crates/bioformats/src/
  reader.rs, writer.rs   format traits            [IFormatReader, IFormatWriter]
  core.rs, dims.rs       series shape, ZCT index  [CoreMetadata, FormatTools]
  meta.rs, ome_xml.rs    OME model and XML        [ome.xml.model, OMEXMLService]
  io.rs                  positional file reads    [RandomAccessInputStream]
  codec/                 LZW, zlib, PackBits, predictor [loci.formats.codec]
  formats/leica/         LOF, LIF + shared LMS XML [LOFReader, LIFReader, LeicaMicrosystemsMetadata]
  formats/tiff/          TIFF/OME-TIFF in and out [TiffParser, TiffSaver, *TiffReader, *TiffWriter]
  formats/mod.rs         format detection         [ImageReader, readers.txt]
  convert.rs             conversion engine        [ImageConverter]
crates/bfconvert/        bfconvert + showinf CLIs
tools/bf-check/          Java Bio-Formats cross-checker
```

## Verification

`cargo test` runs unit tests plus integration tests that generate synthetic
LOF files (RGB, BGR, 16-bit, Z-stacks, mosaic tiles, padded rows), synthetic
LIF projects (nested folders, v1/v2 block headers, zero-padded tails,
out-of-order blocks, ProcessingHistory duplicates, XZ line scans,
FrameProperties blocks), and
round-trip synthetic images through every writer option.

Outputs are also checked against the Java reference with
`tools/bf-check/check.sh`, which reads a file with Bio-Formats and prints
per-plane MD5s:

```bash
BF_JAR=path/to/bioformats_package.jar tools/bf-check/check.sh output.ome.tif validate
```

Results so far:

- A real 11200×17664×3 LOF (593 MB) converts to OME-TIFF with pixels
  identical to the raw LOF data. Java Bio-Formats reads the result with
  `OMETiffReader` and its schema validation accepts the OME-XML.
- Every synthetic LOF reads identically in Java and Rust, with one exception:
  rows padded to 4-byte alignment. Java's `LOFReader` infers padding from the
  file length, which includes the XML trailer, so it misses it; this port
  uses the `BytesInc` stride from the XML.
- Every synthetic LIF reads identically in Java and Rust (dimensions, names
  including folder paths, and per-plane MD5s), except three cases where Java
  is wrong (see "Known deviations" in ROADMAP.md): multi-plane images with
  padded rows, padded mosaics, and memory blocks stored out of XML order.
  For the padded case Java's formula does handle (one plane, last in the
  file), the two agree.
- A real LAS X project (`Intro.lif`, 605 MB, 57 series including mosaics,
  Z-stacks and merged regions): every one of its 1,129 planes matches the
  bytes of the memory block its XML references. Java Bio-Formats 7.3 gets
  only 666 right. LAS X stores small non-image `FrameProperties` blocks
  between image blocks, and Java matches blocks to images by position, so
  from the second image on it reads its neighbour's pixels. Dimensions,
  names, physical sizes and dates agree for all 57 series. The conversion
  takes 0.5 s and 61 MB of RAM (Java: 6 s and 416 MB just to read it).
- Every compression/tiling/BigTIFF/multi-series variant the writer produces
  decodes in Java to the same MD5s as uncompressed ground truth.

Converting the 593 MB LOF on an M4 Pro, warm cache:

| Tool | Time | Peak RSS | Output |
|---|---|---|---|
| Java Bio-Formats 7.3 (`TiffWriter`, uncompressed) | 6.2–8.7 s | 560–580 MB | 594 MB |
| `bfconvert` uncompressed | 0.75 s | 177 MB | 594 MB |
| `bfconvert --compression zlib` | 1.0 s | 203 MB | 380 MB |
| `bfconvert --compression zlib --predictor` | 1.15 s | 203 MB | 352 MB |
| `bfconvert --compression lzw` | 1.0 s | 197 MB | 436 MB |

## License

GPL-2.0-or-later. The Leica readers are ports of code from Bio-Formats'
GPL-licensed `formats-gpl` component, so this crate follows that license.
Bio-Formats' TIFF code is BSD-2-Clause.
