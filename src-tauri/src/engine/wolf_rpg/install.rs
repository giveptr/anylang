use crate::backup;
use crate::engine::wolf_rpg::held::Held;
use crate::engine::wolf_rpg::reached::Reached;
use crate::engine::wolf_rpg::source::Which;
use crate::engine::wolf_rpg::{archive, fonts, harvest, held, pictures, reading, source};
use crate::engine::{Install, sheet};
use crate::scope::slashed;
use anyhow::{Context, Result};
use futures::future::BoxFuture;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::Instrument;

fn named_of(at: &Path, staged: &Path) -> Option<String> {
    let inside = at.strip_prefix(staged).ok()?;
    let named = slashed(inside);

    Some(
        named
            .strip_suffix(&format!(".{}", sheet::SUFFIX))?
            .to_string(),
    )
}

struct Taken {
    held: Held,
    slots: Vec<harvest::Slot>,
}

#[tracing::instrument(name = "wolf.read", skip_all)]
async fn read_each(
    at: &Install<'_>,
    files: &[source::File],
    reached: &Reached,
) -> Vec<Result<Taken>> {
    let mut out = Vec::with_capacity(files.len());

    for one in files {
        out.push(
            reading::read(at.store, at.game_dir, one)
                .await
                .map(|held| Taken {
                    slots: harvest::sift(&held.pieces, reached),
                    held,
                }),
        );
    }

    out
}

#[tracing::instrument(name = "wolf.agreed", skip_all)]
fn agreed(
    files: &[source::File],
    read: &[Result<Taken>],
    said: &BTreeMap<String, BTreeMap<String, String>>,
) -> harvest::Agreed {
    let mut out = harvest::Agreed::default();
    let empty = BTreeMap::new();

    for (one, read) in files.iter().zip(read) {
        if let Ok(read) = read {
            out.saw(&read.slots, said.get(&one.named).unwrap_or(&empty));
        }
    }

    out
}

struct Making<'a> {
    at: &'a Install<'a>,
    reached: &'a Reached,
    agreed: &'a harvest::Agreed,
}

#[tracing::instrument(level = "debug", name = "wolf.rebuilt", skip_all)]
async fn rebuilt(
    making: &Making<'_>,
    one: &source::File,
    read: &Taken,
    lines: Option<&BTreeMap<String, String>>,
    telling: bool,
) -> Result<Option<Vec<u8>>> {
    let Making {
        at,
        reached,
        agreed,
    } = making;
    let empty = BTreeMap::new();

    let mut edits = harvest::changed(
        &read.held,
        &read.slots,
        lines.unwrap_or(&empty),
        agreed,
        reached,
    );

    if telling {
        let sending = fonts::sending(at, &read.held).await?;
        edits.extend(fonts::told(&read.held, &sending));
    }

    if edits.is_empty() {
        return Ok(None);
    }

    let body = held::wrapped(&read.held, edits)
        .map_err(|why| anyhow::anyhow!("{} could not be written: {why}", one.named))?;

    let body = reading::sealed(at.store, at.game_dir, &one.at, body)
        .await
        .with_context(|| format!("{} could not be guarded again", one.named))?;

    Ok(Some(body))
}

#[derive(Default)]
struct Tally {
    written: u32,
    drawn: u32,
    swapped: u32,
    given: u32,
    stuck: u32,
}

impl Tally {
    fn stumbled(&mut self, at: &Install<'_>, why: &anyhow::Error) {
        self.stuck += 1;
        at.progress.warn(at.doing, &format!("{why:#}"));
    }

    fn told(&self, at: &Install<'_>) {
        if self.written > 0 {
            at.progress
                .info(at.doing, &format!("{} file(s) written in", self.written));
        }
        if self.drawn > 0 {
            at.progress
                .info(at.doing, &format!("{} picture(s) swapped in", self.drawn));
        }
        if self.swapped > 0 {
            at.progress
                .info(at.doing, &format!("{} font(s) swapped in", self.swapped));
        }
        if self.given > 0 {
            at.progress
                .info(at.doing, &format!("{} file(s) put back", self.given));
        }
    }
}

async fn put_back(at: &Install<'_>, one: &Path, tally: &mut Tally) {
    match backup::put_back(at.store, at.game_dir, one).await {
        Ok(true) => tally.given += 1,
        Ok(false) => {}
        Err(why) => tally.stumbled(at, &why),
    }
}

#[tracing::instrument(name = "wolf.reseal", skip_all)]
async fn resealed(
    at: &Install<'_>,
    root: &Path,
    archive: &Path,
    weight: Option<u32>,
    fresh: BTreeMap<PathBuf, Vec<u8>>,
) -> Result<()> {
    let key = archive::key_for(archive, weight)
        .map_err(|why| anyhow::anyhow!("{}: {why}", archive.display()))?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "{} is not an archive this writer knows how to seal",
                archive.display()
            )
        })?;

    let under = source::unpacks_into(root, archive);

    let from = backup::original_at(at.store, at.game_dir, archive).await?;

    let held = tracing::info_span!("wolf.reseal.build");
    let sealed = tokio::task::spawn_blocking(move || {
        held.in_scope(|| archive::resealed(&from, &key, &under, &fresh))
    })
    .await?
    .map_err(anyhow::Error::msg)?;

    for one in &sealed.missed {
        at.progress.warn(
            at.doing,
            &format!(
                "{} holds no slot for {}, so the lines written into it stayed out of the game",
                archive.display(),
                one.display()
            ),
        );
    }

    backup::replace(at.store, at.game_dir, archive, sealed.body).await
}

async fn drawn_in(
    at: &Install<'_>,
    root: &Path,
    sealing: &mut BTreeMap<PathBuf, BTreeMap<PathBuf, Vec<u8>>>,
    tally: &mut Tally,
) -> BTreeMap<PathBuf, u32> {
    let picked = pictures::picked(at).await;
    for why in &picked.said {
        at.progress.warn(at.doing, why);
    }

    let mut drew: BTreeMap<PathBuf, u32> = BTreeMap::new();
    for (archive, inside) in picked.sealing {
        *drew.entry(archive.clone()).or_default() += inside.len() as u32;

        let under = source::unpacks_into(root, &archive);
        let landed = sealing.entry(archive).or_default();
        for (one, body) in inside {
            landed.insert(under.join(one), body);
        }
    }

    let mut swapped = Vec::new();
    for (one, body) in picked.loose {
        match backup::replace(at.store, at.game_dir, &one, body).await {
            Ok(()) => {
                tally.drawn += 1;
                swapped.push(one);
            }
            Err(why) => tally.stumbled(at, &why),
        }
    }

    match pictures::let_go(at, &swapped).await {
        Ok(given) => tally.given += given as u32,
        Err(why) => tally.stumbled(at, &why),
    }

    drew
}

async fn sealed_back(
    at: &Install<'_>,
    root: &Path,
    taken: &[PathBuf],
    weight: Option<u32>,
    mut sealing: BTreeMap<PathBuf, BTreeMap<PathBuf, Vec<u8>>>,
    mut drew: BTreeMap<PathBuf, u32>,
    tally: &mut Tally,
) {
    for one in taken {
        match sealing.remove(one) {
            Some(fresh) => {
                let count = fresh.len() as u32;
                let drew = drew.remove(one).unwrap_or_default();

                match resealed(at, root, one, weight, fresh).await {
                    Ok(()) => {
                        tally.written += count.saturating_sub(drew);
                        tally.drawn += drew;
                    }
                    Err(why) => tally.stumbled(at, &why),
                }
            }
            None => put_back(at, one, tally).await,
        }
    }
}

pub fn run(at: Install<'_>) -> BoxFuture<'_, Result<()>> {
    Box::pin(
        async move {
            let root = source::root(at.game_dir, at.store);
            if !source::read_out(&root) {
                at.progress.warn(
                at.doing,
                &format!(
                    "nothing was written into the game: the data this reader took the text out of \
                     is no longer at {}, so read the game again before applying",
                    root.display()
                ),
            );

                return Ok(());
            }

            let was_packed = source::still_packed(at.game_dir, &root);

            fonts::tidied(&at).await;

            let reached = reading::looked_up(at.store, at.game_dir, &root).await;
            let files = source::files(&root).await;
            let said = sheet::staged(
                &at,
                |one| named_of(one, at.staged),
                |named| files.iter().any(|file| file.named == named),
            )
            .await?;

            let read = read_each(&at, &files, &reached).await;
            let agreed = agreed(&files, &read, &said);
            let making = Making {
                at: &at,
                reached: &reached,
                agreed: &agreed,
            };
            let taken = source::archives(at.game_dir);
            let weight = source::weight(at.game_dir);

            let mut tally = Tally::default();
            let mut untouched = Vec::new();
            let mut sealing: BTreeMap<PathBuf, BTreeMap<PathBuf, Vec<u8>>> = BTreeMap::new();

            async {
                for (one, read) in files.iter().zip(&read) {
                    let lines = said.get(&one.named);
                    let telling = matches!(one.which, Which::Game);

                    let made = match read {
                        Ok(read) => rebuilt(&making, one, read, lines, telling).await,
                        Err(_) if lines.is_none() => {
                            untouched.push(one);
                            continue;
                        }
                        Err(why) => Err(anyhow::anyhow!("{why:#}")),
                    };

                    let fresh = match made {
                        Ok(Some(fresh)) => fresh,
                        Ok(None) => {
                            untouched.push(one);
                            continue;
                        }
                        Err(why) => {
                            tally.stumbled(&at, &why);
                            continue;
                        }
                    };

                    if !was_packed {
                        match backup::replace(at.store, at.game_dir, &one.at, fresh).await {
                            Ok(()) => tally.written += 1,
                            Err(why) => tally.stumbled(&at, &why),
                        }
                        continue;
                    }

                    match source::archive_of(&taken, &root, &one.at) {
                        Some(archive) => {
                            sealing
                                .entry(archive)
                                .or_default()
                                .insert(one.at.clone(), fresh);
                        }
                        None => tally.stumbled(
                            &at,
                            &anyhow::anyhow!(
                                "{} came from no archive this writer can find",
                                one.named
                            ),
                        ),
                    }
                }
            }
            .instrument(tracing::info_span!("wolf.rebuild"))
            .await;

            match fonts::swapped_in(&at, &root).await {
                Ok(done) => {
                    tally.swapped += done.swapped;
                    tally.given += done.given;
                }
                Err(why) => tally.stumbled(&at, &why),
            }

            let drew = drawn_in(&at, &root, &mut sealing, &mut tally).await;

            if !sealing.is_empty() {
                let whole: u64 = sealing
                    .keys()
                    .filter_map(|one| fs::metadata(one).ok())
                    .map(|held| held.len())
                    .sum();

                at.progress.info(
                    at.doing,
                    &format!(
                        "{} archive(s) to seal again ({} MB)",
                        sealing.len(),
                        whole / 1_000_000
                    ),
                );
            }

            sealed_back(&at, &root, &taken, weight, sealing, drew, &mut tally).await;

            if !was_packed {
                for one in untouched {
                    put_back(&at, &one.at, &mut tally).await;
                }
            }

            tally.told(&at);

            if tally.stuck > 0 {
                anyhow::bail!(
                    "{} file(s) could not be written, the rest went in",
                    tally.stuck
                );
            }

            Ok(())
        }
        .instrument(tracing::info_span!("wolf.install")),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas;
    use crate::canvas::Canvas;
    use crate::engine::fonts::Fonts;
    use crate::engine::pictures::Pictures;
    use crate::engine::wolf_rpg::fixture::{self, a_jpeg, dotted, sandbox};
    use crate::engine::wolf_rpg::{game, map, prepare};
    use crate::engine::{Prepare, Swap};
    use crate::progress::{Heard, Quiet};

    struct Swapping {
        game: tempfile::TempDir,
        store: tempfile::TempDir,
        bare: tempfile::TempDir,
        staged: PathBuf,
        picked: PathBuf,
    }

    impl Swapping {
        async fn laid_out(packed: bool) -> Self {
            let game = sandbox();
            let store = sandbox();
            let root = game.path();

            match packed {
                false => {
                    fixture::lay_out(root);
                    let loose = root.join(source::DATA).join("Picture");
                    fs::create_dir_all(&loose).unwrap();
                    fs::write(
                        loose.join("title.png"),
                        dotted(20, 10, 0).png().expect("a png"),
                    )
                    .unwrap();
                }
                true => {
                    let data = root.join(source::DATA);
                    fs::create_dir_all(&data).unwrap();
                    fs::write(root.join("Game.exe"), [0; 4]).unwrap();

                    let (plan, base) = fixture::database(&[fixture::Type {
                        name: "\u{30a2}\u{30a4}\u{30c6}\u{30e0}",
                        fields: &["\u{540d}\u{524d}"],
                        words: &[0],
                        entries: &[&["\u{7dd1}\u{8336}"]],
                        named_by: None,
                    }]);

                    fs::write(
                        data.join("BasicData.wolf"),
                        archive::archived(
                            &[
                                (
                                    "Game.dat",
                                    fixture::game("\u{9060}\u{3044}\u{9053}", "", "MS Gothic")
                                        .as_slice(),
                                ),
                                ("DataBase.dat", base.as_slice()),
                                ("DataBase.project", plan.as_slice()),
                            ],
                            None,
                        ),
                    )
                    .unwrap();
                    fs::write(
                        data.join("Picture.wolf"),
                        archive::archived(
                            &[
                                (
                                    "title.png",
                                    dotted(20, 10, 0).png().expect("a png").as_slice(),
                                ),
                                (
                                    "beside.png",
                                    dotted(4, 4, 9).png().expect("a png").as_slice(),
                                ),
                                ("room.jpg", a_jpeg(24, 12).as_slice()),
                                ("torn.jpg", b"\xff\xd8\xff\xe0 not really a jpeg".as_slice()),
                            ],
                            None,
                        ),
                    )
                    .unwrap();
                }
            }

            let held = Self {
                staged: store.path().join("staged"),
                picked: store.path().join("picked.png"),
                game,
                store,
                bare: sandbox(),
            };

            prepare::run(
                "Wolf RPG",
                Prepare::over(
                    held.game.path(),
                    &held.store.path().join("source"),
                    held.store.path(),
                ),
            )
            .await
            .expect("the game is read");

            held
        }

        fn key_for(&self, name: &str) -> String {
            pictures::LEDGER
                .remembered(self.store.path())
                .into_iter()
                .find(|one| one.name == name)
                .unwrap_or_else(|| panic!("{name} was listed"))
                .key
        }

        fn pick(&self, held: &Canvas) -> Pictures {
            self.pick_for("title.png", held)
        }

        fn pick_for(&self, name: &str, held: &Canvas) -> Pictures {
            fs::write(&self.picked, held.png().expect("a png")).expect("a picture to pick");

            Pictures {
                swaps: vec![Swap {
                    from: self.key_for(name),
                    to: self.picked.to_string_lossy().to_string(),
                }],
                ..Pictures::default()
            }
        }

        async fn install(&self, images: &Pictures) -> Result<()> {
            run(Install::over(self.game.path(), &self.staged, self.store.path()).drawing(images))
                .await
        }

        fn shipped(&self, name: &str) -> Vec<u8> {
            pictures::picture(self.game.path(), self.store.path(), &self.key_for(name))
                .expect("the picture is still there")
        }

        fn drawn(&self, name: &str) -> Vec<u8> {
            pictures::picture(self.game.path(), self.bare.path(), &self.key_for(name))
                .expect("the picture the game loads")
        }
    }

    #[tokio::test]
    async fn a_picture_the_game_keeps_as_a_jpg_goes_back_into_the_archive_as_a_jpg() {
        let held = Swapping::laid_out(true).await;

        let photographed = a_jpeg(24, 12);
        fs::write(&held.picked, &photographed).expect("a jpg to pick");
        held.install(&Pictures {
            swaps: vec![Swap {
                from: held.key_for("room.jpg"),
                to: held.picked.to_string_lossy().to_string(),
            }],
            ..Pictures::default()
        })
        .await
        .expect("the picture goes in");

        assert_eq!(
            held.shipped("room.jpg"),
            photographed,
            "a jpg of the very size the game reads goes into the archive as the reader saved it, \
             byte for byte: writing it out again would cost them a round of jpg for nothing"
        );

        held.install(&held.pick_for("room.jpg", &dotted(48, 24, 60)))
            .await
            .expect("a picture of another size goes in too");

        let after = held.shipped("room.jpg");
        assert_eq!(
            canvas::kind_of(&after),
            Some("jpg"),
            "the game asks its archive for room.jpg and hands what comes back to a decoder chosen \
             by that name, so what goes in stays a jpg however the reader saved their own file"
        );
        let drawn = Canvas::read(&after).expect("it reads back");
        assert_eq!(
            (drawn.wide as u32, drawn.high as u32),
            (24, 12),
            "and it keeps the size the game shipped"
        );
    }

    #[tokio::test]
    async fn a_pick_against_a_picture_this_reader_cannot_open_is_turned_away_out_loud() {
        let held = Swapping::laid_out(true).await;
        let was = fs::read(held.game.path().join(source::DATA).join("Picture.wolf"))
            .expect("the archive");

        let heard = Heard::default();
        run(
            Install::over(held.game.path(), &held.staged, held.store.path())
                .drawing(&held.pick_for("torn.jpg", &dotted(24, 12, 60)))
                .heard_by(&heard),
        )
        .await
        .expect("an install that goes through");

        assert_eq!(
            fs::read(held.game.path().join(source::DATA).join("Picture.wolf"))
                .expect("the archive"),
            was,
            "nothing in this game says what size that picture is, so there is no size to fit a \
             replacement to and the archive is left exactly as it shipped"
        );
        assert!(
            heard
                .warnings()
                .iter()
                .any(|said| said.starts_with("torn.jpg:")),
            "the row said up front that this one is not ours to swap, and picking a file for it \
             anyway has to say so again rather than pass in silence: {:?}",
            heard.warnings()
        );
    }

    #[tokio::test]
    async fn a_picked_picture_lands_in_the_archive_the_game_reads_and_leaves_its_neighbour_alone() {
        let held = Swapping::laid_out(true).await;
        let was = held.shipped("beside.png");

        let fresh = dotted(20, 10, 200);
        held.install(&held.pick(&fresh))
            .await
            .expect("the picture goes in");

        assert_eq!(
            held.drawn("title.png"),
            fresh.png().expect("a png"),
            "a Wolf game reads its pictures out of the archive it ships, so a pick that never \
             reaches the archive is a pick the player never sees"
        );
        assert_eq!(
            held.drawn("beside.png"),
            was,
            "one picture is swapped at a time, and the hundreds beside it in the same archive \
             belong to the game"
        );
        assert_eq!(
            held.shipped("title.png"),
            dotted(20, 10, 0).png().expect("a png"),
            "the editor shows the picture the game shipped even after a pick is in, or the reader \
             loses the one thing they were comparing their own picture against"
        );

        held.install(&Pictures::default())
            .await
            .expect("a second install with nothing picked");

        assert_eq!(
            held.drawn("title.png"),
            dotted(20, 10, 0).png().expect("a png"),
            "letting the pick go has to hand the game back the picture it shipped, or a swap made \
             once can never be undone"
        );
    }

    #[tokio::test]
    async fn a_picked_picture_lying_loose_beside_the_game_goes_in_and_can_be_put_back() {
        let held = Swapping::laid_out(false).await;
        let live = held
            .game
            .path()
            .join(source::DATA)
            .join("Picture")
            .join("title.png");
        let was = fs::read(&live).expect("the picture the game shipped");

        let fresh = dotted(20, 10, 111);
        held.install(&held.pick(&fresh))
            .await
            .expect("the picture goes in");

        assert_eq!(
            fs::read(&live).expect("the picture now"),
            fresh.png().expect("a png"),
            "a game read out of its archives draws from the files beside it, and those are the \
             ones a pick has to land on"
        );
        assert!(
            backup::everything_kept(held.store.path(), held.game.path())
                .await
                .expect("what is kept")
                .contains(&live),
            "the picture the game shipped has to be kept, or Restore original files has nothing to \
             put back"
        );

        held.install(&Pictures::default())
            .await
            .expect("a second install with nothing picked");

        assert_eq!(
            fs::read(&live).expect("the picture now"),
            was,
            "dropping the pick gives the game its own picture back, byte for byte"
        );
    }

    #[tokio::test]
    async fn a_pick_this_writer_cannot_stand_behind_is_said_out_loud_and_changes_nothing() {
        let held = Swapping::laid_out(true).await;
        let was = fs::read(held.game.path().join(source::DATA).join("Picture.wolf"))
            .expect("the archive");

        let mut asked = held.pick(&dotted(20, 10, 3));
        asked.swaps[0].from = "Data/Picture.wolf|nothing.png".to_string();

        let heard = Heard::default();
        run(
            Install::over(held.game.path(), &held.staged, held.store.path())
                .drawing(&asked)
                .heard_by(&heard),
        )
        .await
        .expect("an install that goes through");

        assert_eq!(
            fs::read(held.game.path().join(source::DATA).join("Picture.wolf"))
                .expect("the archive"),
            was,
            "a key left over from a game read before may not land on whatever picture happens to \
             sit there now"
        );
        assert!(
            heard
                .warnings()
                .iter()
                .any(|said| said.contains("not in this game any more")),
            "the reader picked a file and nothing came of it, so saying nothing would leave them \
             waiting on a change that is never coming: {:?}",
            heard.warnings()
        );
    }

    #[test]
    fn a_sheet_is_matched_to_the_game_file_it_was_read_out_of() {
        let staged = Path::new("/store/staged/japanese");

        assert_eq!(
            named_of(&staged.join("MapData").join("Dungeon.mps.sheet"), staged),
            Some("MapData/Dungeon.mps".to_string()),
            "the sheet carries the game's own path so two maps of one name never collide"
        );
        assert_eq!(
            named_of(&staged.join("BasicData").join("Game.dat.sheet"), staged),
            Some("BasicData/Game.dat".to_string())
        );
        assert_eq!(named_of(&staged.join("notes.txt"), staged), None);
    }

    #[tokio::test]
    async fn a_map_nobody_translated_this_time_goes_back_to_the_words_it_shipped() {
        let at = sandbox();
        let root = at.path();
        let store = sandbox();
        let staged = store.path().join("staged");
        fixture::lay_out(root);

        let map = root.join(source::DATA).join("MapData").join("Dungeon.mps");
        let shipped = fs::read(&map).expect("the map");

        let landing = staged.join("MapData");
        fs::create_dir_all(&landing).unwrap();
        fs::write(
            landing.join("Dungeon.mps.sheet"),
            sheet::write([("e0/p0/c0/s0".to_string(), "The door is locked.".to_string())])
                .expect("a sheet"),
        )
        .unwrap();

        let quiet = Quiet;
        let fonts = Fonts::default();
        let told = || {
            Install::over(root, &staged, store.path())
                .sending(&fonts)
                .heard_by(&quiet)
        };

        run(told()).await.expect("the sheet goes in");

        let after = fs::read(&map).expect("the map");
        assert_ne!(after, shipped);
        assert_eq!(
            map::read(&after).expect("it still reads").pieces[0].said[0].text,
            "The door is locked.",
        );

        fs::remove_file(landing.join("Dungeon.mps.sheet")).unwrap();
        run(told()).await.expect("taking it back out");

        assert_eq!(
            fs::read(&map).expect("the map"),
            shipped,
            "with nothing staged the game gets its own words back, byte for byte"
        );
    }

    #[tokio::test]
    async fn a_scolded_game_is_never_written_to_by_a_read_and_still_gives_up_its_own_words_after() {
        let at = sandbox();
        let root = at.path();
        let store = sandbox();
        let source_dir = store.path().join("source");
        let staged = store.path().join("staged");
        fixture::lay_out(root);

        let game_dat = root.join(source::DATA).join(source::BASIC).join("Game.dat");
        let mut scolded = b"Extracting data violates the guidelines.\0".to_vec();
        scolded.extend(fs::read(&game_dat).unwrap());
        fs::write(&game_dat, &scolded).unwrap();

        let quiet = Quiet;
        let fonts = Fonts::default();
        let read = async || {
            prepare::run(
                "Wolf RPG",
                Prepare::over(root, &source_dir, store.path()).heard_by(&quiet),
            )
            .await
            .expect("the game is read");

            let page = fs::read_to_string(source_dir.join("BasicData").join("Game.dat.sheet"))
                .expect("the sheet");

            sheet::lines(&page)
                .expect("its lines")
                .get("title/s0")
                .cloned()
                .expect("the title")
        };

        assert_eq!(read().await, "\u{9060}\u{3044}\u{9053}");
        assert_eq!(
            fs::read(&game_dat).unwrap(),
            scolded,
            "the guard comes off in memory, so reading a game leaves every byte of it where the \
             game put it and Restore original files still has something true to give back"
        );

        let landing = staged.join("BasicData");
        fs::create_dir_all(&landing).unwrap();
        fs::write(
            landing.join("Game.dat.sheet"),
            sheet::write([("title/s0".to_string(), "The Long Road Home".to_string())])
                .expect("a sheet"),
        )
        .unwrap();

        run(Install::over(root, &staged, store.path())
            .sending(&fonts)
            .heard_by(&quiet))
        .await
        .expect("the translation goes in");

        let after = fs::read(&game_dat).unwrap();
        let past = b"Extracting data violates the guidelines.\0".len();
        assert!(
            after.starts_with(b"Extracting data violates the guidelines.\0"),
            "the game reads its own scolding, so what is written back has to wear it again"
        );
        assert_eq!(
            game::read(&after[past..])
                .expect("the game can still read it")
                .pieces[0]
                .said[0]
                .text,
            "The Long Road Home",
            "the size Game.dat carries is counted from past the scolding, so a title that grew \
             has to leave a file the engine still opens"
        );

        assert_eq!(
            read().await,
            "\u{9060}\u{3044}\u{9053}",
            "reading the game a second time has to hand back the words the game shipped, not the \
             translation just written into it"
        );
    }

    #[tokio::test]
    async fn a_script_nobody_could_read_does_not_stand_in_the_way_of_the_rest() {
        let at = sandbox();
        let root = at.path();
        let store = sandbox();
        let staged = store.path().join("staged");
        fixture::lay_out(root);

        let scripts = root.join(source::DATA).join("Text_Script").join("old");
        fs::create_dir_all(&scripts).unwrap();
        fs::write(
            scripts.join("WIP.txt"),
            [0x40, 0x6d, 0x65, 0x73, 0x20, 0x82, 0xa0],
        )
        .unwrap();

        let landing = staged.join("MapData");
        fs::create_dir_all(&landing).unwrap();
        fs::write(
            landing.join("Dungeon.mps.sheet"),
            sheet::write([("e0/p0/c0/s0".to_string(), "The door is locked.".to_string())])
                .expect("a sheet"),
        )
        .unwrap();

        let quiet = Quiet;
        let fonts = Fonts::default();

        run(Install::over(root, &staged, store.path())
            .sending(&fonts)
            .heard_by(&quiet))
        .await
        .expect(
            "a script the reader already turned away at import holds no translation, so it is \
             left alone rather than failing the whole apply",
        );

        let map = root.join(source::DATA).join("MapData").join("Dungeon.mps");
        assert_eq!(
            map::read(&fs::read(&map).expect("the map"))
                .expect("it still reads")
                .pieces[0]
                .said[0]
                .text,
            "The door is locked.",
        );
    }
}
