use crate::canvas;
use regex::Regex;
use std::collections::HashMap;
use std::path::Path;

pub fn unicode_escape(source: &str) -> Option<(char, usize)> {
    let unit = code_unit(source)?;

    if let Some(character) = char::from_u32(unit) {
        return Some((character, 4));
    }

    let low = code_unit(source.get(4..)?.strip_prefix("\\u")?)?;
    if !(0xD800..0xDC00).contains(&unit) || !(0xDC00..0xE000).contains(&low) {
        return None;
    }

    let joined = 0x10000 + ((unit - 0xD800) << 10) + (low - 0xDC00);
    char::from_u32(joined).map(|character| (character, 10))
}

fn code_unit(source: &str) -> Option<u32> {
    let digits = source.get(..4)?;
    if !digits.bytes().all(|digit| digit.is_ascii_hexdigit()) {
        return None;
    }

    u32::from_str_radix(digits, 16).ok()
}

pub fn hand_written(text: &str) -> bool {
    text.chars()
        .any(|one| one.is_alphabetic() && !one.is_ascii())
}

pub fn quoted(said: &str) -> String {
    format!("\"{}\"", said.replace('\\', "\\\\").replace('"', "\\\""))
}

pub fn marks<'t>(pattern: &Regex, text: &'t str) -> HashMap<&'t str, usize> {
    let mut counted = HashMap::new();
    for found in pattern.find_iter(text) {
        *counted.entry(found.as_str()).or_insert(0) += 1;
    }

    counted
}

pub fn only_in(these: &HashMap<&str, usize>, others: &HashMap<&str, usize>) -> String {
    let mut alone: Vec<&str> = these
        .iter()
        .filter(|(mark, count)| others.get(*mark).unwrap_or(&0) < count)
        .map(|(mark, _)| *mark)
        .collect();
    alone.sort_unstable();

    alone.join(" ")
}

const MARKS: [char; 15] = [
    ':', '=', '#', '|', '_', '{', '}', '<', '>', '$', '@', '^', ';', '[', ']',
];

const ASSET: [&str; 27] = [
    "ogg", "wav", "mp3", "m4a", "mid", "midi", "avi", "mp4", "webm", "psd", "json", "xml", "csv",
    "txt", "dat", "sav", "mps", "project", "wolf", "wolfx", "prefab", "asset", "anim", "mat",
    "fbx", "ttf", "otf",
];

pub fn names_a_file(text: &str) -> bool {
    let bare = text.trim();
    if bare.contains('\n') {
        return false;
    }

    let Some((stem, kind)) = bare.rsplit_once('.') else {
        return false;
    };

    !stem.is_empty()
        && (canvas::is_picture(Path::new(bare))
            || ASSET.iter().any(|known| kind.eq_ignore_ascii_case(known)))
}

fn wrapped(text: &str) -> bool {
    let Some(first) = text.chars().next() else {
        return false;
    };

    if first == '.' || first == '_' || first.is_alphanumeric() {
        return false;
    }

    let lead = text.chars().take_while(|one| *one == first).count();
    let tail = text.chars().rev().take_while(|one| *one == first).count();

    lead >= 2 && tail >= 2 && text.chars().count() > lead + tail
}

pub fn symbolic(bare: &str) -> bool {
    let text = bare.trim();

    if !text.chars().any(char::is_alphabetic) {
        return true;
    }
    if text.chars().any(char::is_whitespace) {
        return false;
    }
    if names_a_file(text) {
        return true;
    }
    if text
        .chars()
        .any(|one| one.is_alphabetic() && !one.is_ascii())
    {
        return false;
    }
    if text.chars().count() == 1 {
        return true;
    }
    if humped(text) {
        return true;
    }

    let core = text.trim_end_matches(|one: char| !one.is_ascii_alphanumeric());
    if core.chars().any(|one| MARKS.contains(&one)) {
        return true;
    }

    wrapped(text)
}

pub fn has_words(pattern: &Regex, text: &str) -> bool {
    let mut at = 0;

    for found in pattern.find_iter(text) {
        if text[at..found.start()].chars().any(char::is_alphabetic) {
            return true;
        }
        at = found.end();
    }

    text[at..].chars().any(char::is_alphabetic)
}

pub fn humped(text: &str) -> bool {
    text.chars()
        .zip(text.chars().skip(1))
        .any(|(before, after)| before.is_ascii_lowercase() && after.is_ascii_uppercase())
}

pub fn worth(said: &str) -> Option<&str> {
    Some(said.trim()).filter(|said| !said.is_empty())
}

pub fn filled(translation: &str) -> Result<(), String> {
    if translation.trim().is_empty() {
        return Err("translation is empty".to_string());
    }

    Ok(())
}

pub fn same_marks(
    kind: &str,
    pattern: &Regex,
    source: &str,
    translation: &str,
) -> Result<(), String> {
    let before = marks(pattern, source);
    let after = marks(pattern, translation);

    let missing = only_in(&before, &after);
    let extra = only_in(&after, &before);

    if missing.is_empty() && extra.is_empty() {
        return Ok(());
    }

    let mut said = Vec::new();
    if !missing.is_empty() {
        said.push(format!("missing {missing}"));
    }
    if !extra.is_empty() {
        said.push(format!("unexpected {extra}"));
    }

    Err(format!("{kind}: {}", said.join(", ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_a_game_ships_is_told_from_a_label_a_player_reads() {
        for named in [
            "\u{30ad}\u{30e3}\u{30e9}.png",
            "img/\u{30ad}\u{30e3}\u{30e9}.png",
        ] {
            assert!(
                symbolic(named),
                "{named} is a file this game ships, and a game names its own files in its own \
                 alphabet: reading the letters before the ending is what sent an asset path to \
                 the model and broke the game it named"
            );
        }

        assert!(
            !symbolic("\u{30aa}\u{30f3}/\u{30aa}\u{30d5}"),
            "On/Off is a label a player reads, and a slash alone cannot tell it from a folder: \
             only the game's own files can, so the slash may not be read as a path here"
        );
        assert!(
            !symbolic("\u{7b2c}\\cself[65]\u{7ae0}"),
            "a backslash opens a control code in RPG Maker and Wolf, so it may not be read as a \
             path separator: the line around it is talk"
        );
        assert!(
            symbolic("---Drops---"),
            "a wrapped run is how a menu marks a heading rather than how a person writes one"
        );
        assert!(
            !symbolic("\u{3053}\u{3046}\u{3052}\u{304d}"),
            "a bare word in the language the game was written in is a word, whatever else it \
             might also be"
        );
    }
}
