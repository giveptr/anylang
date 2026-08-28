use crate::canvas::Canvas;
use crate::engine::wolf_rpg::coder::{self, line, repacked};
use crate::engine::wolf_rpg::{archive, event, game, keying, source};
use std::fs;
use std::io::Cursor;
use std::path::Path;
use tempfile::TempDir;

pub fn sandbox() -> TempDir {
    tempfile::tempdir().expect("a temp folder")
}

fn word(value: u32) -> Vec<u8> {
    value.to_le_bytes().to_vec()
}

fn headed(kind: u8, utf8_at: usize) -> Vec<u8> {
    let mut out = vec![0x57, 0x00, 0x00, 0x4F, 0x4C, 0x00, 0x46, kind, 0x00];
    out[utf8_at] = coder::UTF8_MARK;

    out
}

pub fn command(code: u32, args: &[u32], said: &[&str]) -> Vec<u8> {
    let mut out = vec![(args.len() as u8).wrapping_add(1)];
    out.extend(word(code));
    for arg in args {
        out.extend(word(*arg));
    }

    out.push(0);
    out.push(said.len() as u8);
    for one in said {
        out.extend(line(one));
    }

    out.push(0);

    out
}

pub fn tailed(code: u32, args: &[u32], said: &[&str]) -> Vec<u8> {
    let mut out = command(code, args, said);
    out.push(3);
    out.extend_from_slice(&[9; 3]);

    out
}

pub fn moved(route: &[(u8, &[u32])]) -> Vec<u8> {
    let mut out = vec![1];
    out.extend(word(event::MOVE));
    out.push(0);
    out.push(0);
    out.push(1);
    out.extend_from_slice(&[0; 6]);

    out.extend(word(route.len() as u32));
    for (id, args) in route {
        out.push(*id);
        out.push(args.len() as u8);
        for arg in *args {
            out.extend(word(*arg));
        }
        out.extend_from_slice(&[0x01, 0x00]);
    }

    out
}

fn page(commands: &[Vec<u8>]) -> Vec<u8> {
    let mut out = vec![0x79];
    out.extend(word(0));
    out.extend(line("CharaChip/hand3.png"));
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&[0; 1 + 4 + 16 + 16]);
    out.extend_from_slice(&[0; 4]);
    out.push(0);
    out.push(0);
    out.extend(word(0));

    out.extend(word(commands.len() as u32));
    for one in commands {
        out.extend_from_slice(one);
    }

    out.extend(word(3));
    out.extend_from_slice(&[0; 3]);
    out.push(0x7A);

    out
}

fn map_one(which: u32, pages: &[&[Vec<u8>]]) -> Vec<u8> {
    let mut out = vec![0x6F];
    out.extend_from_slice(&[0x39, 0x30, 0x00, 0x00]);
    out.extend(word(which));
    out.extend(line("\u{753b}\u{50cf}\u{6d88}\u{53bb}"));
    out.extend(word(0));
    out.extend(word(0));
    out.extend(word(pages.len() as u32));
    out.extend_from_slice(&[0; 4]);

    for one in pages {
        out.extend(page(one));
    }

    out.push(0x70);

    out
}

fn map_body(events: &[&[Vec<u8>]], layered: bool) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend(line("\u{306a}\u{3057}"));
    out.extend(word(0));
    out.extend(word(2));
    out.extend(word(2));
    out.extend(word(events.len() as u32));

    if layered {
        out.extend(word(0));
        out.extend(word(3));
    }

    out.extend(word(0xFFFF_FFFF));

    for (which, pages) in events.iter().enumerate() {
        out.extend(map_one(which as u32, &[pages]));
    }

    out.push(0x66);

    out
}

fn map_head(version: u32) -> Vec<u8> {
    let mut out = vec![0; 10];
    out.extend_from_slice(b"WOLFM");
    out.push(0);
    out.push(coder::UTF8_MARK);
    out.extend_from_slice(&[0; 3]);
    out.extend(word(version));
    out.push(0);

    out
}

pub fn map(events: &[&[Vec<u8>]]) -> Vec<u8> {
    let mut out = map_head(0x64);
    out.extend(map_body(events, false));

    out
}

pub fn newest_map(events: &[&[Vec<u8>]]) -> Vec<u8> {
    let mut plain = map_head(0x67);
    let head = plain.len();
    plain.extend(map_body(events, true));

    repacked(&plain, head).expect("it packs")
}

pub fn packed_map(events: &[&[Vec<u8>]]) -> Vec<u8> {
    let mut plain = map_head(0x65);
    let head = plain.len();
    plain.extend(map_body(events, false));

    repacked(&plain, head).expect("it packs")
}

fn common_one(which: u32, commands: &[Vec<u8>]) -> Vec<u8> {
    let mut out = vec![0x8E];
    out.extend(word(which));
    out.extend(word(0));
    out.extend_from_slice(&[0; 7]);
    out.extend(line("\u{25a0}\u{6226}\u{95d8}"));

    out.extend(word(commands.len() as u32));
    for one in commands {
        out.extend_from_slice(one);
    }

    out.extend(line(" "));
    out.extend(line(" "));
    out.push(0x8F);

    out.extend(word(0));
    out.extend(word(0));
    out.extend(word(0));
    out.extend(word(0));
    out.extend_from_slice(&[0; 0x1D]);

    for _ in 0..100 {
        out.extend(line(" "));
    }

    out.push(0x91);
    out.extend(line(" "));
    out.push(0x91);

    out
}

fn commons_at(version: u8, packed: bool, events: &[&[Vec<u8>]]) -> Vec<u8> {
    let mut out = vec![0];
    out.extend(headed(0x43, 5));
    out.push(version);

    let head = out.len();

    out.extend(word(events.len() as u32));
    for (which, commands) in events.iter().enumerate() {
        out.extend(common_one(which as u32, commands));
    }

    out.push(0x8A);

    match packed {
        true => repacked(&out, head).expect("it packs"),
        false => out,
    }
}

pub fn commons(events: &[&[Vec<u8>]]) -> Vec<u8> {
    commons_at(0xC9, false, events)
}

pub fn newest_commons(events: &[&[Vec<u8>]]) -> Vec<u8> {
    commons_at(0x93, true, events)
}

pub struct Type<'a> {
    pub name: &'a str,
    pub fields: &'a [&'a str],
    pub words: &'a [usize],
    pub entries: &'a [&'a [&'a str]],
    pub named_by: Option<usize>,
}

pub fn database(types: &[Type<'_>]) -> (Vec<u8>, Vec<u8>) {
    let mut plan = word(types.len() as u32);

    for held in types {
        plan.extend(line(held.name));
        plan.extend(word(held.fields.len() as u32));
        for one in held.fields {
            plan.extend(line(one));
        }

        let naming = held
            .named_by
            .and_then(|field| held.words.iter().position(|one| *one == field));

        plan.extend(word(held.entries.len() as u32));
        for (which, row) in held.entries.iter().enumerate() {
            match naming.and_then(|at| row.get(at)) {
                Some(name) => plan.extend(line(name)),
                None => plan.extend(line(&format!("row{which}"))),
            }
        }

        plan.extend(line(" "));
        plan.extend(word(held.fields.len() as u32));
        plan.extend(vec![0u8; held.fields.len()]);

        plan.extend(word(held.fields.len() as u32));
        for _ in held.fields {
            plan.extend(line(" "));
        }

        plan.extend(word(held.fields.len() as u32));
        for _ in held.fields {
            plan.extend(word(0));
        }

        plan.extend(word(held.fields.len() as u32));
        for _ in held.fields {
            plan.extend(word(0));
        }

        plan.extend(word(held.fields.len() as u32));
        for _ in held.fields {
            plan.extend(word(0));
        }
    }

    let mut data = vec![0];
    data.extend(headed(0x4D, 5));
    data.push(0xC2);
    data.extend(word(types.len() as u32));

    for held in types {
        data.extend_from_slice(&[0xFE, 0xFF, 0xFF, 0xFF]);
        data.extend(word(0));
        data.extend(word(held.fields.len() as u32));

        let mut slot = 0;
        let mut numbers = 0;
        for which in 0..held.fields.len() {
            match held.words.contains(&which) {
                true => {
                    data.extend(word(0x07D0 + slot));
                    slot += 1;
                }
                false => {
                    data.extend(word(0x03E8 + numbers));
                    numbers += 1;
                }
            }
        }

        data.extend(word(held.entries.len() as u32));
        for row in held.entries {
            for _ in 0..numbers {
                data.extend(word(0));
            }
            for said in *row {
                data.extend(line(said));
            }
        }
    }

    data.push(0xC2);

    (plan, data)
}

pub fn game(title: &str, plus: &str, font: &str) -> Vec<u8> {
    drawn_by(title, plus, &[font])
}

pub fn dotted(wide: usize, high: usize, tint: u8) -> Canvas {
    let mut held = Canvas::blank(wide, high);

    for (which, byte) in held.pixels.iter_mut().enumerate() {
        *byte = ((which * 5) as u8).wrapping_add(tint);
    }

    held
}

pub fn a_png(wide: usize, high: usize, tint: u8) -> Vec<u8> {
    dotted(wide, high, tint).png().expect("a png")
}

pub fn a_jpeg(wide: u32, high: u32) -> Vec<u8> {
    let held = image::RgbImage::from_fn(wide, high, |across, down| {
        image::Rgb([(across * 7) as u8, (down * 5) as u8, 40])
    });

    let mut out = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(held)
        .write_to(&mut out, image::ImageFormat::Jpeg)
        .expect("a jpeg");

    out.into_inner()
}

pub fn drawn_by(title: &str, plus: &str, fonts: &[&str]) -> Vec<u8> {
    told_by(13, title, plus, fonts)
}

pub fn told_by(told: u32, title: &str, plus: &str, fonts: &[&str]) -> Vec<u8> {
    let mut out = vec![0];
    out.extend(headed(0x4D, 8));

    out.extend(word(4));
    out.extend_from_slice(&[0; 4]);
    out.extend(word(told));

    out.extend(line(title));
    out.extend(line("0000-0000"));
    out.extend(word(3));
    out.extend_from_slice(b"key");
    for which in 0..1 + game::SUB_FONTS {
        out.extend(line(fonts.get(which).copied().unwrap_or_default()));
    }
    out.extend(line("CharaChip/hand3.png"));

    if told >= 9 {
        out.extend(line(plus));
    }

    if told > 9 {
        out.extend(line(" "));
        out.extend(line(" "));
        out.extend(line("Press any key"));
        out.extend(line("A tale of two"));
    }

    if told > 13 {
        out.extend(line(" "));
    }

    let size = out.len();
    out.extend(word(0));
    out.extend(word(0));
    out.extend(word(2));
    out.extend_from_slice(&[0; 4]);
    out.extend(word(1000));
    out.extend(word(2000));
    out.extend_from_slice(&[7; 8]);

    let whole = (out.len() - 1) as u32;
    coder::put_word(&mut out, size, whole).expect("room for the size");

    out
}

pub fn older_archive() -> Vec<u8> {
    let mut out = vec![0; keying::HEAD_LEN as usize];
    out[..2].copy_from_slice(&archive::MARK.to_le_bytes());
    out[2..4].copy_from_slice(&6u16.to_le_bytes());

    out
}

pub fn lay_out(root: &Path) {
    let data = root.join(source::DATA);
    let basic = data.join(source::BASIC);
    fs::create_dir_all(data.join("MapData")).unwrap();
    fs::create_dir_all(&basic).unwrap();

    fs::write(
        data.join("MapData").join("Dungeon.mps"),
        map(&[&[command(
            101,
            &[],
            &["\u{6249}\u{306f}\u{9589}\u{307e}\u{3063}\u{3066}\u{3044}\u{308b}"],
        )]]),
    )
    .unwrap();

    fs::write(
        basic.join("CommonEvent.dat"),
        commons(&[&[command(
            102,
            &[0],
            &["\u{306f}\u{3044}", "\u{3044}\u{3044}\u{3048}"],
        )]]),
    )
    .unwrap();

    fs::write(
        basic.join("Game.dat"),
        game("\u{9060}\u{3044}\u{9053}", " + DLC", "Pixelify Sans"),
    )
    .unwrap();

    let (plan, data_body) = database(&[Type {
        name: "\u{30a2}\u{30a4}\u{30c6}\u{30e0}",
        fields: &["\u{540d}\u{524d}", "\u{5024}\u{6bb5}", "\u{8aac}\u{660e}"],
        words: &[0, 2],
        entries: &[&["\u{7dd1}\u{8336}", "HP\u{3092}30\u{56de}\u{5fa9}"]],
        named_by: None,
    }]);
    fs::write(basic.join("DataBase.project"), plan).unwrap();
    fs::write(basic.join("DataBase.dat"), data_body).unwrap();
}
