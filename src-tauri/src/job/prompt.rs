use crate::engine::{Engine, TranslationUnit, humped};
use crate::project::{Era, Fidelity, Mood, Project, Register};
use anyhow::Result;
use serde::Deserialize;
use serde::de::value::StrDeserializer;
use std::collections::BTreeMap;

fn fidelity_rule(fidelity: Fidelity) -> &'static str {
    match fidelity {
        Fidelity::Literal => {
            "Stay close to the original wording and sentence shape. Do not embellish, cut or \
             reorder."
        }
        Fidelity::Balanced => {
            "Translate the meaning and the intent, not word for word. Every line should read as \
             if it were written in the target language."
        }
        Fidelity::Free => {
            "Adapt freely. Keep what each line does in the scene, but rewrite it however reads \
             best in the target language."
        }
    }
}

fn worded(key: &str) -> String {
    if key.contains(char::is_whitespace) || !humped(key) {
        return key.to_string();
    }

    let mut said = String::with_capacity(key.len() + 2);
    let mut last = ' ';

    for one in key.chars() {
        if last.is_ascii_lowercase() && one.is_ascii_uppercase() {
            said.push('-');
        }
        said.push(one.to_ascii_lowercase());
        last = one;
    }

    said
}

fn genre_name(key: &str) -> Option<String> {
    (!key.contains(char::is_whitespace)).then(|| worded(key))
}

fn era_phrase(era: Era) -> Option<&'static str> {
    match era {
        Era::Any => None,
        Era::Ancient => Some("in the ancient world"),
        Era::Medieval => Some("in a medieval world"),
        Era::EarlyModern => Some("a few centuries back"),
        Era::Victorian => Some("in the nineteenth century"),
        Era::EarlyTwentieth => Some("in the early twentieth century"),
        Era::LateTwentieth => Some("in the closing decades of the twentieth century"),
        Era::Modern => Some("in the present day"),
        Era::NearFuture => Some("a short way into the future"),
        Era::FarFuture => Some("far into the future"),
    }
}

fn setting_rule(genres: &[String], era: Era) -> Option<String> {
    let listed = join_names(genres);
    let said = match (listed.is_empty(), era_phrase(era)) {
        (true, None) => return None,
        (true, Some(when)) => format!("The game is set {when}."),
        (false, None) => format!("The game is {} {listed} title.", article(&listed)),
        (false, Some(when)) => {
            format!(
                "The game is {} {listed} title, set {when}.",
                article(&listed)
            )
        }
    };

    Some(format!(
        "{said} Pick vocabulary and forms of address that fit it, and keep out words that would \
         not exist there."
    ))
}

fn register_rule(register: Register) -> Option<&'static str> {
    match register {
        Register::Any => None,
        Register::Coarse => Some("Pitch the language coarse and foul-mouthed."),
        Register::Casual => Some("Pitch the language loose and everyday."),
        Register::Formal => Some("Pitch the language measured and formal."),
        Register::Elevated => Some("Pitch the language grand and elevated."),
    }
}

fn voice_phrase(key: &str) -> Option<&'static str> {
    let mood = Mood::deserialize(StrDeserializer::<serde::de::value::Error>::new(key)).ok()?;

    Some(match mood {
        Mood::Comic => "a comic streak played for laughs",
        Mood::Witty => "quick wit and sharp comebacks",
        Mood::Sarcastic => "a dry sarcastic edge",
        Mood::Playful => "a light playful energy",
        Mood::Cute => "a soft endearing charm",
        Mood::Warm => "a gentle warmth",
        Mood::Melancholic => "a wistful melancholic undertone",
        Mood::Dramatic => "heightened emotion in the heavy scenes",
        Mood::Epic => "a grand heroic sweep",
        Mood::Dark => "a grim heavy undertone",
        Mood::Unsettling => "a creeping unease under ordinary talk",
        Mood::Tense => "tight tense pacing",
        Mood::Deadpan => "a flat deadpan delivery",
        Mood::Explicit => {
            "a raw frankness that calls things what they are rather than reaching for clinical or \
             coy wording"
        }
    })
}

fn article(following: &str) -> &'static str {
    match following.chars().next() {
        Some('a' | 'e' | 'i' | 'o' | 'u' | 'A' | 'E' | 'I' | 'O' | 'U') => "an",
        _ => "a",
    }
}

fn split_known<'a, T>(
    keys: &'a [String],
    name: impl Fn(&'a str) -> Option<T>,
) -> (Vec<T>, Vec<&'a str>) {
    let mut known = Vec::new();
    let mut rest = Vec::new();

    for key in keys {
        match name(key) {
            Some(found) => known.push(found),
            None => rest.push(key.as_str()),
        }
    }

    (known, rest)
}

fn join_names<T: AsRef<str>>(names: &[T]) -> String {
    match names {
        [] => String::new(),
        [one] => one.as_ref().to_string(),
        [head @ .., last] => format!(
            "{} and {}",
            head.iter()
                .map(AsRef::as_ref)
                .collect::<Vec<_>>()
                .join(", "),
            last.as_ref()
        ),
    }
}

const OUTPUT_CONTRACT: &str = r#"--- OUTPUT CONTRACT ---
- The input is a JSON array of objects with "id" (integer) and "text" (string).
- Return a JSON object with one key, "items": an array holding one object per input object.
- Each object in "items" carries the same "id", plus "translation" (string). The translated
  words go under "translation". An object that answers with "text" is thrown away unread.
- Return exactly as many objects as the input has. Never merge, split, reorder or drop items.
- Translate only the value of "text". Never translate or invent ids.
- Output only the JSON object. No markdown fences, no comments, no explanation."#;

pub fn system_instruction(engine: &dyn Engine, project: &Project) -> String {
    let style = &project.style;
    let mut rules: Vec<String> = Vec::new();

    let (genres, other_genres) = split_known(&style.genres, genre_name);

    if let Some(said) = setting_rule(&genres, style.era) {
        rules.push(said);
    }
    if !other_genres.is_empty() {
        rules.push(format!(
            "The game is also described as: {}.",
            other_genres.join(", ")
        ));
    }

    rules.push(fidelity_rule(style.fidelity).to_string());

    if let Some(rule) = register_rule(style.register) {
        rules.push(rule.to_string());
    }

    let (voices, other_voices) = split_known(&style.voices, voice_phrase);

    if !voices.is_empty() {
        rules.push(format!("The lines should carry {}.", join_names(&voices)));
    }
    if !other_voices.is_empty() {
        let asked: Vec<String> = other_voices.iter().map(|key| worded(key)).collect();
        rules.push(format!(
            "Aim for this in the mood too: {}.",
            asked.join(", ")
        ));
    }

    if !style.notes.trim().is_empty() {
        rules.push(format!(
            "Follow these notes exactly. They come from the person you are translating for:\n{}",
            style.notes.trim()
        ));
    }

    let guide: String = rules
        .iter()
        .enumerate()
        .map(|(index, rule)| format!("{}. {rule}", index + 1))
        .collect::<Vec<_>>()
        .join("\n");

    let told = engine.rules();
    let shape = told
        .shape
        .map(|shape| format!("\n\n--- LINE LAYOUT ---\n{shape}"))
        .unwrap_or_default();

    format!(
        "You are an expert game localization translator, translating {} to **{}**.\n\n\
         --- STYLE GUIDELINES ---\n{guide}\n\n\
         {OUTPUT_CONTRACT}\n\n\
         --- COPY THESE THROUGH BYTE FOR BYTE ---\n{}{shape}",
        project.source_language, project.language, told.markup
    )
}

pub fn user_prompt(
    engine: &dyn Engine,
    units: &[TranslationUnit],
    refused: &BTreeMap<u32, String>,
) -> Result<String> {
    let listed = serde_json::to_string(units)?;

    let named: Vec<String> = units
        .iter()
        .filter_map(|unit| {
            refused
                .get(&unit.id)
                .map(|why| format!("- id {}: {why}", unit.id))
        })
        .collect();

    if named.is_empty() {
        return Ok(format!(
            "Translate the {} item(s) below and return the JSON array.\n\n{listed}",
            units.len(),
        ));
    }

    Ok(format!(
        "Your previous answer was refused for these items:\n{}\n\n{}\nA translation must never \
         come back empty. Translate the items again.\n\n{listed}",
        named.join("\n"),
        engine.rules().retry
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Offer;
    use crate::engine::renpy::RenPy;

    #[test]
    fn a_line_the_reader_was_never_asked_about_still_carries_its_words_to_the_model() {
        let units = vec![TranslationUnit {
            id: 7,
            text: "Hello".to_string(),
            offer: Offer::Listed,
        }];

        let prompt = user_prompt(&RenPy, &units, &BTreeMap::new()).unwrap();
        assert!(
            prompt.contains(r#"[{"id":7,"text":"Hello"}]"#),
            "how a line came to be offered is this tool's own bookkeeping: the model is owed \
             the words and the number, and nothing else"
        );
    }
}
