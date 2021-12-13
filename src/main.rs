use ::anyhow::{Context, Result};
use ::structopt::{clap::arg_enum, StructOpt};

use self::gen::new_image;

/// Generate pictures using random flood fill.
#[derive(StructOpt, Debug)]
#[structopt(name = "fictures")]
struct Cli {
    /// Path to save output image to
    #[structopt(name = "output-file", parse(from_os_str))]
    out_path: std::path::PathBuf,

    /// Image width in pixels
    #[structopt(short = "W", long, default_value = "1000")]
    width: u32,

    /// Image height in pixels
    #[structopt(short = "H", long, default_value = "1000")]
    height: u32,

    /// Whether to skip writing output image to a file
    #[structopt(short = "N", long)]
    no_save: bool,

    /// Which generator to use for calculating pixel colours
    #[structopt(short = "C", possible_values = &ColourGen::variants(), case_insensitive = true, default_value = "Test")]
    colour_gen: ColourGen,

    /// Which generator to use for calculating adjacencies for pixels
    #[structopt(short = "T", possible_values = &TreeGen::variants(), case_insensitive = true, default_value = "Test")]
    tree_gen: TreeGen,

    /// Maximum displacement of a colour channel if using a random colour
    /// generator
    #[structopt(short = "D", default_value = "10")]
    step_size: u8,

    /// Seed for random number generator
    ///
    /// If no seed is specified, will generate a seed using system calls.
    #[structopt(short = "S")]
    seed: Option<u64>,

    /// Column to start tree at, expressed as coords in 0..1
    #[structopt(short = "X", default_value = "0.0", validator = check_unit_interval)]
    x: f64,

    /// Row to start tree at, expressed as coords in 0..1
    #[structopt(short = "Y", default_value = "0.0")]
    y: f64,
}

arg_enum! {
    #[derive(Debug, Copy, Clone, Eq, PartialEq)]
    enum ColourGen {
        Test,
        Rand,
    }
}
arg_enum! {
    #[derive(Debug, Copy, Clone, Eq, PartialEq)]
    enum TreeGen {
        Test,
        Spiral,
        Prim,
    }
}

fn check_unit_interval(s: String) -> Result<(), String> {
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
    let args = Cli::from_args();
    let no_save = args.no_save;
    let out_path = args.out_path.clone();
    let buf = new_image(args).context("Failed to generate image")?;
    if !no_save {
        buf.save(out_path).context("Failed to write output file")?;
    }
    Ok(())
}

mod gen;
