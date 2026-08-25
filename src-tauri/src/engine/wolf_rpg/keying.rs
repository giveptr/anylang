pub const PAD: usize = 7;
const PLANE: usize = 256;
pub const PLANES: usize = 3 * PLANE;
pub const WORD: usize = 15;
const SALT: usize = 128;
pub const MASK_AT: usize = 8;
pub const MASK_LEN: usize = 32;
pub const RESERVE_AT: usize = 49;
pub const HEAD_LEN: u64 = 64;
const LEAST_BODY: u64 = 0x400;
const LONGEST_SOURCE: usize = 2040;

const FALLBACK: &[u8] = b"DXBDXARC";
const SHORT_SOURCE: usize = 4;

const FAMILY_FROM: u16 = 331;
const FAMILY_UPTO: u16 = 1000;
const PRO_FROM: u16 = 1010;
const REMADE_FROM: u16 = 0x15E;
const REMADE_UPTO: u16 = 0x3E8;
const PRO_REMADE_FROM: u16 = 0x3FC;
const SALTED_FROM: u16 = 0x154;
const OWN_SALT: u16 = 0x15E;
const OWN_SALT_SOURCE: &[u8] = b"958";

const SECOND_FACTOR_REPLACES: bool = true;

pub fn newer(crypt: u16) -> bool {
    (FAMILY_FROM..FAMILY_UPTO).contains(&crypt) || crypt >= PRO_FROM
}

fn remade(crypt: u16) -> bool {
    (REMADE_FROM..REMADE_UPTO).contains(&crypt) || crypt >= PRO_REMADE_FROM
}

fn stirred(crypt: u16) -> bool {
    crypt >= SALTED_FROM && !(crypt > REMADE_UPTO && crypt < PRO_REMADE_FROM)
}

pub struct Rolling(u32);

impl Rolling {
    pub fn from(seed: u32) -> Self {
        Self(seed)
    }

    pub fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(214013).wrapping_add(2531011);

        (self.0 >> 16) & 0x7FFF
    }
}

pub struct Shifting(pub u32);

impl Shifting {
    pub fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 0xB;
        self.0 ^= self.0 >> 0x13;
        self.0 ^= self.0 << 0x7;

        self.0
    }
}

pub fn hashed(body: &[u8]) -> u32 {
    let mut held = crc32fast::Hasher::new();
    held.update(body);

    held.finalize()
}

pub fn padded(source: &[u8]) -> [u8; PAD] {
    let mut held = Vec::new();

    match source.len() < SHORT_SOURCE {
        true => {
            held.extend_from_slice(source);
            held.extend_from_slice(FALLBACK);
        }
        false => held.extend_from_slice(source),
    }

    let evens: Vec<u8> = held.iter().step_by(2).copied().collect();
    let odds: Vec<u8> = held.iter().skip(1).step_by(2).copied().collect();

    let first = hashed(&evens).to_le_bytes();
    let second = hashed(&odds).to_le_bytes();

    [
        first[0], first[1], first[2], first[3], second[0], second[1], second[2],
    ]
}

pub fn sourced(pass: &[u8], name: &[u8], chain: &[u8]) -> Vec<u8> {
    let room = LONGEST_SOURCE.saturating_sub(pass.len());

    let mut tail = Vec::with_capacity(name.len().saturating_add(chain.len()));
    tail.extend_from_slice(name);
    tail.extend_from_slice(chain);
    tail.truncate(room);

    let mut out = Vec::with_capacity(pass.len().saturating_add(tail.len()));
    out.extend_from_slice(pass);
    out.extend_from_slice(&tail);

    out
}

pub fn salted(source: &[u8]) -> [u8; SALT] {
    let mut out = [0u8; SALT];

    if source.is_empty() {
        return out;
    }

    for (i, one) in out.iter_mut().enumerate() {
        *one = ((i / source.len()) as u8).wrapping_add(source[i % source.len()]);
    }

    out
}

fn folded(pw: &[u8; WORD], other: bool) -> u8 {
    let (many, turn) = match other {
        true => (pw[8] / 4, 2),
        false => (pw[11] / 3, 3),
    };

    let mut out = 0u8;

    for i in 0..many {
        out = i ^ (out ^ pw[usize::from(i) % WORD]).rotate_right(turn);
    }

    out
}

fn glazed(crypt: u16, key: &mut [u8; PLANES], mut third: u8, salting: &[u8]) {
    let salt = salted(match crypt == OWN_SALT {
        true => OWN_SALT_SOURCE,
        false => salting,
    });

    let mut spread: u32 = 7;

    if remade(crypt) {
        third = third.wrapping_add(0x22);
        spread = 16;
    }

    for i in 0..3usize {
        let mut told = i32::from(third);

        for j in 0..PLANE {
            let mut dodge = false;

            let one = salt[j & 0x7F];
            let two = salt[(j + i) % 0x80];
            let was = key[i * PLANE + j];
            let mixed = one ^ was;

            let round = ((u32::from(two) | (u32::from(one) << 8)) % spread) as u8;
            let mut now = mixed;

            match round {
                1 if two.is_multiple_of(0x0B) => now = was,
                2 if one.is_multiple_of(0x1D) => now = !mixed,
                3 if (u32::from(round) + j as u32).is_multiple_of(0x25) => now = two ^ mixed,
                4 if (u32::from(one) + u32::from(two)).is_multiple_of(97) => {
                    now = one.wrapping_add(mixed)
                }
                5 if (j as u32)
                    .wrapping_mul(u32::from(round))
                    .is_multiple_of(0x7B) =>
                {
                    now = mixed ^ told as u8
                }
                6 if one == 0xFF && two == 0 => {
                    now = 0;
                    dodge = true;
                }
                7 if stirred(crypt)
                    && ((u32::from(round) + j as u32).is_multiple_of(0x33)
                        || crypt >= PRO_REMADE_FROM) =>
                {
                    now ^= one
                }
                8 if stirred(crypt) && (one.is_multiple_of(0x1D) || crypt >= PRO_REMADE_FROM) => {
                    now ^= one
                }
                _ => {}
            }

            match ((j + i) as u32).is_multiple_of(u32::from(one % 5) + 1) {
                true => now ^= told as u8,
                false => {
                    if dodge {
                        now = !mixed;
                    }
                }
            }

            key[i * PLANE + j] = now;
            told += i as i32;
        }
    }
}

pub fn planed(crypt: u16, pw: &[u8; WORD], other: bool, salting: &[u8]) -> [u8; PLANES] {
    let mut factor = [0u8; 3];

    let first = pw[2];
    let second = pw[5];
    let third_of = pw[12];
    let third = folded(pw, other);

    let seed = u32::from(first) * u32::from(second) + u32::from(third_of) + u32::from(third);
    let mut roll = Rolling::from(seed);

    factor[usize::from(third % 3)] = (roll.next() % 256) as u8;

    if !other && remade(crypt) {
        let drawn = (roll.next() % 0xFB) as u8;

        factor[1] = match SECOND_FACTOR_REPLACES {
            true => drawn,
            false => factor[1].wrapping_add(drawn),
        };
    }

    let mut key = [0u8; PLANES];

    for i in 0..PLANE {
        let paired = (roll.next() & 0xFFFF) as u16;

        key[i] = factor[0] ^ (roll.next() & 0xFF) as u8;
        key[i + PLANE] = factor[1] ^ (paired >> 8) as u8;
        key[i + 2 * PLANE] = factor[2] ^ paired as u8;
    }

    if other {
        glazed(crypt, &mut key, third, salting);
    }

    key
}

pub fn streamed(key: &[u8; PLANES], body: &mut [u8], at: u64, crypt: u16) {
    let mut one = (at % 256) as usize;
    let mut two = ((at / 256) % 256) as usize;
    let mut three = ((at / 0x10000) % 256) as usize;

    if remade(crypt) {
        let mut bent = [0u8; 2 * PLANE];

        for (i, held) in bent.iter_mut().enumerate() {
            *held = key[i % PLANE] ^ (7u32.wrapping_mul(i as u32) as u8);
        }

        for held in body.iter_mut() {
            *held ^= bent[one] ^ bent[two + PLANE];

            one += 1;
            if one == PLANE {
                one = 0;
                two = (two + 1) % PLANE;
            }
        }

        return;
    }

    for held in body.iter_mut() {
        *held ^= key[one] ^ key[two + PLANE] ^ key[three + 2 * PLANE];

        one += 1;
        if one == PLANE {
            one = 0;
            two += 1;

            if two == PLANE {
                two = 0;
                three = (three + 1) % PLANE;
            }
        }
    }
}

pub fn aesed(crypt: u16, pw: &[u8; WORD]) -> ([u8; 16], [u8; 16]) {
    let mut key = [0u8; 16];
    let mut once = [0u8; 16];

    for i in 0..WORD {
        let step = i as u32;

        match remade(crypt) {
            true => {
                let key_at = (((step * 7) ^ (3 * u32::from(pw[i]))) % WORD as u32) as usize;
                let once_at =
                    (((step * 0x0B) ^ (5 * u32::from(pw[(i + 3) % WORD]))) % WORD as u32) as usize;

                key[i] ^= ((step + (u32::from(pw[key_at]) << (i % 3))) % 0xFB) as u8;
                once[i] ^= (((u32::from(pw[once_at]) >> (i % 2)) + step * step) % 0xF6) as u8;

                key[WORD] ^= ((7 * (u32::from(pw[i]) + step + 1)) % 0xFD) as u8;

                let lowered = u32::from(pw[i]).wrapping_sub(step * 2) as u16;
                once[WORD] ^= ((11 * u32::from(lowered)) % 0x100) as u8;
            }
            false => {
                key[i] ^= u32::from(pw[(i * 7) % WORD]).wrapping_add(step * step) as u8;
                once[i] ^= u32::from(pw[(i * 11) % WORD]).wrapping_sub(step * step) as u8;

                key[WORD] ^= u32::from(pw[i]).wrapping_add(step * 3) as u8;
                once[WORD] ^= u32::from(pw[i]).wrapping_add(step * 5) as u8;
            }
        }
    }

    (key, once)
}

pub fn bodied(crypt: u16, pw: &[u8; WORD], whole: u64) -> Option<u64> {
    if whole > i32::MAX as u64 {
        return None;
    }

    let past = whole.checked_sub(HEAD_LEN)?;
    if past < LEAST_BODY {
        return None;
    }

    if !remade(crypt) {
        return Some(LEAST_BODY);
    }

    let mut seed = u32::from(pw[2]) * u32::from(pw[4]) + u32::from(pw[12]);
    if seed == 0 {
        seed = 1;
    }

    let mut shift = Shifting(seed);
    shift.next();

    if whole >= u64::from(shift.next() % 500 + 800) {
        shift.next();
    }

    let mut body = past as u32;

    if body >= shift.next() % 500 + 800 {
        body = shift.next() % 500 + 800;
    }

    Some(u64::from(body))
}

fn stamped(out: &mut [u8; MASK_LEN], at: usize, value: u16) {
    let bytes = value.to_le_bytes();

    out[at * 2] = bytes[0];
    out[at * 2 + 1] = bytes[1];
}

fn stamped_wide(out: &mut [u8; MASK_LEN], at: usize, value: u32) {
    let bytes = value.to_le_bytes();

    for (i, one) in bytes.iter().enumerate() {
        out[at * 2 + i] = *one;
    }
}

pub fn addressed(pw: &[u8; WORD], crypt: u16) -> [u8; MASK_LEN] {
    let mut out = [0u8; MASK_LEN];

    if remade(crypt) {
        let seed = 0x0C + u32::from(pw[9]) * u32::from(pw[10]) + u32::from(pw[3]);
        let mut roll = Rolling::from(seed);
        let mut at = 0usize;

        for _ in 0..2 {
            for j in (0..4).rev() {
                stamped(&mut out, at + j, roll.next() as u16);
            }

            at += 4;
        }

        let low = u64::from(roll.next()) << 17;
        let high = u64::from(roll.next()) << 31;

        let near = ((low & 0xFFFF_FFFF) | (high & 0xFFFF_FFFF) | u64::from(roll.next())) as u32;
        let far = ((low >> 32) | (high >> 32)) as u32;

        stamped_wide(&mut out, at, near);
        stamped_wide(&mut out, at + 2, far);

        at += 4;

        for j in (0..4).rev() {
            stamped(&mut out, at + j, roll.next() as u16);
        }

        return out;
    }

    let seed = u32::from(pw[0]) + u32::from(pw[7]) * u32::from(pw[12]);
    let mut roll = Rolling::from(seed);
    let mut at = 0usize;

    for _ in 0..4 {
        for j in (0..4).rev() {
            stamped(&mut out, at + j, roll.next() as u16);
        }

        at += 4;
    }

    out
}

pub fn worded(pw: &[u8]) -> Option<[u8; WORD]> {
    pw.get(..WORD).and_then(|some| some.try_into().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::wolf_rpg::archive::RESERVE;
    use crate::engine::wolf_rpg::unseal;

    const FROM_REFERENCE: &str = "these bytes are what the UberWolf reference prints for this very \
                                  input, taken from the vectors harness rather than worked out by \
                                  hand";

    fn keyed_331() -> Vec<u8> {
        unseal::spoken(0x14B)
            .expect("the key string 3.31 ships with")
            .to_vec()
    }

    fn keyed_350() -> Vec<u8> {
        unseal::spoken(0x15E)
            .expect("the key string 3.50 ships with")
            .to_vec()
    }

    #[test]
    fn the_hash_the_older_editor_leans_on_is_the_same_crc_the_whole_world_calls_crc_thirty_two() {
        assert_eq!(
            hashed(b"123456789"),
            0xcbf4_3926,
            "the reference builds its own table from the reflected polynomial and the harness \
             prints cbf43926 for the nine digits, which is the check value every CRC-32 catalogue \
             lists, so the crate can stand in for the table"
        );
        assert_eq!(hashed(b"DXBDXARC"), 0x2f95_0e06, "{FROM_REFERENCE}");
        assert_eq!(hashed(b""), 0);
    }

    #[test]
    fn the_seven_byte_pad_a_three_double_zero_archive_is_sealed_with_comes_off_its_own_key_string()
    {
        let pass = unseal::spoken(0x12C).expect("the key string");

        assert_eq!(pass.len(), 40, "the trailing nul is not key material");
        assert_eq!(
            padded(pass),
            [0xbb, 0x12, 0xf2, 0x1d, 0x5e, 0x6b, 0xaa],
            "{FROM_REFERENCE}"
        );
    }

    #[test]
    fn the_seven_byte_pad_a_three_fourteen_archive_is_sealed_with_comes_off_its_own_key_string() {
        let pass = unseal::spoken(0x13A).expect("the key string");

        assert_eq!(pass.len(), 45, "the trailing nul is not key material");
        assert_eq!(
            padded(pass),
            [0x3f, 0x05, 0xf9, 0xee, 0xf7, 0x1e, 0xcc],
            "{FROM_REFERENCE}"
        );
    }

    #[test]
    fn a_source_shorter_than_four_bytes_is_lengthened_with_the_string_the_library_falls_back_on() {
        assert_eq!(
            padded(b"ab"),
            [0xa0, 0xd8, 0x1f, 0xf2, 0x0f, 0xbf, 0x4d],
            "{FROM_REFERENCE}"
        );
    }

    #[test]
    fn the_pad_a_single_file_is_sealed_with_reads_its_own_name_and_then_the_folders_holding_it() {
        let pass = unseal::spoken(0x12C).expect("the key string");

        assert_eq!(
            padded(&sourced(pass, b"GAME.DAT", b"")),
            [0x42, 0x88, 0x44, 0xc5, 0xca, 0x41, 0x74],
            "{FROM_REFERENCE}"
        );
        assert_eq!(
            padded(&sourced(pass, b"MAP001.MPS", b"MAPDATA")),
            [0x56, 0xfe, 0x59, 0x64, 0xaf, 0xec, 0x0a],
            "{FROM_REFERENCE}"
        );
        assert_eq!(
            padded(&sourced(pass, b"FACE01.PNG", b"FACEPICTURE")),
            [0x58, 0xec, 0xb8, 0x48, 0x57, 0x94, 0x70],
            "the innermost folder comes first and the root is left out entirely. {FROM_REFERENCE}"
        );
        assert_eq!(
            padded(&sourced(pass, b"DATABASE.DAT", b"")),
            [0xfc, 0x31, 0x1f, 0xef, 0x10, 0x96, 0xbe],
            "{FROM_REFERENCE}"
        );
        assert_eq!(
            padded(&sourced(pass, b"WIDE.BIN", b"")),
            [0x60, 0x6c, 0x57, 0x94, 0xd9, 0x37, 0xf8],
            "{FROM_REFERENCE}"
        );
    }

    #[test]
    fn the_two_families_are_told_apart_by_the_version_the_archive_carries_and_nothing_else() {
        assert!(
            !newer(0x12C) && !newer(0x13A),
            "3.00 and 3.14 are the classic cipher"
        );
        assert!(
            newer(0x14B) && newer(0x15E),
            "3.31 and 3.50 are the plane cipher"
        );
        assert!(
            !newer(0x64) && !newer(0xC8),
            "the two chacha modes are neither"
        );
        assert!(
            !newer(1000) && !newer(1009),
            "the pro band in between is refused"
        );
        assert!(newer(1010));

        assert!(!remade(0x14B), "3.31 keeps the three plane stream");
        assert!(remade(0x15E), "3.50 folds the planes into one bent table");
        assert!(!remade(0x3E8) && !remade(0x3FB));
        assert!(remade(0x3FC));
    }

    #[test]
    fn the_three_key_planes_a_body_read_is_unsealed_with_come_out_as_the_reference_lays_them() {
        let inner = planed(0x14B, &RESERVE, false, &keyed_331());

        assert_eq!(
            &inner[..8],
            &[0xb7, 0x80, 0x2f, 0x93, 0x06, 0x57, 0xdf, 0x2e],
            "{FROM_REFERENCE}"
        );
        assert_eq!(
            &inner[248..256],
            &[0xf4, 0xb1, 0xf6, 0x36, 0xfc, 0xf2, 0x16, 0xa4]
        );
        assert_eq!(
            &inner[256..264],
            &[0x05, 0x3c, 0x1d, 0x30, 0x36, 0x33, 0x29, 0x3c]
        );
        assert_eq!(
            &inner[512..520],
            &[0x55, 0x77, 0x61, 0x75, 0x5c, 0xde, 0x7e, 0xd5]
        );
        assert_eq!(
            hashed(&inner),
            0x968c_e888,
            "every one of the 768 bytes. {FROM_REFERENCE}"
        );

        let inner = planed(0x15E, &RESERVE, false, &keyed_350());

        assert_eq!(
            &inner[..8],
            &[0xb1, 0xa7, 0xb3, 0x9a, 0x18, 0xb8, 0x13, 0xca],
            "{FROM_REFERENCE}"
        );
        assert_eq!(
            &inner[760..768],
            &[0x32, 0x77, 0x30, 0xf0, 0x3a, 0x34, 0xd0, 0x62]
        );
        assert_eq!(
            hashed(&inner),
            0x05ca_0e5d,
            "every one of the 768 bytes. {FROM_REFERENCE}"
        );
    }

    #[test]
    fn the_second_factor_three_fifty_draws_for_a_body_read_is_written_over_rather_than_added_to() {
        const FIVES: [u8; WORD] = [5u8; WORD];

        assert_eq!(
            folded(&FIVES, false) % 3,
            1,
            "the reference draws one factor and stores it at the seed byte modulo three, then at \
             3.50 only draws a second and stores it at one. The harness searched out this password \
             as one where those two land on the same slot, so it is the only shape of input where \
             writing over and adding to part company"
        );

        let inner = planed(OWN_SALT, &FIVES, false, &keyed_350());

        assert_eq!(
            &inner[256..264],
            &[0xf9, 0xed, 0xc2, 0xe3, 0xcd, 0xd7, 0xc2, 0x86],
            "{FROM_REFERENCE}"
        );
        assert_eq!(
            hashed(&inner),
            0x7648_3e23,
            "the reference writes that second factor with a plain assignment and carries a note \
             beside it saying it might have to be an add instead. Every retail game the reference \
             opens is opened with the plain write, so the plain write is what this ships and \
             SECOND_FACTOR_REPLACES is the one line to turn over if a game ever says otherwise. \
             {FROM_REFERENCE}"
        );

        assert_eq!(
            folded(&RESERVE, false) % 3,
            0,
            "with the password every other vector here uses the two readings agree, which is why \
             none of them could have told them apart"
        );
    }

    #[test]
    fn the_outer_key_the_whole_archive_is_wrapped_in_is_salted_and_stirred_the_reference_way() {
        let outer = planed(0x14B, &RESERVE, true, &keyed_331());

        assert_eq!(
            &outer[..8],
            &[0x44, 0x1c, 0xb3, 0xb9, 0x73, 0xc8, 0xd9, 0xc3],
            "{FROM_REFERENCE}"
        );
        assert_eq!(
            hashed(&outer),
            0xe2b2_c204,
            "every one of the 768 bytes. {FROM_REFERENCE}"
        );

        let outer = planed(0x15E, &RESERVE, true, &keyed_350());

        assert_eq!(
            &outer[..8],
            &[0x95, 0x21, 0xe5, 0xde, 0xcf, 0xfc, 0x38, 0xea],
            "{FROM_REFERENCE}"
        );
        assert_eq!(
            hashed(&outer),
            0x0ee7_7cf7,
            "every one of the 768 bytes. {FROM_REFERENCE}"
        );
    }

    #[test]
    fn the_salt_the_outer_key_is_stirred_with_comes_off_the_key_string_except_at_three_fifty() {
        assert_eq!(
            &salted(&keyed_331())[..8],
            &[0xca, 0x08, 0x4c, 0x5d, 0x17, 0x0d, 0xda, 0xa1],
            "{FROM_REFERENCE}"
        );
        assert_eq!(
            &salted(b"958")[..8],
            &[0x39, 0x35, 0x38, 0x3a, 0x36, 0x39, 0x3b, 0x37],
            "3.50 throws its own key string away and salts with the three digits 958 instead. \
             {FROM_REFERENCE}"
        );
        assert_eq!(
            &salted(b"958")[120..],
            &[0x61, 0x5d, 0x60, 0x62, 0x5e, 0x61, 0x63, 0x5f]
        );
        assert_eq!(
            salted(b""),
            [0u8; SALT],
            "an empty salt source is no divisor at all"
        );
    }

    #[test]
    fn the_stream_a_body_is_unsealed_with_lands_on_the_same_bytes_the_reference_lays_at_a_position()
    {
        for (crypt, salting, at_nought, at_thousand, at_seventy_thousand) in [
            (
                0x14Bu16,
                keyed_331(),
                [0xe7, 0xd0, 0x7f, 0xc3, 0x56, 0x07, 0x8f, 0x7e],
                [0xcd, 0xbd, 0x54, 0x4b, 0x1c, 0xed, 0xdd, 0xfb],
                [0xfd, 0xe9, 0xef, 0xa9, 0xeb, 0x29, 0xdd, 0x4e],
            ),
            (
                0x15E,
                keyed_350(),
                [0x00, 0x11, 0x0c, 0x3e, 0xb5, 0x2a, 0x88, 0x4a],
                [0x3c, 0xe6, 0x32, 0x25, 0x28, 0x1a, 0x4a, 0x9f],
                [0xdc, 0x7e, 0xff, 0xcc, 0x9d, 0xaf, 0x77, 0xc0],
            ),
        ] {
            let key = planed(crypt, &RESERVE, false, &salting);

            let mut body = [0u8; 8];
            streamed(&key, &mut body, 0, crypt);
            assert_eq!(body, at_nought, "at nought, {crypt:#x}. {FROM_REFERENCE}");

            let mut body = [0u8; 8];
            streamed(&key, &mut body, 1000, crypt);
            assert_eq!(
                body, at_thousand,
                "at a thousand, {crypt:#x}. {FROM_REFERENCE}"
            );

            let mut body = [0u8; 8];
            streamed(&key, &mut body, 70000, crypt);
            assert_eq!(
                body, at_seventy_thousand,
                "past the sixty five thousandth byte the third plane starts turning, and a picture \
                 archive is far bigger than that. {crypt:#x}. {FROM_REFERENCE}"
            );
        }
    }

    #[test]
    fn the_outer_stream_covers_the_archive_from_sixty_four_and_stops_sixty_four_short_of_the_end() {
        let whole = 2000usize;

        for (crypt, salting, at_head, at_thousand, at_last) in [
            (
                0x14Bu16,
                keyed_331(),
                [0x90, 0xd4, 0x1b, 0xf5, 0xfc, 0xf8, 0x3e, 0x4b],
                [0xa4, 0xe6, 0x7a, 0x69, 0x4a, 0x9e, 0x7b, 0x4d],
                [0x91, 0x50, 0x83, 0x6d, 0xff, 0x29, 0xdd, 0x8f],
            ),
            (
                0x15E,
                keyed_350(),
                [0x87, 0x32, 0xd1, 0xb2, 0x4c, 0x19, 0x15, 0xe7],
                [0xee, 0x49, 0x31, 0xa6, 0x63, 0x15, 0x03, 0x02],
                [0x0f, 0xa2, 0x7e, 0xae, 0x89, 0xa5, 0x4a, 0x1e],
            ),
        ] {
            let key = planed(crypt, &RESERVE, true, &salting);
            let mut room = vec![0u8; whole];

            let over = whole - 64;
            streamed(&key, &mut room[64..over], 64, crypt);

            assert_eq!(&room[64..72], &at_head, "{crypt:#x}. {FROM_REFERENCE}");
            assert_eq!(
                &room[1000..1008],
                &at_thousand,
                "{crypt:#x}. {FROM_REFERENCE}"
            );
            assert_eq!(&room[1928..1936], &at_last, "{crypt:#x}. {FROM_REFERENCE}");
            assert_eq!(
                &room[1936..1944],
                &[0u8; 8],
                "the reference hands the pass a start of 64 and an end of size minus 64 and the \
                 pass runs end minus start bytes from the start, so the last sixty four bytes of \
                 an archive are left to the block cipher alone. Deciding to read that literally is \
                 the whole of this assertion. {crypt:#x}. {FROM_REFERENCE}"
            );
        }
    }

    #[test]
    fn the_block_cipher_key_and_counter_are_folded_out_of_the_password_the_header_carries() {
        assert_eq!(
            aesed(0x14B, &RESERVE),
            (
                [
                    0x11, 0x89, 0x03, 0x80, 0xfe, 0x7f, 0x01, 0x86, 0x0c, 0x95, 0x1f, 0xac, 0x3a,
                    0xcb, 0x5d, 0x3d
                ],
                [
                    0x11, 0xcb, 0x84, 0x3b, 0xef, 0xa2, 0x53, 0x02, 0xae, 0x59, 0x02, 0xa9, 0x4d,
                    0xf0, 0x91, 0xfb
                ]
            ),
            "{FROM_REFERENCE}"
        );
        assert_eq!(
            aesed(0x15E, &RESERVE),
            (
                [
                    0x77, 0x16, 0x8a, 0x8b, 0x6a, 0x1a, 0x6c, 0xd3, 0x61, 0x0d, 0x4e, 0xd2, 0x3f,
                    0xcc, 0x4d, 0xf5
                ],
                [
                    0xbb, 0x78, 0x9d, 0x6f, 0x19, 0x6e, 0x79, 0x9f, 0x73, 0xa6, 0x18, 0x02, 0x99,
                    0xed, 0x9a, 0x56
                ]
            ),
            "3.50 folds the password through a pair of shuffled indexes instead. {FROM_REFERENCE}"
        );
    }

    #[test]
    fn the_thirty_two_bytes_that_hide_the_four_addresses_are_drawn_off_the_password_and_the_version()
     {
        assert_eq!(
            addressed(&RESERVE, 0x14B),
            [
                0xe7, 0x26, 0xa2, 0x4a, 0x00, 0x52, 0xc4, 0x7f, 0x5c, 0x3b, 0x90, 0x5c, 0x78, 0x11,
                0x0b, 0x43, 0x0b, 0x24, 0xef, 0x37, 0xbb, 0x41, 0x31, 0x75, 0xc6, 0x4d, 0x7e, 0x4c,
                0xf9, 0x3d, 0xff, 0x0c
            ],
            "{FROM_REFERENCE}"
        );
        assert_eq!(
            addressed(&RESERVE, 0x15E),
            [
                0x15, 0x78, 0xbe, 0x20, 0xc7, 0x0a, 0xb0, 0x16, 0x43, 0x77, 0x83, 0x34, 0x46, 0x3d,
                0x85, 0x16, 0xb3, 0x29, 0xa0, 0xe9, 0x96, 0x16, 0x00, 0x00, 0x12, 0x4f, 0x6d, 0x71,
                0x86, 0x19, 0x3b, 0x59
            ],
            "3.50 hides the middle eight bytes behind a pair of wide draws instead of four narrow \
             ones, which is why two of them come out nought. {FROM_REFERENCE}"
        );
    }

    #[test]
    fn the_run_of_bytes_the_block_cipher_covers_is_a_thousand_and_twenty_four_until_three_fifty() {
        assert_eq!(bodied(0x14B, &RESERVE, 1088), Some(1024));
        assert_eq!(bodied(0x14B, &RESERVE, 2000), Some(1024));
        assert_eq!(bodied(0x14B, &RESERVE, 5_000_000), Some(1024));

        assert_eq!(
            bodied(0x15E, &RESERVE, 1088),
            Some(1024),
            "{FROM_REFERENCE}"
        );
        assert_eq!(bodied(0x15E, &RESERVE, 2000), Some(956), "{FROM_REFERENCE}");
        assert_eq!(
            bodied(0x15E, &RESERVE, 100_000),
            Some(956),
            "{FROM_REFERENCE}"
        );
        assert_eq!(
            bodied(0x15E, &RESERVE, 5_000_000),
            Some(956),
            "{FROM_REFERENCE}"
        );

        assert_eq!(
            bodied(0x15E, &RESERVE, 1087),
            None,
            "the reference gives up on an archive with fewer than 0x400 bytes past its head rather \
             than reaching past the end, and so does this"
        );
        assert_eq!(
            bodied(0x15E, &RESERVE, u64::from(u32::MAX)),
            None,
            "the reference measures the archive into a signed four byte count, so a file over two \
             gigabytes turns the length negative and it gives up. Refusing is the honest reading"
        );
    }

    #[test]
    fn a_password_shorter_than_the_fifteen_bytes_the_header_reserves_hands_back_nothing() {
        assert!(worded(&[0u8; 14]).is_none());
        assert!(worded(&[0u8; 15]).is_some());
        assert!(
            worded(&[0u8; 64]).is_some(),
            "the whole header is longer than the password in it"
        );
    }
}
