use anyhow::{Result, bail};
use std::collections::{BTreeMap, BTreeSet};

pub const FILE: &str = "catalog.bin";
pub const OLDER: &str = "catalog.json";

const FIELD: usize = 4;
const CRC_BEFORE_SIZE: usize = FIELD;
const MARKER_AFTER_SIZE: usize = FIELD;
const LEAST_AGREE: usize = 2;

pub struct Checks {
    pub catalog: Option<Vec<u8>>,
    pub lifted: Vec<u32>,
    pub unconfirmed: Vec<u32>,
}

pub fn lift(raw: &[u8], all_sizes: &[u32], wanted_sizes: &[u32]) -> Result<Checks> {
    let looked = spots(
        raw,
        &all_sizes.iter().chain(wanted_sizes).copied().collect(),
    );

    let Some(marker) = marker_of(raw, all_sizes, &looked) else {
        let unconfirmed = wanted_sizes
            .iter()
            .copied()
            .filter(|size| looked.get(size).is_some_and(|spots| !spots.is_empty()))
            .collect();

        return Ok(Checks {
            catalog: None,
            lifted: Vec::new(),
            unconfirmed,
        });
    };

    let mut want: BTreeMap<u32, usize> = BTreeMap::new();
    for size in wanted_sizes {
        *want.entry(*size).or_default() += 1;
    }

    let mut out: Option<Vec<u8>> = None;
    let mut lifted = Vec::new();
    let mut muddled = Vec::new();

    for (size, count) in &want {
        let found: Vec<usize> = looked
            .get(size)
            .into_iter()
            .flatten()
            .copied()
            .filter(|at| reads(raw, at + MARKER_AFTER_SIZE) == Some(marker))
            .collect();

        if found.is_empty() {
            continue;
        }

        if found.len() == *count && found.iter().all(|at| *at >= CRC_BEFORE_SIZE) {
            let fresh = out.get_or_insert_with(|| raw.to_vec());
            for at in &found {
                let crc = at - CRC_BEFORE_SIZE;
                fresh[crc..crc + FIELD].copy_from_slice(&0u32.to_le_bytes());
            }
            lifted.extend(vec![*size; *count]);
        } else {
            muddled.push(*size);
        }
    }

    if !muddled.is_empty() {
        bail!(
            "this catalog names {} in more than one place, and picking the wrong one would break a \
             bundle nobody asked to change",
            muddled
                .iter()
                .map(|size| format!("a bundle of {size} byte(s)"))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    Ok(Checks {
        catalog: out,
        lifted,
        unconfirmed: Vec::new(),
    })
}

fn marker_of(raw: &[u8], all_sizes: &[u32], looked: &BTreeMap<u32, Vec<usize>>) -> Option<u32> {
    let every: BTreeSet<u32> = all_sizes.iter().copied().collect();
    let present = every
        .iter()
        .filter(|size| looked.get(size).is_some_and(|spots| !spots.is_empty()))
        .count();
    let mut seen: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();

    for size in &every {
        for at in looked.get(size).into_iter().flatten() {
            if let Some(after) = reads(raw, at + MARKER_AFTER_SIZE) {
                seen.entry(after).or_default().insert(*size);
            }
        }
    }

    seen.into_iter()
        .max_by_key(|(_, sizes)| sizes.len())
        .filter(|(_, sizes)| sizes.len() >= LEAST_AGREE && sizes.len() * 2 > present)
        .map(|(after, _)| after)
}

fn spots(raw: &[u8], wanted: &BTreeSet<u32>) -> BTreeMap<u32, Vec<usize>> {
    let mut found: BTreeMap<u32, Vec<usize>> = BTreeMap::new();

    for (at, four) in raw.windows(FIELD).enumerate() {
        let value = u32::from_le_bytes([four[0], four[1], four[2], four[3]]);
        if wanted.contains(&value) {
            found.entry(value).or_default().push(at);
        }
    }

    found
}

fn reads(raw: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        raw.get(at..at + FIELD)?.try_into().ok()?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MARKER: u32 = 37550;

    fn record(crc: u32, size: u32) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&MARKER.to_le_bytes());
        out.extend_from_slice(&[0xAB; 12]);

        out
    }

    fn forge(bundles: &[(u32, u32)]) -> Vec<u8> {
        let mut out = vec![0x7f; 32];
        for (crc, size) in bundles {
            out.extend_from_slice(&record(*crc, *size));
        }
        out.extend_from_slice(&[0x7f; 16]);

        out
    }

    fn crc_of(raw: &[u8], size: u32) -> u32 {
        let looked = spots(raw, &BTreeSet::from([size]));
        let at = looked
            .get(&size)
            .into_iter()
            .flatten()
            .copied()
            .find(|at| reads(raw, at + 4) == Some(MARKER))
            .expect("a record for that size");

        reads(raw, at - 4).expect("a crc before it")
    }

    #[test]
    fn the_check_is_lifted_only_for_the_bundles_that_changed() {
        let all = [869_764u32, 2226, 2350];
        let raw = forge(&[
            (1_244_097_045, 869_764),
            (852_316_474, 2226),
            (333_911_545, 2350),
        ]);

        let lifted = lift(&raw, &all, &[869_764, 2226]).unwrap();
        let out = lifted.catalog.expect("a rewritten catalog");

        assert_eq!(lifted.lifted.len(), 2);
        assert!(lifted.unconfirmed.is_empty());
        assert_eq!(crc_of(&out, 869_764), 0);
        assert_eq!(crc_of(&out, 2226), 0);
        assert_eq!(
            crc_of(&out, 2350),
            333_911_545,
            "a bundle nobody touched keeps its check"
        );
        assert_eq!(out.len(), raw.len(), "the catalog must not change size");
    }

    #[test]
    fn a_number_that_only_looks_like_a_size_is_not_mistaken_for_a_record() {
        let all = [4444u32, 5555];
        let mut raw = forge(&[(11, 4444), (22, 5555)]);

        let decoy = raw.len();
        raw.extend_from_slice(&4444u32.to_le_bytes());
        raw.extend_from_slice(&999u32.to_le_bytes());

        let lifted = lift(&raw, &all, &[4444]).unwrap();
        let out = lifted.catalog.expect("a rewritten catalog");

        assert_eq!(lifted.lifted, [4444]);
        assert_eq!(crc_of(&out, 4444), 0);
        assert_eq!(
            &out[decoy..decoy + 4],
            &4444u32.to_le_bytes(),
            "the decoy is left exactly as it was"
        );
    }

    #[test]
    fn a_size_the_catalog_names_twice_is_refused_out_loud() {
        let all = [7777u32, 8888];
        let raw = forge(&[(11, 7777), (22, 8888), (33, 7777)]);

        assert!(
            lift(&raw, &all, &[7777]).is_err(),
            "guessing between two records could zero the wrong bundle"
        );
        assert!(lift(&raw, &all, &[8888]).is_ok());
    }

    #[test]
    fn a_catalog_holding_only_some_of_the_game_still_elects_its_marker() {
        let all = [4444u32, 5555, 6666, 7777, 8888, 9999];
        let raw = forge(&[(11, 4444), (22, 5555)]);

        let lifted = lift(&raw, &all, &[4444]).unwrap();

        assert_eq!(
            lifted.lifted,
            [4444],
            "bundles split across another catalog must not out-vote the ones in this one"
        );
        assert_eq!(
            crc_of(&lifted.catalog.expect("a rewritten catalog"), 4444),
            0
        );
    }

    #[test]
    fn two_changed_bundles_sharing_a_size_lift_both_records() {
        let all = [7777u32, 8888];
        let raw = forge(&[(11, 7777), (22, 8888), (33, 7777)]);

        let lifted = lift(&raw, &all, &[7777, 7777]).unwrap();
        let out = lifted.catalog.expect("a rewritten catalog");

        assert_eq!(lifted.lifted, [7777, 7777]);
        assert!(lifted.unconfirmed.is_empty());
        assert_eq!(crc_of(&out, 7777), 0);
        assert_eq!(
            crc_of(&out, 8888),
            22,
            "the bundle nobody changed keeps its check"
        );
    }

    #[test]
    fn a_bundle_the_catalog_never_heard_of_needs_nothing_lifted() {
        let all = [4444u32, 5555];
        let raw = forge(&[(11, 4444), (22, 5555)]);

        let lifted = lift(&raw, &all, &[9999]).unwrap();

        assert!(
            lifted.lifted.is_empty(),
            "a bundle loaded without Addressables has no check to lift"
        );
        assert!(
            lifted.unconfirmed.is_empty(),
            "and it is not held up as unsure either"
        );
        assert!(lifted.catalog.is_none(), "and the catalog is left alone");
    }

    #[test]
    fn one_bundle_alone_cannot_vote_a_marker_into_being() {
        let raw = forge(&[(11, 4444)]);

        let lifted = lift(&raw, &[4444], &[4444]).unwrap();

        assert!(
            lifted.lifted.is_empty(),
            "a single coincidental match must never decide where a crc lives"
        );
        assert!(lifted.catalog.is_none());
        assert_eq!(
            lifted.unconfirmed,
            [4444],
            "a size the catalog may be checking is never called safe"
        );
    }

    #[test]
    fn a_file_that_is_not_a_catalog_changes_nothing() {
        let lifted = lift(b"not a catalog at all", &[1234], &[1234]).unwrap();

        assert!(lifted.lifted.is_empty());
        assert!(lifted.unconfirmed.is_empty());
        assert!(
            lifted.catalog.is_none(),
            "any file may be called catalog.bin, and writing a rebuilt one over something we \
             did not understand would break whatever it really was"
        );
    }
}
