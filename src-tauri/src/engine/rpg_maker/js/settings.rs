use crate::backup;
use crate::engine::Install;
use crate::engine::rpg_maker::js::{DATA, SYSTEM};
use anyhow::Result;
use serde_json::Value;
use std::path::Path;
use std::str;

const LOCALE: &str = "locale";
const LATIN: &str = "en_US";

pub async fn run(root: &Path, at: &Install<'_>) -> Result<()> {
    if at.reverting {
        return Ok(());
    }

    let file = root.join(DATA).join(SYSTEM);
    let body = match tokio::fs::read(&file).await {
        Ok(body) => body,
        Err(why) => {
            at.progress
                .warn(at.doing, &format!("reading {}: {why}", file.display()));

            return Ok(());
        }
    };

    let Some(body) = typing_in_latin(&body) else {
        return Ok(());
    };

    backup::replace(at.store, at.game_dir, &file, body).await
}

fn typing_in_latin(body: &[u8]) -> Option<Vec<u8>> {
    let held = str::from_utf8(body).ok()?;
    let mut root: Value =
        serde_json::from_str(held.strip_prefix('\u{feff}').unwrap_or(held)).ok()?;
    let held = root.get_mut(LOCALE)?;

    if held == LATIN {
        return None;
    }

    *held = Value::String(LATIN.to_string());

    serde_json::to_vec(&root).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_game_that_stops_calling_itself_japanese_hands_the_player_a_keyboard_they_can_type_on() {
        let held = typing_in_latin(r#"{"locale":"ja_JP","gameTitle":"電車"}"#.as_bytes())
            .expect("the field is rewritten");
        let back: Value = serde_json::from_slice(&held).expect("it is still json");

        assert_eq!(
            back[LOCALE], LATIN,
            "RPG Maker picks its name entry keyboard off this one field, so a translated game \
             that still says ja_JP asks the player to spell an English name out of hiragana"
        );
        assert_eq!(
            back["gameTitle"], "電車",
            "and nothing else in the file is ours to touch"
        );
    }

    #[test]
    fn a_settings_file_with_nothing_to_change_is_left_where_it_is() {
        assert!(
            typing_in_latin(br#"{"locale":"en_US"}"#).is_none(),
            "rewriting a file that already reads the way we want it would take a backup slot and \
             a mark for a change nobody made"
        );
        assert!(
            typing_in_latin(br#"{"gameTitle":"one"}"#).is_none(),
            "a game that never had the field is left without one rather than taught a new field \
             this build invented"
        );
        assert!(
            typing_in_latin(b"not json at all").is_none(),
            "and a settings file this reader cannot open is left exactly as it shipped"
        );
    }
}
