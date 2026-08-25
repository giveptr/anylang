use crate::engine::pictures::remember;
use crate::engine::wolf_rpg::{archive, game, guard, harvest, pictures, reading, source};
use crate::engine::{Prepare, sheet};
use crate::progress::Source;
use crate::{backup, walk};
use anyhow::{Context, Result, bail};
use futures::StreamExt;
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;
use tokio::fs;

async fn spread(into: &Path, one: &Path, key: Vec<u8>) -> Result<usize> {
    let under = into.to_path_buf();
    let from = one.to_path_buf();

    tokio::task::spawn_blocking(move || {
        archive::poured(&from, &key, &under, source::worth_reading, |held| {
            if let Some(parent) = held.at.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|why| format!("making {}: {why}", parent.display()))?;
            }

            std::fs::write(&held.at, &held.body)
                .map_err(|why| format!("writing {}: {why}", held.at.display()))
        })
    })
    .await?
    .map_err(anyhow::Error::msg)
}

#[tracing::instrument(name = "wolf.unpack", skip_all)]
async fn unpacked(at: &Prepare<'_>, root: &Path) -> Result<()> {
    let taken = source::archives(at.game_dir);

    if taken.is_empty() {
        bail!("this game keeps its data packed and no archive turned up to open");
    }

    let weight = source::weight(at.game_dir);

    let unpacking: BTreeSet<_> = taken
        .iter()
        .map(|one| source::unpacks_into(root, one))
        .collect();
    for into in &unpacking {
        let covered = unpacking
            .iter()
            .any(|other| other != into && into.starts_with(other));
        if !covered {
            walk::cleared(into).await?;
        }
    }

    let mut written = 0;
    let mut turned = 0;

    for one in &taken {
        let named = one
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let into = source::unpacks_into(root, one);

        let key = match archive::key_for(one, weight) {
            Ok(Some(key)) => key,
            Ok(None) => continue,
            Err(why) => {
                at.progress
                    .warn(Source::Prepare, &format!("{named}: {why}"));
                turned += 1;
                continue;
            }
        };

        let from = backup::original_at(at.store, at.game_dir, one).await?;

        match spread(&into, &from, key).await {
            Ok(count) => written += count,
            Err(why) => {
                at.progress
                    .warn(Source::Prepare, &format!("{named}: {why:#}"));
                turned += 1;
            }
        }
    }

    if turned > 0 {
        at.progress.warn(
            Source::Prepare,
            &match written {
                0 => format!(
                    "none of the {} archive(s) in this game could be opened",
                    taken.len()
                ),
                _ => format!("{turned} archive(s) stayed shut"),
            },
        );
    }

    at.progress.info(
        Source::Prepare,
        &format!("{written} file(s) out of {} archive(s)", taken.len()),
    );

    let sealed = source::sealed(at.game_dir);
    if !sealed.is_empty() {
        at.progress.warn(
            Source::Prepare,
            &format!(
                "{} file(s) stay sealed and hold no text to read",
                sealed.len()
            ),
        );
    }

    Ok(())
}

async fn drawn(at: &Prepare<'_>) -> Result<()> {
    let held = pictures::found(at.game_dir).await;

    remember(at, &pictures::LEDGER, &held.shots, &held.shut).await
}

const STEPS: [&str; 3] = [
    "Reading the game data",
    "Listing the pictures",
    "Taking the text in",
];

#[tracing::instrument(name = "wolf.prepare", skip_all)]
pub async fn run(label: &str, at: Prepare<'_>) -> Result<()> {
    at.progress.stage(&STEPS, 0);

    let root = source::root(at.game_dir, at.store);
    let was_packed = source::still_packed(at.game_dir, &root);

    if was_packed {
        unpacked(&at, &root).await?;
    }

    if !source::read_out(&root) {
        match was_packed {
            true => bail!(
                "the archive holding the data of this game could not be opened, so there is \
                 nothing in {} to read",
                root.join(source::DATA).join(source::BASIC).display()
            ),
            false => bail!(
                "no Wolf RPG data in {}. A Wolf RPG game keeps it in Data/BasicData",
                at.game_dir.display()
            ),
        }
    }

    let freed = guard::lifted(&at, &root).await?;
    if freed > 0 {
        at.progress.info(
            Source::Prepare,
            &format!("{freed} file(s) let go of their Wolf RPG Pro guard"),
        );
    }

    at.progress.stage(&STEPS, 1);
    drawn(&at).await?;

    let raw = reading::held_by(at.store, at.game_dir, &source::game_dat(&root)).await?;
    game::spelled(&raw).map_err(anyhow::Error::msg)?;

    let reached = reading::looked_up(at.store, at.game_dir, &root).await;
    let files = source::files(&root).await;
    if files.is_empty() {
        bail!("{} holds no Wolf RPG data file", root.display());
    }

    at.progress.info(Source::Prepare, label);
    at.progress.stage(&STEPS, 2);

    walk::reset(at.source).await?;

    let reached = Arc::new(reached);
    let reading = files.into_iter().map(|one| {
        let reached = Arc::clone(&reached);
        let named = one.named.clone();

        async move {
            let held = match reading::read(at.store, at.game_dir, &one).await {
                Ok(held) => held,
                Err(why) => return (named, Err(format!("{why:#}"))),
            };

            let lines = tokio::task::spawn_blocking(move || {
                harvest::sift(&held.pieces, &reached)
                    .into_iter()
                    .map(|slot| sheet::Line {
                        spot: slot.spot,
                        said: slot.said,
                        offer: slot.offer,
                    })
                    .collect::<Vec<sheet::Line>>()
            })
            .await;

            (named, lines.map_err(|why| format!("{why}")))
        }
    });

    let mut written = 0;
    let mut skipped = 0;

    for (named, held) in futures::stream::iter(reading)
        .buffered(walk::at_once())
        .collect::<Vec<_>>()
        .await
    {
        let lines = match held {
            Ok(lines) => lines,
            Err(why) => {
                skipped += 1;
                at.progress.warn(Source::Prepare, &why);
                continue;
            }
        };

        if lines.is_empty() {
            continue;
        }

        let landing = at.source.join(format!("{named}.{}", sheet::SUFFIX));
        if let Some(parent) = landing.parent() {
            fs::create_dir_all(parent)
                .await
                .with_context(|| format!("making {}", parent.display()))?;
        }

        fs::write(&landing, sheet::page(lines)?)
            .await
            .with_context(|| format!("writing {}", landing.display()))?;

        written += 1;
    }

    if written == 0 {
        bail!(
            "{} holds no text this reader could take in",
            at.game_dir.display()
        );
    }

    at.progress.info(
        Source::Prepare,
        &match skipped {
            0 => format!("{written} file(s) taken in"),
            _ => format!("{written} file(s) taken in, {skipped} could not be read"),
        },
    );

    Ok(())
}
