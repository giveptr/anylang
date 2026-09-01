mod assembly;
mod atlas;
mod blob;
mod bundle;
mod catalog;
mod cursor;
mod dotnet;
#[cfg(test)]
mod fake;
mod fonts;
mod format;
mod install;
mod layout;
mod localization;
mod mono_behaviour;
mod mono_script;
mod naming;
mod opened;
mod patch;
mod pictures;
mod prepare;
mod seal;
mod serial;
mod settle;
mod shapes;
mod sprite;
mod text_asset;
mod texture;

use crate::engine::pictures::{Handed, Shot};
use crate::engine::unity::serial::Container;
use crate::engine::{
    Engine, Font, Group, Install, Landing, Parsed, Prepare, Rules, Undo, filled, same_marks, sheet,
    symbolic,
};
use crate::scope::slashed;
use anyhow::{Context, Result};
use futures::future::BoxFuture;
use regex::Regex;
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use tokio::io::AsyncReadExt;

static RE_TAG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\[[-/=#.:A-Za-z0-9_]{1,32}\]|</?[A-Za-z#][^<>\n]{0,78}>")
        .expect("RE_TAG is a valid pattern")
});

static RE_SLOT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\{[A-Za-z0-9_.]{1,24}(?:[,:][^{}\n]{1,32})?\}")
        .expect("RE_SLOT is a valid pattern")
});

const MARKUP_RULES: &str = r"- anything in square brackets, e.g. [MC] [-MC] [4_NAME] [/i]
- anything in angle brackets, e.g. <b> </b> <color=#ff0000> <size=24>
- anything in curly braces, e.g. {0} {player} {item_count} {0:N0}";

const SHAPE_RULES: &str = "- An item with no newline in it is one line the game reads on its own: \
                           return it as one line.\n\
                           - An item holding newlines: return exactly as many lines as you were \
                           given, in the same order.";

const RETRY_RULES: &str = "Keep every [tag], <tag> and {placeholder} exactly as it appears in the \
                           source. Never drop one, and never add one the source does not have.";

const DATA: &str = "_Data";

pub struct Harvest {
    pub at: PathBuf,
    pub body: String,
    pub lines: u32,
}

impl Harvest {
    pub fn sheets(piled: BTreeMap<PathBuf, Vec<sheet::Line>>) -> Result<Vec<Self>> {
        piled
            .into_iter()
            .map(|(at, lines)| {
                Ok(Self {
                    at,
                    lines: lines.len() as u32,
                    body: sheet::page(lines)?,
                })
            })
            .collect()
    }
}

struct Unity;

pub fn detect(dir: &Path) -> Option<Box<dyn Engine>> {
    data_dir(dir).map(|_| Box::new(Unity) as Box<dyn Engine>)
}

pub fn forget() {
    pictures::forget();
}

pub fn data_dir(game_dir: &Path) -> Option<PathBuf> {
    let here: Vec<PathBuf> = fs::read_dir(game_dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect();

    here.iter()
        .filter(|at| at.is_dir())
        .find(|at| player_of(at).is_some_and(|player| stands_beside(&player, &here)))
        .cloned()
}

fn player_of(data: &Path) -> Option<String> {
    let name = data.file_name()?.to_string_lossy();

    Some(name.strip_suffix(DATA)?.to_string())
}

fn stands_beside(player: &str, here: &[PathBuf]) -> bool {
    here.iter().filter(|at| at.is_file()).any(|at| {
        at.file_name()
            .is_some_and(|name| names(&name.to_string_lossy(), player))
    })
}

fn names(file: &str, player: &str) -> bool {
    let Some((head, rest)) = file.split_at_checked(player.len()) else {
        return false;
    };

    head.eq_ignore_ascii_case(player) && (rest.is_empty() || rest.starts_with('.'))
}

impl Engine for Unity {
    fn label(&self) -> &str {
        "Unity"
    }

    fn wants(&self, path: &Path) -> bool {
        sheet::wants(path)
    }

    fn shown<'n>(&self, name: &'n str) -> Cow<'n, str> {
        sheet::shown(name)
    }

    fn parse(&self, _at: &Path, text: &str) -> Box<dyn Parsed> {
        Box::new(sheet::read(text, |said| {
            self.bare(said) == said && symbolic(said)
        }))
    }

    fn validate(&self, source: &str, translation: &str) -> Result<(), String> {
        filled(translation)?;

        if translation.chars().any(is_stray) {
            return Err("the translation holds a control character".to_string());
        }

        same_marks("text tags", &RE_TAG, source, translation)?;
        same_marks("placeholders", &RE_SLOT, source, translation)
    }

    fn rules(&self) -> Rules {
        Rules {
            markup: MARKUP_RULES,
            shape: Some(SHAPE_RULES),
            retry: RETRY_RULES,
        }
    }

    fn bare<'t>(&self, text: &'t str) -> Cow<'t, str> {
        match RE_TAG.replace_all(text, "") {
            Cow::Borrowed(bare) => RE_SLOT.replace_all(bare, ""),
            Cow::Owned(bare) => Cow::Owned(RE_SLOT.replace_all(&bare, "").into_owned()),
        }
    }

    fn fonts(&self, _game_dir: &Path, store: &Path) -> Vec<Font> {
        fonts::remembered(store)
    }

    fn pictures(&self, store: &Path) -> Vec<Shot> {
        pictures::LEDGER.remembered(store)
    }

    fn picture(&self, game_dir: &Path, store: &Path, key: &str) -> Result<Handed> {
        Ok(Handed::Drawn(pictures::drawn(game_dir, store, key)?))
    }

    fn group(&self, key: &str) -> Option<Group> {
        let (source, rest) = key.split_once('/')?;

        let (kind, apart) = match source {
            assembly::NAME => (assembly::KIND, false),
            mono_behaviour::NAME => (mono_behaviour::KIND, false),
            localization::NAME => (localization::KIND, false),
            text_asset::NAME => (text_asset::KIND, true),
            _ => return None,
        };

        let (above, _) = rest.rsplit_once('/')?;
        let holder = match apart {
            true => rest,
            false => above,
        };

        Some(Group {
            key: format!("{source}/{holder}"),
            kind: kind.to_string(),
            label: self.shown(holder).into_owned(),
        })
    }

    fn output(&self, at: Landing<'_>) -> PathBuf {
        at.staged()
    }

    fn undo(&self) -> Undo {
        Undo::Remove
    }

    fn wanted_by_default(&self) -> bool {
        false
    }

    fn install<'a>(&'a self, at: Install<'a>) -> BoxFuture<'a, Result<()>> {
        install::run(at)
    }

    fn prepare<'a>(&'a self, at: Prepare<'a>) -> BoxFuture<'a, Result<()>> {
        Box::pin(prepare::run(at))
    }
}

pub async fn container_kind(at: &Path) -> Result<Option<(bool, u64)>> {
    let size = tokio::fs::metadata(at)
        .await
        .with_context(|| format!("looking at {}", at.display()))?
        .len();

    let mut head = vec![0u8; serial::HEAD];
    let mut file = tokio::fs::File::open(at)
        .await
        .with_context(|| format!("opening {}", at.display()))?;

    let mut filled = 0;
    while filled < head.len() {
        let read = file
            .read(&mut head[filled..])
            .await
            .with_context(|| format!("reading the head of {}", at.display()))?;

        if read == 0 {
            break;
        }
        filled += read;
    }
    head.truncate(filled);

    if !head.starts_with(bundle::MAGIC) && !serial::announces_itself(&head, size) {
        return Ok(None);
    }

    Ok(Some((head.starts_with(bundle::MAGIC), size)))
}

fn is_stray(letter: char) -> bool {
    letter.is_control() && !matches!(letter, '\n' | '\r' | '\t')
}

fn holder_of(relative: &Path, data: Option<&Path>) -> String {
    let inside = data
        .and_then(|data| relative.strip_prefix(data).ok())
        .unwrap_or(relative);

    slashed(inside)
}

pub struct Known {
    pub assemblies: dotnet::Assemblies,
    pub classes: mono_script::Names,
    pub named: pictures::Named,
    pub books: localization::Collections,
}

async fn assemblies_beside(game_dir: &Path) -> dotnet::Assemblies {
    let managed = data_dir(game_dir).unwrap_or_default().join("Managed");

    tokio::task::spawn_blocking(move || dotnet::Assemblies::read(&managed))
        .await
        .unwrap_or_default()
}

#[derive(Default)]
struct Learning {
    classes: mono_script::Names,
    named: pictures::Named,
    books: localization::Collections,
}

impl Learning {
    fn take_in(&mut self, one: &Container, assemblies: &dotnet::Assemblies) {
        self.classes.learn(one);
        self.named.learn(one);
        self.books.learn(one, assemblies);
    }

    fn done(mut self, assemblies: dotnet::Assemblies) -> Known {
        self.books.confirm(&self.classes);

        Known {
            assemblies,
            classes: self.classes,
            named: self.named,
            books: self.books,
        }
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::engine::Offer;

    #[test]
    fn only_a_line_nobody_wrote_to_be_read_is_a_symbol() {
        for text in [
            "//",
            "...",
            "->",
            "\u{2026}",
            "scene_042_intro",
            "MAX_HP",
            "ui/bg_town.png",
            "portrait.psd",
            "***CHOICE***",
            "***day_change***",
            "rotate:90",
            "size:4096,4096",
            "pma:true",
            "bounds:3075,710,253,143",
        ] {
            assert!(symbolic(text), "{text} carries nothing to translate");
        }

        for text in ["a", "A", "x", "S", "0", "-"] {
            assert!(
                symbolic(text),
                "{text} is one ascii character, and there is nothing in one letter to translate"
            );
        }

        for text in [
            "isWalk",
            "MusicVolume",
            "RestartRound",
            "SetCurrentMap",
            "PlayerMapDetector",
            "OnPlayerEnterMap",
        ] {
            assert!(
                symbolic(text),
                "{text} is a name a compiler reads: a lowercase letter running straight into a \
                 capital is how code spells a phrase and how nobody spells a sentence"
            );
        }

        for text in [
            "Peter",
            "Helloooooo",
            "WHAT?!",
            "GAH!",
            "OW!",
            "Hm?",
            "Nice~",
            "Y-Yes...~",
            "N-No",
            "V-I-V-I-A-N",
            "T-shirt",
            "0.82.x",
            "Wait. I know that.",
            "\u{5f7c}\u{5973}\u{306f}\u{9996}\u{3092}\u{304b}\u{305f}\u{3080}\u{3051}\u{305f}\u{3002}",
            "Priscilla",
            "OK",
            "Pr\u{e9}-requisito:",
            "\u{540d}\u{524d}:\u{592a}\u{90ce}",
            "\u{d55c}\u{ad6d}\u{c5b4}",
            "\u{53f0}",
            "\u{79c1}",
            "Locations:",
            "Strength:",
            "money:",
        ] {
            assert!(!symbolic(text), "{text} is a word somebody wrote");
        }

        assert!(
            symbolic("img/\u{30ad}\u{30e3}\u{30e9}.png"),
            "a path is a path whatever the folder is called, so the slash has to be read before \
             the letters are"
        );
        assert!(
            symbolic("rotate:90") && symbolic("size:4096,4096"),
            "a mark between two runs of letters belongs to the code, while the same mark trailing at \
             the end is a person's punctuation"
        );

        assert!(
            !symbolic("The resident sociopath/bully. It's hard to say anything good."),
            "a slash stands between two words often enough that it cannot outrank a space: \
             calling a line of talk a symbol is the one mistake that loses real text"
        );
        assert!(
            !symbolic("mary/at home/mary_6"),
            "and the price of that is a path holding a space, which reads as talk"
        );
        for text in [
            "peter", "mouth1", "vertices", "en", "beer", "wine", "amigo", "cerveja",
        ] {
            assert!(
                !symbolic(text),
                "{text} reads like a key and often is one, but a lone lowercase word is also how \
                 ten languages write beer, wine and friend. Measured over 135,000 localized \
                 lines it caught 106 real words and not one key, and every key it would have \
                 caught sits in a Spine group the reader excludes whole at the tree"
            );
        }
        assert!(
            symbolic("**Thud**"),
            "on its own a wrapped sound cannot be told from a wrapped marker: the tags around \
             it are what save it, which is why the symbols offer looks at those first"
        )
    }

    #[test]
    fn only_a_file_wearing_the_sheet_ending_is_ours_to_install() {
        assert!(Unity.wants(Path::new("localization/UI_GENERAL/en/UI_GENERAL_en.sheet")));
        assert!(Unity.wants(Path::new("text_asset/resources.assets/line/scene051.sheet")));
        assert!(!Unity.wants(Path::new("text_asset/resources.assets/notes.md")));
        assert!(!Unity.wants(Path::new("StreamingAssets/aa/catalog.bin")));
        assert!(!Unity.wants(Path::new("resources.assets")));
    }

    #[test]
    fn a_mark_the_source_carries_and_the_translation_drops_is_refused() {
        let source = "(Forget the junk, [MC]Peter[-MC]. Move it.)";

        assert!(
            Unity
                .validate(source, "([MC]Peter[-MC]、そのゴミは放っておけ。)")
                .is_ok()
        );
        assert!(
            Unity
                .validate(source, "(Peter、そのゴミは放っておけ。)")
                .is_err()
        );
        assert!(
            Unity
                .validate("Ready in {0} turns", "あと数ターンで完了")
                .is_err()
        );
    }

    #[test]
    fn every_mark_the_rules_show_is_one_the_engine_refuses_to_lose() {
        let told = Unity.rules().markup;
        let shown: Vec<&str> = RE_TAG
            .find_iter(told)
            .chain(RE_SLOT.find_iter(told))
            .map(|one| one.as_str())
            .collect();

        assert!(
            shown.len() >= 8,
            "the rules name marks by example, so there have to be examples to read: {shown:?}"
        );

        for one in shown {
            assert!(
                Unity
                    .validate(&format!("She said {one} there."), "そこでそう言った。")
                    .is_err(),
                "{one} is held out to the model as a mark to copy, but dropping it is not refused"
            );
        }
    }

    #[test]
    fn a_format_placeholder_and_a_long_rich_text_tag_are_marks() {
        assert!(
            Unity
                .validate("You have {0:N0} gold", "金貨を持っている")
                .is_err(),
            "the format tail names how the number prints, and losing the slot loses the number"
        );
        assert!(
            Unity
                .validate("You have {0:N0} gold", "{0:N0} 金貨を持っている")
                .is_ok()
        );

        let opened = "<font=\"Noto Sans JP SDF\" material=\"Outline\">Warning</font>";
        assert!(
            Unity.validate(opened, "警告</font>").is_err(),
            "a translation keeping only the closer ships a stray close tag"
        );

        assert!(
            Unity
                .validate("Damage < 50 or Armor > 10", "ダメージ50未満か防具10超")
                .is_ok(),
            "a comparison written in prose is not a tag, and demanding it byte for byte refuses \
             every faithful translation"
        );
    }

    #[test]
    fn a_tag_that_closes_one_counts_as_a_tag_of_its_own() {
        let source = "[i]She tilted her head.[/i]";

        assert!(
            Unity
                .validate(source, "[i]彼女は首をかたむけた。[/i]")
                .is_ok()
        );
        assert!(
            Unity.validate(source, "[i]彼女は首をかたむけた。").is_err(),
            "dropping the closing tag leaves the rest of the screen italic, so it may not pass"
        );
        assert!(
            Unity
                .validate("[b]Ready[/b] now", "[b]準備[/b]できた")
                .is_ok(),
            "the slash may only stand where a tag closes"
        );
    }

    fn marked(text: &str) -> bool {
        let page = sheet::write([("0".to_string(), text.to_string())]).expect("a sheet");

        Unity.parse(Path::new("one.sheet"), &page).units()[0].offer != Offer::Asked
    }

    #[test]
    fn a_rule_written_as_prose_reaches_the_model_as_one_line() {
        assert!(
            !RETRY_RULES.contains('\n'),
            "retry rules are one paragraph, so a break in them is only this file's wrapping and \
             the model reads it as a break the author never meant"
        );

        for listed in [MARKUP_RULES, SHAPE_RULES] {
            for line in listed.lines() {
                assert!(
                    line.starts_with("- "),
                    "every line of a rule list is its own bullet, so a wrapped one reads as a \
                     rule of its own: {line:?}"
                );
            }
        }
    }

    #[test]
    fn a_line_the_game_marks_up_for_reading_is_never_called_a_symbol() {
        for text in [
            "[i]**Crash**[/i]",
            "[i]**Thud**[/i]",
            "[b]**Clang**[/b]",
            "[u]ui/bg.png[/u]",
            "[i]Soon.[/i]",
        ] {
            assert!(
                !marked(text),
                "{text} is wrapped for a reader to see, so it holds words"
            );
        }

        for text in [
            "***CHOICE***",
            "***day_change***",
            "ui/bg.png",
            "rotate:90",
            "//",
        ] {
            assert!(!text.contains('['), "{text} carries no markup to save it");
            assert!(marked(text), "{text} would break the game if translated");
        }
    }

    #[test]
    fn a_line_the_format_asked_about_is_judged_on_its_structure_not_its_letters() {
        assert!(
            Unity.answered("...{0}?", "...{0} \u{304b}?").is_ok(),
            "a line of nothing but a placeholder and punctuation still reaches the model when \
             the format calls it text, and the natural rendering adds a word; counting letters \
             in the source would refuse the very answer a translator would write"
        );
        assert!(
            Unity.answered("...{0}?", "... \u{304b}?").is_err(),
            "what keeps the game whole is the placeholder, and that is still checked"
        );
        assert!(
            Unity.answered("6'1\"", "185 cm").is_ok(),
            "a measure with no letters in it is still a line someone reads"
        );
    }

    #[test]
    fn a_line_no_text_field_could_carry_is_refused_before_it_reaches_any_container() {
        assert!(
            Unity.validate("Wait.", "one\u{1}two").is_err(),
            "a control character has no place in a line a player reads"
        );
        assert!(
            Unity.validate("Wait.", "  ").is_err(),
            "a blank translation would wipe the line"
        );
        assert!(
            Unity
                .validate("Wait.\nMorning.", "待って。\nおはよう。")
                .is_ok(),
            "the breaks a script is written with have to come through"
        );
    }

    #[test]
    fn nothing_scratch_is_ever_written_into_the_game_folder() {
        let game = Path::new("/games/Fake");
        let store = Path::new("/data/store/taffy");

        let landing = Unity.output(Landing {
            game_dir: game,
            store,
            language: "japanese",
        });

        assert!(
            landing.starts_with(store),
            "the staging tree belongs to the store, not to the game: {}",
            landing.display()
        );
        assert!(!landing.starts_with(game));
    }

    #[test]
    fn every_piece_of_text_names_the_container_it_came_out_of() {
        let asset = Unity
            .group("text_asset/resources.assets/line/scene051_two_talkers.sheet")
            .expect("a text asset comes out of a container");

        assert_eq!(
            asset.key, "text_asset/resources.assets/line/scene051_two_talkers.sheet",
            "an asset is a file the author wrote and named, so it is a row of its own: a game \
             that ships its own translations keeps each language in one, and gathering them would \
             leave no way to skip a language"
        );
        assert_eq!(asset.label, "resources.assets/line/scene051_two_talkers");
        assert_eq!(asset.kind, "TextAsset");

        let field = Unity
            .group("mono_behaviour/sharedassets0.assets/SceneHandler/dialogCells.text/43165.sheet")
            .expect("one field of one class");

        assert_eq!(
            field.key,
            "mono_behaviour/sharedassets0.assets/SceneHandler/dialogCells.text"
        );
        assert_eq!(
            field.label, "sharedassets0.assets/SceneHandler/dialogCells.text",
            "one class and field pair lives in more than one container, so the label has to name it"
        );
        assert_eq!(field.kind, "MonoBehaviour");

        let table = Unity
            .group("localization/UI_GENERAL/zh-Hans/UI_GENERAL_zh-Hans.sheet")
            .expect("a collection in one language is one thing to translate");

        assert_eq!(table.key, "localization/UI_GENERAL/zh-Hans");
        assert_eq!(
            table.label, "UI_GENERAL/zh-Hans",
            "one row is one collection in one language, so translating it never reaches another"
        );
        assert_eq!(table.kind, "StringTable");
    }

    #[test]
    fn the_folder_a_source_is_sorted_into_is_ours_and_never_part_of_a_label() {
        for key in [
            "text_asset/resources.assets/line/scene051_two_talkers.sheet",
            "mono_behaviour/sharedassets0.assets/SceneHandler/dialogCells.text/43165.sheet",
            "localization/UI_GENERAL/zh-Hans/UI_GENERAL_zh-Hans.sheet",
            "assembly/Assembly-CSharp.dll/DialogueManager/0.sheet",
        ] {
            let source = key.split('/').next().expect("a source folder");
            let group = Unity.group(key).expect("a source this engine knows");
            let under = group
                .key
                .strip_prefix(&format!("{source}/"))
                .expect("a key filed under the folder its source was sorted into");

            assert_eq!(
                Unity.shown(under),
                group.label,
                "a label is the key with our own sorting folder taken off, so filtering the file \
                 rail by name reaches every part of a path the reader can see and none of ours: \
                 {source} is a word we chose, and nobody reading the screen can guess it"
            );
            assert!(
                !group.kind.is_empty(),
                "a kind is the one thing telling a grouped row from a lone file, and the rail \
                 filter reads a label and a kind where a lone file gives up its whole path: a \
                 group naming no kind would hand {source} back to the reader"
            );
        }
    }

    #[test]
    fn a_staging_suffix_of_ours_is_never_shown_to_the_reader() {
        assert_eq!(Unity.shown("34951.sheet"), "34951");
        assert_eq!(Unity.shown("credits.sheet"), "credits");
        assert_eq!(
            Unity.shown("scene051.txt"),
            "scene051.txt",
            "an ending the game itself gave the asset is the reader's to see"
        );
        assert_eq!(
            Unity.shown("credits.sheet.sheet"),
            "credits.sheet",
            "an asset the game really did call credits.sheet keeps that name: only the one \
             ending we added comes off"
        );
        assert_eq!(Unity.shown("34951"), "34951");
    }

    #[test]
    fn text_from_a_source_this_engine_does_not_know_stays_ungrouped() {
        assert!(Unity.group("script.rpy").is_none());
        assert!(Unity.group("text_asset/resources.assets").is_none());
        assert!(Unity.group("somethingnew/holder/one.txt").is_none());
    }

    #[test]
    fn the_versioned_data_folder_never_becomes_part_of_a_key() {
        let data = Some(Path::new("Fake_1.2.3_Data"));

        assert_eq!(
            holder_of(Path::new("Fake_1.2.3_Data/resources.assets"), data),
            "resources.assets",
            "the folder name carries the game version, so a key holding it would break on every update"
        );
        assert_eq!(
            holder_of(Path::new("Fake_1.2.3_Data/StreamingAssets/one.unity"), data),
            "StreamingAssets/one.unity"
        );
        assert_eq!(
            holder_of(Path::new("Bundles/extra.bundle"), data),
            "Bundles/extra.bundle",
            "a container shipped outside the data folder keeps its own path"
        );
    }

    #[test]
    fn a_folder_without_a_unity_data_directory_is_not_claimed() {
        let sandbox = tempfile::tempdir().expect("a temp folder");

        assert!(detect(sandbox.path()).is_none());

        fs::write(sandbox.path().join("Fake_Data"), []).expect("a decoy");
        assert!(
            detect(sandbox.path()).is_none(),
            "a plain file wearing the name is not the data folder"
        );
    }

    #[test]
    fn a_data_folder_with_no_player_beside_it_is_somebody_elses() {
        let sandbox = tempfile::tempdir().expect("a temp folder");
        fs::create_dir_all(sandbox.path().join("backup_Data")).expect("a folder");

        assert!(
            detect(sandbox.path()).is_none(),
            "any folder may end in _Data: only Unity names one after the player beside it"
        );
    }

    #[test]
    fn the_player_and_its_data_folder_are_the_whole_signature() {
        for player in ["Fake.exe", "Fake.x86_64", "Fake"] {
            let sandbox = tempfile::tempdir().expect("a temp folder");
            let data = sandbox.path().join("Fake_Data");

            fs::create_dir_all(data.join("il2cpp_data")).expect("a data folder");
            fs::write(data.join("data.unity3d"), []).expect("a packed build");
            fs::write(sandbox.path().join(player), []).expect("a player");

            assert!(
                detect(sandbox.path()).is_some(),
                "{player}: a build that packs everything into data.unity3d carries no loose \
                 settings file, so nothing inside may be required to recognise it"
            );
        }
    }

    #[test]
    fn a_player_spelled_with_other_capitals_than_its_data_folder_still_owns_it() {
        let sandbox = tempfile::tempdir().expect("a temp folder");
        fs::create_dir_all(sandbox.path().join("man of the house_Data")).expect("a data folder");
        fs::write(sandbox.path().join("Man of the house.exe"), []).expect("a player");

        assert!(
            detect(sandbox.path()).is_some(),
            "the two names are typed by hand and drift apart; Windows hides that because it \
             matches them without case, so a case-sensitive filesystem must too"
        );
    }

    #[test]
    fn a_player_whose_name_merely_starts_the_same_does_not_own_the_data_folder() {
        let sandbox = tempfile::tempdir().expect("a temp folder");
        fs::create_dir_all(sandbox.path().join("Fake_Data")).expect("a data folder");
        fs::write(sandbox.path().join("Faker.exe"), []).expect("a stranger");

        assert!(
            detect(sandbox.path()).is_none(),
            "claiming a folder this tool cannot really read would offer the reader a game it \
             then fails to open, and the data folder belongs to whichever player is named exactly"
        );
    }
}
