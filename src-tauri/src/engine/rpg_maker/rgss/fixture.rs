pub use crate::engine::rpg_maker::fixture::sandbox;
use crate::engine::rpg_maker::rgss::marshal::long_bytes;
use crate::engine::rpg_maker::rgss::packed;

pub fn tagged(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    out.extend_from_slice(&long_bytes(body.len() as i64));
    out.extend_from_slice(body);

    out
}

pub fn said(body: &str) -> Vec<u8> {
    tagged(b'"', body.as_bytes())
}

pub fn name(body: &str) -> Vec<u8> {
    tagged(b':', body.as_bytes())
}

pub fn number(value: i64) -> Vec<u8> {
    let mut out = vec![b'i'];
    out.extend_from_slice(&long_bytes(value));

    out
}

pub fn list(items: &[Vec<u8>]) -> Vec<u8> {
    let mut out = vec![b'['];
    out.extend_from_slice(&long_bytes(items.len() as i64));
    for one in items {
        out.extend_from_slice(one);
    }

    out
}

pub fn hash(pairs: &[(Vec<u8>, Vec<u8>)], default: Option<Vec<u8>>) -> Vec<u8> {
    let mut out = vec![if default.is_some() { b'}' } else { b'{' }];
    out.extend_from_slice(&long_bytes(pairs.len() as i64));

    for (key, value) in pairs {
        out.extend_from_slice(key);
        out.extend_from_slice(value);
    }
    if let Some(one) = default {
        out.extend_from_slice(&one);
    }

    out
}

pub fn object(class: &str, fields: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut out = vec![b'o'];
    out.extend_from_slice(&name(class));
    out.extend_from_slice(&long_bytes(fields.len() as i64));

    for (called, value) in fields {
        out.extend_from_slice(&name(&format!("@{called}")));
        out.extend_from_slice(value);
    }

    out
}

pub fn command(code: i64, parameters: &[Vec<u8>]) -> Vec<u8> {
    object(
        "RPG::EventCommand",
        &[
            ("code", number(code)),
            ("indent", number(0)),
            ("parameters", list(parameters)),
        ],
    )
}

pub fn map(commands: &[Vec<u8>]) -> Vec<u8> {
    stream(&object("RPG::Map", &[("list", list(commands))]))
}

pub fn scripts(each: &[(&str, &str)]) -> Vec<u8> {
    let listed: Vec<Vec<u8>> = each
        .iter()
        .enumerate()
        .map(|(which, (called, source))| {
            list(&[
                number(which as i64),
                said(called),
                tagged(b'"', &packed::shut(source.as_bytes()).expect("it packs")),
            ])
        })
        .collect();

    stream(&list(&listed))
}

pub fn stream(body: &[u8]) -> Vec<u8> {
    let mut out = vec![4, 8];
    out.extend_from_slice(body);

    out
}
