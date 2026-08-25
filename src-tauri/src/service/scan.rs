use crate::engine::{Engine, TranslationUnit};
use crate::scope::Scope;
use crate::store::{FileState, Stamp, Store};
use crate::{store, walk};
use anyhow::{Context, Result};
use futures::stream::{self, Stream};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::VecDeque;
use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use tokio::sync::mpsc::{self, Receiver};

const AHEAD: usize = 16;
const KEPT: usize = 4;

struct Read {
    at: PathBuf,
    stamp: Stamp,
    units: Arc<Vec<TranslationUnit>>,
}

static LAST_READ: LazyLock<Mutex<VecDeque<Read>>> = LazyLock::new(|| Mutex::new(VecDeque::new()));

async fn stamp_of(file: &Path) -> Stamp {
    store::stamped(tokio::fs::metadata(file).await)
}

fn read_before(file: &Path, stamp: &Stamp) -> Option<Arc<Vec<TranslationUnit>>> {
    let kept = LAST_READ.lock().expect("parsed lock");

    kept.iter()
        .find(|one| one.at == file && &one.stamp == stamp)
        .map(|one| Arc::clone(&one.units))
}

fn keep_read(file: &Path, stamp: Stamp, units: &Arc<Vec<TranslationUnit>>) {
    let mut kept = LAST_READ.lock().expect("parsed lock");

    kept.retain(|one| one.at != file);
    kept.push_back(Read {
        at: file.to_path_buf(),
        stamp,
        units: Arc::clone(units),
    });

    while kept.len() > KEPT {
        kept.pop_front();
    }
}

pub struct Scanned {
    pub path: PathBuf,
    pub units: Vec<TranslationUnit>,
    pub state: Result<FileState>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    pub total: u32,
    pub translated: u32,
    pub untranslated: u32,
}

impl Scanned {
    pub fn worth(&self) -> Counts {
        let mut counts = Counts::default();

        for unit in &self.units {
            let done = self
                .state
                .as_ref()
                .is_ok_and(|state| state.said().contains_key(&unit.id));

            if !unit.offer.unlocked() {
                continue;
            }

            if !done && !unit.offer.asked() {
                continue;
            }

            counts.total += 1;
            if done {
                counts.translated += 1;
            }
        }

        counts.untranslated = counts.total - counts.translated;

        counts
    }
}

pub fn scan_files<'a>(
    source: &Path,
    engine: Arc<dyn Engine>,
    store: &'a Store,
) -> impl Stream<Item = Scanned> + 'a {
    stream::unfold(
        (parse_all(source, engine), store),
        |(mut parsed, store)| async move {
            let (path, units) = parsed.recv().await?;

            let scanned = match units {
                Ok(units) => {
                    let state = store.load(&path, &units).await;
                    Scanned { path, units, state }
                }
                Err(error) => Scanned {
                    state: Err(error).with_context(|| format!("reading {}", path.display())),
                    units: Vec::new(),
                    path,
                },
            };

            Some((scanned, (parsed, store)))
        },
    )
}

pub async fn parse_file(
    engine: &Arc<dyn Engine>,
    file: &Path,
) -> Result<Arc<Vec<TranslationUnit>>> {
    let stamp = stamp_of(file).await;
    if let Some(found) = read_before(file, &stamp) {
        return Ok(found);
    }

    let text = tokio::fs::read_to_string(file)
        .await
        .with_context(|| format!("reading {}", file.display()))?;

    let engine = Arc::clone(engine);

    let here = file.to_path_buf();
    let units = Arc::new(
        tokio::task::spawn_blocking(move || engine.parse(&here, &text).units().to_vec()).await?,
    );
    keep_read(file, stamp, &units);

    Ok(units)
}

pub fn unlocked_in(units: &[TranslationUnit], scope: &Scope, id: u32) -> Result<TranslationUnit> {
    let held = units
        .iter()
        .find(|unit| unit.id == id)
        .cloned()
        .with_context(|| format!("line {id} is no longer in {scope}"))?;

    match held.offer.unlocked() {
        true => Ok(held),
        false => Err(anyhow::anyhow!(
            "line {id} in {scope} is one the game's own format spells out as code, so nothing \
             written onto it would reach the game"
        )),
    }
}

fn parse_all(
    source: &Path,
    engine: Arc<dyn Engine>,
) -> Receiver<(PathBuf, Result<Vec<TranslationUnit>, io::Error>)> {
    let root = source.to_path_buf();
    let (send, recv) = mpsc::channel(AHEAD);

    tokio::task::spawn_blocking(move || {
        let mut files: Vec<PathBuf> = walk::files_now(&root)
            .into_iter()
            .filter(|path| engine.wants(path))
            .collect();
        files.sort();

        for path in files {
            let sent = match fs::read_to_string(&path) {
                Ok(text) => {
                    let units = engine.parse(&path, &text).units().to_vec();
                    if units.is_empty() {
                        continue;
                    }

                    (path, Ok(units))
                }
                Err(error) if error.kind() == ErrorKind::NotFound => continue,
                Err(error) => (path, Err(error)),
            };

            if send.blocking_send(sent).is_err() {
                return;
            }
        }
    });

    recv
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Offer;

    fn scanned(lines: &[(u32, &str, Option<&str>)], symbolic: impl Fn(&str) -> bool) -> Scanned {
        let mut state = FileState::default();
        for (id, _, said) in lines {
            if let Some(said) = said {
                state.settled(*id, (*said).to_string());
            }
        }

        Scanned {
            path: PathBuf::from("a.sheet"),
            units: lines
                .iter()
                .map(|(id, text, _)| TranslationUnit {
                    id: *id,
                    offer: Offer::default().or_listed(symbolic(text)),
                    text: (*text).to_string(),
                })
                .collect(),
            state: Ok(state),
        }
    }

    #[test]
    fn the_only_way_a_command_reaches_a_line_by_id_refuses_the_ones_the_format_locks() {
        let units: Vec<TranslationUnit> = [Offer::Asked, Offer::Listed, Offer::Locked]
            .into_iter()
            .enumerate()
            .map(|(id, offer)| TranslationUnit {
                id: id as u32,
                offer,
                text: "const x = 20;".to_string(),
            })
            .collect();

        let scope = Scope::read("plugins.js").expect("a scope");

        assert!(unlocked_in(&units, &scope, 0).is_ok());
        assert!(
            unlocked_in(&units, &scope, 1).is_ok(),
            "a rule only guessed this one is not text, and settling it by hand is the reader's to \
             do"
        );
        assert!(
            unlocked_in(&units, &scope, 2).is_err(),
            "saving onto a locked line reports it done and then loses it, and asking the model \
             for one is billed for an answer that can never be written in: both commands reach a \
             line only through here, so here is where the format's no is kept"
        );
    }

    #[tokio::test]
    async fn a_file_already_read_is_not_read_again_until_it_changes() {
        let at = tempfile::tempdir().expect("a temp folder");
        let file = at.path().join("Scenario.json");
        fs::write(&file, "{}").unwrap();

        let units = Arc::new(vec![TranslationUnit {
            id: 0,
            text: "The lamp went out before he answered.".to_string(),
            offer: Default::default(),
        }]);
        let stamp = stamp_of(&file).await;
        keep_read(&file, stamp, &units);

        let again = read_before(&file, &stamp_of(&file).await).expect("the same file");
        assert!(
            Arc::ptr_eq(&units, &again),
            "the editor asks for a window and a save on the same file, and a 10 MB sheet costs \
             most of a second to read"
        );

        fs::write(&file, "{\"one\": []}").unwrap();
        assert!(
            read_before(&file, &stamp_of(&file).await).is_none(),
            "preparing the game again rewrites the sheet, and the old lines would be stale"
        );
    }

    #[test]
    fn a_line_nobody_will_be_asked_to_translate_is_not_counted_as_work_left() {
        let found = scanned(
            &[
                (0, "Wait for me.", None),
                (1, "ui/bg_town.png", None),
                (2, "rotate:90", None),
            ],
            |text| text.contains('/') || text.contains(':'),
        );

        let worth = found.worth();

        assert_eq!(
            worth.total, 1,
            "two of these three will never be sent, so counting them leaves the bar short of \
             full forever"
        );
        assert_eq!(worth.translated, 0);
    }

    #[test]
    fn a_line_someone_translated_by_hand_counts_however_it_reads() {
        let found = scanned(
            &[
                (0, "Wait for me.", Some("\u{5f85}\u{3063}\u{3066}\u{3002}")),
                (1, "ui/bg_town.png", Some("ui/bg_ville.png")),
                (2, "rotate:90", None),
            ],
            |text| text.contains('/') || text.contains(':'),
        );

        let worth = found.worth();

        assert_eq!(
            worth.total, 2,
            "the reader settled the asset path by hand, so it is work done and not work skipped"
        );
        assert_eq!(
            worth.translated, 2,
            "export writes what the store holds, so a count that ignored it would hide the \
             reader's own work"
        );
    }
}
