use crate::engine::Prepare;
use crate::engine::rpg_maker::js::pictures::listed;
use crate::engine::rpg_maker::js::{DATA, content_root, wanted};
use crate::engine::rpg_maker::{STEPS, pictures};
use crate::progress::Source;
use crate::walk;
use anyhow::{Result, bail};

#[tracing::instrument(name = "js.prepare", skip_all)]
pub async fn run(label: &str, at: Prepare<'_>) -> Result<()> {
    at.progress.stage(&STEPS, 0);

    let root = content_root(at.game_dir);
    if !root.join(DATA).is_dir() {
        bail!(
            "no data folder under {}. An RPG Maker game keeps its text in data/*.json",
            at.game_dir.display()
        );
    }

    at.progress.info(Source::Prepare, label);

    at.progress.stage(&STEPS, 1);

    walk::reset(at.source).await?;

    let copied = walk::copy(&root, at.source, wanted).await?;

    if copied == 0 {
        bail!("{} holds no translatable files", root.display());
    }

    at.progress
        .info(Source::Prepare, &format!("{copied} file(s) taken in"));

    at.progress.stage(&STEPS, 2);

    let shots = listed(&root).await;
    pictures::remembered(&at, &shots).await?;

    Ok(())
}
