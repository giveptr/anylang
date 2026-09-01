mod archive;
mod compiled;
mod fonts;
mod names;
mod parameterized;
mod pictures;
mod prepare;
mod python;
mod script;
mod shipped;
mod switch;
mod table;
mod text;

use crate::engine::fonts::Fonts;
use crate::engine::pictures::{Handed, Shot};
use crate::engine::renpy::fonts::faces;
pub use crate::engine::renpy::switch::SWITCH_FILE;
use crate::engine::renpy::switch::{switch, switch_file};
use crate::engine::{Engine, Extra, Font, Install, Landing, Parsed, Prepare, Rules, Tweaks, Undo};
use crate::walk;
use anyhow::Result;
use futures::future::{BoxFuture, FutureExt};
use regex::Regex;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::borrow::Cow;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use tracing::Instrument;
use walkdir::WalkDir;

static RE_EXPORTED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"preferences\.language\s*=\s*"([^"]*)""#).expect("RE_EXPORTED is a valid pattern")
});

pub const WORKING: &str = env!("CARGO_PKG_NAME");

const GAME_DIR: &str = "game";
const TL_DIR: &str = "tl";
const ENGINE_DIR: &str = "renpy";
const LIB_DIR: &str = "lib";

const STEPS: [&str; 5] = [READING, ARCHIVES, SCRIPTS, TEXT, PICTURES];

pub const READING: &str = "Reading the folder";
pub const ARCHIVES: &str = "Opening archives";
pub const SCRIPTS: &str = "Recovering scripts";
pub const PICTURES: &str = "Listing the pictures";
pub const TEXT: &str = "Taking the text in";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Options {
    #[serde(default)]
    pub shipped: String,
}

fn has_ext(path: &Path, extension: &str) -> bool {
    path.extension().is_some_and(|found| found == extension)
}

fn scripts(at: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = walk::files_now(at)
        .into_iter()
        .filter(|path| has_ext(path, "rpy"))
        .collect();
    found.sort();

    found
}

fn scripted(game: &Path) -> bool {
    WalkDir::new(game)
        .follow_links(true)
        .into_iter()
        .filter_map(Result::ok)
        .any(|entry| {
            ["rpy", "rpyc", "rpa"]
                .iter()
                .any(|extension| has_ext(entry.path(), extension))
        })
}

pub struct RenPy;

pub fn forget() {
    archive::forget();
}

pub fn detect(dir: &Path) -> Option<Box<dyn Engine>> {
    let looks_like_renpy = dir.join(ENGINE_DIR).is_dir() || scripted(&dir.join(GAME_DIR));
    looks_like_renpy.then(|| Box::new(RenPy) as Box<dyn Engine>)
}

impl Engine for RenPy {
    fn label(&self) -> &str {
        "Ren'Py"
    }

    fn wants(&self, path: &Path) -> bool {
        has_ext(path, "rpy")
    }

    fn parse(&self, _at: &Path, text: &str) -> Box<dyn Parsed> {
        Box::new(script::parse(text))
    }

    fn validate(&self, source: &str, translation: &str) -> Result<(), String> {
        text::validate(source, translation)
    }

    fn piles(&self) -> bool {
        false
    }

    fn rules(&self) -> Rules {
        Rules {
            markup: text::MARKUP_RULES,
            shape: None,
            retry: text::RETRY_RULES,
        }
    }

    fn output(&self, at: Landing<'_>) -> PathBuf {
        at.game_dir
            .join(GAME_DIR)
            .join(TL_DIR)
            .join(slug(at.language))
    }

    fn prepare<'a>(&'a self, at: Prepare<'a>) -> BoxFuture<'a, Result<()>> {
        prepare::run(at).boxed()
    }

    fn tweaks(&self) -> Tweaks {
        Tweaks::RenPy(Options::default())
    }

    fn retarget<'t>(&self, text: &'t str, language: &str) -> Cow<'t, str> {
        script::retarget(text, &slug(language))
    }

    fn install<'a>(&'a self, at: Install<'a>) -> BoxFuture<'a, Result<()>> {
        async move {
            if at.reverting {
                walk::cleared(at.staged).await?;
            } else {
                compiled::dropped_under(at.staged).await?;
            }

            compiled::dropped(&switch_file(at.game_dir)).await?;

            fonts::tidied(&at).await;

            let held = pictures::land(&at).await?;

            for why in held.dropped {
                at.progress.warn(at.doing, &why);
            }
            if held.written > 0 {
                at.progress
                    .info(at.doing, &format!("{} picture(s) written in", held.written));
            }
            if held.put_back > 0 {
                at.progress.info(
                    at.doing,
                    &format!("{} picture(s) put back the way they shipped", held.put_back),
                );
            }

            Ok(())
        }
        .instrument(tracing::info_span!("renpy.install"))
        .boxed()
    }

    fn undo(&self) -> Undo {
        Undo::Remove
    }

    fn pictures(&self, store: &Path) -> Vec<Shot> {
        pictures::LEDGER.remembered(store)
    }

    fn picture(&self, game_dir: &Path, store: &Path, key: &str) -> Result<Handed> {
        Ok(Handed::Shipped(pictures::picture(game_dir, store, key)?))
    }

    fn fonts(&self, game_dir: &Path, store: &Path) -> Vec<Font> {
        faces(game_dir, store)
    }

    fn sources(&self, game_dir: &Path) -> Vec<String> {
        let mut skip = vec![WORKING.to_string()];
        skip.extend(exported(game_dir));

        shipped::sources(game_dir, &skip)
    }

    fn source_key(&self, tweaks: &Tweaks) -> String {
        chosen(tweaks).to_string()
    }

    fn extras(&self, at: Landing<'_>, tweaks: &Tweaks, fonts: &Fonts) -> Vec<Extra> {
        let language = at.language;
        let placed = fonts::landings(fonts);

        let Some(body) = switch(language, tweaks, fonts, &placed) else {
            return Vec::new();
        };

        let mut wanted = fonts::carried(language, &placed);

        wanted.push(Extra::Write {
            at: PathBuf::from(GAME_DIR).join(SWITCH_FILE),
            body,
        });

        wanted
    }
}

fn slug(language: &str) -> String {
    let mut out = String::new();
    for character in language.to_lowercase().chars() {
        if character.is_ascii_alphanumeric() || character == '_' {
            out.push(character);
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }

    let cleaned = out.trim_matches('_');
    if cleaned.is_empty() {
        "translated".to_string()
    } else if cleaned.starts_with(|first: char| first.is_ascii_digit()) {
        format!("_{cleaned}")
    } else {
        cleaned.to_string()
    }
}

fn exported(game_dir: &Path) -> Option<String> {
    let body = fs::read_to_string(switch_file(game_dir)).ok()?;

    Some(RE_EXPORTED.captures(&body)?[1].to_string())
}

fn chosen(tweaks: &Tweaks) -> &str {
    match tweaks {
        Tweaks::RenPy(options) => options.shipped.trim(),
        Tweaks::None => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_language_that_would_break_python_is_slugged() {
        assert_eq!(slug("Japanese"), "japanese");
        assert_eq!(slug("ja\"pa nese"), "ja_pa_nese");
        assert_eq!(slug("$$$"), "translated");
    }

    #[test]
    fn a_language_starting_with_a_digit_is_still_a_name_the_lexer_takes() {
        assert_eq!(slug("2nd Language"), "_2nd_language");
    }

    #[test]
    fn another_engines_tweaks_are_refused_rather_than_guessed_at() {
        assert!(
            RenPy
                .extras(
                    Landing::over(Path::new("/game"), Path::new("/store"), "Japanese"),
                    &Tweaks::None,
                    &Fonts::default(),
                )
                .is_empty()
        );
    }

    #[tokio::test]
    async fn a_translation_taken_out_is_not_left_behind_in_what_ren_py_compiled() {
        let sandbox = tempfile::tempdir().expect("a temp folder");
        let game_dir = sandbox.path();
        let inner = game_dir.join(GAME_DIR);
        let staged = inner.join(TL_DIR).join("japanese");
        fs::create_dir_all(&staged).expect("a folder");

        for at in [
            staged.join("script.rpy"),
            staged.join("script.rpyc"),
            staged.join("options.rpyc"),
            inner.join(SWITCH_FILE),
            inner.join("anylang.rpyc"),
        ] {
            fs::write(&at, "").expect("a file");
        }

        let fonts = Fonts::default();
        let told = |reverting| {
            Install::over(game_dir, &staged, game_dir)
                .sending(&fonts)
                .putting_back(reverting)
        };

        RenPy.install(told(false)).await.expect("an install");

        assert!(
            !staged.join("options.rpyc").exists(),
            "the reader deleted the last translation in that file, so the file went, and Ren'Py \
             reads what it compiled from it unless that goes too"
        );
        assert!(
            staged.join("script.rpyc").is_file(),
            "this one still has its words beside it, and recompiling it is the game's own business"
        );
        assert!(inner.join("anylang.rpyc").is_file());

        fs::remove_file(inner.join(SWITCH_FILE)).expect("the switch going out");
        RenPy.install(told(true)).await.expect("an install");

        assert!(
            !staged.exists(),
            "undoing everything leaves no folder behind"
        );
        assert!(
            !inner.join("anylang.rpyc").exists(),
            "the language stays switched over for as long as this one is readable"
        );
    }

    #[test]
    fn a_game_folder_with_nothing_ren_py_wrote_in_it_is_not_a_ren_py_game() {
        let at = tempfile::tempdir().expect("a temp folder");
        let game = at.path().join("game");
        fs::create_dir_all(&game).unwrap();
        assert!(detect(at.path()).is_none());

        fs::write(game.join("script.rpyc"), "").unwrap();
        assert!(detect(at.path()).is_some());
    }

    #[test]
    fn a_script_is_wanted_by_its_ending_and_a_compiled_one_never_is() {
        assert!(RenPy.wants(Path::new("game/tl/french/script.rpy")));
        assert!(!RenPy.wants(Path::new("game/script.rpyc")));
        assert!(!RenPy.wants(Path::new("game/tl/french/notes.txt")));
    }

    #[test]
    fn the_folder_the_last_export_named_is_dropped_from_what_can_be_read() {
        let sandbox = tempfile::tempdir().expect("a temp folder");
        let game = sandbox.path();
        let inner = game.join("game");

        for at in ["tl/english/a.rpy", "tl/french/a.rpy"] {
            let at = inner.join(at);
            fs::create_dir_all(at.parent().expect("a folder")).expect("a folder");
            fs::write(&at, "translate x a_1:\n    e \"Something.\"\n").expect("a script");
        }

        assert_eq!(RenPy.sources(game), ["english", "french"]);

        fs::write(
            inner.join(SWITCH_FILE),
            switch::switched("French", &RenPy.tweaks(), &Fonts::default()).expect("a switch file"),
        )
        .expect("an export");

        assert_eq!(
            RenPy.sources(game),
            ["english"],
            "the export names the folder it wrote, so reading that one back would translate this \
             tool's own answers"
        );
    }

    #[test]
    fn a_blank_shipped_folder_is_no_folder_at_all() {
        let tweaked = |shipped: &str| {
            Tweaks::RenPy(Options {
                shipped: shipped.to_string(),
            })
        };

        assert_eq!(chosen(&tweaked("")), "");
        assert_eq!(chosen(&tweaked("   ")), "");
        assert_eq!(chosen(&tweaked(" english ")), "english");
        assert_eq!(chosen(&Tweaks::None), "");
    }
}
