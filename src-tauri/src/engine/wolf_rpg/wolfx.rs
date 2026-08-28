const MAGIC: [u8; 5] = [0x57, 0x4F, 0x4C, 0x46, 0x58];
const HEAD: usize = 15;
const SEED_AT: usize = 5;
const DRAWN_AT: usize = 10;
const DRAWN: usize = 5;
const MARKS: usize = 5;
const BLOB: usize = 256;
const GRAIN: usize = 64;
const BODY_FROM: usize = 512;

const NAMED_STRING: u32 = 10_000;
const NAMED_NUMBER: u32 = 1_000_000;

const STEP: u32 = 1_664_525;
const TURN: u32 = 1_013_904_223;
const PRIME: u32 = 0x0100_0193;
const START: u32 = 0x811C_9DC5;

fn fnv1(body: &[u8]) -> u32 {
    body.iter().fold(START, |held, one| {
        PRIME.wrapping_mul(held ^ u32::from(*one))
    })
}

fn grain() -> [u8; GRAIN] {
    let mut out = [0xAAu8; GRAIN];

    for _ in 0..5 {
        for at in 0..GRAIN {
            let held = out[(at + 13) % GRAIN];
            let mixed = out[at] ^ out[(at + 7) % GRAIN];

            out[at] = held.wrapping_add(mixed).rotate_right(7);
        }
    }

    for one in out.iter_mut() {
        if *one == 0 {
            *one = 1;
        }
    }

    out
}

fn rolled(mut seed: u32, grain: &[u8; GRAIN]) -> [u8; BLOB] {
    for one in grain {
        seed ^= u32::from(*one);
    }

    let mut out = [0u8; BLOB];
    for one in out.iter_mut() {
        seed = seed.wrapping_mul(STEP).wrapping_add(TURN);
        *one = seed as u8;
    }

    out
}

fn word_at(raw: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([raw[at], raw[at + 1], raw[at + 2], raw[at + 3]])
}

struct Keyed {
    blob: [u8; BLOB],
    body_at: usize,
}

fn keyed(raw: &[u8]) -> Option<Keyed> {
    if !seals(raw) {
        return None;
    }

    let grain = grain();
    let body_at = BODY_FROM + usize::from(grain[0]) + usize::from(grain[1]);
    if body_at >= raw.len() {
        return None;
    }

    let mut blob = rolled(fnv1(&grain) ^ word_at(raw, SEED_AT), &grain);

    let mut drawn = [0u8; DRAWN];
    for (at, one) in drawn.iter_mut().enumerate() {
        *one = raw[DRAWN_AT + at] ^ blob[at];
    }

    let named = u32::from(drawn[0]) << 8 | u32::from(drawn[1]);
    if named < NAMED_STRING {
        return None;
    }

    let told = fnv1(&[]);
    let numbered = ((told & 0xFFFF_0000) >> 8)
        ^ (told & 0xFFFF)
        ^ (u32::from(drawn[2]) << 16 | u32::from(drawn[3]) << 8 | u32::from(drawn[4]));
    if numbered < NAMED_NUMBER {
        return None;
    }

    let told = numbered.to_be_bytes();
    for (at, one) in blob.iter_mut().enumerate() {
        *one ^= drawn[at % 2] ^ told[1 + at % 3];
    }

    Some(Keyed { blob, body_at })
}

fn turned(blob: &[u8; BLOB], body: &mut [u8]) {
    for at in HEAD..body.len() {
        body[at] ^= blob[(at - DRAWN_AT) % BLOB];
    }
}

fn marks(body: &[u8]) -> [u8; MARKS] {
    let last = (body.len() - 1) as f64;
    let mut out = [0u8; MARKS];

    for (at, one) in out.iter_mut().enumerate() {
        *one = body[(last * 0.25 * at as f64) as usize];
    }

    out
}

pub fn seals(raw: &[u8]) -> bool {
    raw.len() >= HEAD && raw[..MAGIC.len()] == MAGIC
}

pub fn opened(raw: &[u8]) -> Option<Vec<u8>> {
    let held = keyed(raw)?;

    let mut body = raw.to_vec();
    turned(&held.blob, &mut body);

    let told: [u8; MARKS] = body.get(HEAD..HEAD + MARKS)?.try_into().ok()?;
    let inside = body.get(held.body_at..)?;

    match inside.is_empty() || marks(inside) != told {
        true => None,
        false => Some(inside.to_vec()),
    }
}

pub fn sealed(raw: &[u8], body: &[u8]) -> Option<Vec<u8>> {
    opened(raw)?;

    let held = keyed(raw)?;
    if body.is_empty() {
        return None;
    }

    let mut out = raw.get(..held.body_at)?.to_vec();
    turned(&held.blob, &mut out);

    out[HEAD..HEAD + MARKS].copy_from_slice(&marks(body));
    out.extend_from_slice(body);
    turned(&held.blob, &mut out);

    Some(out)
}

#[cfg(test)]
pub fn as_shipped(body: &[u8]) -> Vec<u8> {
    let mut raw = MAGIC.to_vec();
    raw.extend_from_slice(&[0x17, 0xB2, 0xBF, 0xA7, 0xE1, 0x57, 0x16, 0xEB, 0xCA, 0x72]);
    raw.resize(BODY_FROM * 2, 0);

    let held = keyed(&raw).expect("a header this reader knows");

    raw.resize(held.body_at, 0);
    raw[HEAD..HEAD + MARKS].copy_from_slice(&marks(body));
    raw.extend_from_slice(body);
    turned(&held.blob, &mut raw);

    raw
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_font_the_game_sealed_is_opened_and_sealed_back_around_another_one() {
        let held: Vec<u8> = (0..40_000usize).map(|one| (one * 7 % 251) as u8).collect();
        let raw = as_shipped(&held);

        assert_ne!(
            &raw[HEAD..HEAD + 8],
            &held[..8],
            "it is not left in the clear"
        );
        assert_eq!(opened(&raw).as_deref(), Some(held.as_slice()));

        let fresh: Vec<u8> = (0..90_000usize).map(|one| (one * 13 % 251) as u8).collect();
        let out = sealed(&raw, &fresh).expect("a font of our own goes back in");

        assert_eq!(
            opened(&out).as_deref(),
            Some(fresh.as_slice()),
            "the engine checks this file against five bytes sampled out of its own body, so a \
             font of another length has to leave that check true or the game draws nothing"
        );
        assert_eq!(&out[..HEAD - MARKS], &raw[..HEAD - MARKS]);
    }

    const FROM_REFERENCE: &str = "these bytes are what the UberWolf reference works out for the \
                                  key every game falls back on, so they are the one thing a round \
                                  trip through this module can never settle on its own";

    #[test]
    fn the_key_schedule_lands_on_the_bytes_the_reference_lays_down() {
        let grain = grain();

        assert_eq!(
            &grain[..8],
            &[0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0x6C, 0x6C, 0x6C],
            "{FROM_REFERENCE}"
        );
        assert_eq!(
            &grain[56..],
            &[0x97, 0xF5, 0xF5, 0xF5, 0xF5, 0xF5, 0x4E, 0x98]
        );
        assert_eq!(fnv1(&grain), 0xD280_E551);
        assert_eq!(
            BODY_FROM + usize::from(grain[0]) + usize::from(grain[1]),
            852,
            "the body starts where the key says it does, and every game sealed with the default \
             key starts it here"
        );
        assert_eq!(
            &rolled(0, &grain)[..8],
            &[0x91, 0xBC, 0xEB, 0x4E, 0x55, 0xB0, 0x4F, 0x62],
            "{FROM_REFERENCE}"
        );
    }

    #[test]
    fn the_body_is_turned_against_the_key_five_bytes_in() {
        let blob = [7u8; BLOB];
        let mut body = vec![0u8; HEAD + 1];

        turned(&blob, &mut body);

        assert_eq!(
            &body[..HEAD],
            &[0u8; HEAD],
            "the header is the one part the key never touches"
        );

        let mut blob = [0u8; BLOB];
        blob[HEAD - DRAWN_AT] = 0xFF;
        let mut body = vec![0u8; HEAD + 1];
        turned(&blob, &mut body);

        assert_eq!(
            body[HEAD], 0xFF,
            "the first body byte is turned against the key ten places back from where it sits, \
             and a round trip through this module reads the same either way, so only this says \
             which"
        );
    }

    #[test]
    fn anything_that_is_not_a_sealed_font_is_turned_away_rather_than_read_askew() {
        assert_eq!(opened(b""), None);
        assert_eq!(
            opened(&[0x57, 0x4F, 0x4C, 0x46, 0x58, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
            None,
            "a file one byte short of a whole header is refused before a single byte is read \
             out of it"
        );
        assert_eq!(opened(&[0u8; 4096]), None);

        let mut torn = as_shipped(&(0..40_000usize).map(|one| one as u8).collect::<Vec<u8>>());
        let at = torn.len() - 1;
        torn[at] ^= 0xFF;

        assert_eq!(
            opened(&torn),
            None,
            "a file whose body no longer matches the five bytes it carries is one this reader \
             cannot stand behind, and offering it would put a font in the game that draws nothing"
        );
    }
}
