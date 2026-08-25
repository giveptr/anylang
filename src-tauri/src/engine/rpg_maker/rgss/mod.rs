mod archive;
mod data;
#[cfg(test)]
mod fixture;
mod fonts;
mod install;
mod marshal;
mod packed;
mod pictures;
mod prepare;
mod reading;
mod scripts;
mod settings;
mod source;
mod tes;

use crate::engine::fonts::Fonts;
use crate::engine::pictures::{Handed, Shot};
use crate::engine::rpg_maker::pictures::LEDGER;
use crate::engine::rpg_maker::text;
use crate::engine::{
    Engine, Extra, Font, Install, Landing, Parsed, Prepare, Rules, Tweaks, Undo, sheet,
};
use anyhow::Result;
use futures::future::BoxFuture;
use std::borrow::Cow;
use std::path::{Path, PathBuf};

struct VxAce;

pub fn detect(dir: &Path) -> Option<Box<dyn Engine>> {
    source::holds_a_game(dir).then(|| Box::new(VxAce) as Box<dyn Engine>)
}

pub fn refused(dir: &Path) -> Option<String> {
    let (_, named) = source::older_at(dir)?;

    Some(format!(
        "{named} cannot draw text outside ASCII, only VX Ace and newer keep UTF-8."
    ))
}

impl Engine for VxAce {
    fn label(&self) -> &str {
        "RPG Maker VX Ace"
    }

    fn wants(&self, path: &Path) -> bool {
        sheet::wants(path)
    }

    fn shown<'n>(&self, name: &'n str) -> Cow<'n, str> {
        match name.rsplit_once('.') {
            Some((bare, kind)) if kind.eq_ignore_ascii_case(sheet::SUFFIX) => {
                Cow::Owned(format!("{bare}.{}", data::SUFFIX))
            }
            _ => Cow::Borrowed(name),
        }
    }

    fn parse(&self, _at: &Path, body: &str) -> Box<dyn Parsed> {
        Box::new(sheet::read(body, |_| false))
    }

    fn validate(&self, source: &str, translation: &str) -> Result<(), String> {
        text::validate(source, translation)
    }

    fn bare<'t>(&self, text: &'t str) -> Cow<'t, str> {
        text::unmarked(text)
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
        fonts::faces(game_dir, store)
    }

    fn pictures(&self, store: &Path) -> Vec<Shot> {
        LEDGER.remembered(store)
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
    use crate::engine::rpg_maker::rgss::fixture::{self, sandbox};
    use crate::engine::{Landing, Offer, STAGED, Swap, fonts as face};
    use crate::progress::Quiet;
    use std::fs;

    fn exporting<'a>(
        root: &'a Path,
        staged: &'a Path,
        store: &'a Path,
        fonts: &'a Fonts,
        progress: &'a Quiet,
    ) -> Install<'a> {
        Install::over(root, staged, store)
            .sending(fonts)
            .heard_by(progress)
    }

    #[test]
    fn a_game_is_taken_for_vx_ace_by_its_data_whatever_runs_it() {
        let at = sandbox();
        let root = at.path();

        assert!(detect(root).is_none(), "an empty folder is no game");

        fs::write(root.join("Game.rgss3a"), [0u8; 4]).unwrap();
        let engine = detect(root).expect("a packed game, as Windows ships it");

        assert_eq!(engine.label(), "RPG Maker VX Ace");
        assert!(engine.wants(Path::new("Map001.sheet")));
        assert!(!engine.wants(Path::new("Map001.rvdata2")));

        let ported = sandbox();
        fs::create_dir_all(ported.path().join("Data")).unwrap();
        fs::write(
            ported.path().join("Data").join("Map001.rvdata2"),
            [4, 8, b'0'],
        )
        .unwrap();

        assert!(
            detect(ported.path()).is_some(),
            "a game unpacked for mkxp on linux or mac has no archive and no RGSS dll"
        );
    }

    #[test]
    fn an_xp_or_vx_game_is_no_game_to_this_app_and_says_so_in_the_editor_it_was_made_with() {
        for (named, packed, loose) in [
            ("RPG Maker XP", "Game.rgssad", "Map001.rxdata"),
            ("RPG Maker VX", "Game.rgss2a", "Map001.rvdata"),
        ] {
            let at = sandbox();
            let root = at.path();
            fs::write(root.join(packed), [0u8; 8]).unwrap();

            assert!(
                detect(root).is_none(),
                "taking it for a game opens a project and lays a store down on disk for something \
                 that can never be translated"
            );
            let why = refused(root).expect("a reason");
            assert!(
                why.contains(named) && why.contains("UTF-8"),
                "\"not an RPG Maker game\" would be a flat lie to someone holding one, so the \
                 folder is turned away by name: {why}"
            );

            let ported = sandbox();
            fs::create_dir_all(ported.path().join("Data")).unwrap();
            fs::write(ported.path().join("Data").join(loose), [4, 8, b'0']).unwrap();
            assert!(
                refused(ported.path()).is_some_and(|why| why.contains(named)),
                "a game unpacked by hand has no archive left to name it, so the data files have to"
            );
        }
    }

    #[test]
    fn a_vx_ace_game_is_never_read_as_an_older_one_over_a_stray_data_file() {
        let at = sandbox();
        let root = at.path();
        fs::create_dir_all(root.join("Data")).unwrap();
        fs::write(root.join("Game.rgss3a"), [0u8; 4]).unwrap();
        fs::write(root.join("Data").join("Map001.rxdata"), [4, 8, b'0']).unwrap();

        assert_eq!(
            detect(root).expect("a game").label(),
            "RPG Maker VX Ace",
            "a leftover file from an older editor cannot be what decides, or a game that reads \
             fine would be turned away"
        );
    }

    #[test]
    fn an_mv_game_is_never_mistaken_for_this_one() {
        let at = sandbox();
        let root = at.path();
        fs::create_dir_all(root.join("js")).unwrap();
        fs::write(root.join("Game.ini"), "[Game]\nTitle=Demo\n").unwrap();
        fs::write(root.join("js/rpg_core.js"), "").unwrap();

        assert!(
            detect(root).is_none(),
            "MV writes no RGSS runtime into Game.ini"
        );
    }

    #[test]
    fn the_sheets_are_staged_under_the_store_and_undoing_means_removing_them() {
        let engine = VxAce;
        let at = sandbox();

        assert_eq!(
            engine.output(Landing {
                game_dir: at.path(),
                store: Path::new("/store/demo"),
                language: "Japanese",
            }),
            Path::new("/store/demo").join(STAGED).join("japanese"),
            "the game's own archive is patched by install, so nothing lands beside it"
        );
        assert_eq!(engine.undo(), Undo::Remove);
    }

    #[tokio::test]
    async fn a_picked_font_is_carried_in_and_told_to_the_script_that_boots_the_game() {
        let at = sandbox();
        let root = at.path();
        let store = sandbox();
        let staged = store.path().join("staged");
        let held = sandbox();
        fs::create_dir_all(&staged).unwrap();
        fs::create_dir_all(root.join("Fonts")).unwrap();

        fs::write(
            root.join("Game.rgss3a"),
            archive::packed(&[(
                "Data\\Scripts.rvdata2",
                &fixture::scripts(&[(
                    "Main",
                    "Font.default_name = [\"Niagara Solid\"]\r\nrgss_main { }\r\n",
                )]),
            )]),
        )
        .unwrap();

        let called = face::fake::called;
        fs::write(root.join("Fonts/NIAGSOL.TTF"), called("Niagara Solid")).unwrap();
        fs::write(root.join("Fonts/Alphmamack.ttf"), called("AlphaMack AOE")).unwrap();

        let picked = held.path().join("Sarabun-Medium.ttf");
        fs::write(&picked, called("Sarabun Medium")).unwrap();
        fs::write(held.path().join("Emulogic.ttf"), called("Emulogic")).unwrap();

        let engine = detect(root).expect("a vx ace game");
        let quiet = Quiet;
        let swap = |from: &str, to: &str| Swap {
            from: from.to_string(),
            to: held.path().join(to).to_string_lossy().to_string(),
        };
        let fonts = Fonts {
            swaps: vec![
                swap("Alphmamack.ttf", "Emulogic.ttf"),
                swap("NIAGSOL.TTF", "Sarabun-Medium.ttf"),
            ],
        };

        let told = |reverting| {
            Install::over(root, &staged, store.path())
                .sending(&fonts)
                .putting_back(reverting)
                .heard_by(&quiet)
        };

        for extra in engine.extras("Japanese", &Tweaks::None, &fonts) {
            let Extra::Copy { from, at } = extra else {
                panic!("a font is carried in as a file of its own");
            };
            fs::copy(from, root.join(at)).unwrap();
        }

        let landed = format!("{}-Sarabun-Medium.ttf", face::CARRIED);
        assert!(
            root.join("Fonts").join(&landed).is_file(),
            "RGSS reads every font in that folder, so the file has to be there before it draws"
        );
        assert_eq!(
            engine
                .fonts(root, store.path())
                .into_iter()
                .map(|one| one.name)
                .collect::<Vec<String>>(),
            vec!["Alphmamack.ttf".to_string(), "NIAGSOL.TTF".to_string()],
            "the font carried in is not one of the game's own to swap"
        );

        engine.install(told(false)).await.expect("the font goes in");

        let named = |raw: &[u8]| {
            let entries = archive::entries(raw).expect("it still reads as an archive");
            let listed = entries
                .iter()
                .find(|one| one.name == "Data/Scripts.rvdata2")
                .expect("the script list is still there");

            scripts::sources(&archive::body(raw, listed))
                .expect("the scripts")
                .into_iter()
                .map(|(_, source)| source)
                .collect::<Vec<String>>()
        };

        let raw = fs::read(root.join("Game.rgss3a")).expect("the patched archive");
        let [boot] = &named(&raw)[..] else {
            panic!("the one script is still the one script")
        };
        assert!(
            boot.contains(
                "PATCH_FONTS = {\"AlphaMack AOE\" => \"Emulogic\", \"Niagara Solid\" => \
                 \"Sarabun Medium\"}"
            ),
            "every face the reader picked for is written down under the family it replaces: \
             {boot:?}"
        );
        assert!(
            boot.contains("PATCH_FONT_ALL = nil"),
            "the faces went to different files, so there is no one font to answer a name this \
             game ships no file for: {boot:?}"
        );
        assert!(
            boot.starts_with("Font.default_name = [\"Niagara Solid\"]\r\n")
                && boot.ends_with("rgss_main { }\r\n"),
            "the hook goes in after the author's own font and before the game boots: {boot:?}"
        );

        engine.install(told(true)).await.expect("the game back");

        let raw = fs::read(root.join("Game.rgss3a")).expect("the archive");
        assert_eq!(
            named(&raw),
            vec!["Font.default_name = [\"Niagara Solid\"]\r\nrgss_main { }\r\n".to_string()],
            "asking for the game back leaves the script as its author wrote it"
        );
    }

    #[tokio::test]
    async fn a_game_nobody_picked_a_font_for_is_left_as_its_author_wrote_it() {
        let at = sandbox();
        let root = at.path();
        let store = sandbox();
        let staged = store.path().join("staged");
        fs::create_dir_all(&staged).unwrap();
        fs::create_dir_all(root.join("Fonts")).unwrap();

        let scripts = fixture::scripts(&[("Main", "rgss_main { }\r\n")]);
        fs::write(
            root.join("Game.rgss3a"),
            archive::packed(&[("Data\\Scripts.rvdata2", &scripts)]),
        )
        .unwrap();
        fs::write(
            root.join("Fonts/NIAGSOL.TTF"),
            face::fake::called("Niagara Solid"),
        )
        .unwrap();

        let before = fs::read(root.join("Game.rgss3a")).unwrap();
        let engine = detect(root).expect("a vx ace game");
        let quiet = Quiet;
        let nothing = Fonts::default();

        engine
            .install(exporting(root, &staged, store.path(), &nothing, &quiet))
            .await
            .expect("an export with nothing staged and no font picked");

        assert_eq!(
            fs::read(root.join("Game.rgss3a")).unwrap(),
            before,
            "a reader who never asked for a font is not handed a patched script"
        );
    }

    #[tokio::test]
    async fn a_game_goes_out_as_sheets_and_a_translation_lands_back_in_the_archive() {
        let at = sandbox();
        let root = at.path();
        let store = sandbox();
        let source = store.path().join("source");
        let staged = store.path().join("staged");
        fs::create_dir_all(&staged).unwrap();

        let sheets = fixture::map(&[
            fixture::command(101, &[fixture::said("")]),
            fixture::command(401, &[fixture::said("The door is")]),
            fixture::command(401, &[fixture::said("locked tight.")]),
        ]);
        fs::write(
            root.join("Game.ini"),
            "[Game]\nLibrary=System\\RGSS301.dll\n",
        )
        .unwrap();
        fs::write(
            root.join("Game.rgss3a"),
            archive::packed(&[
                ("Data\\Map001.rvdata2", &sheets),
                (
                    "Data\\Scripts.rvdata2",
                    &fixture::scripts(&[("Vocab", "module Vocab\n  ShopBuy = \"Buy\"\nend\n")]),
                ),
                ("Graphics\\Faces\\Actor1.png", &[137, 80, 78, 71]),
            ]),
        )
        .unwrap();

        let engine = detect(root).expect("a vx ace game");
        let quiet = Quiet;
        let nothing = Fonts::default();

        engine
            .prepare(Prepare::over(root, &source, store.path()).heard_by(&quiet))
            .await
            .expect("the archive is read out into sheets");

        let terms = sheet::lines(
            &fs::read_to_string(source.join("Scripts.sheet")).expect("a script sheet"),
        )
        .expect("its terms");
        assert_eq!(terms["0/ShopBuy"], "Buy");
        fs::write(
            staged.join("Scripts.sheet"),
            sheet::write([("0/ShopBuy".to_string(), "買う".to_string())]).expect("a sheet"),
        )
        .unwrap();

        let page = fs::read_to_string(source.join("Map001.sheet")).expect("a sheet");
        let taken = sheet::lines(&page).expect("its lines");
        assert_eq!(
            taken.values().next().map(String::as_str),
            Some("The door is\nlocked tight."),
            "the message run reads as one line to translate"
        );

        let door = "\u{6249}\u{306f}\n\u{9589}\u{3055}\u{308c}\u{3066}\u{3044}\u{308b}";
        let said: Vec<(String, String)> = taken
            .keys()
            .map(|spot| (spot.clone(), door.to_string()))
            .collect();
        fs::write(
            staged.join("Map001.sheet"),
            sheet::write(said).expect("a rendered sheet"),
        )
        .unwrap();

        engine
            .install(exporting(root, &staged, store.path(), &nothing, &quiet))
            .await
            .expect("the sheets go into the archive");

        let raw = fs::read(root.join("Game.rgss3a")).expect("the patched archive");
        let entries = archive::entries(&raw).expect("it still reads as an archive");
        assert_eq!(entries.len(), 3, "no entry may be lost");

        let listed = entries
            .iter()
            .find(|one| one.name == "Data/Scripts.rvdata2")
            .expect("the script list is still there");
        assert_eq!(
            scripts::lines_of(&archive::body(&raw, listed), &Default::default())
                .expect("its terms")
                .into_iter()
                .map(|line| (line.spot, line.said, line.offer != Offer::Asked))
                .collect::<Vec<_>>(),
            vec![("0/ShopBuy".to_string(), "買う".to_string(), false)],
            "a term of the game's own vocabulary went in packed"
        );

        let map = entries
            .iter()
            .find(|one| one.name == "Data/Map001.rvdata2")
            .expect("the sheet is still listed");
        let after = marshal::read(&archive::body(&raw, map)).expect("the sheet still reads");
        let rows: Vec<&str> = after.texts.iter().map(|one| one.text.as_str()).collect();

        assert_eq!(
            rows,
            [
                "",
                "\u{6249}\u{306f}",
                "\u{9589}\u{3055}\u{308c}\u{3066}\u{3044}\u{308b}"
            ],
            "the translation is in the game, one row per command"
        );

        let art = entries
            .iter()
            .find(|one| one.name.ends_with(".png"))
            .expect("the graphic");
        assert_eq!(
            archive::body(&raw, art),
            [137, 80, 78, 71],
            "a file nobody translated may not change"
        );

        engine
            .install(exporting(root, &staged, store.path(), &nothing, &quiet))
            .await
            .expect("exporting twice is allowed");

        assert_eq!(
            fs::read(root.join("Game.rgss3a")).expect("the archive"),
            raw,
            "a second export writes the same archive, it does not pile onto the first"
        );

        fs::remove_file(staged.join("Map001.sheet")).unwrap();
        fs::remove_file(staged.join("Scripts.sheet")).unwrap();

        engine
            .install(exporting(root, &staged, store.path(), &nothing, &quiet))
            .await
            .expect("taking the translation back out");

        let after = fs::read(root.join("Game.rgss3a")).expect("the archive");
        let entries = archive::entries(&after).expect("it still reads");
        let map = entries
            .iter()
            .find(|one| one.name == "Data/Map001.rvdata2")
            .expect("the sheet");
        let rows = marshal::read(&archive::body(&after, map)).expect("it reads");

        assert_eq!(
            rows.texts
                .iter()
                .map(|one| one.text.as_str())
                .collect::<Vec<_>>(),
            ["", "The door is", "locked tight."],
            "with nothing staged the game gets its own words back"
        );
    }

    #[tokio::test]
    async fn a_game_unpacked_for_a_port_goes_out_and_back_in_the_same_way() {
        let at = sandbox();
        let root = at.path();
        let store = sandbox();
        let source = store.path().join("source");
        let staged = store.path().join("staged");
        fs::create_dir_all(&staged).unwrap();
        fs::create_dir_all(root.join("Data")).unwrap();

        fs::write(
            root.join("Data").join("Map001.rvdata2"),
            fixture::map(&[
                fixture::command(101, &[fixture::said("")]),
                fixture::command(401, &[fixture::said("The door is")]),
            ]),
        )
        .unwrap();

        let engine = detect(root).expect("a game with loose data");
        let quiet = Quiet;
        let nothing = Fonts::default();

        engine
            .prepare(Prepare::over(root, &source, store.path()).heard_by(&quiet))
            .await
            .expect("the folder is read into sheets");

        let taken =
            sheet::lines(&fs::read_to_string(source.join("Map001.sheet")).expect("a sheet"))
                .expect("its lines");
        let spot = taken.keys().next().expect("a line").clone();

        fs::write(
            staged.join("Map001.sheet"),
            sheet::write([(spot, "\u{6249}\u{306f}".to_string())]).expect("a sheet"),
        )
        .unwrap();

        engine
            .install(exporting(root, &staged, store.path(), &nothing, &quiet))
            .await
            .expect("the sheet goes back into the game's own file");

        let after = fs::read(root.join("Data").join("Map001.rvdata2")).expect("the sheet");
        let rows = marshal::read(&after).expect("it still reads");
        assert_eq!(
            rows.texts
                .iter()
                .map(|one| one.text.as_str())
                .collect::<Vec<_>>(),
            ["", "\u{6249}\u{306f}"],
            "no archive in sight, and the translation still lands"
        );

        fs::remove_file(staged.join("Map001.sheet")).unwrap();
        engine
            .install(exporting(root, &staged, store.path(), &nothing, &quiet))
            .await
            .expect("taking it back out");

        let back = fs::read(root.join("Data").join("Map001.rvdata2")).expect("the sheet");
        let rows = marshal::read(&back).expect("it still reads");
        assert_eq!(
            rows.texts
                .iter()
                .map(|one| one.text.as_str())
                .collect::<Vec<_>>(),
            ["", "The door is"],
            "and it comes back out the same way"
        );
    }
}
