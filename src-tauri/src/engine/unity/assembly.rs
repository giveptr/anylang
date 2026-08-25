use crate::engine::unity::{Harvest, dotnet, naming};
use crate::engine::{Offer, sheet};
use anyhow::{Context, Result, bail};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const NAME: &str = "assembly";
pub const KIND: &str = "Assembly";

const BUCKET: u32 = 256;
const OURS: [&str; 2] = ["Assembly-CSharp", "Assembly-UnityScript"];

const LDSTR: u8 = 0x72;
const SWITCH: u8 = 0x45;
const WIDE: u8 = 0xFE;
const STRING_TOKEN: u32 = 0x7000_0000;
const OFFSET: u32 = 0x00FF_FFFF;

const TYPE_DEF: usize = 0x02;
const METHOD_DEF: usize = 0x06;

const DISCARDABLE: u32 = 0x0200_0000;
const READABLE: u32 = 0x4000_0000;
const INITIALIZED: u32 = 0x0000_0040;

pub fn ours(at: &Path) -> bool {
    let named = at.file_stem().and_then(|stem| stem.to_str());
    let dll = at
        .extension()
        .is_some_and(|kind| kind.eq_ignore_ascii_case("dll"));

    dll && named.is_some_and(|stem| {
        OURS.iter()
            .any(|one| stem.eq_ignore_ascii_case(one) || same_family(stem, one))
    })
}

fn same_family(stem: &str, head: &str) -> bool {
    stem.as_bytes()
        .split_at_checked(head.len())
        .is_some_and(|(front, rest)| {
            front.eq_ignore_ascii_case(head.as_bytes()) && rest.first() == Some(&b'-')
        })
}

fn a_key(text: &str) -> bool {
    let body = text.trim();

    if body
        .chars()
        .any(|one| one.is_alphabetic() && !one.is_ascii())
    {
        return false;
    }

    !(body.chars().any(char::is_whitespace) && body.chars().any(char::is_alphabetic))
}

struct Section {
    head: usize,
    address: u32,
    virtual_size: u32,
    raw_at: u32,
    raw_size: u32,
}

impl Section {
    fn holds(&self, address: u32) -> bool {
        address >= self.address && address < self.address + self.virtual_size.max(self.raw_size)
    }
}

pub struct Stream {
    head: usize,
    name: String,
    pub at: u32,
    pub size: u32,
}

pub struct Pe {
    optional: usize,
    file_step: u32,
    memory_step: u32,
    sections: Vec<Section>,
    cli: usize,
    pub meta: usize,
    meta_size: usize,
    streams: Vec<Stream>,
}

fn four(raw: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        raw.get(at..at + 4)
            .with_context(|| format!("this file ends before {at:#x}"))?
            .try_into()?,
    ))
}

fn two(raw: &[u8], at: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(
        raw.get(at..at + 2)
            .with_context(|| format!("this file ends before {at:#x}"))?
            .try_into()?,
    ))
}

fn put(out: &mut [u8], at: usize, value: u32) {
    out[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

impl Pe {
    pub fn read(raw: &[u8]) -> Result<Self> {
        let head = four(raw, 0x3C)? as usize;
        if raw.get(head..head + 4) != Some(b"PE\0\0") {
            bail!("this is not a PE file");
        }

        let file_header = head + 4;
        let count = two(raw, file_header + 2)? as usize;
        let optional_size = two(raw, file_header + 16)? as usize;
        let optional = file_header + 20;
        let magic = two(raw, optional)?;
        if magic != 0x10B && magic != 0x20B {
            bail!("this PE file has no optional header this reader knows");
        }

        let directories = optional + if magic == 0x10B { 96 } else { 112 };
        let memory_step = four(raw, optional + 32)?;
        let file_step = four(raw, optional + 36)?;

        let mut sections = Vec::with_capacity(count);
        for which in 0..count {
            let head = optional + optional_size + which * 40;
            sections.push(Section {
                head,
                virtual_size: four(raw, head + 8)?,
                address: four(raw, head + 12)?,
                raw_size: four(raw, head + 16)?,
                raw_at: four(raw, head + 20)?,
            });
        }

        let mut image = Self {
            optional,
            file_step,
            memory_step,
            sections,
            cli: 0,
            meta: 0,
            meta_size: 0,
            streams: Vec::new(),
        };

        let cli_at = four(raw, directories + 14 * 8)?;
        image.cli = image
            .flat(cli_at)
            .context("this assembly has no CLI header")?;

        let root = four(raw, image.cli + 8)?;
        image.meta_size = four(raw, image.cli + 12)? as usize;
        image.meta = image
            .flat(root)
            .context("this assembly has no metadata root")?;
        image.streams = read_streams(raw, image.meta)?;

        Ok(image)
    }

    fn flat(&self, address: u32) -> Option<usize> {
        self.sections
            .iter()
            .find(|one| one.holds(address))
            .map(|one| (one.raw_at + (address - one.address)) as usize)
    }

    pub fn stream(&self, name: &str) -> Option<&Stream> {
        self.streams.iter().find(|one| one.name == name)
    }

    fn last(&self) -> Result<&Section> {
        let by_file = self
            .sections
            .iter()
            .max_by_key(|one| one.raw_at)
            .context("this assembly has no sections")?;
        let by_memory = self
            .sections
            .iter()
            .max_by_key(|one| one.address)
            .context("this assembly has no sections")?;

        if by_file.head != by_memory.head {
            bail!("this assembly lays its sections out back to front, so nothing can be appended");
        }

        Ok(by_file)
    }
}

fn read_streams(raw: &[u8], meta: usize) -> Result<Vec<Stream>> {
    if raw.get(meta..meta + 4) != Some(b"BSJB") {
        bail!("this assembly has no metadata signature");
    }

    let version = four(raw, meta + 12)? as usize;
    let mut walk = meta + 16 + version + 2;
    let count = two(raw, walk)? as usize;
    walk += 2;

    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let head = walk;
        let at = four(raw, walk)?;
        let size = four(raw, walk + 4)?;
        walk += 8;

        let end = raw[walk..]
            .iter()
            .position(|byte| *byte == 0)
            .map(|step| walk + step)
            .context("a stream name never ends")?;

        out.push(Stream {
            head,
            name: String::from_utf8_lossy(&raw[walk..end]).into_owned(),
            at,
            size,
        });

        walk = meta + (end + 1 - meta).next_multiple_of(4);
    }

    Ok(out)
}

fn width(code: u8) -> Option<usize> {
    Some(match code {
        0x00..=0x0D => 0,
        0x0E..=0x13 => 1,
        0x14..=0x1E => 0,
        0x1F => 1,
        0x20 => 4,
        0x21 => 8,
        0x22 => 4,
        0x23 => 8,
        0x25 | 0x26 => 0,
        0x27..=0x29 => 4,
        0x2A => 0,
        0x2B..=0x37 => 1,
        0x38..=0x44 => 4,
        0x46..=0x6E => 0,
        0x6F..=0x75 => 4,
        0x76 => 0,
        0x79 => 4,
        0x7A => 0,
        0x7B..=0x81 => 4,
        0x82..=0x8B => 0,
        0x8C | 0x8D => 4,
        0x8E => 0,
        0x8F => 4,
        0x90..=0xA2 => 0,
        0xA3..=0xA5 => 4,
        0xB3..=0xBA => 0,
        0xC2 => 4,
        0xC3 => 0,
        0xC6 => 4,
        0xD0 => 4,
        0xD1..=0xDC => 0,
        0xDD => 4,
        0xDE => 1,
        0xDF | 0xE0 => 0,
        _ => return None,
    })
}

fn wide_width(code: u8) -> Option<usize> {
    Some(match code {
        0x00..=0x05 => 0,
        0x06 | 0x07 => 4,
        0x09..=0x0E => 2,
        0x0F => 0,
        0x11 => 0,
        0x12 => 1,
        0x13 | 0x14 => 0,
        0x15 | 0x16 => 4,
        0x17 | 0x18 => 0,
        0x19 => 1,
        0x1A => 0,
        0x1C => 4,
        0x1D | 0x1E => 0,
        _ => return None,
    })
}

fn body_of(raw: &[u8], at: usize) -> Option<(usize, usize)> {
    let first = *raw.get(at)?;

    match first & 3 {
        2 => Some((at + 1, (first >> 2) as usize)),
        3 => {
            let flags = two(raw, at).ok()?;
            let head = ((flags >> 12) & 0xF) as usize * 4;
            let size = four(raw, at + 4).ok()? as usize;

            (head >= 12).then_some((at + head, size))
        }
        _ => None,
    }
}

fn each_ldstr(raw: &[u8], from: usize, size: usize, mut out: impl FnMut(u32, usize)) {
    let Some(code) = raw.get(from..from + size) else {
        return;
    };

    let mut at = 0;
    while at < code.len() {
        let opcode = code[at];
        at += 1;

        let step = if opcode == WIDE {
            let Some(second) = code.get(at).copied() else {
                return;
            };
            at += 1;

            wide_width(second)
        } else if opcode == SWITCH {
            let Some(cases) = code.get(at..at + 4) else {
                return;
            };

            Some(4 + u32::from_le_bytes(cases.try_into().unwrap_or_default()) as usize * 4)
        } else {
            width(opcode)
        };

        let Some(step) = step else {
            return;
        };
        if at + step > code.len() {
            return;
        }

        if opcode == LDSTR {
            let token = u32::from_le_bytes(code[at..at + 4].try_into().unwrap_or_default());
            if token & 0xFF00_0000 == STRING_TOKEN {
                out(token & OFFSET, from + at);
            }
        }

        at += step;
    }
}

fn owners(tables: &dotnet::Image) -> Vec<(u32, String)> {
    let mut out = Vec::new();

    for row in 0..tables.count(TYPE_DEF) {
        let name = tables.text(tables.cell(TYPE_DEF, row, 1));
        let space = tables.text(tables.cell(TYPE_DEF, row, 2));
        let first = tables.cell(TYPE_DEF, row, 5);

        out.push((first, dotnet::full_name(space, name)));
    }

    out
}

fn owner_of(owners: &[(u32, String)], method: usize) -> &str {
    let wanted = method as u32 + 1;

    owners
        .iter()
        .rev()
        .find(|(first, _)| *first != 0 && *first <= wanted)
        .map(|(_, name)| name.as_str())
        .unwrap_or("")
}

struct Literal {
    at: u32,
    text: String,
    owner: String,
}

type Sites = BTreeMap<u32, Vec<usize>>;

struct Reading {
    literals: Vec<Literal>,
    sites: Sites,
}

fn read_strings(raw: &[u8]) -> Result<Reading> {
    let pe = Pe::read(raw)?;
    let heap = pe
        .stream("#US")
        .context("this assembly keeps no user strings")?;
    let heap_at = pe.meta + heap.at as usize;
    let heap_size = heap.size as usize;

    let tables = dotnet::Image::inside(raw, &pe)?;
    let holders = owners(&tables);

    let mut sites: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
    let mut first_seen: BTreeMap<u32, usize> = BTreeMap::new();

    for row in 0..tables.count(METHOD_DEF) {
        let address = tables.cell(METHOD_DEF, row, 0);
        if address == 0 {
            continue;
        }

        let Some(head) = pe.flat(address) else {
            continue;
        };
        let Some((from, size)) = body_of(raw, head) else {
            continue;
        };

        each_ldstr(raw, from, size, |at, spot| {
            sites.entry(at).or_default().push(spot);
            first_seen.entry(at).or_insert(row);
        });
    }

    let mut literals = Vec::with_capacity(sites.len());
    for at in sites.keys() {
        let Some(text) = string_at(raw, heap_at, heap_size, *at) else {
            continue;
        };
        if text.is_empty() {
            continue;
        }

        let row = first_seen.get(at).copied().unwrap_or(0);
        literals.push(Literal {
            at: *at,
            text,
            owner: owner_of(&holders, row).to_string(),
        });
    }

    Ok(Reading { literals, sites })
}

fn string_at(raw: &[u8], heap_at: usize, heap_size: usize, at: u32) -> Option<String> {
    let at = at as usize;
    if at >= heap_size {
        return None;
    }

    let (size, step) = dotnet::unpacked(raw, heap_at + at)?;
    if size == 0 || at + step + size > heap_size {
        return None;
    }

    let body = raw.get(heap_at + at + step..heap_at + at + step + size - 1)?;
    let units: Vec<u16> = body
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect();

    String::from_utf16(&units).ok()
}

fn packed(size: usize, out: &mut Vec<u8>) -> Result<()> {
    match size {
        0..0x80 => out.push(size as u8),
        0x80..0x4000 => {
            out.push(0x80 | (size >> 8) as u8);
            out.push(size as u8);
        }
        0x4000..0x2000_0000 => {
            out.push(0xC0 | (size >> 24) as u8);
            out.push((size >> 16) as u8);
            out.push((size >> 8) as u8);
            out.push(size as u8);
        }
        _ => bail!("a line of {size} bytes is longer than any assembly can hold"),
    }

    Ok(())
}

fn entry(text: &str) -> Result<Vec<u8>> {
    let units: Vec<u16> = text.encode_utf16().collect();
    let mut body = Vec::with_capacity(units.len() * 2 + 1);
    for unit in &units {
        body.extend_from_slice(&unit.to_le_bytes());
    }
    body.push(u8::from(units.iter().any(|one| *one >= 0x80)));

    let mut out = Vec::with_capacity(body.len() + 4);
    packed(body.len(), &mut out)?;
    out.extend_from_slice(&body);

    Ok(out)
}

pub fn take(holder: &str, raw: &[u8]) -> Result<Vec<Harvest>> {
    let read = read_strings(raw)?;
    let mut piled: BTreeMap<PathBuf, Vec<sheet::Line>> = BTreeMap::new();

    for one in read.literals {
        let floor = one.at / BUCKET * BUCKET;
        let under = PathBuf::from(NAME)
            .join(holder)
            .join(naming::named(&one.owner, i64::from(one.at)))
            .join(format!("{floor}.{}", sheet::SUFFIX));

        piled.entry(under).or_default().push(sheet::Line {
            spot: one.at.to_string(),
            offer: Offer::default().or_listed(a_key(&one.text)),
            said: one.text,
        });
    }

    Harvest::sheets(piled)
}

pub fn put_back(raw: &[u8], lines: &BTreeMap<String, String>) -> Result<Option<(Vec<u8>, usize)>> {
    let read = read_strings(raw)?;

    let mut swaps: BTreeMap<u32, String> = BTreeMap::new();
    for one in read.literals {
        let Some(said) = lines.get(&one.at.to_string()) else {
            continue;
        };

        if *said != one.text && !said.is_empty() {
            swaps.insert(one.at, said.clone());
        }
    }

    if swaps.is_empty() {
        return Ok(None);
    }

    let pieces = swaps.len();

    rewrite(raw, &swaps, &read.sites).map(|bytes| Some((bytes, pieces)))
}

fn rewrite(raw: &[u8], swaps: &BTreeMap<u32, String>, sites: &Sites) -> Result<Vec<u8>> {
    let pe = Pe::read(raw)?;
    let heap = pe
        .stream("#US")
        .context("this assembly keeps no user strings")?;
    let grown = pe.last()?;

    let mut extra: Vec<u8> = Vec::new();
    let mut moved: BTreeMap<u32, u32> = BTreeMap::new();

    for (at, said) in swaps {
        if !sites.contains_key(at) {
            continue;
        }

        let landing = heap.size as usize + extra.len();
        if landing > OFFSET as usize {
            bail!("the strings in this assembly no longer fit the room a token can point at");
        }

        extra.extend_from_slice(&entry(said)?);
        moved.insert(*at, landing as u32);
    }

    while !(heap.size as usize + extra.len()).is_multiple_of(4) {
        extra.push(0);
    }

    let end = pe.meta + pe.meta_size;
    let cut = pe.meta + heap.at as usize + heap.size as usize;
    if end > raw.len() || cut > end {
        bail!("this assembly's metadata does not reach the user strings it points at");
    }

    let mut blob = Vec::with_capacity(pe.meta_size + extra.len());
    blob.extend_from_slice(&raw[pe.meta..cut]);
    blob.extend_from_slice(&extra);
    blob.extend_from_slice(&raw[cut..end]);

    let step = extra.len() as u32;
    for one in &pe.streams {
        let head = one.head - pe.meta;

        if one.name == heap.name {
            put(&mut blob, head + 4, one.size + step);
        } else if one.at > heap.at {
            put(&mut blob, head, one.at + step);
        }
    }

    let mut out = raw.to_vec();
    let floor = (grown.raw_at + grown.raw_size) as usize;
    if out.len() < floor {
        out.resize(floor, 0);
    }
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }

    let landing = out.len();
    let address = grown.address + (landing as u32 - grown.raw_at);
    out.extend_from_slice(&blob);

    let virtual_size = out.len() as u32 - grown.raw_at;
    let raw_size = virtual_size.next_multiple_of(pe.file_step.max(1));
    out.resize((grown.raw_at + raw_size) as usize, 0);

    put(&mut out, grown.head + 8, virtual_size);
    put(&mut out, grown.head + 16, raw_size);
    let manners = four(&out, grown.head + 36)?;
    put(
        &mut out,
        grown.head + 36,
        (manners & !DISCARDABLE) | READABLE | INITIALIZED,
    );

    let reach = (grown.address + virtual_size).next_multiple_of(pe.memory_step.max(1));
    put(&mut out, pe.optional + 56, reach);
    put(&mut out, pe.optional + 64, 0);
    put(&mut out, pe.cli + 8, address);
    put(&mut out, pe.cli + 12, blob.len() as u32);

    for (was, now) in &moved {
        for spot in sites.get(was).into_iter().flatten() {
            put(&mut out, *spot, STRING_TOKEN | now);
        }
    }

    reads_back(&out, swaps, &moved)?;

    Ok(out)
}

fn reads_back(out: &[u8], swaps: &BTreeMap<u32, String>, moved: &BTreeMap<u32, u32>) -> Result<()> {
    let pe = Pe::read(out).context("the rewritten assembly no longer reads back")?;
    let heap = pe
        .stream("#US")
        .context("the rewritten assembly lost its user strings")?;
    let heap_at = pe.meta + heap.at as usize;

    for (was, now) in moved {
        let said = string_at(out, heap_at, heap.size as usize, *now)
            .with_context(|| format!("line {was} did not land in the rewritten assembly"))?;

        if Some(&said) != swaps.get(was) {
            bail!("line {was} reads back as {said:?} after being written");
        }
    }

    Ok(())
}

#[cfg(test)]
pub mod forge {
    use super::*;

    const PE_AT: usize = 0x80;
    const OPTIONAL: usize = PE_AT + 24;
    const SECTION: usize = OPTIONAL + 224;
    const RAW_AT: u32 = 0x200;
    const ADDRESS: u32 = 0x2000;

    pub struct Dll {
        pub raw: Vec<u8>,
        pub strings: Vec<u32>,
    }

    fn tables_stream(methods: &[u32], names: &[(u32, u32, u32)]) -> Vec<u8> {
        let mut out = vec![0u8; 24];
        out[6] = 0;

        let valid: u64 = (1 << TYPE_DEF) | (1 << METHOD_DEF);
        out[8..16].copy_from_slice(&valid.to_le_bytes());
        out[16..24].copy_from_slice(&valid.to_le_bytes());
        out.extend_from_slice(&(names.len() as u32).to_le_bytes());
        out.extend_from_slice(&(methods.len() as u32).to_le_bytes());

        for (name, space, first) in names {
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(&(*name as u16).to_le_bytes());
            out.extend_from_slice(&(*space as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&(*first as u16).to_le_bytes());
        }

        for address in methods {
            out.extend_from_slice(&address.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
        }

        while !out.len().is_multiple_of(4) {
            out.push(0);
        }

        out
    }

    fn heap_of(said: &[&str]) -> (Vec<u8>, Vec<u32>) {
        let mut out = vec![0u8];
        let mut spots = Vec::new();

        for one in said {
            spots.push(out.len() as u32);
            out.extend_from_slice(&entry(one).expect("a user string"));
        }

        while !out.len().is_multiple_of(4) {
            out.push(0);
        }

        (out, spots)
    }

    fn metadata(streams: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let version = b"v4.0.30319\0\0";
        let mut head = Vec::new();
        head.extend_from_slice(b"BSJB");
        head.extend_from_slice(&1u16.to_le_bytes());
        head.extend_from_slice(&1u16.to_le_bytes());
        head.extend_from_slice(&0u32.to_le_bytes());
        head.extend_from_slice(&(version.len() as u32).to_le_bytes());
        head.extend_from_slice(version);
        head.extend_from_slice(&0u16.to_le_bytes());
        head.extend_from_slice(&(streams.len() as u16).to_le_bytes());

        let mut directory = Vec::new();
        for (name, _) in streams {
            directory.extend_from_slice(&0u32.to_le_bytes());
            directory.extend_from_slice(&0u32.to_le_bytes());
            directory.extend_from_slice(name.as_bytes());
            directory.push(0);
            while !directory.len().is_multiple_of(4) {
                directory.push(0);
            }
        }

        let mut out = head;
        let start = out.len();
        out.extend_from_slice(&directory);

        let mut walk = start;
        for (name, body) in streams {
            let at = out.len() as u32;
            out.extend_from_slice(body);

            out[walk..walk + 4].copy_from_slice(&at.to_le_bytes());
            out[walk + 4..walk + 8].copy_from_slice(&(body.len() as u32).to_le_bytes());
            walk += 8 + (name.len() + 1).next_multiple_of(4);
        }

        out
    }

    pub fn dll(said: &[&str]) -> Dll {
        let (heap, spots) = heap_of(said);

        let mut code = Vec::new();
        for spot in &spots {
            code.push(LDSTR);
            code.extend_from_slice(&(STRING_TOKEN | spot).to_le_bytes());
            code.push(0x26);
        }
        code.push(0x2A);

        let mut body = vec![(code.len() as u8) << 2 | 2];
        body.extend_from_slice(&code);
        while !body.len().is_multiple_of(4) {
            body.push(0);
        }

        let mut strings = b"\0Talker\0".to_vec();
        while !strings.len().is_multiple_of(4) {
            strings.push(0);
        }

        let meta = metadata(&[
            ("#~", tables_stream(&[ADDRESS + 0x48], &[(1, 0, 1)])),
            ("#Strings", strings),
            ("#US", heap),
            ("#Blob", vec![0, 0, 0, 0]),
        ]);

        let meta_at = 0x100u32;
        let mut section = vec![0u8; 0x48];
        section[0..4].copy_from_slice(&0x48u32.to_le_bytes());
        section[4..6].copy_from_slice(&2u16.to_le_bytes());
        section[6..8].copy_from_slice(&5u16.to_le_bytes());
        section[8..12].copy_from_slice(&(ADDRESS + meta_at).to_le_bytes());
        section[12..16].copy_from_slice(&(meta.len() as u32).to_le_bytes());
        section.extend_from_slice(&body);

        section.resize(meta_at as usize, 0);
        section.extend_from_slice(&meta);

        let mut raw = vec![0u8; RAW_AT as usize];
        raw[0..2].copy_from_slice(b"MZ");
        raw[0x3C..0x40].copy_from_slice(&(PE_AT as u32).to_le_bytes());
        raw[PE_AT..PE_AT + 4].copy_from_slice(b"PE\0\0");
        raw[PE_AT + 6..PE_AT + 8].copy_from_slice(&1u16.to_le_bytes());
        raw[PE_AT + 20..PE_AT + 22].copy_from_slice(&224u16.to_le_bytes());
        raw[OPTIONAL..OPTIONAL + 2].copy_from_slice(&0x10Bu16.to_le_bytes());
        raw[OPTIONAL + 32..OPTIONAL + 36].copy_from_slice(&0x2000u32.to_le_bytes());
        raw[OPTIONAL + 36..OPTIONAL + 40].copy_from_slice(&0x200u32.to_le_bytes());

        let directories = OPTIONAL + 96;
        raw[directories + 14 * 8..directories + 14 * 8 + 4].copy_from_slice(&ADDRESS.to_le_bytes());

        raw[SECTION..SECTION + 8].copy_from_slice(b".text\0\0\0");
        raw[SECTION + 8..SECTION + 12].copy_from_slice(&(section.len() as u32).to_le_bytes());
        raw[SECTION + 12..SECTION + 16].copy_from_slice(&ADDRESS.to_le_bytes());
        raw[SECTION + 16..SECTION + 20]
            .copy_from_slice(&(section.len() as u32).next_multiple_of(0x200).to_le_bytes());
        raw[SECTION + 20..SECTION + 24].copy_from_slice(&RAW_AT.to_le_bytes());
        raw[SECTION + 36..SECTION + 40].copy_from_slice(&0x6000_0020u32.to_le_bytes());

        let mut whole = raw;
        whole.extend_from_slice(&section);
        whole.resize(RAW_AT as usize + (section.len()).next_multiple_of(0x200), 0);

        let mut built = Dll {
            raw: whole,
            strings: spots,
        };
        built.strings.sort_unstable();

        built
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reread(raw: &[u8]) -> Vec<String> {
        read_strings(raw)
            .expect("the assembly reads")
            .literals
            .into_iter()
            .map(|one| one.text)
            .collect()
    }

    #[test]
    fn an_address_belongs_to_the_one_section_that_spans_it() {
        let section = |address, virtual_size, raw_size| Section {
            head: 0,
            address,
            virtual_size,
            raw_at: 0,
            raw_size,
        };

        let text = section(0x2000, 0x1000, 0x1200);
        assert!(text.holds(0x2000), "the first byte is inside");
        assert!(text.holds(0x21FF), "and so is the last one it covers");
        assert!(
            !text.holds(0x1FFF),
            "an address below the section is not in it, and taking it would read a header as code"
        );
        assert!(
            !text.holds(0x4000),
            "an address past the section is not in it either: a lookup that answers for both \
             ends maps every rva onto the first section and the metadata is read from nowhere"
        );

        let empty = section(0x9000, 0, 0);
        assert!(
            !empty.holds(0x9000),
            "a section the linker sized at nothing spans nothing, so no address may resolve \
             through it"
        );
    }

    #[test]
    fn a_line_a_script_hard_codes_is_read_out_of_the_assembly() {
        let built = forge::dll(&["\u{53f0}\u{6240}", "MusicVolume", "Press any key"]);

        assert_eq!(
            reread(&built.raw),
            ["\u{53f0}\u{6240}", "MusicVolume", "Press any key"],
            "a string a compiler put in the user heap is text the game shows and nothing else \
             reaches it"
        );
    }

    #[test]
    fn an_instruction_carrying_a_token_is_stepped_over_whole() {
        let mut code = vec![LDSTR];
        code.extend_from_slice(&(STRING_TOKEN | 1).to_le_bytes());
        code.push(0x8C);
        code.extend_from_slice(&0x0100_0020u32.to_le_bytes());
        code.push(LDSTR);
        code.extend_from_slice(&(STRING_TOKEN | 9).to_le_bytes());
        code.push(0x2A);

        let mut found = Vec::new();
        each_ldstr(&code, 0, code.len(), |at, _| found.push(at));

        assert_eq!(
            found,
            [1, 9],
            "boxing a number is how a script builds half its sentences, and reading its type \
             token as code loses every line after it and can point a rewrite at the middle of an \
             instruction"
        );
    }

    #[test]
    fn a_longer_line_lands_in_the_assembly_and_the_rest_stays_where_it_was() {
        let built = forge::dll(&["\u{53f0}\u{6240}", "MusicVolume", "Press any key"]);
        let kitchen = built.strings[0];

        let lines = BTreeMap::from([(
            kitchen.to_string(),
            "The kitchen, and the pantry behind it".to_string(),
        )]);

        assert!(
            "The kitchen, and the pantry behind it"
                .encode_utf16()
                .count()
                > "\u{53f0}\u{6240}".encode_utf16().count(),
            "the whole point is a line that cannot be written where the old one sat, so a \
             shorter replacement would quietly stop testing anything"
        );

        let (out, pieces) = put_back(&built.raw, &lines)
            .expect("the assembly rewrites")
            .expect("something changed");
        assert_eq!(pieces, 1, "one line was staged and one line was written");

        let back = reread(&out);
        assert!(
            back.contains(&"The kitchen, and the pantry behind it".to_string()),
            "the longer line has to land somewhere the tokens can still reach"
        );
        assert!(
            back.contains(&"MusicVolume".to_string())
                && back.contains(&"Press any key".to_string()),
            "every other line keeps the offset its own ldstr still points at"
        );
        assert!(
            !back.contains(&"\u{53f0}\u{6240}".to_string()),
            "and nothing still reaches the line that was replaced"
        );
    }

    #[test]
    fn an_assembly_nobody_translated_is_left_byte_for_byte() {
        let built = forge::dll(&["\u{505c}\u{6b62}"]);
        let same = BTreeMap::from([(built.strings[0].to_string(), "\u{505c}\u{6b62}".to_string())]);

        assert!(
            put_back(&built.raw, &same).expect("it reads").is_none(),
            "writing the same words back would grow the file for nothing"
        );
        assert!(
            put_back(&built.raw, &BTreeMap::new())
                .expect("it reads")
                .is_none(),
            "and a sheet nobody filled in asks for nothing either"
        );
    }

    #[test]
    fn only_a_line_written_to_be_read_is_offered_to_a_translator() {
        for text in [
            "\u{505c}\u{6b62}",
            "\u{6226}\u{95d8}\u{56de}\u{6570}: ",
            "Pr\u{e9}-requisito:",
            "No Data",
            "Press any key to start",
        ] {
            assert!(!a_key(text), "{text} is a line a player reads");
        }

        for text in [
            "MusicVolume",
            "Slot{0}_day",
            "coin",
            "Horizontal",
            "RestartRound",
            "N0",
            "{0} / {1}",
        ] {
            assert!(
                a_key(text),
                "{text} is a key a script looks itself up by, and translating it is how a save \
                 file stops loading"
            );
        }
    }

    #[test]
    fn only_the_assemblies_a_game_writes_are_opened() {
        for name in [
            "Managed/Assembly-CSharp.dll",
            "Managed/Assembly-CSharp-firstpass.dll",
            "Managed/assembly-csharp.dll",
            "Managed/Assembly-UnityScript.dll",
        ] {
            assert!(
                ours(Path::new(name)),
                "{name} is where a Unity game keeps its own code"
            );
        }

        for name in [
            "Managed/UnityEngine.CoreModule.dll",
            "Managed/mscorlib.dll",
            "Managed/System.Xml.dll",
            "Managed/Unity.TextMeshPro.dll",
            "Managed/Assembly-CSharp.pdb",
            "Managed/Assembly-CSharpish.dll",
        ] {
            assert!(
                !ours(Path::new(name)),
                "{name} ships with the engine, and its English is the runtime talking to itself"
            );
        }
    }
}
