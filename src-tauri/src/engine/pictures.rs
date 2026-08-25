use crate::canvas::{self, Canvas};
use crate::engine::{Prepare, Swap, worth};
use crate::progress::Source;
use crate::store;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(test)]
use std::sync::LazyLock;
use std::{fs, io};

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Shot {
    pub key: String,
    pub holder: String,
    pub name: String,
    pub kind: String,
    pub atlas: String,
    pub wide: u32,
    pub high: u32,
    pub format: String,
    pub saved_as: String,
    pub locked: Option<String>,
    pub drawable: bool,
    #[serde(default)]
    pub at: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Pictures {
    #[serde(default)]
    pub swaps: Vec<Swap>,
    #[serde(default)]
    pub marked: Vec<String>,
}

#[cfg(test)]
pub static NOTHING: LazyLock<Pictures> = LazyLock::new(Pictures::default);

const KIND: &str = "Picture";
const APART: char = '|';

pub struct Ledger(pub &'static str);

impl Ledger {
    fn at(&self, store: &Path) -> PathBuf {
        store.join(self.0)
    }

    pub async fn remember(&self, store: &Path, shots: &[Shot]) -> Result<()> {
        let body =
            serde_json::to_string(shots).context("listing the pictures a game draws with")?;

        store::write_atomically(&self.at(store), body).await
    }

    pub fn remembered(&self, store: &Path) -> Vec<Shot> {
        let Ok(body) = fs::read_to_string(self.at(store)) else {
            return Vec::new();
        };

        serde_json::from_str(&body).unwrap_or_default()
    }
}

pub fn key_of(holder: &str, inside: Option<&str>) -> String {
    match inside {
        Some(inside) => format!("{holder}{APART}{inside}"),
        None => holder.to_string(),
    }
}

pub fn packed_in(key: &str) -> Option<(&str, &str)> {
    key.split_once(APART)
}

pub const HEAD: usize = 1024;

pub struct Head {
    pub raw: Vec<u8>,
    pub whole: bool,
}

pub fn read_up_to(file: &mut impl io::Read, most: u64) -> io::Result<Vec<u8>> {
    use std::io::Read;

    let mut out = Vec::with_capacity(usize::try_from(most).unwrap_or(0));
    file.by_ref().take(most).read_to_end(&mut out)?;

    Ok(out)
}

pub fn head_of(at: &Path) -> io::Result<Head> {
    let mut file = fs::File::open(at)?;
    let raw = read_up_to(&mut file, HEAD as u64)?;
    let whole = raw.len() < HEAD;

    Ok(Head { raw, whole })
}

pub enum Measured {
    Sized {
        wide: u32,
        high: u32,
        format: String,
    },
    Unopened {
        format: String,
    },
    Unknown,
}

impl Measured {
    pub fn sized(&self) -> Option<(u32, u32)> {
        match self {
            Self::Sized { wide, high, .. } if *wide > 0 && *high > 0 => Some((*wide, *high)),
            _ => None,
        }
    }

    pub fn format(&self) -> &str {
        match self {
            Self::Sized { format, .. } | Self::Unopened { format } => format,
            Self::Unknown => "",
        }
    }
}

pub fn measured(raw: &[u8]) -> Measured {
    let Some(kind) = canvas::kind_of(raw) else {
        return Measured::Unknown;
    };
    let format = kind.to_uppercase();

    match canvas::measured(raw) {
        Some((wide, high)) if wide > 0 && high > 0 => Measured::Sized { wide, high, format },
        _ => Measured::Unopened { format },
    }
}

pub fn measured_or(head: &Head, rest: impl FnOnce() -> Option<Vec<u8>>) -> Measured {
    let held = measured(&head.raw);
    if head.whole || held.sized().is_some() {
        return held;
    }

    match rest() {
        Some(raw) => measured(&raw),
        None => held,
    }
}

fn complaint(held: &Measured) -> Option<String> {
    if held.sized().is_some() {
        return None;
    }

    match held {
        Measured::Unopened { format } if !canvas::reads(format) => Some(format!(
            "a {format} drawing is built from lines rather than pixels, so there is no picture \
             here to put another one over"
        )),
        Measured::Unopened { format } => Some(format!(
            "this file starts like a {format} and stops before it says its size, so there is \
             nothing here to show and nothing safe to write over it"
        )),
        _ => Some(
            "these bytes are not a picture this reader can open, so there is nothing here to show \
             and nothing safe to write over it"
                .to_string(),
        ),
    }
}

fn refusal(held: &Measured) -> Option<String> {
    let format = held.format();

    match held.sized().is_some() && !canvas::writes(format) {
        true => Some(format!(
            "this game ships it as a {format}, which this build can read but not write back, so \
             it is shown here and left as it shipped"
        )),
        false => None,
    }
}

pub struct Named {
    pub key: String,
    pub holder: String,
    pub name: String,
    pub at: String,
}

impl Named {
    pub fn beside(key: String, holder: &str, at: &str) -> Self {
        Self {
            key,
            holder: holder.to_string(),
            name: at.rsplit('/').next().unwrap_or(at).to_string(),
            at: at.to_string(),
        }
    }
}

pub fn unread(named: Named, why: impl std::fmt::Display) -> Shot {
    shot(
        named,
        &Measured::Unknown,
        Some(format!("this picture could not be read: {why}")),
    )
}

pub fn of_file(named: Named, at: &Path) -> Shot {
    match head_of(at) {
        Ok(head) => shot(named, &measured_or(&head, || fs::read(at).ok()), None),
        Err(why) => unread(named, why),
    }
}

pub fn shot(named: Named, held: &Measured, refused: Option<String>) -> Shot {
    let (wide, high) = held.sized().unwrap_or((0, 0));

    Shot {
        key: named.key,
        holder: named.holder,
        name: named.name,
        at: named.at,
        kind: KIND.to_string(),
        atlas: String::new(),
        wide,
        high,
        format: held.format().to_string(),
        saved_as: held.format().to_lowercase(),
        locked: refused
            .or_else(|| refusal(held))
            .or_else(|| complaint(held)),
        drawable: held.sized().is_some(),
    }
}

pub enum Handed {
    Shipped(Vec<u8>),
    Drawn(Arc<Canvas>),
}

impl Handed {
    pub fn drawn(self) -> Result<Arc<Canvas>> {
        match self {
            Self::Shipped(raw) => Canvas::read(&raw).map(Arc::new),
            Self::Drawn(held) => Ok(held),
        }
    }

    pub fn drawn_within(self, most: usize) -> Result<Canvas> {
        match self {
            Self::Shipped(raw) => Canvas::read_within(&raw, most),
            Self::Drawn(held) => match most {
                0 => Ok(Arc::unwrap_or_clone(held)),
                most => held.within(most),
            },
        }
    }
}

pub struct Found {
    pub shots: Vec<Shot>,
    pub shut: Vec<String>,
}

pub async fn remember(
    at: &Prepare<'_>,
    ledger: &Ledger,
    shots: &[Shot],
    shut: &[String],
) -> Result<()> {
    for why in shut {
        at.progress.warn(Source::Prepare, why);
    }

    if let Some(said) = counted(shots) {
        at.progress.info(Source::Prepare, &said);
    }

    ledger.remember(at.store, shots).await
}

pub fn counted(shots: &[Shot]) -> Option<String> {
    if shots.is_empty() {
        return None;
    }

    let locked = shots.iter().filter(|one| one.locked.is_some()).count();
    let mut said = format!("{} picture(s)", shots.len());
    if locked > 0 {
        said.push_str(&format!(
            " ({locked} of them can be viewed but not replaced)"
        ));
    }

    Some(said)
}

pub fn fitted(shipped: &str, sized: Option<(u32, u32)>, picked: &[u8]) -> Result<Vec<u8>> {
    if let Some(mine) = measured(picked).sized()
        && sized.is_none_or(|wanted| mine == wanted)
        && canvas::kind_of(picked).is_some_and(|kind| canvas::same_format(kind, shipped))
    {
        return Ok(picked.to_vec());
    }

    let held = Canvas::read(picked)?;
    let same =
        sized.is_none_or(|(wide, high)| held.wide as u32 == wide && held.high as u32 == high);

    if !canvas::writes(shipped) {
        let said = match sized {
            Some((wide, high)) => format!(" of exactly {wide}x{high}"),
            None => String::new(),
        };

        bail!(
            "this game draws that picture as {shipped}, and the only file that can go in its \
             place is a {shipped}{said}"
        );
    }

    match sized.filter(|_| !same) {
        Some((wide, high)) => held
            .scaled(wide as usize, high as usize)?
            .written_as(shipped),
        None => held.written_as(shipped),
    }
}

pub struct Chosen {
    pub raw: Vec<u8>,
    pub drawn: Canvas,
}

impl Pictures {
    pub fn filled(&self) -> Vec<(&str, &str)> {
        self.swaps
            .iter()
            .filter_map(|one| worth(&one.to).map(|at| (one.from.as_str(), at)))
            .collect()
    }

    pub async fn chosen(&self) -> (BTreeMap<String, Chosen>, Vec<String>) {
        let mut out: BTreeMap<String, Chosen> = BTreeMap::new();
        let mut complaints = Vec::new();
        let mut read = HashSet::new();

        for (_, at) in self.filled() {
            if !read.insert(at) {
                continue;
            }

            let held = match tokio::fs::read(at).await {
                Ok(raw) => Canvas::read(&raw)
                    .map(|drawn| Chosen { raw, drawn })
                    .with_context(|| format!("reading {at}")),
                Err(why) => Err(anyhow::anyhow!("reading {at}: {why}")),
            };

            match held {
                Ok(held) => {
                    out.insert(at.to_string(), held);
                }
                Err(why) => complaints.push(format!("{why:#}")),
            }
        }

        (out, complaints)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Swap;
    use std::cell::Cell;

    fn drawn(wide: usize, high: usize, tint: u8) -> Canvas {
        Canvas::of(wide, high, vec![tint; wide * high * 4]).expect("a picture")
    }

    #[tokio::test]
    async fn one_swap_file_shared_by_many_keys_is_read_once() {
        let dir = tempfile::tempdir().expect("a temp folder");
        let at = dir.path().join("picked.png");
        fs::write(&at, drawn(2, 2, 9).png().expect("a png")).expect("a file");

        let picked = Pictures {
            swaps: ["a", "b", "c"]
                .into_iter()
                .map(|key| Swap {
                    from: key.to_string(),
                    to: at.to_string_lossy().to_string(),
                })
                .collect(),
            ..Pictures::default()
        };

        let (chosen, complaints) = picked.chosen().await;

        assert!(complaints.is_empty(), "{complaints:?}");
        assert_eq!(
            chosen.len(),
            1,
            "three keys share one file, so it is read and decoded once for all of them"
        );
    }

    #[tokio::test]
    async fn one_unreadable_swap_file_shared_by_many_keys_is_complained_about_once() {
        let dir = tempfile::tempdir().expect("a temp folder");
        let at = dir.path().join("picked.png");
        fs::write(&at, b"not a picture at all").expect("a file");

        let picked = Pictures {
            swaps: ["a", "b", "c"]
                .into_iter()
                .map(|key| Swap {
                    from: key.to_string(),
                    to: at.to_string_lossy().to_string(),
                })
                .collect(),
            ..Pictures::default()
        };

        let (chosen, complaints) = picked.chosen().await;

        assert!(chosen.is_empty(), "nothing here can be drawn");
        assert_eq!(
            complaints.len(),
            1,
            "a file that cannot be read is still one file, so the reader hears about it once \
             rather than once per key pointing at it: {complaints:?}"
        );
    }

    const LEDGER: Ledger = Ledger("test-pictures.json");

    fn a_shot(key: &str) -> Shot {
        shot(
            Named::beside(key.to_string(), "img/pictures", key),
            &Measured::Sized {
                wide: 343,
                high: 624,
                format: "PNG".to_string(),
            },
            None,
        )
    }

    #[tokio::test]
    async fn the_pictures_a_read_found_are_the_ones_a_later_run_offers() {
        let sandbox = tempfile::tempdir().expect("a temp folder");
        let store = sandbox.path();

        assert!(
            LEDGER.remembered(store).is_empty(),
            "a game nobody has read yet offers no picture, which is what keeps the tab off the \
             screen until there is something on it"
        );

        LEDGER
            .remember(
                store,
                &[
                    a_shot("img/pictures/1Akane1_1.rpgmvp"),
                    a_shot("img/pictures/1Akane1_2.rpgmvp"),
                ],
            )
            .await
            .expect("a list of pictures");

        let back = LEDGER.remembered(store);
        assert_eq!(back.len(), 2);
        assert_eq!(
            back[0].key, "img/pictures/1Akane1_1.rpgmvp",
            "a pick is written down against this key and read back at install, so it has to \
             survive the trip through the ledger unchanged"
        );
        assert_eq!(back[1].name, "1Akane1_2.rpgmvp");
        assert_eq!(
            back[1].at, "img/pictures/1Akane1_2.rpgmvp",
            "the editor shows the path the game keeps a picture at, so it has to come back too"
        );

        LEDGER.remember(store, &[]).await.expect("an empty list");
        assert!(
            LEDGER.remembered(store).is_empty(),
            "reading a game again after its pictures went has to take the rows away too"
        );
    }

    #[test]
    fn a_file_this_reader_cannot_open_is_still_listed_and_says_why() {
        let held = measured(b"not an image at all, whatever it is named");
        let one = shot(
            Named::beside(
                "img/pictures/odd.png".to_string(),
                "img/pictures",
                "img/pictures/odd.png",
            ),
            &held,
            None,
        );

        assert!(
            !one.drawable,
            "handing bytes that are not an image to the editor would draw a broken tile with no \
             word about why"
        );
        assert!(
            one.locked.is_some_and(|why| why.contains("not a picture")),
            "a picture this reader cannot open is shown and marked, never quietly dropped: a \
             reader counting the rows has to be able to see it"
        );
        assert_eq!((one.wide, one.high), (0, 0));
        assert!(one.format.is_empty());
    }

    #[test]
    fn a_picture_is_measured_from_its_first_bytes_and_only_read_whole_when_that_needs_it() {
        let png = drawn(40, 24, 3).png().expect("a png");
        let head = Head {
            raw: png[..64].to_vec(),
            whole: false,
        };

        let held = measured_or(&head, || panic!("a png says its size in its first bytes"));
        assert_eq!(
            held.sized(),
            Some((40, 24)),
            "this game ships five gigabytes of pictures, so listing them may not decode them: the \
             size sits in the header of every one"
        );
        assert_eq!(held.format(), "PNG");

        let torn = Head {
            raw: vec![b'B', b'M'],
            whole: false,
        };
        let asked = Cell::new(0);
        let held = measured_or(&torn, || {
            asked.set(asked.get() + 1);
            None
        });

        assert_eq!(
            asked.get(),
            1,
            "a header too short to answer is read again from the whole file rather than reported \
             as a picture nobody can open"
        );
        assert!(held.sized().is_none());
    }

    #[test]
    fn a_format_this_build_could_read_but_not_write_is_shown_and_left_alone() {
        let sized = |format: &str| Measured::Sized {
            wide: 64,
            high: 64,
            format: format.to_string(),
        };
        let one = |format: &str| {
            shot(
                Named::beside(
                    "Graphics/one.dds".to_string(),
                    "Graphics",
                    "Graphics/one.dds",
                ),
                &sized(format),
                None,
            )
        };

        let held = one("DDS");
        assert!(
            held.drawable,
            "a picture this build can open is drawn whatever else is true of it, so the reader \
             can see what they are looking at"
        );
        assert!(
            held.locked.is_some_and(|why| why.contains("DDS")),
            "the reader has to be told which format is in the way, not just that something is"
        );
        assert!(
            one("PNG").locked.is_none(),
            "and a format this build writes may never be held back"
        );
    }

    #[test]
    fn a_read_that_found_nothing_says_nothing_and_a_locked_one_says_how_many() {
        assert_eq!(
            counted(&[]),
            None,
            "a game with no picture in it stays quiet"
        );

        let mut locked = a_shot("img/pictures/one.png");
        locked.locked = Some("this reader cannot open it".to_string());

        assert_eq!(
            counted(&[a_shot("img/pictures/two.png")]),
            Some("1 picture(s)".to_string())
        );
        assert_eq!(
            counted(&[a_shot("img/pictures/two.png"), locked]),
            Some("2 picture(s) (1 of them can be viewed but not replaced)".to_string()),
            "a reader who sees a row they cannot use has to be told how many of those there are \
             before they go hunting for the reason one by one"
        );
    }

    #[test]
    fn a_pick_that_already_matches_goes_in_untouched() {
        let held = drawn(4, 2, 9);
        let png = held.png().expect("a png");

        assert_eq!(
            fitted("png", Some((4, 2)), &png).expect("it fits"),
            png,
            "re-encoding a file that was already the right shape would throw away its palette, \
             its bit depth and whether it had an alpha channel at all, and some engines key \
             transparency off exactly that"
        );

        let jpeg = held.written_as("jpg").expect("a jpeg");
        assert_eq!(
            fitted("jpeg", Some((4, 2)), &jpeg).expect("it fits"),
            jpeg,
            "jpg and jpeg are one format under two endings, so a reader picking either has to be \
             taken at their word"
        );
    }

    #[test]
    fn a_pick_of_another_format_is_written_out_as_the_one_the_game_ships() {
        let png = drawn(4, 2, 200).png().expect("a png");
        let held = fitted("jpg", Some((4, 2)), &png).expect("it fits");

        assert_eq!(
            canvas::kind_of(&held),
            Some("jpg"),
            "handing a game PNG bytes under a jpg name is a guess about how it picks its decoder, \
             and guessing wrong is a picture the player never sees"
        );
        assert_eq!(
            Canvas::read(&held).expect("it reads back").wide,
            4,
            "and what comes out has to still be the picture that went in"
        );
    }

    #[test]
    fn a_pick_of_the_wrong_size_is_scaled_into_the_spot_it_has_to_fill() {
        let png = drawn(16, 8, 40).png().expect("a png");
        let held = fitted("png", Some((4, 2)), &png).expect("it fits");
        let back = Canvas::read(&held).expect("it reads back");

        assert_eq!((back.wide, back.high), (4, 2));
    }

    #[test]
    fn a_format_this_build_cannot_write_is_refused_out_loud() {
        let png = drawn(4, 2, 40).png().expect("a png");

        let why = fitted("dds", Some((4, 2)), &png).expect_err("dds cannot be written here");
        assert!(
            format!("{why:#}").contains("dds"),
            "the reader has to be told which file would work, not just that theirs did not: \
             {why:#}"
        );

        assert!(
            fitted("png", Some((4, 2)), b"not a picture at all").is_err(),
            "and a file that is not a picture is refused before any of that"
        );
    }

    #[test]
    fn a_game_that_does_not_care_about_size_keeps_the_picture_the_reader_picked() {
        let png = drawn(16, 8, 40).png().expect("a png");
        let held = fitted("png", None, &png).expect("it fits");

        assert_eq!(
            held, png,
            "Ren'Py lays a picture out by script, so a bigger one is the reader's choice to make \
             and scaling it down would throw away what they picked it for"
        );

        let held = fitted("webp", None, &png).expect("it fits");
        assert_eq!(
            canvas::kind_of(&held),
            Some("webp"),
            "the ending still has to hold what it says, even when the size is free"
        );
        assert_eq!(Canvas::read(&held).expect("it reads back").wide, 16);
    }

    #[test]
    fn a_row_holding_nothing_but_whitespace_is_a_row_nobody_picked() {
        let held = Pictures {
            swaps: vec![Swap {
                from: "sharedassets1.assets|12".to_string(),
                to: "  ".to_string(),
            }],
            ..Pictures::default()
        };

        assert!(
            held.filled().is_empty(),
            "a row the reader cleared is a row with nothing picked, and an export must not rebuild \
             a container for it"
        );
    }

    #[test]
    fn a_row_the_reader_left_alone_is_never_handed_on_as_a_pick() {
        let held = Pictures {
            swaps: vec![
                Swap {
                    from: "sharedassets1.assets|11".to_string(),
                    to: "/a.png".to_string(),
                },
                Swap {
                    from: "resources.assets|11".to_string(),
                    to: String::new(),
                },
                Swap {
                    from: "StreamingAssets/aa/one.bundle|3".to_string(),
                    to: "/c.png".to_string(),
                },
            ],
            ..Pictures::default()
        };

        assert_eq!(
            held.filled(),
            [
                ("sharedassets1.assets|11", "/a.png"),
                ("StreamingAssets/aa/one.bundle|3", "/c.png")
            ],
            "an empty pick would send the writer looking for a file nobody chose, and a container \
             inside a folder keeps its whole path as its key"
        );
    }
}
