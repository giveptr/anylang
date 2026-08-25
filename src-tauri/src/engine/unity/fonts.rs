use crate::engine::fonts::{family, metrics};
use crate::engine::unity::serial;
use crate::engine::unity::serial::{Object, Value};
use crate::engine::{Font, Install, fonts as face};
use crate::store;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const LEDGER: &str = "unity-fonts.json";

const SFNT: [[u8; 4]; 4] = [[0x00, 0x01, 0x00, 0x00], *b"OTTO", *b"true", *b"ttcf"];

const A_HAIR: f32 = 0.01;

fn drawn_by(value: &Value) -> Option<(String, &[u8])> {
    let held = shipped(value)?;
    let said = value.field("m_Name")?.text()?;

    let name = Some(said.trim().to_string())
        .filter(|said| !said.is_empty())
        .or_else(|| family(held))?;

    Some((name, held))
}

pub fn face_of(object: &Object) -> Option<String> {
    if object.class_id != serial::FONT {
        return None;
    }

    let (name, _) = drawn_by(&object.value()?)?;

    Some(name)
}

#[derive(Default)]
pub struct Lifting {
    pub landed: BTreeMap<String, PathBuf>,
    missed: BTreeSet<String>,
}

impl Lifting {
    pub fn take_in(&mut self, store: &Path, objects: &[Object]) {
        for object in objects.iter().filter(|one| one.class_id == serial::FONT) {
            let Some(value) = object.value() else {
                continue;
            };
            let Some((name, held)) = drawn_by(&value) else {
                continue;
            };
            if self.landed.contains_key(&name) {
                continue;
            }

            match face::lift(store, &name, held) {
                Some(copy) => {
                    self.missed.remove(&name);
                    self.landed.insert(name, copy);
                }
                None => {
                    self.missed.insert(name);
                }
            }
        }
    }

    pub fn missed(&self) -> impl Iterator<Item = &str> {
        self.missed.iter().map(String::as_str)
    }
}

fn shipped(value: &Value) -> Option<&[u8]> {
    let held = match value.field("m_FontData")? {
        Value::Bytes(raw) => raw.as_slice(),
        _ => return None,
    };

    (is_sfnt(held) && family(held).is_some()).then_some(held)
}

pub fn swapped(object: &Object, picked: &[u8]) -> Option<Vec<u8>> {
    if object.class_id != serial::FONT {
        return None;
    }

    let mut value = object.value()?;
    let was = shipped(&value)?.to_vec();

    redrawn(&mut value, &was, picked);

    let room = value.field_mut("m_FontData")?;
    *room = Value::Bytes(picked.to_vec());

    object.written(&value).ok()
}

#[cfg(test)]
pub fn shipped_in(object: &Object) -> Option<Vec<u8>> {
    Some(shipped(&object.value()?)?.to_vec())
}

fn redrawn(value: &mut Value, was: &[u8], picked: &[u8]) -> Option<()> {
    let size = value.field("m_FontSize")?.real()? as f32;
    let before = metrics(was)?.drawn_at(size);
    let after = metrics(picked)?.drawn_at(size);

    let held = |at: &str| {
        value
            .field(at)
            .and_then(Value::real)
            .map(|held| held as f32)
    };
    if !alike(held("m_LineSpacing")?, before.line)
        || !alike(held("m_Ascent")?, before.ascent)
        || !alike(held("m_Descent")?, before.descent)
    {
        return None;
    }

    drawn_at(value, "m_LineSpacing", after.line)?;
    drawn_at(value, "m_Ascent", after.ascent)?;
    drawn_at(value, "m_Descent", after.descent)
}

fn drawn_at(value: &mut Value, at: &str, held: f32) -> Option<()> {
    let room = value.field_mut(at)?;
    *room = Value::Real(f64::from(held));

    Some(())
}

fn alike(one: f32, other: f32) -> bool {
    (one - other).abs() <= A_HAIR * one.abs().max(other.abs()).max(1.0)
}

fn is_sfnt(raw: &[u8]) -> bool {
    raw.get(..4)
        .is_some_and(|head| SFNT.iter().any(|one| one == head))
}

fn ledger(store: &Path) -> PathBuf {
    store.join(LEDGER)
}

#[derive(Serialize, Deserialize)]
struct Kept {
    name: String,
    shown: String,
}

pub async fn remember(store: &Path, held: &BTreeMap<String, PathBuf>) -> Result<()> {
    let kept: Vec<Kept> = held
        .iter()
        .map(|(name, at)| Kept {
            name: name.clone(),
            shown: at.to_string_lossy().to_string(),
        })
        .collect();

    face::swept(store, &held.values().cloned().collect::<Vec<_>>());

    let body = serde_json::to_string(&kept).context("listing the fonts a game draws with")?;

    store::write_atomically(&ledger(store), body).await
}

pub fn remembered(store: &Path) -> Vec<Font> {
    let Ok(body) = fs::read_to_string(ledger(store)) else {
        return Vec::new();
    };

    serde_json::from_str::<Vec<Kept>>(&body)
        .unwrap_or_default()
        .into_iter()
        .map(|one| Font {
            name: one.name,
            at: String::new(),
            shown: one.shown,
            builtin: false,
        })
        .collect()
}

#[derive(Default)]
pub struct Chosen {
    picked: BTreeMap<String, Vec<u8>>,
    sent: BTreeMap<String, String>,
}

impl Chosen {
    pub fn is_empty(&self) -> bool {
        self.sent.is_empty()
    }

    pub fn wanted(&self) -> impl Iterator<Item = &str> {
        self.sent.keys().map(String::as_str)
    }

    pub fn for_name(&self, name: &str) -> Option<&[u8]> {
        self.picked.get(self.sent.get(name)?).map(Vec::as_slice)
    }
}

pub async fn chosen(at: &Install<'_>) -> Result<Chosen> {
    if at.reverting {
        return Ok(Chosen::default());
    }

    let picked = at.fonts.picked().await?;
    if picked.is_empty() {
        return Ok(Chosen::default());
    }

    for (from, body) in &picked {
        if family(body).is_none() {
            bail!("{from} is not a font file this reader can read a name out of");
        }
    }

    let mut sent = BTreeMap::new();
    for one in &at.fonts.swaps {
        let Some(to) = at.fonts.sent_to(&one.from) else {
            continue;
        };

        if picked.contains_key(to) {
            sent.insert(one.from.clone(), to.to_string());
        }
    }

    Ok(Chosen { picked, sent })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::fonts::Drawn;
    use crate::engine::fonts::fake::called;
    use crate::engine::unity::{fake, serial};
    use std::ops::Range;

    const GOTHIC: &str = "\u{ff2d}\u{ff33} \u{30b4}\u{30b7}\u{30c3}\u{30af}";
    const SIZE: f32 = 16.0;

    const LIBERATION: (u16, i16, i16, i16) = (2048, 1854, -434, 67);
    const DOS: (u16, i16, i16, i16) = (1024, 768, -256, 0);

    fn measured(family: &str, (upm, asc, desc, gap): (u16, i16, i16, i16)) -> Vec<u8> {
        face::fake::measured(family, upm, asc, desc, gap)
    }

    fn drawn_for(data: &[u8]) -> Drawn {
        metrics(data)
            .map(|held| held.drawn_at(SIZE))
            .unwrap_or(Drawn {
                line: SIZE,
                ascent: SIZE,
                descent: -SIZE,
            })
    }

    fn a_font(path_id: i64, name: &str, data: &[u8]) -> Object {
        Object::forged(
            serial::FONT,
            path_id,
            fake::a_font(name, data, SIZE, drawn_for(data)),
        )
    }

    fn measured_as(name: &str, data: &[u8], drawn: Drawn) -> Object {
        Object::forged(serial::FONT, 11, fake::a_font(name, data, SIZE, drawn))
    }

    fn lines_of(object: &Object) -> (f32, f32, f32) {
        let value = object.value().expect("a font that reads");
        let held = |at: &str| value.field(at).and_then(Value::real).expect(at) as f32;

        (held("m_LineSpacing"), held("m_Ascent"), held("m_Descent"))
    }

    #[test]
    fn a_font_asset_gives_up_the_name_the_game_asks_for_it_by() {
        let object = a_font(11, "NotoSans", &called("Noto Sans"));
        let face = face_of(&object).expect("a font asset with a face inside it");

        assert_eq!(
            face, "NotoSans",
            "the asset name is what the game loads the font by and what a pick has to key on, \
             not the family the file happens to call itself"
        );
    }

    #[test]
    fn a_font_asset_nobody_named_falls_back_to_the_family_the_file_carries() {
        let object = a_font(11, "", &called(GOTHIC));
        let face = face_of(&object).expect("a font asset with a face inside it");

        assert_eq!(
            face, GOTHIC,
            "an asset shipped without a name still has to be pickable, and the only other name \
             it answers to is the one written into the font itself"
        );
    }

    #[test]
    fn a_font_the_game_only_names_and_never_ships_is_never_offered() {
        assert!(
            face_of(&a_font(11, "Arial", &[])).is_none(),
            "an asset carrying no font of its own is drawn by whatever the machine has, so there \
             is nothing here to put a picked font into"
        );
        assert!(
            face_of(&a_font(11, "Arial", b"not a font at all")).is_none(),
            "and bytes that are not a font are not one to hand a reader either"
        );
    }

    #[test]
    fn only_a_font_asset_is_read_as_one() {
        let body = fake::a_font("NotoSans", &called("Noto Sans"), SIZE, drawn_for(&[]));

        assert!(
            face_of(&Object::forged(serial::TEXT_ASSET, 11, body)).is_none(),
            "a script that happens to hold a font inside it is not a font the engine draws with, \
             and rewriting it would put bytes where a reader's text belongs"
        );
    }

    #[test]
    fn a_picked_font_lands_where_the_old_one_was_and_leaves_the_rest_of_the_object_alone() {
        for picked in [called("Sarabun"), called(GOTHIC), called("Noto")] {
            let object = a_font(11, "NotoSans", &called("Noto Sans"));
            let fresh = Object::forged(
                serial::FONT,
                11,
                swapped(&object, &picked).expect("a font asset this reader wrote itself"),
            );

            let after = face_of(&fresh).expect("it still reads back");
            assert_eq!(after, "NotoSans", "the name the game loads by never moves");
            assert_eq!(
                shipped_in(&fresh).as_deref(),
                Some(picked.as_slice()),
                "the bytes read back out have to be the very bytes that went in, or the game \
                 draws with half a font"
            );

            let value = fresh.value().expect("it reads");
            assert_eq!(
                value.field("m_FontSize").and_then(Value::real),
                Some(f64::from(SIZE)),
                "everything the object holds besides the font is the game's own and has to come \
                 through untouched"
            );
        }
    }

    #[test]
    fn a_font_that_grew_and_a_font_that_shrank_both_read_back_as_themselves() {
        let long = called(&"W".repeat(400));
        let short = called("A");
        assert!(
            long.len() > called("Noto Sans").len() && short.len() < called("Noto Sans").len(),
            "one pick has to be bigger than what it replaces and the other smaller, or this test \
             only ever proves the easy case"
        );

        for picked in [long, short] {
            let object = a_font(11, "NotoSans", &called("Noto Sans"));
            let fresh = Object::forged(
                serial::FONT,
                11,
                swapped(&object, &picked).expect("it writes back"),
            );
            face_of(&fresh).expect("it reads back");

            assert_eq!(shipped_in(&fresh).as_deref(), Some(picked.as_slice()));
        }
    }

    #[test]
    fn a_swapped_font_takes_the_line_height_the_new_file_asks_for() {
        let was = measured("Liberation Sans", LIBERATION);
        let object = measured_as("LiberationSans", &was, drawn_for(&was));
        let picked = measured("Perfect DOS VGA 437", DOS);
        let fresh = Object::forged(
            serial::FONT,
            11,
            swapped(&object, &picked).expect("a font asset this reader wrote itself"),
        );

        let (line, ascent, descent) = lines_of(&fresh);
        assert!((line - 16.0).abs() < 0.01, "line {line}");
        assert!((ascent - 12.0).abs() < 0.01, "ascent {ascent}");
        assert!((descent + 4.0).abs() < 0.01, "descent {descent}");
    }

    #[test]
    fn a_font_whose_baked_lines_do_not_describe_it_is_left_measured_as_it_was() {
        let was = measured("Liberation Sans", LIBERATION);
        let odd = Drawn {
            line: 99.0,
            ascent: 88.0,
            descent: -77.0,
        };
        let object = measured_as("LiberationSans", &was, odd);
        let picked = measured("Perfect DOS VGA 437", DOS);
        let fresh = Object::forged(
            serial::FONT,
            11,
            swapped(&object, &picked).expect("the font itself still swaps"),
        );

        assert_eq!(
            lines_of(&fresh),
            (99.0, 88.0, -77.0),
            "the numbers here do not describe the font that was in the object, so this reader is \
             reading fields the game means for something else: writing to them would scramble \
             whatever they really are, and the font swap alone is still worth landing"
        );
    }

    #[test]
    fn a_picked_font_that_says_nothing_about_its_lines_leaves_them_alone() {
        let was = measured("Liberation Sans", LIBERATION);
        let drawn = drawn_for(&was);
        let object = measured_as("LiberationSans", &was, drawn);
        let picked = called("No Metrics");
        let fresh = Object::forged(
            serial::FONT,
            11,
            swapped(&object, &picked).expect("the swap still lands"),
        );

        let (line, ..) = lines_of(&fresh);
        assert!(
            (line - drawn.line).abs() < 0.01,
            "with nothing to work a new line height out of, the one the game shipped is a better \
             answer than a made up one: {line}"
        );
    }

    fn a_collection_of(one: &[u8]) -> Vec<u8> {
        const HEAD: u32 = 16;
        const OFFSET: Range<usize> = 20..24;

        let mut out = b"ttcf".to_vec();
        out.extend_from_slice(&0x0001_0000u32.to_be_bytes());
        out.extend_from_slice(&1u32.to_be_bytes());
        out.extend_from_slice(&HEAD.to_be_bytes());

        let mut inner = one.to_vec();
        let at = u32::from_be_bytes(
            inner
                .get(OFFSET)
                .and_then(|raw| raw.try_into().ok())
                .expect("the one table record a forged font carries"),
        );
        inner[OFFSET].copy_from_slice(&(at + HEAD).to_be_bytes());
        out.extend_from_slice(&inner);

        out
    }

    #[test]
    fn a_collection_of_faces_is_read_and_replaced_as_the_one_file_it_is() {
        let collection = a_collection_of(&called("Inner"));
        let object = a_font(11, "Collection", &collection);
        face_of(&object).expect("a collection names the faces it holds");

        assert_eq!(
            shipped_in(&object).as_deref(),
            Some(collection.as_slice()),
            "a collection is one file the engine loads whole, so swapping the face inside it \
             would leave the wrapper around it pointing at bytes that moved"
        );
    }

    #[tokio::test]
    async fn the_fonts_a_read_found_are_the_ones_a_later_run_offers() {
        let sandbox = tempfile::tempdir().expect("a temp folder");
        let store = sandbox.path();

        assert!(
            remembered(store).is_empty(),
            "a game nobody has read yet offers no font to swap, which is what keeps the tab off \
             the screen until there is something on it"
        );

        let held = BTreeMap::from([
            ("NotoSans".to_string(), called("Noto Sans")),
            (GOTHIC.to_string(), called(GOTHIC)),
        ]);

        let mut lifting = Lifting::default();
        lifting.take_in(
            store,
            &[
                a_font(11, "NotoSans", &held["NotoSans"]),
                a_font(12, "", &held[GOTHIC]),
            ],
        );
        assert_eq!(
            lifting.missed().count(),
            0,
            "a face this reader validated and then could not copy out is an error the screen has \
             to hear about, so it comes back by name rather than going quiet"
        );
        assert_eq!(
            lifting.landed.keys().collect::<Vec<_>>(),
            ["NotoSans", GOTHIC],
            "a read has to come out of the containers holding paths to the copies it lifted, \
             never the font bodies themselves"
        );

        remember(store, &lifting.landed)
            .await
            .expect("a list of fonts");

        let offered = remembered(store);
        assert_eq!(
            offered
                .iter()
                .map(|one| one.name.as_str())
                .collect::<Vec<_>>(),
            ["NotoSans", GOTHIC]
        );
        assert!(
            offered.iter().all(|one| one.at.is_empty()),
            "a font inside a container is at no path inside the game a reader could open, and \
             naming one would send the screen looking for a file that is not there"
        );
        for one in &offered {
            assert_eq!(
                fs::read(&one.shown).expect("the face this row draws its sample with"),
                held[&one.name],
                "the copy lifted out of the game has to be the very bytes the game draws with"
            );
        }

        remember(store, &BTreeMap::new())
            .await
            .expect("an empty list");
        assert!(
            remembered(store).is_empty(),
            "reading a game again after its fonts went has to take the rows away too"
        );
        assert!(
            !face::lifted_in(store)
                .read_dir()
                .is_ok_and(|mut listed| listed.next().is_some()),
            "and the copies it lifted out go with them"
        );
    }

    #[test]
    fn a_face_that_lands_on_a_later_try_is_no_longer_missed() {
        let sandbox = tempfile::tempdir().expect("a temp folder");
        let blocked = sandbox.path().join("blocked");
        fs::write(&blocked, b"a file where the store should be").expect("a blocking file");
        let store = sandbox.path().join("store");

        let body = called("Noto Sans");
        let mut lifting = Lifting::default();

        lifting.take_in(&blocked, &[a_font(11, "NotoSans", &body)]);
        assert_eq!(
            lifting.missed().collect::<Vec<_>>(),
            ["NotoSans"],
            "a face that could not be copied out has to come back by name or nobody hears of it"
        );

        lifting.take_in(&store, &[a_font(12, "NotoSans", &body)]);
        assert_eq!(
            lifting.missed().count(),
            0,
            "warning about a name is only right once the whole read is over and no container \
             gave the copy up, so a later landing has to clear the miss"
        );
        assert!(lifting.landed.contains_key("NotoSans"));
    }
}
