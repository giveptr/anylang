use crate::service::editor::Tally;
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Default)]
struct Open {
    game: String,
    root: Option<PathBuf>,
    survey: Option<Tally>,
}

#[derive(Default)]
pub struct Session(Mutex<Open>);

impl Session {
    pub fn opening(&self, game: &str, root: PathBuf) {
        *self.0.lock().expect("session lock") = Open {
            game: game.to_string(),
            root: Some(root),
            survey: None,
        };
    }

    pub fn opened(&self, game: &str, root: PathBuf, survey: Tally) {
        *self.0.lock().expect("session lock") = Open {
            game: game.to_string(),
            root: Some(root),
            survey: Some(survey),
        };
    }

    pub fn root(&self) -> Option<PathBuf> {
        self.0.lock().expect("session lock").root.clone()
    }

    pub fn closed(&self) {
        *self.0.lock().expect("session lock") = Open::default();
    }

    pub fn game_dir(&self) -> String {
        self.0.lock().expect("session lock").game.clone()
    }

    pub fn survey(&self) -> Option<Tally> {
        self.0.lock().expect("session lock").survey
    }
}
