use crate::cancel::{Cancel, Tokens};
use crate::job::batch::BatchOutcome;
use crate::job::{Job, Marking, Plan, batch, prompt};
use crate::llm;
use crate::progress::{Progress, Source};
use crate::project::Project;
use crate::scope::{self, Scope, key, leaf};
use crate::service::locate::{Game, engine_at, files_under, open_game};
use crate::service::scan::{parse_file, unlocked_in};
use crate::settings::Settings;
use anyhow::{Result, anyhow, bail};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[tracing::instrument(name = "translate.run", skip_all)]
pub async fn run(
    game_dir: &Path,
    settings: &Settings,
    progress: Arc<dyn Progress>,
    marking: Arc<dyn Marking>,
    tokens: Arc<Tokens>,
    reach: &[Scope],
) -> Result<()> {
    progress.running(true);
    let held = translating(
        game_dir,
        settings,
        Arc::clone(&progress),
        marking,
        tokens,
        reach,
    )
    .await;
    progress.running(false);

    held
}

async fn translating(
    game_dir: &Path,
    settings: &Settings,
    progress: Arc<dyn Progress>,
    marking: Arc<dyn Marking>,
    tokens: Arc<Tokens>,
    reach: &[Scope],
) -> Result<()> {
    let game = open_game(game_dir).await?;
    languages(&game.project)?;

    let ruling = game.project.picks.ruling(game.engine.wanted_by_default());
    let files: Vec<PathBuf> = files_under(&game.source, game.engine.as_ref(), reach)
        .await
        .into_iter()
        .filter(|at| ruling.wants(&key(&game.source, at)))
        .collect();

    if files.is_empty() {
        bail!("there is no text to translate in {}", scope::named(reach));
    }

    let job = build(game, settings, progress, marking, tokens).await?;

    Arc::new(job).run(files).await;

    Ok(())
}

pub async fn one_line(
    game_dir: &Path,
    settings: &Settings,
    scope: &Scope,
    id: u32,
    cancel: &Cancel,
    progress: &dyn Progress,
) -> Result<String> {
    let game = open_game(game_dir).await?;
    languages(&game.project)?;

    let tuning = settings.tuning();
    let model = llm::build(settings, &tuning).await?;
    let file = scope.under(&game.source);
    let units = parse_file(&game.engine, &file).await?;
    let unit = unlocked_in(&units, scope, id)?;

    let system = prompt::system_instruction(game.engine.as_ref(), &game.project);

    let (answered, spent) = batch::translate(
        model.as_ref(),
        game.engine.as_ref(),
        &system,
        vec![unit],
        &tuning,
        cancel,
        progress,
    )
    .await;

    if spent.total() > 0 {
        progress.info(
            Source::Translate,
            &format!("{}: {}", leaf(scope.as_str()), spent.told()),
        );
    }

    let BatchOutcome {
        translations,
        notes,
        ..
    } = answered.map_err(|error| anyhow!(error))?;

    match translations.into_values().next() {
        Some(translation) => Ok(translation),
        None => bail!(
            "{}",
            notes
                .first()
                .map(String::as_str)
                .unwrap_or("the model returned nothing usable for this line")
        ),
    }
}

pub async fn preview(game_dir: &Path, project: &Project) -> Result<String> {
    let engine = engine_at(game_dir).await?;

    Ok(prompt::system_instruction(engine.as_ref(), project))
}

fn languages(project: &Project) -> Result<()> {
    if project.source_language.trim().is_empty() {
        bail!("pick the language the game is written in under Languages.");
    }

    if project.language.trim().is_empty() {
        bail!("pick the language to translate into under Languages.");
    }

    if project.source_language.trim().to_lowercase() == project.language.trim().to_lowercase() {
        bail!(
            "translating {} into {} would change nothing. Pick a different target language.",
            project.source_language.trim(),
            project.language.trim()
        );
    }

    Ok(())
}

async fn build(
    game: Game,
    settings: &Settings,
    progress: Arc<dyn Progress>,
    marking: Arc<dyn Marking>,
    tokens: Arc<Tokens>,
) -> Result<Job> {
    let tuning = settings.tuning();

    let model = llm::build(settings, &tuning).await?;
    let system = prompt::system_instruction(game.engine.as_ref(), &game.project);

    Ok(Job::new(Plan {
        engine: game.engine,
        model,
        store: game.store,
        source: game.source,
        tuning,
        system,
        progress,
        marking,
        tokens,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_language_in_a_different_case_is_refused() {
        let project = Project {
            source_language: "english".to_string(),
            language: "English".to_string(),
            ..Project::default()
        };

        assert!(
            languages(&project).is_err(),
            "the store folder lowercases, so these are one language wearing two spellings"
        );
    }
}
