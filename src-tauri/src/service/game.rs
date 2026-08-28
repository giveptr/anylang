use crate::engine::{Engine, Font, Landing, Prepare, Undo};
use crate::progress::{Progress, Source};
use crate::project::{self, Project};
use crate::scope::{self, Scope, slashed};
use crate::service::editor::{self, Tally};
use crate::service::export::{self, Push};
use crate::service::locate::{self, engine_at, files_under, keys_of, open_game, source_of};
use crate::service::logs::{self, LogEntry};
use crate::service::pictures;
use crate::store::{self, Store};
use crate::{backup, walk};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Opened {
    pub game_dir: String,
    pub project: Project,
    pub fresh: bool,
    pub piles: bool,
    pub sources: Vec<String>,
    pub faces: Vec<Font>,
    pub logs: Vec<LogEntry>,
    pub survey: Option<Tally>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Ready {
    pub survey: Tally,
    pub faces: Vec<Font>,
}

async fn faces_of(engine: &Arc<dyn Engine>, game_dir: &Path, root: &Path) -> Result<Vec<Font>> {
    let held = engine.clone();
    let here = game_dir.to_path_buf();
    let store = root.to_path_buf();

    Ok(tokio::task::spawn_blocking(move || held.fonts(&here, &store)).await?)
}

async fn ready_with(
    engine: &Arc<dyn Engine>,
    game_dir: &Path,
    root: &Path,
    survey: Tally,
) -> Result<Ready> {
    Ok(Ready {
        survey,
        faces: faces_of(engine, game_dir, root).await?,
    })
}

pub async fn resumable(root: &Path, reading: &str) -> Result<bool> {
    let read_before = store::read_if_there(&store::source_mark(root))
        .await?
        .unwrap_or_default();

    Ok(read_before == reading && !walk::relative(&store::source_dir(root)).await.is_empty())
}

pub async fn opening(dropped: &Path, survey: Option<Tally>) -> Result<(PathBuf, Opened)> {
    let game_dir = folder_of(dropped)?;
    let root = store::root_for(&game_dir)?;
    let (engine, logs, saved) = tokio::try_join!(
        engine_at(&game_dir),
        logs::load(&game_dir),
        project::load(&game_dir),
    )?;

    let engine: Arc<dyn Engine> = engine.into();
    let project = saved.unwrap_or_default().under(engine.as_ref());
    let fresh = !resumable(&root, &engine.source_key(&project.tweaks)).await?;

    let held = engine.clone();
    let here = game_dir.clone();
    let sources = tokio::task::spawn_blocking(move || held.sources(&here)).await?;
    let faces = faces_of(&engine, &game_dir, &root).await?;

    Ok((
        root,
        Opened {
            game_dir: game_dir.to_string_lossy().to_string(),
            project,
            fresh,
            piles: engine.piles(),
            sources,
            faces,
            logs,
            survey,
        },
    ))
}

fn folder_of(dropped: &Path) -> Result<PathBuf> {
    let at = dunce::canonicalize(dropped)
        .with_context(|| format!("{} is not there any more", dropped.display()))?;

    if at.is_dir() {
        return Ok(at);
    }

    at.parent()
        .map(Path::to_path_buf)
        .with_context(|| format!("{} has no folder around it", at.display()))
}

pub async fn save(game_dir: &Path, project: &Project) -> Result<()> {
    project::save_keeping_picks(game_dir, project).await?;

    if let Ok(root) = store::root_for(game_dir) {
        pictures::forget_stray_swaps(&root, &project.pictures).await;
    }

    Ok(())
}

#[tracing::instrument(name = "game.prepare", skip_all)]
pub async fn prepare(
    game_dir: &Path,
    project: &Project,
    progress: &dyn Progress,
    afresh: bool,
) -> Result<Ready> {
    let language = project.folder();
    save(game_dir, project).await?;

    let engine: Arc<dyn Engine> = engine_at(game_dir).await?.into();

    progress.info(
        Source::Session,
        &format!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION")),
    );

    let root = store::ensure_root(game_dir).await?;
    walk::cleared(&store::tmp_dir(&root)).await?;
    pictures::forget_thumbs(&root).await;

    let reading = engine.source_key(&project.tweaks);
    let mark = store::source_mark(&root);
    let read_before = store::read_if_there(&mark).await?.unwrap_or_default();

    let kept = !afresh && resumable(&root, &reading).await?;

    if !kept {
        if !afresh && read_before != reading {
            progress.info(
                Source::Prepare,
                "the text was read from somewhere else last time, so reading the game again",
            );
        }

        as_it_shipped(
            engine.as_ref(),
            game_dir,
            &root,
            project,
            &language,
            progress,
        )
        .await?;

        let source = store::source_dir(&root);
        let staging = store::tmp_dir(&root).join("source");
        walk::cleared(&staging).await?;

        engine
            .prepare(Prepare {
                game_dir,
                source: &staging,
                store: &root,
                tweaks: &project.tweaks,
                progress,
            })
            .await?;

        walk::cleared(&source).await?;
        tokio::fs::rename(&staging, &source)
            .await
            .with_context(|| format!("moving {} into place", source.display()))?;

        store::write_atomically(&mark, &reading).await?;
    }

    let done = survey(game_dir, progress).await?;
    progress.info(Source::Prepare, &counted(&done));

    ready_with(&engine, game_dir, &root, done).await
}

async fn as_it_shipped(
    engine: &dyn Engine,
    game_dir: &Path,
    root: &Path,
    project: &Project,
    language: &str,
    progress: &dyn Progress,
) -> Result<()> {
    let back = put_back_under(root, game_dir).await?;
    if back > 0 {
        progress.info(
            Source::Prepare,
            &format!("{back} game file(s) put back before reading them again"),
        );
    }

    if engine.undo() != Undo::Remove {
        return Ok(());
    }

    let at = Landing {
        game_dir,
        store: root,
        language,
    };
    let landed = engine.output(at);

    export::remove_extras(engine, project, at).await?;

    if backup::taken_over(root, game_dir, &landed).await? {
        progress.info(
            Source::Prepare,
            &format!(
                "this game ships its own {} and that copy is kept aside: putting the game back \
                 takes the reader's work out and returns the game's own",
                landed
                    .file_name()
                    .map(|held| held.to_string_lossy().to_string())
                    .unwrap_or_default()
            ),
        );
    }

    let swept = walk::cleared(&landed).await?;

    if swept && let Ok(inside) = landed.strip_prefix(game_dir) {
        progress.info(
            Source::Prepare,
            &format!(
                "{} taken back out of the game, which reads it too",
                slashed(inside)
            ),
        );
    }

    Ok(())
}

#[tracing::instrument(name = "game.clear", skip_all)]
pub async fn clear_scope(
    game_dir: &Path,
    progress: &dyn Progress,
    reach: &[Scope],
) -> Result<Vec<String>> {
    let game = open_game(game_dir).await?;
    let source = &game.source;

    let found = files_under(source, game.engine.as_ref(), reach).await;
    let mut emptied = 0;
    for at in &found {
        if game.store.forget(at).await? {
            emptied += 1;
        }
    }

    let what = match emptied {
        0 => "nothing to clear".to_string(),
        gone => format!("{gone} file(s) cleared"),
    };
    progress.info(Source::Clear, &scope::prefixed(reach, what));

    export::export(game_dir, progress, Push::Back(reach)).await?;

    Ok(keys_of(source, &found))
}

async fn put_back_under(root: &Path, game_dir: &Path) -> Result<u32> {
    let mut back = 0;
    for file in backup::everything_kept(root, game_dir).await? {
        if backup::put_back(root, game_dir, &file).await? {
            back += 1;
        }
    }

    Ok(back)
}

pub async fn forget(game_dir: &Path) -> Result<()> {
    let root = store::root_for(game_dir)?;

    walk::cleared(&root).await?;

    Ok(())
}

fn counted(survey: &Tally) -> String {
    format!("{} line(s) across {} file(s)", survey.total, survey.files)
}

#[tracing::instrument(name = "game.survey", skip_all)]
pub async fn survey(game_dir: &Path, progress: &dyn Progress) -> Result<Tally> {
    let source = source_of(game_dir)?;

    if !source.is_dir() {
        return Ok(Tally::default());
    }

    let Some(project) = project::load(game_dir).await? else {
        return Ok(Tally::default());
    };
    let (language, picks) = (project.folder(), project.picks);

    let engine: Arc<dyn Engine> = engine_at(game_dir).await?.into();
    let store = Store::open(game_dir, &language).await?;
    let ruling = picks.ruling(engine.wanted_by_default());
    let landed = locate::landed(game_dir, engine.as_ref(), store.root(), &language).await?;

    Ok(
        editor::outline(&source, &engine, &store, &ruling, &landed, Some(progress))
            .await
            .chosen,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::renpy::RenPy;
    use crate::progress::Quiet;

    async fn wrote(at: &Path, body: &str) {
        tokio::fs::create_dir_all(at.parent().expect("a folder"))
            .await
            .expect("a folder");
        tokio::fs::write(at, body).await.expect("a file");
    }

    #[tokio::test]
    async fn letting_a_project_go_never_touches_the_game_itself() {
        let at = tempfile::tempdir().expect("a folder");
        let game_dir = at.path();
        let held = game_dir.join("Data").join("BasicData.wolf");

        wrote(&held, "the bytes the game shipped with").await;

        let root = store::root_for(game_dir).expect("a store");
        backup::replace(&root, game_dir, &held, b"a translation".to_vec())
            .await
            .expect("a translation goes in");

        assert_eq!(
            tokio::fs::read_to_string(&held).await.expect("the file"),
            "a translation",
            "the translation has to be in before letting go means anything"
        );

        forget(game_dir).await.expect("the project is let go");

        assert_eq!(
            tokio::fs::read_to_string(&held).await.expect("the file"),
            "a translation",
            "the reader finished their translation and is only letting the project data go, so \
             the game keeps the translation: anyone who wants the original back reverts first, \
             then deletes"
        );
        assert!(
            !root.is_dir(),
            "the store and the backups inside it are swept"
        );
    }

    #[test]
    fn reading_a_shipped_folder_is_a_different_source_than_reading_the_games_own_script() {
        use crate::engine::Tweaks;
        use crate::engine::renpy::Options;

        let english = Tweaks::RenPy(Options {
            shipped: "english".to_string(),
        });
        let own = Tweaks::RenPy(Options {
            shipped: String::new(),
        });

        assert_eq!(RenPy.source_key(&english), "english");
        assert_ne!(
            RenPy.source_key(&english),
            RenPy.source_key(&own),
            "reading the game's own script is not the same source as reading a shipped folder, \
             so a skeleton built from one may never be handed back for the other"
        );
        assert_eq!(
            RenPy.source_key(&own),
            RenPy.source_key(&Tweaks::None),
            "with nothing chosen there is only one source a game can be read from"
        );
    }

    #[tokio::test]
    async fn a_game_is_read_as_it_shipped_and_not_as_this_tool_left_it() {
        let sandbox = tempfile::tempdir().expect("a temp folder");
        let game_dir = sandbox.path().join("game_dir");
        let root = sandbox.path().join("store");

        let project = Project {
            language: "French".to_string(),
            tweaks: RenPy.tweaks(),
            ..Project::default()
        };

        let landed = game_dir.join("game").join("tl").join("french");
        wrote(&landed.join("a.rpy"), "translate french a_1:\n").await;
        let switch = game_dir.join("game").join("anylang.rpy");
        wrote(&switch, "init 9999 python:\n").await;

        as_it_shipped(
            &RenPy,
            &game_dir,
            &root,
            &project,
            &project.folder(),
            &Quiet,
        )
        .await
        .expect("a game put back");

        assert!(
            !landed.is_dir(),
            "an engine whose install is undone by removing has to have it removed before the \
             game is read, or the game is read with our own answers already inside it"
        );
        assert!(
            !switch.is_file(),
            "the file that switches the game over is part of what was written into it, so \
             leaving it behind leaves the game changed"
        );
    }
}
