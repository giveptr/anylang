use anyhow::{Context, Result};
use std::io::ErrorKind;
use std::num::NonZero;
use std::path::{Path, PathBuf};
use std::thread;
use tokio::fs;
use walkdir::WalkDir;

pub fn at_once() -> usize {
    thread::available_parallelism()
        .map(NonZero::get)
        .unwrap_or(4)
        .clamp(2, 16)
}

pub fn files_now(root: &Path) -> Vec<PathBuf> {
    WalkDir::new(root)
        .follow_links(true)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(walkdir::DirEntry::into_path)
        .collect()
}

pub fn outside(root: &Path, tops: &[&str]) -> Vec<PathBuf> {
    WalkDir::new(root)
        .follow_links(true)
        .into_iter()
        .filter_entry(|entry| {
            entry.depth() != 1 || !tops.contains(&entry.file_name().to_string_lossy().as_ref())
        })
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(walkdir::DirEntry::into_path)
        .collect()
}

pub async fn accounted(root: &Path) -> Result<Vec<PathBuf>> {
    let root = root.to_path_buf();

    tokio::task::spawn_blocking(move || {
        let mut found = Vec::new();

        for entry in WalkDir::new(&root).follow_links(true) {
            let entry = match entry {
                Ok(one) => one,
                Err(why)
                    if why
                        .io_error()
                        .is_some_and(|io| io.kind() == ErrorKind::NotFound) =>
                {
                    continue;
                }
                Err(why) => {
                    return Err(why).with_context(|| format!("walking {}", root.display()));
                }
            };

            if entry.file_type().is_file() {
                found.push(entry.into_path());
            }
        }

        Ok(found)
    })
    .await?
}

pub async fn files(root: &Path) -> Vec<PathBuf> {
    let root = root.to_path_buf();

    tokio::task::spawn_blocking(move || files_now(&root))
        .await
        .expect("walking files")
}

pub async fn relative(root: &Path) -> Vec<PathBuf> {
    files(root)
        .await
        .into_iter()
        .filter_map(|path| Some(path.strip_prefix(root).ok()?.to_path_buf()))
        .collect()
}

pub async fn removed(at: &Path) -> Result<bool> {
    match fs::remove_file(at).await {
        Ok(()) => Ok(true),
        Err(why) if why.kind() == ErrorKind::NotFound => Ok(false),
        Err(why) => Err(why).with_context(|| format!("removing {}", at.display())),
    }
}

pub async fn cleared(at: &Path) -> Result<bool> {
    match fs::remove_dir_all(at).await {
        Ok(()) => Ok(true),
        Err(why) if why.kind() == ErrorKind::NotFound => Ok(false),
        Err(why) => Err(why).with_context(|| format!("clearing {}", at.display())),
    }
}

pub async fn reset(at: &Path) -> Result<()> {
    cleared(at).await?;

    fs::create_dir_all(at)
        .await
        .with_context(|| format!("creating {}", at.display()))
}

pub async fn copy(from: &Path, to: &Path, wanted: impl Fn(&Path) -> bool) -> Result<u32> {
    let mut copied = 0;

    for at in relative(from).await {
        if !wanted(&at) {
            continue;
        }

        let target = to.join(&at);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).await?;
        }

        fs::copy(from.join(&at), &target)
            .await
            .with_context(|| format!("copying {}", target.display()))?;

        copied += 1;
    }

    Ok(copied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[tokio::test]
    async fn relative_paths_stay_inside_the_folder_they_came_from() {
        let sandbox = tempfile::tempdir().expect("a temp folder");
        let root = sandbox.path();
        let nested = root.join("common").join("deep");
        fs::create_dir_all(&nested).unwrap();
        fs::write(root.join("script.rpy"), "").unwrap();
        fs::write(nested.join("screens.rpy"), "").unwrap();

        let mut found = relative(root).await;
        found.sort();

        assert_eq!(
            found,
            vec![
                PathBuf::from("common").join("deep").join("screens.rpy"),
                PathBuf::from("script.rpy"),
            ],
            "export joins these onto the game folder, so an absolute path would escape it"
        );
    }

    #[tokio::test]
    async fn copy_keeps_the_nesting_and_asks_about_game_relative_paths() {
        let sandbox = tempfile::tempdir().expect("a temp folder");
        let from = sandbox.path().join("from");
        let to = sandbox.path().join("to");
        let wanted = PathBuf::from("data").join("deep").join("Map001.json");
        fs::create_dir_all(from.join("data").join("deep")).unwrap();
        fs::create_dir_all(from.join("audio")).unwrap();
        fs::write(from.join(&wanted), "kept").unwrap();
        fs::write(from.join("audio").join("theme.ogg"), "skipped").unwrap();

        let copied = copy(&from, &to, |at| at.starts_with("data")).await.unwrap();

        assert_eq!(copied, 1);
        assert_eq!(
            fs::read_to_string(to.join(&wanted)).unwrap(),
            "kept",
            "prepare reads the copy at the same relative spot it saw in the game"
        );
        assert!(!to.join("audio").exists());
    }

    #[tokio::test]
    async fn a_missing_folder_counts_as_empty() {
        let sandbox = tempfile::tempdir().expect("a temp folder");

        assert!(
            accounted(&sandbox.path().join("nowhere"))
                .await
                .unwrap()
                .is_empty(),
            "a store with no backups yet is empty, not broken"
        );
    }
}
