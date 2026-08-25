use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Mode {
    #[default]
    Free,
    Held,
    Clearing,
}

#[derive(Default)]
struct State {
    mode: Mode,
    writers: usize,
}

#[derive(Default)]
pub struct Gate(Arc<Mutex<State>>);

pub struct Pass(Arc<Mutex<State>>);

pub struct Writing(Arc<Mutex<State>>);

impl Drop for Pass {
    fn drop(&mut self) {
        held(&self.0).mode = Mode::Free;
    }
}

impl Drop for Writing {
    fn drop(&mut self) {
        let mut state = held(&self.0);
        state.writers = state.writers.saturating_sub(1);
    }
}

fn held(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Gate {
    pub fn enter(&self) -> Option<Pass> {
        self.hold(Mode::Held)
    }

    pub fn enter_clearing(&self) -> Option<Pass> {
        self.hold(Mode::Clearing)
    }

    pub fn writing(&self) -> Option<Writing> {
        let mut state = held(&self.0);
        if state.mode == Mode::Clearing {
            return None;
        }

        state.writers += 1;

        Some(Writing(Arc::clone(&self.0)))
    }

    fn hold(&self, mode: Mode) -> Option<Pass> {
        let mut state = held(&self.0);
        if state.mode != Mode::Free || (mode == Mode::Clearing && state.writers > 0) {
            return None;
        }

        state.mode = mode;

        Some(Pass(Arc::clone(&self.0)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_one_action_holds_the_gate() {
        let gate = Gate::default();

        let first = gate.enter().expect("the first action goes through");
        assert!(
            gate.enter().is_none(),
            "a second action must be turned away, not queued behind the first"
        );
        assert!(gate.enter_clearing().is_none());

        drop(first);
        assert!(gate.enter().is_some(), "and the next one goes through");
    }

    #[test]
    fn a_save_is_refused_while_the_store_is_being_cleared() {
        let gate = Gate::default();

        let wiping = gate.enter_clearing().expect("a pass");
        assert!(
            gate.writing().is_none(),
            "a line written into a store that is being deleted would bring it back"
        );

        drop(wiping);
        assert!(gate.writing().is_some());
    }

    #[test]
    fn a_clearing_waits_for_every_writer_not_just_the_last() {
        let gate = Gate::default();

        let first = gate.writing().expect("a lease");
        let second = gate.writing().expect("another lease");

        drop(first);
        assert!(
            gate.enter_clearing().is_none(),
            "one lease coming back does not mean the store is quiet"
        );

        drop(second);
        assert!(gate.enter_clearing().is_some());
    }

    #[test]
    fn a_run_and_a_hand_edit_never_wait_on_each_other() {
        let gate = Gate::default();

        let translating = gate.enter().expect("a pass");
        assert!(
            gate.writing().is_some(),
            "a run holds the gate for minutes, so hand edits have to keep working through it"
        );
        drop(translating);

        let _writing = gate.writing().expect("a lease");
        assert!(
            gate.enter().is_some(),
            "and only clearing waits for writers to finish, or one hand edit still landing would \
             turn a whole run away"
        );
    }
}
