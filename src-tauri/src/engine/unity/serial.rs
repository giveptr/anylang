use crate::engine::unity::cursor::At;
use crate::engine::unity::shapes;
use crate::store;
use crate::store::Stamp;
use anyhow::{Context, Result, bail};
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const FONT: i32 = 128;
pub const MONO_BEHAVIOUR: i32 = 114;
pub const MONO_SCRIPT: i32 = 115;
pub const SPRITE: i32 = 213;
pub const TEXT_ASSET: i32 = 49;
pub const TEXTURE_2D: i32 = 28;

pub const HEAD: usize = 32;

pub const SIZE_AT: usize = 4;
pub const WHOLE_AT: usize = 24;
pub const WIDE: u32 = 22;

const VERSION_AT: usize = 8;
const DATA_AT: usize = 12;
const ENDIAN_AT: usize = 16;
const NARROW_PAST: usize = 20;
const WIDE_DATA_AT: usize = 32;
const WIDE_PAST: usize = 48;

const OLDEST: u32 = 17;
const HASHED: u32 = 19;
const LEANING: u32 = 21;

fn header_field(head: &[u8], at: usize, wide: usize) -> Option<u64> {
    let raw = head.get(at..at.checked_add(wide)?)?;

    Some(
        raw.iter()
            .fold(0u64, |value, byte| (value << 8) | u64::from(*byte)),
    )
}

pub fn announces_itself(head: &[u8], size: u64) -> bool {
    let Some(version) = header_field(head, VERSION_AT, 4) else {
        return false;
    };
    if version < u64::from(OLDEST) {
        return false;
    }

    let whole = match version >= u64::from(WIDE) {
        true => header_field(head, WHOLE_AT, 8),
        false => header_field(head, SIZE_AT, 4),
    };

    whole == Some(size)
}

pub enum Value {
    Number(i64),
    Real(f64),
    Bytes(Vec<u8>),
    List(Vec<Value>),
    Tree(Vec<(String, Value)>),
}

#[derive(Clone)]
enum Blob {
    Held(Arc<[u8]>),
    Away(Arc<Away>),
}

struct Away {
    at: PathBuf,
    stamp: Stamp,
}

pub struct Object {
    pub class_id: i32,
    pub path_id: i64,
    at: Range<usize>,
    blob: Blob,
    shape: Option<Arc<Node>>,
}

struct Field {
    at: Range<usize>,
    big: bool,
}

impl Field {
    fn little(at: Range<usize>) -> Self {
        Self { at, big: false }
    }

    fn big(at: Range<usize>) -> Self {
        Self { at, big: true }
    }

    fn stamp(&self, out: &mut [u8], value: usize, what: &str) -> Result<()> {
        let room = out
            .get_mut(self.at.clone())
            .ok_or_else(|| anyhow::anyhow!("the {what} field is not inside the file"))?;

        let narrow = || anyhow::anyhow!("a {what} of {value} no longer fits {} bytes", room.len());

        match (room.len(), self.big) {
            (4, false) => {
                room.copy_from_slice(&u32::try_from(value).map_err(|_| narrow())?.to_le_bytes())
            }
            (4, true) => {
                room.copy_from_slice(&u32::try_from(value).map_err(|_| narrow())?.to_be_bytes())
            }
            (8, false) => {
                room.copy_from_slice(&i64::try_from(value).map_err(|_| narrow())?.to_le_bytes())
            }
            (8, true) => {
                room.copy_from_slice(&i64::try_from(value).map_err(|_| narrow())?.to_be_bytes())
            }
            (wide, _) => bail!("a {what} field {wide} bytes wide is not one this reader wrote"),
        }

        Ok(())
    }
}

pub struct Slot {
    pub path_id: i64,
    pub start: usize,
    pub size: usize,
    at_start: Field,
    at_size: Field,
    which: usize,
}

impl Slot {
    pub fn sits_at(&self, out: &mut [u8], start: usize, size: usize) -> Result<()> {
        self.at_start.stamp(out, start, "start")?;
        self.at_size.stamp(out, size, "size")
    }
}

pub struct Layout {
    pub data_at: usize,
    pub slots: Vec<Slot>,
    pub externals: Vec<String>,
    pub built_by: String,
    at_whole: Field,
}

impl Layout {
    pub fn announce(&self, out: &mut [u8]) -> Result<()> {
        self.at_whole.stamp(out, out.len(), "file length")
    }
}

pub struct Container {
    pub name: String,
    pub objects: Vec<Object>,
    pub externals: Vec<String>,
    pub built_by: String,
}

impl Object {
    pub fn shaped(&self) -> bool {
        self.shape.is_some()
    }

    pub fn body(&self) -> Result<Cow<'_, [u8]>> {
        match &self.blob {
            Blob::Held(held) => held
                .get(self.at.clone())
                .map(Cow::Borrowed)
                .ok_or_else(|| anyhow::anyhow!("object {} sits past its own file", self.path_id)),
            Blob::Away(away) => away.read(&self.at).map(Cow::Owned),
        }
    }

    pub fn has(&self, field: &str) -> bool {
        self.shape
            .as_deref()
            .is_some_and(|node| node.kids.iter().any(|kid| kid.name == field))
    }

    pub fn value(&self) -> Option<Value> {
        read_value(self.shape.as_deref()?, &mut At::new(&self.body().ok()?)).ok()
    }

    pub fn written(&self, value: &Value) -> Result<Vec<u8>> {
        let shape = self.shape.as_deref().ok_or_else(|| {
            anyhow::anyhow!("object {} has no type tree to write by", self.path_id)
        })?;

        let was = self
            .value()
            .ok_or_else(|| anyhow::anyhow!("object {} cannot be read back", self.path_id))?;

        let body = self.body()?;
        let mut base = Vec::new();
        write_value(shape, &was, &mut base)?;

        if !body.starts_with(&base) {
            bail!(
                "object {} does not come back the way it went in, refusing to change it",
                self.path_id
            );
        }

        let mut out = Vec::with_capacity(body.len());
        write_value(shape, value, &mut out)?;
        out.extend_from_slice(&body[base.len()..]);

        Ok(out)
    }
}

#[cfg(test)]
pub const BUILT_BY: &str = "2022.3.45f1";

#[cfg(test)]
pub fn blank_of(class_id: i32) -> Option<Value> {
    Some(blank(shipped(class_id, BUILT_BY)?.as_ref()))
}

#[cfg(test)]
fn blank(node: &Node) -> Value {
    if let Some(holder) = holds_a_list(node) {
        let item = &holder.kids[1];

        return match item.size == 1 && item.kids.is_empty() {
            true => Value::Bytes(Vec::new()),
            false => Value::List(Vec::new()),
        };
    }

    if node.kids.is_empty() {
        return match (node.size, real_kind(&node.kind)) {
            (_, true) => Value::Real(0.0),
            (1 | 2 | 4 | 8, _) => Value::Number(0),
            _ => Value::Bytes(vec![0; node.size.max(0) as usize]),
        };
    }

    Value::Tree(
        node.kids
            .iter()
            .map(|kid| (kid.name.clone(), blank(kid)))
            .collect(),
    )
}

#[cfg(test)]
pub fn body_of(class_id: i32, value: &Value) -> Result<Vec<u8>> {
    let shape = shipped(class_id, BUILT_BY)
        .ok_or_else(|| anyhow::anyhow!("the pack says nothing about class {class_id}"))?;

    let mut out = Vec::new();
    write_value(&shape, value, &mut out)?;

    Ok(out)
}

#[cfg(test)]
impl Object {
    pub fn forged(class_id: i32, path_id: i64, body: Vec<u8>) -> Self {
        Self {
            shape: shipped(class_id, BUILT_BY),
            class_id,
            path_id,
            at: 0..body.len(),
            blob: Blob::Held(Arc::from(body)),
        }
    }
}

impl Value {
    pub fn field(&self, name: &str) -> Option<&Value> {
        match self {
            Value::Tree(kids) => kids.iter().find(|(had, _)| had == name).map(|(_, it)| it),
            _ => None,
        }
    }

    pub fn nth(&self, index: usize) -> Option<&Value> {
        match self {
            Value::Tree(kids) => kids.get(index).map(|(_, it)| it),
            Value::List(items) => items.get(index),
            _ => None,
        }
    }

    pub fn text(&self) -> Option<String> {
        match self {
            Value::Bytes(raw) => String::from_utf8(raw.clone()).ok(),
            _ => None,
        }
    }

    pub fn number(&self) -> Option<i64> {
        match self {
            Value::Number(it) => Some(*it),
            _ => None,
        }
    }

    pub fn real(&self) -> Option<f64> {
        match self {
            Value::Real(it) => Some(*it),
            Value::Number(it) => Some(*it as f64),
            _ => None,
        }
    }

    pub fn items(&self) -> &[Value] {
        match self {
            Value::List(items) => items,
            _ => &[],
        }
    }

    pub fn field_mut(&mut self, name: &str) -> Option<&mut Value> {
        match self {
            Value::Tree(kids) => kids
                .iter_mut()
                .find(|(had, _)| had == name)
                .map(|(_, it)| it),
            _ => None,
        }
    }

    pub fn items_mut(&mut self) -> &mut [Value] {
        match self {
            Value::List(items) => items,
            _ => &mut [],
        }
    }

    pub fn put(&mut self, name: &str, text: &str) -> bool {
        match self.field_mut(name) {
            Some(slot) => {
                *slot = Value::Bytes(text.as_bytes().to_vec());
                true
            }
            None => false,
        }
    }
}

#[derive(Clone)]
pub struct Node {
    pub kind: String,
    pub size: i32,
    pub flag: i32,
    pub listed: bool,
    pub name: String,
    pub kids: Vec<Node>,
}

impl Away {
    fn read(&self, at: &Range<usize>) -> Result<Vec<u8>> {
        if store::stamp_of(&self.at) != self.stamp {
            bail!(
                "{} changed while it was open, so read the game again",
                self.at.display()
            );
        }

        let mut file =
            File::open(&self.at).with_context(|| format!("opening {}", self.at.display()))?;
        file.seek(SeekFrom::Start(at.start as u64))
            .with_context(|| format!("reaching into {}", self.at.display()))?;

        let mut raw = vec![0u8; at.len()];
        file.read_exact(&mut raw)
            .with_context(|| format!("reading out of {}", self.at.display()))?;

        Ok(raw)
    }
}

fn told_at(head: &[u8]) -> Option<usize> {
    let data_at = match header_field(head, VERSION_AT, 4)? >= u64::from(WIDE) {
        true => header_field(head, WIDE_DATA_AT, 8)?,
        false => header_field(head, DATA_AT, 4)?,
    };

    usize::try_from(data_at).ok()
}

pub fn open_at(at: &Path, name: &str) -> Result<Container> {
    let stamp = store::stamp_of(at);
    let mut file = File::open(at).with_context(|| format!("opening {}", at.display()))?;
    let whole = file
        .metadata()
        .with_context(|| format!("measuring {}", at.display()))?
        .len() as usize;

    let mut head = [0u8; 64];
    let read = file
        .read(&mut head)
        .with_context(|| format!("reading {}", at.display()))?;
    let data_at = told_at(&head[..read])
        .filter(|held| *held <= whole)
        .ok_or_else(|| anyhow::anyhow!("{} does not say where its data starts", at.display()))?;

    let mut said = vec![0u8; data_at];
    file.seek(SeekFrom::Start(0))?;
    file.read_exact(&mut said)
        .with_context(|| format!("reading the head of {}", at.display()))?;

    let (layout, shapes) = read_from(&said, whole)?;
    let blob = Blob::Away(Arc::new(Away {
        at: at.to_path_buf(),
        stamp,
    }));

    laid_out(name, layout, shapes, blob)
}

pub fn open(blob: &[u8], name: &str) -> Result<Container> {
    open_told(blob, "", name)
}

pub fn open_told(blob: &[u8], told: &str, name: &str) -> Result<Container> {
    let (mut layout, shapes) = read_from(blob, blob.len())?;

    if unnamed(&layout.built_by) {
        layout.built_by = told.to_string();
    }

    laid_out(name, layout, shapes, Blob::Held(Arc::from(blob)))
}

pub fn unnamed(built_by: &str) -> bool {
    built_by.is_empty() || built_by.starts_with("0.0")
}

fn laid_out(name: &str, layout: Layout, shapes: Kinds, blob: Blob) -> Result<Container> {
    let externals = layout.externals;
    let built_by = layout.built_by;
    let mut bundled: BTreeMap<i32, Option<Arc<Node>>> = BTreeMap::new();

    let objects = layout
        .slots
        .into_iter()
        .map(|slot| {
            let (class_id, shape) = shapes
                .get(slot.which)
                .map(|(class_id, shape)| (*class_id, shape.clone()))
                .ok_or_else(|| {
                    anyhow::anyhow!("object points at type {}, which is not there", slot.which)
                })?;

            let shape = shape.or_else(|| {
                bundled
                    .entry(class_id)
                    .or_insert_with(|| shipped(class_id, &built_by))
                    .clone()
            });

            let from = layout.data_at + slot.start;

            Ok(Object {
                class_id,
                path_id: slot.path_id,
                at: from..from + slot.size,
                blob: blob.clone(),
                shape,
            })
        })
        .collect::<Result<Vec<Object>>>()?;

    Ok(Container {
        name: name.to_string(),
        objects,
        externals,
        built_by,
    })
}

fn shipped(class_id: i32, built_by: &str) -> Option<Arc<Node>> {
    if class_id == MONO_BEHAVIOUR {
        return None;
    }

    shapes::told(class_id, built_by)
}

pub fn layout(blob: &[u8]) -> Result<Layout> {
    Ok(read_from(blob, blob.len())?.0)
}

type Kinds = Vec<(i32, Option<Arc<Node>>)>;

fn read_from(blob: &[u8], whole_is: usize) -> Result<(Layout, Kinds)> {
    let field = |at: usize, wide: usize| {
        header_field(blob, at, wide)
            .ok_or_else(|| anyhow::anyhow!("this serialized file ends inside its own header"))
    };

    let version = field(VERSION_AT, 4)? as u32;
    if version < OLDEST {
        bail!("serialized file version {version} is older than this reader handles");
    }

    if *blob
        .get(ENDIAN_AT)
        .ok_or_else(|| anyhow::anyhow!("this serialized file ends inside its own header"))?
        != 0
    {
        bail!("this serialized file is big-endian, which this reader does not handle");
    }

    let (whole, data_at, at_whole, head) = match version >= WIDE {
        true => (
            field(WHOLE_AT, 8)? as usize,
            field(WIDE_DATA_AT, 8)? as usize,
            Field::big(WHOLE_AT..WHOLE_AT + 8),
            WIDE_PAST,
        ),
        false => (
            field(SIZE_AT, 4)? as usize,
            field(DATA_AT, 4)? as usize,
            Field::big(SIZE_AT..SIZE_AT + 4),
            NARROW_PAST,
        ),
    };

    let mut at = At::new(blob);
    at.take(head)?;

    if whole != whole_is {
        bail!("serialized file says it is {whole} bytes but is {whole_is}");
    }

    if data_at > whole_is {
        bail!("serialized file says its data starts at {data_at}, past its own end");
    }

    let built_by = at.zero_ended()?;
    at.i32()?;
    let has_tree = at.byte()? != 0;

    let mut shapes = Vec::new();
    let kinds = at.i32()?;
    for _ in 0..kinds {
        shapes.push(read_shape(&mut at, has_tree, version, &built_by)?);
    }

    let mut slots = Vec::new();
    let count = at.i32()?;
    for _ in 0..count {
        at.align(4);

        let path_id = at.i64()?;

        let from = at.seen;
        let start = match version >= WIDE {
            true => at.i64()?,
            false => i64::from(at.u32()?),
        };
        let at_start = Field::little(from..at.seen);

        let from = at.seen;
        let size = at.u32()? as usize;
        let at_size = Field::little(from..at.seen);

        let which = at.i32()?;

        let (Ok(start), Ok(which)) = (usize::try_from(start), usize::try_from(which)) else {
            bail!("object {path_id} sits at an offset no file could hold");
        };

        let past = data_at
            .checked_add(start)
            .and_then(|from| from.checked_add(size))
            .ok_or_else(|| anyhow::anyhow!("object {path_id} claims a size no file could hold"))?;

        if past > whole_is {
            bail!("object {path_id} reaches past the file");
        }

        slots.push(Slot {
            path_id,
            start,
            size,
            at_start,
            at_size,
            which,
        });
    }

    let externals = read_externals(&mut at).unwrap_or_default();

    Ok((
        Layout {
            data_at,
            slots,
            externals,
            built_by,
            at_whole,
        },
        shapes,
    ))
}

fn read_externals(at: &mut At<'_>) -> Result<Vec<String>> {
    let named = at.i32()?;
    for _ in 0..named {
        at.i32()?;
        at.align(4);
        at.i64()?;
    }

    let count = at.i32()?;
    let mut out = Vec::new();
    for _ in 0..count {
        at.zero_ended()?;
        at.take(16)?;
        at.i32()?;
        out.push(at.zero_ended()?);
    }

    Ok(out)
}

fn read_shape(
    at: &mut At<'_>,
    has_tree: bool,
    version: u32,
    built_by: &str,
) -> Result<(i32, Option<Arc<Node>>)> {
    let class_id = at.i32()?;
    at.take(3)?;

    if class_id == MONO_BEHAVIOUR {
        at.take(16)?;
    }
    at.take(16)?;

    if !has_tree {
        return Ok((class_id, None));
    }

    let (Ok(nodes), Ok(letters)) = (usize::try_from(at.i32()?), usize::try_from(at.i32()?)) else {
        bail!("a type tree claims a node count no file could hold");
    };

    let mut raw = Vec::with_capacity(nodes.min(4096));
    for _ in 0..nodes {
        at.u16()?;
        let level = at.byte()? as usize;
        let listed = at.byte()? & 1 == 1;
        let kind_at = at.u32()? as usize;
        let name_at = at.u32()? as usize;
        let size = at.i32()?;
        at.i32()?;
        let flag = at.i32()?;
        if version >= HASHED {
            at.i64()?;
        }

        raw.push((level, size, flag, listed, kind_at, name_at));
    }

    let letters = at.take(letters)?;

    if version >= LEANING {
        let leans = at.i32()?;
        let wide = usize::try_from(leans)
            .ok()
            .and_then(|many| many.checked_mul(4))
            .ok_or_else(|| anyhow::anyhow!("a type tree claims {leans} dependencies"))?;
        at.take(wide)?;
    }

    let name_of = |spot: usize| -> String {
        if spot & 0x8000_0000 != 0 {
            return shapes::common(spot & 0x7FFF_FFFF, built_by)
                .unwrap_or_default()
                .to_string();
        }

        let rest = &letters[spot.min(letters.len())..];
        let end = rest.iter().position(|byte| *byte == 0).unwrap_or(0);

        String::from_utf8_lossy(&rest[..end]).into_owned()
    };

    let flat: Vec<Flat> = raw
        .into_iter()
        .map(|(level, size, flag, listed, kind_at, name_at)| Flat {
            level,
            size,
            flag,
            listed,
            kind: name_of(kind_at),
            name: name_of(name_at),
        })
        .collect();

    let mut seen = 0;
    let root = grow(&flat, &mut seen, 0)?;

    Ok((class_id, Some(Arc::new(root))))
}

struct Flat {
    level: usize,
    size: i32,
    flag: i32,
    listed: bool,
    kind: String,
    name: String,
}

fn grow(flat: &[Flat], seen: &mut usize, level: usize) -> Result<Node> {
    let Some(held) = flat.get(*seen) else {
        bail!("a type tree ends before its nodes do");
    };

    let mut node = Node {
        kind: held.kind.clone(),
        size: held.size,
        flag: held.flag,
        listed: held.listed,
        name: held.name.clone(),
        kids: Vec::new(),
    };
    *seen += 1;

    while flat.get(*seen).is_some_and(|held| held.level > level) {
        node.kids.push(grow(flat, seen, level + 1)?);
    }

    Ok(node)
}

fn real_kind(kind: &str) -> bool {
    matches!(kind, "float" | "double")
}

fn holds_a_list(node: &Node) -> Option<&Node> {
    if node.listed && node.kids.len() == 2 {
        return Some(node);
    }

    node.kids
        .first()
        .filter(|held| held.listed && held.kids.len() == 2)
}

fn read_value(node: &Node, at: &mut At<'_>) -> Result<Value> {
    if node.kids.is_empty() {
        let raw = at.take(node.size.max(0) as usize)?;
        let out = match (node.size, real_kind(&node.kind)) {
            (4, true) => Value::Real(f64::from(f32::from_le_bytes(raw.try_into()?))),
            (8, true) => Value::Real(f64::from_le_bytes(raw.try_into()?)),
            (1, _) => Value::Number(i64::from(raw[0])),
            (2, _) => Value::Number(i64::from(i16::from_le_bytes(raw.try_into()?))),
            (4, _) => Value::Number(i64::from(i32::from_le_bytes(raw.try_into()?))),
            (8, _) => Value::Number(i64::from_le_bytes(raw.try_into()?)),
            _ => Value::Bytes(raw.to_vec()),
        };

        if node.flag & 0x4000 != 0 {
            at.align(4);
        }

        return Ok(out);
    }

    if let Some(holder) = holds_a_list(node) {
        let count = at.i32()? as usize;
        if count > at.raw.len().saturating_sub(at.seen) {
            bail!("a list claims {count} items the file cannot hold");
        }

        let item = &holder.kids[1];

        let out = if item.size == 1 && item.kids.is_empty() {
            Value::Bytes(at.take(count)?.to_vec())
        } else {
            let mut items = Vec::with_capacity(count.min(4096));
            for _ in 0..count {
                items.push(read_value(item, at)?);
            }
            Value::List(items)
        };

        if holder.flag & 0x4000 != 0 {
            at.align(4);
        }
        if node.flag & 0x4000 != 0 {
            at.align(4);
        }

        return Ok(out);
    }

    let mut kids = Vec::with_capacity(node.kids.len());
    for kid in &node.kids {
        kids.push((kid.name.clone(), read_value(kid, at)?));
    }

    if node.flag & 0x4000 != 0 {
        at.align(4);
    }

    Ok(Value::Tree(kids))
}

fn pad(out: &mut Vec<u8>) {
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}

fn write_value(node: &Node, value: &Value, out: &mut Vec<u8>) -> Result<()> {
    if node.kids.is_empty() {
        match (node.size, value) {
            (4, Value::Real(it)) => out.extend_from_slice(&(*it as f32).to_le_bytes()),
            (8, Value::Real(it)) => out.extend_from_slice(&it.to_le_bytes()),
            (1, Value::Number(it)) => out.push(*it as u8),
            (2, Value::Number(it)) => out.extend_from_slice(&(*it as i16).to_le_bytes()),
            (4, Value::Number(it)) => out.extend_from_slice(&(*it as i32).to_le_bytes()),
            (8, Value::Number(it)) => out.extend_from_slice(&it.to_le_bytes()),
            (_, Value::Bytes(raw)) => out.extend_from_slice(raw),
            _ => bail!("{} does not hold what its type asks for", node.name),
        }

        if node.flag & 0x4000 != 0 {
            pad(out);
        }

        return Ok(());
    }

    if let Some(holder) = holds_a_list(node) {
        let item = &holder.kids[1];

        match value {
            Value::Bytes(raw) => {
                out.extend_from_slice(&(raw.len() as i32).to_le_bytes());
                out.extend_from_slice(raw);
            }
            Value::List(items) => {
                out.extend_from_slice(&(items.len() as i32).to_le_bytes());
                for one in items {
                    write_value(item, one, out)?;
                }
            }
            _ => bail!("{} is a list but does not hold one", node.name),
        }

        if holder.flag & 0x4000 != 0 {
            pad(out);
        }
        if node.flag & 0x4000 != 0 {
            pad(out);
        }

        return Ok(());
    }

    let Value::Tree(kids) = value else {
        bail!("{} is a struct but does not hold one", node.name)
    };

    if kids.len() != node.kids.len() {
        bail!(
            "{} has {} field(s) but {} were given",
            node.name,
            node.kids.len(),
            kids.len()
        );
    }

    for (kid, (_, value)) in node.kids.iter().zip(kids) {
        write_value(kid, value, out)?;
    }

    if node.flag & 0x4000 != 0 {
        pad(out);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::unity::fake;
    use std::fs;

    #[test]
    fn a_container_read_from_the_file_is_the_one_read_from_its_bytes() {
        let raw = fake::forge(&[(11, "one", "hello"), (12, "two", "there")]);
        let sandbox = tempfile::tempdir().expect("a temp folder");
        let at = sandbox.path().join("resources.assets");
        fs::write(&at, &raw).expect("the container");

        let away = open_at(&at, "").expect("it opens from the file");
        let held = open(&raw, "").expect("it opens from its bytes");

        assert_eq!(
            away.objects.len(),
            held.objects.len(),
            "reading the table off the disk has to find the same objects"
        );

        for (one, two) in away.objects.iter().zip(held.objects.iter()) {
            assert_eq!((one.class_id, one.path_id), (two.class_id, two.path_id));
            assert_eq!(
                one.body().expect("read from the file"),
                two.body().expect("read from the bytes"),
                "an object read back off the disk has to be the very bytes it was, or a picture \
                 comes out of the wrong place in the file"
            );
        }

        fs::write(&at, b"something else entirely").expect("they change it");
        assert!(
            away.objects[0].body().is_err(),
            "once the file has changed under it, reading by offset would hand back other bytes, \
             so it has to refuse instead"
        );
    }

    #[test]
    fn the_deepest_a_type_tree_can_nest_is_what_one_byte_of_level_allows() {
        let flat: Vec<Flat> = (0..=u8::MAX as usize)
            .map(|level| Flat {
                level,
                size: 4,
                flag: 0,
                listed: false,
                kind: "int".to_string(),
                name: format!("deep{level}"),
            })
            .collect();

        let mut seen = 0;
        let held = grow(&flat, &mut seen, 0).expect("the deepest tree the format can spell");

        let mut walk = &held;
        let mut deep = 0;
        while let Some(kid) = walk.kids.first() {
            walk = kid;
            deep += 1;
        }

        assert_eq!(
            deep,
            u8::MAX as usize,
            "read_shape takes each level from a single byte, so the longest chain a file can ask \
             for is 256 frames of grow and no capping of its own is needed: adding one could only \
             turn a real game's tree away, and containers_in swallows that error, so the whole \
             container would go missing with nobody told"
        );
    }

    #[test]
    fn the_externals_list_gives_the_names_cross_container_scripts_resolve_by() {
        let mut raw = Vec::new();
        raw.extend_from_slice(&1i32.to_le_bytes());
        raw.extend_from_slice(&0i32.to_le_bytes());
        raw.extend_from_slice(&7i64.to_le_bytes());
        raw.extend_from_slice(&2i32.to_le_bytes());
        for name in ["sharedassets0.assets", "globalgamemanagers.assets"] {
            raw.push(0);
            raw.extend_from_slice(&[0u8; 16]);
            raw.extend_from_slice(&0i32.to_le_bytes());
            raw.extend_from_slice(name.as_bytes());
            raw.push(0);
        }

        assert_eq!(
            read_externals(&mut At::new(&raw)).unwrap(),
            ["sharedassets0.assets", "globalgamemanagers.assets"],
            "owner_of counts on this order staying exactly as written"
        );
    }

    fn forge_with(start: i64, size: u32) -> Vec<u8> {
        let mut meta = Vec::new();
        meta.extend_from_slice(b"2021.3.8f1\0");
        meta.extend_from_slice(&2i32.to_le_bytes());
        meta.push(0);
        meta.extend_from_slice(&1i32.to_le_bytes());

        meta.extend_from_slice(&49i32.to_le_bytes());
        meta.push(0);
        meta.extend_from_slice(&(-1i16).to_le_bytes());
        meta.extend_from_slice(&[0; 16]);

        meta.extend_from_slice(&1i32.to_le_bytes());
        while !meta.len().is_multiple_of(4) {
            meta.push(0);
        }
        meta.extend_from_slice(&7i64.to_le_bytes());
        meta.extend_from_slice(&start.to_le_bytes());
        meta.extend_from_slice(&size.to_le_bytes());
        meta.extend_from_slice(&0i32.to_le_bytes());

        let data_at = (48 + meta.len() + 15) & !15;
        let mut out = vec![0u8; 48];
        out[8..12].copy_from_slice(&22u32.to_be_bytes());
        out[20..24].copy_from_slice(&(meta.len() as u32).to_be_bytes());
        out[32..40].copy_from_slice(&(data_at as i64).to_be_bytes());
        out.extend_from_slice(&meta);
        out.resize(data_at + 64, 0);

        let whole = out.len() as i64;
        out[24..32].copy_from_slice(&whole.to_be_bytes());

        out
    }

    #[test]
    fn an_offset_no_file_could_hold_is_refused_instead_of_panicking() {
        assert!(
            open(&forge_with(16, 8), "").is_ok(),
            "an honest object still reads"
        );

        for (start, size) in [(-1i64, 8u32), (i64::MIN, 8), (8, u32::MAX), (i64::MAX, 8)] {
            assert!(
                open(&forge_with(start, size), "").is_err(),
                "start {start} size {size} must come back as an error, never a panic"
            );
        }
    }

    #[test]
    fn a_container_announces_itself_in_its_first_bytes() {
        let mut head = vec![0u8; HEAD];
        head[VERSION_AT..VERSION_AT + 4].copy_from_slice(&22u32.to_be_bytes());
        head[WHOLE_AT..WHOLE_AT + 8].copy_from_slice(&50_293_680i64.to_be_bytes());

        assert!(
            announces_itself(&head, 50_293_680),
            "a serialized file carries its own length, which is what names it"
        );
        assert!(
            !announces_itself(&head, 50_293_681),
            "a file whose length does not match its header is not one"
        );

        let mut older = vec![0u8; HEAD];
        older[VERSION_AT..VERSION_AT + 4].copy_from_slice(&OLDEST.to_be_bytes());
        older[SIZE_AT..SIZE_AT + 4].copy_from_slice(&50_293_680u32.to_be_bytes());

        assert!(
            announces_itself(&older, 50_293_680),
            "a build from before large files keeps that length four bytes wide, in another field, \
             and the wide field is still zero here"
        );
        assert!(!announces_itself(&older, 50_293_681));

        older[VERSION_AT..VERSION_AT + 4].copy_from_slice(&(OLDEST - 1).to_be_bytes());
        assert!(!announces_itself(&older, 50_293_680), "too old to read");

        assert!(
            !announces_itself(b"just some text", 14),
            "nothing that is not a container may slip through"
        );
        assert!(!announces_itself(&[], 0));
    }

    #[test]
    fn a_file_that_is_too_old_is_refused_out_loud() {
        let mut old = vec![0u8; 64];
        old[VERSION_AT..VERSION_AT + 4].copy_from_slice(&(OLDEST - 1).to_be_bytes());

        assert!(
            open(&old, "").is_err(),
            "an older build lays its header out differently, so reading it by today's widths \
             would hand back objects pointing anywhere in the file"
        );
        assert!(open(b"", "").is_err());
    }

    #[test]
    fn the_file_version_moves_the_numbers_without_changing_what_comes_out() {
        let assets: &[(i64, &str, &str)] = &[
            (11, "one", "Peter\nShe tilted her head.\n\n"),
            (22, "two", "Mary\n待って。\n\n"),
        ];

        let narrow = fake::forge_as(OLDEST, assets);
        let wide = fake::forge_as(WIDE, assets);

        assert_ne!(
            narrow.len(),
            wide.len(),
            "the older build writes a shorter header and narrower entries, so a fixture that \
             ignored the version would prove nothing"
        );

        let listed = |blob: &[u8]| -> Vec<(i32, i64, Vec<u8>)> {
            open(blob, "")
                .expect("a container")
                .objects
                .iter()
                .map(|one| {
                    (
                        one.class_id,
                        one.path_id,
                        one.body().expect("its body").into_owned(),
                    )
                })
                .collect()
        };

        assert_eq!(listed(&narrow), listed(&wide));
    }

    #[test]
    fn the_common_string_table_answers_at_the_offsets_unity_writes() {
        let at = |spot: usize, built_by: &str| shapes::common(spot, built_by).unwrap_or_default();

        assert_eq!(at(0, "2022.3.45f1"), "AABB");
        assert_eq!(at(49, "2022.3.45f1"), "Array");
        assert_eq!(at(106, "2022.3.45f1"), "data");
        assert_eq!(at(427, "2022.3.45f1"), "m_Name");
        assert_eq!(at(490, "2022.3.45f1"), "m_Script");
        assert_eq!(
            at(741, "2022.3.45f1"),
            "Quaternionf",
            "a table written out by hand went stale here and every name past this offset came \
              back as the word after it, which cost one real game 1088 pictures"
        );
        assert_eq!(at(795, "2022.3.45f1"), "size");
        assert_eq!(at(840, "2022.3.45f1"), "string");
        assert_eq!(at(427, "9999.1.0f1"), "m_Name");
        assert_eq!(
            at(1200, "2022.3.74f1"),
            "EntityId",
            "the last word in the table is the one that says the table was read whole"
        );
        assert_eq!(
            at(1209, "6000.5.0a8"),
            "LoadableReference",
            "the word 6000.5 wrote at this offset"
        );
        assert_eq!(
            at(1209, "6000.6.0a6"),
            "LoadableObjectId",
            "6000.6 rewrote the tail of the table, the first offset Unity ever gave a second \
             meaning, so the table has to follow the version the game was built by"
        );
    }
}
