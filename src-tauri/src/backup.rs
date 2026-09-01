use crate::hash::{Rolling, xxh3};
use crate::store::Stamp;
use crate::{store, walk};
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use tokio::fs;

const HELD: &str = "backup";
const SHIPPED: &str = "shipped";

const MARKS: &str = "marks";
const PARTS: &str = "parts";
const PART: &str = "stagedpart";

const REMEMBERED: usize = 4096;

static SAID: LazyLock<Mutex<HashMap<PathBuf, (Stamp, String)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn recalled(file: &Path, stamp: Stamp) -> Option<String> {
    stamp.1?;

    let said = SAID.lock().expect("the hashes already worked out");

    said.get(file)
        .filter(|(held, _)| *held == stamp)
        .map(|(_, said)| said.clone())
}

fn recall(file: &Path, stamp: Stamp, said: &str) {
    if stamp.1.is_none() {
        return;
    }

    let mut held = SAID.lock().expect("the hashes already worked out");

    if held.len() >= REMEMBERED {
        held.clear();
    }

    held.insert(file.to_path_buf(), (stamp, said.to_string()));
}

fn with_suffix(at: &Path, suffix: &str) -> PathBuf {
    let mut name = at.as_os_str().to_owned();
    name.push(".");
    name.push(suffix);

    PathBuf::from(name)
}

fn under(root: &Path, tree: &str, game_dir: &Path, file: &Path) -> Result<PathBuf> {
    let relative = file
        .strip_prefix(game_dir)
        .with_context(|| format!("{} is outside {}", file.display(), game_dir.display()))?;

    Ok(root.join(tree).join(relative))
}

fn mark_of(root: &Path, game_dir: &Path, file: &Path) -> Result<PathBuf> {
    under(root, MARKS, game_dir, file)
}

fn shipped(root: &Path, game_dir: &Path, at: &Path) -> Option<PathBuf> {
    under(root, SHIPPED, game_dir, at).ok()
}

pub fn is_part(at: &Path) -> bool {
    at.extension().is_some_and(|kind| kind == PART)
}

pub struct Pending {
    part: PathBuf,
    at: PathBuf,
    said: String,
}

pub async fn stage(file: &Path, body: Vec<u8>) -> Result<Pending> {
    let part = with_suffix(file, PART);

    fs::write(&part, &body)
        .await
        .with_context(|| format!("staging {}", part.display()))?;

    Ok(Pending {
        part,
        at: file.to_path_buf(),
        said: tokio::task::spawn_blocking(move || xxh3(body)).await?,
    })
}

impl Pending {
    pub fn at(&self) -> &Path {
        &self.at
    }

    pub async fn land(self, root: &Path, game_dir: &Path) -> Result<()> {
        let Self { part, at, said } = self;

        guarded(root, game_dir, &at, said, async || {
            fs::rename(&part, &at)
                .await
                .with_context(|| format!("moving {} into place", at.display()))
        })
        .await
    }

    pub async fn let_go(self) {
        let _ = fs::remove_file(&self.part).await;
    }
}

pub async fn land_all(root: &Path, game_dir: &Path, pending: Vec<Pending>) -> Result<Vec<PathBuf>> {
    let mut waiting = pending.into_iter();
    let mut done = Vec::new();

    while let Some(one) = waiting.next() {
        let at = one.at().to_path_buf();

        if let Err(why) = one.land(root, game_dir).await {
            for rest in waiting {
                rest.let_go().await;
            }

            return Err(why);
        }

        done.push(at);
    }

    Ok(done)
}

pub async fn let_all_go(pending: Vec<Pending>) {
    for one in pending {
        one.let_go().await;
    }
}

pub fn slot(root: &Path, game_dir: &Path, file: &Path) -> Result<PathBuf> {
    under(root, HELD, game_dir, file)
}

#[tracing::instrument(level = "debug", name = "backup.replace", skip_all)]
pub async fn replace(root: &Path, game_dir: &Path, file: &Path, body: Vec<u8>) -> Result<()> {
    let held = tracing::debug_span!("backup.mark");
    let (said, body) =
        tokio::task::spawn_blocking(move || held.in_scope(|| (xxh3(&body), body))).await?;

    guarded(root, game_dir, file, said, async || {
        store::write_atomically(file, body).await
    })
    .await
}

async fn guarded(
    root: &Path,
    game_dir: &Path,
    file: &Path,
    said: String,
    install: impl AsyncFnOnce() -> Result<()>,
) -> Result<()> {
    keep(root, game_dir, file).await?;
    widen(root, game_dir, file, &said).await?;

    install().await?;

    write_mark(root, game_dir, file, said).await
}

fn as_marks(said: &str) -> Vec<String> {
    said.lines().map(str::to_string).collect()
}

fn among(every: &[String], said: &str) -> bool {
    every.iter().any(|one| one == said)
}

fn marks_now(root: &Path, game_dir: &Path, file: &Path) -> Result<Vec<String>> {
    let mark = mark_of(root, game_dir, file)?;

    match std::fs::read_to_string(&mark) {
        Ok(said) => Ok(as_marks(&said)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error).with_context(|| format!("reading {}", mark.display())),
    }
}

async fn blocking<T: Send + 'static>(
    root: &Path,
    game_dir: &Path,
    file: &Path,
    work: impl FnOnce(&Path, &Path, &Path) -> Result<T> + Send + 'static,
) -> Result<T> {
    let (root, game_dir, file) = (
        root.to_path_buf(),
        game_dir.to_path_buf(),
        file.to_path_buf(),
    );

    tokio::task::spawn_blocking(move || work(&root, &game_dir, &file)).await?
}

async fn marks(root: &Path, game_dir: &Path, file: &Path) -> Result<Vec<String>> {
    blocking(root, game_dir, file, marks_now).await
}

async fn widen(root: &Path, game_dir: &Path, file: &Path, said: &str) -> Result<()> {
    let mut every = marks(root, game_dir, file).await?;
    every.retain(|one| one != said);
    every.push(said.to_string());

    write_mark(root, game_dir, file, every.join("\n")).await
}

async fn write_mark(root: &Path, game_dir: &Path, file: &Path, said: String) -> Result<()> {
    let mark = mark_of(root, game_dir, file)?;
    if let Some(parent) = mark.parent() {
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("making {}", parent.display()))?;
    }

    store::write_atomically(&mark, said)
        .await
        .with_context(|| format!("marking {}", mark.display()))
}

fn ours_now(root: &Path, game_dir: &Path, file: &Path) -> Result<bool> {
    let every = marks_now(root, game_dir, file)?;
    if every.is_empty() {
        return Ok(false);
    }

    Ok(among(&every, &hashed_now(file)?))
}

async fn ours(root: &Path, game_dir: &Path, file: &Path) -> Result<bool> {
    blocking(root, game_dir, file, ours_now).await
}

pub async fn original_at(root: &Path, game_dir: &Path, file: &Path) -> Result<PathBuf> {
    blocking(root, game_dir, file, original_at_now).await
}

pub async fn original(root: &Path, game_dir: &Path, file: &Path) -> Result<Vec<u8>> {
    let reading = original_at(root, game_dir, file).await?;

    fs::read(&reading)
        .await
        .with_context(|| format!("reading {}", reading.display()))
}

pub fn original_at_now(root: &Path, game_dir: &Path, file: &Path) -> Result<PathBuf> {
    let slot = slot(root, game_dir, file)?;

    Ok(match slot.is_file() && ours_now(root, game_dir, file)? {
        true => slot,
        false => file.to_path_buf(),
    })
}

pub fn original_now(root: &Path, game_dir: &Path, file: &Path) -> Option<Vec<u8>> {
    std::fs::read(original_at_now(root, game_dir, file).ok()?).ok()
}

#[tracing::instrument(level = "debug", name = "backup.hash", skip_all)]
fn hashed_now(file: &Path) -> Result<String> {
    let before = store::stamp_of(file);
    if let Some(said) = recalled(file, before) {
        return Ok(said);
    }

    let mut reading =
        std::fs::File::open(file).with_context(|| format!("opening {}", file.display()))?;

    let mut hash = Rolling::default();
    let mut chunk = vec![0u8; 1 << 20];

    let said = loop {
        let got = reading
            .read(&mut chunk)
            .with_context(|| format!("reading {}", file.display()))?;
        if got == 0 {
            break hash.done();
        }

        hash.push(&chunk[..got]);
    };

    if store::stamp_of(file) == before {
        recall(file, before, &said);
    }

    Ok(said)
}

async fn drop_kept(root: &Path, game_dir: &Path, file: &Path) -> Result<()> {
    let slot = slot(root, game_dir, file)?;
    fs::remove_file(&slot)
        .await
        .with_context(|| format!("letting go of {}", slot.display()))?;
    let _ = fs::remove_file(mark_of(root, game_dir, file)?).await;

    Ok(())
}

pub async fn everything_kept(root: &Path, game_dir: &Path) -> Result<Vec<PathBuf>> {
    let held = root.join(HELD);

    Ok(walk::accounted(&held)
        .await?
        .into_iter()
        .filter_map(|at| Some(game_dir.join(at.strip_prefix(&held).ok()?)))
        .collect())
}

pub async fn put_back_the_rest(
    store: &Path,
    game_dir: &Path,
    mine: impl Fn(&Path) -> bool,
    wanted: &[PathBuf],
) -> Result<usize> {
    let mut given = 0;

    for one in everything_kept(store, game_dir).await? {
        if wanted.contains(&one) || !mine(&one) {
            continue;
        }

        if put_back(store, game_dir, &one).await? {
            given += 1;
        }
    }

    Ok(given)
}

async fn is_file(at: &Path) -> bool {
    fs::metadata(at)
        .await
        .map(|found| found.is_file())
        .unwrap_or(false)
}

async fn is_dir(at: &Path) -> bool {
    fs::metadata(at)
        .await
        .map(|found| found.is_dir())
        .unwrap_or(false)
}

#[tracing::instrument(level = "debug", name = "backup.keep", skip_all)]
pub async fn keep(root: &Path, game_dir: &Path, file: &Path) -> Result<()> {
    if !is_file(file).await {
        return Ok(());
    }

    let slot = slot(root, game_dir, file)?;
    if is_file(&slot).await {
        if ours(root, game_dir, file).await? {
            return Ok(());
        }
    } else if ours(root, game_dir, file).await? {
        anyhow::bail!(
            "the kept original of {} is gone from the backup",
            file.display()
        );
    }

    let staging = under(&store::tmp_dir(root), PARTS, game_dir, file)?;
    if let Some(parent) = staging.parent() {
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("making {}", parent.display()))?;
    }

    land_via(file, &staging, &slot, "backing up").await
}

async fn land_via(from: &Path, staging: &Path, to: &Path, doing: &str) -> Result<()> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("making {}", parent.display()))?;
    }

    if let Err(error) = fs::copy(from, staging).await {
        let _ = fs::remove_file(staging).await;
        return Err(error).with_context(|| format!("{doing} {}", from.display()));
    }

    if let Err(error) = fs::rename(staging, to).await {
        let _ = fs::remove_file(staging).await;
        return Err(error).with_context(|| format!("{doing} {}", to.display()));
    }

    Ok(())
}

pub async fn taken_over(root: &Path, game_dir: &Path, at: &Path) -> Result<bool> {
    let Some(kept) = shipped(root, game_dir, at) else {
        return Ok(false);
    };

    if is_dir(&kept).await || !is_dir(at).await {
        return Ok(false);
    }

    Ok(walk::copy(at, &kept, |_| true).await? > 0)
}

pub async fn handed_back(root: &Path, game_dir: &Path, at: &Path) -> Result<bool> {
    let Some(kept) = shipped(root, game_dir, at) else {
        return Ok(false);
    };

    if !is_dir(&kept).await {
        return Ok(false);
    }

    walk::cleared(at).await?;
    walk::copy(&kept, at, |_| true).await?;
    walk::cleared(&kept).await?;

    Ok(true)
}

pub async fn put_back(root: &Path, game_dir: &Path, file: &Path) -> Result<bool> {
    let slot = slot(root, game_dir, file)?;
    if !is_file(&slot).await {
        return Ok(false);
    }

    if is_file(file).await && !ours(root, game_dir, file).await? {
        drop_kept(root, game_dir, file).await?;
        return Ok(false);
    }

    land_via(&slot, &with_suffix(file, PART), file, "putting back").await?;

    drop_kept(root, game_dir, file).await?;

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).await.unwrap();
        fs::write(path, text).await.unwrap();
    }

    struct Sandbox {
        _held: tempfile::TempDir,
        game: PathBuf,
        root: PathBuf,
        file: PathBuf,
    }

    fn sandbox() -> Sandbox {
        let held = tempfile::tempdir().expect("a temp folder");
        let game = held.path().join("game");
        let root = held.path().join("store");
        let file = game.join("data").join("Map001.json");

        Sandbox {
            _held: held,
            game,
            root,
            file,
        }
    }

    #[tokio::test]
    async fn a_landing_that_is_not_in_the_game_holds_nothing_the_game_shipped() {
        let sandbox = tempfile::tempdir().expect("a temp folder");
        let game = sandbox.path().join("game");
        let root = sandbox.path().join("store");
        let staged = root.join("staged").join("japanese");

        write(&staged.join("Map001.json.sheet"), "what the reader wrote").await;

        assert!(
            !taken_over(&root, &game, &staged).await.unwrap(),
            "an engine that stages its work in the store hands that folder in here, and a folder \
             outside the game can never be one the game shipped"
        );
        assert!(
            !handed_back(&root, &game, &staged).await.unwrap(),
            "there is no folder out here for the game to have shipped, so asking has to answer \
             that rather than fail and take the whole of Restore original files down with it"
        );
    }

    #[tokio::test]
    async fn a_folder_the_game_shipped_is_handed_back_whole() {
        let sandbox = tempfile::tempdir().expect("a temp folder");
        let game = sandbox.path().join("game");
        let root = sandbox.path().join("store");
        let folder = game.join("game").join("tl").join("japanese");

        write(&folder.join("script.rpy"), "what the game shipped").await;

        assert!(taken_over(&root, &game, &folder).await.unwrap());
        assert!(
            !taken_over(&root, &game, &folder).await.unwrap(),
            "keeping it a second time would keep the reader's own work in place of the game's, \
             which is the one copy nobody else has"
        );

        walk::cleared(&folder).await.unwrap();
        write(&folder.join("ours.rpy"), "what the reader wrote").await;

        assert!(handed_back(&root, &game, &folder).await.unwrap());
        assert_eq!(
            fs::read_to_string(folder.join("script.rpy")).await.unwrap(),
            "what the game shipped"
        );
        assert!(
            !folder.join("ours.rpy").exists(),
            "handing the game its own folder back means the folder it shipped, not that folder \
             with the reader's files still sitting in it"
        );
        assert!(!handed_back(&root, &game, &folder).await.unwrap());
    }

    #[tokio::test]
    async fn a_file_that_changed_under_us_is_never_answered_with_the_hash_it_had_before() {
        let sandbox = tempfile::tempdir().expect("a temp folder");
        let at = sandbox.path().join("archive.wolf");

        write(&at, "the bytes the game shipped with").await;
        let first = hashed_now(&at).expect("a hash");

        write(&at, "the bytes somebody else wrote!!").await;

        assert_ne!(
            hashed_now(&at).expect("a hash"),
            first,
            "a file that changed under us has to be read again, or revert would refuse to put \
             back a game it no longer recognises"
        );
    }

    #[tokio::test]
    async fn a_staged_write_stays_out_of_the_game_until_it_lands() {
        let Sandbox {
            _held,
            game,
            root,
            file,
        } = sandbox();

        write(&file, "english").await;
        let waiting = stage(&file, b"translated".to_vec()).await.unwrap();

        assert_eq!(
            fs::read_to_string(&file).await.unwrap(),
            "english",
            "staging may not touch the game: a later failure has to find it whole"
        );
        assert!(
            !is_file(&slot(&root, &game, &file).unwrap()).await,
            "and nothing is kept for a write that has not happened"
        );

        waiting.land(&root, &game).await.unwrap();
        assert_eq!(fs::read_to_string(&file).await.unwrap(), "translated");
        assert!(put_back(&root, &game, &file).await.unwrap());
        assert_eq!(
            fs::read_to_string(&file).await.unwrap(),
            "english",
            "landing has to keep the original first, or Restore has nothing to give back"
        );
    }

    #[tokio::test]
    async fn a_staged_write_let_go_of_leaves_the_game_folder_clean() {
        let sandbox = tempfile::tempdir().expect("a temp folder");
        let game = sandbox.path().join("game");
        let file = game.join("data").join("Map001.json");

        write(&file, "english").await;
        let waiting = stage(&file, b"translated".to_vec()).await.unwrap();
        let part = waiting.part.clone();

        waiting.let_go().await;

        assert!(!is_file(&part).await, "a part nobody wants may not be left");
        assert_eq!(fs::read_to_string(&file).await.unwrap(), "english");
    }

    #[tokio::test]
    async fn a_write_that_failed_after_the_mark_does_not_cost_the_original() {
        let Sandbox {
            _held,
            game,
            root,
            file,
        } = sandbox();

        write(&file, "english").await;
        replace(&root, &game, &file, b"first pass".to_vec())
            .await
            .unwrap();

        keep(&root, &game, &file).await.unwrap();
        widen(&root, &game, &file, &xxh3(b"second pass"))
            .await
            .unwrap();

        assert!(
            ours(&root, &game, &file).await.unwrap(),
            "the game still holds the first pass, which is ours: a write that never landed \
             may not make the file look like a stranger's"
        );

        keep(&root, &game, &file).await.unwrap();
        assert!(
            put_back(&root, &game, &file).await.unwrap(),
            "and the original is still there to give back"
        );
        assert_eq!(fs::read_to_string(&file).await.unwrap(), "english");
    }

    #[tokio::test]
    async fn a_backup_the_game_has_moved_past_is_let_go_of_untouched() {
        let Sandbox {
            _held,
            game,
            root,
            file,
        } = sandbox();

        write(&file, "english").await;
        replace(&root, &game, &file, b"translated".to_vec())
            .await
            .unwrap();

        write(&file, "english of the next build").await;

        assert!(
            !put_back(&root, &game, &file).await.unwrap(),
            "the original we kept belongs to a build nobody has any more"
        );
        assert_eq!(
            fs::read_to_string(&file).await.unwrap(),
            "english of the next build",
            "and writing it back would undo the update"
        );
        assert!(
            everything_kept(&root, &game).await.unwrap().is_empty(),
            "a backup that can never be used again may not be offered"
        );
    }

    #[tokio::test]
    async fn a_write_that_never_finished_leaves_the_original_reachable() {
        let Sandbox {
            _held,
            game,
            root,
            file,
        } = sandbox();

        write(&file, "english").await;
        keep(&root, &game, &file).await.unwrap();
        widen(&root, &game, &file, &xxh3(b"translated"))
            .await
            .unwrap();

        assert!(
            !ours(&root, &game, &file).await.unwrap(),
            "the mark is ahead of the file, so nothing of ours is in the game yet"
        );

        keep(&root, &game, &file).await.unwrap();

        let slot = slot(&root, &game, &file).expect("a slot");
        assert_eq!(
            fs::read_to_string(&slot).await.unwrap(),
            "english",
            "the kept original must never be replaced by our own text"
        );
        assert_eq!(
            fs::read_to_string(&file).await.unwrap(),
            "english",
            "and the game was never written to"
        );
    }

    #[tokio::test]
    async fn a_new_build_of_a_file_becomes_the_original_we_keep() {
        let Sandbox {
            _held,
            game,
            root,
            file,
        } = sandbox();

        write(&file, "english").await;
        replace(&root, &game, &file, b"translated".to_vec())
            .await
            .unwrap();

        write(&file, "english of the next build").await;
        replace(&root, &game, &file, b"translated again".to_vec())
            .await
            .unwrap();

        put_back(&root, &game, &file).await.unwrap();
        assert_eq!(
            fs::read_to_string(&file).await.unwrap(),
            "english of the next build",
            "the kept original has to follow the build the game is on"
        );
    }

    #[tokio::test]
    async fn every_file_ever_kept_can_be_named_and_each_one_put_back() {
        let sandbox = tempfile::tempdir().expect("a temp folder");
        let game = sandbox.path().join("game");
        let root = sandbox.path().join("store");

        let one = game.join("Game_Data").join("resources.assets");
        let two = game.join("Bundles").join("extra.bundle");

        write(&one, "english one").await;
        write(&two, "english two").await;
        replace(&root, &game, &one, b"translated".to_vec())
            .await
            .unwrap();
        replace(&root, &game, &two, b"translated".to_vec())
            .await
            .unwrap();

        let mut named = everything_kept(&root, &game).await.unwrap();
        named.sort();
        assert_eq!(named, vec![two.clone(), one.clone()]);

        for at in [&one, &two] {
            assert!(put_back(&root, &game, at).await.unwrap());
        }
        assert_eq!(fs::read_to_string(&one).await.unwrap(), "english one");
        assert_eq!(fs::read_to_string(&two).await.unwrap(), "english two");

        assert!(
            everything_kept(&root, &game).await.unwrap().is_empty(),
            "once the original is back where it belongs there is nothing left to put back"
        );
    }

    #[tokio::test]
    async fn a_backup_that_cannot_be_checked_is_never_destroyed() {
        let Sandbox {
            _held,
            game,
            root,
            file,
        } = sandbox();

        write(&file, "english").await;
        replace(&root, &game, &file, b"translated".to_vec())
            .await
            .unwrap();

        let slot = slot(&root, &game, &file).unwrap();
        let mark = mark_of(&root, &game, &file).unwrap();
        fs::remove_file(&mark).await.unwrap();
        fs::create_dir(&mark).await.unwrap();

        assert!(
            put_back(&root, &game, &file).await.is_err(),
            "an unreadable mark is not an answer: guessing either way could cost the original"
        );
        assert!(
            is_file(&slot).await,
            "the only copy of the original has to survive the failure"
        );

        assert!(keep(&root, &game, &file).await.is_err());
        assert_eq!(
            fs::read_to_string(&slot).await.unwrap(),
            "english",
            "and keep must not overwrite it with our own bytes"
        );
    }

    #[tokio::test]
    async fn a_backup_deleted_by_hand_is_not_replaced_with_our_own_text() {
        let Sandbox {
            _held,
            game,
            root,
            file,
        } = sandbox();

        write(&file, "english").await;
        replace(&root, &game, &file, b"translated".to_vec())
            .await
            .unwrap();

        let slot = slot(&root, &game, &file).unwrap();
        fs::remove_file(&slot).await.unwrap();

        assert!(
            keep(&root, &game, &file).await.is_err(),
            "the game holds our text and the original is gone: adopting it would make \
             Restore hand back the translation"
        );
        assert!(
            !is_file(&slot).await,
            "and nothing of ours may take the original's place"
        );
    }

    #[tokio::test]
    async fn nothing_kept_means_nothing_to_put_back() {
        let sandbox = tempfile::tempdir().expect("a temp folder");

        assert!(
            everything_kept(sandbox.path(), sandbox.path())
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            !put_back(
                sandbox.path(),
                sandbox.path(),
                &sandbox.path().join("nothing")
            )
            .await
            .unwrap()
        );
    }

    #[tokio::test]
    async fn a_second_export_does_not_overwrite_the_english_copy() {
        let sandbox = tempfile::tempdir().expect("a temp folder");
        let game = sandbox.path().join("game");
        let root = sandbox.path().join("store");
        let file = game.join("data").join("Items.json");

        write(&file, "english").await;
        replace(&root, &game, &file, b"first pass".to_vec())
            .await
            .unwrap();
        replace(&root, &game, &file, b"second pass".to_vec())
            .await
            .unwrap();

        put_back(&root, &game, &file).await.unwrap();
        assert_eq!(
            fs::read_to_string(&file).await.unwrap(),
            "english",
            "the backup has to stay the original, not the last export"
        );
    }
}
