use crate::engine::{filled, marks, only_in, same_marks};
use regex::Regex;
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::LazyLock;

pub const LITERAL: &str = r#"(?P<quoted>'(?:[^'\\\n]|\\.)*'|"(?:[^"\\\n]|\\.)*")"#;

const FORMAT_SPEC: &str = r"%(?:\([^)]*\)[-+ #0]*|[-+#0]*)\d*(?:\.\d+)?[hlL]?[diouxXeEfFgGcrsa]";
const VARIABLE: &str = r"\[(?:[^\[\]]|\[[^\[\]]*\])+\]";
const TAG: &str = r"\{[^{}]*\}";

static RE_FORMAT_SPEC: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!("%%|{FORMAT_SPEC}")).expect("RE_FORMAT_SPEC is a valid pattern")
});

pub const MARKUP_RULES: &str = r#"- variables in square brackets, e.g. [player_name], [renpy.display.tts.last]
- text tags in curly braces, e.g. {i} {/i} {b} {w} {size=40} {color=#00ff00} {#note}
- escape sequences: \n \" \' \\
- printf placeholders: %s %d %(name)s"#;

pub const RETRY_RULES: &str = "Every [variable] and every % placeholder a source string carries \
                               has to appear in your translation of that string, spelled exactly \
                               the same, with none added. Text tags in curly braces may be left \
                               out, but never split a pair the source carries, and never add a \
                               tag the source does not have: a tag the source leaves unpaired \
                               stays unpaired.";

static RE_VARIABLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(VARIABLE).expect("RE_VARIABLE is a valid pattern"));
static RE_MARKS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!("{TAG}|{VARIABLE}|{FORMAT_SPEC}")).expect("RE_MARKS is a valid pattern")
});

static RE_ASIDE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\{#[^{}]*\}").expect("RE_ASIDE is a valid pattern"));
static RE_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(TAG).expect("RE_TAG is a valid pattern"));
static RE_FORMAT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(FORMAT_SPEC).expect("RE_FORMAT is a valid pattern"));

pub fn validate(source: &str, translation: &str) -> Result<(), String> {
    filled(translation)?;

    slots(source, translation)?;

    let allowed = marks(&RE_TAG, source);
    let used = marks(&RE_TAG, translation);
    let invented = only_in(&used, &allowed);
    if !invented.is_empty() {
        return Err(format!("text tags: unexpected {invented}"));
    }

    for (mark, opened) in &used {
        let Some(closer) = closer_of(mark) else {
            continue;
        };

        if allowed.contains_key(closer.as_str()) && used.get(closer.as_str()).unwrap_or(&0) < opened
        {
            return Err(format!("text tags: {mark} needs a closing {closer}"));
        }
    }

    for mark in used.keys() {
        let Some(name) = mark.strip_prefix("{/").and_then(|it| it.strip_suffix('}')) else {
            continue;
        };

        if unmatched(&used, mark) > unmatched(&allowed, mark) {
            return Err(format!("text tags: {mark} needs an opening {{{name}}}"));
        }
    }

    Ok(())
}

fn unmatched(seen: &HashMap<&str, usize>, closer: &str) -> usize {
    let closed = seen.get(closer).copied().unwrap_or(0);
    let opened = seen
        .iter()
        .filter(|(mark, _)| closer_of(mark).is_some_and(|it| it == closer))
        .map(|(_, count)| count)
        .sum::<usize>();

    closed.saturating_sub(opened)
}

fn closer_of(mark: &str) -> Option<String> {
    let inside = mark.trim_start_matches('{').trim_end_matches('}');
    if inside.starts_with('/') || inside.is_empty() {
        return None;
    }

    let name = inside.split('=').next().unwrap_or(inside);

    Some(format!("{{/{name}}}"))
}

fn slots(source: &str, translation: &str) -> Result<(), String> {
    same_marks("variables", &RE_VARIABLE, source, translation)?;
    same_marks("format specifiers", &RE_FORMAT, source, translation)
}

pub fn same_slots(was: &str, instead: &str) -> bool {
    slots(was, instead).is_ok()
}

pub fn unmarked(text: &str) -> Cow<'_, str> {
    RE_MARKS.replace_all(text, "")
}

pub fn spoken(text: &str) -> Cow<'_, str> {
    RE_ASIDE.replace_all(text, "")
}

pub fn requoted(literal: &str) -> String {
    let mut letters = literal.chars();

    let Some(quote) = letters.next().filter(|held| ['"', '\''].contains(held)) else {
        return literal.to_string();
    };

    let body = letters.as_str();
    let mut letters = body.strip_suffix(quote).unwrap_or(body).chars();
    let mut out = String::with_capacity(body.len() + 4);

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

pub fn escape(text: &str) -> String {
    const KNOWN_ESCAPES: &[char] = &['n', '"', '\'', '\\', ' '];

    let spans: HashMap<usize, usize> = RE_FORMAT_SPEC
        .find_iter(text)
        .chain(RE_VARIABLE.find_iter(text))
        .chain(RE_TAG.find_iter(text))
        .map(|found| (found.start(), found.end()))
        .collect();

    let mut out = String::with_capacity(text.len() + 8);
    let mut characters = text.char_indices().peekable();

    while let Some((index, character)) = characters.next() {
        match character {
            '\\' => match characters.peek() {
                Some((_, next)) if KNOWN_ESCAPES.contains(next) => {
                    out.push('\\');
                    out.push(*next);
                    characters.next();
                }
                Some(_) => out.push_str("\\\\"),
                None => {}
            },
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            '%' | '[' | '{' => match spans.get(&index) {
                Some(&end) => {
                    let mut slashed = false;
                    for letter in text[index..end].chars() {
                        match letter {
                            '\\' => {
                                out.push('\\');
                                slashed = !slashed;
                            }
                            '"' => {
                                if !slashed {
                                    out.push('\\');
                                }
                                out.push('"');
                                slashed = false;
                            }
                            _ => {
                                out.push(letter);
                                slashed = false;
                            }
                        }
                    }
                    while characters.peek().is_some_and(|(next, _)| *next < end) {
                        characters.next();
                    }
                }
                None => {
                    out.push(character);
                    out.push(character);
                    if characters
                        .peek()
                        .is_some_and(|(_, next)| *next == character)
                    {
                        characters.next();
                    }
                }
            },
            other => out.push(other),
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rule_written_as_prose_reaches_the_model_as_one_line() {
        assert!(
            !RETRY_RULES.contains('\n'),
            "retry rules are one paragraph, so a break in them is only this file's wrapping and \
             the model reads it as a break the author never meant"
        );

        for line in MARKUP_RULES.lines() {
            assert!(
                line.starts_with("- "),
                "every line of a rule list is its own bullet, so a wrapped one reads as a rule of \
                 its own: {line:?}"
            );
        }
    }

    #[test]
    fn escaping_is_added_only_where_ren_py_would_misread_the_translation() {
        assert_eq!(escape(r#"He said \"hi\""#), r#"He said \"hi\""#);
        assert_eq!(escape(r#"He said "hi""#), r#"He said \"hi\""#);
        assert_eq!(escape("line 1\nline 2"), r"line 1\nline 2");
        assert_eq!(escape(r"keep \n intact"), r"keep \n intact");
        assert_eq!(escape(r"stray \x backslash"), r"stray \\x backslash");
        assert_eq!(
            escape("Press [[ to open"),
            "Press [[ to open",
            "one the writer already doubled is left alone: doubling it again prints two"
        );
        assert_eq!(escape("Use {{ here"), "Use {{ here");
        assert_eq!(escape(r"hard\ space"), r"hard\ space");
        assert_eq!(escape("a real\ttab"), "a real\ttab");
        assert_eq!(escape(r"trailing \"), "trailing ");
    }

    #[test]
    fn a_stray_percent_is_escaped_so_the_game_does_not_read_it_as_a_slot() {
        assert_eq!(escape("sure 100%"), "sure 100%%");
        assert_eq!(escape("50% off today"), "50%% off today");
        assert_eq!(escape("bare 100%."), "bare 100%%.");
        assert_eq!(escape("already 100%%"), "already 100%%");
        assert_eq!(escape("got %d coins"), "got %d coins");
        assert_eq!(escape("got %-5d coins"), "got %-5d coins");
        assert_eq!(escape("%+d HP"), "%+d HP");
        assert_eq!(escape("at %#x"), "at %#x");
        assert_eq!(escape("hp %(hp)5.1f left"), "hp %(hp)5.1f left");
        assert_eq!(escape("hi %(name)s there"), "hi %(name)s there");
        assert_eq!(escape("%(count)d items"), "%(count)d items");
    }

    #[test]
    fn a_stray_bracket_is_escaped_so_the_game_does_not_read_it_as_markup() {
        assert_eq!(escape("lone [ bracket"), "lone [[ bracket");
        assert_eq!(escape("lone { brace"), "lone {{ brace");
        assert_eq!(escape("[name] stays whole"), "[name] stays whole");
        assert_eq!(escape("{i}kept{/i}"), "{i}kept{/i}");
    }

    #[test]
    fn a_translation_whose_marks_do_not_match_the_source_is_refused() {
        assert!(validate("Hi [name]", "Hello [name]").is_ok());
        assert!(validate("Hi [name]", "Hello there").is_err());
        assert!(validate("Hi [name]", "Hello [nom]").is_err());
        assert!(validate("{color=#00ff00}Hi{/color}", "{color=#00ff00}Hey{/color}").is_ok());
        assert!(validate("Hi", "{b}Hey{/b}").is_err());
        assert!(validate("Got %d coins", "Won %d coins").is_ok());
        assert!(validate("Got %d coins", "Won coins").is_err());
        assert!(validate("Load %(count)03d", "Load").is_err());
        assert!(validate("{i}Hi{/i}", "{i}Hey").is_err());
        assert!(validate("{i}Hi{/i}", "Hey{/i}").is_err());
        assert!(validate("{i}Hi{/i}", "Hey").is_ok());
        assert!(validate("Careful{/i}", "Watch out{/i}").is_ok());
        assert!(validate("{i}Careful", "{i}Watch out").is_ok());
        assert!(validate("{i}Careful", "Watch out").is_ok());
        assert!(
            validate("{i}Careful", "{i}Watch out{/i}").is_err(),
            "the game leaves this tag unclosed on purpose, so closing it writes markup the \
             script never carried"
        );
        assert!(validate("Got %-5d pts", "Won %-5d pts").is_ok());
        assert!(validate("Got %-5d pts", "Won pts").is_err());
        assert!(validate("Hi", "   ").is_err());
    }

    #[test]
    fn every_placeholder_the_rules_name_is_a_mark_and_not_text() {
        assert_eq!(unmarked("hp %(hp)s left"), "hp  left");
        assert_eq!(unmarked("%(hp)s"), "");
        assert_eq!(unmarked("{i}[name]{/i} %d"), " ");
    }

    #[test]
    fn a_variable_with_dots_in_it_is_still_the_same_variable() {
        let source = r#"Self-voicing would say \"[renpy.display.tts.last]\"."#;
        let translation = r#"The reader would say \"[renpy.display.tts.last]\"."#;
        assert!(validate(source, translation).is_ok());
    }

    #[test]
    fn a_variable_indexing_into_a_list_is_one_variable() {
        let said = "You have [inventory[0].name] left";

        assert!(
            validate(
                said,
                "[inventory[0].name] \u{306f}\u{6b8b}\u{3063}\u{3066}\u{3044}\u{308b}"
            )
            .is_ok()
        );
        assert!(
            validate(said, "[0] \u{306f}\u{6b8b}\u{3063}\u{3066}\u{3044}\u{308b}").is_err(),
            "the outer variable is the one the game substitutes, so a translation keeping only \
             the inner index loses the name"
        );
        assert_eq!(
            escape(said),
            said,
            "doubling the outer bracket would print a literal bracket and interpolate a bare 0"
        );
    }

    #[test]
    fn a_quote_a_span_already_escapes_is_not_escaped_a_second_time() {
        assert_eq!(escape(r#"{a="b"}x"#), r#"{a=\"b\"}x"#);
        assert_eq!(
            escape(r#"{a=\"b\"}x"#),
            r#"{a=\"b\"}x"#,
            "a tag the script already escaped has to come back the same; doubling the backslash \
             closes the string early and the whole file stops compiling"
        );
    }
}
