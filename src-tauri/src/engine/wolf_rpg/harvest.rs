use crate::engine::wolf_rpg::event::{
    CALL_BY_NAME, CHOICES, DB_WRITE, MESSAGE, PICTURE, SET_STRING, STRING_CONDITION,
};
use crate::engine::wolf_rpg::held::{Edits, Held, Kind, Piece, Shape};
use crate::engine::wolf_rpg::reached::Reached;
use crate::engine::wolf_rpg::script::{ARGUMENTS, PICKED, PROMPT, SPEAKER};
use crate::engine::wolf_rpg::{coder, text};
use crate::engine::{Offer, names_a_file, symbolic};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

const DATA_NAME: usize = 2;

const DRAWS_TEXT: u32 = 2;

const WIDE: &str = "\u{3000}";

pub struct Slot {
    pub spot: String,
    pub whole: String,
    pub said: String,
    pub at: Range<usize>,
    pub offer: Offer,
    pub apart: bool,
    pub matched: bool,
}

#[derive(Clone, Copy)]
enum Care {
    Shown,
    Label,
    Kept,
    Called,
    Names,
    NamedBy,
    Plain,
    Compared,
}

fn marked_up(text: &str) -> bool {
    text.chars()
        .zip(text.chars().skip(1))
        .any(|(one, next)| one == '\\' && next.is_ascii_alphabetic())
}

fn keylike(text: &str) -> bool {
    let held = text.trim();
    let (head, rest) = held.split_once(char::is_whitespace).unwrap_or((held, ""));

    if head.is_empty() || !head.is_ascii() || marked_up(head) {
        return false;
    }

    rest.is_empty() || (text::marked(rest) && !text::has_words(rest))
}

fn told_to(text: &str) -> bool {
    let held = text.trim();

    held.starts_with("<<") && held[2..].contains(">>")
}

const APART: [char; 2] = ['\u{3001}', ','];

fn tabled(text: &str) -> bool {
    let held = text.trim();
    if !held.contains(APART) {
        return false;
    }

    let parts: Vec<&str> = held.split(APART).map(str::trim).collect();

    let numbers = parts
        .iter()
        .filter(|one| !one.is_empty() && one.chars().all(|held| held.is_ascii_digit()))
        .count();

    parts.len() >= 3
        && numbers >= 2
        && numbers * 2 >= parts.len()
        && parts.iter().all(|one| {
            !one.is_empty()
                && (one.chars().all(|held| held.is_ascii_digit()) || one.chars().count() == 1)
        })
}

const FOLDERS: [char; 2] = ['/', '\\'];

fn under_a_folder(part: &str) -> Option<&str> {
    part.rsplit_once(FOLDERS).map(|(_, tail)| tail)
}

fn shipped_names(text: &str, reached: &Reached) -> bool {
    let mut parts = text
        .split(APART)
        .map(str::trim)
        .filter(|one| !one.is_empty())
        .peekable();

    parts.peek().is_some()
        && parts.all(|one| {
            reached.kept(one) || under_a_folder(one).is_some_and(|tail| reached.kept(tail))
        })
}

fn suffix_only(text: &str) -> bool {
    let held = text.trim();

    held.len() > 1
        && held.len() <= 6
        && held.starts_with('.')
        && held[1..].chars().all(|one| one.is_ascii_alphanumeric())
}

fn for_the_engine(text: &str) -> bool {
    symbolic(text) || told_to(text) || suffix_only(text) || tabled(text) || keylike(text)
}

impl Care {
    fn held_back(self, whole: &str, shown: &str, reached: &Reached) -> bool {
        match self {
            Self::Shown => false,
            Self::Called | Self::Kept => true,
            Self::Label | Self::Compared | Self::Names | Self::NamedBy | Self::Plain => {
                for_the_engine(whole) || shipped_names(shown, reached)
            }
        }
    }

    fn matched(self) -> bool {
        matches!(
            self,
            Self::Label | Self::Compared | Self::Names | Self::NamedBy
        )
    }
}

fn draws_text(args: &[u32]) -> bool {
    args.first()
        .is_some_and(|held| (held >> 4) & 0x07 == DRAWS_TEXT)
}

fn care_of(piece: &Piece, which: usize) -> Option<Care> {
    match &piece.kind {
        Kind::Font => None,
        Kind::Title => Some(Care::Shown),
        Kind::Value => Some(Care::Plain),
        Kind::Naming => Some(Care::NamedBy),
        Kind::Command { code, args } => Some(match (*code, which) {
            (MESSAGE | CHOICES | PICKED | PROMPT, _) => Care::Shown,
            (SPEAKER, _) => Care::Label,
            (PICTURE, 0) if draws_text(args) => Care::Shown,
            (SET_STRING | DB_WRITE, 0) => Care::Names,
            (CALL_BY_NAME, 0) => Care::Called,
            (CALL_BY_NAME, _) => Care::Names,
            (STRING_CONDITION, _) => Care::Compared,
            _ => Care::Kept,
        }),
    }
}

fn read_apart(piece: &Piece, which: usize, reached: &Reached) -> bool {
    match &piece.kind {
        Kind::Command { code, .. } if ARGUMENTS.contains(code) => true,
        Kind::Command { code, args } if (*code, which) == (SET_STRING, 0) => {
            args.first().is_some_and(|held| reached.read_apart(*held))
                && !piece.said[which].text.contains('\n')
        }
        _ => false,
    }
}

pub fn sift(pieces: &[Piece], reached: &Reached) -> Vec<Slot> {
    let mut found = Vec::new();

    for piece in pieces {
        for (which, said) in piece.said.iter().enumerate() {
            let Some(care) = care_of(piece, which) else {
                continue;
            };

            let asked = text::asked(&said.text, reached);
            let shown = asked.as_deref().unwrap_or(&said.text);

            if !text::has_words(shown) {
                continue;
            }

            let handed = reached.handed(shown.trim()) || reached.handed(said.text.trim());
            let hard = reached.hardcoded(shown.trim()) || reached.hardcoded(said.text.trim());
            let matched = care.matched() || handed;
            let keyed = matched && (handed || hard);

            let offer = match care {
                Care::Called => Offer::Locked,
                _ => Offer::default().or_listed(
                    asked.is_none()
                        || care.held_back(&said.text, shown, reached)
                        || names_a_file(shown)
                        || keyed,
                ),
            };

            found.push(Slot {
                spot: format!("{}/s{which}", piece.spot),
                offer,
                said: asked.unwrap_or_else(|| said.text.clone()),
                whole: said.text.clone(),
                at: said.at.clone(),
                apart: read_apart(piece, which, reached),
                matched,
            });
        }
    }

    found
}

pub fn found_by(pieces: &[Piece], out: &mut Reached) {
    for piece in pieces {
        let Kind::Command { code, .. } = &piece.kind else {
            continue;
        };
        if *code != DB_WRITE {
            continue;
        }

        let Some(said) = piece.said.get(DATA_NAME) else {
            continue;
        };

        let text = said.text.trim();
        if !text.is_empty() {
            out.codes(text);
        }
    }
}

#[derive(Default)]
pub struct Agreed {
    matched: BTreeSet<String>,
    apart: BTreeSet<String>,
    told: BTreeMap<String, (u8, String)>,
}

impl Agreed {
    pub fn saw(&mut self, slots: &[Slot], said: &BTreeMap<String, String>) {
        for slot in slots {
            if !slot.offer.unlocked() {
                continue;
            }

            if slot.matched {
                self.matched.insert(slot.said.clone());
            }
            if slot.apart {
                self.apart.insert(slot.said.clone());
            }

            let Some(fresh) = said.get(&slot.spot) else {
                continue;
            };

            let rank = u8::from(slot.matched);
            if self.told.get(&slot.said).is_none_or(|(was, _)| rank < *was) {
                self.told.insert(slot.said.clone(), (rank, fresh.clone()));
            }
        }
    }

    fn told(&self, said: &str) -> Option<(&str, bool)> {
        if !self.matched.contains(said) {
            return None;
        }

        self.told
            .get(said)
            .map(|(_, text)| (text.as_str(), self.apart.contains(said)))
    }
}

pub fn changed(
    held: &Held,
    slots: &[Slot],
    said: &BTreeMap<String, String>,
    agreed: &Agreed,
    reached: &Reached,
) -> Edits {
    let mut edits = Vec::new();

    for slot in slots {
        let shared = slot
            .offer
            .unlocked()
            .then(|| agreed.told(&slot.said))
            .flatten()
            .map(|(fresh, apart)| (fresh, slot.apart || apart));

        let staged = slot
            .offer
            .unlocked()
            .then(|| said.get(&slot.spot))
            .flatten()
            .map(|fresh| (fresh.as_str(), slot.apart));

        let answer = match slot.matched {
            true => shared.or(staged),
            false => staged.or(shared),
        };

        let Some((fresh, apart)) = answer else {
            continue;
        };

        let fresh = text::shaped(&slot.whole, fresh, reached);
        let fresh = match apart {
            true => fresh.replace(' ', WIDE),
            false => fresh,
        };

        if fresh == slot.whole {
            continue;
        }

        let body = match held.shape {
            Shape::Loose => fresh.into_bytes(),
            _ => coder::line(&fresh),
        };

        edits.push((slot.at.clone(), body));
    }

    edits
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::wolf_rpg::held::Said;
    use crate::engine::wolf_rpg::script;
    use std::slice;

    fn command(code: u32, args: &[u32], said: &[&str]) -> Piece {
        Piece {
            spot: "e0/p0/c0".to_string(),
            kind: Kind::Command {
                code,
                args: args.to_vec(),
            },
            said: said
                .iter()
                .enumerate()
                .map(|(which, text)| Said {
                    text: (*text).to_string(),
                    at: which * 8..which * 8 + 8,
                })
                .collect(),
        }
    }

    fn taken(piece: Piece) -> Vec<String> {
        sift(&[piece], &Reached::new())
            .into_iter()
            .map(|one| one.said)
            .collect()
    }

    fn asked(piece: Piece) -> Vec<String> {
        sift(&[piece], &Reached::new())
            .into_iter()
            .filter(|one| one.offer.asked())
            .map(|one| one.said)
            .collect()
    }

    #[test]
    fn a_command_word_trailed_by_nothing_but_codes_is_held_back_and_a_name_with_a_number_is_not() {
        for one in [
            r"move \cself[7] \cself[41] \cself[20]",
            r"turn \cself[7] \cself[40]",
            "Potion",
            "$sys_",
        ] {
            assert!(
                keylike(one),
                "{one:?} is a word the engine looks up, and the codes after it are its arguments"
            );
        }

        for said in [
            "Musca 2",
            "Manticore 2",
            "Basilisk 1",
            "Hello there",
            r"\c[6]Iron Key\c[0]",
            "Rest here. It costs 10 gold.",
        ] {
            assert!(
                !keylike(said),
                "{said:?} is a name or a sentence a player reads, and holding it back would \
                 leave part of the game in the language it shipped in"
            );
        }
    }

    #[test]
    fn everything_the_game_holds_is_listed_and_only_the_asking_is_narrowed() {
        for one in [
            "SE/[Action]DoorOpen_Komori.ogg",
            "CharaChip/you-A.png",
            "Save/System.sav",
        ] {
            let piece = command(140, &[], &[one]);
            assert_eq!(
                taken(piece),
                [one],
                "{one:?} is a file on disk, and a reader who cannot see it cannot tell whether \
                 anything is missing"
            );
            assert!(
                asked(command(140, &[], &[one])).is_empty(),
                "{one:?} is loaded off disk by that name, so it is never asked about"
            );
        }

        let quiet = command(213, &[], &["\u{79fb}\u{52d5}\u{6642}\u{56de}\u{5fa9}"]);
        assert_eq!(
            taken(quiet).len(),
            1,
            "a label is jumped to by name, and it is listed all the same"
        );
        assert!(
            asked(command(
                213,
                &[],
                &["\u{79fb}\u{52d5}\u{6642}\u{56de}\u{5fa9}"]
            ))
            .is_empty()
        );
    }

    #[test]
    fn a_row_of_settings_the_game_takes_apart_at_its_commas_is_never_asked_about() {
        for one in [
            "\u{6709}\u{3001}13\u{3001}250\u{3001}1\u{3001}6",
            "\u{7121}\u{3001}30\u{3001}10\u{3001}0\u{3001}2",
            "1,0,0,255",
        ] {
            assert!(
                asked(command(SET_STRING, &[], &[one])).is_empty(),
                "{one:?} is read apart at its commas, so a word in place of any part of it \
                 leaves the game parsing a number that is not there"
            );
        }

        for said in [
            concat!(
                "\u{6b8b}\u{7559}\u{306e}\u{6709}\u{7121}\u{3001}EVNo\u{3001}",
                "X\u{ff0b}\u{7bc4}\u{56f2}"
            ),
            "\u{3053}\u{308c}\u{306f}\u{5263}\u{3060}\u{3001}\u{3068}\u{601d}\u{3046}",
            "\u{306f}\u{3044}\u{3001}\u{3044}\u{3044}\u{3048}",
            "\u{3048}\u{3001}\u{3048}\u{3001}\u{3048}",
            "\u{306b}\u{3001}\u{306b}\u{3001}\u{9003}\u{3052}\u{308d}",
        ] {
            assert!(
                !asked(command(SET_STRING, &[], &[said])).is_empty(),
                "{said:?} is speech that happens to hold a comma, and a stutter is not a row \
                 of settings however short its parts are"
            );
        }
    }

    #[test]
    fn a_row_of_names_the_game_glues_a_folder_and_a_suffix_onto_is_never_asked_about() {
        let safe = "\u{5b89}\u{5168}";
        let unknown = "\u{4e0d}\u{660e}";
        let gained = "\u{7d4c}\u{9a13}\u{5024}";
        let risen = "\u{4f53}\u{529b}\u{4e0a}\u{6607}";

        let mut reached = Reached::new();
        for one in [safe, unknown, gained, risen] {
            reached.keeps(one);
        }

        for said in [
            format!("{safe},"),
            format!("{unknown},"),
            format!("{gained},{risen},"),
        ] {
            let row = command(SET_STRING, &[], &[&said]);
            assert!(
                sift(slice::from_ref(&row), &reached)[0].offer != Offer::Asked,
                "{said:?} is glued into a picture path a folder and a suffix at a time, and a \
                 word in place of any part of it leaves the game looking for a file it never \
                 shipped"
            );

            let value = Piece {
                spot: "t25/d7/f8".to_string(),
                kind: Kind::Value,
                said: vec![Said {
                    text: said.clone(),
                    at: 0..16,
                }],
            };
            assert!(
                sift(&[value], &reached)[0].offer != Offer::Asked,
                "and the database column the same row is typed into is where a game keeps a \
                 hundred of them"
            );
        }

        let alone = command(SET_STRING, &[], &[gained]);
        assert!(
            sift(slice::from_ref(&alone), &reached)[0].offer != Offer::Asked,
            "a word standing on its own is held back the same way: a name the game ships a file \
             under costs a picture the game can no longer find, and a word held back is still \
             shown and still settled by hand"
        );

        let plain = command(SET_STRING, &[], &["\u{9060}\u{3044}\u{9053}"]);
        assert!(
            sift(slice::from_ref(&plain), &reached)[0].offer.asked(),
            "and a word the game ships nothing under is a word a player reads"
        );

        let under = command(SET_STRING, &[], &[&format!("CharaChip/{gained}")]);
        assert!(
            sift(slice::from_ref(&under), &reached)[0].offer != Offer::Asked,
            "a row that carries its own folder and waits only on a suffix reaches the same file, \
             so the folder in front of it may not hide the name behind it"
        );

        let drawn = command(MESSAGE, &[], &[gained]);
        assert!(
            sift(slice::from_ref(&drawn), &reached)[0].offer.asked(),
            "a box draws the word it holds and never looks a file up by it, so the same spelling \
             in a message is asked about as plainly as any other line"
        );
    }

    #[test]
    fn a_sound_the_game_loads_is_still_told_apart_when_its_settings_ride_along() {
        let said = "SystemFile/[SE]MenuOpen.ogg\n80\n108";

        assert!(
            asked(command(SET_STRING, &[], &[said])).is_empty(),
            "the rows below the name are a volume and a pitch, and once they are set aside what \
             is left is a file on disk, so it may never be offered however many rows it arrived on"
        );
    }

    #[test]
    fn an_order_the_engine_reads_is_kept_out_of_what_a_translator_is_shown() {
        let said = concat!(
            "@1\n\u{300c}",
            "\\cself[8]",
            "\u{300d}\u{3092}\u{5931}\u{3063}\u{305f}\u{3002}"
        );

        assert_eq!(
            taken(command(MESSAGE, &[], &[said])),
            [concat!(
                "\u{300c}",
                "\\cself[8]",
                "\u{300d}\u{3092}\u{5931}\u{3063}\u{305f}\u{3002}"
            )],
            "the first row tells the engine how to draw the box, and a model that rewrites it \
             stops the box being drawn at all"
        );
    }

    #[test]
    fn a_box_of_nothing_but_orders_and_values_is_still_listed_but_never_asked_about() {
        let said = concat!("@standby\n", "\\cself[10]", "\n", "\\cself[5]");

        assert_eq!(
            taken(command(MESSAGE, &[], &[said])),
            [said],
            "whoever reads the rail is shown every line the game holds, spelled as the game \
             spells it"
        );
        assert!(
            asked(command(MESSAGE, &[], &[said])).is_empty(),
            "there is not a word in it, so there is nothing to pay a model for and nothing that \
             could come back safely"
        );
    }

    #[test]
    fn what_the_player_reads_is_asked_about_and_what_the_engine_reads_is_never_asked_about() {
        assert_eq!(
            asked(command(
                MESSAGE,
                &[],
                &["\u{300c}\u{3053}\u{308c}\u{306f}\u{5263}\u{3060}"]
            )),
            ["\u{300c}\u{3053}\u{308c}\u{306f}\u{5263}\u{3060}"]
        );
        assert_eq!(
            asked(command(CHOICES, &[0], &["Train", "\u{4f11}\u{3080}"])),
            ["Train", "\u{4f11}\u{3080}"]
        );

        assert!(
            asked(command(
                103,
                &[],
                &["\u{2015}\u{2015}\u{2015} note to self"]
            ))
            .is_empty(),
            "a comment is written for whoever opens the editor and never shown to a player"
        );
        assert!(
            asked(command(
                106,
                &[],
                &["\u{25a0}1.4\u{6539}\u{4fee}\u{7b87}\u{6240}"]
            ))
            .is_empty(),
            "a debug message is not in the game the player runs"
        );
        assert!(
            asked(command(213, &[], &["A"])).is_empty(),
            "a label is jumped to by name, and renaming half of the pair breaks the jump"
        );
        assert!(
            asked(command(
                DB_WRITE,
                &[],
                &[
                    "",
                    "\u{30a2}\u{30a4}\u{30c6}\u{30e0}",
                    "\u{85ac}\u{8349}",
                    ""
                ]
            ))
            .is_empty(),
            "the second and third words name the type and the row this reaches into, and \
             translating either finds nothing"
        );
        assert_eq!(
            asked(command(
                DB_WRITE,
                &[],
                &["\u{4f55}\u{3082}\u{3057}\u{306a}\u{3044}", "", "", ""]
            )),
            ["\u{4f55}\u{3082}\u{3057}\u{306a}\u{3044}"],
            "the first word is the value being written in, and a menu draws it for the player"
        );
    }

    #[test]
    fn a_string_the_game_branches_on_is_asked_about_and_an_order_in_one_is_not() {
        for one in ["@mes", "$sys_", "<<ERROR>>", "!="] {
            assert!(
                asked(command(STRING_CONDITION, &[], &[one])).is_empty(),
                "{one:?} is an order the engine reads, not a word, so there is nothing to ask"
            );
        }

        assert_eq!(
            asked(command(STRING_CONDITION, &[], &["\u{306f}\u{3044}"])),
            ["\u{306f}\u{3044}"],
            "the choice this is matched against is carried over, so leaving this half behind is \
             what turns the branch off, and both halves move together"
        );

        assert_eq!(
            asked(command(
                MESSAGE,
                &[],
                &["ERROR: @mes_effect is not an argument"]
            )),
            ["ERROR: @mes_effect is not an argument"],
            "a message that names a command is shown to the player, so it is still carried over"
        );
    }

    #[test]
    fn a_value_the_engine_reads_as_an_order_is_never_asked_about_plainly() {
        for one in [
            "<<GET_FILE_EXIST>>data/\\cself[5]",
            "<<BLANK>>",
            "<<NotFound>>",
        ] {
            assert!(
                asked(command(SET_STRING, &[], &[one])).is_empty(),
                "{one:?} is an order to the engine, and carrying it over stops the order being \
                 understood"
            );
        }

        for one in [".png", ".ogg", ".sav", "Data/", "Data\\", "Save/"] {
            assert!(
                asked(command(SET_STRING, &[], &[one])).is_empty(),
                "{one:?} is glued onto a file name, and carrying it over loses the file"
            );
        }

        for one in ["BL", "GM-MN", "SUR", "Base"] {
            assert!(
                asked(command(SET_STRING, &[], &[one])).is_empty(),
                "{one:?} stands in for a name the game looks things up by"
            );
        }

        assert_eq!(
            asked(command(SET_STRING, &[], &["\u{7b2c}\\cself[65]\u{7ae0}"])),
            ["\u{7b2c}\\cself[65]\u{7ae0}"],
            "a line the player reads is still asked about, markup and all"
        );
    }

    #[test]
    fn a_message_ending_in_a_file_name_is_still_a_message_the_player_reads() {
        assert_eq!(
            asked(command(
                MESSAGE,
                &[],
                &["\u{30c7}\u{30fc}\u{30bf}\u{306f}save01.sav\u{306b}\u{66f8}\u{3044}\u{305f}"]
            )),
            ["\u{30c7}\u{30fc}\u{30bf}\u{306f}save01.sav\u{306b}\u{66f8}\u{3044}\u{305f}"],
            "the box shows this line to the player, so naming a file inside it changes nothing"
        );
        assert!(
            asked(command(SET_STRING, &[], &["Picture/save01.png"])).is_empty(),
            "a value naming a file is looked up by that name, and translating it finds nothing"
        );
    }

    #[test]
    fn an_english_message_is_still_a_message_whatever_language_it_was_written_in() {
        assert_eq!(
            asked(command(MESSAGE, &[], &["The door is locked tight."])),
            ["The door is locked tight."],
            "a game already carried into one language has to be carriable into the next"
        );
    }

    #[test]
    fn a_picture_is_taken_only_when_it_draws_words_rather_than_loading_a_file() {
        assert_eq!(
            asked(command(PICTURE, &[0x20, 1], &[r"\f[36]Defeat"])),
            [r"\f[36]Defeat"],
            "type 2 in the flags means the picture is text the engine draws"
        );
        assert!(
            asked(command(PICTURE, &[0x00, 1], &["Picture/title01.jpg"])).is_empty(),
            "type 0 is a file, and the same string slot holds its name"
        );
    }

    #[test]
    fn a_name_a_database_reach_spells_out_is_held_back_and_ordinary_words_are_not() {
        let layer = "\u{753b}\u{9762}\u{30d1}\u{30fc}\u{30c6}\u{30a3}\u{30af}\u{30eb}";
        let plain = "\u{304a}\u{3082}\u{3061}\u{3083}\u{306e}\u{30ea}\u{30f3}\u{30b4}";

        let mut reached = Reached::new();
        found_by(
            &[command(DB_WRITE, &[], &["", "41", layer, ""])],
            &mut reached,
        );

        assert!(
            reached.hardcoded(layer),
            "the game says \"the data name does not exist\" when this row is renamed, so the \
             reach itself is what tells us to leave it alone"
        );
        assert!(!reached.hardcoded(plain));

        assert_eq!(
            sift(&[command(SET_STRING, &[], &[layer])], &reached)
                .into_iter()
                .map(|one| !one.offer.asked())
                .collect::<Vec<bool>>(),
            [true]
        );
        assert_eq!(
            sift(&[command(SET_STRING, &[], &[plain])], &reached)
                .into_iter()
                .map(|one| !one.offer.asked())
                .collect::<Vec<bool>>(),
            [false],
            "a word no reach spells out is a word a player reads"
        );
    }

    #[test]
    fn only_the_row_a_reach_names_becomes_a_key_and_the_column_it_names_does_not() {
        let row = "\u{753b}\u{9762}\u{30d1}\u{30fc}\u{30c6}\u{30a3}\u{30af}\u{30eb}";
        let column = "\u{30b5}\u{30ad}";

        let mut reached = Reached::new();
        found_by(
            &[command(DB_WRITE, &[], &["", "41", row, column])],
            &mut reached,
        );

        assert!(
            reached.hardcoded(row),
            "a row is named inside the data file, so renaming it here without renaming it there \
             leaves the reach looking for a row that is gone"
        );
        assert!(
            !reached.hardcoded(column),
            "a column is named in the plan beside the data, which is never written back, so the \
             reach and the plan still spell it the same and every other line holding this word is \
             free"
        );
    }

    #[test]
    fn a_string_the_engine_compares_is_carried_over_with_the_line_it_is_matched_against() {
        let found = sift(
            &[command(
                STRING_CONDITION,
                &[],
                &["\u{5f37}\u{904b}\u{306e}", "\u{306f}\u{3044}"],
            )],
            &Reached::new(),
        );

        assert_eq!(found.len(), 2);
        assert!(
            found.iter().all(|one| one.offer.asked() && one.matched),
            "both halves of a comparison are words, and the way to keep the branch working is to \
             carry them over together, not to leave them behind"
        );

        assert!(
            sift(
                &[command(STRING_CONDITION, &[], &["Dauntless"])],
                &Reached::new()
            )[0]
            .offer
                != Offer::Asked,
            "a lone ASCII word is still the sort of name the engine looks things up by"
        );
    }

    #[test]
    fn a_line_the_engine_matches_reads_the_same_wherever_the_game_spells_it() {
        let yes = "\u{306f}\u{3044}";

        let held = |kind: Piece| Held {
            plain: Vec::new(),
            shape: Shape::Loose,
            pieces: vec![kind],
        };

        let choice = held(command(CHOICES, &[0], &[yes]));
        let branch = held(command(STRING_CONDITION, &[], &[yes]));

        let mut agreed = Agreed::default();
        let reached = Reached::new();

        agreed.saw(
            &sift(&branch.pieces, &reached),
            &BTreeMap::from([("e0/p0/c0/s0".to_string(), "Yes indeed".to_string())]),
        );
        agreed.saw(
            &sift(&choice.pieces, &reached),
            &BTreeMap::from([("e0/p0/c0/s0".to_string(), "Yes".to_string())]),
        );

        assert_eq!(
            agreed.told(yes),
            Some(("Yes", false)),
            "the button the player reads is where this line was translated with something around \
             it, so that is the wording both halves take"
        );

        let written = |one: &Held| {
            String::from_utf8_lossy(
                &changed(
                    one,
                    &sift(&one.pieces, &reached),
                    &BTreeMap::new(),
                    &agreed,
                    &reached,
                )[0]
                .1,
            )
            .to_string()
        };

        assert_eq!(written(&branch), "Yes");
        assert_eq!(
            written(&choice),
            "Yes",
            "one wording, laid into every place the game spells the same line, or the branch \
             stops matching the button"
        );
    }

    #[test]
    fn a_name_a_script_hands_to_its_commands_is_held_back_in_every_spelling_of_it() {
        let who = "\u{30a2}\u{30ad}\u{30e9}";

        let mut reached = Reached::new();
        script::taken_apart(
            "@setface \u{30a2}\u{30ad}\u{30e9} tatie/blank\r\n".as_bytes(),
            &mut reached,
        );

        let token = command(SPEAKER, &[], &[who]);
        let display = Piece {
            spot: "t27/d5/f3".to_string(),
            kind: Kind::Value,
            said: vec![Said {
                text: who.to_string(),
                at: 0..16,
            }],
        };

        assert!(
            sift(&[token], &reached)[0].offer != Offer::Asked,
            "the game matches this token against a portrait registry filled by event code \
             nothing here can rewrite, so a translated token is a portrait it can no longer find"
        );
        assert!(
            sift(&[display], &reached)[0].offer != Offer::Asked,
            "and the display field feeding that registry keeps the same spelling, or the two \
             sides of the match drift apart"
        );

        assert!(
            sift(&[command(PICKED, &[], &["\u{306f}\u{3044}"])], &reached)[0]
                .offer
                .asked(),
            "a choice is handed to no command, so it is still asked about"
        );
    }

    #[test]
    fn a_name_a_database_reach_spells_out_is_kept_in_the_token_and_still_drawn_in_the_box() {
        let who = "\u{30ea}\u{30f3}";

        let mut reached = Reached::new();
        reached.codes(who);

        let token = command(SPEAKER, &[], &[who]);
        let display = Piece {
            spot: "t27/d3/f3".to_string(),
            kind: Kind::Value,
            said: vec![Said {
                text: who.to_string(),
                at: 0..16,
            }],
        };

        assert!(
            sift(&[token], &reached)[0].offer != Offer::Asked,
            "the box is opened by this name, and a reach spells the same name out inside the \
             data, so the token the event code writes has to keep it"
        );
        assert!(
            sift(&[display], &reached)[0].offer.asked(),
            "the field the box actually draws is a different field from the one the reach names, \
             so holding it back too is what leaves every speaker in the language the game shipped \
             in with nothing gained"
        );
    }

    #[test]
    fn a_choice_the_model_translated_carries_the_branch_that_compares_against_it() {
        let picked = "Yes_1";

        let held = |piece: Piece| Held {
            plain: Vec::new(),
            shape: Shape::Loose,
            pieces: vec![piece],
        };

        let choice = held(command(CHOICES, &[0], &[picked]));
        let branch = held(command(STRING_CONDITION, &[], &[picked]));

        let reached = Reached::new();

        assert!(
            sift(&choice.pieces, &reached)[0].offer.asked(),
            "the player reads a choice, so it is always asked about"
        );
        assert!(
            !sift(&branch.pieces, &reached)[0].offer.asked(),
            "and a rule guesses the branch compares against a name, which is the whole setup"
        );

        let said = BTreeMap::from([("e0/p0/c0/s0".to_string(), "Hai_1".to_string())]);

        let mut agreed = Agreed::default();
        agreed.saw(&sift(&choice.pieces, &reached), &said);
        agreed.saw(&sift(&branch.pieces, &reached), &said);

        assert_eq!(
            String::from_utf8_lossy(
                &changed(
                    &branch,
                    &sift(&branch.pieces, &reached),
                    &BTreeMap::new(),
                    &agreed,
                    &reached
                )[0]
                .1
            ),
            "Hai_1",
            "the choice went into the game translated, so a branch left comparing against what \
             the choice used to say can never match again and the story stops there"
        );

        let byhand = BTreeMap::from([("e0/p0/c0/s0".to_string(), "Sou_1".to_string())]);

        assert_eq!(
            String::from_utf8_lossy(
                &changed(
                    &branch,
                    &sift(&branch.pieces, &reached),
                    &byhand,
                    &agreed,
                    &reached
                )[0]
                .1
            ),
            "Hai_1",
            "and a wording typed straight into the branch loses to the one the choice took, \
             because the option on screen is the one the player picks: settling one side of a \
             comparison on its own can only build a branch that never fires again"
        );
    }

    #[test]
    fn a_line_no_player_reads_takes_the_wording_the_reader_gave_it_wherever_it_turns_up() {
        let picked = "Yes_1";

        let branch = Held {
            plain: Vec::new(),
            shape: Shape::Loose,
            pieces: vec![command(STRING_CONDITION, &[], &[picked])],
        };
        let label = Held {
            plain: Vec::new(),
            shape: Shape::Loose,
            pieces: vec![command(SPEAKER, &[], &[picked])],
        };

        let reached = Reached::new();
        let byhand = BTreeMap::from([("e0/p0/c0/s0".to_string(), "Sou_1".to_string())]);

        let mut agreed = Agreed::default();
        agreed.saw(&sift(&branch.pieces, &reached), &byhand);
        agreed.saw(&sift(&label.pieces, &reached), &BTreeMap::new());

        assert_eq!(
            String::from_utf8_lossy(
                &changed(
                    &label,
                    &sift(&label.pieces, &reached),
                    &BTreeMap::new(),
                    &agreed,
                    &reached
                )[0]
                .1
            ),
            "Sou_1",
            "no line here was ever sent to the model, so what the reader typed is the only \
             wording there is, and every place the game compares this string has to carry it"
        );
    }

    #[test]
    fn a_line_read_apart_in_one_place_is_written_the_same_way_in_all_of_them() {
        let narration = "\u{5730}\u{306e}\u{6587}2";

        let held = |piece: Piece| Held {
            plain: Vec::new(),
            shape: Shape::Loose,
            pieces: vec![piece],
        };

        let token = held(command(SPEAKER, &[], &[narration]));
        let branch = held(command(STRING_CONDITION, &[], &[narration]));

        let reached = Reached::new();
        let said = BTreeMap::from([("e0/p0/c0/s0".to_string(), "Narration two".to_string())]);

        let mut agreed = Agreed::default();
        agreed.saw(&sift(&token.pieces, &reached), &said);
        agreed.saw(&sift(&branch.pieces, &reached), &said);

        let written = |one: &Held| {
            String::from_utf8_lossy(
                &changed(
                    one,
                    &sift(&one.pieces, &reached),
                    &BTreeMap::new(),
                    &agreed,
                    &reached,
                )[0]
                .1,
            )
            .to_string()
        };

        assert_eq!(
            written(&token),
            "Narration\u{3000}two",
            "the script reads this line apart at its spaces, so it goes in wide"
        );
        assert_eq!(
            written(&branch),
            "Narration\u{3000}two",
            "and the line it is matched against has to be spelled the same way, wide space and \
             all, or the two stop being the same line"
        );
    }

    #[test]
    fn a_value_a_script_splices_into_a_command_line_is_written_with_wide_spaces() {
        let held = Held {
            plain: Vec::new(),
            shape: Shape::Loose,
            pieces: vec![command(
                SET_STRING,
                &[script::STRINGS_FROM + 7],
                &["\u{4eba}\u{9593}"],
            )],
        };

        let said = BTreeMap::from([("e0/p0/c0/s0".to_string(), "The human".to_string())]);

        let mut reached = Reached::new();
        assert_eq!(
            String::from_utf8_lossy(
                &changed(
                    &held,
                    &sift(&held.pieces, &reached),
                    &said,
                    &Agreed::default(),
                    &reached
                )[0]
                .1
            ),
            "The human",
            "a value no script hands to a command line is left as it was written"
        );

        script::taken_apart(b"@setface \\s[7] tatie/blank W\r\n", &mut reached);

        assert_eq!(
            String::from_utf8_lossy(
                &changed(
                    &held,
                    &sift(&held.pieces, &reached),
                    &said,
                    &Agreed::default(),
                    &reached
                )[0]
                .1
            ),
            "The\u{3000}human",
            "the script splices this variable into a line the engine reads apart at its spaces, \
             so a space inside the value shifts every argument after it along one, and the wide \
             space is the only one this engine does not read as one"
        );
    }

    #[test]
    fn a_value_of_several_rows_is_no_argument_however_the_script_reads_the_variable() {
        let held = Held {
            plain: Vec::new(),
            shape: Shape::Loose,
            pieces: vec![command(
                SET_STRING,
                &[script::STRINGS_FROM + 46],
                &["\u{9b54}\u{738b}\n\u{30df}\u{30ca}"],
            )],
        };

        let said = BTreeMap::from([(
            "e0/p0/c0/s0".to_string(),
            "The demon lord\nMina".to_string(),
        )]);

        let mut reached = Reached::new();
        script::taken_apart(b"@mes \\s[46] talk\r\n", &mut reached);

        let written = String::from_utf8_lossy(
            &changed(
                &held,
                &sift(&held.pieces, &reached),
                &said,
                &Agreed::default(),
                &reached,
            )[0]
            .1,
        )
        .to_string();

        assert!(
            written.contains(' ') && !written.contains(WIDE),
            "the engine reads a script one row at a time, so a value the game wrote across rows \
             was never standing in for one argument, and holding its words together would only \
             show up as gaps on screen: {written:?}"
        );
    }

    #[test]
    fn the_field_a_row_is_found_by_is_held_back_and_the_name_beside_it_is_still_asked_about() {
        let who = "\u{30b5}\u{30ad}";

        let mut reached = Reached::new();
        reached.codes(who);

        let held = |kind: Kind| Piece {
            spot: "t27/d4/f0".to_string(),
            kind,
            said: vec![Said {
                text: who.to_string(),
                at: 0..12,
            }],
        };

        assert!(
            sift(&[held(Kind::Naming)], &reached)[0].offer != Offer::Asked,
            "the event code finds this row by this word, and renaming it here leaves the code \
             looking for a row that is no longer there"
        );
        assert!(
            sift(&[held(Kind::Value)], &reached)[0].offer.asked(),
            "the same word standing in a field nobody finds a row by is the name the box draws, \
             and holding it back is what leaves every speaker in the language the game shipped in"
        );
    }

    #[test]
    fn a_database_value_keeps_the_field_it_came_from_and_is_asked_about_plainly() {
        let held = Piece {
            spot: "t2/d0/f1".to_string(),
            kind: Kind::Value,
            said: vec![Said {
                text: "Heals 30 HP.".to_string(),
                at: 0..16,
            }],
        };

        let found = sift(&[held], &Reached::new());

        assert_eq!(found[0].spot, "t2/d0/f1/s0");
        assert!(
            found[0].offer.asked(),
            "an item's description is the plainest text in the game"
        );
    }

    #[test]
    fn a_name_the_engine_calls_an_event_by_is_locked_and_no_staged_line_reaches_it() {
        let called = command(CALL_BY_NAME, &[], &["\u{56de}\u{5fa9}\u{51e6}\u{7406}"]);

        assert_eq!(
            sift(slice::from_ref(&called), &Reached::new())[0].offer,
            Offer::Locked,
            "the command code itself says this word is the name the engine looks the event up by, \
             so this is not a guess to be overruled: renaming it leaves the call finding nothing"
        );

        let held = Held {
            plain: Vec::new(),
            shape: Shape::Loose,
            pieces: vec![called],
        };
        let said = BTreeMap::from([("e0/p0/c0/s0".to_string(), "Healing".to_string())]);

        assert!(
            changed(
                &held,
                &sift(&held.pieces, &Reached::new()),
                &said,
                &Agreed::default(),
                &Reached::new()
            )
            .is_empty(),
            "and a line staged against it by hand is refused here too, not only at the sheet"
        );
    }

    #[test]
    fn a_line_left_as_the_game_wrote_it_is_never_written_back() {
        let held = Held {
            plain: vec![0; 32],
            shape: Shape::Plain,
            pieces: vec![
                Piece {
                    spot: "title".to_string(),
                    kind: Kind::Title,
                    said: vec![Said {
                        text: "the same".to_string(),
                        at: 0..12,
                    }],
                },
                Piece {
                    spot: "titlePlus".to_string(),
                    kind: Kind::Title,
                    said: vec![Said {
                        text: "the same".to_string(),
                        at: 12..24,
                    }],
                },
            ],
        };

        let said = BTreeMap::from([
            ("title/s0".to_string(), "the same".to_string()),
            ("titlePlus/s0".to_string(), "different".to_string()),
            ("nowhere/s0".to_string(), "different".to_string()),
        ]);

        let edits = changed(
            &held,
            &sift(&held.pieces, &Reached::new()),
            &said,
            &Agreed::default(),
            &Reached::new(),
        );

        assert_eq!(edits.len(), 1, "only the line that changed is spliced");
        assert_eq!(edits[0].0, 12..24);
        assert_eq!(
            coder::Reader::over(&edits[0].1, 0)
                .said()
                .expect("it reads back")
                .0,
            "different"
        );
    }

    #[test]
    fn the_font_game_dat_names_is_not_a_line_anybody_translates() {
        let held = Piece {
            spot: "fonts".to_string(),
            kind: Kind::Font,
            said: vec![Said {
                text: "Pixelify Sans".to_string(),
                at: 0..16,
            }],
        };

        assert!(sift(&[held], &Reached::new()).is_empty());
    }
}
