use ::anyhow::{Context, Result};
use ::structopt::StructOpt;

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
}

fn main() -> Result<()> {
    use gen::new_image;
    // parse command line arguments
    let args = Cli::from_args();
    let buf = new_image(args.width, args.height).context("Failed to generate image")?;
    if !args.no_save {
        buf.save(args.out_path)
            .context("Failed to write output file")?;
    }
    Ok(())
}

mod gen {
    use ::anyhow::{Context, Result, bail};
    use ::bitflags::bitflags;
    use ::image::{ImageBuffer, Pixel, Rgb};
    use ::indicatif::{ProgressBar, ProgressStyle};
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

    fn reverse(direction: Neighbours) -> Option<Neighbours> {
        match direction {
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

    fn shift(direction: Neighbours, (mut row, mut col): (u32, u32)) -> (u32, u32) {
        if Neighbours::NORTHWARD.contains(direction) {
            row -= 1
        } else if Neighbours::SOUTHWARD.contains(direction) {
            row += 1
        }
        if Neighbours::WESTWARD.contains(direction) {
            col -= 1
        } else if Neighbours::EASTWARD.contains(direction) {
            col += 1
        }
        (row, col)
    }

    pub(super) fn new_image(width: u32, height: u32) -> Result<ImageBuffer<Rgb<u8>, Vec<u8>>> {
        let style = ProgressStyle::default_bar()
            .progress_chars("## ")
            .template("{prefix}[{bar}] {percent}% done, {eta} left");
        let (usize_width, usize_height) = (
            width
                .try_into()
                .context("Failed to convert width u32 to usize")?,
            height
                .try_into()
                .context("Failed to convert height u32 to usize")?,
        );
        let mut tree = fake_tree(usize_width, usize_height, style.clone())
            .context("Failed to generate tree for image")?;
        prune_edges(usize_width, usize_height, style.clone(), &mut tree)
            .context("Failed to prune tree at edge of grid")?;
        println!("{:#?}", tree);
        let buf = ImageBuffer::new(width, height);
        let buf = lay_colours(
            Arc::new(tree),
            (0, 0),
            *Pixel::from_slice(&[0, 0, 0]),
            buf,
            style,
        )
        .context("Failed to place colours on image")?;
        Ok(buf)
    }

    fn fake_tree(width: usize, height: usize, style: ProgressStyle) -> Result<Vec<Neighbours>> {
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
        let cols_bar = ProgressBar::new(u64_width)
            .with_style(style)
            .with_prefix("Bottom row");
        let mut points = vec![Neighbours::empty(); num_pixels];
        for val in pixels_bar.wrap_iter(points.iter_mut()) {
            // let (row, col) = (i / width as usize, i % width as usize);
            *val = Neighbours::NORTH | Neighbours::SOUTH;
        }
        pixels_bar.finish_with_message("Done!");
        for i in cols_bar.wrap_iter(0..width as usize) {
            points[i] |= Neighbours::EAST | Neighbours::WEST;
        }
        cols_bar.finish_with_message("Done!");
        Ok(points)
    }

    fn fake_colour(old_colour: Rgb<u8>) -> Rgb<u8> {
        *Pixel::from_slice(&match old_colour.channels() {
            &[255, 255, 255] => [0, 0, 0],
            &[255, 255, b] => [255, 255, b + 1],
            &[255, g, b] => [255, g + 1, b],
            &[r, g, b] => [r + 1, g, b],
            _ => return old_colour,
        })
    }

    fn prune_edges(
        width: usize,
        height: usize,
        style: ProgressStyle,
        grid: &mut Vec<Neighbours>,
    ) -> Result<()> {
        let main_bar = ProgressBar::new(4)
            .with_style(style.clone())
            .with_prefix("Edges");
        for (row, flag) in [
            (0, Neighbours::NORTHWARD),
            (height - 1, Neighbours::SOUTHWARD),
        ]
        .iter()
        {
            let sub_bar = ProgressBar::new(width as u64)
                .with_style(style.clone())
                .with_prefix("Edge");
            main_bar.tick();
            for col in 0..width {
                // Remove directions that go off the edge of the grid vertically
                grid.get_mut(row * width + col)
                    .context(format!("Index out of bounds at {}, {}", row, col))?
                    .remove(*flag);
                sub_bar.tick()
            }
        }
        for (col, flag) in [(0, Neighbours::WESTWARD), (width - 1, Neighbours::EASTWARD)].iter() {
            let sub_bar = ProgressBar::new(height as u64)
                .with_style(style.clone())
                .with_prefix("Edge");
            main_bar.tick();
            for row in 0..height {
                // Remove directions that go off the edge of the grid horizontally
                grid.get_mut(row * width + col)
                    .context(format!("Index out of bounds at {}, {}", row, col))?
                    .remove(*flag);
                sub_bar.tick()
            }
        }
        Ok(())
    }

    fn lay_colours(
        tree: Arc<Vec<Neighbours>>,
        root: (u32, u32),
        colour: Rgb<u8>,
        mut image: ImageBuffer<Rgb<u8>, Vec<u8>>,
        style: ProgressStyle,
    ) -> Result<ImageBuffer<Rgb<u8>, Vec<u8>>> {
        let (height, width) = (image.height(), image.width());
        let num_pixels = width * height;
        let bar = ProgressBar::new(num_pixels.into()).with_style(style);
        let (enqueue_pixel, dequeue_pixel) = channel();
        let handle = thread::spawn(move || {
            for ((row, col), colour) in dequeue_pixel {
                image.put_pixel(col, row, colour);
                bar.tick();
            }
            image
        });
        scope(|thread_scope| {
            lay_colours_in_subtree(
                thread_scope,
                tree,
                root,
                Neighbours::empty(),
                colour,
                (height, width),
                enqueue_pixel,
            )
        })
        .context("Failed to assign colours to the image")?;
        match handle.join() {
            Ok(image) =>
            Ok(image),
            Err(_) => bail!("Failed to join image-mutator thread"),
        }
    }

    fn lay_colours_in_subtree(
        thread_scope: &Scope,
        tree: Arc<Vec<Neighbours>>,
        (root_row, root_col): (u32, u32),
        visited_directions: Neighbours,
        initial_colour: Rgb<u8>,
        (height, width): (u32, u32),
        enqueue_pixel: Sender<((u32, u32), Rgb<u8>)>,
    ) -> Result<()> {
        // eprintln!("Laying colours: {:?} {:?} {:?} {:?}", (root_row, root_col),
        // visited_directions, initial_colour, (height, width)); tree must not
        // contain any cycles
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
        for &child in DIRECTIONS
            .iter()
            .filter(|&&dir| unvisited_directions.contains(dir))
        {
            let enqueue_pixel = enqueue_pixel.clone();
            let tree = tree.clone();
            let (row, col) = shift(child, (root_row, root_col));
            thread_scope.spawn(move |s| {
                lay_colours_in_subtree(
                    s,
                    tree,
                    (row, col),
                    reverse(child).unwrap_or(Neighbours::empty()),
                    fake_colour(initial_colour),
                    (height, width),
                    enqueue_pixel.clone(),
                )
                .unwrap_or_else(|e| panic!("Thread panicking due to error:\n{}\n", e));
            });
        }
        Ok(())
    }
}
