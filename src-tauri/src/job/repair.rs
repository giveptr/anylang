use anyhow::{Result, anyhow};
use regex::{Captures, Regex};
use serde::Deserialize;
use serde::de::Error;
use std::sync::LazyLock;

#[derive(Debug)]
pub struct TranslatedItem {
    pub id: u32,
    pub translation: String,
}

impl<'de> Deserialize<'de> for TranslatedItem {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = serde_json::Map::deserialize(deserializer)?;

        let id = match raw.get("id") {
            Some(serde_json::Value::Number(number)) => number
                .as_u64()
                .or_else(|| {
                    number
                        .as_f64()
                        .filter(|id| id.fract() == 0.0 && (0.0..=u32::MAX as f64).contains(id))
                        .map(|id| id as u64)
                })
                .and_then(|id| u32::try_from(id).ok())
                .ok_or_else(|| D::Error::custom("id is not a line number"))?,
            Some(serde_json::Value::String(text)) => text
                .trim()
                .parse()
                .map_err(|_| D::Error::custom("id is not a line number"))?,
            _ => return Err(D::Error::custom("an item with no id")),
        };

        let translation = ["translation", "value"]
            .iter()
            .find_map(|key| raw.get(*key)?.as_str())
            .ok_or_else(|| D::Error::custom("an item with no translation"))?
            .to_string();

        Ok(Self { id, translation })
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Payload {
    Array(Vec<TranslatedItem>),
    Wrapped {
        #[serde(alias = "translations", alias = "results", alias = "data")]
        items: Vec<TranslatedItem>,
    },
}

impl Payload {
    fn into_items(self) -> Vec<TranslatedItem> {
        match self {
            Payload::Array(items) | Payload::Wrapped { items } => items,
        }
    }
}

const ARRAYS_TRIED: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseQuality {
    Clean,
    Repaired,
    Salvaged,
}

pub struct ParseOutcome {
    pub items: Vec<TranslatedItem>,
    pub quality: ParseQuality,
}

pub fn parse_items(raw: &str) -> Result<ParseOutcome> {
    let text = strip_code_fence(raw.trim());

    for start in text.match_indices('[').map(|(at, _)| at).take(ARRAYS_TRIED) {
        if let Some(array) = extract_array(&text[start..])
            && let Some(outcome) = read(&array)
        {
            return Ok(outcome);
        }
    }

    if let Some(outcome) = read(text) {
        return Ok(outcome);
    }

    let preview: String = raw.trim().chars().take(240).collect();
    Err(anyhow!("could not read a JSON array from: {preview}"))
}

fn read(candidate: &str) -> Option<ParseOutcome> {
    if let Ok(payload) = serde_json::from_str::<Payload>(candidate) {
        return Some(ParseOutcome {
            items: payload.into_items(),
            quality: ParseQuality::Clean,
        });
    }

    let repaired = repair_syntax(candidate);
    if let Ok(payload) = serde_json::from_str::<Payload>(&repaired) {
        return Some(ParseOutcome {
            items: payload.into_items(),
            quality: ParseQuality::Repaired,
        });
    }

    let salvaged = salvage_objects(&repaired);
    if !salvaged.is_empty() {
        return Some(ParseOutcome {
            items: salvaged,
            quality: ParseQuality::Salvaged,
        });
    }

    None
}

fn outside_strings(text: &str) -> impl Iterator<Item = (usize, u8)> + '_ {
    let mut inside_string = false;
    let mut escaped = false;

    text.bytes().enumerate().filter(move |&(_, byte)| {
        if inside_string {
            match byte {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => inside_string = false,
                _ => {}
            }
            return false;
        }

        if byte == b'"' {
            inside_string = true;
            return false;
        }

        true
    })
}

fn extract_array(text: &str) -> Option<String> {
    let mut depth = 0usize;
    let mut last_complete_object = None;

    for (index, byte) in outside_strings(text) {
        match byte {
            b'[' | b'{' => depth += 1,
            b']' | b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 1 && byte == b'}' {
                    last_complete_object = Some(index + 1);
                }
                if depth == 0 {
                    return Some(text[..=index].to_string());
                }
            }
            _ => {}
        }
    }

    last_complete_object.map(|end| format!("{}]", &text[..end]))
}

fn strip_code_fence(text: &str) -> &str {
    let Some(rest) = text.strip_prefix("```") else {
        return text;
    };
    let rest = rest.strip_prefix("json").unwrap_or(rest).trim();
    rest.strip_suffix("```").unwrap_or(rest)
}

static RE_LINE_COMMENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*//.*$").expect("RE_LINE_COMMENT is a valid pattern"));
static RE_BLOCK_COMMENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)/\*.*?\*/").expect("RE_BLOCK_COMMENT is a valid pattern"));
static RE_BARE_KEY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"([{,]\s*)(?:'([A-Za-z_]\w*)'|([A-Za-z_]\w*))(\s*:)")
        .expect("RE_BARE_KEY is a valid pattern")
});
static RE_MISSING_COMMA: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\}\s*\{").expect("RE_MISSING_COMMA is a valid pattern"));
static RE_TRAILING_COMMA: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r",(\s*[\]}])").expect("RE_TRAILING_COMMA is a valid pattern"));
static RE_UNQUOTED_VALUE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#""(translation|text)"\s*:\s*(?P<value>[^"\s\[\]{},tfn][^,}\n]*?)\s*(?P<end>[,}])"#)
        .expect("RE_UNQUOTED_VALUE is a valid pattern")
});

fn repair_syntax(text: &str) -> String {
    let mut fixed = RE_LINE_COMMENT.replace_all(text, "").to_string();
    fixed = RE_BLOCK_COMMENT.replace_all(&fixed, "").to_string();
    fixed = splice_outside(&fixed, &RE_BARE_KEY, |caps| {
        let name = caps
            .get(2)
            .or_else(|| caps.get(3))
            .map(|found| found.as_str())
            .unwrap_or_default();
        format!("{}\"{name}\"{}", &caps[1], &caps[4])
    });
    fixed = splice_outside(&fixed, &RE_MISSING_COMMA, |_| "}, {".to_string());
    fixed = RE_UNQUOTED_VALUE
        .replace_all(&fixed, r#""$1": "$value"$end"#)
        .to_string();
    splice_outside(&fixed, &RE_TRAILING_COMMA, |caps| caps[1].to_string())
}

fn splice_outside(text: &str, pattern: &Regex, rewrite: impl Fn(&Captures) -> String) -> String {
    let mut outside = outside_strings(text).map(|(at, _)| at).peekable();
    let mut out = String::with_capacity(text.len());
    let mut behind = 0;

    for caps in pattern.captures_iter(text) {
        let whole = caps.get(0).expect("a match always has a whole capture");

        while outside.peek().is_some_and(|at| *at < whole.start()) {
            outside.next();
        }
        if outside.peek() != Some(&whole.start()) {
            continue;
        }

        out.push_str(&text[behind..whole.start()]);
        out.push_str(&rewrite(&caps));
        behind = whole.end();
    }

    out.push_str(&text[behind..]);
    out
}

fn salvage_objects(text: &str) -> Vec<TranslatedItem> {
    let mut items = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;

    for (index, byte) in outside_strings(text) {
        match byte {
            b'{' => {
                if depth == 0 {
                    start = index;
                }
                depth += 1;
            }
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0
                    && let Ok(item) = serde_json::from_str::<TranslatedItem>(&text[start..=index])
                {
                    items.push(item);
                }
            }
            _ => {}
        }
    }

    items
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(outcome: &ParseOutcome) -> Vec<u32> {
        outcome.items.iter().map(|item| item.id).collect()
    }

    #[test]
    fn an_answer_that_is_already_sound_is_read_as_it_stands() {
        let outcome = parse_items(r#"[{"id": 0, "translation": "Hello"}]"#).unwrap();
        assert_eq!(outcome.quality, ParseQuality::Clean);
        assert_eq!(
            outcome.items[0].translation, "Hello",
            "an answer that needed no mending must not be marked as mended, or the run reports \
             trouble the model never caused"
        );
    }

    #[test]
    fn an_id_the_model_wrote_as_a_float_still_lands_on_its_line() {
        let raw = r#"[{"id": 1.0, "translation": "Hello"}]"#;
        assert_eq!(
            ids(&parse_items(raw).unwrap()),
            vec![1],
            "a model that writes a whole number with a decimal point still means that line, \
             and refusing it would drop a translation already paid for"
        );

        assert!(
            serde_json::from_str::<TranslatedItem>(r#"{"id": 1.5, "translation": "Hello"}"#)
                .is_err(),
            "half a line number is no line number"
        );
    }

    #[test]
    fn an_answer_wrapped_in_prose_or_fences_still_gives_up_its_lines() {
        let raw = "Here you go:\n```json\n[{\"id\": 1, \"translation\": \"Hello\"}]\n```";
        assert_eq!(
            ids(&parse_items(raw).unwrap()),
            vec![1],
            "models like to greet you and fence their code, and throwing the whole answer away \
             over that would pay for the batch twice"
        );
    }

    #[test]
    fn an_answer_that_only_echoes_the_prompt_back_is_not_a_translation() {
        assert!(
            parse_items(r#"[{"id": 2, "text": "Hello"}]"#).is_err(),
            "text is the field we send the source in: reading it back as the translation \
             would mark every line done with its own English"
        );
    }

    #[test]
    fn an_array_hidden_inside_an_object_is_still_found() {
        let raw = r#"{"translations": [{"id": 3, "translation": "Hello"}]}"#;
        assert_eq!(
            ids(&parse_items(raw).unwrap()),
            vec![3],
            "an answer wrapped in a field of its own is still the answer, and the lines inside \
             it were paid for either way"
        );
    }

    #[test]
    fn a_comma_too_few_or_too_many_does_not_lose_the_answer() {
        let raw = r#"[{"id": 0, "translation": "A"} {"id": 1, "translation": "B"},]"#;
        let outcome = parse_items(raw).unwrap();
        assert_eq!(ids(&outcome), vec![0, 1]);
        assert_eq!(
            outcome.quality,
            ParseQuality::Repaired,
            "the lines came through but the shape did not, and saying so is what lets a run \
             tell a clean answer from one it had to guess at"
        );
    }

    #[test]
    fn keys_without_quotes_and_stray_comments_are_read_through() {
        let raw = "[\n  // translations\n  {id: 0, translation: \"A\"}\n]";
        assert_eq!(
            ids(&parse_items(raw).unwrap()),
            vec![0],
            "a model writing loose JSON is writing what it was trained on, not making a \
             mistake worth losing the batch over"
        );
    }

    #[test]
    fn an_answer_cut_short_keeps_every_line_that_did_arrive() {
        let raw =
            r#"[{"id": 0, "translation": "A"}, {"id": 1, "translation": "B"}, {"id": 2, "transl"#;
        assert_eq!(
            ids(&parse_items(raw).unwrap()),
            vec![0, 1],
            "an answer cut off at the token limit still holds real work, and dropping it would \
             ask for those lines a second time"
        );
    }

    #[test]
    fn brackets_inside_a_translation_do_not_confuse_the_scanner() {
        let raw = r#"[{"id": 0, "translation": "Press [key] {i}now{/i} ok]"}]"#;
        let outcome = parse_items(raw).unwrap();

        assert_eq!(outcome.items[0].translation, "Press [key] {i}now{/i} ok]");
        assert_eq!(
            outcome.quality,
            ParseQuality::Clean,
            "salvaging hands back the same words here, so only the reading says whether the \
             scanner counted a bracket inside a translation as the end of the answer"
        );
    }

    #[test]
    fn a_translation_the_model_left_unquoted_is_still_taken() {
        let raw = r#"[{"id": 0, "translation": Hello there}]"#;
        let outcome = parse_items(raw).unwrap();
        assert_eq!(
            outcome.items[0].translation, "Hello there",
            "the words are there to read even without their quotes, and refusing them loses a \
             line that was already paid for"
        );
    }

    #[test]
    fn one_broken_entry_does_not_cost_the_ones_beside_it() {
        let raw = r#"[{"id": 0, "translation": "A"}, {"id": 1, "translation": }, {"id": 2, "translation": "C"}]"#;
        let outcome = parse_items(raw).unwrap();
        assert_eq!(outcome.quality, ParseQuality::Salvaged);
        assert_eq!(
            ids(&outcome),
            vec![0, 2],
            "one entry the model fumbled is one line to ask again for, not a whole batch to \
             throw away"
        );
    }

    #[test]
    fn an_answer_holding_no_line_at_all_is_refused() {
        assert!(parse_items("Sorry, I cannot translate this content.").is_err());
    }

    #[test]
    fn a_model_echoing_the_source_or_quoting_the_id_is_understood() {
        let raw = r#"[{"id": "0", "text": "Hello", "translation": "Bonjour"}]"#;
        let outcome = parse_items(raw).unwrap();
        assert_eq!(
            outcome.items[0].id, 0,
            "an id the model quoted still names the same line, and reading it as a stranger \
             would file the translation against nothing"
        );
        assert_eq!(outcome.items[0].translation, "Bonjour");
    }

    #[test]
    fn repairing_the_shape_never_edits_the_words_inside() {
        let raw =
            r#"[{"id": 0, "translation": "{/b}{i}Hi{/i}"} {"id": 1, "translation": "a, b: c,]"}]"#;
        let outcome = parse_items(raw).unwrap();
        assert_eq!(outcome.quality, ParseQuality::Repaired);
        assert_eq!(outcome.items[0].translation, "{/b}{i}Hi{/i}");
        assert_eq!(outcome.items[1].translation, "a, b: c,]");
    }

    #[test]
    fn a_preamble_with_brackets_does_not_hide_the_array() {
        let raw = "Here are the lines [1-4]:\n[{\"id\": 4, \"translation\": \"D\"}]";
        assert_eq!(
            ids(&parse_items(raw).unwrap()),
            vec![4],
            "brackets in the model's chatter must not be mistaken for the start of the answer, \
             or the scanner reads a range of numbers as the translations"
        );
    }
}
