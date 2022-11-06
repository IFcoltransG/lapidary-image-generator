use ::anyhow::{Context, Result};
use ::clap::{ArgEnum, Parser};

mod gen;

use self::gen::new_image;

/// Generate pictures using random flood fill.
#[derive(Parser, Debug)]
#[clap(name = env!("CARGO_PKG_NAME"), version = env!("CARGO_PKG_VERSION"))]
struct Cli {
    /// Path to save output image to (supports .png and .jpg)
    #[clap(name = "output-file", parse(from_os_str))]
    out_path: std::path::PathBuf,

    /// Image width in pixels
    #[clap(short = 'W', long, default_value = "1000", help_heading = "DIMENSIONS")]
    width: u32,

    /// Image height in pixels
    #[clap(short = 'H', long, default_value = "1000", help_heading = "DIMENSIONS")]
    height: u32,

    /// Whether to skip writing output image to a file [unimplemented]
    #[clap(short = 'N', long)]
    no_save: bool,

    /// Which generator to use for calculating pixel colours
    #[clap(
        short = 'C',
        arg_enum,
        ignore_case = true,
        default_value = "test",
        help_heading = "COLOURS"
    )]
    colour_gen: ColourGen,

    /// Which generator to use for calculating adjacencies for pixels
    #[clap(
        short = 'T',
        arg_enum,
        ignore_case = true,
        default_value = "test",
        help_heading = "FILL ORDER"
    )]
    tree_gen: TreeGen,

    /// Maximum displacement of a colour channel if using a random colour
    /// generator
    #[clap(short = 'D', default_value = "10", help_heading = "COLOURS")]
    step_size: u8,

    /// Seed for random number generator
    ///
    /// If no seed is specified, will generate a seed using system calls.
    #[clap(short = 'S', long)]
    seed: Option<u64>,

    /// Column to start tree at, expressed as coords in 0..1
    #[clap(short = 'X', default_value = "0.0", validator = check_unit_interval, help_heading = "FILL ORDER")]
    x: f64,

    /// Row to start tree at, expressed as coords in 0..1
    #[clap(short = 'Y', default_value = "0.0", help_heading = "FILL ORDER")]
    y: f64,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, ArgEnum)]
enum ColourGen {
    /// Colours that move linearly through white, yellow, red and black
    Test,
    /// A randomly perturbed colour compared to previous colour
    Rand,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, ArgEnum)]
enum TreeGen {
    /// A horizontal connection through the middle of the image, vertical connections down every column
    Test,
    /// Every pixel connected to two other pixels in a square spiral
    Spiral,
    /// Uses Prim's Algorithm to connect all pixels randomly into a tree
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
