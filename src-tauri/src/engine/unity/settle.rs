use crate::canvas::Canvas;
use crate::engine::sheet;
use crate::engine::unity::opened::Inside;
use crate::engine::unity::seal::Sealed;
use crate::engine::unity::serial::{Container, Object};
use crate::engine::unity::{
    Known, bundle, fonts, format, localization, mono_behaviour, opened, patch, pictures, serial,
    text_asset,
};
use anyhow::Result;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

#[derive(Default)]
pub struct Staged {
    pub assets: Vec<(String, PathBuf)>,
    pub behaviours: Vec<(String, PathBuf)>,
}

impl Staged {
    pub fn is_empty(&self) -> bool {
        self.assets.is_empty() && self.behaviours.is_empty()
    }
}

pub type Sidecars = BTreeMap<String, Vec<pictures::Written>>;

pub enum Done {
    Nothing,
    Ready {
        bytes: Vec<u8>,
        beside: Sidecars,
        pieces: usize,
        faces: Vec<String>,
        pictures: usize,
        sealed: Option<Sealed>,
        dropped: Vec<String>,
    },
    Refused {
        why: String,
        lost: usize,
    },
}

pub struct Settling<'a> {
    pub staged: &'a Staged,
    pub staged_root: &'a Path,
    pub known: &'a Known,
    pub picked: &'a fonts::Chosen,
    pub pictures: &'a BTreeMap<(usize, i64), Canvas>,
    pub folder: &'a Path,
    pub store: &'a Path,
    pub game_dir: &'a Path,
}

pub enum Loaded {
    Bundled {
        bundle: bundle::Bundle,
        nodes: Vec<(usize, Container)>,
        held: Inside,
    },
    Alone(Container),
}

pub fn load(original: &[u8], holder: &str) -> Option<Loaded> {
    if original.starts_with(bundle::MAGIC) {
        let bundle = bundle::read(original).ok()?;
        if bundle.nodes.is_empty() {
            return None;
        }

        let (nodes, held) = opened::apart(&bundle);

        return Some(Loaded::Bundled {
            bundle,
            nodes,
            held,
        });
    }

    serial::open(original, holder).ok().map(Loaded::Alone)
}

pub fn settle(original: Vec<u8>, loaded: Loaded, at: &Settling<'_>) -> Done {
    let mut per_node: Vec<(Option<usize>, Swaps)> = Vec::new();
    let mut dropped: Vec<String> = Vec::new();
    let mut parsed = None;

    match loaded {
        Loaded::Bundled {
            bundle,
            nodes: opened,
            held,
        } => {
            let views: Vec<&[Object]> = opened
                .iter()
                .map(|(_, one)| one.objects.as_slice())
                .collect();
            let assets = text_asset::scripts_across(&views);

            let every: Vec<&Container> = opened.iter().map(|(_, one)| one).collect();
            let shared = mono_behaviour::shared_ids(&every);

            let inside = pictures::Nearby {
                held: &held,
                folder: at.folder,
                store: at.store,
                game_dir: at.game_dir,
            };

            let mut ends = BTreeMap::new();
            for (which, (node, one)) in opened.iter().enumerate() {
                let mine: Vec<&text_asset::Found> =
                    assets.iter().filter(|one| one.node == which).collect();

                let swaps = at.swaps_for(
                    &mine,
                    one,
                    which,
                    &shared,
                    &inside,
                    Keeping {
                        dropped: &mut dropped,
                        ends: &mut ends,
                    },
                );
                if !swaps.is_empty() {
                    per_node.push((Some(*node), swaps));
                }
            }

            parsed = Some(bundle);
        }
        Loaded::Alone(one) => {
            let assets = text_asset::scripts_in(&one.objects);
            let mine: Vec<&text_asset::Found> = assets.iter().collect();

            let empty = BTreeMap::new();
            let inside = pictures::Nearby {
                held: &empty,
                folder: at.folder,
                store: at.store,
                game_dir: at.game_dir,
            };

            let swaps = at.swaps_for(
                &mine,
                &one,
                0,
                &BTreeSet::new(),
                &inside,
                Keeping {
                    dropped: &mut dropped,
                    ends: &mut BTreeMap::new(),
                },
            );
            if !swaps.is_empty() {
                per_node.push((None, swaps));
            }
        }
    }

    let pieces: usize = per_node.iter().map(|(_, swaps)| swaps.pieces).sum();
    let pictures: usize = per_node.iter().map(|(_, swaps)| swaps.pictures).sum();
    let faces: Vec<String> = per_node
        .iter()
        .flat_map(|(_, swaps)| swaps.faces.iter().cloned())
        .collect();

    let lost = pieces + pictures + faces.len() + dropped.len();

    if pieces == 0 && pictures == 0 && faces.is_empty() {
        return if dropped.is_empty() {
            Done::Nothing
        } else {
            Done::Refused {
                lost,
                why: dropped.join("; "),
            }
        };
    }

    match rebuild(&original, parsed, per_node) {
        Ok((bytes, beside, sealed)) => Done::Ready {
            bytes,
            beside,
            pieces,
            faces,
            pictures,
            sealed,
            dropped,
        },
        Err(why) => Done::Refused {
            why: format!("{why:#}"),
            lost,
        },
    }
}

pub fn laid_into(room: &mut Vec<u8>, held: &pictures::Written) -> Result<()> {
    let at = usize::try_from(held.at)?;
    let end = at.checked_add(held.body.len()).ok_or_else(|| {
        anyhow::anyhow!(
            "a picture of {} bytes reaches past any file",
            held.body.len()
        )
    })?;

    if room.len() < end {
        room.resize(end, 0);
    }
    room[at..end].copy_from_slice(&held.body);

    Ok(())
}

#[derive(Default)]
struct Swaps {
    bodies: BTreeMap<i64, Vec<u8>>,
    beside: Sidecars,
    faces: Vec<String>,
    pictures: usize,
    pieces: usize,
}

impl Swaps {
    fn is_empty(&self) -> bool {
        self.bodies.is_empty()
    }
}

struct Keeping<'a> {
    dropped: &'a mut Vec<String>,
    ends: &'a mut BTreeMap<String, u64>,
}

struct Patching<'a> {
    swaps: Swaps,
    dropped: &'a mut Vec<String>,
}

impl Patching<'_> {
    fn staged_lines(
        &mut self,
        staged: &[(String, PathBuf)],
        prefixed: bool,
        wanted: impl Fn(&str) -> bool,
    ) -> BTreeMap<String, BTreeMap<String, String>> {
        let mut out: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();

        for (stem, path) in staged {
            let text = match fs::read_to_string(path) {
                Ok(text) => text,
                Err(error) => {
                    self.dropped.push(format!("{stem}: {error}"));
                    continue;
                }
            };

            let lines = match sheet::lines(&text) {
                Ok(lines) => lines,
                Err(error) => {
                    self.dropped.push(format!("{stem}: {error}"));
                    continue;
                }
            };

            for (key, body) in lines {
                let (whose, spot) = match prefixed.then(|| key.split_once('/')).flatten() {
                    Some((whose, spot)) => (whose.to_string(), spot.to_string()),
                    None => (stem.clone(), key),
                };

                if !wanted(&whose) {
                    continue;
                }

                out.entry(whose).or_default().insert(spot, body);
            }
        }

        out
    }

    fn assets(&mut self, container: &Container, assets: &[&text_asset::Found], staged: &Staged) {
        let mut byname: BTreeMap<&str, &text_asset::Found> = BTreeMap::new();
        let mut doubled: Vec<&str> = Vec::new();
        for one in assets {
            if byname.insert(one.stem.as_str(), *one).is_some() {
                doubled.push(one.stem.as_str());
            }
        }
        for stem in doubled {
            byname.remove(stem);
        }

        for (stem, lines) in
            self.staged_lines(&staged.assets, false, |stem| byname.contains_key(stem))
        {
            let Some(one) = byname.get(stem.as_str()) else {
                continue;
            };

            let Some(object) = container
                .objects
                .iter()
                .find(|held| held.path_id == one.path_id)
            else {
                continue;
            };

            match format::put_back(&one.body, &lines) {
                Ok(script) if script == one.body => {}
                Ok(script) => match text_asset::written(object, &script) {
                    Ok(body) => {
                        self.swaps.bodies.insert(one.path_id, body);
                        self.swaps.pieces += 1;
                    }
                    Err(error) => self.dropped.push(format!("{stem}: {error:#}")),
                },
                Err(error) => self.dropped.push(format!("{stem}: {error:#}")),
            }
        }
    }

    fn behaviours(
        &mut self,
        container: &Container,
        which: usize,
        shared: &BTreeSet<i64>,
        staged: &Staged,
        known: &Known,
    ) {
        let bystem: BTreeMap<String, (&Object, Vec<mono_behaviour::Piece>)> = container
            .objects
            .iter()
            .filter_map(|object| {
                let kind = known.classes.of(container, object);
                let pieces = mono_behaviour::pieces_in(
                    container,
                    object,
                    kind,
                    &known.assemblies,
                    &known.books,
                );

                (!pieces.is_empty()).then(|| {
                    (
                        mono_behaviour::id_of(object, which, shared),
                        (object, pieces),
                    )
                })
            })
            .collect();

        for (stem, lines) in
            self.staged_lines(&staged.behaviours, true, |stem| bystem.contains_key(stem))
        {
            let Some((object, pieces)) = bystem.get(&stem) else {
                continue;
            };

            match mono_behaviour::put_back(object, pieces, &lines) {
                Ok(body) => {
                    self.swaps.bodies.insert(object.path_id, body);
                    self.swaps.pieces += 1;
                }
                Err(error) => self.dropped.push(format!("{stem}: {error:#}")),
            }
        }
    }

    fn tables(&mut self, container: &Container, staged_root: &Path, known: &Known) {
        for object in &container.objects {
            let kind = known.classes.of(container, object);
            let Some(table) =
                localization::table_of(container, object, kind, &known.assemblies, &known.books)
            else {
                continue;
            };
            let at = table.sheet();

            let text = match fs::read_to_string(staged_root.join(&at)) {
                Ok(text) => text,
                Err(error) if error.kind() == ErrorKind::NotFound => continue,
                Err(error) => {
                    self.dropped.push(format!("{}: {error}", at.display()));
                    continue;
                }
            };

            let lines = match sheet::lines(&text) {
                Ok(lines) => lines,
                Err(error) => {
                    self.dropped.push(format!("{}: {error}", at.display()));
                    continue;
                }
            };

            match localization::put_back(object, table, &lines) {
                Ok(body) => {
                    self.swaps.bodies.insert(object.path_id, body);
                    self.swaps.pieces += 1;
                }
                Err(error) => self.dropped.push(format!("{}: {error:#}", at.display())),
            }
        }
    }

    fn pictures(
        &mut self,
        container: &Container,
        which: usize,
        picked: &BTreeMap<(usize, i64), Canvas>,
        beside: &dyn pictures::Beside,
        ends: &mut BTreeMap<String, u64>,
    ) {
        if picked.is_empty() {
            return;
        }

        let mine: BTreeMap<i64, &Canvas> = picked
            .iter()
            .filter(|((node, _), _)| *node == which)
            .filter(|((_, path_id), _)| container.objects.iter().any(|one| one.path_id == *path_id))
            .map(|((_, path_id), held)| (*path_id, held))
            .collect();
        if mine.is_empty() {
            return;
        }

        let done = pictures::swapped(container, &mine, beside, ends);

        self.dropped.extend(done.dropped);
        self.swaps.pictures += done.pictures;
        for (path_id, body) in done.bodies {
            self.swaps.bodies.insert(path_id, body);
        }
        for (name, written) in done.beside {
            self.swaps.beside.entry(name).or_default().extend(written);
        }
    }

    fn faces(&mut self, container: &Container, picked: &fonts::Chosen) {
        if picked.is_empty() {
            return;
        }

        for object in &container.objects {
            let Some(name) = fonts::face_of(object) else {
                continue;
            };
            let Some(drawn) = picked.for_name(&name) else {
                continue;
            };

            match fonts::swapped(object, drawn) {
                Some(body) => {
                    self.swaps.bodies.insert(object.path_id, body);
                    self.swaps.faces.push(name);
                }
                None => self.dropped.push(format!(
                    "{name}: this font sits in the object in a shape this writer cannot put one \
                     back into"
                )),
            }
        }
    }
}

impl Settling<'_> {
    fn swaps_for(
        &self,
        assets: &[&text_asset::Found],
        container: &Container,
        which: usize,
        shared: &BTreeSet<i64>,
        beside: &dyn pictures::Beside,
        keeping: Keeping<'_>,
    ) -> Swaps {
        let mut patching = Patching {
            swaps: Swaps::default(),
            dropped: keeping.dropped,
        };

        patching.assets(container, assets, self.staged);
        patching.behaviours(container, which, shared, self.staged, self.known);
        patching.tables(container, self.staged_root, self.known);
        patching.faces(container, self.picked);
        patching.pictures(container, which, self.pictures, beside, keeping.ends);

        patching.swaps
    }
}

fn rebuild(
    original: &[u8],
    bundle: Option<bundle::Bundle>,
    per_node: Vec<(Option<usize>, Swaps)>,
) -> Result<(Vec<u8>, Sidecars, Option<Sealed>)> {
    let Some(mut bundle) = bundle else {
        let (_, swaps) = per_node
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("nothing matched the text that was staged"))?;

        return Ok((patch::rewrite(original, &swaps.bodies)?, swaps.beside, None));
    };

    for (at, swaps) in per_node {
        let at =
            at.ok_or_else(|| anyhow::anyhow!("a bundle needs the node each swap came from"))?;

        for (name, written) in swaps.beside {
            let held = bundle
                .nodes
                .iter_mut()
                .find(|node| node.name == name)
                .ok_or_else(|| anyhow::anyhow!("{name} is not in this bundle any more"))?;

            for one in written {
                laid_into(&mut held.body, &one)?;
            }
        }

        let node = bundle
            .nodes
            .get_mut(at)
            .ok_or_else(|| anyhow::anyhow!("node {at} left the bundle"))?;

        node.body = patch::rewrite(&node.body, &swaps.bodies)?;
    }

    let was = bundle.crc;
    let packed = bundle.pack()?;

    Ok((
        packed.bytes,
        BTreeMap::new(),
        Some(Sealed {
            was,
            now: packed.crc,
        }),
    ))
}
