use crate::engine::renpy::script::{self, Spot};
use crate::engine::renpy::{GAME_DIR, TL_DIR, scripts, text};
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

const ORIGINAL: &str = "None";

fn beside(game_dir: &Path) -> PathBuf {
    game_dir.join(GAME_DIR).join(TL_DIR)
}

fn folder(game_dir: &Path, name: &str) -> PathBuf {
    beside(game_dir).join(name)
}

pub fn sources(game_dir: &Path, skip: &[String]) -> Vec<String> {
    let Ok(reading) = fs::read_dir(beside(game_dir)) else {
        return Vec::new();
    };

    let mut out: Vec<String> = reading
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .filter(|name| name != ORIGINAL && !skip.contains(name))
        .filter(|name| offers(&folder(game_dir, name)))
        .collect();

    out.sort();

    out
}

fn offers(at: &Path) -> bool {
    scripts(at)
        .into_iter()
        .any(|path| fs::read_to_string(&path).is_ok_and(|text| script::offers(&text)))
}

#[derive(Debug, Default)]
pub struct Shipped {
    said: HashMap<Spot, String>,
}

impl Shipped {
    fn take(&mut self, text: &str) {
        for (spot, said) in script::harvest(text) {
            self.said.entry(spot).or_insert(said);
        }
    }

    fn lines(&self) -> u32 {
        self.said.len() as u32
    }

    fn offer(&self, spot: &Spot, was: &str) -> Option<String> {
        let instead = self.said.get(spot)?;
        if instead == was || !text::same_slots(was, instead) {
            return None;
        }

        Some(instead.clone())
    }
}

fn read(game_dir: &Path, name: &str) -> Result<Shipped> {
    let mut shipped = Shipped::default();

    for path in scripts(&folder(game_dir, name)) {
        let text =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;

        shipped.take(&text);
    }

    Ok(shipped)
}

#[derive(Debug, Default)]
pub struct Counted {
    pub taken: u32,
    pub kept: u32,
    pub lines: u32,
}

pub fn apply(source: &Path, game_dir: &Path, name: &str) -> Result<Counted> {
    let shipped = read(game_dir, name)?;
    let offer = |spot: &Spot, was: &str| shipped.offer(spot, was);

    let mut counted = Counted {
        lines: shipped.lines(),
        ..Counted::default()
    };

    for path in scripts(source) {
        let text =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;

        let overlaid = script::overlay(&text, &offer);
        counted.taken += overlaid.taken;
        counted.kept += overlaid.kept;

        if let Some(written) = overlaid.text {
            fs::write(&path, written).with_context(|| format!("writing {}", path.display()))?;
        }
    }

    Ok(counted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Engine;
    use crate::engine::renpy::RenPy;
    use std::collections::BTreeMap;

    const SKELETON: &str = concat!(
        "translate patch cap_arr_13ae5179:\n",
        "\n",
        "    # mia \"Дверь заперта.\"\n",
        "    mia \"Дверь заперта.\"\n",
        "\n",
        "translate patch strings:\n",
        "\n",
        "    old \"Назад\"\n",
        "    new \"\"\n",
    );

    const SHIPPED: &str = concat!(
        "translate english cap_arr_13ae5179:\n",
        "\n",
        "    mia \"The door is locked.\"\n",
        "\n",
        "translate english strings:\n",
        "\n",
        "    old \"Назад\"\n",
        "    new \"Back\"\n",
    );

    fn overlaid(skeleton: &str, shipped: &str) -> script::Overlaid {
        let mut kept = Shipped::default();
        kept.take(shipped);

        script::overlay(skeleton, &|spot, was| kept.offer(spot, was))
    }

    fn sources_of(text: &str) -> Vec<String> {
        RenPy
            .parse(Path::new("script.rpy"), text)
            .units()
            .iter()
            .map(|unit| unit.text.clone())
            .collect()
    }

    #[test]
    fn what_the_game_is_handed_is_a_translation_of_the_script_it_actually_runs() {
        let read_from_english = overlaid(SKELETON, SHIPPED)
            .text
            .expect("both lines were offered in English");

        let engine = RenPy;
        let parsed = engine.parse(Path::new("script.rpy"), &read_from_english);
        let said: BTreeMap<u32, String> = parsed
            .units()
            .iter()
            .map(|unit| {
                let into = match unit.text.as_str() {
                    "Back" => "\u{623b}\u{308b}",
                    _ => "\u{6249}\u{306f}\u{9589}\u{307e}\u{3063}\u{3066}\u{3044}\u{308b}\u{3002}",
                };

                (unit.id, into.to_string())
            })
            .collect();

        let (rendered, applied) = parsed.render(&said).expect("renders");
        let exported = engine.retarget(&rendered, "French");

        assert_eq!(applied.lines, 2);
        assert!(
            exported.contains("translate french cap_arr_13ae5179:"),
            "the identifier Ren'Py looks the line up by has to survive the whole trip:\n{exported}"
        );
        assert!(exported.contains(
            "    mia \"\u{6249}\u{306f}\u{9589}\u{307e}\u{3063}\u{3066}\u{3044}\u{308b}\u{3002}\""
        ));
        assert!(
            exported.contains("    old \"Назад\"\n    new \"\u{623b}\u{308b}\""),
            "the key stays the game's own words and only the answer beside it is ours:\n{exported}"
        );
        assert!(
            !exported.contains("Дверь"),
            "no line of the original may be left where the game would speak it:\n{exported}"
        );
    }

    #[test]
    fn a_shipped_line_becomes_the_text_to_translate_while_the_game_keeps_its_own_words() {
        let done = overlaid(SKELETON, SHIPPED);
        let written = done.text.expect("both lines were offered in English");

        assert_eq!((done.taken, done.kept), (2, 0));
        assert_eq!(
            sources_of(&written),
            ["The door is locked.", "Back"],
            "the translator has to be reading the English now"
        );
        assert!(
            written.contains("    mia \"Дверь заперта.\"\n"),
            "the line the game plays may not be touched, only the comment above it:\n{written}"
        );
        assert!(
            written.contains("    old \"Назад\"\n"),
            "old is what Ren'Py looks the string up by at run time:\n{written}"
        );
    }

    #[test]
    fn a_line_the_shipped_folder_never_translated_keeps_the_words_the_game_shipped() {
        let done = overlaid(
            SKELETON,
            concat!(
                "translate english cap_arr_13ae5179:\n",
                "    mia \"\"\n",
                "translate english strings:\n",
                "    old \"Назад\"\n",
                "    new \"Назад\"\n",
            ),
        );

        assert_eq!((done.taken, done.kept), (0, 2));
        assert!(
            done.text.is_none(),
            "nothing was offered, so the file is left exactly as Ren'Py wrote it"
        );
    }

    #[test]
    fn a_shipped_line_that_lost_a_variable_is_refused() {
        let skeleton = concat!(
            "translate patch mia_1:\n",
            "    # mia \"Где [playername]?\"\n",
            "    mia \"Где [playername]?\"\n",
        );

        let done = overlaid(
            skeleton,
            "translate english mia_1:\n    mia \"Where is he?\"\n",
        );

        assert_eq!(
            (done.taken, done.kept),
            (0, 1),
            "translating this would drop the player's name from the game and nothing downstream \
             could tell, because the English never had it"
        );
    }

    #[test]
    fn the_language_the_shipped_header_names_is_ignored() {
        let done = overlaid(
            SKELETON,
            concat!(
                "translate russian cap_arr_13ae5179:\n",
                "    mia \"The door is locked.\"\n",
            ),
        );

        assert_eq!(
            done.taken, 1,
            "a shipped folder stamps blocks with whatever language its author left there, so only \
             the identifier can be trusted"
        );
    }

    #[test]
    fn a_line_belonging_to_no_block_is_never_offered_to_another_file() {
        let done = overlaid(
            "translate patch a_1:\n    # e \"Первая.\"\n    e \"Первая.\"\n",
            "    e \"A line in a file that declares no block at all.\"\n",
        );

        assert_eq!(
            (done.taken, done.kept),
            (0, 1),
            "with no identifier over it there is nothing to match it by, and every such line in \
             the folder would answer for every other"
        );
    }

    #[test]
    fn every_line_of_a_block_is_matched_by_where_it_sits_in_it() {
        let skeleton = concat!(
            "translate patch both_1:\n",
            "    # e \"Первая.\"\n",
            "    # m \"Вторая.\"\n",
            "    e \"Первая.\"\n",
            "    m \"Вторая.\"\n",
        );

        let written = overlaid(
            skeleton,
            "translate english both_1:\n    e \"First.\"\n    m \"Second.\"\n",
        )
        .text
        .expect("both lines were offered");

        assert_eq!(
            sources_of(&written),
            ["First.", "Second."],
            "two lines under one identifier are told apart by their order, or they swap places"
        );
    }

    #[test]
    fn the_first_of_two_blocks_sharing_an_identifier_wins() {
        let done = overlaid(
            SKELETON,
            concat!(
                "translate english cap_arr_13ae5179:\n",
                "    mia \"The door is locked.\"\n",
                "translate english cap_arr_13ae5179:\n",
                "    mia \"A line left over from an older build.\"\n",
            ),
        );

        let written = done.text.expect("the line was offered");
        assert!(sources_of(&written).contains(&"The door is locked.".to_string()));
    }

    #[test]
    fn a_folder_holding_nothing_to_read_is_not_offered_at_all() {
        let sandbox = tempfile::tempdir().expect("a temp folder");
        let game = sandbox.path();

        for (at, body) in [
            ("english/script.rpy", SHIPPED),
            ("spanish/script.rpy", "translate spanish a_1:\n    e \"\"\n"),
            (
                "None/common.rpym",
                "translate None strings:\n    old \"a\"\n    new \"b\"\n",
            ),
        ] {
            let at = folder(game, at);
            fs::create_dir_all(at.parent().expect("a folder")).expect("a folder");
            fs::write(&at, body).expect("a script");
        }

        assert_eq!(
            sources(game, &[]),
            ["english"],
            "an abandoned folder full of empty lines is not a language anyone can translate from, \
             and None is Ren'Py's slot for the game's own words rather than a language of its own"
        );
    }

    #[test]
    fn the_folder_being_written_into_is_never_offered_as_something_to_read() {
        let sandbox = tempfile::tempdir().expect("a temp folder");
        let game = sandbox.path();

        let at = folder(game, "french/script.rpy");
        fs::create_dir_all(at.parent().expect("a folder")).expect("a folder");
        fs::write(&at, SHIPPED).expect("a script");

        assert!(
            sources(game, &["french".to_string()]).is_empty(),
            "reading the export it is about to overwrite would translate the translation"
        );
    }
}
