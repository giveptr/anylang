use anyhow::{Context, Result};
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Seeking {
    pub cased: bool,
    pub whole: bool,
    pub regex: bool,
}

pub fn looking_for(needle: &str, how: Seeking) -> Result<Regex> {
    let body = match how.regex {
        true => needle.to_string(),
        false => regex::escape(needle),
    };
    let body = match how.whole {
        true => format!(r"\b(?:{body})\b"),
        false => body,
    };

    RegexBuilder::new(&body)
        .case_insensitive(!how.cased)
        .size_limit(1 << 20)
        .build()
        .with_context(|| format!("{needle} is not a pattern this can search for"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn holds(needle: &str, how: Seeking, text: &str) -> bool {
        looking_for(needle, how).expect("a pattern").is_match(text)
    }

    #[test]
    fn a_plain_search_ignores_case_and_takes_the_needle_literally() {
        let how = Seeking::default();

        assert!(holds("hello", how, "Hello there"));
        assert!(
            !holds("a.c", how, "abc"),
            "a dot is a dot until the reader asks for a pattern"
        );
        assert!(holds("(1)", how, "slot (1) is free"));
    }

    #[test]
    fn asking_for_case_makes_the_search_exact() {
        let how = Seeking {
            cased: true,
            ..Seeking::default()
        };

        assert!(holds("Hello", how, "Hello there"));
        assert!(!holds("hello", how, "Hello there"));
    }

    #[test]
    fn a_whole_word_search_skips_the_middle_of_a_longer_word() {
        let how = Seeking {
            whole: true,
            ..Seeking::default()
        };

        assert!(holds("art", how, "the art of it"));
        assert!(!holds("art", how, "started"));
    }

    #[test]
    fn a_pattern_search_reads_the_needle_as_one() {
        let how = Seeking {
            regex: true,
            ..Seeking::default()
        };

        assert!(holds(r"\d+ gold", how, "you found 25 gold"));
        assert!(!holds(r"\d+ gold", how, "you found gold"));
    }

    #[test]
    fn a_pattern_that_makes_no_sense_is_refused_by_name() {
        let how = Seeking {
            regex: true,
            ..Seeking::default()
        };

        let why = looking_for("(unclosed", how)
            .expect_err("a broken pattern")
            .to_string();

        assert_eq!(
            why, "(unclosed is not a pattern this can search for",
            "the message has to quote what was typed"
        );
    }

    #[test]
    fn a_word_search_can_still_be_a_pattern() {
        let how = Seeking {
            whole: true,
            regex: true,
            cased: false,
        };

        assert!(holds("go(ld|al)", how, "one goal here"));
        assert!(!holds("go(ld|al)", how, "goalkeeper"));
    }
}
