use crate::engine::sheet;
use crate::engine::unity::serial::Object;
use crate::engine::unity::{Harvest, format, naming, serial};
use anyhow::{Result, bail};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

pub const NAME: &str = "text_asset";
pub const KIND: &str = "TextAsset";

const NAMED: &str = "m_Name";
const SCRIPT: &str = "m_Script";

pub struct Found {
    pub node: usize,
    pub path_id: i64,
    pub stem: String,
    pub body: String,
}

pub fn take(holder: &str, nodes: &[&[Object]]) -> Result<Vec<Harvest>> {
    let mut out = Vec::new();

    for one in scripts_across(nodes) {
        let mut byfield: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
        for piece in format::pieces(&one.body) {
            byfield
                .entry(piece.stem)
                .or_default()
                .push((piece.path, piece.text));
        }

        for (stem, lines) in byfield {
            out.push(Harvest {
                at: PathBuf::from(NAME)
                    .join(holder)
                    .join(naming::named(&stem, one.path_id))
                    .join(format!("{}.{}", one.stem, sheet::SUFFIX)),
                lines: lines.len() as u32,
                body: sheet::write(lines)?,
            });
        }
    }

    Ok(out)
}

pub fn scripts_in(objects: &[Object]) -> Vec<Found> {
    scripts_across(&[objects])
}

pub fn scripts_across(nodes: &[&[Object]]) -> Vec<Found> {
    let found: Vec<(usize, i64, String, String)> = nodes
        .iter()
        .enumerate()
        .flat_map(|(node, objects)| {
            objects
                .iter()
                .filter(|object| object.class_id == serial::TEXT_ASSET)
                .filter_map(move |object| {
                    let (name, body) = split_asset(object)?;
                    Some((node, object.path_id, name, body))
                })
        })
        .collect();

    let mut times: BTreeMap<String, usize> = BTreeMap::new();
    for (_, path_id, name, _) in &found {
        *times.entry(naming::named(name, *path_id)).or_default() += 1;
    }
    let shared: BTreeSet<String> = times
        .into_iter()
        .filter(|(_, seen)| *seen > 1)
        .map(|(name, _)| name)
        .collect();

    found
        .into_iter()
        .map(|(node, path_id, name, body)| {
            let plain = naming::named(&name, path_id);

            Found {
                stem: if shared.contains(&plain) {
                    naming::named(&format!("{name}#{node}#{path_id}"), path_id)
                } else {
                    plain
                },
                node,
                path_id,
                body,
            }
        })
        .collect()
}

pub fn written(object: &Object, script: &str) -> Result<Vec<u8>> {
    let mut value = object
        .value()
        .ok_or_else(|| anyhow::anyhow!("asset {} no longer reads by its shape", object.path_id))?;

    if !value.put(SCRIPT, script) {
        bail!("asset {} holds no {SCRIPT} to write to", object.path_id);
    }

    object.written(&value)
}

fn split_asset(object: &Object) -> Option<(String, String)> {
    let value = object.value()?;

    Some((value.field(NAMED)?.text()?, value.field(SCRIPT)?.text()?))
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::engine::unity::fake;

    const SCENE: &str = "Peter\nClass was different today.\n\nWalter\nGood Morning Class.\n\n";

    fn packed(name: &str, script: &str) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(name.len() as u32).to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        while out.len() % 4 != 0 {
            out.push(0);
        }
        out.extend_from_slice(&(script.len() as u32).to_le_bytes());
        out.extend_from_slice(script.as_bytes());

        out
    }

    #[test]
    fn two_assets_sharing_a_name_both_survive() {
        let raw = fake::forge(&[
            (11, "scene001_shop_closed", SCENE),
            (22, "scene001_shop_closed", SCENE),
            (33, "scene002", SCENE),
        ]);

        let found = scripts_in(
            &serial::open(&raw, "")
                .expect("a container that opens")
                .objects,
        );
        let stems: Vec<&str> = found.iter().map(|one| one.stem.as_str()).collect();

        assert_eq!(
            stems,
            [
                "scene001_shop_closed#0#11",
                "scene001_shop_closed#0#22",
                "scene002"
            ],
            "a shared name has to keep the two apart, or staging one overwrites the other"
        );
    }

    #[test]
    fn two_names_the_filesystem_cannot_tell_apart_are_kept_apart() {
        let raw = fake::forge(&[(11, "day:1", SCENE), (22, "day_1", SCENE)]);

        let found = scripts_in(
            &serial::open(&raw, "")
                .expect("a container that opens")
                .objects,
        );
        let stems: Vec<&str> = found.iter().map(|one| one.stem.as_str()).collect();

        assert_eq!(
            stems,
            ["day_1#0#11", "day_1#0#22"],
            "both names sanitize to one filename, so without the split the second sheet \
             overwrites the first"
        );
    }

    #[test]
    fn a_text_asset_gives_up_its_name_and_its_script() {
        let object = Object::forged(serial::TEXT_ASSET, 11, packed("talk", SCENE));
        let (name, script) = split_asset(&object).expect("a name and a script");
        assert_eq!(name, "talk");
        assert_eq!(
            script, SCENE,
            "the name is what the sheet is filed under and the script is what the reader \
             translates, so reading them the wrong way round loses both"
        );
    }
}
