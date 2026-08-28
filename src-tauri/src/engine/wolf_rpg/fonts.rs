use crate::backup;
use crate::engine::fonts::Fonts;
use crate::engine::pictures::key_of;
use crate::engine::wolf_rpg::held::{Edits, Held};
use crate::engine::wolf_rpg::{archive, coder, game, reading, source, wolfx};
use crate::engine::{Extra, Font, Install, Landing, fonts as face};
use crate::scope::slashed;
use anyhow::Result;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const SLOT: &str = "font";

struct Shipped {
    family: String,
    at: String,
    shown: String,
}

struct Drawn {
    family: String,
    file: Option<PathBuf>,
    named: String,
    shown: String,
}

fn loose(game_dir: &Path) -> Vec<Shipped> {
    let mut found = Vec::new();

    for one in face::faces(game_dir, game_dir) {
        if face::ours(&one.name) {
            continue;
        }

        let Ok(body) = fs::read(&one.shown) else {
            continue;
        };

        for family in face::families(&body) {
            found.push(Shipped {
                family,
                at: one.at.clone(),
                shown: one.shown.clone(),
            });
        }
    }

    found
}

fn packed(game_dir: &Path, store: &Path) -> (Vec<Shipped>, Vec<PathBuf>) {
    let mut found = Vec::new();
    let mut lifted = Vec::new();

    for one in source::archives(game_dir) {
        let from = face::untouched(store, game_dir, &one);
        let Ok(Some(sealed)) = archive::key_for(&from, source::weight(game_dir)) else {
            continue;
        };
        let holder = slashed(one.strip_prefix(game_dir).unwrap_or(&one));

        let _ = archive::poured(&from, &sealed, Path::new(""), face::is_face, |held| {
            let at = key_of(&holder, Some(&slashed(&held.at)));
            let Some(copy) = face::lift(store, &at, &held.body) else {
                return Ok(());
            };

            for family in face::families(&held.body) {
                found.push(Shipped {
                    family,
                    at: at.clone(),
                    shown: copy.to_string_lossy().to_string(),
                });
            }
            lifted.push(copy);

            Ok(())
        });
    }

    (found, lifted)
}

fn beside(game_dir: &Path, store: &Path) -> (Vec<Shipped>, Vec<PathBuf>) {
    let mut found = loose(game_dir);
    let (packed, lifted) = packed(game_dir, store);

    found.extend(packed);

    (found, lifted)
}

pub fn slot_of(at: &Path) -> Option<usize> {
    let bare = match source::suffixed(at, source::SEAL) {
        true => Path::new(at.file_stem()?),
        false => at,
    };

    if !face::is_face(bare) {
        return None;
    }

    let stem = bare.file_stem()?.to_str()?.to_ascii_lowercase();
    let told = stem.strip_prefix(SLOT)?;

    match told.len() == 1 && told.starts_with(|one: char| one.is_ascii_digit()) {
        true => told.parse().ok().filter(|slot| *slot < 1 + game::SUB_FONTS),
        false => None,
    }
}

fn bound(game_dir: &Path) -> BTreeMap<usize, PathBuf> {
    let Ok(listed) = fs::read_dir(game_dir) else {
        return BTreeMap::new();
    };

    listed
        .filter_map(std::result::Result::ok)
        .map(|one| one.path())
        .filter_map(|at| Some((slot_of(&at)?, at)))
        .collect()
}

fn face_of(store: &Path, game_dir: &Path, at: &Path) -> Option<Vec<u8>> {
    let raw = fs::read(face::untouched(store, game_dir, at)).ok()?;

    match wolfx::seals(&raw) {
        true => wolfx::opened(&raw),
        false => Some(raw),
    }
}

fn drawn(game_dir: &Path, store: &Path, held: &Held) -> Vec<Drawn> {
    let bound = bound(game_dir);
    let (beside, mut lifted) = beside(game_dir, store);
    let mut out = Vec::new();

    for (slot, said) in held.fonts().iter().enumerate() {
        let told = said.text.trim();

        let Some(at) = bound.get(&slot) else {
            if told.is_empty() {
                continue;
            }

            let found = beside.iter().find(|one| one.family == told);
            out.push(Drawn {
                family: told.to_string(),
                file: None,
                named: found.map(|one| one.at.clone()).unwrap_or_default(),
                shown: found.map(|one| one.shown.clone()).unwrap_or_default(),
            });

            continue;
        };

        let Some(body) = face_of(store, game_dir, at) else {
            continue;
        };

        let named = slashed(at.strip_prefix(game_dir).unwrap_or(at));
        let family = match told.is_empty() {
            false => told.to_string(),
            true => match face::families(&body).into_iter().next() {
                Some(family) => family,
                None => continue,
            },
        };

        let copy = face::lift(store, &named, &body);
        let shown = copy
            .as_ref()
            .map(|one| one.to_string_lossy().to_string())
            .unwrap_or_default();
        lifted.extend(copy);

        out.push(Drawn {
            family,
            file: Some(at.clone()),
            named,
            shown,
        });
    }

    face::swept(store, &lifted);

    out
}

fn named(held: &Held) -> Vec<&str> {
    let mut found: Vec<&str> = Vec::new();

    for said in held.fonts() {
        let family = said.text.trim();
        if !family.is_empty() && !found.contains(&family) {
            found.push(family);
        }
    }

    found
}

pub fn faces(game_dir: &Path, held: &Held, store: &Path) -> Vec<Font> {
    let mut out: Vec<Font> = Vec::new();

    for one in drawn(game_dir, store, held) {
        if out.iter().any(|held| held.name == one.family) {
            continue;
        }

        out.push(Font {
            name: one.family,
            builtin: false,
            at: one.named,
            shown: one.shown,
        });
    }

    out
}

#[derive(Default)]
pub struct Done {
    pub swapped: u32,
    pub given: u32,
}

pub async fn swapped_in(at: &Install<'_>, root: &Path) -> Result<Done> {
    let bound = bound(at.game_dir);
    if bound.is_empty() {
        return Ok(Done::default());
    }

    let mut out = Done::default();

    if at.reverting {
        for one in bound.values() {
            if backup::put_back(at.store, at.game_dir, one).await? {
                out.given += 1;
            }
        }

        return Ok(out);
    }

    let held = reading::game_now(at.store, at.game_dir, root)
        .ok_or_else(|| anyhow::anyhow!("{} could not be read", source::game_dat(root).display()))?;
    let picked = at.fonts.picked().await?;

    for one in drawn(at.game_dir, at.store, &held) {
        let Some(file) = one.file else {
            continue;
        };

        let fresh = at
            .fonts
            .sent_to(&one.family)
            .and_then(|from| picked.get(from))
            .filter(|body| face::family(body).is_some());

        let Some(body) = fresh else {
            if backup::put_back(at.store, at.game_dir, &file).await? {
                out.given += 1;
            }

            continue;
        };

        let raw = backup::original(at.store, at.game_dir, &file).await?;
        let out_body = match wolfx::seals(&raw) {
            true => match wolfx::sealed(&raw, body) {
                Some(sealed) => sealed,
                None => continue,
            },
            false => body.clone(),
        };

        backup::replace(at.store, at.game_dir, &file, out_body).await?;
        out.swapped += 1;
    }

    Ok(out)
}

fn drawn_by_name(at: Landing<'_>) -> Option<Vec<String>> {
    let root = source::root(at.game_dir, at.store);
    let held = reading::game_now(at.store, at.game_dir, &root)?;

    Some(
        drawn(at.game_dir, at.store, &held)
            .into_iter()
            .filter(|one| one.file.is_none())
            .map(|one| one.family)
            .collect(),
    )
}

pub fn carried(at: Landing<'_>, fonts: &Fonts) -> Vec<Extra> {
    let swaps = match drawn_by_name(at) {
        Some(named) => fonts
            .swaps
            .iter()
            .filter(|one| named.contains(&one.from))
            .cloned()
            .collect(),
        None => fonts.swaps.clone(),
    };

    face::carried(&face::landed(&Fonts { swaps }), Path::new(""))
}

pub async fn tidied(at: &Install<'_>) -> u32 {
    face::tidied(at.game_dir, at).await
}

#[derive(Default)]
pub struct Sending {
    pub each: Vec<(String, String)>,
}

impl Sending {
    fn sent_to(&self, family: &str) -> Option<&str> {
        self.each
            .iter()
            .find(|(from, _)| from == family)
            .map(|(_, to)| to.as_str())
    }
}

pub async fn sending(at: &Install<'_>, held: &Held) -> Result<Sending> {
    if at.reverting {
        return Ok(Sending::default());
    }

    let picked = at.fonts.picked().await?;
    if picked.is_empty() {
        return Ok(Sending::default());
    }

    let mut each: Vec<(String, String)> = Vec::new();

    for family in named(held) {
        let Some(from) = at.fonts.sent_to(family) else {
            continue;
        };

        let to = picked
            .get(from)
            .and_then(|body| face::family(body))
            .ok_or_else(|| anyhow::anyhow!("{from} does not say what family it belongs to"))?;

        each.push((family.to_string(), to));
    }

    Ok(Sending { each })
}

pub fn told(held: &Held, sending: &Sending) -> Edits {
    let mut edits = Vec::new();

    for said in held.fonts() {
        let family = said.text.trim();
        let Some(to) = sending.sent_to(family) else {
            continue;
        };
        if to == family {
            continue;
        }

        edits.push((said.at.clone(), coder::line(to)));
    }

    edits
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Swap;
    use crate::engine::wolf_rpg::{fixture, game, held};
    use crate::progress::Quiet;

    const GOTHIC: &str = "\u{ff2d}\u{ff33} \u{30b4}\u{30b7}\u{30c3}\u{30af}";

    fn laid_out() -> (tempfile::TempDir, tempfile::TempDir, Held) {
        let at = fixture::sandbox();
        let store = fixture::sandbox();
        let root = at.path();
        let data = root.join(source::DATA);
        fs::create_dir_all(&data).unwrap();

        let called = face::fake::called;
        fs::write(root.join("PixelifySans.ttf"), called("Pixelify Sans")).unwrap();
        fs::write(data.join("08gothic.ttf"), called(GOTHIC)).unwrap();

        let held = game::read(&fixture::drawn_by(
            "Title",
            "",
            &["Pixelify Sans", GOTHIC, "MS Gothic"],
        ))
        .expect("Game.dat");

        (at, store, held)
    }

    #[test]
    fn a_face_carries_the_file_that_draws_it_and_nothing_at_all_when_the_system_does() {
        let (at, store, held) = laid_out();

        assert_eq!(
            faces(at.path(), &held, store.path())
                .iter()
                .map(|one| (one.name.as_str(), one.at.as_str()))
                .collect::<Vec<_>>(),
            [
                ("Pixelify Sans", "PixelifySans.ttf"),
                (GOTHIC, "Data/08gothic.ttf"),
                ("MS Gothic", ""),
            ],
            "a family only Windows ships has no file to point at, and naming one anyway sends \
             the reader a spot on disk that is not there"
        );
    }

    #[test]
    fn a_face_the_game_keeps_inside_an_archive_is_drawn_for_the_reader_all_the_same() {
        let at = fixture::sandbox();
        let store = fixture::sandbox();
        let root = at.path();
        let data = root.join(source::DATA);
        fs::create_dir_all(&data).unwrap();

        let body = face::fake::called("Pixelify Sans");
        fs::write(
            data.join("Locker.wolf"),
            archive::archived(&[("font/PixelifySans.ttf", body.as_slice())], None),
        )
        .unwrap();
        fs::write(root.join("Game.exe"), [0; 4]).unwrap();

        let held =
            game::read(&fixture::drawn_by("Title", "", &["Pixelify Sans"])).expect("Game.dat");
        let found = faces(root, &held, store.path());

        assert_eq!(
            found.iter().map(|one| one.at.as_str()).collect::<Vec<_>>(),
            ["Data/Locker.wolf|font/PixelifySans.ttf"],
            "a face the game packed away is still a face, and the row has to say which archive \
             it came out of"
        );
        assert_eq!(
            fs::read(&found[0].shown).expect("the copy lifted out of the archive"),
            body,
            "the copy a reader draws the sample with has to be the very bytes the game draws with"
        );
    }

    #[test]
    fn a_font_the_game_keeps_sealed_is_drawn_and_a_slot_that_will_not_open_is_not_offered() {
        let at = fixture::sandbox();
        let store = fixture::sandbox();
        let root = at.path();
        fs::write(root.join("Game.exe"), [0; 4]).unwrap();

        let body = face::fake::called("Pixelify Sans");
        fs::write(root.join("font0.ttf.wolfx"), wolfx::as_shipped(&body)).unwrap();
        fs::write(root.join("font1.ttf.wolfx"), b"WOLFX and nothing else").unwrap();

        let held = game::read(&fixture::drawn_by(
            "Title",
            "",
            &["Pixelify Sans", GOTHIC, "MS Gothic"],
        ))
        .expect("Game.dat");

        let found = faces(root, &held, store.path());

        assert_eq!(
            found
                .iter()
                .map(|one| (one.name.as_str(), one.at.as_str()))
                .collect::<Vec<_>>(),
            [("Pixelify Sans", "font0.ttf.wolfx"), ("MS Gothic", ""),],
            "the engine draws a slot it has a file for out of that file and never looks at the \
             name, so a slot whose file this reader cannot open is one no pick could ever reach \
             and offering it would leave the reader waiting on a change that is not coming"
        );
        assert_eq!(
            fs::read(&found[0].shown).expect("the copy lifted out of the seal"),
            body,
            "and the face drawn for the reader is the very one the game draws with"
        );
    }

    #[test]
    fn one_family_named_in_every_slot_is_one_row_and_every_slot_takes_the_pick() {
        let held = game::read(&fixture::drawn_by(
            "Title",
            "",
            &["MS Gothic", "MS Gothic", "MS Gothic", "MS Gothic"],
        ))
        .expect("Game.dat");

        assert_eq!(
            faces(
                Path::new("/no/such/game"),
                &held,
                Path::new("/no/such/store")
            )
            .len(),
            1,
            "the reader picks against a family, and four rows saying the same thing is four \
             chances to disagree with themselves"
        );

        let sending = Sending {
            each: vec![("MS Gothic".to_string(), "Sarabun".to_string())],
        };
        let fresh = held::wrapped(&held, told(&held, &sending)).expect("a whole Game.dat");

        assert_eq!(
            game::read(&fresh)
                .expect("it still reads")
                .fonts()
                .iter()
                .map(|one| one.text.as_str())
                .collect::<Vec<_>>(),
            ["Sarabun"; 4],
            "one pick has to reach every slot that named the family, or the game draws some of \
             its text with a face that cannot spell it"
        );
    }

    #[test]
    fn a_picked_font_is_written_in_under_the_family_it_belongs_to() {
        let (at, store, held) = laid_out();
        let sending = Sending {
            each: vec![("Pixelify Sans".to_string(), "Sarabun".to_string())],
        };

        let edits = told(&held, &sending);
        assert_eq!(edits.len(), 1);

        let fresh = held::wrapped(&held, edits).expect("a whole Game.dat");
        let after = game::read(&fresh).expect("it still reads");

        assert_eq!(
            after
                .fonts()
                .iter()
                .map(|one| one.text.as_str())
                .collect::<Vec<_>>(),
            ["Sarabun", GOTHIC, "MS Gothic", ""],
            "the engine draws with whatever family the slot names, so the name is the whole swap"
        );
        assert!(
            faces(at.path(), &after, store.path())
                .iter()
                .any(|one| one.name == "Sarabun"),
            "and the face that is now in the file is the one a reader would see next"
        );
    }

    #[test]
    fn a_family_already_naming_the_picked_font_is_left_untouched() {
        let (_at, _store, held) = laid_out();
        let sending = Sending {
            each: vec![("Pixelify Sans".to_string(), "Pixelify Sans".to_string())],
        };

        assert!(
            told(&held, &sending).is_empty(),
            "writing a name the slot already holds would mark Game.dat as changed for nothing"
        );
        assert!(told(&held, &Sending::default()).is_empty());
    }

    #[tokio::test]
    async fn a_font_carried_in_under_a_pick_since_let_go_of_does_not_stay_beside_the_game() {
        let at = fixture::sandbox();
        let root = at.path();
        let store = fixture::sandbox();

        for name in [
            "anylang-noto.ttf",
            "anylang-sarabun.ttf",
            "Game.exe",
            "shipped.ttf",
        ] {
            fs::write(root.join(name), [0u8; 4]).unwrap();
        }

        let fonts = Fonts {
            swaps: vec![Swap {
                from: "MS Gothic".to_string(),
                to: "/fonts/sarabun.ttf".to_string(),
            }],
        };
        let quiet = Quiet;
        let told = |reverting| {
            Install::over(root, root, store.path())
                .sending(&fonts)
                .putting_back(reverting)
                .heard_by(&quiet)
        };

        let left = || {
            let mut found: Vec<String> = fs::read_dir(root)
                .expect("the folder")
                .filter_map(Result::ok)
                .map(|one| one.file_name().to_string_lossy().to_string())
                .collect();
            found.sort();

            found
        };

        assert_eq!(tidied(&told(false)).await, 1);
        assert_eq!(
            left(),
            vec![
                "Game.exe".to_string(),
                "anylang-sarabun.ttf".to_string(),
                "shipped.ttf".to_string(),
            ],
            "the engine reads every font beside the game, so a font no pick lands as any more \
             would keep answering to its family"
        );

        assert_eq!(
            tidied(&told(true)).await,
            1,
            "asking for the game back leaves none of ours behind"
        );
        assert_eq!(
            left(),
            vec!["Game.exe".to_string(), "shipped.ttf".to_string()],
            "and a face the game shipped is never one of ours to take"
        );
    }

    #[tokio::test]
    async fn a_font_a_slot_draws_from_a_file_goes_into_that_file_and_comes_back_out_on_revert() {
        let at = fixture::sandbox();
        let store = fixture::sandbox();
        let root = at.path();

        let basic = root.join(source::DATA).join(source::BASIC);
        fs::create_dir_all(&basic).unwrap();
        fs::write(root.join("Game.exe"), [0; 4]).unwrap();
        fs::write(
            basic.join("Game.dat"),
            fixture::drawn_by("Title", "", &["Sealed Face", "Plain Face"]),
        )
        .unwrap();

        let shipped = face::fake::called("Sealed Face");
        fs::write(root.join("font0.ttf.wolfx"), wolfx::as_shipped(&shipped)).unwrap();
        fs::write(root.join("font1.ttf"), face::fake::called("Plain Face")).unwrap();

        let picked = store.path().join("picked.ttf");
        let body = face::fake::called("Sarabun");
        fs::write(&picked, &body).unwrap();

        let fonts = Fonts {
            swaps: ["Sealed Face", "Plain Face"]
                .into_iter()
                .map(|from| Swap {
                    from: from.to_string(),
                    to: picked.to_string_lossy().to_string(),
                })
                .collect(),
        };

        let quiet = Quiet;
        let staged = store.path().join("staged");
        let told = |reverting| {
            Install::over(root, &staged, store.path())
                .sending(&fonts)
                .putting_back(reverting)
                .heard_by(&quiet)
        };

        let done = swapped_in(&told(false), root).await.expect("both go in");
        assert_eq!(done.swapped, 2);

        let sealed = fs::read(root.join("font0.ttf.wolfx")).unwrap();
        assert_eq!(
            wolfx::opened(&sealed).as_deref(),
            Some(body.as_slice()),
            "the game draws this slot straight out of its own sealed file, so the pick has to go \
             back inside that seal or the engine draws nothing at all"
        );
        assert_eq!(fs::read(root.join("font1.ttf")).unwrap(), body);

        let done = swapped_in(&told(true), root).await.expect("both come out");
        assert_eq!(done.given, 2);
        assert_eq!(
            wolfx::opened(&fs::read(root.join("font0.ttf.wolfx")).unwrap()).as_deref(),
            Some(shipped.as_slice()),
            "asking for the game back has to give it its own face again"
        );
    }

    #[tokio::test]
    async fn a_sealed_font_this_reader_cannot_open_is_left_alone_and_never_offered() {
        let at = fixture::sandbox();
        let store = fixture::sandbox();
        let root = at.path();

        let basic = root.join(source::DATA).join(source::BASIC);
        fs::create_dir_all(&basic).unwrap();
        fs::write(root.join("Game.exe"), [0; 4]).unwrap();
        fs::write(
            basic.join("Game.dat"),
            fixture::drawn_by("Title", "", &["MS Gothic", "MS Gothic"]),
        )
        .unwrap();

        let shut = b"WOLFX and nothing this reader holds the key to".to_vec();
        fs::write(root.join("font0.ttf.wolfx"), &shut).unwrap();

        let held = game::read(&fs::read(basic.join("Game.dat")).unwrap()).expect("Game.dat");
        assert_eq!(
            faces(root, &held, store.path())
                .iter()
                .map(|one| one.name.as_str())
                .collect::<Vec<_>>(),
            ["MS Gothic"],
            "the second slot reads this family by name, so the row is still the reader's to swap"
        );

        let picked = store.path().join("picked.ttf");
        fs::write(&picked, face::fake::called("Sarabun")).unwrap();

        let fonts = Fonts {
            swaps: vec![Swap {
                from: "MS Gothic".to_string(),
                to: picked.to_string_lossy().to_string(),
            }],
        };

        let quiet = Quiet;
        let staged = store.path().join("staged");
        let done = swapped_in(
            &Install::over(root, &staged, store.path())
                .sending(&fonts)
                .heard_by(&quiet),
            root,
        )
        .await
        .expect("a slot this reader cannot open is skipped, not failed over");

        assert_eq!(done.swapped, 0);
        assert_eq!(
            fs::read(root.join("font0.ttf.wolfx")).unwrap(),
            shut,
            "nothing was ever promised for this slot, so nothing may be written into it, and \
             least of all bytes sealed with a key that is not the game's"
        );
    }

    #[test]
    fn a_font_for_a_slot_the_game_draws_out_of_its_own_file_is_not_carried_in_beside_it() {
        let at = fixture::sandbox();
        let store = fixture::sandbox();
        let root = at.path();

        let basic = root.join(source::DATA).join(source::BASIC);
        fs::create_dir_all(&basic).unwrap();
        fs::write(root.join("Game.exe"), [0; 4]).unwrap();
        fs::write(
            basic.join("Game.dat"),
            fixture::drawn_by("Title", "", &["Pixelify Sans", GOTHIC]),
        )
        .unwrap();
        fs::write(
            root.join("font0.ttf.wolfx"),
            wolfx::as_shipped(&face::fake::called("Pixelify Sans")),
        )
        .unwrap();

        let fonts = Fonts {
            swaps: vec![
                Swap {
                    from: "Pixelify Sans".to_string(),
                    to: "/fonts/sarabun.ttf".to_string(),
                },
                Swap {
                    from: GOTHIC.to_string(),
                    to: "/fonts/comic.otf".to_string(),
                },
            ],
        };

        assert_eq!(
            carried(Landing::over(root, store.path(), "japanese"), &fonts,)
                .into_iter()
                .map(|extra| extra.at().to_string_lossy().to_string())
                .collect::<Vec<String>>(),
            vec![format!("{}-comic.otf", face::CARRIED)],
            "the engine draws the first slot straight out of the file it ships and never looks at \
             a name for it, so a copy carried in for that family would sit beside the game doing \
             nothing"
        );
    }

    #[test]
    fn every_font_a_reader_picks_lands_beside_the_game_once_under_a_name_of_its_own() {
        let fonts = Fonts {
            swaps: vec![
                Swap {
                    from: "Pixelify Sans".to_string(),
                    to: "/fonts/sarabun.ttf".to_string(),
                },
                Swap {
                    from: "MS Gothic".to_string(),
                    to: "/fonts/sarabun.ttf".to_string(),
                },
                Swap {
                    from: GOTHIC.to_string(),
                    to: "/fonts/comic.otf".to_string(),
                },
            ],
        };

        let nowhere = Landing::over(
            Path::new("/no/such/game"),
            Path::new("/no/such/store"),
            "japanese",
        );

        assert_eq!(
            carried(nowhere, &fonts)
                .into_iter()
                .map(|extra| extra.at().to_string_lossy().to_string())
                .collect::<Vec<String>>(),
            vec![
                format!("{}-sarabun.ttf", face::CARRIED),
                format!("{}-comic.otf", face::CARRIED)
            ],
            "the engine reads every font file beside the game, so one copy answers both \
             families, and the name says both who put it there and which font it is"
        );
    }
}
