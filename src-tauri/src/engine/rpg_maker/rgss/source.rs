use crate::backup;
use crate::engine::rpg_maker::rgss::archive;
use crate::engine::rpg_maker::rgss::data::SUFFIX;
use anyhow::Result;
use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

const DATA: &str = "Data";
const PACKED: &str = "rgss3a";

const OLDER: [(&str, &str, &str); 2] = [
    ("rgssad", "rxdata", "RPG Maker XP"),
    ("rgss2a", "rvdata", "RPG Maker VX"),
];

pub enum Source {
    Packed {
        at: PathBuf,
        raw: Vec<u8>,
        entries: Vec<archive::Entry>,
    },
    Loose {
        each: Vec<PathBuf>,
        store: PathBuf,
        game_dir: PathBuf,
    },
}

pub fn holds_a_game(game_dir: &Path) -> bool {
    packed_at(game_dir).is_some() || !loose_in(&game_dir.join(DATA)).is_empty()
}

impl Source {
    pub async fn open(game_dir: &Path, store: &Path) -> Result<Option<Self>> {
        if let Some(at) = packed_at(game_dir) {
            let raw = backup::original(store, game_dir, &at).await?;
            let entries = archive::entries(&raw)
                .map_err(|why| anyhow::anyhow!("{} is not an archive: {why}", at.display()))?;

            return Ok(Some(Self::Packed { at, raw, entries }));
        }

        let each = loose_in(&game_dir.join(DATA));

        Ok((!each.is_empty()).then_some(Self::Loose {
            each,
            store: store.to_path_buf(),
            game_dir: game_dir.to_path_buf(),
        }))
    }

    pub fn sheets(&self) -> Vec<(usize, String)> {
        match self {
            Self::Packed { entries, .. } => entries
                .iter()
                .enumerate()
                .filter(|(_, one)| {
                    one.name.starts_with("Data/")
                        && Path::new(&one.name)
                            .extension()
                            .is_some_and(|kind| kind == SUFFIX)
                })
                .map(|(which, one)| (which, named(&one.name)))
                .collect(),
            Self::Loose { each, .. } => each
                .iter()
                .enumerate()
                .map(|(which, at)| (which, named(&at.to_string_lossy())))
                .collect(),
        }
    }

    pub fn known(&self) -> HashSet<String> {
        match self {
            Self::Packed { entries, .. } => entries.iter().map(|one| named(&one.name)).collect(),
            Self::Loose { each, .. } => {
                each.iter().map(|at| named(&at.to_string_lossy())).collect()
            }
        }
    }

    pub async fn body(&self, which: usize) -> Result<Vec<u8>> {
        match self {
            Self::Packed { raw, entries, .. } => Ok(archive::body(raw, &entries[which])),
            Self::Loose {
                each,
                store,
                game_dir,
            } => backup::original(store, game_dir, &each[which]).await,
        }
    }

    pub async fn write(
        &self,
        store: &Path,
        game_dir: &Path,
        edits: Vec<(usize, Vec<u8>)>,
    ) -> Result<()> {
        match self {
            Self::Packed { at, raw, entries } => {
                let carried: Vec<(&archive::Entry, Vec<u8>)> = edits
                    .into_iter()
                    .map(|(which, body)| (&entries[which], body))
                    .collect();

                let patched = archive::patched(raw, &carried).map_err(|why| {
                    anyhow::anyhow!("{} could not be packed: {why}", at.display())
                })?;

                backup::replace(store, game_dir, at, patched).await
            }
            Self::Loose { each, .. } => {
                let mut written: HashSet<usize> = HashSet::new();

                for (which, body) in edits {
                    backup::replace(store, game_dir, &each[which], body).await?;
                    written.insert(which);
                }

                for (which, at) in each.iter().enumerate() {
                    if !written.contains(&which) {
                        backup::put_back(store, game_dir, at).await?;
                    }
                }

                Ok(())
            }
        }
    }

    pub async fn put_back(&self, store: &Path, game_dir: &Path) -> Result<usize> {
        let files: Vec<&PathBuf> = match self {
            Self::Packed { at, .. } => vec![at],
            Self::Loose { each, .. } => each.iter().collect(),
        };

        let mut given = 0;
        for one in files {
            if backup::put_back(store, game_dir, one).await? {
                given += 1;
            }
        }

        Ok(given)
    }
}

pub fn named(at: &str) -> String {
    Path::new(at)
        .file_stem()
        .and_then(OsStr::to_str)
        .unwrap_or_default()
        .to_string()
}

pub fn packed_at(game_dir: &Path) -> Option<PathBuf> {
    ending_in(game_dir, PACKED)
}

fn ending_in(folder: &Path, kind: &str) -> Option<PathBuf> {
    fs::read_dir(folder)
        .ok()?
        .filter_map(Result::ok)
        .map(|one| one.path())
        .find(|at| {
            at.extension()
                .is_some_and(|held| held.eq_ignore_ascii_case(kind))
        })
}

pub fn older_at(game_dir: &Path) -> Option<(PathBuf, &'static str)> {
    OLDER.into_iter().find_map(|(packed, loose, named)| {
        ending_in(game_dir, packed)
            .or_else(|| ending_in(&game_dir.join(DATA), loose))
            .map(|at| (at, named))
    })
}

fn loose_in(data: &Path) -> Vec<PathBuf> {
    let Ok(found) = fs::read_dir(data) else {
        return Vec::new();
    };

    let mut each: Vec<PathBuf> = found
        .filter_map(Result::ok)
        .map(|one| one.path())
        .filter(|at| at.extension().is_some_and(|kind| kind == SUFFIX))
        .collect();
    each.sort();

    each
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::rpg_maker::rgss::fixture::sandbox;

    #[tokio::test]
    async fn a_game_is_found_however_it_was_packed() {
        let at = sandbox();
        let root = at.path();

        let store = sandbox();
        let store = store.path();

        assert!(
            Source::open(root, store).await.expect("no game").is_none(),
            "an empty folder holds no rgss game"
        );

        fs::create_dir_all(root.join(DATA)).unwrap();
        fs::write(root.join(DATA).join("Map001.rvdata2"), [4, 8, b'0']).unwrap();
        fs::write(root.join(DATA).join("readme.txt"), "hello").unwrap();

        let found = Source::open(root, store)
            .await
            .expect("a folder of sheets")
            .expect("a game unpacked beside the player");

        assert_eq!(
            found.sheets(),
            vec![(0, "Map001".to_string())],
            "an unpacked game is still a game, whatever runs it"
        );
        assert_eq!(found.body(0).await.expect("its bytes"), [4, 8, b'0']);

        fs::write(
            root.join("Game.rgss3a"),
            archive::packed(&[("Data\\Map002.rvdata2", &[4, 8, b'T'])]),
        )
        .unwrap();

        let found = Source::open(root, store)
            .await
            .expect("an archive")
            .expect("the packed game wins");

        assert_eq!(found.sheets(), vec![(0, "Map002".to_string())]);
    }

    #[tokio::test]
    async fn a_loose_sheet_nobody_translated_this_time_goes_back_to_the_words_it_shipped() {
        let at = sandbox();
        let root = at.path();
        let store = sandbox();
        let store = store.path();

        fs::create_dir_all(root.join(DATA)).unwrap();
        for name in ["Map001.rvdata2", "Map002.rvdata2"] {
            fs::write(root.join(DATA).join(name), [4, 8, b'0']).unwrap();
        }

        let held = Source::open(root, store)
            .await
            .expect("a folder of sheets")
            .expect("a game");

        held.write(store, root, vec![(0, vec![4, 8, b'T'])])
            .await
            .expect("the first export");
        held.write(store, root, vec![(1, vec![4, 8, b'T'])])
            .await
            .expect("an export that no longer holds the first sheet");

        assert_eq!(
            fs::read(root.join(DATA).join("Map001.rvdata2")).unwrap(),
            [4, 8, b'0'],
            "a sheet left out of this export may not keep the last one's translation"
        );
        assert_eq!(
            fs::read(root.join(DATA).join("Map002.rvdata2")).unwrap(),
            [4, 8, b'T']
        );
    }
}
