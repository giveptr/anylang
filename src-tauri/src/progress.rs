use anyhow::Error;
use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Session,
    Project,
    Prepare,
    Translate,
    Export,
    Restore,
    Clear,
    Exclude,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum FileOutcome {
    Completed { lines: u32 },
    Partial { done: u32, total: u32 },
}

pub struct Batch<'a> {
    pub file: &'a str,
    pub filled: usize,
    pub added: usize,
}

pub trait Progress: Send + Sync {
    fn stage(&self, steps: &[&str], at: usize);

    fn file_started(&self, file: &str);

    fn file_done(&self, file: &str, outcome: FileOutcome);

    fn batch_done(&self, batch: Batch<'_>);

    fn say(&self, source: Source, message: &str, level: Level);

    fn running(&self, _yes: bool) {}

    fn info(&self, source: Source, message: &str) {
        self.say(source, message, Level::Info);
    }

    fn warn(&self, source: Source, message: &str) {
        self.say(source, message, Level::Warn);
    }

    fn error(&self, source: Source, message: &str) {
        self.say(source, message, Level::Error);
    }

    fn failed(&self, source: Source, error: &Error) {
        self.say(source, &format!("{error:#}"), Level::Error);
    }
}

#[cfg(test)]
pub struct Quiet;

#[cfg(test)]
impl Progress for Quiet {
    fn stage(&self, _: &[&str], _: usize) {}

    fn file_started(&self, _: &str) {}

    fn file_done(&self, _: &str, _: FileOutcome) {}

    fn batch_done(&self, _: Batch<'_>) {}

    fn say(&self, _: Source, _: &str, _: Level) {}
}

#[cfg(test)]
use std::sync::Mutex;

#[cfg(test)]
#[derive(Default)]
pub struct Heard {
    said: Mutex<Vec<(Level, String)>>,
}

#[cfg(test)]
impl Heard {
    pub fn warnings(&self) -> Vec<String> {
        self.said
            .lock()
            .expect("no other thread holds what was heard")
            .iter()
            .filter(|(level, _)| *level == Level::Warn)
            .map(|(_, said)| said.clone())
            .collect()
    }
}

#[cfg(test)]
impl Progress for Heard {
    fn stage(&self, _: &[&str], _: usize) {}

    fn file_started(&self, _: &str) {}

    fn file_done(&self, _: &str, _: FileOutcome) {}

    fn batch_done(&self, _: Batch<'_>) {}

    fn say(&self, _: Source, message: &str, level: Level) {
        self.said
            .lock()
            .expect("no other thread holds what was heard")
            .push((level, message.to_string()));
    }
}
