#[cfg(test)]
use crate::canvas::Canvas;
use crate::engine::Install;
use crate::engine::pictures::{Head, Measured, Shot, head_of, measured, measured_or, shot, unread};
use crate::engine::rpg_maker::js::{CORE, DATA, SCRIPTS, SYSTEM, content_root, crypt};
use crate::engine::rpg_maker::pictures;
use crate::scope::Scope;
use crate::{backup, walk};
use anyhow::{Context, Result, bail};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

const NAMES_THE_KEY: &str = "encryptionKey";

const LOCKED: [&str; 2] = ["rpgmvp", "png_"];

const TRIED: usize = 64;
const AGREED: usize = 2;

const NO_KEY: &str = "this picture is locked, neither data/System.json nor this game's own \
                      scripts name the key, and no two of its pictures agree on one, so it can be \
                      neither shown nor replaced";

fn ours(name: &str) -> bool {
    pictures::drawn_name(name) || locked_name(name)
}

fn locked_name(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, kind)| LOCKED.iter().any(|one| kind.eq_ignore_ascii_case(one)))
}

fn named_key(root: &Path) -> Option<crypt::Key> {
    let body = fs::read_to_string(root.join(DATA).join(SYSTEM)).ok()?;
    let held: serde_json::Value = serde_json::from_str(&body).ok()?;

    crypt::Key::read(held.get(NAMES_THE_KEY)?.as_str()?)
}

fn core_key(root: &Path) -> Option<crypt::Key> {
    static RE_KEY: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r#"_encryptionKey\s*=\s*["']([0-9A-Fa-f]{32})["']"#)
            .expect("RE_KEY is a valid pattern")
    });

    for name in CORE {
        let Ok(body) = fs::read_to_string(root.join(SCRIPTS).join(name)) else {
            continue;
        };

        if let Some(found) = RE_KEY.captures(&body).and_then(|held| held.get(1))
            && let Some(key) = crypt::Key::read(found.as_str())
        {
            return Some(key);
        }
    }

    None
}

fn agreed_key(locked: &[PathBuf]) -> Option<crypt::Key> {
    let mut seen: Vec<(crypt::Key, usize)> = Vec::new();

    for at in locked.iter().take(TRIED) {
        let Ok(head) = head_of(at) else {
            continue;
        };
        let Some(candidate) = crypt::key_behind(&head.raw) else {
            continue;
        };

        match seen.iter_mut().find(|(held, _)| *held == candidate) {
            Some((_, many)) => *many += 1,
            None => seen.push((candidate, 1)),
        }
    }

    seen.into_iter()
        .filter(|(_, many)| *many >= AGREED)
        .max_by_key(|(_, many)| *many)
        .map(|(held, _)| held)
}

fn opens_one(key: &crypt::Key, locked: &[PathBuf]) -> bool {
    if locked.is_empty() {
        return true;
    }

    locked
        .iter()
        .take(TRIED)
        .any(|at| head_of(at).is_ok_and(|head| tried(Some(key), &head.raw).is_some()))
}

fn key_of(root: &Path, locked: impl FnOnce() -> Vec<PathBuf>) -> Option<crypt::Key> {
    let locked = locked();

    named_key(root)
        .filter(|key| opens_one(key, &locked))
        .or_else(|| core_key(root).filter(|key| opens_one(key, &locked)))
        .or_else(|| agreed_key(&locked))
}

fn tried(key: Option<&crypt::Key>, raw: &[u8]) -> Option<Vec<u8>> {
    let body = key?.opened(raw)?;

    measured(&body).sized().map(|_| body)
}

fn locked_in(root: &Path) -> Vec<PathBuf> {
    ours_in(root)
        .into_iter()
        .filter(|(_, name)| locked_name(name))
        .map(|(at, _)| at)
        .collect()
}

fn ours_in(root: &Path) -> Vec<(PathBuf, String)> {
    let mut found: Vec<(PathBuf, String)> = walk::files_now(root)
        .into_iter()
        .filter_map(|at| {
            let relative = at.strip_prefix(root).ok()?;
            if backup::is_part(relative) {
                return None;
            }

            let name = relative.file_name()?.to_str()?.to_string();

            ours(&name).then_some((at, name))
        })
        .collect();
    found.sort();

    found
}

fn opened(key: Option<&crypt::Key>, raw: &[u8]) -> Result<Vec<u8>, String> {
    if crypt::locked_head(raw).is_none() {
        return Ok(raw.to_vec());
    }

    let key = key.ok_or_else(|| NO_KEY.to_string())?;

    key.opened(raw).ok_or_else(|| {
        "this picture carries the header the engine locks its pictures with and nothing behind it"
            .to_string()
    })
}

fn whole(at: &Path, key: Option<&crypt::Key>) -> Option<Vec<u8>> {
    opened(key, &fs::read(at).ok()?).ok()
}

fn one(at: &Path, relative: &Path, key: Option<&crypt::Key>) -> Shot {
    let named = pictures::named_at(relative);

    let head = match head_of(at) {
        Ok(head) => head,
        Err(why) => return unread(named, why),
    };

    let body = match opened(key, &head.raw) {
        Ok(body) => body,
        Err(why) => return shot(named, &Measured::Unknown, Some(why)),
    };

    let held = measured_or(
        &Head {
            raw: body,
            whole: head.whole,
        },
        || whole(at, key),
    );
    shot(named, &held, None)
}

pub async fn listed(root: &Path) -> Vec<Shot> {
    let found = ours_in(root);
    let key = key_of(root, || {
        found
            .iter()
            .filter(|(_, name)| locked_name(name))
            .map(|(at, _)| at.clone())
            .collect()
    });

    let apart = found.len().div_ceil(walk::at_once()).max(1);
    let reading = found.chunks(apart).map(|held| {
        let held: Vec<PathBuf> = held.iter().map(|(at, _)| at.clone()).collect();
        let root = root.to_path_buf();

        tokio::task::spawn_blocking(move || measured_in(&root, &held, key.as_ref()))
    });

    let mut out: Vec<Shot> = futures::future::join_all(reading)
        .await
        .into_iter()
        .flat_map(Result::unwrap_or_default)
        .collect();
    out.sort_by(|left, right| left.key.cmp(&right.key));

    out
}

fn measured_in(root: &Path, held: &[PathBuf], key: Option<&crypt::Key>) -> Vec<Shot> {
    held.iter()
        .filter_map(|at| {
            let relative = at.strip_prefix(root).ok()?;

            Some(one(at, relative, key))
        })
        .collect()
}

pub fn picture(game_dir: &Path, store: &Path, key: &str) -> Result<Vec<u8>> {
    let root = content_root(game_dir);
    let at = Scope::read(key)?.under(&root);
    let from = backup::original_at_now(store, game_dir, &at)?;
    let raw = fs::read(&from).with_context(|| format!("reading {}", from.display()))?;

    if crypt::locked_head(&raw).is_none() {
        return match measured(&raw).sized() {
            Some(_) => Ok(raw),
            None => bail!("{key} did not come out as an image this reader can open"),
        };
    }

    let held = key_of(&root, || locked_in(&root));
    match tried(held.as_ref(), &raw) {
        Some(body) => Ok(body),
        None => bail!("{key}: {NO_KEY}"),
    }
}

fn shaped(was: &[u8], key: Option<&crypt::Key>, picked: &[u8]) -> Result<Vec<u8>> {
    let Some(head) = crypt::locked_head(was) else {
        return pictures::written(was, picked);
    };

    let key = key.ok_or_else(|| anyhow::anyhow!(NO_KEY))?;
    let shipped = key.opened(was).unwrap_or_default();
    let body = pictures::written(&shipped, picked)?;

    Ok(key.locked(head, &body))
}

pub async fn run(root: &Path, at: &Install<'_>) -> Result<()> {
    let chosen = at.chosen().await;
    let picked = at.picked(&chosen);

    let key = key_of(root, || locked_in(root));
    let mine = |file: &Path| {
        file.strip_prefix(root)
            .ok()
            .and_then(|relative| relative.file_name())
            .and_then(OsStr::to_str)
            .is_some_and(ours)
    };

    let wrote = pictures::into_folder(at, root, &picked, mine, move |shipped, held| {
        shaped(&shipped, key.as_ref(), &held)
    })
    .await?;

    if wrote > 0 {
        at.progress
            .info(at.doing, &format!("{wrote} picture(s) written in"));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::pictures::Pictures;
    use crate::engine::rpg_maker::fixture::{self, Game};
    use crate::engine::rpg_maker::pictures::dotted;
    use crate::progress::{Heard, Progress, Quiet};

    const SAID: &str = "d41d8cd98f00b204e9800998ecf8427e";

    fn a_game(names_a_key: bool) -> Game {
        let game = fixture::a_game();
        let system = match names_a_key {
            true => format!("{{\"{NAMES_THE_KEY}\":\"{SAID}\",\"hasEncryptedImages\":true}}"),
            false => "{\"hasEncryptedImages\":true}".to_string(),
        };

        game.put(&format!("{DATA}/{SYSTEM}"), system.as_bytes());

        game
    }

    fn a_key() -> crypt::Key {
        crypt::Key::read(SAID).expect("a key")
    }

    fn ship(game: &Game, at: &str, drawn: &Canvas) -> Vec<u8> {
        let png = drawn.png().expect("a png");
        let body = match locked_name(at) {
            true => {
                let mut head = [0u8; crypt::HEAD];
                head[..5].copy_from_slice(b"RPGMV");
                head[9] = 3;
                head[10] = 1;

                a_key().locked(&head, &png)
            }
            false => png.clone(),
        };

        game.put(at, &body);

        png
    }

    async fn install(
        game: &Game,
        picked: &Pictures,
        reverting: bool,
        progress: &dyn Progress,
    ) -> Result<()> {
        run(
            game.root(),
            &Install::over(game.root(), game.staged.path(), game.store.path())
                .drawing(picked)
                .putting_back(reverting)
                .heard_by(progress),
        )
        .await
    }

    #[tokio::test]
    async fn every_picture_a_game_ships_is_listed_whether_this_reader_can_open_it_or_not() {
        let game = a_game(true);
        ship(&game, "img/pictures/talk.rpgmvp", &dotted(40, 24, 1));
        ship(&game, "img/system/Window.png", &dotted(16, 16, 2));
        game.put(
            "img/pictures/torn.rpgmvp",
            b"RPGMV\0\0\0\0\x03\x01\0\0\0\0\0not a picture",
        );
        game.put("audio/bgm/theme.rpgmvo", b"never ours");
        game.put("data/Map001.json", b"{}");

        let shots = listed(game.root()).await;
        let keys: Vec<&str> = shots.iter().map(|one| one.key.as_str()).collect();

        assert_eq!(
            keys,
            [
                "img/pictures/talk.rpgmvp",
                "img/pictures/torn.rpgmvp",
                "img/system/Window.png"
            ],
            "the audio a game locks the same way is somebody else's job, and the pictures are \
             listed in one order so the editor's tree does not shuffle between reads"
        );

        let talk = &shots[0];
        assert_eq!((talk.wide, talk.high), (40, 24));
        assert_eq!(talk.format, "PNG");
        assert_eq!(talk.holder, "img/pictures");
        assert_eq!(talk.name, "talk.rpgmvp");
        assert!(
            talk.drawable && talk.locked.is_none(),
            "a locked picture is the normal case in this game: 1857 of its 1860 are locked, so a \
             reader who cannot reach them can reach nothing"
        );

        assert!(
            !shots[1].drawable && shots[1].locked.is_some(),
            "a file that does not come out as an image is shown and marked, never dropped: \
             dropping it would leave the reader counting rows that do not add up"
        );
    }

    #[tokio::test]
    async fn a_game_that_no_longer_names_its_key_shows_its_pictures_and_says_why_they_are_shut() {
        let game = a_game(false);
        ship(&game, "img/pictures/talk.rpgmvp", &dotted(8, 8, 3));

        let held = listed(game.root()).await;
        let [one] = &held[..] else {
            panic!("the one picture is still listed")
        };

        assert!(
            !one.drawable,
            "without the key there is nothing behind the header, and drawing the locked bytes \
             would show the reader noise"
        );
        assert_eq!(
            one.locked.as_deref(),
            Some(NO_KEY),
            "the reader has to be told the game is missing the field, not that the picture is broken"
        );
    }

    #[tokio::test]
    async fn a_game_that_kept_its_key_out_of_system_json_still_gives_its_pictures_up() {
        let game = a_game(false);
        let was = ship(&game, "img/pictures/one.rpgmvp", &dotted(40, 24, 1));
        ship(&game, "img/pictures/two.rpgmvp", &dotted(16, 16, 2));

        let shots = listed(game.root()).await;
        assert_eq!(shots.len(), 2);
        assert!(
            shots.iter().all(|one| one.drawable && one.locked.is_none()),
            "a game deployed with the field stripped out is still a game whose every picture \
             opens with the same sixteen png bytes, so the key it hid is the key its own files \
             agree on: {:?}",
            shots
                .iter()
                .map(|one| one.locked.clone())
                .collect::<Vec<_>>()
        );
        assert_eq!((shots[0].wide, shots[0].high), (40, 24));

        assert_eq!(
            picture(game.root(), game.store(), "img/pictures/one.rpgmvp").expect("it opens"),
            was,
            "and the picture handed to the editor is the one the game draws, not a guess"
        );
    }

    #[test]
    fn a_locked_picture_is_handed_over_as_the_png_the_game_draws() {
        let game = a_game(true);
        let png = ship(&game, "img/pictures/talk.rpgmvp", &dotted(40, 24, 9));

        let held = picture(game.root(), game.store(), "img/pictures/talk.rpgmvp")
            .expect("the picture opens");
        assert_eq!(
            held, png,
            "the preview a reader sees has to be the picture that is in the game"
        );
        assert_eq!(
            Canvas::read(&held).expect("it draws").wide,
            40,
            "and it has to be a picture an image reader can actually open"
        );
    }

    #[test]
    fn a_game_that_kept_its_key_only_in_its_own_engine_still_opens_its_pictures() {
        let game = a_game(false);
        game.put(
            "js/rpg_core.js",
            format!("Decrypter._encryptionKey = \"{SAID}\";").as_bytes(),
        );
        let png = ship(&game, "img/pictures/talk.rpgmvp", &dotted(40, 24, 9));

        assert_eq!(
            picture(game.root(), game.store(), "img/pictures/talk.rpgmvp")
                .expect("the picture opens"),
            png,
            "a game whose key was stripped out of System.json still draws its pictures, so the \
             key its own engine kept is the same key this reader has to use"
        );
    }

    #[test]
    fn a_key_that_could_climb_out_of_the_game_folder_never_reaches_the_filesystem() {
        let game = a_game(true);

        for asked in ["../../etc/passwd", "..", "img/../../outside.png"] {
            let why = picture(game.root(), game.store(), asked)
                .expect_err("a key that climbs out of the game folder")
                .to_string();

            assert!(
                why.contains("is not part of this game's text"),
                "{asked} came out of a ledger on disk, and a key is not a path this app follows \
                 wherever it points, so it has to be turned away for climbing out rather than \
                 for whatever it happened to land on: {why}"
            );
        }
    }

    #[tokio::test]
    async fn a_picked_picture_goes_in_locked_and_the_game_gets_its_own_back_when_it_is_dropped() {
        let game = a_game(true);
        let was = ship(&game, "img/pictures/talk.rpgmvp", &dotted(40, 24, 1));
        let shipped = game.bytes("img/pictures/talk.rpgmvp");

        let fresh = dotted(40, 24, 77);
        let picked = game.pick("img/pictures/talk.rpgmvp", &fresh);

        install(&game, &picked, false, &Quiet)
            .await
            .expect("an install that goes through");

        let after = game.bytes("img/pictures/talk.rpgmvp");
        assert_ne!(after, shipped, "the file on disk was rewritten");
        assert!(
            crypt::locked_head(&after).is_some(),
            "the engine checks all sixteen header bytes before it draws, so a picture written in \
             plain would leave the game unable to load it"
        );

        let held =
            picture(game.root(), game.store(), "img/pictures/talk.rpgmvp").expect("it opens again");
        assert_eq!(
            Canvas::read(&held).expect("it draws").pixels,
            dotted(40, 24, 1).pixels,
            "the editor shows the picture the game shipped even after a pick is in, or the reader \
             loses the one thing they were comparing their own picture against"
        );

        install(&game, &Pictures::default(), false, &Quiet)
            .await
            .expect("a second install with nothing picked");

        assert_eq!(
            game.bytes("img/pictures/talk.rpgmvp"),
            shipped,
            "dropping the pick has to give the game back the file it shipped, byte for byte"
        );
        assert_eq!(
            picture(game.root(), game.store(), "img/pictures/talk.rpgmvp").expect("it opens"),
            was
        );
    }

    #[tokio::test]
    async fn a_picture_of_another_size_is_fitted_to_the_spot_the_game_draws_it_in() {
        let game = a_game(true);
        ship(&game, "img/system/Window.png", &dotted(96, 96, 4));

        let picked = game.pick("img/system/Window.png", &dotted(300, 120, 8));
        install(&game, &picked, false, &Quiet)
            .await
            .expect("an install that goes through");

        let held = Canvas::read(&game.bytes("img/system/Window.png")).expect("it draws");
        assert_eq!(
            (held.wide, held.high),
            (96, 96),
            "a window skin is read by fixed pixel rects, so a replacement of another size would \
             leave the game drawing corners out of the middle of the picture"
        );
    }

    #[tokio::test]
    async fn asking_for_the_game_back_puts_every_picture_back_even_with_the_pick_still_set() {
        let game = a_game(true);
        ship(&game, "img/pictures/talk.rpgmvp", &dotted(20, 20, 1));
        let shipped = game.bytes("img/pictures/talk.rpgmvp");

        let picked = game.pick("img/pictures/talk.rpgmvp", &dotted(20, 20, 60));
        install(&game, &picked, false, &Quiet)
            .await
            .expect("the picture goes in");
        assert_ne!(game.bytes("img/pictures/talk.rpgmvp"), shipped);

        install(&game, &picked, true, &Quiet)
            .await
            .expect("the game back");

        assert_eq!(
            game.bytes("img/pictures/talk.rpgmvp"),
            shipped,
            "Restore original files has to undo a picture the same way it undoes a line of text, \
             whatever the reader still has picked"
        );
    }

    #[tokio::test]
    async fn a_pick_naming_a_picture_this_game_no_longer_ships_is_said_out_loud() {
        let game = a_game(true);
        ship(&game, "img/pictures/talk.rpgmvp", &dotted(8, 8, 1));

        let heard = Heard::default();
        let picked = game.pick("img/pictures/gone.rpgmvp", &dotted(8, 8, 2));
        install(&game, &picked, false, &heard)
            .await
            .expect("an install that goes through");

        assert_eq!(
            heard.warnings(),
            ["img/pictures/gone.rpgmvp is not a picture this game ships any more"],
            "the reader picked a file and nothing came of it, so saying nothing would leave them \
             waiting on a change that is never coming"
        );
        assert!(!game.root().join("img/pictures/gone.rpgmvp").exists());
    }

    #[tokio::test]
    async fn a_game_that_locks_its_pictures_with_a_key_it_never_names_is_not_written_over() {
        let game = a_game(false);
        ship(&game, "img/pictures/talk.rpgmvp", &dotted(8, 8, 1));
        let shipped = game.bytes("img/pictures/talk.rpgmvp");

        let heard = Heard::default();
        let picked = game.pick("img/pictures/talk.rpgmvp", &dotted(8, 8, 5));
        install(&game, &picked, false, &heard)
            .await
            .expect("an install that goes through");

        assert_eq!(
            game.bytes("img/pictures/talk.rpgmvp"),
            shipped,
            "a picture written in without the lock the game expects would leave that picture \
             missing from the game: guessing at the key is worse than leaving it alone"
        );
        assert!(heard.warnings().iter().any(|said| said.contains(NO_KEY)));
    }
}
