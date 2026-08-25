pub use crate::engine::rpg_maker::fixture::sandbox;
use std::fs;
use std::path::Path;

pub fn put(root: &Path, at: &str, body: &str) {
    let path = root.join(at);
    fs::create_dir_all(path.parent().expect("a parent")).unwrap();
    fs::write(path, body).unwrap();
}
