use crate::engine::rpg_maker::text;
use crate::engine::unicode_escape;
use regex::Regex;
use std::collections::HashSet;
use std::ops::Range;
use std::sync::LazyLock;

static RE_SET_VALUE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"setValue\s*\(\s*(?<cell>[^,()]+?)\s*,\s*(?:"(?<dq>(?:[^"\\]|\\.)*)"|'(?<sq>(?:[^'\\]|\\.)*)')"#,
    )
    .expect("RE_SET_VALUE is a valid pattern")
});

static RE_LITERAL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#""(?<dq>(?:[^"\\]|\\.)*)"|'(?<sq>(?:[^'\\]|\\.)*)'"#)
        .expect("RE_LITERAL is a valid pattern")
});

static RE_IDENTIFIER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[A-Za-z0-9_.\-/\\]+$").expect("RE_IDENTIFIER is a valid pattern")
});

pub struct Stored {
    pub at: Range<usize>,
    pub text: String,
    pub beside: bool,
}

pub fn stored_text(line: &str) -> Vec<Stored> {
    let found: Vec<regex::Captures<'_>> = RE_SET_VALUE.captures_iter(line).collect();
    let row: HashSet<i64> = found.iter().filter_map(cell_of).collect();

    found
        .iter()
        .filter_map(|one| {
            stored(one, |cell| {
                cell.is_some_and(|id| row.contains(&(id - 1)) || row.contains(&(id + 1)))
            })
        })
        .collect()
}

pub fn literals(line: &str) -> Vec<Stored> {
    RE_LITERAL
        .captures_iter(line)
        .filter_map(|found| stored(&found, |_| false))
        .collect()
}

fn cell_of(found: &regex::Captures<'_>) -> Option<i64> {
    found.name("cell")?.as_str().parse().ok()
}

fn stored(found: &regex::Captures<'_>, beside: impl Fn(Option<i64>) -> bool) -> Option<Stored> {
    let body = found.name("dq").or_else(|| found.name("sq"))?;
    let text = js_unescape(body.as_str());

    worth_translating(&text).then(|| Stored {
        at: body.range(),
        text,
        beside: beside(cell_of(found)),
    })
}

fn worth_translating(body: &str) -> bool {
    let trimmed = body.trim();

    if !trimmed.ends_with('.') && RE_IDENTIFIER.is_match(trimmed) {
        return false;
    }

    text::has_words(trimmed)
}

fn js_unescape(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut rest = source;

    while let Some(step) = rest.find('\\') {
        out.push_str(&rest[..step]);
        rest = &rest[step..];

        let (decoded, used) = match rest[1..].chars().next() {
            Some('\\') => (Some('\\'), 2),
            Some('"') => (Some('"'), 2),
            Some('\'') => (Some('\''), 2),
            Some('n') => (Some('\n'), 2),
            Some('r') => (Some('\r'), 2),
            Some('t') => (Some('\t'), 2),
            Some('u') => match unicode_escape(&rest[2..]) {
                Some((character, taken)) => (Some(character), 2 + taken),
                None => (None, 1),
            },
            Some('x') => match hex_byte(&rest[2..]) {
                Some(character) => (Some(character), 4),
                None => (None, 1),
            },
            _ => (None, 1),
        };

        match decoded {
            Some(character) => out.push(character),
            None => out.push('\\'),
        }
        rest = &rest[used..];
    }

    out.push_str(rest);
    out
}

fn hex_byte(source: &str) -> Option<char> {
    let digits = source.get(..2)?;
    if !digits.bytes().all(|digit| digit.is_ascii_hexdigit()) {
        return None;
    }

    char::from_u32(u32::from_str_radix(digits, 16).ok()?)
}

pub fn js_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);

    for character in text.chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\'' => out.push_str("\\'"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stored_sentence_is_found_and_the_variable_id_is_left_alone() {
        let line = r#"$gameVariables.setValue(21, "Open your inventory to look.")"#;
        let found = stored_text(line);

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].text, "Open your inventory to look.");
        assert_eq!(&line[found[0].at.clone()], "Open your inventory to look.");
    }

    #[test]
    fn a_costume_or_file_name_is_not_a_sentence() {
        for name in [
            "MCTowel",
            "EthanShirt",
            "MC",
            "pictures/bg1",
            "$",
            "1.5",
            "1.",
            "bg\\main",
        ] {
            let line = format!(r#"$gameVariables.setValue(3, "{name}")"#);
            assert!(
                stored_text(&line).is_empty(),
                "{name:?} is an identifier, translating it breaks the game"
            );
        }
    }

    #[test]
    fn a_single_quoted_sentence_is_found_too() {
        let line = "$gameVariables.setValue(3, 'Wait here.')";
        let found = stored_text(line);

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].text, "Wait here.");
        assert_eq!(&line[found[0].at.clone()], "Wait here.");
    }

    #[test]
    fn an_apostrophe_is_escaped_so_a_single_quoted_string_stays_whole() {
        let found = stored_text(r"setValue(4, 'It\'s locked.')");

        assert_eq!(found[0].text, "It's locked.");
        assert_eq!(js_escape(&found[0].text), r"It\'s locked.");
    }

    #[test]
    fn a_single_word_farewell_with_an_ellipsis_is_prose_not_an_identifier() {
        let found = stored_text(r#"setValue(5, "Goodbye...")"#);

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].text, "Goodbye...");
    }

    #[test]
    fn a_unicode_escape_becomes_the_character_the_game_prints() {
        let found = stored_text(r#"setValue(8, "See \u30A2 here.")"#);

        assert_eq!(found[0].text, "See ア here.");
        assert_eq!(js_escape(&found[0].text), "See ア here.");
    }

    #[test]
    fn a_surrogate_pair_is_joined_and_a_broken_escape_is_left_alone() {
        assert_eq!(js_unescape(r"\uD83D\uDE00!"), "😀!");
        assert_eq!(js_unescape(r"\uD83D alone"), r"\uD83D alone");
        assert_eq!(js_unescape(r"\u30A"), r"\u30A");
        assert_eq!(js_unescape(r"\xE9 ok"), "é ok");
        assert_eq!(js_unescape(r"\xZZ"), r"\xZZ");
    }

    #[test]
    fn text_that_is_not_written_in_spaces_is_still_text() {
        let line = r#"$gameVariables.setValue(7, "扉はかたく閉ざされている。")"#;

        assert_eq!(
            stored_text(line).len(),
            1,
            "a Japanese source has no spaces between words"
        );
    }

    #[test]
    fn a_quote_inside_the_sentence_survives_the_round_trip() {
        let line = r#"$gameVariables.setValue(4, "He said \"hello\" once.")"#;
        let found = stored_text(line);

        assert_eq!(found[0].text, r#"He said "hello" once."#);
        assert_eq!(
            js_escape(&found[0].text),
            r#"He said \"hello\" once."#,
            "an unescaped quote would end the string early and break the script"
        );
    }

    #[test]
    fn a_backslash_control_code_keeps_its_doubled_backslash() {
        let line = r#"$gameVariables.setValue(9, "You have \\v[12] gold left.")"#;
        let found = stored_text(line);

        assert_eq!(found[0].text, r"You have \v[12] gold left.");
        assert_eq!(js_escape(&found[0].text), r"You have \\v[12] gold left.");
    }

    #[test]
    fn every_call_on_one_line_is_found() {
        let line = r#"$gameVariables.setValue(1, "First one."); $gameVariables.setValue(2, "Second one.");"#;
        let found = stored_text(line);

        assert_eq!(found.len(), 2);
        assert_eq!(found[0].text, "First one.");
        assert_eq!(found[1].text, "Second one.");
    }

    #[test]
    fn a_value_that_is_only_a_control_code_holds_no_words_to_translate() {
        for code in [r"\\v[45]", r"\\c[14]\\v[51]"] {
            let line = format!(r#"setValue(3, "{code}")"#);
            assert!(stored_text(&line).is_empty(), "{code} has nothing to say");
        }
    }

    #[test]
    fn a_script_that_stores_nothing_offers_nothing() {
        for line in [
            "$gameVariables.value(21)",
            "AudioManager.playSe({name: \"Door_Open\", volume: 90})",
            "if ($gameSwitches.value(3)) { doThing(); }",
        ] {
            assert!(stored_text(line).is_empty(), "{line}");
        }
    }

    #[test]
    fn a_line_break_inside_the_sentence_is_kept_as_an_escape() {
        let found = stored_text(r#"setValue(2, "Line one.\nLine two.")"#);

        assert_eq!(found[0].text, "Line one.\nLine two.");
        assert_eq!(js_escape(&found[0].text), r"Line one.\nLine two.");
    }
}
