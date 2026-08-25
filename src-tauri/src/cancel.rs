use crate::scope::{self, Scope};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::watch;

#[derive(Debug)]
pub struct Cancel(watch::Sender<bool>);

impl Default for Cancel {
    fn default() -> Self {
        Self(watch::channel(false).0)
    }
}

impl Cancel {
    pub fn stop(&self) {
        self.0.send_replace(true);
    }

    pub fn stopped(&self) -> bool {
        *self.0.borrow()
    }

    pub async fn cancelled(&self) {
        let mut watching = self.0.subscribe();

        if *watching.borrow_and_update() {
            return;
        }

        let _ = watching.changed().await;
    }
}

#[derive(Default)]
pub struct Tokens {
    whole: Arc<Cancel>,
    each: Mutex<HashMap<String, Arc<Cancel>>>,
    shut: Mutex<Vec<Scope>>,
}

impl Tokens {
    pub fn stopped(&self) -> bool {
        self.whole.stopped()
    }

    pub fn whole(&self) -> Arc<Cancel> {
        self.whole.clone()
    }

    pub fn enlist(&self, file: &str) -> Arc<Cancel> {
        let shut = self.shut.lock().expect("scope lock");
        let mut each = self.each.lock().expect("token lock");

        let token = each
            .entry(file.to_string())
            .or_insert_with(|| Arc::new(Cancel::default()))
            .clone();

        if shut.iter().any(|scope| scope.holds(file)) {
            token.stop();
        }

        token
    }

    pub fn files(&self) -> Vec<String> {
        self.each
            .lock()
            .expect("token lock")
            .keys()
            .cloned()
            .collect()
    }

    pub fn retire(&self, file: &str) {
        self.each.lock().expect("token lock").remove(file);
    }

    pub fn stop(&self, reach: &[Scope]) {
        if scope::anywhere(reach) {
            self.whole.stop();
        }

        let mut shut = self.shut.lock().expect("scope lock");
        let each = self.each.lock().expect("token lock");

        shut.extend(reach.iter().cloned());

        for (file, cancel) in each.iter() {
            if reach.iter().any(|one| one.holds(file)) {
                cancel.stop();
            }
        }
    }
}

type Slot = Arc<Mutex<Option<Arc<Tokens>>>>;

#[derive(Default)]
pub struct Runs(Slot);

pub struct Claim {
    slot: Slot,
    pub tokens: Arc<Tokens>,
}

impl Drop for Claim {
    fn drop(&mut self) {
        *self.slot.lock().expect("cancel lock") = None;
    }
}

impl Runs {
    pub fn claim(&self) -> Option<Claim> {
        let mut slot = self.0.lock().expect("cancel lock");
        if slot.is_some() {
            return None;
        }

        let tokens = Arc::new(Tokens::default());
        *slot = Some(tokens.clone());

        Some(Claim {
            slot: self.0.clone(),
            tokens,
        })
    }

    pub fn running(&self) -> bool {
        self.0.lock().expect("cancel lock").is_some()
    }

    pub fn active(&self) -> Vec<String> {
        self.0
            .lock()
            .expect("cancel lock")
            .as_ref()
            .map(|tokens| tokens.files())
            .unwrap_or_default()
    }

    pub fn stop(&self, reach: &[Scope]) {
        if let Some(tokens) = self.0.lock().expect("cancel lock").as_ref() {
            tokens.stop(reach);
        }
    }
}

#[derive(Default)]
pub struct Solo(Mutex<Option<(String, Arc<Cancel>)>>);

impl Solo {
    pub fn afresh(&self, file: &str) -> Arc<Cancel> {
        let fresh = Arc::new(Cancel::default());

        if let Some((_, before)) = self
            .0
            .lock()
            .expect("solo lock")
            .replace((file.to_string(), Arc::clone(&fresh)))
        {
            before.stop();
        }

        fresh
    }

    pub fn stop(&self, reach: &[Scope]) {
        if let Some((file, cancel)) = self.0.lock().expect("solo lock").as_ref()
            && reach.iter().any(|one| one.holds(file))
        {
            cancel.stop();
        }
    }
}

#[derive(Default)]
pub struct Seeker(Mutex<Option<Arc<Cancel>>>);

impl Seeker {
    pub fn afresh(&self) -> Arc<Cancel> {
        let fresh = Arc::new(Cancel::default());

        if let Some(before) = self
            .0
            .lock()
            .expect("seeker lock")
            .replace(Arc::clone(&fresh))
        {
            before.stop();
        }

        fresh
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn only_one_run_at_a_time() {
        let runs = Runs::default();
        let first = runs.claim().expect("the first claim wins");

        assert!(runs.claim().is_none(), "a second run is refused");

        drop(first);
        assert!(runs.claim().is_some(), "the slot frees when the run ends");
    }

    #[test]
    fn stopping_everything_reaches_the_run_and_every_file() {
        let runs = Runs::default();
        let claim = runs.claim().unwrap();
        let one = claim.tokens.enlist("data/Map001.json");

        assert!(!claim.tokens.stopped());
        runs.stop(&[Scope::default()]);

        assert!(claim.tokens.stopped());
        assert!(one.stopped(), "a file already running has to stop too");
    }

    #[test]
    fn stopping_a_group_reaches_every_file_that_came_out_of_it() {
        let runs = Runs::default();
        let claim = runs.claim().unwrap();
        let here = claim
            .tokens
            .enlist("text_asset/resources.assets/scene001.txt");
        let elsewhere = claim
            .tokens
            .enlist("text_asset/sharedassets0.assets/mom.txt");

        runs.stop(&[Scope::read("text_asset/resources.assets").unwrap()]);

        assert!(here.stopped());
        assert!(!elsewhere.stopped(), "another container keeps going");
        assert!(!claim.tokens.stopped(), "the run itself keeps going");
    }

    #[test]
    fn stopping_the_matches_of_a_search_reaches_those_files_and_no_others() {
        let runs = Runs::default();
        let claim = runs.claim().unwrap();
        let one = claim
            .tokens
            .enlist("text_asset/resources.assets/hero.atlas");
        let two = claim
            .tokens
            .enlist("text_asset/sharedassets0.assets/boss.atlas");
        let other = claim
            .tokens
            .enlist("text_asset/resources.assets/scene001.txt");

        runs.stop(
            &scope::reach(&[
                "text_asset/resources.assets/hero.atlas".to_string(),
                "text_asset/sharedassets0.assets/boss.atlas".to_string(),
            ])
            .unwrap(),
        );

        assert!(one.stopped());
        assert!(two.stopped());
        assert!(
            !other.stopped(),
            "a file in the same group that the search did not match keeps going"
        );
        assert!(!claim.tokens.stopped(), "the run itself keeps going");
    }

    #[test]
    fn a_reopened_window_can_still_see_which_files_are_running() {
        let runs = Runs::default();
        assert!(!runs.running());

        let claim = runs.claim().unwrap();
        claim.tokens.enlist("data/Map001.json");
        claim.tokens.enlist("data/Map002.json");

        assert!(runs.running());

        let mut files = runs.active();
        files.sort();
        assert_eq!(files, ["data/Map001.json", "data/Map002.json"]);

        claim.tokens.retire("data/Map001.json");
        assert_eq!(runs.active(), ["data/Map002.json"]);
    }

    #[test]
    fn a_file_that_had_not_started_yet_is_still_stopped() {
        let runs = Runs::default();
        let claim = runs.claim().unwrap();

        runs.stop(&[Scope::read("text_asset/resources.assets").unwrap()]);

        let late = claim
            .tokens
            .enlist("text_asset/resources.assets/scene404.txt");
        assert!(
            late.stopped(),
            "a file the run had not reached yet must not start translating after a group stop"
        );

        let elsewhere = claim
            .tokens
            .enlist("text_asset/sharedassets0.assets/mom.txt");
        assert!(
            !elsewhere.stopped(),
            "and a file outside the stopped group still runs"
        );
    }

    #[test]
    fn a_retired_file_is_no_longer_busy() {
        let runs = Runs::default();
        let claim = runs.claim().unwrap();
        let one = claim.tokens.enlist("data/Map001.json");
        claim.tokens.retire("data/Map001.json");

        runs.stop(&[Scope::read("data/Map001.json").unwrap()]);

        assert!(runs.active().is_empty());
        assert!(!one.stopped(), "a file that already finished is left alone");
    }

    #[test]
    fn a_new_run_starts_unstopped() {
        let runs = Runs::default();
        let first = runs.claim().unwrap();
        runs.stop(&[Scope::default()]);
        drop(first);

        let second = runs.claim().unwrap();
        assert!(!second.tokens.stopped(), "the old stop does not carry over");
    }

    #[test]
    fn one_line_answers_to_the_stop_button_like_any_other_translation() {
        let solo = Solo::default();

        let first = solo.afresh("data/Map001.json");
        let second = solo.afresh("data/Map002.json");
        assert!(
            first.stopped(),
            "only the newest suggestion is being waited on, so the one before it has to give \
             the API back"
        );
        assert!(!second.stopped());

        solo.stop(&[Scope::read("data/Map001.json").unwrap()]);
        assert!(
            !second.stopped(),
            "a stop aimed at another file leaves this line alone"
        );

        solo.stop(&[Scope::default()]);
        assert!(
            second.stopped(),
            "stop everything has to reach a single line too, or the button lies while the call \
             keeps billing"
        );
    }

    #[test]
    fn a_fresh_search_calls_off_the_one_before_it() {
        let seeker = Seeker::default();

        let first = seeker.afresh();
        assert!(!first.stopped(), "the only search running is not stopped");

        let second = seeker.afresh();
        assert!(
            first.stopped(),
            "a scan nobody is waiting on any more has to give the disk back"
        );
        assert!(!second.stopped());

        let third = seeker.afresh();
        assert!(second.stopped());
        assert!(!third.stopped());
    }

    #[tokio::test]
    async fn a_task_already_waiting_is_released_by_the_stop() {
        let cancel = Arc::new(Cancel::default());
        let waiting = cancel.clone();

        let task = tokio::spawn(async move { waiting.cancelled().await });
        tokio::task::yield_now().await;

        cancel.stop();

        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("a waiter parked before the stop still has to wake up")
            .expect("the task should not panic");
    }

    #[tokio::test]
    async fn waiting_on_an_already_stopped_run_returns_at_once() {
        let cancel = Cancel::default();
        cancel.stop();

        tokio::time::timeout(Duration::from_secs(1), cancel.cancelled())
            .await
            .expect("a stop that happened before the wait must not park forever");
    }
}
