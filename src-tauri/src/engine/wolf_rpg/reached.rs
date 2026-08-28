use std::collections::BTreeSet;

#[derive(Debug, Default)]
pub struct Reached {
    coded: BTreeSet<String>,
    parts: BTreeSet<String>,
    named: BTreeSet<String>,
    longest: usize,
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
        self.parts.insert(part.to_string());
    }

    pub fn builds(&self, token: &str) -> bool {
        self.parts
            .range(token.to_string()..)
            .take_while(|one| one.starts_with(token))
            .any(|one| one[token.len()..].chars().all(|held| held.is_ascii_digit()))
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
