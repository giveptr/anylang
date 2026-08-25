use crate::engine::rpg_maker::rgss::reading::Reading;
use crate::engine::rpg_maker::rgss::source::Source;
use crate::engine::rpg_maker::rgss::{fonts, pictures, scripts, settings, source};
use crate::engine::{Install, sheet};
use crate::walk;
use anyhow::Result;
use futures::StreamExt;
use futures::future::BoxFuture;
use std::sync::Arc;
use tracing::Instrument;

pub fn run(at: Install<'_>) -> BoxFuture<'_, Result<()>> {
    Box::pin(
        async move {
            let Some(held) = Source::open(at.game_dir, at.store).await? else {
                return Ok(());
            };

            let reading = Reading::of(&held).await?;
            let sheets = held.sheets();
            let said = sheet::staged(
                &at,
                |one| sheet::wants(one).then(|| source::named(&one.to_string_lossy())),
                |named| sheets.iter().any(|(_, sheet)| sheet == named),
            )
            .await?;

            let sending = fonts::sending(&at).await?;

            fonts::tidied(&at).await;

            let pictures = pictures::written(&held, &at).await?;
            let held = Arc::new(held);
            let reading = Arc::new(reading);
            let sending = Arc::new(sending);

            let splicing = sheets.clone().into_iter().filter_map(|(which, named)| {
                let lines = said.get(&named)?.clone();
                let held = Arc::clone(&held);
                let reading = Arc::clone(&reading);
                Some(async move {
                    let held: Result<(usize, Vec<u8>, u32)> = async {
                        let body = held.body(which).await?;
                        let (fresh, written) = tokio::task::spawn_blocking(move || {
                            reading.spliced(&reading.held(&named, &body), &body, &lines)
                        })
                        .await?
                        .map_err(|why| anyhow::anyhow!("a sheet could not be written: {why}"))?;

                        Ok((which, fresh, written))
                    }
                    .await;

                    held
                })
            });

            let mut edits: Vec<(usize, Vec<u8>)> = Vec::new();

            for one in futures::stream::iter(splicing)
                .buffered(walk::at_once())
                .collect::<Vec<_>>()
                .await
            {
                let (which, fresh, written) = one?;
                if written > 0 {
                    edits.push((which, fresh));
                }
            }

            if sending.wanted()
                && let Some((which, named)) = sheets.iter().find(|(_, one)| one == scripts::NAME)
            {
                let staged = edits.iter().position(|(at, _)| at == which);
                let body = match staged {
                    Some(held) => edits.remove(held).1,
                    None => held.body(*which).await?,
                };

                let (spoken, count) = fonts::told(&body, &sending)
                    .map_err(|why| anyhow::anyhow!("{named} could not be told a font: {why}"))?;

                if count > 0 || staged.is_some() {
                    edits.push((*which, spoken));
                }
            }

            let sheets_in = edits.len();

            if !at.reverting
                && let Some((which, _)) = sheets.iter().find(|(_, one)| one == settings::NAME)
            {
                match edits.iter_mut().find(|(at, _)| at == which) {
                    Some(staged) => {
                        if let Some(fresh) = settings::typing_in_latin(&staged.1) {
                            staged.1 = fresh;
                        }
                    }
                    None => {
                        let body = held.body(*which).await?;
                        if let Some(fresh) = settings::typing_in_latin(&body) {
                            edits.push((*which, fresh));
                        }
                    }
                }
            }

            edits.extend(pictures);

            if edits.is_empty() {
                let given = held.put_back(at.store, at.game_dir).await?;
                if given > 0 {
                    at.progress
                        .info(at.doing, &format!("{given} file(s) put back"));
                }

                return Ok(());
            }

            if sheets_in > 0 {
                at.progress
                    .info(at.doing, &format!("{sheets_in} file(s) written in"));
            }

            held.write(at.store, at.game_dir, edits).await
        }
        .instrument(tracing::info_span!("rgss.install")),
    )
}
