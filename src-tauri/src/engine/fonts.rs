use crate::engine::{Extra, Font, Install, Swap, worth};
use crate::hash::xxh3;
use crate::scope::slashed;
use crate::{backup, walk};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::LazyLock;
use walkdir::WalkDir;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Fonts {
    #[serde(default)]
    pub swaps: Vec<Swap>,
}

impl Fonts {
    pub fn sent_to(&self, name: &str) -> Option<&str> {
        self.swaps
            .iter()
            .find(|one| one.from == name)
            .and_then(|swap| worth(&swap.to))
    }

    pub fn every(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();

        for one in self
            .swaps
            .iter()
            .map(|swap| swap.to.as_str())
            .filter_map(worth)
        {
            if !out.contains(&one) {
                out.push(one);
            }
        }

        out
    }

    pub fn one_for_every(&self, faces: &[Font]) -> Option<&str> {
        let mut sent = faces.iter().map(|one| self.sent_to(&one.name));
        let first = sent.next().flatten()?;

        sent.all(|one| one == Some(first)).then_some(first)
    }

    pub async fn picked(&self) -> Result<BTreeMap<String, Vec<u8>>> {
        let mut bodies = BTreeMap::new();

        for one in self.every() {
            let body = tokio::fs::read(one)
                .await
                .with_context(|| format!("reading {one}"))?;

            bodies.insert(one.to_string(), body);
        }

        Ok(bodies)
    }
}

const FACES: [&str; 5] = ["ttf", "otf", "ttc", "woff", "woff2"];

const LIFTED: &str = "faces";

const NAMED_AT_MOST: usize = 32;

pub const CARRIED: &str = env!("CARGO_PKG_NAME");

const COLLECTION: [u8; 4] = *b"ttcf";
const NAME: [u8; 4] = *b"name";
const HEAD: [u8; 4] = *b"head";
const HHEA: [u8; 4] = *b"hhea";
const UNITS_PER_EM: usize = 18;
const FAMILY: u16 = 1;
const WINDOWS: u16 = 3;
const MACINTOSH: u16 = 1;
const ENGLISH: u16 = 0x409;

pub fn untouched(store: &Path, game_dir: &Path, at: &Path) -> PathBuf {
    backup::original_at_now(store, game_dir, at).unwrap_or_else(|_| at.to_path_buf())
}

pub fn is_face(at: &Path) -> bool {
    at.extension()
        .is_some_and(|kind| FACES.iter().any(|face| kind.eq_ignore_ascii_case(face)))
}

pub fn ours(name: &str) -> bool {
    let at = Path::new(name);
    let carried = at
        .file_stem()
        .and_then(OsStr::to_str)
        .and_then(|stem| stem.strip_prefix(CARRIED))
        .is_some_and(|rest| rest.starts_with('-'));

    carried && is_face(at)
}

#[cfg(test)]
pub static NOTHING: LazyLock<Fonts> = LazyLock::new(Fonts::default);

pub fn landed(fonts: &Fonts) -> Vec<(String, String)> {
    landings(&fonts.every(), by_prefix(CARRIED))
}

pub fn carried(placed: &[(String, String)], into: &Path) -> Vec<Extra> {
    placed
        .iter()
        .map(|(from, name)| Extra::Copy {
            from: PathBuf::from(from),
            at: into.join(name),
        })
        .collect()
}

pub async fn tidied(within: &Path, at: &Install<'_>) -> u32 {
    let keeping: Vec<String> = match at.reverting {
        true => Vec::new(),
        false => landed(at.fonts).into_iter().map(|(_, name)| name).collect(),
    };

    let mut let_go = 0;

    let mut listed = match tokio::fs::read_dir(within).await {
        Ok(listed) => listed,
        Err(_) => return 0,
    };

    while let Ok(Some(one)) = listed.next_entry().await {
        if !one.file_type().await.is_ok_and(|kind| kind.is_file()) {
            continue;
        }

        let one = one.path();
        let named = one.file_name().and_then(OsStr::to_str);
        let Some(named) = named.filter(|named| ours(named)) else {
            continue;
        };

        if keeping.iter().any(|kept| kept == named) {
            continue;
        }

        match walk::removed(&one).await {
            Ok(true) => let_go += 1,
            Ok(false) => {}
            Err(why) => at.progress.warn(at.doing, &format!("{why:#}")),
        }
    }

    if let_go > 0 {
        at.progress
            .info(at.doing, &format!("{let_go} font(s) we carried in let go"));
    }

    let_go
}

pub fn lifted_in(store: &Path) -> PathBuf {
    store.join(LIFTED)
}

fn slug(name: &str) -> String {
    name.chars()
        .take(NAMED_AT_MOST)
        .map(|one| match one.is_ascii_alphanumeric() {
            true => one.to_ascii_lowercase(),
            false => '-',
        })
        .collect()
}

fn kind(body: &[u8]) -> &'static str {
    match body.get(..4) {
        Some(head) if head == b"OTTO" => "otf",
        Some(head) if head == b"ttcf" => "ttc",
        _ => "ttf",
    }
}

pub fn lift(store: &Path, name: &str, body: &[u8]) -> Option<PathBuf> {
    let at = lifted_in(store).join(format!("{}-{}.{}", slug(name), xxh3(body), kind(body)));

    if std::fs::metadata(&at).is_ok_and(|found| found.len() == body.len() as u64) {
        return Some(at);
    }

    std::fs::create_dir_all(lifted_in(store)).ok()?;
    std::fs::write(&at, body).ok()?;

    Some(at)
}

pub fn swept(store: &Path, keeping: &[PathBuf]) {
    let Ok(listed) = std::fs::read_dir(lifted_in(store)) else {
        return;
    };

    for one in listed.filter_map(std::result::Result::ok) {
        let at = one.path();
        if at.is_file() && !keeping.contains(&at) {
            let _ = std::fs::remove_file(&at);
        }
    }
}

pub fn faces(game_dir: &Path, from: &Path) -> Vec<Font> {
    let mut found: Vec<Font> = WalkDir::new(from)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|one| one.file_type().is_file() && is_face(one.path()))
        .filter_map(|one| {
            let at = one.path().strip_prefix(game_dir).ok()?;

            Some(Font {
                name: one.file_name().to_string_lossy().to_string(),
                builtin: false,
                at: slashed(at),
                shown: one.path().to_string_lossy().to_string(),
            })
        })
        .collect();

    found.sort_by(|a, b| a.at.cmp(&b.at));

    found
}

fn beside(name: &str, again: usize) -> String {
    match name.rsplit_once('.') {
        Some((stem, kind)) => format!("{stem}-{again}.{kind}"),
        None => format!("{name}-{again}"),
    }
}

pub fn landings(each: &[&str], named: impl Fn(&Path) -> Option<String>) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();

    for from in each {
        let from = from.trim();
        if from.is_empty() || out.iter().any(|(taken, _)| taken == from) {
            continue;
        }

        let Some(name) = named(Path::new(from)).filter(|name| !name.is_empty()) else {
            continue;
        };

        let mut at = name.clone();
        let mut again = 1;
        while out.iter().any(|(_, taken)| *taken == at) {
            again += 1;
            at = beside(&name, again);
        }

        out.push((from.to_string(), at));
    }

    out
}

pub fn by_name(at: &Path) -> Option<String> {
    Some(at.file_name()?.to_string_lossy().to_string())
}

fn by_prefix(stem: &str) -> impl Fn(&Path) -> Option<String> + '_ {
    move |at| {
        Some(format!(
            "{stem}-{}.{}",
            at.file_stem()?.to_string_lossy(),
            at.extension()?.to_string_lossy()
        ))
    }
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn i16_at(bytes: &[u8], at: usize) -> Option<i16> {
    Some(i16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn table(bytes: &[u8], tag: [u8; 4]) -> Option<&[u8]> {
    let head = match bytes.get(..4)? == COLLECTION {
        true => u32_at(bytes, 12)? as usize,
        false => 0,
    };

    let count = u16_at(bytes, head + 4)? as usize;

    for which in 0..count {
        let record = head + 12 + which * 16;
        if bytes.get(record..record + 4)? != tag {
            continue;
        }

        let at = u32_at(bytes, record + 8)? as usize;
        let length = u32_at(bytes, record + 12)? as usize;

        return bytes.get(at..at + length);
    }

    None
}

fn name_table(bytes: &[u8]) -> Option<&[u8]> {
    table(bytes, NAME)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Metrics {
    upm: u16,
    ascent: i16,
    descent: i16,
    gap: i16,
}

impl Metrics {
    pub fn drawn_at(&self, size: f32) -> Drawn {
        let scale = size / f32::from(self.upm);
        let whole = i32::from(self.ascent) - i32::from(self.descent) + i32::from(self.gap);

        Drawn {
            line: whole as f32 * scale,
            ascent: f32::from(self.ascent) * scale,
            descent: f32::from(self.descent) * scale,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Drawn {
    pub line: f32,
    pub ascent: f32,
    pub descent: f32,
}

pub fn metrics(bytes: &[u8]) -> Option<Metrics> {
    let head = table(bytes, HEAD)?;
    let hhea = table(bytes, HHEA)?;

    Some(Metrics {
        upm: u16_at(head, UNITS_PER_EM).filter(|upm| *upm != 0)?,
        ascent: i16_at(hhea, 4)?,
        descent: i16_at(hhea, 6)?,
        gap: i16_at(hhea, 8)?,
    })
}

fn spelled(platform: u16, raw: &[u8]) -> Option<String> {
    if platform == MACINTOSH {
        return raw.is_ascii().then(|| String::from_utf8_lossy(raw).into());
    }

    let wide: Vec<u16> = raw
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_be_bytes(*pair))
        .collect();

    String::from_utf16(&wide).ok()
}

pub fn family(bytes: &[u8]) -> Option<String> {
    ranked_families(bytes)
        .into_iter()
        .min_by_key(|(rank, _)| Reverse(*rank))
        .map(|(_, name)| name)
}

pub fn families(bytes: &[u8]) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();

    for (_, name) in ranked_families(bytes) {
        if !found.contains(&name) {
            found.push(name);
        }
    }

    found
}

fn ranked_families(bytes: &[u8]) -> Vec<(u8, String)> {
    let Some(table) = name_table(bytes) else {
        return Vec::new();
    };
    let (Some(count), Some(store)) = (u16_at(table, 2), u16_at(table, 4)) else {
        return Vec::new();
    };
    let (count, store) = (count as usize, store as usize);
    let mut found = Vec::new();

    for which in 0..count {
        let record = 6 + which * 12;
        let (Some(platform), Some(language), Some(name_id)) = (
            u16_at(table, record),
            u16_at(table, record + 4),
            u16_at(table, record + 6),
        ) else {
            break;
        };

        if name_id != FAMILY {
            continue;
        }

        let (Some(length), Some(offset)) = (u16_at(table, record + 8), u16_at(table, record + 10))
        else {
            break;
        };
        let (length, at) = (length as usize, store + offset as usize);
        let Some(raw) = table.get(at..at + length) else {
            continue;
        };

        let rank = match (platform, language) {
            (WINDOWS, ENGLISH) => 3,
            (WINDOWS, _) => 2,
            (MACINTOSH, _) => 1,
            _ => 0,
        };

        if let Some(name) = spelled(platform, raw).filter(|name| !name.trim().is_empty()) {
            found.push((rank, name.trim().to_string()));
        }
    }

    found
}

#[cfg(test)]
pub mod fake {
    use crate::engine::fonts::{
        ENGLISH, FAMILY, HEAD, HHEA, MACINTOSH, NAME, UNITS_PER_EM, WINDOWS,
    };

    fn record(platform: u16, language: u16, name_id: u16, at: usize, length: usize) -> Vec<u8> {
        let mut out = Vec::new();
        for one in [platform, 1, language, name_id, length as u16, at as u16] {
            out.extend_from_slice(&one.to_be_bytes());
        }

        out
    }

    pub fn font(names: &[(u16, u16, u16, &str)]) -> Vec<u8> {
        let mut records = Vec::new();
        let mut store = Vec::new();

        for (platform, language, name_id, text) in names {
            let raw: Vec<u8> = match *platform {
                MACINTOSH => text.as_bytes().to_vec(),
                _ => text.encode_utf16().flat_map(u16::to_be_bytes).collect(),
            };

            records.extend(record(
                *platform,
                *language,
                *name_id,
                store.len(),
                raw.len(),
            ));
            store.extend(raw);
        }

        let mut table = Vec::new();
        table.extend_from_slice(&0u16.to_be_bytes());
        table.extend_from_slice(&(names.len() as u16).to_be_bytes());
        table.extend_from_slice(&((6 + records.len()) as u16).to_be_bytes());
        table.extend(records);
        table.extend(store);

        sfnt(&[(NAME, table)])
    }

    fn sfnt(tables: &[([u8; 4], Vec<u8>)]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&0x0001_0000u32.to_be_bytes());
        out.extend_from_slice(&(tables.len() as u16).to_be_bytes());
        out.extend_from_slice(&[0; 6]);

        let mut at = 12 + 16 * tables.len();
        for (tag, body) in tables {
            out.extend_from_slice(tag);
            out.extend_from_slice(&0u32.to_be_bytes());
            out.extend_from_slice(&(at as u32).to_be_bytes());
            out.extend_from_slice(&(body.len() as u32).to_be_bytes());
            at += body.len().next_multiple_of(4);
        }

        for (which, (_, body)) in tables.iter().enumerate() {
            out.extend_from_slice(body);

            if which + 1 < tables.len() {
                while !out.len().is_multiple_of(4) {
                    out.push(0);
                }
            }
        }

        out
    }

    pub fn called(family: &str) -> Vec<u8> {
        font(&[(WINDOWS, ENGLISH, FAMILY, family)])
    }

    pub fn measured(family: &str, upm: u16, ascent: i16, descent: i16, gap: i16) -> Vec<u8> {
        let mut head = vec![0u8; 54];
        head[UNITS_PER_EM..UNITS_PER_EM + 2].copy_from_slice(&upm.to_be_bytes());

        let mut hhea = vec![0u8; 36];
        hhea[4..6].copy_from_slice(&ascent.to_be_bytes());
        hhea[6..8].copy_from_slice(&descent.to_be_bytes());
        hhea[8..10].copy_from_slice(&gap.to_be_bytes());

        let named = called(family);
        let table = named[28..].to_vec();

        sfnt(&[(HEAD, head), (HHEA, hhea), (NAME, table)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::fonts::fake::font;

    fn face(name: &str) -> Font {
        Font {
            name: name.to_string(),
            builtin: false,
            at: format!("Fonts/{name}"),
            shown: String::new(),
        }
    }

    #[test]
    fn the_lines_a_font_asks_for_are_the_ones_unity_bakes_beside_it() {
        let liberation = fake::measured("Liberation Sans", 2048, 1854, -434, 67);
        let drawn = metrics(&liberation)
            .expect("a font says how tall it wants its lines")
            .drawn_at(16.0);

        assert!((drawn.line - 18.398).abs() < 0.001, "{}", drawn.line);
        assert!((drawn.ascent - 14.484).abs() < 0.001, "{}", drawn.ascent);
        assert!((drawn.descent + 3.391).abs() < 0.001, "{}", drawn.descent);

        let dos = fake::measured("Perfect DOS VGA 437", 1024, 768, -256, 0);
        let drawn = metrics(&dos).expect("a font").drawn_at(16.0);

        assert!((drawn.line - 16.0).abs() < 0.001, "{}", drawn.line);
        assert!((drawn.ascent - 12.0).abs() < 0.001, "{}", drawn.ascent);
        assert!((drawn.descent + 4.0).abs() < 0.001, "{}", drawn.descent);
    }

    #[test]
    fn a_font_that_says_nothing_about_its_lines_is_not_measured_at_all() {
        assert!(
            metrics(&fake::called("Nameless Only")).is_none(),
            "a file carrying no head or hhea gives nothing to work out a line height from, and \
             guessing one would write a number Unity never would"
        );
        assert!(metrics(b"not a font at all").is_none());
        assert!(
            metrics(&fake::measured("Broken", 0, 100, -20, 0)).is_none(),
            "a font claiming no units to the em would divide by nothing"
        );
    }

    #[test]
    fn only_a_name_this_writer_writes_counts_as_ours() {
        assert!(ours(&format!("{CARRIED}-Sarabun-Medium.ttf")));
        assert!(
            ours(&format!("{CARRIED}-sarabun-2.otf")),
            "two picks of one name land apart under a numbered tail, and both are ours"
        );

        for named in ["patch.ttf", "patch-2.ttf", "NIAGSOL.TTF", "sarabun.ttf"] {
            assert!(
                !ours(named),
                "{named} is a font the game ships, and letting go of one would leave the game \
                 drawing with nothing"
            );
        }
        assert!(
            !ours(&format!("{CARRIED}.ttf")),
            "a bare stem is a name this writer never makes, so it can only be someone else's"
        );
        assert!(
            !ours(&format!("{CARRIED}-notes.txt")),
            "only a font file is ever ours to take"
        );
    }

    fn sending(swaps: &[(&str, &str)]) -> Fonts {
        Fonts {
            swaps: swaps
                .iter()
                .map(|(from, to)| Swap {
                    from: (*from).to_string(),
                    to: (*to).to_string(),
                })
                .collect(),
        }
    }

    #[test]
    fn one_font_answers_for_every_name_only_when_every_face_was_sent_to_it() {
        let listed = [face("a.ttf"), face("b.ttf")];

        assert_eq!(
            sending(&[("a.ttf", "/x/Noto.ttf"), ("b.ttf", "/x/Noto.ttf")]).one_for_every(&listed),
            Some("/x/Noto.ttf"),
            "every face went to the same file, so a name this game ships no file for can be sent \
             there too"
        );
        assert_eq!(
            sending(&[("a.ttf", "/x/Noto.ttf"), ("b.ttf", "/y/Charm.otf")]).one_for_every(&listed),
            None,
            "the faces went to different files, so no single one may stand in for a name nobody \
             picked for"
        );
        assert_eq!(
            sending(&[("a.ttf", "/x/Noto.ttf")]).one_for_every(&listed),
            None,
            "b was left alone, so answering for it would change a face the reader never touched"
        );
        assert_eq!(
            sending(&[]).one_for_every(&listed),
            None,
            "nothing was picked at all"
        );
        assert_eq!(
            sending(&[("a.ttf", "/x/Noto.ttf")]).one_for_every(&[]),
            None,
            "a game showing no face of its own gives nothing to read a pick from"
        );
    }

    #[test]
    fn a_font_asked_for_twice_lands_once_and_two_of_a_name_never_collide() {
        let each = [
            " /fonts/sarabun.ttf ",
            "/fonts/sarabun.ttf",
            "",
            "/elsewhere/sarabun.ttf",
            "/fonts/comic.otf",
        ];

        assert_eq!(
            landings(&each, by_name),
            vec![
                ("/fonts/sarabun.ttf".to_string(), "sarabun.ttf".to_string()),
                (
                    "/elsewhere/sarabun.ttf".to_string(),
                    "sarabun-2.ttf".to_string()
                ),
                ("/fonts/comic.otf".to_string(), "comic.otf".to_string()),
            ],
            "two fonts of the same name would overwrite each other where they land"
        );

        assert_eq!(
            landings(&each, by_prefix(CARRIED))
                .into_iter()
                .map(|(_, at)| at)
                .collect::<Vec<String>>(),
            vec![
                format!("{CARRIED}-sarabun.ttf"),
                format!("{CARRIED}-sarabun-2.ttf"),
                format!("{CARRIED}-comic.otf"),
            ],
            "under a stem of ours two fonts of one name still land apart"
        );

        assert!(landings(&["/fonts/nameless"], by_prefix(CARRIED)).is_empty());
    }

    #[test]
    fn a_font_carried_in_is_named_after_the_one_it_came_from_so_dropping_a_pick_moves_nothing() {
        let both = ["/fonts/sarabun.ttf", "/elsewhere/noto.ttf"];

        assert_eq!(
            landings(&both, by_prefix(CARRIED))
                .into_iter()
                .map(|(_, at)| at)
                .collect::<Vec<String>>(),
            vec![
                format!("{CARRIED}-sarabun.ttf"),
                format!("{CARRIED}-noto.ttf")
            ]
        );
        assert_eq!(
            landings(&both[1..], by_prefix(CARRIED)),
            vec![(
                "/elsewhere/noto.ttf".to_string(),
                format!("{CARRIED}-noto.ttf")
            )],
            "letting go of the first pick may not rename the second: the game keeps only the \
             names the picks make now, so a renamed file is left behind for the engine to find"
        );
    }

    #[test]
    fn every_name_a_font_answers_to_is_offered_because_a_game_may_ask_for_any_of_them() {
        let japanese = "\u{ff2d}\u{ff33} \u{30b4}\u{30b7}\u{30c3}\u{30af}";
        let bytes = font(&[
            (MACINTOSH, 0, FAMILY, "Mac Name"),
            (WINDOWS, 0x411, FAMILY, japanese),
            (WINDOWS, ENGLISH, FAMILY, "MS Gothic"),
            (WINDOWS, ENGLISH, 4, "MS Gothic Regular"),
        ]);

        assert_eq!(
            families(&bytes),
            vec![
                "Mac Name".to_string(),
                japanese.to_string(),
                "MS Gothic".to_string()
            ],
            "a Wolf game names its face in whichever of these the author saw, so matching only \
             the English one leaves the file unfound"
        );
        assert_eq!(
            family(&bytes).as_deref(),
            Some("MS Gothic"),
            "the one name to swap by is still the one the engine asks for"
        );
        assert!(families(b"not a font at all").is_empty());
    }

    #[test]
    fn a_font_with_only_a_japanese_windows_name_still_answers() {
        let bytes = font(&[(
            WINDOWS,
            0x411,
            FAMILY,
            "\u{ff2d}\u{ff33} \u{30b4}\u{30b7}\u{30c3}\u{30af}",
        )]);

        assert_eq!(
            family(&bytes).as_deref(),
            Some("\u{ff2d}\u{ff33} \u{30b4}\u{30b7}\u{30c3}\u{30af}")
        );
        assert_eq!(family(b"not a font at all"), None);
    }
}
