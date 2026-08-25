use crate::engine::Prepare;
use crate::engine::pictures::{Shot, counted};
use crate::engine::unity::serial::{Container, Object};
use crate::engine::unity::{
    Harvest, Known, Learning, assemblies_beside, assembly, container_kind, data_dir, fonts,
    holder_of, localization, mono_behaviour, opened, pictures, serial, text_asset,
};
use crate::progress::Source;
use crate::{backup, walk};
use anyhow::{Context, Result};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const STEPS: &[&str] = &["Reading the game's containers", "Taking the text in"];

#[tracing::instrument(name = "unity.prepare", skip_all)]
pub async fn run(at: Prepare<'_>) -> Result<()> {
    let data = data_dir(at.game_dir)
        .with_context(|| format!("no Unity data folder in {}", at.game_dir.display()))?;
    let inside = data.strip_prefix(at.game_dir).ok();

    at.progress.stage(STEPS, 0);

    let listing: Vec<PathBuf> = walk::relative(at.game_dir)
        .await
        .into_iter()
        .filter(|relative| !backup::is_part(relative))
        .collect();

    let mut parsed: Vec<(String, opened::Opened)> = Vec::new();
    for relative in &listing {
        let holder = holder_of(relative, inside);
        let path = at.game_dir.join(relative);
        match container_kind(&path).await {
            Ok(Some(_)) => {}
            Ok(None) => continue,
            Err(why) => {
                at.progress
                    .warn(Source::Prepare, &format!("{holder}: {why:#}"));
                continue;
            }
        }

        let here = holder.clone();
        match tokio::task::spawn_blocking(move || opened::harvested_from(&here, &path)).await? {
            Ok(one) => parsed.push((holder, one)),
            Err(why) => at
                .progress
                .warn(Source::Prepare, &format!("{holder}: {why:#}")),
        }
    }

    let assemblies = assemblies_beside(at.game_dir).await;
    let mut learning = Learning::default();
    let mut lifting = fonts::Lifting::default();
    for (_, one) in &parsed {
        for held in &one.containers {
            learning.take_in(held, &assemblies);
            lifting.take_in(at.store, &held.objects);
        }
    }
    for name in lifting.missed() {
        at.progress.warn(
            Source::Prepare,
            &format!("{name} could not be copied out of the game, so it is not offered to swap"),
        );
    }
    fonts::remember(at.store, &lifting.landed).await?;
    let known = Arc::new(learning.done(assemblies));

    let mut reaped: Vec<Harvest> = Vec::new();
    let mut shots: Vec<Shot> = Vec::new();
    let mut containers = 0;

    for (holder, one) in parsed {
        let told = Arc::clone(&known);
        let here = holder.clone();
        match tokio::task::spawn_blocking(move || reap(&here, one, &told)).await? {
            Ok(Some(held)) => {
                containers += 1;
                if let Some(told) = held.told {
                    at.progress
                        .warn(Source::Prepare, &format!("{holder}: {told}"));
                }
                reaped.extend(held.found);
                shots.extend(held.shots);
            }
            Ok(None) => {}
            Err(why) => at
                .progress
                .warn(Source::Prepare, &format!("{holder}: {why:#}")),
        }
    }

    let (found, assemblies) = reap_assemblies(&at, &listing, inside).await?;
    reaped.extend(found);

    let pieces: u32 = reaped.iter().map(|one| one.lines).sum();
    let mut said = format!("{containers} container(s)");
    if assemblies > 0 {
        said.push_str(&format!(" and {assemblies} assembly(s)"));
    }
    said.push_str(&format!(
        " out of {} file(s), {pieces} piece(s) of text",
        listing.len()
    ));
    if !lifting.landed.is_empty() {
        said.push_str(&format!(
            ", {} font(s) that can be swapped",
            lifting.landed.len()
        ));
    }
    if let Some(told) = counted(&shots) {
        said.push_str(&format!(", {told}"));
    }

    at.progress.info(Source::Prepare, &said);
    pictures::LEDGER.remember(at.store, &shots).await?;

    at.progress.stage(STEPS, 1);

    let (reaped, clashed) = apart(reaped);
    for one in &clashed {
        at.progress.warn(
            Source::Prepare,
            &format!(
                "two pieces of this game both land on {}, so the second is left unread rather than \
                 written over the first",
                one.display()
            ),
        );
    }

    for one in reaped {
        let landing = at.source.join(&one.at);
        if let Some(parent) = landing.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        tokio::fs::write(&landing, one.body)
            .await
            .with_context(|| format!("writing {}", landing.display()))?;
    }

    Ok(())
}

fn apart(reaped: Vec<Harvest>) -> (Vec<Harvest>, Vec<PathBuf>) {
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut kept = Vec::with_capacity(reaped.len());
    let mut clashed = Vec::new();

    for one in reaped {
        match seen.insert(one.at.clone()) {
            true => kept.push(one),
            false => clashed.push(one.at),
        }
    }

    (kept, clashed)
}

async fn reap_assemblies(
    at: &Prepare<'_>,
    listing: &[PathBuf],
    inside: Option<&Path>,
) -> Result<(Vec<Harvest>, u32)> {
    let mut reaped = Vec::new();
    let mut read = 0;

    for relative in listing.iter().filter(|one| assembly::ours(one)) {
        let holder = holder_of(relative, inside);
        let raw = match tokio::fs::read(at.game_dir.join(relative)).await {
            Ok(bytes) => bytes,
            Err(why) => {
                at.progress
                    .warn(Source::Prepare, &format!("{holder}: {why}"));
                continue;
            }
        };

        let here = holder.clone();
        match tokio::task::spawn_blocking(move || assembly::take(&here, &raw)).await? {
            Ok(found) if found.is_empty() => {}
            Ok(found) => {
                read += 1;
                reaped.extend(found);
            }
            Err(why) => at
                .progress
                .warn(Source::Prepare, &format!("{holder}: {why:#}")),
        }
    }

    Ok((reaped, read))
}

struct Reaped {
    found: Vec<Harvest>,
    shots: Vec<Shot>,
    told: Option<String>,
}

fn reap(holder: &str, opened: opened::Opened, known: &Known) -> Result<Option<Reaped>> {
    let containers = &opened.containers;
    if containers.is_empty() {
        return Ok(None);
    }

    let unreadable = containers
        .iter()
        .all(|one| serial::unnamed(&one.built_by) && !one.objects.iter().any(Object::shaped));
    let told = unreadable.then(|| {
        "this file does not say which Unity version built it, and it does not describe its own \
         contents either, so nothing in it could be read"
            .to_string()
    });

    let shots = opened.shots(holder, &known.named);

    let nodes: Vec<&[Object]> = containers
        .iter()
        .map(|one| one.objects.as_slice())
        .collect();
    let mut found = text_asset::take(holder, &nodes)?;

    let every: Vec<&Container> = containers.iter().collect();
    let shared = mono_behaviour::shared_ids(&every);

    for (node, one) in containers.iter().enumerate() {
        found.extend(localization::take(
            one,
            |object| known.classes.of(one, object),
            &known.assemblies,
            &known.books,
        )?);
        found.extend(mono_behaviour::take(
            holder,
            one,
            node,
            containers.len(),
            &shared,
            known,
        )?);
    }

    Ok(Some(Reaped { found, shots, told }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_pieces_landing_on_one_sheet_keep_the_first_and_name_the_second() {
        let held = |at: &str, lines: u32| Harvest {
            at: PathBuf::from(at),
            body: format!("{at} body"),
            lines,
        };

        let (kept, clashed) = apart(vec![
            held("localization/UI_General/en/Menu.sheet", 4),
            held("assets/Intro.sheet", 2),
            held("localization/UI_General/en/Menu.sheet", 9),
        ]);

        assert_eq!(
            kept.iter().map(|one| one.lines).collect::<Vec<_>>(),
            [4, 2],
            "a collection called UI/General and one called UI_General fold to the same folder, and \
             writing the second over the first would hand one table's lines to both at install: \
             the reader is told instead"
        );
        assert_eq!(
            clashed,
            [PathBuf::from("localization/UI_General/en/Menu.sheet")]
        );
    }
}
