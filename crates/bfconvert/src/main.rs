//! Command-line converter. Port of Bio-Formats' `bfconvert`
//! (`loci.formats.tools.ImageConverter`).

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use bioformats::convert::{ConvertOptions, Progress, convert};
use bioformats::{Compression, WriterOptions};
use clap::Parser;

#[derive(Parser)]
#[command(
    version,
    about = "Convert microscopy images between formats (a Rust port of Bio-Formats' bfconvert)"
)]
struct Args {
    /// Input file, in any supported format.
    input: PathBuf,
    /// Output file; the format is chosen by suffix (.ome.tif, .tif, .btf, ...).
    output: PathBuf,
    /// Only convert these series (0-based). Repeat or comma-separate.
    #[arg(long, value_delimiter = ',')]
    series: Option<Vec<usize>>,
    /// Compression: uncompressed, lzw or zlib.
    #[arg(long, default_value = "uncompressed")]
    compression: Compression,
    /// zlib compression level (0-9).
    #[arg(long, default_value_t = 6)]
    level: u32,
    /// Apply the TIFF horizontal predictor before compressing.
    #[arg(long)]
    predictor: bool,
    /// Write tiles of WxH pixels (multiples of 16) instead of strips.
    #[arg(long, value_parser = parse_size)]
    tile_size: Option<(u32, u32)>,
    /// Always write BigTIFF.
    #[arg(long, conflicts_with = "no_bigtiff")]
    bigtiff: bool,
    /// Never write BigTIFF (fails if the output exceeds 4 GiB).
    #[arg(long)]
    no_bigtiff: bool,
    /// Do not embed the input's original metadata as OME annotations.
    #[arg(long)]
    no_original_metadata: bool,
    /// Overwrite the output if it exists.
    #[arg(long)]
    overwrite: bool,
    /// Worker threads (default: all cores).
    #[arg(long)]
    threads: Option<usize>,
}

fn parse_size(s: &str) -> Result<(u32, u32), String> {
    let (w, h) = s.split_once(['x', 'X']).unwrap_or((s, s));
    let parse = |v: &str| v.trim().parse::<u32>().map_err(|e| format!("{v:?}: {e}"));
    Ok((parse(w)?, parse(h)?))
}

fn main() -> ExitCode {
    let args = Args::parse();
    if let Some(n) = args.threads {
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build_global()
            .ok();
    }
    if args.output.exists() && !args.overwrite {
        eprintln!(
            "{} exists; pass --overwrite to replace it",
            args.output.display()
        );
        return ExitCode::FAILURE;
    }
    let compression = match args.compression {
        Compression::Deflate { .. } => Compression::Deflate { level: args.level },
        c => c,
    };
    let options = ConvertOptions {
        series: args.series,
        writer: WriterOptions {
            compression,
            tile_size: args.tile_size,
            big_tiff: if args.bigtiff {
                Some(true)
            } else if args.no_bigtiff {
                Some(false)
            } else {
                None
            },
            predictor: args.predictor,
            original_metadata: !args.no_original_metadata,
        },
    };

    let start = Instant::now();
    let last_pct = AtomicU64::new(u64::MAX);
    let report = |p: Progress| {
        let pct = p.planes_done * 100 / p.planes_total.max(1);
        if last_pct.swap(pct, Ordering::Relaxed) != pct {
            eprint!(
                "\r[{pct:3}%] series {} plane {} ({}/{})",
                p.series, p.plane, p.planes_done, p.planes_total
            );
        }
    };
    match convert(&args.input, &args.output, &options, &report) {
        Ok(summary) => {
            let secs = start.elapsed().as_secs_f64();
            eprintln!(
                "\n{} -> {}: {} series, {} planes, {:.1} MB in {secs:.2}s ({:.0} MB/s) [{}]",
                args.input.display(),
                args.output.display(),
                summary.series,
                summary.planes,
                summary.bytes_read as f64 / 1e6,
                summary.bytes_read as f64 / 1e6 / secs.max(1e-9),
                summary.reader_format,
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("\nerror: {e}");
            let _ = std::fs::remove_file(&args.output);
            ExitCode::FAILURE
        }
    }
}
