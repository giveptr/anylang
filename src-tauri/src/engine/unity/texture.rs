use crate::canvas::Canvas;
use crate::engine::unity::serial;
use crate::engine::unity::serial::{Object, Value};
use anyhow::{Context, Result, bail};

pub const RGBA32: i32 = 4;

const WIDEST: usize = 32768;
const DEEPEST: i32 = 16;

pub struct Payload {
    pub bytes: Vec<u8>,
    pub format: i32,
}

pub enum Held {
    Inline(usize),
    Streamed { path: String, at: u64, size: usize },
}

impl Held {
    pub fn size(&self) -> usize {
        match self {
            Self::Inline(size) => *size,
            Self::Streamed { size, .. } => *size,
        }
    }
}

pub struct Picture {
    pub name: String,
    pub wide: usize,
    pub high: usize,
    pub format: i32,
    mips: i32,
    pub held: Held,
    pub stripped: i32,
    whole: usize,
}

pub fn picture_of(object: &Object) -> Option<Picture> {
    if object.class_id != serial::TEXTURE_2D {
        return None;
    }

    pictured(&object.value()?)
}

pub fn pictured(value: &Value) -> Option<Picture> {
    read(value).filter(sound)
}

pub struct Unread {
    pub name: String,
    pub wide: u32,
    pub high: u32,
    pub format: i32,
}

pub fn unread(value: Option<&Value>) -> Option<Unread> {
    let Some(value) = value else {
        return Some(Unread {
            name: String::new(),
            wide: 0,
            high: 0,
            format: 0,
        });
    };

    if pictured(value).is_some() {
        return None;
    }

    let number = |at: &str| {
        value
            .field(at)
            .and_then(Value::number)
            .and_then(|held| u32::try_from(held).ok())
            .unwrap_or(0)
    };

    let (wide, high) = (number("m_Width"), number("m_Height"));
    if wide == 0 || high == 0 {
        return None;
    }

    Some(Unread {
        name: value
            .field("m_Name")
            .and_then(Value::text)
            .unwrap_or_default(),
        wide,
        high,
        format: i32::try_from(number("m_TextureFormat")).unwrap_or(0),
    })
}

fn read(value: &Value) -> Option<Picture> {
    let number = |at: &str| value.field(at).and_then(Value::number);

    let stream = value.field("m_StreamData");
    let path = stream
        .and_then(|one| one.field("path"))
        .and_then(Value::text)
        .unwrap_or_default();

    let held = match path.is_empty() {
        true => Held::Inline(match value.field("image data") {
            Some(Value::Bytes(raw)) => raw.len(),
            _ => 0,
        }),
        false => Held::Streamed {
            path,
            at: u64::try_from(stream?.field("offset")?.number()?).ok()?,
            size: usize::try_from(stream?.field("size")?.number()?).ok()?,
        },
    };

    Some(Picture {
        name: value.field("m_Name")?.text()?,
        wide: usize::try_from(number("m_Width")?).ok()?,
        high: usize::try_from(number("m_Height")?).ok()?,
        format: i32::try_from(number("m_TextureFormat")?).ok()?,
        mips: i32::try_from(number("m_MipCount")?).ok()?,
        held,
        stripped: number("m_MipsStripped")
            .and_then(|held| i32::try_from(held).ok())
            .unwrap_or(0),
        whole: usize::try_from(number("m_CompleteImageSize")?).ok()?,
    })
}

fn sound(held: &Picture) -> bool {
    let sized = (1..=WIDEST).contains(&held.wide) && (1..=WIDEST).contains(&held.high);
    let deep = (1..=DEEPEST).contains(&held.mips) && (0..held.mips).contains(&held.stripped);

    if !sized || !deep || shape_of(held.format).is_none() {
        return false;
    }

    let stored = held.held.size();
    let fits = stored == held.whole || stored == 0;

    match streams(held.format) {
        true => held.whole > 0 && fits,
        false => held.chain() == Some(held.whole) && fits,
    }
}

impl Picture {
    fn stored(&self) -> (usize, usize) {
        let step = u32::try_from(self.stripped).unwrap_or(0);
        let held = |side: usize| side.checked_shr(step).unwrap_or(0).max(1);

        (held(self.wide), held(self.high))
    }

    fn chain(&self) -> Option<usize> {
        let mut whole = 0usize;
        let mut wide = self.wide;
        let mut high = self.high;

        for level in 0..self.mips {
            if level >= self.stripped {
                whole = whole.checked_add(level_size(self.format, wide, high)?)?;
            }

            wide = (wide / 2).max(1);
            high = (high / 2).max(1);
        }

        Some(whole)
    }

    pub fn inside(&self, object: &Object) -> Option<Vec<u8>> {
        match self.held {
            Held::Inline(_) => match object.value()?.field("image data")? {
                Value::Bytes(raw) => Some(raw.clone()),
                _ => None,
            },
            Held::Streamed { .. } => None,
        }
    }

    pub fn drawn(&self, raw: &[u8], built_by: &str) -> Result<Canvas> {
        if streams(self.format) {
            let held = crunched(self.format, raw, self.wide, self.high, built_by)?;

            return Ok(Canvas::of(self.wide, self.high, held)?.flipped());
        }

        let (wide, high) = self.stored();
        let wanted = level_size(self.format, wide, high).ok_or_else(|| {
            anyhow::anyhow!("format {} is not one this reader knows", self.format)
        })?;

        let held = raw.get(..wanted).ok_or_else(|| {
            anyhow::anyhow!(
                "{} says it holds {wanted} byte(s) of pixels and only {} were found",
                self.name,
                raw.len()
            )
        })?;

        let held = drawn(self.format, held, wide, high)?.flipped();

        match (wide, high) == (self.wide, self.high) {
            true => Ok(held),
            false => held.scaled(self.wide, self.high),
        }
    }

    pub fn payload(&self, drawn: &Canvas) -> Result<Payload> {
        if drawn.wide != self.wide || drawn.high != self.high {
            bail!(
                "{} is {}x{} in the game and the picture handed back is {}x{}",
                self.name,
                self.wide,
                self.high,
                drawn.wide,
                drawn.high
            );
        }

        let format = match packs(self.format) {
            true => self.format,
            false => RGBA32,
        };

        let mut bytes = Vec::new();
        let mut level = drawn.flipped();
        let mut wide = self.wide;
        let mut high = self.high;

        for _ in 0..self.mips.max(1) {
            bytes.extend_from_slice(&packed(format, &level)?);

            wide = (wide / 2).max(1);
            high = (high / 2).max(1);
            level = level.scaled(wide, high)?;
        }

        Ok(Payload { bytes, format })
    }

    pub fn written(
        &self,
        object: &Object,
        held: &Payload,
        beside: Option<(&str, u64)>,
    ) -> Result<Vec<u8>> {
        let mut value = object
            .value()
            .ok_or_else(|| anyhow::anyhow!("{} no longer reads by its type tree", self.name))?;

        put_number(&mut value, "m_CompleteImageSize", held.bytes.len() as i64)?;
        put_number(&mut value, "m_TextureFormat", i64::from(held.format))?;
        if value.field("m_MipsStripped").is_some() {
            put_number(&mut value, "m_MipsStripped", 0)?;
        }
        if let Some(blob) = value.field_mut("m_PlatformBlob") {
            *blob = Value::Bytes(Vec::new());
        }

        let inside = value
            .field_mut("image data")
            .ok_or_else(|| anyhow::anyhow!("{} holds no picture to write to", self.name))?;
        *inside = Value::Bytes(match beside {
            Some(_) => Vec::new(),
            None => held.bytes.clone(),
        });

        let stream = value
            .field_mut("m_StreamData")
            .ok_or_else(|| anyhow::anyhow!("{} says nothing about a sidecar", self.name))?;

        match beside {
            Some((path, at)) => {
                put_number(stream, "offset", at as i64)?;
                put_number(stream, "size", held.bytes.len() as i64)?;
                stream.put("path", path);
            }
            None => {
                put_number(stream, "offset", 0)?;
                put_number(stream, "size", 0)?;
                stream.put("path", "");
            }
        }

        object.written(&value)
    }
}

fn put_number(value: &mut Value, at: &str, held: i64) -> Result<()> {
    let room = value
        .field_mut(at)
        .ok_or_else(|| anyhow::anyhow!("no {at} to write to"))?;
    *room = Value::Number(held);

    Ok(())
}

pub fn called(format: i32) -> String {
    let held = match format {
        1 => "Alpha8",
        2 => "ARGB4444",
        3 => "RGB24",
        4 => "RGBA32",
        5 => "ARGB32",
        6 => "ARGBFloat",
        7 => "RGB565",
        8 => "BGR24",
        9 => "R16",
        10 => "DXT1",
        11 => "DXT3",
        12 => "DXT5",
        13 => "RGBA4444",
        14 => "BGRA32",
        15 => "RHalf",
        16 => "RGHalf",
        17 => "RGBAHalf",
        18 => "RFloat",
        19 => "RGFloat",
        20 => "RGBAFloat",
        21 => "YUY2",
        22 => "RGB9e5Float",
        23 => "RGBFloat",
        24 => "BC6H",
        25 => "BC7",
        26 => "BC4",
        27 => "BC5",
        28 => "DXT1 Crunched",
        29 => "DXT5 Crunched",
        30 => "PVRTC_RGB2",
        31 => "PVRTC_RGBA2",
        32 => "PVRTC_RGB4",
        33 => "PVRTC_RGBA4",
        34 => "ETC_RGB4",
        35 => "ATC_RGB4",
        36 => "ATC_RGBA8",
        41 => "EAC_R",
        42 => "EAC_R signed",
        43 => "EAC_RG",
        44 => "EAC_RG signed",
        45 => "ETC2_RGB",
        60 => "ETC_RGB4 3DS",
        61 => "ETC_RGBA8 3DS",
        46 => "ETC2_RGBA1",
        47 => "ETC2_RGBA8",
        48..=59 => "ASTC",
        62 => "RG16",
        63 => "R8",
        64 => "ETC_RGB4 Crunched",
        65 => "ETC2_RGBA8 Crunched",
        66..=71 => "ASTC HDR",
        72 => "RG32",
        73 => "RGB48",
        74 => "RGBA64",
        75 => "R8 signed",
        76 => "RG16 signed",
        77 => "RGB24 signed",
        78 => "RGBA32 signed",
        79 => "R16 signed",
        80 => "RG32 signed",
        81 => "RGB48 signed",
        82 => "RGBA64 signed",
        _ => return format!("format {format}"),
    };

    held.to_string()
}

pub fn draws(format: i32) -> bool {
    matches!(
        format,
        1..=5 | 7..=14 | 24..=29 | 34..=36 | 41 | 43 | 45..=59 | 62..=65 | 66..=71
    )
}

enum Shape {
    Flat(usize),
    Block(usize, usize, usize),
    Stream,
}

fn shape_of(format: i32) -> Option<Shape> {
    let held = match format {
        1 | 63 | 75 => Shape::Flat(1),
        2 | 7 | 9 | 13 | 15 | 62 | 76 | 79 => Shape::Flat(2),
        3 | 8 | 77 => Shape::Flat(3),
        4 | 5 | 14 | 16 | 18 | 22 | 72 | 78 | 80 => Shape::Flat(4),
        73 | 81 => Shape::Flat(6),
        17 | 19 | 74 | 82 => Shape::Flat(8),
        23 => Shape::Flat(12),
        6 | 20 => Shape::Flat(16),
        60 => Shape::Block(4, 4, 8),
        61 => Shape::Block(4, 4, 16),
        28 | 29 | 64 | 65 => Shape::Stream,
        10 | 26 | 34 | 41 | 45 | 46 => Shape::Block(4, 4, 8),
        11 | 12 | 24 | 25 | 27 | 43 | 47 => Shape::Block(4, 4, 16),
        35 => Shape::Block(4, 4, 8),
        36 => Shape::Block(4, 4, 16),
        48 | 54 | 66 => Shape::Block(4, 4, 16),
        49 | 55 | 67 => Shape::Block(5, 5, 16),
        50 | 56 | 68 => Shape::Block(6, 6, 16),
        51 | 57 | 69 => Shape::Block(8, 8, 16),
        52 | 58 | 70 => Shape::Block(10, 10, 16),
        53 | 59 | 71 => Shape::Block(12, 12, 16),
        _ => return None,
    };

    Some(held)
}

fn level_size(format: i32, wide: usize, high: usize) -> Option<usize> {
    match shape_of(format)? {
        Shape::Flat(bytes) => wide.checked_mul(high)?.checked_mul(bytes),
        Shape::Block(across, down, bytes) => wide
            .div_ceil(across)
            .checked_mul(high.div_ceil(down))?
            .checked_mul(bytes),
        Shape::Stream => None,
    }
}

fn streams(format: i32) -> bool {
    matches!(shape_of(format), Some(Shape::Stream))
}

fn packs(format: i32) -> bool {
    matches!(format, 1 | 3 | 4 | 5 | 14 | 63)
}

fn packed(format: i32, drawn: &Canvas) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(drawn.pixels.len());

    for pixel in drawn.pixels.as_chunks::<4>().0 {
        let [red, green, blue, alpha] = *pixel;

        match format {
            1 => out.push(alpha),
            63 => out.push(red),
            3 => out.extend_from_slice(&[red, green, blue]),
            4 => out.extend_from_slice(&[red, green, blue, alpha]),
            5 => out.extend_from_slice(&[alpha, red, green, blue]),
            14 => out.extend_from_slice(&[blue, green, red, alpha]),
            _ => bail!("format {format} is not one this writer can pack a picture into"),
        }
    }

    Ok(out)
}

fn crunched(format: i32, raw: &[u8], wide: usize, high: usize, built_by: &str) -> Result<Vec<u8>> {
    let pixels = wide
        .checked_mul(high)
        .filter(|pixels| *pixels <= raw.len().saturating_mul(64))
        .with_context(|| {
            format!(
                "this picture claims {wide}x{high} pixels but carries only {} crunched byte(s)",
                raw.len()
            )
        })?;

    let mut held = vec![0u32; pixels];

    let done = match unity_crunch(format, built_by) {
        true => texture2ddecoder::decode_unity_crunch(raw, wide, high, &mut held),
        false => texture2ddecoder::decode_crunch(raw, wide, high, &mut held),
    };

    if let Err(why) = done {
        bail!("this picture does not unpack as format {format}: {why}");
    }

    Ok(spread(held))
}

fn unity_crunch(format: i32, built_by: &str) -> bool {
    if matches!(format, 64 | 65) {
        return true;
    }

    let mut walk = built_by.split('.').map(str::parse::<u32>);
    let Some(Ok(year)) = walk.next() else {
        return true;
    };
    let minor = walk.next().and_then(Result::ok).unwrap_or(0);

    (year, minor) >= (2017, 3)
}

fn spread(held: Vec<u32>) -> Vec<u8> {
    let mut out = Vec::with_capacity(held.len() * 4);

    for pixel in held {
        let [blue, green, red, alpha] = pixel.to_le_bytes();
        out.extend_from_slice(&[red, green, blue, alpha]);
    }

    out
}

fn drawn(format: i32, raw: &[u8], wide: usize, high: usize) -> Result<Canvas> {
    let held = match shape_of(format) {
        Some(Shape::Flat(bytes)) => flat(format, raw, bytes, wide, high)?,
        Some(Shape::Block(..)) => squished(format, raw, wide, high)?,
        Some(Shape::Stream) => bail!("format {format} unpacks as a stream, not as pixels"),
        None => bail!("format {format} is not one this reader knows"),
    };

    Canvas::of(wide, high, held)
}

fn flat(format: i32, raw: &[u8], bytes: usize, wide: usize, high: usize) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(wide * high * 4);

    for pixel in raw.chunks_exact(bytes) {
        let held: [u8; 4] = match format {
            1 => [255, 255, 255, pixel[0]],
            63 => [pixel[0], pixel[0], pixel[0], 255],
            9 => [pixel[1], pixel[1], pixel[1], 255],
            62 => [pixel[0], pixel[1], 0, 255],
            3 => [pixel[0], pixel[1], pixel[2], 255],
            8 => [pixel[2], pixel[1], pixel[0], 255],
            4 => [pixel[0], pixel[1], pixel[2], pixel[3]],
            5 => [pixel[1], pixel[2], pixel[3], pixel[0]],
            14 => [pixel[2], pixel[1], pixel[0], pixel[3]],
            7 => short(pixel, (5, 6, 5, 0)),
            2 => nibbles(pixel, true),
            13 => nibbles(pixel, false),
            _ => bail!("format {format} is not one this reader can draw yet"),
        };

        out.extend_from_slice(&held);
    }

    Ok(out)
}

fn short(pixel: &[u8], (red, green, blue, _alpha): (u32, u32, u32, u32)) -> [u8; 4] {
    let held = u32::from(u16::from_le_bytes([pixel[0], pixel[1]]));
    let blue_at = 0;
    let green_at = blue;
    let red_at = blue + green;

    let spread = |value: u32, bits: u32| -> u8 {
        let most = (1u32 << bits) - 1;
        ((value * 255 + most / 2) / most) as u8
    };

    [
        spread((held >> red_at) & ((1 << red) - 1), red),
        spread((held >> green_at) & ((1 << green) - 1), green),
        spread((held >> blue_at) & ((1 << blue) - 1), blue),
        255,
    ]
}

fn nibbles(pixel: &[u8], alpha_first: bool) -> [u8; 4] {
    let held = u16::from_le_bytes([pixel[0], pixel[1]]);
    let step = |shift: u32| -> u8 {
        let value = ((held >> shift) & 0xf) as u8;

        value << 4 | value
    };

    match alpha_first {
        true => [step(8), step(4), step(0), step(12)],
        false => [step(12), step(8), step(4), step(0)],
    }
}

fn squished(format: i32, raw: &[u8], wide: usize, high: usize) -> Result<Vec<u8>> {
    let mut held = vec![0u32; wide * high];

    let done = match format {
        10 => texture2ddecoder::decode_bc1(raw, wide, high, &mut held),
        11 => texture2ddecoder::decode_bc2(raw, wide, high, &mut held),
        12 => texture2ddecoder::decode_bc3(raw, wide, high, &mut held),
        24 => texture2ddecoder::decode_bc6_unsigned(raw, wide, high, &mut held),
        25 => texture2ddecoder::decode_bc7(raw, wide, high, &mut held),
        26 => texture2ddecoder::decode_bc4(raw, wide, high, &mut held),
        27 => texture2ddecoder::decode_bc5(raw, wide, high, &mut held),
        34 => texture2ddecoder::decode_etc1(raw, wide, high, &mut held),
        35 => texture2ddecoder::decode_atc_rgb4(raw, wide, high, &mut held),
        36 => texture2ddecoder::decode_atc_rgba8(raw, wide, high, &mut held),
        41 => texture2ddecoder::decode_eacr(raw, wide, high, &mut held),
        43 => texture2ddecoder::decode_eacrg(raw, wide, high, &mut held),
        45 => texture2ddecoder::decode_etc2_rgb(raw, wide, high, &mut held),
        46 => texture2ddecoder::decode_etc2_rgba1(raw, wide, high, &mut held),
        47 => texture2ddecoder::decode_etc2_rgba8(raw, wide, high, &mut held),
        48..=59 | 66..=71 => match shape_of(format) {
            Some(Shape::Block(across, down, _)) => {
                texture2ddecoder::decode_astc(raw, wide, high, across, down, &mut held)
            }
            _ => bail!("format {format} is not one this reader knows"),
        },
        _ => bail!("format {format} is not one this reader can draw yet"),
    };

    if let Err(why) = done {
        bail!("this picture does not decode as format {format}: {why}");
    }

    Ok(spread(held))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::unity::{fake, serial};

    #[test]
    fn a_texture_whose_first_mips_were_stripped_draws_at_the_size_the_game_draws_it() {
        let level = vec![9u8; 8 * 4 * 4];
        let object = Object::forged(
            serial::TEXTURE_2D,
            21,
            fake::a_texture(&fake::Drawn {
                mips: 2,
                stripped: 1,
                data: &level,
                ..fake::drawn("stripped", 16, 8, RGBA32)
            }),
        );

        let held = picture_of(&object).expect("a stripped texture is still a picture");
        let drawn = held
            .drawn(
                &held.inside(&object).expect("pixels inside the object"),
                serial::BUILT_BY,
            )
            .expect("it draws the level the file really holds");

        assert_eq!(
            (drawn.wide, drawn.high),
            (16, 8),
            "the game draws this one at 16x8 and the sprites cut from it are measured in those \
             numbers, so a smaller stored level has to come back scaled to them"
        );
    }

    #[test]
    fn every_format_this_reader_claims_to_draw_is_one_it_really_draws() {
        for format in 0..=90 {
            if streams(format) {
                assert!(
                    draws(format),
                    "format {format} ({}) unpacks as a stream, so it is one this reader draws",
                    called(format)
                );
                assert!(
                    crunched(format, &[0; 64], 4, 4, "2022.3.45f1").is_err(),
                    "format {format} has to turn away bytes that are not a crunched stream \
                     rather than hand back whatever it made of them"
                );
                continue;
            }

            let sized = level_size(format, 4, 4);
            let raw = vec![0u8; sized.unwrap_or(64)];
            let done = drawn(format, &raw, 4, 4).is_ok();

            assert_eq!(
                draws(format),
                done,
                "format {format} ({}) is listed as {} and draws as {done}, so a picture is either \
                 offered and then fails on screen or refused while it could have been shown",
                called(format),
                draws(format)
            );
        }
    }

    fn dotted(wide: usize, high: usize) -> Canvas {
        let mut held = Canvas::blank(wide, high);

        for line in 0..high {
            for column in 0..wide {
                let at = (line * wide + column) * 4;
                held.pixels[at] = (column * 17 % 256) as u8;
                held.pixels[at + 1] = (line * 29 % 256) as u8;
                held.pixels[at + 2] = 40;
                held.pixels[at + 3] = 255;
            }
        }

        held
    }

    fn forged(name: &str, wide: usize, high: usize, format: i32, mips: i32, data: &[u8]) -> Object {
        let body = fake::a_texture(&fake::Drawn {
            mips,
            data,
            ..fake::drawn(name, wide, high, format)
        });

        Object::forged(serial::TEXTURE_2D, 11, body)
    }

    fn streamed(name: &str, wide: usize, high: usize, format: i32, at: u64, size: usize) -> Object {
        let body = fake::a_texture(&fake::Drawn {
            whole: size,
            at,
            sidecar: "sharedassets0.assets.resS",
            ..fake::drawn(name, wide, high, format)
        });

        Object::forged(serial::TEXTURE_2D, 12, body)
    }

    #[test]
    fn a_texture_gives_up_the_name_size_and_format_the_game_holds_it_at() {
        let drawn = dotted(8, 4);
        let data = packed(RGBA32, &drawn.flipped()).expect("rgba packs");
        let object = forged("Background", 8, 4, RGBA32, 1, &data);

        let held = picture_of(&object).expect("a texture this reader wrote itself");

        assert_eq!(held.name, "Background");
        assert_eq!((held.wide, held.high), (8, 4));
        assert_eq!(held.format, RGBA32);
        assert!(matches!(held.held, Held::Inline(_)));
    }

    #[test]
    fn the_pixels_read_back_are_the_pixels_that_went_in() {
        let drawn = dotted(8, 4);
        let data = packed(RGBA32, &drawn.flipped()).expect("rgba packs");
        let object = forged("Background", 8, 4, RGBA32, 1, &data);
        let held = picture_of(&object).expect("a texture");

        let back = held
            .drawn(
                &held.inside(&object).expect("pixels inside the object"),
                serial::BUILT_BY,
            )
            .expect("it draws");

        assert_eq!(
            back.pixels, drawn.pixels,
            "Unity keeps its rows bottom up: a reader that forgot to flip would show every \
             picture in the game upside down"
        );
    }

    #[test]
    fn a_texture_whose_pixels_live_in_a_sidecar_names_the_file_and_the_spot() {
        let object = streamed(
            "sactx-0-15600x300",
            15600,
            300,
            RGBA32,
            811_890_000,
            15600 * 300 * 4,
        );
        let held = picture_of(&object).expect("a streamed texture");

        match held.held {
            Held::Streamed { path, at, size } => {
                assert_eq!(path, "sharedassets0.assets.resS");
                assert_eq!(at, 811_890_000);
                assert_eq!(size, 15600 * 300 * 4);
            }
            Held::Inline(_) => panic!("this one is streamed"),
        }
    }

    #[test]
    fn a_texture_that_does_not_add_up_is_refused_instead_of_shown_as_noise() {
        let drawn = dotted(8, 4);
        let data = packed(RGBA32, &drawn).expect("rgba packs");

        assert!(
            picture_of(&forged("Odd", 8, 5, RGBA32, 1, &data)).is_none(),
            "the numbers in the object have to describe the pixels beside them, or this reader is \
             reading some other field as a size and everything after it is noise"
        );
        assert!(
            picture_of(&forged("Odd", 8, 4, 999, 1, &data)).is_none(),
            "a format nobody named cannot be sized, so it cannot be checked either"
        );
    }

    #[test]
    fn only_a_texture_is_read_as_one() {
        let data = packed(RGBA32, &dotted(4, 4)).expect("rgba packs");
        let mut object = forged("Background", 4, 4, RGBA32, 1, &data);
        object.class_id = serial::TEXT_ASSET;

        assert!(
            picture_of(&object).is_none(),
            "a script that happens to hold numbers looking like a size is not a picture, and \
             rewriting it would put pixels where a reader's text belongs"
        );
    }

    #[test]
    fn a_picture_written_back_reads_out_as_the_picture_that_was_handed_over() {
        let was = dotted(16, 8);
        let data = packed(RGBA32, &was.flipped()).expect("rgba packs");
        let object = forged("Background", 16, 8, RGBA32, 1, &data);
        let held = picture_of(&object).expect("a texture");

        let mut fresh = dotted(16, 8);
        fresh.pixels[0..4].copy_from_slice(&[1, 2, 3, 4]);

        let payload = held.payload(&fresh).expect("it packs");
        let body = held
            .written(&object, &payload, None)
            .expect("it writes back");
        let fresh_object = Object::forged(serial::TEXTURE_2D, 11, body);
        let after = picture_of(&fresh_object).expect("it reads back");

        assert_eq!((after.wide, after.high), (16, 8));
        assert_eq!(after.format, RGBA32);

        let back = after
            .drawn(
                &after
                    .inside(&fresh_object)
                    .expect("pixels inside the object"),
                serial::BUILT_BY,
            )
            .expect("it draws");
        assert_eq!(
            back.pixels, fresh.pixels,
            "what the reader picked has to be what the game shows, to the pixel"
        );
    }

    #[test]
    fn a_streamed_texture_written_back_still_points_at_the_sidecar_it_shipped_with() {
        let object = streamed("Background", 8, 4, RGBA32, 4096, 8 * 4 * 4);
        let held = picture_of(&object).expect("a streamed texture");

        let payload = held.payload(&dotted(8, 4)).expect("it packs");
        assert_eq!(
            payload.bytes.len(),
            8 * 4 * 4,
            "a picture of the same size in the same format is the same number of bytes, which is \
             what lets it go back exactly where it came from"
        );

        let body = held
            .written(&object, &payload, Some(("sharedassets0.assets.resS", 4096)))
            .expect("it writes back");
        let after =
            picture_of(&Object::forged(serial::TEXTURE_2D, 12, body)).expect("it reads back");

        match &after.held {
            Held::Streamed { path, at, size } => {
                assert_eq!(
                    (path.as_str(), *at, *size),
                    ("sharedassets0.assets.resS", 4096, 8 * 4 * 4),
                    "pixels the game streams stay streamed, so the container this object lives in \
                     does not grow by a megabyte every time one button is replaced"
                );
            }
            Held::Inline(_) => panic!("it was streamed and has to stay streamed"),
        }
        assert!(
            after
                .inside(&Object::forged(serial::TEXTURE_2D, 12, Vec::new()))
                .is_none(),
            "and the pixels may not be written twice, once beside the game and once inside it"
        );
    }

    #[test]
    fn a_picture_of_the_wrong_size_is_refused_rather_than_stretched_in_silence() {
        let data = packed(RGBA32, &dotted(8, 4).flipped()).expect("rgba packs");
        let object = forged("Background", 8, 4, RGBA32, 1, &data);
        let held = picture_of(&object).expect("a texture");

        assert!(
            held.payload(&dotted(9, 4)).is_err(),
            "a sprite rect points at pixels by number, so a texture that changed size moves every \
             sprite cut out of it: the caller has to scale first and say so"
        );
    }

    #[test]
    fn a_compressed_texture_is_written_back_as_plain_pixels() {
        let mut data = vec![0u8; 8];
        data[1] = 0xf8;
        let object = forged("Squished", 4, 4, 10, 1, &data);
        let held = picture_of(&object).expect("a dxt1 texture");

        assert_eq!(held.format, 10);
        held.drawn(&data, serial::BUILT_BY).expect("dxt1 draws");

        let body = held
            .written(
                &object,
                &held.payload(&dotted(4, 4)).expect("it packs"),
                None,
            )
            .expect("it writes back");
        let after =
            picture_of(&Object::forged(serial::TEXTURE_2D, 11, body)).expect("it reads back");

        assert_eq!(
            after.format, RGBA32,
            "squishing a picture again would cost the reader quality this tool cannot give back, \
             and the engine reads whatever format the object names"
        );
    }

    #[test]
    fn the_mip_chain_is_written_out_again_so_nothing_looks_softer_than_it_did() {
        let mips: i32 = 3;
        let whole = (0..mips)
            .map(|level| {
                let wide = (16usize >> level).max(1);
                let high = (8usize >> level).max(1);

                wide * high * 4
            })
            .sum::<usize>();

        assert!(
            picture_of(&streamed("Chain", 16, 8, RGBA32, 0, whole)).is_none(),
            "one level of pixels is not three, so an object claiming one does not add up"
        );

        let object = Object::forged(
            serial::TEXTURE_2D,
            12,
            fake::a_texture(&fake::Drawn {
                mips,
                whole,
                at: 4096,
                sidecar: "sharedassets0.assets.resS",
                ..fake::drawn("Chain", 16, 8, RGBA32)
            }),
        );

        let held = picture_of(&object).expect("three levels that add up");
        assert_eq!(held.mips, mips);

        let fresh = held
            .written(
                &object,
                &held.payload(&dotted(16, 8)).expect("it packs"),
                None,
            )
            .expect("it writes back");
        let after =
            picture_of(&Object::forged(serial::TEXTURE_2D, 12, fresh)).expect("it reads back");

        assert_eq!(after.mips, mips);
        assert_eq!(
            after.held.size(),
            whole,
            "a texture the game shipped with three levels gets three levels back, or it turns \
             soft at a distance and the reader is left wondering why"
        );
    }

    #[test]
    fn every_format_this_reader_names_can_be_sized_and_every_size_is_the_one_unity_writes() {
        for (format, wide, high, wanted) in [
            (4, 32, 32, 4096),
            (3, 32, 32, 3072),
            (1, 32, 32, 1024),
            (10, 32, 32, 512),
            (12, 32, 32, 1024),
            (7, 32, 32, 2048),
            (25, 33, 33, 1296),
            (48, 8, 8, 64),
        ] {
            assert_eq!(
                level_size(format, wide, high),
                Some(wanted),
                "format {format} at {wide}x{high}"
            );
        }
    }
}
