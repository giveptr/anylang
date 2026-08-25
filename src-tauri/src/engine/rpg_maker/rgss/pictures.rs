#[cfg(test)]
use crate::canvas::Canvas;
use crate::engine::Install;
use crate::engine::pictures::{
    Chosen, HEAD, Head, Named, Shot, key_of, measured, measured_or, of_file, packed_in, shot,
};
use crate::engine::rpg_maker::pictures;
use crate::engine::rpg_maker::rgss::archive;
use crate::engine::rpg_maker::rgss::source::Source;
use crate::scope::{Scope, slashed};
use crate::store::Stamp;
use crate::{backup, store, walk};
use anyhow::{Context, Result, bail};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

const GRAPHICS: &str = "Graphics";

const SHADOWED: &str = "the game reads the archive before the folder, so replacing this loose \
                        copy would change nothing on screen. Replace the packed copy instead";

type Table = Arc<Vec<archive::Entry>>;
type Remembered = Option<(PathBuf, Stamp, Table)>;

static TABLE: LazyLock<Mutex<Remembered>> = LazyLock::new(|| Mutex::new(None));

fn table_of(at: &Path) -> Result<Table> {
    let stamp = store::stamp_of(at);

    if let Ok(held) = TABLE.lock()
        && let Some((was, when, entries)) = held.as_ref()
        && was == at
        && *when == stamp
    {
        return Ok(Arc::clone(entries));
    }

    let entries = Arc::new(
        archive::opened(at)
            .map_err(|why| anyhow::anyhow!("{} is not an archive: {why}", at.display()))?,
    );

    if let Ok(mut held) = TABLE.lock() {
        *held = Some((at.to_path_buf(), stamp, Arc::clone(&entries)));
    }

    Ok(entries)
}

fn ranged(at: &Path, entry: &archive::Entry) -> Result<Vec<u8>> {
    archive::read(at, entry).map_err(|why| anyhow::anyhow!("{why}"))
}

fn named_inside(archive: &str, entry: &str) -> Named {
    let folder = entry.rsplit_once('/').map(|(folder, _)| folder);
    let holder = match folder {
        Some(folder) => format!("{archive}/{folder}"),
        None => archive.to_string(),
    };

    Named::beside(key_of(archive, Some(entry)), &holder, entry)
}

fn archive_named(at: &Path) -> String {
    at.file_name()
        .map(|one| one.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn packed_shots(at: &Path, raw: &[u8], entries: &[archive::Entry]) -> Vec<Shot> {
    let named = archive_named(at);
    let mut out = Vec::new();

    for one in entries {
        if !pictures::drawn_name(&one.name) {
            continue;
        }

        let (ahead, whole) = archive::head(raw, one, HEAD);
        let head = Head { raw: ahead, whole };
        let held = measured_or(&head, || Some(archive::body(raw, one)));

        out.push(shot(named_inside(&named, &one.name), &held, None));
    }

    out
}

fn graphics_in(game_dir: &Path) -> Vec<(PathBuf, PathBuf)> {
    walk::files_now(&game_dir.join(GRAPHICS))
        .into_iter()
        .filter_map(|at| {
            let relative = at.strip_prefix(game_dir).ok()?.to_path_buf();
            let name = relative.file_name()?.to_str()?;

            (!backup::is_part(&relative) && pictures::drawn_name(name)).then_some((at, relative))
        })
        .collect()
}

fn loose_shots(game_dir: &Path) -> Vec<Shot> {
    let mut out = Vec::new();

    for (at, relative) in graphics_in(game_dir) {
        let named = pictures::named_at(&relative);

        out.push(of_file(named, &at));
    }

    out
}

pub struct Listed {
    pub shots: Vec<Shot>,
    pub behind: usize,
}

pub fn listed(held: &Source, game_dir: &Path) -> Listed {
    let shut = shut_out(held);
    let mut out = loose_shots(game_dir);
    let before = out.len();

    out.retain(|one| !shut.contains(&one.at));
    let behind = before - out.len();

    if let Source::Packed { at, raw, entries } = held {
        out.extend(packed_shots(at, raw, entries));
    }

    out.sort_by(|left, right| left.key.cmp(&right.key));

    Listed { shots: out, behind }
}

pub fn picture(game_dir: &Path, store: &Path, key: &str) -> Result<Vec<u8>> {
    let body = match packed_in(key) {
        Some((named, entry)) => out_of(game_dir, store, named, entry)?,
        None => {
            let at = Scope::read(key)?.under(game_dir);
            let from = backup::original_at_now(store, game_dir, &at)?;

            fs::read(&from).with_context(|| format!("reading {}", from.display()))?
        }
    };

    match measured(&body).sized() {
        Some(_) => Ok(body),
        None => bail!("{key} did not come out as an image this reader can open"),
    }
}

fn out_of(game_dir: &Path, store: &Path, named: &str, entry: &str) -> Result<Vec<u8>> {
    let at = backup::original_at_now(store, game_dir, &Scope::read(named)?.under(game_dir))?;
    let entries = table_of(&at)?;
    let found = entries
        .iter()
        .find(|one| one.name == entry)
        .ok_or_else(|| anyhow::anyhow!("{named} no longer holds {entry}"))?;

    ranged(&at, found)
}

fn wanted<'e>(
    named: &str,
    entries: &'e [archive::Entry],
    key: &str,
) -> Option<(usize, &'e archive::Entry)> {
    let (whose, entry) = packed_in(key)?;
    if whose != named {
        return None;
    }

    entries
        .iter()
        .enumerate()
        .find(|(_, one)| one.name == entry)
}

type Picked<'p> = Vec<(&'p str, &'p Chosen)>;

fn into_archive(
    at: &Install<'_>,
    named: &str,
    raw: &[u8],
    entries: &[archive::Entry],
    picked: Picked<'_>,
) -> Vec<(usize, Vec<u8>)> {
    let mut edits = Vec::new();

    for (key, held) in picked {
        let Some((which, entry)) = wanted(named, entries, key) else {
            at.progress.warn(
                at.doing,
                &format!("{key} is not a picture {named} holds any more"),
            );
            continue;
        };

        let shipped = archive::body(raw, entry);
        match pictures::written(&shipped, &held.raw) {
            Ok(body) => edits.push((which, body)),
            Err(why) => at.progress.warn(at.doing, &format!("{key}: {why:#}")),
        }
    }

    edits
}

async fn into_folder(
    at: &Install<'_>,
    picked: Picked<'_>,
    shadowed: &BTreeSet<String>,
) -> Result<usize> {
    let mut wanted = Vec::with_capacity(picked.len());

    for (key, held) in picked {
        match shadowed.contains(key) {
            true => at.progress.warn(at.doing, &format!("{key}: {SHADOWED}")),
            false => wanted.push((key, held)),
        }
    }

    let mine = |file: &Path| {
        file.strip_prefix(at.game_dir)
            .is_ok_and(|under| under.starts_with(GRAPHICS) && pictures::drawn_name(&slashed(under)))
    };

    pictures::into_folder(at, at.game_dir, &wanted, mine, |shipped, held| {
        pictures::written(&shipped, &held)
    })
    .await
}

pub async fn written(held: &Source, at: &Install<'_>) -> Result<Vec<(usize, Vec<u8>)>> {
    let chosen = at.chosen().await;

    let (inside, beside): (Picked<'_>, Picked<'_>) = at
        .picked(&chosen)
        .into_iter()
        .partition(|(key, _)| packed_in(key).is_some());

    let edits = match held {
        Source::Packed {
            at: named,
            raw,
            entries,
        } => into_archive(at, &archive_named(named), raw, entries, inside),
        Source::Loose { .. } => {
            for (key, _) in &inside {
                at.progress.warn(
                    at.doing,
                    &format!("{key} names a picture inside an archive this game no longer has"),
                );
            }

            Vec::new()
        }
    };

    let many = edits.len() + into_folder(at, beside, &shut_out(held)).await?;
    if many > 0 {
        at.progress
            .info(at.doing, &format!("{many} picture(s) written in"));
    }

    Ok(edits)
}

fn shut_out(held: &Source) -> BTreeSet<String> {
    match held {
        Source::Packed { entries, .. } => entries.iter().map(|one| one.name.clone()).collect(),
        Source::Loose { .. } => BTreeSet::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas;
    use crate::engine::fonts::Fonts;
    use crate::engine::pictures::Pictures;
    use crate::engine::rpg_maker::fixture::{Game, a_game};
    use crate::engine::rpg_maker::pictures::dotted;
    use crate::engine::rpg_maker::rgss::install;
    use crate::progress::{Heard, Progress, Quiet};
    use std::io::Cursor;

    const ARCHIVE: &str = "Game.rgss3a";
    const FACE: &str = "Graphics/Faces/Actor1.png";

    fn a_bmp(wide: u32, high: u32) -> Vec<u8> {
        let mut out = Vec::new();
        image::RgbaImage::new(wide, high)
            .write_to(&mut Cursor::new(&mut out), image::ImageFormat::Bmp)
            .expect("a bmp");

        out
    }

    fn pack(game: &Game, files: &[(&str, &[u8])]) {
        fs::write(game.root().join(ARCHIVE), archive::packed(files)).expect("an archive");
    }

    async fn opened(game: &Game) -> Source {
        Source::open(game.root(), game.store())
            .await
            .expect("a game that opens")
            .expect("a game")
    }

    async fn install(game: &Game, picked: &Pictures, progress: &dyn Progress) -> Result<()> {
        let fonts = Fonts::default();

        install::run(
            Install::over(game.root(), game.staged.path(), game.store())
                .sending(&fonts)
                .drawing(picked)
                .heard_by(progress),
        )
        .await
    }

    fn archived(game: &Game) -> Vec<u8> {
        game.bytes(ARCHIVE)
    }

    fn body_of(raw: &[u8], name: &str) -> Vec<u8> {
        let entries = archive::entries(raw).expect("it still reads as an archive");
        let found = entries
            .iter()
            .find(|one| one.name == name)
            .expect("the entry is still listed");

        archive::body(raw, found)
    }

    #[tokio::test]
    async fn every_picture_packed_into_the_archive_is_listed_and_handed_over_drawn() {
        let game = a_game();
        let face = dotted(48, 48, 3).png().expect("a png");
        pack(
            &game,
            &[
                ("Data\\Map001.rvdata2", &[4, 8, b'0']),
                ("Graphics\\Faces\\Actor1.png", &face),
                ("Graphics\\System\\Window.bmp", &a_bmp(128, 128)),
                ("Audio\\BGM\\theme.ogg", b"never ours"),
            ],
        );

        let held = opened(&game).await;
        let found = listed(&held, game.root());
        let keys: Vec<&str> = found.shots.iter().map(|one| one.key.as_str()).collect();

        assert_eq!(
            keys,
            [
                "Game.rgss3a|Graphics/Faces/Actor1.png",
                "Game.rgss3a|Graphics/System/Window.bmp"
            ],
            "a key names the archive and the entry inside it, so a game shipping the same picture \
             loose and packed cannot hand one reader's pick to the other"
        );

        let one = &found.shots[0];
        assert_eq!((one.wide, one.high), (48, 48));
        assert_eq!(one.format, "PNG");
        assert_eq!(one.holder, "Game.rgss3a/Graphics/Faces");
        assert_eq!(one.name, "Actor1.png");
        assert!(one.drawable && one.locked.is_none());

        assert_eq!(
            picture(game.root(), game.store(), &one.key).expect("the picture opens"),
            face,
            "the preview a reader sees has to be the picture that is packed in the game"
        );
    }

    #[tokio::test]
    async fn a_picture_the_game_ships_in_another_format_goes_back_in_that_format() {
        let game = a_game();
        pack(
            &game,
            &[
                ("Data\\Map001.rvdata2", &[4, 8, b'0']),
                ("Graphics\\System\\Window.bmp", &a_bmp(128, 128)),
            ],
        );

        let held = opened(&game).await;
        let [one] = &listed(&held, game.root()).shots[..] else {
            panic!("the one picture is listed")
        };

        assert_eq!(one.format, "BMP");
        assert!(
            one.drawable && one.locked.is_none(),
            "this build reads a bmp and writes one, so there is nothing to hold the reader back: \
             {:?}",
            one.locked
        );

        let fresh = dotted(128, 128, 9);
        install(&game, &game.pick(&one.key, &fresh), &Quiet)
            .await
            .expect("the picture goes in");

        let back = body_of(&archived(&game), "Graphics/System/Window.bmp");
        assert_eq!(
            canvas::kind_of(&back),
            Some("bmp"),
            "RGSS picks the reader for a picture by the name it is filed under, so a png written \
             behind a .bmp name would leave the game unable to load it"
        );
        assert_eq!(
            Canvas::read(&back).expect("it reads").pixels,
            fresh.pixels,
            "and it still has to be the picture the reader chose, alpha and all: a bmp holds a \
             per-pixel alpha channel, so writing one back flattened would paint a solid box over \
             everything the picture was drawn to show through"
        );
    }

    #[tokio::test]
    async fn letting_a_loose_picture_go_leaves_the_archive_the_text_went_into_alone() {
        let game = a_game();
        let face = dotted(48, 48, 3).png().expect("a png");
        pack(
            &game,
            &[
                ("Data\\Map001.rvdata2", &[4, 8, b'0']),
                ("Graphics\\Faces\\Actor1.png", &face),
            ],
        );

        let loose = dotted(32, 32, 7).png().expect("a png");
        game.put("Graphics/Pictures/title.png", &loose);

        let shots = listed(&opened(&game).await, game.root()).shots;
        let one = shots
            .iter()
            .find(|one| one.at == "Graphics/Pictures/title.png")
            .expect("the loose picture");

        let fresh = dotted(32, 32, 200);
        install(&game, &game.pick(&one.key, &fresh), &Quiet)
            .await
            .expect("the pick goes in");

        let patched = archived(&game);
        assert_eq!(
            fs::read(game.root().join("Graphics/Pictures/title.png")).expect("the picture"),
            pictures::written(&loose, &fresh.png().expect("a png")).expect("what goes in"),
            "the loose file the game reads first is the one a pick has to land on"
        );

        install(&game, &Pictures::default(), &Quiet)
            .await
            .expect("a second install with nothing picked");

        assert_eq!(
            fs::read(game.root().join("Graphics/Pictures/title.png")).expect("the picture"),
            loose,
            "letting the pick go has to hand the game back the picture it shipped"
        );
        assert_eq!(
            archived(&game),
            patched,
            "and it may never reach into the archive the translated text went into, or clearing \
             one picture would throw away every line the reader had written"
        );
    }

    #[tokio::test]
    async fn a_picked_picture_goes_into_the_archive_and_no_other_entry_moves() {
        let game = a_game();
        let face = dotted(48, 48, 3).png().expect("a png");
        pack(
            &game,
            &[
                ("Data\\Map001.rvdata2", &[4, 8, b'0']),
                ("Graphics\\Faces\\Actor1.png", &face),
            ],
        );

        let fresh = dotted(48, 48, 88);
        let key = key_of(ARCHIVE, Some(FACE));
        install(&game, &game.pick(&key, &fresh), &Quiet)
            .await
            .expect("the picture goes in");

        let raw = archived(&game);
        assert_eq!(
            body_of(&raw, "Data/Map001.rvdata2"),
            [4, 8, b'0'],
            "an entry nobody picked may not change"
        );
        assert_eq!(
            Canvas::read(&body_of(&raw, FACE))
                .expect("the packed picture still reads")
                .pixels,
            fresh.pixels,
            "what the reader picked is what the game now draws with"
        );

        install(&game, &Pictures::default(), &Quiet)
            .await
            .expect("a second install with nothing picked");

        assert_eq!(
            body_of(&archived(&game), FACE),
            face,
            "the archive is always rebuilt from the original, so dropping the pick hands the game \
             back the picture it shipped"
        );
    }

    #[tokio::test]
    async fn a_picture_of_another_size_is_fitted_to_the_spot_the_game_draws_it_in() {
        let game = a_game();
        pack(
            &game,
            &[
                ("Data\\Map001.rvdata2", &[4, 8, b'0']),
                (
                    "Graphics\\Faces\\Actor1.png",
                    &dotted(96, 96, 3).png().expect("a png"),
                ),
            ],
        );

        let key = key_of(ARCHIVE, Some(FACE));
        install(&game, &game.pick(&key, &dotted(300, 120, 8)), &Quiet)
            .await
            .expect("the picture goes in");

        let held = Canvas::read(&body_of(&archived(&game), FACE)).expect("it reads");
        assert_eq!(
            (held.wide, held.high),
            (96, 96),
            "a face sheet is cut into eight fixed squares by pixel count, so a replacement of \
             another size would leave the game drawing a face out of two"
        );
    }

    #[tokio::test]
    async fn a_game_unpacked_for_a_port_lists_and_replaces_its_pictures_the_same_way() {
        let game = a_game();
        let face = dotted(48, 48, 3).png().expect("a png");
        game.put("Data/Map001.rvdata2", &[4, 8, b'0']);
        game.put(FACE, &face);

        let held = opened(&game).await;
        let [one] = &listed(&held, game.root()).shots[..] else {
            panic!("the one picture is listed")
        };

        assert_eq!(
            one.key, FACE,
            "with no archive in sight a picture is named by where it sits in the game folder"
        );
        assert_eq!(one.holder, "Graphics/Faces");
        assert_eq!(
            picture(game.root(), game.store(), &one.key).expect("it opens"),
            face
        );

        let fresh = dotted(48, 48, 42);
        install(&game, &game.pick(FACE, &fresh), &Quiet)
            .await
            .expect("the picture goes in");

        assert_eq!(
            Canvas::read(&fs::read(game.root().join(FACE)).expect("the file"))
                .expect("it reads")
                .pixels,
            fresh.pixels
        );

        install(&game, &Pictures::default(), &Quiet)
            .await
            .expect("a second install with nothing picked");

        assert_eq!(
            fs::read(game.root().join(FACE)).expect("the file"),
            face,
            "dropping the pick has to give the game back the file it shipped, byte for byte"
        );
    }

    #[tokio::test]
    async fn a_pick_naming_a_picture_this_game_no_longer_holds_is_said_out_loud() {
        let game = a_game();
        pack(&game, &[("Data\\Map001.rvdata2", &[4, 8, b'0'])]);

        let heard = Heard::default();
        let key = key_of(ARCHIVE, Some(FACE));
        install(&game, &game.pick(&key, &dotted(8, 8, 1)), &heard)
            .await
            .expect("an install that goes through");

        assert_eq!(
            heard.warnings(),
            [format!("{key} is not a picture {ARCHIVE} holds any more")],
            "the reader picked a file and nothing came of it, so saying nothing would leave them \
             waiting on a change that is never coming"
        );
    }

    #[test]
    fn a_key_that_could_climb_out_of_the_game_folder_never_reaches_the_filesystem() {
        let game = a_game();

        for asked in [
            "../../etc/passwd",
            "..",
            "Graphics/../../outside.png",
            "../Game.rgss3a|Graphics/Faces/Actor1.png",
        ] {
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
    async fn a_picture_the_game_also_ships_loose_is_the_packed_one_the_reader_gets_to_replace() {
        let game = a_game();
        let packed = dotted(96, 96, 1).png().expect("a png");
        let beside = dotted(96, 96, 2).png().expect("another png");
        pack(
            &game,
            &[
                ("Data\\Map001.rvdata2", &[4, 8, b'0']),
                ("Graphics\\System\\Window.png", &packed),
            ],
        );
        game.put("Graphics/System/Window.png", &beside);

        let held = opened(&game).await;
        let found = listed(&held, game.root());
        let shots = &found.shots;
        let keys: Vec<&str> = shots.iter().map(|one| one.key.as_str()).collect();

        assert_eq!(
            keys,
            ["Game.rgss3a|Graphics/System/Window.png"],
            "mkxp and mkxp-z both mount the archive before the game folder, and mkxp-z had to add \
             a patches mount above both to let anyone override a packed file: the archive wins, so \
             the loose copy is the one the game never draws and listing it would offer a row that \
             changes nothing"
        );
        assert_eq!(
            found.behind, 1,
            "the reader is told how many loose copies were left out, or leaving them out is \
             hiding something"
        );

        let packed_row = &shots[0];
        assert!(packed_row.locked.is_none() && packed_row.drawable);

        assert_eq!(
            picture(game.root(), game.store(), &packed_row.key).expect("the packed one opens"),
            packed
        );
        assert_eq!(
            picture(game.root(), game.store(), "Graphics/System/Window.png")
                .expect("the loose one still opens"),
            beside,
            "leaving the row out may not put the bytes out of reach, so a pick made before the \
             archive was read still reads what it named"
        );

        let heard = Heard::default();
        install(
            &game,
            &game.pick("Graphics/System/Window.png", &dotted(96, 96, 7)),
            &heard,
        )
        .await
        .expect("an install that goes through");

        assert_eq!(
            fs::read(game.root().join("Graphics/System/Window.png")).expect("the loose file"),
            beside,
            "a pick against the shadowed loose copy may not rewrite a file the game never opens"
        );
        assert!(heard.warnings().iter().any(|said| said.contains(SHADOWED)));

        let before = archived(&game);
        let fresh = dotted(96, 96, 9);
        install(&game, &game.pick(&packed_row.key, &fresh), &Quiet)
            .await
            .expect("the packed picture goes in");

        assert_ne!(
            archived(&game),
            before,
            "the archive is the copy the game opens, so a pick has to land in there"
        );
        assert_eq!(
            fs::read(game.root().join("Graphics/System/Window.png")).expect("the loose file"),
            beside,
            "and writing the packed copy is no reason to touch the loose file"
        );

        install(&game, &Pictures::default(), &Quiet)
            .await
            .expect("a second install with nothing picked");

        assert_eq!(
            archived(&game),
            before,
            "dropping the pick has to give the game back the archive it shipped"
        );
    }
}
