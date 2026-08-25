use crate::engine::rpg_maker::rgss::{marshal, packed, scripts};
use regex::Regex;
use std::sync::LazyLock;

const HOME: &str = "module TES";

static RE_KEYS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"def\s+self\.keys\s*\n\s*"([0-9a-fA-F]+)""#).expect("RE_KEYS is a valid pattern")
});

pub fn keys_of(scripts: &[u8]) -> Option<String> {
    for (_, source) in scripts::sources(scripts).ok()? {
        if !source.contains(HOME) {
            continue;
        }

        if let Some(found) = RE_KEYS.captures(&source) {
            return Some(found[1].to_string());
        }
    }

    None
}

pub fn looks_locked(bytes: &[u8]) -> bool {
    holds_one_said(bytes)
        && marshal::read(bytes).is_ok_and(|sheet| sheet.view.is_string() && sheet.texts.len() == 1)
}

fn holds_one_said(bytes: &[u8]) -> bool {
    match bytes.get(2) {
        Some(b'"') => true,
        Some(b'I') => bytes.get(3) == Some(&b'"'),
        _ => false,
    }
}

pub fn decoded(bytes: &[u8], keys: &str) -> Option<Vec<u8>> {
    if !holds_one_said(bytes) {
        return None;
    }

    let sheet = marshal::read(bytes).ok()?;
    let held = sheet.texts.first().filter(|_| sheet.texts.len() == 1)?;
    if !sheet.view.is_string() {
        return None;
    }

    let out = packed::opened(&turned(&bytes[held.at.clone()], keys)).ok()?;

    out.starts_with(&[4, 8]).then_some(out)
}

pub fn encoded(inner: &[u8], keys: &str) -> Result<Vec<u8>, String> {
    let body = turned(&packed::shut(inner)?, keys);
    let mut out = vec![4, 8, b'"'];
    out.extend_from_slice(&marshal::long_bytes(body.len() as i64));
    out.extend_from_slice(&body);

    Ok(out)
}

fn turned(body: &[u8], keys: &str) -> Vec<u8> {
    let keys = keys.as_bytes();

    body.iter()
        .enumerate()
        .map(|(which, byte)| byte ^ keys[which % keys.len()])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::rpg_maker::rgss::fixture;

    const SCRIPT: &str = "module TES\n  SOURCE_DIR = \"../Scenario/\"\nend\n\n  def self.keys\n    \
                          \"b61a0f29\"\n  end\n";

    #[test]
    fn the_game_hands_over_the_key_its_own_scenario_was_locked_with() {
        let raw = fixture::scripts(&[("Scene_Map", "class Scene_Map\nend\n"), ("TES", SCRIPT)]);

        assert_eq!(keys_of(&raw).as_deref(), Some("b61a0f29"));
        assert_eq!(
            keys_of(&fixture::scripts(&[(
                "Scene_Map",
                "class Scene_Map\nend\n"
            )])),
            None,
            "a game without that system has no scenario to unlock"
        );
    }

    #[test]
    fn a_sheet_locked_by_a_reader_we_do_not_know_is_still_seen_as_locked() {
        let one = encoded(&[4, 8, b'0'], "b61a0f29").expect("it packs");

        assert!(
            looks_locked(&one),
            "one string and nothing else is a container"
        );
        assert!(
            decoded(&one, "a key from another game").is_none(),
            "and this reader cannot open it"
        );
        assert!(
            !looks_locked(&[4, 8, b'[', 6, b'i', 6]),
            "an ordinary sheet holds a list, not one locked string"
        );
    }

    #[test]
    fn a_scenario_goes_out_and_back_in_the_shape_the_game_reads() {
        let inner = vec![4u8, 8, b'[', 6, b'"', 10, b'h', b'i'];
        let keys = "b61a0f29";

        let packed = encoded(&inner, keys).expect("it packs");
        assert!(
            !packed.windows(2).any(|two| two == b"hi"),
            "the words may not sit in the file in the clear"
        );

        assert_eq!(
            decoded(&packed, keys).as_deref(),
            Some(inner.as_slice()),
            "what the game unlocks has to be what we locked"
        );
        assert_eq!(
            decoded(&packed, "another key"),
            None,
            "a wrong key gives nothing rather than nonsense"
        );
        assert_eq!(
            decoded(&[4, 8, b'[', 6, b'i', 6], keys),
            None,
            "an ordinary sheet is not a scenario"
        );
    }
}
