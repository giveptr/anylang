use crate::engine::TranslationUnit;
use crate::hash::xxh3;
use crate::walk;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::SystemTime;
use tokio::fs;
use tokio::io::AsyncWriteExt;

const APP_DIR: &str = env!("CARGO_PKG_NAME");
const PROJECTS_DIR: &str = "projects";
const WEBVIEW_DIR: &str = "webview";
const TOOLS_DIR: &str = "tools";
const SOURCE_DIR: &str = "source";
const TEXT_DIR: &str = "text";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FileState {
    said: BTreeMap<u32, String>,
    #[serde(default)]
    asked: BTreeSet<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct Kept {
    lines: Vec<KeptLine>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct KeptLine {
    id: u32,
    was: String,
    text: String,
    #[serde(default)]
    asked: bool,
}

fn stale(unit: &TranslationUnit, line: &KeptLine) -> bool {
    !unit.offer.unlocked() || (!unit.offer.asked() && line.asked)
}

fn mark(unit: &TranslationUnit) -> String {
    xxh3(&unit.text)
}

impl FileState {
    pub fn said(&self) -> &BTreeMap<u32, String> {
        &self.said
    }

    pub fn into_said(self) -> BTreeMap<u32, String> {
        self.said
    }

    pub fn answered(&mut self, held: BTreeMap<u32, String>) {
        self.asked.extend(held.keys().copied());
        self.said.extend(held);
    }

    pub fn settled(&mut self, id: u32, text: String) {
        self.asked.remove(&id);
        self.said.insert(id, text);
    }

    pub fn dropped(&mut self, id: u32) {
        self.asked.remove(&id);
        self.said.remove(&id);
    }

    fn tidied(&mut self) {
        self.said.retain(|_, one| !one.trim().is_empty());

        let said = &self.said;
        self.asked.retain(|id| said.contains_key(id));
    }

    fn recall(&mut self, id: u32, line: &KeptLine) {
        if line.asked {
            self.asked.insert(id);
        }
        self.said.insert(id, line.text.clone());
    }

    pub fn missing(&self, units: &[TranslationUnit]) -> Vec<TranslationUnit> {
        units
            .iter()
            .filter(|unit| !self.said.contains_key(&unit.id))
            .cloned()
            .collect()
    }

    fn taken(kept: Kept, units: &[TranslationUnit]) -> Self {
        let mut out = Self::default();
        let mut used = vec![false; kept.lines.len()];

        let mut byid: HashMap<u32, usize> = HashMap::new();
        for (at, line) in kept.lines.iter().enumerate() {
            byid.entry(line.id).or_insert(at);
        }

        for unit in units {
            let Some(&at) = byid.get(&unit.id) else {
                continue;
            };
            if kept.lines[at].was != mark(unit) {
                continue;
            }

            used[at] = true;
            if stale(unit, &kept.lines[at]) {
                continue;
            }

            out.recall(unit.id, &kept.lines[at]);
        }

        let mut spare: HashMap<&str, VecDeque<usize>> = HashMap::new();
        for (at, line) in kept.lines.iter().enumerate() {
            if used[at] {
                continue;
            }

            spare.entry(line.was.as_str()).or_default().push_back(at);
        }

        for unit in units {
            if out.said.contains_key(&unit.id) {
                continue;
            }

            let Some(waiting) = spare.get_mut(mark(unit).as_str()) else {
                continue;
            };
            let Some(which) = waiting.iter().position(|&at| !stale(unit, &kept.lines[at])) else {
                continue;
            };
            let at = waiting.remove(which).expect("a line still waiting");

            out.recall(unit.id, &kept.lines[at]);
        }

        out
    }

    fn keeping(&self, units: &[TranslationUnit]) -> Kept {
        Kept {
            lines: units
                .iter()
                .filter_map(|unit| {
                    let text = self.said.get(&unit.id)?;

                    Some(KeptLine {
                        id: unit.id,
                        was: mark(unit),
                        text: text.clone(),
                        asked: self.asked.contains(&unit.id),
                    })
                })
                .collect(),
        }
    }
}

#[derive(Clone)]
pub struct Store {
    root: PathBuf,
    text: Option<PathBuf>,
}

static DESKS: LazyLock<Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn desk(at: &Path) -> Arc<tokio::sync::Mutex<()>> {
    let mut desks = DESKS.lock().expect("desk lock");
    desks.retain(|_, lock| Arc::strong_count(lock) > 1);

    desks
        .entry(at.to_path_buf())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

#[cfg(not(test))]
pub fn app_dir() -> Result<PathBuf> {
    Ok(dirs::data_local_dir()
        .context("no data folder for this platform")?
        .join(APP_DIR))
}

#[cfg(test)]
pub fn app_dir() -> Result<PathBuf> {
    thread_local! {
        static HOME: tempfile::TempDir = tempfile::Builder::new()
            .prefix(APP_DIR)
            .tempdir()
            .expect("a place for the tests to keep their stores");
    }

    Ok(HOME.with(|at| at.path().to_path_buf()))
}

pub fn webview_dir() -> Result<PathBuf> {
    Ok(app_dir()?.join(WEBVIEW_DIR))
}

pub fn tools_dir() -> Result<PathBuf> {
    Ok(app_dir()?.join(TOOLS_DIR))
}

pub fn root_for(game_dir: &Path) -> Result<PathBuf> {
    Ok(app_dir()?.join(PROJECTS_DIR).join(identity(game_dir)))
}

pub fn source_dir(root: &Path) -> PathBuf {
    root.join(SOURCE_DIR)
}

pub fn tmp_dir(root: &Path) -> PathBuf {
    root.join("tmp")
}

pub fn source_mark(root: &Path) -> PathBuf {
    root.join("source.from")
}

pub async fn ensure_root(game_dir: &Path) -> Result<PathBuf> {
    let root = root_for(game_dir)?;
    fs::create_dir_all(&root)
        .await
        .with_context(|| format!("creating {}", root.display()))?;

    Ok(root)
}

pub fn folder_name(language: &str) -> String {
    language.trim().to_lowercase()
}

fn one_folder(language: &str) -> Result<String> {
    let folder = folder_name(language);

    if folder.is_empty() {
        return Ok(folder);
    }

    let mut parts = Path::new(&folder).components();
    let lone = matches!(
        (parts.next(), parts.next()),
        (Some(Component::Normal(_)), None)
    );
    if !lone || folder.contains(['/', '\\']) {
        bail!("{language} cannot name a folder to keep the translations in");
    }

    Ok(folder)
}

impl Store {
    pub async fn open(game_dir: &Path, language: &str) -> Result<Self> {
        Self::at(root_for(game_dir)?, language).await
    }

    pub async fn at(root: PathBuf, language: &str) -> Result<Self> {
        let folder = one_folder(language)?;
        if folder.is_empty() {
            return Ok(Self { root, text: None });
        }

        let text = root.join(TEXT_DIR).join(folder);
        fs::create_dir_all(&text)
            .await
            .with_context(|| format!("creating {}", text.display()))?;

        Ok(Self {
            root,
            text: Some(text),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub async fn load(&self, source: &Path, units: &[TranslationUnit]) -> Result<FileState> {
        match self.path_for(source) {
            Some(at) => self.read(&at, units).await,
            None => Ok(FileState::default()),
        }
    }

    pub async fn amend<T>(
        &self,
        source: &Path,
        units: &[TranslationUnit],
        change: impl FnOnce(&mut FileState) -> T,
    ) -> Result<T> {
        let Some(at) = self.path_for(source) else {
            bail!("no language is picked, so there is nowhere to keep a translation");
        };
        let desk = desk(&at);
        let _held = desk.lock().await;

        let mut state = self.read(&at, units).await?;
        let out = change(&mut state);

        write_atomically(&at, serde_json::to_vec(&state.keeping(units))?).await?;

        Ok(out)
    }

    async fn read(&self, at: &Path, units: &[TranslationUnit]) -> Result<FileState> {
        let Some(raw) = read_if_there(at).await? else {
            return Ok(FileState::default());
        };

        let kept = serde_json::from_str::<Kept>(&raw)
            .with_context(|| format!("{} is not readable. Fix or delete it", at.display()))?;

        let mut state = FileState::taken(kept, units);
        state.tidied();

        Ok(state)
    }

    pub async fn forget(&self, source: &Path) -> Result<bool> {
        let Some(path) = self.path_for(source) else {
            return Ok(false);
        };
        let desk = desk(&path);
        let _held = desk.lock().await;

        walk::removed(&path).await
    }

    pub fn path_for(&self, source: &Path) -> Option<PathBuf> {
        let name = source
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        Some(self.text.as_ref()?.join(format!(
            "{name}-{}.json",
            xxh3(source.to_string_lossy().as_bytes())
        )))
    }
}

pub async fn read_if_there(at: &Path) -> Result<Option<String>> {
    match fs::read_to_string(at).await {
        Ok(raw) => Ok(Some(raw)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("reading {}", at.display())),
    }
}

pub type Stamp = (u64, Option<SystemTime>);

pub fn stamped(found: std::io::Result<std::fs::Metadata>) -> Stamp {
    match found {
        Ok(found) => (found.len(), found.modified().ok()),
        Err(_) => (0, None),
    }
}

pub fn stamp_of(at: &Path) -> Stamp {
    stamped(std::fs::metadata(at))
}

pub async fn write_atomically(path: &Path, bytes: impl AsRef<[u8]>) -> Result<()> {
    static TICKET: AtomicU64 = AtomicU64::new(0);

    let temporary = path.with_extension(format!(
        "tmp{}-{}",
        process::id(),
        TICKET.fetch_add(1, Ordering::Relaxed)
    ));

    let written = async {
        let mut file = fs::File::create(&temporary).await?;
        file.write_all(bytes.as_ref()).await?;
        file.sync_all().await
    }
    .await;

    if let Err(error) = written {
        let _ = fs::remove_file(&temporary).await;
        return Err(error).with_context(|| format!("writing {}", temporary.display()));
    }

    if let Err(error) = fs::rename(&temporary, path).await {
        let _ = fs::remove_file(&temporary).await;
        return Err(error).with_context(|| format!("replacing {}", path.display()));
    }

    Ok(())
}

fn identity(game_dir: &Path) -> String {
    let absolute = dunce::canonicalize(game_dir).unwrap_or_else(|_| game_dir.to_path_buf());
    let name = absolute
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "game".to_string());

    format!("{name}-{}", xxh3(absolute.to_string_lossy().as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Offer;
    use tempfile::TempDir;

    fn held(id: u32, text: &str, offer: Offer) -> TranslationUnit {
        TranslationUnit {
            id,
            text: text.to_string(),
            offer,
        }
    }

    fn units(count: usize) -> Vec<TranslationUnit> {
        (0..count as u32)
            .map(|id| TranslationUnit {
                id,
                text: format!("line {id}"),
                offer: Default::default(),
            })
            .collect()
    }

    struct Sandbox {
        game: TempDir,
        store: Store,
    }

    impl Sandbox {
        async fn open() -> Self {
            let game = tempfile::tempdir().expect("a temp folder");
            let store = Store::at(game.path().join("state"), "japanese")
                .await
                .unwrap();

            Self { game, store }
        }

        fn at(&self, name: &str) -> PathBuf {
            self.game.path().join(name)
        }
    }

    #[test]
    fn a_rule_that_changed_takes_back_what_it_asked_for_and_leaves_the_rest() {
        let asked = [
            held(0, "\u{58c1}\u{7d19}", Offer::Asked),
            held(1, "\u{30ab}\u{30fc}\u{30bd}\u{30eb}", Offer::Asked),
        ];
        let mut state = FileState::default();
        state.answered(BTreeMap::from([
            (0, "the wallpaper".to_string()),
            (1, "the cursor".to_string()),
        ]));
        let kept = state.keeping(&asked);

        let now = [
            held(0, "\u{58c1}\u{7d19}", Offer::Listed),
            held(1, "\u{30ab}\u{30fc}\u{30bd}\u{30eb}", Offer::Asked),
        ];
        let read = FileState::taken(kept, &now);

        assert_eq!(
            read.said().get(&0),
            None,
            "the reader asked for this one and has since changed its mind, so the answer goes \
             back to nothing rather than being laid into the game under a rule that is gone"
        );
        assert_eq!(
            read.said().get(&1).map(String::as_str),
            Some("the cursor"),
            "a line still asked about keeps its answer"
        );
    }

    #[test]
    fn a_rule_that_changed_takes_its_answer_back_even_when_the_line_moved_up_the_file() {
        let heading = "\u{898b}\u{51fa}\u{3057}";
        let paper = "\u{58c1}\u{7d19}";
        let cursor = "\u{30ab}\u{30fc}\u{30bd}\u{30eb}";

        let asked = [
            held(0, heading, Offer::Asked),
            held(1, paper, Offer::Asked),
            held(2, cursor, Offer::Asked),
        ];
        let mut state = FileState::default();
        state.answered(BTreeMap::from([
            (0, "the heading".to_string()),
            (1, "the wallpaper".to_string()),
            (2, "the cursor".to_string()),
        ]));
        let kept = state.keeping(&asked);

        let now = [held(0, paper, Offer::Listed), held(1, cursor, Offer::Asked)];
        let read = FileState::taken(kept, &now);

        assert_eq!(
            read.said().get(&0),
            None,
            "one rule dropped a line above this one and another turned this one into a name the \
             engine looks up, and a line found again by what it says must not slip past the rule \
             that just took its answer away"
        );
        assert_eq!(
            read.said().get(&1).map(String::as_str),
            Some("the cursor"),
            "a line still asked about is found again by what it says however far it moved"
        );
    }

    #[test]
    fn a_line_the_format_turns_out_to_lock_loses_even_what_a_person_wrote_on_it() {
        let script = "const x = 20;";
        let listed = [held(0, script, Offer::Listed)];

        let mut state = FileState::default();
        state.settled(0, "a hand written script".to_string());
        let kept = state.keeping(&listed);

        assert_eq!(
            FileState::taken(kept.clone(), &listed)
                .said()
                .get(&0)
                .map(String::as_str),
            Some("a hand written script"),
            "while the reader was only guessing, what a person settled by hand is theirs to keep"
        );

        let locked = [held(0, script, Offer::Locked)];

        assert_eq!(
            FileState::taken(kept, &locked).said().get(&0),
            None,
            "once the format itself says this parameter holds code, the answer goes even though a \
             person wrote it, because nothing may reach a line the game reads as code"
        );
    }

    #[test]
    fn what_a_person_wrote_stays_theirs_however_often_the_file_is_written_out_again() {
        let paper = "\u{58c1}\u{7d19}";
        let asked = [held(0, paper, Offer::Asked)];

        let mut state = FileState::default();
        state.settled(0, "a name a person settled".to_string());

        for _ in 0..3 {
            state = FileState::taken(state.keeping(&asked), &asked);
        }

        assert_eq!(
            FileState::taken(state.keeping(&asked), &[held(0, paper, Offer::Listed)])
                .said()
                .get(&0)
                .map(String::as_str),
            Some("a name a person settled"),
            "every save writes the whole file out again, so working out who wrote a line from \
             what the rules say about it today relabels a person's work as the model's, and the \
             next turn of the rules throws it away"
        );
    }

    #[test]
    fn a_line_a_person_wrote_is_still_found_behind_one_the_reader_has_taken_back() {
        let paper = "\u{58c1}\u{7d19}";

        let both = [held(0, paper, Offer::Asked), held(1, paper, Offer::Listed)];
        let mut state = FileState::default();
        state.answered(BTreeMap::from([(0, "the wallpaper".to_string())]));
        state.settled(1, "a name a person settled".to_string());
        let kept = state.keeping(&both);

        let now = [held(5, paper, Offer::Listed)];
        let read = FileState::taken(kept, &now);

        assert_eq!(
            read.said().get(&5).map(String::as_str),
            Some("a name a person settled"),
            "two lines said the same thing, one the reader asked about and one a person wrote by \
             hand, and the one the reader has since changed its mind about must not stand in \
             front of the other and hide it"
        );
    }

    #[test]
    fn a_line_already_settled_is_not_asked_for_or_billed_a_second_time() {
        let all = units(3);
        let mut state = FileState::default();
        state.settled(1, "Second".to_string());

        let left = state.missing(&all);
        assert_eq!(left.len(), 2);
        assert_eq!(left[0].id, 0);
        assert_eq!(
            left[1].id, 2,
            "what is missing is what the next run pays for, so counting a settled line among \
             them asks for it and bills for it twice"
        );
    }

    #[tokio::test]
    async fn a_game_and_the_webview_never_write_into_each_other() {
        let game = tempfile::tempdir().expect("a temp folder");
        let root = root_for(game.path()).unwrap();
        let webview = webview_dir().unwrap();

        assert!(
            root.starts_with(app_dir().unwrap().join(PROJECTS_DIR)),
            "a game named after one of the webview's own folders would otherwise stand beside it, \
             and a reader looking for their translations would be reading a browser cache"
        );
        assert!(!root.starts_with(&webview));
        assert!(!webview.starts_with(app_dir().unwrap().join(PROJECTS_DIR)));
    }

    #[tokio::test]
    async fn two_games_sharing_a_name_get_their_own_folder() {
        let sandbox = tempfile::tempdir().expect("a temp folder");
        let first = sandbox.path().join("a").join("game");
        let second = sandbox.path().join("b").join("game");
        fs::create_dir_all(&first).await.unwrap();
        fs::create_dir_all(&second).await.unwrap();

        assert_ne!(
            identity(&first),
            identity(&second),
            "two games called the same thing sharing one store would hand each other's \
             translations back and overwrite each other's backups"
        );
    }

    #[tokio::test]
    async fn a_replace_that_fails_leaves_no_temporary_in_the_game_folder() {
        let at = tempfile::tempdir().expect("a temp folder");
        let taken = at.path().join("Map001.json");
        std::fs::create_dir(&taken).expect("renaming onto a folder has to fail");

        assert!(write_atomically(&taken, b"translated").await.is_err());

        let left: Vec<String> = std::fs::read_dir(at.path())
            .expect("readable")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| name != "Map001.json")
            .collect();

        assert!(
            left.is_empty(),
            "a half-written export must not litter the game folder: {left:?}"
        );
    }

    #[tokio::test]
    async fn translations_survive_the_run_that_made_them() {
        let sandbox = Sandbox::open().await;
        let source = sandbox.at("script.rpy");
        let all = units(1);

        sandbox
            .store
            .amend(&source, &all, |state| {
                state.settled(0, "First".to_string());
            })
            .await
            .unwrap();

        let reopened = Store::at(sandbox.game.path().join("state"), "japanese")
            .await
            .unwrap();
        let loaded = reopened.load(&source, &all).await.unwrap();
        assert_eq!(loaded.said().get(&0).map(String::as_str), Some("First"));
    }

    #[tokio::test]
    async fn one_line_changing_leaves_every_other_translation_in_place() {
        let sandbox = Sandbox::open().await;
        let source = sandbox.at("script.rpy");
        let before = units(3);

        sandbox
            .store
            .amend(&source, &before, |state| {
                for id in 0..3 {
                    state.answered(BTreeMap::from([(id, format!("done {id}"))]));
                }
            })
            .await
            .unwrap();

        let mut after = before.clone();
        after[1].text = "the game changed this one".to_string();

        let loaded = sandbox.store.load(&source, &after).await.unwrap();

        assert_eq!(
            loaded.said().len(),
            2,
            "a game update that touches one line may not cost the other two"
        );
        assert_eq!(loaded.said().get(&0).map(String::as_str), Some("done 0"));
        assert_eq!(loaded.said().get(&2).map(String::as_str), Some("done 2"));
        assert!(!loaded.said().contains_key(&1));
    }

    #[tokio::test]
    async fn a_line_that_moved_or_was_renumbered_keeps_its_translation() {
        let sandbox = Sandbox::open().await;
        let source = sandbox.at("script.rpy");
        let before = units(3);

        sandbox
            .store
            .amend(&source, &before, |state| {
                for id in 0..3 {
                    state.answered(BTreeMap::from([(id, format!("done {id}"))]));
                }
            })
            .await
            .unwrap();

        let after: Vec<TranslationUnit> = before
            .iter()
            .rev()
            .enumerate()
            .map(|(at, unit)| TranslationUnit {
                id: at as u32,
                text: unit.text.clone(),
                offer: Default::default(),
            })
            .collect();

        let loaded = sandbox.store.load(&source, &after).await.unwrap();

        assert_eq!(
            loaded.said().len(),
            3,
            "the same words keep their translation however the reader numbers them"
        );
        assert_eq!(loaded.said().get(&0).map(String::as_str), Some("done 2"));
        assert_eq!(loaded.said().get(&2).map(String::as_str), Some("done 0"));
    }

    #[test]
    fn a_language_that_cannot_name_a_folder_is_refused() {
        assert_eq!(
            one_folder("").unwrap(),
            "",
            "a game is prepared before a language is picked, so this one has to pass"
        );
        assert!(
            one_folder("../escape").is_err(),
            "a language is one folder name, never a path"
        );
        assert!(one_folder("ja/jp").is_err());
    }

    #[test]
    fn a_language_names_the_same_folder_however_it_was_typed() {
        assert_eq!(
            one_folder(" Japanese ").unwrap(),
            "japanese",
            "a reader who typed a stray space picks their work back up out of the folder they \
             left it in, so the name has to settle the same way every time"
        );
    }

    #[tokio::test]
    async fn a_blank_translation_counts_as_missing() {
        let sandbox = Sandbox::open().await;
        let source = sandbox.at("script.rpy");
        let all = units(2);

        sandbox
            .store
            .amend(&source, &all, |state| {
                state.settled(0, "  ".to_string());
                state.settled(1, "Second".to_string());
            })
            .await
            .unwrap();

        let loaded = sandbox.store.load(&source, &all).await.unwrap();
        assert_eq!(loaded.said().len(), 1);
        assert_eq!(loaded.missing(&all).len(), 1);
    }

    #[tokio::test]
    async fn two_hundred_writers_landing_at_once_are_each_read_by_the_next() {
        let sandbox = Sandbox::open().await;
        let source = sandbox.at("script.rpy");
        let all = units(200);

        let store = Arc::new(sandbox.store);
        let mut writing = Vec::new();

        for id in 0..200u32 {
            let store = store.clone();
            let source = source.clone();
            let all = all.clone();

            writing.push(tokio::spawn(async move {
                store
                    .amend(&source, &all, move |state| {
                        state.answered(BTreeMap::from([(id, format!("answer {id}"))]));
                    })
                    .await
                    .expect("a state file that always writes");
            }));
        }

        for one in writing {
            one.await.expect("no writer may be lost");
        }

        let loaded = store.load(&source, &all).await.unwrap();
        assert_eq!(
            loaded.said().len(),
            200,
            "every writer read what the last one wrote, so nothing was overwritten"
        );
        for id in 0..200u32 {
            assert_eq!(
                loaded.said().get(&id).map(String::as_str),
                Some(format!("answer {id}").as_str()),
                "a writer that read the source back instead of the answer would look like every \
                 line landing"
            );
        }
    }

    #[tokio::test]
    async fn two_files_with_the_same_name_do_not_collide() {
        let sandbox = Sandbox::open().await;

        let first = sandbox.store.path_for(Path::new("game/chapter1/a.rpy"));
        let second = sandbox.store.path_for(Path::new("game/chapter2/a.rpy"));

        assert_ne!(
            first, second,
            "a game may well name two scripts the same in two folders, and folding them onto \
             one file would show chapter one's answers inside chapter two"
        );
    }
}
