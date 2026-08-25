use crate::engine::Install;
use crate::engine::pictures::{
    self, Found, Head, Measured, Named, Shot, fitted, key_of, measured_or, of_file, shot,
};
use crate::engine::wolf_rpg::{archive, source};
use crate::scope::{Scope, key, slashed};
use crate::{backup, canvas, walk};
use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const LEDGER: pictures::Ledger = pictures::Ledger("wolf-pictures.json");

struct Spot {
    file: PathBuf,
    inside: Option<PathBuf>,
}

fn stepped(said: &str) -> Result<String> {
    let held = Scope::read(said)?;
    if held.everything() {
        anyhow::bail!("{said} names no file in this game");
    }

    Ok(held.as_str().to_string())
}

fn spot_of(game_dir: &Path, key: &str) -> Result<Spot> {
    let (holder, inside) = match pictures::packed_in(key) {
        Some((holder, inside)) => (holder, Some(stepped(inside)?)),
        None => (key, None),
    };

    Ok(Spot {
        file: game_dir.join(stepped(holder)?),
        inside: inside.map(PathBuf::from),
    })
}

fn lifted(at: &Path, sealed: &[u8], inside: &Path) -> Result<Vec<u8>> {
    let mut out = None;

    archive::poured(
        at,
        sealed,
        Path::new(""),
        |held| held == inside,
        |held| {
            out = Some(held.body);
            Ok(())
        },
    )
    .map_err(anyhow::Error::msg)?;

    out.ok_or_else(|| anyhow::anyhow!("{} holds no {} any more", at.display(), slashed(inside)))
}

fn sealing(game_dir: &Path, at: &Path) -> Result<Vec<u8>> {
    archive::key_for(at, source::weight(game_dir))
        .map_err(anyhow::Error::msg)?
        .ok_or_else(|| anyhow::anyhow!("{} is not an archive this reader can open", at.display()))
}

pub fn picture(game_dir: &Path, store: &Path, key: &str) -> Result<Vec<u8>> {
    let spot = spot_of(game_dir, key)?;
    let from = backup::original_at_now(store, game_dir, &spot.file)?;

    let Some(inside) = spot.inside else {
        return fs::read(&from).with_context(|| format!("reading {}", from.display()));
    };

    let sealed = sealing(game_dir, &from)?;

    lifted(&from, &sealed, &inside)
}

fn over(at: &Path) -> String {
    at.parent().map(slashed).unwrap_or_default()
}

fn under(holder: &str, over: &str) -> String {
    match over.is_empty() {
        true => holder.to_string(),
        false => format!("{holder}/{over}"),
    }
}

fn loose(game_dir: &Path) -> Vec<Shot> {
    let data = game_dir.join(source::DATA);
    let mut out = Vec::new();

    for at in walk::files_now(&data) {
        if !canvas::is_picture(&at) {
            continue;
        }

        let Ok(relative) = at.strip_prefix(game_dir) else {
            continue;
        };

        let named = Named::beside(
            key_of(&slashed(relative), None),
            &over(relative),
            &at.strip_prefix(&data)
                .map(slashed)
                .unwrap_or_else(|_| slashed(relative)),
        );

        out.push(of_file(named, &at));
    }

    out
}

fn archived(game_dir: &Path, at: &Path, weight: Option<u32>) -> Result<Vec<Shot>, String> {
    let Some(sealed) = archive::key_for(at, weight)? else {
        return Ok(Vec::new());
    };

    let holder = key(game_dir, at);
    let stem = slashed(Path::new(at.file_stem().unwrap_or_default()));
    let mut out = Vec::new();

    archive::peeked(
        at,
        &sealed,
        Path::new(""),
        pictures::HEAD,
        canvas::is_picture,
        |held| {
            let inside = slashed(&held.at);
            let named = Named::beside(
                key_of(&holder, Some(&inside)),
                &under(&holder, &over(&held.at)),
                &format!("{stem}/{inside}"),
            );

            out.push(match held.head {
                Ok(raw) => {
                    let head = Head {
                        whole: raw.len() >= held.size,
                        raw,
                    };
                    let found = measured_or(&head, || lifted(at, &sealed, &held.at).ok());

                    shot(named, &found, None)
                }
                Err(why) => shot(
                    named,
                    &Measured::Unknown,
                    Some(format!(
                        "this picture could not be lifted out of the archive holding it: {why}"
                    )),
                ),
            });

            Ok(())
        },
    )?;

    Ok(out)
}

pub async fn found(game_dir: &Path) -> Found {
    let weight = source::weight(game_dir);
    let mut waiting = Vec::new();

    for one in source::archives(game_dir) {
        let here = game_dir.to_path_buf();

        waiting.push(tokio::task::spawn_blocking(move || {
            let named = key(&here, &one);

            archived(&here, &one, weight).map_err(|why| format!("{named}: {why}"))
        }));
    }

    let here = game_dir.to_path_buf();
    let mut shots = tokio::task::spawn_blocking(move || loose(&here))
        .await
        .unwrap_or_default();
    let mut shut = Vec::new();

    for held in futures::future::join_all(waiting).await {
        match held {
            Ok(Ok(found)) => shots.extend(found),
            Ok(Err(why)) => shut.push(why),
            Err(_) => shut.push("listing the pictures of one archive was cut short".to_string()),
        }
    }

    shots.sort_by(|one, other| (&one.holder, &one.name).cmp(&(&other.holder, &other.name)));

    Found { shots, shut }
}

#[derive(Default)]
pub struct Picked {
    pub loose: Vec<(PathBuf, Vec<u8>)>,
    pub sealing: BTreeMap<PathBuf, BTreeMap<PathBuf, Vec<u8>>>,
    pub said: Vec<String>,
}

fn gone(what: &Path, name: &str) -> String {
    format!(
        "{} is not in this game any more, so the picture picked for {name} stayed out",
        what.display()
    )
}

#[tracing::instrument(name = "wolf.pictures", skip_all)]
pub async fn picked(at: &Install<'_>) -> Picked {
    let mut out = Picked::default();

    let chosen = at.chosen().await;
    let held = at.picked(&chosen);

    if held.is_empty() {
        return out;
    }

    let known = LEDGER.remembered(at.store);
    let taken = source::archives(at.game_dir);
    let mut lost = Vec::new();

    for (key, pick) in held {
        let Some(shot) = known.iter().find(|one| one.key == key) else {
            lost.push(key.to_string());
            continue;
        };

        if let Some(why) = &shot.locked {
            out.said.push(format!("{}: {why}", shot.name));
            continue;
        }

        let spot = match spot_of(at.game_dir, key) {
            Ok(spot) => spot,
            Err(why) => {
                out.said.push(format!("{key}: {why:#}"));
                continue;
            }
        };

        let body = match fitted(&shot.format, Some((shot.wide, shot.high)), &pick.raw) {
            Ok(body) => body,
            Err(why) => {
                out.said.push(format!("{}: {why:#}", shot.name));
                continue;
            }
        };

        let Some(inside) = spot.inside else {
            match spot.file.is_file() {
                true => out.loose.push((spot.file, body)),
                false => out.said.push(gone(&spot.file, &shot.name)),
            }

            continue;
        };

        match taken.iter().find(|one| **one == spot.file) {
            Some(archive) => {
                out.sealing
                    .entry(archive.clone())
                    .or_default()
                    .insert(inside, body);
            }
            None => out.said.push(gone(&spot.file, &shot.name)),
        }
    }

    if !lost.is_empty() {
        out.said.push(format!(
            "{} picture(s) picked are not in this game any more, so they stayed out: {}",
            lost.len(),
            lost.join(", ")
        ));
    }

    out
}

pub async fn let_go(at: &Install<'_>, swapped: &[PathBuf]) -> Result<usize> {
    backup::put_back_the_rest(at.store, at.game_dir, canvas::is_picture, swapped).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::Canvas;
    use crate::engine::wolf_rpg::fixture::{a_jpeg, a_png, sandbox};

    fn a_game(root: &Path) {
        let data = root.join(source::DATA);
        fs::create_dir_all(data.join("Picture")).expect("a picture folder");
        fs::write(data.join("Picture").join("title.png"), a_png(20, 10, 0))
            .expect("a loose picture");
        fs::write(data.join("Picture").join("notes.txt"), "not a picture").expect("a loose file");

        let bundled = archive::archived(
            &[
                ("menu.png", a_png(30, 8, 40).as_slice()),
                ("room.jpg", a_jpeg(24, 12).as_slice()),
                ("torn.jpg", b"\xff\xd8\xff\xe0 not really a jpeg".as_slice()),
                ("talk001.txt", b"@mes hello".as_slice()),
            ],
            None,
        );
        fs::write(data.join("Locker.wolf"), &bundled).expect("an archive");
        fs::write(root.join("Game.exe"), [0; 4]).expect("a runner");
    }

    #[tokio::test]
    async fn every_picture_a_game_ships_is_listed_whether_it_lies_loose_or_sits_in_an_archive() {
        let at = sandbox();
        let root = at.path();
        a_game(root);

        let held = found(root).await;

        assert!(held.shut.is_empty(), "{:?}", held.shut);
        assert_eq!(
            held.shots
                .iter()
                .map(|one| (
                    one.key.as_str(),
                    one.holder.as_str(),
                    one.name.as_str(),
                    one.at.as_str(),
                    one.wide,
                    one.high,
                    one.format.as_str(),
                    one.drawable
                ))
                .collect::<Vec<_>>(),
            [
                (
                    "Data/Locker.wolf|menu.png",
                    "Data/Locker.wolf",
                    "menu.png",
                    "Locker/menu.png",
                    30,
                    8,
                    "PNG",
                    true
                ),
                (
                    "Data/Locker.wolf|room.jpg",
                    "Data/Locker.wolf",
                    "room.jpg",
                    "Locker/room.jpg",
                    24,
                    12,
                    "JPG",
                    true
                ),
                (
                    "Data/Locker.wolf|torn.jpg",
                    "Data/Locker.wolf",
                    "torn.jpg",
                    "Locker/torn.jpg",
                    0,
                    0,
                    "JPG",
                    false
                ),
                (
                    "Data/Picture/title.png",
                    "Data/Picture",
                    "title.png",
                    "Picture/title.png",
                    20,
                    10,
                    "PNG",
                    true
                ),
            ],
            "a Wolf game hands the player as much drawn Japanese as written, and half of it never \
             leaves the archive: a reader who cannot see those rows cannot translate the half of \
             the game that is a picture"
        );

        let held_as = |name: &str| {
            held.shots
                .iter()
                .find(|one| one.name == name)
                .unwrap_or_else(|| panic!("{name} is listed"))
                .clone()
        };

        assert!(
            held_as("torn.jpg").locked.is_some(),
            "a picture we cannot open is shown with the reason rather than dropped, or the reader \
             counts the rows and is left wondering what became of the rest"
        );
        assert!(
            held_as("room.jpg").locked.is_none(),
            "this build writes a jpg back as readily as a png, so a picture the game keeps as one \
             is the reader's to swap like any other"
        );
        assert!(
            held_as("menu.png").locked.is_none(),
            "and the png beside it is the reader's to swap"
        );
        assert!(
            held.shots.iter().all(|one| one.atlas.is_empty()),
            "nothing in a Wolf game is cut out of a sheet, so naming an atlas would send the \
             reader looking for a picture inside another"
        );
        assert!(
            !held.shots.iter().any(|one| one.name == "notes.txt"),
            "a script bundled in among the pictures is text, and offering it as a picture would \
             ask the reader to paint over their own translation"
        );
    }

    #[tokio::test]
    async fn a_key_written_down_once_still_names_the_very_bytes_the_game_draws_with() {
        let at = sandbox();
        let root = at.path();
        let store = sandbox();
        a_game(root);

        let held = found(root).await;
        LEDGER
            .remember(store.path(), &held.shots)
            .await
            .expect("the list is kept");

        let back = LEDGER.remembered(store.path());
        assert_eq!(
            back.len(),
            held.shots.len(),
            "the rows a read found are the rows the next run offers"
        );

        assert_eq!(
            back.iter()
                .filter(|one| one.locked.is_some())
                .map(|one| one.name.as_str())
                .collect::<Vec<_>>(),
            ["torn.jpg"],
            "this build writes back every format it reads, so the only picture that is not the \
             reader's to swap is one nobody can open at all"
        );

        for one in back.iter().filter(|one| one.drawable) {
            let raw = picture(root, store.path(), &one.key)
                .unwrap_or_else(|why| panic!("{} has to come back: {why:#}", one.key));
            let shown = Canvas::read(&raw).expect("it reads as a picture");

            assert_eq!(
                (shown.wide as u32, shown.high as u32),
                (one.wide, one.high),
                "{} was listed at one size and drawn at another, so the reader would be handed \
                 somebody else's picture to paint over",
                one.key
            );
        }

        assert!(
            LEDGER.remembered(&store.path().join("nothing")).is_empty(),
            "a game nobody has read yet offers no picture, which is what keeps the tab off the \
             screen until there is something on it"
        );
    }

    #[test]
    fn a_key_that_would_climb_out_of_the_game_reaches_nothing_at_all() {
        let game = Path::new("/games/demo");

        for key in ["../../etc/passwd", "..", "", "Data/../../etc|x.png"] {
            assert!(
                spot_of(game, key).is_err(),
                "{key} would let a saved project reach a file outside the game it belongs to"
            );
        }

        let spot = spot_of(game, "Data/Locker.wolf|Chara/face001.png").expect("a plain key");
        assert_eq!(spot.file, game.join("Data/Locker.wolf"));
        assert_eq!(
            spot.inside,
            Some(PathBuf::from("Chara/face001.png")),
            "the archive and the path inside it both have to come back out of the key, or a pick \
             made today lands on nothing tomorrow"
        );
        assert_eq!(
            spot_of(game, "Data/Picture/title.png")
                .expect("a plain key")
                .inside,
            None
        );
    }
}
