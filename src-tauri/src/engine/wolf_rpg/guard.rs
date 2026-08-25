use crate::engine::wolf_rpg::{database, source, unprot};
use crate::engine::{Install, Prepare};
use crate::progress::Source;
use anyhow::Result;
use std::path::{Path, PathBuf};
use tokio::fs;

const GUARDS: &str = "guards";

fn kept_for(store: &Path, root: &Path, at: &Path) -> PathBuf {
    let under = at
        .strip_prefix(root)
        .expect("a guarded file sits under the root it was read from");

    store.join(GUARDS).join(under)
}

async fn kept_yet(spot: &Path) -> bool {
    fs::metadata(spot).await.is_ok_and(|held| held.len() > 0)
}

async fn kept_at(spot: &Path, body: Vec<u8>) -> Result<()> {
    if let Some(over) = spot.parent() {
        fs::create_dir_all(over).await?;
    }

    fs::write(spot, body).await?;

    Ok(())
}

pub async fn lifted(at: &Prepare<'_>, root: &Path) -> Result<usize> {
    let basic = root.join(source::DATA).join(source::BASIC);
    let mut freed = 0;

    let mut listed = match fs::read_dir(&basic).await {
        Ok(listed) => listed,
        Err(_) => return Ok(0),
    };

    let mut found = Vec::new();
    while let Some(one) = listed.next_entry().await? {
        found.push(one.path());
    }
    found.sort();

    for one in &found {
        let named = one
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let Some(kind) = unprot::Kind::of(&named) else {
            continue;
        };

        let spot = kept_for(at.store, root, one);
        let mut body = fs::read(one).await?;

        if !unprot::wanted(&body) {
            if !kept_yet(&spot).await {
                kept_at(&spot, Vec::new()).await?;
            }

            continue;
        }

        let kept = match unprot::freed(&mut body, kind) {
            Ok(kept) => kept,
            Err(why) => {
                at.progress
                    .warn(Source::Prepare, &format!("{named}: {why}"));
                continue;
            }
        };

        kept_at(&spot, kept.packed()).await?;

        fs::write(one, &body).await?;
        freed += 1;
    }

    Ok(freed + replanned(&found).await?)
}

async fn replanned(found: &[PathBuf]) -> Result<usize> {
    let mut freed = 0;

    for one in found {
        if one.extension().and_then(|kind| kind.to_str()) != Some(database::PLAN) {
            continue;
        }

        let plan = fs::read(one).await?;
        if database::plan(&plan).is_ok() {
            continue;
        }

        let mut turned = plan;
        unprot::unplanned(&mut turned);
        if database::plan(&turned).is_err() {
            continue;
        }

        fs::write(one, &turned).await?;
        freed += 1;
    }

    Ok(freed)
}

pub async fn again(
    at: &Install<'_>,
    root: &Path,
    one: &source::File,
    body: &mut Vec<u8>,
) -> Result<()> {
    let Some(kind) = one
        .at
        .file_name()
        .and_then(|named| named.to_str())
        .and_then(unprot::Kind::of)
    else {
        return Ok(());
    };

    let spot = kept_for(at.store, root, &one.at);

    let raw = fs::read(&spot).await.map_err(|_| {
        anyhow::anyhow!(
            "nothing is kept about how {} was guarded, so writing it would leave the game \
             unable to read it. Read the game again.",
            one.named
        )
    })?;

    if raw.is_empty() {
        return Ok(());
    }

    let kept = unprot::Guard::unpacked(&raw)
        .map_err(|why| anyhow::anyhow!("the guard kept for {} is unreadable: {why}", one.named))?;

    unprot::reguarded(body, kind, &kept)
        .map_err(|why| anyhow::anyhow!("{} could not be guarded again: {why}", one.named))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::wolf_rpg::fixture::sandbox;
    use crate::progress::Quiet;
    use std::fs;

    #[test]
    fn what_is_kept_about_a_guarded_file_sits_beside_the_game_it_came_from() {
        let store = Path::new("/store/demo");
        let root = Path::new("/games/demo");

        assert_eq!(
            kept_for(store, root, &root.join("Data/BasicData/Game.dat")),
            store.join(GUARDS).join("Data/BasicData/Game.dat"),
            "the guard is looked up by where the file sits in the game, so two files of one name \
             never share what is kept about them"
        );
    }

    #[tokio::test]
    async fn reading_a_guarded_game_a_second_time_still_knows_how_to_put_the_guard_back() {
        let held = sandbox();
        let store = sandbox();
        let root = held.path();

        let basic = root.join(source::DATA).join(source::BASIC);
        fs::create_dir_all(&basic).unwrap();

        let mut shipped: Vec<u8> = (0..200usize).map(|one| (one * 7 % 251) as u8).collect();
        shipped[1] = 0x50;
        shipped[5] = 0x57;
        fs::write(basic.join("DataBase.dat"), &shipped).unwrap();

        let quiet = Quiet;
        let source_dir = store.path().join("source");
        let at = Prepare::over(root, &source_dir, store.path()).heard_by(&quiet);

        assert_eq!(lifted(&at, root).await.expect("the guard lifts"), 1);

        let spot = kept_for(store.path(), root, &basic.join("DataBase.dat"));
        let kept = fs::read(&spot).expect("what was kept");
        assert!(!kept.is_empty(), "the guard that came off has to be kept");

        assert_eq!(
            lifted(&at, root).await.expect("reading the game again"),
            0,
            "the file on disk has already let go of its guard"
        );

        assert_eq!(
            fs::read(&spot).expect("what is kept now"),
            kept,
            "reading the game again may not forget the guard, or the only copy of it is gone and \
             the game is handed data files it will not open"
        );
    }
}
