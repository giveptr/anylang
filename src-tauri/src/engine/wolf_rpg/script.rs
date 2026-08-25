use crate::engine::wolf_rpg::event::MESSAGE;
use crate::engine::wolf_rpg::held::{Held, Kind, Piece, Said, Shape};
use crate::engine::wolf_rpg::reached::Reached;
use regex::Regex;
use std::ops::Range;
use std::str;
use std::sync::LazyLock;

pub const SUFFIX: &str = "txt";

pub const PICKED: u32 = 1;
pub const PROMPT: u32 = 2;
pub const SPEAKER: u32 = 3;

pub const ARGUMENTS: [u32; 3] = [PICKED, PROMPT, SPEAKER];

const MARK: &str = "\u{feff}";

const MES: &str = "@mes";
const SELECT: &str = "@select";
const SPOKEN: [(&str, u32); 3] = [(MES, MESSAGE), ("@putselect", PICKED), (SELECT, PROMPT)];

fn past_counts(said: &str) -> usize {
    let mut at = 0;

    loop {
        let rest = &said[at..];
        let lead = rest.len() - rest.trim_start().len();
        let token = rest[lead..].split_whitespace().next().unwrap_or_default();

        if token.is_empty() || !token.chars().all(|one| one.is_ascii_digit()) {
            return at + lead;
        }

        at += lead + token.len();
    }
}

fn spoken(line: &str) -> Option<(&'static str, u32)> {
    SPOKEN.into_iter().find(|(command, _)| {
        line.strip_prefix(command)
            .is_some_and(|rest| rest.starts_with(' '))
    })
}

const OPENERS: [&str; 3] = ["@", "#", "::"];
const ORDER: char = '@';

fn closes(body: &str) -> bool {
    body.trim().is_empty() || OPENERS.iter().any(|one| body.starts_with(one))
}

pub const STRINGS_FROM: u32 = 3_000_000;

static RE_STRING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\\s\[(\d+)\]").expect("a valid pattern"));

fn offered(command: &str) -> bool {
    SPOKEN.iter().any(|(spoken, _)| *spoken == command)
}

pub fn taken_apart(raw: &[u8], out: &mut Reached) {
    let Ok(whole) = str::from_utf8(raw) else {
        return;
    };

    for line in whole.split('\n') {
        let body = line.strip_suffix('\r').unwrap_or(line);
        let body = body.strip_prefix(MARK).unwrap_or(body);

        if !body.starts_with(ORDER) {
            continue;
        }

        for held in RE_STRING.captures_iter(body) {
            if let Ok(which) = held[1].parse::<u32>() {
                out.takes_apart(STRINGS_FROM + which);
            }
        }

        let command = body.split(' ').next().unwrap_or_default();
        if offered(command) {
            continue;
        }

        for token in body.split(' ').skip(1).filter(|one| !one.is_empty()) {
            out.hands(token.trim_end_matches('\r'));
        }
    }
}

struct Boxful {
    line: usize,
    at: Range<usize>,
}

fn one_piece(spot: usize, code: u32, at: Range<usize>, whole: &str) -> Piece {
    Piece {
        spot: format!("l{spot}"),
        kind: Kind::Command {
            code,
            args: Vec::new(),
        },
        said: vec![Said {
            text: whole[at.clone()].to_string(),
            at,
        }],
    }
}

fn flushed(boxful: &mut Option<Boxful>, pieces: &mut Vec<Piece>, whole: &str) {
    if let Some(held) = boxful.take() {
        pieces.push(one_piece(held.line, MESSAGE, held.at, whole));
    }
}

pub fn read(raw: &[u8]) -> Result<Held, String> {
    let whole = str::from_utf8(raw)
        .map_err(|_| "this script is not the UTF-8 the newer editor writes".to_string())?;

    let mut pieces = Vec::new();
    let mut at = 0usize;
    let mut telling = false;
    let mut boxful: Option<Boxful> = None;

    for (which, line) in whole.split('\n').enumerate() {
        let step = line.len() + 1;
        let body = line.strip_suffix('\r').unwrap_or(line);

        let skipped = match body.strip_prefix(MARK) {
            Some(_) => MARK.len(),
            None => 0,
        };
        let body = &body[skipped..];

        let (from, said, code) = match spoken(body) {
            Some((MES, _)) => {
                flushed(&mut boxful, &mut pieces, whole);
                telling = true;

                let from = MES.len() + 1;
                let rest = &body[from..];
                let lead = rest.len() - rest.trim_start().len();
                let token = rest[lead..].split(' ').next().unwrap_or_default();

                (skipped + from + lead, token, SPEAKER)
            }
            Some((command, code)) => {
                flushed(&mut boxful, &mut pieces, whole);
                telling = false;

                let from = command.len() + 1;
                let step = match command == SELECT {
                    true => past_counts(&body[from..]),
                    false => 0,
                };

                (skipped + from + step, &body[from + step..], code)
            }
            None => {
                if closes(body) {
                    flushed(&mut boxful, &mut pieces, whole);
                    telling = false;
                    at += step;
                    continue;
                }

                if telling {
                    let lead = body.len() - body.trim_start().len();
                    let trimmed = body.trim();
                    let start = at + skipped + lead;
                    let end = start + trimmed.len();

                    match &mut boxful {
                        Some(held) => held.at.end = end,
                        None => {
                            boxful = Some(Boxful {
                                line: which,
                                at: start..end,
                            });
                        }
                    }
                }

                at += step;
                continue;
            }
        };

        let lead = said.len() - said.trim_start().len();
        let trimmed = said.trim();

        if !trimmed.is_empty() {
            let start = at + from + lead;
            pieces.push(one_piece(which, code, start..start + trimmed.len(), whole));
        }

        at += step;
    }

    flushed(&mut boxful, &mut pieces, whole);

    Ok(Held {
        plain: raw.to_vec(),
        shape: Shape::Loose,
        pieces,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::wolf_rpg::{harvest, held};
    use std::collections::BTreeMap;

    fn sifted(raw: &[u8]) -> Vec<String> {
        let held = read(raw).expect("a script");

        harvest::sift(&held.pieces, &Default::default())
            .into_iter()
            .map(|one| one.said)
            .collect()
    }

    #[test]
    fn a_message_row_opening_with_a_code_is_words_all_the_same() {
        let raw = concat!(
            "@mes \\s[7]\r\n",
            "\u{3060}\u{3093}\u{3060}\u{3093}\u{601d}\u{3044}\u{51fa}\u{3057}\u{3066}\u{304d}\u{305f}\r\n",
            "\\s[6]\u{306e}\u{540d}\u{524d}\u{306f}\u{2026}\r\n"
        );

        assert_eq!(
            sifted(raw.as_bytes()),
            [concat!(
                "\u{3060}\u{3093}\u{3060}\u{3093}\u{601d}\u{3044}\u{51fa}\u{3057}\u{3066}\u{304d}\u{305f}",
                "\n",
                "\\s[6]\u{306e}\u{540d}\u{524d}\u{306f}\u{2026}"
            )],
            "the rows of one box are asked about together, and a row that opens with a code is \
             still a row a player reads"
        );
    }

    #[test]
    fn the_words_a_command_carries_of_its_own_are_not_read_back_as_names_it_hands_on() {
        let raw = concat!(
            "@mes \u{5730}\u{306e}\u{6587}1\r\n",
            "\u{3053}\u{308c}\u{306f}\u{5263}\u{3060}\r\n",
            "@putselect \u{306f}\u{3044}\r\n",
            "@select 0 100 \u{3069}\u{3061}\u{3089}\r\n"
        )
        .as_bytes();

        let held = read(raw).expect("a script");
        let mut reached = Reached::new();
        taken_apart(raw, &mut reached);

        assert_eq!(
            harvest::sift(&held.pieces, &reached)
                .into_iter()
                .map(|one| (one.said, !one.offer.asked()))
                .collect::<Vec<(String, bool)>>(),
            [
                ("\u{5730}\u{306e}\u{6587}1".to_string(), false),
                (
                    "\u{3053}\u{308c}\u{306f}\u{5263}\u{3060}".to_string(),
                    false
                ),
                ("\u{306f}\u{3044}".to_string(), false),
                ("\u{3069}\u{3061}\u{3089}".to_string(), false),
            ],
            "these commands draw their own words, so reading the same line back as a name the \
             script hands on holds back every speaker and every button in the game"
        );
    }

    #[test]
    fn a_line_the_engine_reads_apart_at_its_spaces_is_written_with_wide_ones() {
        let raw =
            "@select 0 100 \u{30aa}\u{30fc}\u{30d7}\u{30cb}\u{30f3}\u{30b0}\u{3092}\u{2026}\r\n";
        let held = read(raw.as_bytes()).expect("a script");
        let reached = Reached::new();
        let taken = harvest::sift(&held.pieces, &reached);

        assert_eq!(taken.len(), 1, "the question is a line of the game");
        assert_eq!(
            taken[0].said, "\u{30aa}\u{30fc}\u{30d7}\u{30cb}\u{30f3}\u{30b0}\u{3092}\u{2026}",
            "the two counts in front of it are not part of it"
        );
        assert!(
            taken[0].offer.asked(),
            "and it is a question a player reads"
        );

        let said = BTreeMap::from([(taken[0].spot.clone(), "The opening...".to_string())]);
        let edits = harvest::changed(
            &held,
            &harvest::sift(&held.pieces, &reached),
            &said,
            &Default::default(),
            &reached,
        );

        assert_eq!(edits.len(), 1);
        assert_eq!(
            String::from_utf8_lossy(&edits[0].1),
            "The\u{3000}opening...",
            "the engine reads this line apart at its spaces, so a space inside the words has to \
             be one it does not read apart at, and a no-break space will not do because this \
             engine reads that one apart too"
        );
    }

    #[test]
    fn the_name_beside_a_message_is_taken_and_held_back_only_when_something_reads_it() {
        let raw =
            "@mes \u{5730}\u{306e}\u{6587}1\r\n\u{3053}\u{308c}\u{306f}\u{5263}\u{3060}\r\n@mes_RESET\r\n"
                .as_bytes();

        assert_eq!(
            sifted(raw),
            [
                "\u{5730}\u{306e}\u{6587}1",
                "\u{3053}\u{308c}\u{306f}\u{5263}\u{3060}"
            ],
            "the name beside a message is drawn in a box of its own, so it is a line of the game"
        );

        let held = read(raw).expect("a script");
        let mut reached = Reached::new();
        reached.codes("\u{5730}\u{306e}\u{6587}1");

        let taken = harvest::sift(&held.pieces, &reached);
        assert_eq!(
            taken
                .iter()
                .map(|one| (one.said.as_str(), !one.offer.asked()))
                .collect::<Vec<(&str, bool)>>(),
            [
                ("\u{5730}\u{306e}\u{6587}1", true),
                ("\u{3053}\u{308c}\u{306f}\u{5263}\u{3060}", false)
            ],
            "a window this name opens is looked up by it, and a speaker nobody looks up is a \
             label a player reads"
        );
        assert!(taken[0].apart, "and it is one argument of a command line");
    }

    #[test]
    fn a_message_running_over_several_lines_is_taken_whole() {
        let raw = concat!(
            "@toki-ev 15\r\n",
            "@mes \u{5730}\u{306e}\u{6587}1\r\n",
            "\u{3053}\u{306e}\u{307e}\u{307e}\u{3067}\u{3082}\r\n",
            "\u{6700}\u{65b0}\u{7248}\u{306b}\r\n",
            "\r\n",
            "@toki-ev 16\r\n",
            "\u{62fe}\u{308f}\u{308c}\u{306a}\u{3044}\r\n",
        )
        .as_bytes();

        assert_eq!(
            sifted(raw),
            [
                "\u{5730}\u{306e}\u{6587}1",
                concat!(
                    "\u{3053}\u{306e}\u{307e}\u{307e}\u{3067}\u{3082}",
                    "\n",
                    "\u{6700}\u{65b0}\u{7248}\u{306b}"
                )
            ],
            "the box is one thing to translate however many rows the author broke it into, and a \
             blank line closes it"
        );
    }

    #[test]
    fn a_choice_the_player_picks_is_taken_from_the_command_itself() {
        let raw = "@putselect \u{306f}\u{3044}\r\n@setface a/b/c\r\n".as_bytes();

        assert_eq!(
            sifted(raw),
            ["\u{306f}\u{3044}"],
            "a choice reads as words on the button, not as a name looked up elsewhere"
        );
    }

    #[test]
    fn a_directive_standing_where_a_message_would_be_is_left_alone() {
        let raw = "@mes \\s[7]\r\n\\c[3]\r\n@wait 15\r\n".as_bytes();

        assert!(sifted(raw).is_empty());
    }

    #[test]
    fn a_bare_line_under_a_command_that_is_not_a_message_is_left_alone() {
        let raw = "@setface a/b/c\r\n\u{5730}\u{306e}\u{6587}\r\n".as_bytes();

        assert!(
            sifted(raw).is_empty(),
            "only a message carries its words on the lines below it"
        );
    }

    #[test]
    fn the_byte_mark_some_of_these_files_open_with_does_not_shift_a_line() {
        let raw = "\u{feff}@mes \u{5730}\u{306e}\u{6587}1\r\n\u{3053}\u{308c}\u{306f}\u{5263}\r\n"
            .as_bytes();
        let held = read(raw).expect("a script");
        let taken = harvest::sift(&held.pieces, &Default::default());

        for (slot, wanted) in taken.iter().zip([
            "\u{5730}\u{306e}\u{6587}1",
            "\u{3053}\u{308c}\u{306f}\u{5263}",
        ]) {
            assert_eq!(
                &raw[slot.at.clone()],
                wanted.as_bytes(),
                "the span has to land on the words themselves"
            );
        }
    }

    #[test]
    fn a_translation_lands_where_the_words_were_and_leaves_the_line_ending_be() {
        let raw = "@mes \u{5730}\u{306e}\u{6587}1\r\n\u{5263}\r\n@wait 15\r\n".as_bytes();
        let held = read(raw).expect("a script");

        let said = [("l1/s0".to_string(), "A sword".to_string())]
            .into_iter()
            .collect();

        let fresh = held::wrapped(
            &held,
            harvest::changed(
                &held,
                &harvest::sift(&held.pieces, &Default::default()),
                &said,
                &Default::default(),
                &Default::default(),
            ),
        )
        .expect("a written script");

        assert_eq!(
            String::from_utf8(fresh).expect("utf8"),
            "@mes \u{5730}\u{306e}\u{6587}1\r\nA sword\r\n@wait 15\r\n",
            "the speaker beside the message is left exactly as it was"
        );
    }

    #[test]
    fn a_translated_box_goes_back_on_the_rows_the_author_drew() {
        let raw = concat!(
            "@mes \u{5730}\u{306e}\u{6587}1\r\n",
            "\u{3053}\u{306e}\u{307e}\u{307e}\u{3067}\u{3082}\r\n",
            "\u{6700}\u{65b0}\u{7248}\u{306b}\r\n",
            "@wait 15\r\n"
        )
        .as_bytes();
        let held = read(raw).expect("a script");

        let said = [(
            "l1/s0".to_string(),
            "Even as it stands\nin the newest".to_string(),
        )]
        .into_iter()
        .collect();

        let fresh = held::wrapped(
            &held,
            harvest::changed(
                &held,
                &harvest::sift(&held.pieces, &Default::default()),
                &said,
                &Default::default(),
                &Default::default(),
            ),
        )
        .expect("a written script");

        assert_eq!(
            String::from_utf8(fresh).expect("utf8"),
            concat!(
                "@mes \u{5730}\u{306e}\u{6587}1\r\n",
                "Even as it stands\r\n",
                "in the newest\r\n",
                "@wait 15\r\n"
            ),
            "the box keeps the rows and the line endings the author drew, or the file stops \
             lining up with itself"
        );
    }

    #[test]
    fn a_script_that_is_not_utf8_is_turned_away_rather_than_mangled() {
        assert!(read(&[0x40, 0x6d, 0x65, 0x73, 0x20, 0x82, 0xa0]).is_err());
    }
}
