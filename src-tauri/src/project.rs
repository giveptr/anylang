use crate::engine::fonts::Fonts;
use crate::engine::pictures::Pictures;
use crate::engine::{Engine, Tweaks};
use crate::picks::Picks;
use crate::scope::Scope;
use crate::store;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::mem;
use std::path::Path;

const FILE: &str = "project.json";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Fidelity {
    Literal,
    #[default]
    Balanced,
    Free,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Era {
    #[default]
    Any,
    Ancient,
    Medieval,
    EarlyModern,
    Victorian,
    EarlyTwentieth,
    LateTwentieth,
    Modern,
    NearFuture,
    FarFuture,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Register {
    #[default]
    Any,
    Coarse,
    Casual,
    Formal,
    Elevated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Mood {
    Comic,
    Cute,
    Dark,
    Deadpan,
    Dramatic,
    Epic,
    Explicit,
    Melancholic,
    Playful,
    Sarcastic,
    Tense,
    Unsettling,
    Warm,
    Witty,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Style {
    #[serde(default)]
    pub fidelity: Fidelity,
    #[serde(default)]
    pub era: Era,
    #[serde(default)]
    pub register: Register,
    pub genres: Vec<String>,
    pub voices: Vec<String>,
    pub notes: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub source_language: String,
    pub language: String,
    pub style: Style,
    #[serde(default)]
    pub fonts: Fonts,
    #[serde(default)]
    pub tweaks: Tweaks,
    #[serde(default)]
    pub picks: Picks,
    #[serde(default)]
    pub pictures: Pictures,
}

impl Project {
    pub fn folder(&self) -> String {
        store::folder_name(&self.language)
    }

    pub fn under(mut self, engine: &dyn Engine) -> Self {
        let theirs = engine.tweaks();
        if mem::discriminant(&self.tweaks) != mem::discriminant(&theirs) {
            self.tweaks = theirs;
        }

        self
    }
}

pub async fn load(game_dir: &Path) -> Result<Option<Project>> {
    let file = store::root_for(game_dir)?.join(FILE);
    let Some(raw) = store::read_if_there(&file).await? else {
        return Ok(None);
    };

    let unreadable = || format!("{} is not readable. Fix or delete it", file.display());
    let mut held: serde_json::Value = serde_json::from_str(&raw).with_context(unreadable)?;
    let tweaks = held
        .as_object_mut()
        .and_then(|fields| fields.remove("tweaks"))
        .unwrap_or_default();

    let mut project: Project = serde_json::from_value(held).with_context(unreadable)?;
    project.tweaks = serde_json::from_value(tweaks).unwrap_or_default();

    Ok(Some(project))
}

pub async fn require(game_dir: &Path) -> Result<Project> {
    load(game_dir)
        .await?
        .context("this game has not been set up yet")
}

pub async fn save_keeping_picks(game_dir: &Path, project: &Project) -> Result<()> {
    let picks = load(game_dir)
        .await?
        .map(|stored| stored.picks)
        .unwrap_or_default();

    write(
        game_dir,
        &Project {
            picks,
            ..project.clone()
        },
    )
    .await
}

pub async fn pick(game_dir: &Path, asked: &[Scope], on: bool, by_default: bool) -> Result<()> {
    let mut project = require(game_dir).await?;
    for one in asked {
        project.picks.set(one, on, by_default);
    }

    write(game_dir, &project).await
}

async fn write(game_dir: &Path, project: &Project) -> Result<()> {
    let file = store::ensure_root(game_dir).await?.join(FILE);

    store::write_atomically(&file, serde_json::to_vec_pretty(project)?).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::renpy::Options;
    use crate::engine::{Swap, rpg_maker};
    use std::fs;

    #[test]
    fn tweaks_belong_to_an_engine_but_the_fonts_a_reader_picked_do_not() {
        let at = tempfile::tempdir().expect("a temp folder");
        let root = at.path();
        fs::create_dir_all(root.join("data")).unwrap();
        fs::write(root.join("data").join("System.json"), "{}").unwrap();

        let engine = rpg_maker::detect(root).expect("an rpg maker game");
        let held = Project {
            language: "Japanese".to_string(),
            tweaks: Tweaks::RenPy(Options::default()),
            fonts: Fonts {
                swaps: vec![Swap {
                    from: "message.ttf".to_string(),
                    to: "/fonts/sarabun.ttf".to_string(),
                }],
            },
            ..Project::default()
        };

        let settled = held.clone().under(engine.as_ref());

        assert_eq!(
            settled.tweaks,
            engine.tweaks(),
            "tweaks a game's engine cannot read are not its own to carry"
        );
        assert_eq!(
            settled.fonts, held.fonts,
            "a font is a font whichever engine draws it, so a reader keeps their pick"
        );
        assert_eq!(settled.language, "Japanese", "nothing else moves");
    }

    async fn written(raw: &str) -> tempfile::TempDir {
        let at = tempfile::tempdir().expect("a temp folder");
        let file = store::ensure_root(at.path())
            .await
            .expect("a store")
            .join(FILE);
        tokio::fs::write(&file, raw).await.expect("a project file");

        at
    }

    const STYLE: &str = r#""sourceLanguage":"Japanese","language":"English",
        "style":{"fidelity":"balanced","genres":[],"voices":[],"notes":""}"#;

    #[tokio::test]
    async fn a_project_holding_tweaks_this_build_never_heard_of_still_opens() {
        let at = written(&format!(
            r#"{{{STYLE},"tweaks":{{"kind":"whateverComesNext","font":"/fonts/sarabun.ttf"}}}}"#
        ))
        .await;

        let held = load(at.path()).await.expect("it reads").expect("a project");

        assert_eq!(
            held.tweaks,
            Tweaks::None,
            "tweaks meant for something else are dropped, not read as an error a reader has to go \
             and fix by hand"
        );
    }
}
