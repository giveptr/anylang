use crate::cancel::Cancel;
use crate::engine::Engine;
use crate::picks::Ruling;
use crate::progress::{Progress, Source};
use crate::scope::{Scope, key};
use crate::service::lines::{Only, hits_among};
use crate::service::locate::{Game, files_under, keys_of, landed_in, open_game};
use crate::service::scan::{parse_file, scan_files};
use crate::service::seek::{Seeking, looking_for};
use crate::store::Store;
use crate::{project, walk};
use anyhow::Result;
use futures::StreamExt;
use regex::Regex;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::pin::pin;
use std::sync::Arc;

const SHORTEST: usize = 2;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Found {
    pub key: String,
    pub lines: Vec<u32>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Tally {
    pub files: u32,
    pub applied: u32,
    pub translated: u32,
    pub total: u32,
}

impl Tally {
    fn add(&mut self, applied: bool, translated: u32, total: u32) {
        self.files += 1;
        if applied {
            self.applied += 1;
        }
        self.translated += translated;
        self.total += total;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub scope: String,
    pub label: String,
    pub kind: String,
    pub excluded: bool,
    pub tally: Tally,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Outline {
    pub rows: Vec<Row>,
    pub chosen: Tally,
    pub whole: Tally,
}

#[tracing::instrument(name = "editor.rows", skip_all)]
pub async fn list_rows(game_dir: &Path) -> Result<Outline> {
    let game = open_game(game_dir).await?;

    let ruling = game.project.picks.ruling(game.engine.wanted_by_default());
    let landed = landed_in(&game).await?;

    Ok(outline(
        &game.source,
        &game.engine,
        &game.store,
        &ruling,
        &landed,
        None,
    )
    .await)
}

pub async fn outline(
    source: &Path,
    engine: &Arc<dyn Engine>,
    store: &Store,
    ruling: &Ruling,
    landed: &HashSet<String>,
    progress: Option<&dyn Progress>,
) -> Outline {
    let mut grouped: BTreeMap<String, Row> = BTreeMap::new();
    let mut alone: Vec<Row> = Vec::new();
    let mut out = Outline::default();

    let mut scanning = pin!(scan_files(source, engine.clone(), store));
    while let Some(found) = scanning.next().await {
        let at = key(source, &found.path);
        if let (Some(progress), Err(error)) = (progress, &found.state) {
            progress.warn(Source::Project, &format!("{at}: {error:#}"));
        }
        let excluded = !ruling.wants(&at);
        let applied = landed.contains(&at);
        let worth = found.worth();
        let total = worth.total;
        let translated = worth.translated;

        out.whole.add(applied, translated, total);
        if !excluded {
            out.chosen.add(applied, translated, total);
        }

        match engine.group(&at) {
            Some(group) => {
                let row = grouped.entry(group.key.clone()).or_insert_with(|| Row {
                    scope: group.key,
                    label: group.label,
                    kind: group.kind,
                    excluded: true,
                    tally: Tally::default(),
                });

                row.excluded &= excluded;
                row.tally.add(applied, translated, total);
            }
            None => {
                let mut row = Row {
                    label: engine.shown(&at).into_owned(),
                    scope: at,
                    kind: String::new(),
                    excluded,
                    tally: Tally::default(),
                };
                row.tally.add(applied, translated, total);
                alone.push(row);
            }
        }
    }

    let mut piled: Vec<Row> = grouped.into_values().collect();
    piled.sort_by(most_text_first);
    alone.sort_by(most_text_first);

    out.rows = piled;
    out.rows.append(&mut alone);

    out
}

fn most_text_first(a: &Row, b: &Row) -> Ordering {
    b.tally
        .total
        .cmp(&a.tally.total)
        .then_with(|| a.label.cmp(&b.label))
        .then_with(|| a.scope.cmp(&b.scope))
}

pub async fn exclude(game_dir: &Path, reach: &[Scope], excluded: bool) -> Result<Vec<String>> {
    let game = open_game(game_dir).await?;
    let source = &game.source;

    project::pick(game_dir, reach, !excluded, game.engine.wanted_by_default()).await?;

    let found = files_under(source, game.engine.as_ref(), reach).await;

    Ok(keys_of(source, &found))
}

#[tracing::instrument(name = "editor.search", skip_all)]
pub async fn search(
    game_dir: &Path,
    needle: &str,
    how: Seeking,
    cancel: Arc<Cancel>,
) -> Result<Option<Vec<Found>>> {
    let needle = needle.trim();
    if needle.chars().count() < SHORTEST && needle.is_ascii() {
        return Ok(Some(Vec::new()));
    }

    let looking = looking_for(needle, how)?;
    let game = open_game(game_dir).await?;

    let plain = !how.regex
        && needle
            .chars()
            .all(|one| one.is_ascii() && !one.is_control() && !['"', '\\', '\''].contains(&one));
    let maybe = worth_reading(&game, &looking, plain, Arc::clone(&cancel)).await?;
    if cancel.stopped() {
        return Ok(None);
    }

    let mut found = Vec::new();

    for file in maybe {
        if cancel.stopped() {
            return Ok(None);
        }

        let Ok(units) = parse_file(&game.engine, &file).await else {
            continue;
        };
        let state = game.store.load(&file, &units).await.unwrap_or_default();
        let hits = hits_among(&units, &state, Only::Yours, Some(&looking));

        let lines: Vec<u32> = hits.into_iter().map(|hit| hit.id).collect();
        if !lines.is_empty() {
            found.push(Found {
                key: key(&game.source, &file),
                lines,
            });
        }
    }

    Ok(Some(found))
}

async fn worth_reading(
    game: &Game,
    looking: &Regex,
    plain: bool,
    cancel: Arc<Cancel>,
) -> Result<Vec<PathBuf>> {
    let source = game.source.clone();
    let engine = game.engine.clone();
    let store = game.store.clone();
    let looking = looking.clone();

    let found: Vec<PathBuf> = tokio::task::spawn_blocking(move || {
        if !plain {
            return walk::files_now(&source)
                .into_iter()
                .filter(|at| engine.wants(at))
                .collect();
        }

        let says = |at: &Path| {
            !cancel.stopped() && fs::read_to_string(at).is_ok_and(|text| looking.is_match(&text))
        };

        walk::files_now(&source)
            .into_iter()
            .filter(|at| engine.wants(at))
            .filter(|at| store.path_for(at).is_some_and(|kept| says(&kept)) || says(at))
            .collect()
    })
    .await?;

    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tally_counts_files_apart_from_the_lines_inside_them() {
        let mut tally = Tally::default();

        tally.add(true, 3, 10);
        tally.add(false, 0, 4);

        assert_eq!(tally.files, 2);
        assert_eq!(tally.applied, 1, "applied counts files, not lines");
        assert_eq!(tally.translated, 3);
        assert_eq!(tally.total, 14);
    }

    #[test]
    fn the_files_holding_the_most_text_are_offered_first() {
        let row = |scope: &str, translated: u32, total: u32| Row {
            scope: scope.to_string(),
            label: scope.to_string(),
            kind: String::new(),
            excluded: false,
            tally: Tally {
                files: 1,
                applied: 0,
                translated,
                total,
            },
        };

        let mut rows = [
            row("apple.rpy", 0, 12),
            row("zebra.rpy", 900, 900),
            row("melon.rpy", 5, 400),
        ];
        rows.sort_by(most_text_first);

        let order: Vec<&str> = rows.iter().map(|one| one.scope.as_str()).collect();
        assert_eq!(
            order,
            ["zebra.rpy", "melon.rpy", "apple.rpy"],
            "the rail is sorted by how much text a file holds, not by its name, and a file that \
             is already done keeps its place rather than sinking as it is translated"
        );
    }
}
