use crate::engine::renpy::{GAME_DIR, TL_DIR, WORKING, script, scripts, text};
use anyhow::{Context, Result};
use regex::Regex;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::sync::LazyLock;

const FILE: &str = concat!(env!("CARGO_PKG_NAME"), "_names.rpy");

static RE_CHARACTER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r#"(?m)(?:^|[^A-Za-z0-9_])(?:NVL|Bubble)?Character\s*\(\s*(?:name\s*=\s*)?(?:_\s*\(\s*)?"#,
        r#"(?:'(?P<single>(?:[^'\\]|\\.)*)'|"(?P<double>(?:[^"\\]|\\.)*)")"#
    ))
    .expect("RE_CHARACTER is a valid pattern")
});

fn doubled(body: &str, quote: char) -> String {
    let mut out = String::with_capacity(body.len() + 4);
    let mut letters = body.chars().peekable();

    while let Some(letter) = letters.next() {
        match letter {
            '\\' => match letters.next() {
                Some('"') => out.push_str("\\\""),
                Some(next) if next == quote => out.push(next),
                Some(next) => {
                    out.push('\\');
                    out.push(next);
                }
                None => {}
            },
            '"' => out.push_str("\\\""),
            other => out.push(other),
        }
    }

    out
}

fn found(game_dir: &Path) -> Vec<String> {
    let inner = game_dir.join(GAME_DIR);
    let mut seen = HashSet::new();
    let mut out = Vec::new();

    for path in scripts(&inner) {
        if path.starts_with(inner.join(TL_DIR)) {
            continue;
        }

        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };

        for found in RE_CHARACTER.captures_iter(&text) {
            let name = match (found.name("single"), found.name("double")) {
                (Some(quoted), _) => doubled(quoted.as_str(), '\''),
                (_, Some(quoted)) => doubled(quoted.as_str(), '"'),
                _ => continue,
            };

            if text::unmarked(&name).chars().any(char::is_alphabetic) && seen.insert(name.clone()) {
                out.push(name);
            }
        }
    }

    out
}

pub fn add(source: &Path, game_dir: &Path) -> Result<u32> {
    let mut already: HashSet<String> = HashSet::new();
    for path in scripts(source) {
        let text =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;

        already.extend(script::keys(&text));
    }

    let wanted: Vec<String> = found(game_dir)
        .into_iter()
        .filter(|name| !already.contains(name))
        .collect();

    if wanted.is_empty() {
        return Ok(0);
    }

    let mut body = format!("translate {WORKING} strings:\n");
    for name in &wanted {
        body.push_str(&format!("\n    old \"{name}\"\n    new \"{name}\"\n"));
    }

    let at = source.join(FILE);
    fs::write(&at, body).with_context(|| format!("writing {}", at.display()))?;

    Ok(wanted.len() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sandbox(body: &str) -> tempfile::TempDir {
        let at = tempfile::tempdir().expect("a temp folder");
        fs::create_dir_all(at.path().join("game")).expect("a folder");
        fs::write(at.path().join("game").join("characters.rpy"), body).expect("a script");

        at
    }

    #[test]
    fn a_name_the_game_speaks_through_is_found_however_it_was_written() {
        let game = sandbox(concat!(
            "define mc = Character('[playername]', color='#5058c2')\n",
            "default vall = Character('Аня', color='#aaa11b')\n",
            "default ala = Character('Леди [heroine]')\n",
            "define nar = Character(None, kind=nvl)\n",
            "define e = Character(_(\"Eileen\"))\n",
            "define q = Character(\"The \\\"Boss\\\"\")\n",
            "define s = Character('It\\'s her')\n",
            "define el = Character (\"Eliza\", who_color='#ca61ed')\n",
            "define nv = NVLCharacter('Narrator')\n",
            "define bb = BubbleCharacter('Bubbles')\n",
            "Character('Nested inside a line')\n",
            "define k = Character(name=\"Keyed\", color='#101010')\n",
            "define w = Character(\n    \"Wrapped\",\n    color='#202020',\n)\n",
            "define kw = Character(\n    name=_(\"Keyed and wrapped\"),\n)\n",
        ));

        assert_eq!(
            found(game.path()),
            [
                "Аня",
                "Леди [heroine]",
                "Eileen",
                "The \\\"Boss\\\"",
                "It's her",
                "Eliza",
                "Narrator",
                "Bubbles",
                "Nested inside a line",
                "Keyed",
                "Wrapped",
                "Keyed and wrapped"
            ],
            "a name is the words a reader sees over the line, whichever quote the writer reached \
             for and whatever spacing they left, and one that is nothing but a variable holds no \
             word to translate. A name written past the first argument or on a line of its own is \
             the same name: missing it would leave one speaker in the source language while every \
             other one is translated"
        );
    }

    #[test]
    fn a_name_that_is_really_a_variable_to_look_up_is_left_alone() {
        let game = sandbox("define mc = DynamicCharacter(\"name\", color='#ffaf05')\n");

        assert!(
            found(game.path()).is_empty(),
            "this one names the variable to read the name out of, so translating it would leave a \
             translation registered for whatever else the game calls \"name\""
        );
    }

    #[test]
    fn a_name_ren_py_already_laid_out_is_not_written_a_second_time() {
        let game = sandbox("default vall = Character('Аня')\ndefault a = Character('Ольга')\n");
        let source = tempfile::tempdir().expect("a temp folder");

        fs::write(
            source.path().join("screens.rpy"),
            "translate patch strings:\n    old \"Аня\"\n    new \"Аня\"\n",
        )
        .expect("a skeleton");

        assert_eq!(add(source.path(), game.path()).expect("a strings file"), 1);

        let written =
            fs::read_to_string(source.path().join(FILE)).expect("the names it had to add");
        assert!(
            !written.contains("Аня"),
            "Ren'Py registers one translation per string, so naming it twice loses one of them:\n\
             {written}"
        );
        assert!(written.contains("    old \"Ольга\"\n    new \"Ольга\"\n"));
    }

    #[test]
    fn an_untranslated_name_still_reads_as_the_one_the_game_shipped() {
        let game = sandbox("default vall = Character('Аня')\n");
        let source = tempfile::tempdir().expect("a temp folder");

        add(source.path(), game.path()).expect("a strings file");
        let written = fs::read_to_string(source.path().join(FILE)).expect("a strings file");

        assert!(
            written.contains("new \"Аня\""),
            "an answer left empty is an empty name over the line, so the pair has to start out \
             holding the words the game shipped:\n{written}"
        );
        assert_eq!(script::keys(&written), ["Аня"]);
    }

    #[test]
    fn a_translation_written_here_is_the_one_the_game_looks_up() {
        let game = sandbox("default ala = Character('Леди [heroine]')\n");
        let source = tempfile::tempdir().expect("a temp folder");

        add(source.path(), game.path()).expect("a strings file");
        let written = fs::read_to_string(source.path().join(FILE)).expect("a strings file");

        assert!(
            written.contains("old \"Леди [heroine]\""),
            "Ren'Py looks the name up before it fills the brackets in, so the key may not be \
             escaped or the lookup misses:\n{written}"
        );
    }
}
