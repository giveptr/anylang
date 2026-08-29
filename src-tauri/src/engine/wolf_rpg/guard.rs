use crate::engine::wolf_rpg::{database, unprot};
use std::path::Path;

fn kind_of(at: &Path) -> Option<unprot::Kind> {
    at.file_name()
        .and_then(|named| named.to_str())
        .and_then(unprot::Kind::of)
}

fn is_plan(at: &Path) -> bool {
    at.extension()
        .is_some_and(|kind| kind.eq_ignore_ascii_case(database::PLAN))
}

fn scrambled(raw: &[u8]) -> bool {
    if database::plan(raw).is_ok() {
        return false;
    }

    let mut turned = raw.to_vec();
    unprot::unplanned(&mut turned);

    database::plan(&turned).is_ok()
}

fn replanned(mut raw: Vec<u8>) -> Vec<u8> {
    if scrambled(&raw) {
        unprot::unplanned(&mut raw);
    }

    raw
}

pub fn wraps(at: &Path) -> bool {
    is_plan(at) || kind_of(at).is_some()
}

pub fn opened(raw: Vec<u8>, at: &Path) -> Result<Vec<u8>, String> {
    if is_plan(at) {
        return Ok(replanned(raw));
    }

    let Some(kind) = kind_of(at) else {
        return Ok(raw);
    };

    let mut freed = raw;
    unprot::freed(&mut freed, kind)?;

    Ok(freed)
}

pub fn sealed(shipped: &[u8], at: &Path, body: &mut Vec<u8>) -> Result<(), String> {
    if is_plan(at) {
        if scrambled(shipped) {
            unprot::unplanned(body);
        }

        return Ok(());
    }

    let Some(kind) = kind_of(at) else {
        return Ok(());
    };

    let mut freed = shipped.to_vec();
    let kept = unprot::freed(&mut freed, kind)?;

    unprot::reguarded(body, kind, &kept)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::wolf_rpg::{fixture, game};

    fn scolded() -> Vec<u8> {
        let mut out = b"Extracting data violates the guidelines.\0".to_vec();
        out.extend_from_slice(&fixture::game("A Title", "", "MS Gothic"));

        out
    }

    #[test]
    fn a_file_is_opened_out_of_its_scolding_and_sealed_back_into_it_from_the_bytes_alone() {
        let shipped = scolded();
        let at = Path::new("/game/Data/BasicData/Game.dat");

        let mut body = opened(shipped.clone(), at).expect("it opens");
        assert_ne!(body, shipped, "the scolding is off while the file is read");

        sealed(&shipped, at, &mut body).expect("it seals back");
        assert_eq!(
            body, shipped,
            "nothing about the guard is kept anywhere, so what goes back has to come out of the \
             game's own bytes every time"
        );
    }

    #[test]
    fn a_file_the_game_never_guarded_passes_through_both_ways_untouched() {
        let shipped = fixture::game("A Title", "", "MS Gothic");
        let at = Path::new("/game/Data/BasicData/Game.dat");

        let mut body = opened(shipped.clone(), at).expect("it opens");
        assert_eq!(body, shipped);

        sealed(&shipped, at, &mut body).expect("it seals back");
        assert_eq!(body, shipped);
    }

    #[test]
    fn a_pro_guarded_game_dat_is_opened_and_sealed_back_out_of_its_own_bytes_alone() {
        let at = Path::new("/game/Data/BasicData/Game.dat");
        let plain = fixture::game("A Title", " + DLC", "MS Gothic");

        for scolding in [
            b"".as_slice(),
            b"Extracting data violates the guidelines.\0".as_slice(),
        ] {
            let shipped = unprot::as_shipped(&plain, scolding, unprot::Kind::Game);

            let mut body = opened(shipped.clone(), at).expect("it opens");
            assert_eq!(
                body, plain,
                "Game.dat is the one kind whose own size has to be written fresh when the \
                 hundred and forty three byte head comes off"
            );

            sealed(&shipped, at, &mut body).expect("it seals back");
            assert_eq!(
                body, shipped,
                "the guard is kept nowhere, so sealing it back has to work the whole encryption \
                 out of the game's own bytes again"
            );
        }
    }

    #[test]
    fn an_older_game_dat_behind_a_scolding_is_opened_far_enough_to_be_refused_by_name() {
        let mut shipped = b"Extracting data violates the guidelines.\0".to_vec();
        shipped.extend_from_slice(&[0x00, 0x57, 0x00, 0x00, 0x4F, 0x4C, 0x00, 0x46, 0x4D, 0x00]);
        shipped.extend([0u8; 40]);

        let body = opened(shipped, Path::new("/game/Data/BasicData/Game.dat")).expect("it opens");

        assert_eq!(
            game::read(&body).err(),
            Some("convert the game with Wolf RPG Editor 3".to_string()),
            "the scolding has to come off a Wolf 2 game as well, or the reader is told it handed \
             over something that is not a Wolf game at all instead of what to do about it"
        );
    }

    fn a_plan() -> Vec<u8> {
        fixture::database(&[fixture::Type {
            name: "\u{30a2}\u{30a4}\u{30c6}\u{30e0}",
            fields: &["\u{540d}\u{524d}"],
            words: &[0],
            entries: &[&["\u{7dd1}\u{8336}"]],
            rows: &[],
            named_by: None,
        }])
        .0
    }

    #[test]
    fn a_plan_the_game_scrambled_is_handed_back_in_the_clear() {
        let plan = a_plan();

        let mut hidden = plan.clone();
        unprot::unplanned(&mut hidden);
        assert_ne!(hidden, plan);

        assert_eq!(
            opened(hidden, Path::new("/game/Data/BasicData/DataBase.project")),
            Ok(plan),
            "a Pro game scrambles the plan beside its database, and without the plan the database \
             is a wall of numbers nobody can name"
        );
    }

    #[test]
    fn a_plan_the_game_scrambled_is_scrambled_again_before_it_goes_back_in() {
        let plan = a_plan();

        let mut hidden = plan.clone();
        unprot::unplanned(&mut hidden);

        let at = Path::new("/game/Data/BasicData/SysDatabase.project");
        assert!(
            wraps(at),
            "the places a plan names are written back into it now, so a plan has to be handed \
             the wrapping it came in"
        );

        let mut body = opened(hidden.clone(), at).expect("it opens");
        sealed(&hidden, at, &mut body).expect("it seals back");

        assert_eq!(
            body, hidden,
            "the game reads its own plan through the scrambling it wrote, so laying the plain \
             bytes back where a scrambled file stood leaves it reading a wall of noise"
        );

        let mut body = opened(plan.clone(), at).expect("it opens");
        sealed(&plan, at, &mut body).expect("it seals back");

        assert_eq!(
            body, plan,
            "and a plan the game never scrambled is left exactly where it stands"
        );
    }

    #[test]
    fn a_map_that_would_scramble_into_a_plan_is_still_handed_straight_back() {
        let mut raw = a_plan();
        unprot::unplanned(&mut raw);

        let at = Path::new("/game/Data/MapData/Dungeon.mps");

        assert_eq!(
            opened(raw.clone(), at),
            Ok(raw.clone()),
            "which wrapping a file carries is read off its name and never sniffed out of its \
             bytes, or the first map that happens to scramble into something that parses comes \
             back rewritten"
        );

        let mut body = raw.clone();
        assert_eq!(sealed(&raw, at, &mut body), Ok(()));
        assert_eq!(body, raw);
    }
}
