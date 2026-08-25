use crate::engine::{Offer, TranslationUnit};
use crate::job::Marking;
use crate::scope::{Scope, key, leaf};
use crate::service::locate::{Game, files_under, open_game};
use crate::service::scan::{Counts, parse_file, unlocked_in};
use crate::service::seek::{Seeking, looking_for};
use crate::store::FileState;
use anyhow::{Result, anyhow};
use regex::Regex;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::slice;
use std::sync::{Mutex, MutexGuard};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Show {
    #[default]
    All,
    Translated,
    Untranslated,
}

impl Show {
    fn keeps(self, done: bool) -> bool {
        match self {
            Self::All => true,
            Self::Translated => done,
            Self::Untranslated => !done,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Only {
    #[default]
    Yours,
    Asked,
    Listed,
}

impl Only {
    fn keeps(self, offer: Offer) -> bool {
        match self {
            Self::Yours => offer.unlocked(),
            Self::Asked => offer.asked(),
            Self::Listed => matches!(offer, Offer::Listed),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Sift {
    pub show: Show,
    pub only: Only,
    pub needle: String,
    pub how: Seeking,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub file: String,
    pub name: String,
    pub id: u32,
    pub source: String,
    pub translation: Option<String>,
    pub offer: Offer,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Window {
    pub from: u32,
    pub kept: u32,
    pub counts: Counts,
    pub lines: Vec<Entry>,
}

struct Spot {
    file: u32,
    id: u32,
    long: u32,
    done: bool,
}

#[derive(Default)]
struct Index {
    scope: String,
    needle: String,
    how: Seeking,
    only: Only,
    files: Vec<PathBuf>,
    spots: Vec<Spot>,
}

struct Take {
    from: u32,
    kept: u32,
    counts: Counts,
    want: Vec<(PathBuf, Vec<u32>)>,
}

impl Index {
    fn answers(&self, scope: &Scope, sift: &Sift) -> bool {
        self.scope == scope.as_str()
            && self.needle == sift.needle
            && self.how == sift.how
            && self.only == sift.only
    }

    fn counts(&self) -> Counts {
        let translated = self.spots.iter().filter(|one| one.done).count() as u32;

        Counts {
            total: self.spots.len() as u32,
            translated,
            untranslated: self.spots.len() as u32 - translated,
        }
    }

    fn kept(&self, show: Show) -> impl Iterator<Item = &Spot> {
        self.spots.iter().filter(move |spot| show.keeps(spot.done))
    }

    fn take(&self, show: Show, from: u32, count: u32) -> Take {
        let kept = self.kept(show).count() as u32;
        let from = from.min(kept.saturating_sub(1));

        let mut want: Vec<(PathBuf, Vec<u32>)> = Vec::new();

        for spot in self
            .kept(show)
            .skip(from as usize)
            .take(count.max(1) as usize)
        {
            let file = &self.files[spot.file as usize];

            match want.last_mut() {
                Some((last, ids)) if last == file => ids.push(spot.id),
                _ => want.push((file.clone(), vec![spot.id])),
            }
        }

        Take {
            from,
            kept,
            counts: self.counts(),
            want,
        }
    }

    fn place(&self, show: Show, file: u32, id: u32) -> Option<u32> {
        self.kept(show)
            .position(|spot| spot.file == file && spot.id == id)
            .map(|at| at as u32)
    }

    fn named(&self, source: &Path, wanted: &str) -> Option<u32> {
        self.files
            .iter()
            .position(|one| key(source, one) == wanted)
            .map(|at| at as u32)
    }

    fn mark(&mut self, file: &Path, ids: &[u32], done: bool) {
        let Some(at) = self.files.iter().position(|one| one == file) else {
            return;
        };

        let at = at as u32;
        for spot in &mut self.spots {
            if spot.file == at && ids.contains(&spot.id) {
                spot.done = done;
            }
        }
    }
}

#[derive(Default)]
pub struct Sheets(Mutex<Option<Index>>);

impl Marking for Sheets {
    fn filled(&self, file: &Path, ids: &[u32]) {
        let mut kept = self.slot();
        let Some(one) = kept.as_mut() else { return };

        match one.needle.is_empty() {
            true => one.mark(file, ids, true),
            false => *kept = None,
        }
    }
}

impl Sheets {
    pub fn forget(&self) {
        *self.slot() = None;
    }

    #[cfg(test)]
    pub fn held(&self) -> bool {
        self.slot().is_some()
    }

    #[cfg(test)]
    pub fn stand_in() -> Self {
        Self(Mutex::new(Some(Index::default())))
    }

    fn answers(&self, scope: &Scope, sift: &Sift) -> bool {
        self.slot()
            .as_ref()
            .is_some_and(|one| one.answers(scope, sift))
    }

    pub fn mark(&self, file: &Path, id: u32, done: bool) {
        if let Some(one) = self.slot().as_mut() {
            one.mark(file, slice::from_ref(&id), done);
        }
    }

    fn slot(&self) -> MutexGuard<'_, Option<Index>> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn ask<T>(&self, scope: &Scope, sift: &Sift, of: impl FnOnce(&Index) -> T) -> Option<T> {
        self.slot()
            .as_ref()
            .filter(|one| one.answers(scope, sift))
            .map(of)
    }
}

fn sieve(needle: &str, how: Seeking) -> Result<Option<Regex>> {
    if needle.is_empty() {
        return Ok(None);
    }

    looking_for(needle, how).map(Some)
}

pub struct Hit {
    pub id: u32,
    pub long: u32,
    pub done: bool,
}

pub async fn sifted(
    game: &Game,
    file: &Path,
    only: Only,
    sieve: Option<&Regex>,
) -> Result<Vec<Hit>> {
    let units = parse_file(&game.engine, file).await?;
    let state = game.store.load(file, &units).await?;

    Ok(hits_among(&units, &state, only, sieve))
}

pub fn hits_among(
    units: &[TranslationUnit],
    state: &FileState,
    only: Only,
    sieve: Option<&Regex>,
) -> Vec<Hit> {
    units
        .iter()
        .filter(|unit| only.keeps(unit.offer))
        .filter(|unit| match sieve {
            None => true,
            Some(sieve) => {
                sieve.is_match(&unit.text)
                    || state
                        .said()
                        .get(&unit.id)
                        .is_some_and(|told| sieve.is_match(told))
            }
        })
        .map(|unit| Hit {
            id: unit.id,
            long: unit.text.chars().count() as u32,
            done: state.said().contains_key(&unit.id),
        })
        .collect()
}

async fn index(game: &Game, scope: &Scope, sift: &Sift) -> Result<Index> {
    let files = files_under(&game.source, game.engine.as_ref(), slice::from_ref(scope)).await;

    let sieve = sieve(&sift.needle, sift.how)?;

    let mut spots = Vec::new();

    for (at, file) in files.iter().enumerate() {
        for hit in sifted(game, file, sift.only, sieve.as_ref()).await? {
            spots.push(Spot {
                file: at as u32,
                id: hit.id,
                long: hit.long,
                done: hit.done,
            });
        }
    }

    Ok(Index {
        scope: scope.as_str().to_string(),
        needle: sift.needle.clone(),
        how: sift.how,
        only: sift.only,
        files,
        spots: arranged(spots, sift.only),
    })
}

fn arranged(mut spots: Vec<Spot>, only: Only) -> Vec<Spot> {
    if only == Only::Listed {
        spots.sort_by_key(|spot| Reverse(spot.long));
    }

    spots
}

async fn ready(sheets: &Sheets, game: &Game, scope: &Scope, sift: &Sift) -> Result<()> {
    if sheets.answers(scope, sift) {
        return Ok(());
    }

    let fresh = index(game, scope, sift).await?;
    *sheets.slot() = Some(fresh);

    Ok(())
}

fn dropped(scope: &Scope) -> anyhow::Error {
    anyhow!("the lines of {scope} were dropped while they were being read")
}

struct Loaded {
    at: String,
    name: String,
    texts: HashMap<u32, (String, Offer)>,
    translations: BTreeMap<u32, String>,
}

async fn fill(game: &Game, taken: Take) -> Result<Window> {
    let mut loaded: HashMap<PathBuf, Loaded> = HashMap::new();

    for (file, _) in &taken.want {
        if loaded.contains_key(file) {
            continue;
        }

        let units = parse_file(&game.engine, file).await?;
        let state = game.store.load(file, &units).await?;

        let at = key(&game.source, file);
        let name = game.engine.shown(&leaf(&at)).into_owned();

        loaded.insert(
            file.clone(),
            Loaded {
                at,
                name,
                texts: units
                    .iter()
                    .map(|unit| (unit.id, (unit.text.clone(), unit.offer)))
                    .collect(),
                translations: state.into_said(),
            },
        );
    }

    let mut lines = Vec::new();

    for (file, ids) in taken.want {
        let one = &loaded[&file];

        for id in ids {
            let Some((source, offer)) = one.texts.get(&id) else {
                continue;
            };

            lines.push(Entry {
                file: one.at.clone(),
                name: one.name.clone(),
                id,
                source: source.clone(),
                translation: one.translations.get(&id).cloned(),
                offer: *offer,
            });
        }
    }

    Ok(Window {
        from: taken.from,
        kept: taken.kept,
        counts: taken.counts,
        lines,
    })
}

#[tracing::instrument(name = "lines.read", skip_all)]
pub async fn read(
    sheets: &Sheets,
    game_dir: &Path,
    scope: &Scope,
    sift: &Sift,
    from: u32,
    count: u32,
) -> Result<Window> {
    let game = open_game(game_dir).await?;
    ready(sheets, &game, scope, sift).await?;

    let taken = sheets
        .ask(scope, sift, |one| one.take(sift.show, from, count))
        .ok_or_else(|| dropped(scope))?;

    fill(&game, taken).await
}

pub async fn read_around(
    sheets: &Sheets,
    game_dir: &Path,
    scope: &Scope,
    sift: &Sift,
    file: &str,
    id: u32,
    count: u32,
) -> Result<Window> {
    let game = open_game(game_dir).await?;
    ready(sheets, &game, scope, sift).await?;

    let taken = sheets
        .ask(scope, sift, |one| {
            let at = one
                .named(&game.source, file)
                .and_then(|which| one.place(sift.show, which, id));

            one.take(
                sift.show,
                at.map_or(0, |at| at.saturating_sub(count / 2)),
                count,
            )
        })
        .ok_or_else(|| dropped(scope))?;

    fill(&game, taken).await
}

#[tracing::instrument(name = "lines.save", skip_all)]
pub async fn save(
    sheets: &Sheets,
    game_dir: &Path,
    scope: &Scope,
    id: u32,
    translation: Option<String>,
) -> Result<()> {
    let game = open_game(game_dir).await?;
    let file = scope.under(&game.source);

    let units = parse_file(&game.engine, &file).await?;
    let source = unlocked_in(&units, scope, id)?.text;

    let translation = translation.filter(|text| !text.trim().is_empty());

    if let Some(text) = &translation {
        game.engine
            .validate(&source, text)
            .map_err(|why| anyhow!(why))?;
    }

    let done = translation.is_some();

    game.store
        .amend(&file, &units, |state| match translation {
            None => state.dropped(id),
            Some(text) => state.settled(id, text),
        })
        .await?;

    sheets.mark(&file, id, done);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asked(needle: &str) -> Sift {
        Sift {
            needle: needle.to_string(),
            ..Sift::default()
        }
    }

    #[test]
    fn a_file_is_found_by_the_key_the_editor_asks_for_and_no_other() {
        let held = listing(&[false, false, false, false]);
        let source = Path::new("");

        assert_eq!(
            held.named(source, "b.sheet"),
            Some(1),
            "the reader asked to be taken to b, so b is where the window opens"
        );
        assert_eq!(
            held.named(source, "a.sheet"),
            Some(0),
            "and asking for the first file may not answer with whichever one differs from it"
        );
        assert_eq!(held.named(source, "c.sheet"), None, "no such file here");
    }

    fn listing(done: &[bool]) -> Index {
        Index {
            scope: "group".to_string(),
            needle: String::new(),
            how: Seeking::default(),
            only: Only::Yours,
            files: vec![PathBuf::from("a.sheet"), PathBuf::from("b.sheet")],
            spots: done
                .iter()
                .enumerate()
                .map(|(at, done)| Spot {
                    file: (at / 2) as u32 % 2,
                    id: at as u32,
                    long: 0,
                    done: *done,
                })
                .collect(),
        }
    }

    #[test]
    fn a_window_asks_only_for_the_files_it_shows() {
        let one = listing(&[false; 8]);
        let taken = one.take(Show::All, 2, 3);

        assert_eq!(taken.from, 2);
        assert_eq!(taken.kept, 8);
        assert_eq!(
            taken.want,
            vec![
                (PathBuf::from("b.sheet"), vec![2, 3]),
                (PathBuf::from("a.sheet"), vec![4]),
            ]
        );
    }

    #[test]
    fn choosing_what_to_show_narrows_the_window_and_what_it_asks_for() {
        let one = listing(&[true, false, true, false]);

        assert_eq!(one.take(Show::All, 0, 10).kept, 4);
        assert_eq!(one.take(Show::Translated, 0, 10).kept, 2);
        assert_eq!(one.take(Show::Untranslated, 0, 10).kept, 2);

        let taken = one.take(Show::Translated, 0, 10);
        assert_eq!(
            taken.want,
            vec![
                (PathBuf::from("a.sheet"), vec![0]),
                (PathBuf::from("b.sheet"), vec![2]),
            ]
        );
    }

    #[test]
    fn the_counts_stay_whole_whatever_is_shown() {
        let one = listing(&[true, false, true, true]);

        for show in [Show::All, Show::Translated, Show::Untranslated] {
            let counts = one.take(show, 0, 10).counts;

            assert_eq!(counts.total, 4);
            assert_eq!(counts.translated, 3);
            assert_eq!(counts.untranslated, 1);
        }
    }

    #[test]
    fn a_window_asked_for_past_the_end_lands_on_the_last_line() {
        let one = listing(&[false; 4]);
        let taken = one.take(Show::All, 900, 2);

        assert_eq!(taken.from, 3);
        assert_eq!(taken.want, vec![(PathBuf::from("b.sheet"), vec![3])]);
    }

    #[test]
    fn an_empty_group_asks_for_nothing() {
        let one = listing(&[]);
        let taken = one.take(Show::All, 0, 200);

        assert_eq!(taken.from, 0);
        assert_eq!(taken.kept, 0);
        assert!(taken.want.is_empty());
    }

    #[test]
    fn a_line_is_placed_by_where_it_sits_in_what_is_shown() {
        let one = listing(&[true, false, true, false]);

        assert_eq!(one.place(Show::All, 1, 2), Some(2));
        assert_eq!(
            one.place(Show::Translated, 1, 2),
            Some(1),
            "the untranslated line before it is not counted"
        );
        assert_eq!(
            one.place(Show::Untranslated, 1, 2),
            None,
            "a translated line is nowhere to be found among the untranslated"
        );
    }

    #[test]
    fn a_line_is_marked_in_the_file_that_holds_it_and_not_in_another_that_shares_the_id() {
        let sheets = Sheets::default();
        *sheets.slot() = Some(listing(&[false; 4]));

        sheets.mark(Path::new("a.sheet"), 1, true);
        assert_eq!(
            sheets.ask(&Scope::read("group").unwrap(), &asked(""), Index::counts),
            Some(Counts {
                total: 4,
                translated: 1,
                untranslated: 3,
            })
        );

        sheets.mark(Path::new("b.sheet"), 1, true);
        assert_eq!(
            sheets
                .ask(&Scope::read("group").unwrap(), &asked(""), Index::counts)
                .map(|counts| counts.translated),
            Some(1),
            "line 1 lives in a.sheet, so b.sheet has nothing to mark"
        );
    }

    #[test]
    fn a_run_writing_under_a_search_throws_the_list_away_rather_than_ticking_it_off() {
        let group = Scope::read("group").unwrap();

        let sheets = Sheets::default();
        *sheets.slot() = Some(listing(&[false; 4]));
        sheets.filled(Path::new("a.sheet"), &[0, 1]);

        assert_eq!(
            sheets
                .ask(&group, &asked(""), Index::counts)
                .map(|counts| counts.translated),
            Some(2),
            "with no search on, what belongs to the list cannot change, so a batch only ticks \
             off the lines it wrote"
        );

        let sheets = Sheets::default();
        *sheets.slot() = Some(Index {
            needle: "wonder".to_string(),
            ..listing(&[false; 4])
        });
        sheets.filled(Path::new("a.sheet"), &[0, 1]);

        assert!(
            !sheets.held(),
            "a search keeps a line for matching its translation as well as its source, so a line \
             the run just wrote can belong to the list where it did not before. Ticking off what \
             is already listed would leave that one out until the search changes"
        );
    }

    #[test]
    fn lines_kept_for_one_group_are_not_handed_to_another() {
        let sheets = Sheets::default();
        *sheets.slot() = Some(listing(&[false; 4]));

        let group = Scope::read("group").unwrap();
        assert!(sheets.ask(&group, &asked(""), Index::counts).is_some());
        assert!(
            sheets
                .ask(&group, &asked("wonder"), Index::counts)
                .is_none(),
            "a different needle means a different list"
        );
        assert!(
            sheets
                .ask(&Scope::read("other").unwrap(), &asked(""), Index::counts)
                .is_none()
        );

        sheets.forget();
        assert!(sheets.ask(&group, &asked(""), Index::counts).is_none());
    }

    #[test]
    fn no_pile_a_reader_can_open_ever_hands_them_a_line_the_format_locks() {
        for only in [Only::Yours, Only::Asked, Only::Listed] {
            assert!(
                !only.keeps(Offer::Locked),
                "{only:?} showed a line nothing may ever be written into, and a row a reader \
                 cannot act on is only in their way"
            );
        }

        for offer in [Offer::Asked, Offer::Listed] {
            let held = [Only::Asked, Only::Listed]
                .into_iter()
                .filter(|only| only.keeps(offer))
                .count();

            assert_eq!(
                held, 1,
                "{offer:?} belongs to exactly one pile, or a reader narrowing the list either \
                 sees a line twice or never sees it at all"
            );
        }

        assert!(Only::default().keeps(Offer::Asked));
        assert!(
            Only::default().keeps(Offer::Listed),
            "a line a rule only guessed about is still the reader's to overrule, so it belongs in \
             the list they work through"
        );
    }

    #[test]
    fn asking_for_the_listed_lines_puts_the_longest_first_and_leaves_the_rest_alone() {
        let long = "y".repeat(30);
        let lines = ["...", &long, "ui/bg.png", "//"];

        let both = |only: Only| {
            let spots: Vec<Spot> = lines
                .iter()
                .enumerate()
                .map(|(at, text)| Spot {
                    file: 0,
                    id: at as u32,
                    long: text.chars().count() as u32,
                    done: false,
                })
                .collect();

            Index {
                scope: "group".to_string(),
                needle: String::new(),
                how: Seeking::default(),
                only,
                files: vec![PathBuf::from("a.sheet")],
                spots: arranged(spots, only),
            }
            .take(Show::All, 0, 10)
            .want[0]
                .1
                .clone()
        };

        assert_eq!(
            both(Only::Yours),
            vec![0, 1, 2, 3],
            "left alone, a line sits where the game holds it"
        );
        assert_eq!(
            both(Only::Listed),
            vec![1, 2, 0, 3],
            "the longest line is where a translator's time goes furthest, so it is asked for \
             first and the shortest sinks to the end"
        );
    }

    #[test]
    fn an_empty_needle_sifts_nothing_so_every_line_still_counts() {
        assert!(
            sieve("", Seeking::default()).unwrap().is_none(),
            "a reader who cleared the box is not searching, and sifting on an empty needle would \
             count every line as a match"
        );
    }
}
