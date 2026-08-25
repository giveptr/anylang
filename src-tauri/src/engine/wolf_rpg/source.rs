use crate::engine::wolf_rpg::{common, database, game, map, script};
use crate::scope::slashed;
use crate::walk;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub const DATA: &str = "Data";
pub const BASIC: &str = "BasicData";
const SHIPPED_PLAN: &str = "SysDataBaseBasic";
pub const UNPACKED: &str = "unpacked";

const ARCHIVES: [&str; 8] = [
    "wolf", "data", "pak", "bin", "assets", "content", "res", "resource",
];
const SEALED: [&str; 1] = ["wolfx"];
const CARRIERS: [&str; 5] = ["Game", "List", "Data2", "GameFile", "BasicData2"];
const RUNNERS: [&str; 2] = ["Game.exe", "GamePro.exe"];
const DEEP: usize = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Which {
    Map,
    Script,
    Common,
    Database { plan: PathBuf },
    Game,
}

#[derive(Debug, Clone)]
pub struct File {
    pub at: PathBuf,
    pub named: String,
    pub which: Which,
}

pub fn holds_a_game(game_dir: &Path) -> bool {
    read_out(game_dir) || packed(game_dir)
}

pub fn game_dat(game_dir: &Path) -> PathBuf {
    game_dir
        .join(DATA)
        .join(BASIC)
        .join(format!("{}.{}", game::NAME, database::SUFFIX))
}

pub fn read_out(game_dir: &Path) -> bool {
    game_dat(game_dir).is_file()
}

pub fn root(game_dir: &Path, store: &Path) -> PathBuf {
    match read_out(game_dir) {
        true => game_dir.to_path_buf(),
        false => store.join(UNPACKED),
    }
}

pub fn still_packed(game_dir: &Path, root: &Path) -> bool {
    root != game_dir
}

fn whole_data(at: &Path) -> bool {
    at.file_stem()
        .is_some_and(|stem| stem.to_string_lossy().eq_ignore_ascii_case(DATA))
}

pub fn unpacks_into(root: &Path, at: &Path) -> PathBuf {
    let data = root.join(DATA);

    match whole_data(at) {
        true => data,
        false => data.join(at.file_stem().unwrap_or_default()),
    }
}

pub fn archive_of(taken: &[PathBuf], root: &Path, at: &Path) -> Option<PathBuf> {
    let under = at.strip_prefix(root.join(DATA)).ok()?;
    let stem = under.components().next()?.as_os_str();

    taken
        .iter()
        .find(|one| one.file_stem() == Some(stem))
        .or_else(|| taken.iter().find(|one| whole_data(one)))
        .cloned()
}

fn looks_like_wolf(game_dir: &Path) -> bool {
    game_dir.join(DATA).is_dir() || RUNNERS.iter().any(|named| game_dir.join(named).is_file())
}

fn suffixed_by(at: &Path, known: &[&str]) -> bool {
    at.extension()
        .is_some_and(|kind| known.iter().any(|one| kind.eq_ignore_ascii_case(one)))
}

fn carries(at: &Path) -> bool {
    at.file_stem().is_some_and(|stem| {
        CARRIERS
            .iter()
            .any(|known| stem.eq_ignore_ascii_case(known))
    })
}

fn shallow(game_dir: &Path) -> impl Iterator<Item = PathBuf> {
    WalkDir::new(game_dir)
        .max_depth(DEEP)
        .follow_links(true)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|one| one.file_type().is_file())
        .map(walkdir::DirEntry::into_path)
}

fn packed(game_dir: &Path) -> bool {
    looks_like_wolf(game_dir)
        && shallow(game_dir).any(|at| suffixed_by(&at, &ARCHIVES) || suffixed_by(&at, &SEALED))
}

fn carrier(game_dir: &Path) -> Option<PathBuf> {
    shallow(game_dir).find(|at| carries(at) && suffixed_by(at, &ARCHIVES))
}

pub fn weight(game_dir: &Path) -> Option<u32> {
    let held = fs::metadata(carrier(game_dir)?).ok()?;

    Some(held.len() as u32)
}

pub fn archives(game_dir: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = shallow(game_dir)
        .filter(|at| suffixed_by(at, &ARCHIVES))
        .filter(|at| !carries(at))
        .collect();

    found.sort();
    found
}

pub fn sealed(game_dir: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = shallow(game_dir)
        .filter(|at| suffixed_by(at, &SEALED))
        .collect();

    found.sort();
    found
}

fn suffixed(at: &Path, kind: &str) -> bool {
    suffixed_by(at, &[kind])
}

pub fn worth_reading(at: &Path) -> bool {
    suffixed_by(
        at,
        &[
            map::SUFFIX,
            database::SUFFIX,
            database::PLAN,
            script::SUFFIX,
        ],
    )
}

pub async fn files(game_dir: &Path) -> Vec<File> {
    let root = game_dir.to_path_buf();

    tokio::task::spawn_blocking(move || listed(&root))
        .await
        .expect("walking the game data")
}

fn listed(game_dir: &Path) -> Vec<File> {
    let data = game_dir.join(DATA);
    let basic = data.join(BASIC);

    let under = walk::files_now(&data);

    let mut found: Vec<File> = under
        .iter()
        .filter(|at| suffixed(at, map::SUFFIX))
        .filter_map(|at| named(&data, at.clone(), Which::Map))
        .collect();

    found.extend(
        under
            .iter()
            .filter(|at| suffixed(at, script::SUFFIX))
            .filter(|at| at.parent().is_some_and(|up| up != data))
            .filter_map(|at| named(&data, at.clone(), Which::Script)),
    );

    for (name, which) in [(common::NAME, Which::Common), (game::NAME, Which::Game)] {
        let at = basic.join(format!("{name}.{}", database::SUFFIX));
        if at.is_file() {
            found.extend(named(&data, at, which));
        }
    }

    let mut plans: Vec<PathBuf> = match fs::read_dir(&basic) {
        Ok(listed) => listed
            .filter_map(Result::ok)
            .map(|one| one.path())
            .filter(|at| suffixed(at, database::PLAN))
            .filter(|at| at.file_stem().is_some_and(|stem| stem != SHIPPED_PLAN))
            .collect(),
        Err(_) => Vec::new(),
    };
    plans.sort();

    for plan in plans {
        let at = plan.with_extension(database::SUFFIX);
        if found.iter().any(|taken| taken.at == at) {
            continue;
        }
        if at.is_file() {
            found.extend(named(&data, at, Which::Database { plan }));
        }
    }

    found.sort_by(|a, b| a.named.cmp(&b.named));

    found
}

fn named(data: &Path, at: PathBuf, which: Which) -> Option<File> {
    let named = slashed(at.strip_prefix(data).ok()?);

    Some(File { at, named, which })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::wolf_rpg::fixture::{self, sandbox};

    #[tokio::test]
    async fn every_file_a_wolf_game_keeps_its_text_in_is_found_and_nothing_else_is() {
        let at = sandbox();
        let root = at.path();
        fixture::lay_out(root);

        fs::write(root.join(DATA).join(BASIC).join("MapTree.dat"), [0; 8]).unwrap();
        fs::write(root.join(DATA).join(BASIC).join("TileSetData.dat"), [0; 8]).unwrap();
        fs::write(
            root.join(DATA).join(BASIC).join("CommonEvent.project"),
            [0; 8],
        )
        .unwrap();
        fs::write(
            root.join(DATA).join(BASIC).join("SysDataBaseBasic.project"),
            [0; 8],
        )
        .unwrap();
        fs::write(
            root.join(DATA).join(BASIC).join("SysDataBaseBasic.dat"),
            [0; 8],
        )
        .unwrap();

        assert!(holds_a_game(root));
        assert!(read_out(root));

        let found = files(root).await;
        let named: Vec<&str> = found.iter().map(|one| one.named.as_str()).collect();

        assert_eq!(
            named,
            [
                "BasicData/CommonEvent.dat",
                "BasicData/DataBase.dat",
                "BasicData/Game.dat",
                "MapData/Dungeon.mps",
            ],
            "the editor's own shipped database and the map tree hold nothing a player reads, and \
             the tile set is not a database at all"
        );

        assert!(matches!(
            found
                .iter()
                .find(|one| one.named == "BasicData/DataBase.dat")
                .expect("the database")
                .which,
            Which::Database { .. }
        ));

        assert!(
            matches!(
                found
                    .iter()
                    .find(|one| one.named == "BasicData/CommonEvent.dat")
                    .expect("the common events")
                    .which,
                Which::Common
            ),
            "a stray plan beside the common events never lists the file a second time: the \
             second reading would put back what the first just wrote in"
        );
    }

    #[test]
    fn a_game_that_packs_the_whole_data_folder_into_one_archive_unpacks_over_it_rather_than_under()
    {
        let root = Path::new("/store/unpacked");
        let whole = Path::new("/game/Data.wolf");
        let apart = Path::new("/game/Data/BasicData.wolf");

        assert_eq!(
            unpacks_into(root, whole),
            root.join(DATA),
            "an archive named after the data folder holds that folder's own tree, so unpacking it \
             into a folder of its own name buries BasicData one step deeper than the game keeps it"
        );
        assert_eq!(
            unpacks_into(root, apart),
            root.join(DATA).join(BASIC),
            "and a game that packs each folder on its own still lands each one where it belongs"
        );

        let taken = [whole.to_path_buf()];
        assert_eq!(
            archive_of(&taken, root, &root.join(DATA).join(BASIC).join("Game.dat")).as_deref(),
            Some(whole),
            "every file under the data folder came out of that one archive, so writing any of \
             them back has to find it again"
        );
    }

    #[tokio::test]
    async fn a_game_still_in_its_archive_is_recognised_so_the_reader_can_be_told_why_not() {
        let at = sandbox();
        let root = at.path();
        fs::create_dir_all(root.join(DATA)).unwrap();
        fs::write(root.join(DATA).join("BasicData.wolf"), [0; 16]).unwrap();

        assert!(
            holds_a_game(root),
            "turning it away as no game at all tells the reader nothing about what to do next"
        );
        assert!(!read_out(root));
        assert!(packed(root));
        assert!(files(root).await.is_empty());
    }

    #[test]
    fn a_folder_that_is_no_wolf_game_is_left_alone() {
        let at = sandbox();

        assert!(!holds_a_game(at.path()));

        fs::create_dir_all(at.path().join("www").join("data")).unwrap();
        fs::write(at.path().join("www").join("data").join("Map001.json"), "{}").unwrap();

        assert!(!holds_a_game(at.path()));
    }

    #[test]
    fn an_archive_in_a_game_no_wolf_exe_runs_is_not_reason_enough_to_claim_it() {
        let at = sandbox();
        let root = at.path();
        fs::create_dir_all(root.join("Bundles")).unwrap();
        fs::write(root.join("Bundles").join("extra.wolf"), [0; 16]).unwrap();

        assert!(
            !holds_a_game(root),
            "another engine may ship a file of that name, and claiming it would take the game \
             away from the reader who could actually translate it"
        );

        fs::write(root.join("Game.exe"), [0; 4]).unwrap();
        assert!(holds_a_game(root));
    }
}
