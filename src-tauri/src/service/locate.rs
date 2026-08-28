use crate::engine::{self, Engine, Landing, Undo};
use crate::project::{self, Project};
use crate::scope::{Scope, key, slashed};
use crate::store::{self, Store};
use crate::{backup, walk};
use anyhow::Result;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub struct Game {
    pub at: PathBuf,
    pub project: Project,
    pub source: PathBuf,
    pub store: Store,
    pub engine: Arc<dyn Engine>,
}

pub async fn open_game(game_dir: &Path) -> Result<Game> {
    let engine: Arc<dyn Engine> = engine_at(game_dir).await?.into();
    let project = project::require(game_dir).await?.under(engine.as_ref());
    let source = source_of(game_dir)?;
    let store = Store::open(game_dir, &project.folder()).await?;

    Ok(Game {
        at: game_dir.to_path_buf(),
        project,
        source,
        store,
        engine,
    })
}

fn engine_for(game_dir: &Path) -> Result<Box<dyn Engine>> {
    if !game_dir.is_dir() {
        anyhow::bail!("That folder is not there any more.");
    }

    if let Some(held) = engine::detect(game_dir) {
        return Ok(held);
    }

    match engine::refused(game_dir) {
        Some(why) => anyhow::bail!(why),
        None => anyhow::bail!("Not a Ren'Py, RPG Maker, Wolf RPG or Unity game."),
    }
}

pub async fn engine_at(game_dir: &Path) -> Result<Box<dyn Engine>> {
    let here = game_dir.to_path_buf();

    tokio::task::spawn_blocking(move || engine_for(&here)).await?
}

pub fn source_of(game_dir: &Path) -> Result<PathBuf> {
    Ok(store::source_dir(&store::root_for(game_dir)?))
}

pub async fn landed(
    game_dir: &Path,
    engine: &dyn Engine,
    store: &Path,
    language: &str,
) -> Result<HashSet<String>> {
    let target = engine.output(Landing {
        game_dir,
        store,
        language,
    });

    if engine.undo() == Undo::Restore {
        return Ok(backup::everything_kept(store, game_dir)
            .await?
            .iter()
            .filter_map(|at| at.strip_prefix(&target).ok().map(slashed))
            .collect());
    }

    Ok(walk::relative(&target)
        .await
        .iter()
        .map(|at| slashed(at))
        .collect())
}

pub async fn landed_in(game: &Game) -> Result<HashSet<String>> {
    landed(
        &game.at,
        game.engine.as_ref(),
        game.store.root(),
        &game.project.folder(),
    )
    .await
}

pub async fn files_under(source: &Path, engine: &dyn Engine, reach: &[Scope]) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = walk::relative(source)
        .await
        .into_iter()
        .filter(|at| {
            let key = slashed(at);
            reach.iter().any(|one| one.holds(&key))
        })
        .map(|at| source.join(at))
        .filter(|at| engine.wants(at))
        .collect();

    found.sort();
    found
}

pub fn keys_of(source: &Path, found: &[PathBuf]) -> Vec<String> {
    found.iter().map(|at| key(source, at)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::renpy::RenPy;
    use std::fs;

    async fn sandbox() -> (tempfile::TempDir, PathBuf) {
        let sandbox = tempfile::tempdir().expect("a temp folder");
        let source = sandbox.path().join("source");

        for at in [
            "game/script.rpy",
            "game/deep/screens.rpy",
            "game/notes.txt",
            "other/script.rpy",
        ] {
            let landing = source.join(at);
            tokio::fs::create_dir_all(landing.parent().unwrap())
                .await
                .unwrap();
            tokio::fs::write(&landing, "").await.unwrap();
        }

        (sandbox, source)
    }

    #[tokio::test]
    async fn a_scope_names_every_file_below_it_that_the_engine_wants() {
        let (_held, source) = sandbox().await;

        let all = files_under(&source, &RenPy, &[Scope::default()]).await;
        assert_eq!(
            keys_of(&source, &all),
            [
                "game/deep/screens.rpy",
                "game/script.rpy",
                "other/script.rpy"
            ],
            "notes.txt is not text this engine reads"
        );

        let folder = files_under(&source, &RenPy, &[Scope::read("game").unwrap()]).await;
        assert_eq!(
            keys_of(&source, &folder),
            ["game/deep/screens.rpy", "game/script.rpy"]
        );

        let one = files_under(&source, &RenPy, &[Scope::read("game/script.rpy").unwrap()]).await;
        assert_eq!(keys_of(&source, &one), ["game/script.rpy"]);
    }

    #[tokio::test]
    async fn a_scope_that_names_nothing_comes_back_empty() {
        let (_held, source) = sandbox().await;

        assert!(
            files_under(&source, &RenPy, &[Scope::read("gam").unwrap()])
                .await
                .is_empty(),
            "half a folder name must not reach into it"
        );
    }

    #[test]
    fn a_game_from_an_editor_this_app_turns_down_is_named_rather_than_called_no_game_at_all() {
        let sandbox = tempfile::tempdir().expect("a temp folder");
        fs::write(sandbox.path().join("Game.rgssad"), [0u8; 8]).expect("an archive");

        let why = match engine_for(sandbox.path()) {
            Ok(found) => panic!("{} claimed a game it cannot translate", found.label()),
            Err(why) => format!("{why:#}"),
        };

        assert_eq!(
            why, "RPG Maker XP cannot draw text outside ASCII, only VX Ace and newer keep UTF-8.",
            "telling someone holding an RPG Maker game that this is not an RPG Maker game sends \
             them hunting for a bug in us"
        );
    }

    #[test]
    fn a_stray_file_from_an_older_editor_does_not_lock_the_reader_out_of_a_game_that_reads_fine() {
        let ace = tempfile::tempdir().expect("a temp folder");
        fs::write(ace.path().join("Game.rgss3a"), [0u8; 4]).expect("an archive");
        fs::create_dir_all(ace.path().join("Data")).expect("a data folder");
        fs::write(ace.path().join("Data").join("Map001.rxdata"), [4, 8, b'0'])
            .expect("a file left over from an older editor");

        assert!(
            engine_for(ace.path()).is_ok(),
            "a game an engine claims is opened whatever else lies around it, or one stray file \
             from an older editor would lock the reader out of a game that reads fine"
        );
    }

    #[test]
    fn a_folder_no_engine_claims_is_turned_down_in_one_line() {
        let sandbox = tempfile::tempdir().expect("a temp folder");
        fs::write(sandbox.path().join("readme.txt"), []).expect("a file");

        let why = match engine_for(sandbox.path()) {
            Ok(found) => panic!("{} claimed a folder with nothing in it", found.label()),
            Err(why) => format!("{why:#}"),
        };

        assert_eq!(
            why, "Not a Ren'Py, RPG Maker, Wolf RPG or Unity game.",
            "this reaches the drop screen as it is written, so it stays one short line and never \
             names back the folder the reader dropped a moment ago"
        );
    }

    #[test]
    fn a_folder_that_is_no_longer_there_is_not_blamed_for_being_no_game() {
        let why = match engine_for(Path::new("")) {
            Ok(found) => panic!("{} claimed a folder that was never named", found.label()),
            Err(why) => format!("{why:#}"),
        };

        assert_eq!(
            why, "That folder is not there any more.",
            "a path with no folder behind it says nothing about which engine wrote the game, and \
             a reader who closed their project reads that as the app losing their game"
        );
    }
}
