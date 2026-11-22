//! PNG / APNG reading and writing: every colour type and bit depth,
//! interlacing, palettes, animation frames, and round trips through the
//! PNG and TIFF writers.

mod common;

use std::path::Path;

use bioformats::convert::{ConvertOptions, convert};
use bioformats::{DimensionOrder, PixelType, Region};
use common::{pattern, scratch};

/// Encodes `data` (packed, big-endian samples, as PNG stores them).
#[allow(clippy::too_many_arguments)]
fn write_png(
    path: &Path,
    w: u32,
    h: u32,
    color: png::ColorType,
    depth: png::BitDepth,
    interlaced: bool,
    data: &[u8],
    palette: Option<&[u8]>,
) {
    if interlaced {
        return write_interlaced(
            path,
            w as usize,
            h as usize,
            color,
            depth as usize,
            data,
            palette,
        );
    }
    let file = std::fs::File::create(path).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(color);
    enc.set_depth(depth);
    if let Some(p) = palette {
        enc.set_palette(p.to_vec());
    }
    let mut writer = enc.write_header().unwrap();
    writer.write_image_data(data).unwrap();
}

/// A minimal independent encoder for Adam7-interlaced PNGs (the `png` crate
/// cannot write them). `packed` is the non-interlaced packed image.
fn write_interlaced(
    path: &Path,
    w: usize,
    h: usize,
    color: png::ColorType,
    bits: usize,
    packed: &[u8],
    palette: Option<&[u8]>,
) {
    use std::io::Write;
    let spp = samples(color);
    let row_bytes = (w * spp * bits).div_ceil(8);
    // Read sample `i` of row `y` (in samples, not bytes) from the packed image.
    let get = |y: usize, i: usize| -> u32 {
        if bits >= 8 {
            let b = bits / 8;
            let off = y * row_bytes + i * b;
            packed[off..off + b]
                .iter()
                .fold(0, |v, &x| v << 8 | x as u32)
        } else {
            let bit = i * bits;
            ((packed[y * row_bytes + bit / 8] >> (8 - bits - bit % 8)) & ((1 << bits) - 1)) as u32
        }
    };
    const PASSES: [(usize, usize, usize, usize); 7] = [
        (0, 0, 8, 8),
        (4, 0, 8, 8),
        (0, 4, 4, 8),
        (2, 0, 4, 4),
        (0, 2, 2, 4),
        (1, 0, 2, 2),
        (0, 1, 1, 2),
    ];
    let mut raw = Vec::new();
    for (x0, y0, dx, dy) in PASSES {
        let pw = (w + dx - 1 - x0) / dx;
        let ph = (h + dy - 1 - y0) / dy;
        if pw == 0 || ph == 0 || x0 >= w || y0 >= h {
            continue;
        }
        for py in 0..ph {
            let y = y0 + py * dy;
            raw.push(0); // filter: none
            let mut row = vec![0u8; (pw * spp * bits).div_ceil(8)];
            for px in 0..pw {
                let x = x0 + px * dx;
                for s in 0..spp {
                    let v = get(y, x * spp + s);
                    let i = px * spp + s;
                    if bits >= 8 {
                        let b = bits / 8;
                        for k in 0..b {
                            row[i * b + k] = (v >> (8 * (b - 1 - k))) as u8;
                        }
                    } else {
                        let bit = i * bits;
                        row[bit / 8] |= (v as u8) << (8 - bits - bit % 8);
                    }
                }
            }
            raw.extend(row);
        }
    }
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(&raw).unwrap();
    let idat = z.finish().unwrap();
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut chunk = |kind: &[u8], body: &[u8]| {
        out.extend((body.len() as u32).to_be_bytes());
        let mut crc = flate2::Crc::new();
        crc.update(kind);
        crc.update(body);
        out.extend(kind);
        out.extend(body);
        out.extend(crc.sum().to_be_bytes());
    };
    let code = match color {
        png::ColorType::Grayscale => 0,
        png::ColorType::Rgb => 2,
        png::ColorType::Indexed => 3,
        png::ColorType::GrayscaleAlpha => 4,
        png::ColorType::Rgba => 6,
    };
    let mut ihdr = (w as u32).to_be_bytes().to_vec();
    ihdr.extend((h as u32).to_be_bytes());
    ihdr.extend([bits as u8, code, 0, 0, 1]);
    chunk(b"IHDR", &ihdr);
    if let Some(p) = palette {
        chunk(b"PLTE", p);
    }
    chunk(b"IDAT", &idat);
    chunk(b"IEND", &[]);
    std::fs::write(path, out).unwrap();
}

fn samples(color: png::ColorType) -> usize {
    match color {
        png::ColorType::Grayscale | png::ColorType::Indexed => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
    }
}

/// Unpacked sample values for a `w` x `h` image at `bits` per sample, and the
/// same values packed the way PNG stores them.
fn image(w: usize, h: usize, spp: usize, bits: usize, seed: usize) -> (Vec<u8>, Vec<u8>) {
    let n = w * h * spp;
    if bits >= 8 {
        let bytes = bits / 8;
        let packed: Vec<u8> = (0..n * bytes).map(|i| pattern(seed, 1, i)).collect();
        return (packed.clone(), packed);
    }
    let max = (1u16 << bits) as usize;
    let values: Vec<u8> = (0..n)
        .map(|i| (pattern(seed, 1, i) as usize % max) as u8)
        .collect();
    let row_bytes = (w * spp * bits).div_ceil(8);
    let mut packed = vec![0u8; row_bytes * h];
    for y in 0..h {
        for i in 0..w * spp {
            let bit = i * bits;
            packed[y * row_bytes + bit / 8] |= values[y * w * spp + i] << (8 - bits - bit % 8);
        }
    }
    (values, packed)
}

#[test]
fn every_color_type_and_depth() {
    use png::{BitDepth::*, ColorType::*};
    let dir = scratch("png-types");
    let cases = [
        (Grayscale, One),
        (Grayscale, Two),
        (Grayscale, Four),
        (Grayscale, Eight),
        (Grayscale, Sixteen),
        (GrayscaleAlpha, Eight),
        (GrayscaleAlpha, Sixteen),
        (Rgb, Eight),
        (Rgb, Sixteen),
        (Rgba, Eight),
        (Rgba, Sixteen),
        (Indexed, Two),
        (Indexed, Eight),
    ];
    for (i, &(color, depth)) in cases.iter().enumerate() {
        for interlaced in [false, true] {
            let (w, h) = (37, 23);
            let spp = samples(color);
            let bits = depth as usize;
            let (values, packed) = image(w, h, spp, bits, i);
            let palette: Vec<u8> = (0..(1usize << bits.min(8)) * 3)
                .map(|i| (i * 7) as u8)
                .collect();
            let path = dir.join(format!("c{i}-i{interlaced}.png"));
            let pal = (color == Indexed).then_some(palette.as_slice());
            write_png(
                &path, w as u32, h as u32, color, depth, interlaced, &packed, pal,
            );

            let r = bioformats::open(&path).unwrap();
            let core = &r.series()[0];
            let name = format!("{color:?} {bits}-bit interlaced={interlaced}");
            assert_eq!(r.format_name(), "Animated PNG");
            assert_eq!(
                (core.size_x, core.size_y, core.rgb_channel_count as usize),
                (w as u32, h as u32, spp),
                "{name}"
            );
            assert_eq!(
                core.pixel_type,
                if bits == 16 {
                    PixelType::Uint16
                } else {
                    PixelType::Uint8
                },
                "{name}"
            );
            assert_eq!(core.bits_per_pixel as usize, bits, "{name}");
            assert!(!core.little_endian, "{name}: PNG samples are big-endian");
            assert_eq!(core.indexed, color == Indexed, "{name}");
            if color == Indexed {
                let lut = core.lut.as_ref().unwrap();
                assert_eq!(lut[1], [palette[3], palette[4], palette[5]], "{name}");
            }
            assert!(r.read_plane(0, 0).unwrap() == values, "{name}");

            // A region read matches the full plane.
            let region = Region::new(5, 3, 20, 11);
            let mut buf = vec![0; core.region_bytes(region.w, region.h)];
            r.open_bytes(0, 0, region, &mut buf).unwrap();
            let px = core.bytes_per_pixel();
            for y in 0..11 {
                let src = ((3 + y) * w + 5) * px;
                assert_eq!(
                    &buf[y * 20 * px..(y + 1) * 20 * px],
                    &values[src..src + 20 * px],
                    "{name}"
                );
            }
        }
    }
}

/// An APNG whose second frame only covers part of the canvas.
#[test]
fn animated_png_frames_are_composited() {
    let dir = scratch("png-apng");
    let path = dir.join("anim.png");
    let (w, h) = (30u32, 20u32);
    let file = std::fs::File::create(&path).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_animated(3, 0).unwrap();
    let mut writer = enc.write_header().unwrap();
    let full = |seed| {
        (0..(w * h * 3) as usize)
            .map(|i| pattern(seed, 2, i))
            .collect::<Vec<u8>>()
    };
    let f0 = full(0);
    writer.write_image_data(&f0).unwrap();
    let sub: Vec<u8> = (0..10 * 6 * 3).map(|i| pattern(1, 2, i)).collect();
    writer.set_frame_dimension(10, 6).unwrap();
    writer.set_frame_position(7, 4).unwrap();
    writer.write_image_data(&sub).unwrap();
    writer.reset_frame_position().unwrap();
    writer.reset_frame_dimension().unwrap();
    let f2 = full(2);
    writer.write_image_data(&f2).unwrap();
    writer.finish().unwrap();

    let r = bioformats::open(&path).unwrap();
    let core = &r.series()[0];
    assert_eq!(
        (core.size_t, core.image_count, core.dimension_order),
        (3, 3, DimensionOrder::XYCTZ)
    );
    let mut f1 = f0.clone();
    for y in 0..6 {
        let dst = ((4 + y) * w as usize + 7) * 3;
        f1[dst..dst + 30].copy_from_slice(&sub[y * 30..(y + 1) * 30]);
    }
    // Out-of-order access restarts the decoder.
    for p in [2, 0, 1, 1, 0, 2] {
        let expected = [&f0, &f1, &f2][p as usize];
        assert!(r.read_plane(0, p).unwrap() == *expected, "frame {p}");
    }
}

/// Our writer's output reads back identically, for gray/RGB(A), 8/16-bit,
/// palettes and multi-plane (APNG) series, and converts on to TIFF.
#[test]
fn writer_round_trips() {
    use png::{BitDepth::*, ColorType::*};
    let dir = scratch("png-write");
    let cases = [
        (Grayscale, Sixteen),
        (Rgb, Eight),
        (Rgba, Sixteen),
        (GrayscaleAlpha, Eight),
        (Indexed, Eight),
    ];
    for (i, &(color, depth)) in cases.iter().enumerate() {
        let (values, packed) = image(41, 29, samples(color), depth as usize, i + 10);
        let palette: Vec<u8> = (0..256 * 3).map(|i| (i * 11) as u8).collect();
        let src = dir.join(format!("src{i}.png"));
        write_png(
            &src,
            41,
            29,
            color,
            depth,
            false,
            &packed,
            (color == Indexed).then_some(palette.as_slice()),
        );
        let out = dir.join(format!("out{i}.png"));
        convert(&src, &out, &ConvertOptions::default(), &|_| {}).unwrap();
        let back = bioformats::open(&out).unwrap();
        let (a, b) = (
            &bioformats::open(&src).unwrap().series()[0].clone(),
            &back.series()[0],
        );
        assert_eq!(
            (a.size_x, a.size_y, a.rgb_channel_count, a.pixel_type),
            (b.size_x, b.size_y, b.rgb_channel_count, b.pixel_type)
        );
        assert_eq!(a.lut, b.lut, "palette survives PNG -> PNG");
        assert!(
            back.read_plane(0, 0).unwrap() == values,
            "{color:?} {depth:?} PNG -> PNG"
        );

        // PNG -> OME-TIFF -> PNG keeps pixels and the palette.
        let tif = dir.join(format!("out{i}.ome.tif"));
        convert(&src, &tif, &ConvertOptions::default(), &|_| {}).unwrap();
        let t = bioformats::open(&tif).unwrap();
        assert_eq!(t.series()[0].lut, a.lut, "palette survives PNG -> TIFF");
        let png2 = dir.join(format!("out{i}-from-tif.png"));
        convert(&tif, &png2, &ConvertOptions::default(), &|_| {}).unwrap();
        assert!(
            bioformats::open(&png2).unwrap().read_plane(0, 0).unwrap() == values,
            "{color:?} via TIFF"
        );
    }
}

#[test]
fn multi_plane_tiff_becomes_apng() {
    let dir = scratch("png-multi");
    // Build a 4-plane 16-bit TIFF via the PNG frames of an APNG, then back.
    let src = dir.join("stack.png");
    let file = std::fs::File::create(&src).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), 16, 9);
    enc.set_color(png::ColorType::Grayscale);
    enc.set_depth(png::BitDepth::Sixteen);
    enc.set_animated(4, 0).unwrap();
    let mut w = enc.write_header().unwrap();
    let frames: Vec<Vec<u8>> = (0..4)
        .map(|f| (0..16 * 9 * 2).map(|i| pattern(f, 4, i)).collect())
        .collect();
    for f in &frames {
        w.write_image_data(f).unwrap();
    }
    w.finish().unwrap();

    let tif = dir.join("stack.ome.tif");
    convert(&src, &tif, &ConvertOptions::default(), &|_| {}).unwrap();
    let out = dir.join("stack-out.png");
    convert(&tif, &out, &ConvertOptions::default(), &|_| {}).unwrap();
    let r = bioformats::open(&out).unwrap();
    assert_eq!(r.series()[0].image_count, 4);
    for (p, f) in frames.iter().enumerate() {
        assert!(r.read_plane(0, p as u32).unwrap() == *f, "frame {p}");
    }
}

#[test]
fn unsupported_outputs_are_rejected() {
    let dir = scratch("png-reject");
    let src = dir.join("f.ome.tif");
    // A float image cannot be PNG.
    let mut core = bioformats::CoreMetadata::new(8, 8, PixelType::Float);
    core.update_image_count();
    let err = bioformats::create_writer(
        &dir.join("x.png"),
        &[core.clone()],
        &Default::default(),
        &Default::default(),
    );
    assert!(err.is_err());
    // Two series cannot be one PNG.
    let core8 = bioformats::CoreMetadata::new(8, 8, PixelType::Uint8);
    let err = bioformats::create_writer(
        &dir.join("y.png"),
        &[core8.clone(), core8],
        &Default::default(),
        &Default::default(),
    );
    assert!(err.is_err());
    let _ = src;
}

/// APNG compositing per the spec: a default image outside the animation,
/// alpha "over" blending, and the "previous" and "background" dispose ops.
#[test]
fn apng_blend_and_dispose_ops() {
    use png::{BlendOp, DisposeOp};
    let dir = scratch("png-ops");
    let path = dir.join("ops.png");
    let (w, h) = (8usize, 6usize);
    let rgba = |seed: usize, n: usize, alpha: Option<u8>| -> Vec<u8> {
        (0..n * 4)
            .map(|i| {
                if i % 4 == 3 {
                    alpha.unwrap_or(255)
                } else {
                    pattern(seed, 5, i)
                }
            })
            .collect()
    };
    let default = vec![77u8; w * h * 4];
    let a = rgba(1, w * h, None);
    let sub1 = rgba(2, 3 * 2, Some(100)); // translucent
    let sub2 = rgba(3, 2 * 2, None);
    let sub3 = rgba(4, 1, None);

    let file = std::fs::File::create(&path).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_animated(4, 0).unwrap();
    enc.set_sep_def_img(true).unwrap();
    let mut wr = enc.write_header().unwrap();
    wr.write_image_data(&default).unwrap(); // not part of the animation
    let frame = |wr: &mut png::Writer<_>, data: &[u8], fw, fh, x, y, blend, dispose| {
        wr.set_frame_dimension(fw, fh).unwrap();
        wr.set_frame_position(x, y).unwrap();
        wr.set_blend_op(blend).unwrap();
        wr.set_dispose_op(dispose).unwrap();
        wr.write_image_data(data).unwrap();
        // After the last frame the encoder leaves animation mode, so these
        // may fail; that is fine.
        wr.reset_frame_position().ok();
        wr.reset_frame_dimension().ok();
    };
    frame(&mut wr, &a, 8, 6, 0, 0, BlendOp::Source, DisposeOp::None);
    frame(
        &mut wr,
        &sub1,
        3,
        2,
        2,
        1,
        BlendOp::Over,
        DisposeOp::Previous,
    );
    frame(
        &mut wr,
        &sub2,
        2,
        2,
        5,
        3,
        BlendOp::Source,
        DisposeOp::Background,
    );
    frame(&mut wr, &sub3, 1, 1, 0, 0, BlendOp::Source, DisposeOp::None);
    wr.finish().unwrap();

    // Reference compositing, written out longhand.
    let put = |canvas: &mut Vec<u8>, data: &[u8], fw: usize, fh: usize, x: usize, y: usize| {
        for r in 0..fh {
            let d = ((y + r) * w + x) * 4;
            canvas[d..d + fw * 4].copy_from_slice(&data[r * fw * 4..(r + 1) * fw * 4]);
        }
    };
    let out0 = a.clone();
    let mut out1 = a.clone();
    for r in 0..2 {
        for c in 0..3 {
            let d = ((1 + r) * w + 2 + c) * 4;
            let s = (r * 3 + c) * 4;
            let (fs, fd) = (100.0 / 255.0, 1.0 * (1.0 - 100.0 / 255.0));
            for k in 0..3 {
                out1[d + k] =
                    ((sub1[s + k] as f64 * fs + a[d + k] as f64 * fd) / (fs + fd)).round() as u8;
            }
            out1[d + 3] = ((fs + fd) * 255.0).round() as u8;
        }
    }
    let mut out2 = a.clone(); // frame 1 was disposed to "previous"
    put(&mut out2, &sub2, 2, 2, 5, 3);
    let mut out3 = a.clone(); // frame 2's rectangle cleared
    put(&mut out3, &[0; 16], 2, 2, 5, 3);
    put(&mut out3, &sub3, 1, 1, 0, 0);

    let r = bioformats::open(&path).unwrap();
    assert_eq!(
        r.series()[0].image_count,
        4,
        "the default image is not a frame"
    );
    for (p, expected) in [out0, out1, out2, out3].iter().enumerate() {
        assert!(r.read_plane(0, p as u32).unwrap() == *expected, "frame {p}");
    }
}

/// 1, 2 and 4-bit gray and palette images are written back at their depth.
#[test]
fn low_bit_depths_are_preserved() {
    use png::{BitDepth::*, ColorType::*};
    let dir = scratch("png-lowbits");
    for (i, (color, depth)) in [(Grayscale, One), (Grayscale, Four), (Indexed, Two)]
        .into_iter()
        .enumerate()
    {
        let (values, packed) = image(19, 7, 1, depth as usize, i + 30);
        let palette: Vec<u8> = (0..4 * 3).map(|i| (i * 40) as u8).collect();
        let src = dir.join(format!("src{i}.png"));
        write_png(
            &src,
            19,
            7,
            color,
            depth,
            false,
            &packed,
            (color == Indexed).then_some(palette.as_slice()),
        );
        let out = dir.join(format!("out{i}.png"));
        convert(&src, &out, &ConvertOptions::default(), &|_| {}).unwrap();
        let header = std::fs::read(&out).unwrap();
        assert_eq!(header[24], depth as u8, "IHDR bit depth for {color:?}");
        let back = bioformats::open(&out).unwrap();
        assert!(
            back.read_plane(0, 0).unwrap() == values,
            "{color:?} {depth:?}"
        );
    }
}
