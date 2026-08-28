use crate::engine::renpy::{GAME_DIR, TL_DIR, scripts, table, text};
use anyhow::Result;
use regex::Regex;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

const FILE: &str = concat!(env!("CARGO_PKG_NAME"), "_parameterized.rpy");

const BUILT_IN: [&str; 2] = ["text", "vtext"];

static RE_DECLARED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?m)^[\t ]*image[\t ]+(?P<name>[A-Za-z_][A-Za-z0-9_ ]*?)",
        r"[\t ]*=[\t ]*(?:renpy\.)?ParameterizedText[\t ]*\("
    ))
    .expect("RE_DECLARED is a valid pattern")
});

static RE_SHOWN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?m)^[\t ]*(?:show|scene)[\t ]+(?P<name>[A-Za-z_][A-Za-z0-9_ ]*?)[\t ]+{}",
        text::LITERAL
    ))
    .expect("RE_SHOWN is a valid pattern")
});

fn spaced(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn found(game_dir: &Path) -> Vec<String> {
    let inner = game_dir.join(GAME_DIR);
    let beside = inner.join(TL_DIR);
    let held: Vec<PathBuf> = scripts(&inner)
        .into_iter()
        .filter(|path| !path.starts_with(&beside))
        .collect();

    let mut text_images: HashSet<String> = BUILT_IN.iter().map(|one| one.to_string()).collect();
    for path in &held {
        let Ok(body) = fs::read_to_string(path) else {
            continue;
        };

        for image in RE_DECLARED.captures_iter(&body) {
            text_images.insert(spaced(&image["name"]));
        }
    }

    let mut seen = HashSet::new();
    let mut out = Vec::new();

    for path in &held {
        let Ok(body) = fs::read_to_string(path) else {
            continue;
        };

        for shown in RE_SHOWN.captures_iter(&body) {
            if !text_images.contains(&spaced(&shown["name"])) {
                continue;
            }

            let said = text::requoted(&shown["quoted"]);

            if text::unmarked(&said).chars().any(char::is_alphabetic) && seen.insert(said.clone()) {
                out.push(said);
            }
        }
    }

    out
}

pub fn add(source: &Path, game_dir: &Path) -> Result<u32> {
    table::added(source, FILE, found(game_dir))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::renpy::script;

    fn sandbox(each: &[(&str, &str)]) -> tempfile::TempDir {
        let at = tempfile::tempdir().expect("a temp folder");

        for (name, body) in each {
            let landing = at.path().join("game").join(name);
            fs::create_dir_all(landing.parent().expect("a folder")).expect("a folder");
            fs::write(landing, body).expect("a script");
        }

        at
    }

    #[test]
    fn a_line_drawn_through_a_text_image_is_found_and_a_picture_path_is_not() {
        let game = sandbox(&[
            (
                "chapters.rpy",
                concat!(
                    "image top_text = ParameterizedText(xalign=0.85, size=45)\n",
                    "image chapter  title = renpy.ParameterizedText(size=80)\n",
                    "    show top_text \"運命の日\"\n",
                    "    show chapter title \"第一章\"\n",
                    "    show text 'クリックして続ける'\n",
                    "    show top_text \"運命の日\"\n",
                ),
            ),
            (
                "ending.rpy",
                concat!(
                    "    show top_text \"エピローグ\"\n",
                    "    show side_art \"art/frame.png\"\n",
                    "    show expression \"#000\"\n",
                    "    scene bg room\n",
                    "    show top_text \"[hero]\"\n",
                    "    show top_text \"\"\n",
                ),
            ),
            ("tl/english/chapters.rpy", "    show text \"leaked\"\n"),
        ]);

        assert_eq!(
            found(game.path()),
            ["運命の日", "第一章", "クリックして続ける", "エピローグ"],
            "a string handed to a ParameterizedText image is words on the screen wherever the \
             image was declared, and Ren'Py registers one translation per string. A string shown \
             under any other name is a picture to load, an expression is whatever it evaluates \
             to, a bare variable holds no words, and a shipped translation folder is somebody \
             else's answer sheet: translating any of those breaks the game or wastes the asking"
        );
    }

    #[test]
    fn a_line_ren_py_already_laid_out_is_not_written_a_second_time() {
        let game = sandbox(&[(
            "chapters.rpy",
            concat!(
                "image top_text = ParameterizedText(size=45)\n",
                "    show top_text \"運命の日\"\n",
                "    show top_text \"第一章\"\n",
            ),
        )]);
        let source = tempfile::tempdir().expect("a temp folder");

        fs::write(
            source.path().join("screens.rpy"),
            "translate patch strings:\n    old \"第一章\"\n    new \"第一章\"\n",
        )
        .expect("a skeleton");

        assert_eq!(add(source.path(), game.path()).expect("a strings file"), 1);

        let written =
            fs::read_to_string(source.path().join(FILE)).expect("the lines it had to add");
        assert!(
            !written.contains("第一章"),
            "Ren'Py refuses to load a game that registers one string twice:\n{written}"
        );
        assert!(written.contains("    old \"運命の日\"\n    new \"運命の日\"\n"));
    }

    #[test]
    fn an_untranslated_line_still_reads_as_the_one_the_game_shipped() {
        let game = sandbox(&[(
            "chapters.rpy",
            concat!(
                "image top_text = ParameterizedText(size=45)\n",
                "    show top_text \"ようこそ [hero]\"\n",
                "    show text 'It\\'s time'\n",
                "    show top_text \"T H E   E N D\"\n",
            ),
        )]);
        let source = tempfile::tempdir().expect("a temp folder");

        add(source.path(), game.path()).expect("a strings file");
        let written = fs::read_to_string(source.path().join(FILE)).expect("a strings file");

        assert_eq!(
            script::keys(&written),
            ["ようこそ [hero]", "It's time", "T H E   E N D"],
            "Ren'Py looks the line up before it fills the brackets in, so the key has to be the \
             very string the show statement hands over, requoted but never escaped further. Its \
             own writer escapes a backslash, a quote and the control characters and nothing else, \
             and it reads an old line back through Python's eval, where a backslash before a \
             space is no escape at all: holding a run of spaces open would leave a backslash \
             sitting inside the key, and the key would no longer be the line:\n{written}"
        );
        assert!(
            written.contains("new \"ようこそ [hero]\""),
            "an answer left empty is an empty screen, so the pair starts out holding the words \
             the game shipped:\n{written}"
        );
    }

    #[test]
    fn a_game_that_draws_no_text_images_adds_nothing() {
        let game = sandbox(&[("script.rpy", "    show bg room\n    e \"こんにちは\"\n")]);
        let source = tempfile::tempdir().expect("a temp folder");

        assert_eq!(add(source.path(), game.path()).expect("no strings"), 0);
        assert!(
            !source.path().join(FILE).exists(),
            "a file holding an empty strings block is a parse error Ren'Py refuses to load"
        );
    }
}
