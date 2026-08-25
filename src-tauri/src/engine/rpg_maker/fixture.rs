use crate::canvas::Canvas;
use crate::engine::Swap;
use crate::engine::pictures::Pictures;
use crate::hash::xxh3;
use std::fs;
use std::path::Path;

pub fn sandbox() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temp folder")
}

pub struct Game {
    pub dir: tempfile::TempDir,
    pub store: tempfile::TempDir,
    pub staged: tempfile::TempDir,
    pub held: tempfile::TempDir,
}

pub fn a_game() -> Game {
    Game {
        dir: sandbox(),
        store: sandbox(),
        staged: sandbox(),
        held: sandbox(),
    }
}

impl Game {
    pub fn root(&self) -> &Path {
        self.dir.path()
    }

    pub fn store(&self) -> &Path {
        self.store.path()
    }

    pub fn put(&self, at: &str, body: &[u8]) {
        let landing = self.root().join(at);
        fs::create_dir_all(landing.parent().expect("a folder")).expect("a folder");
        fs::write(landing, body).expect("a game file");
    }

    pub fn pick(&self, key: &str, drawn: &Canvas) -> Pictures {
        let at = self.held.path().join(format!("{}.png", xxh3(key)));
        fs::write(&at, drawn.png().expect("a png")).expect("a picture to pick");

        Pictures {
            swaps: vec![Swap {
                from: key.to_string(),
                to: at.to_string_lossy().to_string(),
            }],
            ..Pictures::default()
        }
    }

    pub fn bytes(&self, at: &str) -> Vec<u8> {
        fs::read(self.root().join(at)).expect("a game file")
    }
}
