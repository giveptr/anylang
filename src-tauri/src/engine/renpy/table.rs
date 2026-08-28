use crate::engine::renpy::{WORKING, script, scripts};
use anyhow::{Context, Result};
use std::collections::HashSet;
use std::fs;
use std::path::Path;

pub fn added(source: &Path, named: &str, found: Vec<String>) -> Result<u32> {
    let mut already: HashSet<String> = HashSet::new();
    for path in scripts(source) {
        let text =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;

        already.extend(script::keys(&text));
    }

    let wanted: Vec<String> = found
        .into_iter()
        .filter(|one| !already.contains(one))
        .collect();

    if wanted.is_empty() {
        return Ok(0);
    }

    let mut body = format!("translate {WORKING} strings:\n");
    for one in &wanted {
        body.push_str(&format!("\n    old \"{one}\"\n    new \"{one}\"\n"));
    }

    let at = source.join(named);
    fs::write(&at, body).with_context(|| format!("writing {}", at.display()))?;

    Ok(wanted.len() as u32)
}
