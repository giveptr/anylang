use crate::canvas::Canvas;
use crate::engine::pictures::{Ledger, Shot};
use crate::engine::unity::opened::{self, Opened};
use crate::engine::unity::serial::{Container, Object, Value};
use crate::engine::unity::texture::{Held, Picture};
use crate::engine::unity::{atlas, data_dir, mono_script, serial, sprite, texture};
use crate::store::Stamp;
use crate::{backup, store};
use anyhow::{Context, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::iter;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const AS_PNG: &str = "png";
const TEXTURE: &str = "Texture2D";
const SPRITE: &str = "Sprite";

const ASSET_BUNDLE: i32 = 142;
const RESOURCE_MANAGER: i32 = 147;

const ROOMY: usize = 192 * 1024 * 1024;

type Page = Arc<Mutex<Option<Arc<Canvas>>>>;

struct Kept {
    holder: String,
    stamp: Stamp,
    opened: Arc<Opened>,
    drawn: Vec<(String, Page)>,
    sheets: Vec<(usize, Arc<atlas::Sheets>)>,
}

static KEPT: Mutex<Option<Kept>> = Mutex::new(None);

pub fn forget() {
    if let Ok(mut kept) = KEPT.lock() {
        *kept = None;
    }
}

fn container_at(game_dir: &Path, holder: &str) -> Result<PathBuf> {
    data_dir(game_dir)
        .map(|data| data.join(holder))
        .filter(|at| at.is_file())
        .or_else(|| Some(game_dir.join(holder)).filter(|at| at.is_file()))
        .with_context(|| format!("{holder} is not in this game any more"))
}

fn opened_at(game_dir: &Path, store: &Path, holder: &str) -> Result<Arc<Opened>> {
    let at = container_at(game_dir, holder)?;
    let stamp = store::stamp_of(&at);

    if let Ok(kept) = KEPT.lock()
        && let Some(held) = kept.as_ref()
        && held.holder == holder
        && held.stamp == stamp
    {
        return Ok(Arc::clone(&held.opened));
    }

    let reading = backup::original_at_now(store, game_dir, &at)?;
    let opened = Arc::new(opened::opened_from(holder, &reading)?);

    if let Ok(mut kept) = KEPT.lock() {
        *kept = Some(Kept {
            holder: holder.to_string(),
            stamp,
            opened: Arc::clone(&opened),
            drawn: Vec::new(),
            sheets: Vec::new(),
        });
    }

    Ok(opened)
}

fn with_kept<R>(opened: &Arc<Opened>, then: impl FnOnce(&mut Kept) -> R) -> Option<R> {
    let mut kept = KEPT.lock().ok()?;
    let held = kept
        .as_mut()
        .filter(|held| Arc::ptr_eq(&held.opened, opened))?;

    Some(then(held))
}

fn sheets_at(opened: &Arc<Opened>, which: usize) -> Arc<atlas::Sheets> {
    let built = || {
        Arc::new(
            opened
                .containers
                .get(which)
                .map(|one| atlas::Sheets::read(&one.objects))
                .unwrap_or_default(),
        )
    };

    with_kept(opened, |held| {
        match held.sheets.iter().find(|(at, _)| *at == which) {
            Some((_, sheets)) => Arc::clone(sheets),
            None => {
                let fresh = built();
                held.sheets.push((which, Arc::clone(&fresh)));

                fresh
            }
        }
    })
    .unwrap_or_else(built)
}

fn page_of(opened: &Arc<Opened>, at: &str) -> Option<Page> {
    with_kept(opened, |held| {
        match held.drawn.iter().find(|(named, _)| named == at) {
            Some((_, page)) => Arc::clone(page),
            None => {
                let page = Page::default();
                held.drawn.push((at.to_string(), Arc::clone(&page)));

                page
            }
        }
    })
}

fn room_for(page: &Page) -> usize {
    match page.try_lock() {
        Ok(held) => held.as_ref().map_or(0, |one| one.pixels.len()),
        Err(_) => 0,
    }
}

fn tidied() {
    let Ok(mut kept) = KEPT.lock() else {
        return;
    };
    let Some(held) = kept.as_mut() else {
        return;
    };

    let mut sized: Vec<usize> = held.drawn.iter().map(|(_, one)| room_for(one)).collect();
    let mut room: usize = sized.iter().sum();
    let mut oldest = 0;
    while room > ROOMY && oldest + 1 < held.drawn.len() {
        match sized[oldest] {
            0 => oldest += 1,
            gone => {
                held.drawn.remove(oldest);
                sized.remove(oldest);
                room -= gone;
            }
        }
    }
}

fn whole_of(
    opened: &Arc<Opened>,
    at: &str,
    draw: impl FnOnce() -> Result<Canvas>,
) -> Result<Arc<Canvas>> {
    let Some(page) = page_of(opened, at) else {
        return Ok(Arc::new(draw()?));
    };
    let Ok(mut held) = page.lock() else {
        return Ok(Arc::new(draw()?));
    };

    if let Some(one) = held.as_ref() {
        return Ok(Arc::clone(one));
    }

    let one = Arc::new(draw()?);
    *held = Some(Arc::clone(&one));
    drop(held);

    tidied();

    Ok(one)
}

pub fn drawn(game_dir: &Path, store: &Path, key: &str) -> Result<Arc<Canvas>> {
    let holder = holder_in(key).with_context(|| format!("{key} does not name a picture"))?;
    let opened = opened_at(game_dir, store, holder)?;
    let beside = container_at(game_dir, holder)?
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| game_dir.to_path_buf());

    let which = node_in(key);
    let sheets = sheets_at(&opened, which);

    let wants = opened.wants(key, &sheets).with_context(|| {
        format!("{holder} holds no picture this reader can draw for {key} on its own")
    })?;
    let of = match &wants {
        Wants::Whole(of) => *of,
        Wants::Cut(cut) => cut.of,
    };

    let page = key_of(holder, which, of);
    let whole = whole_of(&opened, &page, || {
        opened.whole_at(&beside, store, game_dir, which, of)
    })?;

    match wants {
        Wants::Whole(_) => Ok(whole),
        Wants::Cut(cut) => cut.cut(&whole).map(Arc::new),
    }
}

pub const LEDGER: Ledger = Ledger("unity-pictures.json");

pub fn key_of(holder: &str, which: usize, path_id: i64) -> String {
    format!("{holder}#{which}|{path_id}")
}

pub fn holder_in(key: &str) -> Option<&str> {
    let (head, _) = key.rsplit_once('|')?;

    Some(match head.rsplit_once('#') {
        Some((holder, which)) if which.parse::<usize>().is_ok() => holder,
        _ => head,
    })
}

pub fn node_in(key: &str) -> usize {
    key.rsplit_once('|')
        .and_then(|(head, _)| head.rsplit_once('#'))
        .and_then(|(_, which)| which.parse().ok())
        .unwrap_or(0)
}

enum Wants {
    Whole(i64),
    Cut(sprite::Cut),
}

impl Opened {
    pub fn shots(&self, holder: &str, named: &Named) -> Vec<Shot> {
        let mut out = Vec::new();

        for (which, one) in self.containers.iter().enumerate() {
            let mut pictures: Vec<(i64, Picture)> = Vec::new();
            let mut cuts: Vec<sprite::Cut> = Vec::new();
            let mut lost: Vec<(i64, Adrift)> = Vec::new();
            let sheets = atlas::Sheets::read(&one.objects);

            for object in &one.objects {
                match object.class_id {
                    serial::TEXTURE_2D => {
                        let value = object.value();
                        match value.as_ref().and_then(texture::pictured) {
                            Some(held) => pictures.push((object.path_id, held)),
                            None => lost.extend(
                                lost_texture(value.as_ref()).map(|held| (object.path_id, held)),
                            ),
                        }
                    }
                    serial::SPRITE => {
                        let value = object.value();
                        match value
                            .as_ref()
                            .and_then(|held| sprite::cut_in(held, &sheets, object.path_id))
                        {
                            Some(held) => cuts.push(held),
                            None => lost.extend(
                                lost_sprite(value.as_ref(), &sheets)
                                    .map(|held| (object.path_id, held)),
                            ),
                        }
                    }
                    _ => {}
                }
            }

            let pictured: BTreeMap<i64, &Picture> =
                pictures.iter().map(|(id, held)| (*id, held)).collect();
            let crowded = crowding(&cuts);
            cuts.sort_by_cached_key(|cut| drawn_from(&pictured, cut));

            for (path_id, held) in lost {
                out.push(Shot {
                    key: key_of(holder, which, path_id),
                    holder: holder.to_string(),
                    name: shown(&held.name, path_id),
                    kind: held.kind.to_string(),
                    atlas: String::new(),
                    wide: held.wide,
                    high: held.high,
                    format: String::new(),
                    saved_as: String::new(),
                    locked: Some(held.why),
                    drawable: false,
                    at: named.of(&one.name, path_id).unwrap_or_default().to_string(),
                });
            }

            for (path_id, held) in &pictures {
                let drawable = texture::draws(held.format);

                out.push(Shot {
                    key: key_of(holder, which, *path_id),
                    holder: holder.to_string(),
                    name: shown(&held.name, *path_id),
                    kind: TEXTURE.to_string(),
                    atlas: String::new(),
                    wide: held.wide as u32,
                    high: held.high as u32,
                    format: texture::called(held.format),
                    saved_as: AS_PNG.to_string(),
                    locked: (!drawable).then(|| unreadable(held.format)),
                    drawable,
                    at: named
                        .of(&one.name, *path_id)
                        .unwrap_or_default()
                        .to_string(),
                });
            }

            for cut in &cuts {
                let atlas = match cut.elsewhere {
                    true => None,
                    false => pictured.get(&cut.of),
                };

                let locked = if cut.elsewhere {
                    Some(
                        "the texture this sprite is cut from sits in another file, which this \
                         reader does not follow yet"
                            .to_string(),
                    )
                } else if atlas.is_none() {
                    Some("the texture this sprite is cut from was not read".to_string())
                } else if cut.turn.odd() {
                    Some(
                        "the packer turned this sprite inside its atlas, so a picture painted \
                         back would show up sideways"
                            .to_string(),
                    )
                } else if cut.alpha_apart {
                    Some(
                        "this game keeps this sprite's transparency in a second texture, which \
                         this reader does not paint back"
                            .to_string(),
                    )
                } else if crowded.contains(&cut.at) {
                    Some(format!(
                        "this sprite shares its spot in the atlas with another one, so painting \
                         it would draw over its neighbour. Open {} in the rail, save a copy, \
                         paint on that and put the whole atlas back instead",
                        atlas.map(|held| held.name.as_str()).unwrap_or("the atlas")
                    ))
                } else if let Some(held) = atlas.filter(|held| !texture::draws(held.format)) {
                    Some(unreadable(held.format))
                } else if !atlas.is_some_and(|held| cut.inside(held.wide, held.high)) {
                    Some("this sprite does not sit inside its own atlas".to_string())
                } else {
                    None
                };

                out.push(Shot {
                    key: key_of(holder, which, cut.at),
                    holder: holder.to_string(),
                    name: shown(&cut.name, cut.at),
                    kind: SPRITE.to_string(),
                    atlas: atlas
                        .map(|held| shown(&held.name, cut.of))
                        .unwrap_or_default(),
                    wide: cut.wide as u32,
                    high: cut.high as u32,
                    format: atlas
                        .map(|held| texture::called(held.format))
                        .unwrap_or_default(),
                    saved_as: AS_PNG.to_string(),
                    locked,
                    drawable: atlas.is_some_and(|held| {
                        texture::draws(held.format) && cut.inside(held.wide, held.high)
                    }),
                    at: named.of(&one.name, cut.at).unwrap_or_default().to_string(),
                });
            }
        }

        out
    }

    fn wants(&self, key: &str, sheets: &atlas::Sheets) -> Option<Wants> {
        let path_id = named_in(key)?;
        let object = self.holding(node_in(key), path_id)?;

        if texture::picture_of(object).is_some() {
            return Some(Wants::Whole(path_id));
        }

        sprite::cut_of(object, sheets)
            .filter(|cut| !cut.elsewhere)
            .map(Wants::Cut)
    }

    fn holding(&self, which: usize, path_id: i64) -> Option<&Object> {
        self.containers
            .get(which)?
            .objects
            .iter()
            .find(|held| held.path_id == path_id)
    }

    fn whole_at(
        &self,
        beside: &Path,
        store: &Path,
        game_dir: &Path,
        which: usize,
        of: i64,
    ) -> Result<Canvas> {
        let one = self
            .containers
            .get(which)
            .ok_or_else(|| anyhow::anyhow!("this file holds no container {which}"))?;

        whole(one, of, &self.beside(beside, store, game_dir))
    }

    fn beside<'b>(&'b self, folder: &'b Path, store: &'b Path, game_dir: &'b Path) -> Nearby<'b> {
        Nearby {
            held: &self.inside,
            folder,
            store,
            game_dir,
        }
    }
}

pub struct Nearby<'a> {
    pub held: &'a BTreeMap<String, Vec<u8>>,
    pub folder: &'a Path,
    pub store: &'a Path,
    pub game_dir: &'a Path,
}

impl Nearby<'_> {
    fn shipped_at(&self, name: &str) -> Option<PathBuf> {
        backup::original_at_now(self.store, self.game_dir, &self.folder.join(name)).ok()
    }
}

impl Beside for Nearby<'_> {
    fn part(&self, name: &str, at: u64, size: usize) -> Option<Vec<u8>> {
        if let Some(held) = self.held.get(name) {
            let from = usize::try_from(at).ok()?;

            return held.get(from..from.checked_add(size)?).map(<[u8]>::to_vec);
        }

        let mut file = File::open(self.shipped_at(name)?).ok()?;
        file.seek(SeekFrom::Start(at)).ok()?;

        let mut raw = vec![0u8; size];
        file.read_exact(&mut raw).ok()?;

        Some(raw)
    }

    fn size(&self, name: &str) -> Option<u64> {
        match self.held.get(name) {
            Some(held) => Some(held.len() as u64),
            None => fs::metadata(self.shipped_at(name)?)
                .ok()
                .map(|held| held.len()),
        }
    }
}

pub fn whole(container: &Container, of: i64, beside: &dyn Beside) -> Result<Canvas> {
    let object = container
        .objects
        .iter()
        .find(|held| held.path_id == of)
        .ok_or_else(|| anyhow::anyhow!("{} holds no object at {of}", container.name))?;

    let held = texture::picture_of(object).ok_or_else(|| {
        anyhow::anyhow!(
            "the object at {of} in {} is not a picture this reader knows",
            container.name
        )
    })?;

    let pixels = match &held.held {
        Held::Inline(_) => held
            .inside(object)
            .ok_or_else(|| anyhow::anyhow!("{} holds no pixels", held.name))?,
        Held::Streamed { path, at, size } => {
            let name = path.rsplit('/').next().unwrap_or(path);

            beside.part(name, *at, *size).with_context(|| {
                format!(
                    "reading the {size} byte(s) {name} holds for {} at {at}",
                    held.name
                )
            })?
        }
    };

    held.drawn(&pixels, &container.built_by)
}

pub trait Beside {
    fn part(&self, name: &str, at: u64, size: usize) -> Option<Vec<u8>>;

    fn size(&self, name: &str) -> Option<u64>;
}

pub struct Written {
    pub at: u64,
    pub body: Vec<u8>,
}

#[derive(Default)]
pub struct Swapped {
    pub bodies: BTreeMap<i64, Vec<u8>>,
    pub beside: BTreeMap<String, Vec<Written>>,
    pub pictures: usize,
    pub dropped: Vec<String>,
}

pub fn swapped(
    container: &Container,
    picks: &BTreeMap<i64, &Canvas>,
    beside: &dyn Beside,
    ends: &mut BTreeMap<String, u64>,
) -> Swapped {
    let mut painting: BTreeMap<i64, Vec<(Option<sprite::Cut>, &Canvas)>> = BTreeMap::new();
    let mut dropped = Vec::new();
    let mut pictures = 0;

    let sheets = atlas::Sheets::read(&container.objects);
    let crowded = crowding(&sprite::cuts_in(&container.objects, &sheets));

    for (path_id, held) in picks {
        let Some(object) = container.objects.iter().find(|one| one.path_id == *path_id) else {
            continue;
        };

        if texture::picture_of(object).is_some() {
            painting.entry(*path_id).or_default().push((None, held));
            continue;
        }

        if crowded.contains(path_id) {
            dropped.push(format!(
                "the sprite at {path_id} shares its spot in the atlas with another one, so \
                 painting it would draw over its neighbour. Put the whole atlas back instead"
            ));
            continue;
        }

        match sprite::cut_of(object, &sheets) {
            Some(cut) if !cut.elsewhere && !cut.turn.odd() && !cut.alpha_apart => {
                painting.entry(cut.of).or_default().push((Some(cut), held))
            }
            Some(cut) => dropped.push(format!(
                "{}: the packer turned this sprite, it keeps its transparency elsewhere or it \
                 lives in another container, so this writer will not paint over it",
                cut.shown()
            )),
            None => dropped.push(format!(
                "the object at {path_id} is neither a texture nor a sprite this reader knows"
            )),
        }
    }

    let mut bodies = BTreeMap::new();
    let mut written: BTreeMap<String, Vec<Written>> = BTreeMap::new();

    for (of, held) in painting {
        match painted(container, of, &held, beside, ends) {
            Ok((body, many, laid)) => {
                bodies.insert(of, body);
                pictures += many;

                if let Some((name, one)) = laid {
                    written.entry(name).or_default().push(one);
                }
            }
            Err(why) => dropped.push(format!("{why:#}")),
        }
    }

    Swapped {
        bodies,
        beside: written,
        pictures,
        dropped,
    }
}

type Laid = Option<(String, Written)>;

fn painted(
    container: &Container,
    of: i64,
    held: &[(Option<sprite::Cut>, &Canvas)],
    beside: &dyn Beside,
    ends: &mut BTreeMap<String, u64>,
) -> Result<(Vec<u8>, usize, Laid)> {
    let object = container
        .objects
        .iter()
        .find(|one| one.path_id == of)
        .ok_or_else(|| anyhow::anyhow!("no object in this container answers to {of}"))?;
    let picture = texture::picture_of(object)
        .ok_or_else(|| anyhow::anyhow!("the object at {of} is not a picture this reader knows"))?;

    let mut whole = match held.iter().find(|(cut, _)| cut.is_none()) {
        Some((_, drawn)) => drawn.scaled(picture.wide, picture.high)?,
        None => whole(container, of, beside)?,
    };
    let mut many = usize::from(held.iter().any(|(cut, _)| cut.is_none()));

    for (cut, drawn) in held {
        let Some(cut) = cut else { continue };

        let fitted = drawn.scaled(cut.wide, cut.high)?;
        cut.paint(&mut whole, &fitted)?;
        many += 1;
    }

    let payload = picture.payload(&whole)?;
    let Some((name, at, path)) = laid_out(&picture, &payload, beside, ends) else {
        return Ok((picture.written(object, &payload, None)?, many, None));
    };

    let body = picture.written(object, &payload, Some((&path, at)))?;

    Ok((
        body,
        many,
        Some((
            name,
            Written {
                at,
                body: payload.bytes,
            },
        )),
    ))
}

fn laid_out(
    picture: &Picture,
    payload: &texture::Payload,
    beside: &dyn Beside,
    ends: &mut BTreeMap<String, u64>,
) -> Option<(String, u64, String)> {
    let Held::Streamed { path, at, size } = &picture.held else {
        return None;
    };
    let name = path.rsplit('/').next().unwrap_or(path).to_string();

    if payload.bytes.len() == *size {
        return Some((name, *at, path.clone()));
    }

    let end = match ends.get(&name) {
        Some(end) => *end,
        None => beside.size(&name)?,
    };
    ends.insert(name.clone(), end + payload.bytes.len() as u64);

    Some((name, end, path.clone()))
}

pub fn named_in(key: &str) -> Option<i64> {
    key.rsplit_once('|')
        .and_then(|(_, path_id)| path_id.parse().ok())
}

#[derive(Default)]
pub struct Named {
    paths: BTreeMap<(String, i64), String>,
}

impl Named {
    pub fn learn(&mut self, container: &Container) {
        for object in &container.objects {
            if object.class_id != ASSET_BUNDLE && object.class_id != RESOURCE_MANAGER {
                continue;
            }
            let Some(value) = object.value() else {
                continue;
            };

            let listed = value.field("m_Container").map(Value::items).unwrap_or(&[]);
            let preload = value
                .field("m_PreloadTable")
                .map(Value::items)
                .unwrap_or(&[]);

            for pair in listed {
                let Some(said) = pair.field("first").and_then(Value::text) else {
                    continue;
                };
                let Some(held) = pair.field("second") else {
                    continue;
                };
                let asset = held.field("asset").unwrap_or(held);

                for spot in iter::once(asset).chain(leaning(held, preload)) {
                    if let Some(at) = pointed_at(container, spot) {
                        self.paths.entry(at).or_insert_with(|| said.clone());
                    }
                }
            }
        }
    }

    pub fn of(&self, file: &str, path_id: i64) -> Option<&str> {
        self.paths
            .get(&(mono_script::file_key(file), path_id))
            .map(String::as_str)
    }
}

fn pointed_at(container: &Container, held: &Value) -> Option<(String, i64)> {
    let path_id = held.field("m_PathID")?.number()?;
    if path_id == 0 {
        return None;
    }

    let file = held.field("m_FileID")?.number()?;
    let named = mono_script::owner_of(container, i32::try_from(file).ok()?)?;

    Some((named, path_id))
}

fn leaning<'v>(held: &Value, preload: &'v [Value]) -> impl Iterator<Item = &'v Value> {
    let from = held
        .field("preloadIndex")
        .and_then(Value::number)
        .and_then(|held| usize::try_from(held).ok())
        .unwrap_or(0);
    let many = held
        .field("preloadSize")
        .and_then(Value::number)
        .and_then(|held| usize::try_from(held).ok())
        .unwrap_or(0);

    preload
        .get(from..from.saturating_add(many))
        .unwrap_or_default()
        .iter()
}

struct Adrift {
    kind: &'static str,
    name: String,
    wide: u32,
    high: u32,
    why: String,
}

const UNREAD: &str = "this picture did not come back the way the game describes it, so there is \
                      nothing here to show and nothing safe to write over it";

fn why_adrift(why: sprite::Adrift) -> String {
    match why {
        sprite::Adrift::Packed(named) if !named.is_empty() => {
            return format!(
                "nothing in this file says where {named} keeps this sprite. That atlas's own \
                 picture is named after it, so look for {named} in the list and replace that \
                 instead"
            );
        }
        sprite::Adrift::Packed(_) => {
            "nothing in this file says where the SpriteAtlas this sprite was packed into keeps \
             it, so it cannot be replaced on its own. Replace the whole atlas picture instead"
        }
        sprite::Adrift::Twice => {
            "two SpriteAtlases in this file both hold this sprite and they disagree on where it \
             sits, so this reader will not guess which one the game draws"
        }
        sprite::Adrift::Scaled => {
            "the SpriteAtlas holding this sprite keeps it at another scale, which this reader \
             does not follow"
        }
        sprite::Adrift::Bare => {
            "this sprite points at no texture at all, so there is nothing here to show"
        }
        sprite::Adrift::Odd => {
            "where this sprite sits in the picture it comes from is not something this reader can \
             read"
        }
    }
    .to_string()
}

fn unreadable(format: i32) -> String {
    format!(
        "this game keeps it as {}, which this reader cannot draw yet",
        texture::called(format)
    )
}

fn lost_texture(value: Option<&Value>) -> Option<Adrift> {
    let held = texture::unread(value)?;

    Some(Adrift {
        kind: TEXTURE,
        name: held.name,
        wide: held.wide,
        high: held.high,
        why: match texture::draws(held.format) {
            true => UNREAD.to_string(),
            false => unreadable(held.format),
        },
    })
}

fn lost_sprite(value: Option<&Value>, sheets: &atlas::Sheets) -> Option<Adrift> {
    let Some(value) = value else {
        return Some(Adrift {
            kind: SPRITE,
            name: String::new(),
            wide: 0,
            high: 0,
            why: UNREAD.to_string(),
        });
    };
    let held = sprite::adrift(value, sheets)?;

    Some(Adrift {
        kind: SPRITE,
        name: held.name,
        wide: held.wide as u32,
        high: held.high as u32,
        why: why_adrift(held.why),
    })
}

fn drawn_from(pictured: &BTreeMap<i64, &Picture>, cut: &sprite::Cut) -> (bool, String, i64) {
    match pictured.get(&cut.of) {
        Some(held) => (false, shown(&held.name, cut.of), cut.of),
        None => (true, String::new(), cut.of),
    }
}

fn crowding(cuts: &[sprite::Cut]) -> BTreeSet<i64> {
    let mut out = BTreeSet::new();

    for (which, cut) in cuts.iter().enumerate() {
        for held in cuts.iter().skip(which + 1) {
            if held.of != cut.of || !overlapping(cut, held) {
                continue;
            }

            out.insert(cut.at);
            out.insert(held.at);
        }
    }

    out
}

fn overlapping(one: &sprite::Cut, other: &sprite::Cut) -> bool {
    one.from_x < other.from_x + other.wide
        && other.from_x < one.from_x + one.wide
        && one.from_y < other.from_y + other.high
        && other.from_y < one.from_y + one.high
}

fn shown(name: &str, path_id: i64) -> String {
    match name.trim().is_empty() {
        true => format!("{path_id}"),
        false => name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::unity::serial::{Object, TEXTURE_2D};
    use crate::engine::unity::texture::RGBA32;
    use crate::engine::unity::{fake, settle};

    const ATLAS: i64 = 7;

    #[test]
    fn a_sheet_many_tiles_sit_on_is_drawn_once_however_many_ask_at_the_same_moment() {
        use std::sync::Barrier;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::thread;
        use std::time::Duration;

        struct Forgetting;
        impl Drop for Forgetting {
            fn drop(&mut self) {
                forget();
            }
        }
        let _swept_even_when_an_assert_fails = Forgetting;

        let opened = Arc::new(Opened {
            containers: Vec::new(),
            inside: BTreeMap::new(),
        });
        if let Ok(mut kept) = KEPT.lock() {
            *kept = Some(Kept {
                holder: "one.bundle".to_string(),
                stamp: store::stamp_of(Path::new("no such file")),
                opened: Arc::clone(&opened),
                drawn: Vec::new(),
                sheets: Vec::new(),
            });
        }

        let many = 8;
        let drawn = Arc::new(AtomicUsize::new(0));
        let together = Arc::new(Barrier::new(many));
        let asking: Vec<_> = (0..many)
            .map(|_| {
                let (drawn, together) = (Arc::clone(&drawn), Arc::clone(&together));
                let opened = Arc::clone(&opened);

                thread::spawn(move || {
                    together.wait();

                    whole_of(&opened, "a page", || {
                        drawn.fetch_add(1, Ordering::SeqCst);
                        thread::sleep(Duration::from_millis(20));

                        Ok(Canvas::blank(64, 32))
                    })
                    .map(|held| (held.wide, held.high))
                })
            })
            .collect();

        for one in asking {
            assert_eq!(
                one.join().expect("it finishes").expect("a sheet"),
                (64, 32),
                "every tile that waited has to end up with the same sheet, not an empty one"
            );
        }

        assert_eq!(
            drawn.load(Ordering::SeqCst),
            1,
            "a screenful of tiles cut from one atlas asks for that sheet at the same moment, and \
             decoding a 4096 square sheet costs an eighth of a second and sixty seven megabytes: \
             letting each asker draw its own copy burned both, and made asking in parallel slower \
             than asking one at a time"
        );
    }

    fn dotted(wide: usize, high: usize, tint: u8) -> Canvas {
        let mut held = Canvas::blank(wide, high);

        for (which, byte) in held.pixels.iter_mut().enumerate() {
            *byte = ((which * 3) as u8).wrapping_add(tint);
        }

        held
    }

    type Sprites<'s> = [(i64, &'s str, (f32, f32, f32, f32))];

    fn a_container(sprites: &Sprites<'_>) -> Container {
        let data: Vec<u8> = dotted(16, 8, 0).pixels;
        let mut objects = vec![Object::forged(
            TEXTURE_2D,
            ATLAS,
            fake::a_texture(&fake::Drawn {
                data: &data,
                ..fake::drawn("sactx-0-16x8", 16, 8, RGBA32)
            }),
        )];

        for (path_id, name, rect) in sprites {
            objects.push(Object::forged(
                serial::SPRITE,
                *path_id,
                fake::a_sprite(name, ATLAS, *rect, 0),
            ));
        }

        fake::container("sharedassets1.assets", objects, &[])
    }

    fn an_atlas_of_two() -> Container {
        let data: Vec<u8> = dotted(16, 8, 0).pixels;
        let key = |local: i64| fake::Key {
            guid: [1, 2, 3, 4],
            local,
        };

        fake::container(
            "sharedassets1.assets",
            vec![
                Object::forged(
                    TEXTURE_2D,
                    ATLAS,
                    fake::a_texture(&fake::Drawn {
                        data: &data,
                        ..fake::drawn("sactx-0-16x8", 16, 8, RGBA32)
                    }),
                ),
                Object::forged(
                    serial::SPRITE,
                    11,
                    fake::a_packed_sprite("left", 9, (8.0, 8.0), key(1), ""),
                ),
                Object::forged(
                    serial::SPRITE,
                    12,
                    fake::a_packed_sprite("right", 9, (8.0, 8.0), key(2), ""),
                ),
                Object::forged(
                    atlas::SPRITE_ATLAS,
                    9,
                    fake::an_atlas(
                        "one_Atlas",
                        &[
                            fake::Entry {
                                key: key(1),
                                of: ATLAS,
                                rect: (0.0, 0.0, 8.0, 8.0),
                                settings: 65,
                                ..fake::entry()
                            },
                            fake::Entry {
                                key: key(2),
                                of: ATLAS,
                                rect: (8.0, 0.0, 8.0, 8.0),
                                settings: 77,
                                ..fake::entry()
                            },
                        ],
                    ),
                ),
            ],
            &[],
        )
    }

    struct Nothing;

    impl Beside for Nothing {
        fn part(&self, _: &str, _: u64, _: usize) -> Option<Vec<u8>> {
            None
        }

        fn size(&self, _: &str) -> Option<u64> {
            None
        }
    }

    fn back(container: &Container, done: &Swapped, key: &str) -> Canvas {
        let mut objects = Vec::new();
        for one in &container.objects {
            let body = done
                .bodies
                .get(&one.path_id)
                .cloned()
                .unwrap_or_else(|| one.body().expect("its body").into_owned());

            objects.push(Object::forged(one.class_id, one.path_id, body));
        }

        let fresh = fake::container(&container.name, objects, &[]);
        let path_id = named_in(key).expect("a path id");

        match fresh
            .objects
            .iter()
            .find(|one| one.path_id == path_id)
            .map(|one| one.class_id)
        {
            Some(TEXTURE_2D) => whole(&fresh, path_id, &Nothing).expect("a texture"),
            _ => {
                let object = fresh
                    .objects
                    .iter()
                    .find(|one| one.path_id == path_id)
                    .expect("an object");
                let sheets = atlas::Sheets::read(&fresh.objects);
                let cut = sprite::cut_of(object, &sheets).expect("a sprite");

                cut.cut(&whole(&fresh, cut.of, &Nothing).expect("its atlas"))
                    .expect("a cut")
            }
        }
    }

    #[test]
    fn pixels_the_game_streams_go_back_into_the_sidecar_and_the_container_stays_the_size_it_was() {
        let shipped = dotted(16, 8, 0).flipped().pixels;
        let ahead = vec![7u8; 4096];
        let mut sidecar = ahead.clone();
        sidecar.extend_from_slice(&shipped);

        let object = Object::forged(
            TEXTURE_2D,
            ATLAS,
            fake::a_texture(&fake::Drawn {
                sidecar: "sharedassets1.assets.sidecar",
                at: ahead.len() as u64,
                whole: shipped.len(),
                ..fake::drawn("sactx-0-16x8", 16, 8, RGBA32)
            }),
        );
        let was = object.body().expect("its body").len();
        let node = fake::container("sharedassets1.assets", vec![object], &[]);

        let mut held = BTreeMap::new();
        held.insert("sharedassets1.assets.sidecar".to_string(), sidecar.clone());
        let beside = Nearby {
            held: &held,
            folder: Path::new("."),
            store: Path::new("."),
            game_dir: Path::new("."),
        };

        let fresh = dotted(16, 8, 99);
        let mut picks = BTreeMap::new();
        picks.insert(ATLAS, &fresh);

        let done = swapped(&node, &picks, &beside, &mut BTreeMap::new());
        assert!(done.dropped.is_empty(), "{:?}", done.dropped);
        assert_eq!(done.pictures, 1);

        assert_eq!(
            done.bodies[&ATLAS].len(),
            was,
            "the pixels stay beside the game, so the object is written back the same length and a \
             container of thirty six megabytes does not become one of three hundred"
        );

        let written = &done.beside["sharedassets1.assets.sidecar"];
        assert_eq!(written.len(), 1);
        assert_eq!(
            (written[0].at, written[0].body.len()),
            (ahead.len() as u64, shipped.len()),
            "a picture of the same size in the same format goes back exactly where it came from, \
             so the sidecar does not grow either"
        );

        let mut after = sidecar.clone();
        settle::laid_into(&mut after, &written[0]).expect("it lands");
        assert_eq!(
            after.len(),
            sidecar.len(),
            "and the file stays the size it was"
        );
        assert_eq!(
            &after[..ahead.len()],
            &ahead[..],
            "every other picture in the sidecar belongs to the game"
        );

        let mut held = BTreeMap::new();
        held.insert("sharedassets1.assets.sidecar".to_string(), after);
        let node = fake::container(
            "sharedassets1.assets",
            vec![Object::forged(
                TEXTURE_2D,
                ATLAS,
                done.bodies[&ATLAS].clone(),
            )],
            &[],
        );

        let back = whole(
            &node,
            ATLAS,
            &Nearby {
                held: &held,
                folder: Path::new("."),
                store: Path::new("."),
                game_dir: Path::new("."),
            },
        )
        .expect("it reads back out of the sidecar");

        assert_eq!(
            back.pixels, fresh.pixels,
            "and what the reader picked is what the game draws"
        );
    }

    #[test]
    fn a_picture_that_no_longer_fits_where_it_lay_is_laid_after_everything_else() {
        const DXT5: i32 = 12;

        let squished = vec![0u8; 16 * 8];
        let ahead = vec![7u8; 4096];
        let mut sidecar = ahead.clone();
        sidecar.extend_from_slice(&squished);

        let object = Object::forged(
            TEXTURE_2D,
            ATLAS,
            fake::a_texture(&fake::Drawn {
                sidecar: "sharedassets1.assets.sidecar",
                at: ahead.len() as u64,
                whole: squished.len(),
                ..fake::drawn("sactx-0-16x8", 16, 8, DXT5)
            }),
        );
        let node = fake::container("sharedassets1.assets", vec![object], &[]);

        let mut held = BTreeMap::new();
        held.insert("sharedassets1.assets.sidecar".to_string(), sidecar.clone());

        let fresh = dotted(16, 8, 99);
        let mut picks = BTreeMap::new();
        picks.insert(ATLAS, &fresh);

        let done = swapped(
            &node,
            &picks,
            &Nearby {
                held: &held,
                folder: Path::new("."),
                store: Path::new("."),
                game_dir: Path::new("."),
            },
            &mut BTreeMap::new(),
        );
        assert!(done.dropped.is_empty(), "{:?}", done.dropped);

        let written = &done.beside["sharedassets1.assets.sidecar"];
        assert_eq!(
            (written[0].at, written[0].body.len()),
            (sidecar.len() as u64, 16 * 8 * 4),
            "this build has no packer for the format the game shipped, so the picture comes back \
             four times the size and cannot go back in its old spot: it is laid after everything \
             else and nothing that came before it moves"
        );

        let mut after = sidecar.clone();
        settle::laid_into(&mut after, &written[0]).expect("it lands");
        assert_eq!(
            &after[..sidecar.len()],
            &sidecar[..],
            "the pixels of every other picture stay exactly where the game left them"
        );
    }

    #[test]
    fn replacing_one_sprite_leaves_every_other_sprite_in_the_atlas_alone() {
        let node = a_container(&[
            (11, "left", (0.0, 0.0, 8.0, 8.0)),
            (12, "right", (8.0, 0.0, 8.0, 8.0)),
        ]);

        let fresh = dotted(8, 8, 99);
        let mut picks = BTreeMap::new();
        picks.insert(11, &fresh);

        let done = swapped(&node, &picks, &Nothing, &mut BTreeMap::new());
        assert!(done.dropped.is_empty(), "{:?}", done.dropped);
        assert_eq!(done.pictures, 1);
        assert_eq!(
            done.bodies.len(),
            1,
            "one atlas holds both sprites, so one object is written and no other"
        );

        let was = back(
            &node,
            &Swapped {
                beside: BTreeMap::new(),
                bodies: BTreeMap::new(),
                pictures: 0,
                dropped: Vec::new(),
            },
            "sharedassets1.assets|12",
        );

        assert_eq!(
            back(&node, &done, "sharedassets1.assets|11").pixels,
            fresh.pixels,
            "the sprite the reader picked has to be the sprite the game shows"
        );
        assert_eq!(
            back(&node, &done, "sharedassets1.assets|12").pixels,
            was.pixels,
            "an atlas strip holds forty six frames in this game: replacing one may not touch the \
             forty five beside it"
        );
    }

    #[test]
    fn replacing_a_sprite_the_packer_packed_lands_in_the_texture_the_atlas_names() {
        let node = an_atlas_of_two();
        let was = back(&node, &Swapped::default(), "sharedassets1.assets|12");
        let fresh = dotted(8, 8, 99);
        let done = swapped(
            &node,
            &BTreeMap::from([(11, &fresh)]),
            &Nothing,
            &mut BTreeMap::new(),
        );

        assert!(done.dropped.is_empty(), "{:?}", done.dropped);
        assert_eq!(done.pictures, 1);
        assert_eq!(
            done.bodies.keys().collect::<Vec<_>>(),
            [&ATLAS],
            "a packed sprite carries no pixels of its own, so the only object that changes is the \
             texture the atlas cut it out of"
        );

        assert_eq!(
            back(&node, &done, "sharedassets1.assets|11").pixels,
            fresh.pixels,
            "the way in and the way out both run through the atlas, so what a reader picked is \
             what the game draws"
        );
        assert_eq!(
            back(&node, &done, "sharedassets1.assets|12").pixels,
            was.pixels,
            "and the sprite packed beside it, turned the other way up, is left exactly as it was"
        );
    }

    #[test]
    fn a_sprite_exported_and_handed_straight_back_changes_nothing_at_all() {
        let node = an_atlas_of_two();
        let was = node.objects[0].body().expect("its body").into_owned();

        for key in ["sharedassets1.assets|11", "sharedassets1.assets|12"] {
            let shipped = back(&node, &Swapped::default(), key);
            let exported = shipped.png().expect("the png the Export button writes");
            let handed = Canvas::read(&exported).expect("the file a reader hands back");

            assert_eq!(
                handed.pixels, shipped.pixels,
                "{key}: a png carries every channel as it was, transparent pixels included, so \
                 the file leaving and the file coming back have to be the same picture"
            );

            let path_id = named_in(key).expect("a path id");
            let done = swapped(
                &node,
                &BTreeMap::from([(path_id, &handed)]),
                &Nothing,
                &mut BTreeMap::new(),
            );
            assert!(done.dropped.is_empty(), "{:?}", done.dropped);

            assert_eq!(
                back(&node, &done, key).pixels,
                shipped.pixels,
                "{key}: whoever exports a sprite to paint the words on it and hands it back \
                 unchanged has to get the very picture the game shipped, or the flip on the way \
                 out and the flip on the way in are not the same flip"
            );
            assert_eq!(
                done.bodies[&ATLAS], was,
                "{key}: and the texture itself comes back byte for byte, so a round trip through \
                 the Export button leaves the game exactly as it was"
            );
        }
    }

    #[test]
    fn a_picture_of_another_size_is_scaled_to_the_spot_it_has_to_fill() {
        let node = a_container(&[(11, "left", (0.0, 0.0, 8.0, 8.0))]);

        let fresh = dotted(32, 32, 5);
        let mut picks = BTreeMap::new();
        picks.insert(11, &fresh);

        let done = swapped(&node, &picks, &Nothing, &mut BTreeMap::new());
        assert!(done.dropped.is_empty(), "{:?}", done.dropped);

        let back = back(&node, &done, "sharedassets1.assets|11");
        assert_eq!(
            (back.wide, back.high),
            (8, 8),
            "a sprite rect points at pixels by number, so a picture that came back bigger is \
             scaled into the spot rather than refused: the reader is told what happened"
        );
    }

    #[test]
    fn picking_both_an_atlas_and_a_sprite_inside_it_lands_the_sprite_on_top() {
        let node = a_container(&[(11, "left", (0.0, 0.0, 8.0, 8.0))]);

        let ground = dotted(16, 8, 40);
        let sprite = dotted(8, 8, 200);
        let mut picks = BTreeMap::new();
        picks.insert(ATLAS, &ground);
        picks.insert(11, &sprite);

        let done = swapped(&node, &picks, &Nothing, &mut BTreeMap::new());
        assert!(done.dropped.is_empty(), "{:?}", done.dropped);
        assert_eq!(done.pictures, 2);

        assert_eq!(
            back(&node, &done, "sharedassets1.assets|11").pixels,
            sprite.pixels,
            "the reader asked for both, and the sprite is the narrower answer of the two"
        );
    }

    fn a_bundle_naming(held: &[(&str, i64, &[i64])]) -> Object {
        let mut preload: Vec<Value> = Vec::new();
        let mut listed: Vec<Value> = Vec::new();

        for (said, path_id, leaning) in held {
            let from = preload.len();
            for one in *leaning {
                preload.push(Value::Tree(vec![
                    ("m_FileID".to_string(), Value::Number(0)),
                    ("m_PathID".to_string(), Value::Number(*one)),
                ]));
            }

            listed.push(Value::Tree(vec![
                ("first".to_string(), Value::Bytes(said.as_bytes().to_vec())),
                (
                    "second".to_string(),
                    Value::Tree(vec![
                        ("preloadIndex".to_string(), Value::Number(from as i64)),
                        (
                            "preloadSize".to_string(),
                            Value::Number(leaning.len() as i64),
                        ),
                        (
                            "asset".to_string(),
                            Value::Tree(vec![
                                ("m_FileID".to_string(), Value::Number(0)),
                                ("m_PathID".to_string(), Value::Number(*path_id)),
                            ]),
                        ),
                    ]),
                ),
            ]));
        }

        Object::forged(
            ASSET_BUNDLE,
            9,
            fake::drawing(
                ASSET_BUNDLE,
                vec![
                    ("m_Name", fake::text("one.bundle")),
                    ("m_PreloadTable", Value::List(preload)),
                    ("m_Container", Value::List(listed)),
                ],
            ),
        )
    }

    #[test]
    fn a_sprite_whose_atlas_is_in_another_file_is_shown_with_the_atlas_to_look_for() {
        let key = fake::Key {
            guid: [5, 6, 7, 8],
            local: 21300000,
        };
        let mut node = a_container(&[(11, "left", (0.0, 0.0, 8.0, 8.0))]);
        node.objects.push(Object::forged(
            serial::SPRITE,
            12,
            fake::a_packed_sprite("packed", 99, (64.0, 32.0), key, "Journal_Atlas"),
        ));
        node.objects.push(Object::forged(
            serial::SPRITE,
            13,
            fake::a_packed_sprite("nameless", 99, (64.0, 32.0), key, ""),
        ));

        let opened = Opened {
            containers: vec![node],
            inside: BTreeMap::new(),
        };
        let shots = opened.shots("one.bundle", &Named::default());
        let one = shots
            .iter()
            .find(|one| one.name == "packed")
            .expect("a sprite no cut comes out of still gets a row of its own");

        assert!(
            one.locked
                .as_deref()
                .is_some_and(|said| said.contains("Journal_Atlas")),
            "the atlas holding it is in another container, and its own picture is named after \
             that atlas: naming it is the whole difference between a reader who can find the \
             picture and one who cannot: {:?}",
            one.locked
        );
        assert!(!one.drawable);
        assert_eq!(
            (one.wide, one.high),
            (64, 32),
            "its own rect still says how big the game draws it, which is what tells a reader \
             which picture this row is"
        );

        let bare = shots
            .iter()
            .find(|one| one.name == "nameless")
            .expect("a sprite carrying no tag is still a row");
        assert!(
            bare.locked
                .as_deref()
                .is_some_and(|said| said.contains("SpriteAtlas")),
            "a sprite the packer left no atlas name on can still only be told what happened to \
             it: {:?}",
            bare.locked
        );
    }

    #[test]
    fn a_bundle_hands_its_pictures_the_paths_the_game_was_built_from() {
        let mut node = a_container(&[(11, "left", (0.0, 0.0, 8.0, 8.0))]);
        node.objects.push(a_bundle_naming(&[(
            "assets/ui/buttons/left.png",
            11,
            &[ATLAS],
        )]));

        let opened = Opened {
            containers: vec![node],
            inside: BTreeMap::new(),
        };
        let mut named = Named::default();
        for one in &opened.containers {
            named.learn(one);
        }
        let shots = opened.shots("one.bundle", &named);

        let sprite = shots
            .iter()
            .find(|one| one.kind == SPRITE)
            .expect("the sprite");
        assert_eq!(
            sprite.at, "assets/ui/buttons/left.png",
            "a bundle knows the path every asset was built from, and that path is the only name a \
             reader would recognise: without it the rail shows object names nobody chose"
        );

        let texture = shots
            .iter()
            .find(|one| one.kind == TEXTURE)
            .expect("the atlas");
        assert_eq!(
            texture.at, "assets/ui/buttons/left.png",
            "the texture behind a sprite is listed in the preload table of that sprite's entry, \
             which is the only place its path can come from"
        );
    }

    #[test]
    fn a_path_naming_an_asset_in_another_file_is_left_for_that_file_to_name() {
        let mut node = a_container(&[(11, "left", (0.0, 0.0, 8.0, 8.0))]);
        let mut bundle = a_bundle_naming(&[("assets/ui/buttons/left.png", 11, &[])]);

        let mut value = bundle.value().expect("a bundle that reads");
        let held = value
            .field_mut("m_Container")
            .expect("the container")
            .items_mut();
        if let Some(one) = held.first_mut()
            && let Some(asset) = one
                .field_mut("second")
                .and_then(|held| held.field_mut("asset"))
            && let Some(file) = asset.field_mut("m_FileID")
        {
            *file = Value::Number(1);
        }
        bundle = Object::forged(ASSET_BUNDLE, 9, bundle.written(&value).expect("it writes"));
        node.objects.push(bundle);

        let opened = Opened {
            containers: vec![node],
            inside: BTreeMap::new(),
        };
        let mut named = Named::default();
        for one in &opened.containers {
            named.learn(one);
        }
        let shots = opened.shots("one.bundle", &named);

        assert!(
            shots.iter().all(|one| one.at.is_empty()),
            "a number that means something in another file may not be read as one of ours, or a \
             picture wears a path that belongs to a stranger"
        );
    }

    #[test]
    fn a_sprite_naming_a_texture_in_another_file_is_never_cut_out_of_a_local_one() {
        let data: Vec<u8> = dotted(16, 8, 0).pixels;
        let node = fake::container(
            "sharedassets2.assets",
            vec![
                Object::forged(
                    TEXTURE_2D,
                    ATLAS,
                    fake::a_texture(&fake::Drawn {
                        data: &data,
                        ..fake::drawn("someone elses number", 16, 8, RGBA32)
                    }),
                ),
                Object::forged(
                    serial::SPRITE,
                    11,
                    fake::a_sprite_from("UISprite", 2, ATLAS, (0.0, 0.0, 8.0, 8.0), 0),
                ),
            ],
            &["Resources/unity_builtin_extra"],
        );

        let key = key_of("sharedassets2.assets", 0, 11);
        let opened = Opened {
            containers: vec![node],
            inside: BTreeMap::new(),
        };

        let shots = opened.shots("sharedassets2.assets", &Named::default());
        let held = shots
            .iter()
            .find(|one| one.kind == SPRITE)
            .expect("the sprite is still listed");

        assert!(
            !held.drawable,
            "this game holds 28 sprites whose texture lives in another file at a number some \
             local object happens to share: drawing them from here shows the reader a picture the \
             game never draws"
        );
        assert!(held.locked.is_some());
        assert!(
            held.atlas.is_empty(),
            "and naming the local texture as its atlas would tell the reader the same lie"
        );

        assert!(
            opened.wants(&key, &atlas::Sheets::default()).is_none(),
            "a key left over from an older read may not reach the wrong texture either"
        );

        let fresh = dotted(8, 8, 99);
        let done = swapped(
            &opened.containers[0],
            &BTreeMap::from([(11, &fresh)]),
            &Nothing,
            &mut BTreeMap::new(),
        );
        assert!(
            done.bodies.is_empty() && !done.dropped.is_empty(),
            "and a pick against it is turned away out loud rather than painted over a stranger"
        );
    }

    #[test]
    fn sprites_cut_from_one_picture_are_listed_together_and_the_pictures_in_order() {
        let data: Vec<u8> = dotted(16, 8, 0).pixels;
        let sheet = |path_id: i64, name: &str| {
            Object::forged(
                TEXTURE_2D,
                path_id,
                fake::a_texture(&fake::Drawn {
                    data: &data,
                    ..fake::drawn(name, 16, 8, RGBA32)
                }),
            )
        };

        let mut objects = vec![sheet(7, "sheet_b"), sheet(8, "sheet_a")];
        for (path_id, of) in [(11, 7), (12, 8), (13, 7), (14, 8), (15, 7)] {
            objects.push(Object::forged(
                serial::SPRITE,
                path_id,
                fake::a_sprite(&format!("piece{path_id}"), of, (0.0, 0.0, 8.0, 8.0), 0),
            ));
        }

        let shots = Opened {
            containers: vec![fake::container("sharedassets1.assets", objects, &[])],
            inside: BTreeMap::new(),
        }
        .shots("sharedassets1.assets", &Named::default());

        let listed: Vec<&str> = shots
            .iter()
            .filter(|one| one.kind == SPRITE)
            .map(|one| one.atlas.as_str())
            .collect();

        assert_eq!(
            listed,
            ["sheet_a", "sheet_a", "sheet_b", "sheet_b", "sheet_b"],
            "drawing one sprite decodes the whole picture it was cut from, and one game holds \
             pages of 8192x4096: listing the sprites in the order the file happens to hold them \
             made a single pass through the rail decode 46 GB of pixels where 6 GB would do. The \
             pages go in the order their names do, which is the order the rail beside them lists"
        );

        let names: Vec<&str> = shots
            .iter()
            .filter(|one| one.kind == SPRITE)
            .map(|one| one.name.as_str())
            .collect();
        assert_eq!(
            names,
            ["piece12", "piece14", "piece11", "piece13", "piece15"],
            "and inside one page the sprites stay in the order the game holds them, so nothing a \
             reader learned about a sheet is shuffled from one read to the next"
        );
    }

    #[test]
    fn a_sprite_sharing_its_spot_with_another_is_shown_but_never_painted_over() {
        let node = a_container(&[
            (11, "left", (0.0, 0.0, 8.0, 8.0)),
            (12, "over", (4.0, 0.0, 8.0, 8.0)),
        ]);

        let shots = Opened {
            containers: vec![node],
            inside: BTreeMap::new(),
        }
        .shots("sharedassets1.assets", &Named::default());

        assert_eq!(shots.len(), 3, "one texture and two sprites");
        assert!(
            shots
                .iter()
                .filter(|one| one.kind == SPRITE)
                .all(|one| one.locked.is_some()),
            "two rects over the same pixels cannot both be replaced on their own, and painting \
             one would draw over the other"
        );
        assert!(
            shots
                .iter()
                .find(|one| one.kind == TEXTURE)
                .expect("the atlas")
                .locked
                .is_none(),
            "the whole atlas is still the reader's to replace, which is the way out of that"
        );
    }

    #[test]
    fn a_pick_made_before_the_atlas_grew_a_neighbour_is_turned_away_rather_than_painted() {
        let node = a_container(&[
            (11, "left", (0.0, 0.0, 8.0, 8.0)),
            (12, "over", (4.0, 0.0, 8.0, 8.0)),
        ]);

        let fresh = dotted(8, 8, 40);
        let held = swapped(
            &node,
            &BTreeMap::from([(11, &fresh)]),
            &Nothing,
            &mut BTreeMap::new(),
        );

        assert!(
            held.bodies.is_empty() && held.pictures == 0,
            "a pick lives in the project and survives a re-read, so a sprite that only became \
             crowded later would paint over its neighbour without the rail ever warning again"
        );
        assert!(
            held.dropped.iter().any(|why| why.contains("neighbour")),
            "and the reader has to hear why nothing happened: {:?}",
            held.dropped
        );
    }

    fn shot(key: &str, name: &str) -> Shot {
        Shot {
            key: key.to_string(),
            holder: "sharedassets1.assets".to_string(),
            name: name.to_string(),
            kind: SPRITE.to_string(),
            atlas: "sactx-0-300x300".to_string(),
            wide: 300,
            high: 300,
            format: "RGBA32".to_string(),
            saved_as: AS_PNG.to_string(),
            locked: None,
            drawable: true,
            at: String::new(),
        }
    }

    #[tokio::test]
    async fn the_pictures_a_read_found_are_the_ones_a_later_run_offers() {
        let sandbox = tempfile::tempdir().expect("a temp folder");
        let store = sandbox.path();

        assert!(
            LEDGER.remembered(store).is_empty(),
            "a game nobody has read yet offers no picture, which is what keeps the tab off the \
             screen until there is something on it"
        );

        LEDGER
            .remember(
                store,
                &[
                    shot("sharedassets1.assets|11", "button_yes"),
                    shot("sharedassets1.assets|12", "button_no"),
                ],
            )
            .await
            .expect("a list of pictures");

        let back = LEDGER.remembered(store);
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].name, "button_yes");
        assert_eq!(back[1].key, "sharedassets1.assets|12");

        LEDGER.remember(store, &[]).await.expect("an empty list");
        assert!(
            LEDGER.remembered(store).is_empty(),
            "reading a game again after its pictures went has to take the rows away too"
        );
    }

    #[test]
    fn a_key_names_the_file_the_node_inside_it_and_the_object() {
        let key = key_of("sharedassets1.assets", 0, -7);

        assert_eq!(holder_in(&key), Some("sharedassets1.assets"));
        assert_eq!(named_in(&key), Some(-7));
        assert_eq!(
            node_in(&key),
            0,
            "a pick is written down against this key and read back at install, so every part of \
             the way back to the object has to come out of it"
        );
        assert_eq!(
            holder_in(&key_of("StreamingAssets/aa/one.bundle", 2, 3)),
            Some("StreamingAssets/aa/one.bundle"),
            "a container inside a folder keeps its path, slashes and all"
        );
        assert_eq!(node_in(&key_of("StreamingAssets/aa/one.bundle", 2, 3)), 2);

        let odd = key_of("levels/level#2", 1, 5);
        assert_eq!(
            (holder_in(&odd), node_in(&odd)),
            (Some("levels/level#2"), 1),
            "a game may name a file with a hash of its own, and only the last one is ours"
        );
    }

    #[test]
    fn two_pictures_sharing_a_name_or_a_number_still_get_a_key_of_their_own() {
        assert_ne!(
            key_of("sharedassets1.assets", 0, 11),
            key_of("sharedassets1.assets", 0, 12),
            "this game ships thirty six textures called s16-Sheet and the like: a key made of \
             the name alone would hand one reader's pick to another picture"
        );
        assert_ne!(
            key_of("one.bundle", 0, 11),
            key_of("one.bundle", 1, 11),
            "one bundle holds more than one container, and two of them may hold an object at the \
             same number: a pick that named only the bundle would land on both"
        );
    }

    #[test]
    fn a_picture_nobody_named_is_shown_by_the_number_the_game_holds_it_at() {
        assert_eq!(shown("", 42), "42");
        assert_eq!(shown("  ", 42), "42");
        assert_eq!(shown("Background", 42), "Background");
    }
}
