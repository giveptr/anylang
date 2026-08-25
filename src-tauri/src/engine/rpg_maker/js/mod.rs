mod crypt;
mod data;
#[cfg(test)]
mod fixture;
mod fonts;
mod pictures;
mod plugins;
mod prepare;
mod script;
mod settings;
mod vocabulary;

use crate::engine::pictures::{Handed, Shot};
use crate::engine::rpg_maker::js::vocabulary::Vocabulary;
use crate::engine::rpg_maker::pictures::LEDGER;
use crate::engine::rpg_maker::text;
use crate::engine::{Engine, Font, Install, Landing, Parsed, Prepare, Rules, Undo};
use anyhow::Result;
use futures::future::{BoxFuture, FutureExt};
use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use tracing::Instrument;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flavour {
    Mv,
    Mz,
    Unknown,
}

impl Flavour {
    fn label(self) -> &'static str {
        match self {
            Flavour::Mv => "RPG Maker MV",
            Flavour::Mz => "RPG Maker MZ",
            Flavour::Unknown => "RPG Maker",
        }
    }
}

struct RpgMaker {
    flavour: Flavour,
    root: PathBuf,
    words: OnceLock<Arc<Vocabulary>>,
}

pub fn forget() {
    vocabulary::forget();
}

pub fn detect(dir: &Path) -> Option<Box<dyn Engine>> {
    let root = content_root(dir);
    if !root.join(DATA).join(SYSTEM).is_file() {
        return None;
    }

    Some(Box::new(RpgMaker {
        flavour: flavour_at(&root),
        root,
        words: OnceLock::new(),
    }))
}

impl RpgMaker {
    fn words(&self) -> Arc<Vocabulary> {
        self.words
            .get_or_init(|| Vocabulary::shared(&self.root))
            .clone()
    }
}

pub fn wanted(at: &Path) -> bool {
    data::translatable(at) || plugins::is_list(at)
}

pub const DATA: &str = "data";
pub const SCRIPTS: &str = "js";
pub const SYSTEM: &str = "System.json";
pub const CORE: [&str; 2] = ["rmmz_core.js", "rpg_core.js"];
const PACKED: &str = "www";

fn content_root(game_dir: &Path) -> PathBuf {
    let packed = game_dir.join(PACKED);

    match packed.join(DATA).is_dir() {
        true => packed,
        false => game_dir.to_path_buf(),
    }
}

fn flavour_at(root: &Path) -> Flavour {
    let js = root.join(SCRIPTS);

    for (name, flavour) in CORE.iter().zip([Flavour::Mz, Flavour::Mv]) {
        if js.join(name).is_file() {
            return flavour;
        }
    }

    Flavour::Unknown
}

impl Engine for RpgMaker {
    fn label(&self) -> &str {
        self.flavour.label()
    }

    fn wants(&self, path: &Path) -> bool {
        wanted(path)
    }

    fn parse(&self, at: &Path, body: &str) -> Box<dyn Parsed> {
        match plugins::is_list(at) {
            true => Box::new(plugins::parse(body, &self.words())),
            false => Box::new(data::parse(body, &self.words())),
        }
    }

    fn validate(&self, source: &str, translation: &str) -> Result<(), String> {
        text::validate(source, translation)
    }

    fn bare<'t>(&self, text: &'t str) -> Cow<'t, str> {
        text::unmarked(text)
    }

    fn pictures(&self, store: &Path) -> Vec<Shot> {
        LEDGER.remembered(store)
    }

    fn picture(&self, game_dir: &Path, store: &Path, key: &str) -> Result<Handed> {
        Ok(Handed::Shipped(pictures::picture(game_dir, store, key)?))
    }

    fn rules(&self) -> Rules {
        Rules {
            markup: text::MARKUP_RULES,
            shape: Some(text::SHAPE_RULES),
            retry: text::RETRY_RULES,
        }
    }

    fn output(&self, at: Landing<'_>) -> PathBuf {
        content_root(at.game_dir)
    }

    fn install<'a>(&'a self, at: Install<'a>) -> BoxFuture<'a, Result<()>> {
        async move {
            fonts::run(&self.root, &at).await?;
            settings::run(&self.root, &at).await?;

            pictures::run(&self.root, &at).await
        }
        .instrument(tracing::info_span!("js.install"))
        .boxed()
    }

    fn fonts(&self, game_dir: &Path, _store: &Path) -> Vec<Font> {
        fonts::faces(game_dir, &self.root)
    }

    fn undo(&self) -> Undo {
        Undo::Restore
    }

    fn prepare<'a>(&'a self, at: Prepare<'a>) -> BoxFuture<'a, Result<()>> {
        prepare::run(self.flavour.label(), at).boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::rpg_maker::js::fixture::{put, sandbox};
    use crate::progress::Quiet;
    use std::collections::BTreeMap;

    fn touch(root: &Path, at: &str) {
        put(root, at, "{}");
    }

    fn message(rows: &[&str]) -> String {
        let list: Vec<serde_json::Value> = rows
            .iter()
            .map(|row| serde_json::json!({"code": 401, "indent": 0, "parameters": [row]}))
            .collect();

        serde_json::json!([{"id": 1, "list": list}]).to_string()
    }

    #[test]
    fn a_row_of_plugin_tags_is_kept_out_of_the_asking_and_put_back_where_it_was() {
        let at = sandbox();
        touch(at.path(), "data/System.json");
        let engine = detect(at.path()).expect("an rpg maker game");

        let raw = message(&[
            "<center>",
            "You picked the newest update.",
            "It starts here.",
        ]);
        let parsed = engine.parse(Path::new("data/CommonEvents.json"), &raw);

        let said: Vec<&str> = parsed.units().iter().map(|one| one.text.as_str()).collect();
        assert_eq!(
            said,
            ["You picked the newest update.\nIt starts here."],
            "the tag row draws the box, so it is never part of what is asked for"
        );

        let id = parsed.units()[0].id;
        let (out, _) = parsed
            .render(&BTreeMap::from([(
                id,
                "最新の更新を選んだ。\nここから始まる。".to_string(),
            )]))
            .expect("the file renders");

        let back = serde_json::from_str::<serde_json::Value>(&out).expect("json");
        let rows: Vec<&str> = back[0]["list"]
            .as_array()
            .expect("a command list")
            .iter()
            .map(|one| one["parameters"][0].as_str().expect("a row"))
            .collect();

        assert_eq!(
            rows,
            ["<center>", "最新の更新を選んだ。", "ここから始まる。"],
            "the tag stayed on its own row and the words landed on the rows they were taken from"
        );
    }

    #[test]
    fn a_box_of_nothing_but_tags_is_never_asked_about_at_all() {
        let at = sandbox();
        touch(at.path(), "data/System.json");
        let engine = detect(at.path()).expect("an rpg maker game");

        let raw = message(&["<center>", "<b></b>"]);
        let parsed = engine.parse(Path::new("data/CommonEvents.json"), &raw);

        assert!(
            parsed.units().is_empty(),
            "there is no word in it, so there is nothing to pay a model for"
        );
    }

    #[test]
    fn a_plugin_argument_that_names_something_is_told_apart_from_a_line_of_talk() {
        for reference in [
            "event:4",
            "ripple_walk",
            "00introA_scene00",
            "Change_the_Beats",
            "x=250;y=200;placeholder=Hero;",
            "Marketif(s[133])",
        ] {
            assert!(
                text::symbolic_line(reference),
                "{reference} names something"
            );
        }

        let marked_talk = [
            "ON if(s[1201])",
            "\\c[18]Kill...",
            "\u{30aa}\u{30f3}/\u{30aa}\u{30d5}",
        ];
        for talk in text::TALK.iter().chain(&marked_talk) {
            assert!(!text::symbolic_line(talk), "{talk} is text a player reads");
        }
    }

    #[test]
    fn a_folder_without_the_database_is_not_an_rpg_maker_game() {
        let at = sandbox();
        touch(at.path(), "js/rmmz_core.js");

        assert!(detect(at.path()).is_none());
    }

    #[test]
    fn the_two_engine_versions_are_told_apart_for_the_log() {
        let mz = sandbox();
        touch(mz.path(), "data/System.json");
        touch(mz.path(), "js/rmmz_core.js");
        assert_eq!(detect(mz.path()).unwrap().label(), "RPG Maker MZ");

        let mv = sandbox();
        touch(mv.path(), "data/System.json");
        touch(mv.path(), "js/rpg_core.js");
        assert_eq!(detect(mv.path()).unwrap().label(), "RPG Maker MV");

        let bare = sandbox();
        touch(bare.path(), "data/System.json");
        assert_eq!(detect(bare.path()).unwrap().label(), "RPG Maker");
    }

    #[test]
    fn a_game_packed_under_www_is_found_and_written_back_to_the_same_place() {
        let at = sandbox();
        touch(at.path(), "www/data/System.json");
        touch(at.path(), "www/js/rpg_core.js");

        let engine = detect(at.path()).expect("www layouts are still RPG Maker");

        assert_eq!(engine.label(), "RPG Maker MV");
        assert_eq!(
            engine.output(landing(at.path())),
            at.path().join("www"),
            "what is written back has to land where the packed game reads it from"
        );
    }

    fn landing(game_dir: &Path) -> Landing<'_> {
        Landing {
            game_dir,
            store: Path::new("/nowhere"),
            language: "japanese",
        }
    }

    #[tokio::test]
    async fn the_source_tree_mirrors_the_game_so_every_file_finds_its_folder_again() {
        let at = sandbox();
        let root = at.path();
        let source = sandbox();

        touch(root, "data/System.json");
        touch(root, "data/Map001.json");
        touch(root, "data/Animations.json");
        touch(root, "js/plugins.js");
        touch(root, "js/plugins/SRPG_core_MZ.js");
        touch(root, "img/pictures/title.png");

        detect(root)
            .expect("an rpg maker game")
            .prepare(Prepare {
                game_dir: root,
                source: source.path(),
                store: source.path(),
                tweaks: &Default::default(),
                progress: &Quiet,
            })
            .await
            .expect("the game is read in");

        let taken = |at: &str| source.path().join(at).is_file();

        assert!(taken("data/System.json") && taken("data/Map001.json"));
        assert!(
            taken("js/plugins.js"),
            "the parameter list holds menu words a plugin prints"
        );
        assert!(!taken("data/Animations.json"), "it holds no text");
        assert!(
            !taken("js/plugins/SRPG_core_MZ.js"),
            "a plugin's source is read for what it declares, never rewritten"
        );
        assert!(!taken("img/pictures/title.png"));
        assert!(
            !source.path().join("System.json").exists(),
            "a flat copy would land data files on top of the game's own folders"
        );
    }

    #[test]
    fn the_game_files_are_the_ones_that_get_overwritten() {
        let at = sandbox();
        touch(at.path(), "data/System.json");
        let engine = detect(at.path()).unwrap();

        assert_eq!(
            engine.undo(),
            Undo::Restore,
            "there is no tl/<language> folder to write into, so the originals need a backup"
        );
        assert_eq!(engine.output(landing(at.path())), at.path().to_path_buf());
    }
}
