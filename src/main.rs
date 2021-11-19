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

mod gen {
    use super::{Cli, ColourGen, TreeGen};
    use ::anyhow::{bail, Context, Result};
    use ::bitflags::bitflags;
    use ::image::{ImageBuffer, Pixel, Rgb};
    use ::indicatif::{ProgressBar, ProgressStyle};
    use ::rand::prelude::{Rng, SeedableRng};
    use ::rand_xoshiro::Xoshiro128PlusPlus;
    use ::rayon::{scope, Scope};
    use ::std::{
        sync::{
            mpsc::{channel, Sender},
            Arc,
        },
        thread,
    };

    bitflags! {
        struct Neighbours: u8 {
            /// Represents the pixel above, in the negative y direction.
            const NORTH = 1 << 0;
            /// Represents the pixel above and to the right, in the positive x, negative y direction.
            const NORTHEAST = 1 << 1;
            /// Represents the pixel to the right, in the positive x direction.
            const EAST = 1 << 2;
            /// Represents the pixel below and to the right, in the positive x, positive y direction.
            const SOUTHEAST = 1 << 3;
            /// Represents the pixel below, in the positive y direction.
            const SOUTH = 1 << 4;
            /// Represents the pixel below and to the left, in the negative x, positive y direction.
            const SOUTHWEST = 1 << 5;
            /// Represents the pixel to the left, in the negative x direction.
            const WEST = 1 << 6;
            /// Represents the pixel above and to the left, in the negative x, negative y direction.
            const NORTHWEST = 1 << 7;
            /// North or northeast or northwest.
            const NORTHWARD = Self::NORTH.bits | Self::NORTHEAST.bits | Self::NORTHWEST.bits;
            /// South or southeast or southwest.
            const SOUTHWARD = Self::SOUTH.bits | Self::SOUTHEAST.bits | Self::SOUTHWEST.bits;
            /// East or northeast or southeast.
            const EASTWARD = Self::EAST.bits | Self::SOUTHEAST.bits | Self::NORTHEAST.bits;
            /// West or northwest or southwest.
            const WESTWARD = Self::WEST.bits | Self::SOUTHWEST.bits | Self::NORTHWEST.bits;
        }
    }

    impl Neighbours {
        const DIRECTIONS: [Neighbours; 8] = [
            Neighbours::NORTH,
            Neighbours::NORTHEAST,
            Neighbours::EAST,
            Neighbours::SOUTHEAST,
            Neighbours::SOUTH,
            Neighbours::SOUTHWEST,
            Neighbours::WEST,
            Neighbours::NORTHWEST,
        ];

        fn reverse(self: Self) -> Option<Neighbours> {
            match self {
                Neighbours::NORTH => Some(Neighbours::SOUTH),
                Neighbours::NORTHEAST => Some(Neighbours::SOUTHWEST),
                Neighbours::EAST => Some(Neighbours::WEST),
                Neighbours::SOUTHEAST => Some(Neighbours::NORTHWEST),
                Neighbours::SOUTH => Some(Neighbours::NORTH),
                Neighbours::SOUTHWEST => Some(Neighbours::NORTHEAST),
                Neighbours::WEST => Some(Neighbours::EAST),
                Neighbours::NORTHWEST => Some(Neighbours::SOUTHEAST),
                _ => None,
            }
        }

        fn step(self: Self, (mut row, mut col): (u32, u32)) -> (u32, u32) {
            if Neighbours::NORTHWARD.contains(self) {
                row -= 1
            } else if Neighbours::SOUTHWARD.contains(self) {
                row += 1
            }
            if Neighbours::WESTWARD.contains(self) {
                col -= 1
            } else if Neighbours::EASTWARD.contains(self) {
                col += 1
            }
            (row, col)
        }

        fn step_usize(self: Self, (mut row, mut col): (usize, usize)) -> (usize, usize) {
            if Neighbours::NORTHWARD.contains(self) {
                row -= 1
            } else if Neighbours::SOUTHWARD.contains(self) {
                row += 1
            }
            if Neighbours::WESTWARD.contains(self) {
                col -= 1
            } else if Neighbours::EASTWARD.contains(self) {
                col += 1
            }
            (row, col)
        }

        fn rotate_left(self, places: u32) -> Option<Self> {
            Self::from_bits(self.bits().rotate_right(places))
        }

        fn rotate_right(self, places: u32) -> Option<Self> {
            Self::from_bits(self.bits().rotate_left(places))
        }
    }

    pub(super) fn new_image(args: Cli) -> Result<ImageBuffer<Rgb<u8>, Vec<u8>>> {
        let Cli {
            width,
            height,
            colour_gen,
            tree_gen,
            seed,
            step_size,
            ..
        } = args;
        let tree_gen: Arc<dyn GenTree> = match tree_gen {
            TreeGen::Test => Arc::new(TestGen),
            TreeGen::Spiral => Arc::new(SpiralTree),
            TreeGen::Prim => Arc::new(PrimTree),
        };
        let style = ProgressStyle::default_bar()
            .progress_chars("## ")
            .template("[{bar}] {prefix} - {percent}% done, {eta} left - {msg}");
        let (usize_width, usize_height) = (
            width
                .try_into()
                .context("Failed to convert width u32 to usize")?,
            height
                .try_into()
                .context("Failed to convert height u32 to usize")?,
        );
        let mut tree = tree_gen
            .tree(usize_width, usize_height, style.clone())
            .context("Failed to generate tree for image")?;
        eprintln!("Finished generating tree");
        prune_edges(usize_width, usize_height, style.clone(), &mut tree)
            .context("Failed to prune tree at edge of grid")?;
        eprintln!("Finished pruning tree");
        let buf = ImageBuffer::new(width, height);
        eprintln!("Empty buffer allocated");
        let buf = match colour_gen {
            ColourGen::Test => lay_colours(
                Arc::new(tree),
                (0, 0),
                *Pixel::from_slice(&[0, 0, 0]),
                TestGen,
                buf,
                style,
            ),
            ColourGen::Rand => lay_colours(
                Arc::new(tree),
                (0, 0),
                *Pixel::from_slice(&[0, 0, 0]),
                {
                    let rng = match seed {
                        Some(seed) => Xoshiro128PlusPlus::seed_from_u64(seed),
                        None => Xoshiro128PlusPlus::from_entropy(),
                    };
                    RandColour { step_size, rng }
                },
                buf,
                style,
            ),
        }
        .context("Failed to place colours on image")?;
        eprintln!("Coloured pixels placed");
        Ok(buf)
    }

    trait GenTree: Sync + Send {
        fn tree(
            &self,
            width: usize,
            height: usize,
            style: ProgressStyle,
        ) -> Result<Vec<Neighbours>>;
    }

    trait GenColour: Sync + Send {
        fn colour(&mut self, old_colour: Rgb<u8>, direction_into: Neighbours) -> Rgb<u8>;
        fn new(&mut self) -> Self;
    }

    #[derive(Debug, Copy, Clone, Eq, PartialEq)]
    struct TestGen;

    #[derive(Debug, Clone, Eq, PartialEq)]
    struct RandColour {
        step_size: u8,
        rng: Xoshiro128PlusPlus,
    }

    impl RandColour {
        fn rand_channel(&mut self, old: u8, step_size: u8) -> u8 {
            let max = old.saturating_add(step_size);
            let min = old.saturating_sub(step_size);
            self.rng.gen_range(min..max)
        }
    }

    #[derive(Debug, Copy, Clone, Eq, PartialEq)]
    struct SpiralTree;

    #[derive(Debug, Copy, Clone, Eq, PartialEq)]
    struct PrimTree;

    impl GenTree for TestGen {
        fn tree(
            &self,
            width: usize,
            height: usize,
            style: ProgressStyle,
        ) -> Result<Vec<Neighbours>> {
            let num_pixels = width * height;
            let u64_num_pixels = num_pixels
                .try_into()
                .context("Failed to convert number of pixels usize to u64")?;
            let u64_width = width
                .try_into()
                .context("Failed to convert width usize to u64")?;
            let pixels_bar = ProgressBar::new(u64_num_pixels)
                .with_style(style.clone())
                .with_prefix("All pixels");
            pixels_bar.tick();
            let mut points = vec![Neighbours::empty(); num_pixels];
            for val in pixels_bar.wrap_iter(points.iter_mut()) {
                // let (row, col) = (i / width as usize, i % width as usize);
                *val = Neighbours::NORTH | Neighbours::SOUTH;
            }
            let cols_bar = ProgressBar::new(u64_width)
                .with_style(style)
                .with_prefix("Bottom row");
            cols_bar.tick();
            pixels_bar.finish_with_message("Done!");
            let col = height / 2;
            for i in cols_bar.wrap_iter(0..width as usize) {
                points[col * width + i] |= Neighbours::EAST | Neighbours::WEST;
            }
            cols_bar.finish_with_message("Done!");
            Ok(points)
        }
    }

    impl GenTree for SpiralTree {
        fn tree(
            &self,
            width: usize,
            height: usize,
            style: ProgressStyle,
        ) -> Result<Vec<Neighbours>> {
            let num_pixels = width * height;
            let u64_num_pixels = num_pixels
                .try_into()
                .context("Failed to convert number of pixels usize to u64")?;
            let bar = ProgressBar::new(u64_num_pixels)
                .with_style(style.clone())
                .with_prefix("Tree connections");
            bar.tick();
            let mut points = vec![Neighbours::empty(); num_pixels];
            let index = |row, col| row * width + col;
            let (mut row, mut col) = (0, 0);
            let mut direction = Neighbours::SOUTH;
            let distance = |turns: isize| {
                (if turns % 2 == 0 { width } else { height }) - (turns / 2).unsigned_abs() - 1
            };
            let mut turns = -1;
            while distance(turns) > 0 || distance(turns + 1) > 0 {
                for _ in 0..distance(turns) {
                    let (prev_row, prev_col) = (row, col);
                    let reverse = direction
                        .reverse()
                        .context("Failed to reverse invalid direction")?;
                    {
                        // move forward
                        let new_pos = direction.step_usize((row, col));
                        row = new_pos.0;
                        col = new_pos.1;
                    }
                    bar.inc(1);
                    *points
                        .get_mut(index(row, col))
                        .context("Couldn't access current position")? |= reverse;
                    *points
                        .get_mut(index(prev_row, prev_col))
                        .context("Couldn't access previous position")? |= direction;
                }
                // turn left
                direction = direction
                    .rotate_left(2)
                    .context("Failed to turn invalid direction")?;
                turns += 1;
            }
            bar.finish_with_message("Spiral done");
            Ok(points)
        }
    }

    impl GenTree for PrimTree {
        fn tree(
            &self,
            width: usize,
            height: usize,
            style: ProgressStyle,
        ) -> Result<Vec<Neighbours>> {
            let num_pixels = width * height;
            let u64_num_pixels = num_pixels
                .try_into()
                .context("Failed to convert number of pixels usize to u64")?;
            let bar = ProgressBar::new(u64_num_pixels)
                .with_style(style.clone())
                .with_prefix("Tree connections");
            bar.tick();
            todo!()
        }
    }

    impl GenColour for TestGen {
        fn colour(&mut self, old_colour: Rgb<u8>, _: Neighbours) -> Rgb<u8> {
            *Pixel::from_slice(&match old_colour.channels() {
                &[255, 255, 255] => [0, 0, 0],
                &[255, 255, b] => [255, 255, b + 1],
                &[255, g, b] => [255, g + 1, b],
                &[r, g, b] => [r + 1, g, b],
                _ => return old_colour,
            })
        }

        fn new(&mut self) -> Self {
            *self
        }
    }

    impl GenColour for RandColour {
        fn colour(&mut self, old_colour: Rgb<u8>, _: Neighbours) -> Rgb<u8> {
            if let &[r, g, b] = old_colour.channels() {
                *Pixel::from_slice(&[
                    self.rand_channel(r, self.step_size),
                    self.rand_channel(g, self.step_size),
                    self.rand_channel(b, self.step_size),
                ])
            } else {
                old_colour
            }
        }

        fn new(&mut self) -> Self {
            let mut rng = self.rng.clone();
            self.rng.long_jump();
            rng.jump();
            RandColour {
                step_size: self.step_size,
                rng,
            }
        }
    }

    fn prune_edges(
        width: usize,
        height: usize,
        style: ProgressStyle,
        grid: &mut Vec<Neighbours>,
    ) -> Result<()> {
        let main_bar = ProgressBar::new(4)
            .with_style(style.clone())
            .with_prefix("Pruning edges");
        main_bar.tick();
        for (row, flag) in [
            (0, Neighbours::NORTHWARD),
            (height - 1, Neighbours::SOUTHWARD),
        ]
        .iter()
        {
            let sub_bar = ProgressBar::new(width as u64)
                .with_style(style.clone())
                .with_prefix("Edge");
            sub_bar.tick();
            main_bar.inc(1);
            for col in 0..width {
                // Remove directions that go off the edge of the grid vertically
                grid.get_mut(row * width + col)
                    .context(format!("Index out of bounds at {}, {}", row, col))?
                    .remove(*flag);
                sub_bar.inc(1)
            }
            sub_bar.finish_with_message("Done")
        }
        for (col, flag) in [(0, Neighbours::WESTWARD), (width - 1, Neighbours::EASTWARD)].iter() {
            let sub_bar = ProgressBar::new(height as u64)
                .with_style(style.clone())
                .with_prefix("Edge");
            sub_bar.tick();
            main_bar.inc(1);
            for row in 0..height {
                // Remove directions that go off the edge of the grid horizontally
                grid.get_mut(row * width + col)
                    .context(format!("Index out of bounds at {}, {}", row, col))?
                    .remove(*flag);
                sub_bar.inc(1)
            }
            sub_bar.finish_with_message("Done")
        }
        main_bar.finish_with_message("Done");
        Ok(())
    }

    fn lay_colours<G: GenColour + 'static>(
        tree: Arc<Vec<Neighbours>>,
        root: (u32, u32),
        colour: Rgb<u8>,
        colour_gen: G,
        mut image: ImageBuffer<Rgb<u8>, Vec<u8>>,
        style: ProgressStyle,
    ) -> Result<ImageBuffer<Rgb<u8>, Vec<u8>>> {
        let (height, width) = (image.height(), image.width());
        let num_pixels = width * height;
        let bar = ProgressBar::new(num_pixels.into())
            .with_style(style)
            .with_prefix("Plotting pixels");
        bar.tick();
        let (enqueue_pixel, dequeue_pixel) = channel();
        let handle = thread::spawn(move || {
            for ((row, col), colour) in dequeue_pixel {
                image.put_pixel(col, row, colour);
                bar.inc(1);
            }
            bar.finish_with_message("Done");
            image
        });
        scope(|thread_scope| {
            lay_colours_in_subtree(
                thread_scope,
                tree,
                root,
                Neighbours::empty(),
                colour,
                colour_gen,
                (height, width),
                enqueue_pixel,
            )
        })
        .context("Failed to assign colours to the image")?;
        match handle.join() {
            Ok(image) => Ok(image),
            Err(_) => bail!("Failed to join image-mutator thread"),
        }
    }

    fn lay_colours_in_subtree<G: GenColour + 'static>(
        thread_scope: &Scope,
        tree: Arc<Vec<Neighbours>>,
        (root_row, root_col): (u32, u32),
        visited_directions: Neighbours,
        initial_colour: Rgb<u8>,
        mut colour_gen: G,
        (height, width): (u32, u32),
        enqueue_pixel: Sender<((u32, u32), Rgb<u8>)>,
    ) -> Result<()> {
        // tree must not contain any cycles
        let index = root_row * width + root_col;
        let &tree_directions = tree
            .get(usize::try_from(index).context("Failed to convert index u32 to usize")?)
            .context("Index out of bounds reading from tree")?;
        let unvisited_directions = tree_directions - visited_directions;
        // Add new colour to image
        enqueue_pixel
            .send(((root_row, root_col), initial_colour))
            .context("Main thread closed connection before all workers finished")?;
        // Check next directions
        for &child in Neighbours::DIRECTIONS
            .iter()
            .filter(|&&dir| unvisited_directions.contains(dir))
        {
            let enqueue_pixel = enqueue_pixel.clone();
            let tree = tree.clone();
            let new_colour = colour_gen.colour(initial_colour, child);
            let new_colour_gen = colour_gen.new();
            let (row, col) = child.step((root_row, root_col));
            thread_scope.spawn(move |s| {
                lay_colours_in_subtree(
                    s,
                    tree,
                    (row, col),
                    child.reverse().unwrap_or(Neighbours::empty()),
                    new_colour,
                    new_colour_gen,
                    (height, width),
                    enqueue_pixel.clone(),
                )
                .unwrap_or_else(|e| panic!("Thread panicking due to error:\n{}\n", e));
            });
        }
        Ok(())
    }
}
