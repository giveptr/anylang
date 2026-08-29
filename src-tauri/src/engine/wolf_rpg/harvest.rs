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
    pub drawn: bool,
    pub matched: bool,
}

#[derive(Clone, Copy)]
enum Care {
    Shown,
    Chosen,
    Handed,
    Label,
    Engine,
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

fn a_shipped_name(one: &str, reached: &Reached) -> bool {
    reached.kept(one) || reached.a_part(one)
}

fn shipped_names(text: &str, reached: &Reached) -> bool {
    let mut parts = text
        .split(APART)
        .map(str::trim)
        .filter(|one| !one.is_empty())
        .peekable();

    parts.peek().is_some()
        && parts.all(|one| {
            a_shipped_name(one, reached)
                || under_a_folder(one).is_some_and(|tail| a_shipped_name(tail, reached))
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
    fn locked(self) -> bool {
        matches!(self, Self::Called | Self::Handed)
    }

    fn drawn(self) -> bool {
        matches!(self, Self::Shown | Self::Chosen)
    }

    fn reads_a_name(self) -> bool {
        self.keys() && self.rank().is_none()
    }

    fn anywhere(_: Care) -> bool {
        true
    }

    fn homes(self) -> bool {
        self.rank().is_some() || self.keys()
    }

    fn held_back(self, whole: &str, outside: bool, read_them_all: bool) -> bool {
        match self {
            Self::Shown => false,
            Self::Chosen => outside,
            Self::Called | Self::Handed | Self::Engine => true,
            Self::Label | Self::NamedBy => for_the_engine(whole) || outside || !read_them_all,
            Self::Compared | Self::Names | Self::Plain => for_the_engine(whole) || outside,
        }
    }

    fn rank(self) -> Option<u8> {
        match self {
            Self::NamedBy => Some(0),
            Self::Names => Some(1),
            Self::Label => Some(2),
            Self::Plain => Some(3),
            _ => None,
        }
    }

    fn keys(self) -> bool {
        matches!(
            self,
            Self::Called | Self::Handed | Self::Engine | Self::Compared | Self::NamedBy
        )
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
        Kind::Called => Some(Care::Called),
        Kind::Handed => Some(Care::Handed),
        Kind::Title => Some(Care::Shown),
        Kind::Value => Some(Care::Plain),
        Kind::Naming => Some(Care::NamedBy),
        Kind::Command { code, args } => Some(match (*code, which) {
            (MESSAGE, _) => Care::Shown,
            (CHOICES | PICKED | PROMPT, _) => Care::Chosen,
            (SPEAKER, _) => Care::Label,
            (PICTURE, 0) if draws_text(args) => Care::Shown,
            (SET_STRING | DB_WRITE, 0) => Care::Names,
            (CALL_BY_NAME, 0) => Care::Called,
            (CALL_BY_NAME, _) => Care::Names,
            (STRING_CONDITION, _) => Care::Compared,
            _ => Care::Engine,
        }),
    }
}

fn read_apart(piece: &Piece, which: usize, reached: &Reached) -> bool {
    match &piece.kind {
        Kind::Handed => true,
        Kind::Command { code, .. } if ARGUMENTS.contains(code) => true,
        Kind::Command { code, args } if (*code, which) == (SET_STRING, 0) => {
            args.first().is_some_and(|held| reached.read_apart(*held))
                && !piece.said[which].text.contains('\n')
        }
        _ => false,
    }
}

struct Reading<'a> {
    care: Care,
    shown: String,
    carried: bool,
    piece: &'a Piece,
    which: usize,
    spot: String,
}

fn each_said(
    pieces: &[Piece],
    reached: &Reached,
    wants: fn(Care) -> bool,
    mut take: impl FnMut(Reading<'_>),
) {
    for piece in pieces {
        for (which, said) in piece.said.iter().enumerate() {
            let Some(care) = care_of(piece, which) else {
                continue;
            };

            if !wants(care) {
                continue;
            }

            let asked = text::asked(&said.text, reached);
            let carried = asked.is_none();
            let shown = asked.unwrap_or_else(|| said.text.clone());

            if !text::has_words(&shown) {
                continue;
            }

            take(Reading {
                care,
                carried,
                shown,
                piece,
                which,
                spot: format!("{}/s{which}", piece.spot),
            });
        }
    }
}

pub fn sift(pieces: &[Piece], named: &str, reached: &Reached) -> Vec<Slot> {
    let mut found = Vec::new();

    each_said(pieces, reached, Care::anywhere, |held| {
        let piece = held.piece;
        let said = &piece.said[held.which];
        let shown = held.shown.as_str();
        let care = held.care;

        let handed = reached.handed(shown.trim()) || reached.handed(said.text.trim());
        let hard = reached.hardcoded(shown.trim()) || reached.hardcoded(said.text.trim());

        let planned = reached.a_plan_name(shown);
        let outside = planned || shipped_names(shown, reached);
        let home = reached.at_home(shown, named, &held.spot);
        let matched = care.matched() || handed || (home && !outside);
        let away = !care.drawn() && reached.a_name(shown) && reached.written_down(shown) && !home;
        let keyed = matched && (handed || hard) && !home;

        let out_of_sight = care.locked() || away || (care.reads_a_name() && planned);

        let offer = match out_of_sight {
            true => Offer::Locked,
            false => Offer::default().or_listed(
                held.carried
                    || care.held_back(&said.text, outside, reached.read_them_all())
                    || names_a_file(shown)
                    || keyed,
            ),
        };

        let whole = said.text.clone();
        let at = said.at.clone();
        let apart = read_apart(piece, held.which, reached);

        found.push(Slot {
            spot: held.spot,
            said: held.shown,
            offer,
            whole,
            at,
            apart,
            drawn: care.drawn(),
            matched,
        });
    });

    found
}

struct Home {
    care: Care,
    text: String,
    spot: String,
}

fn homes_of(pieces: &[Piece], reached: &Reached) -> Vec<Home> {
    let mut out = Vec::new();

    each_said(pieces, reached, Care::homes, |held| {
        out.push(Home {
            care: held.care,
            text: held.shown,
            spot: held.spot,
        });
    });

    out
}

pub fn homes_in(pieces: &[Piece], named: &str, out: &mut Reached) {
    for home in homes_of(pieces, out) {
        if home.care.keys() {
            out.keyed_by(&home.text);
        }

        if let Some(rank) = home.care.rank() {
            out.homing(&home.text, rank, named, &home.spot);
        }
    }
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
            if slot.apart {
                self.apart.insert(slot.said.clone());
            }

            if slot.matched {
                self.matched.insert(slot.said.clone());
            }

            if !slot.offer.unlocked() {
                continue;
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
        let shared = agreed
            .told(&slot.said)
            .map(|(fresh, apart)| (fresh, slot.apart || (apart && !slot.drawn)));

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
        sift(&[piece], "", &Reached::new())
            .into_iter()
            .map(|one| one.said)
            .collect()
    }

    fn asked(piece: Piece) -> Vec<String> {
        sift(&[piece], "", &Reached::new())
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
                sift(slice::from_ref(&row), "", &reached)[0].offer != Offer::Asked,
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
                sift(&[value], "", &reached)[0].offer != Offer::Asked,
                "and the database column the same row is typed into is where a game keeps a \
                 hundred of them"
            );
        }

        let alone = command(SET_STRING, &[], &[gained]);
        assert!(
            sift(slice::from_ref(&alone), "", &reached)[0].offer != Offer::Asked,
            "a word standing on its own is held back the same way: a name the game ships a file \
             under costs a picture the game can no longer find, and a word held back is still \
             shown and still settled by hand"
        );

        let plain = command(SET_STRING, &[], &["\u{9060}\u{3044}\u{9053}"]);
        assert!(
            sift(slice::from_ref(&plain), "", &reached)[0].offer.asked(),
            "and a word the game ships nothing under is a word a player reads"
        );

        let under = command(SET_STRING, &[], &[&format!("CharaChip/{gained}")]);
        assert!(
            sift(slice::from_ref(&under), "", &reached)[0].offer != Offer::Asked,
            "a row that carries its own folder and waits only on a suffix reaches the same file, \
             so the folder in front of it may not hide the name behind it"
        );

        let drawn = command(MESSAGE, &[], &[gained]);
        assert!(
            sift(slice::from_ref(&drawn), "", &reached)[0].offer.asked(),
            "a box draws the word it holds and never looks a file up by it, so the same spelling \
             in a message is asked about as plainly as any other line"
        );

        let branch = command(STRING_CONDITION, &[], &[gained]);
        assert_eq!(
            sift(slice::from_ref(&branch), "", &reached)[0].offer,
            Offer::Listed,
            "a file lying beside the game under the same spelling is a resemblance and not a \
             ruling, so a branch comparing the word is laid in front of the reader rather than \
             kept out of sight: only what the format itself settles is ever locked"
        );
    }

    #[test]
    fn the_wide_space_a_command_needs_is_not_laid_into_the_box_that_only_draws_the_name() {
        let who = "\u{4eba}\u{9593} \u{5175}";

        let piece = |spot: &str, kind: Kind| Piece {
            spot: spot.to_string(),
            kind,
            said: vec![Said {
                text: who.to_string(),
                at: 0..16,
            }],
        };
        let held = |piece: Piece| Held {
            plain: Vec::new(),
            shape: Shape::Loose,
            pieces: vec![piece],
        };

        let mut reached = Reached::new();
        reached.hands(who);

        let handed = held(piece("l3/a1", Kind::Handed));
        let drawn = held(command(MESSAGE, &[], &[who]));

        let said = BTreeMap::from([("e0/p0/c0/s0".to_string(), "Human Soldier".to_string())]);
        let mut agreed = Agreed::default();
        agreed.saw(&sift(&handed.pieces, "", &reached), &said);
        agreed.saw(&sift(&drawn.pieces, "", &reached), &said);

        let laid = |one: &Held| {
            String::from_utf8_lossy(
                &changed(
                    one,
                    &sift(&one.pieces, "", &reached),
                    &said,
                    &agreed,
                    &reached,
                )[0]
                .1,
            )
            .into_owned()
        };

        assert_eq!(
            laid(&handed),
            "Human\u{3000}Soldier",
            "a script reads its orders one space at a time, so the token it hands over is spelled \
             with the space the engine will not break on"
        );
        assert_eq!(
            laid(&drawn),
            "Human Soldier",
            "but the box only draws the name, and widening its space to suit a command the box \
             never reaches leaves the player looking at a gap nobody asked for"
        );
    }

    #[test]
    fn a_button_spelling_a_name_the_game_looks_up_by_is_held_back_and_a_box_saying_it_is_not() {
        let kind = "\u{30a2}\u{30a4}\u{30c6}\u{30e0}";
        let quit = "\u{3084}\u{3081}\u{308b}";

        let mut reached = Reached::new();
        reached.plans(kind);

        let menu = command(CHOICES, &[], &[kind, quit]);
        let picked = sift(slice::from_ref(&menu), "", &reached);

        assert_eq!(
            picked[0].offer,
            Offer::Listed,
            "the game hands back whichever button was pressed and reaches a database type by \
             that very word, so a wording of its own would send the engine after a type the \
             plan never spelled out"
        );
        assert!(
            picked[1].offer.asked(),
            "and the button beside it names nothing the plan spells out, so it is asked about \
             like any other word the player reads"
        );

        let box_of = command(MESSAGE, &[], &[kind]);
        assert!(
            sift(slice::from_ref(&box_of), "", &reached)[0]
                .offer
                .asked(),
            "a box hands nothing back: it draws the word and the word goes nowhere, so the same \
             spelling in a message is a line to read like any other"
        );
    }

    #[test]
    fn a_word_the_game_glues_onto_the_end_of_a_picture_name_is_held_back_standing_alone() {
        let mood = "\u{666e}\u{901a}";
        let who = "\u{30ec}\u{30aa}\u{30eb}";

        let mut reached = Reached::new();
        for stem in [
            format!("{who}_{mood}"),
            format!("chara_{who}"),
            "face_Normal".to_string(),
        ] {
            reached.keeps(&stem);
            reached.ships(stem.split_once('_').expect("a stem with a tail").1);
        }

        let spelled = command(SET_STRING, &[], &["normal"]);
        assert!(
            sift(slice::from_ref(&spelled), "", &reached)[0].offer != Offer::Asked,
            "the game looks its files up on a disk that reads Normal and normal as one name, so a \
             tail spelled either way reaches the same picture and is held back either way"
        );

        for said in [mood, who] {
            let row = command(SET_STRING, &[], &[said]);
            assert!(
                sift(slice::from_ref(&row), "", &reached)[0].offer != Offer::Asked,
                "{said:?} is the tail the game glues onto a face to spell the picture it draws, \
                 and a word in its place leaves the game looking for a face no file answers to"
            );
        }

        let drawn = command(MESSAGE, &[], &[mood]);
        assert!(
            sift(slice::from_ref(&drawn), "", &reached)[0].offer.asked(),
            "the same word inside a box is drawn rather than looked up, so it is asked about as \
             plainly as any other line the player reads"
        );
    }

    #[test]
    fn a_speaker_is_held_back_once_a_script_this_reader_could_not_open_may_have_named_it() {
        let who = "\u{5730}\u{306e}\u{6587}1";
        let token = command(SPEAKER, &[], &[who]);

        assert!(
            sift(slice::from_ref(&token), "", &Reached::new())[0]
                .offer
                .asked(),
            "a box draws the name it is headed by, so with every script read there is nothing \
             left that could have named this one a key"
        );

        let mut blind = Reached::new();
        blind.missed_a_script();

        assert!(
            sift(slice::from_ref(&token), "", &blind)[0].offer != Offer::Asked,
            "but a script this reader could not open is exactly where the game hands a portrait \
             to a name, and guessing that it did not leaves the box looking for a face under a \
             word nobody wrote"
        );

        let row = Piece {
            spot: "t27/d4/f0".to_string(),
            kind: Kind::Naming,
            said: vec![Said {
                text: who.to_string(),
                at: 0..12,
            }],
        };

        assert!(
            sift(slice::from_ref(&row), "", &Reached::new())[0]
                .offer
                .asked(),
            "with every script read, the row a name is written down in is the one place to ask"
        );
        assert!(
            sift(slice::from_ref(&row), "", &blind)[0].offer != Offer::Asked,
            "and the row the rest of the game finds by this word is held back for the same \
             reason the speaker is: renaming it lays one wording into every place this reader \
             found, while the script it could not open goes on handing over the old one"
        );
    }

    #[test]
    fn a_word_one_place_holds_back_as_a_key_is_held_back_in_every_other_place_too() {
        let key = "[\u{30b5}\u{30d6}]\u{30a2}\u{30a4}\u{30c6}\u{30e0}";

        let row = Piece {
            spot: "t27/d2/f4".to_string(),
            kind: Kind::Value,
            said: vec![Said {
                text: key.to_string(),
                at: 0..16,
            }],
        };

        assert!(
            sift(slice::from_ref(&row), "", &Reached::new())[0]
                .offer
                .asked(),
            "a database row nobody looks up by name is a row a player reads"
        );

        let mut reached = Reached::new();
        reached.keyed_by(key);
        reached.homing(key, 0, "elsewhere", "t0/d0/f0/s0");

        assert!(
            sift(slice::from_ref(&row), "", &reached)[0].offer == Offer::Locked,
            "event code elsewhere in the game reaches a row by this very word and the word is \
             written down in one place already, so this copy is kept out of sight and follows \
             whatever that one place is called"
        );
    }

    #[test]
    fn a_word_no_place_writes_down_is_asked_about_where_it_stands_rather_than_kept_out_of_sight() {
        let choice = command(CHOICES, &[], &["\u{306f}\u{3044}"]);
        let branch = command(STRING_CONDITION, &[], &["\u{306f}\u{3044}"]);
        let pieces = [choice, branch];

        let mut reached = Reached::new();
        homes_in(&pieces, "MapData/Dungeon.mps", &mut reached);

        assert!(
            sift(&pieces, "MapData/Dungeon.mps", &reached)
                .iter()
                .all(|one| one.offer.asked()),
            "a branch compares against the button beside it and neither of the two is a row \
             written down anywhere else, so keeping both out of sight would leave the player \
             reading the language the game shipped in with no place left to answer"
        );
    }

    #[test]
    fn a_name_with_a_space_in_it_still_reaches_a_command_as_one_word() {
        let who = "\u{4eba}\u{9593}\u{5175}\u{58ebA}";
        let arg = Piece {
            spot: "l3/a1".to_string(),
            kind: Kind::Handed,
            said: vec![Said {
                text: who.to_string(),
                at: 0..16,
            }],
        };

        assert!(
            sift(slice::from_ref(&arg), "", &Reached::new())[0].apart,
            "a script reads its orders one space at a time, so a wording of two words laid in \
             here would leave the engine holding the first of them and calling the rest an \
             argument it cannot make sense of"
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
            sift(&[command(SET_STRING, &[], &[layer])], "", &reached)
                .into_iter()
                .map(|one| !one.offer.asked())
                .collect::<Vec<bool>>(),
            [true]
        );
        assert_eq!(
            sift(&[command(SET_STRING, &[], &[plain])], "", &reached)
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
            "",
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
                "",
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
            &sift(&branch.pieces, "", &reached),
            &BTreeMap::from([("e0/p0/c0/s0".to_string(), "Yes indeed".to_string())]),
        );
        agreed.saw(
            &sift(&choice.pieces, "", &reached),
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
                    &sift(&one.pieces, "", &reached),
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
            sift(&[token], "", &reached)[0].offer != Offer::Asked,
            "the game matches this token against a portrait registry filled by event code \
             nothing here can rewrite, so a translated token is a portrait it can no longer find"
        );
        assert!(
            sift(&[display], "", &reached)[0].offer != Offer::Asked,
            "and the display field feeding that registry keeps the same spelling, or the two \
             sides of the match drift apart"
        );

        assert!(
            sift(&[command(PICKED, &[], &["\u{306f}\u{3044}"])], "", &reached)[0]
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
            sift(&[token], "", &reached)[0].offer != Offer::Asked,
            "the box is opened by this name, and a reach spells the same name out inside the \
             data, so the token the event code writes has to keep it"
        );
        assert!(
            sift(&[display], "", &reached)[0].offer.asked(),
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
            sift(&choice.pieces, "", &reached)[0].offer.asked(),
            "the player reads a choice, so it is always asked about"
        );
        assert!(
            !sift(&branch.pieces, "", &reached)[0].offer.asked(),
            "and a rule guesses the branch compares against a name, which is the whole setup"
        );

        let said = BTreeMap::from([("e0/p0/c0/s0".to_string(), "Hai_1".to_string())]);

        let mut agreed = Agreed::default();
        agreed.saw(&sift(&choice.pieces, "", &reached), &said);
        agreed.saw(&sift(&branch.pieces, "", &reached), &said);

        assert_eq!(
            String::from_utf8_lossy(
                &changed(
                    &branch,
                    &sift(&branch.pieces, "", &reached),
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
                    &sift(&branch.pieces, "", &reached),
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
    fn a_name_the_database_plan_spells_out_is_never_written_down_so_no_wording_follows_it() {
        let kind = "\u{6280}\u{80fd}";

        let label = Piece {
            spot: "t18/d24/f1".to_string(),
            kind: Kind::Value,
            said: vec![Said {
                text: kind.to_string(),
                at: 0..8,
            }],
        };
        let reads = command(DB_WRITE, &[], &["", kind]);

        let mut reached = Reached::new();
        reached.plans(kind);
        homes_in(
            slice::from_ref(&label),
            "BasicData/CDataBase.dat",
            &mut reached,
        );

        assert!(
            !sift(slice::from_ref(&label), "BasicData/CDataBase.dat", &reached)[0]
                .offer
                .asked(),
            "and the row is held back rather than asked about, because a word the engine looks a \
             type up by cannot be renamed halfway"
        );
        let taken = sift(slice::from_ref(&reads), "", &reached);
        let handed = taken
            .iter()
            .find(|one| one.said == kind)
            .expect("the name the command hands over");

        assert_eq!(
            handed.offer,
            Offer::Locked,
            "while the command that hands the name over spells out no word of its own, so it is \
             kept out of sight rather than laid in front of a reader who has nothing to decide"
        );

        let held = Held {
            plain: Vec::new(),
            shape: Shape::Loose,
            pieces: vec![reads],
        };
        let said = BTreeMap::from([("t18/d24/f1/s0".to_string(), "Skill".to_string())]);

        let mut agreed = Agreed::default();
        agreed.saw(
            &sift(slice::from_ref(&label), "BasicData/CDataBase.dat", &reached),
            &said,
        );

        assert!(
            changed(
                &held,
                &sift(&held.pieces, "", &reached),
                &said,
                &agreed,
                &reached
            )
            .is_empty(),
            "so translating that row by hand leaves every command that reaches the type alone, \
             rather than sending the engine after a type the database was never told about"
        );
    }

    #[test]
    fn a_name_written_down_once_is_laid_into_every_hidden_place_the_game_spells_it() {
        let who = "\u{30ab}\u{30eb}\u{30df}\u{30a2}";

        let piece = |spot: &str, kind: Kind| Piece {
            spot: spot.to_string(),
            kind,
            said: vec![Said {
                text: who.to_string(),
                at: 0..16,
            }],
        };
        let held = |piece: Piece| Held {
            plain: Vec::new(),
            shape: Shape::Loose,
            pieces: vec![piece],
        };

        let written = held(piece("t0/d0/f0", Kind::Naming));
        let handed = held(piece("l3/a1", Kind::Handed));

        let mut reached = Reached::new();
        reached.hands(who);
        reached.keyed_by(who);
        reached.homing(who, 0, "", "t0/d0/f0/s0");
        reached.homing(who, 2, "", "l3/a1/s0");

        let slots = sift(&written.pieces, "", &reached);
        assert!(
            slots[0].offer.asked(),
            "the row the plan spells the name out in is the one place it is written down, so \
             that is where the reader is asked for a wording, however many hidden places spell \
             the same name: a copy may never be the reason its own original is held back"
        );
        assert_eq!(
            sift(&handed.pieces, "", &reached)[0].offer,
            Offer::Locked,
            "and the script handing the same name to a command is no line to read twice: it is \
             kept out of sight so the reader answers once"
        );

        let said = BTreeMap::from([("t0/d0/f0/s0".to_string(), "Kalmia Rose".to_string())]);
        let mut agreed = Agreed::default();
        agreed.saw(&slots, &said);
        agreed.saw(&sift(&handed.pieces, "", &reached), &said);

        let laid = |one: &Held| {
            String::from_utf8_lossy(
                &changed(
                    one,
                    &sift(&one.pieces, "", &reached),
                    &said,
                    &agreed,
                    &reached,
                )[0]
                .1,
            )
            .into_owned()
        };

        assert_eq!(
            laid(&handed),
            "Kalmia\u{3000}Rose",
            "the hidden place still has to follow, or the game looks a portrait up under a name \
             only half the game was renamed to, and a script reads its orders one space at a time"
        );
        assert_eq!(
            laid(&written),
            laid(&handed),
            "and the row the name is written down in has to be spelled the way the script hands \
             it over, or the one word the engine carries reaches a row of another name"
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
        agreed.saw(&sift(&branch.pieces, "", &reached), &byhand);
        agreed.saw(&sift(&label.pieces, "", &reached), &BTreeMap::new());

        assert_eq!(
            String::from_utf8_lossy(
                &changed(
                    &label,
                    &sift(&label.pieces, "", &reached),
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
        agreed.saw(&sift(&token.pieces, "", &reached), &said);
        agreed.saw(&sift(&branch.pieces, "", &reached), &said);

        let written = |one: &Held| {
            String::from_utf8_lossy(
                &changed(
                    one,
                    &sift(&one.pieces, "", &reached),
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
                    &sift(&held.pieces, "", &reached),
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
                    &sift(&held.pieces, "", &reached),
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
                &sift(&held.pieces, "", &reached),
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
            sift(&[held(Kind::Naming)], "", &reached)[0].offer != Offer::Asked,
            "the event code finds this row by this word, so with nothing yet saying where the \
             word is written down there is no place safe to rename it"
        );

        reached.homing(who, 0, "", "t27/d4/f0/s0");

        assert!(
            sift(&[held(Kind::Naming)], "", &reached)[0].offer.asked(),
            "and once this row is known to be the one place the word is written down, asking \
             here once is what lets one wording be laid into every place that reaches the row"
        );
        assert!(
            sift(&[held(Kind::Value)], "", &reached)[0].offer.asked(),
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

        let found = sift(&[held], "", &Reached::new());

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
            sift(slice::from_ref(&called), "", &Reached::new())[0].offer,
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
                &sift(&held.pieces, "", &Reached::new()),
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
            &sift(&held.pieces, "", &Reached::new()),
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

        assert!(sift(&[held], "", &Reached::new()).is_empty());
    }
}
