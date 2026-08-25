use std::collections::BTreeSet;

#[derive(Debug, Default)]
pub struct Reached {
    coded: BTreeSet<String>,
    parts: BTreeSet<String>,
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
