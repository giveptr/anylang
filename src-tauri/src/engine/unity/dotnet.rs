use crate::engine::unity::assembly::Pe;
use anyhow::{Context, Result, bail};
use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::{fs, str};

const TYPE_REF: usize = 0x01;
const TYPE_DEF: usize = 0x02;
const TYPE_SPEC: usize = 0x1B;
const FIELD: usize = 0x04;
const METHOD_DEF: usize = 0x06;
const MEMBER_REF: usize = 0x0A;
const CUSTOM_ATTRIBUTE: usize = 0x0C;
const NESTED_CLASS: usize = 0x29;

const SERIALIZE_FIELD: &str = "UnityEngine.SerializeField";
const SERIALIZE_REFERENCE: &str = "UnityEngine.SerializeReference";

const GENERIC: u8 = 0x15;
const DEEPEST: usize = 8;

fn word_at(raw: &[u8], at: usize) -> Result<u32> {
    let held = raw
        .get(at..at.saturating_add(4))
        .ok_or_else(|| anyhow::anyhow!("this assembly ends before byte {at}"))?;

    Ok(u32::from_le_bytes(held.try_into()?))
}

fn long_at(raw: &[u8], at: usize) -> Result<u64> {
    let held = raw
        .get(at..at.saturating_add(8))
        .ok_or_else(|| anyhow::anyhow!("this assembly ends before byte {at}"))?;

    Ok(u64::from_le_bytes(held.try_into()?))
}

const FIELD_STATIC: u32 = 0x0010;
const FIELD_INIT_ONLY: u32 = 0x0020;
const FIELD_LITERAL: u32 = 0x0040;
const FIELD_NOT_SERIALIZED: u32 = 0x0080;
const FIELD_PUBLIC: u32 = 0x0006;
const FIELD_ACCESS: u32 = 0x0007;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Column {
    Byte,
    Short,
    Int,
    Text,
    Blob,
    Guid,
    Row(usize),
    Coded(Coded),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Coded {
    TypeDefOrRef,
    HasConstant,
    HasCustomAttribute,
    HasFieldMarshal,
    HasDeclSecurity,
    MemberRefParent,
    HasSemantics,
    MethodDefOrRef,
    MemberForwarded,
    Implementation,
    CustomAttributeType,
    ResolutionScope,
    TypeOrMethodDef,
}

impl Coded {
    fn tables(self) -> &'static [usize] {
        match self {
            Coded::TypeDefOrRef => &[0x02, 0x01, 0x1B],
            Coded::HasConstant => &[0x04, 0x08, 0x17],
            Coded::HasCustomAttribute => &[
                0x06, 0x04, 0x01, 0x02, 0x08, 0x09, 0x0A, 0x00, 0x0E, 0x17, 0x14, 0x11, 0x1A, 0x1B,
                0x20, 0x23, 0x26, 0x27, 0x28, 0x2A, 0x2C, 0x2B,
            ],
            Coded::HasFieldMarshal => &[0x04, 0x08],
            Coded::HasDeclSecurity => &[0x02, 0x06, 0x20],
            Coded::MemberRefParent => &[0x02, 0x01, 0x1A, 0x06, 0x1B],
            Coded::HasSemantics => &[0x14, 0x17],
            Coded::MethodDefOrRef => &[0x06, 0x0A],
            Coded::MemberForwarded => &[0x04, 0x06],
            Coded::Implementation => &[0x26, 0x23, 0x27],
            Coded::CustomAttributeType => &[0xFF, 0xFF, 0x06, 0x0A, 0xFF],
            Coded::ResolutionScope => &[0x00, 0x1A, 0x23, 0x01],
            Coded::TypeOrMethodDef => &[0x02, 0x06],
        }
    }

    fn bits(self) -> u32 {
        let many = self.tables().len() as u32;
        (many - 1).max(1).ilog2() + 1
    }
}

fn schema(table: usize) -> &'static [Column] {
    use Column::{Blob, Byte, Guid, Int, Row, Short, Text};

    match table {
        0x00 => &[Short, Text, Guid, Guid, Guid],
        0x01 => &[Column::Coded(Coded::ResolutionScope), Text, Text],
        0x02 => &[
            Int,
            Text,
            Text,
            Column::Coded(Coded::TypeDefOrRef),
            Row(0x04),
            Row(0x06),
        ],
        0x03 => &[Row(0x04)],
        0x04 => &[Short, Text, Blob],
        0x05 => &[Row(0x06)],
        0x06 => &[Int, Short, Short, Text, Blob, Row(0x08)],
        0x07 => &[Row(0x08)],
        0x08 => &[Short, Short, Text],
        0x09 => &[Row(0x02), Column::Coded(Coded::TypeDefOrRef)],
        0x0A => &[Column::Coded(Coded::MemberRefParent), Text, Blob],
        0x0B => &[Byte, Byte, Column::Coded(Coded::HasConstant), Blob],
        0x0C => &[
            Column::Coded(Coded::HasCustomAttribute),
            Column::Coded(Coded::CustomAttributeType),
            Blob,
        ],
        0x0D => &[Column::Coded(Coded::HasFieldMarshal), Blob],
        0x0E => &[Short, Column::Coded(Coded::HasDeclSecurity), Blob],
        0x0F => &[Short, Int, Row(0x02)],
        0x10 => &[Int, Row(0x04)],
        0x11 => &[Blob],
        0x12 => &[Row(0x02), Row(0x14)],
        0x14 => &[Short, Text, Column::Coded(Coded::TypeDefOrRef)],
        0x15 => &[Row(0x02), Row(0x17)],
        0x17 => &[Short, Text, Blob],
        0x18 => &[Short, Row(0x06), Column::Coded(Coded::HasSemantics)],
        0x19 => &[
            Row(0x02),
            Column::Coded(Coded::MethodDefOrRef),
            Column::Coded(Coded::MethodDefOrRef),
        ],
        0x1A => &[Text],
        0x1B => &[Blob],
        0x1C => &[
            Short,
            Column::Coded(Coded::MemberForwarded),
            Text,
            Row(0x1A),
        ],
        0x1D => &[Int, Row(0x04)],
        0x20 => &[Int, Short, Short, Short, Short, Int, Blob, Text, Text],
        0x23 => &[Short, Short, Short, Short, Int, Blob, Text, Text, Blob],
        0x26 => &[Int, Text, Blob],
        0x27 => &[Int, Int, Text, Text, Column::Coded(Coded::Implementation)],
        0x28 => &[Int, Int, Text, Column::Coded(Coded::Implementation)],
        0x29 => &[Row(0x02), Row(0x02)],
        0x2A => &[Short, Short, Column::Coded(Coded::TypeOrMethodDef), Text],
        0x2B => &[Column::Coded(Coded::MethodDefOrRef), Blob],
        0x2C => &[Row(0x2A), Column::Coded(Coded::TypeDefOrRef)],
        _ => &[],
    }
}

pub struct Image<'r> {
    raw: &'r [u8],
    strings: usize,
    blobs: usize,
    rows: [u32; 64],
    starts: [usize; 64],
    widths: [usize; 64],
    text_wide: bool,
    blob_wide: bool,
    guid_wide: bool,
}

impl<'r> Image<'r> {
    pub fn read(raw: &'r [u8]) -> Result<Self> {
        Self::inside(raw, &Pe::read(raw)?)
    }

    pub fn inside(raw: &'r [u8], pe: &Pe) -> Result<Self> {
        let held = |name: &str| pe.stream(name).map(|one| pe.meta + one.at as usize);
        let tables = held("#~")
            .or_else(|| held("#-"))
            .context("this assembly has no table stream")?;

        let mut image = Self {
            raw,
            strings: held("#Strings").unwrap_or(0),
            blobs: held("#Blob").unwrap_or(0),
            rows: [0; 64],
            starts: [0; 64],
            widths: [0; 64],
            text_wide: false,
            blob_wide: false,
            guid_wide: false,
        };
        image.read_tables(tables)?;

        Ok(image)
    }

    fn read_tables(&mut self, at: usize) -> Result<()> {
        let heaps = *self
            .raw
            .get(at + 6)
            .ok_or_else(|| anyhow::anyhow!("the table stream ends early"))?;
        self.text_wide = heaps & 1 != 0;
        self.guid_wide = heaps & 2 != 0;
        self.blob_wide = heaps & 4 != 0;

        let valid = long_at(self.raw, at + 8)?;
        let mut walk = at + 24;

        for table in 0..64 {
            if valid >> table & 1 == 1 {
                self.rows[table] = word_at(self.raw, walk)?;
                walk += 4;
            }
        }

        for table in 0..64 {
            self.widths[table] = schema(table)
                .iter()
                .map(|column| self.size_of(*column))
                .sum();
        }

        if [0x03, 0x05, 0x07].iter().any(|ptr| self.rows[*ptr] != 0) {
            bail!("this assembly indirects its member tables, which this reader cannot follow");
        }

        for table in 0..64 {
            if self.rows[table] == 0 {
                continue;
            }
            if schema(table).is_empty() {
                bail!("table {table:#x} is present but this reader has no shape for it");
            }

            self.starts[table] = walk;
            walk += self.rows[table] as usize * self.widths[table];
        }

        Ok(())
    }

    fn size_of(&self, column: Column) -> usize {
        match column {
            Column::Byte => 1,
            Column::Short => 2,
            Column::Int => 4,
            Column::Text => wide(self.text_wide),
            Column::Blob => wide(self.blob_wide),
            Column::Guid => wide(self.guid_wide),
            Column::Row(table) => {
                if self.rows[table] < 0x10000 {
                    2
                } else {
                    4
                }
            }
            Column::Coded(kind) => {
                let bits = kind.bits();
                let biggest = kind
                    .tables()
                    .iter()
                    .filter(|table| **table != 0xFF)
                    .map(|table| self.rows[*table])
                    .max()
                    .unwrap_or(0);

                if u64::from(biggest) < 1u64 << (16 - bits) {
                    2
                } else {
                    4
                }
            }
        }
    }

    pub fn count(&self, table: usize) -> usize {
        self.rows[table] as usize
    }

    pub fn cell(&self, table: usize, row: usize, column: usize) -> u32 {
        let columns = schema(table);
        let mut at = self.starts[table] + row * self.widths[table];

        for one in &columns[..column] {
            at += self.size_of(*one);
        }

        let size = self.size_of(columns[column]);
        let mut value = 0u32;
        for step in 0..size {
            let Some(byte) = self.raw.get(at.saturating_add(step)) else {
                return 0;
            };

            value |= u32::from(*byte) << (8 * step);
        }

        value
    }

    pub fn tagged(&self, table: usize, row: usize, column: usize) -> (usize, usize) {
        let Column::Coded(kind) = schema(table)[column] else {
            return (0, 0);
        };

        let value = self.cell(table, row, column);
        let bits = kind.bits();
        let tag = (value & ((1 << bits) - 1)) as usize;

        (
            kind.tables().get(tag).copied().unwrap_or(0xFF),
            (value >> bits) as usize,
        )
    }

    pub fn text(&self, at: u32) -> &str {
        let from = self.strings.saturating_add(at as usize);
        let Some(rest) = self.raw.get(from..) else {
            return "";
        };

        let end = rest
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(rest.len());

        str::from_utf8(&rest[..end]).unwrap_or("")
    }

    pub fn blob(&self, at: u32) -> &[u8] {
        let mut walk = self.blobs + at as usize;
        let Some((size, step)) = unpacked(self.raw, walk) else {
            return &[];
        };
        walk += step;

        self.raw.get(walk..walk + size).unwrap_or_default()
    }
}

fn wide(yes: bool) -> usize {
    if yes { 4 } else { 2 }
}

pub fn unpacked(raw: &[u8], at: usize) -> Option<(usize, usize)> {
    let first = *raw.get(at)? as usize;

    if first & 0x80 == 0 {
        return Some((first, 1));
    }
    if first & 0xC0 == 0x80 {
        return Some((((first & 0x3F) << 8) | *raw.get(at + 1)? as usize, 2));
    }

    Some((
        ((first & 0x1F) << 24)
            | (*raw.get(at + 1)? as usize) << 16
            | (*raw.get(at + 2)? as usize) << 8
            | *raw.get(at + 3)? as usize,
        4,
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shape {
    Bool,
    Reference,
    Byte,
    Short,
    Int,
    Long,
    Float,
    Double,
    Text,
    Named(String),
    List(Box<Shape>),
    Unknown,
}

#[derive(Debug, Clone)]
pub struct Field {
    pub name: String,
    pub shape: Shape,
}

pub struct Class {
    pub base: String,
    pub fields: Vec<Field>,
    pub enumeration: Option<Shape>,
    pub serializable: bool,
}

#[derive(Default)]
pub struct Assemblies {
    classes: BTreeMap<String, Class>,
}

pub fn full_name(space: &str, name: &str) -> String {
    if space.is_empty() {
        name.to_string()
    } else {
        format!("{space}.{name}")
    }
}

impl Assemblies {
    pub fn read(managed: &Path) -> Self {
        let mut classes = BTreeMap::new();

        let Ok(listing) = fs::read_dir(managed) else {
            return Self { classes };
        };

        for entry in listing.filter_map(Result::ok) {
            let at = entry.path();
            if at.extension().is_none_or(|kind| kind != "dll") {
                continue;
            }

            let Ok(raw) = fs::read(&at) else {
                continue;
            };

            let Ok(image) = Image::read(&raw) else {
                continue;
            };

            let mut found = BTreeMap::new();
            gather(&image, &mut found);

            for (name, one) in found {
                classes.entry(name).or_insert(one);
            }
        }

        Self { classes }
    }

    pub fn named(&self, full: &str) -> Option<&Class> {
        self.classes.get(full)
    }

    #[cfg(test)]
    pub fn forged(classes: Vec<(&str, Class)>) -> Self {
        Self {
            classes: classes
                .into_iter()
                .map(|(name, one)| (name.to_string(), one))
                .collect(),
        }
    }
}

fn gather(image: &Image, out: &mut BTreeMap<String, Class>) {
    let (marks, pointed) = serialize_marks(image);
    let nests = nesting(image);
    let rows = image.count(TYPE_DEF);

    for row in 0..rows {
        let name = named_at(image, TYPE_DEF, row, &nests);
        let marked = image.cell(TYPE_DEF, row, 0);
        let (base_table, base_row) = image.tagged(TYPE_DEF, row, 3);

        let base = match base_table {
            TYPE_DEF if base_row > 0 => named_at(image, TYPE_DEF, base_row - 1, &nests),
            TYPE_REF if base_row > 0 => named_at(image, TYPE_REF, base_row - 1, &nests),
            TYPE_SPEC if base_row > 0 => filled_in(image, base_row - 1, &nests),
            _ => String::new(),
        };

        let first = image.cell(TYPE_DEF, row, 4) as usize;
        let past = if row + 1 < rows {
            image.cell(TYPE_DEF, row + 1, 4) as usize
        } else {
            image.count(FIELD) + 1
        };

        let mut fields = Vec::new();
        let mut enumeration = None;

        for which in first..past {
            if which == 0 || which > image.count(FIELD) {
                continue;
            }

            let flags = image.cell(FIELD, which - 1, 0);
            let name = image.text(image.cell(FIELD, which - 1, 1)).to_string();
            let shape = shape_of(image, image.blob(image.cell(FIELD, which - 1, 2)), &nests);

            if base == "System.Enum" {
                if name == "value__" {
                    enumeration = Some(shape);
                }
                continue;
            }

            let stored = flags & FIELD_STATIC == 0
                && flags & FIELD_INIT_ONLY == 0
                && flags & FIELD_LITERAL == 0
                && flags & FIELD_NOT_SERIALIZED == 0
                && (flags & FIELD_ACCESS == FIELD_PUBLIC
                    || marks.contains(&which)
                    || pointed.contains(&which));

            if stored {
                let shape = match pointed.contains(&which) {
                    true => by_reference(shape),
                    false => shape,
                };

                fields.push(Field { name, shape });
            }
        }

        out.entry(name).or_insert(Class {
            base,
            fields,
            enumeration,
            serializable: marked & 0x2000 != 0,
        });
    }
}

fn nesting(image: &Image) -> BTreeMap<usize, usize> {
    (0..image.count(NESTED_CLASS))
        .map(|row| {
            (
                image.cell(NESTED_CLASS, row, 0) as usize,
                image.cell(NESTED_CLASS, row, 1) as usize,
            )
        })
        .collect()
}

fn named_at(image: &Image, table: usize, row: usize, nests: &BTreeMap<usize, usize>) -> String {
    named_deep(image, table, row, nests, 0)
}

fn named_deep(
    image: &Image,
    table: usize,
    row: usize,
    nests: &BTreeMap<usize, usize>,
    depth: usize,
) -> String {
    let name = image.text(image.cell(table, row, 1));
    let space = image.text(image.cell(table, row, 2));

    if table == TYPE_DEF
        && space.is_empty()
        && depth < DEEPEST
        && let Some(outer) = nests.get(&(row + 1))
    {
        return format!(
            "{}/{name}",
            named_deep(image, TYPE_DEF, outer - 1, nests, depth + 1)
        );
    }

    full_name(space, name)
}

fn plain_name(image: &Image, table: usize, row: usize) -> String {
    full_name(
        image.text(image.cell(table, row, 2)),
        image.text(image.cell(table, row, 1)),
    )
}

fn method_owners(image: &Image) -> Vec<usize> {
    let mut out = vec![0usize; image.count(METHOD_DEF) + 2];
    let rows = image.count(TYPE_DEF);

    for row in 0..rows {
        let first = image.cell(TYPE_DEF, row, 5) as usize;
        let past = if row + 1 < rows {
            image.cell(TYPE_DEF, row + 1, 5) as usize
        } else {
            image.count(METHOD_DEF) + 1
        };

        for which in first..past {
            if let Some(slot) = out.get_mut(which) {
                *slot = row + 1;
            }
        }
    }

    out
}

fn attribute_of(image: &Image, row: usize, owners: &[usize]) -> String {
    let (holder, which) = image.tagged(CUSTOM_ATTRIBUTE, row, 1);
    if which == 0 {
        return String::new();
    }

    match holder {
        MEMBER_REF => match image.tagged(MEMBER_REF, which - 1, 0) {
            (TYPE_REF, at) if at > 0 => plain_name(image, TYPE_REF, at - 1),
            (TYPE_DEF, at) if at > 0 => plain_name(image, TYPE_DEF, at - 1),
            _ => String::new(),
        },
        METHOD_DEF => match owners.get(which).copied().unwrap_or(0) {
            0 => String::new(),
            owner => plain_name(image, TYPE_DEF, owner - 1),
        },
        _ => String::new(),
    }
}

type Marks = HashSet<usize>;

fn serialize_marks(image: &Image) -> (Marks, Marks) {
    let owners = method_owners(image);
    let mut kept = Marks::new();
    let mut pointed = Marks::new();

    for row in 0..image.count(CUSTOM_ATTRIBUTE) {
        let (parent, field) = image.tagged(CUSTOM_ATTRIBUTE, row, 0);
        if parent != FIELD || field == 0 {
            continue;
        }

        match attribute_of(image, row, &owners).as_str() {
            SERIALIZE_FIELD => kept.insert(field),
            SERIALIZE_REFERENCE => pointed.insert(field),
            _ => false,
        };
    }

    (kept, pointed)
}

fn by_reference(shape: Shape) -> Shape {
    match shape {
        Shape::List(_) => Shape::List(Box::new(Shape::Reference)),
        _ => Shape::Reference,
    }
}

fn filled_in(image: &Image, row: usize, nests: &BTreeMap<usize, usize>) -> String {
    let raw = image.blob(image.cell(TYPE_SPEC, row, 0));
    let at = usize::from(raw.first() == Some(&GENERIC));

    match read_shape(image, raw, at, nests, 0) {
        Some((Shape::Named(name), _)) => name,
        _ => String::new(),
    }
}

fn shape_of(image: &Image, raw: &[u8], nests: &BTreeMap<usize, usize>) -> Shape {
    read_shape(image, raw, 1, nests, 0)
        .map(|(shape, _)| shape)
        .unwrap_or(Shape::Unknown)
}

fn read_shape(
    image: &Image,
    raw: &[u8],
    at: usize,
    nests: &BTreeMap<usize, usize>,
    depth: usize,
) -> Option<(Shape, usize)> {
    if depth > DEEPEST {
        return None;
    }

    let kind = *raw.get(at)?;
    let at = at + 1;

    let simple = match kind {
        0x02 => Shape::Bool,
        0x04 | 0x05 => Shape::Byte,
        0x03 | 0x06 | 0x07 => Shape::Short,
        0x08 | 0x09 => Shape::Int,
        0x0A | 0x0B => Shape::Long,
        0x0C => Shape::Float,
        0x0D => Shape::Double,
        0x0E => Shape::Text,
        _ => Shape::Unknown,
    };

    if simple != Shape::Unknown {
        return Some((simple, at));
    }

    match kind {
        0x11 | 0x12 => {
            let (value, step) = unpacked(raw, at)?;
            let tag = value & 3;
            let row = value >> 2;
            let table = match tag {
                0 => TYPE_DEF,
                1 => TYPE_REF,
                _ => return Some((Shape::Unknown, at + step)),
            };

            if row == 0 {
                return Some((Shape::Unknown, at + step));
            }

            Some((
                Shape::Named(named_at(image, table, row - 1, nests)),
                at + step,
            ))
        }
        0x1D => {
            let (inner, past) = read_shape(image, raw, at, nests, depth + 1)?;
            Some((Shape::List(Box::new(inner)), past))
        }
        GENERIC => {
            let (inner, past) = read_shape(image, raw, at, nests, depth + 1)?;
            let (count, step) = unpacked(raw, past)?;
            let mut walk = past + step;
            let mut first = None;

            for _ in 0..count {
                let (one, next) = read_shape(image, raw, walk, nests, depth + 1)?;
                first.get_or_insert(one);
                walk = next;
            }

            match inner {
                Shape::Named(name) if name == "System.Collections.Generic.List`1" => {
                    Some((Shape::List(Box::new(first?)), walk))
                }
                _ => Some((Shape::Unknown, walk)),
            }
        }
        _ => Some((Shape::Unknown, at)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::unity::assembly;

    #[test]
    fn a_compressed_number_is_read_the_way_the_format_writes_it() {
        assert_eq!(
            unpacked(&[0x03], 0),
            Some((3, 1)),
            "the top bits of the first byte say how wide the number is, so reading one width \
             for another walks the rest of the table out of step"
        );
        assert_eq!(unpacked(&[0x7F], 0), Some((0x7F, 1)));
        assert_eq!(unpacked(&[0x80, 0x80], 0), Some((0x80, 2)));
        assert_eq!(unpacked(&[0xBF, 0xFF], 0), Some((0x3FFF, 2)));
        assert_eq!(unpacked(&[0xC0, 0x00, 0x40, 0x00], 0), Some((0x4000, 4)));
        assert_eq!(unpacked(&[], 0), None);
    }

    #[test]
    fn a_coded_index_spends_as_many_low_bits_as_it_needs_to_name_its_tables() {
        assert_eq!(
            Coded::TypeDefOrRef.bits(),
            2,
            "a coded index spends its low bits naming the table it points at, so counting them \
             wrong shifts every row that follows"
        );
        assert_eq!(Coded::HasCustomAttribute.bits(), 5);
        assert_eq!(Coded::HasSemantics.bits(), 1);
        assert_eq!(Coded::CustomAttributeType.bits(), 3);
    }

    #[test]
    fn a_type_is_known_by_its_namespace_as_well_as_its_name() {
        assert_eq!(full_name("UnityEngine", "Vector3"), "UnityEngine.Vector3");
        assert_eq!(
            full_name("", "DialogSceneCell"),
            "DialogSceneCell",
            "a game script usually sits in no namespace at all"
        );
    }

    #[test]
    fn nothing_that_is_not_an_assembly_is_taken_for_one() {
        assert!(
            Image::read(b"not a pe file at all").is_err(),
            "a game folder is full of files that are not assemblies, and reading one as an \
             assembly would hand back bytes that were never text"
        );
        assert!(Image::read(&[]).is_err());
    }

    #[test]
    fn an_assembly_cut_short_anywhere_never_takes_the_process_down() {
        let built = assembly::forge::dll(&["Press any key", "MusicVolume"]);

        assert!(
            Image::read(&built.raw).is_ok(),
            "the forged assembly has to be one this reader accepts whole, or cutting it proves \
             nothing"
        );

        for len in (1..built.raw.len()).step_by(7) {
            if let Ok(image) = Image::read(&built.raw[..len]) {
                let mut found = BTreeMap::new();
                gather(&image, &mut found);
            }
        }
    }

    #[test]
    fn a_row_count_reaching_past_the_end_of_the_file_reads_as_nothing() {
        let mut image = Image {
            raw: &[0; 16],
            strings: 8,
            blobs: 0,
            rows: [0; 64],
            starts: [0; 64],
            widths: [0; 64],
            text_wide: false,
            blob_wide: false,
            guid_wide: false,
        };
        image.rows[TYPE_DEF] = 1 << 20;
        image.starts[TYPE_DEF] = 12;

        assert_eq!(
            image.cell(TYPE_DEF, 1 << 19, 1),
            0,
            "the row count comes out of the file being read, so a wrong one aims every read past \
             the end: answering rather than panicking is what keeps a reader who opens one \
             malformed dll from losing the whole import with nothing said"
        );
        assert_eq!(
            image.text(u32::MAX),
            "",
            "and a string offset the file made up reaches the same way"
        );
    }

    #[test]
    fn metadata_that_nests_in_a_circle_or_without_end_still_comes_back() {
        let image = Image {
            raw: &[0; 64],
            strings: 0,
            blobs: 0,
            rows: [0; 64],
            starts: [0; 64],
            widths: [0; 64],
            text_wide: false,
            blob_wide: false,
            guid_wide: false,
        };
        let nests = BTreeMap::from([(1, 1)]);

        assert_eq!(
            named_at(&image, TYPE_DEF, 0, &nests),
            "/".repeat(DEEPEST),
            "a class nested in itself would otherwise recurse until the stack aborts the harvest"
        );
        assert_eq!(
            read_shape(&image, &vec![0x1D; 1 << 20], 0, &nests, 0),
            None,
            "a blob of endless arrays is malformed input, not a shape"
        );
    }
}
