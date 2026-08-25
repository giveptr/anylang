use crate::progress::{Batch, FileOutcome, Level, Progress, Source};
use crate::scope::leaf;
use crate::service::logs::{self, LogEntry, stamp};
use crate::session::Session;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct FileStarted {
    pub file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct FileDone {
    pub file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct BatchDone {
    pub file: String,
    pub filled: u32,
    pub added: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct RunState {
    pub running: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct Preparing {
    pub steps: Vec<String>,
    pub at: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct Notice {
    pub at: String,
    pub source: Source,
    pub message: String,
    pub level: Level,
}

pub struct Ui {
    app: AppHandle,
}

impl Ui {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }

    fn keep(&self, said: &Notice) {
        let Some(root) = self.app.state::<Session>().root() else {
            return;
        };

        logs::queue(
            root,
            LogEntry {
                at: said.at.clone(),
                source: said.source,
                message: said.message.clone(),
                level: said.level,
            },
        );
    }
}

impl Progress for Ui {
    fn stage(&self, steps: &[&str], at: usize) {
        let _ = Preparing {
            steps: steps.iter().map(|step| step.to_string()).collect(),
            at: at as u32,
        }
        .emit(&self.app);
    }

    fn file_started(&self, file: &str) {
        let _ = FileStarted {
            file: file.to_string(),
        }
        .emit(&self.app);
    }

    fn file_done(&self, file: &str, outcome: FileOutcome) {
        let (said, level) = match outcome {
            FileOutcome::Completed { lines } => {
                (format!("{lines} line(s) translated"), Level::Info)
            }
            FileOutcome::Partial { done, total } => (format!("{done}/{total} done"), Level::Warn),
        };

        self.say(Source::Translate, &format!("{}: {said}", leaf(file)), level);

        let _ = FileDone {
            file: file.to_string(),
        }
        .emit(&self.app);
    }

    fn batch_done(&self, batch: Batch<'_>) {
        let _ = BatchDone {
            file: batch.file.to_string(),
            filled: batch.filled as u32,
            added: batch.added as u32,
        }
        .emit(&self.app);
    }

    fn running(&self, yes: bool) {
        let _ = RunState { running: yes }.emit(&self.app);
    }

    fn say(&self, source: Source, message: &str, level: Level) {
        let said = Notice {
            at: stamp(),
            source,
            message: message.to_string(),
            level,
        };

        self.keep(&said);
        let _ = said.emit(&self.app);
    }
}
