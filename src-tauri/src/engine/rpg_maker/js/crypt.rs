use std::fmt::{self, Debug, Formatter};

pub const HEAD: usize = 16;

const MASKED: usize = 16;
const MARK: [u8; 8] = [0x52, 0x50, 0x47, 0x4D, 0x56, 0x00, 0x00, 0x00];

const A_PNG_BEGINS: [u8; MASKED] = [
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Key([u8; MASKED]);

impl Debug for Key {
    fn fmt(&self, out: &mut Formatter<'_>) -> fmt::Result {
        out.write_str("the key this game locks its pictures with")
    }
}

pub fn locked_head(raw: &[u8]) -> Option<&[u8]> {
    raw.get(..HEAD).filter(|head| head.starts_with(&MARK))
}

pub fn key_behind(raw: &[u8]) -> Option<Key> {
    locked_head(raw)?;

    let masked = raw.get(HEAD..HEAD + MASKED)?;
    let mut out = [0u8; MASKED];
    for (which, byte) in out.iter_mut().enumerate() {
        *byte = masked[which] ^ A_PNG_BEGINS[which];
    }

    Some(Key(out))
}

impl Key {
    pub fn read(said: &str) -> Option<Self> {
        let said = said.trim();
        if said.len() != MASKED * 2 {
            return None;
        }

        let mut out = [0u8; MASKED];
        for (which, byte) in out.iter_mut().enumerate() {
            let pair = said.get(which * 2..which * 2 + 2)?;
            *byte = u8::from_str_radix(pair, 16).ok()?;
        }

        Some(Self(out))
    }

    pub fn opened(&self, raw: &[u8]) -> Option<Vec<u8>> {
        locked_head(raw)?;

        let mut out = raw.get(HEAD..)?.to_vec();
        self.turn(&mut out);

        Some(out)
    }

    pub fn locked(&self, head: &[u8], body: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(head.len() + body.len());
        out.extend_from_slice(head);
        out.extend_from_slice(body);
        self.turn(&mut out[head.len()..]);

        out
    }

    fn turn(&self, body: &mut [u8]) {
        for (which, byte) in body.iter_mut().take(MASKED).enumerate() {
            *byte ^= self.0[which];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::rpg_maker::pictures::dotted;

    const SAID: &str = "d41d8cd98f00b204e9800998ecf8427e";

    fn a_key() -> Key {
        Key::read(SAID).expect("the key a game names in System.json")
    }

    fn a_picture() -> Vec<u8> {
        dotted(12, 9, 5).png().expect("a png")
    }

    #[test]
    fn a_picture_locked_the_way_the_engine_locks_it_comes_back_byte_for_byte() {
        let key = a_key();
        let png = a_picture();
        let shipped = key.locked(&[0u8; HEAD], &png);

        assert_ne!(
            shipped[HEAD..HEAD + MASKED],
            png[..MASKED],
            "the first sixteen bytes are the whole of the lock, so a write that left them alone \
             would ship a picture the game refuses to load"
        );
        assert_eq!(
            shipped[HEAD + MASKED..],
            png[MASKED..],
            "nothing past the first sixteen bytes is touched, so a locked picture is the plain one \
             with a head on it"
        );

        let mut head = [0u8; HEAD];
        head[..MARK.len()].copy_from_slice(&MARK);
        let shipped = key.locked(&head, &png);

        assert_eq!(
            key.opened(&shipped).expect("it opens again"),
            png,
            "a reader's picture goes into the game locked and has to come back out of it exactly \
             as it went in, or the game draws noise where the picture was"
        );
        assert_eq!(
            &shipped[..HEAD],
            &head,
            "the game's own decrypter checks all sixteen header bytes, so the head the file \
             shipped with is the head that goes back"
        );
    }

    #[test]
    fn bytes_carrying_no_header_of_the_engines_are_never_read_as_locked() {
        let png = a_picture();

        assert!(
            locked_head(&png).is_none(),
            "a game may name a plain png .rpgmvp, and stripping sixteen bytes off it would hand \
             the reader a picture that is not there"
        );
        assert!(a_key().opened(&png).is_none());
        assert!(
            locked_head(b"RPGMV\0\0\0short").is_none(),
            "a file too short to hold the header holds no picture behind it either"
        );
    }

    #[test]
    fn the_key_a_game_keeps_to_itself_is_read_back_out_of_a_locked_png() {
        let key = a_key();
        let mut head = [0u8; HEAD];
        head[..MARK.len()].copy_from_slice(&MARK);
        let shipped = key.locked(&head, &a_picture());

        assert_eq!(
            key_behind(&shipped),
            Some(key),
            "every png opens with the same sixteen bytes, so a game that deployed without writing \
             its key into System.json still hands the key over in each of its pictures"
        );
        assert!(
            key_behind(&a_picture()).is_none(),
            "a picture that was never locked has no key behind it to read"
        );
    }

    #[test]
    fn a_key_the_game_does_not_spell_out_in_full_is_refused_rather_than_guessed_at() {
        for said in [
            "",
            "d41d8cd9",
            "d41d8cd98f00b204e9800998ecf8427",
            "d41d8cd98f00b204e9800998ecf8427ee",
            "d41d8cd98f00b204e9800998ecf8427g",
        ] {
            assert!(
                Key::read(said).is_none(),
                "{said:?} is not a key this reader can be sure of, and guessing at one would \
                 write pictures into the game that it then cannot open"
            );
        }

        assert!(
            Key::read(&format!("  {SAID}  ")).is_some(),
            "the field is typed by hand into the editor often enough that space around it is not \
             a broken key"
        );
    }
}
