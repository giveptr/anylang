use crate::canvas::Canvas;
use crate::engine::pictures::Chosen;
use crate::engine::unity::seal::{self, Sealed};
use crate::engine::unity::settle::{self, Done, Loaded, Settling, Staged, settle};
use crate::engine::unity::{
    Known, Learning, assemblies_beside, assembly, catalog, container_kind, data_dir, dotnet, fonts,
    holder_of, localization, mono_behaviour, opened, pictures, text_asset,
};
use crate::engine::{Install, sheet};
use crate::scope::{key, slashed};
use crate::{backup, walk};
use anyhow::{Result, bail};
use futures::future::BoxFuture;
use std::collections::{BTreeMap, BTreeSet};
use std::mem;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::Instrument;

struct Rewritten {
    size: u32,
    holder: String,
    live: PathBuf,
    pieces: usize,
    pictures: usize,
    faces: Vec<String>,
    sealed: Vec<PathBuf>,
}

type Pile = BTreeMap<String, Vec<(String, PathBuf)>>;

struct Taken {
    bytes: Vec<u8>,
    loaded: Loaded,
    recorded: Option<u32>,
    staged: Staged,
}

#[derive(Default)]
struct Tally {
    lines: usize,
    pieces: usize,
    pictures: usize,
    faces: BTreeMap<String, usize>,
    reverted: u32,
    refused: BTreeMap<String, usize>,
}

impl Tally {
    fn counted(&self) -> String {
        let mut held = Vec::new();
        if self.lines > 0 {
            held.push(format!("{} line(s) in the game's code", self.lines));
        }
        if self.pieces > 0 {
            held.push(format!("{} piece(s) of text", self.pieces));
        }
        if self.pictures > 0 {
            held.push(format!("{} picture(s)", self.pictures));
        }
        let faces: usize = self.faces.values().sum();
        if faces > 0 {
            held.push(format!("{faces} font(s)"));
        }

        match held.split_last() {
            None => "nothing".to_string(),
            Some((last, [])) => last.clone(),
            Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
        }
    }

    fn note_faces(&mut self, faces: &[String]) {
        for name in faces {
            *self.faces.entry(name.clone()).or_default() += 1;
        }
    }

    fn forget_faces(&mut self, faces: &[String]) {
        for name in faces {
            if let Some(many) = self.faces.get_mut(name) {
                *many = many.saturating_sub(1);
            }
        }

        self.faces.retain(|_, many| *many > 0);
    }
}

struct Writing<'a> {
    at: Install<'a>,
    drawn: BTreeMap<String, Chosen>,
    data: PathBuf,
    assets: Pile,
    behaviours: Pile,
    assemblies: Pile,
    known: Arc<Known>,
    picked: Arc<fonts::Chosen>,
    staged_root: PathBuf,
    books: Vec<PathBuf>,
    older_only: bool,
    tally: Tally,
    every_bundle: Vec<u32>,
    changed: Vec<Rewritten>,
    pending: Vec<backup::Pending>,
    wrote: BTreeSet<PathBuf>,
    handed: BTreeSet<String>,
    originals: BTreeMap<PathBuf, Taken>,
}

pub fn run<'a>(at: Install<'a>) -> BoxFuture<'a, Result<()>> {
    Box::pin(
        async move {
            let listing = without_leftovers(at.game_dir, walk::relative(at.game_dir).await).await;
            let mut writing = Writing::open(at, &listing).await?;

            if let Err(why) = writing.stage_all(&listing).await {
                writing.give_up().await;
                return Err(why);
            }

            writing.land().await?;
            writing.put_back_the_rest().await?;

            writing.report()
        }
        .instrument(tracing::info_span!("unity.install")),
    )
}

async fn without_leftovers(game_dir: &Path, listing: Vec<PathBuf>) -> Vec<PathBuf> {
    let (parts, rest): (Vec<PathBuf>, Vec<PathBuf>) =
        listing.into_iter().partition(|at| backup::is_part(at));

    for at in parts {
        if rest.contains(&at.with_extension("")) {
            let _ = tokio::fs::remove_file(game_dir.join(at)).await;
        }
    }

    rest
}

fn any_drawn(at: &Install<'_>, drawn: &BTreeMap<String, Chosen>, holder: &str) -> bool {
    at.pictures.filled().into_iter().any(|(key, from)| {
        pictures::holder_in(key) == Some(holder)
            && pictures::named_in(key).is_some()
            && drawn.contains_key(from)
    })
}

async fn take_in_alone(
    learning: &mut Learning,
    assemblies: &dotnet::Assemblies,
    holder: &str,
    reading: &Path,
) {
    let inside = holder.to_string();
    let at = reading.to_path_buf();
    let found = tokio::task::spawn_blocking(move || {
        opened::harvested_from(&inside, &at)
            .map(|held| held.containers)
            .unwrap_or_default()
    })
    .await
    .unwrap_or_default();

    for one in &found {
        learning.take_in(one, assemblies);
    }
}

#[allow(clippy::too_many_arguments)]
async fn learned(
    at: &Install<'_>,
    listing: &[PathBuf],
    data: &Path,
    assets: &Pile,
    behaviours: &Pile,
    tables_staged: bool,
    picked: &fonts::Chosen,
    drawn: &BTreeMap<String, Chosen>,
) -> Result<(Known, BTreeMap<PathBuf, Taken>, Vec<u32>)> {
    let assemblies = assemblies_beside(at.game_dir).await;
    let mut learning = Learning::default();
    let mut originals = BTreeMap::new();
    let mut every_bundle = Vec::new();

    for relative in listing {
        if assembly::ours(relative) {
            continue;
        }

        let live = at.game_dir.join(relative);
        let reading = backup::original_at(at.store, at.game_dir, &live).await?;
        let kept = reading == backup::slot(at.store, at.game_dir, &live)?;
        let holder = holder_of(relative, Some(data));

        let (bundled, length) = match container_kind(&reading).await {
            Ok(Some(kind)) => kind,
            Ok(None) => continue,
            Err(why) => {
                at.progress.warn(at.doing, &format!("{holder}: {why:#}"));
                continue;
            }
        };

        let recorded = match bundled {
            true => match u32::try_from(length) {
                Ok(size) => {
                    every_bundle.push(size);
                    Some(size)
                }
                Err(_) => {
                    at.progress.warn(
                        at.doing,
                        &format!(
                            "{holder}: too big for its catalog record, so its check cannot be \
                             lifted, leaving it untouched"
                        ),
                    );
                    take_in_alone(&mut learning, &assemblies, &holder, &reading).await;
                    continue;
                }
            },
            false => None,
        };

        let staged = Staged {
            assets: everything_under(assets, &holder),
            behaviours: everything_under(behaviours, &holder),
        };
        let wanted = !staged.is_empty()
            || kept
            || tables_staged
            || !picked.is_empty()
            || any_drawn(at, drawn, &holder);

        if !wanted {
            take_in_alone(&mut learning, &assemblies, &holder, &reading).await;
            continue;
        }

        let bytes = match tokio::fs::read(&reading).await {
            Ok(bytes) => bytes,
            Err(why) => {
                at.progress.warn(at.doing, &format!("{holder}: {why}"));
                continue;
            }
        };

        let inside = holder.clone();
        let Some((bytes, loaded)) = tokio::task::spawn_blocking(move || {
            settle::load(&bytes, &inside).map(|loaded| (bytes, loaded))
        })
        .await?
        else {
            at.progress.warn(
                at.doing,
                &format!("{holder}: this container cannot be rebuilt, so it is left untouched"),
            );
            take_in_alone(&mut learning, &assemblies, &holder, &reading).await;
            continue;
        };

        match &loaded {
            Loaded::Bundled { nodes, .. } => {
                for (_, one) in nodes {
                    learning.take_in(one, &assemblies);
                }
            }
            Loaded::Alone(one) => learning.take_in(one, &assemblies),
        }

        originals.insert(
            relative.clone(),
            Taken {
                bytes,
                loaded,
                recorded,
                staged,
            },
        );
    }

    Ok((learning.done(assemblies), originals, every_bundle))
}

impl<'a> Writing<'a> {
    async fn open(at: Install<'a>, listing: &[PathBuf]) -> Result<Self> {
        let data = data_dir(at.game_dir)
            .and_then(|found| found.strip_prefix(at.game_dir).ok().map(Path::to_path_buf))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "{}: this no longer holds a Unity data folder, so nothing here can be matched \
                     to what was read out of it",
                    at.game_dir.display()
                )
            })?;

        let staged_root = at.staged.to_path_buf();
        let picked = Arc::new(fonts::chosen(&at).await?);

        let drawn = at.chosen().await;
        let books = all_called(listing, at.game_dir, catalog::FILE);
        let older_only =
            books.is_empty() && !all_called(listing, at.game_dir, catalog::OLDER).is_empty();

        let assets = piles_under(&at.staged.join(text_asset::NAME)).await;
        let behaviours = piles_under(&at.staged.join(mono_behaviour::NAME)).await;
        let assemblies = piles_under(&at.staged.join(assembly::NAME)).await;
        let tables_staged = holds_tables(&staged_root);

        let (known, originals, every_bundle) = learned(
            &at,
            listing,
            &data,
            &assets,
            &behaviours,
            tables_staged,
            &picked,
            &drawn,
        )
        .await?;

        Ok(Self {
            drawn,
            data,
            assets,
            behaviours,
            assemblies,
            known: Arc::new(known),
            picked,
            staged_root,
            books,
            older_only,
            at,
            tally: Tally::default(),
            every_bundle,
            changed: Vec::new(),
            pending: Vec::new(),
            wrote: BTreeSet::new(),
            handed: BTreeSet::new(),
            originals,
        })
    }

    async fn stage_all(&mut self, listing: &[PathBuf]) -> Result<()> {
        for relative in listing {
            self.one(relative).await?;
        }

        self.lift().await
    }

    async fn give_up(&mut self) {
        backup::let_all_go(mem::take(&mut self.pending)).await;
    }

    async fn land(&mut self) -> Result<()> {
        let wrote = backup::land_all(
            self.at.store,
            self.at.game_dir,
            mem::take(&mut self.pending),
        )
        .await?;
        self.wrote.extend(wrote);

        Ok(())
    }

    fn drawn_under(&mut self, holder: &str) -> BTreeMap<(usize, i64), Canvas> {
        let mut out = BTreeMap::new();
        let mut handed = Vec::new();

        for (key, at) in self.at.pictures.filled() {
            if pictures::holder_in(key) != Some(holder) {
                continue;
            }
            let Some(path_id) = pictures::named_in(key) else {
                continue;
            };
            let Some(held) = self.drawn.get(at) else {
                continue;
            };

            handed.push(key.to_string());
            out.insert((pictures::node_in(key), path_id), held.drawn.clone());
        }

        self.handed.extend(handed);

        out
    }

    fn warn(&self, holder: &str, why: &str) {
        self.at
            .progress
            .warn(self.at.doing, &format!("{holder}: {why}"));
    }

    fn refuse(&mut self, holder: &str, lost: usize, why: &str) {
        self.warn(holder, why);
        *self.tally.refused.entry(holder.to_string()).or_default() += lost;
    }

    async fn reseal(&mut self, holder: &str, live: &Path, sealed: &Sealed) -> Result<Vec<PathBuf>> {
        let found = seal::beside(self.at.store, self.at.game_dir, live, sealed).await;
        if found.is_empty() {
            return Ok(Vec::new());
        }

        let mut written = Vec::with_capacity(found.len());
        for one in found {
            self.pending
                .push(backup::stage(&one.at, one.body.into_bytes()).await?);
            written.push(one.at);
        }

        self.at.progress.info(
            self.at.doing,
            &format!(
                "check lifted beside {holder} in {}",
                written
                    .iter()
                    .filter_map(|at| at.file_name())
                    .map(|name| name.to_string_lossy())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        );

        Ok(written)
    }

    async fn one_assembly(
        &mut self,
        holder: &str,
        live: &Path,
        reading: &Path,
        kept: bool,
    ) -> Result<()> {
        let staged = everything_under(&self.assemblies, holder);
        if staged.is_empty() {
            if kept && backup::put_back(self.at.store, self.at.game_dir, live).await? {
                self.tally.reverted += 1;
            }

            return Ok(());
        }

        let mut lines: BTreeMap<String, String> = BTreeMap::new();
        for (stem, sheet) in &staged {
            let text = match tokio::fs::read_to_string(sheet).await {
                Ok(text) => text,
                Err(why) => {
                    self.warn(holder, &format!("{stem}: {why}"));
                    continue;
                }
            };

            match sheet::lines(&text) {
                Ok(found) => lines.extend(found),
                Err(why) => self.warn(holder, &format!("{why:#}")),
            }
        }

        let original = match tokio::fs::read(reading).await {
            Ok(bytes) => bytes,
            Err(why) => {
                self.warn(holder, &why.to_string());
                return Ok(());
            }
        };

        let held = lines.len();
        let done =
            tokio::task::spawn_blocking(move || assembly::put_back(&original, &lines)).await?;

        match done {
            Err(why) => self.refuse(holder, held, &format!("{why:#}")),
            Ok(None) => {
                if backup::put_back(self.at.store, self.at.game_dir, live).await? {
                    self.tally.reverted += 1;
                }
            }
            Ok(Some((bytes, pieces))) => {
                self.pending.push(backup::stage(live, bytes).await?);
                self.tally.lines += pieces;
            }
        }

        Ok(())
    }

    async fn one(&mut self, relative: &Path) -> Result<()> {
        let holder = holder_of(relative, Some(&self.data));
        let live = self.at.game_dir.join(relative);

        if assembly::ours(relative) {
            let slot = backup::slot(self.at.store, self.at.game_dir, &live)?;
            let reading = backup::original_at(self.at.store, self.at.game_dir, &live).await?;
            let kept = reading == slot;

            return self
                .one_assembly(&holder, &live, reading.as_path(), kept)
                .await;
        }

        let Some(Taken {
            bytes: original,
            loaded,
            recorded,
            staged,
        }) = self.originals.remove(relative)
        else {
            let lost = everything_under(&self.assets, &holder).len()
                + everything_under(&self.behaviours, &holder).len();
            if lost > 0 {
                self.refuse(
                    &holder,
                    lost,
                    "this container could not be read back, so what was staged for it stays out",
                );
            }
            return Ok(());
        };
        let bundled = recorded.is_some();
        let drawn = self.drawn_under(&holder);

        let root = self.staged_root.clone();
        let known = Arc::clone(&self.known);
        let picked = Arc::clone(&self.picked);
        let nearby = live
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.at.game_dir.to_path_buf());
        let beside = nearby.clone();
        let store = self.at.store.to_path_buf();
        let game_dir = self.at.game_dir.to_path_buf();
        let done = tokio::task::spawn_blocking(move || {
            settle(
                original,
                loaded,
                &Settling {
                    staged: &staged,
                    staged_root: &root,
                    known: &known,
                    picked: &picked,
                    pictures: &drawn,
                    folder: &beside,
                    store: &store,
                    game_dir: &game_dir,
                },
            )
        })
        .await?;

        match done {
            Done::Nothing => {
                if backup::put_back(self.at.store, self.at.game_dir, &live).await? {
                    self.tally.reverted += 1;
                }
            }
            Done::Refused { why, lost } => self.refuse(&holder, lost, &why),
            Done::Ready {
                bytes,
                beside: written,
                pieces,
                faces,
                pictures,
                sealed,
                dropped,
            } => {
                for why in dropped {
                    self.refuse(&holder, 1, &why);
                }

                if bundled && self.older_only {
                    self.refuse(
                        &holder,
                        pieces + pictures + faces.len(),
                        &format!(
                            "an AssetBundle listed in {}, and only the check in {} can be lifted \
                             so far, leaving it untouched",
                            catalog::OLDER,
                            catalog::FILE
                        ),
                    );

                    return Ok(());
                }

                self.pending.push(backup::stage(&live, bytes).await?);
                self.lay_beside(&nearby, written).await?;
                self.tally.pieces += pieces;
                self.tally.pictures += pictures;
                self.tally.note_faces(&faces);

                let sealed = match sealed {
                    Some(sealed) => self.reseal(&holder, &live, &sealed).await?,
                    None => Vec::new(),
                };

                if let Some(size) = recorded {
                    self.changed.push(Rewritten {
                        size,
                        holder,
                        live,
                        pieces,
                        pictures,
                        faces,
                        sealed,
                    });
                }
            }
        }

        Ok(())
    }

    async fn lay_beside(&mut self, folder: &Path, written: settle::Sidecars) -> Result<()> {
        for (name, held) in written {
            let at = folder.join(&name);
            let mut room = backup::original(self.at.store, self.at.game_dir, &at).await?;

            for one in &held {
                settle::laid_into(&mut room, one)?;
            }

            self.pending.push(backup::stage(&at, room).await?);
        }

        Ok(())
    }

    async fn lift(&mut self) -> Result<()> {
        if self.changed.is_empty() {
            return Ok(());
        }

        let lifted = lift_checks(&self.at, &self.books, &self.every_bundle, &self.changed).await;

        let (books, unsure) = match lifted {
            Ok(out) => out,
            Err(why) => {
                self.at.progress.warn(
                    self.at.doing,
                    &format!(
                        "no check was lifted ({why:#}), leaving every bundle on its own bytes"
                    ),
                );

                return self.drop_changed(|_| true).await;
            }
        };

        self.pending.splice(..0, books);

        if unsure.is_empty() {
            return Ok(());
        }

        self.at.progress.warn(
            self.at.doing,
            &format!(
                "a catalog names {} but its checks could not be found, so they stay on their own \
                 bytes",
                names_of(&unsure.iter().copied().collect::<Vec<_>>(), &self.changed)
            ),
        );

        self.drop_changed(|one| unsure.contains(&one.size)).await
    }

    async fn drop_changed(&mut self, dropping: impl Fn(&Rewritten) -> bool) -> Result<()> {
        let (dropped, kept): (Vec<Rewritten>, Vec<Rewritten>) = mem::take(&mut self.changed)
            .into_iter()
            .partition(|one| dropping(one));
        self.changed = kept;

        let mut giving_up = BTreeSet::new();
        for one in dropped {
            let held = one.pieces + one.pictures + one.faces.len();

            self.tally.pieces = self.tally.pieces.saturating_sub(one.pieces);
            self.tally.pictures = self.tally.pictures.saturating_sub(one.pictures);
            self.tally.forget_faces(&one.faces);
            *self.tally.refused.entry(one.holder).or_default() += held;
            giving_up.insert(one.live);
            giving_up.extend(one.sealed);
        }

        let mut left = Vec::new();
        for one in mem::take(&mut self.pending) {
            if giving_up.contains(one.at()) {
                one.let_go().await;
            } else {
                left.push(one);
            }
        }
        self.pending = left;

        for live in giving_up {
            if backup::put_back(self.at.store, self.at.game_dir, &live).await? {
                self.tally.reverted += 1;
            }
        }

        Ok(())
    }

    async fn put_back_the_rest(&mut self) -> Result<()> {
        self.tally.reverted += backup::put_back_the_rest(
            self.at.store,
            self.at.game_dir,
            |_| true,
            &self.wrote.iter().cloned().collect::<Vec<_>>(),
        )
        .await? as u32;

        Ok(())
    }

    fn audit_faces(&self) {
        let missed: Vec<&str> = self
            .picked
            .wanted()
            .filter(|name| !self.tally.faces.contains_key(*name))
            .collect();

        if missed.is_empty() {
            return;
        }

        self.at.progress.warn(
            self.at.doing,
            &format!(
                "no font in this game is called {}, so the file picked for it was never put in",
                missed.join(", ")
            ),
        );
    }

    fn audit_pictures(&self) {
        let missed: Vec<&str> = self
            .at
            .pictures
            .filled()
            .into_iter()
            .filter(|(key, at)| self.drawn.contains_key(*at) && !self.handed.contains(*key))
            .map(|(key, _)| key)
            .collect();

        if missed.is_empty() {
            return;
        }

        self.at.progress.warn(
            self.at.doing,
            &format!(
                "nothing in this game is filed under {}, so the picture(s) picked for them were \
                 never put in. Read the game again if it was updated",
                missed.join(", ")
            ),
        );
    }

    fn report(&self) -> Result<()> {
        self.audit_faces();
        self.audit_pictures();

        let mut said = Vec::new();
        if !self.wrote.is_empty() {
            said.push(format!(
                "{} written into {} game file(s)",
                self.tally.counted(),
                self.wrote.len()
            ));
        }
        if self.tally.reverted > 0 {
            said.push(format!("{} container(s) put back", self.tally.reverted));
        }
        if !said.is_empty() {
            self.at.progress.info(self.at.doing, &said.join(", "));
        }

        if self.tally.refused.is_empty() {
            return Ok(());
        }

        let listed = self
            .tally
            .refused
            .iter()
            .map(|(holder, lost)| format!("{lost} item(s) in {holder}"))
            .collect::<Vec<_>>()
            .join(", ");

        if self.tally.lines == 0
            && self.tally.pieces == 0
            && self.tally.pictures == 0
            && self.tally.faces.is_empty()
        {
            bail!("the text is translated and saved, but none of it reached the game: {listed}");
        }

        bail!(
            "{} reached the game, but these did not: {listed}. What was written is in place. Use \
             Restore original files to take it back out.",
            self.tally.counted()
        );
    }
}

async fn lift_checks(
    at: &Install<'_>,
    books: &[PathBuf],
    every_bundle: &[u32],
    changed: &[Rewritten],
) -> Result<(Vec<backup::Pending>, BTreeSet<u32>)> {
    let mut staged = Vec::new();

    match lift_each(at, books, every_bundle, changed, &mut staged).await {
        Ok(unsure) => Ok((staged, unsure)),
        Err(why) => {
            for one in staged {
                one.let_go().await;
            }

            Err(why)
        }
    }
}

async fn lift_each(
    at: &Install<'_>,
    books: &[PathBuf],
    every_bundle: &[u32],
    changed: &[Rewritten],
    staged: &mut Vec<backup::Pending>,
) -> Result<BTreeSet<u32>> {
    let wanted: Vec<u32> = changed.iter().map(|one| one.size).collect();
    let mut covered: BTreeSet<u32> = BTreeSet::new();
    let mut unsure: BTreeSet<u32> = BTreeSet::new();

    for book in books {
        backup::keep(at.store, at.game_dir, book).await?;
        let original = tokio::fs::read(backup::slot(at.store, at.game_dir, book)?).await?;

        let lifted = catalog::lift(&original, every_bundle, &wanted)?;
        covered.extend(&lifted.lifted);
        unsure.extend(&lifted.unconfirmed);

        let Some(fresh) = lifted.catalog else {
            continue;
        };

        staged.push(backup::stage(book, fresh).await?);
        at.progress.info(
            at.doing,
            &format!(
                "check lifted in {} for {}",
                key(at.game_dir, book),
                names_of(&lifted.lifted, changed)
            ),
        );
    }

    unsure.retain(|size| !covered.contains(size));

    Ok(unsure)
}

fn names_of(sizes: &[u32], changed: &[Rewritten]) -> String {
    let mut seen = BTreeSet::new();
    let mut names = Vec::new();

    for size in sizes {
        if !seen.insert(*size) {
            continue;
        }

        let holders: Vec<String> = changed
            .iter()
            .filter(|one| one.size == *size)
            .map(|one| one.holder.clone())
            .collect();

        if holders.is_empty() {
            names.push(format!("a bundle of {size} byte(s)"));
        } else {
            names.extend(holders);
        }
    }

    names.join(", ")
}

fn all_called(listing: &[PathBuf], game_dir: &Path, wanted: &str) -> Vec<PathBuf> {
    listing
        .iter()
        .filter(|relative| relative.file_name().is_some_and(|name| name == wanted))
        .map(|relative| game_dir.join(relative))
        .collect()
}

fn holds_tables(staged_root: &Path) -> bool {
    staged_root.join(localization::NAME).is_dir()
}

fn everything_under(piles: &Pile, holder: &str) -> Vec<(String, PathBuf)> {
    let deeper = format!("{holder}/");

    piles
        .get(holder)
        .into_iter()
        .chain(
            piles
                .range(deeper.clone()..)
                .take_while(|(key, _)| key.starts_with(&deeper))
                .map(|(_, staged)| staged),
        )
        .flat_map(|staged| staged.iter().cloned())
        .collect()
}

async fn piles_under(root: &Path) -> Pile {
    let mut piles: Pile = BTreeMap::new();

    for relative in walk::relative(root).await {
        let Some(stem) = relative
            .file_stem()
            .map(|it| it.to_string_lossy().to_string())
        else {
            continue;
        };
        let Some(holder) = relative.parent().map(slashed) else {
            continue;
        };

        piles
            .entry(holder)
            .or_default()
            .push((stem, root.join(&relative)));
    }

    for staged in piles.values_mut() {
        staged.sort();
    }

    piles
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::fonts::{Drawn, Fonts};
    use crate::engine::pictures::Pictures;
    use crate::engine::unity::dotnet::{Assemblies, Shape};
    use crate::engine::unity::{Harvest, fake, layout, mono_script, serial};
    use crate::engine::{Swap, fonts as face, sheet};
    use crate::progress::{Heard, Progress, Quiet};
    use std::fs;

    struct Sandbox {
        dir: tempfile::TempDir,
        game: PathBuf,
        store: PathBuf,
        staged: PathBuf,
        container: PathBuf,
    }

    fn sandbox(assets: &[(i64, &str, &str)]) -> Sandbox {
        let dir = tempfile::tempdir().expect("a temp folder");
        let game = dir.path().join("game");
        let data = game.join("Fake_Data");
        fs::create_dir_all(&data).expect("a data folder");
        fs::write(game.join("Fake.x86_64"), []).expect("a player");

        let container = data.join("resources.assets");
        fs::write(&container, fake::forge(assets)).expect("a container");

        Sandbox {
            game,
            store: dir.path().join("store"),
            staged: dir.path().join("store").join("staged").join("japanese"),
            container,
            dir,
        }
    }

    impl Sandbox {
        async fn install(&self) -> Result<()> {
            self.install_drawing(&Fonts::default(), &Quiet).await
        }

        async fn install_drawing(&self, fonts: &Fonts, progress: &dyn Progress) -> Result<()> {
            run(Install::over(&self.game, &self.staged, &self.store)
                .sending(fonts)
                .heard_by(progress))
            .await
        }

        fn drawing(&self, name: &str, data: &[u8]) {
            let mut objects: Vec<(i64, i32, Vec<u8>)> = text_asset::scripts_in(
                &serial::open(&self.bytes(), "")
                    .expect("a container that opens")
                    .objects,
            )
            .into_iter()
            .map(|one| {
                (
                    one.path_id,
                    serial::TEXT_ASSET,
                    fake::a_text_asset(&one.stem, &one.body),
                )
            })
            .collect();

            objects.push((
                55,
                serial::FONT,
                fake::a_font(
                    name,
                    data,
                    16.0,
                    Drawn {
                        line: 16.0,
                        ascent: 12.0,
                        descent: -4.0,
                    },
                ),
            ));
            fs::write(&self.container, fake::forge_objects(&objects)).expect("a container");
        }

        fn pick(&self, name: &str, family: &str) -> Fonts {
            let at = self.dir.path().join(format!("{family}.ttf"));
            fs::write(&at, face::fake::called(family)).expect("a font to pick");

            Fonts {
                swaps: vec![Swap {
                    from: name.to_string(),
                    to: at.to_string_lossy().to_string(),
                }],
            }
        }

        fn face_named(&self, name: &str) -> Vec<u8> {
            let opened = serial::open(&self.bytes(), "").expect("a container that opens");

            opened
                .objects
                .iter()
                .find_map(|object| {
                    let face = fonts::face_of(object)?;

                    (face == name).then(|| fonts::shipped_in(object))?
                })
                .expect("the font asset is still there")
                .to_vec()
        }

        fn sheet_at(&self, stem: &str) -> PathBuf {
            self.staged
                .join(text_asset::NAME)
                .join("resources.assets")
                .join("line")
                .join(format!("{stem}.{}", sheet::SUFFIX))
        }

        fn with_assembly(&self, said: &[&str]) -> (PathBuf, Vec<u32>) {
            let built = assembly::forge::dll(said);
            let at = self
                .game
                .join("Fake_Data")
                .join("Managed")
                .join("Assembly-CSharp.dll");

            fs::create_dir_all(at.parent().expect("a folder")).expect("a managed folder");
            fs::write(&at, built.raw).expect("an assembly");

            (at, built.strings)
        }

        fn stage_assembly(&self, lines: &[(u32, &str)]) {
            let at = self
                .staged
                .join(assembly::NAME)
                .join("Managed")
                .join("Assembly-CSharp.dll")
                .join("Talker")
                .join(format!("0.{}", sheet::SUFFIX));

            fs::create_dir_all(at.parent().expect("a folder")).expect("a staged folder");
            let page = sheet::write(
                lines
                    .iter()
                    .map(|(spot, said)| (spot.to_string(), (*said).to_string())),
            )
            .expect("a sheet");

            fs::write(at, page).expect("a staged sheet");
        }

        fn stage(&self, stem: &str, said: &str) {
            let at = self.sheet_at(stem);
            let lines = [("line[1]".to_string(), said.to_string())];

            fs::create_dir_all(at.parent().expect("a holder")).expect("a staged folder");
            fs::write(at, sheet::write(lines).expect("a sheet")).expect("a staged file");
        }

        fn unstage(&self, stem: &str) {
            fs::remove_file(self.sheet_at(stem)).expect("a staged file to drop");
        }

        fn scripts(&self) -> BTreeMap<String, String> {
            let raw = fs::read(&self.container).expect("a container");
            text_asset::scripts_in(
                &serial::open(&raw, "")
                    .expect("a container that opens")
                    .objects,
            )
            .into_iter()
            .map(|one| (one.stem, one.body))
            .collect()
        }

        fn bytes(&self) -> Vec<u8> {
            fs::read(&self.container).expect("a container")
        }

        async fn parts(&self) -> Vec<PathBuf> {
            walk::relative(&self.game)
                .await
                .into_iter()
                .filter(|at| backup::is_part(at))
                .collect()
        }
    }

    const ONE: &str = "Peter\nShe tilted her head.\n\n";
    const TWO: &str = "Mary\nWait.\n\n";

    impl Sandbox {
        fn a_picture(&self, path_id: i64, wide: usize, high: usize) -> Vec<u8> {
            let pixels: Vec<u8> = (0..wide * high * 4)
                .map(|which| (which % 251) as u8)
                .collect();

            let mut objects: Vec<(i64, i32, Vec<u8>)> = text_asset::scripts_in(
                &serial::open(&self.bytes(), "")
                    .expect("a container that opens")
                    .objects,
            )
            .into_iter()
            .map(|one| {
                (
                    one.path_id,
                    serial::TEXT_ASSET,
                    fake::a_text_asset(&one.stem, &one.body),
                )
            })
            .collect();

            objects.push((
                path_id,
                serial::TEXTURE_2D,
                fake::a_texture(&fake::Drawn {
                    data: &pixels,
                    ..fake::drawn("Background", wide, high, 4)
                }),
            ));
            fs::write(&self.container, fake::forge_objects(&objects)).expect("a container");

            pixels
        }

        fn pick_picture(&self, key: &str, drawn: &Canvas) -> Pictures {
            let at = self.dir.path().join("picked.png");
            fs::write(&at, drawn.png().expect("a png")).expect("a picture to pick");

            Pictures {
                swaps: vec![Swap {
                    from: key.to_string(),
                    to: at.to_string_lossy().to_string(),
                }],
                ..Pictures::default()
            }
        }

        async fn install_pictures(&self, picked: &Pictures) -> Result<()> {
            run(Install::over(&self.game, &self.staged, &self.store).drawing(picked)).await
        }

        async fn install_pictures_heard(&self, picked: &Pictures, heard: &Heard) -> Result<()> {
            run(Install::over(&self.game, &self.staged, &self.store)
                .drawing(picked)
                .heard_by(heard))
            .await
        }

        fn picture_at(&self, path_id: i64) -> Canvas {
            let raw = self.bytes();
            let node = serial::open(&raw, "").expect("a container that opens");

            pictures::whole(
                &node,
                path_id,
                &pictures::Nearby {
                    held: &BTreeMap::new(),
                    folder: Path::new("."),
                    store: &self.store,
                    game_dir: &self.game,
                },
            )
            .expect("a picture that reads back")
        }
    }

    #[tokio::test]
    async fn a_picked_picture_lands_in_the_game_and_the_original_comes_back_when_it_is_dropped() {
        let sand = sandbox(&[(11, "one", ONE)]);
        let was = sand.a_picture(77, 8, 4);
        let before = sand.bytes();

        let mut fresh = sand.picture_at(77);
        fresh.pixels.iter_mut().for_each(|byte| *byte = 200);

        let picked = sand.pick_picture("resources.assets|77", &fresh);
        sand.install_pictures(&picked)
            .await
            .expect("an install that goes through");

        assert_ne!(sand.bytes(), before, "the container on disk was rewritten");
        assert_eq!(
            sand.picture_at(77).pixels,
            fresh.pixels,
            "what the reader picked is what the game now draws with"
        );
        assert_eq!(
            sand.scripts()["one"],
            ONE,
            "the text beside the picture is nobody's to touch"
        );

        sand.install_pictures(&Pictures::default())
            .await
            .expect("a second install with nothing picked");

        assert_eq!(
            sand.bytes(),
            before,
            "dropping the pick has to give the game back the container it shipped, byte for byte"
        );
        assert_eq!(sand.picture_at(77).pixels.len(), was.len());
    }

    #[tokio::test]
    async fn a_pick_naming_a_container_this_game_does_not_have_is_left_alone_and_said_out_loud() {
        let sand = sandbox(&[(11, "one", ONE)]);
        sand.a_picture(77, 8, 4);
        let before = sand.bytes();

        let mut fresh = sand.picture_at(77);
        fresh.pixels.iter_mut().for_each(|byte| *byte = 9);

        let heard = Heard::default();
        sand.install_pictures_heard(
            &sand.pick_picture("sharedassets9.assets|77", &fresh),
            &heard,
        )
        .await
        .expect("an install that goes through");

        assert_eq!(
            sand.bytes(),
            before,
            "a key belongs to one container, and a pick for a container that is not here may not \
             land on whatever object happens to share its number"
        );
        assert!(
            heard
                .warnings()
                .iter()
                .any(|said| said.contains("sharedassets9.assets|77")),
            "the reader picked a file and nothing came of it, so saying nothing leaves them \
             waiting on a change that is never coming: {:?}",
            heard.warnings()
        );
    }

    #[tokio::test]
    async fn a_line_hard_coded_in_a_script_is_written_into_the_assembly_and_can_be_taken_back_out()
    {
        let sand = sandbox(&[(11, "one", ONE)]);
        let (at, spots) = sand.with_assembly(&["\u{505c}\u{6b62}", "MusicVolume"]);
        let before = fs::read(&at).expect("the assembly");

        sand.stage_assembly(&[(spots[0], "Hold it right there")]);
        sand.install().await.expect("an install that goes through");

        let after = fs::read(&at).expect("the assembly");
        assert_ne!(after, before, "the assembly on disk was rewritten");

        let mut said: Vec<String> = assembly::take("Managed/Assembly-CSharp.dll", &after)
            .expect("the rewritten assembly still reads")
            .iter()
            .flat_map(|one| sheet::lines(&one.body).expect("a sheet as it was written"))
            .map(|(_, said)| said)
            .collect();
        said.sort();

        assert_eq!(
            said,
            ["Hold it right there", "MusicVolume"],
            "the translation is what the script now loads, and the line nobody staged is left as \
             the game wrote it"
        );

        fs::remove_dir_all(sand.staged.join(assembly::NAME)).expect("the staging goes");
        sand.install().await.expect("a second install");
        assert_eq!(
            fs::read(&at).expect("the assembly"),
            before,
            "with nothing staged the game gets its own assembly back"
        );
    }

    #[tokio::test]
    async fn a_picked_font_reaches_the_container_with_no_text_staged_at_all() {
        let sand = sandbox(&[(11, "one", ONE)]);
        sand.drawing("NotoSans", &face::fake::called("Noto Sans"));
        let before = sand.bytes();

        sand.install_drawing(&sand.pick("NotoSans", "Sarabun"), &Quiet)
            .await
            .expect("an install that goes through");

        assert_eq!(
            sand.face_named("NotoSans"),
            face::fake::called("Sarabun"),
            "a font is the whole of what a reader picked here: an export carrying no line of \
             text still has to carry the font, or picking one does nothing until something else \
             happens to be translated"
        );
        assert_eq!(
            sand.scripts()["one"],
            ONE,
            "and the text nobody staged is left exactly as the game wrote it"
        );

        sand.install().await.expect("a second install");
        assert_eq!(
            sand.bytes(),
            before,
            "letting the pick go has to hand the game back the font it shipped with, byte for byte"
        );
    }

    #[tokio::test]
    async fn a_pick_for_a_font_no_container_holds_is_said_out_loud_and_changes_nothing() {
        let sand = sandbox(&[(11, "one", ONE)]);
        sand.drawing("NotoSans", &face::fake::called("Noto Sans"));
        let before = sand.bytes();

        let heard = Heard::default();
        sand.install_drawing(&sand.pick("NoSuchFont", "Sarabun"), &heard)
            .await
            .expect("an install that goes through");

        assert_eq!(
            sand.bytes(),
            before,
            "a pick that matched nothing may not leave the game rewritten"
        );
        let told = "no font in this game is called NoSuchFont, so the file picked for it was \
                    never put in";

        assert_eq!(
            heard.warnings(),
            [told],
            "the reader picked a file and nothing came of it, so saying nothing would leave them \
             waiting on a change that is never coming"
        );
    }

    #[tokio::test]
    async fn a_font_and_the_text_beside_it_land_in_one_container_together() {
        let sand = sandbox(&[(11, "one", ONE), (22, "two", TWO)]);
        sand.drawing("NotoSans", &face::fake::called("Noto Sans"));

        sand.stage("one", "彼女は首をかたむけた。");
        sand.install_drawing(&sand.pick("NotoSans", "Sarabun"), &Quiet)
            .await
            .expect("an install that goes through");

        assert_eq!(sand.scripts()["one"], "Peter\n彼女は首をかたむけた。\n\n");
        assert_eq!(sand.scripts()["two"], TWO);
        assert_eq!(
            sand.face_named("NotoSans"),
            face::fake::called("Sarabun"),
            "the font and the text share one rewrite of one container, so a font that grew has \
             to leave every script after it findable at its new place"
        );
    }

    #[tokio::test]
    async fn staged_text_reaches_the_container_and_nothing_else_moves() {
        let sand = sandbox(&[(11, "one", ONE), (22, "two", TWO)]);
        let before = sand.bytes();

        sand.stage("one", "彼女は首をかたむけた。");
        sand.install().await.expect("an install that goes through");

        let back = sand.scripts();
        assert_eq!(back["one"], "Peter\n彼女は首をかたむけた。\n\n");
        assert_eq!(back["two"], TWO, "the asset nobody staged is untouched");
        assert_ne!(sand.bytes(), before);
    }

    #[tokio::test]
    async fn an_install_that_goes_through_leaves_nothing_staged_beside_the_game() {
        let sand = sandbox(&[(11, "one", ONE), (22, "two", TWO)]);

        sand.stage("one", "彼女は首をかたむけた。");
        sand.install().await.expect("an install that goes through");

        assert!(
            sand.parts().await.is_empty(),
            "every staged write either landed or was let go of"
        );
    }

    #[tokio::test]
    async fn a_part_left_by_a_killed_run_is_swept_and_never_read_as_a_container() {
        let sand = sandbox(&[(11, "one", ONE), (22, "two", TWO)]);
        let before = sand.bytes();

        let leftover = with_part(&sand.container);
        fs::write(&leftover, b"half a container").expect("a leftover");
        let sidecar = sand.game.join("Fake_Data").join("resources.assets.part");
        fs::write(&sidecar, b"the game's own").expect("a sidecar");
        let alone = sand.game.join("Fake_Data").join("stray.part");
        fs::write(&alone, b"not ours").expect("a stray");

        sand.install().await.expect("an install that goes through");

        assert_eq!(sand.bytes(), before, "the container nobody staged is whole");
        assert!(
            !leftover.exists(),
            "a part beside a file we install into belongs to a run that died"
        );
        assert!(
            sidecar.exists(),
            "a game file ending in .part beside a container is not staging and not ours"
        );
        assert!(
            alone.exists(),
            "a file of the game's own that happens to end in .part is not ours to delete"
        );
    }

    fn with_part(at: &Path) -> PathBuf {
        let mut name = at.as_os_str().to_owned();
        name.push(".stagedpart");

        PathBuf::from(name)
    }

    #[tokio::test]
    async fn a_container_goes_back_byte_for_byte_once_its_text_is_gone() {
        let sand = sandbox(&[(11, "one", ONE), (22, "two", TWO)]);
        let before = sand.bytes();

        sand.stage("one", "彼女は首をかたむけた。");
        sand.install().await.expect("first install");
        assert_ne!(sand.bytes(), before);

        sand.unstage("one");
        sand.install().await.expect("second install");

        assert_eq!(
            sand.bytes(),
            before,
            "clearing the text and exporting again has to undo the write, not leave it behind"
        );
    }

    #[tokio::test]
    async fn a_second_install_rebuilds_from_the_original_rather_than_the_last_write() {
        let sand = sandbox(&[(11, "one", ONE)]);
        let before = sand.bytes();

        sand.stage("one", "一回目。");
        sand.install().await.expect("first install");

        sand.stage("one", "二回目。");
        sand.install().await.expect("second install");

        assert_eq!(sand.scripts()["one"], "Peter\n二回目。\n\n");

        sand.unstage("one");
        sand.install().await.expect("third install");
        assert_eq!(
            sand.bytes(),
            before,
            "two exports must not compound: the backup stays the untouched original"
        );
    }

    #[tokio::test]
    async fn nothing_staged_leaves_a_game_nobody_touched_alone() {
        let sand = sandbox(&[(11, "one", ONE)]);
        let before = sand.bytes();

        sand.install().await.expect("an install with nothing to do");

        assert_eq!(sand.bytes(), before);
        assert!(
            backup::everything_kept(&sand.store, &sand.game)
                .await
                .unwrap()
                .is_empty(),
            "an install that wrote nothing must not leave a backup behind"
        );
    }

    #[test]
    fn only_the_listed_files_carrying_the_asked_name_are_resolved() {
        let listing = [
            PathBuf::from("Fake_Data/StreamingAssets/aa/catalog.bin"),
            PathBuf::from("Fake_Data/resources.assets"),
            PathBuf::from("catalog.bin"),
        ];

        let found = all_called(&listing, Path::new("/games/fake"), catalog::FILE);

        assert_eq!(
            found,
            [
                PathBuf::from("/games/fake/Fake_Data/StreamingAssets/aa/catalog.bin"),
                PathBuf::from("/games/fake/catalog.bin"),
            ]
        );
        assert!(all_called(&listing, Path::new("/games/fake"), "nothing.bin").is_empty());
    }

    #[test]
    fn a_size_with_no_name_behind_it_is_still_reported_readably() {
        let changed = vec![Rewritten {
            size: 869_764,
            holder: "StreamingAssets/aa/one.bundle".to_string(),
            live: PathBuf::new(),
            pieces: 1,
            pictures: 0,
            faces: Vec::new(),
            sealed: Vec::new(),
        }];

        assert_eq!(
            names_of(&[869_764], &changed),
            "StreamingAssets/aa/one.bundle"
        );
        assert_eq!(names_of(&[4444], &changed), "a bundle of 4444 byte(s)");
    }

    #[test]
    fn one_path_id_used_by_two_nodes_still_gets_a_stem_of_its_own() {
        let one = fake::forge(&[(5, "story", ONE)]);
        let two = fake::forge(&[(5, "story", TWO)]);
        let (one, two) = (
            serial::open(&one, "").expect("a container that opens"),
            serial::open(&two, "").expect("a container that opens"),
        );

        let found = text_asset::scripts_across(&[&one.objects, &two.objects]);
        let stems: Vec<&str> = found.iter().map(|script| script.stem.as_str()).collect();

        assert_eq!(
            stems.len(),
            2,
            "a path id is only unique inside one node of a bundle"
        );
        assert_ne!(
            stems[0], stems[1],
            "one stem for both would stage node 0's text over node 1's and write it back into \
             the wrong asset"
        );
    }

    #[test]
    fn a_name_shared_across_nodes_still_gets_its_own_stem() {
        let one = fake::forge(&[(5, "story", ONE)]);
        let two = fake::forge(&[(9, "story", TWO)]);
        let (one, two) = (
            serial::open(&one, "").expect("a container that opens"),
            serial::open(&two, "").expect("a container that opens"),
        );

        let found = text_asset::scripts_across(&[&one.objects, &two.objects]);
        let stems: Vec<(usize, &str)> = found
            .iter()
            .map(|script| (script.node, script.stem.as_str()))
            .collect();

        assert_eq!(
            stems,
            [(0, "story#0#5"), (1, "story#1#9")],
            "install has to look for the same stems prepare staged"
        );
    }

    #[tokio::test]
    async fn staged_files_are_gathered_under_the_container_they_came_from() {
        let sandbox = tempfile::tempdir().expect("a temp folder");
        let root = sandbox.path().join(text_asset::NAME);

        for at in [
            "resources.assets/line/one.sheet",
            "resources.assets/line/two.sheet",
            "StreamingAssets/aa/x.bundle/line/three.sheet",
        ] {
            let landing = root.join(at);
            fs::create_dir_all(landing.parent().expect("a holder")).expect("a folder");
            fs::write(landing, "").expect("a staged file");
        }

        let piles = piles_under(&root).await;

        assert_eq!(
            piles["resources.assets/line"]
                .iter()
                .map(|(stem, _)| stem.as_str())
                .collect::<Vec<_>>(),
            ["one", "two"]
        );
        assert_eq!(
            piles["StreamingAssets/aa/x.bundle/line"]
                .iter()
                .map(|(stem, _)| stem.as_str())
                .collect::<Vec<_>>(),
            ["three"],
            "a container nested any number of folders deep still becomes one holder"
        );
    }

    struct Labelled {
        raw: Vec<u8>,
        known: Known,
        harvested: Vec<Harvest>,
    }

    fn a_ui_label(said: &[(i64, &str)]) -> Labelled {
        let mut objects = vec![(900, serial::MONO_SCRIPT, fake::a_mono_script("UILabel"))];
        for (path_id, line) in said {
            objects.push((
                *path_id,
                serial::MONO_BEHAVIOUR,
                fake::a_mono_behaviour(900, &[line]),
            ));
        }

        let raw = fake::forge_objects(&objects);
        let assemblies = Assemblies::forged(vec![(
            "UILabel",
            fake::class("UnityEngine.MonoBehaviour", vec![("line", Shape::Text)]),
        )]);

        let one = serial::open(&raw, "resources.assets").expect("a readable container");

        let opened = vec![one];
        let mut classes = mono_script::Names::default();
        for one in &opened {
            classes.learn(one);
        }

        let known = Known {
            assemblies,
            classes,
            named: pictures::Named::default(),
            books: localization::Collections::default(),
        };

        let harvested = mono_behaviour::take(
            "resources.assets",
            &opened[0],
            0,
            1,
            &BTreeSet::new(),
            &known,
        )
        .expect("sheets");

        Labelled {
            raw,
            known,
            harvested,
        }
    }

    impl Labelled {
        fn stage(
            &self,
            whose: &str,
            said: impl Fn(&str) -> String,
        ) -> (tempfile::TempDir, PathBuf, Staged) {
            let dir = tempfile::tempdir().expect("a temp folder");
            let staged_root = dir.path().join("staged");
            let sheet_at = staged_root.join(&self.harvested[0].at);
            fs::create_dir_all(sheet_at.parent().expect("a holder")).expect("a sheet folder");

            let lines: Vec<(String, String)> = sheet::lines(&self.harvested[0].body)
                .expect("a sheet we just wrote")
                .into_keys()
                .map(|spot| {
                    let done = said(&spot);
                    (spot, done)
                })
                .collect();
            fs::write(&sheet_at, sheet::write(lines).expect("a sheet")).expect("a staged sheet");

            let staged = Staged {
                assets: Vec::new(),
                behaviours: vec![(whose.to_string(), sheet_at)],
            };

            (dir, staged_root, staged)
        }

        fn settled(&self, staged: &Staged, staged_root: &Path, land: &str) -> (Vec<u8>, usize) {
            self.settled_with(staged, staged_root, &fonts::Chosen::default(), land)
        }

        fn settled_with(
            &self,
            staged: &Staged,
            staged_root: &Path,
            picked: &fonts::Chosen,
            land: &str,
        ) -> (Vec<u8>, usize) {
            let Done::Ready {
                bytes,
                pieces,
                dropped,
                ..
            } = settle(
                self.raw.clone(),
                settle::load(&self.raw, "resources.assets").expect("a container that loads"),
                &Settling {
                    staged,
                    staged_root,
                    known: &self.known,
                    picked,
                    pictures: &BTreeMap::new(),
                    folder: Path::new("."),
                    store: Path::new("."),
                    game_dir: Path::new("."),
                },
            )
            else {
                panic!("{land}");
            };
            assert!(dropped.is_empty(), "nothing may be dropped: {dropped:?}");

            (bytes, pieces)
        }

        fn text_of(&self, bytes: &[u8], path_id: i64) -> String {
            let back = serial::open(bytes, "").expect("the rewritten container still opens");
            let object = back
                .objects
                .iter()
                .find(|one| one.path_id == path_id)
                .expect("the behaviour is still there");

            layout::strings_in(
                &self.known.assemblies,
                "UILabel",
                &object.body().expect("its body"),
            )
            .expect("the rewritten object still walks")
            .last()
            .map(|one| one.text.clone())
            .expect("a string in the object")
        }
    }

    const WAITED: &str = "\u{5f85}\u{3063}\u{3066}\u{3002}";
    const GREETED: &str = "\u{304a}\u{306f}\u{3088}\u{3046}\u{3002}";

    #[tokio::test]
    async fn a_behaviour_translation_reaches_its_object_through_the_class_name() {
        let label = a_ui_label(&[(7, "Wait for me.")]);

        assert_eq!(
            label.harvested[0].at,
            PathBuf::from("mono_behaviour/resources.assets/UILabel/line/0.sheet"),
            "the harvest has to go through the class name, or this test guards nothing"
        );

        let (dir, staged_root, staged) = label.stage("7", |_| WAITED.to_string());
        let (bytes, pieces) = label.settled(&staged, &staged_root, "the staged line has to land");
        assert_eq!(pieces, 1);

        assert_eq!(
            label.text_of(&bytes, 7),
            WAITED,
            "the translation has to sit in the field the sheet named"
        );

        let _ = &dir;
    }

    #[tokio::test]
    async fn two_objects_sharing_one_sheet_never_swap_their_lines() {
        let label = a_ui_label(&[(7, "Wait for me."), (8, "Morning.")]);

        assert_eq!(
            label.harvested.len(),
            1,
            "both objects have to land in one sheet, or this test guards nothing"
        );

        let (dir, staged_root, staged) = label.stage("7", |spot| {
            match spot.split_once('/') {
                Some(("7", _)) => WAITED,
                _ => GREETED,
            }
            .to_string()
        });
        let (bytes, pieces) =
            label.settled(&staged, &staged_root, "both staged lines have to land");
        assert_eq!(pieces, 2);

        assert_eq!(
            label.text_of(&bytes, 7),
            WAITED,
            "object 7 must keep its own line even though object 8 shares the sheet"
        );
        assert_eq!(
            label.text_of(&bytes, 8),
            GREETED,
            "object 8 must not be given object 7's line"
        );

        let _ = &dir;
    }
}
