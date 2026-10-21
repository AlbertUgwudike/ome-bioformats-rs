//! Reads synthetic Leica LOF files and checks every plane against the raw
//! pixel block it was built from.

mod common;

use bioformats::convert::{ConvertOptions, convert};
use bioformats::{DimensionOrder, PixelType};
use common::{ImageSpec, expected_plane, scratch};

fn check(name: &str, spec: ImageSpec) {
    let dir = scratch(&format!("lof-{name}"));
    let path = dir.join(format!("{name}.lof"));
    let data = spec.write_lof(&path);
    let reader = bioformats::open(&path).unwrap();
    assert_eq!(reader.format_name(), "Leica Object Format");
    assert_eq!(
        reader.series().len(),
        spec.tiles as usize,
        "{name}: one series per tile"
    );
    for tile in 0..spec.tiles {
        let core = &reader.series()[tile as usize];
        let channels = spec.luts.len() as u32 / if spec.rgb { 3 } else { 1 };
        assert_eq!(
            (core.size_x, core.size_y, core.size_z),
            (spec.width, spec.height, spec.z),
            "{name}"
        );
        assert_eq!(core.effective_size_c(), channels, "{name}");
        assert_eq!(core.is_rgb(), spec.rgb, "{name}");
        assert_eq!(
            core.pixel_type,
            if spec.bytes == 1 {
                PixelType::Uint8
            } else {
                PixelType::Uint16
            }
        );
        assert_eq!(core.dimension_order, DimensionOrder::XYCZT, "{name}");
        for p in 0..core.image_count {
            let zct = core.zct(p).unwrap();
            let bgr = spec.rgb && spec.luts[..3] != ["Red", "Green", "Blue"];
            let expected = expected_plane(&spec, &data, tile, zct.z, zct.c, bgr);
            assert!(
                reader.read_plane(tile as usize, p).unwrap() == expected,
                "{name}: tile {tile} plane {p} differs"
            );
        }
    }
    let image = &reader.metadata().images[0];
    assert!(
        (image.physical_size_x.as_ref().unwrap().value - 0.5).abs() < 1e-9,
        "{name}: physical size"
    );
    assert!(image.acquisition_date.is_some(), "{name}: timestamp");

    // And the whole thing survives conversion to OME-TIFF.
    let out = dir.join(format!("{name}.ome.tif"));
    convert(&path, &out, &ConvertOptions::default(), &|_| {}).unwrap();
    let back = bioformats::open(&out).unwrap();
    for s in 0..reader.series().len() {
        for p in 0..reader.series()[s].image_count {
            assert!(
                back.read_plane(s, p).unwrap() == reader.read_plane(s, p).unwrap(),
                "{name}: converted s{s} p{p}"
            );
        }
    }
}

fn spec(luts: &[&'static str]) -> ImageSpec {
    ImageSpec::simple(luts)
}

#[test]
fn planar_three_channels() {
    check("planar", spec(&["Red", "Green", "Blue"]));
}

#[test]
fn rgb_in_rgb_order() {
    check(
        "rgb",
        ImageSpec {
            rgb: true,
            ..spec(&["Red", "Green", "Blue"])
        },
    );
}

#[test]
fn rgb_without_luts_is_bgr() {
    check(
        "bgr",
        ImageSpec {
            rgb: true,
            ..spec(&["", "", ""])
        },
    );
}

#[test]
fn sixteen_bit_z_stack() {
    check(
        "z16",
        ImageSpec {
            bytes: 2,
            z: 4,
            ..spec(&["Green", "Magenta"])
        },
    );
}

#[test]
fn padded_rows() {
    check(
        "padded",
        ImageSpec {
            width: 63,
            row_padding: 1,
            ..spec(&["Gray"])
        },
    );
}

#[test]
fn mosaic_tiles_become_series() {
    check(
        "tiles",
        ImageSpec {
            tiles: 3,
            z: 2,
            ..spec(&["Red", "Cyan"])
        },
    );
}

#[test]
fn rejects_non_lof() {
    let dir = scratch("lof-reject");
    let path = dir.join("not.lof");
    std::fs::write(&path, b"definitely not a LOF file").unwrap();
    assert!(bioformats::open(&path).is_err());
}
