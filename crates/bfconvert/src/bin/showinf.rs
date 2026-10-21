//! Prints what a reader sees in a file. A minimal port of Bio-Formats'
//! `showinf` (`loci.formats.tools.ImageInfo`).

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;

#[derive(Parser)]
#[command(version, about = "Show the dimensions and metadata of an image file")]
struct Args {
    file: PathBuf,
    /// Also print original (format-specific) metadata.
    #[arg(long)]
    metadata: bool,
    /// Print the OME-XML the file would produce.
    #[arg(long)]
    omexml: bool,
}

fn main() -> ExitCode {
    let args = Args::parse();
    let reader = match bioformats::open(&args.file) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    println!("Format: {}", reader.format_name());
    println!("Used files: {:?}", reader.used_files());
    println!("Series count: {}", reader.series().len());
    for (s, core) in reader.series().iter().enumerate() {
        let image = &reader.metadata().images[s];
        println!(
            "Series #{s}{}:",
            image
                .name
                .as_deref()
                .map(|n| format!(" ({n})"))
                .unwrap_or_default()
        );
        println!("\tImage count = {}", core.image_count);
        println!("\tRGB = {} ({})", core.is_rgb(), core.rgb_channel_count);
        println!("\tInterleaved = {}", core.interleaved);
        println!("\tWidth = {}", core.size_x);
        println!("\tHeight = {}", core.size_y);
        println!("\tSizeZ = {}", core.size_z);
        println!("\tSizeT = {}", core.size_t);
        println!(
            "\tSizeC = {} (effective {})",
            core.size_c,
            core.effective_size_c()
        );
        println!("\tDimension order = {}", core.dimension_order);
        println!("\tPixel type = {}", core.pixel_type.ome_name());
        println!("\tValid bits per pixel = {}", core.bits_per_pixel);
        println!("\tLittle endian = {}", core.little_endian);
        let (tw, th) = reader.optimal_tile_size(s);
        println!("\tOptimal tile = {tw}x{th}");
        let size = |q: &Option<bioformats::meta::Quantity>| {
            q.as_ref()
                .map(|q| format!("{} {}", q.value, q.unit))
                .unwrap_or_else(|| "-".into())
        };
        println!(
            "\tPhysical size = {} x {} x {}",
            size(&image.physical_size_x),
            size(&image.physical_size_y),
            size(&image.physical_size_z)
        );
        if let Some(date) = &image.acquisition_date {
            println!("\tAcquired = {date}");
        }
        if args.metadata {
            for (k, v) in &core.series_metadata {
                println!("\t\t{k}: {v}");
            }
        }
    }
    if args.metadata {
        println!("Global metadata:");
        for (k, v) in &reader.metadata().global_metadata {
            println!("\t{k}: {v}");
        }
    }
    if args.omexml {
        let opts = bioformats::ome_xml::XmlOptions {
            uuid: None,
            tiff_data: &[],
            original_metadata: false,
        };
        println!(
            "{}",
            bioformats::ome_xml::write(reader.series(), reader.metadata(), &opts)
        );
    }
    ExitCode::SUCCESS
}
