use crate::canvas::Canvas;
use crate::service::pictures;
use anyhow::{Result, bail};
use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

const AS_FILE: &str = "file://";

static AT_HAND: LazyLock<Mutex<Option<arboard::Clipboard>>> = LazyLock::new(|| Mutex::new(None));

pub async fn picture(store: &Path) -> Result<Option<PathBuf>> {
    for at in files().await {
        if pictures::opens(&at).await {
            return pictures::kept(store, &at).await.map(Some);
        }
    }

    if let Some((wide, high, rgba)) = drawn().await {
        return pictures::pasted(store, wide, high, rgba).await.map(Some);
    }

    if let Some(at) = pathed(&said().await)
        && pictures::opens(&at).await
    {
        return pictures::kept(store, &at).await.map(Some);
    }

    Ok(None)
}

fn pathed(said: &str) -> Option<PathBuf> {
    let held = said.trim();
    let held = held.strip_prefix(AS_FILE).unwrap_or(held);

    match held.is_empty() {
        true => None,
        false => Some(PathBuf::from(held)),
    }
}

async fn files() -> Vec<PathBuf> {
    at_hand(|held| held.get().file_list())
        .await
        .unwrap_or_default()
        .iter()
        .filter_map(|at| pathed(&at.to_string_lossy()))
        .collect()
}

async fn said() -> String {
    at_hand(|held| held.get().text()).await.unwrap_or_default()
}

async fn drawn() -> Option<(usize, usize, Vec<u8>)> {
    let held = at_hand(|held| held.get().image()).await?;

    Some((held.width, held.height, held.bytes.into_owned()))
}

pub async fn drew(drawn: Arc<Canvas>) -> Result<()> {
    let done = at_hand(move |held| {
        held.set_image(arboard::ImageData {
            width: drawn.wide,
            height: drawn.high,
            bytes: Cow::Borrowed(&drawn.pixels),
        })
    })
    .await;

    match done {
        Some(()) => Ok(()),
        None => bail!("this picture would not go to the clipboard"),
    }
}

fn taken<T>(take: impl FnOnce(&mut arboard::Clipboard) -> Result<T, arboard::Error>) -> Option<T> {
    let mut held = AT_HAND.lock().ok()?;
    if held.is_none() {
        *held = arboard::Clipboard::new().ok();
    }

    match take(held.as_mut()?) {
        Ok(one) => Some(one),
        Err(_) => {
            *held = None;
            None
        }
    }
}

async fn at_hand<T: Send + 'static>(
    take: impl FnOnce(&mut arboard::Clipboard) -> Result<T, arboard::Error> + Send + 'static,
) -> Option<T> {
    tokio::task::spawn_blocking(move || taken(take))
        .await
        .ok()
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_copied_as_text_is_read_the_way_a_file_manager_hands_it_over() {
        assert_eq!(
            pathed("file:///home/one/muni_all.gif"),
            Some(PathBuf::from("/home/one/muni_all.gif")),
            "a file manager copies a path as a uri, and that is still a file on this disk"
        );
        assert_eq!(
            pathed("  /home/one/muni_all.gif\n"),
            Some(PathBuf::from("/home/one/muni_all.gif")),
            "a path copied out of a terminal carries the newline with it"
        );
        assert_eq!(pathed("   "), None, "and empty text is no path at all");
    }
}
