use crate::walk;
use anyhow::Result;
use std::path::{Path, PathBuf};

const SCRIPTS: [&str; 2] = ["rpy", "rpym"];

fn twin_of(script: &Path) -> Option<PathBuf> {
    let written = script.extension()?.to_str()?;

    SCRIPTS
        .contains(&written)
        .then(|| script.with_extension(format!("{written}c")))
}

const REN: &str = "_ren.py";

fn written_as(compiled: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();

    if let Some(script) = script_of(compiled) {
        out.push(script);
    }

    if compiled.extension().and_then(|kind| kind.to_str()) == Some("rpyc")
        && let Some(stem) = compiled.file_stem().and_then(|held| held.to_str())
    {
        out.push(compiled.with_file_name(format!("{stem}{REN}")));
    }

    out
}

pub async fn orphans(inside: &Path) -> Vec<PathBuf> {
    among(&walk::files(inside).await).await
}

pub async fn among(files: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();

    for at in files {
        let written = written_as(at);
        if written.is_empty() {
            continue;
        }

        let mut held = false;
        for source in &written {
            if there(source).await {
                held = true;
                break;
            }
        }

        if !held {
            out.push(at.clone());
        }
    }

    out.sort();

    out
}

fn script_of(compiled: &Path) -> Option<PathBuf> {
    let written = compiled.extension()?.to_str()?.strip_suffix('c')?;

    SCRIPTS
        .contains(&written)
        .then(|| compiled.with_extension(written))
}

async fn there(at: &Path) -> bool {
    tokio::fs::metadata(at).await.is_ok()
}

pub async fn dropped(script: &Path) -> Result<()> {
    let Some(twin) = twin_of(script) else {
        return Ok(());
    };

    if there(script).await {
        return Ok(());
    }

    walk::removed(&twin).await.map(|_| ())
}

pub async fn dropped_under(root: &Path) -> Result<()> {
    for path in walk::files(root).await {
        let Some(script) = script_of(&path) else {
            continue;
        };

        if there(&script).await {
            continue;
        }

        walk::removed(&path).await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn sandbox(names: &[&str]) -> tempfile::TempDir {
        let at = tempfile::tempdir().expect("a temp folder");

        for name in names {
            let path = at.path().join(name);
            fs::create_dir_all(path.parent().expect("a folder")).expect("a folder");
            fs::write(&path, "").expect("a file");
        }

        at
    }

    #[test]
    fn a_script_and_what_ren_py_compiled_from_it_name_each_other() {
        assert_eq!(
            twin_of(Path::new("game/tl/japanese/script.rpy")),
            Some(PathBuf::from("game/tl/japanese/script.rpyc"))
        );
        assert_eq!(
            twin_of(Path::new("game/common.rpym")),
            Some(PathBuf::from("game/common.rpymc"))
        );
        assert_eq!(twin_of(Path::new("game/gui/font.ttf")), None);
        assert_eq!(twin_of(Path::new("game/archive.rpa")), None);

        assert_eq!(
            script_of(Path::new("game/script.rpyc")),
            Some(PathBuf::from("game/script.rpy"))
        );
        assert_eq!(
            script_of(Path::new("game/common.rpymc")),
            Some(PathBuf::from("game/common.rpym"))
        );
        assert_eq!(script_of(Path::new("game/archive.rpa")), None);
        assert_eq!(script_of(Path::new("game/script.rpy")), None);
    }

    #[tokio::test]
    async fn a_compiled_script_left_without_its_words_is_taken_out() {
        let at = sandbox(&[
            "options.rpyc",
            "screens.rpy",
            "screens.rpyc",
            "deep/script.rpyc",
            "gui/font.ttf",
        ]);

        dropped_under(at.path()).await.expect("the sweep runs");

        assert!(!at.path().join("options.rpyc").exists());
        assert!(!at.path().join("deep").join("script.rpyc").exists());
        assert!(
            at.path().join("screens.rpyc").is_file(),
            "the script beside it is still being translated, and Ren'Py compiles it again itself"
        );
        assert!(at.path().join("gui").join("font.ttf").is_file());
    }

    #[tokio::test]
    async fn a_folder_that_was_never_written_is_nothing_to_sweep() {
        let at = tempfile::tempdir().expect("a temp folder");

        assert!(
            dropped_under(&at.path().join("nowhere")).await.is_ok(),
            "a game nobody has read yet has no folder to sweep, and refusing here would stop an \
             export before it wrote a line"
        );
    }

    #[tokio::test]
    async fn the_twin_of_a_script_taken_back_out_goes_with_it() {
        let at = sandbox(&["anylang.rpyc", "made_names.rpy", "made_names.rpyc"]);

        dropped(&at.path().join("anylang.rpy"))
            .await
            .expect("the twin goes with it");
        assert!(!at.path().join("anylang.rpyc").exists());

        dropped(&at.path().join("made_names.rpy"))
            .await
            .expect("the sweep runs");
        assert!(
            at.path().join("made_names.rpyc").is_file(),
            "the script is still there, so its compiled twin is the game's to keep"
        );
    }
}
