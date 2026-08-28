use std::ops::Range;

pub const UTF8_MARK: u8 = 0x55;

const OLDER_EDITOR: &str = "convert the game with Wolf RPG Editor 3";

fn read(raw: &[u8]) -> Result<String, String> {
    let raw = match raw.split_last() {
        Some((0, rest)) => rest,
        _ => raw,
    };

    String::from_utf8(raw.to_vec())
        .map_err(|_| "this line is not the UTF-8 the newer editor writes".to_string())
}

pub fn line(said: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(said.len() + 5);
    out.extend_from_slice(&((said.len() + 1) as u32).to_le_bytes());
    out.extend_from_slice(said.as_bytes());
    out.push(0);

    out
}

fn short(at: usize) -> String {
    format!("this file stops before the number at {at}")
}

fn spanned(body: &[u8], at: usize, len: usize) -> Result<&[u8], String> {
    at.checked_add(len)
        .and_then(|end| body.get(at..end))
        .ok_or_else(|| short(at))
}

fn room(body: &mut [u8], at: usize, len: usize) -> Result<&mut [u8], String> {
    at.checked_add(len)
        .and_then(|end| body.get_mut(at..end))
        .ok_or_else(|| format!("there is no room for the number at {at}"))
}

pub fn byte_at(body: &[u8], at: usize) -> Result<u8, String> {
    body.get(at)
        .copied()
        .ok_or_else(|| format!("this file stops before the byte at {at}"))
}

pub fn half_at(body: &[u8], at: usize) -> Result<u16, String> {
    let two = spanned(body, at, 2)?;

    Ok(u16::from_le_bytes(two.try_into().expect("two bytes")))
}

pub fn word_at(body: &[u8], at: usize) -> Result<u32, String> {
    let four = spanned(body, at, 4)?;

    Ok(u32::from_le_bytes(four.try_into().expect("four bytes")))
}

pub fn long_at(body: &[u8], at: usize) -> Result<u64, String> {
    let eight = spanned(body, at, 8)?;

    Ok(u64::from_le_bytes(eight.try_into().expect("eight bytes")))
}

pub fn put_word(body: &mut [u8], at: usize, value: u32) -> Result<(), String> {
    room(body, at, 4)?.copy_from_slice(&value.to_le_bytes());

    Ok(())
}

pub fn put_long(body: &mut [u8], at: usize, value: u64) -> Result<(), String> {
    room(body, at, 8)?.copy_from_slice(&value.to_le_bytes());

    Ok(())
}

pub struct Reader<'a> {
    body: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    pub fn over(body: &'a [u8], at: usize) -> Self {
        Self { body, at }
    }

    pub fn offset(&self) -> usize {
        self.at
    }

    pub fn done(&self) -> bool {
        self.at >= self.body.len()
    }

    pub fn seek(&mut self, at: usize) {
        self.at = at;
    }

    pub fn take(&mut self, count: usize) -> Result<&'a [u8], String> {
        let end = self
            .at
            .checked_add(count)
            .ok_or_else(|| format!("{count} bytes asked for at {} runs off the end", self.at))?;
        let got = self.body.get(self.at..end).ok_or_else(|| {
            format!(
                "this file holds {} bytes and {count} more were asked for at {}",
                self.body.len(),
                self.at
            )
        })?;

        self.at = end;

        Ok(got)
    }

    pub fn skip(&mut self, count: usize) -> Result<(), String> {
        self.take(count).map(|_| ())
    }

    pub fn byte(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    pub fn word(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().expect("four bytes"),
        ))
    }

    pub fn count(&mut self) -> Result<usize, String> {
        let asked = self.word()? as usize;
        let left = self.body.len().saturating_sub(self.at);

        match asked > left {
            true => Err(format!(
                "a list of {asked} at {} cannot fit in the {left} bytes that are left",
                self.at - 4
            )),
            false => Ok(asked),
        }
    }

    pub fn said(&mut self) -> Result<(String, Range<usize>), String> {
        let from = self.at;
        let count = self.word()? as usize;
        if count == 0 {
            return Err(format!("a line of no length at all sits at {from}"));
        }

        let raw = self.take(count)?;

        Ok((read(raw)?, from..self.at))
    }

    pub fn past_said(&mut self) -> Result<(), String> {
        let from = self.at;
        let count = self.word()? as usize;
        if count == 0 {
            return Err(format!("a line of no length at all sits at {from}"));
        }

        self.skip(count)
    }

    pub fn past_saids(&mut self, count: usize) -> Result<(), String> {
        for _ in 0..count {
            self.past_said()?;
        }

        Ok(())
    }

    pub fn past_said_lists(&mut self) -> Result<(), String> {
        let count = self.count()?;
        for _ in 0..count {
            let inner = self.count()?;
            self.past_saids(inner)?;
        }

        Ok(())
    }

    pub fn past_word_lists(&mut self) -> Result<(), String> {
        let count = self.count()?;
        for _ in 0..count {
            let inner = self.count()?;
            for _ in 0..inner {
                self.word()?;
            }
        }

        Ok(())
    }

    pub fn marker(&mut self, wanted: u8, about: &str) -> Result<(), String> {
        let at = self.at;
        let found = self.byte()?;

        match found == wanted {
            true => Ok(()),
            false => Err(format!(
                "{about} should be {wanted:#04x} at {at} and is {found:#04x}"
            )),
        }
    }

    pub fn expect(&mut self, wanted: &[u8], about: &str) -> Result<(), String> {
        let at = self.at;
        let found = self.take(wanted.len())?;

        match found == wanted {
            true => Ok(()),
            false => Err(format!("{about} is not at {at} where it belongs")),
        }
    }

    pub fn ended(&self) -> Result<(), String> {
        match self.done() {
            true => Ok(()),
            false => Err(format!(
                "this file holds {} bytes more than were read",
                self.body.len() - self.at
            )),
        }
    }
}

const HUGE: usize = 256 << 20;
const WIDEST_OPENING: usize = 255;

pub fn unpacked(body: &[u8], head: usize) -> Result<Vec<u8>, String> {
    let plain = word_at(body, head)? as usize;
    let squeezed = word_at(body, head + 4)? as usize;

    if plain > HUGE || plain > squeezed.saturating_mul(WIDEST_OPENING).saturating_add(16) {
        return Err(format!("this file claims to open out to {plain} bytes"));
    }

    let from = body.get(head + 8..head + 8 + squeezed).ok_or_else(|| {
        format!("this file holds less than the {squeezed} packed bytes it claims")
    })?;

    if head + 8 + squeezed != body.len() {
        return Err(
            "this file carries bytes after its packed half that writing it back would lose"
                .to_string(),
        );
    }

    let out = lz4::block::decompress(from, Some(plain as i32))
        .map_err(|why| format!("the packed half of this file would not open: {why}"))?;

    let mut whole = body
        .get(..head)
        .ok_or("this file is shorter than its own header")?
        .to_vec();
    whole.extend(out);

    Ok(whole)
}

pub fn repacked(plain: &[u8], head: usize) -> Result<Vec<u8>, String> {
    let payload = plain
        .get(head..)
        .ok_or("this file is shorter than its own header")?;

    let squeezed = lz4::block::compress(payload, None, false)
        .map_err(|why| format!("this file would not pack back up: {why}"))?;

    let mut out = plain[..head].to_vec();
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&(squeezed.len() as u32).to_le_bytes());
    out.extend(squeezed);

    Ok(out)
}

fn alike(found: &[u8], magic: &[u8], utf8_at: usize) -> bool {
    found
        .iter()
        .zip(magic)
        .enumerate()
        .all(|(which, (found, wanted))| which == utf8_at || found == wanted)
}

pub fn opens(magic: &[u8], utf8_at: usize, raw: &[u8], from: usize) -> bool {
    raw.get(from..from + magic.len())
        .is_some_and(|found| alike(found, magic, utf8_at))
}

pub fn spelled(magic: &[u8], utf8_at: usize, raw: &[u8], from: usize) -> Result<(), String> {
    let found = raw
        .get(from..from + magic.len())
        .ok_or("this file is too short to say what it is")?;

    if !alike(found, magic, utf8_at) {
        return Err("this is not a Wolf RPG file of the kind its name says".to_string());
    }

    if found[utf8_at] != UTF8_MARK {
        return Err(OLDER_EDITOR.to_string());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::iter;

    #[test]
    fn a_line_carries_its_own_length_and_a_closing_zero_the_engine_expects() {
        let raw = line("\u{5263}");

        assert_eq!(raw, [4, 0, 0, 0, 0xE5, 0x89, 0xA3, 0]);

        let mut reader = Reader::over(&raw, 0);
        let (said, at) = reader.said().expect("it reads back");

        assert_eq!(said, "\u{5263}");
        assert_eq!(
            at,
            0..raw.len(),
            "the range covers the length as well as the bytes"
        );
    }

    #[test]
    fn a_line_of_no_length_is_refused_rather_than_read_as_empty() {
        let raw = [0u8, 0, 0, 0];

        assert!(
            Reader::over(&raw, 0).said().is_err(),
            "the engine never writes one, so reading one means the parse has drifted and every \
             byte after it would be nonsense"
        );
    }

    #[test]
    fn a_count_larger_than_the_file_is_refused_before_anything_is_allocated() {
        let raw = [0xFFu8, 0xFF, 0xFF, 0x7F, 1, 2];

        assert!(Reader::over(&raw, 0).count().is_err());
    }

    #[test]
    fn a_packed_half_opens_out_and_packs_back_to_the_same_bytes() {
        let plain: Vec<u8> = iter::repeat_n(b"WOLF".iter().copied(), 64)
            .flatten()
            .collect();
        let mut whole = b"header".to_vec();
        whole.extend_from_slice(&plain);

        let packed = repacked(&whole, 6).expect("it packs");

        assert_eq!(&packed[..6], b"header", "the header is never squeezed");
        assert_eq!(
            unpacked(&packed, 6).expect("it opens back out"),
            whole,
            "a map the engine packs has to come back byte for byte"
        );

        let mut padded = packed.clone();
        padded.push(0);
        assert!(
            unpacked(&padded, 6).is_err(),
            "a byte after the packed half has nowhere to go when the file is written back, so \
             the file is refused whole rather than written back smaller"
        );
    }

    #[test]
    fn a_packed_half_claiming_more_than_packing_can_hold_is_refused_unallocated() {
        let mut raw = b"header".to_vec();
        raw.extend_from_slice(&0x4000_0000u32.to_le_bytes());
        raw.extend_from_slice(&4u32.to_le_bytes());
        raw.extend_from_slice(&[1, 2, 3, 4]);

        assert!(
            unpacked(&raw, 6).is_err(),
            "four packed bytes can never open out to a gigabyte, and believing the claim means \
             allocating one"
        );
    }

    #[test]
    fn the_magic_admits_the_newer_writing_and_turns_the_older_one_away_by_name() {
        let magic = [0x57, 0x00, 0x00, 0x4F, 0x4C, 0x00, 0x46, 0x4D, 0x00];

        let mut older = vec![0u8];
        older.extend_from_slice(&magic);
        assert_eq!(
            spelled(&magic, 5, &older, 1),
            Err("convert the game with Wolf RPG Editor 3".to_string()),
            "whoever hits this is told what converts the game, not just no"
        );

        let mut newer = older.clone();
        newer[6] = UTF8_MARK;
        assert_eq!(spelled(&magic, 5, &newer, 1), Ok(()));

        let mut wrong = older.clone();
        wrong[4] = b'X';
        assert!(spelled(&magic, 5, &wrong, 1).is_err());
        assert!(spelled(&magic, 5, &[0, 1], 1).is_err());
    }
}
