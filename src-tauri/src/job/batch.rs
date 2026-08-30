use crate::cancel::Cancel;
use crate::engine::{Engine, TranslationUnit};
use crate::job::prompt;
use crate::job::repair::{self, ParseQuality};
use crate::llm::{CallError, Model, Request, Usage, generate_with_retry};
use crate::progress::Progress;
use crate::tuning::Tuning;
use futures::future::{BoxFuture, FutureExt};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

#[derive(Debug, Default)]
pub struct BatchOutcome {
    pub translations: BTreeMap<u32, String>,
    pub skipped: usize,
    pub notes: Vec<String>,
    pub fatal: Option<CallError>,
}

impl BatchOutcome {
    fn merge(&mut self, other: BatchOutcome) {
        self.translations.extend(other.translations);
        self.skipped += other.skipped;
        self.notes.extend(other.notes);
        self.fatal = self.fatal.take().or(other.fatal);
    }
}

struct Run<'a> {
    model: &'a dyn Model,
    engine: &'a dyn Engine,
    system: &'a str,
    tuning: &'a Tuning,
    cancel: &'a Cancel,
    progress: &'a dyn Progress,
    spent: Mutex<Usage>,
}

pub async fn translate(
    model: &dyn Model,
    engine: &dyn Engine,
    system: &str,
    units: Vec<TranslationUnit>,
    tuning: &Tuning,
    cancel: &Cancel,
    progress: &dyn Progress,
) -> (Result<BatchOutcome, CallError>, Usage) {
    let run = Run {
        model,
        engine,
        system,
        tuning,
        cancel,
        progress,
        spent: Mutex::new(Usage::default()),
    };

    let answered = ask(&run, units, 0, BTreeMap::new()).await;
    let spent = *run.spent.lock().expect("usage lock");

    (answered, spent)
}

fn ask<'a>(
    run: &'a Run<'a>,
    units: Vec<TranslationUnit>,
    round: u32,
    refused: BTreeMap<u32, String>,
) -> BoxFuture<'a, Result<BatchOutcome, CallError>> {
    async move {
        if units.is_empty() {
            return Ok(BatchOutcome::default());
        }

        if run.cancel.stopped() {
            return Err(CallError::Stopped);
        }

        let sent: Vec<TranslationUnit> = units
            .iter()
            .enumerate()
            .map(|(at, unit)| TranslationUnit {
                id: at as u32,
                text: unit.text.clone(),
                offer: Default::default(),
            })
            .collect();

        let user = prompt::user_prompt(run.engine, &sent, &refused).map_err(CallError::Fatal)?;
        let request = Request {
            system: run.system,
            user: &user,
            cancel: run.cancel,
        };

        let generation =
            match generate_with_retry(run.model, request, run.tuning, run.progress).await {
                Ok(generation) => generation,
                Err(CallError::Blocked(reason)) => {
                    return blocked(run, units, round, reason).await;
                }
                Err(other) => return Err(other),
            };

        run.spent.lock().expect("usage lock").add(generation.usage);

        let parsed = match repair::parse_items(&generation.text) {
            Ok(parsed) => parsed,
            Err(error) => {
                return if units.len() > 1 && generation.truncated {
                    split(run, units, round).await
                } else if round >= run.tuning.repair_rounds {
                    Err(CallError::Retryable(format!("{error:#}")))
                } else {
                    ask(run, units, round + 1, BTreeMap::new()).await
                };
            }
        };

        let Sifted {
            mut outcome,
            refused,
            missing,
        } = sift(run.engine, &sent, parsed);

        if missing.is_empty() {
            return Ok(outcome);
        }

        if round >= run.tuning.repair_rounds {
            outcome.skipped += missing.len();
            return Ok(outcome);
        }

        let again: Vec<TranslationUnit> = missing
            .iter()
            .map(|&at| units[at as usize].clone())
            .collect();
        let carried: BTreeMap<u32, String> = missing
            .iter()
            .enumerate()
            .filter_map(|(child, at)| refused.get(at).map(|why| (child as u32, why.clone())))
            .collect();

        let left = missing.len();
        let repaired = if missing.len() > 1
            && (generation.truncated || (refused.is_empty() && outcome.translations.is_empty()))
        {
            split(run, again, round + 1).await
        } else {
            ask(run, again, round + 1, carried).await
        }
        .map(|part| rekey(part, |child| missing[child as usize]));
        absorb(&mut outcome, repaired, left)?;

        Ok(outcome)
    }
    .boxed()
}

struct Sifted {
    outcome: BatchOutcome,
    refused: BTreeMap<u32, String>,
    missing: Vec<u32>,
}

fn rekey(mut outcome: BatchOutcome, key: impl Fn(u32) -> u32) -> BatchOutcome {
    outcome.translations = outcome
        .translations
        .into_iter()
        .map(|(at, translation)| (key(at), translation))
        .collect();

    outcome
}

fn sift(engine: &dyn Engine, units: &[TranslationUnit], parsed: repair::ParseOutcome) -> Sifted {
    let mut outcome = BatchOutcome::default();
    if parsed.quality != ParseQuality::Clean {
        outcome
            .notes
            .push(format!("JSON needed repair ({:?})", parsed.quality));
    }

    let sources: BTreeMap<u32, &str> = units
        .iter()
        .map(|unit| (unit.id, unit.text.as_str()))
        .collect();

    let answered: BTreeSet<u32> = parsed.items.iter().map(|item| item.id).collect();
    let stray: Vec<u32> = answered
        .iter()
        .filter(|id| !sources.contains_key(id))
        .copied()
        .collect();
    let absent = sources.keys().any(|id| !answered.contains(id));

    if !stray.is_empty() && absent {
        outcome.notes.push(format!(
            "answered with id(s) nobody asked for ({}) while leaving asked ones out, so the \
             numbering has moved and no line in this answer is known to sit where it belongs",
            stray
                .iter()
                .take(4)
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ));

        return Sifted {
            outcome,
            refused: BTreeMap::new(),
            missing: units.iter().map(|unit| unit.id).collect(),
        };
    }

    let mut refused: BTreeMap<u32, String> = BTreeMap::new();
    for item in parsed.items {
        let Some(source) = sources.get(&item.id) else {
            continue;
        };

        if let Err(reason) = engine.answered(source, &item.translation) {
            outcome.notes.push(format!(
                "rejected: {reason}\n  from: {source}\n  into: {}",
                item.translation
            ));
            refused.insert(item.id, reason);
            continue;
        }

        outcome.translations.insert(item.id, item.translation);
    }

    let missing: Vec<u32> = units
        .iter()
        .filter(|unit| !outcome.translations.contains_key(&unit.id))
        .map(|unit| unit.id)
        .collect();

    Sifted {
        outcome,
        refused,
        missing,
    }
}

fn absorb(
    outcome: &mut BatchOutcome,
    asked: Result<BatchOutcome, CallError>,
    count: usize,
) -> Result<bool, CallError> {
    match asked {
        Ok(part) => {
            outcome.merge(part);
            Ok(outcome.fatal.is_none())
        }
        Err(CallError::Retryable(message)) => {
            outcome.skipped += count;
            outcome
                .notes
                .push(format!("{count} line(s) skipped: {message}"));
            Ok(true)
        }
        Err(CallError::Stopped) => {
            outcome.skipped += count;
            outcome
                .notes
                .push(format!("{count} line(s) left when the run stopped"));
            Ok(false)
        }
        Err(other) => {
            if outcome.translations.is_empty() {
                return Err(other);
            }

            outcome.skipped += count;
            outcome.fatal = outcome.fatal.take().or(Some(other));
            Ok(false)
        }
    }
}

fn blocked<'a>(
    run: &'a Run<'a>,
    units: Vec<TranslationUnit>,
    round: u32,
    reason: String,
) -> BoxFuture<'a, Result<BatchOutcome, CallError>> {
    async move {
        if units.len() == 1 {
            return Ok(BatchOutcome {
                skipped: 1,
                notes: vec![format!(
                    "blocked by the API ({reason})\n  from: {}",
                    units[0].text
                )],
                ..BatchOutcome::default()
            });
        }

        split(run, units, round).await
    }
    .boxed()
}

fn split<'a>(
    run: &'a Run<'a>,
    units: Vec<TranslationUnit>,
    round: u32,
) -> BoxFuture<'a, Result<BatchOutcome, CallError>> {
    async move {
        let (left, right) = units.split_at(units.len() / 2);
        let halves = [left.to_vec(), right.to_vec()];

        let mut outcome = BatchOutcome::default();
        let mut stopped = false;
        let mut begins = 0;
        for half in halves {
            let count = half.len();
            let from = begins;
            begins += count as u32;
            if stopped {
                outcome.skipped += count;
                continue;
            }

            let asked = ask(run, half, round, BTreeMap::new())
                .await
                .map(|part| rekey(part, |at| at + from));
            if !absorb(&mut outcome, asked, count)? {
                stopped = true;
            }
        }

        Ok(outcome)
    }
    .boxed()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::renpy::RenPy;
    use crate::llm::{Generation, Speaks};
    use crate::progress::Quiet;
    use std::sync::atomic::{AtomicUsize, Ordering};

    enum Behaviour {
        Everything,
        FirstOnly,
        AlwaysBlocked,
        DropsVariables,
        FirstThenProse,
        FirstThenFatal,
        FirstAsZero,
        ShiftedByOne,
        JunkBeside,
        FirstTruncated,
    }

    struct Fake {
        behaviour: Behaviour,
        calls: AtomicUsize,
    }

    impl Fake {
        fn new(behaviour: Behaviour) -> Self {
            Self {
                behaviour,
                calls: AtomicUsize::new(0),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::Relaxed)
        }
    }

    impl Speaks for Fake {
        async fn call(&self, request: Request<'_>) -> Result<Generation, CallError> {
            let call = self.calls.fetch_add(1, Ordering::Relaxed);

            let start = request
                .user
                .rfind("\n\n")
                .map(|at| at + 2)
                .expect("prompt ends with the array");
            let asked: Vec<TranslationUnit> =
                serde_json::from_str(&request.user[start..]).expect("prompt is valid JSON");

            if matches!(self.behaviour, Behaviour::AlwaysBlocked) {
                return Err(CallError::Blocked("SAFETY".to_string()));
            }

            if matches!(self.behaviour, Behaviour::FirstThenProse) && call > 0 {
                return Ok(Generation {
                    text: "sorry, I cannot help with that".to_string(),
                    truncated: false,
                    usage: Usage::default(),
                });
            }

            if matches!(self.behaviour, Behaviour::FirstThenFatal) && call > 0 {
                return Err(CallError::Fatal(anyhow::anyhow!("bad key")));
            }

            if matches!(self.behaviour, Behaviour::ShiftedByOne) {
                let items: Vec<serde_json::Value> = asked
                    .iter()
                    .map(|unit| {
                        serde_json::json!({
                            "id": unit.id + 1,
                            "translation": format!("translated {}", unit.text),
                        })
                    })
                    .collect();

                return Ok(Generation {
                    text: serde_json::Value::Array(items).to_string(),
                    truncated: false,
                    usage: Usage::default(),
                });
            }

            if matches!(self.behaviour, Behaviour::JunkBeside) {
                let mut items: Vec<serde_json::Value> = asked
                    .iter()
                    .map(|unit| {
                        serde_json::json!({
                            "id": unit.id,
                            "translation": format!("translated {}", unit.text),
                        })
                    })
                    .collect();
                items.push(serde_json::json!({ "id": 999, "translation": "leftovers" }));

                return Ok(Generation {
                    text: serde_json::Value::Array(items).to_string(),
                    truncated: false,
                    usage: Usage::default(),
                });
            }

            if matches!(self.behaviour, Behaviour::FirstAsZero) {
                let items = serde_json::json!([{
                    "id": 0,
                    "translation": format!("translated {}", asked[0].text),
                }]);

                return Ok(Generation {
                    text: items.to_string(),
                    truncated: false,
                    usage: Usage::default(),
                });
            }

            let answered = match self.behaviour {
                Behaviour::FirstOnly
                | Behaviour::FirstThenProse
                | Behaviour::FirstThenFatal
                | Behaviour::FirstTruncated => &asked[..1],
                _ => &asked[..],
            };

            let items: Vec<_> = answered
                .iter()
                .map(|unit| {
                    let translation = match self.behaviour {
                        Behaviour::DropsVariables => "no variable here".to_string(),
                        _ => format!("translated {} for [name]", unit.id),
                    };
                    serde_json::json!({ "id": unit.id, "translation": translation })
                })
                .collect();

            Ok(Generation {
                text: serde_json::to_string(&items).expect("serialisable"),
                truncated: matches!(self.behaviour, Behaviour::FirstTruncated),
                usage: Usage::default(),
            })
        }

        async fn reach(&self, _cancel: &Cancel) -> Result<(), CallError> {
            Ok(())
        }
    }

    fn units(count: usize) -> Vec<TranslationUnit> {
        (0..count as u32)
            .map(|id| TranslationUnit {
                id,
                text: format!("line {id} for [name]"),
                offer: Default::default(),
            })
            .collect()
    }

    async fn run(model: &Fake, count: usize) -> Result<BatchOutcome, CallError> {
        let (outcome, _) = translate(
            model,
            &RenPy,
            "system",
            units(count),
            &Tuning::instant(),
            &Cancel::default(),
            &Quiet,
        )
        .await;

        outcome
    }

    #[tokio::test]
    async fn a_complete_answer_needs_one_call() {
        let model = Fake::new(Behaviour::Everything);
        let outcome = run(&model, 4).await.unwrap();

        assert_eq!(outcome.translations.len(), 4);
        assert_eq!(outcome.skipped, 0);
        assert_eq!(
            model.calls(),
            1,
            "an answer with nothing missing has nothing to ask again for, and splitting it \
             anyway would bill the reader for work already done"
        );
    }

    #[tokio::test]
    async fn missing_items_are_asked_for_again_then_given_up() {
        let model = Fake::new(Behaviour::FirstOnly);
        let outcome = run(&model, 4).await.unwrap();

        assert_eq!(
            outcome.translations.len(),
            (Tuning::instant().repair_rounds + 1) as usize,
            "one item per round"
        );
        assert_eq!(outcome.skipped, 1, "the rest are reported, never invented");
        assert!(model.calls() > 1, "it retried the missing items");
    }

    #[tokio::test]
    async fn lines_already_translated_survive_a_failing_repair_round() {
        let model = Fake::new(Behaviour::FirstThenProse);
        let outcome = run(&model, 4).await.unwrap();

        assert_eq!(outcome.translations.len(), 1, "the paid answer is kept");
        assert_eq!(outcome.skipped, 3, "the rest are given up, not lost");
    }

    #[tokio::test]
    async fn a_fatal_mid_batch_keeps_what_landed_and_still_carries_the_fatality_out() {
        let model = Fake::new(Behaviour::FirstThenFatal);
        let outcome = run(&model, 4).await.unwrap();

        assert_eq!(outcome.translations.len(), 1, "the paid answer is kept");
        assert_eq!(outcome.skipped, 3, "the rest are given up, not lost");
        assert!(
            matches!(outcome.fatal, Some(CallError::Fatal(_))),
            "a wrong key fails every later request too, so the fatality has to ride out beside \
             the kept lines and stop the run, not soften into one more warning"
        );
    }

    #[tokio::test]
    async fn a_blocked_batch_is_split_down_to_single_items() {
        let model = Fake::new(Behaviour::AlwaysBlocked);
        let outcome = run(&model, 4).await.unwrap();

        assert!(outcome.translations.is_empty());
        assert_eq!(outcome.skipped, 4, "each item is skipped on its own");
        assert_eq!(
            model.calls(),
            7,
            "4 items means 1 + 2 + 4 attempts, and splitting more often than halving needs is \
             money spent on calls the reader never asked for"
        );
    }

    #[tokio::test]
    async fn a_translation_that_loses_a_variable_is_refused() {
        let model = Fake::new(Behaviour::DropsVariables);
        let outcome = run(&model, 2).await.unwrap();

        assert!(outcome.translations.is_empty(), "nothing broken is written");
        assert_eq!(outcome.skipped, 2);
    }

    #[tokio::test]
    async fn an_empty_batch_costs_nothing() {
        let model = Fake::new(Behaviour::Everything);
        let outcome = run(&model, 0).await.unwrap();

        assert!(outcome.translations.is_empty());
        assert_eq!(
            model.calls(),
            0,
            "asking a model about no lines still costs a call and still waits on the network"
        );
    }

    #[tokio::test]
    async fn a_stopped_run_asks_for_nothing() {
        let model = Fake::new(Behaviour::Everything);
        let cancel = Cancel::default();
        cancel.stop();

        let (result, _) = translate(
            &model,
            &RenPy,
            "system",
            units(4),
            &Tuning::instant(),
            &cancel,
            &Quiet,
        )
        .await;

        assert!(
            matches!(result, Err(CallError::Stopped)),
            "a run that was stopped is not a run that failed, and filing it as a fault would \
             have the reader hunting for a bug in us"
        );
        assert_eq!(
            model.calls(),
            0,
            "stop has to be read before the call goes out, or pressing it still spends money on \
             a batch nobody is waiting for"
        );
    }

    #[tokio::test]
    async fn a_line_asked_again_lands_on_its_own_number_not_the_one_it_was_sent_under() {
        let model = Fake::new(Behaviour::FirstAsZero);
        let outcome = run(&model, 4).await.unwrap();

        for (id, translation) in &outcome.translations {
            assert_eq!(
                translation,
                &format!("translated line {id} for [name]"),
                "every batch is sent renumbered from zero, so the answer has to be carried back \
                 to the line it was written for, never a neighbour's"
            );
        }
        assert_eq!(
            outcome.translations.len(),
            (Tuning::instant().repair_rounds + 1) as usize
        );
    }

    #[tokio::test]
    async fn a_truncated_answer_that_parsed_partially_is_split_not_reasked_whole() {
        let model = Fake::new(Behaviour::FirstTruncated);
        let outcome = run(&model, 4).await.unwrap();

        assert_eq!(
            outcome.translations.len(),
            4,
            "halving fits under the cap that cut the answer; re-asking everything at once \
             never does"
        );
        assert_eq!(outcome.skipped, 0);
    }

    #[tokio::test]
    async fn an_answer_whose_numbering_moved_is_not_believed_for_any_of_its_lines() {
        let model = Fake::new(Behaviour::ShiftedByOne);
        let outcome = run(&model, 4).await.unwrap();

        assert!(
            outcome.translations.is_empty(),
            "every line but the last would have matched an id that belongs to its neighbour, and \
             a wrong translation that looks done is never retried"
        );
        let told = "answered with id(s) nobody asked for (4) while leaving asked ones out, so \
                    the numbering has moved and no line in this answer is known to sit where it \
                    belongs";

        assert!(
            outcome.notes.iter().any(|one| one == told),
            "the reader has to be told why nothing landed: {:?}",
            outcome.notes
        );
    }

    #[tokio::test]
    async fn one_leftover_item_does_not_throw_away_the_answers_beside_it() {
        let model = Fake::new(Behaviour::JunkBeside);
        let outcome = run(&model, 4).await.unwrap();

        assert_eq!(
            outcome.translations.len(),
            4,
            "every id asked for came back, so the extra one is junk to drop, not a reason to pay \
             for the whole batch again"
        );
    }
}
