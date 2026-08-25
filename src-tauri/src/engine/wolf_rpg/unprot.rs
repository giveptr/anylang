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

pub fn wanted(body: &[u8]) -> bool {
    guarded(body) || scolding(body).is_some_and(|past| guarded(&body[past..]))
}

fn spanned(body: &[u8], span: usize) -> usize {
    let mut rolls = keying::Rolling::from(u32::from(body[KEY_AT]));

    match span >= (rolls.next() % 126 + 200) as usize {
        true => span.min((rolls.next() % 126 + 200) as usize),
        false => span,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Guard {
    decoy: Vec<u8>,
    head: Vec<u8>,
    seed: [u8; KEY_LEN],
    once: [u8; ONCE_LEN],
    span: usize,
}

pub fn freed(body: &mut Vec<u8>, kind: Kind) -> Result<Guard, String> {
    let decoy = match guarded(body) {
        true => Vec::new(),
        false => {
            let past = scolding(body)
                .filter(|past| guarded(&body[*past..]))
                .ok_or("this file carries no Wolf RPG Pro guard to lift")?;

            body.drain(..past).collect()
        }
    };

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
        decoy,
        head,
        seed,
        once,
        span,
    })
}

impl Guard {
    pub fn packed(&self) -> Vec<u8> {
        let mut out = Vec::new();

        for part in [&self.decoy, &self.head] {
            out.extend_from_slice(&(part.len() as u32).to_le_bytes());
            out.extend_from_slice(part);
        }

        out.extend_from_slice(&self.seed);
        out.extend_from_slice(&self.once);
        out.extend_from_slice(&(self.span as u32).to_le_bytes());

        out
    }

    pub fn unpacked(raw: &[u8]) -> Result<Self, String> {
        let mut at = 0;
        let mut taken = Vec::new();

        for _ in 0..2 {
            let len = coder::word_at(raw, at)? as usize;
            at += 4;
            taken.push(
                raw.get(at..at + len)
                    .ok_or("this guard stops before the bytes it names")?
                    .to_vec(),
            );
            at += len;
        }

        let seed: [u8; KEY_LEN] = raw
            .get(at..at + KEY_LEN)
            .and_then(|held| held.try_into().ok())
            .ok_or("this guard holds no key")?;
        at += KEY_LEN;

        let once: [u8; ONCE_LEN] = raw
            .get(at..at + ONCE_LEN)
            .and_then(|held| held.try_into().ok())
            .ok_or("this guard holds no nonce")?;
        at += ONCE_LEN;

        Ok(Self {
            head: taken.pop().expect("two parts"),
            decoy: taken.pop().expect("two parts"),
            seed,
            once,
            span: coder::word_at(raw, at)? as usize,
        })
    }
}

pub fn reguarded(body: &mut Vec<u8>, kind: Kind, kept: &Guard) -> Result<(), String> {
    let plain = body.len();

    if body.len() < kind.spell().len() {
        return Err("this file is too short to have come from a guarded one".to_string());
    }

    if kind == Kind::Game {
        let fresh = (plain - kind.spell().len() + kept.head.len() - 1) as u32;
        restamp(body, plain, fresh)?;
    }

    body.splice(..kind.spell().len(), kept.head.iter().copied());

    let span = body
        .len()
        .checked_sub(DATA_AT)
        .ok_or_else(|| "this file is too short to carry the guard it came with".to_string())?;
    let span = spanned(body, span);

    aes::counted(&mut body[DATA_AT..DATA_AT + span], &kept.seed, &kept.once);

    unmask(body, kind.seeds());
    body.splice(0..0, kept.decoy.iter().copied());

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::wolf_rpg::fixture;

    fn guarded_like(len: usize) -> Vec<u8> {
        let mut out: Vec<u8> = (0..len).map(|one| (one * 7 % 251) as u8).collect();
        out[GUARDED_AT] = GUARDED;
        out[NEWEST_AT] = NEWEST;

        out
    }

    #[test]
    fn a_file_let_go_of_its_guard_and_given_it_back_is_the_file_the_game_shipped() {
        for kind in [Kind::Common, Kind::Database, Kind::TileSet] {
            for decoy in ["", "Extracting data violates the guidelines."] {
                let mut shipped = decoy.as_bytes().to_vec();
                if !decoy.is_empty() {
                    shipped.push(0);
                }
                shipped.extend(guarded_like(GUARD + 60));

                let mut body = shipped.clone();
                assert!(wanted(&body), "this is the shape a guarded file arrives in");

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
    fn a_guarded_file_that_grew_is_still_guarded_when_it_goes_back_in() {
        let mut body = guarded_like(GUARD + 60);
        let kept = freed(&mut body, Kind::Database).expect("the guard lifts");

        body.extend_from_slice("a translation longer than what it replaced".as_bytes());
        reguarded(&mut body, Kind::Database, &kept).expect("the guard goes back on");

        assert!(
            wanted(&body),
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

    #[test]
    fn a_guard_written_beside_a_file_reads_back_the_way_it_was_kept() {
        let kept = Guard {
            decoy: b"Extracting data...".to_vec(),
            head: (0..GUARD as u8).collect(),
            seed: [7; KEY_LEN],
            once: [9; ONCE_LEN],
            span: 261,
        };

        assert_eq!(
            Guard::unpacked(&kept.packed()),
            Ok(kept),
            "the guard is all that lets a written file go back looking the way it came, so it \
             has to survive the trip through the store"
        );
    }

    #[test]
    fn a_guard_cut_short_is_refused_rather_than_read_askew() {
        let whole = Guard {
            decoy: Vec::new(),
            head: vec![1, 2, 3],
            seed: [0; KEY_LEN],
            once: [0; ONCE_LEN],
            span: 8,
        }
        .packed();

        for cut in [0, 4, whole.len() - 1] {
            assert!(
                Guard::unpacked(&whole[..cut]).is_err(),
                "a guard missing its last bytes would put the file back wrong"
            );
        }
    }
}
