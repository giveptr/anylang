use crate::engine::unity::serial::Container;
use crate::engine::unity::{bundle, serial};
use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

pub type Inside = BTreeMap<String, Vec<u8>>;

pub struct Opened {
    pub containers: Vec<Container>,
    pub inside: Inside,
}

pub fn opened_from(holder: &str, at: &Path) -> Result<Opened> {
    match packed(at)? {
        None => alone(holder, at),
        Some(raw) => {
            let held = bundle::read(&raw)?;
            let (opened, inside) = apart(&held);

            Ok(Opened {
                containers: opened.into_iter().map(|(_, one)| one).collect(),
                inside,
            })
        }
    }
}

pub fn harvested_from(holder: &str, at: &Path) -> Result<Opened> {
    match packed(at)? {
        None => alone(holder, at),
        Some(raw) => {
            let held = bundle::open(&raw)?;

            Ok(Opened {
                containers: named_containers(&held),
                inside: BTreeMap::new(),
            })
        }
    }
}

fn packed(at: &Path) -> Result<Option<Vec<u8>>> {
    let mut head = [0u8; 8];
    let read = File::open(at)
        .and_then(|mut file| file.read(&mut head))
        .with_context(|| format!("reading {}", at.display()))?;

    if !head[..read].starts_with(bundle::MAGIC) {
        return Ok(None);
    }

    fs::read(at)
        .with_context(|| format!("reading {}", at.display()))
        .map(Some)
}

fn alone(holder: &str, at: &Path) -> Result<Opened> {
    Ok(Opened {
        containers: vec![serial::open_at(at, holder)?],
        inside: BTreeMap::new(),
    })
}

fn named_containers(held: &bundle::Bundle) -> Vec<Container> {
    held.nodes
        .iter()
        .filter_map(|node| serial::open_told(&node.body, &held.revision, &node.name).ok())
        .collect()
}

pub fn apart(held: &bundle::Bundle) -> (Vec<(usize, Container)>, Inside) {
    let mut opened = Vec::new();
    let mut inside = BTreeMap::new();

    for (which, node) in held.nodes.iter().enumerate() {
        match serial::open_told(&node.body, &held.revision, &node.name) {
            Ok(one) => opened.push((which, one)),
            Err(_) => {
                inside.insert(node.name.clone(), node.body.clone());
            }
        }
    }

    (opened, inside)
}
