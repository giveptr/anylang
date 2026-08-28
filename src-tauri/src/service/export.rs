use crate::engine::{Engine, Extra, Install, Landing, Undo};
use crate::progress::{Progress, Source};
use crate::project::Project;
use crate::scope::{self, Scope, key};
use crate::service::locate::{files_under, open_game};
use crate::store::Store;
use crate::{backup, store, walk};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::fmt;
use std::path::Path;
use std::sync::Arc;

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Exported {
    pub files: u32,
    pub lines: u32,
    pub reverted: u32,
    pub landed: Vec<String>,
    pub gone: Vec<String>,
}

pub enum Push<'a> {
    Into(&'a [Scope]),
    Back(&'a [Scope]),
}

enum Landed {
    Written(u32),
    Reverted,
    Untouched,
}

struct Shipment<'a> {
    engine: Arc<dyn Engine>,
    store: &'a Store,
    project: &'a Project,
    progress: &'a dyn Progress,
    at: Landing<'a>,
    source: &'a Path,
    target: &'a Path,
}

#[tracing::instrument(name = "export.run", skip_all)]
pub async fn export(game_dir: &Path, progress: &dyn Progress, push: Push<'_>) -> Result<Exported> {
    let game = open_game(game_dir).await?;

    let source = &game.source;
    let engine = &game.engine;
    let store = &game.store;
    let at = Landing {
        game_dir: &game.at,
        store: store.root(),
        language: &game.project.language,
    };
    let target = engine.output(at);

    let shipment = Shipment {
        engine: Arc::clone(engine),
        store,
        project: &game.project,
        progress,
        at,
        source,
        target: &target,
    };

    let (undoing, reach) = match push {
        Push::Into(asked) => (false, asked),
        Push::Back(asked) => (true, asked),
    };
    let doing = match undoing {
        true => Source::Restore,
        false => Source::Export,
    };
    let everything = undoing_everything(undoing, reach);

    let mut done = Exported::default();

    let ruling = game.project.picks.ruling(engine.wanted_by_default());

    for path in files_under(source, engine.as_ref(), reach).await {
        let at = key(source, &path);
        let dropped = undoing || !ruling.wants(&at);

        match shipment.one(&path, dropped).await? {
            Landed::Written(lines) => {
                done.files += 1;
                done.lines += lines;
                done.landed.push(at);
            }
            Landed::Reverted => {
                done.reverted += 1;
                done.gone.push(at);
            }
            Landed::Untouched => {
                if dropped {
                    done.gone.push(at);
                }
            }
        }
    }

    shipment.extras(undoing, reach).await?;

    engine
        .install(Install {
            game_dir: &game.at,
            staged: &target,
            store: store.root(),
            fonts: &game.project.fonts,
            pictures: &game.project.pictures,
            reverting: everything,
            progress,
            doing,
        })
        .await?;

    if everything && backup::handed_back(store.root(), game_dir, &target).await? {
        progress.info(
            doing,
            "the translation this game ships of its own is back where it was",
        );
    }

    progress.info(doing, &told(&done, undoing, reach));

    Ok(done)
}

fn told(done: &Exported, undoing: bool, reach: &[Scope]) -> String {
    let what = match undoing {
        true => match done.reverted {
            0 => "nothing to put back".to_string(),
            put => format!("{put} file(s) put back"),
        },
        false => {
            let mut what = format!(
                "{} line(s) written across {} file(s)",
                done.lines, done.files
            );
            if done.reverted > 0 {
                what.push_str(&format!(", {} put back to the original", done.reverted));
            }

            what
        }
    };

    scope::prefixed(reach, what)
}

fn undoing_everything(undoing: bool, reach: &[Scope]) -> bool {
    undoing && scope::anywhere(reach)
}

impl Shipment<'_> {
    async fn gave_up(&self, path: &Path, landing: &Path, why: impl fmt::Display) -> Result<Landed> {
        self.progress.warn(
            Source::Export,
            &format!("{}: {why}, putting the original back", path.display()),
        );

        self.revert(landing).await
    }

    async fn one(&self, path: &Path, dropped: bool) -> Result<Landed> {
        let landing = self.target.join(key(self.source, path));

        if dropped {
            return self.revert(&landing).await;
        }

        let text = match tokio::fs::read_to_string(path).await {
            Ok(text) => text,
            Err(why) => {
                return self.gave_up(path, &landing, why).await;
            }
        };

        let engine = Arc::clone(&self.engine);
        let here = path.to_path_buf();
        let parsed = tokio::task::spawn_blocking(move || engine.parse(&here, &text)).await?;

        let state = self.store.load(path, parsed.units()).await?;

        if state.said().is_empty() {
            return self.revert(&landing).await;
        }

        let translations = state.into_said();
        let rendered = tokio::task::spawn_blocking(move || {
            parsed
                .render(&translations)
                .map_err(|why| format!("{why:#}"))
        })
        .await?;

        let (text, applied) = match rendered {
            Ok(done) => done,
            Err(why) => {
                return self.gave_up(path, &landing, why).await;
            }
        };
        let text = self.engine.retarget(&text, &self.project.language);

        self.land(&landing, text.into_owned()).await?;

        Ok(Landed::Written(applied.lines))
    }

    async fn revert(&self, landing: &Path) -> Result<Landed> {
        let undone = match self.engine.undo() {
            Undo::Restore => backup::put_back(self.store.root(), self.at.game_dir, landing).await?,
            Undo::Remove => walk::removed(landing).await?,
        };

        Ok(if undone {
            Landed::Reverted
        } else {
            Landed::Untouched
        })
    }

    async fn land(&self, landing: &Path, body: String) -> Result<()> {
        if let Some(parent) = landing.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        if self.engine.undo() == Undo::Restore {
            return backup::replace(
                self.store.root(),
                self.at.game_dir,
                landing,
                body.into_bytes(),
            )
            .await;
        }

        store::write_atomically(landing, body).await
    }

    async fn extras(&self, undoing: bool, reach: &[Scope]) -> Result<()> {
        if undoing {
            if !scope::anywhere(reach) {
                return Ok(());
            }

            return remove_extras(self.engine.as_ref(), self.project, self.at).await;
        }

        for extra in self
            .engine
            .extras(self.at, &self.project.tweaks, &self.project.fonts)
        {
            let landing = self.at.game_dir.join(extra.at());

            if let Some(parent) = landing.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }

            match extra {
                Extra::Write { body, .. } => store::write_atomically(&landing, &body).await?,
                Extra::Copy { from, .. } => {
                    tokio::fs::copy(&from, &landing).await.with_context(|| {
                        format!("copying {} to {}", from.display(), landing.display())
                    })?;
                }
            }
        }

        Ok(())
    }
}

pub async fn remove_extras(engine: &dyn Engine, project: &Project, at: Landing<'_>) -> Result<()> {
    for extra in engine.extras(at, &project.tweaks, &project.fonts) {
        walk::removed(&at.game_dir.join(extra.at())).await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Tweaks;
    use crate::engine::renpy::{Options, RenPy, SWITCH_FILE};
    use crate::progress::Quiet;

    #[test]
    fn a_game_takes_its_own_files_back_only_when_the_whole_of_it_is_undone() {
        let whole = [Scope::read("").expect("the whole game")];
        let one = [Scope::read("data/Map001.json").expect("one file")];

        assert!(
            undoing_everything(true, &whole),
            "undoing the whole game is what puts the fonts and the files it shipped with back"
        );
        assert!(
            !undoing_everything(false, &whole),
            "applying a translation to the whole game is not an undo: reading it as one throws \
             away the fonts the reader picked and the apply looks like it did nothing"
        );
        assert!(
            !undoing_everything(true, &one),
            "undoing one file leaves the rest of the game standing, fonts and all"
        );
        assert!(!undoing_everything(false, &one));
    }

    #[tokio::test]
    async fn extras_survive_a_partial_revert_and_leave_with_a_full_one() {
        let game = tempfile::tempdir().expect("a temp folder");
        let store = Store::at(game.path().join("state"), "japanese")
            .await
            .unwrap();
        let project = Project {
            language: "Japanese".to_string(),
            tweaks: Tweaks::RenPy(Options::default()),
            ..Project::default()
        };
        let source = game.path().join("state/source");
        let target = game.path().join("game/tl/japanese");

        let shipment = Shipment {
            engine: Arc::new(RenPy),
            store: &store,
            project: &project,
            progress: &Quiet,
            at: Landing::over(game.path(), store.root(), "Japanese"),
            source: &source,
            target: &target,
        };

        let switch = game.path().join("game").join(SWITCH_FILE);

        shipment.extras(false, &[Scope::default()]).await.unwrap();
        assert!(switch.is_file());

        shipment
            .extras(true, &[Scope::read("game/script.rpy").unwrap()])
            .await
            .unwrap();
        assert!(
            switch.is_file(),
            "putting one file back must not deactivate the rest of the translation"
        );

        shipment.extras(true, &[Scope::default()]).await.unwrap();
        assert!(!switch.exists());
    }
}
