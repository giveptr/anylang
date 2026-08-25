use crate::engine::fonts::Fonts;
use crate::engine::pictures::key_of;
use crate::engine::wolf_rpg::held::{Edits, Held};
use crate::engine::wolf_rpg::{archive, coder, source};
use crate::engine::{Extra, Font, Install, fonts as face};
use crate::scope::slashed;
use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

struct Shipped {
    family: String,
    at: String,
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

fn shipped(game_dir: &Path, store: &Path) -> Vec<Shipped> {
    let mut found = loose(game_dir);
    let (packed, lifted) = packed(game_dir, store);

    found.extend(packed);
    face::swept(store, &lifted);

    found
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
    let named = named(held);
    if named.is_empty() {
        return Vec::new();
    }

    let files = shipped(game_dir, store);

    named
        .into_iter()
        .map(|family| {
            let found = files.iter().find(|one| one.family == family);

            Font {
                name: family.to_string(),
                builtin: false,
                at: found.map(|one| one.at.clone()).unwrap_or_default(),
                shown: found.map(|one| one.shown.clone()).unwrap_or_default(),
            }
        })
        .collect()
}

pub fn carried(fonts: &Fonts) -> Vec<Extra> {
    face::carried(&face::landed(fonts), Path::new(""))
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

        assert_eq!(
            carried(&fonts)
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
