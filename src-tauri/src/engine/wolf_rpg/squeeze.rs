use crate::engine::wolf_rpg::coder;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

const LEAVES: usize = 256;
const NODES: usize = LEAVES + 255;
const ROOT: usize = NODES - 1;
const LEAST: u32 = 4;
const HEAD: usize = 9;
const WIDEST_RUN: usize = 8195;

struct Bits<'a> {
    body: &'a [u8],
    bytes: usize,
    bits: u8,
}

impl<'a> Bits<'a> {
    fn over(body: &'a [u8]) -> Self {
        Self {
            body,
            bytes: 0,
            bits: 0,
        }
    }

    fn read(&mut self, count: u8) -> Result<u64, String> {
        let mut out = 0u64;

        for i in 0..count {
            let holding = self
                .body
                .get(self.bytes)
                .ok_or("the packed head stops before its own table")?;

            out |= u64::from((holding >> (7 - self.bits)) & 1) << (count - 1 - i);

            self.bits += 1;
            if self.bits == 8 {
                self.bytes += 1;
                self.bits = 0;
            }
        }

        Ok(out)
    }

    fn sized(&mut self) -> Result<u64, String> {
        let width = self.read(6)? as u8 + 1;
        self.read(width)
    }

    fn eaten(&self) -> usize {
        self.bytes + usize::from(self.bits != 0)
    }
}

pub fn unhuffed(body: &[u8]) -> Result<Vec<u8>, String> {
    unhuffed_upto(body, usize::MAX)
}

pub fn unhuffed_upto(body: &[u8], most: usize) -> Result<Vec<u8>, String> {
    let mut bits = Bits::over(body);

    let wanted = bits.sized()? as usize;
    bits.sized()?;

    let mut weight = [0u16; LEAVES];
    for i in 0..LEAVES {
        let width = (bits.read(3)? as u8 + 1) * 2;
        let minus = bits.read(1)? == 1;
        let step = bits.read(width)? as u16;

        weight[i] = match (i, minus) {
            (0, _) => step,
            (_, true) => weight[i - 1].wrapping_sub(step),
            (_, false) => weight[i - 1].wrapping_add(step),
        };
    }

    let stream = body
        .get(bits.eaten()..)
        .ok_or("the packed head has no room for its own body")?;

    let stop = wanted.min(most);

    if stop > stream.len().saturating_mul(8) {
        return Err(format!(
            "the packed head names {stop} bytes and its shortest code is a bit, so the {} packed \
             bytes could never hold them",
            stream.len()
        ));
    }

    let mut load = [0u64; NODES];
    let mut kids = [[usize::MAX; 2]; NODES];

    for (leaf, held) in weight.iter().enumerate() {
        load[leaf] = u64::from(*held);
    }

    let mut waiting: BinaryHeap<Reverse<(u64, usize)>> = (0..LEAVES)
        .map(|leaf| Reverse((load[leaf], leaf)))
        .collect();
    let mut next = LEAVES;

    while waiting.len() > 1 {
        let (Some(Reverse((low, at))), Some(Reverse((second, beside)))) =
            (waiting.pop(), waiting.pop())
        else {
            break;
        };

        load[next] = low + second;
        kids[next] = [at, beside];
        waiting.push(Reverse((load[next], next)));

        next += 1;
    }

    let mut out = roomy(stop)?;
    let mut at = 0usize;
    let mut bit = 0u8;

    while out.len() < stop {
        let mut node = ROOT;

        while node > 255 {
            let Some(holding) = stream.get(at) else {
                if stop < wanted {
                    return Ok(out);
                }

                return Err("the packed body ends in the middle of a code".to_string());
            };

            bit += 1;
            let taken = (holding >> (bit - 1)) & 1;

            if bit == 8 {
                bit = 0;
                at += 1;
            }

            node = kids[node][usize::from(taken)];
            if node == usize::MAX {
                return Err("a packed code leads nowhere in its own table".to_string());
            }
        }

        out.push(node as u8);
    }

    Ok(out)
}

fn roomy(wanted: usize) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();

    out.try_reserve_exact(wanted)
        .map_err(|_| format!("a squeezed file claims to open out to {wanted} bytes"))?;

    Ok(out)
}

pub fn unlzed(body: &[u8]) -> Result<Vec<u8>, String> {
    let wanted = coder::word_at(body, 0)? as usize;
    let whole = coder::word_at(body, 4)? as usize;
    let mark = *body.get(8).ok_or("the squeezed head holds no mark")?;

    let stream = body
        .get(HEAD..whole)
        .ok_or("the squeezed head names more bytes than it carries")?;

    if wanted > stream.len().saturating_mul(WIDEST_RUN) {
        return Err(format!(
            "the squeezed head names {wanted} bytes and the {} packed bytes could never hold them",
            stream.len()
        ));
    }

    let mut at = 0usize;
    let mut out: Vec<u8> = roomy(wanted)?;

    let short = || "the squeezed body ends in the middle of a run".to_string();

    while at < stream.len() {
        let lead = stream[at];

        if lead != mark {
            out.push(lead);
            at += 1;
            continue;
        }

        let next = *stream.get(at + 1).ok_or_else(short)?;
        if next == mark {
            out.push(mark);
            at += 2;
            continue;
        }

        let mut code = u32::from(next);
        if code > u32::from(mark) {
            code -= 1;
        }
        at += 2;

        let mut run = code >> 3;
        if code & 0b100 != 0 {
            run |= u32::from(*stream.get(at).ok_or_else(short)?) << 5;
            at += 1;
        }
        run += LEAST;

        let back = match code & 0b11 {
            0 => {
                at += 1;
                u32::from(*stream.get(at - 1).ok_or_else(short)?)
            }
            1 => {
                let pair = stream.get(at..at + 2).ok_or_else(short)?;
                at += 2;
                u32::from(u16::from_le_bytes(pair.try_into().expect("two bytes")))
            }
            _ => {
                let three = stream.get(at..at + 3).ok_or_else(short)?;
                at += 3;
                u32::from(u16::from_le_bytes(
                    three[..2].try_into().expect("two bytes"),
                )) | (u32::from(three[2]) << 16)
            }
        } + 1;

        let from = out
            .len()
            .checked_sub(back as usize)
            .ok_or("a squeezed run reaches back before the start")?;

        for step in 0..run as usize {
            let one = *out
                .get(from + step)
                .ok_or("a squeezed run reaches past what it has written")?;

            out.push(one);
        }
    }

    match out.len() == wanted {
        true => Ok(out),
        false => Err(format!(
            "the squeezed head names {wanted} bytes and {} came out",
            out.len()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str;

    fn packed(hex: &str) -> Vec<u8> {
        hex.as_bytes()
            .chunks(2)
            .map(|two| u8::from_str_radix(str::from_utf8(two).expect("hex"), 16).expect("a byte"))
            .collect()
    }

    const PACKED_LINE: &str = concat!(
        "17315d000000000000000000000000000000000000000000000000ca0a36828000000000000000000a505b50",
        "5000000000000000000000000000000000a50502d4140000a505b5050294140b505a505b505000029416d414",
        "0000000000031414ba0ab505031414bf0fba0aaf0f02fc3e94140029416e82a9416d4169416a82ad416d4140",
        "b505000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "00000000000000000000000000000000cf796cbdafbd3f89bcb6bc895f35496fa4dafc9a553106ccd9951130",
        "04"
    );

    const PACKED_EVERY_BYTE: &str = concat!(
        "22004a00dfe00000000000000000000000000000000000000000000000000000000000000000000000000000",
        "0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
        "000000000000000000000000000000000000000000008040c020a060e0109050d030b070f0088848c828a868",
        "e8189858d838b878f8048444c424a464e4149454d434b474f40c8c4ccc2cac6cec1c9c5cdc3cbc7cfc028242",
        "c222a262e2129252d232b272f20a8a4aca2aaa6aea1a9a5ada3aba7afa068646c626a666e6169656d636b676",
        "f60e8e4ece2eae6eee1e9e5ede3ebe7efe018141c121a161e1119151d131b171f1098949c929a969e9199959",
        "d939b979f9058545c525a565e5159555d535b575f50d8d4dcd2dad6ded1d9d5ddd3dbd7dfd038343c323a363",
        "e3139353d333b373f30b8b4bcb2bab6beb1b9b5bdb3bbb7bfb078747c727a767e7179757d737b777f70f8f4f",
        "cf2faf6fef1f9f5fdf3fbf7fff"
    );

    #[test]
    fn a_table_the_editor_itself_wrote_opens_out_to_the_bytes_it_was_given() {
        assert_eq!(
            unhuffed(&packed(PACKED_LINE)).as_deref(),
            Ok(b"WOLF RPG keeps its archives behind a huffman table.".as_slice()),
            "these are the bytes the editor's own packer produced for that line, so reading them \
             back wrong means every archive this reader opens is nonsense"
        );
    }

    #[test]
    fn asking_for_the_first_bytes_only_decodes_those_and_stops() {
        let held = packed(PACKED_LINE);

        assert_eq!(
            unhuffed_upto(&held, 8).as_deref(),
            Ok(b"WOLF RPG".as_slice()),
            "listing a game of twenty thousand packed pictures may not open every one of them out \
             whole just to read the size out of its header"
        );

        let short = &held[..held.len() - 8];
        let held = unhuffed_upto(short, 8).expect("what the prefix could give");
        assert!(
            held.starts_with(b"WOLF"),
            "a prefix of the packed bytes is all the reader lifted on purpose, so running out of \
             stream is the end of a peek and not a broken archive"
        );

        assert!(
            unhuffed(short).is_err(),
            "asking for the whole file and getting a stream that ends mid code is still an \
             archive this reader will not guess at"
        );
    }

    #[test]
    fn a_table_where_every_byte_weighs_the_same_still_leads_each_one_to_its_own_leaf() {
        let every: Vec<u8> = (0..=255u8).collect();

        assert_eq!(
            unhuffed(&packed(PACKED_EVERY_BYTE)).as_deref(),
            Ok(every.as_slice()),
            "each of the 256 values appears once, so the packer had nothing to break a tie with \
             and the tree this reader builds has to break them the same way the editor did"
        );
    }

    fn squeezed(mark: u8, stream: &[u8], wanted: usize) -> Vec<u8> {
        let mut out = (wanted as u32).to_le_bytes().to_vec();
        out.extend_from_slice(&((HEAD + stream.len()) as u32).to_le_bytes());
        out.push(mark);
        out.extend_from_slice(stream);

        out
    }

    #[test]
    fn a_run_the_squeezer_wrote_as_a_reach_backwards_is_copied_out_again() {
        let raw = squeezed(0xFF, &[b'a', b'b', b'c', b'd', 0xFF, 0x00, 0x03], 8);

        assert_eq!(
            unlzed(&raw).as_deref(),
            Ok(b"abcdabcd".as_slice()),
            "the token names a run of four reaching four bytes back, and the four it copies are \
             the four literals in front of it"
        );
    }

    #[test]
    fn a_byte_that_looks_like_the_mark_is_written_twice_and_read_back_once() {
        let raw = squeezed(0x99, &[b'a', 0x99, 0x99, b'b'], 3);

        assert_eq!(
            unlzed(&raw).as_deref(),
            Ok(b"a\x99b".as_slice()),
            "the squeezer has to be able to carry its own mark as a plain byte, and doubling it is \
             how it says so"
        );
    }

    #[test]
    fn a_run_reaching_back_before_the_first_byte_is_refused_rather_than_read_from_nowhere() {
        let raw = squeezed(0xFF, &[b'a', 0xFF, 0x00, 0x09], 8);

        assert!(
            unlzed(&raw).is_err(),
            "there is one byte written so far and this token reaches ten back, which means the \
             file was read wrong somewhere upstream"
        );
    }

    #[test]
    fn a_squeezed_half_that_does_not_open_out_to_the_size_it_names_is_refused() {
        let raw = squeezed(0xFF, b"ab", 8);

        assert!(
            unlzed(&raw).is_err(),
            "the count is how the archive says the file came out whole, so falling short of it \
             means writing a file the game would read as cut off"
        );
    }

    #[test]
    fn a_head_naming_more_bytes_than_packing_could_ever_hold_is_refused_before_any_room_is_taken() {
        let huge = squeezed(0xFF, b"a", u32::MAX as usize);
        assert!(
            unlzed(&huge).is_err(),
            "one packed byte cannot open out to four gigabytes, and believing the claim means \
             asking the machine for four gigabytes"
        );

        assert!(
            unhuffed(&vec![0xFF; 4096]).is_err(),
            "this head says its own width is sixty four bits and then names the largest number \
             that fits, which no run of codes could ever fill"
        );
    }

    #[test]
    fn a_packed_half_of_any_bytes_at_all_is_either_opened_or_refused_and_never_panics() {
        let mut seed = 0x1234_5678u32;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;

            (seed >> 8) as u8
        };

        let widths = [0usize, 1, 8, 9, 64, 700, 4096];
        let mut answered = Vec::new();

        for len in widths {
            for _ in 0..40 {
                let body: Vec<u8> = (0..len).map(|_| next()).collect();

                answered.push(unhuffed(&body).is_ok());
                answered.push(unlzed(&body).is_ok());
            }
        }

        assert_eq!(
            answered.len(),
            widths.len() * 40 * 2,
            "every one of these came back with an answer, which is the whole claim: a head this \
             reader cannot make sense of names sizes no machine could hold, and asking for that \
             room aborts the process where no error could ever be caught"
        );
    }
}
