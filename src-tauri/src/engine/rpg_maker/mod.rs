use crate::engine::{Offer, TranslationUnit};
#[cfg(test)]
mod fixture;
mod harvest;
mod js;
mod pictures;
mod rgss;
mod text;

use crate::engine::Engine;
use std::path::Path;

pub const STEPS: [&str; 3] = [
    "Reading the game data",
    "Taking the text in",
    "Listing the pictures",
];

pub fn forget() {
    js::forget();
}

pub fn detect(dir: &Path) -> Option<Box<dyn Engine>> {
    js::detect(dir).or_else(|| rgss::detect(dir))
}

pub fn refused(dir: &Path) -> Option<String> {
    rgss::refused(dir)
}

#[derive(Debug)]
pub struct Gathered<S> {
    units: Vec<TranslationUnit>,
    spots: Vec<S>,
}

impl<S> Default for Gathered<S> {
    fn default() -> Self {
        Self {
            units: Vec::new(),
            spots: Vec::new(),
        }
    }
}

impl<S> Gathered<S> {
    pub fn take(&mut self, text: &str, spot: S, offer: Offer) {
        self.units.push(TranslationUnit {
            id: self.units.len() as u32,
            offer,
            text: text.to_string(),
        });
        self.spots.push(spot);
    }

    pub fn done(self) -> (Vec<TranslationUnit>, Vec<S>) {
        (self.units, self.spots)
    }
}
