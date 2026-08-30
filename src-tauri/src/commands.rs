use crate::cancel::{Cancel, Runs, Seeker, Solo};
use crate::engine::forget;
use crate::engine::pictures::Shot;
use crate::events::Ui;
use crate::gate::{Gate, Pass, Writing};
use crate::job::Marking;
use crate::progress::{Progress, Source};
use crate::project::{self, Project};
use crate::scope::{self, Scope};
use crate::service::editor::{Found, Outline, Tally};
use crate::service::export::{Exported, Push};
use crate::service::game::Opened;
use crate::service::lines::{self, Sheets, Sift, Window};
use crate::service::seek::Seeking;
use crate::service::{clipboard, editor, export, game, logs, pictures, translate};
use crate::session::Session;
use crate::settings::{self, Settings};
use crate::{canvas, llm, store};
use anyhow::Context;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{AppHandle, Manager, State};

type Reply<T> = Result<T, String>;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Live {
    pub running: bool,
    pub files: Vec<String>,
    pub opened: Option<Opened>,
    pub failed: Option<String>,
}

fn readable(error: anyhow::Error) -> String {
    format!("{error:#}")
}

fn blamed(app: &AppHandle, source: Source) -> impl Fn(anyhow::Error) -> String + use<'_> {
    move |error| {
        Ui::new(app.clone()).failed(source, &error);

        readable(error)
    }
}

const BUSY: &str = "this game is busy. Wait for the last action to finish";

fn pass(gate: &Gate) -> Reply<Pass> {
    gate.enter().ok_or_else(|| BUSY.to_string())
}

fn pass_clearing(gate: &Gate) -> Reply<Pass> {
    gate.enter_clearing().ok_or_else(|| BUSY.to_string())
}

struct Rewriting<'a> {
    _pass: Pass,
    sheets: &'a Sheets,
}

impl Drop for Rewriting<'_> {
    fn drop(&mut self) {
        self.sheets.forget();
    }
}

fn rewriting(pass: Pass, sheets: &Sheets) -> Rewriting<'_> {
    Rewriting {
        _pass: pass,
        sheets,
    }
}

fn shut(session: &Session, sheets: &Sheets) {
    forget();
    session.closed();
    sheets.forget();
}

#[tracing::instrument(level = "debug", name = "pictures", skip_all)]
#[tauri::command]
#[specta::specta]
pub async fn pictures(app: AppHandle, game_dir: String) -> Reply<Vec<Shot>> {
    pictures::listed(Path::new(&game_dir))
        .await
        .map_err(blamed(&app, Source::Project))
}

fn rooted(app: &AppHandle, game_dir: &str) -> Reply<PathBuf> {
    store::root_for(Path::new(game_dir)).map_err(blamed(app, Source::Project))
}

#[tracing::instrument(level = "debug", name = "picture_shown", skip_all)]
#[tauri::command]
#[specta::specta]
pub async fn picture_shown(
    app: AppHandle,
    game_dir: String,
    key: String,
    most: u32,
) -> Reply<pictures::Shown> {
    let root = rooted(&app, &game_dir)?;

    pictures::shown(Path::new(&game_dir), &root, &key, most)
        .await
        .map_err(blamed(&app, Source::Project))
}

async fn picked(game_dir: &str) -> Reply<()> {
    let project = project::require(Path::new(game_dir))
        .await
        .map_err(readable)?;

    match project.folder().is_empty() {
        false => Ok(()),
        true => Err(
            "no language is picked yet, so there is nowhere to keep a translation. Choose one \
             under Languages first."
                .to_string(),
        ),
    }
}

fn writing(gate: &Gate) -> Reply<Writing> {
    gate.writing()
        .ok_or_else(|| "this game is being cleared. Wait for it to finish".to_string())
}

#[tracing::instrument(level = "info", name = "pick_game", skip_all)]
#[tauri::command]
#[specta::specta]
pub async fn pick_game(
    app: AppHandle,
    session: State<'_, Session>,
    dropped: String,
) -> Reply<Opened> {
    let (root, opened) = game::opening(Path::new(&dropped), None)
        .await
        .map_err(blamed(&app, Source::Project))?;

    session.opening(&opened.game_dir, root);

    Ok(opened)
}

#[tracing::instrument(level = "debug", name = "font_shown", skip_all)]
#[tauri::command]
#[specta::specta]
pub async fn font_shown(app: AppHandle, at: String) -> Reply<()> {
    let blame = blamed(&app, Source::Project);
    let path = Path::new(&at);

    let found = tokio::fs::metadata(path)
        .await
        .with_context(|| format!("reading {}", path.display()))
        .map_err(&blame)?;
    if !found.is_file() {
        return Err(blame(anyhow::anyhow!(
            "{} is not a file this reader can preview",
            path.display()
        )));
    }

    app.asset_protocol_scope()
        .allow_file(path)
        .context("letting the window read this font")
        .map_err(&blame)
}

#[tracing::instrument(level = "debug", name = "picture_kinds", skip_all)]
#[tauri::command]
#[specta::specta]
pub fn picture_kinds() -> Vec<String> {
    canvas::kinds().into_iter().map(str::to_string).collect()
}

#[tracing::instrument(level = "debug", name = "pictures_at_once", skip_all)]
#[tauri::command]
#[specta::specta]
pub fn pictures_at_once() -> u32 {
    pictures::at_once() as u32
}

#[tracing::instrument(level = "info", name = "save_picture", skip_all)]
#[tauri::command]
#[specta::specta]
pub async fn save_picture(app: AppHandle, game_dir: String, key: String, at: String) -> Reply<()> {
    let root = rooted(&app, &game_dir)?;

    pictures::saved(Path::new(&game_dir), &root, &key, Path::new(&at))
        .await
        .map_err(blamed(&app, Source::Project))
}

#[tracing::instrument(level = "debug", name = "copy_picture", skip_all)]
#[tauri::command]
#[specta::specta]
pub async fn copy_picture(app: AppHandle, game_dir: String, key: String) -> Reply<()> {
    let root = rooted(&app, &game_dir)?;

    let held = pictures::drawn(Path::new(&game_dir), &root, &key)
        .await
        .map_err(blamed(&app, Source::Project))?;

    clipboard::drew(held)
        .await
        .map_err(blamed(&app, Source::Project))
}

#[tracing::instrument(level = "info", name = "keep_picture", skip_all)]
#[tauri::command]
#[specta::specta]
pub async fn keep_picture(app: AppHandle, game_dir: String, at: String) -> Reply<String> {
    let root = rooted(&app, &game_dir)?;

    pictures::kept(&root, Path::new(&at))
        .await
        .map(|at| at.to_string_lossy().to_string())
        .map_err(blamed(&app, Source::Project))
}

#[tracing::instrument(level = "info", name = "paste_picture", skip_all)]
#[tauri::command]
#[specta::specta]
pub async fn paste_picture(app: AppHandle, game_dir: String) -> Reply<Option<String>> {
    let root = rooted(&app, &game_dir)?;

    clipboard::picture(&root)
        .await
        .map(|kept| kept.map(|at| at.to_string_lossy().to_string()))
        .map_err(blamed(&app, Source::Project))
}

#[tracing::instrument(level = "debug", name = "replacement_shown", skip_all)]
#[tauri::command]
#[specta::specta]
pub async fn replacement_shown(app: AppHandle, at: String) -> Reply<pictures::Replacement> {
    pictures::from_file(Path::new(&at))
        .await
        .map_err(blamed(&app, Source::Project))
}

#[tracing::instrument(level = "info", name = "save_project", skip_all)]
#[tauri::command]
#[specta::specta]
pub async fn save_project(
    app: AppHandle,
    gate: State<'_, Gate>,
    game_dir: String,
    project: Project,
) -> Reply<()> {
    let _pass = pass(&gate)?;

    game::save(Path::new(&game_dir), &project)
        .await
        .map_err(blamed(&app, Source::Project))
}

#[tracing::instrument(level = "debug", name = "preview_prompt", skip_all)]
#[tauri::command]
#[specta::specta]
pub async fn preview_prompt(app: AppHandle, game_dir: String, project: Project) -> Reply<String> {
    translate::preview(Path::new(&game_dir), &project)
        .await
        .map_err(blamed(&app, Source::Project))
}

#[tracing::instrument(level = "debug", name = "survey", skip_all)]
#[tauri::command]
#[specta::specta]
pub async fn survey(app: AppHandle, game_dir: String) -> Reply<Tally> {
    game::survey(Path::new(&game_dir), &Ui::new(app.clone()))
        .await
        .map_err(blamed(&app, Source::Project))
}

#[tracing::instrument(level = "info", name = "prepare_game", skip_all, fields(afresh))]
#[tauri::command]
#[specta::specta]
pub async fn prepare_game(
    app: AppHandle,
    gate: State<'_, Gate>,
    session: State<'_, Session>,
    sheets: State<'_, Arc<Sheets>>,
    game_dir: String,
    project: Project,
    afresh: bool,
) -> Reply<game::Ready> {
    let _rewriting = rewriting(pass(&gate)?, &sheets);
    forget();

    let blame = blamed(&app, Source::Prepare);

    let root = store::root_for(Path::new(&game_dir)).map_err(&blame)?;
    session.opening(&game_dir, root.clone());

    let ready = game::prepare(
        Path::new(&game_dir),
        &project,
        &Ui::new(app.clone()),
        afresh,
    )
    .await
    .map_err(&blame)?;

    session.opened(&game_dir, root, ready.survey);

    Ok(ready)
}

#[tracing::instrument(level = "info", name = "forget_logs", skip_all)]
#[tauri::command]
#[specta::specta]
pub async fn forget_logs(app: AppHandle, game_dir: String) -> Reply<()> {
    logs::forget(Path::new(&game_dir))
        .await
        .map_err(blamed(&app, Source::Clear))
}

#[tracing::instrument(level = "info", name = "export_scope", skip_all, fields(scope = %scopes.join(" ")))]
#[tauri::command]
#[specta::specta]
pub async fn export_scope(
    app: AppHandle,
    gate: State<'_, Gate>,
    game_dir: String,
    scopes: Vec<String>,
) -> Reply<Exported> {
    let _pass = pass(&gate)?;

    let reach = scope::reach(&scopes).map_err(blamed(&app, Source::Export))?;

    export::export(
        Path::new(&game_dir),
        &Ui::new(app.clone()),
        Push::Into(&reach),
    )
    .await
    .map_err(blamed(&app, Source::Export))
}

#[tracing::instrument(level = "info", name = "clear_scope", skip_all, fields(scope = %scopes.join(" ")))]
#[tauri::command]
#[specta::specta]
pub async fn clear_scope(
    app: AppHandle,
    gate: State<'_, Gate>,
    sheets: State<'_, Arc<Sheets>>,
    game_dir: String,
    scopes: Vec<String>,
) -> Reply<Vec<String>> {
    let _rewriting = rewriting(pass_clearing(&gate)?, &sheets);

    let reach = scope::reach(&scopes).map_err(blamed(&app, Source::Clear))?;

    game::clear_scope(Path::new(&game_dir), &Ui::new(app.clone()), &reach)
        .await
        .map_err(blamed(&app, Source::Clear))
}

#[tracing::instrument(level = "info", name = "revert_scope", skip_all, fields(scope = %scopes.join(" ")))]
#[tauri::command]
#[specta::specta]
pub async fn revert_scope(
    app: AppHandle,
    gate: State<'_, Gate>,
    game_dir: String,
    scopes: Vec<String>,
) -> Reply<Exported> {
    let _pass = pass(&gate)?;

    let reach = scope::reach(&scopes).map_err(blamed(&app, Source::Restore))?;

    export::export(
        Path::new(&game_dir),
        &Ui::new(app.clone()),
        Push::Back(&reach),
    )
    .await
    .map_err(blamed(&app, Source::Restore))
}

#[tracing::instrument(level = "info", name = "close_project", skip_all)]
#[tauri::command]
#[specta::specta]
pub fn close_project(session: State<'_, Session>, sheets: State<'_, Arc<Sheets>>) {
    shut(&session, &sheets);
}

#[tracing::instrument(level = "info", name = "delete_project", skip_all)]
#[tauri::command]
#[specta::specta]
pub async fn delete_project(
    app: AppHandle,
    gate: State<'_, Gate>,
    session: State<'_, Session>,
    sheets: State<'_, Arc<Sheets>>,
    game_dir: String,
) -> Reply<()> {
    let _pass = pass_clearing(&gate)?;
    let dir = Path::new(&game_dir);

    game::forget(dir)
        .await
        .map_err(blamed(&app, Source::Project))?;

    shut(&session, &sheets);

    Ok(())
}

#[tracing::instrument(level = "debug", name = "load_settings", skip_all)]
#[tauri::command]
#[specta::specta]
pub async fn load_settings(app: AppHandle) -> Reply<Settings> {
    settings::load()
        .await
        .map_err(blamed(&app, Source::Session))
}

#[tracing::instrument(level = "info", name = "try_settings", skip_all)]
#[tauri::command]
#[specta::specta]
pub async fn try_settings(settings: Settings) -> Reply<()> {
    let model = llm::build(&settings, &settings.tuning().probing())
        .await
        .map_err(readable)?;

    model
        .reachable(&Cancel::default())
        .await
        .map_err(|error| error.to_string())
}

#[tracing::instrument(level = "info", name = "save_settings", skip_all)]
#[tauri::command]
#[specta::specta]
pub async fn save_settings(app: AppHandle, settings: Settings) -> Reply<()> {
    settings::save(&settings)
        .await
        .map_err(blamed(&app, Source::Session))
}

#[tracing::instrument(level = "info", name = "translate_scope", skip_all, fields(scope = %scopes.join(" ")))]
#[tauri::command]
#[specta::specta]
pub async fn translate_scope(
    app: AppHandle,
    runs: State<'_, Runs>,
    gate: State<'_, Gate>,
    sheets: State<'_, Arc<Sheets>>,
    game_dir: String,
    scopes: Vec<String>,
) -> Reply<()> {
    let _pass = pass(&gate)?;
    picked(&game_dir).await?;

    let blame = blamed(&app, Source::Translate);

    let reach = scope::reach(&scopes).map_err(&blame)?;
    let claim = runs
        .claim()
        .ok_or_else(|| "a translation is already running".to_string())?;

    let saved = settings::load().await.map_err(&blame)?;
    let progress = Arc::new(Ui::new(app.clone()));

    translate::run(
        Path::new(&game_dir),
        &saved,
        progress,
        Arc::clone(&sheets) as Arc<dyn Marking>,
        claim.tokens.clone(),
        &reach,
    )
    .await
    .map_err(&blame)
}

#[tracing::instrument(level = "info", name = "translate_line", skip_all, fields(%file, id))]
#[tauri::command]
#[specta::specta]
pub async fn translate_line(
    app: AppHandle,
    gate: State<'_, Gate>,
    solo: State<'_, Solo>,
    game_dir: String,
    file: String,
    id: u32,
) -> Reply<String> {
    let _writing = writing(&gate)?;
    let cancel = solo.afresh(&file);
    picked(&game_dir).await?;

    let blame = blamed(&app, Source::Translate);

    let saved = settings::load().await.map_err(&blame)?;
    let scope = Scope::read(&file).map_err(&blame)?;

    translate::one_line(
        Path::new(&game_dir),
        &saved,
        &scope,
        id,
        &cancel,
        &Ui::new(app.clone()),
    )
    .await
    .map_err(|why| match why.downcast_ref::<llm::CallError>() {
        Some(llm::CallError::Stopped) => why.to_string(),
        _ => blame(why),
    })
}

#[tracing::instrument(level = "info", name = "stop_line", skip_all)]
#[tauri::command]
#[specta::specta]
pub fn stop_line(solo: State<'_, Solo>) {
    solo.stop(&[Scope::default()]);
}

#[tracing::instrument(level = "debug", name = "live", skip_all)]
#[tauri::command]
#[specta::specta]
pub async fn live(
    app: AppHandle,
    runs: State<'_, Runs>,
    session: State<'_, Session>,
    sheets: State<'_, Arc<Sheets>>,
) -> Reply<Live> {
    let running = runs.running();
    let files = runs.active();

    let held = session.game_dir();
    if held.is_empty() {
        return Ok(Live {
            running,
            files,
            opened: None,
            failed: None,
        });
    }

    match game::opening(Path::new(&held), session.survey())
        .await
        .map_err(blamed(&app, Source::Project))
    {
        Ok((_, opened)) => Ok(Live {
            running,
            files,
            opened: Some(opened),
            failed: None,
        }),
        Err(why) => {
            if !running {
                shut(&session, &sheets);
            }

            Ok(Live {
                running,
                files,
                opened: None,
                failed: Some(why),
            })
        }
    }
}

#[tracing::instrument(level = "info", name = "stop_scope", skip_all, fields(scope = %scopes.join(" ")))]
#[tauri::command]
#[specta::specta]
pub fn stop_scope(
    app: AppHandle,
    runs: State<'_, Runs>,
    solo: State<'_, Solo>,
    scopes: Vec<String>,
) -> Reply<()> {
    let reach = scope::reach(&scopes).map_err(blamed(&app, Source::Translate))?;

    Ui::new(app).warn(
        Source::Translate,
        &format!("stopping {}", scope::named(&reach)),
    );
    runs.stop(&reach);
    solo.stop(&reach);

    Ok(())
}

#[tracing::instrument(level = "debug", name = "list_rows", skip_all)]
#[tauri::command]
#[specta::specta]
pub async fn list_rows(app: AppHandle, game_dir: String) -> Reply<Outline> {
    editor::list_rows(Path::new(&game_dir))
        .await
        .map_err(blamed(&app, Source::Project))
}

#[tracing::instrument(level = "debug", name = "search", skip_all)]
#[tauri::command]
#[specta::specta]
pub async fn search(
    app: AppHandle,
    seeker: State<'_, Seeker>,
    game_dir: String,
    query: String,
    how: Seeking,
) -> Reply<Option<Vec<Found>>> {
    editor::search(Path::new(&game_dir), &query, how, seeker.afresh())
        .await
        .map_err(blamed(&app, Source::Project))
}

#[tracing::instrument(level = "info", name = "exclude_scope", skip_all, fields(scope = %scopes.join(" "), excluded))]
#[tauri::command]
#[specta::specta]
pub async fn exclude_scope(
    app: AppHandle,
    gate: State<'_, Gate>,
    sheets: State<'_, Arc<Sheets>>,
    game_dir: String,
    scopes: Vec<String>,
    excluded: bool,
) -> Reply<Vec<String>> {
    let _rewriting = rewriting(pass(&gate)?, &sheets);

    let reach = scope::reach(&scopes).map_err(blamed(&app, Source::Exclude))?;

    editor::exclude(Path::new(&game_dir), &reach, excluded)
        .await
        .map_err(blamed(&app, Source::Exclude))
}

#[tracing::instrument(level = "debug", name = "read_lines", skip_all)]
#[tauri::command]
#[specta::specta]
pub async fn read_lines(
    app: AppHandle,
    sheets: State<'_, Arc<Sheets>>,
    game_dir: String,
    scope: String,
    sift: Sift,
    from: u32,
    count: u32,
) -> Reply<Window> {
    let blame = blamed(&app, Source::Project);
    let scope = Scope::read(&scope).map_err(&blame)?;

    lines::read(&sheets, Path::new(&game_dir), &scope, &sift, from, count)
        .await
        .map_err(&blame)
}

#[tauri::command]
#[specta::specta]
#[allow(clippy::too_many_arguments)]
#[tracing::instrument(level = "debug", name = "read_lines_around", skip_all)]
pub async fn read_lines_around(
    app: AppHandle,
    sheets: State<'_, Arc<Sheets>>,
    game_dir: String,
    scope: String,
    sift: Sift,
    file: String,
    id: u32,
    count: u32,
) -> Reply<Window> {
    let blame = blamed(&app, Source::Project);
    let scope = Scope::read(&scope).map_err(&blame)?;

    lines::read_around(
        &sheets,
        Path::new(&game_dir),
        &scope,
        &sift,
        &file,
        id,
        count,
    )
    .await
    .map_err(&blame)
}

#[tracing::instrument(level = "info", name = "save_entry", skip_all, fields(%file, id))]
#[tauri::command]
#[specta::specta]
pub async fn save_entry(
    app: AppHandle,
    gate: State<'_, Gate>,
    sheets: State<'_, Arc<Sheets>>,
    game_dir: String,
    file: String,
    id: u32,
    translation: Option<String>,
) -> Reply<()> {
    let _writing = writing(&gate)?;
    picked(&game_dir).await?;

    let blame = blamed(&app, Source::Project);
    let scope = Scope::read(&file).map_err(&blame)?;

    lines::save(&sheets, Path::new(&game_dir), &scope, id, translation)
        .await
        .map_err(&blame)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_that_rewrote_the_store_drops_the_row_index_on_the_way_out() {
        let gate = Gate::default();
        let sheets = Sheets::stand_in();

        let held = rewriting(pass(&gate).expect("the first action goes through"), &sheets);
        assert!(
            sheets.held(),
            "the index is what the open editor is reading from, so it may not go until the \
             rewrite is over"
        );

        drop(held);
        assert!(
            !sheets.held(),
            "the rows counted lines that have just been rewritten, and an early return out of \
             the command body is the path most likely to leave them standing"
        );
        assert!(
            gate.enter().is_some(),
            "the index goes before the gate does: a command let in between the two would read \
             rows that no longer say what the store says"
        );
    }
}
