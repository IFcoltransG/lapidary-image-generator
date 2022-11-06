use super::{colour::TestGen, prune_edges, GenTree};
use ::anyhow::{bail, Context, Result};
use ::bitflags::bitflags;
use ::indicatif::{ProgressBar, ProgressStyle};
use ::rand::prelude::{Rng, SliceRandom};
use ::rand_xoshiro::Xoshiro128PlusPlus;
use ::std::mem::replace;

bitflags! {
  /// Bit flags for which neighbours of a pixel including diagonals are connected
  pub(crate) struct Neighbours: u8 {
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
    pub(crate) const DIRECTIONS: [Neighbours; 8] = [
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
    pub(crate) fn reverse(self: Self) -> Option<Neighbours> {
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
    pub(crate) fn step(self: Self, (mut row, mut col): (u32, u32)) -> (u32, u32) {
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

    /// Move a point in a direction, with the point represented by usize coordinates
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

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub(crate) struct SpiralTree;

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct PrimTree<F, G>
where
    F: Fn(&Neighbours) -> u64,
    G: Fn((usize, usize)) -> F,
{
    pub(crate) rng: Xoshiro128PlusPlus,
    pub(crate) initial_points: Vec<usize>,
    pub(crate) weights: G,
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
