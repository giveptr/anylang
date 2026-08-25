use crate::engine::fonts as face;
use crate::engine::unity::dotnet::{Class, Field, Shape};
use crate::engine::unity::serial::{Container, Object, Value};
use crate::engine::unity::{atlas, serial};
use std::collections::BTreeMap;

const WIDEST: usize = 16;

pub fn class(base: &str, fields: Vec<(&str, Shape)>) -> Class {
    Class {
        base: base.to_string(),
        fields: fields
            .into_iter()
            .map(|(name, shape)| Field {
                name: name.to_string(),
                shape,
            })
            .collect(),
        enumeration: None,
        serializable: true,
    }
}

pub fn container(name: &str, objects: Vec<Object>, externals: &[&str]) -> Container {
    Container {
        built_by: serial::BUILT_BY.to_string(),
        name: name.to_string(),
        objects,
        externals: externals.iter().map(|one| one.to_string()).collect(),
    }
}

pub fn forge(assets: &[(i64, &str, &str)]) -> Vec<u8> {
    forge_as(serial::WIDE, assets)
}

pub fn forge_as(version: u32, assets: &[(i64, &str, &str)]) -> Vec<u8> {
    let bodies: Vec<(i64, i32, Vec<u8>)> = assets
        .iter()
        .map(|(path_id, name, script)| (*path_id, serial::TEXT_ASSET, a_text_asset(name, script)))
        .collect();

    forge_objects_as(version, &bodies)
}

pub fn a_mono_script(class: &str) -> Vec<u8> {
    let (space, name) = class.rsplit_once('.').unwrap_or(("", class));

    drawing(
        serial::MONO_SCRIPT,
        vec![
            ("m_Name", text(name)),
            ("m_ClassName", text(name)),
            ("m_Namespace", text(space)),
        ],
    )
}

pub fn a_mono_behaviour(script_path_id: i64, texts: &[&str]) -> Vec<u8> {
    let mut out = vec![0u8; 16];
    out.extend_from_slice(&0i32.to_le_bytes());
    out.extend_from_slice(&script_path_id.to_le_bytes());
    out.extend_from_slice(&string(""));
    for text in texts {
        out.extend_from_slice(&string(text));
    }

    out
}

pub fn drawing(class_id: i32, held: Vec<(&str, Value)>) -> Vec<u8> {
    let mut value = serial::blank_of(class_id).expect("the pack knows this class");

    for (at, held) in held {
        put(&mut value, at, held);
    }

    serial::body_of(class_id, &value).expect("a body the pack can write")
}

fn put(value: &mut Value, at: &str, held: Value) {
    let (head, rest) = match at.split_once('.') {
        Some((head, rest)) => (head, Some(rest)),
        None => (at, None),
    };

    let room = value
        .field_mut(head)
        .unwrap_or_else(|| panic!("{head} is not a field of this class"));

    match rest {
        Some(rest) => put(room, rest, held),
        None => *room = held,
    }
}

pub fn number(held: i64) -> Value {
    Value::Number(held)
}

pub fn real(held: f32) -> Value {
    Value::Real(f64::from(held))
}

pub fn text(said: &str) -> Value {
    Value::Bytes(said.as_bytes().to_vec())
}

pub fn bytes(raw: &[u8]) -> Value {
    Value::Bytes(raw.to_vec())
}

pub fn a_text_asset(name: &str, script: &str) -> Vec<u8> {
    drawing(
        serial::TEXT_ASSET,
        vec![("m_Name", text(name)), ("m_Script", text(script))],
    )
}

pub fn a_font(name: &str, data: &[u8], size: f32, drawn: face::Drawn) -> Vec<u8> {
    drawing(
        serial::FONT,
        vec![
            ("m_Name", text(name)),
            ("m_LineSpacing", real(drawn.line)),
            ("m_FontSize", real(size)),
            ("m_FontData", bytes(data)),
            ("m_Ascent", real(drawn.ascent)),
            ("m_Descent", real(drawn.descent)),
        ],
    )
}

pub struct Drawn<'d> {
    pub name: &'d str,
    pub wide: usize,
    pub high: usize,
    pub format: i32,
    pub mips: i32,
    pub stripped: i32,
    pub whole: usize,
    pub data: &'d [u8],
    pub sidecar: &'d str,
    pub at: u64,
}

pub fn drawn(name: &str, wide: usize, high: usize, format: i32) -> Drawn<'_> {
    Drawn {
        name,
        wide,
        high,
        format,
        mips: 1,
        stripped: 0,
        whole: 0,
        data: &[],
        sidecar: "",
        at: 0,
    }
}

pub fn a_texture(held: &Drawn<'_>) -> Vec<u8> {
    let whole = held.whole.max(held.data.len());
    let mut fields = vec![("m_MipsStripped", number(i64::from(held.stripped)))];
    fields.retain(|_| held.stripped > 0);

    fields.extend(vec![
        ("m_Name", text(held.name)),
        ("m_Width", number(held.wide as i64)),
        ("m_Height", number(held.high as i64)),
        ("m_CompleteImageSize", number(whole as i64)),
        ("m_TextureFormat", number(i64::from(held.format))),
        ("m_MipCount", number(i64::from(held.mips))),
        ("m_ImageCount", number(1)),
        ("m_TextureDimension", number(2)),
        ("image data", bytes(held.data)),
        ("m_StreamData.offset", number(held.at as i64)),
        (
            "m_StreamData.size",
            number(match held.sidecar.is_empty() {
                true => 0,
                false => whole as i64,
            }),
        ),
        ("m_StreamData.path", text(held.sidecar)),
    ]);

    drawing(serial::TEXTURE_2D, fields)
}

pub fn a_sprite(name: &str, of: i64, rect: (f32, f32, f32, f32), settings: u32) -> Vec<u8> {
    a_sprite_from(name, 0, of, rect, settings)
}

pub fn a_sprite_from(
    name: &str,
    file: i32,
    of: i64,
    rect: (f32, f32, f32, f32),
    settings: u32,
) -> Vec<u8> {
    drawing(
        serial::SPRITE,
        vec![
            ("m_Name", text(name)),
            ("m_RD.texture.m_FileID", number(i64::from(file))),
            ("m_RD.texture.m_PathID", number(of)),
            ("m_RD.textureRect.x", real(rect.0)),
            ("m_RD.textureRect.y", real(rect.1)),
            ("m_RD.textureRect.width", real(rect.2)),
            ("m_RD.textureRect.height", real(rect.3)),
            ("m_RD.settingsRaw", number(i64::from(settings))),
        ],
    )
}

#[derive(Clone, Copy)]
pub struct Key {
    pub guid: [i64; 4],
    pub local: i64,
}

pub struct Entry {
    pub key: Key,
    pub of: i64,
    pub rect: (f32, f32, f32, f32),
    pub settings: u32,
    pub downscale: f32,
}

pub fn entry() -> Entry {
    Entry {
        key: Key {
            guid: [0; 4],
            local: 0,
        },
        of: 0,
        rect: (0.0, 0.0, 0.0, 0.0),
        settings: 0,
        downscale: 1.0,
    }
}

pub fn a_packed_sprite(name: &str, atlas: i64, sized: (f32, f32), key: Key, tag: &str) -> Vec<u8> {
    let mut fields = vec![
        ("m_Name", text(name)),
        ("m_RD.texture.m_FileID", number(0)),
        ("m_RD.texture.m_PathID", number(0)),
        ("m_Rect.width", real(sized.0)),
        ("m_Rect.height", real(sized.1)),
        ("m_SpriteAtlas.m_PathID", number(atlas)),
    ];
    fields.extend(keyed(key));

    if !tag.is_empty() {
        fields.push(("m_AtlasTags", Value::List(vec![text(tag)])));
    }

    drawing(serial::SPRITE, fields)
}

pub fn a_sprite_in_an_atlas(
    name: &str,
    of: i64,
    rect: (f32, f32, f32, f32),
    atlas: i64,
    key: Key,
) -> Vec<u8> {
    let mut fields = vec![
        ("m_Name", text(name)),
        ("m_RD.texture.m_PathID", number(of)),
        ("m_RD.textureRect.x", real(rect.0)),
        ("m_RD.textureRect.y", real(rect.1)),
        ("m_RD.textureRect.width", real(rect.2)),
        ("m_RD.textureRect.height", real(rect.3)),
        ("m_SpriteAtlas.m_PathID", number(atlas)),
    ];
    fields.extend(keyed(key));

    drawing(serial::SPRITE, fields)
}

fn keyed(key: Key) -> Vec<(&'static str, Value)> {
    vec![
        ("m_RenderDataKey.first.data[0]", number(key.guid[0])),
        ("m_RenderDataKey.first.data[1]", number(key.guid[1])),
        ("m_RenderDataKey.first.data[2]", number(key.guid[2])),
        ("m_RenderDataKey.first.data[3]", number(key.guid[3])),
        ("m_RenderDataKey.second", number(key.local)),
    ]
}

pub fn an_atlas(name: &str, held: &[Entry]) -> Vec<u8> {
    let listed: Vec<Value> = held
        .iter()
        .map(|one| {
            Value::Tree(vec![
                (
                    "first".to_string(),
                    Value::Tree(vec![
                        (
                            "first".to_string(),
                            Value::Tree(
                                one.key
                                    .guid
                                    .iter()
                                    .enumerate()
                                    .map(|(which, held)| {
                                        (format!("data[{which}]"), Value::Number(*held))
                                    })
                                    .collect(),
                            ),
                        ),
                        ("second".to_string(), Value::Number(one.key.local)),
                    ]),
                ),
                (
                    "second".to_string(),
                    Value::Tree(vec![
                        ("texture".to_string(), pointing(one.of)),
                        ("alphaTexture".to_string(), pointing(0)),
                        (
                            "textureRect".to_string(),
                            sided(
                                &["x", "y", "width", "height"],
                                &[one.rect.0, one.rect.1, one.rect.2, one.rect.3],
                            ),
                        ),
                        (
                            "textureRectOffset".to_string(),
                            sided(&["x", "y"], &[0.0, 0.0]),
                        ),
                        (
                            "atlasRectOffset".to_string(),
                            sided(&["x", "y"], &[one.rect.0, one.rect.1]),
                        ),
                        (
                            "uvTransform".to_string(),
                            sided(&["x", "y", "z", "w"], &[1.0, 0.0, 1.0, 0.0]),
                        ),
                        ("downscaleMultiplier".to_string(), real(one.downscale)),
                        ("settingsRaw".to_string(), number(i64::from(one.settings))),
                        ("secondaryTextures".to_string(), Value::List(Vec::new())),
                    ]),
                ),
            ])
        })
        .collect();

    drawing(
        atlas::SPRITE_ATLAS,
        vec![
            ("m_Name", text(name)),
            ("m_RenderDataMap", Value::List(listed)),
        ],
    )
}

fn pointing(path_id: i64) -> Value {
    Value::Tree(vec![
        ("m_FileID".to_string(), Value::Number(0)),
        ("m_PathID".to_string(), Value::Number(path_id)),
    ])
}

fn sided(names: &[&str], held: &[f32]) -> Value {
    Value::Tree(
        names
            .iter()
            .zip(held)
            .map(|(name, one)| (name.to_string(), real(*one)))
            .collect(),
    )
}

pub fn string(text: &str) -> Vec<u8> {
    let mut out = (text.len() as u32).to_le_bytes().to_vec();
    out.extend_from_slice(text.as_bytes());
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }

    out
}

pub fn strings(pieces: &[&str]) -> Vec<u8> {
    pieces.iter().flat_map(|piece| string(piece)).collect()
}

const ALIGN: i32 = 0x4000;

pub enum Kind {
    Number(i32),
    Text,
    Pointer,
    Struct(Vec<(&'static str, Kind)>),
    List(Box<Kind>),
}

pub enum Val {
    Number(i64),
    Text(String),
    Pointer(i32, i64),
    Struct(Vec<Val>),
    List(Vec<Val>),
}

struct Only {
    level: u8,
    listed: bool,
    name: &'static str,
    size: i32,
    flag: i32,
}

fn made(name: &'static str, kind: &Kind, level: u8, out: &mut Vec<Only>) {
    let one = |level, listed, name, size, flag| Only {
        level,
        listed,
        name,
        size,
        flag,
    };

    match kind {
        Kind::Number(size) => out.push(one(level, false, name, *size, 0)),
        Kind::Pointer => {
            out.push(one(level, false, name, 12, 0));
            out.push(one(level + 1, false, "m_FileID", 4, 0));
            out.push(one(level + 1, false, "m_PathID", 8, 0));
        }
        Kind::Text => {
            out.push(one(level, false, name, -1, 0));
            out.push(one(level + 1, true, "Array", -1, ALIGN));
            out.push(one(level + 2, false, "size", 4, 0));
            out.push(one(level + 2, false, "data", 1, 0));
        }
        Kind::Struct(fields) => {
            out.push(one(level, false, name, -1, 0));
            for (had, kid) in fields {
                made(had, kid, level + 1, out);
            }
        }
        Kind::List(item) => {
            out.push(one(level, false, name, -1, 0));
            out.push(one(level + 1, true, "Array", -1, ALIGN));
            out.push(one(level + 2, false, "size", 4, 0));
            made("data", item, level + 2, out);
        }
    }
}

fn tree_of(kind: &Kind) -> Vec<u8> {
    let mut nodes = Vec::new();
    made("Base", kind, 0, &mut nodes);

    let mut letters: Vec<u8> = Vec::new();
    let mut spots: BTreeMap<&str, u32> = Default::default();
    for one in &nodes {
        if !spots.contains_key(one.name) {
            spots.insert(one.name, letters.len() as u32);
            letters.extend_from_slice(one.name.as_bytes());
            letters.push(0);
        }
    }

    let mut out = Vec::new();
    out.extend_from_slice(&(nodes.len() as i32).to_le_bytes());
    out.extend_from_slice(&(letters.len() as i32).to_le_bytes());

    for one in &nodes {
        out.extend_from_slice(&0u16.to_le_bytes());
        out.push(one.level);
        out.push(u8::from(one.listed));
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&spots[one.name].to_le_bytes());
        out.extend_from_slice(&one.size.to_le_bytes());
        out.extend_from_slice(&0i32.to_le_bytes());
        out.extend_from_slice(&one.flag.to_le_bytes());
        out.extend_from_slice(&0i64.to_le_bytes());
    }

    out.extend_from_slice(&letters);
    out.extend_from_slice(&0i32.to_le_bytes());

    out
}

pub fn body_of(kind: &Kind, value: &Val) -> Vec<u8> {
    let mut out = Vec::new();
    written(kind, value, &mut out);

    out
}

fn written(kind: &Kind, value: &Val, out: &mut Vec<u8>) {
    match (kind, value) {
        (Kind::Number(size), Val::Number(number)) => match size {
            1 => out.push(*number as u8),
            2 => out.extend_from_slice(&(*number as i16).to_le_bytes()),
            4 => out.extend_from_slice(&(*number as i32).to_le_bytes()),
            _ => out.extend_from_slice(&number.to_le_bytes()),
        },
        (Kind::Pointer, Val::Pointer(file, path_id)) => {
            out.extend_from_slice(&file.to_le_bytes());
            out.extend_from_slice(&path_id.to_le_bytes());
        }
        (Kind::Text, Val::Text(text)) => {
            out.extend_from_slice(&(text.len() as i32).to_le_bytes());
            out.extend_from_slice(text.as_bytes());
            while !out.len().is_multiple_of(4) {
                out.push(0);
            }
        }
        (Kind::Struct(fields), Val::Struct(values)) => {
            assert_eq!(
                fields.len(),
                values.len(),
                "a struct needs a value per field"
            );
            for ((_, kid), one) in fields.iter().zip(values) {
                written(kid, one, out);
            }
        }
        (Kind::List(item), Val::List(items)) => {
            out.extend_from_slice(&(items.len() as i32).to_le_bytes());
            for one in items {
                written(item, one, out);
            }
            while !out.len().is_multiple_of(4) {
                out.push(0);
            }
        }
        _ => panic!("the value does not match the shape it was written by"),
    }
}

pub fn forge_trees(objects: &[(i64, i32, Kind, Vec<u8>)]) -> Vec<u8> {
    let kinds: Vec<(i32, Option<Vec<u8>>)> = objects
        .iter()
        .map(|(_, class_id, kind, _)| (*class_id, Some(tree_of(kind))))
        .collect();

    let slots: Vec<(i64, usize, &[u8])> = objects
        .iter()
        .enumerate()
        .map(|(which, (path_id, _, _, body))| (*path_id, which, body.as_slice()))
        .collect();

    forged(serial::WIDE, &kinds, &slots)
}

pub fn forge_objects(objects: &[(i64, i32, Vec<u8>)]) -> Vec<u8> {
    forge_objects_as(serial::WIDE, objects)
}

fn forge_objects_as(version: u32, objects: &[(i64, i32, Vec<u8>)]) -> Vec<u8> {
    let mut kinds: Vec<(i32, Option<Vec<u8>>)> = Vec::new();
    for (_, class_id, _) in objects {
        if !kinds.iter().any(|(kind, _)| kind == class_id) {
            kinds.push((*class_id, None));
        }
    }

    let slots: Vec<(i64, usize, &[u8])> = objects
        .iter()
        .map(|(path_id, class_id, body)| {
            let which = kinds
                .iter()
                .position(|(kind, _)| kind == class_id)
                .expect("a class the loop above put there");

            (*path_id, which, body.as_slice())
        })
        .collect();

    forged(version, &kinds, &slots)
}

fn forged(
    version: u32,
    kinds: &[(i32, Option<Vec<u8>>)],
    slots: &[(i64, usize, &[u8])],
) -> Vec<u8> {
    let wide = version >= serial::WIDE;
    let trees = kinds.iter().any(|(_, tree)| tree.is_some());

    let mut meta = Vec::new();
    match wide {
        true => {
            meta.extend_from_slice(serial::BUILT_BY.as_bytes());
            meta.push(0);
        }
        false => meta.extend_from_slice(b"2018.1.0f2\0"),
    }
    meta.extend_from_slice(&2i32.to_le_bytes());
    meta.push(u8::from(trees));
    meta.extend_from_slice(&(kinds.len() as i32).to_le_bytes());

    for (class_id, tree) in kinds {
        meta.extend_from_slice(&class_id.to_le_bytes());
        meta.push(0);
        meta.extend_from_slice(&(-1i16).to_le_bytes());
        if *class_id == serial::MONO_BEHAVIOUR {
            meta.extend_from_slice(&[0; 16]);
        }
        meta.extend_from_slice(&[0; 16]);
        if let Some(tree) = tree {
            meta.extend_from_slice(tree);
        }
    }

    meta.extend_from_slice(&(slots.len() as i32).to_le_bytes());

    let mut at = 0usize;
    for (path_id, which, body) in slots {
        while !meta.len().is_multiple_of(4) {
            meta.push(0);
        }
        while !at.is_multiple_of(WIDEST) {
            at += 1;
        }

        meta.extend_from_slice(&path_id.to_le_bytes());
        match wide {
            true => meta.extend_from_slice(&(at as i64).to_le_bytes()),
            false => meta.extend_from_slice(&(at as u32).to_le_bytes()),
        }
        meta.extend_from_slice(&(body.len() as u32).to_le_bytes());
        meta.extend_from_slice(&(*which as i32).to_le_bytes());

        at += body.len();
    }

    meta.extend_from_slice(&0i32.to_le_bytes());
    meta.extend_from_slice(&0i32.to_le_bytes());

    let head = match wide {
        true => 48,
        false => 20,
    };
    let data_at = (head + meta.len() + 15) & !15;

    let mut out = vec![0u8; head];
    out[8..12].copy_from_slice(&version.to_be_bytes());
    match wide {
        true => {
            out[20..24].copy_from_slice(&(meta.len() as u32).to_be_bytes());
            out[32..40].copy_from_slice(&(data_at as i64).to_be_bytes());
        }
        false => {
            out[0..4].copy_from_slice(&(meta.len() as u32).to_be_bytes());
            out[12..16].copy_from_slice(&(data_at as u32).to_be_bytes());
        }
    }
    out.extend_from_slice(&meta);
    out.resize(data_at, 0);

    for (_, _, body) in slots {
        while !(out.len() - data_at).is_multiple_of(WIDEST) {
            out.push(0);
        }
        out.extend_from_slice(body);
    }

    match wide {
        true => {
            let whole = out.len() as i64;
            out[serial::WHOLE_AT..serial::WHOLE_AT + 8].copy_from_slice(&whole.to_be_bytes());
        }
        false => {
            let whole = out.len() as u32;
            out[serial::SIZE_AT..serial::SIZE_AT + 4].copy_from_slice(&whole.to_be_bytes());
        }
    }

    out
}
