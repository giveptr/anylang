use crate::engine::Install;
use crate::engine::pictures::{
    self, Found, Head, Named, Shot, head_of, key_of, measured_or, of_file, packed_in, shot,
};
use crate::engine::renpy::{GAME_DIR, archive};
use crate::scope::{Scope, slashed};
use crate::{backup, canvas, store};
use anyhow::{Context, Result, bail};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const PACKED: &str = "rpa";

pub const LEDGER: pictures::Ledger = pictures::Ledger("renpy-pictures.json");

fn holder_of(at: &str) -> String {
    let mut walk = at.split('/');
    let head = walk.next().unwrap_or_default();

    let Some(next) = walk.next() else {
        return GAME_DIR.to_string();
    };

    match walk.next() {
        Some(_) => format!("{head}/{next}"),
        None => head.to_string(),
    }
}

pub fn shots(game_dir: &Path) -> Found {
    let inside = game_dir.join(GAME_DIR);
    let mut out: Vec<Shot> = Vec::new();
    let mut shut: Vec<String> = Vec::new();
    let mut loose: BTreeSet<String> = BTreeSet::new();
    let mut archives: Vec<PathBuf> = Vec::new();

    for one in WalkDir::new(&inside)
        .follow_links(true)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|one| one.file_type().is_file())
    {
        let at = one.path();
        if backup::is_part(at) {
            continue;
        }

        if at.extension().is_some_and(|kind| kind == PACKED) {
            archives.push(at.to_path_buf());
            continue;
        }
        if !canvas::is_picture(at) {
            continue;
        }

        let Ok(under) = at.strip_prefix(&inside) else {
            continue;
        };
        let key = slashed(under);
        let named = Named::beside(key.clone(), &holder_of(&key), &key);

        loose.insert(key);
        out.push(of_file(named, at));
    }

    for at in archives {
        let Ok(under) = at.strip_prefix(&inside) else {
            continue;
        };
        let named = slashed(under);

        let index = match archive::listed(&at) {
            Ok(index) => index,
            Err(why) => {
                shut.push(format!("{named}: {why:#}"));
                continue;
            }
        };
        let mut spool = match archive::Spool::over(&at) {
            Ok(spool) => spool,
            Err(why) => {
                shut.push(format!("{named}: {why:#}"));
                continue;
            }
        };

        for (inside, held) in index.iter() {
            if loose.contains(inside) || !canvas::is_picture(Path::new(inside)) {
                continue;
            }

            let head = Head {
                raw: spool.head(held, pictures::HEAD).unwrap_or_default(),
                whole: held.size() as usize <= pictures::HEAD,
            };
            let found = measured_or(&head, || spool.read(held).ok());

            out.push(shot(
                Named::beside(key_of(&named, Some(inside)), &holder_of(inside), inside),
                &found,
                None,
            ));
        }
    }

    out.sort_by(|one, other| one.key.cmp(&other.key));

    Found { shots: out, shut }
}

fn inside_game(game_dir: &Path, key: &str) -> Result<PathBuf> {
    let held = Scope::read(key)?;
    if held.everything() {
        bail!("{key} does not name a picture inside this game");
    }

    Ok(held.under(&game_dir.join(GAME_DIR)))
}

pub fn picture(game_dir: &Path, store: &Path, key: &str) -> Result<Vec<u8>> {
    if let Some((named, inside)) = packed_in(key) {
        let at = backup::original_at_now(store, game_dir, &inside_game(game_dir, named)?)?;
        let held = archive::listed(&at)?;
        let held = held
            .get(inside)
            .with_context(|| format!("{named} no longer holds {inside}"))?;

        return archive::read(&at, held);
    }

    let at = backup::original_at_now(store, game_dir, &inside_game(game_dir, key)?)?;

    fs::read(&at).with_context(|| format!("reading {}", at.display()))
}

pub struct Landed {
    pub written: usize,
    pub put_back: usize,
    pub dropped: Vec<String>,
}

pub async fn land(at: &Install<'_>) -> Result<Landed> {
    let mut held = Landed {
        written: 0,
        put_back: 0,
        dropped: Vec::new(),
    };

    let wanted: Vec<PathBuf> = match at.reverting {
        true => Vec::new(),
        false => at
            .pictures
            .filled()
            .iter()
            .filter_map(|(key, _)| lands_at(at.game_dir, key).ok())
            .collect(),
    };

    held.put_back =
        backup::put_back_the_rest(at.store, at.game_dir, canvas::is_picture, &wanted).await?;

    if at.reverting {
        return Ok(held);
    }

    let known = LEDGER.remembered(at.store);
    let chosen = at.chosen().await;

    for (key, pick) in at.picked(&chosen) {
        if let Some(why) = known
            .iter()
            .find(|one| one.key == key)
            .and_then(|one| one.locked.as_ref())
        {
            held.dropped.push(format!("{key}: {why}"));
            continue;
        }

        let into = match unpacked_at(at.store, at.game_dir, key).await {
            Ok(into) => into,
            Err(why) => {
                held.dropped.push(format!("{key}: {why:#}"));
                continue;
            }
        };

        let shipped = match head_of(&into) {
            Ok(head) => pictures::measured(&head.raw).format().to_lowercase(),
            Err(why) => {
                held.dropped
                    .push(format!("{key}: reading {}: {why}", into.display()));
                continue;
            }
        };

        if shipped.is_empty() {
            held.dropped.push(format!(
                "{key}: the file it would replace is not a picture this reader can open"
            ));
            continue;
        }

        let body = match pictures::fitted(&shipped, None, &pick.raw) {
            Ok(body) => body,
            Err(why) => {
                held.dropped.push(format!("{key}: {why:#}"));
                continue;
            }
        };

        backup::replace(at.store, at.game_dir, &into, body).await?;
        held.written += 1;
    }

    Ok(held)
}

fn lands_at(game_dir: &Path, key: &str) -> Result<PathBuf> {
    match packed_in(key) {
        Some((_, inside)) => inside_game(game_dir, inside),
        None => inside_game(game_dir, key),
    }
}

async fn unpacked_at(store: &Path, game_dir: &Path, key: &str) -> Result<PathBuf> {
    let at = lands_at(game_dir, key)?;
    if at.is_file() {
        return Ok(at);
    }
    if packed_in(key).is_none() {
        bail!("it is not in this game any more, so nothing was written over it");
    }

    let held = key.to_string();
    let game = game_dir.to_path_buf();
    let kept = store.to_path_buf();
    let shipped = tokio::task::spawn_blocking(move || picture(&game, &kept, &held)).await??;

    if let Some(folder) = at.parent() {
        tokio::fs::create_dir_all(folder).await?;
    }
    store::write_atomically(&at, shipped).await?;

    Ok(at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::Canvas;
    use crate::engine::Swap;
    use crate::engine::pictures::Pictures;

    #[test]
    fn an_archive_this_reader_cannot_open_is_said_out_loud_rather_than_skipped() {
        let sandbox = tempfile::tempdir().expect("a temp folder");
        let inside = sandbox.path().join(GAME_DIR);
        fs::create_dir_all(&inside).expect("a game folder");
        fs::write(
            inside.join("broken.rpa"),
            b"RPA-9.9 nothing this reader knows",
        )
        .expect("an archive");

        let held = shots(sandbox.path());

        assert!(
            held.shots.is_empty(),
            "there is nothing in it this reader can list"
        );
        assert!(
            held.shut.iter().any(|why| why.contains("broken.rpa")),
            "every picture inside an archive that will not open is gone from the list, so the \
             reader has to be told which archive it was: {:?}",
            held.shut
        );
    }

    fn a_png(fill: u8) -> Vec<u8> {
        Canvas::of(4, 2, vec![fill; 32])
            .expect("a picture")
            .png()
            .expect("a png")
    }

    #[test]
    fn a_picture_still_inside_an_archive_is_listed_without_unpacking_a_single_byte() {
        let sandbox = tempfile::tempdir().expect("a temp folder");
        let game = sandbox.path().join(GAME_DIR);
        fs::create_dir_all(game.join("art")).expect("a folder");

        let held = a_png(7);
        fs::write(game.join("art").join("loose.png"), &held).expect("a loose picture");
        fs::write(
            game.join("art.rpa"),
            archive::sealed(
                &[
                    ("art/shot/day.png", &held, 0),
                    ("art/loose.png", &a_png(9), 0),
                    ("audio/theme.ogg", b"not a picture", 0),
                ],
                0x4242_4242,
                false,
            ),
        )
        .expect("an archive");

        let found = shots(sandbox.path()).shots;

        assert_eq!(
            found.iter().map(|one| one.key.as_str()).collect::<Vec<_>>(),
            ["art.rpa|art/shot/day.png", "art/loose.png"],
            "a game ships a gigabyte of pictures inside its archives, and unpacking them to see \
             them would double the game on disk for nothing"
        );
        assert_eq!(
            found[0].at, "art/shot/day.png",
            "the reader searches by the path Ren'Py loads a picture at, whether it is packed or \
             not, so the archive name belongs in the key and never in the path"
        );
        assert_eq!((found[0].wide, found[0].high), (4, 2));
        assert!(found[0].drawable && found[0].locked.is_none());
        assert_eq!(found[0].holder, "art/shot");

        assert_eq!(
            found[1].key, "art/loose.png",
            "Ren'Py reads a file on disk before it opens an archive, so a picture that sits loose \
             hides the packed copy and listing both would offer a replacement that changes nothing"
        );

        assert_eq!(
            picture(
                sandbox.path(),
                &sandbox.path().join("store"),
                "art.rpa|art/shot/day.png"
            )
            .expect("its bytes"),
            held,
            "showing a packed picture has to hand back exactly what the archive holds"
        );
    }

    #[tokio::test]
    async fn replacing_a_packed_picture_lays_the_shipped_one_down_first_so_it_can_come_back() {
        let sandbox = tempfile::tempdir().expect("a temp folder");
        let store = sandbox.path().join("store");
        let held = sandbox.path().join("held");
        let game = held.join(GAME_DIR);
        fs::create_dir_all(&game).expect("a folder");

        let was = a_png(7);
        fs::write(
            game.join("art.rpa"),
            archive::sealed(&[("art/shot/day.png", &was, 0)], 0x4242_4242, false),
        )
        .expect("an archive");

        let fresh = a_png(9);
        let picked = sandbox.path().join("picked.png");
        fs::write(&picked, &fresh).expect("a picked picture");

        let asked = Pictures {
            swaps: vec![Swap {
                from: "art.rpa|art/shot/day.png".to_string(),
                to: picked.to_string_lossy().to_string(),
            }],
            ..Pictures::default()
        };

        let done = land(&Install::over(&held, &store, &store).drawing(&asked))
            .await
            .expect("it writes in");
        assert_eq!((done.written, done.put_back), (1, 0));

        let at = game.join("art").join("shot").join("day.png");
        assert_eq!(
            fs::read(&at).expect("the picture"),
            fresh,
            "a loose file wins over the archive, so writing the pick beside the archive is what \
             makes the game draw it, and the archive is left untouched"
        );

        assert_eq!(
            picture(&held, &store, "art.rpa|art/shot/day.png").expect("the shipped picture"),
            was,
            "the editor shows the picture the game shipped even after a pick is in, or the reader \
             loses the one thing they were comparing their own picture against"
        );

        let done = land(&Install::over(&held, &store, &store).drawing(&Pictures::default()))
            .await
            .expect("it puts back");
        assert_eq!((done.written, done.put_back), (0, 1));
        assert_eq!(
            fs::read(&at).expect("the picture"),
            was,
            "clearing the pick has to leave the bytes the archive shipped where the game looks \
             first, or the reader is stuck with a picture they no longer want"
        );
    }

    #[test]
    fn what_a_packed_picture_holds_decides_its_format_and_never_the_name_it_wears() {
        let sandbox = tempfile::tempdir().expect("a temp folder");
        let game = sandbox.path().join(GAME_DIR);
        fs::create_dir_all(&game).expect("a folder");

        let held = Canvas::of(4, 2, vec![7; 32])
            .expect("a picture")
            .written_as("webp")
            .expect("a webp");
        fs::write(
            game.join("art.rpa"),
            archive::sealed(&[("art/day.png", &held, 0)], 0x4242_4242, false),
        )
        .expect("an archive");

        let found = shots(sandbox.path()).shots;

        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].format, "WEBP",
            "this game ships webp bytes under png names in their thousands, and trusting the name \
             would tell the reader a lie and write their replacement back in the wrong format"
        );
        assert_eq!((found[0].wide, found[0].high), (4, 2));
    }

    #[test]
    fn a_picture_is_named_by_the_path_renpy_loads_it_at() {
        let sandbox = tempfile::tempdir().expect("a temp folder");
        let game = sandbox.path().join(GAME_DIR);
        fs::create_dir_all(game.join("art").join("shot").join("bed")).expect("a folder");
        fs::create_dir_all(game.join("gui")).expect("a folder");

        let held = Canvas::of(4, 2, vec![7; 32])
            .expect("a picture")
            .png()
            .expect("a png");
        fs::write(
            game.join("art").join("shot").join("bed").join("day.png"),
            &held,
        )
        .expect("a picture");
        fs::write(game.join("gui").join("frame.png"), &held).expect("a picture");
        fs::write(game.join("script.rpy"), "label start:").expect("a script");

        let found = shots(sandbox.path()).shots;

        assert_eq!(
            found.iter().map(|one| one.at.as_str()).collect::<Vec<_>>(),
            ["art/shot/bed/day.png", "gui/frame.png"],
            "Ren'Py loads a picture by its path under the game folder, so that path is the name \
             the reader knows it by and the only one they could search for"
        );
        assert_eq!(
            (found[0].wide, found[0].high),
            (4, 2),
            "the size shown has to be the size in the file, or a replacement of the wrong shape \
             looks right until the game draws it"
        );
        assert!(found.iter().all(|one| one.drawable && one.locked.is_none()));
        assert_eq!(
            found[0].holder, "art/shot",
            "one row per folder would be thousands of rows in a game like this, so the rail \
             groups by the first two steps of the path and the search box does the rest"
        );
        assert_eq!(found[1].holder, "gui");

        fs::write(game.join("presplash.png"), &held).expect("a picture at the root");
        let held = shots(sandbox.path()).shots;
        assert_eq!(
            held.iter()
                .find(|one| one.name == "presplash.png")
                .map(|one| one.holder.as_str()),
            Some(GAME_DIR),
            "a picture sitting loose at the root belongs to the folder it is in, not to a rail \
             row named after itself"
        );
    }

    #[test]
    fn a_file_that_only_wears_a_picture_ending_is_shown_and_marked() {
        let sandbox = tempfile::tempdir().expect("a temp folder");
        let game = sandbox.path().join(GAME_DIR);
        fs::create_dir_all(&game).expect("a folder");
        fs::write(game.join("broken.png"), b"not a picture at all").expect("a file");

        let found = shots(sandbox.path()).shots;

        assert_eq!(found.len(), 1);
        assert!(
            !found[0].drawable && found[0].locked.is_some(),
            "dropping it would leave the reader wondering where a file they can see on disk went"
        );
    }

    #[tokio::test]
    async fn a_replaced_picture_is_put_back_the_moment_nobody_picks_it_any_more() {
        let sandbox = tempfile::tempdir().expect("a temp folder");
        let store = sandbox.path().join("store");
        let held = sandbox.path().join("held");
        let game = held.join(GAME_DIR);
        fs::create_dir_all(game.join("art")).expect("a folder");

        let was = Canvas::of(4, 2, vec![7; 32])
            .expect("a picture")
            .png()
            .expect("a png");
        let at = game.join("art").join("day.png");
        fs::write(&at, &was).expect("the shipped picture");

        let fresh = Canvas::of(4, 2, vec![9; 32])
            .expect("a picture")
            .png()
            .expect("a png");
        let picked = sandbox.path().join("picked.png");
        fs::write(&picked, &fresh).expect("a picked picture");

        let asked = Pictures {
            swaps: vec![Swap {
                from: "art/day.png".to_string(),
                to: picked.to_string_lossy().to_string(),
            }],
            ..Pictures::default()
        };

        let done = land(&Install::over(&held, &store, &store).drawing(&asked))
            .await
            .expect("it writes in");
        assert_eq!((done.written, done.put_back), (1, 0));
        assert_eq!(
            fs::read(&at).expect("the picture"),
            fresh,
            "what the reader picked has to be what the game loads"
        );

        let done = land(&Install::over(&held, &store, &store).drawing(&Pictures::default()))
            .await
            .expect("it puts back");
        assert_eq!((done.written, done.put_back), (0, 1));
        assert_eq!(
            fs::read(&at).expect("the picture"),
            was,
            "clearing a pick has to give the game back the file it shipped, or the reader can \
             never undo a picture they changed their mind about"
        );

        let done = land(
            &Install::over(&held, &store, &store)
                .drawing(&asked)
                .putting_back(true),
        )
        .await
        .expect("it reverts");
        assert_eq!(
            (done.written, done.put_back),
            (0, 0),
            "with nothing replaced there is nothing to put back, and reverting may not write a \
             pick in"
        );
    }

    #[tokio::test]
    async fn reverting_puts_back_a_picture_even_while_its_pick_still_stands() {
        let sandbox = tempfile::tempdir().expect("a temp folder");
        let store = sandbox.path().join("store");
        let held = sandbox.path().join("held");
        let game = held.join(GAME_DIR);
        fs::create_dir_all(&game).expect("a folder");

        let was = Canvas::of(4, 2, vec![7; 32])
            .expect("a picture")
            .png()
            .expect("a png");
        let at = game.join("title.png");
        fs::write(&at, &was).expect("the shipped picture");

        let picked = sandbox.path().join("picked.png");
        fs::write(
            &picked,
            Canvas::of(4, 2, vec![9; 32])
                .expect("a picture")
                .png()
                .expect("a png"),
        )
        .expect("a picked picture");

        let asked = Pictures {
            swaps: vec![Swap {
                from: "title.png".to_string(),
                to: picked.to_string_lossy().to_string(),
            }],
            ..Pictures::default()
        };

        land(&Install::over(&held, &store, &store).drawing(&asked))
            .await
            .expect("it writes");
        let done = land(
            &Install::over(&held, &store, &store)
                .drawing(&asked)
                .putting_back(true),
        )
        .await
        .expect("it reverts");

        assert_eq!(done.put_back, 1);
        assert_eq!(
            fs::read(&at).expect("the picture"),
            was,
            "Restore original files means the game goes back to how it shipped whatever the \
             reader still has picked"
        );
    }

    #[tokio::test]
    async fn a_pick_on_a_locked_row_is_dropped_with_its_reason() {
        let sandbox = tempfile::tempdir().expect("a temp folder");
        let store = sandbox.path().join("store");
        let held = sandbox.path().join("held");
        let game = held.join(GAME_DIR);
        fs::create_dir_all(&game).expect("a folder");
        fs::create_dir_all(&store).expect("a store");

        let was = Canvas::of(4, 2, vec![7; 32])
            .expect("a picture")
            .png()
            .expect("a png");
        let at = game.join("title.png");
        fs::write(&at, &was).expect("the shipped picture");

        LEDGER
            .remember(
                &store,
                &[Shot {
                    key: "title.png".to_string(),
                    holder: String::new(),
                    name: "title.png".to_string(),
                    kind: "Picture".to_string(),
                    atlas: String::new(),
                    wide: 4,
                    high: 2,
                    format: "PNG".to_string(),
                    saved_as: String::new(),
                    locked: Some("the editor marked this one as not replaceable".to_string()),
                    drawable: true,
                    at: String::new(),
                }],
            )
            .await
            .expect("a ledger");

        let picked = sandbox.path().join("picked.png");
        fs::write(
            &picked,
            Canvas::of(4, 2, vec![9; 32])
                .expect("a picture")
                .png()
                .expect("a png"),
        )
        .expect("a picked picture");

        let asked = Pictures {
            swaps: vec![Swap {
                from: "title.png".to_string(),
                to: picked.to_string_lossy().to_string(),
            }],
            ..Pictures::default()
        };

        let done = land(&Install::over(&held, &store, &store).drawing(&asked))
            .await
            .expect("it lands");

        assert_eq!(done.written, 0);
        assert!(
            done.dropped
                .iter()
                .any(|why| why.contains("not replaceable")),
            "the reason the row was locked is the reason the reader is shown: {:?}",
            done.dropped
        );
        assert_eq!(
            fs::read(&at).expect("the picture"),
            was,
            "a locked row keeps the file the game shipped"
        );
    }

    #[test]
    fn a_key_reaching_out_of_the_game_folder_is_refused() {
        let game = Path::new("/games/one");

        let under = game.join(GAME_DIR);

        for key in [
            "../../etc/passwd",
            "/etc/passwd",
            "",
            "art/../../../etc/passwd",
            r"C:\Windows\win.ini",
            "C:/Windows/win.ini",
            r"\\server\share\x.png",
        ] {
            match inside_game(game, key) {
                Err(_) => {}
                Ok(at) => assert!(
                    at.starts_with(&under),
                    "{key} landed at {} outside the game, and a key comes off a ledger on disk \
                     rather than from anyone this app trusts",
                    at.display()
                ),
            }
        }

        assert_eq!(
            inside_game(game, "art/day.png").expect("a picture inside the game"),
            game.join(GAME_DIR).join("art/day.png"),
            "a key names a file under the game folder and nowhere else"
        );
        assert_eq!(
            inside_game(game, "art/day..night.png").expect("a picture with dots in its name"),
            game.join(GAME_DIR).join("art/day..night.png"),
            "two dots inside a name are part of the name, and refusing them would hide a picture \
             the game really ships"
        );
    }
}
