mod aes;
mod archive;
mod coder;
mod common;
mod database;
mod event;
#[cfg(test)]
mod fixture;
mod fonts;
mod game;
mod guard;
mod harvest;
mod held;
mod install;
mod keying;
mod map;
mod pictures;
mod prepare;
mod reached;
mod reading;
mod script;
mod sha;
mod source;
mod squeeze;
mod text;
mod unprot;
mod unseal;

use crate::engine::fonts::Fonts;
use crate::engine::pictures::{Handed, Shot};
use crate::engine::{
    Engine, Extra, Font, Install, Landing, Parsed, Prepare, Rules, Tweaks, Undo, sheet,
};
use anyhow::Result;
use futures::future::BoxFuture;
use std::borrow::Cow;
use std::path::{Path, PathBuf};

struct WolfRpg;

pub fn detect(dir: &Path) -> Option<Box<dyn Engine>> {
    source::holds_a_game(dir).then(|| Box::new(WolfRpg) as Box<dyn Engine>)
}

impl Engine for WolfRpg {
    fn label(&self) -> &str {
        "Wolf RPG"
    }

    fn wants(&self, path: &Path) -> bool {
        sheet::wants(path)
    }

    fn shown<'n>(&self, name: &'n str) -> Cow<'n, str> {
        sheet::shown(name)
    }

    fn parse(&self, _at: &Path, body: &str) -> Box<dyn Parsed> {
        Box::new(sheet::read(body, |_| false))
    }

    fn validate(&self, source: &str, translation: &str) -> Result<(), String> {
        text::validate(source, translation)
    }

    fn bare<'t>(&self, said: &'t str) -> Cow<'t, str> {
        text::RE_MARK.replace_all(said, "")
    }

    fn rules(&self) -> Rules {
        Rules {
            markup: text::MARKUP_RULES,
            shape: Some(text::SHAPE_RULES),
            retry: text::RETRY_RULES,
        }
    }

    fn output(&self, at: Landing<'_>) -> PathBuf {
        at.staged()
    }

    fn undo(&self) -> Undo {
        Undo::Remove
    }

    fn fonts(&self, game_dir: &Path, store: &Path) -> Vec<Font> {
        let root = source::root(game_dir, store);

        reading::opened_now(store, game_dir, &source::game_dat(&root))
            .and_then(|raw| game::read(&raw).ok())
            .map(|held| fonts::faces(game_dir, &held, store))
            .unwrap_or_default()
    }

    fn pictures(&self, store: &Path) -> Vec<Shot> {
        pictures::LEDGER.remembered(store)
    }

    fn picture(&self, game_dir: &Path, store: &Path, key: &str) -> Result<Handed> {
        Ok(Handed::Shipped(pictures::picture(game_dir, store, key)?))
    }

    fn extras(&self, _language: &str, _tweaks: &Tweaks, fonts: &Fonts) -> Vec<Extra> {
        fonts::carried(fonts)
    }

    fn prepare<'a>(&'a self, at: Prepare<'a>) -> BoxFuture<'a, Result<()>> {
        Box::pin(prepare::run(self.label(), at))
    }

    fn install<'a>(&'a self, at: Install<'a>) -> BoxFuture<'a, Result<()>> {
        install::run(at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::wolf_rpg::fixture::{self, sandbox};
    use crate::engine::{Offer, STAGED};
    use crate::progress::Quiet;
    use crate::scope::slashed;
    use crate::walk;
    use std::collections::BTreeMap;
    use std::fs;

    #[test]
    fn a_wolf_game_is_taken_for_one_by_the_data_folder_the_editor_writes() {
        let at = sandbox();
        let root = at.path();

        assert!(detect(root).is_none(), "an empty folder is no game");

        fixture::lay_out(root);
        let engine = detect(root).expect("a wolf game");

        assert_eq!(engine.label(), "Wolf RPG");
        assert!(engine.wants(Path::new("Dungeon.mps.sheet")));
        assert!(!engine.wants(Path::new("Dungeon.mps")));
        assert_eq!(engine.shown("Dungeon.mps.sheet"), "Dungeon.mps");
        assert_eq!(engine.undo(), Undo::Remove);
        assert_eq!(
            engine.output(Landing {
                game_dir: root,
                store: Path::new("/store/demo"),
                language: "Japanese",
            }),
            Path::new("/store/demo").join(STAGED).join("japanese"),
            "the game's own files are patched in place, so nothing lands beside them"
        );
    }

    #[test]
    fn an_rpg_maker_game_is_never_mistaken_for_a_wolf_one() {
        let at = sandbox();
        let root = at.path();
        fs::create_dir_all(root.join("Data")).unwrap();
        fs::write(root.join("Data").join("Map001.rvdata2"), [4, 8, b'0']).unwrap();
        fs::write(
            root.join("Game.ini"),
            "[Game]\nLibrary=System\\RGSS301.dll\n",
        )
        .unwrap();

        assert!(
            detect(root).is_none(),
            "RGSS keeps its data loose in Data too, and only Wolf writes a BasicData beside it"
        );
    }

    #[test]
    fn the_font_the_engine_draws_with_is_offered_to_the_reader() {
        let at = sandbox();
        let root = at.path();
        fixture::lay_out(root);

        let store = sandbox();
        let engine = detect(root).expect("a wolf game");

        assert_eq!(
            engine
                .fonts(root, store.path())
                .into_iter()
                .map(|one| (one.name, one.at))
                .collect::<Vec<(String, String)>>(),
            vec![("Pixelify Sans".to_string(), String::new())],
            "a Wolf game names its face in Game.dat and draws every letter with it, so a \
             translation into a writing that face has no glyph for is unreadable"
        );
    }

    #[tokio::test]
    async fn the_font_is_offered_just_the_same_when_the_game_keeps_its_data_packed() {
        let at = sandbox();
        let root = at.path();
        let store = sandbox();

        let data = root.join(source::DATA);
        fs::create_dir_all(&data).unwrap();
        fs::write(root.join("Game.exe"), [0; 4]).unwrap();
        fs::write(
            data.join("BasicData.wolf"),
            archive::archived(
                &[(
                    "Game.dat",
                    fixture::game("Title", "", "Pixelify Sans").as_slice(),
                )],
                None,
            ),
        )
        .unwrap();

        let engine = detect(root).expect("a wolf game");
        let quiet = Quiet;

        engine
            .prepare(
                Prepare::over(root, &store.path().join("source"), store.path()).heard_by(&quiet),
            )
            .await
            .expect("the game is read");

        assert_eq!(
            engine
                .fonts(root, store.path())
                .into_iter()
                .map(|one| one.name)
                .collect::<Vec<String>>(),
            ["Pixelify Sans"],
            "the Game.dat of a packed game is opened out into the store, which is outside the \
             game, so looking for it through the backup finds nothing and the reader is offered \
             no face to swap at all"
        );
    }

    #[tokio::test]
    async fn a_game_goes_out_as_sheets_and_a_translation_lands_back_in_its_own_files() {
        let at = sandbox();
        let root = at.path();
        let store = sandbox();
        let source = store.path().join("source");
        let staged = store.path().join("staged");
        fs::create_dir_all(&staged).unwrap();
        fixture::lay_out(root);

        let engine = detect(root).expect("a wolf game");
        let quiet = Quiet;

        engine
            .prepare(Prepare::over(root, &source, store.path()).heard_by(&quiet))
            .await
            .expect("the game is read out into sheets");

        let mut written: Vec<String> = walk::relative(&source)
            .await
            .into_iter()
            .map(|at| slashed(&at))
            .collect();
        written.sort();

        assert_eq!(
            written,
            [
                "BasicData/CommonEvent.dat.sheet",
                "BasicData/DataBase.dat.sheet",
                "BasicData/Game.dat.sheet",
                "MapData/Dungeon.mps.sheet",
            ],
            "each sheet sits where the file it came from sits"
        );

        let page =
            fs::read_to_string(source.join("MapData/Dungeon.mps.sheet")).expect("the map sheet");
        let taken = sheet::lines(&page).expect("its lines");
        assert_eq!(
            taken.values().next().map(String::as_str),
            Some("\u{6249}\u{306f}\u{9589}\u{307e}\u{3063}\u{3066}\u{3044}\u{308b}")
        );

        let landing = staged.join("MapData");
        fs::create_dir_all(&landing).unwrap();
        let said: BTreeMap<String, String> = taken
            .keys()
            .map(|spot| (spot.clone(), "The door is locked tight.".to_string()))
            .collect();
        fs::write(
            landing.join("Dungeon.mps.sheet"),
            sheet::write(said).expect("a sheet"),
        )
        .unwrap();

        let items = fs::read_to_string(source.join("BasicData/DataBase.dat.sheet"))
            .expect("the database sheet");
        let items = sheet::lines(&items).expect("its lines");
        assert_eq!(items["t0/d0/f0/s0"], "\u{7dd1}\u{8336}");

        fs::create_dir_all(staged.join("BasicData")).unwrap();
        fs::write(
            staged.join("BasicData").join("DataBase.dat.sheet"),
            sheet::write([("t0/d0/f0/s0".to_string(), "Green Tea".to_string())]).expect("a sheet"),
        )
        .unwrap();

        let told = Install::over(root, &staged, store.path()).heard_by(&quiet);

        engine.install(told).await.expect("the sheets go in");

        let raw = fs::read(root.join("Data/MapData/Dungeon.mps")).expect("the map");
        let read = map::read(&raw).expect("it still reads as a map");
        assert_eq!(read.pieces[0].said[0].text, "The door is locked tight.");

        let raw = fs::read(root.join("Data/BasicData/DataBase.dat")).expect("the database");
        let plan = fs::read(root.join("Data/BasicData/DataBase.project")).expect("the plan");
        let plan = database::plan(&plan).expect("the plan reads");
        let read = database::read(&raw, &plan).expect("it still reads as a database");
        assert_eq!(read.pieces[0].said[0].text, "Green Tea");
    }

    #[tokio::test]
    async fn a_game_still_in_its_archives_is_read_out_of_them_and_sealed_back_into_them() {
        let at = sandbox();
        let root = at.path();
        let store = sandbox();
        let source = store.path().join("source");
        let staged = store.path().join("staged");

        let data = root.join(source::DATA);
        fs::create_dir_all(&data).unwrap();
        fs::write(root.join("Game.exe"), [0; 4]).unwrap();

        let (plan, base) = fixture::database(&[fixture::Type {
            name: "\u{30a2}\u{30a4}\u{30c6}\u{30e0}",
            fields: &["\u{540d}\u{524d}"],
            words: &[0],
            entries: &[&["\u{7dd1}\u{8336}"]],
            named_by: None,
        }]);
        let basics = archive::archived(
            &[
                (
                    "Game.dat",
                    fixture::game("\u{9060}\u{3044}\u{9053}", "", "MS Gothic").as_slice(),
                ),
                ("DataBase.dat", base.as_slice()),
                ("DataBase.project", plan.as_slice()),
            ],
            None,
        );
        let maps = archive::archived(
            &[(
                "Dungeon.mps",
                fixture::map(&[&[fixture::command(
                    101,
                    &[],
                    &["\u{6249}\u{306f}\u{9589}\u{307e}\u{3063}\u{3066}\u{3044}\u{308b}"],
                )]])
                .as_slice(),
            )],
            None,
        );

        let bundled = archive::archived(
            &[
                ("face001.png", b"\x89PNG\r\n\x1a\n".as_slice()),
                (
                    "talk001.txt",
                    "@mes \u{30a2}\u{30ad}\u{30e9}\r\n\u{3053}\u{308c}\u{306f}\u{5263}\u{3060}\r\n"
                        .as_bytes(),
                ),
                ("face002.png", b"\x89PNG\r\n\x1a\n".as_slice()),
            ],
            None,
        );

        fs::write(data.join("BasicData.wolf"), &basics).unwrap();
        fs::write(data.join("MapData.wolf"), maps).unwrap();
        fs::write(data.join("Locker.wolf"), bundled).unwrap();
        fs::write(data.join("mdb.wolf"), vec![7; 2002]).unwrap();

        let engine = detect(root).expect("a game inside its archives");
        let quiet = Quiet;

        engine
            .prepare(Prepare::over(root, &source, store.path()).heard_by(&quiet))
            .await
            .expect("the archives are opened and the text read out");

        let mut written: Vec<String> = walk::relative(&source)
            .await
            .into_iter()
            .map(|at| slashed(&at))
            .collect();
        written.sort();

        assert_eq!(
            written,
            [
                "BasicData/DataBase.dat.sheet",
                "BasicData/Game.dat.sheet",
                "Locker/talk001.txt.sheet",
                "MapData/Dungeon.mps.sheet",
            ],
            "each sheet sits where the file sat inside the archive it came out of, and a game \
             that bundles a handful of scripts in among its pictures still gives them up"
        );

        assert!(
            !store
                .path()
                .join(source::UNPACKED)
                .join(source::DATA)
                .join("Locker")
                .join("face001.png")
                .exists(),
            "and the pictures beside those scripts are left in the archive rather than copied \
             out, or a game of a few gigabytes is written twice to read a few kilobytes of it"
        );

        let landing = staged.join("MapData");
        fs::create_dir_all(&landing).unwrap();
        fs::write(
            landing.join("Dungeon.mps.sheet"),
            sheet::write([("e0/p0/c0/s0".to_string(), "The door is locked.".to_string())])
                .expect("a sheet"),
        )
        .unwrap();

        engine
            .install(Install::over(root, &staged, store.path()).heard_by(&quiet))
            .await
            .expect("the sheet is sealed back into the archive it came from");

        let mut held = BTreeMap::new();
        archive::poured(
            &data.join("MapData.wolf"),
            &[],
            Path::new(""),
            |_| true,
            |one| {
                held.insert(one.at, one.body);
                Ok(())
            },
        )
        .expect("the written archive opens");

        let raw = held
            .get(Path::new("Dungeon.mps"))
            .expect("the map is still in there");

        assert_eq!(
            map::read(raw).expect("it still reads as a map").pieces[0].said[0].text,
            "The door is locked.",
            "a packed game is never unpacked beside itself, so the only way a translation reaches \
             the player is by going back into the archive the game actually reads"
        );
        assert_eq!(
            fs::read(data.join("BasicData.wolf")).expect("the other archive"),
            basics,
            "and the archive nobody translated out of is left byte for byte as it shipped"
        );
    }

    #[tokio::test]
    async fn an_archive_this_reader_cannot_open_says_so_rather_than_failing_blankly() {
        let at = sandbox();
        let root = at.path();
        let store = sandbox();
        fs::create_dir_all(root.join("Data")).unwrap();
        fs::write(root.join("Data").join("BasicData.wolf"), [0; 16]).unwrap();

        let engine = detect(root).expect("a wolf game, archive and all");
        let quiet = Quiet;

        let why = engine
            .prepare(
                Prepare::over(root, &store.path().join("source"), store.path()).heard_by(&quiet),
            )
            .await
            .expect_err("nothing can be read out of an archive this reader cannot open");

        let unread = source::root(root, store.path())
            .join(source::DATA)
            .join(source::BASIC);

        assert_eq!(
            format!("{why:#}"),
            format!(
                "the archive holding the data of this game could not be opened, so there is \
                 nothing in {} to read",
                unread.display()
            ),
            "the reader has to be told that the archive is what stood in the way"
        );
    }

    #[test]
    fn a_line_the_sheet_calls_shown_stays_translatable_however_symbolic_it_looks() {
        let page = sheet::page(vec![
            sheet::Line {
                spot: "e0/p0/c0/s0".to_string(),
                said: "NewGame".to_string(),
                offer: Offer::Asked,
            },
            sheet::Line {
                spot: "e0/p0/c1/s0".to_string(),
                said: "@item".to_string(),
                offer: Offer::Listed,
            },
        ])
        .expect("a sheet");

        let parsed = WolfRpg.parse(Path::new("Dungeon.mps.sheet"), &page);
        assert_eq!(
            parsed
                .units()
                .iter()
                .map(|one| (one.text.as_str(), !one.offer.asked()))
                .collect::<Vec<_>>(),
            [("NewGame", false), ("@item", true)],
            "the sheet already says which lines the player reads, and second-guessing it here \
             would quietly drop a one-word choice from every translation"
        );
    }

    #[test]
    fn a_message_box_that_cannot_grow_keeps_the_rows_it_shipped_with() {
        let source = "\u{300c}\u{3053}\u{308c}\u{306f}\u{5263}\u{3060}\n\u{3068}\u{3066}\u{3082}\u{9245}\u{3044}";
        let said = "\"This is a sword, and a very heavy one at that,\" said the shopkeeper slowly.";
        let laid = text::shaped(source, said, &Default::default());
        let rows: Vec<&str> = laid.split('\n').collect();

        assert_eq!(rows.len(), 2, "the window holds the rows the author drew");
        assert_eq!(
            rows.join(" "),
            said,
            "the box is rewrapped across the rows the author drew and not one character of the \
             translation is dropped to make it fit: {rows:?}"
        );
        assert!(
            rows.iter().all(|row| !row.trim().is_empty()),
            "an empty row is a blank line the game would draw: {rows:?}"
        );
        assert_eq!(
            text::shaped(
                "one row only",
                "a single row of its own",
                &Default::default()
            ),
            "a single row of its own",
            "a choice or an item name is one row and reflowing it would invent a break"
        );
    }
}
