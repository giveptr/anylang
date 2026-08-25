use crate::backup::{self, Pending};
use crate::canvas;
#[cfg(test)]
use crate::canvas::Canvas;
use crate::engine::pictures::{self, Chosen};
use crate::engine::{Install, Prepare};
use crate::scope::{Scope, slashed};
use anyhow::{Context, Result};
use std::mem;
use std::path::Path;

pub const LEDGER: pictures::Ledger = pictures::Ledger("rpgmaker-pictures.json");

const NOT_OURS: &str = "the picture this game ships here is not one this reader can open, so \
                            nothing will be written over it";

pub fn drawn_name(name: &str) -> bool {
    canvas::is_picture(Path::new(name))
}

pub fn written(shipped: &[u8], picked: &[u8]) -> Result<Vec<u8>> {
    let held = pictures::measured(shipped);
    let Some(sized) = held.sized() else {
        anyhow::bail!("{NOT_OURS}");
    };

    pictures::fitted(held.format(), Some(sized), picked)
}

#[cfg(test)]
pub fn dotted(wide: usize, high: usize, tint: u8) -> Canvas {
    let mut held = Canvas::blank(wide, high);

    for (which, byte) in held.pixels.iter_mut().enumerate() {
        *byte = ((which * 7) as u8).wrapping_add(tint);
    }

    held
}

pub async fn remembered(at: &Prepare<'_>, shots: &[pictures::Shot]) -> Result<()> {
    pictures::remember(at, &LEDGER, shots, &[]).await
}

pub fn named_at(relative: &Path) -> pictures::Named {
    let at = slashed(relative);

    pictures::Named::beside(
        at.clone(),
        &relative.parent().map(slashed).unwrap_or_default(),
        &at,
    )
}

pub async fn into_folder(
    at: &Install<'_>,
    root: &Path,
    picked: &[(&str, &Chosen)],
    mine: impl Fn(&Path) -> bool,
    body: impl Fn(Vec<u8>, Vec<u8>) -> Result<Vec<u8>> + Clone + Send + 'static,
) -> Result<usize> {
    let mut pending: Vec<Pending> = Vec::new();

    for (key, picked) in picked {
        let live = match Scope::read(key).map(|one| one.under(root)) {
            Ok(live) if live.is_file() => live,
            _ => {
                at.progress.warn(
                    at.doing,
                    &format!("{key} is not a picture this game ships any more"),
                );
                continue;
            }
        };

        let shipped = tokio::fs::read(&live)
            .await
            .with_context(|| format!("reading {}", live.display()))?;
        let held = picked.raw.clone();
        let make = body.clone();
        let made = tokio::task::spawn_blocking(move || make(shipped, held)).await?;

        match made {
            Ok(made) => match backup::stage(&live, made).await {
                Ok(one) => pending.push(one),
                Err(why) => {
                    backup::let_all_go(mem::take(&mut pending)).await;

                    return Err(why);
                }
            },
            Err(why) => at.progress.warn(at.doing, &format!("{key}: {why:#}")),
        }
    }

    let wrote = backup::land_all(at.store, at.game_dir, pending).await?;
    let given = backup::put_back_the_rest(at.store, at.game_dir, mine, &wrote).await?;

    if given > 0 {
        at.progress
            .info(at.doing, &format!("{given} picture(s) put back"));
    }

    Ok(wrote.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn a_pick_already_shaped_like_the_one_it_replaces_goes_in_as_the_reader_saved_it() {
        let shipped = dotted(32, 32, 3).png().expect("a png");

        let mut grey = image::GrayImage::new(32, 32);
        for (which, pixel) in grey.pixels_mut().enumerate() {
            *pixel = image::Luma([which as u8]);
        }
        let mut held = Cursor::new(Vec::new());
        image::DynamicImage::ImageLuma8(grey)
            .write_to(&mut held, image::ImageFormat::Png)
            .expect("a png");
        let picked = held.into_inner();

        assert_eq!(
            written(&shipped, &picked).expect("what goes in"),
            picked,
            "Ren'Py and Wolf hand a matching pick straight through, and RPG Maker reading this \
             one out and writing it again would hand the game four channels where the reader \
             saved one"
        );

        let bigger = dotted(64, 64, 200).png().expect("a png");
        let held = written(&shipped, &bigger).expect("what goes in");
        assert_eq!(
            (Canvas::read(&held).expect("it reads").wide, held == bigger),
            (32, false),
            "a pick of another size still has to be made to fit the spot it fills"
        );

        assert!(
            written(b"not a picture at all", &picked).is_err(),
            "and a game file this reader cannot measure is left alone rather than written over"
        );
    }

    #[test]
    fn only_a_name_this_build_can_open_counts_as_a_picture() {
        for name in ["Actor1.png", "title.PNG", "Graphics/Faces/Actor1.png"] {
            assert!(drawn_name(name), "{name} is a picture a game draws with");
        }

        for name in [
            "Map001.json",
            "theme.ogg",
            "voice.rpgmvo",
            "talk.rpgmvp",
            "Boss.png_",
            "gamefont.ttf",
            "Quests.txt",
            "pictures",
        ] {
            assert!(
                !drawn_name(name),
                "{name} is not a picture this build opens by its ending, and offering it as one \
                 would send the reader looking for an image that never draws"
            );
        }

        assert!(
            canvas::kinds()
                .into_iter()
                .all(|kind| drawn_name(&format!("one.{kind}"))),
            "the endings a game is listed by are the endings this build can read back, so a \
             format the app grew a decoder for is offered without anyone remembering to add it here"
        );
    }
}
