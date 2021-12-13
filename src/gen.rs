use super::{Cli, ColourGen, TreeGen};
use ::anyhow::{bail, Context, Result};
use ::bitflags::bitflags;
use ::image::{ImageBuffer, Pixel, Rgb};
use ::indicatif::{ProgressBar, ProgressStyle};
use ::rand::prelude::{Rng, SeedableRng, SliceRandom};
use ::rand_xoshiro::Xoshiro128PlusPlus;
use ::rayon::{scope, Scope};
use ::std::{
    mem::replace,
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

    /// Return the backwards version of a direction
    ///
    /// Returns none if the Neighbours has more than one direction set
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

    /// Move a point in a direction
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

    /// Move a point in a direction but it's a usize point
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

    /// Turn a direction anticlockwise
    fn rotate_left(self, places: u32) -> Option<Self> {
        Self::from_bits(self.bits().rotate_right(places))
    }

    /// Turn a direction clockwise
    fn rotate_right(self, places: u32) -> Option<Self> {
        Self::from_bits(self.bits().rotate_left(places))
    }

    fn random_direction<R: Rng, F: Fn(&Neighbours) -> u64>(
        self,
        rng: &mut R,
        weight: F,
    ) -> Result<Neighbours> {
        if self.is_empty() {
            bail!("No directions to choose randomly from")
        }
        let directions = self.collect::<Vec<_>>();
        let &ans = directions
            .as_slice()
            .choose_weighted(rng, weight)
            .context("Weights assigned inadequately")?;
        Ok(ans)
    }
}

impl Iterator for Neighbours {
    type Item = Neighbours;

    fn next(&mut self) -> Option<Self::Item> {
        match Neighbours::DIRECTIONS.iter().find(|&&d| self.contains(d)) {
            Some(&dir) => {
                *self -= dir;
                Some(dir)
            }
            None => None,
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, Some(8))
    }
}

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
        TreeGen::Test => TestGen
            .tree(usize_width, usize_height, style.clone())
            .context("Failed to generate test tree for image")?,
        TreeGen::Spiral => SpiralTree
            .tree(usize_width, usize_height, style.clone())
            .context("Failed to generate spiral tree for image")?,
        TreeGen::Prim => PrimTree {
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
            TestGen,
            buf,
            style,
        ),
        ColourGen::Rand => {
            let rand = RandColour { step_size, rng };
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

#[derive(Debug, Clone, Eq, PartialEq)]
struct PrimTree<F, G>
where
    F: Fn(&Neighbours) -> u64,
    G: Fn((usize, usize)) -> F,
{
    rng: Xoshiro128PlusPlus,
    initial_points: Vec<usize>,
    weights: G,
}

impl GenTree for TestGen {
    fn tree(
        &mut self,
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
        &mut self,
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

impl<F, G> GenTree for PrimTree<F, G>
where
    F: Fn(&Neighbours) -> u64,
    G: Fn((usize, usize)) -> F + Sync + Send,
{
    fn tree(
        &mut self,
        width: usize,
        height: usize,
        style: ProgressStyle,
    ) -> Result<Vec<Neighbours>> {
        let num_pixels = width * height;
        // initialise output to have no connections
        let mut output_points = vec![Neighbours::empty(); num_pixels];
        // initialise a vec with connections to every neighbour
        let mut possible_edges = vec![Neighbours::all(); num_pixels];
        prune_edges(width, height, style.clone(), &mut possible_edges)
            .context("Failed to prune initial complete tree when generating spanning tree")?;
        let u64_num_pixels = num_pixels
            .try_into()
            .context("Failed to convert number of pixels usize to u64")?;
        // create progress bar
        let bar = ProgressBar::new(u64_num_pixels)
            .with_style(style)
            .with_prefix("Tree connections");
        // display progress bar
        bar.tick();
        // store whether node has been added to the queue before as a neighbour of a
        // processed node
        let mut seen = vec![false; num_pixels];
        // store whether a node has been joined to another node as part of the tree
        let mut processed = vec![false; num_pixels];
        // queue can only contain each point once
        let mut point_queue = Vec::with_capacity(num_pixels);
        // start with configured initial points
        for &index in &self.initial_points {
            point_queue.push(index);
            *processed
                .get_mut(index)
                .context("Initial point out of range to set processed status")? = true;
        }
        // run through queue
        while !point_queue.is_empty() {
            // this should be processed
            let from_index = self.rng.gen_range(0..point_queue.len());
            let last_index = point_queue.len() - 1;
            point_queue.swap(last_index, from_index);
            // randomly select point
            let point_index = point_queue
                .pop()
                .context("Point vanished after moving it to the back of vector")?;
            // get random edges until there are none left
            // or break out of loop when an edge leads to a point that can be processed
            while let (Ok(edge), point) = {
                // access which edges are possible from this point
                let point = possible_edges
                    .get_mut(point_index)
                    .context("Failed to access point ")?;
                (
                    point.random_direction(
                        &mut self.rng,
                        (self.weights)((point_index / width, point_index % width)),
                    ),
                    point,
                )
            } {
                // this edge is no longer available
                *point -= edge;
                // follow edge
                let (end_row, end_col) =
                    edge.step_usize((point_index / width, point_index % width));
                let endpoint = end_row * width + end_col;
                // direction back to the randomly chosen point
                let backwards = edge
                    .reverse()
                    .context("Couldn't calculate reverse of direction to a point")?;
                // remove this edge from available ones
                let endpoint_pointer = possible_edges
                    .get_mut(endpoint)
                    .context("Failed to remove neighbour point edge")?;
                *endpoint_pointer -= backwards;
                // if not already added to queue, add it to queue
                if !replace(
                    seen.get_mut(endpoint)
                        .context("Couldn't read seen status of index")?,
                    true,
                ) {
                    point_queue.push(endpoint)
                }
                // if not already added to tree, add it to tree, then break out of loop
                if !replace(
                    processed
                        .get_mut(endpoint)
                        .context("Couldn't read processed status of index")?,
                    true,
                ) {
                    // add start of this edge to output
                    *output_points
                        .get_mut(point_index)
                        .context("Failed to access point ")? |= edge;
                    // add end of this edge to output
                    *output_points
                        .get_mut(endpoint)
                        .context("Failed to add neighbour point edge")? |= backwards;
                    break;
                }
            }
            if possible_edges
                .get(point_index)
                .context("Failed to access point to review possible edges")?
                .is_empty()
            {
                // point finished
                bar.inc(1);
            } else {
                // randomly chosen point has more edges connecting to it
                // return it to queue for later
                point_queue.push(point_index);
            }
        }
        bar.finish_with_message("Done");
        Ok(output_points)
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
