//! Reads synthetic Leica LIF projects and checks every plane of every image
//! against the raw memory blocks they were built from.

mod common;

use bioformats::convert::{ConvertOptions, convert};
use bioformats::{DimensionOrder, PixelType};
use common::{ImageSpec, LifSpec, expected_plane, scratch};

/// Writes `lif`, checks the reader's view of it against the specs, and that
/// it survives conversion to OME-TIFF. `names` are the expected series names
/// of each pixel image's first tile.
fn check(name: &str, lif: LifSpec, names: &[&str]) {
    let dir = scratch(&format!("lif-{name}"));
    let path = dir.join(format!("{name}.lif"));
    let datas = lif.write(&path);
    let specs: Vec<&ImageSpec> = lif.images.iter().filter_map(|(_, _, s)| *s).collect();
    let reader = bioformats::open(&path).unwrap();
    assert_eq!(reader.format_name(), "Leica Image File Format");
    let total_tiles: u32 = specs.iter().map(|s| s.tiles).sum();
    assert_eq!(
        reader.series().len(),
        total_tiles as usize,
        "{name}: series count"
    );

    let mut series = 0;
    for (i, (spec, data)) in specs.iter().zip(&datas).enumerate() {
        let first_name = reader.metadata().images[series].name.clone().unwrap();
        assert_eq!(first_name, names[i], "{name}: image {i} name");
        for tile in 0..spec.tiles {
            let core = &reader.series()[series];
            assert_eq!(
                (core.size_x, core.size_y, core.size_z),
                (spec.width, spec.height, spec.z),
                "{name} s{series}"
            );
            assert_eq!(core.effective_size_c(), spec.channels(), "{name} s{series}");
            assert_eq!(core.is_rgb(), spec.rgb, "{name} s{series}");
            let ty = if spec.bytes == 1 {
                PixelType::Uint8
            } else {
                PixelType::Uint16
            };
            assert_eq!(core.pixel_type, ty, "{name} s{series}");
            assert!(
                matches!(
                    core.dimension_order,
                    DimensionOrder::XYCZT | DimensionOrder::XYZCT
                ),
                "{name} s{series}: {}",
                core.dimension_order
            );
            for p in 0..core.image_count {
                let zct = core.zct(p).unwrap();
                // LIF stores RGB as BGR unconditionally.
                let expected = expected_plane(spec, data, tile, zct.z, zct.c, spec.rgb);
                assert!(
                    reader.read_plane(series, p).unwrap() == expected,
                    "{name}: series {series} plane {p} differs"
                );
            }
            series += 1;
        }
    }

    let out = dir.join(format!("{name}.ome.tif"));
    convert(&path, &out, &ConvertOptions::default(), &|_| {}).unwrap();
    let back = bioformats::open(&out).unwrap();
    assert_eq!(back.series().len(), reader.series().len());
    for s in 0..reader.series().len() {
        assert_eq!(
            back.metadata().images[s].name,
            reader.metadata().images[s].name
        );
        for p in 0..reader.series()[s].image_count {
            assert!(
                back.read_plane(s, p).unwrap() == reader.read_plane(s, p).unwrap(),
                "{name}: converted s{s} p{p}"
            );
        }
    }
}

fn z16() -> ImageSpec {
    ImageSpec {
        bytes: 2,
        z: 3,
        seed: 1,
        ..ImageSpec::simple(&["Green", "Magenta"])
    }
}

fn rgb() -> ImageSpec {
    ImageSpec {
        rgb: true,
        seed: 2,
        ..ImageSpec::simple(&["Red", "Green", "Blue"])
    }
}

fn padded_tiles() -> ImageSpec {
    ImageSpec {
        width: 63,
        row_padding: 1,
        tiles: 2,
        seed: 3,
        ..ImageSpec::simple(&["Gray"])
    }
}

#[test]
fn project_with_folders_v2() {
    let (a, b, c) = (z16(), rgb(), padded_tiles());
    let lif = LifSpec {
        images: vec![
            ("", "Series001", Some(&a)),
            ("", "Folder", None),
            ("/Folder", "Inner", Some(&b)),
            ("", "Series003", Some(&c)),
        ],
        version: 2,
        reverse_blocks: false,
        zero_tail: false,
        processing_history: false,
    };
    check("project", lif, &["Series001", "Folder/Inner", "Series003"]);
}

#[test]
fn version_1_blocks_with_zero_tail() {
    let (a, b) = (z16(), rgb());
    let lif = LifSpec {
        images: vec![("", "A", Some(&a)), ("", "B", Some(&b))],
        version: 1,
        reverse_blocks: false,
        zero_tail: true,
        processing_history: false,
    };
    check("v1", lif, &["A", "B"]);
}

#[test]
fn blocks_are_matched_by_id_not_position() {
    let (a, b) = (z16(), rgb());
    let lif = LifSpec {
        images: vec![("", "A", Some(&a)), ("", "B", Some(&b))],
        version: 2,
        reverse_blocks: true,
        zero_tail: false,
        processing_history: false,
    };
    check("reversed", lif, &["A", "B"]);
}

#[test]
fn processing_history_images_are_skipped() {
    let a = z16();
    let lif = LifSpec {
        images: vec![("", "A", Some(&a))],
        version: 2,
        reverse_blocks: false,
        zero_tail: false,
        processing_history: true,
    };
    check("history", lif, &["A"]);
}

#[test]
fn padded_rows() {
    let a = ImageSpec {
        width: 63,
        row_padding: 1,
        z: 2,
        seed: 5,
        ..ImageSpec::simple(&["Gray"])
    };
    let b = ImageSpec {
        width: 63,
        row_padding: 1,
        seed: 6,
        ..ImageSpec::simple(&["Gray"])
    };
    let lif = LifSpec {
        images: vec![("", "Padded", Some(&a)), ("", "Single", Some(&b))],
        version: 2,
        reverse_blocks: false,
        zero_tail: false,
        processing_history: false,
    };
    check("padded", lif, &["Padded", "Single"]);
}

#[test]
fn xz_line_scan() {
    let a = ImageSpec {
        xz: true,
        seed: 4,
        ..ImageSpec::simple(&["Red", "Green"])
    };
    let lif = LifSpec {
        images: vec![("", "XZ", Some(&a))],
        version: 2,
        reverse_blocks: false,
        zero_tail: false,
        processing_history: false,
    };
    check("xz", lif, &["XZ"]);
}

#[test]
fn lof_is_not_mistaken_for_lif() {
    let dir = scratch("lif-not-lof");
    let path = dir.join("x.lif");
    ImageSpec::simple(&["Gray"]).write_lof(&path);
    // Content wins over the misleading suffix.
    assert_eq!(
        bioformats::open(&path).unwrap().format_name(),
        "Leica Object Format"
    );
}
