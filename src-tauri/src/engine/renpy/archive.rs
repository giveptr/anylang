use crate::engine::pictures::read_up_to;
use crate::store;
use crate::store::Stamp;
use anyhow::{Context, Result, bail};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::str;
use std::sync::{Arc, Mutex};

const NEWER: &str = "RPA-3.0";
const OLDER: &str = "RPA-2.0";
const LONGEST_HEAD: usize = 64;
const DEEPEST: usize = 32;
const ROOMIEST_LISTING: u64 = 256 * 1024 * 1024;

enum Part {
    Lead(Vec<u8>),
    Spot { at: u64, size: u64 },
}

pub struct Held {
    parts: Vec<Part>,
}

impl Held {
    pub fn size(&self) -> u64 {
        self.parts
            .iter()
            .map(|part| match part {
                Part::Lead(raw) => raw.len() as u64,
                Part::Spot { size, .. } => *size,
            })
            .sum()
    }
}

type Index = BTreeMap<String, Held>;
type Kept = BTreeMap<PathBuf, (Stamp, Arc<Index>)>;

static KEPT: Mutex<Kept> = Mutex::new(BTreeMap::new());

pub fn forget() {
    if let Ok(mut kept) = KEPT.lock() {
        kept.clear();
    }
}

pub fn listed(at: &Path) -> Result<Arc<Index>> {
    let stamp = store::stamp_of(at);

    if let Ok(kept) = KEPT.lock()
        && let Some((then, held)) = kept.get(at)
        && *then == stamp
    {
        return Ok(Arc::clone(held));
    }

    let held = Arc::new(index_of(at)?);

    if let Ok(mut kept) = KEPT.lock() {
        kept.insert(at.to_path_buf(), (stamp, Arc::clone(&held)));
    }

    Ok(held)
}

fn index_of(at: &Path) -> Result<Index> {
    let mut file = File::open(at).with_context(|| format!("opening {}", at.display()))?;

    let head = read_up_to(&mut file, LONGEST_HEAD as u64)?;

    let line = head
        .iter()
        .position(|byte| *byte == b'\n')
        .map(|end| &head[..end])
        .with_context(|| format!("{} does not open like a Ren'Py archive", at.display()))?;
    let said = str::from_utf8(line)
        .with_context(|| format!("{} does not name itself in letters", at.display()))?;

    let mut told = said.split_whitespace();
    let kind = told.next().unwrap_or_default();
    if kind != NEWER && kind != OLDER {
        bail!(
            "{} is a {kind} archive, which this reader does not open",
            at.display()
        );
    }

    let spot = u64::from_str_radix(told.next().unwrap_or_default(), 16)
        .with_context(|| format!("{} does not say where its listing sits", at.display()))?;
    let key = match kind {
        NEWER => u64::from_str_radix(told.next().unwrap_or_default(), 16)
            .with_context(|| format!("{} does not say how it hides its numbers", at.display()))?,
        _ => 0,
    };

    file.seek(SeekFrom::Start(spot))?;
    let mut packed = Vec::new();
    file.read_to_end(&mut packed)?;

    let listing = unpacked(&packed)
        .with_context(|| format!("{} keeps a listing this reader cannot open", at.display()))?;

    entries(&opened(&listing)?, key)
}

fn unpacked(packed: &[u8]) -> Result<Vec<u8>> {
    use flate2::read::ZlibDecoder;

    let mut out = Vec::new();
    ZlibDecoder::new(packed)
        .take(ROOMIEST_LISTING + 1)
        .read_to_end(&mut out)?;
    if out.len() as u64 > ROOMIEST_LISTING {
        bail!(
            "this listing opens out past {ROOMIEST_LISTING} bytes, more than any Ren'Py archive would hold"
        );
    }

    Ok(out)
}

pub struct Spool {
    file: File,
}

impl Spool {
    pub fn over(at: &Path) -> Result<Self> {
        let file = File::open(at).with_context(|| format!("opening {}", at.display()))?;

        Ok(Self { file })
    }

    pub fn read(&mut self, held: &Held) -> Result<Vec<u8>> {
        self.body(held, held.size() as usize)
    }

    pub fn head(&mut self, held: &Held, most: usize) -> Result<Vec<u8>> {
        self.body(held, most.min(held.size() as usize))
    }

    fn body(&mut self, held: &Held, want: usize) -> Result<Vec<u8>> {
        let whole = self.file.metadata()?.len();
        let mut out = Vec::new();

        for part in &held.parts {
            let Some(room) = want.checked_sub(out.len()).filter(|room| *room > 0) else {
                break;
            };

            match part {
                Part::Lead(raw) => out.extend_from_slice(&raw[..room.min(raw.len())]),
                Part::Spot { at, size } => {
                    self.file.seek(SeekFrom::Start(*at))?;

                    let held_by_file = whole.saturating_sub(*at).min(*size) as usize;
                    let rest = room.min(held_by_file);
                    let space = read_up_to(&mut self.file, rest as u64)?;
                    out.extend_from_slice(&space);
                }
            }
        }

        Ok(out)
    }
}

pub fn read(at: &Path, held: &Held) -> Result<Vec<u8>> {
    Spool::over(at)?.read(held)
}

#[derive(Clone)]
enum Told {
    Mark,
    Nothing,
    Number(i64),
    Text(String),
    Blob(Vec<u8>),
    List(Vec<Told>),
    Pair(Vec<(Told, Told)>),
}

impl Told {
    fn said(&self) -> Option<String> {
        match self {
            Told::Text(said) => Some(said.clone()),
            Told::Blob(raw) => Some(String::from_utf8_lossy(raw).into_owned()),
            _ => None,
        }
    }

    fn number(&self) -> Option<i64> {
        match self {
            Told::Number(held) => Some(*held),
            _ => None,
        }
    }

    fn items(&self) -> &[Told] {
        match self {
            Told::List(held) => held,
            _ => &[],
        }
    }
}

fn entries(held: &Told, key: u64) -> Result<BTreeMap<String, Held>> {
    let Told::Pair(listed) = held else {
        bail!("the listing is not the table of names this reader expects");
    };

    let mut out = BTreeMap::new();

    for (name, spots) in listed {
        let Some(name) = name.said() else { continue };

        let mut parts = Vec::new();

        for spot in spots.items() {
            let held = spot.items();

            let (Some(at), Some(size)) = (
                held.first().and_then(Told::number),
                held.get(1).and_then(Told::number),
            ) else {
                continue;
            };

            match held.get(2) {
                Some(Told::Blob(raw)) if !raw.is_empty() => parts.push(Part::Lead(raw.clone())),
                Some(Told::Text(said)) if !said.is_empty() => {
                    parts.push(Part::Lead(said.as_bytes().to_vec()));
                }
                _ => {}
            }

            parts.push(Part::Spot {
                at: (at as u64) ^ key,
                size: (size as u64) ^ key,
            });
        }

        if parts.is_empty() {
            continue;
        }

        out.insert(name.replace('\\', "/"), Held { parts });
    }

    Ok(out)
}

struct Reading<'a> {
    raw: &'a [u8],
    at: usize,
}

impl Reading<'_> {
    fn byte(&mut self) -> Result<u8> {
        let held = *self
            .raw
            .get(self.at)
            .context("the listing ends before it says stop")?;
        self.at += 1;

        Ok(held)
    }

    fn take(&mut self, many: usize) -> Result<&[u8]> {
        let end = self.at.checked_add(many).context("a listing this long")?;
        let held = self
            .raw
            .get(self.at..end)
            .context("the listing ends early")?;
        self.at = end;

        Ok(held)
    }

    fn number(&mut self, wide: usize) -> Result<i64> {
        let raw = self.take(wide)?;
        let mut held = 0u64;

        for (which, byte) in raw.iter().enumerate() {
            held |= u64::from(*byte) << (8 * which);
        }

        Ok(held as i64)
    }
}

fn opened(raw: &[u8]) -> Result<Told> {
    let mut at = Reading { raw, at: 0 };
    let mut stack: Vec<Told> = Vec::new();
    let mut memo: BTreeMap<u32, Told> = BTreeMap::new();
    let mut deep = 0;

    loop {
        let held = at.byte()?;

        match held {
            b'.' => break,
            0x80 => {
                at.byte()?;
            }
            0x95 => {
                at.number(8)?;
            }
            b'(' => {
                deep += 1;
                if deep > DEEPEST {
                    bail!("this listing nests deeper than any Ren'Py archive would");
                }
                stack.push(Told::Mark);
            }
            b'}' => stack.push(Told::Pair(Vec::new())),
            b']' | b')' => stack.push(Told::List(Vec::new())),
            b'N' => stack.push(Told::Nothing),
            0x88 | 0x89 => stack.push(Told::Nothing),
            b'K' => {
                let held = at.number(1)?;
                stack.push(Told::Number(held));
            }
            b'M' => {
                let held = at.number(2)?;
                stack.push(Told::Number(held));
            }
            b'J' => {
                let held = at.number(4)? as i32;
                stack.push(Told::Number(i64::from(held)));
            }
            0x8a | 0x8b => {
                let wide = match held {
                    0x8a => at.byte()? as usize,
                    _ => at.number(4)? as usize,
                };
                let raw = at.take(wide)?;
                let mut held = 0u64;
                for (which, byte) in raw.iter().take(8).enumerate() {
                    held |= u64::from(*byte) << (8 * which);
                }
                stack.push(Told::Number(held as i64));
            }
            b'U' | b'T' | 0x43 | 0x42 | 0x8e => {
                let wide = match held {
                    b'U' | 0x43 => at.byte()? as usize,
                    0x8e => at.number(8)? as usize,
                    _ => at.number(4)? as usize,
                };
                let held = at.take(wide)?.to_vec();
                stack.push(Told::Blob(held));
            }
            0x8c | b'X' | 0x8d => {
                let wide = match held {
                    0x8c => at.byte()? as usize,
                    0x8d => at.number(8)? as usize,
                    _ => at.number(4)? as usize,
                };
                let held = at.take(wide)?.to_vec();
                stack.push(Told::Text(String::from_utf8_lossy(&held).into_owned()));
            }
            b'q' | b'r' | 0x94 => {
                let which = match held {
                    b'q' => u32::from(at.byte()?),
                    b'r' => at.number(4)? as u32,
                    _ => memo.len() as u32,
                };
                let held = stack
                    .last()
                    .context("the listing writes down something it never made")?;

                memo.insert(which, held.clone());
            }
            b'h' | b'j' => {
                let which = match held {
                    b'h' => u32::from(at.byte()?),
                    _ => at.number(4)? as u32,
                };
                let held = memo
                    .get(&which)
                    .context("the listing looks back at something it never wrote down")?;

                stack.push(held.clone());
            }
            0x85..=0x87 => {
                let many = match held {
                    0x85 => 1,
                    0x86 => 2,
                    _ => 3,
                };
                let held = pulled(&mut stack, many)?;
                stack.push(Told::List(held));
            }
            b't' => {
                let held = since_mark(&mut stack)?;
                deep = deep.saturating_sub(1);
                stack.push(Told::List(held));
            }
            b'a' => {
                let held = pulled(&mut stack, 1)?;
                appended(&mut stack, held)?;
            }
            b'e' => {
                let held = since_mark(&mut stack)?;
                deep = deep.saturating_sub(1);
                appended(&mut stack, held)?;
            }
            b's' => {
                let held = pulled(&mut stack, 2)?;
                paired(&mut stack, held)?;
            }
            b'u' => {
                let held = since_mark(&mut stack)?;
                deep = deep.saturating_sub(1);
                paired(&mut stack, held)?;
            }
            other => bail!("a Ren'Py listing does not hold {other:#x}"),
        }
    }

    stack.pop().context("the listing says nothing at all")
}

fn pulled(stack: &mut Vec<Told>, many: usize) -> Result<Vec<Told>> {
    if stack.len() < many {
        bail!("the listing asks for more than it wrote down");
    }

    Ok(stack.split_off(stack.len() - many))
}

fn since_mark(stack: &mut Vec<Told>) -> Result<Vec<Told>> {
    let at = stack
        .iter()
        .rposition(|held| matches!(held, Told::Mark))
        .context("the listing closes something it never opened")?;

    let held = stack.split_off(at + 1);
    stack.pop();

    Ok(held)
}

fn appended(stack: &mut [Told], held: Vec<Told>) -> Result<()> {
    match stack.last_mut() {
        Some(Told::List(into)) => {
            into.extend(held);
            Ok(())
        }
        _ => bail!("the listing appends to something that is not a list"),
    }
}

fn paired(stack: &mut [Told], held: Vec<Told>) -> Result<()> {
    let Some(Told::Pair(into)) = stack.last_mut() else {
        bail!("the listing writes into something that is not a table");
    };

    let mut walk = held.into_iter();
    while let (Some(name), Some(value)) = (walk.next(), walk.next()) {
        into.push((name, value));
    }

    Ok(())
}

#[cfg(test)]
fn told_head(at: usize, key: u64, older: bool) -> String {
    match older {
        true => format!("{OLDER} {at:016x}\n"),
        false => format!("{NEWER} {at:016x} {key:08x}\n"),
    }
}

#[cfg(test)]
fn pickled(listing: &mut Vec<u8>, held: i64) {
    listing.push(0x8a);
    let raw = held.to_le_bytes();
    let wide = raw
        .iter()
        .rposition(|byte| *byte != 0)
        .map_or(1, |at| at + 1);
    listing.push(wide as u8);
    listing.extend_from_slice(&raw[..wide]);
}

#[cfg(test)]
fn wrapped(body: &[u8], listing: &[u8], key: u64, older: bool) -> Vec<u8> {
    use std::io::Write;

    let mut squeezed = Vec::new();
    let mut held = flate2::write::ZlibEncoder::new(&mut squeezed, flate2::Compression::fast());
    held.write_all(listing).expect("it squeezes");
    held.finish().expect("it finishes");

    let mut out = told_head(0, key, older).into_bytes();
    let head = out.len();
    out.extend_from_slice(body);
    let at = out.len();
    out.extend_from_slice(&squeezed);
    out[..head].copy_from_slice(told_head(at, key, older).as_bytes());

    out
}

#[cfg(test)]
fn pieced(files: &[(&str, &[&[u8]])], key: u64) -> Vec<u8> {
    let mut body = b"padding before anything".to_vec();
    let mut listing: Vec<u8> = vec![0x80, 2, b'}', b'q', 0, b'('];
    let ahead_of_body = told_head(0, key, false).len();

    for (name, held) in files {
        listing.push(0x8c);
        listing.push(name.len() as u8);
        listing.extend_from_slice(name.as_bytes());
        listing.push(b']');

        for piece in *held {
            let at = (ahead_of_body + body.len()) as u64;
            body.extend_from_slice(piece);

            listing.push(b'(');
            pickled(&mut listing, (at ^ key) as i64);
            pickled(&mut listing, (piece.len() as u64 ^ key) as i64);
            listing.push(b't');
            listing.push(b'a');
        }
    }

    listing.push(b'u');
    listing.push(b'.');

    wrapped(&body, &listing, key, false)
}

#[cfg(test)]
pub fn sealed(files: &[(&str, &[u8], usize)], key: u64, older: bool) -> Vec<u8> {
    let mut body = b"padding before anything".to_vec();
    let mut listing: Vec<u8> = vec![0x80, 2, b'}', b'q', 0, b'('];
    let ahead_of_body = told_head(0, key, older).len();

    for (name, held, ahead) in files {
        let ahead = (*ahead).min(held.len());
        let at = (ahead_of_body + body.len()) as u64;
        body.extend_from_slice(&held[ahead..]);

        listing.push(0x8c);
        listing.push(name.len() as u8);
        listing.extend_from_slice(name.as_bytes());

        listing.push(b']');
        listing.push(b'(');
        pickled(&mut listing, (at ^ key) as i64);
        pickled(&mut listing, ((held.len() - ahead) as u64 ^ key) as i64);
        listing.push(b'U');
        listing.push(ahead as u8);
        listing.extend_from_slice(&held[..ahead]);
        listing.push(b't');
        listing.push(b'a');
    }

    listing.push(b'u');
    listing.push(b'.');

    wrapped(&body, &listing, key, older)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn written(raw: &[u8]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("a temp folder");
        let at = dir.path().join("art.rpa");
        fs::write(&at, raw).expect("an archive");

        (dir, at)
    }

    #[test]
    fn an_archive_gives_up_where_every_picture_sits_without_being_unpacked() {
        let raw = sealed(
            &[
                ("art/shot/bed/day.png", &[7u8; 32], 0),
                ("gui/frame.png", b"lead and twelve more", 4),
            ],
            0x4242_4242,
            false,
        );
        let (_dir, at) = written(&raw);

        let held = listed(&at).expect("a listing");
        assert_eq!(
            held.keys().collect::<Vec<_>>(),
            ["art/shot/bed/day.png", "gui/frame.png"],
            "the reader finds a picture by the path Ren'Py loads it at, so the listing has to \
             come back keyed by exactly that"
        );
        assert_eq!(held["art/shot/bed/day.png"].size(), 32);

        let body = read(&at, &held["gui/frame.png"]).expect("its bytes");
        assert_eq!(
            body.len(),
            20,
            "Ren'Py makes a lead a part of its own and still reads the whole run the entry names \
             after it, so an entry carrying a lead is that lead plus what sits in the archive, and \
             counting the lead inside the run, the way rpatool does, hands back a picture short by \
             however long the lead is"
        );
        assert!(
            body.starts_with(b"lead"),
            "the lead comes first, the way Ren'Py puts it back together"
        );
    }

    #[test]
    fn an_entry_a_packer_split_in_two_is_handed_back_in_one_piece() {
        let raw = pieced(
            &[(
                "art/day.png",
                &[
                    b"the first run of bytes".as_slice(),
                    b"and the second".as_slice(),
                ],
            )],
            0x4242_4242,
        );
        let (_dir, at) = written(&raw);

        let held = listed(&at).expect("a listing");
        assert_eq!(held["art/day.png"].size(), 36);
        assert_eq!(
            read(&at, &held["art/day.png"]).expect("its bytes"),
            b"the first run of bytesand the second",
            "Ren'Py joins every run an entry names, so a picture a packer laid down in two pieces \
             has to come back whole rather than as whichever piece is listed first"
        );
    }

    #[test]
    fn only_the_first_bytes_of_a_packed_entry_are_read_when_a_head_is_asked_for() {
        let raw = sealed(&[("art/day.png", &[7u8; 4096], 0)], 0x1234_5678, false);
        let (_dir, at) = written(&raw);

        let held = listed(&at).expect("a listing");
        let mut spool = Spool::over(&at).expect("the archive opens");
        let head = spool.head(&held["art/day.png"], 64).expect("its head");

        assert_eq!(
            head.len(),
            64,
            "listing a game of nine thousand pictures cannot mean reading every byte of every \
             one of them"
        );
    }

    #[test]
    fn an_older_archive_that_hides_nothing_reads_too() {
        let raw = sealed(&[("art/day.png", &[7u8; 8], 0)], 0, true);
        let (_dir, at) = written(&raw);

        let held = listed(&at).expect("a listing");
        assert_eq!(held["art/day.png"].size(), 8);
    }

    #[test]
    fn anything_that_is_not_an_archive_this_reader_knows_is_refused_out_loud() {
        let (_dir, at) = written(b"RPA-1.0 000000000000000a\nnothing");
        assert!(
            listed(&at).is_err(),
            "an archive from before the listing moved inside is one this reader cannot open, and \
             guessing at it would hand back offsets pointing anywhere"
        );

        let (_other, at) = written(b"not an archive at all");
        assert!(listed(&at).is_err());

        let (_third, at) = written(b"RPA-3.0 00000000000000ff 42424242\nshort");
        assert!(
            listed(&at).is_err(),
            "a listing that sits past the end of the file is refused rather than read as noise"
        );
    }

    #[test]
    fn a_listing_this_reader_cannot_make_sense_of_is_refused() {
        assert!(
            opened(&[0x80, 2, b'c', b'o', b's', b'.']).is_err(),
            "a listing may not name a thing to call: this reader walks names and numbers and \
             nothing that could run"
        );
        assert!(opened(&[]).is_err());
        assert!(opened(&[0x80, 2, b'.']).is_err());
    }
}
