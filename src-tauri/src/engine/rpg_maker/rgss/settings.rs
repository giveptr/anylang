pub const NAME: &str = "System";

const FIELD: &[u8] = b":\x0e@japanese";
const JAPANESE: u8 = b'T';
const NOT: u8 = b'F';

pub fn typing_in_latin(body: &[u8]) -> Option<Vec<u8>> {
    let at = body.windows(FIELD.len()).position(|held| held == FIELD)? + FIELD.len();

    if *body.get(at)? != JAPANESE {
        return None;
    }

    let mut held = body.to_vec();
    held[at] = NOT;

    Some(held)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_system(japanese: u8) -> Vec<u8> {
        let mut held = b"\x04\x08o:\x11RPG::System\x07:\x11@game_titleI\"\x0f\
                         \xe9\xa8\x8e\xe7\xb4\x85\xe5\xa3\xab\x06:\x06ET"
            .to_vec();
        held.extend_from_slice(FIELD);
        held.push(japanese);
        held.extend_from_slice(b":\x14@opt_use_midiF");

        held
    }

    #[test]
    fn a_game_that_stops_calling_itself_japanese_hands_the_player_a_keyboard_they_can_type_on() {
        let was = a_system(JAPANESE);
        let held = typing_in_latin(&was).expect("the flag is turned off");

        assert_eq!(
            held.len(),
            was.len(),
            "this is one byte inside a Marshal stream, so anything that moved the bytes after it \
             would leave the rest of the file unreadable"
        );
        assert_eq!(
            held,
            a_system(NOT),
            "VX Ace picks its name entry keyboard off this one flag, so a translated game that \
             still says it is Japanese asks the player to spell an English name out of hiragana"
        );
    }

    #[test]
    fn a_settings_file_with_nothing_to_change_is_left_where_it_is() {
        assert!(
            typing_in_latin(&a_system(NOT)).is_none(),
            "rewriting a file that already reads the way we want it would put a sheet into the \
             archive for a change nobody made"
        );
        assert!(
            typing_in_latin(b"\x04\x08o:\x11RPG::System\x00").is_none(),
            "a game that never carried the flag is left without one rather than taught a field \
             this build invented"
        );
        assert!(
            typing_in_latin(FIELD).is_none(),
            "and a stream that ends on the name before it says the value is not ours to guess at"
        );
    }
}
