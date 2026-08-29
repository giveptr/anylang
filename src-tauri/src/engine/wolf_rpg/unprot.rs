use crate::engine::wolf_rpg::{aes, coder, keying, sha};

const GUARD: usize = 143;
const DATA_AT: usize = 20;
const BODY_AT: usize = 0xA;
const KEY_AT: usize = 12;
const ONCE_AT: usize = 73;
const KEY_LEN: usize = 16;
const ONCE_LEN: usize = 16;
const GUARDED_AT: usize = 1;
const GUARDED: u8 = 0x50;
const NEWEST_AT: usize = 5;
const NEWEST: u8 = 0x57;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Game,
    Common,
    Database,
    TileSet,
}

impl Kind {
    pub fn of(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "game.dat" => Some(Self::Game),
            "commonevent.dat" => Some(Self::Common),
            "database.dat" | "cdatabase.dat" | "sysdatabase.dat" => Some(Self::Database),
            "tilesetdata.dat" => Some(Self::TileSet),
            _ => None,
        }
    }

    fn salt(&self) -> &'static str {
        match self {
            Self::Game => "basicD1",
            Self::Common => "Commo2",
            Self::Database => "DBase4",
            Self::TileSet => "TilesetA",
        }
    }

    fn spell(&self) -> [u8; 10] {
        match self {
            Self::Game => [0x00, 0x57, 0x00, 0x00, 0x4F, 0x4C, 0x00, 0x46, 0x4D, 0x55],
            Self::Common => [0x00, 0x57, 0x00, 0x00, 0x4F, 0x4C, 0x55, 0x46, 0x43, 0x00],
            Self::Database | Self::TileSet => {
                [0x00, 0x57, 0x00, 0x00, 0x4F, 0x4C, 0x55, 0x46, 0x4D, 0x00]
            }
        }
    }

    fn seeds(&self) -> [usize; 3] {
        match self {
            Self::Game => [0, 8, 6],
            _ => [0, 3, 9],
        }
    }
}

fn unmask(body: &mut [u8], seeds: [usize; 3]) {
    let seed = (0xB << 24)
        | (u32::from(body[seeds[0]]) << 16)
        | (u32::from(body[seeds[1]]) << 8)
        | u32::from(body[seeds[2]]);

    let mut rn = keying::Shifting(seed).next() as i32;

    for one in body.iter_mut().skip(BODY_AT) {
        let mixed = ((rn.wrapping_shl(0xF) ^ rn) >> 0x15) ^ rn.wrapping_shl(0xF) ^ rn;
        rn = mixed.wrapping_shl(0x9) ^ mixed;

        *one ^= (rn % 0xF9) as u8;
    }
}

fn salted(body: &[u8], kind: Kind) -> Vec<u8> {
    let first = body[7];
    let second = body[11];
    let third = body[13];

    let grain = [
        ((u32::from(first) + 2 * u32::from(second)) % 0xF6) as u8,
        third ^ body[14],
        first ^ body[12],
        first.wrapping_add(third).wrapping_sub(second),
    ];

    let mut out: Vec<u8> = grain
        .iter()
        .map(|one| match one {
            0 => 1,
            _ => *one,
        })
        .collect();

    out.extend_from_slice(kind.salt().as_bytes());

    out
}

pub fn unplanned(body: &mut [u8]) {
    let mut rolls = keying::Rolling::from(0);

    for one in body.iter_mut() {
        *one ^= rolls.next() as u8;
    }
}

fn guarded(body: &[u8]) -> bool {
    body.len() >= GUARD
        && body[GUARDED_AT] == GUARDED
        && body.get(NEWEST_AT).is_some_and(|one| *one >= NEWEST)
}

fn scolding(body: &[u8]) -> Option<usize> {
    let end = body
        .iter()
        .take_while(|one| one.is_ascii_graphic() || **one == b' ')
        .count();

    match body.get(end) {
        Some(0) => Some(end + 1),
        _ => None,
    }
}

fn spells(body: &[u8], kind: Kind) -> bool {
    let spell = kind.spell();
    let mark = spell
        .iter()
        .position(|one| *one == coder::UTF8_MARK)
        .expect("every spell carries the UTF-8 mark");

    coder::opens(&spell, mark, body, 0)
}

fn past_scolding(body: &[u8], kind: Kind) -> Option<usize> {
    scolding(body).filter(|past| guarded(&body[*past..]) || spells(&body[*past..], kind))
}

fn restamp(body: &mut [u8], was: usize, fresh: u32) -> Result<(), String> {
    let mut at = BODY_AT;

    at += coder::word_at(body, at)? as usize + 4;
    at += 4;

    for _ in 0..4 {
        at += coder::word_at(body, at)? as usize + 4;
    }

    let stale = (was - 1) as u32;

    loop {
        let told = coder::word_at(body, at)?;
        if told == stale {
            break;
        }

        at += told as usize + 4;
    }

    coder::put_word(body, at, fresh)?;

    Ok(())
}

fn spanned(body: &[u8], span: usize) -> usize {
    let mut rolls = keying::Rolling::from(u32::from(body[KEY_AT]));

    match span >= (rolls.next() % 126 + 200) as usize {
        true => span.min((rolls.next() % 126 + 200) as usize),
        false => span,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Pro {
    head: Vec<u8>,
    seed: [u8; KEY_LEN],
    once: [u8; ONCE_LEN],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Guard {
    scolding: Vec<u8>,
    pro: Option<Pro>,
}

pub fn freed(body: &mut Vec<u8>, kind: Kind) -> Result<Guard, String> {
    let scolding: Vec<u8> = match guarded(body) {
        true => Vec::new(),
        false => match past_scolding(body, kind) {
            Some(past) => body.drain(..past).collect(),
            None => Vec::new(),
        },
    };

    if !guarded(body) {
        return Ok(Guard {
            scolding,
            pro: None,
        });
    }

    let was = body.len();

    unmask(body, kind.seeds());

    let span = spanned(body, was - DATA_AT);

    let spelled = sha::letters(&salted(body, kind));
    let letters = spelled.as_bytes();

    let seed: [u8; KEY_LEN] = letters[KEY_AT..KEY_AT + KEY_LEN]
        .try_into()
        .expect("sixteen letters");

    let once: [u8; ONCE_LEN] = letters[ONCE_AT..ONCE_AT + ONCE_LEN]
        .try_into()
        .expect("sixteen letters");

    aes::counted(&mut body[DATA_AT..DATA_AT + span], &seed, &once);

    let head: Vec<u8> = body.drain(..GUARD).collect();
    body.splice(0..0, kind.spell());

    if kind == Kind::Game {
        let fresh = (body.len() - 1) as u32;
        restamp(body, was, fresh)?;
    }

    Ok(Guard {
        scolding,
        pro: Some(Pro { head, seed, once }),
    })
}

pub fn reguarded(body: &mut Vec<u8>, kind: Kind, kept: &Guard) -> Result<(), String> {
    let Some(pro) = &kept.pro else {
        body.splice(0..0, kept.scolding.iter().copied());

        return Ok(());
    };

    let plain = body.len();

    if body.len() < kind.spell().len() {
        return Err("this file is too short to have come from a guarded one".to_string());
    }

    if kind == Kind::Game {
        let fresh = (plain - kind.spell().len() + pro.head.len() - 1) as u32;
        restamp(body, plain, fresh)?;
    }

    body.splice(..kind.spell().len(), pro.head.iter().copied());

    let span = body
        .len()
        .checked_sub(DATA_AT)
        .ok_or_else(|| "this file is too short to carry the guard it came with".to_string())?;
    let span = spanned(body, span);

    aes::counted(&mut body[DATA_AT..DATA_AT + span], &pro.seed, &pro.once);

    unmask(body, kind.seeds());
    body.splice(0..0, kept.scolding.iter().copied());

    Ok(())
}

#[cfg(test)]
pub fn as_shipped(plain: &[u8], scolding: &[u8], kind: Kind) -> Vec<u8> {
    let mut head: Vec<u8> = (0..GUARD).map(|one| (one * 7 % 251) as u8).collect();
    head[GUARDED_AT] = GUARDED;
    head[NEWEST_AT] = NEWEST;

    let spelled = sha::letters(&salted(&head, kind));
    let letters = spelled.as_bytes();

    let kept = Guard {
        scolding: scolding.to_vec(),
        pro: Some(Pro {
            head,
            seed: letters[KEY_AT..KEY_AT + KEY_LEN]
                .try_into()
                .expect("sixteen letters"),
            once: letters[ONCE_AT..ONCE_AT + ONCE_LEN]
                .try_into()
                .expect("sixteen letters"),
        }),
    };

    let mut body = plain.to_vec();
    reguarded(&mut body, kind, &kept).expect("a file the game would ship");

    body
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::wolf_rpg::{fixture, game, harvest, held};
    use std::collections::BTreeMap;

    fn guarded_like(len: usize) -> Vec<u8> {
        let mut out: Vec<u8> = (0..len).map(|one| (one * 7 % 251) as u8).collect();
        out[GUARDED_AT] = GUARDED;
        out[NEWEST_AT] = NEWEST;

        out
    }

    #[test]
    fn a_file_let_go_of_its_guard_and_given_it_back_is_the_file_the_game_shipped() {
        for kind in [Kind::Common, Kind::Database, Kind::TileSet] {
            for scolding in ["", "Extracting data violates the guidelines."] {
                let mut shipped = scolding.as_bytes().to_vec();
                if !scolding.is_empty() {
                    shipped.push(0);
                }
                shipped.extend(guarded_like(GUARD + 60));

                let mut body = shipped.clone();
                let kept = freed(&mut body, kind).expect("the guard lifts");
                assert!(
                    body.starts_with(&kind.spell()),
                    "what is left has to open like the file the editor would write"
                );

                reguarded(&mut body, kind, &kept).expect("the guard goes back on");

                assert_eq!(
                    body, shipped,
                    "a reader who translates nothing in a guarded file still has it written back, \
                     so the two halves have to be exact opposites or the game is handed bytes it \
                     cannot read"
                );
            }
        }
    }

    #[test]
    fn a_file_that_is_scolded_but_never_guarded_still_has_the_scolding_taken_off_and_put_back() {
        for kind in [Kind::Game, Kind::Common, Kind::Database, Kind::TileSet] {
            let mut shipped =
                b"Extracting data from encrypted files violates the guidelines.\0".to_vec();
            let past = shipped.len();
            shipped.extend_from_slice(&kind.spell());
            shipped.extend((0..200usize).map(|one| (one * 3 % 251) as u8));

            let mut body = shipped.clone();
            let kept = freed(&mut body, kind).expect("the scolding comes off");

            assert_eq!(
                body,
                shipped[past..],
                "the engine writes this scolding in front of a file it left unencrypted, and \
                 every reader after it would see the words where the magic belongs"
            );

            reguarded(&mut body, kind, &kept).expect("the scolding goes back on");
            assert_eq!(
                body, shipped,
                "the game reads what it shipped, so a file nothing was done to has to go back \
                 wearing the same words it came with"
            );
        }
    }

    #[test]
    fn a_file_that_is_neither_scolded_nor_guarded_is_left_where_it_stands() {
        for kind in [Kind::Game, Kind::Common, Kind::Database, Kind::TileSet] {
            let mut body = kind.spell().to_vec();
            body.extend((0..200usize).map(|one| (one * 3 % 251) as u8));
            let shipped = body.clone();

            let kept = freed(&mut body, kind).expect("there is nothing to lift");
            assert_eq!(
                body, shipped,
                "a Wolf file opens on a nul, and taking that for the end of a scolding would eat \
                 the first byte of every game that was never guarded at all"
            );

            reguarded(&mut body, kind, &kept).expect("and nothing goes back on");
            assert_eq!(body, shipped);
        }
    }

    #[test]
    fn a_scolded_game_dat_that_grew_carries_the_size_the_translation_left_in_it() {
        let mut shipped = b"Extracting data violates the guidelines.\0".to_vec();
        let past = shipped.len();
        shipped.extend_from_slice(&fixture::game("Short", "", "MS Gothic"));

        let mut body = shipped.clone();
        let kept = freed(&mut body, Kind::Game).expect("the scolding comes off");

        let read = game::read(&body).expect("Game.dat");
        let said = BTreeMap::from([("title/s0".to_string(), "A Rather Longer Title".to_string())]);
        let edits = harvest::changed(
            &read,
            &harvest::sift(&read.pieces, "", &Default::default()),
            &said,
            &Default::default(),
            &Default::default(),
        );
        let mut grown = held::wrapped(&read, edits).expect("a whole Game.dat");

        reguarded(&mut grown, Kind::Game, &kept).expect("the scolding goes back on");

        assert_eq!(&grown[..past], &shipped[..past]);
        assert_eq!(
            game::read(&grown[past..]).expect("it still reads").pieces[0].said[0].text,
            "A Rather Longer Title",
            "the engine reads this file's own size from past the scolding, so a title that grew \
             has to leave that number true with the scolding back in front of it"
        );
    }

    #[test]
    fn a_guarded_file_that_grew_is_still_guarded_when_it_goes_back_in() {
        let mut body = guarded_like(GUARD + 60);
        let kept = freed(&mut body, Kind::Database).expect("the guard lifts");

        body.extend_from_slice("a translation longer than what it replaced".as_bytes());
        reguarded(&mut body, Kind::Database, &kept).expect("the guard goes back on");

        assert!(
            guarded(&body),
            "a translation makes the file longer, and the engine still has to see a guarded file \
             at the other end"
        );
    }

    #[test]
    fn the_size_game_dat_carries_is_found_wherever_it_sits_and_written_fresh() {
        let mut body = fixture::game("A Title", " + DLC", "MS Gothic");
        let was = body.len();

        restamp(&mut body, was, 4242).expect("the size Game.dat carries is found");
        restamp(&mut body, 4243, 99).expect("and the fresh size is where the next look finds it");

        assert!(
            restamp(&mut body, 4243, 1).is_err(),
            "the engine reads this number to know how far the file goes, so a size that is not \
             there is a layout this reader must not write into"
        );
    }
}
