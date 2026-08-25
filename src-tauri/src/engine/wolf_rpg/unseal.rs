use crate::engine::wolf_rpg::{aes, keying};
use chacha20::ChaCha20;
use chacha20::cipher::{KeyIvInit, StreamCipher, StreamCipherSeek};
use std::ops::Range;

const SEED_LEN: usize = 32;
const ONCE_AT: usize = 34;
const ONCE_LEN: usize = 12;
const CARRIER_TRIM: u32 = 31;
const BLOCK: u64 = 64;

const CARRIED: u16 = 0xC8;
pub const SHIPPED: u16 = 0x64;

pub const SPOKEN_300: u16 = 0x12C;
pub const SPOKEN_314: u16 = 0x13A;
pub const SPOKEN_331: u16 = 0x14B;
pub const SPOKEN_350: u16 = 0x15E;

const KEY_300: &[u8] = &[
    0x0F, 0x53, 0xE1, 0x3E, 0x8E, 0xB5, 0x41, 0x91, 0x52, 0x16, 0x55, 0xAE, 0x34, 0xC9, 0x8F, 0x79,
    0x59, 0x2F, 0x59, 0x6B, 0x95, 0x19, 0x9B, 0x1B, 0x35, 0x9A, 0x2F, 0xDE, 0xC9, 0x7C, 0x12, 0x96,
    0xC3, 0x14, 0xB5, 0x0F, 0x53, 0xE1, 0x3E, 0x8E,
];

const KEY_314: &[u8] = &[
    0x31, 0xF9, 0x01, 0x36, 0xA3, 0xE3, 0x8D, 0x3C, 0x7B, 0xC3, 0x7D, 0x25, 0xAD, 0x63, 0x28, 0x19,
    0x1B, 0xF7, 0x8E, 0x6C, 0xC4, 0xE5, 0xE2, 0x76, 0x82, 0xEA, 0x4F, 0xED, 0x61, 0xDA, 0xE0, 0x44,
    0x5B, 0xB6, 0x46, 0x3B, 0x06, 0xD5, 0xCE, 0xB6, 0x78, 0x58, 0xD0, 0x7C, 0x82,
];

const KEY_331: &[u8] = &[
    0xCA, 0x08, 0x4C, 0x5D, 0x17, 0x0D, 0xDA, 0xA1, 0xD7, 0x27, 0xC8, 0x41, 0x54, 0x38, 0x82, 0x32,
    0x54, 0xB7, 0xF9, 0x46, 0x8E, 0x13, 0x6B, 0xCA, 0xD0, 0x5C, 0x95, 0x95, 0xE2, 0xDC, 0x03, 0x53,
    0x60, 0x9B, 0x4A, 0x38, 0x17, 0xF3, 0x69, 0x59, 0xA4, 0xC7, 0x9A, 0x43, 0x63, 0xE6, 0x54, 0xAF,
    0xDB, 0xBB, 0x43, 0x58,
];

const KEY_350: &[u8] = &[
    0xD2, 0x84, 0xCE, 0x28, 0xCE, 0x88, 0x82, 0xE4, 0x2A, 0x18, 0x2E, 0x4C, 0x06, 0xB4, 0xEA, 0x84,
    0x06, 0xB8, 0xC6, 0x88, 0x5A, 0xA0, 0x9E, 0x7C, 0x56, 0x40, 0xBA, 0x34, 0x52, 0xCC, 0xC6, 0x7C,
    0x2E, 0x14, 0x12, 0x68, 0xFE, 0x5C, 0x76, 0x94, 0x86, 0x78, 0x8E, 0x4C, 0xBE, 0x88, 0x66, 0x9C,
    0x1E, 0xE0, 0x8E, 0x6C,
];

fn carried(carrier: u32) -> Vec<u8> {
    let size = carrier.wrapping_sub(CARRIER_TRIM);

    let crossed = size ^ 0x70;
    let summed = size % 0x064B + 152;
    let quarter = (size / 4) + 1285;
    let halved = (size / 2) + 171;

    let grain: [u8; 4] = [
        (quarter ^ halved) as u8,
        crossed.wrapping_add(summed) as u8,
        crossed.wrapping_sub(quarter) as u8,
        quarter.wrapping_mul(crossed) as u8,
    ];

    const ONE: [u8; 4] = [0x3F, 0xA7, 0xD2, 0x1C];
    const TWO: [u8; 4] = [0xB4, 0xE1, 0x9D, 0x58];
    const THREE: [u8; 4] = [0x6A, 0x2B, 0x4C, 0x8E];

    (0..64u32)
        .map(|i| {
            let at = (i % 4) as usize;
            let mixed = grain[at].wrapping_add(TWO[at]) ^ ONE[at].wrapping_add((17 * i) as u8);
            let turned = match i % 2 {
                0 => mixed.rotate_left(3),
                _ => mixed.rotate_left(6),
            };

            !(turned ^ grain[at] ^ THREE[at])
        })
        .collect()
}

fn laid(seed: [u8; SEED_LEN], once: [u8; ONCE_LEN]) -> Vec<u8> {
    let mut out = vec![0u8; ONCE_AT + ONCE_LEN];
    out[..SEED_LEN].copy_from_slice(&seed);
    out[ONCE_AT..].copy_from_slice(&once);

    out
}

fn shipped(crypt: u16) -> Option<Vec<u8>> {
    match crypt {
        SHIPPED => Some(laid(
            [
                0xC9, 0x82, 0xF8, 0xB4, 0x2C, 0x93, 0x9E, 0x83, 0x0E, 0xBC, 0xBC, 0x92, 0x68, 0x8D,
                0x59, 0xA1, 0x4A, 0x9E, 0x7F, 0xB0, 0xAC, 0xAF, 0x1D, 0x8F, 0x8E, 0xB8, 0x3B, 0x9E,
                0xE8, 0x89, 0xD9, 0xAD,
            ],
            [
                0xFF, 0xBC, 0x2D, 0xAB, 0x9D, 0x8B, 0x0F, 0xB4, 0xBB, 0x9A, 0x69, 0x85,
            ],
        )),
        _ => None,
    }
}

pub fn spoken(crypt: u16) -> Option<&'static [u8]> {
    match crypt {
        SPOKEN_300 => Some(KEY_300),
        SPOKEN_314 => Some(KEY_314),
        SPOKEN_331 => Some(KEY_331),
        SPOKEN_350 => Some(KEY_350),
        _ => None,
    }
}

pub fn keyed(crypt: u16, carrier: Option<u32>) -> Option<Vec<u8>> {
    if let Some(said) = spoken(crypt) {
        return Some(said.to_vec());
    }

    shipped(crypt).or_else(|| match crypt == CARRIED {
        true => carrier.map(carried),
        false => None,
    })
}

fn unkey(key: &[u8], body: &mut [u8], at: u64) -> Result<(), String> {
    let seed: &[u8; SEED_LEN] = key
        .get(..SEED_LEN)
        .and_then(|some| some.try_into().ok())
        .ok_or("the archive key is too short to hold a seed")?;

    let once: &[u8; ONCE_LEN] = key
        .get(ONCE_AT..ONCE_AT + ONCE_LEN)
        .and_then(|some| some.try_into().ok())
        .ok_or("the archive key is too short to hold a nonce")?;

    let mut cipher = ChaCha20::new(seed.into(), once.into());

    cipher
        .try_seek(at.saturating_add(BLOCK))
        .map_err(|_| format!("this archive seals a file at {at}, which its key cannot reach"))?;

    cipher.apply_keystream(body);

    Ok(())
}

fn padding(body: &mut [u8], pad: &[u8; keying::PAD], at: u64) {
    let mut which = (at % keying::PAD as u64) as usize;

    for one in body.iter_mut() {
        *one ^= pad[which];

        which += 1;
        if which == keying::PAD {
            which = 0;
        }
    }
}

fn shared(len: usize, abs: u64, from: u64, upto: u64) -> Option<(Range<usize>, u64)> {
    let end = abs.saturating_add(len as u64);

    let low = abs.max(from);
    let high = end.min(upto);

    match low < high {
        true => Some(((low - abs) as usize..(high - abs) as usize, low)),
        false => None,
    }
}

#[derive(Clone)]
pub struct Fresh {
    read: Option<Box<[u8; keying::PLANES]>>,
    outer: Box<[u8; keying::PLANES]>,
    key: [u8; aes::BLOCK],
    once: [u8; aes::BLOCK],
    word: [u8; keying::WORD],
    body: u64,
    names_at: u64,
    whole: u64,
    crypt: u16,
}

impl Fresh {
    fn turned(&self, body: &mut [u8], dxa: u64, abs: u64) {
        if let Some(read) = &self.read {
            keying::streamed(read, body, dxa, self.crypt);
        }

        let over = self.whole.saturating_sub(keying::HEAD_LEN);
        if let Some((span, at)) = shared(body.len(), abs, keying::HEAD_LEN, over) {
            keying::streamed(&self.outer, &mut body[span], at, self.crypt);
        }

        let leading = keying::HEAD_LEN.saturating_add(self.body);
        if let Some((span, at)) = shared(body.len(), abs, keying::HEAD_LEN, leading) {
            aes::streamed(
                &mut body[span],
                &self.key,
                &self.once,
                at - keying::HEAD_LEN,
            );
        }

        let block = aes::BLOCK as u64;
        let after = self.body.div_ceil(block).saturating_mul(block);

        if let Some((span, at)) = shared(body.len(), abs, self.names_at, self.whole) {
            aes::streamed(
                &mut body[span],
                &self.key,
                &self.once,
                after.saturating_add(at - self.names_at),
            );
        }
    }
}

#[derive(Clone)]
pub enum Seal {
    Loose,
    Shipped(Vec<u8>),
    Classic {
        arch: [u8; keying::PAD],
        pass: Vec<u8>,
    },
    Fresh(Box<Fresh>),
}

pub fn sealed(
    crypt: u16,
    key: &[u8],
    head: &[u8],
    keyless: bool,
    whole: u64,
    names_at: u64,
) -> Result<Seal, String> {
    match crypt {
        SPOKEN_331 | SPOKEN_350 => {
            let word = keying::worded(head.get(keying::RESERVE_AT..).unwrap_or_default()).ok_or(
                "this archive is sealed the newer way but keeps no password where the editor \
                 writes one",
            )?;

            let body = keying::bodied(crypt, &word, whole).ok_or_else(|| {
                format!(
                    "this archive is {whole} bytes long, which the newer seal cannot measure a \
                     body out of"
                )
            })?;

            let (aes_key, once) = keying::aesed(crypt, &word);

            Ok(Seal::Fresh(Box::new(Fresh {
                read: (!keyless).then(|| Box::new(keying::planed(crypt, &word, false, key))),
                outer: Box::new(keying::planed(crypt, &word, true, key)),
                key: aes_key,
                once,
                word,
                body,
                names_at,
                whole,
                crypt,
            })))
        }
        _ if keyless => Ok(Seal::Loose),
        SHIPPED | CARRIED => Ok(Seal::Shipped(key.to_vec())),
        SPOKEN_300 | SPOKEN_314 => Ok(Seal::Classic {
            arch: keying::padded(key),
            pass: key.to_vec(),
        }),
        _ => Err(format!(
            "sealed the {crypt:#x} way, which this reader cannot open"
        )),
    }
}

impl Seal {
    pub fn per_file(&self) -> bool {
        matches!(self, Seal::Classic { .. })
    }

    pub fn fresh(&self) -> bool {
        matches!(self, Seal::Fresh(_))
    }

    pub fn filing(&self, name: &[u8], chain: &[u8]) -> [u8; keying::PAD] {
        match self {
            Seal::Classic { pass, .. } => keying::padded(&keying::sourced(pass, name, chain)),
            _ => [0u8; keying::PAD],
        }
    }

    pub fn turned(
        &self,
        body: &mut [u8],
        dxa: u64,
        abs: u64,
        pad: Option<&[u8; keying::PAD]>,
    ) -> Result<(), String> {
        match self {
            Seal::Loose => Ok(()),
            Seal::Shipped(key) => unkey(key, body, dxa),
            Seal::Classic { arch, .. } => {
                padding(body, pad.unwrap_or(arch), dxa);

                Ok(())
            }
            Seal::Fresh(fresh) => {
                fresh.turned(body, dxa, abs);

                Ok(())
            }
        }
    }

    pub fn moved(&self, whole: u64, names_at: u64, carried: u64) -> Result<Seal, String> {
        let Seal::Fresh(fresh) = self else {
            return Ok(self.clone());
        };

        let body = keying::bodied(fresh.crypt, &fresh.word, whole).ok_or_else(|| {
            format!("a written archive of {whole} bytes is one the newer seal cannot measure")
        })?;

        if body != fresh.body {
            return Err(format!(
                "the newer seal draws the length it covers from the size of the whole archive, and \
                 writing this one out at {whole} bytes moves that length from {} to {body}, which \
                 would hand the game an archive it cannot open",
                fresh.body
            ));
        }

        let reach = fresh.whole.min(whole).saturating_sub(keying::HEAD_LEN);
        if carried > reach {
            return Err(format!(
                "the bytes carried over untouched reach to {carried} and the outer pass only \
                 reaches {reach}, so copying them would leave the tail of the data sealed the \
                 wrong way"
            ));
        }

        let mut next = fresh.clone();
        next.body = body;
        next.names_at = names_at;
        next.whole = whole;

        Ok(Seal::Fresh(next))
    }
}

pub fn unmasked(crypt: u16, head: &mut [u8]) -> Result<(), String> {
    if !keying::newer(crypt) {
        return Ok(());
    }

    let word = keying::worded(head.get(keying::RESERVE_AT..).unwrap_or_default())
        .ok_or("this archive hides its addresses behind a password it does not carry")?;

    let mask = keying::addressed(&word, crypt);

    let room = head
        .get_mut(keying::MASK_AT..keying::MASK_AT + keying::MASK_LEN)
        .ok_or("this archive stops before the four addresses it hides")?;

    for (one, over) in room.iter_mut().zip(mask.iter()) {
        *one ^= over;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::wolf_rpg::archive;

    #[test]
    fn the_key_a_pro_game_hands_over_is_read_from_the_size_of_its_carrier() {
        let key = carried(24175);

        assert_eq!(key.len(), 64);
        assert_eq!(&key[..8], &[0xd1, 0xe7, 0xb2, 0x9c, 0x34, 0xf6, 0xd4, 0xab]);
        assert_eq!(&key[ONCE_AT..ONCE_AT + 4], &[0xb5, 0x94, 0x35, 0xce]);
    }

    #[test]
    fn a_carrier_too_small_to_hold_a_header_still_gives_a_key_of_its_own() {
        assert_eq!(carried(0).len(), 64);
        assert_eq!(carried(31).len(), 64);

        assert_ne!(
            carried(0),
            carried(31),
            "a carrier smaller than the header it is measured against wraps round rather than \
             running off the bottom, and two sizes that wrap must not land on one key"
        );
    }

    #[test]
    fn the_key_a_plain_game_ships_with_is_laid_out_where_the_reader_looks_for_it() {
        let key = shipped(SHIPPED).expect("the shipped key");

        assert_eq!(key.len(), ONCE_AT + ONCE_LEN);
        assert_eq!(key[0], 0xC9);
        assert_eq!(key[ONCE_AT], 0xFF);
        assert!(shipped(CARRIED).is_none(), "a Pro game carries its own key");
    }

    #[test]
    fn a_game_sealed_a_way_this_reader_does_not_know_hands_back_no_key_at_all() {
        assert!(keyed(0x1010, Some(24175)).is_none());
        assert!(
            keyed(CARRIED, None).is_none(),
            "a Pro game with no carrier beside it leaves nothing to read the key from"
        );
        assert!(keyed(CARRIED, Some(24175)).is_some());
    }

    #[test]
    fn a_file_sealed_further_out_than_the_keystream_reaches_says_so_rather_than_panicking() {
        let key = carried(24175);
        let mut body = [0u8; 16];

        assert!(
            unkey(&key, &mut body, u64::MAX - 1).is_err(),
            "the counter this cipher walks is four bytes wide, and asking it to jump past the end \
             of that is a refusal, never a panic in the middle of an import"
        );
        assert!(unkey(&key, &mut body, 0).is_ok());
    }

    #[test]
    fn each_of_the_four_wolf_three_seals_hands_back_the_key_string_the_editor_ships_with() {
        assert_eq!(keyed(SPOKEN_300, None).map(|key| key.len()), Some(40));
        assert_eq!(keyed(SPOKEN_314, None).map(|key| key.len()), Some(45));
        assert_eq!(keyed(SPOKEN_331, None).map(|key| key.len()), Some(52));
        assert_eq!(keyed(SPOKEN_350, None).map(|key| key.len()), Some(52));

        assert_eq!(
            spoken(SPOKEN_331).map(|key| key[0]),
            Some(0xCA),
            "the table in the reference terminates every one of these with a nul and measures them \
             with strlen, so the nul is not key material and must not be carried into the key"
        );
    }

    #[test]
    fn a_classic_archive_is_sealed_a_seven_byte_pad_at_a_time_from_wherever_the_read_starts() {
        let seal = sealed(SPOKEN_300, KEY_300, &[0u8; 64], false, 4000, 3000)
            .expect("a three double zero seal");

        let mut body: Vec<u8> = (0..24u8).collect();
        seal.turned(&mut body, 10, 900, None)
            .expect("the pad goes on");

        assert_eq!(
            body,
            vec![
                0x1d, 0x5f, 0x69, 0xa9, 0xbf, 0x17, 0xf4, 0x1a, 0x56, 0x62, 0xa0, 0xb0, 0x1e, 0xff,
                0x13, 0x51, 0x7b, 0xbb, 0xa9, 0x01, 0xe6, 0x08, 0x48, 0x7c
            ],
            "these bytes are what the UberWolf reference prints for the same twenty four bytes at \
             the same position, taken from the vectors harness"
        );

        seal.turned(&mut body, 10, 900, None)
            .expect("the pad comes off");
        assert_eq!(
            body,
            (0..24u8).collect::<Vec<u8>>(),
            "the pad is its own undoing"
        );
    }

    #[test]
    fn the_classic_pad_wraps_on_the_position_of_the_read_and_not_on_where_it_lands_in_the_file() {
        let seal = sealed(SPOKEN_314, KEY_314, &[0u8; 64], false, 4000, 3000)
            .expect("a three fourteen seal");

        let mut once = vec![0u8; 21];
        let mut again = vec![0u8; 21];

        seal.turned(&mut once, 0, 64, None)
            .expect("sealed at nought");
        seal.turned(&mut again, 7, 900, None)
            .expect("sealed at seven");

        assert_eq!(
            once, again,
            "the reference takes the position modulo seven before it walks the pad, so a read at \
             seven lands on the same pad as a read at nought however far apart the two sit in the \
             archive"
        );
    }

    #[test]
    fn a_newer_archive_reaches_for_its_password_in_the_header_and_refuses_a_header_without_one() {
        assert!(
            sealed(SPOKEN_331, KEY_331, &[0u8; 40], false, 4000, 3000).is_err(),
            "the fifteen bytes the password lives in start at 49, so a header cut short of that \
             has to be refused rather than read out of bounds"
        );
        assert!(sealed(SPOKEN_331, KEY_331, &[0u8; 64], false, 4000, 3000).is_ok());
        assert!(
            sealed(SPOKEN_331, KEY_331, &[0u8; 64], false, 200, 150).is_err(),
            "an archive too short to hold the run the block cipher covers is refused"
        );
    }

    #[test]
    fn an_archive_that_says_it_holds_no_key_still_has_its_addresses_and_its_outside_unwrapped() {
        let mut head = [0u8; 64];
        head[keying::RESERVE_AT..].copy_from_slice(&archive::RESERVE);

        let with = sealed(SPOKEN_331, KEY_331, &head, false, 2000, 1500).expect("a seal");
        let without = sealed(SPOKEN_331, KEY_331, &head, true, 2000, 1500).expect("a seal");

        let mut one = vec![0u8; 32];
        let mut two = vec![0u8; 32];
        with.turned(&mut one, 0, 200, None).expect("sealed");
        without.turned(&mut two, 0, 200, None).expect("sealed");

        assert_ne!(
            one, two,
            "the read stream is the only layer the no key flag turns off"
        );
        assert_ne!(
            two,
            vec![0u8; 32],
            "the reference gates only the per read pass on that flag and wraps the archive from \
             the outside either way, so an archive claiming no key is still wrapped"
        );
    }

    #[test]
    fn writing_a_newer_archive_out_at_a_size_that_moves_the_run_the_block_cipher_covers_is_refused()
    {
        let mut head = [0u8; 64];
        head[keying::RESERVE_AT..].copy_from_slice(&archive::RESERVE);

        let seal = sealed(SPOKEN_350, KEY_350, &head, false, 1088, 900).expect("a seal");

        assert!(
            seal.moved(4000, 3000, 800).is_err(),
            "at 1088 bytes the run comes out 1024 and at 4000 it comes out 956, so growing this \
             archive has to fail loudly rather than write out an archive the game reads as noise"
        );

        let seal = sealed(SPOKEN_350, KEY_350, &head, false, 4000, 3000).expect("a seal");
        assert!(seal.moved(5000, 3800, 2900).is_ok());

        assert!(
            seal.moved(5000, 3800, 4990).is_err(),
            "the outer pass stops sixty four bytes short of the end, so bytes carried over from \
             past that point would come out sealed the wrong way"
        );
    }

    #[test]
    fn the_four_addresses_a_newer_archive_hides_are_the_same_bytes_again_once_the_mask_is_off() {
        let mut head = vec![0u8; 64];
        head[keying::RESERVE_AT..].copy_from_slice(&archive::RESERVE);
        head[8..16].copy_from_slice(&64u64.to_le_bytes());

        let was = head.clone();
        unmasked(SPOKEN_331, &mut head).expect("the mask goes on");

        assert_ne!(head[8..40], was[8..40], "the four addresses are hidden");
        assert_eq!(head[..8], was[..8], "the mark and the head size stay plain");
        assert_eq!(
            head[40..],
            was[40..],
            "the code page, the flags and the password stay plain"
        );

        unmasked(SPOKEN_331, &mut head).expect("the mask comes off");
        assert_eq!(head, was);
    }

    #[test]
    fn every_layer_a_newer_archive_is_wrapped_in_lands_where_the_reference_lands_it() {
        let whole = 2000usize;
        let names_at = 1500usize;

        for (
            crypt,
            key,
            at_head,
            at_thousand_and_eighty,
            at_names,
            past_the_outer,
            at_the_end,
            all,
        ) in [
            (
                SPOKEN_331,
                KEY_331,
                [0xf6, 0x6d, 0xe0, 0x3d, 0xc0, 0xdc, 0x43, 0x05],
                [0x08, 0xb0, 0xef, 0xb7, 0xd0, 0x5b, 0x4b, 0x82],
                [0xc1, 0x63, 0x11, 0x84, 0x53, 0x40, 0x2c, 0xce],
                [0xbd, 0x15, 0xe1, 0x38, 0x13, 0x6e, 0x2a, 0xf3],
                [0x59, 0xaf, 0xe7, 0x3c, 0x9d, 0x9a, 0x46, 0xd7],
                0x37a2_1333u32,
            ),
            (
                SPOKEN_350,
                KEY_350,
                [0x4c, 0x41, 0x75, 0xeb, 0x26, 0x42, 0x0c, 0x6e],
                [0x9d, 0x6e, 0x70, 0xe6, 0x29, 0x5b, 0x71, 0xce],
                [0xa2, 0x72, 0x21, 0x4d, 0x90, 0xf1, 0x23, 0xfd],
                [0xec, 0xa7, 0x8a, 0xf1, 0x78, 0xfa, 0x1a, 0x94],
                [0x3c, 0x87, 0xbe, 0x58, 0x1f, 0x5f, 0xfe, 0x43],
                0x96b5_9612u32,
            ),
        ] {
            let mut room = vec![0u8; whole];
            room[keying::RESERVE_AT..keying::RESERVE_AT + keying::WORD]
                .copy_from_slice(&archive::RESERVE);

            let seal = sealed(crypt, key, &room[..64], true, whole as u64, names_at as u64)
                .expect("a seal with no read stream of its own");

            seal.turned(&mut room[64..], 0, 64, None)
                .expect("the layers go on");
            unmasked(crypt, &mut room[..64]).expect("the addresses are hidden");

            assert_eq!(
                &room[64..72],
                &at_head,
                "{crypt:#x}, at the head of the data"
            );
            assert_eq!(
                &room[1080..1088],
                &at_thousand_and_eighty,
                "{crypt:#x}, at the far end of the run the block cipher covers"
            );
            assert_eq!(
                &room[1500..1508],
                &at_names,
                "{crypt:#x}, where the second run the block cipher covers begins, whose counter \
                 carries straight on from where the first one left it"
            );
            assert_eq!(
                &room[1936..1944],
                &past_the_outer,
                "{crypt:#x}, past where the outer pass stops"
            );
            assert_eq!(
                &room[1992..2000],
                &at_the_end,
                "{crypt:#x}, at the very end"
            );
            assert_eq!(
                keying::hashed(&room),
                all,
                "and every one of the two thousand bytes. All of these come from running the \
                 UberWolf reference over the same two thousand noughts in the vectors harness. \
                 {crypt:#x}"
            );
        }
    }

    #[test]
    fn a_classic_archive_leaves_its_header_alone_because_the_packer_writes_it_out_again_in_plain() {
        let mut head = vec![7u8; 64];
        let was = head.clone();

        unmasked(SPOKEN_300, &mut head).expect("nothing to unmask");
        assert_eq!(head, was);

        unmasked(SHIPPED, &mut head).expect("nothing to unmask");
        assert_eq!(head, was);
    }
}
