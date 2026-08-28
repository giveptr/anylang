use crate::engine::wolf_rpg::held::Held;
use crate::engine::wolf_rpg::reached::Reached;
use crate::engine::wolf_rpg::source::{File, Which};
use crate::engine::wolf_rpg::{
    archive, common, database, game, guard, harvest, map, script, source,
};
use crate::{backup, walk};
use anyhow::{Context, Result};
use std::borrow::Cow;
use std::path::Path;

async fn shipped(store: &Path, game_dir: &Path, at: &Path) -> Result<Vec<u8>> {
    match at.starts_with(game_dir) {
        true => backup::original(store, game_dir, at).await,
        false => tokio::fs::read(at)
            .await
            .with_context(|| format!("reading {}", at.display())),
    }
}

fn shipped_now(store: &Path, game_dir: &Path, at: &Path) -> Option<Vec<u8>> {
    match at.starts_with(game_dir) {
        true => backup::original_now(store, game_dir, at),
        false => std::fs::read(at).ok(),
    }
}

fn called(at: &Path) -> Cow<'_, str> {
    at.file_name().unwrap_or_default().to_string_lossy()
}

pub fn game_now(store: &Path, game_dir: &Path, root: &Path) -> Option<Held> {
    let raw = opened_now(store, game_dir, &source::game_dat(root))?;

    game::read(&raw).ok()
}

pub async fn sealed(
    store: &Path,
    game_dir: &Path,
    at: &Path,
    mut body: Vec<u8>,
) -> Result<Vec<u8>> {
    if !guard::wraps(at) {
        return Ok(body);
    }

    let raw = shipped(store, game_dir, at).await?;

    guard::sealed(&raw, at, &mut body).map_err(|why| anyhow::anyhow!("{}: {why}", called(at)))?;

    Ok(body)
}

pub async fn opened(store: &Path, game_dir: &Path, at: &Path) -> Result<Vec<u8>> {
    let raw = shipped(store, game_dir, at).await?;

    guard::opened(raw, at).map_err(|why| anyhow::anyhow!("{}: {why}", called(at)))
}

pub fn opened_now(store: &Path, game_dir: &Path, at: &Path) -> Option<Vec<u8>> {
    guard::opened(shipped_now(store, game_dir, at)?, at).ok()
}

pub async fn read(store: &Path, game_dir: &Path, one: &File) -> Result<Held> {
    let raw = opened(store, game_dir, &one.at).await?;

    let held = match &one.which {
        Which::Map => map::read(&raw),
        Which::Script => script::read(&raw),
        Which::Common => common::read(&raw),
        Which::Game => game::read(&raw),
        Which::Database { plan } => paired(&raw, &opened(store, game_dir, plan).await?),
    };

    held.map_err(|why| anyhow::anyhow!("{} could not be read: {why}", one.named))
}

fn paired(raw: &[u8], drawn: &[u8]) -> Result<Held, String> {
    let plan = database::plan(drawn).map_err(|why| format!("its plan could not be read: {why}"))?;

    database::read(raw, &plan)
}

fn parted(stem: &str, out: &mut Reached) {
    let mut at = 0;

    while let Some(step) = stem[at..].find(['_', '-']) {
        at += step + 1;
        out.ships(&stem[at..]);
    }
}

fn stems(game_dir: &Path, root: &Path, out: &mut Reached) {
    let weighed = source::weight(game_dir);

    for one in source::archives(game_dir) {
        let Ok(Some(key)) = archive::key_for(&one, weighed) else {
            continue;
        };
        let Ok(named) = archive::named_inside(&one, &key) else {
            continue;
        };

        for held in named {
            if let Some(stem) = Path::new(&held).file_stem().and_then(|one| one.to_str()) {
                parted(stem, out);
            }
        }
    }

    for at in walk::files_now(root) {
        if let Some(stem) = at.file_stem().and_then(|one| one.to_str()) {
            parted(stem, out);
        }
    }
}

pub async fn looked_up(store: &Path, game_dir: &Path, root: &Path) -> Reached {
    let here = game_dir.to_path_buf();
    let over = root.to_path_buf();

    let mut out = tokio::task::spawn_blocking(move || {
        let mut out = Reached::new();
        stems(&here, &over, &mut out);

        out
    })
    .await
    .expect("looking over what the game ships");

    for one in source::files(root).await {
        if let Ok(held) = read(store, game_dir, &one).await {
            if one.which == Which::Script {
                script::taken_apart(&held.plain, &mut out);
            }

            harvest::found_by(&held.pieces, &mut out);
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::wolf_rpg::fixture::{self, sandbox};
    use crate::engine::wolf_rpg::harvest;

    #[test]
    fn the_name_of_a_picture_beside_the_game_gives_up_the_key_a_box_is_headed_by() {
        let mut out = Reached::new();
        parted("chara_hu-n1", &mut out);

        assert!(
            out.builds("hu-n"),
            "the game glues a number onto this row to pick which of its portraits to draw, and \
             the only way to know that is to have seen a file called hu-n1 ship beside it"
        );
        assert!(
            !out.builds("chara_hu-n"),
            "a stem is never a key for the whole of itself, or the first row of every box that \
             happens to name a file would be held out of the translation"
        );
        assert!(
            !out.builds("hu-x"),
            "a row that no shipped picture is named after is a row a player reads"
        );
    }

    #[tokio::test]
    async fn every_kind_of_file_a_wolf_game_ships_is_read_by_the_same_call() {
        let at = sandbox();
        let root = at.path();
        let store = sandbox();
        fixture::lay_out(root);

        let mut said: Vec<String> = Vec::new();
        for one in source::files(root).await {
            let held = read(store.path(), root, &one)
                .await
                .unwrap_or_else(|why| panic!("{} should read: {why}", one.named));

            said.extend(
                harvest::sift(&held.pieces, &Default::default())
                    .into_iter()
                    .map(|line| line.said),
            );
        }
        said.sort();

        assert_eq!(
            said,
            [
                " + DLC",
                "A tale of two",
                "HP\u{3092}30\u{56de}\u{5fa9}",
                "Press any key",
                "\u{3044}\u{3044}\u{3048}",
                "\u{306f}\u{3044}",
                "\u{6249}\u{306f}\u{9589}\u{307e}\u{3063}\u{3066}\u{3044}\u{308b}",
                "\u{7dd1}\u{8336}",
                "\u{9060}\u{3044}\u{9053}",
            ],
            "a map, the common events, both halves of a database and Game.dat all give up their \
             text"
        );
    }
}
