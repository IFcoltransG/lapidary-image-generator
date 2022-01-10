use ::anyhow::{Context, Result};
use ::clap::{Parser, ArgEnum};

use self::gen::new_image;

/// Generate pictures using random flood fill.
#[derive(Parser, Debug)]
#[clap(name = "fictures")]
struct Cli {
    /// Path to save output image to
    #[clap(name = "output-file", parse(from_os_str))]
    out_path: std::path::PathBuf,

    /// Image width in pixels
    #[clap(short = 'W', long, default_value = "1000")]
    width: u32,

    /// Image height in pixels
    #[clap(short = 'H', long, default_value = "1000")]
    height: u32,

    /// Whether to skip writing output image to a file
    #[clap(short = 'N', long)]
    no_save: bool,

    /// Which generator to use for calculating pixel colours
    #[clap(short = 'C', arg_enum, ignore_case = true, default_value = "Test")]
    colour_gen: ColourGen,

    /// Which generator to use for calculating adjacencies for pixels
    #[clap(short = 'T', arg_enum, ignore_case = true, default_value = "Test")]
    tree_gen: TreeGen,

    /// Maximum displacement of a colour channel if using a random colour
    /// generator
    #[clap(short = 'D', default_value = "10")]
    step_size: u8,

    /// Seed for random number generator
    ///
    /// If no seed is specified, will generate a seed using system calls.
    #[clap(short = 'S')]
    seed: Option<u64>,

    /// Column to start tree at, expressed as coords in 0..1
    #[clap(short = 'X', default_value = "0.0", validator = check_unit_interval)]
    x: f64,

    /// Row to start tree at, expressed as coords in 0..1
    #[clap(short = 'Y', default_value = "0.0")]
    y: f64,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, ArgEnum)]
enum ColourGen {
    Test,
    Rand,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, ArgEnum)]
enum TreeGen {
    Test,
    Spiral,
    Prim,
}

fn check_unit_interval(s: &str) -> Result<(), String> {
    let float: f64 = s.parse().map_err(|_| "not parseable as float")?;
    if float < 0. {
        return Err("float cannot be negative".to_string());
    }
    if float > 1. {
        return Err("float cannot be greater than 1".to_string());
    }
    Ok(())
}

fn main() -> Result<()> {
    // parse command line arguments
    let args = Cli::parse();
    let no_save = args.no_save;
    let out_path = args.out_path.clone();
    let buf = new_image(args).context("Failed to generate image")?;
    if !no_save {
        buf.save(out_path).context("Failed to write output file")?;
    }
    Ok(())
}

mod gen;
