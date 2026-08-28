use crate::engine::wolf_rpg::{coder, keying, squeeze, unseal};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::mem;
use std::path::{Path, PathBuf};

pub const MARK: u16 = 0x5844;
pub const UNMARKED: &str = "this file does not open like a Wolf RPG archive";
const NEWEST: u16 = 8;
const NO_KEY: u32 = 0x1;
const NO_HEAD_PRESS: u32 = 0x2;
const FOLDER: u64 = 0x10;
const LOOSE: u64 = u64::MAX;
const FILE_LEN: usize = 72;
const ALL_HUFF: u8 = 0xff;
const DEEPEST: usize = 64;

const NAME_AT: usize = 0;
const MARKS_AT: usize = 8;
const DATA_AT: usize = 40;
const PACKED_TABLE: usize = 1024;
const DATA_LEN: usize = 48;
const PRESSED_AT: usize = 56;
const HUFFED_AT: usize = 64;

struct Head {
    head_size: usize,
    data_at: u64,
    names_at: u64,
    files_at: usize,
    dirs_at: usize,
    char_code: u32,
    flags: u32,
    huff_kb: u8,
}

impl Head {
    fn read(body: &[u8]) -> Result<Self, String> {
        if coder::half_at(body, 0)? != MARK {
            return Err(UNMARKED.to_string());
        }

        let version = coder::half_at(body, 2)?;
        if version != NEWEST {
            return Err(format!(
                "this reader opens the version {NEWEST} archive Wolf RPG Editor 3 writes and \
                 this one is version {version}, so {}",
                coder::OLDER_EDITOR
            ));
        }

        let flags = coder::word_at(body, 44)?;

        let mut told = body
            .get(..keying::HEAD_LEN as usize)
            .ok_or("this archive stops before its own header ends")?
            .to_vec();

        unseal::unmasked((flags >> 16) as u16, &mut told)?;

        Ok(Self {
            head_size: coder::word_at(&told, 4)? as usize,
            data_at: coder::long_at(&told, 8)?,
            names_at: coder::long_at(&told, 16)?,
            files_at: coder::long_at(&told, 24)? as usize,
            dirs_at: coder::long_at(&told, 32)? as usize,
            char_code: coder::word_at(&told, 40)?,
            flags,
            huff_kb: coder::byte_at(&told, 48)?,
        })
    }

    fn crypt(&self) -> u16 {
        (self.flags >> 16) as u16
    }

    fn keyless(&self) -> bool {
        self.flags & NO_KEY != 0
    }

    fn pressed(&self) -> bool {
        self.flags & NO_HEAD_PRESS == 0
    }

    fn span(&self) -> Option<usize> {
        match self.huff_kb {
            ALL_HUFF => None,
            kb => Some(usize::from(kb) * 1024),
        }
    }
}

struct Spool {
    file: File,
    len: u64,
}

impl Spool {
    fn over(at: &Path) -> Result<Self, String> {
        let file = File::open(at).map_err(|why| format!("opening this archive: {why}"))?;
        let len = file
            .metadata()
            .map_err(|why| format!("sizing this archive: {why}"))?
            .len();

        Ok(Self { file, len })
    }

    fn at(&mut self, from: u64, len: usize) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        self.onto(from, len, &mut out)?;

        Ok(out)
    }

    fn onto(&mut self, from: u64, len: usize, out: &mut Vec<u8>) -> Result<(), String> {
        if from.saturating_add(len as u64) > self.len {
            return Err("a run of bytes reaches past the end of this archive".to_string());
        }

        self.file
            .seek(SeekFrom::Start(from))
            .map_err(|why| format!("seeking in this archive: {why}"))?;

        let was = out.len();
        out.resize(was.saturating_add(len), 0);

        self.file
            .read_exact(&mut out[was..])
            .map_err(|why| format!("reading this archive: {why}"))
    }

    fn rest(&mut self, from: u64) -> Result<Vec<u8>, String> {
        let len = self
            .len
            .checked_sub(from)
            .ok_or("this archive names an index past its own end")?;

        self.at(from, len as usize)
    }
}

struct Turned<'s> {
    seal: &'s unseal::Seal,
    pad: Option<&'s [u8; keying::PAD]>,
}

impl Turned<'_> {
    fn over(&self, body: &mut [u8], dxa: u64, abs: u64) -> Result<(), String> {
        self.seal.turned(body, dxa, abs, self.pad)
    }
}

fn unhead(spool: &mut Spool, head: &Head, turned: &Turned) -> Result<Vec<u8>, String> {
    let mut raw = match head.pressed() {
        true => spool.rest(head.names_at)?,
        false => spool.at(head.names_at, head.head_size)?,
    };

    turned.over(&mut raw, 0, head.names_at)?;

    let index = match head.pressed() {
        false => raw,
        true => squeeze::unlzed(&squeeze::unhuffed(&raw)?)?,
    };

    match index.len() == head.head_size {
        true => Ok(index),
        false => Err(format!(
            "this archive names an index of {} bytes and {} came out",
            head.head_size,
            index.len()
        )),
    }
}

const UTF8_PAGE: u32 = 65001;

fn named(index: &[u8], at: usize, char_code: u32) -> Result<String, String> {
    let words = coder::half_at(index, at)? as usize;
    let from = at.saturating_add(4).saturating_add(words.saturating_mul(4));

    let rest = index
        .get(from..)
        .ok_or("a name in this archive reaches past the index")?;

    let end = rest
        .iter()
        .position(|one| *one == 0)
        .ok_or("a name in this archive never ends")?;

    let spelling = match char_code {
        UTF8_PAGE => encoding_rs::UTF_8,
        _ => encoding_rs::SHIFT_JIS,
    };

    let (out, _, _) = spelling.decode(&rest[..end]);

    Ok(out.into_owned())
}

fn lift(
    spool: &mut Spool,
    turned: &Turned,
    from: u64,
    len: usize,
    seek: u64,
) -> Result<Vec<u8>, String> {
    let mut out = spool.at(from, len)?;
    turned.over(&mut out, seek, from)?;

    Ok(out)
}

fn unwrapped(raw: &[u8], head: &Head, target: usize, whole: u64) -> Result<Vec<u8>, String> {
    let out = squeeze::unhuffed(raw)?;

    match head.span() {
        Some(span) if target > span * 2 => {
            if target as u64 > whole {
                return Err(
                    "this archive names a file larger than the archive holding it".to_string(),
                );
            }

            if out.len() < span * 2 {
                return Err(
                    "the two ends of this file came out shorter than they should".to_string(),
                );
            }

            let mut room = vec![0u8; target];
            room[..span].copy_from_slice(&out[..span]);
            room[target - span..].copy_from_slice(&out[span..span * 2]);

            Ok(room)
        }
        _ => Ok(out),
    }
}

struct Body {
    data_at: u64,
    huffed: u64,
    spot: u64,
    target: usize,
}

fn middled(
    spool: &mut Spool,
    head: &Head,
    turned: &Turned,
    room: &mut [u8],
    body: Body,
) -> Result<(), String> {
    let Some(span) = head.span() else {
        return Ok(());
    };

    if body.target <= span * 2 {
        return Ok(());
    }

    let rest = lift(
        spool,
        turned,
        body.data_at.saturating_add(body.huffed),
        body.target - span * 2,
        body.spot.saturating_add(body.huffed),
    )?;

    room[span..body.target - span].copy_from_slice(&rest);

    Ok(())
}

struct Entry {
    data_at: u64,
    size: usize,
    pressed: u64,
    huffed: u64,
}

impl Entry {
    fn read(head: &Head, index: &[u8], at: usize) -> Result<Self, String> {
        Ok(Self {
            data_at: head
                .data_at
                .saturating_add(coder::long_at(index, at.saturating_add(DATA_AT))?),
            size: coder::long_at(index, at.saturating_add(DATA_LEN))? as usize,
            pressed: coder::long_at(index, at.saturating_add(PRESSED_AT))?,
            huffed: coder::long_at(index, at.saturating_add(HUFFED_AT))?,
        })
    }
}

fn unfile(
    spool: &mut Spool,
    head: &Head,
    turned: &Turned,
    entry: &Entry,
) -> Result<Vec<u8>, String> {
    let data_at = entry.data_at;
    let size = entry.size;
    let spot = size as u64;

    if size == 0 {
        return Ok(Vec::new());
    }

    let whole = spool.len;

    match (entry.pressed, entry.huffed) {
        (LOOSE, LOOSE) => lift(spool, turned, data_at, size, spot),

        (LOOSE, huffed) => {
            let raw = lift(spool, turned, data_at, huffed as usize, spot)?;
            let mut room = unwrapped(&raw, head, size, whole)?;

            middled(
                spool,
                head,
                turned,
                &mut room,
                Body {
                    data_at,
                    huffed,
                    spot,
                    target: size,
                },
            )?;

            Ok(room)
        }

        (pressed, LOOSE) => squeeze::unlzed(&lift(spool, turned, data_at, pressed as usize, spot)?),

        (pressed, huffed) => {
            let raw = lift(spool, turned, data_at, huffed as usize, spot)?;
            let target = pressed as usize;
            let mut room = unwrapped(&raw, head, target, whole)?;

            middled(
                spool,
                head,
                turned,
                &mut room,
                Body {
                    data_at,
                    huffed,
                    spot,
                    target,
                },
            )?;

            squeeze::unlzed(&room)
        }
    }
}

struct Filed {
    at: PathBuf,
    head_at: usize,
    pad: [u8; keying::PAD],
}

fn stored(index: &[u8], at: usize) -> Result<&[u8], String> {
    let rest = index
        .get(at.saturating_add(4)..)
        .ok_or("a name in this archive reaches past the index")?;

    let end = rest
        .iter()
        .position(|one| *one == 0)
        .ok_or("a name in this archive never ends")?;

    Ok(&rest[..end])
}

struct Walk<'r> {
    head: &'r Head,
    seal: &'r unseal::Seal,
    index: &'r [u8],
}

fn walked(
    walk: &Walk,
    dir_at: usize,
    under: &Path,
    chain: &[u8],
    deep: usize,
    out: &mut Vec<Filed>,
) -> Result<(), String> {
    if deep > DEEPEST {
        return Err("this archive nests its folders deeper than any game would".to_string());
    }

    let index = walk.index;
    let listed = walk.head.dirs_at.saturating_add(dir_at);
    let kids = coder::long_at(index, listed.saturating_add(16))? as usize;
    let heads_at = coder::long_at(index, listed.saturating_add(24))? as usize;

    for i in 0..kids {
        let at = walk
            .head
            .files_at
            .saturating_add(heads_at)
            .saturating_add(i.saturating_mul(FILE_LEN));
        let name_at = coder::long_at(index, at.saturating_add(NAME_AT))? as usize;
        let marks = coder::long_at(index, at.saturating_add(MARKS_AT))?;

        let name = named(index, name_at, walk.head.char_code)?;
        if name == "." || name == ".." || name.is_empty() {
            continue;
        }

        if Path::new(&name).components().count() != 1 {
            return Err(format!(
                "this archive holds a name that walks out of it: {name}"
            ));
        }

        let landing = under.join(&name);

        let keyed: &[u8] = match walk.seal.per_file() {
            true => stored(index, name_at)?,
            false => &[],
        };

        if marks & FOLDER != 0 {
            let mut deeper = Vec::with_capacity(keyed.len().saturating_add(chain.len()));
            deeper.extend_from_slice(keyed);
            deeper.extend_from_slice(chain);

            walked(
                walk,
                coder::long_at(index, at.saturating_add(DATA_AT))? as usize,
                &landing,
                &deeper,
                deep + 1,
                out,
            )?;
            continue;
        }

        out.push(Filed {
            at: landing,
            head_at: at,
            pad: walk.seal.filing(keyed, chain),
        });
    }

    Ok(())
}

pub struct Laid {
    pub at: PathBuf,
    pub body: Vec<u8>,
}

pub fn marked(at: &Path) -> Result<bool, String> {
    let mut spool = Spool::over(at)?;

    Ok(spool
        .at(0, 2)
        .and_then(|head| coder::half_at(&head, 0))
        .is_ok_and(|found| found == MARK))
}

pub fn key_for(at: &Path, weight: Option<u32>) -> Result<Option<Vec<u8>>, String> {
    let mut spool = Spool::over(at)?;

    let Ok(head) = spool.at(0, keying::HEAD_LEN as usize) else {
        return Ok(None);
    };
    if coder::half_at(&head, 0)? != MARK {
        return Ok(None);
    }

    let head = Head::read(&head)?;
    if head.keyless() {
        return Ok(Some(Vec::new()));
    }

    let crypt = head.crypt();

    unseal::keyed(crypt, weight)
        .map(Some)
        .ok_or_else(|| format!("sealed the {crypt:#x} way, which this reader cannot open"))
}

struct Opened {
    spool: Spool,
    head: Head,
    seal: unseal::Seal,
    index: Vec<u8>,
    filed: Vec<Filed>,
}

fn opened(at: &Path, key: &[u8], under: &Path) -> Result<Opened, String> {
    let mut spool = Spool::over(at)?;
    let raw = spool.at(0, keying::HEAD_LEN as usize)?;
    let head = Head::read(&raw)?;

    let seal = unseal::sealed(
        head.crypt(),
        key,
        &raw,
        head.keyless(),
        spool.len,
        head.names_at,
    )?;

    let index = unhead(
        &mut spool,
        &head,
        &Turned {
            seal: &seal,
            pad: None,
        },
    )?;
    let mut filed = Vec::new();

    walked(
        &Walk {
            head: &head,
            seal: &seal,
            index: &index,
        },
        0,
        under,
        &[],
        0,
        &mut filed,
    )?;

    Ok(Opened {
        spool,
        head,
        seal,
        index,
        filed,
    })
}

pub fn named_inside(at: &Path, key: &[u8]) -> Result<Vec<PathBuf>, String> {
    Ok(opened(at, key, Path::new(""))?
        .filed
        .into_iter()
        .map(|one| one.at)
        .collect())
}

pub fn poured(
    at: &Path,
    key: &[u8],
    under: &Path,
    wanted: impl Fn(&Path) -> bool,
    mut lay: impl FnMut(Laid) -> Result<(), String>,
) -> Result<usize, String> {
    let mut open = opened(at, key, under)?;
    let mut count = 0;

    for one in mem::take(&mut open.filed) {
        if !wanted(&one.at) {
            continue;
        }

        let entry = Entry::read(&open.head, &open.index, one.head_at)?;
        let body = unfile(
            &mut open.spool,
            &open.head,
            &Turned {
                seal: &open.seal,
                pad: Some(&one.pad),
            },
            &entry,
        )?;

        lay(Laid { at: one.at, body })?;
        count += 1;
    }

    Ok(count)
}

pub struct Peek {
    pub at: PathBuf,
    pub size: usize,
    pub head: Result<Vec<u8>, String>,
}

fn topped(
    spool: &mut Spool,
    head: &Head,
    turned: &Turned,
    entry: &Entry,
    most: usize,
) -> Result<Vec<u8>, String> {
    let data_at = entry.data_at;
    let size = entry.size;

    if size == 0 {
        return Ok(Vec::new());
    }

    match (entry.pressed, entry.huffed) {
        (LOOSE, LOOSE) => lift(spool, turned, data_at, size.min(most), size as u64),
        (LOOSE, huffed) => {
            let enough = match head.span() {
                Some(span) if size > span.saturating_mul(2) => span.min(most),
                _ => most,
            };
            let huffed = huffed as usize;
            let ahead = enough.saturating_add(PACKED_TABLE).min(huffed);

            let raw = lift(spool, turned, data_at, ahead, size as u64)?;
            let held = squeeze::unhuffed_upto(&raw, enough)?;
            if held.len() >= enough || ahead == huffed {
                return Ok(held);
            }

            let raw = lift(spool, turned, data_at, huffed, size as u64)?;

            squeeze::unhuffed_upto(&raw, enough)
        }
        _ => unfile(spool, head, turned, entry),
    }
}

pub fn peeked(
    at: &Path,
    key: &[u8],
    under: &Path,
    most: usize,
    wanted: impl Fn(&Path) -> bool,
    mut lay: impl FnMut(Peek) -> Result<(), String>,
) -> Result<usize, String> {
    let mut open = opened(at, key, under)?;
    let mut count = 0;

    for one in mem::take(&mut open.filed) {
        if !wanted(&one.at) {
            continue;
        }

        let entry = Entry::read(&open.head, &open.index, one.head_at)?;
        let head = topped(
            &mut open.spool,
            &open.head,
            &Turned {
                seal: &open.seal,
                pad: Some(&one.pad),
            },
            &entry,
            most,
        );

        lay(Peek {
            at: one.at,
            size: entry.size,
            head,
        })?;
        count += 1;
    }

    Ok(count)
}

pub struct Sealed {
    pub body: Vec<u8>,
    pub missed: Vec<PathBuf>,
}

pub fn resealed(
    at: &Path,
    key: &[u8],
    under: &Path,
    fresh: &BTreeMap<PathBuf, Vec<u8>>,
) -> Result<Sealed, String> {
    let mut open = opened(at, key, under)?;

    let mut index = mem::take(&mut open.index);

    let held = open
        .head
        .names_at
        .checked_sub(open.head.data_at)
        .ok_or("this archive keeps its names in front of its data")?;

    if open.seal.fresh() && open.head.data_at != keying::HEAD_LEN {
        return Err(format!(
            "this archive is sealed the newer way and starts its data at {}, and every layer of \
             that seal is measured from the head, so writing it back would move each of them",
            open.head.data_at
        ));
    }

    let mut out = Vec::with_capacity((keying::HEAD_LEN as usize).saturating_add(held as usize));
    out.extend_from_slice(&open.spool.at(0, keying::HEAD_LEN as usize)?);
    open.spool
        .onto(open.head.data_at, held as usize, &mut out)?;

    let carried = out.len() as u64;

    let mut found = BTreeSet::new();
    let mut appended = Vec::new();

    for one in mem::take(&mut open.filed) {
        let Some(body) = fresh.get(&one.at) else {
            continue;
        };
        found.insert(one.at.clone());

        let spot = (out.len() - keying::HEAD_LEN as usize) as u64;
        let abs = out.len() as u64;
        let size = body.len() as u64;

        out.extend_from_slice(body);
        appended.push((abs, size, one.pad));

        coder::put_long(&mut index, one.head_at.saturating_add(DATA_AT), spot)?;
        coder::put_long(&mut index, one.head_at.saturating_add(DATA_LEN), size)?;
        coder::put_long(&mut index, one.head_at.saturating_add(PRESSED_AT), LOOSE)?;
        coder::put_long(&mut index, one.head_at.saturating_add(HUFFED_AT), LOOSE)?;
    }

    let names_at = out.len() as u64;
    out.extend_from_slice(&index);
    let whole = out.len() as u64;

    let seal = open.seal.moved(whole, names_at, carried)?;

    for (abs, size, pad) in &appended {
        let span = *abs as usize..abs.saturating_add(*size) as usize;

        seal.turned(
            out.get_mut(span)
                .ok_or("a written body reaches past the archive holding it")?,
            *size,
            *abs,
            Some(pad),
        )?;
    }

    seal.turned(
        out.get_mut(names_at as usize..)
            .ok_or("a written index reaches past the archive holding it")?,
        0,
        names_at,
        None,
    )?;

    let mut head = Vec::with_capacity(keying::HEAD_LEN as usize);
    head.extend_from_slice(&MARK.to_le_bytes());
    head.extend_from_slice(&NEWEST.to_le_bytes());
    head.extend_from_slice(&(index.len() as u32).to_le_bytes());
    head.extend_from_slice(&keying::HEAD_LEN.to_le_bytes());
    head.extend_from_slice(&names_at.to_le_bytes());
    head.extend_from_slice(&(open.head.files_at as u64).to_le_bytes());
    head.extend_from_slice(&(open.head.dirs_at as u64).to_le_bytes());
    head.extend_from_slice(&open.head.char_code.to_le_bytes());
    head.extend_from_slice(&(open.head.flags | NO_HEAD_PRESS).to_le_bytes());
    head.push(open.head.huff_kb);

    out[..head.len()].copy_from_slice(&head);

    unseal::unmasked(
        open.head.crypt(),
        out.get_mut(..keying::HEAD_LEN as usize)
            .ok_or("a written archive stops before its own header ends")?,
    )?;

    Ok(Sealed {
        body: out,
        missed: fresh
            .keys()
            .filter(|one| !found.contains(*one))
            .cloned()
            .collect(),
    })
}

#[cfg(test)]
fn entry(name: &str) -> Vec<u8> {
    let upper = name.to_ascii_uppercase().into_bytes();
    let words = (upper.len() + 1).div_ceil(4);
    let hash: u16 = upper
        .iter()
        .fold(0u16, |held, one| held.wrapping_add(u16::from(*one)));

    let mut out = Vec::new();
    out.extend_from_slice(&(words as u16).to_le_bytes());
    out.extend_from_slice(&hash.to_le_bytes());

    out.extend_from_slice(&upper);
    out.resize(4 + words * 4, 0);

    out.extend_from_slice(name.as_bytes());
    out.push(0);
    while out.len() % 4 != 0 {
        out.push(0);
    }

    out
}

#[cfg(test)]
pub const RESERVE: [u8; keying::WORD] = [
    0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF,
];

#[cfg(test)]
struct Written {
    body: Vec<u8>,
    bit: u8,
}

#[cfg(test)]
impl Written {
    fn put(&mut self, value: u64, count: u8) {
        for i in (0..count).rev() {
            if self.bit == 0 {
                self.body.push(0);
            }

            let at = self.body.len() - 1;
            self.body[at] |= (((value >> i) & 1) as u8) << (7 - self.bit);
            self.bit = (self.bit + 1) % 8;
        }
    }
}

#[cfg(test)]
fn huffed(payload: &[u8]) -> Vec<u8> {
    let mut wrote = Written {
        body: Vec::new(),
        bit: 0,
    };

    wrote.put(31, 6);
    wrote.put(payload.len() as u64, 32);
    wrote.put(3, 6);
    wrote.put(0, 4);

    for _ in 0..256 {
        wrote.put(0, 6);
    }

    let mut out = wrote.body;
    out.extend(payload.iter().map(|one| one.reverse_bits()));

    out
}

#[cfg(test)]
struct Folder {
    name: String,
    files: Vec<usize>,
    kids: Vec<usize>,
    parent: Option<usize>,
}

#[cfg(test)]
struct Placed {
    spot: u64,
    size: u64,
    huff: u64,
    runs: Vec<(usize, usize, u64)>,
}

#[cfg(test)]
fn foldered(named: &[(&str, &[u8])]) -> Vec<Folder> {
    let mut out = vec![Folder {
        name: String::new(),
        files: Vec::new(),
        kids: Vec::new(),
        parent: None,
    }];

    for (which, (path, _)) in named.iter().enumerate() {
        let steps: Vec<&str> = path.split('/').collect();
        let mut here = 0usize;

        for step in &steps[..steps.len().saturating_sub(1)] {
            let found = out[here]
                .kids
                .iter()
                .copied()
                .find(|kid| out[*kid].name == *step);

            here = match found {
                Some(kid) => kid,
                None => {
                    out.push(Folder {
                        name: (*step).to_string(),
                        files: Vec::new(),
                        kids: Vec::new(),
                        parent: Some(here),
                    });

                    let made = out.len() - 1;
                    out[here].kids.push(made);

                    made
                }
            };
        }

        out[here].files.push(which);
    }

    out
}

#[cfg(test)]
fn chained(folders: &[Folder], mut here: usize) -> Vec<u8> {
    let mut out = Vec::new();

    while let Some(up) = folders[here].parent {
        out.extend_from_slice(folders[here].name.to_ascii_uppercase().as_bytes());
        here = up;
    }

    out
}

#[cfg(test)]
fn placed(data: &mut Vec<u8>, body: &[u8], huff_kb: u8) -> Placed {
    let spot = data.len() as u64;
    let size = body.len() as u64;
    let span = usize::from(huff_kb) * 1024;

    if huff_kb == ALL_HUFF || body.len() <= span * 2 {
        data.extend_from_slice(body);

        return Placed {
            spot,
            size,
            huff: LOOSE,
            runs: vec![(spot as usize, body.len(), size)],
        };
    }

    let mut payload = Vec::new();
    payload.extend_from_slice(&body[..span]);
    payload.extend_from_slice(&body[body.len() - span..]);

    let blob = huffed(&payload);
    let middle = &body[span..body.len() - span];

    data.extend_from_slice(&blob);
    data.extend_from_slice(middle);

    Placed {
        spot,
        size,
        huff: blob.len() as u64,
        runs: vec![
            (spot as usize, blob.len(), size),
            (
                spot as usize + blob.len(),
                middle.len(),
                size + blob.len() as u64,
            ),
        ],
    }
}

#[cfg(test)]
fn built(named: &[(&str, &[u8])], crypt: u16, key: &[u8], huff_kb: u8) -> Vec<u8> {
    let folders = foldered(named);

    let mut names = Vec::new();
    let mut folder_names = Vec::new();
    for one in &folders {
        folder_names.push(names.len());
        names.extend_from_slice(&entry(&one.name));
    }

    let mut file_names = Vec::new();
    for (path, _) in named {
        file_names.push(names.len());
        names.extend_from_slice(&entry(path.rsplit('/').next().unwrap_or(path)));
    }

    let mut runs = vec![0usize; folders.len()];
    let mut files_len = FILE_LEN;
    for (i, one) in folders.iter().enumerate() {
        runs[i] = files_len;
        files_len += (one.files.len() + one.kids.len()) * FILE_LEN;
    }

    let mut folder_heads = vec![0usize; folders.len()];
    for (up, one) in folders.iter().enumerate() {
        for (n, kid) in one.kids.iter().enumerate() {
            folder_heads[*kid] = runs[up] + (one.files.len() + n) * FILE_LEN;
        }
    }

    let mut data = Vec::new();
    let mut laid: Vec<Placed> = Vec::new();
    for (_, body) in named {
        let one = placed(&mut data, body, huff_kb);
        laid.push(one);
    }

    let mut files = vec![0u8; files_len];

    let mut stamp = |at: usize, name_at: usize, marks: u64, data_at: u64, one: &Placed| {
        let mut put = |spot: usize, held: u64| {
            files[at + spot..at + spot + 8].copy_from_slice(&held.to_le_bytes());
        };

        put(NAME_AT, name_at as u64);
        put(MARKS_AT, marks);
        put(DATA_AT, data_at);
        put(DATA_LEN, one.size);
        put(PRESSED_AT, LOOSE);
        put(HUFFED_AT, one.huff);
    };

    let empty = Placed {
        spot: 0,
        size: 0,
        huff: LOOSE,
        runs: Vec::new(),
    };

    stamp(0, folder_names[0], FOLDER, 0, &empty);

    for (i, one) in folders.iter().enumerate() {
        let mut at = runs[i];

        for which in &one.files {
            stamp(at, file_names[*which], 0, laid[*which].spot, &laid[*which]);
            at += FILE_LEN;
        }

        for kid in &one.kids {
            stamp(at, folder_names[*kid], FOLDER, (*kid * 32) as u64, &empty);
            at += FILE_LEN;
        }
    }

    let mut dirs = Vec::new();
    for (i, one) in folders.iter().enumerate() {
        dirs.extend_from_slice(&(folder_heads[i] as u64).to_le_bytes());
        dirs.extend_from_slice(
            &match one.parent {
                Some(up) => (up * 32) as u64,
                None => LOOSE,
            }
            .to_le_bytes(),
        );
        dirs.extend_from_slice(&((one.files.len() + one.kids.len()) as u64).to_le_bytes());
        dirs.extend_from_slice(&(runs[i] as u64).to_le_bytes());
    }

    let files_at = names.len();
    let dirs_at = files_at + files.len();

    let mut index = names;
    index.extend_from_slice(&files);
    index.extend_from_slice(&dirs);

    let flags = match crypt {
        0 => NO_KEY | NO_HEAD_PRESS,
        crypt => NO_HEAD_PRESS | (u32::from(crypt) << 16),
    };

    let mut out = Vec::new();
    out.extend_from_slice(&MARK.to_le_bytes());
    out.extend_from_slice(&NEWEST.to_le_bytes());
    out.extend_from_slice(&(index.len() as u32).to_le_bytes());
    out.extend_from_slice(&keying::HEAD_LEN.to_le_bytes());
    out.extend_from_slice(&((keying::HEAD_LEN as usize + data.len()) as u64).to_le_bytes());
    out.extend_from_slice(&(files_at as u64).to_le_bytes());
    out.extend_from_slice(&(dirs_at as u64).to_le_bytes());
    out.extend_from_slice(&932u32.to_le_bytes());
    out.extend_from_slice(&flags.to_le_bytes());
    out.push(huff_kb);
    out.resize(keying::RESERVE_AT, 0);
    out.extend_from_slice(&RESERVE);

    let names_at = out.len() + data.len();
    let whole = names_at + index.len();

    out.extend_from_slice(&data);
    out.extend_from_slice(&index);

    let seal = unseal::sealed(
        crypt,
        key,
        &out[..keying::HEAD_LEN as usize],
        flags & NO_KEY != 0,
        whole as u64,
        names_at as u64,
    )
    .expect("a seal the fixture can lay on");

    for (i, one) in folders.iter().enumerate() {
        let chain = chained(&folders, i);

        for which in &one.files {
            let leaf = named[*which]
                .0
                .rsplit('/')
                .next()
                .unwrap_or(named[*which].0)
                .to_ascii_uppercase();
            let pad = seal.filing(leaf.as_bytes(), &chain);

            for (from, len, dxa) in &laid[*which].runs {
                let abs = keying::HEAD_LEN as usize + from;

                seal.turned(&mut out[abs..abs + len], *dxa, abs as u64, Some(&pad))
                    .expect("the seal goes on");
            }
        }
    }

    seal.turned(&mut out[names_at..], 0, names_at as u64, None)
        .expect("the seal goes on the index");

    unseal::unmasked(crypt, &mut out[..keying::HEAD_LEN as usize])
        .expect("the addresses are hidden");

    out
}

#[cfg(test)]
pub fn archived(named: &[(&str, &[u8])], key: Option<&[u8]>) -> Vec<u8> {
    match key {
        Some(key) => built(named, unseal::SHIPPED, key, ALL_HUFF),
        None => built(named, 0, &[], ALL_HUFF),
    }
}

#[cfg(test)]
fn sealed_as(crypt: u16, named: &[(&str, &[u8])], huff_kb: u8) -> Vec<u8> {
    let key = unseal::keyed(crypt, None).expect("the key string this seal ships with");

    built(named, crypt, &key, huff_kb)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::wolf_rpg::fixture::sandbox;
    use std::fs;
    use std::path::MAIN_SEPARATOR_STR;

    fn plain(named: &[(&str, &[u8])]) -> Vec<u8> {
        archived(named, None)
    }

    fn all(_: &Path) -> bool {
        true
    }

    fn on_disk(raw: &[u8]) -> (tempfile::TempDir, PathBuf) {
        let held = sandbox();
        let at = held.path().join("BasicData.wolf");
        fs::write(&at, raw).expect("an archive on disk");

        (held, at)
    }

    fn laid_out(raw: &[u8], under: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        let (_held, at) = on_disk(raw);

        let mut out = BTreeMap::new();
        poured(&at, &[], under, all, |held| {
            out.insert(held.at, held.body);
            Ok(())
        })
        .expect("the archive opens");

        out
    }

    #[test]
    fn every_file_an_archive_holds_is_lifted_back_out_under_its_own_name_and_body() {
        let under = Path::new("/games/demo/Data/BasicData");
        let raw = plain(&[
            ("Game.dat", b"first body".as_slice()),
            ("DataBase.dat", b"a rather longer second body".as_slice()),
            ("icon000.png", b"\x89PNG\r\n\x1a\n".as_slice()),
        ]);

        assert_eq!(
            laid_out(&raw, under),
            BTreeMap::from([
                (under.join("Game.dat"), b"first body".to_vec()),
                (
                    under.join("DataBase.dat"),
                    b"a rather longer second body".to_vec()
                ),
                (under.join("icon000.png"), b"\x89PNG\r\n\x1a\n".to_vec()),
            ]),
            "every later test reads an archive back through this path, so a body landing under the \
             wrong name here would let all of them pass on the wrong file"
        );
    }

    #[test]
    fn an_archive_nobody_changed_is_handed_back_the_very_bytes_it_came_in_as() {
        let under = Path::new("/games/demo/Data/BasicData");
        let raw = plain(&[
            ("Game.dat", b"first body".as_slice()),
            ("DataBase.dat", b"a rather longer second body".as_slice()),
        ]);

        let (_held, at) = on_disk(&raw);

        assert_eq!(
            resealed(&at, &[], under, &BTreeMap::new()).map(|sealed| sealed.body),
            Ok(raw),
            "a reader who translated nothing still has every archive written back, so writing one \
             out untouched has to be the same bytes it came in as rather than every file in it \
             opened out and packed again"
        );
    }

    #[test]
    fn what_the_editor_left_past_the_fields_this_reader_spells_out_is_carried_over_untouched() {
        let under = Path::new("/games/demo/Data/BasicData");
        let mut raw = plain(&[("Game.dat", b"first body".as_slice())]);
        raw[60] = 0x2A;

        let (_held, at) = on_disk(&raw);

        let mut fresh = BTreeMap::new();
        fresh.insert(under.join("Game.dat"), b"a much longer body".to_vec());

        let out = resealed(&at, &[], under, &fresh)
            .expect("a written archive")
            .body;

        assert_eq!(
            out[60], 0x2A,
            "the fifteen bytes after the huffman size are where a newer archive keeps the password \
             its own address fields are sealed with, so rebuilding the head out of only the nine \
             fields this reader parses would drop that password and hand back an archive the game \
             can no longer open"
        );
    }

    #[test]
    fn only_the_files_that_changed_are_written_and_the_rest_are_carried_over_whole() {
        let under = Path::new("/games/demo/Data/BasicData");
        let raw = plain(&[
            ("Game.dat", b"first body".as_slice()),
            ("DataBase.dat", b"second".as_slice()),
        ]);

        let (_held, at) = on_disk(&raw);

        let said = b"a translation that is much longer than what it replaced";
        let mut fresh = BTreeMap::new();
        fresh.insert(under.join("DataBase.dat"), said.to_vec());

        let out = resealed(&at, &[], under, &fresh)
            .expect("a written archive")
            .body;

        assert_eq!(
            out.len(),
            raw.len() + said.len(),
            "the archive grows by exactly the one line that changed, which is the whole point: a \
             game bundles a handful of scripts in among hundreds of megabytes of pictures, and \
             opening every picture out to reach them is minutes of work for nothing"
        );
        let laid = laid_out(&out, under);
        assert_eq!(
            laid.get(&under.join("DataBase.dat")).map(Vec::as_slice),
            Some(said.as_slice())
        );
        assert_eq!(
            laid.get(&under.join("Game.dat")).map(Vec::as_slice),
            Some(b"first body".as_slice()),
            "the file nobody touched comes back byte for byte, even though the file before it in \
             the table grew and moved everything after it along"
        );
    }

    #[test]
    fn only_the_files_worth_reading_are_lifted_out_of_an_archive_full_of_pictures() {
        let under = Path::new("/games/demo/Data/Locker");
        let raw = plain(&[
            ("talk001.txt", b"@mes hello".as_slice()),
            ("face001.png", b"\x89PNG\r\n\x1a\n".as_slice()),
            ("face002.png", b"\x89PNG\r\n\x1a\n".as_slice()),
            ("talk002.txt", b"@mes there".as_slice()),
        ]);

        let (_held, at) = on_disk(&raw);

        let mut seen = Vec::new();
        let count = poured(
            &at,
            &[],
            under,
            |one| one.extension().is_some_and(|kind| kind == "txt"),
            |held| {
                seen.push(held.at);
                Ok(())
            },
        )
        .expect("the archive opens");

        assert_eq!(count, 2);
        assert_eq!(
            seen,
            vec![under.join("talk001.txt"), under.join("talk002.txt")],
            "a real game bundles a handful of scripts in with hundreds of pictures, so deciding per archive either loses the scripts or unpacks a gigabyte of pictures to reach them"
        );
    }

    #[test]
    fn a_key_is_read_off_an_archive_that_asks_for_none_without_turning_the_game_away() {
        let raw = plain(&[("Game.dat", b"a body".as_slice())]);

        let (_held, at) = on_disk(&raw);
        let key = key_for(&at, None);

        assert_eq!(
            key,
            Ok(Some(Vec::new())),
            "an archive whose flags say it holds no key needs none, and refusing it for want of \
             one would turn away a game this reader can open perfectly well"
        );
    }

    #[test]
    fn a_sealed_archive_is_opened_with_the_key_the_game_ships_and_sealed_again_on_the_way_back() {
        let under = Path::new("/games/demo/Data/BasicData");
        let key = unseal::keyed(unseal::SHIPPED, None).expect("the key a plain game ships with");

        let raw = archived(
            &[
                ("Game.dat", b"the face the game shipped with".as_slice()),
                ("DataBase.dat", b"a rather longer second body".as_slice()),
            ],
            Some(&key),
        );

        let (_held, at) = on_disk(&raw);

        assert_eq!(
            key_for(&at, None),
            Ok(Some(key.clone())),
            "the flags say this one is sealed, so a key has to be read for it rather than none"
        );

        let mut was = BTreeMap::new();
        poured(&at, &key, under, all, |held| {
            was.insert(held.at, held.body);
            Ok(())
        })
        .expect("the archive opens");

        assert_eq!(
            was.get(&under.join("Game.dat")).map(Vec::as_slice),
            Some(b"the face the game shipped with".as_slice()),
            "a sealed body is the same bytes as a loose one once the seal is off"
        );

        let mut fresh = BTreeMap::new();
        fresh.insert(under.join("Game.dat"), b"a face somebody picked".to_vec());

        let out = resealed(&at, &key, under, &fresh)
            .expect("a written archive")
            .body;
        let (_kept, again) = on_disk(&out);

        let mut after = BTreeMap::new();
        poured(&again, &key, under, all, |held| {
            after.insert(held.at, held.body);
            Ok(())
        })
        .expect("the written archive opens");

        assert_eq!(
            after.get(&under.join("Game.dat")).map(Vec::as_slice),
            Some(b"a face somebody picked".as_slice()),
            "what goes back in has to be sealed the way the engine expects, or the game reads a font name out of noise"
        );
        assert_eq!(
            after.get(&under.join("DataBase.dat")).map(Vec::as_slice),
            Some(b"a rather longer second body".as_slice()),
            "and the file nobody touched is unsealed and sealed again byte for byte"
        );
    }

    #[test]
    fn only_the_head_of_a_file_is_read_when_all_that_is_wanted_is_the_size_it_names() {
        let under = Path::new("/games/demo/Data/BG");
        let key = unseal::keyed(unseal::SHIPPED, None).expect("the key a plain game ships with");
        let body: Vec<u8> = (0..300u32).map(|one| (one % 251) as u8).collect();

        let raw = archived(
            &[
                ("wide.png", body.as_slice()),
                ("small.png", b"short".as_slice()),
            ],
            Some(&key),
        );
        let (_held, at) = on_disk(&raw);

        let mut seen = Vec::new();
        peeked(&at, &key, under, 16, all, |held| {
            seen.push((held.at, held.size, held.head));
            Ok(())
        })
        .expect("the archive opens");

        assert_eq!(seen.len(), 2);
        assert_eq!(
            seen[0].1, 300,
            "the size of the whole file has to come through beside the head, or nothing can tell \
             a head short of its own end from a file read whole"
        );
        assert_eq!(
            seen[0].2.as_ref().map(Vec::as_slice),
            Ok(&body[..16]),
            "a picture says its size in its first bytes, and this game keeps three gigabytes of \
             them: reading the head has to land on the same keystream as reading the file whole, \
             or every size listed is noise"
        );
        assert_eq!(
            seen[1].2.as_ref().map(Vec::as_slice),
            Ok(b"short".as_slice()),
            "a file shorter than the head asked for gives up all it has rather than reaching past \
             its own end"
        );
    }

    #[test]
    fn a_file_that_does_not_open_like_an_archive_is_turned_away() {
        assert!(Head::read(b"not an archive at all, really").is_err());
    }

    #[test]
    fn a_data_file_that_merely_ends_in_wolf_is_no_archive_that_stayed_shut() {
        let (_held, at) = on_disk(&vec![7u8; 2002]);

        assert_eq!(
            key_for(&at, None),
            Ok(None),
            "a Pro game keeps small data files of its own beside its archives under the same \
             ending, and calling those archives this reader could not open is a warning about \
             nothing on every single read"
        );
    }

    #[test]
    fn an_archive_of_any_version_but_ours_is_turned_away_and_told_what_would_open_it() {
        for version in [6u16, 9] {
            let mut head = vec![0u8; keying::HEAD_LEN as usize];
            head[..2].copy_from_slice(&MARK.to_le_bytes());
            head[2..4].copy_from_slice(&version.to_le_bytes());

            assert_eq!(
                Head::read(&head).err(),
                Some(format!(
                    "this reader opens the version 8 archive Wolf RPG Editor 3 writes and this \
                     one is version {version}, so {}",
                    coder::OLDER_EDITOR
                )),
                "an archive is turned away by the version it carries, and whichever side of \
                 ours it falls on, laying it out in Wolf RPG Editor 3 is what would get the \
                 game read"
            );
        }
    }

    const WOLF_THREE: [u16; 4] = [
        unseal::SPOKEN_300,
        unseal::SPOKEN_314,
        unseal::SPOKEN_331,
        unseal::SPOKEN_350,
    ];

    const SPLIT_KB: u8 = 1;

    fn wide() -> Vec<u8> {
        (0..3000u32).map(|one| (one % 251) as u8).collect()
    }

    fn spread(wide: &[u8]) -> Vec<(&'static str, &[u8])> {
        vec![
            ("Game.dat", b"the face the game shipped with".as_slice()),
            ("DataBase.dat", b"a rather longer second body".as_slice()),
            ("MapData/Map001.mps", b"@mes hello from a folder".as_slice()),
            ("Picture/Face/face01.png", b"\x89PNG\r\n\x1a\n".as_slice()),
            ("wide.bin", wide),
        ]
    }

    fn wanted(wide: &[u8], under: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        spread(wide)
            .into_iter()
            .map(|(name, body)| {
                (
                    under.join(name.replace('/', MAIN_SEPARATOR_STR)),
                    body.to_vec(),
                )
            })
            .collect()
    }

    fn read_with(raw: &[u8], key: &[u8], under: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        let (_held, at) = on_disk(raw);

        let mut out = BTreeMap::new();
        poured(&at, key, under, all, |held| {
            out.insert(held.at, held.body);
            Ok(())
        })
        .expect("the archive opens");

        out
    }

    #[test]
    fn the_packed_body_the_fixture_writes_is_one_this_reader_opens_back_out_to_what_went_in() {
        let payload: Vec<u8> = (0..2048u32).map(|one| (one % 253) as u8).collect();

        assert_eq!(
            squeeze::unhuffed(&huffed(&payload)),
            Ok(payload),
            "the whole point of the split body in these fixtures is to reach the second read a \
             file needs, so the fixture has to write a packed head this reader really opens"
        );
    }

    #[test]
    fn each_of_the_four_wolf_three_seals_hands_back_every_body_that_went_into_it() {
        let under = Path::new("/games/demo/Data/BasicData");
        let held = wide();

        for crypt in WOLF_THREE {
            let key = unseal::keyed(crypt, None).expect("the key string");
            let raw = sealed_as(crypt, &spread(&held), SPLIT_KB);

            assert_eq!(
                read_with(&raw, &key, under),
                wanted(&held, under),
                "a body sealed the {crypt:#x} way is the same bytes as a loose one once the seal \
                 is off, and that has to hold for a file at the root, a file two folders down, and \
                 a file long enough to be packed at both ends and read in two goes"
            );
        }
    }

    #[test]
    fn the_head_of_a_wolf_three_picture_is_read_off_the_same_stream_as_the_whole_file_would_be() {
        let under = Path::new("/games/demo/Data/BG");
        let held = wide();

        for crypt in WOLF_THREE {
            let key = unseal::keyed(crypt, None).expect("the key string");
            let raw = sealed_as(crypt, &spread(&held), SPLIT_KB);
            let (_kept, at) = on_disk(&raw);

            let mut seen = BTreeMap::new();
            peeked(&at, &key, under, 16, all, |one| {
                seen.insert(one.at, (one.size, one.head));
                Ok(())
            })
            .expect("the archive opens");

            assert_eq!(
                seen.get(&under.join("wide.bin")),
                Some(&(held.len(), Ok(held[..16].to_vec()))),
                "a picture says its size in its first bytes and this game keeps three gigabytes of \
                 them, so reading only the head has to land on the same stream reading the whole \
                 file would. {crypt:#x}"
            );
            assert_eq!(
                seen.get(&under.join("Picture").join("Face").join("face01.png")),
                Some(&(8, Ok(b"\x89PNG\r\n\x1a\n".to_vec()))),
                "a file shorter than the head asked for gives up all it has, and its pad still \
                 comes off the folders holding it. {crypt:#x}"
            );
        }
    }

    #[test]
    fn the_pad_each_file_of_a_classic_archive_is_sealed_with_is_read_off_its_own_name_and_folders()
    {
        let held = wide();
        let raw = sealed_as(unseal::SPOKEN_300, &spread(&held), SPLIT_KB);
        let key = unseal::keyed(unseal::SPOKEN_300, None).expect("the key string");

        let (_kept, at) = on_disk(&raw);
        let open = opened(&at, &key, Path::new("")).expect("the archive opens");

        let pads: BTreeMap<PathBuf, [u8; keying::PAD]> = open
            .filed
            .iter()
            .map(|one| (one.at.clone(), one.pad))
            .collect();

        assert_eq!(
            pads.get(Path::new("Game.dat")),
            Some(&[0x42, 0x88, 0x44, 0xc5, 0xca, 0x41, 0x74]),
            "these bytes come from running the UberWolf reference in the vectors harness"
        );
        assert_eq!(
            pads.get(&PathBuf::from("MapData").join("Map001.mps")),
            Some(&[0x56, 0xfe, 0x59, 0x64, 0xaf, 0xec, 0x0a]),
            "these bytes come from running the UberWolf reference in the vectors harness"
        );
        assert_eq!(
            pads.get(&PathBuf::from("Picture").join("Face").join("face01.png")),
            Some(&[0x58, 0xec, 0xb8, 0x48, 0x57, 0x94, 0x70]),
            "two folders deep the innermost comes first and the root is left out, and these bytes \
             come from running the UberWolf reference in the vectors harness"
        );
        assert_eq!(
            pads.get(Path::new("wide.bin")),
            Some(&[0x60, 0x6c, 0x57, 0x94, 0xd9, 0x37, 0xf8]),
            "these bytes come from running the UberWolf reference in the vectors harness"
        );
    }

    #[test]
    fn a_newer_archive_hides_its_four_addresses_behind_a_mask_read_off_the_password_beside_them() {
        let held = wide();

        for crypt in [unseal::SPOKEN_331, unseal::SPOKEN_350] {
            let raw = sealed_as(crypt, &spread(&held), SPLIT_KB);

            assert_ne!(
                coder::long_at(&raw, 8),
                Ok(keying::HEAD_LEN),
                "the four addresses sit behind a mask, so a reader that took them straight off the \
                 file would look for the names table in the wrong place and turn the whole game \
                 away. {crypt:#x}"
            );

            let mut head = raw[..keying::HEAD_LEN as usize].to_vec();
            unseal::unmasked(crypt, &mut head).expect("the mask comes off");

            assert_eq!(
                coder::long_at(&head, 8),
                Ok(keying::HEAD_LEN),
                "and once it is off the data starts where every packer in this family puts it. \
                 {crypt:#x}"
            );
            assert_eq!(
                &head[keying::RESERVE_AT..],
                &RESERVE,
                "the password the mask itself is drawn from is never hidden, or nothing could \
                 unhide anything. {crypt:#x}"
            );
            assert_eq!(
                coder::word_at(&head, 44),
                coder::word_at(&raw, 44),
                "and neither are the flags the version is read out of. {crypt:#x}"
            );
        }
    }

    #[test]
    fn writing_a_wolf_three_archive_back_untouched_hands_back_every_body_it_held_before() {
        let under = Path::new("/games/demo/Data/BasicData");
        let held = wide();

        for crypt in WOLF_THREE {
            let key = unseal::keyed(crypt, None).expect("the key string");
            let raw = sealed_as(crypt, &spread(&held), SPLIT_KB);
            let (_kept, at) = on_disk(&raw);

            let out = resealed(&at, &key, under, &BTreeMap::new())
                .expect("a written archive")
                .body;

            assert_eq!(
                read_with(&out, &key, under),
                wanted(&held, under),
                "a reader who translated nothing still has every archive written back, so the \
                 {crypt:#x} seal has to survive the trip whether or not anything changed"
            );
            assert_eq!(
                &out[keying::RESERVE_AT..keying::HEAD_LEN as usize],
                &RESERVE,
                "the fifteen bytes of password are the only key material all four of the newer \
                 layers have, so dropping them hands back an archive the game cannot open. \
                 {crypt:#x}"
            );
        }
    }

    #[test]
    fn a_line_written_into_one_file_of_a_wolf_three_archive_leaves_the_others_where_they_were() {
        let under = Path::new("/games/demo/Data/BasicData");
        let held = wide();
        let said = b"a translation that is a good deal longer than the line it replaced";

        for crypt in WOLF_THREE {
            let key = unseal::keyed(crypt, None).expect("the key string");
            let raw = sealed_as(crypt, &spread(&held), SPLIT_KB);
            let (_kept, at) = on_disk(&raw);

            let mut fresh = BTreeMap::new();
            fresh.insert(under.join("MapData").join("Map001.mps"), said.to_vec());

            let out = resealed(&at, &key, under, &fresh)
                .expect("a written archive")
                .body;

            assert_eq!(
                out.len(),
                raw.len() + said.len(),
                "the archive grows by exactly the one line that changed. {crypt:#x}"
            );

            let after = read_with(&out, &key, under);

            assert_eq!(
                after
                    .get(&under.join("MapData").join("Map001.mps"))
                    .map(Vec::as_slice),
                Some(said.as_slice()),
                "what goes back in has to be sealed the way the game expects, which for the newer \
                 family means the read stream at its own length and the outer stream at where it \
                 lands in the file. {crypt:#x}"
            );
            assert_eq!(
                after.get(&under.join("wide.bin")).map(Vec::as_slice),
                Some(held.as_slice()),
                "and the packed file nobody touched is carried over byte for byte rather than \
                 opened out and packed again. {crypt:#x}"
            );
            assert_eq!(
                after.get(&under.join("Picture").join("Face").join("face01.png")),
                Some(&b"\x89PNG\r\n\x1a\n".to_vec()),
                "including the one two folders down, whose pad is read off that very chain. \
                 {crypt:#x}"
            );
        }
    }

    #[test]
    fn only_the_bytes_of_a_wolf_three_archive_that_changed_are_written_and_the_rest_are_untouched()
    {
        let under = Path::new("/games/demo/Data/BasicData");
        let held = wide();

        for crypt in WOLF_THREE {
            let key = unseal::keyed(crypt, None).expect("the key string");
            let raw = sealed_as(crypt, &spread(&held), SPLIT_KB);
            let (_kept, at) = on_disk(&raw);

            let mut fresh = BTreeMap::new();
            fresh.insert(under.join("Game.dat"), b"a shorter line".to_vec());

            let out = resealed(&at, &key, under, &fresh)
                .expect("a written archive")
                .body;

            let mut head = raw[..keying::HEAD_LEN as usize].to_vec();
            unseal::unmasked(crypt, &mut head).expect("the mask comes off");
            let names_at = coder::long_at(&head, 16).expect("the shipped names table");

            assert_eq!(
                &out[keying::HEAD_LEN as usize..names_at as usize],
                &raw[keying::HEAD_LEN as usize..names_at as usize],
                "every body that nobody translated is copied over as the very bytes it came in \
                 as, which is the whole reason a gigabyte of pictures does not have to be opened \
                 out to reach one script. {crypt:#x}"
            );
        }
    }

    #[test]
    fn a_key_for_each_of_the_four_wolf_three_seals_is_read_off_the_archive_rather_than_refused() {
        let held = wide();

        for crypt in WOLF_THREE {
            let raw = sealed_as(crypt, &spread(&held), SPLIT_KB);
            let (_kept, at) = on_disk(&raw);

            assert_eq!(
                key_for(&at, None),
                Ok(Some(
                    unseal::keyed(crypt, None).expect("the key string this seal ships with")
                )),
                "before this these four were turned away with a note about a seal this reader \
                 could not open, and every game shipped under them read as nothing at all. \
                 {crypt:#x}"
            );
        }
    }

    #[test]
    fn a_seal_from_the_pro_editor_is_still_turned_away_with_a_note_saying_which_one_it_was() {
        let mut head = vec![0u8; keying::HEAD_LEN as usize];
        head[..2].copy_from_slice(&MARK.to_le_bytes());
        head[2..4].copy_from_slice(&NEWEST.to_le_bytes());
        head[44..48].copy_from_slice(&(1010u32 << 16).to_le_bytes());
        head[8..16].copy_from_slice(&keying::HEAD_LEN.to_le_bytes());
        head[16..24].copy_from_slice(&keying::HEAD_LEN.to_le_bytes());

        let (_kept, at) = on_disk(&head);

        assert_eq!(
            key_for(&at, None),
            Err("sealed the 0x3f2 way, which this reader cannot open".to_string()),
            "the pro band leans on a second key nobody outside the editor has, so it stays refused \
             rather than being guessed at"
        );
    }

    #[test]
    fn an_archive_whose_folders_lead_back_into_themselves_is_refused_rather_than_running_out_of_stack()
     {
        let mut index = entry("Loop");

        let files_at = index.len();
        index.extend_from_slice(&0u64.to_le_bytes());
        index.extend_from_slice(&FOLDER.to_le_bytes());
        index.extend_from_slice(&[0u8; 24]);
        index.extend_from_slice(&0u64.to_le_bytes());
        index.extend_from_slice(&0u64.to_le_bytes());
        index.extend_from_slice(&LOOSE.to_le_bytes());
        index.extend_from_slice(&LOOSE.to_le_bytes());

        let dirs_at = index.len();
        index.extend_from_slice(&0u64.to_le_bytes());
        index.extend_from_slice(&0u64.to_le_bytes());
        index.extend_from_slice(&1u64.to_le_bytes());
        index.extend_from_slice(&0u64.to_le_bytes());

        let head = Head {
            head_size: index.len(),
            data_at: keying::HEAD_LEN,
            names_at: keying::HEAD_LEN,
            files_at,
            dirs_at,
            char_code: 932,
            flags: NO_KEY | NO_HEAD_PRESS,
            huff_kb: ALL_HUFF,
        };

        let mut out = Vec::new();
        assert!(
            walked(
                &Walk {
                    head: &head,
                    seal: &unseal::Seal::Loose,
                    index: &index,
                },
                0,
                Path::new(""),
                &[],
                0,
                &mut out
            )
            .is_err(),
            "a folder that holds itself would be walked for ever, and running out of stack takes \
             the whole app down where an error only turns one archive away"
        );
    }
}
