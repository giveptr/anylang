pub mod fonts;
pub mod layout;
pub mod pictures;
pub mod renpy;
pub mod rpg_maker;
pub mod sheet;
mod text;
pub mod unity;
pub mod wolf_rpg;

pub use crate::engine::text::{
    filled, hand_written, has_words, humped, marks, names_a_file, only_in, quoted, same_marks,
    symbolic, unicode_escape, worth,
};
#[cfg(test)]
use crate::progress::Quiet;
use crate::progress::{Progress, Source};
use anyhow::Result;
use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Type,
)]
#[serde(rename_all = "camelCase")]
pub enum Offer {
    #[default]
    Asked,
    Listed,
    Locked,
}

impl Offer {
    fn firmer(self, other: Self) -> Self {
        self.max(other)
    }

    pub fn asked(self) -> bool {
        matches!(self, Self::Asked)
    }

    pub fn unlocked(self) -> bool {
        !matches!(self, Self::Locked)
    }

    pub fn or_listed(self, listed: bool) -> Self {
        match listed {
            true => self.firmer(Self::Listed),
            false => self,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct TranslationUnit {
    pub id: u32,
    pub text: String,
    #[serde(skip)]
    pub offer: Offer,
}

impl TranslationUnit {
    pub fn answer<'t>(&self, translations: &'t BTreeMap<u32, String>) -> Option<&'t String> {
        translations.get(&self.id).filter(|_| self.offer.unlocked())
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Applied {
    pub lines: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Tweaks {
    #[default]
    None,
    #[serde(rename = "renpy")]
    RenPy(renpy::Options),
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Font {
    pub name: String,
    pub at: String,
    pub shown: String,
    pub builtin: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Swap {
    pub from: String,
    pub to: String,
}

pub enum Extra {
    Write { at: PathBuf, body: String },
    Copy { from: PathBuf, at: PathBuf },
}

impl Extra {
    pub fn at(&self) -> &Path {
        match self {
            Self::Write { at, .. } | Self::Copy { at, .. } => at,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Undo {
    Restore,
    Remove,
}

pub struct Rules {
    pub markup: &'static str,
    pub shape: Option<&'static str>,
    pub retry: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub key: String,
    pub kind: String,
    pub label: String,
}

pub struct Prepare<'a> {
    pub game_dir: &'a Path,
    pub source: &'a Path,
    pub store: &'a Path,
    pub tweaks: &'a Tweaks,
    pub progress: &'a dyn Progress,
}

pub const STAGED: &str = "staged";

#[derive(Clone, Copy)]
pub struct Landing<'a> {
    pub game_dir: &'a Path,
    pub store: &'a Path,
    pub language: &'a str,
}

#[cfg(test)]
impl<'a> Landing<'a> {
    pub fn over(game_dir: &'a Path, store: &'a Path, language: &'a str) -> Self {
        Self {
            game_dir,
            store,
            language,
        }
    }
}

impl Landing<'_> {
    pub fn staged(&self) -> PathBuf {
        self.store.join(STAGED).join(self.language.to_lowercase())
    }
}

pub struct Install<'a> {
    pub game_dir: &'a Path,
    pub staged: &'a Path,
    pub store: &'a Path,
    pub fonts: &'a fonts::Fonts,
    pub pictures: &'a pictures::Pictures,
    pub reverting: bool,
    pub progress: &'a dyn Progress,
    pub doing: Source,
}

impl Install<'_> {
    pub async fn chosen(&self) -> BTreeMap<String, pictures::Chosen> {
        if self.reverting {
            return BTreeMap::new();
        }

        let (chosen, complaints) = self.pictures.chosen().await;
        for why in &complaints {
            self.progress.warn(self.doing, why);
        }

        chosen
    }

    pub fn picked<'d>(
        &self,
        chosen: &'d BTreeMap<String, pictures::Chosen>,
    ) -> Vec<(&str, &'d pictures::Chosen)> {
        self.pictures
            .filled()
            .into_iter()
            .filter_map(|(key, from)| chosen.get(from).map(|held| (key, held)))
            .collect()
    }
}

#[cfg(test)]
impl<'a> Prepare<'a> {
    pub fn over(game_dir: &'a Path, source: &'a Path, store: &'a Path) -> Self {
        Self {
            game_dir,
            source,
            store,
            tweaks: &Tweaks::None,
            progress: &Quiet,
        }
    }

    pub fn heard_by(mut self, progress: &'a dyn Progress) -> Self {
        self.progress = progress;
        self
    }
}

#[cfg(test)]
impl<'a> Install<'a> {
    pub fn over(game_dir: &'a Path, staged: &'a Path, store: &'a Path) -> Self {
        Self {
            game_dir,
            staged,
            store,
            fonts: &fonts::NOTHING,
            pictures: &pictures::NOTHING,
            reverting: false,
            progress: &Quiet,
            doing: Source::Export,
        }
    }

    pub fn sending(mut self, fonts: &'a fonts::Fonts) -> Self {
        self.fonts = fonts;
        self
    }

    pub fn drawing(mut self, pictures: &'a pictures::Pictures) -> Self {
        self.pictures = pictures;
        self
    }

    pub fn putting_back(mut self, reverting: bool) -> Self {
        self.reverting = reverting;
        self
    }

    pub fn heard_by(mut self, progress: &'a dyn Progress) -> Self {
        self.progress = progress;
        self
    }
}

pub trait Parsed: Send {
    fn units(&self) -> &[TranslationUnit];

    fn render(self: Box<Self>, translations: &BTreeMap<u32, String>) -> Result<(String, Applied)>;
}

pub trait Engine: Send + Sync {
    fn label(&self) -> &str;

    fn wants(&self, path: &Path) -> bool;

    fn parse(&self, at: &Path, text: &str) -> Box<dyn Parsed>;

    fn validate(&self, source: &str, translation: &str) -> Result<(), String>;

    fn rules(&self) -> Rules;

    fn output(&self, at: Landing<'_>) -> PathBuf;

    fn prepare<'a>(&'a self, at: Prepare<'a>) -> BoxFuture<'a, Result<()>>;

    fn install<'a>(&'a self, at: Install<'a>) -> BoxFuture<'a, Result<()>>;

    fn undo(&self) -> Undo;

    fn group(&self, key: &str) -> Option<Group> {
        let _ = key;
        None
    }

    fn bare<'t>(&self, text: &'t str) -> Cow<'t, str>;

    fn worth_asking(&self, text: &str) -> bool {
        self.bare(text).chars().any(char::is_alphabetic)
    }

    fn answered(&self, source: &str, translation: &str) -> Result<(), String> {
        self.validate(source, translation)?;

        if !self.worth_asking(source) && translation != source {
            return Err("this line holds no word to translate".to_string());
        }

        if hand_written(source) && translation.trim() == source.trim() {
            return Err("this came back in the language it was written in".to_string());
        }

        Ok(())
    }

    fn piles(&self) -> bool {
        true
    }

    fn wanted_by_default(&self) -> bool {
        true
    }

    fn tweaks(&self) -> Tweaks {
        Tweaks::None
    }

    fn retarget<'t>(&self, text: &'t str, language: &str) -> Cow<'t, str> {
        let _ = language;
        Cow::Borrowed(text)
    }

    fn shown<'n>(&self, name: &'n str) -> Cow<'n, str> {
        Cow::Borrowed(name)
    }

    fn extras(&self, at: Landing<'_>, tweaks: &Tweaks, fonts: &fonts::Fonts) -> Vec<Extra> {
        let _ = (at, tweaks, fonts);
        Vec::new()
    }

    fn fonts(&self, game_dir: &Path, store: &Path) -> Vec<Font>;

    fn pictures(&self, store: &Path) -> Vec<pictures::Shot>;

    fn picture(&self, game_dir: &Path, store: &Path, key: &str) -> Result<pictures::Handed>;

    fn sources(&self, game_dir: &Path) -> Vec<String> {
        let _ = game_dir;
        Vec::new()
    }

    fn source_key(&self, tweaks: &Tweaks) -> String {
        let _ = tweaks;
        String::new()
    }
}

pub fn detect(dir: &Path) -> Option<Box<dyn Engine>> {
    renpy::detect(dir)
        .or_else(|| rpg_maker::detect(dir))
        .or_else(|| wolf_rpg::detect(dir))
        .or_else(|| unity::detect(dir))
}

pub fn refused(dir: &Path) -> Option<String> {
    rpg_maker::refused(dir)
}

pub fn forget() {
    renpy::forget();
    rpg_maker::forget();
    unity::forget();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::renpy::RenPy;

    #[test]
    fn nothing_a_reader_is_offered_ever_loosens_and_the_wire_spells_each_state_out() {
        assert!(Offer::Asked < Offer::Listed && Offer::Listed < Offer::Locked);

        for held in [Offer::Asked, Offer::Listed, Offer::Locked] {
            for other in [Offer::Asked, Offer::Listed, Offer::Locked] {
                let firmer = held.firmer(other);
                assert!(
                    firmer >= held && firmer >= other,
                    "a field inside another may only ever say the same or less about a line than \
                     the one holding it, so combining two offers may not hand back a looser one"
                );
            }
        }

        assert_eq!(Offer::default(), Offer::Asked);
        assert_eq!(
            [Offer::Asked, Offer::Listed, Offer::Locked]
                .map(|held| serde_json::to_string(&held).expect("an offer goes out as json")),
            [r#""asked""#, r#""listed""#, r#""locked""#],
            "the editor is typed against these three spellings, and the order they are declared \
             in is what firmer reads as strength: reordering them to suit the front end would \
             quietly invert it"
        );
    }

    #[test]
    fn a_model_handing_a_line_straight_back_is_refused_but_a_person_may_keep_it() {
        let said = "「話は聞かせてもらったよ、決闘するんだってね。";

        assert!(
            RenPy.answered(said, said).is_err(),
            "the model sometimes echoes what it was sent, and nothing else notices"
        );
        assert!(
            RenPy.validate(said, said).is_ok(),
            "a reader typing the source back is saying leave this one alone: the only way to \
             pin a name a plugin looks up"
        );
        assert!(RenPy.answered(said, "\"I heard all about it.").is_ok());
        assert!(
            RenPy.answered(said, &format!("  {said}  ")).is_err(),
            "padding it with spaces is the same line"
        );

        for code in ["X07", "B01", "SPAS-12"] {
            assert!(
                RenPy.answered(code, code).is_ok(),
                "{code} is spelled in the alphabet the target uses, so leaving it is an answer"
            );
        }
    }
}
