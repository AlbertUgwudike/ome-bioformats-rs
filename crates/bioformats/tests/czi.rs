//! Reads synthetic Zeiss CZI files: compression, Z/C stacks, scenes, and
//! mosaics with and without pyramids.

mod common;

use bioformats::convert::{ConvertOptions, convert};
use bioformats::{PixelType, Region};
use common::{pattern, scratch};

/// One subblock to write.
struct Block {
    pixel_type: i32,
    compression: i32,
    pyramid_type: u8,
    /// `(letter, start, logical size, stored size)`; X and Y first.
    dims: Vec<(char, i32, i32, i32)>,
    data: Vec<u8>,
}

/// Bytes per pixel for the CZI pixel types used here.
fn bpp(pixel_type: i32) -> usize {
    match pixel_type {
        0 => 1,
        1 => 2,
        3 => 3,
        _ => unreachable!(),
    }
}

fn block(
    pixel_type: i32,
    x: (i32, i32, i32),
    y: (i32, i32, i32),
    other: &[(char, i32)],
    seed: usize,
) -> Block {
    let n = x.2 as usize * y.2 as usize * bpp(pixel_type);
    let mut dims = vec![('X', x.0, x.1, x.2), ('Y', y.0, y.1, y.2)];
    dims.extend(other.iter().map(|&(c, s)| (c, s, 1, 1)));
    Block {
        pixel_type,
        compression: 0,
        pyramid_type: if x.1 != x.2 { 2 } else { 0 },
        dims,
        data: (0..n).map(|i| pattern(seed, 3, i)).collect(),
    }
}

fn segment(out: &mut Vec<u8>, id: &str, data: &[u8]) -> u64 {
    while !out.len().is_multiple_of(32) {
        out.push(0);
    }
    let at = out.len() as u64;
    let mut name = id.as_bytes().to_vec();
    name.resize(16, 0);
    out.extend(name);
    out.extend((data.len() as i64).to_le_bytes());
    out.extend((data.len() as i64).to_le_bytes());
    out.extend(data);
    at
}

fn entry(b: &Block, segment_pos: u64) -> Vec<u8> {
    let mut e = b"DV".to_vec();
    e.extend(b.pixel_type.to_le_bytes());
    e.extend((segment_pos as i64).to_le_bytes());
    e.extend(0i32.to_le_bytes()); // file part
    e.extend(b.compression.to_le_bytes());
    e.push(b.pyramid_type);
    e.extend([0; 5]);
    e.extend((b.dims.len() as i32).to_le_bytes());
    for &(c, start, size, stored) in &b.dims {
        let mut name = [0u8; 4];
        name[0] = c as u8;
        e.extend(name);
        e.extend(start.to_le_bytes());
        e.extend(size.to_le_bytes());
        e.extend(0f32.to_le_bytes());
        e.extend(stored.to_le_bytes());
    }
    e
}

/// Encodes `data` with CZI compression `c` (0, 5 = zstd, 6 = zstd with
/// high/low byte splitting).
fn compress(c: i32, data: &[u8]) -> Vec<u8> {
    match c {
        0 => data.to_vec(),
        5 => zstd::bulk::compress(data, 3).unwrap(),
        6 => {
            let half = data.len() / 2;
            let mut split = vec![0; data.len()];
            for i in 0..data.len() {
                if i % 2 == 0 {
                    split[i / 2] = data[i];
                } else {
                    split[half + i / 2] = data[i];
                }
            }
            // Header: size 3, chunk 1 (hi/lo packing) = 1.
            let mut out = vec![3, 1, 1];
            out.extend(zstd::bulk::compress(&split, 3).unwrap());
            out
        }
        _ => unreachable!(),
    }
}

fn write_czi(path: &std::path::Path, blocks: &[Block], xml: &str) {
    let mut out = Vec::new();
    segment(&mut out, "ZISRAWFILE", &[0; 512]);
    let mut entries = Vec::new();
    for b in blocks {
        let payload = compress(b.compression, &b.data);
        let placeholder = entry(b, 0);
        let mut data = 0i32.to_le_bytes().to_vec(); // metadata size
        data.extend(0i32.to_le_bytes()); // attachment size
        data.extend((payload.len() as i64).to_le_bytes());
        data.extend(&placeholder);
        data.resize(16 + (placeholder.len()).max(240), 0);
        data.extend(&payload);
        let at = segment(&mut out, "ZISRAWSUBBLOCK", &data);
        entries.push(entry(b, at));
    }
    let mut meta = (xml.len() as i32).to_le_bytes().to_vec();
    meta.extend(0i32.to_le_bytes());
    meta.extend([0; 248]);
    meta.extend(xml.as_bytes());
    let meta_at = segment(&mut out, "ZISRAWMETADATA", &meta);
    let mut dir = (entries.len() as i32).to_le_bytes().to_vec();
    dir.extend([0; 124]);
    for e in &entries {
        dir.extend(e);
    }
    let dir_at = segment(&mut out, "ZISRAWDIRECTORY", &dir);
    // Patch the file header: part, directory and metadata positions.
    let h = 32 + 16 + 32;
    out[h..h + 4].copy_from_slice(&0i32.to_le_bytes());
    out[h + 4..h + 12].copy_from_slice(&(dir_at as i64).to_le_bytes());
    out[h + 12..h + 20].copy_from_slice(&(meta_at as i64).to_le_bytes());
    std::fs::write(path, out).unwrap();
}

const XML: &str = r#"<ImageDocument><Metadata><Information><Image><AcquisitionDateAndTime>2024-05-07T09:03:33.1687902Z</AcquisitionDateAndTime></Image></Information>
<Scaling><Items><Distance Id="X"><Value>5E-07</Value></Distance><Distance Id="Y"><Value>5E-07</Value></Distance></Items></Scaling></Metadata></ImageDocument>"#;

/// Writes, opens and converts; returns the reader.
fn open(name: &str, blocks: &[Block]) -> Box<dyn bioformats::FormatReader> {
    let dir = scratch(&format!("czi-{name}"));
    let path = dir.join(format!("{name}.czi"));
    write_czi(&path, blocks, XML);
    let reader = bioformats::open(&path).unwrap();
    assert_eq!(reader.format_name(), "Zeiss CZI");
    // Every series must survive conversion unchanged.
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
    reader
}

#[test]
fn gray16_channels_and_z_with_every_compression() {
    for compression in [0, 5, 6] {
        let mut blocks = Vec::new();
        for z in 0..3 {
            for c in 0..2 {
                let mut b = block(
                    1,
                    (0, 40, 40),
                    (0, 30, 30),
                    &[('C', c), ('Z', z)],
                    (z * 2 + c) as usize,
                );
                b.compression = compression;
                blocks.push(b);
            }
        }
        let r = open(&format!("zc{compression}"), &blocks);
        assert_eq!(r.series().len(), 1);
        let core = &r.series()[0];
        assert_eq!(
            (
                core.size_x,
                core.size_y,
                core.size_z,
                core.size_c,
                core.pixel_type
            ),
            (40, 30, 3, 2, PixelType::Uint16)
        );
        for b in &blocks {
            let zct = bioformats::Zct {
                z: b.dims[3].1 as u32,
                c: b.dims[2].1 as u32,
                t: 0,
            };
            let plane = core.plane_index(zct).unwrap();
            assert!(
                r.read_plane(0, plane).unwrap() == b.data,
                "compression {compression} plane {plane}"
            );
        }
        let image = &r.metadata().images[0];
        assert_eq!(
            image.acquisition_date.as_deref(),
            Some("2024-05-07T09:03:33.168")
        );
        assert!((image.physical_size_x.as_ref().unwrap().value - 0.5).abs() < 1e-9);
    }
}

#[test]
fn scenes_become_series() {
    let blocks = vec![
        block(0, (100, 20, 20), (50, 10, 10), &[('S', 0)], 1),
        block(0, (900, 30, 30), (70, 12, 12), &[('S', 1)], 2),
    ];
    let r = open("scenes", &blocks);
    assert_eq!(r.series().len(), 2);
    assert_eq!((r.series()[1].size_x, r.series()[1].size_y), (30, 12));
    assert!(r.read_plane(0, 0).unwrap() == blocks[0].data);
    assert!(r.read_plane(1, 0).unwrap() == blocks[1].data);
}

#[test]
fn mosaic_without_pyramid_is_stitched() {
    let blocks: Vec<Block> = (0..3)
        .map(|m| block(1, (m * 16, 16, 16), (0, 8, 8), &[('M', m)], m as usize))
        .collect();
    let r = open("tiles", &blocks);
    assert_eq!(r.series().len(), 1);
    assert_eq!((r.series()[0].size_x, r.series()[0].size_y), (48, 8));
    let plane = r.read_plane(0, 0).unwrap();
    for (m, b) in blocks.iter().enumerate() {
        for y in 0..8 {
            let dst = (y * 48 + m * 16) * 2;
            assert_eq!(
                &plane[dst..dst + 32],
                &b.data[y * 32..(y + 1) * 32],
                "tile {m} row {y}"
            );
        }
    }
}

/// A 2x2 RGB mosaic with a gap and an overlap, plus one pyramid block at
/// half resolution: tiles are stitched in file order onto a white canvas,
/// and BGR becomes RGB.
#[test]
fn pyramidal_rgb_mosaic_is_stitched() {
    let tiles = [(0, 0), (20, 0), (0, 14), (18, 16)];
    let mut blocks: Vec<Block> = tiles
        .iter()
        .enumerate()
        .map(|(m, &(x, y))| block(3, (x, 20, 20), (y, 14, 14), &[('M', m as i32)], m))
        .collect();
    blocks.push(block(3, (0, 40, 20), (0, 30, 15), &[('M', 0)], 9));
    let r = open("mosaic", &blocks);
    assert_eq!(
        r.series().len(),
        2,
        "full resolution plus one pyramid level"
    );
    let core = &r.series()[0];
    assert_eq!(
        (core.size_x, core.size_y, core.rgb_channel_count),
        (40, 30, 3)
    );
    assert_eq!((r.series()[1].size_x, r.series()[1].size_y), (20, 15));

    let mut expected = vec![255u8; 40 * 30 * 3];
    for (b, &(tx, ty)) in blocks.iter().zip(&tiles) {
        for y in 0..14 {
            for x in 0..20 {
                let (cx, cy) = (tx + x, ty + y);
                if cx < 40 && cy < 30 {
                    let src = (y * 20 + x) as usize * 3;
                    let dst = (cy * 40 + cx) as usize * 3;
                    expected[dst..dst + 3].copy_from_slice(&b.data[src..src + 3]);
                }
            }
        }
    }
    for px in expected.chunks_exact_mut(3) {
        px.swap(0, 2);
    }
    assert!(r.read_plane(0, 0).unwrap() == expected, "stitched level 0");

    // Region reads agree with the full plane.
    let region = Region::new(15, 10, 12, 9);
    let mut buf = vec![0; core.region_bytes(region.w, region.h)];
    r.open_bytes(0, 0, region, &mut buf).unwrap();
    for y in 0..9 {
        let src = ((10 + y) * 40 + 15) * 3;
        assert_eq!(&buf[y * 36..(y + 1) * 36], &expected[src..src + 36]);
    }

    let mut level1 = blocks[4].data.clone();
    for px in level1.chunks_exact_mut(3) {
        px.swap(0, 2);
    }
    assert!(r.read_plane(1, 0).unwrap() == level1, "pyramid level");
}
