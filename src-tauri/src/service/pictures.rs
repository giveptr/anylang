use crate::canvas::{self, Canvas};
use crate::engine::pictures::{HEAD, Handed, Pictures, Shot};
use crate::hash::xxh3;
use crate::service::locate::engine_at;
use crate::store;
use anyhow::{Context, Result};
use std::collections::BTreeSet;
use std::num::NonZero;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const THUMBS: &str = "thumbs";
const SWAPS: &str = "swaps";
const AS_PNG: &str = "image/png";
const CLOSE: usize = 640;

const SHARE: usize = 2;

pub fn at_once() -> usize {
    let machine = std::thread::available_parallelism().map_or(1, NonZero::get);

    (machine / SHARE).max(1)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Shown {
    pub body: String,
    pub mime: String,
}

fn thumb_at(store: &Path, key: &str, most: u32) -> PathBuf {
    store.join(THUMBS).join(format!("{}-{most}.png", xxh3(key)))
}

pub async fn listed(game_dir: &Path) -> Result<Vec<Shot>> {
    let root = store::root_for(game_dir)?;
    let engine = engine_at(game_dir).await?;

    Ok(tokio::task::spawn_blocking(move || engine.pictures(&root)).await?)
}

pub async fn shown(game_dir: &Path, store: &Path, key: &str, most: u32) -> Result<Shown> {
    if most > 0
        && let Ok(held) = tokio::fs::read(thumb_at(store, key, most)).await
    {
        return Ok(as_png(&held));
    }

    let (body, verbatim) =
        picture_of(game_dir, store, key, move |held| drawn_for(held, most)).await?;

    match verbatim {
        Some(mime) => Ok(Shown {
            body: base64_of(&body),
            mime: mime.to_string(),
        }),
        None => match most {
            0 => Ok(as_png(&body)),
            most => keep(&thumb_at(store, key, most), body).await,
        },
    }
}

fn handed_whole(raw: &[u8]) -> Option<&'static str> {
    const BYTES: usize = 8 * 1024 * 1024;
    const PIXELS: u64 = 8_000_000;

    if raw.len() > BYTES {
        return None;
    }

    let (wide, high) = canvas::measured(raw)?;
    if u64::from(wide) * u64::from(high) > PIXELS {
        return None;
    }

    canvas::shown_as(raw)
}

fn drawn_for(held: Handed, most: u32) -> Result<(Vec<u8>, Option<&'static str>)> {
    if most > 0 {
        return Ok((held.drawn_within(most as usize)?.png()?, None));
    }

    match held {
        Handed::Shipped(raw) => match handed_whole(&raw) {
            Some(mime) => Ok((raw, Some(mime))),
            None => Ok((Canvas::read_within(&raw, CLOSE)?.png()?, None)),
        },
        Handed::Drawn(one) => Ok((one.within(CLOSE)?.png()?, None)),
    }
}

async fn keep(at: &Path, body: Vec<u8>) -> Result<Shown> {
    if let Some(folder) = at.parent() {
        tokio::fs::create_dir_all(folder).await?;
    }
    store::write_atomically(at, &body).await?;

    Ok(as_png(&body))
}

pub async fn pasted(store: &Path, wide: usize, high: usize, rgba: Vec<u8>) -> Result<PathBuf> {
    let body = tokio::task::spawn_blocking(move || Canvas::of(wide, high, rgba)?.png()).await??;

    held_with(store, "pasted.png", body).await
}

pub async fn kept(store: &Path, at: &Path) -> Result<PathBuf> {
    let raw = tokio::fs::read(at)
        .await
        .with_context(|| format!("reading {}", at.display()))?;

    if canvas::kind_of(&raw).is_none() {
        anyhow::bail!("{} is not a picture this reader can open", at.display());
    }

    let named = at
        .file_name()
        .map(|held| held.to_string_lossy().to_string())
        .unwrap_or_else(|| "picked".to_string());

    held_with(store, &named, raw).await
}

async fn held_with(store: &Path, named: &str, body: Vec<u8>) -> Result<PathBuf> {
    let folder = store.join(SWAPS).join(xxh3(&body));
    tokio::fs::create_dir_all(&folder).await?;

    let at = folder.join(named);
    store::write_atomically(&at, body).await?;

    Ok(at)
}

pub async fn opens(at: &Path) -> bool {
    use tokio::io::AsyncReadExt;

    let Ok(file) = tokio::fs::File::open(at).await else {
        return false;
    };

    let mut head = Vec::new();
    let read = file.take(HEAD as u64).read_to_end(&mut head).await;

    read.is_ok() && canvas::kind_of(&head).is_some()
}

pub async fn drawn(game_dir: &Path, store: &Path, key: &str) -> Result<Arc<Canvas>> {
    picture_of(game_dir, store, key, Handed::drawn).await
}

pub async fn saved(game_dir: &Path, store: &Path, key: &str, at: &Path) -> Result<()> {
    let body = picture_of(game_dir, store, key, |held| match held {
        Handed::Shipped(raw) => Ok(raw),
        Handed::Drawn(one) => one.png(),
    })
    .await?;

    store::write_atomically(at, body).await
}

async fn picture_of<T: Send + 'static>(
    game_dir: &Path,
    store: &Path,
    key: &str,
    of: impl FnOnce(Handed) -> Result<T> + Send + 'static,
) -> Result<T> {
    let engine = engine_at(game_dir).await?;
    let held = game_dir.to_path_buf();
    let kept = store.to_path_buf();
    let named = key.to_string();

    tokio::task::spawn_blocking(move || of(engine.picture(&held, &kept, &named)?)).await?
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Replacement {
    pub body: String,
    pub mime: String,
    pub wide: u32,
    pub high: u32,
}

pub async fn from_file(at: &Path) -> Result<Replacement> {
    let raw = tokio::fs::read(at)
        .await
        .with_context(|| format!("reading {}", at.display()))?;

    tokio::task::spawn_blocking(move || -> Result<Replacement> {
        let sized = canvas::measured(&raw);

        if let (Some(mime), Some((wide, high))) = (handed_whole(&raw), sized) {
            return Ok(Replacement {
                body: base64_of(&raw),
                mime: mime.to_string(),
                wide,
                high,
            });
        }

        let (held, wide, high) = match sized {
            Some((wide, high)) => (Canvas::read_within(&raw, CLOSE)?, wide, high),
            None => {
                let whole = Canvas::read(&raw)?;
                let (wide, high) = (whole.wide as u32, whole.high as u32);

                (whole.within(CLOSE)?, wide, high)
            }
        };

        Ok(Replacement {
            body: base64_of(&held.png()?),
            mime: AS_PNG.to_string(),
            wide,
            high,
        })
    })
    .await?
}

fn as_png(body: &[u8]) -> Shown {
    Shown {
        body: base64_of(body),
        mime: AS_PNG.to_string(),
    }
}

fn base64_of(raw: &[u8]) -> String {
    use base64::Engine;

    base64::engine::general_purpose::STANDARD.encode(raw)
}

pub async fn forget_thumbs(store: &Path) {
    let _ = tokio::fs::remove_dir_all(store.join(THUMBS)).await;
}

pub async fn forget_stray_swaps(store: &Path, held: &Pictures) {
    let wanted: BTreeSet<PathBuf> = held
        .swaps
        .iter()
        .filter_map(|one| Path::new(&one.to).parent().map(Path::to_path_buf))
        .collect();

    let folder = store.join(SWAPS);
    let Ok(mut inside) = tokio::fs::read_dir(&folder).await else {
        return;
    };

    while let Ok(Some(one)) = inside.next_entry().await {
        let at = one.path();
        if !wanted.contains(&at) {
            let _ = tokio::fs::remove_dir_all(&at).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Swap;

    #[test]
    fn the_gallery_leaves_the_reader_half_their_machine() {
        let machine = std::thread::available_parallelism().map_or(1, NonZero::get);
        let small = machine < SHARE * 2;

        assert!(at_once() >= 1, "a machine of any size draws one at a time");
        assert!(
            at_once() <= machine / SHARE || small,
            "a tile is a whole picture decoded on a core, not a file waiting on a disk, so this is \
             not walk::at_once: filling every core draws the screen a hair faster and freezes the \
             window doing it, because the webview paints on this same machine"
        );
        assert!(
            at_once() * SHARE >= machine || small,
            "and half is the point rather than a retreat: asking for a quarter of this machine \
             took twice as long to fill a screen, and asking for all of it saved four percent"
        );
    }

    #[test]
    fn a_thumbnail_is_named_after_the_key_and_the_size_it_was_drawn_at() {
        let store = Path::new("/store");
        let one = thumb_at(store, "sharedassets1.assets|11", 128);

        assert_eq!(
            one,
            store.join(THUMBS).join("4beaefb46c4cfb2f-128.png"),
            "a tile drawn on one run is looked for by this name on the next, so the same picture \
             at the same size has to land on the same file or the cache never hits and every \
             scroll decodes an atlas again"
        );
        assert_ne!(
            one,
            thumb_at(store, "sharedassets1.assets|11", 512),
            "a bigger tile is a different picture on disk"
        );
        assert_ne!(one, thumb_at(store, "sharedassets1.assets|12", 128));
        assert!(
            !one.to_string_lossy().contains('|'),
            "a key holds characters no filesystem wants, so the name on disk is a hash of it"
        );
    }

    #[tokio::test]
    async fn a_replacement_lives_with_the_project_and_survives_the_file_it_came_from() {
        let sandbox = tempfile::tempdir().expect("a temp folder");
        let store = sandbox.path();
        let theirs = store.join("my_edit.png");
        let body = Canvas::of(2, 2, vec![7; 16])
            .expect("a picture")
            .png()
            .expect("a png");

        tokio::fs::write(&theirs, &body).await.expect("their file");

        let kept_at = kept(store, &theirs).await.expect("it is kept");
        assert_eq!(
            kept_at.file_name(),
            theirs.file_name(),
            "the name they picked is the name the panel shows, so it has to survive the copy"
        );
        assert!(
            kept_at.starts_with(store.join(SWAPS)),
            "a replacement has to live with the project: {}",
            kept_at.display()
        );

        tokio::fs::remove_file(&theirs)
            .await
            .expect("they delete it");
        assert_eq!(
            tokio::fs::read(&kept_at).await.expect("still there"),
            body,
            "deleting what they picked may not take the replacement with it"
        );
    }

    #[tokio::test]
    async fn a_sweep_takes_the_swaps_nothing_names_any_more_and_leaves_the_ones_still_named() {
        let sandbox = tempfile::tempdir().expect("a temp folder");
        let store = sandbox.path();
        let theirs = store.join("my_edit.png");
        let body = Canvas::of(2, 2, vec![7; 16])
            .expect("a picture")
            .png()
            .expect("a png");

        tokio::fs::write(&theirs, &body).await.expect("their file");

        let kept_at = kept(store, &theirs).await.expect("it is kept");
        let held = Pictures {
            swaps: vec![Swap {
                from: "a/b.png".to_string(),
                to: kept_at.to_string_lossy().to_string(),
            }],
            ..Pictures::default()
        };
        let stray = store.join(SWAPS).join("deadbeef");
        tokio::fs::create_dir_all(&stray)
            .await
            .expect("an old swap");

        forget_stray_swaps(store, &held).await;

        assert!(
            tokio::fs::read(&kept_at).await.is_ok(),
            "a sweep may never take a replacement the project still names"
        );
        assert!(
            !stray.exists(),
            "and what nothing names any more has to go, or the store grows for ever"
        );
    }

    #[test]
    fn a_picture_the_webview_can_draw_is_handed_over_as_it_shipped() {
        let gif = Canvas::of(4, 4, vec![9; 4 * 4 * 4])
            .expect("a picture")
            .written_as("gif")
            .expect("a gif");

        let (body, mime) = drawn_for(Handed::Shipped(gif.clone()), 0).expect("it is handed over");
        assert_eq!(
            mime,
            Some("image/gif"),
            "a gif has to reach the screen as a gif, or an animation arrives as one still frame"
        );
        assert_eq!(body, gif, "and byte for byte, with nothing encoded again");

        let (small, mime) = drawn_for(Handed::Shipped(gif), 16).expect("a tile is drawn");
        assert_eq!(
            mime, None,
            "a tile is one frame scaled down, so it goes out as the png it was drawn into"
        );
        assert!(small.starts_with(&[0x89, b'P', b'N', b'G']));
    }
}
