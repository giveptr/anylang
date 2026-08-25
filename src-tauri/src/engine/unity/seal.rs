use crate::backup;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

const MANIFEST: &str = ".manifest";
const MARK: &str = "CRC: ";
const WIDEST: u64 = 64;

pub struct Reseal {
    pub at: PathBuf,
    pub body: String,
}

pub struct Sealed {
    pub was: u32,
    pub now: u32,
}

fn manifest_of(live: &Path) -> Option<PathBuf> {
    let mut name: OsString = live.file_name()?.to_owned();
    name.push(MANIFEST);

    Some(live.with_file_name(name))
}

fn told(text: &str, was: u32, now: u32) -> Option<String> {
    let held = format!("{MARK}{was}");

    let at = text
        .match_indices(&held)
        .map(|(at, _)| at)
        .find(|at| starts_a_line(text, *at) && ends_a_line(text, at + held.len()))?;

    Some(format!(
        "{}{MARK}{now}{}",
        &text[..at],
        &text[at + held.len()..]
    ))
}

fn starts_a_line(text: &str, at: usize) -> bool {
    at == 0 || text.as_bytes().get(at.wrapping_sub(1)) == Some(&b'\n')
}

fn ends_a_line(text: &str, at: usize) -> bool {
    matches!(text.as_bytes().get(at), None | Some(b'\r') | Some(b'\n'))
}

fn only(text: &str, was: u32, now: u32) -> Option<String> {
    let was = was.to_string();

    (text.trim() == was).then(|| text.replacen(&was, &now.to_string(), 1))
}

fn as_it_shipped(store: &Path, game_dir: &Path, at: &Path) -> Option<String> {
    String::from_utf8(backup::original_now(store, game_dir, at)?).ok()
}

pub async fn beside(store: &Path, game_dir: &Path, live: &Path, sealed: &Sealed) -> Vec<Reseal> {
    let mut found = Vec::new();

    if let Some(at) = manifest_of(live)
        && let Some(text) = as_it_shipped(store, game_dir, &at)
        && let Some(body) = told(&text, sealed.was, sealed.now)
    {
        found.push(Reseal { at, body });
    }

    let Some(folder) = live.parent() else {
        return found;
    };
    let Ok(mut listed) = tokio::fs::read_dir(folder).await else {
        return found;
    };

    while let Ok(Some(one)) = listed.next_entry().await {
        let at = one.path();
        if found.iter().any(|held| held.at == at) {
            continue;
        }
        if !one
            .metadata()
            .await
            .is_ok_and(|held| held.is_file() && held.len() <= WIDEST)
        {
            continue;
        }

        if let Some(text) = as_it_shipped(store, game_dir, &at)
            && let Some(body) = only(&text, sealed.was, sealed.now)
        {
            found.push(Reseal { at, body });
        }
    }

    found.sort_by(|a, b| a.at.cmp(&b.at));

    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const MANIFEST_BODY: &str = "ManifestFileVersion: 0\nUnityVersion: 6000.0.64f1\nCRC: \
                                 1904233381\nHashes:\n  AssetFileHash:\n    Hash: \
                                 de42c4704321955d5e097ad2b749877d\nHashAppended: 0\n";

    #[test]
    fn only_the_line_holding_the_check_a_bundle_had_is_written_over() {
        let fresh = told(MANIFEST_BODY, 1_904_233_381, 42).expect("the manifest names this check");

        assert!(fresh.contains("CRC: 42"));
        assert!(!fresh.contains("1904233381"));
        assert!(
            fresh.contains("Hash: de42c4704321955d5e097ad2b749877d"),
            "the other hashes a manifest carries are not ours to touch: they describe the assets \
             that went in, which we did not build"
        );
        assert!(
            fresh.ends_with('\n'),
            "a line ending the game shipped is part of the file and dropping it rewrites more \
             than the check"
        );
    }

    #[test]
    fn a_manifest_naming_some_other_check_is_left_alone() {
        assert_eq!(
            told(MANIFEST_BODY, 12345, 42),
            None,
            "the number we are replacing has to be the one this bundle actually had, or the \
             manifest belongs to a different bundle and writing to it would break that one"
        );
        assert_eq!(
            told("CRC: 19042333810\n", 1_904_233_381, 42),
            None,
            "a longer number that merely starts the same is a different check"
        );
        assert_eq!(
            told("TypeTreeCRC: 1904233381\n", 1_904_233_381, 42),
            None,
            "some other field ending in the same name is not the bundle's own check"
        );
    }

    #[test]
    fn the_line_endings_a_manifest_was_written_with_come_through_untouched() {
        let windows = "ManifestFileVersion: 0\r\nCRC: 1904233381\r\nHashAppended: 0\r\n";

        assert_eq!(
            told(windows, 1_904_233_381, 42).as_deref(),
            Some("ManifestFileVersion: 0\r\nCRC: 42\r\nHashAppended: 0\r\n"),
            "rewriting the breaks in a file we only meant to lift one number out of would show \
             up as the whole manifest having changed"
        );
    }

    #[test]
    fn a_file_that_is_nothing_but_the_check_is_written_over_whole() {
        assert_eq!(only("1904233381", 1_904_233_381, 42).as_deref(), Some("42"));
        assert_eq!(
            only("1904233381\n", 1_904_233_381, 42).as_deref(),
            Some("42\n"),
            "the whitespace around it is the game's own and comes through untouched"
        );
    }

    #[test]
    fn a_file_that_merely_mentions_the_check_is_never_rewritten() {
        for text in [
            "crc=1904233381 and more",
            "1904233381 1904233381",
            "19042333810",
            "",
        ] {
            assert_eq!(
                only(text, 1_904_233_381, 42),
                None,
                "{text:?} holds something besides the check, so this reader cannot tell what \
                 replacing it would mean"
            );
        }
    }

    #[test]
    fn a_manifest_is_named_after_the_whole_file_and_not_what_is_left_of_it() {
        assert_eq!(
            manifest_of(Path::new("/game/StreamingAssets/restorationpatch")),
            Some(PathBuf::from(
                "/game/StreamingAssets/restorationpatch.manifest"
            ))
        );
        assert_eq!(
            manifest_of(Path::new("/game/one.bundle")),
            Some(PathBuf::from("/game/one.bundle.manifest")),
            "Unity keeps the whole name and adds to it, so trading the ending away would look \
             for a file nobody wrote"
        );
    }

    struct Sandbox {
        dir: tempfile::TempDir,
        game: PathBuf,
        store: PathBuf,
        live: PathBuf,
    }

    fn sandbox() -> Sandbox {
        let dir = tempfile::tempdir().expect("a temp folder");
        let game = dir.path().join("game");
        let folder = game.join("StreamingAssets");
        fs::create_dir_all(&folder).expect("a folder");

        let live = folder.join("restorationpatch");
        fs::write(&live, b"a bundle").expect("a bundle");
        fs::write(folder.join("restorationpatch.manifest"), MANIFEST_BODY).expect("a manifest");
        fs::write(folder.join("check"), "1904233381").expect("a check");
        fs::write(folder.join("localization_percent.txt"), "en 100").expect("a note");
        fs::write(folder.join("other"), "12345").expect("someone else's check");

        Sandbox {
            store: dir.path().join("store"),
            game,
            live,
            dir,
        }
    }

    impl Sandbox {
        async fn found(&self, now: u32) -> Vec<Reseal> {
            beside(
                &self.store,
                &self.game,
                &self.live,
                &Sealed {
                    was: 1_904_233_381,
                    now,
                },
            )
            .await
        }
    }

    #[tokio::test]
    async fn a_seal_written_over_by_an_earlier_export_is_still_read_as_the_game_shipped_it() {
        let sand = sandbox();
        let at = sand.live.with_file_name("check");

        for one in &sand.found(42).await {
            backup::replace(
                &sand.store,
                &sand.game,
                &one.at,
                one.body.clone().into_bytes(),
            )
            .await
            .expect("an export lands");
        }

        assert_eq!(
            fs::read_to_string(&at).expect("the check"),
            "42",
            "the first export has to have moved the check, or this test guards nothing"
        );

        let again = sand.found(7).await;

        assert_eq!(
            again
                .iter()
                .map(|one| one.body.as_str())
                .collect::<Vec<_>>()
                .first()
                .copied(),
            Some("7"),
            "the second export rebuilds the bundle from the original, so the check it is lifting \
             is the original one too: reading what the last export left would find nothing to \
             replace and quietly leave the game sealed against a bundle it no longer has"
        );

        let _ = &sand.dir;
    }

    #[tokio::test]
    async fn every_seal_a_bundle_left_beside_it_is_found_and_nothing_else_is() {
        let sand = sandbox();
        let found = sand.found(42).await;

        assert_eq!(
            found
                .iter()
                .map(|one| one.at.file_name().expect("a name").to_string_lossy())
                .collect::<Vec<_>>(),
            ["check", "restorationpatch.manifest"],
            "a check the game keeps apart from the manifest counts too, and a file holding some \
             other number is somebody else's to keep"
        );
        assert_eq!(found[0].body, "42");
        assert!(found[1].body.contains("CRC: 42"));

        let _ = &sand.dir;
    }

    #[tokio::test]
    async fn a_bundle_nobody_sealed_leaves_nothing_to_write() {
        let sandbox = tempfile::tempdir().expect("a temp folder");
        let live = sandbox.path().join("one.bundle");
        fs::write(&live, b"a bundle").expect("a bundle");

        assert!(
            beside(
                sandbox.path(),
                sandbox.path(),
                &live,
                &Sealed { was: 7, now: 42 }
            )
            .await
            .is_empty(),
            "most bundles carry no check at all, and inventing one would leave a file the game \
             never shipped"
        );
    }
}
