use crate::engine::unity::serial::Node;
use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock};

const PACK: &[u8] = include_bytes!("../../../resources/unity/typetrees.tpk");

const MAGIC: &[u8] = b"TPK*";
const READS: u8 = 2;
const LZ4: u8 = 1;
const TYPE_TREES: u8 = 0;
const HEAD: usize = 20;
const DEEPEST: usize = 255;

const RELEASE_ROOT: u8 = 128;
const EDITOR_ROOT: u8 = 64;
const LISTED: u8 = 1;

struct Held {
    kind: u16,
    name: u16,
    size: i32,
    flag: u32,
    listed: bool,
    kids: Vec<u16>,
}

struct Shapes {
    classes: BTreeMap<i32, Vec<(u64, Option<u16>)>>,
    nodes: Vec<Held>,
    words: Vec<String>,
    commons: Vec<(u64, BTreeMap<usize, String>)>,
}

static BUNDLED: LazyLock<Option<Shapes>> = LazyLock::new(|| read(PACK));

pub fn told(class_id: i32, built_by: &str) -> Option<Arc<Node>> {
    BUNDLED.as_ref()?.shape_of(class_id, built_by).map(Arc::new)
}

pub fn common(spot: usize, built_by: &str) -> Option<&'static str> {
    let commons = &BUNDLED.as_ref()?.commons;
    let table = numbered(built_by)
        .and_then(|wanted| commons.iter().rev().find(|(version, _)| *version <= wanted))
        .or_else(|| commons.first())
        .map(|(_, table)| table)?;

    table.get(&spot).map(String::as_str)
}

impl Shapes {
    fn shape_of(&self, class_id: i32, built_by: &str) -> Option<Node> {
        let wanted = numbered(built_by)?;
        let held = self.classes.get(&class_id)?;

        let root = held
            .iter()
            .rev()
            .find(|(version, _)| *version <= wanted)
            .map(|(_, root)| *root)?;

        self.grown(root?, 0)
    }

    fn grown(&self, which: u16, deep: usize) -> Option<Node> {
        if deep > DEEPEST {
            return None;
        }

        let held = self.nodes.get(which as usize)?;
        let mut kids = Vec::with_capacity(held.kids.len());
        for kid in &held.kids {
            kids.push(self.grown(*kid, deep + 1)?);
        }

        Some(Node {
            kind: self.words.get(held.kind as usize)?.clone(),
            size: held.size,
            flag: held.flag as i32,
            listed: held.listed,
            name: self.words.get(held.name as usize)?.clone(),
            kids,
        })
    }
}

fn numbered(built_by: &str) -> Option<u64> {
    let mut walk = built_by.split('.');
    let major: u64 = walk.next()?.parse().ok()?;
    let minor: u64 = walk.next()?.parse().ok()?;

    let rest = walk.next().unwrap_or("0");
    let end = rest
        .find(|one: char| !one.is_ascii_digit())
        .unwrap_or(rest.len());
    let build: u64 = rest.get(..end)?.parse().unwrap_or(0);

    let tail = rest.get(end..).unwrap_or("");
    let kind = match tail.chars().next() {
        Some('a') => 0,
        Some('b') => 1,
        Some('c') => 2,
        Some('p') => 4,
        Some('x') => 5,
        _ => 3,
    };
    let number: u64 = tail
        .get(1..)
        .and_then(|said| said.parse().ok())
        .unwrap_or(0);

    Some(
        (major & 0xffff) << 48
            | (minor & 0xffff) << 32
            | (build & 0xffff) << 16
            | (kind & 0xff) << 8
            | (number & 0xff),
    )
}

struct Reading<'a> {
    raw: &'a [u8],
    at: usize,
}

impl Reading<'_> {
    fn take(&mut self, many: usize) -> Option<&[u8]> {
        let held = self.raw.get(self.at..self.at.checked_add(many)?)?;
        self.at += many;

        Some(held)
    }

    fn byte(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }

    fn short(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }

    fn signed_short(&mut self) -> Option<i16> {
        Some(i16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }

    fn word(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    fn count(&mut self) -> Option<usize> {
        let held = i32::from_le_bytes(self.take(4)?.try_into().ok()?);
        let held = usize::try_from(held).ok()?;

        (held <= self.raw.len().saturating_sub(self.at)).then_some(held)
    }

    fn long(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }

    fn said(&mut self) -> Option<String> {
        let mut wide = 0usize;
        let mut shift = 0;

        loop {
            let byte = self.byte()?;
            wide |= usize::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                break;
            }

            shift += 7;
            if shift > 28 {
                return None;
            }
        }

        String::from_utf8(self.take(wide)?.to_vec()).ok()
    }
}

fn read(pack: &[u8]) -> Option<Shapes> {
    if !pack.starts_with(MAGIC) || *pack.get(4)? != READS {
        return None;
    }
    if *pack.get(5)? != LZ4 || *pack.get(6)? != TYPE_TREES {
        return None;
    }

    let packed = u32::from_le_bytes(pack.get(12..16)?.try_into().ok()?) as usize;
    let plain = u32::from_le_bytes(pack.get(16..20)?.try_into().ok()?) as usize;

    let mut room = vec![0u8; plain];
    let filled = lz4::block::decompress_to_buffer(
        pack.get(HEAD..HEAD.checked_add(packed)?)?,
        Some(i32::try_from(plain).ok()?),
        &mut room,
    )
    .ok()?;
    room.truncate(filled);

    opened(&room)
}

fn opened(raw: &[u8]) -> Option<Shapes> {
    let mut at = Reading { raw, at: 0 };

    at.long()?;

    let versions = at.count()?;
    for _ in 0..versions {
        at.long()?;
    }

    let mut classes = BTreeMap::new();
    let told = at.count()?;
    for _ in 0..told {
        let class_id = at.word()? as i32;
        let many = at.count()?;

        let mut held = Vec::with_capacity(many.min(4096));
        for _ in 0..many {
            let version = at.long()?;
            let root = match at.byte()? {
                0 => None,
                _ => class_root(&mut at)?,
            };

            held.push((version, root));
        }

        classes.insert(class_id, held);
    }

    let named = at.count()?;
    let mut told = Vec::with_capacity(named.min(64));
    for _ in 0..named {
        let version = at.long()?;

        let entries = at.count()?;
        let mut table = Vec::with_capacity(entries.min(1 << 16));
        for _ in 0..entries {
            let spot = at.short()?;
            let word = at.short()?;
            table.push((spot, word));
        }

        told.push((version, table));
    }

    let many = at.count()?;
    let mut nodes = Vec::with_capacity(many.min(1 << 20));
    for _ in 0..many {
        let kind = at.short()?;
        let name = at.short()?;
        let size = i32::from_le_bytes(at.take(4)?.try_into().ok()?);
        at.signed_short()?;
        let listed = at.byte()? & LISTED == LISTED;
        let flag = at.word()?;

        let kids = at.short()? as usize;
        let mut held = Vec::with_capacity(kids.min(4096));
        for _ in 0..kids {
            held.push(at.short()?);
        }

        nodes.push(Held {
            kind,
            name,
            size,
            flag,
            listed,
            kids: held,
        });
    }

    let said = at.count()?;
    let mut words = Vec::with_capacity(said.min(1 << 20));
    for _ in 0..said {
        words.push(at.said()?);
    }

    let mut commons = Vec::with_capacity(told.len());
    for (version, table) in told {
        let mut common = BTreeMap::new();
        for (spot, which) in table {
            common.insert(spot as usize, words.get(which as usize)?.clone());
        }

        commons.push((version, common));
    }

    Some(Shapes {
        classes,
        nodes,
        words,
        commons,
    })
}

fn class_root(at: &mut Reading<'_>) -> Option<Option<u16>> {
    at.short()?;
    at.short()?;
    let flags = at.byte()?;

    if flags & EDITOR_ROOT == EDITOR_ROOT {
        at.short()?;
    }

    let release = match flags & RELEASE_ROOT == RELEASE_ROOT {
        true => Some(at.short()?),
        false => None,
    };

    Some(release)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::unity::serial;

    fn shape(class_id: i32, built_by: &str) -> Node {
        let held = told(class_id, built_by)
            .unwrap_or_else(|| panic!("the pack knows class {class_id} for {built_by}"));

        Arc::try_unwrap(held).unwrap_or_else(|held| (*held).clone())
    }

    fn field_names(node: &Node) -> Vec<String> {
        node.kids.iter().map(|kid| kid.name.clone()).collect()
    }

    #[test]
    fn the_pack_answers_for_the_classes_this_engine_reads() {
        for class_id in [
            serial::TEXTURE_2D,
            serial::SPRITE,
            serial::FONT,
            49,
            114,
            115,
            142,
            147,
        ] {
            for built_by in [
                "5.6.7f1",
                "2018.4.36f1",
                "2020.3.48f1",
                "2022.3.45f1",
                "6000.0.30f1",
            ] {
                assert!(
                    told(class_id, built_by).is_some(),
                    "class {class_id} at {built_by} is one this engine reads, so a pack that \
                     cannot answer for it would send the reader back to counting bytes by hand"
                );
            }
        }
    }

    #[test]
    fn a_texture_reads_the_fields_this_engine_asks_it_for() {
        let held = shape(serial::TEXTURE_2D, "2022.3.45f1");
        let named = field_names(&held);

        for wanted in [
            "m_Name",
            "m_Width",
            "m_Height",
            "m_CompleteImageSize",
            "m_TextureFormat",
            "m_MipCount",
            "m_MipsStripped",
            "image data",
            "m_StreamData",
        ] {
            assert!(
                named.iter().any(|one| one == wanted),
                "{wanted} is missing from {named:?}"
            );
        }

        let group = named.iter().position(|one| one == "m_MipmapLimitGroupName");
        let streaming = named.iter().position(|one| one == "m_StreamingMipmaps");
        assert!(
            group < streaming,
            "the group name sits between the flags in 2022.2 and later, which is the one thing a \
             hand written reader got wrong: {named:?}"
        );
    }

    #[test]
    fn a_sprite_names_the_texture_it_is_cut_from() {
        let held = shape(serial::SPRITE, "2022.3.45f1");
        let named = field_names(&held);

        assert!(named.iter().any(|one| one == "m_Rect"));
        assert!(named.iter().any(|one| one == "m_RD"));

        let inside = held
            .kids
            .iter()
            .find(|kid| kid.name == "m_RD")
            .map(field_names)
            .expect("the render data");

        assert!(inside.iter().any(|one| one == "texture"));
        assert!(inside.iter().any(|one| one == "textureRect"));
        assert!(inside.iter().any(|one| one == "settingsRaw"));
    }

    #[test]
    fn an_older_build_gets_the_shape_that_build_wrote() {
        let old = field_names(&shape(serial::TEXTURE_2D, "5.6.7f1"));
        let new = field_names(&shape(serial::TEXTURE_2D, "2022.3.45f1"));

        assert_ne!(
            old, new,
            "Unity moved these fields around over ten years, and a pack answering the same shape \
             for every version would be no better than guessing"
        );
        assert!(
            !old.iter().any(|one| one == "m_MipsStripped"),
            "m_MipsStripped arrived in 2020.1, so a 5.6 game must not be read as if it had one"
        );
        assert!(new.iter().any(|one| one == "m_MipsStripped"));
    }

    #[test]
    fn a_version_the_pack_never_heard_of_falls_back_to_the_last_shape_it_knows() {
        let newest = field_names(&shape(serial::TEXTURE_2D, "6000.0.30f1"));
        let ahead = field_names(&shape(serial::TEXTURE_2D, "9999.1.0f1"));

        assert_eq!(
            newest, ahead,
            "a game built with a Unity newer than this pack still has to open: the newest shape \
             the pack knows is a better answer than nothing at all"
        );
    }

    #[test]
    fn a_class_the_pack_never_heard_of_comes_back_as_nothing() {
        assert!(
            told(999_999_999, "2022.3.45f1").is_none(),
            "a game holds class ids no pack knows, so asking about one has to answer rather than \
             panic through a read nothing above it could finish"
        );
        assert!(told(serial::TEXTURE_2D, "").is_none());
        assert!(told(serial::TEXTURE_2D, "not a version").is_none());
    }

    #[test]
    fn a_pack_that_is_not_one_is_refused_instead_of_read_as_rubbish() {
        assert!(read(b"").is_none());
        assert!(read(b"TPK*").is_none());
        assert!(read(&[b'T', b'P', b'K', b'*', 9, 1, 0, 0, 0, 0, 0, 0]).is_none());

        let mut held = PACK.to_vec();
        held[5] = 3;
        assert!(
            read(&held).is_none(),
            "a pack squeezed some other way is one this reader cannot open, and reading it as lz4 \
             would hand back a tree of noise"
        );
    }

    #[test]
    fn the_version_a_game_names_itself_by_sorts_the_way_unity_ships_them() {
        let held = |said: &str| numbered(said).expect(said);

        assert!(held("5.6.7f1") < held("2017.1.0f1"));
        assert!(held("2022.2.0f1") < held("2022.3.45f1"));
        assert!(held("2022.3.45f1") < held("6000.0.30f1"));
        assert!(
            held("2022.3.45b1") < held("2022.3.45f1"),
            "a beta comes before the final of the same build, which is the order the pack keeps"
        );
        assert_eq!(
            held("2022.3"),
            held("2022.3.0f0"),
            "a header that names no build at all reads as that build's release, which is what \
             every shipped game carries"
        );
        assert!(
            numbered("2022").is_none() && numbered("").is_none(),
            "a version this reader cannot make sense of has to come back as nothing, or the pack \
             hands out a shape for a game nobody identified"
        );
    }
}
