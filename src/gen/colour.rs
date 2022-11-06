use super::{trees::Neighbours, GenColour};
use ::image::{Pixel, Rgb};
use ::rand::prelude::Rng;
use ::rand_xoshiro::Xoshiro128PlusPlus;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub(crate) struct TestGen;

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct RandColour {
    pub(crate) step_size: u8,
    pub(crate) rng: Xoshiro128PlusPlus,
}

impl RandColour {
    pub(crate) fn rand_channel(&mut self, old: u8, step_size: u8) -> u8 {
        let max = old.saturating_add(step_size);
        let min = old.saturating_sub(step_size);
        self.rng.gen_range(min..max)
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
