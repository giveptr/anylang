use crate::engine::rpg_maker::rgss::pictures::listed;
use crate::engine::rpg_maker::rgss::reading::{Held, Reading};
use crate::engine::rpg_maker::rgss::source::Source;
use crate::engine::rpg_maker::{STEPS, pictures};
use crate::engine::{Prepare, sheet};
use crate::{progress, walk};
use anyhow::{Context, Result, bail};
use futures::StreamExt;
use std::sync::Arc;
use tokio::fs;

#[tracing::instrument(name = "rgss.prepare", skip_all)]
pub async fn run(label: &str, at: Prepare<'_>) -> Result<()> {
    at.progress.stage(&STEPS, 0);

    let held = Source::open(at.game_dir, at.store).await?.ok_or_else(|| {
        anyhow::anyhow!(
            "no RGSS data in {}. A VX Ace game keeps it in an .rgss3a or in Data/*.rvdata2",
            at.game_dir.display()
        )
    })?;

    at.progress.info(progress::Source::Prepare, label);
    at.progress.stage(&STEPS, 1);

    walk::reset(at.source).await?;

    let held = Arc::new(held);
    let reading = Arc::new(Reading::of(&held).await?);

    let read = held.sheets().into_iter().map(|(which, named)| {
        let held = Arc::clone(&held);
        let reading = Arc::clone(&reading);

        async move {
            let held: Result<(String, bool, Vec<sheet::Line>)> = async {
                let body = held.body(which).await?;
                let holding = reading.held(&named, &body);
                let shut = matches!(holding, Held::Locked);
                let lines = tokio::task::spawn_blocking(move || reading.lines_of(&holding, &body))
                    .await?
                    .map_err(|why| anyhow::anyhow!("{named} could not be read: {why}"))?;

                Ok((named, shut, lines))
            }
            .await;

            held
        }
    });

    let mut written = 0;
    let mut locked = 0;

    for one in futures::stream::iter(read)
        .buffered(walk::at_once())
        .collect::<Vec<_>>()
        .await
    {
        let (named, shut, lines): (String, bool, Vec<sheet::Line>) = one?;

        if shut {
            locked += 1;
            at.progress.warn(
                progress::Source::Prepare,
                &format!("{named} is locked and this reader could not open it"),
            );
        }

        if lines.is_empty() {
            continue;
        }

        let landing = at.source.join(format!("{named}.{}", sheet::SUFFIX));
        fs::write(&landing, sheet::page(lines)?)
            .await
            .with_context(|| format!("writing {}", landing.display()))?;

        written += 1;
    }

    if written == 0 {
        bail!("{} holds no sheet with text in it", at.game_dir.display());
    }

    let said = match locked {
        0 => format!("{written} sheet(s) taken in"),
        _ => format!("{written} sheet(s) taken in, {locked} left locked"),
    };
    at.progress.info(progress::Source::Prepare, &said);

    at.progress.stage(&STEPS, 2);

    let found = listed(&held, at.game_dir);
    if found.behind > 0 {
        at.progress.info(
            progress::Source::Prepare,
            &format!(
                "{} loose picture(s) under Graphics also exist inside the archive. The game only \
                 draws the packed copies, so those are the ones listed",
                found.behind
            ),
        );
    }
    pictures::remembered(&at, &found.shots).await?;

    Ok(())
}
