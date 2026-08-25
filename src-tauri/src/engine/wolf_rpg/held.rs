use crate::engine::wolf_rpg::coder;
use std::ops::Range;

pub type Edits = Vec<(Range<usize>, Vec<u8>)>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Said {
    pub text: String,
    pub at: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Command { code: u32, args: Vec<u32> },
    Value,
    Naming,
    Title,
    Font,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Piece {
    pub spot: String,
    pub kind: Kind,
    pub said: Vec<Said>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Plain,
    Loose,
    Packed { head: usize },
    Measured { size: usize, offsets: [usize; 2] },
}

pub fn opened(raw: &[u8], head: usize, packed: bool) -> Result<(Vec<u8>, Shape), String> {
    match packed {
        true => Ok((coder::unpacked(raw, head)?, Shape::Packed { head })),
        false => Ok((raw.to_vec(), Shape::Plain)),
    }
}

pub struct Held {
    pub plain: Vec<u8>,
    pub shape: Shape,
    pub pieces: Vec<Piece>,
}

impl Held {
    pub fn fonts(&self) -> &[Said] {
        self.pieces
            .iter()
            .find(|piece| piece.kind == Kind::Font)
            .map(|piece| piece.said.as_slice())
            .unwrap_or_default()
    }
}

pub fn wrapped(held: &Held, mut edits: Edits) -> Result<Vec<u8>, String> {
    edits.sort_by_key(|(at, _)| at.start);

    let mut out = Vec::with_capacity(held.plain.len());
    let mut done = 0;

    for (at, body) in edits {
        if at.start < done {
            return Err("two lines of this file lay claim to the same bytes".to_string());
        }

        if held.shape != Shape::Loose {
            let told = coder::word_at(&held.plain, at.start)? as usize;
            if at.end - at.start != told + 4 {
                return Err(
                    "a line to be replaced is not where the file says a line sits".to_string(),
                );
            }
        }

        out.extend_from_slice(
            held.plain
                .get(done..at.start)
                .ok_or("a line was found outside the file it came from")?,
        );
        out.extend(body);
        done = at.end;
    }

    out.extend_from_slice(
        held.plain
            .get(done..)
            .ok_or("a line was found outside the file it came from")?,
    );

    match held.shape {
        Shape::Plain | Shape::Loose => Ok(out),
        Shape::Packed { head } => coder::repacked(&out, head),
        Shape::Measured { size, offsets } => grown(out, &held.plain, size, offsets),
    }
}

fn grown(
    mut out: Vec<u8>,
    before: &[u8],
    size: usize,
    offsets: [usize; 2],
) -> Result<Vec<u8>, String> {
    let step = out.len() as i64 - before.len() as i64;
    let moved = |at: usize| -> Result<usize, String> {
        usize::try_from(at as i64 + step)
            .map_err(|_| format!("the number at {at} moved off the front of the file"))
    };

    let checked = |at: usize| -> Result<(usize, u32, usize), String> {
        let held = coder::word_at(before, at)?;
        let now = moved(at)?;

        match coder::word_at(&out, now)? == held {
            true => Ok((at, held, now)),
            false => Err(
                "this file keeps its own size after the text, and the text moved it".to_string(),
            ),
        }
    };

    let (_, _, size_at) = checked(size)?;
    let shifting = [checked(offsets[0])?, checked(offsets[1])?];

    let whole = u32::try_from(out.len() - 1)
        .map_err(|_| "this translation makes Game.dat too large for the engine".to_string())?;
    coder::put_word(&mut out, size_at, whole)?;

    for (at, held, now) in shifting {
        let shifted = u32::try_from(held as i64 + step)
            .map_err(|_| format!("the offset at {at} moved off the front of the file"))?;

        coder::put_word(&mut out, now, shifted)?;
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn held(plain: Vec<u8>, shape: Shape) -> Held {
        Held {
            plain,
            shape,
            pieces: Vec::new(),
        }
    }

    #[test]
    fn a_translation_is_laid_into_the_bytes_it_replaces_and_nothing_else_moves() {
        let mut plain = b"before".to_vec();
        let at = plain.len();
        plain.extend(coder::line("ab"));
        plain.extend_from_slice(b"after");
        let ends = plain.len() - 5;

        let out = wrapped(
            &held(plain, Shape::Plain),
            vec![(at..ends, coder::line("longer"))],
        )
        .expect("it splices");

        assert!(out.starts_with(b"before") && out.ends_with(b"after"));
        assert_eq!(
            coder::Reader::over(&out, at).said().expect("a line").0,
            "longer",
            "the length in front of the line has to grow with it"
        );
    }

    #[test]
    fn two_lines_claiming_the_same_bytes_are_refused_instead_of_writing_a_broken_file() {
        let mut plain = vec![0u8; 32];
        coder::put_word(&mut plain, 4, 4).unwrap();
        coder::put_word(&mut plain, 8, 4).unwrap();

        assert!(
            wrapped(
                &held(plain, Shape::Plain),
                vec![(4..12, vec![1, 2]), (8..16, vec![3, 4])]
            )
            .is_err(),
            "both ranges hold what looks like a line, so the overlap itself is what refuses"
        );
    }

    #[test]
    fn an_edit_landing_where_the_file_holds_no_line_is_refused() {
        let plain = vec![0u8; 32];

        assert!(
            wrapped(&held(plain, Shape::Plain), vec![(4..12, vec![1, 2])]).is_err(),
            "a range that does not sit on a length-prefixed line is a mis-parse, and writing \
             through it would corrupt live bytes"
        );
    }

    #[test]
    fn game_dat_keeps_its_own_size_and_offsets_true_after_the_title_changes() {
        let mut plain = vec![0u8; 4];
        let title = plain.len();
        plain.extend(coder::line("Title"));
        let size = plain.len();
        plain.extend_from_slice(&0u32.to_le_bytes());
        let first = plain.len();
        plain.extend_from_slice(&1000u32.to_le_bytes());
        let second = plain.len();
        plain.extend_from_slice(&2000u32.to_le_bytes());
        plain.extend_from_slice(&[7; 8]);

        let was = plain.len();
        coder::put_word(&mut plain, size, (was - 1) as u32).unwrap();

        let ends = size;
        let out = wrapped(
            &held(
                plain,
                Shape::Measured {
                    size,
                    offsets: [first, second],
                },
            ),
            vec![(title..ends, coder::line("A Much Longer Title"))],
        )
        .expect("it splices");

        let step = out.len() as i64 - was as i64;
        assert!(step > 0);
        assert_eq!(
            coder::word_at(&out, (size as i64 + step) as usize),
            Ok((out.len() - 1) as u32),
            "the engine reads this number to know how far the file goes"
        );
        assert_eq!(
            coder::word_at(&out, (first as i64 + step) as usize),
            Ok((1000 + step) as u32)
        );
        assert_eq!(
            coder::word_at(&out, (second as i64 + step) as usize),
            Ok((2000 + step) as u32)
        );
    }
}
