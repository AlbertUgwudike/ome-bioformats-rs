# Porting roadmap

Ordered by value for conversion work: formats people actually convert from,
then the features that make outputs better. Each format lands with synthetic
fixtures and a `tools/bf-check` comparison against Java Bio-Formats.

## Adding a format

1. Read the Java reader in `reference/bioformats/components/formats-{gpl,bsd}/src/loci/formats/in/`.
   (`git clone --depth 1 https://github.com/ome/bioformats reference/bioformats`
   if the reference checkout is missing.)
2. Implement `FormatReader` in `crates/bioformats/src/formats/<family>/`, with
   a `ReaderFormat` constant (`is_this_type` + `open`), and add it to
   `formats::READERS` in the same relative order as Bio-Formats' `readers.txt`.
3. Keep readers `Sync`: no seek cursors; read with `RandomAccessFile::read_exact_at`.
4. Add tests with a synthetic generator (see `tests/common/mod.rs` for the LOF
   one), then compare real files against Java with `tools/bf-check/check.sh`.

## Phase 1: the Leica / Zeiss pipeline

- [x] LOF reader
- [x] LIF reader. Shares metadata and pixel access with LOF
      (`formats/leica/{lms,pixels}.rs`) but keeps `LIFReader`'s own
      dimension rules (`lms::Flavor::Lif`).
- [ ] Test against real LIF files: so far LIF is verified only on synthetic
      projects. Sample files are at https://downloads.openmicroscopy.org/images/Leica-LIF/
- [ ] XLIF/XLEF/XLCF (LAS X "project" XML pointing at LOF/TIFF files;
      `XlifDocument` etc.). Mostly path resolution on top of LOF.
- [ ] **Zeiss CZI reader** (`ZeissCZIReader`, 4.8k lines): ZISRAW segments,
      subblock directory, JPEG-XR and zstd subblocks, mosaics and pyramids.
      Needs the zstd and JPEG-XR codecs below.
- [ ] Leica instrument metadata (lasers, detectors, filters, objectives) and
      ROIs: the rest of `LMSMetadataExtractor` and `MetadataStoreInitializer`.
      Needs the instrument part of the OME model.

## Phase 2: output quality

- [ ] **Pyramidal OME-TIFF** (`PyramidOMETiffWriter`): SubIFDs plus a parallel
      downsampler in the chunk pipeline. This matters for 10k×17k images like
      the sample LOF, which viewers struggle to open without a pyramid.
- [ ] Reading pyramids: `SubIFDs` in TIFF and `IPyramidHandler`-style resolution API.
- [ ] Codecs: JPEG (read via `zune-jpeg`; write), zstd, JPEG 2000 (`openjpeg`),
      JPEG-XR (CZI). Floating-point predictor (3).
- [ ] OME-Zarr / NGFF writer, the natural target for cloud pipelines (not in
      Bio-Formats core; bioformats2raw does this).
- [ ] Pipelined writing: overlap the sequential `write_chunk` of batch N with
      encoding batch N+1 (currently they alternate).

## Phase 3: breadth (most-used readers)

Approximate size of the Java readers:

| Reader | Lines | Notes |
|---|---|---|
| Nikon ND2 (`ND2Reader` + `ND2Handler`) | ~3.8k | chunk map, JPEG 2000 variants |
| Olympus OIR / FV1000 / cellSens VSI | 1.7k / 1.8k / 2.8k | VSI pairs with ETS tiles |
| Zeiss LSM / ZVI | 2.6k / 0.75k | LSM is TIFF-based, reuses `formats/tiff` |
| Aperio SVS, Hamamatsu NDPI | 0.8k / 1k | TIFF-based pyramids |
| Imaris HDF (`.ims`) | 0.75k | needs HDF5 |
| MetaMorph STK/ND | 2.4k | TIFF-based |
| ICS, DICOM, NIfTI, FITS, PNG/JPEG/BMP, AVI | small | simple containers |

## Phase 4: completeness

- [ ] The full OME data model (instrument, experiment, ROI, plate/screen),
      ideally generated from the XSD as Bio-Formats does.
- [ ] Remaining writers: OME-XML, ICS, JPEG, PNG, AVI, DICOM.
- [ ] `ReaderWrapper` equivalents: `ChannelSeparator`, `DimensionSwapper`, `Memoizer`.
- [ ] Python bindings (PyO3) exposing the reader API as NumPy arrays.

## Known deviations from Bio-Formats

- Original metadata is written as an OME `MapAnnotation` (namespace
  `openmicroscopy.org/OriginalMetadata`), not Bio-Formats' `XMLAnnotation`.
  Bio-Formats keeps it but does not show it as original metadata.
- Leica readers take row padding from the XML `BytesInc` stride. Bio-Formats
  infers it from block offsets: for LOF the "next offset" is the file length
  (which includes the XML trailer), so padding is never found; for LIF the
  spare bytes are divided per row without accounting for the number of
  planes or tiles, so only single-plane, untiled, last-in-file images read
  correctly (others are shifted, or throw past end of file).
- LIF memory blocks are matched to images by `MemoryBlockID`. Bio-Formats
  matches by position (when the counts agree), so it reads the wrong pixels
  if blocks are not stored in XML order. Positional matching is the fallback
  when no IDs match.
- Channel colours from `Gradient(...)` LUTs are decoded for LIF too;
  Bio-Formats' LIF reader shows them as white.
- Plain TIFF inputs with IFDs of different shapes are split into one series
  per run of same-shaped IFDs.
