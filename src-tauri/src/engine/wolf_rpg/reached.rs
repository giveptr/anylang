use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound;

#[derive(Debug)]
struct Written {
    rank: u8,
    named: String,
    spot: String,
}

#[derive(Debug, Default)]
pub struct Reached {
    coded: BTreeSet<String>,
    parts: BTreeSet<String>,
    named: BTreeSet<String>,
    longest: usize,
    missed: bool,
    planned: BTreeSet<String>,
    keys: BTreeSet<String>,
    homes: BTreeMap<String, Written>,
    apart: BTreeSet<u32>,
    handed: BTreeSet<String>,
}

impl Reached {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn codes(&mut self, name: &str) {
        self.coded.insert(name.to_string());
    }

    pub fn hardcoded(&self, text: &str) -> bool {
        self.coded.contains(text)
    }

    pub fn ships(&mut self, part: &str) {
        self.parts.insert(part.to_ascii_lowercase());
    }

    pub fn a_part(&self, text: &str) -> bool {
        self.parts.contains(&text.to_ascii_lowercase())
    }

    pub fn builds(&self, token: &str) -> bool {
        let token = token.to_ascii_lowercase();

        self.parts
            .range::<str, _>((Bound::Included(token.as_str()), Bound::Unbounded))
            .take_while(|one| one.starts_with(token.as_str()))
            .any(|one| one[token.len()..].chars().all(|held| held.is_ascii_digit()))
    }

    pub fn missed_a_script(&mut self) {
        self.missed = true;
    }

    pub fn read_them_all(&self) -> bool {
        !self.missed
    }

    pub fn homing(&mut self, text: &str, rank: u8, named: &str, spot: &str) {
        if self.homes.get(text).is_some_and(|held| held.rank <= rank) {
            return;
        }

        self.homes.insert(
            text.to_string(),
            Written {
                rank,
                named: named.to_string(),
                spot: spot.to_string(),
            },
        );
    }

    pub fn written_down(&self, text: &str) -> bool {
        self.homes.contains_key(text)
    }

    pub fn at_home(&self, text: &str, named: &str, spot: &str) -> bool {
        self.homes
            .get(text)
            .is_some_and(|held| held.named == named && held.spot == spot)
    }

    pub fn plans(&mut self, name: &str) {
        self.planned.insert(name.trim().to_string());
    }

    pub fn a_plan_name(&self, text: &str) -> bool {
        self.planned.contains(text.trim())
    }

    pub fn keyed_by(&mut self, text: &str) {
        self.keys.insert(text.to_string());
    }

    pub fn a_name(&self, text: &str) -> bool {
        self.keys.contains(text)
    }

    pub fn keeps(&mut self, name: &str) {
        self.longest = self.longest.max(name.len());
        self.named.insert(name.to_ascii_lowercase());
    }

    pub fn kept(&self, name: &str) -> bool {
        name.len() <= self.longest && self.named.contains(&name.to_ascii_lowercase())
    }

    pub fn takes_apart(&mut self, which: u32) {
        self.apart.insert(which);
    }

    pub fn read_apart(&self, which: u32) -> bool {
        self.apart.contains(&which)
    }

    pub fn hands(&mut self, token: &str) {
        self.handed.insert(token.to_string());
    }

    pub fn handed(&self, text: &str) -> bool {
        self.handed.contains(text)
    }
}
