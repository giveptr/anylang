pub mod batch;
pub mod prompt;
pub mod repair;

use crate::cancel::{Cancel, Tokens};
use crate::engine::{Engine, TranslationUnit};
use crate::llm::{CallError, Model, Usage};
use crate::progress::{Batch, FileOutcome, Progress, Source};
use crate::scope::key;
use crate::store::Store;
use crate::tuning::Tuning;
use anyhow::{Context, Result};
use futures::stream::{self, StreamExt};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::fs;

struct Work {
    path: PathBuf,
    label: String,
    units: Vec<TranslationUnit>,
    before: u32,
    left: AtomicUsize,
    cancel: Arc<Cancel>,
}

struct Piece {
    work: Arc<Work>,
    unit: TranslationUnit,
}

type PerFile = BTreeMap<String, (Arc<Work>, BTreeMap<u32, String>)>;

pub struct Job {
    plan: Plan,
    spent: Mutex<Usage>,
    already: AtomicUsize,
    wordless: AtomicUsize,
}

pub trait Marking: Send + Sync {
    fn filled(&self, file: &Path, ids: &[u32]);
}

pub struct Plan {
    pub engine: Arc<dyn Engine>,
    pub model: Box<dyn Model>,
    pub store: Store,
    pub source: PathBuf,
    pub tuning: Tuning,
    pub system: String,
    pub progress: Arc<dyn Progress>,
    pub marking: Arc<dyn Marking>,
    pub tokens: Arc<Tokens>,
}

impl Job {
    pub fn new(plan: Plan) -> Self {
        Self {
            plan,
            spent: Mutex::new(Usage::default()),
            already: AtomicUsize::new(0),
            wordless: AtomicUsize::new(0),
        }
    }

    pub async fn run(self: Arc<Self>, files: Vec<PathBuf>) {
        let started = Instant::now();

        self.plan.progress.info(
            Source::Translate,
            &format!("{} file(s) queued", files.len()),
        );

        stream::iter(files)
            .then(|path| {
                let job = self.clone();
                async move { job.open(path).await }
            })
            .flat_map(stream::iter)
            .chunks(self.plan.tuning.lines_per_request)
            .for_each_concurrent(self.plan.tuning.parallel_requests, |chunk| {
                let job = self.clone();
                async move { job.fill(chunk).await }
            })
            .await;

        let already = self.already.load(Ordering::Relaxed);
        let wordless = self.wordless.load(Ordering::Relaxed);
        let mut skipped = Vec::new();
        if already > 0 {
            skipped.push(format!("{already} file(s) already done"));
        }
        if wordless > 0 {
            skipped.push(format!("{wordless} file(s) with no text"));
        }
        if !skipped.is_empty() {
            self.plan
                .progress
                .info(Source::Translate, &skipped.join(", "));
        }

        let spent = *self.spent.lock().expect("usage lock");
        if spent.total() > 0 {
            self.plan.progress.info(
                Source::Translate,
                &format!("{} in {}", spent.told(), elapsed(started.elapsed())),
            );
        }

        if self.plan.tokens.stopped() {
            self.plan
                .progress
                .warn(Source::Translate, "stopped: progress so far is saved");
        }
    }

    async fn open(&self, path: PathBuf) -> Vec<Piece> {
        if self.plan.tokens.stopped() {
            return Vec::new();
        }

        let label = key(&self.plan.source, &path);

        let text = match read(&path).await {
            Ok(text) => text,
            Err(error) => {
                self.plan
                    .progress
                    .failed(Source::Translate, &error.context(label.clone()));
                return Vec::new();
            }
        };

        let engine = Arc::clone(&self.plan.engine);
        let here = path.clone();
        let units =
            match tokio::task::spawn_blocking(move || engine.parse(&here, &text).units().to_vec())
                .await
            {
                Ok(units) => units,
                Err(error) => {
                    self.plan.progress.failed(
                        Source::Translate,
                        &anyhow::anyhow!("{error}").context(label),
                    );
                    return Vec::new();
                }
            };
        if units.is_empty() {
            self.wordless.fetch_add(1, Ordering::Relaxed);
            return Vec::new();
        }

        let state = match self.plan.store.load(&path, &units).await {
            Ok(state) => state,
            Err(error) => {
                self.plan
                    .progress
                    .failed(Source::Translate, &error.context(label.clone()));
                return Vec::new();
            }
        };
        let pending: Vec<TranslationUnit> = state
            .missing(&units)
            .into_iter()
            .filter(|unit| unit.offer.asked())
            .collect();
        if pending.is_empty() {
            self.already.fetch_add(1, Ordering::Relaxed);
            return Vec::new();
        }

        self.plan.progress.file_started(&label);

        let work = Arc::new(Work {
            cancel: self.plan.tokens.enlist(&label),
            path,
            label,
            before: state.said().len() as u32,
            left: AtomicUsize::new(pending.len()),
            units,
        });

        pending
            .into_iter()
            .map(|unit| Piece {
                work: work.clone(),
                unit,
            })
            .collect()
    }

    async fn fill(&self, chunk: Vec<Piece>) {
        let (going, stopped): (Vec<Piece>, Vec<Piece>) = chunk
            .into_iter()
            .partition(|piece| !piece.work.cancel.stopped());

        for piece in stopped {
            self.settle(&piece.work).await;
        }

        if going.is_empty() {
            return;
        }

        let units: Vec<TranslationUnit> = going.iter().map(|piece| piece.unit.clone()).collect();

        let mut byfile: PerFile = BTreeMap::new();
        for piece in &going {
            byfile
                .entry(piece.work.label.clone())
                .or_insert_with(|| (piece.work.clone(), BTreeMap::new()));
        }

        let halt = Arc::new(Cancel::default());
        let watching = self.watch(&halt, byfile.values().map(|(work, _)| work.cancel.clone()));

        let (outcome, spent) = batch::translate(
            self.plan.model.as_ref(),
            self.plan.engine.as_ref(),
            &self.plan.system,
            units,
            &self.plan.tuning,
            &halt,
            self.plan.progress.as_ref(),
        )
        .await;

        watching.abort();

        self.spent.lock().expect("usage lock").add(spent);

        match outcome {
            Ok(outcome) => self.answered(&going, byfile, outcome).await,
            Err(error) => self.stumbled(&byfile, &error),
        }

        for piece in &going {
            self.settle(&piece.work).await;
        }
    }

    async fn answered(&self, going: &[Piece], mut byfile: PerFile, outcome: batch::BatchOutcome) {
        for (which, translation) in outcome.translations {
            let Some(piece) = going.get(which as usize) else {
                continue;
            };

            if let Some((_, mine)) = byfile.get_mut(&piece.work.label) {
                mine.insert(piece.unit.id, translation);
            }
        }

        for (work, mine) in byfile.into_values() {
            self.merge(&work, mine).await;
        }

        for note in retold(&outcome.notes) {
            self.plan.progress.warn(Source::Translate, &note);
        }

        if outcome.skipped > 0 {
            self.plan.progress.warn(
                Source::Translate,
                &format!("{} line(s) left untranslated", outcome.skipped),
            );
        }

        if let Some(error) = outcome.fatal {
            if !self.plan.tokens.stopped() {
                self.plan
                    .progress
                    .error(Source::Translate, &error.to_string());
            }

            if matches!(error, CallError::Fatal(_)) {
                self.plan.tokens.whole().stop();
            }
        }
    }

    fn stumbled(&self, byfile: &PerFile, error: &CallError) {
        for (work, _) in byfile.values() {
            if self.plan.tokens.stopped() || work.cancel.stopped() {
                continue;
            }

            self.plan
                .progress
                .error(Source::Translate, &format!("{}: {error}", work.label));
        }

        if matches!(error, CallError::Fatal(_)) {
            self.plan.tokens.whole().stop();
        }
    }

    fn watch(
        &self,
        halt: &Arc<Cancel>,
        each: impl Iterator<Item = Arc<Cancel>>,
    ) -> tokio::task::JoinHandle<()> {
        let halt = halt.clone();
        let whole = self.plan.tokens.whole();
        let each: Vec<Arc<Cancel>> = each.collect();

        tokio::spawn(async move {
            tokio::select! {
                () = whole.cancelled() => {}
                () = async {
                    for one in &each {
                        one.cancelled().await;
                    }
                } => {}
            }

            halt.stop();
        })
    }

    async fn merge(&self, work: &Arc<Work>, mine: BTreeMap<u32, String>) {
        if mine.is_empty() {
            return;
        }

        let wrote: Vec<u32> = mine.keys().copied().collect();

        let kept = self
            .plan
            .store
            .amend(&work.path, &work.units, |state| {
                let before = state.said().len();
                state.answered(mine);

                (state.said().len(), state.said().len() - before)
            })
            .await;

        match kept {
            Ok((filled, added)) => {
                self.plan.marking.filled(&work.path, &wrote);

                self.plan.progress.batch_done(Batch {
                    file: &work.label,
                    filled,
                    added,
                });
            }
            Err(error) => self
                .plan
                .progress
                .failed(Source::Translate, &error.context(work.label.clone())),
        }
    }

    async fn settle(&self, work: &Arc<Work>) {
        if work.left.fetch_sub(1, Ordering::Relaxed) == 1 {
            self.close(work).await;
        }
    }

    async fn close(&self, work: &Work) {
        self.plan.tokens.retire(&work.label);

        let state = match self.plan.store.load(&work.path, &work.units).await {
            Ok(state) => state,
            Err(error) => {
                self.plan
                    .progress
                    .failed(Source::Translate, &error.context(work.label.clone()));
                return;
            }
        };
        let filled = state.said().len() as u32;
        let total = work
            .units
            .iter()
            .filter(|unit| state.said().contains_key(&unit.id) || unit.offer.asked())
            .count() as u32;

        let outcome = if filled >= total {
            FileOutcome::Completed {
                lines: filled.saturating_sub(work.before),
            }
        } else {
            FileOutcome::Partial {
                done: filled,
                total,
            }
        };

        self.plan.progress.file_done(&work.label, outcome);
    }
}

async fn read(path: &Path) -> Result<String> {
    fs::read_to_string(path)
        .await
        .with_context(|| format!("reading {}", path.display()))
}

fn elapsed(taken: Duration) -> String {
    let seconds = taken.as_secs();
    let (hours, minutes, rest) = (seconds / 3600, (seconds % 3600) / 60, seconds % 60);

    if hours > 0 {
        format!("{hours}h {minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m {rest}s")
    } else {
        format!("{rest}s")
    }
}

fn retold(notes: &[String]) -> Vec<String> {
    let mut counted: Vec<(&String, usize)> = Vec::new();

    for note in notes {
        match counted.iter_mut().find(|(said, _)| *said == note) {
            Some((_, seen)) => *seen += 1,
            None => counted.push((note, 1)),
        }
    }

    counted
        .into_iter()
        .map(|(note, seen)| match seen {
            1 => note.clone(),
            _ => format!("{note} ({seen} times)"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_note_said_over_and_over_is_told_once_with_its_count() {
        let said = |one: &str| one.to_string();
        let notes = [
            said("JSON needed repair (Repaired)"),
            said("rejected: too long"),
            said("JSON needed repair (Repaired)"),
            said("JSON needed repair (Repaired)"),
        ];

        assert_eq!(
            retold(&notes),
            [
                "JSON needed repair (Repaired) (3 times)",
                "rejected: too long"
            ],
            "a batch split in half repairs the same way at every level of the tree, and a log \
             that repeats the line once per half buries the notes that differ"
        );

        assert_eq!(
            retold(&[said("one of a kind")]),
            ["one of a kind"],
            "a note said once needs no count after it"
        );
    }
}
