//! Writes synthetic images through every writer configuration and reads them
//! back with the TIFF reader.

mod common;

use bioformats::convert::{copy_pixels, output_core};
use bioformats::formats::tiff::TiffWriter;
use bioformats::pixel::interleave;
use bioformats::{Compression, FormatReader, FormatWriter, PixelType, Region, WriterOptions};
use common::{SyntheticReader, core, scratch};

/// Converts `reader` to `name` with `options`, reopens it, and checks every
/// plane matches (after the writer's normalisation to interleaved samples).
fn round_trip(
    reader: &SyntheticReader,
    name: &str,
    options: WriterOptions,
) -> Box<dyn FormatReader> {
    let path = scratch(&format!("rt-{}", name.replace('.', "-"))).join(name);
    let series: Vec<usize> = (0..reader.cores.len()).collect();
    let cores: Vec<_> = reader.cores.iter().map(output_core).collect();
    let mut writer: Box<dyn FormatWriter> =
        Box::new(TiffWriter::create(&path, &cores, &reader.meta, &options).unwrap());
    copy_pixels(reader, writer.as_mut(), &series, &|_| {}).unwrap();
    writer.finish().unwrap();

    let back = bioformats::open(&path).unwrap();
    assert_eq!(
        back.series().len(),
        reader.cores.len(),
        "{name}: series count"
    );
    for (s, orig) in reader.cores.iter().enumerate() {
        let got = &back.series()[s];
        assert_eq!(
            (
                got.size_x,
                got.size_y,
                got.size_z,
                got.size_c,
                got.size_t,
                got.pixel_type,
                got.rgb_channel_count
            ),
            (
                orig.size_x,
                orig.size_y,
                orig.size_z,
                orig.size_c,
                orig.size_t,
                orig.pixel_type,
                orig.rgb_channel_count
            ),
            "{name}: series {s} shape"
        );
        for p in 0..orig.image_count {
            let mut expected = reader.plane(s, p);
            if orig.is_rgb() && !orig.interleaved {
                let mut inter = vec![0; expected.len()];
                interleave(
                    &expected,
                    &mut inter,
                    orig.rgb_channel_count as usize,
                    orig.pixel_type.bytes(),
                );
                expected = inter;
            }
            let actual = back.read_plane(s, p).unwrap();
            assert!(actual == expected, "{name}: series {s} plane {p} differs");
        }
    }
    back
}

#[test]
fn uint8_single_plane_strips() {
    let r = SyntheticReader::new(vec![core(301, 157, PixelType::Uint8, 1, 1, 1, 1, false)]);
    round_trip(&r, "u8.tif", WriterOptions::default());
}

#[test]
fn ome_tiff_multidimensional_uint16() {
    let r = SyntheticReader::new(vec![core(97, 61, PixelType::Uint16, 3, 2, 2, 1, false)]);
    let back = round_trip(&r, "zct.ome.tif", WriterOptions::default());
    assert_eq!(back.format_name(), "OME-TIFF");
    assert_eq!(
        back.metadata().images[0].name.as_deref(),
        Some("synthetic 0")
    );
    assert_eq!(
        back.metadata().images[0]
            .physical_size_x
            .as_ref()
            .map(|q| q.value),
        Some(0.25)
    );
}

#[test]
fn multi_series_ome_tiff() {
    let r = SyntheticReader::new(vec![
        core(64, 48, PixelType::Uint8, 2, 1, 1, 1, false),
        core(33, 17, PixelType::Float, 1, 3, 1, 1, false),
        core(40, 20, PixelType::Uint8, 1, 1, 2, 3, true),
    ]);
    round_trip(&r, "multi.ome.tif", WriterOptions::default());
}

#[test]
fn planar_rgb_is_interleaved_on_write() {
    let r = SyntheticReader::new(vec![core(50, 30, PixelType::Uint16, 1, 1, 1, 3, false)]);
    round_trip(&r, "planar.ome.tif", WriterOptions::default());
}

#[test]
fn every_compression_with_and_without_predictor() {
    let r = SyntheticReader::new(vec![
        core(123, 77, PixelType::Uint16, 2, 1, 1, 1, false),
        core(64, 64, PixelType::Uint8, 1, 1, 1, 3, true),
        core(40, 9, PixelType::Double, 1, 1, 1, 1, false),
    ]);
    for compression in [Compression::Lzw, Compression::Deflate { level: 6 }] {
        for predictor in [false, true] {
            let name = format!("c{}-p{predictor}.ome.tif", compression.tiff_code());
            round_trip(
                &r,
                &name,
                WriterOptions {
                    compression,
                    predictor,
                    ..Default::default()
                },
            );
        }
    }
}

#[test]
fn tiles_with_partial_edges() {
    let r = SyntheticReader::new(vec![core(300, 200, PixelType::Uint16, 1, 2, 1, 1, false)]);
    let options = WriterOptions {
        tile_size: Some((128, 64)),
        compression: Compression::Lzw,
        ..Default::default()
    };
    let back = round_trip(&r, "tiles.ome.tif", options);
    assert_eq!(back.optimal_tile_size(0), (128, 64));
    // A region spanning tile boundaries.
    let region = Region::new(100, 50, 150, 90);
    let mut buf = vec![0; back.series()[0].region_bytes(region.w, region.h)];
    back.open_bytes(0, 1, region, &mut buf).unwrap();
    let full = r.plane(0, 1);
    for y in 0..region.h as usize {
        let src = ((region.y as usize + y) * 300 + region.x as usize) * 2;
        assert_eq!(&buf[y * 300..(y + 1) * 300], &full[src..src + 300]);
    }
}

#[test]
fn forced_big_tiff() {
    let r = SyntheticReader::new(vec![core(70, 70, PixelType::Int16, 1, 1, 3, 1, false)]);
    let options = WriterOptions {
        big_tiff: Some(true),
        compression: Compression::Deflate { level: 1 },
        ..Default::default()
    };
    round_trip(&r, "big.ome.tif", options);
    round_trip(&r, "big-suffix.btf", WriterOptions::default());
}

#[test]
fn plain_tiff_writes_imagej_hyperstack() {
    let mut c = core(32, 32, PixelType::Uint8, 2, 3, 2, 1, false);
    c.dimension_order = bioformats::DimensionOrder::XYCZT;
    let r = SyntheticReader::new(vec![c]);
    let back = round_trip(&r, "hyperstack.tif", WriterOptions::default());
    assert_eq!(
        back.series()[0].dimension_order,
        bioformats::DimensionOrder::XYCZT
    );
}
