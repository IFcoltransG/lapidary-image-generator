use super::{Cli, ColourGen, TreeGen};
use ::anyhow::{bail, Context, Result};
use ::image::{ImageBuffer, Pixel, Rgb};
use ::indicatif::{ProgressBar, ProgressStyle};
use ::rand::prelude::SeedableRng;
use ::rand_xoshiro::Xoshiro128PlusPlus;
use ::rayon::{scope, Scope};
use ::std::{
    sync::{
        mpsc::{channel, Sender},
        Arc,
    },
    thread,
};
use trees::Neighbours;

mod colour;
mod trees;

pub(super) fn new_image(
    Cli {
        width,
        height,
        colour_gen,
        tree_gen,
        seed,
        step_size,
        x,
        y,
        ..
    }: Cli,
) -> Result<ImageBuffer<Rgb<u8>, Vec<u8>>> {
    // Progress bar template
    let style = ProgressStyle::default_bar()
        .progress_chars("## ")
        .template("[{bar}] {prefix} - {percent}% done, {eta} left - {msg}");
    // Image dimensions
    let (usize_width, usize_height) = (
        width
            .try_into()
            .context("Failed to convert width u32 to usize")?,
        height
            .try_into()
            .context("Failed to convert height u32 to usize")?,
    );
    let (start_row, start_col): (usize, usize) = (
        (x * f64::try_from(width).context("Couldn't convert dimensions to float")?) as usize,
        (y * f64::try_from(height).context("Couldn't convert dimensions to float")?) as usize,
    );
    let start_u32 = (
        start_row
            .try_into()
            .context("Couldn't convert start coordinates usize to u32")?,
        start_col
            .try_into()
            .context("Couldn't convert start coordinates usize to u32")?,
    );
    // Random number seeding
    let rng = match seed {
        Some(seed) => Xoshiro128PlusPlus::seed_from_u64(seed),
        None => Xoshiro128PlusPlus::from_entropy(),
    };
    // Choose tree generator
    let tree_gen = match tree_gen {
        TreeGen::Test => colour::TestGen
            .tree(usize_width, usize_height, style.clone())
            .context("Failed to generate test tree for image")?,
        TreeGen::Spiral => trees::SpiralTree
            .tree(usize_width, usize_height, style.clone())
            .context("Failed to generate spiral tree for image")?,
        TreeGen::Prim => trees::PrimTree {
            rng: rng.clone(),
            initial_points: vec![(usize_width * start_col) + start_row],
            weights: move |point| {
                move |&v| {
                    let (x_weight, y_weight) = (
                        u64::try_from(point.0)
                            .expect("Couldn't convert coordinate when weighting colours"),
                        u64::try_from(point.1)
                            .expect("Couldn't convert coordinate when weighting colours"),
                    );
                    1 + if (Neighbours::NORTH | Neighbours::SOUTH).contains(v) {
                        y_weight * 2
                    } else if (Neighbours::EAST | Neighbours::WEST).contains(v) {
                        x_weight * 2
                    } else {
                        y_weight + x_weight
                    }
                }
            },
        }
        .tree(usize_width, usize_height, style.clone())
        .context("Failed to generate Prim's Algorithm tree for image")?,
    };
    let mut tree = tree_gen;
    eprintln!("Finished generating tree");
    prune_edges(usize_width, usize_height, style.clone(), &mut tree)
        .context("Failed to prune tree at edge of grid")?;
    eprintln!("Finished pruning tree");
    // Allocated image in memory
    let buf = ImageBuffer::new(width, height);
    eprintln!("Empty buffer allocated");
    // Choose and apply colour generator
    let buf = match colour_gen {
        ColourGen::Test => lay_colours(
            Arc::new(tree),
            start_u32,
            *Pixel::from_slice(&[0, 0, 0]),
            colour::TestGen,
            buf,
            style,
        ),
        ColourGen::Rand => {
            let rand = colour::RandColour { step_size, rng };
            lay_colours(
                Arc::new(tree),
                start_u32,
                *Pixel::from_slice(&[0, 0, 0]),
                rand,
                buf,
                style,
            )
        }
    }
    .context("Failed to place colours on image")?;
    eprintln!("Coloured pixels placed");
    Ok(buf)
}

trait GenTree: Sync + Send {
    fn tree(
        &mut self,
        width: usize,
        height: usize,
        style: ProgressStyle,
    ) -> Result<Vec<Neighbours>>;
}

trait GenColour: Sync + Send {
    fn colour(&mut self, old_colour: Rgb<u8>, direction_into: Neighbours) -> Rgb<u8>;
    fn new(&mut self) -> Self;
}

fn prune_edges(
    width: usize,
    height: usize,
    style: ProgressStyle,
    grid: &mut Vec<Neighbours>,
) -> Result<()> {
    let main_bar = ProgressBar::new(4)
        .with_style(style)
        .with_prefix("Pruning edges");
    main_bar.tick();
    for (row, flag) in [
        (0, Neighbours::NORTHWARD),
        (height - 1, Neighbours::SOUTHWARD),
    ]
    .iter()
    {
        main_bar.inc(1);
        for col in 0..width {
            // Remove directions that go off the edge of the grid vertically
            grid.get_mut(row * width + col)
                .context(format!("Index out of bounds at {}, {}", row, col))?
                .remove(*flag);
        }
    }
    for (col, flag) in [(0, Neighbours::WESTWARD), (width - 1, Neighbours::EASTWARD)].iter() {
        main_bar.inc(1);
        for row in 0..height {
            // Remove directions that go off the edge of the grid horizontally
            grid.get_mut(row * width + col)
                .context(format!("Index out of bounds at {}, {}", row, col))?
                .remove(*flag);
        }
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
