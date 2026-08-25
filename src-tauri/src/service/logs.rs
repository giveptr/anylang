use crate::progress::{Level, Source};
use crate::store;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, mpsc};
use std::thread;

const FILE: &str = "log.jsonl";
const KEEP: usize = 2_000;
const ROOM: u64 = 1 << 20;

static WRITER: LazyLock<mpsc::Sender<(PathBuf, LogEntry)>> = LazyLock::new(|| {
    let (send, receive) = mpsc::channel::<(PathBuf, LogEntry)>();

    thread::spawn(move || {
        while let Ok((root, entry)) = receive.recv() {
            let _ = add(&root, &entry);
        }
    });

    send
});

pub fn queue(root: PathBuf, entry: LogEntry) {
    let _ = WRITER.send((root, entry));
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    pub at: String,
    pub source: Source,
    pub message: String,
    pub level: Level,
}

fn path_for(game_dir: &Path) -> Result<PathBuf> {
    Ok(store::root_for(game_dir)?.join(FILE))
}

pub async fn load(game_dir: &Path) -> Result<Vec<LogEntry>> {
    let path = path_for(game_dir)?;

    let Some(raw) = store::read_if_there(&path).await? else {
        return Ok(Vec::new());
    };

    let lines: Vec<LogEntry> = raw
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();

    Ok(lines[lines.len().saturating_sub(KEEP)..].to_vec())
}

const STAMP: &str = "%Y-%m-%dT%H:%M:%SZ";

pub fn stamp() -> String {
    chrono::Utc::now().format(STAMP).to_string()
}

fn add(root: &Path, entry: &LogEntry) -> Result<()> {
    use std::io::Write;

    let path = root.join(FILE);
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("opening {}", path.display()))?;

    writeln!(file, "{}", serde_json::to_string(entry)?)
        .with_context(|| format!("writing {}", path.display()))?;

    if file.metadata().is_ok_and(|found| found.len() > ROOM) {
        trim(&path)?;
    }

    Ok(())
}

fn trim(path: &Path) -> Result<()> {
    let raw = fs::read_to_string(path)?;
    let lines: Vec<&str> = raw.lines().collect();
    if lines.len() <= KEEP {
        return Ok(());
    }

    let mut kept = lines[lines.len() - KEEP..].join("\n");
    kept.push('\n');

    let tmp = store::tmp_dir(path.parent().unwrap_or(Path::new("")));
    fs::create_dir_all(&tmp).with_context(|| format!("making {}", tmp.display()))?;

    let staging = tmp.join(FILE);
    fs::write(&staging, kept).with_context(|| format!("trimming {}", path.display()))?;
    fs::rename(&staging, path).with_context(|| format!("trimming {}", path.display()))
}

pub async fn forget(game_dir: &Path) -> Result<()> {
    let path = store::ensure_root(game_dir).await?.join(FILE);

    store::write_atomically(&path, "").await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(message: &str) -> LogEntry {
        LogEntry {
            at: "2026-08-11T00:00:00Z".to_string(),
            source: Source::Prepare,
            message: message.to_string(),
            level: Level::Info,
        }
    }

    #[tokio::test]
    async fn only_the_last_stretch_of_a_long_run_is_loaded_back() {
        let sandbox = tempfile::tempdir().expect("a temp folder");
        let game = sandbox.path();

        let root = store::ensure_root(game).await.expect("a store");
        for at in 0..KEEP + 500 {
            add(&root, &line(&at.to_string())).expect("a written log");
        }

        let back = load(game).await.expect("a read log");

        assert_eq!(back.len(), KEEP, "a burst must not grow the file forever");
        assert_eq!(
            back.first().map(|one| one.message.as_str()),
            Some("500"),
            "and what is dropped has to be the oldest, not the newest"
        );
    }

    #[tokio::test]
    async fn a_chatty_run_does_not_grow_the_file_on_disk_forever() {
        let sandbox = tempfile::tempdir().expect("a temp folder");
        let game = sandbox.path();

        let root = store::ensure_root(game).await.expect("a store");
        let said = "x".repeat(400);
        for at in 0..2_400 {
            add(&root, &line(&format!("{at} {said}"))).expect("a written log");
        }

        let raw = fs::read_to_string(root.join(FILE)).expect("the log file");
        let kept = raw.lines().count();
        assert!(
            kept < 2_400,
            "the file itself has to shrink, not only what load returns: {kept} line(s)"
        );
        assert!(
            kept >= KEEP,
            "trimming must never cost lines a reader is still promised: {kept} line(s)"
        );

        let back = load(game).await.expect("a read log");
        assert_eq!(back.len(), KEEP);
        assert_eq!(
            back.last().expect("a line").message,
            format!("2399 {said}"),
            "and the newest line survives the trim"
        );
    }

    #[test]
    fn a_line_is_stamped_with_the_day_it_happened() {
        let told = |seconds| {
            chrono::DateTime::from_timestamp(seconds, 0)
                .expect("a second inside the calendar")
                .format(STAMP)
                .to_string()
        };

        assert_eq!(told(0), "1970-01-01T00:00:00Z");
        assert_eq!(told(1_786_582_861), "2026-08-13T01:01:01Z");
        assert_eq!(
            told(1_709_164_800),
            "2024-02-29T00:00:00Z",
            "a leap day is a real day, and a stamp that skipped it would sort a run under the \
             wrong date"
        );

        let now = stamp();
        let laid: Vec<char> = now.chars().collect();

        assert_eq!(laid.len(), 20, "a stamp is one fixed width: {now}");
        assert!(
            laid[4] == '-'
                && laid[7] == '-'
                && laid[10] == 'T'
                && laid[13] == ':'
                && laid[16] == ':'
                && laid[19] == 'Z'
                && laid.iter().enumerate().all(|(at, one)| {
                    matches!(at, 4 | 7 | 10 | 13 | 16 | 19) || one.is_ascii_digit()
                }),
            "a reader sorts the log by this text and reads the day off it, so the year has to \
             come first and every field has to keep its width: {now}"
        );
    }

    #[tokio::test]
    async fn a_short_log_is_kept_whole() {
        let sandbox = tempfile::tempdir().expect("a temp folder");
        let game = sandbox.path();

        let root = store::ensure_root(game).await.expect("a store");
        add(&root, &line("one")).expect("a written log");
        add(&root, &line("two")).expect("more of the same log");

        let back = load(game).await.expect("a read log");
        assert_eq!(
            back.len(),
            2,
            "a second write adds to the first, not over it"
        );
        assert_eq!(back[1].message, "two");
    }

    #[tokio::test]
    async fn letting_a_log_go_leaves_nothing_for_the_next_run_to_read() {
        let sandbox = tempfile::tempdir().expect("a temp folder");
        let game = sandbox.path();

        let root = store::ensure_root(game).await.expect("a store");
        add(&root, &line("one")).expect("a written log");

        forget(game).await.expect("an emptied log");

        assert!(
            load(game).await.expect("a read log").is_empty(),
            "a reader who let a project go has to open the next one on a log of its own"
        );
    }
}
