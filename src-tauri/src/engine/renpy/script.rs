use crate::engine::renpy::text;
use crate::engine::renpy::text::escape;
use crate::engine::{Applied, Parsed, TranslationUnit};
use anyhow::Result;
use regex::{Captures, Regex};
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::LazyLock;

const SPEAKER: &str = r#"(?:"(?:[^"\\]|\\.)*"[\t ]+)?(?:(?:-?[A-Za-z_][A-Za-z0-9_]*|@)[\t ]+)*"#;

static RE_SAY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r#"^(?P<indent>[\t ]+)(?P<prefix>{SPEAKER})"(?P<text>(?:[^"\\]|\\.)*)"(?P<suffix>[^"]*)$"#
    ))
    .expect("RE_SAY is a valid pattern")
});

static RE_COMMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r#"^[\t ]*#[\t ]*{SPEAKER}"(?P<text>(?:[^"\\]|\\.)*)"[^"]*$"#
    ))
    .expect("RE_COMMENT is a valid pattern")
});

static RE_TRANSLATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^(?P<lead>[\t ]*translate[\t ]+)[A-Za-z_][A-Za-z0-9_]*")
        .expect("RE_TRANSLATE is a valid pattern")
});

static RE_BLOCK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^[\t ]*translate[\t ]+[A-Za-z_][A-Za-z0-9_]*[\t ]+(?P<block>[A-Za-z_][A-Za-z0-9_]*)[\t ]*:",
    )
    .expect("RE_BLOCK is a valid pattern")
});

pub struct Script {
    source: SourceText,
    extraction: Extraction,
}

pub fn parse(text: &str) -> Script {
    let source = SourceText::parse(text);
    let extraction = scan(&source.lines);

    Script { source, extraction }
}

impl Parsed for Script {
    fn units(&self) -> &[TranslationUnit] {
        &self.extraction.units
    }

    fn render(self: Box<Self>, translations: &BTreeMap<u32, String>) -> Result<(String, Applied)> {
        let mut this = *self;
        let applied = write_in(&mut this.source.lines, &this.extraction, translations);

        Ok((this.source.render(), applied))
    }
}

pub fn retarget<'t>(text: &'t str, language: &str) -> Cow<'t, str> {
    RE_TRANSLATE.replace_all(text, |found: &Captures| {
        format!("{}{language}", &found["lead"])
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Spot {
    Line { block: String, at: u32 },
    Key { text: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ParsedLine {
    SourceComment {
        text: String,
        from: usize,
        to: usize,
    },
    Old {
        text: String,
    },
    Say {
        prefix: String,
        text: String,
        suffix: String,
    },
    Aside,
    Other,
}

const NOT_DIALOGUE: [&str; 8] = [
    "voice", "play", "queue", "stop", "show", "scene", "window", "nvl",
];

fn parse_line(line: &str) -> ParsedLine {
    if line.trim_start().starts_with('#') {
        return RE_COMMENT
            .captures(line)
            .and_then(|found| {
                let words = found.name("text")?;

                Some(ParsedLine::SourceComment {
                    text: words.as_str().to_string(),
                    from: words.start(),
                    to: words.end(),
                })
            })
            .unwrap_or(ParsedLine::Other);
    }

    let Some(captures) = RE_SAY.captures(line) else {
        return ParsedLine::Other;
    };

    let text = captures["text"].to_string();
    let speaker = captures["prefix"].split_whitespace().next();

    if speaker == Some("old") {
        return ParsedLine::Old { text };
    }

    if speaker.is_some_and(|word| NOT_DIALOGUE.contains(&word)) {
        return ParsedLine::Aside;
    }

    ParsedLine::Say {
        prefix: format!("{}{}", &captures["indent"], &captures["prefix"]),
        text,
        suffix: captures["suffix"].to_string(),
    }
}

#[derive(Debug, Clone)]
struct Origin {
    line: usize,
    text: String,
    span: Option<(usize, usize)>,
    key: Option<String>,
}

#[derive(Debug, Clone)]
struct Site {
    line: usize,
    unit: u32,
    prefix: String,
    suffix: String,
    said: String,
    spot: Spot,
    from: Origin,
}

#[derive(Debug, Clone, Default)]
struct Extraction {
    units: Vec<TranslationUnit>,
    sites: Vec<Site>,
}

fn scan(lines: &[String]) -> Extraction {
    let mut extraction = Extraction::default();
    let mut seen: HashMap<String, u32> = HashMap::new();
    let mut sources: VecDeque<Origin> = VecDeque::new();
    let mut block = String::new();
    let mut ordinal = 0;

    for (index, line) in lines.iter().enumerate() {
        match parse_line(line) {
            ParsedLine::SourceComment { text, from, to } => {
                sources.push_back(Origin {
                    line: index,
                    text,
                    span: Some((from, to)),
                    key: None,
                });
            }
            ParsedLine::Old { text } => {
                let above = sources
                    .back()
                    .is_some_and(|last| last.span.is_some() && last.line + 1 == index)
                    .then(|| sources.pop_back())
                    .flatten();

                let mut from = above.unwrap_or_else(|| Origin {
                    line: index,
                    text: text.clone(),
                    span: None,
                    key: None,
                });
                from.key = Some(text);

                sources.clear();
                sources.push_back(from);
            }
            ParsedLine::Aside => {
                sources.pop_front();
            }
            ParsedLine::Other => {
                if let Some(found) = RE_BLOCK.captures(line) {
                    block = found["block"].to_string();
                    ordinal = 0;
                } else if !line.is_empty()
                    && !line.starts_with([' ', '\t'])
                    && !line.starts_with('#')
                {
                    block.clear();
                }
                if !line.starts_with([' ', '\t']) {
                    sources.clear();
                }
            }
            ParsedLine::Say {
                prefix,
                text,
                suffix,
            } => {
                let at = ordinal;
                ordinal += 1;

                let from = sources.pop_front().unwrap_or(Origin {
                    line: index,
                    text: text.clone(),
                    span: None,
                    key: None,
                });

                if !is_translatable(&from.text) {
                    continue;
                }

                let spot = match &from.key {
                    Some(key) => Spot::Key { text: key.clone() },
                    None => Spot::Line {
                        block: block.clone(),
                        at,
                    },
                };

                let source = from.text.clone();
                let unit = *seen.entry(source.clone()).or_insert_with(|| {
                    let id = extraction.units.len() as u32;
                    extraction.units.push(TranslationUnit {
                        id,
                        text: source,
                        offer: Default::default(),
                    });
                    id
                });

                extraction.sites.push(Site {
                    line: index,
                    unit,
                    prefix,
                    suffix,
                    said: text,
                    spot,
                    from,
                });
            }
        }
    }

    extraction
}

fn is_translatable(text: &str) -> bool {
    !text::spoken(text).trim().is_empty()
}

fn write_in(
    lines: &mut [String],
    extraction: &Extraction,
    translations: &BTreeMap<u32, String>,
) -> Applied {
    let mut applied = Applied::default();

    for site in &extraction.sites {
        let said = match translations.get(&site.unit) {
            Some(translation) => {
                applied.lines += 1;

                escape(translation)
            }
            None => site.from.text.clone(),
        };

        lines[site.line] = format!("{}\"{said}\"{}", site.prefix, site.suffix);
    }

    applied
}

pub fn keys(text: &str) -> Vec<String> {
    let source = SourceText::parse(text);

    scan(&source.lines)
        .sites
        .into_iter()
        .filter_map(|site| match site.spot {
            Spot::Key { text } => Some(text),
            Spot::Line { .. } => None,
        })
        .collect()
}

fn worded(site: &Site) -> bool {
    !site.said.trim().is_empty()
        && match &site.spot {
            Spot::Key { text } => site.said.trim() != text.trim(),
            Spot::Line { block, .. } => !block.is_empty(),
        }
}

pub fn harvest(text: &str) -> Vec<(Spot, String)> {
    let source = SourceText::parse(text);

    scan(&source.lines)
        .sites
        .into_iter()
        .filter(worded)
        .map(|site| (site.spot, site.said))
        .collect()
}

pub fn offers(text: &str) -> bool {
    let source = SourceText::parse(text);

    scan(&source.lines).sites.iter().any(worded)
}

#[derive(Debug, Default)]
pub struct Overlaid {
    pub text: Option<String>,
    pub taken: u32,
    pub kept: u32,
}

enum Edit {
    Wrote(String),
    Added(String),
}

pub fn overlay(text: &str, offer: &dyn Fn(&Spot, &str) -> Option<String>) -> Overlaid {
    let source = SourceText::parse(text);
    let extraction = scan(&source.lines);

    let mut edits: BTreeMap<usize, Edit> = BTreeMap::new();
    let mut done = Overlaid::default();

    for site in &extraction.sites {
        let Some(instead) = offer(&site.spot, &site.from.text) else {
            done.kept += 1;
            continue;
        };

        let line = &source.lines[site.from.line];
        let edit = match site.from.span {
            Some((from, to)) => Edit::Wrote(format!("{}{instead}{}", &line[..from], &line[to..])),
            None if site.from.line != site.line => {
                let indent = &line[..line.len() - line.trim_start().len()];

                Edit::Added(format!("{indent}# \"{instead}\""))
            }
            None => {
                done.kept += 1;
                continue;
            }
        };

        edits.insert(site.from.line, edit);
        done.taken += 1;
    }

    if edits.is_empty() {
        return done;
    }

    let mut out = String::with_capacity(text.len() + edits.len() * 16);
    if source.byte_order_mark {
        out.push('\u{feff}');
    }

    for (index, (line, ending)) in source.lines.iter().zip(&source.endings).enumerate() {
        match edits.get(&index) {
            Some(Edit::Added(added)) => {
                out.push_str(added);
                out.push_str(if ending.is_empty() { "\n" } else { ending });
                out.push_str(line);
            }
            Some(Edit::Wrote(written)) => out.push_str(written),
            None => out.push_str(line),
        }
        out.push_str(ending);
    }

    done.text = Some(out);

    done
}

#[derive(Debug, Clone)]
struct SourceText {
    lines: Vec<String>,
    endings: Vec<&'static str>,
    byte_order_mark: bool,
}

impl SourceText {
    fn parse(raw: &str) -> Self {
        let (byte_order_mark, body) = match raw.strip_prefix('\u{feff}') {
            Some(rest) => (true, rest),
            None => (false, raw),
        };

        let mut lines = Vec::new();
        let mut endings = Vec::new();
        let mut rest = body;

        while !rest.is_empty() {
            let (line, ending, after) = match rest.find('\n') {
                Some(at) if rest[..at].ends_with('\r') => {
                    (&rest[..at - 1], "\r\n", &rest[at + 1..])
                }
                Some(at) => (&rest[..at], "\n", &rest[at + 1..]),
                None => (rest, "", ""),
            };

            lines.push(line.to_string());
            endings.push(ending);
            rest = after;
        }

        Self {
            lines,
            endings,
            byte_order_mark,
        }
    }

    fn render(&self) -> String {
        let mut out = String::with_capacity(self.lines.iter().map(|line| line.len() + 2).sum());
        if self.byte_order_mark {
            out.push('\u{feff}');
        }
        for (line, ending) in self.lines.iter().zip(&self.endings) {
            out.push_str(line);
            out.push_str(ending);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn translated(text: &str, pairs: &[(u32, &str)]) -> String {
        let map: BTreeMap<u32, String> = pairs
            .iter()
            .map(|(id, value)| (*id, value.to_string()))
            .collect();

        Box::new(parse(text)).render(&map).expect("renders").0
    }

    fn units(text: &str) -> Vec<TranslationUnit> {
        parse(text).units().to_vec()
    }

    #[test]
    fn the_english_a_line_translates_is_the_comment_above_it() {
        let text = concat!(
            "# game/script.rpy:63\n",
            "translate french endofversion_e6baa63e:\n",
            "\n",
            "    # dev \"End of current version.\"\n",
            "    dev \"End of current version.\"\n"
        );

        let found = units(text);
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].text, "End of current version.",
            "the line under the block is already blank or half translated, so the English to \
             show the reader only survives in the comment Ren'Py wrote above it"
        );
    }

    #[test]
    fn only_the_words_change_when_a_line_is_rewritten() {
        let text = "    # dev \"Hello\"\n    dev \"Hello\"\n";
        let output = translated(text, &[(0, "Greetings")]);

        assert_eq!(
            output, "    # dev \"Hello\"\n    dev \"Greetings\"\n",
            "the comment above is Ren'Py's own record of the English, so rewriting it too would \
             lose the source the next read needs"
        );
    }

    #[test]
    fn a_line_nobody_speaks_is_still_a_line() {
        let text = "    # \"This is you.\"\n    \"This is you.\"\n";
        assert_eq!(units(text)[0].text, "This is you.");
        assert!(translated(text, &[(0, "That is you.")]).contains("    \"That is you.\""));
    }

    #[test]
    fn a_string_pair_keeps_its_old_line_as_the_key() {
        let text = "translate french strings:\n    old \"Save\"\n    new \"Save\"\n";
        let output = translated(text, &[(0, "Store")]);

        assert_eq!(units(text).len(), 1);
        assert!(output.contains("    old \"Save\""));
        assert!(output.contains("    new \"Store\""));
    }

    #[test]
    fn a_comment_written_over_a_string_pair_is_what_gets_translated() {
        let text = concat!(
            "translate patch strings:\n",
            "\n",
            "    # game/screens.rpy:105\n",
            "    # \"Back\"\n",
            "    old \"Назад\"\n",
            "    new \"Назад\"\n",
        );

        assert_eq!(
            units(text)[0].text,
            "Back",
            "old has to stay the key Ren'Py looks the string up by, so the words to translate can \
             only come from a comment beside it"
        );

        let output = translated(text, &[(0, "\u{623b}\u{308b}")]);
        assert!(output.contains("    old \"Назад\""));
        assert!(output.contains("    new \"\u{623b}\u{308b}\""));
    }

    #[test]
    fn each_string_pair_takes_the_comment_written_over_it_and_no_other() {
        let text = concat!(
            "translate patch strings:\n",
            "    # \"Back\"\n",
            "    old \"Назад\"\n",
            "    new \"\"\n",
            "    # \"Skip\"\n",
            "    old \"Пропуск\"\n",
            "    new \"\"\n",
        );

        assert_eq!(
            units(text)
                .iter()
                .map(|one| one.text.as_str())
                .collect::<Vec<_>>(),
            ["Back", "Skip"],
            "a pair reads the comment above itself, or every pair after the first is translating \
             the one before it"
        );
    }

    #[test]
    fn a_string_pair_with_only_ren_pys_own_comments_over_it_still_reads_its_key() {
        let text = concat!(
            "translate patch strings:\n",
            "    # game/screens.rpy:105\n",
            "    old \"Назад\"\n",
            "    new \"\"\n",
        );

        assert_eq!(
            units(text)[0].text,
            "Назад",
            "a strings block keys on the old text itself, so a comment sitting between the \
             header and it must not be mistaken for the key"
        );
    }

    #[test]
    fn a_line_left_over_from_the_block_before_is_not_read_as_a_string() {
        let text = concat!(
            "translate patch strings:\n",
            "    # e \"A line nothing ever consumed.\"\n",
            "\n",
            "    old \"Назад\"\n",
            "    new \"\"\n",
        );

        assert_eq!(
            units(text)
                .iter()
                .map(|one| one.text.as_str())
                .collect::<Vec<_>>(),
            ["Назад"],
            "only a comment written directly over the pair is its own, or a stray line above the \
             block ends up translated in its place"
        );
    }

    #[test]
    fn a_text_tag_reaches_the_game_exactly_as_it_was_written() {
        let text = concat!(
            "    # dev \"{color=#00ff00}End of version.{/color}\"\n",
            "    dev \"{color=#00ff00}End of version.{/color}\"\n"
        );

        assert_eq!(
            units(text)[0].text,
            "{color=#00ff00}End of version.{/color}"
        );
        assert!(
            translated(text, &[(0, "{color=#00ff00}The end.{/color}")])
                .contains("    dev \"{color=#00ff00}The end.{/color}\"")
        );
    }

    #[test]
    fn an_escaped_quote_reads_and_writes_back_the_same() {
        let text = "    el \"He said \\\"hello\\\" once.\"\n";
        assert_eq!(units(text)[0].text, r#"He said \"hello\" once."#);

        let output = translated(text, &[(0, r#"She said \"bye\" twice."#)]);
        assert_eq!(output, "    el \"She said \\\"bye\\\" twice.\"\n");
    }

    #[test]
    fn nothing_a_translator_could_not_translate_is_offered() {
        assert!(
            units("    e \"\"\n    e \"{#tag}\"\n").is_empty(),
            "an empty line and a bare Ren'Py tag hold no word anybody could translate, and \
             asking about them spends a request on nothing"
        );

        assert_eq!(
            units("    e \"New{#project}\"\n")[0].text,
            "New{#project}",
            "Ren'Py hands a writer that tag to tell two identical lines apart and strips it \
             before drawing, so the line around it is real dialogue: refusing every line holding \
             one leaves the game in the language it shipped in with nothing said about it"
        );
    }

    #[test]
    fn a_speaker_wearing_attributes_is_still_the_speaker() {
        let text = concat!(
            "    # mira annoyed \"Careful, [name]!\"\n",
            "    mira annoyed \"Careful, [name]!\"\n"
        );

        assert_eq!(units(text)[0].text, "Careful, [name]!");
        assert!(
            translated(text, &[(0, "Watch out, [name]!")])
                .contains("    mira annoyed \"Watch out, [name]!\"")
        );
    }

    #[test]
    fn a_speaker_shedding_an_attribute_is_still_the_speaker() {
        let text = concat!(
            "    # anon a_phone e_sw f_worried @ -m_talk \"( No text messages from [story.cast.alex]. )\"\n",
            "    anon a_phone e_sw f_worried @ -m_talk \"( No text messages from [story.cast.alex]. )\"\n"
        );

        assert_eq!(
            units(text)[0].text,
            "( No text messages from [story.cast.alex]. )"
        );
        assert!(
            translated(text, &[(0, "( [story.cast.alex]からのメッセージはない。 )")])
                .contains("    anon a_phone e_sw f_worried @ -m_talk \"( [story.cast.alex]からのメッセージはない。 )\"")
        );
    }

    #[test]
    fn code_appended_after_the_blocks_is_not_read_as_their_dialogue() {
        let text = concat!(
            "translate patch start_a1b2c3d4:\n",
            "\n",
            "    e \"Hi.\"\n",
            "\n",
            "style my_text:\n",
            "    font \"foo.ttf\"\n",
        );

        assert_eq!(
            harvest(text)
                .iter()
                .map(|(_, said)| said.as_str())
                .collect::<Vec<_>>(),
            ["Hi.",],
            "a style block is code the game runs, and reading its quoted file name as dialogue \
             would send a font path off to be translated"
        );
    }

    #[test]
    fn a_voice_line_keeps_its_audio_path() {
        let text = concat!(
            "    # voice \"audio/vo/zoe/line.ogg\"\n",
            "    # z \"Say something.\"\n",
            "    voice \"audio/vo/zoe/line.ogg\"\n",
            "    z \"Say something.\"\n"
        );

        let found = units(text);
        assert_eq!(found.len(), 1, "only the dialogue is translated");
        assert_eq!(found[0].text, "Say something.");

        let output = translated(text, &[(0, "Speak up.")]);
        assert!(output.contains("    voice \"audio/vo/zoe/line.ogg\""));
        assert!(output.contains("    z \"Speak up.\""));
    }

    #[test]
    fn a_statement_without_a_string_does_not_lose_the_line_after_it() {
        let text = concat!(
            "    # nvl clear\n",
            "    # e \"Hello.\"\n",
            "    nvl clear\n",
            "    e \"Hello.\"\n"
        );

        let found = units(text);
        assert_eq!(found.len(), 1, "the dialogue is still found");
        assert_eq!(found[0].text, "Hello.");
        assert!(translated(text, &[(0, "こんにちは。")]).contains("    e \"こんにちは。\""));
    }

    #[test]
    fn a_modifier_after_the_words_survives_the_rewrite() {
        let text = "    # j \"Get up!\" nointeract\n    j \"Get up!\" nointeract\n";
        assert!(
            translated(text, &[(0, "Wake up!")]).contains("    j \"Wake up!\" nointeract"),
            "what follows the closing quote tells Ren'Py how to play the line, so dropping it \
             changes how the game behaves and not just how it reads"
        );
    }

    #[test]
    fn a_speaker_named_in_quotes_is_not_mistaken_for_the_line() {
        let text = "    # \"???\" \"BANG!\" nointeract\n    \"???\" \"BANG!\" nointeract\n";

        assert_eq!(units(text)[0].text, "BANG!");
        assert!(translated(text, &[(0, "BOOM!")]).contains("    \"???\" \"BOOM!\" nointeract"));
    }

    #[test]
    fn a_quoted_speaker_wearing_attributes_is_still_the_speaker() {
        let text = "    # \"???\" @ happy \"BANG!\"\n    \"???\" @ happy \"BANG!\"\n";

        assert_eq!(units(text)[0].text, "BANG!");
        assert!(translated(text, &[(0, "BOOM!")]).contains("    \"???\" @ happy \"BOOM!\""));
    }

    #[test]
    fn a_line_holding_two_strings_is_left_alone_rather_than_half_read() {
        assert!(units("    text \"a\" style \"b\"\n").is_empty());
        assert!(units("    $ x = \"a\" + \"b\"\n").is_empty());
    }

    #[test]
    fn the_same_words_twice_are_one_line_to_translate() {
        let found = units("    e \"Yes\"\n    m \"Yes\"\n    e \"No\"\n");
        assert_eq!(
            found.len(),
            2,
            "one wording is one thing to translate however many speakers say it, or the reader \
             is asked the same question twice and can answer it two different ways"
        );
    }

    #[test]
    fn a_line_nobody_translated_comes_back_byte_for_byte() {
        let text = "    e \"One\"\n    e \"Two\"\n";
        let map = BTreeMap::from([(0, "First".to_string())]);
        let (output, applied) = Box::new(parse(text)).render(&map).expect("renders");

        assert_eq!(applied.lines, 1);
        assert!(
            output.contains("    e \"Two\""),
            "a line nobody answered stays exactly as the game shipped it, so a half done \
             translation still runs"
        );
    }

    #[test]
    fn a_translation_taken_back_out_leaves_the_line_saying_what_it_always_said() {
        let text = concat!(
            "translate patch start_a1b2c3d4:\n",
            "\n",
            "    # e \"Hello\"\n",
            "    e \"\"\n",
            "\n",
            "translate patch strings:\n",
            "\n",
            "    old \"Pig\"\n",
            "    new \"\"\n",
            "    old \"Cow\"\n",
            "    new \"\"\n",
        );

        let out = translated(text, &[(2, "ウシ")]);

        assert!(
            out.contains("    e \"Hello\"") && out.contains("    new \"Pig\""),
            "Ren'Py lays every line out emptied, and a block it reads is a block it shows: one \
             left empty puts a blank line in front of the reader instead of the words the game \
             shipped:\n{out}"
        );
        assert!(
            out.contains("    new \"ウシ\""),
            "the lines beside it keep the answers they were given:\n{out}"
        );
    }

    #[test]
    fn a_line_left_untranslated_keeps_the_escapes_the_script_wrote() {
        let text = "    # e \"He said \\\"hi\\\" %d times\"\n    e \"\"\n";

        assert_eq!(
            translated(text, &[]),
            "    # e \"He said \\\"hi\\\" %d times\"\n    e \"He said \\\"hi\\\" %d times\"\n",
            "these words come back off the line the game was written on, so escaping them a \
             second time closes the string early and the file stops compiling"
        );
    }

    #[test]
    fn a_file_keeps_its_byte_order_mark_and_line_endings() {
        for raw in ["\u{feff}a\r\nb\r\n", "a\nb", "a\nb\r\nc\n"] {
            let source = SourceText::parse(raw);
            assert_eq!(
                source.render(),
                raw,
                "the mark at the front and the breaks between lines are the file's own, and \
                 changing them shows up as the whole script having been rewritten"
            );
        }
    }

    #[test]
    fn the_language_token_is_restamped_everywhere() {
        let text = "translate working start_a1b2c3d4:\n    e \"Hi\"\n\ntranslate working strings:\n    old \"Save\"\n";
        let out = retarget(text, "japanese");

        assert!(out.contains("translate japanese start_a1b2c3d4:"));
        assert!(out.contains("translate japanese strings:"));
        assert!(!out.contains("working"));
    }

    #[test]
    fn a_line_that_merely_mentions_translate_is_left_alone() {
        let text = "    e \"translate working please\"\n";
        assert_eq!(
            retarget(text, "japanese"),
            text,
            "the word only names a block when it starts one, so matching it inside a line of \
             dialogue would rewrite what a character says"
        );
    }

    #[test]
    fn a_language_with_regex_metacharacters_is_stamped_literally() {
        let text = "translate working start:\n";
        assert_eq!(
            retarget(text, "$1x"),
            "translate $1x start:\n",
            "a language is a name the reader typed, so reading it as a pattern would let a \
             stray character rewrite the line into something else"
        );
    }

    #[test]
    fn every_translation_lands_on_the_line_its_id_came_from() {
        let text = "translate working a_1:\n    # e \"First\"\n    e \"\"\n\ntranslate working b_2:\n    # m \"Second\"\n    m \"\"\n\ntranslate working strings:\n    old \"Third\"\n    new \"\"\n";

        let parsed = parse(text);
        let map: BTreeMap<u32, String> = parsed
            .units()
            .iter()
            .map(|unit| (unit.id, format!("<{}>", unit.text)))
            .collect();

        let (out, applied) = Box::new(parsed).render(&map).expect("renders");

        assert_eq!(applied.lines, 3);
        assert!(out.contains("e \"<First>\""));
        assert!(out.contains("m \"<Second>\""));
        assert!(out.contains("new \"<Third>\""));
    }

    #[test]
    fn a_second_parse_of_the_written_file_finds_the_english_again() {
        let text = "translate working a_1:\n    # e \"First\"\n    e \"\"\n";
        let map = BTreeMap::from([(0, "こんにちは".to_string())]);
        let (out, _) = Box::new(parse(text)).render(&map).expect("renders");

        assert_eq!(
            parse(&out).units()[0].text,
            "First",
            "reading the game again has to find the same English, or every export would offer \
             the last translation as the new source and drift a little further each round"
        );
    }
}
