use crate::engine::fonts::Fonts;
use crate::engine::pictures::key_of;
use crate::engine::renpy::{GAME_DIR, TL_DIR, archive, slug};
use crate::engine::{Extra, Font, Install, fonts as face};
use crate::scope::slashed;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const FONT_DIR: &str = "fonts";

const BUILT_IN: &str = "renpy/";

const PACKED: &str = "rpa";

fn ours(at: &str) -> bool {
    at.strip_prefix(&format!("{GAME_DIR}/{TL_DIR}/"))
        .and_then(|rest| rest.split_once('/'))
        .is_some_and(|(_, rest)| rest.starts_with(&format!("{FONT_DIR}/")))
}

fn archives(inside: &Path) -> Vec<PathBuf> {
    WalkDir::new(inside)
        .follow_links(true)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|one| one.file_type().is_file())
        .map(walkdir::DirEntry::into_path)
        .filter(|at| at.extension().is_some_and(|kind| kind == PACKED))
        .collect()
}

fn packed(game_dir: &Path, store: &Path) -> (Vec<Font>, Vec<PathBuf>) {
    let mut found = Vec::new();
    let mut lifted = Vec::new();

    for at in archives(&game_dir.join(GAME_DIR)) {
        let (Ok(index), Ok(mut spool)) = (archive::listed(&at), archive::Spool::over(&at)) else {
            continue;
        };
        let holder = slashed(at.strip_prefix(game_dir).unwrap_or(&at));

        for (inside, held) in index.iter() {
            if !face::is_face(Path::new(inside)) || ours(&format!("{GAME_DIR}/{inside}")) {
                continue;
            }

            let at = key_of(&holder, Some(inside));
            let Ok(body) = spool.read(held) else {
                continue;
            };
            let Some(copy) = face::lift(store, &at, &body) else {
                continue;
            };

            found.push(Font {
                name: face::by_name(Path::new(inside)).unwrap_or_else(|| inside.clone()),
                at,
                shown: copy.to_string_lossy().to_string(),
                builtin: false,
            });
            lifted.push(copy);
        }
    }

    (found, lifted)
}

pub fn faces(game_dir: &Path, store: &Path) -> Vec<Font> {
    let mut found = face::faces(game_dir, game_dir);
    found.retain(|one| !ours(&one.at));
    for one in &mut found {
        one.builtin = one.at.starts_with(BUILT_IN);
    }

    let (packed, lifted) = packed(game_dir, store);
    face::swept(store, &lifted);

    for one in packed {
        if !found.iter().any(|loose| loose.name == one.name) {
            found.push(one);
        }
    }

    found.sort_by(|a, b| a.builtin.cmp(&b.builtin).then(a.at.cmp(&b.at)));

    found
}

fn font_dir(language: &str) -> PathBuf {
    Path::new(TL_DIR).join(slug(language)).join(FONT_DIR)
}

pub fn landing_of(language: &str, name: &str) -> String {
    slashed(&font_dir(language).join(name))
}

pub fn landings(fonts: &Fonts) -> Vec<(String, String)> {
    face::landed(fonts)
}

pub async fn tidied(at: &Install<'_>) -> u32 {
    face::tidied(&at.staged.join(FONT_DIR), at).await
}

pub fn carried(language: &str, placed: &[(String, String)]) -> Vec<Extra> {
    face::carried(placed, &PathBuf::from(GAME_DIR).join(font_dir(language)))
}

#[cfg(test)]
mod tests {
    use crate::engine::Landing;

    fn landing(language: &'static str) -> Landing<'static> {
        Landing {
            game_dir: Path::new("/game"),
            store: Path::new("/store"),
            language,
        }
    }
    use super::*;
    use crate::engine::renpy::switch::switched;
    use crate::engine::renpy::{Options, RenPy};
    use crate::engine::{Engine, Swap, Tweaks};
    use crate::progress::Quiet;
    use std::fs;

    fn tweaks() -> Tweaks {
        Tweaks::RenPy(Options::default())
    }

    fn swapping(swaps: &[(&str, &str)]) -> Fonts {
        Fonts {
            swaps: swaps
                .iter()
                .map(|(from, to)| Swap {
                    from: (*from).to_string(),
                    to: (*to).to_string(),
                })
                .collect(),
        }
    }

    #[test]
    fn a_face_named_twice_is_carried_into_the_game_once() {
        let wanted = RenPy.extras(
            landing("French"),
            &tweaks(),
            &swapping(&[("a.ttf", "/y/Charm.otf"), ("b.ttf", "/y/Charm.otf")]),
        );

        let copied: Vec<String> = wanted
            .iter()
            .filter_map(|one| match one {
                Extra::Copy { from, at } => Some(format!("{} -> {}", from.display(), at.display())),
                Extra::Write { .. } => None,
            })
            .collect();

        assert_eq!(
            copied,
            ["/y/Charm.otf -> game/tl/french/fonts/anylang-Charm.otf"],
            "a face named in the map has to be beside the game or the map points at nothing; it \
             lands inside the translation so the whole of it travels as one folder, and one file \
             named twice is still carried once"
        );
    }

    #[test]
    fn two_faces_named_alike_are_carried_under_names_of_their_own() {
        let fonts = swapping(&[
            ("gui/one.ttf", "/a/Noto.ttf"),
            ("gui/old.ttf", "/b/Noto.ttf"),
        ]);

        let carried: Vec<(PathBuf, PathBuf)> = RenPy
            .extras(landing("Japanese"), &tweaks(), &fonts)
            .into_iter()
            .filter_map(|extra| match extra {
                Extra::Copy { from, at } => Some((from, at)),
                Extra::Write { .. } => None,
            })
            .collect();

        assert_eq!(
            carried.len(),
            2,
            "two files chosen means two files copied: {carried:?}"
        );
        assert_ne!(
            carried[0].1, carried[1].1,
            "one name may not be written over the other, or a face renders with another's glyphs"
        );

        let body = switched("Japanese", &tweaks(), &fonts).unwrap();
        for (_, at) in &carried {
            let named = at.file_name().expect("a file name").to_string_lossy();
            assert!(
                body.contains(&format!("\"tl/japanese/{FONT_DIR}/{named}\"")),
                "the map has to name the file the copy actually placed: {body}"
            );
        }
    }

    #[test]
    fn without_a_font_only_the_language_file_is_written() {
        let wanted = RenPy.extras(landing("Japanese"), &tweaks(), &Fonts::default());

        assert_eq!(wanted.len(), 1);
        assert!(matches!(wanted[0], Extra::Write { .. }));
    }

    #[test]
    fn a_face_we_carried_in_last_time_is_not_offered_back_as_one_of_the_games() {
        let sandbox = tempfile::tempdir().expect("a temp folder");
        let store = tempfile::tempdir().expect("a store");
        let game = sandbox.path();

        for at in [
            "game/gui/Lato-Regular.ttf",
            "game/tl/japanese/fonts/mine.ttf",
            "game/tl/korean/fonts/mine.ttf",
        ] {
            let at = game.join(at);
            fs::create_dir_all(at.parent().expect("a folder")).expect("a folder");
            fs::write(&at, []).expect("a font");
        }

        let found: Vec<String> = faces(game, store.path())
            .into_iter()
            .map(|one| one.at)
            .collect();

        assert_eq!(
            found,
            ["game/gui/Lato-Regular.ttf"],
            "carried() lands a chosen face inside the translation, and offering that back as a \
             face to replace would let a reader map our own file over itself"
        );
    }

    #[tokio::test]
    async fn a_face_carried_in_under_a_pick_since_let_go_of_does_not_stay_in_the_translation() {
        let sandbox = tempfile::tempdir().expect("a temp folder");
        let store = tempfile::tempdir().expect("a store");
        let staged = sandbox.path().join(GAME_DIR).join(TL_DIR).join("french");
        let inside = staged.join(FONT_DIR);
        fs::create_dir_all(&inside).expect("a folder");

        for name in ["anylang-NotoSans.ttf", "anylang-Charm.otf", "theirs.ttf"] {
            fs::write(inside.join(name), [0u8; 4]).expect("a font");
        }

        let fonts = swapping(&[("gui/old.ttf", "/x/NotoSans.ttf")]);
        let quiet = Quiet;
        let told = |reverting| {
            Install::over(sandbox.path(), &staged, store.path())
                .sending(&fonts)
                .putting_back(reverting)
                .heard_by(&quiet)
        };

        let left = || {
            let mut found: Vec<String> = fs::read_dir(&inside)
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
            ["anylang-NotoSans.ttf", "theirs.ttf"],
            "a face carried in under a pick the reader has since dropped is dead weight in the \
             player's game, and a face the game shipped in its own translation is not ours to \
             throw away"
        );

        assert_eq!(tidied(&told(true)).await, 1);
        assert_eq!(
            left(),
            ["theirs.ttf"],
            "putting the game back takes every face we carried in with it"
        );
    }

    #[test]
    fn a_face_packed_into_an_archive_is_offered_and_the_loose_copy_wins_a_clash() {
        let sandbox = tempfile::tempdir().expect("a temp folder");
        let store = tempfile::tempdir().expect("a store");
        let game = sandbox.path();
        let inside = game.join(GAME_DIR);
        fs::create_dir_all(inside.join("gui")).expect("a folder");

        let body = face::fake::called("Lato");
        fs::write(inside.join("gui").join("Lato-Regular.ttf"), &body).expect("a loose font");
        fs::write(
            inside.join("archive.rpa"),
            archive::sealed(
                &[
                    ("gui/Lato-Regular.ttf", &body, 0),
                    ("fonts/Zenda.otf", &face::fake::called("Zenda"), 0),
                    ("tl/japanese/fonts/mine.ttf", &body, 0),
                    ("audio/theme.ogg", b"not a font", 0),
                ],
                0x4242_4242,
                false,
            ),
        )
        .expect("an archive");

        let found = faces(game, store.path());

        assert_eq!(
            found
                .iter()
                .map(|one| (one.name.as_str(), one.at.as_str()))
                .collect::<Vec<_>>(),
            [
                ("Zenda.otf", "game/archive.rpa|fonts/Zenda.otf"),
                ("Lato-Regular.ttf", "game/gui/Lato-Regular.ttf"),
            ],
            "a face the build packed away is one the game still draws with, and the loose copy \
             wins a clash because Ren'Py reads the folder before the archive; a face we carried \
             in ourselves is not offered back even from inside an archive"
        );
        assert_eq!(
            fs::read(&found[0].shown).expect("the copy lifted out of the archive"),
            face::fake::called("Zenda"),
            "the copy a reader draws the sample with has to be the very bytes the game draws with"
        );
    }

    #[test]
    fn a_font_the_engine_ships_is_told_apart_from_one_the_game_brought() {
        let sandbox = tempfile::tempdir().expect("a temp folder");
        let store = tempfile::tempdir().expect("a store");
        let game = sandbox.path();

        for at in ["game/gui/Lato-Regular.ttf", "renpy/common/DejaVuSans.ttf"] {
            let at = game.join(at);
            fs::create_dir_all(at.parent().expect("a folder")).expect("a folder");
            fs::write(&at, []).expect("a font");
        }
        fs::write(game.join("game").join("notes.txt"), []).expect("a stray file");

        let found = faces(game, store.path());

        assert_eq!(
            found
                .iter()
                .map(|one| (one.at.as_str(), one.builtin))
                .collect::<Vec<_>>(),
            [
                ("game/gui/Lato-Regular.ttf", false),
                ("renpy/common/DejaVuSans.ttf", true)
            ],
            "the reader has to see which faces the game chose and which ones Ren'Py brought, \
             because only the first are worth keeping"
        );
        assert_eq!(found[0].name, "Lato-Regular.ttf");
    }
}
