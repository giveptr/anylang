use crate::engine::unity::serial;
use anyhow::Result;
use std::collections::BTreeMap;

const WIDEST: usize = 16;
const NARROWEST: usize = 4;

pub fn rewrite(blob: &[u8], swaps: &BTreeMap<i64, Vec<u8>>) -> Result<Vec<u8>> {
    let layout = serial::layout(blob)?;

    let mut order: Vec<&serial::Slot> = layout.slots.iter().collect();
    order.sort_by_key(|slot| slot.start);

    let align = alignment(&order);

    let mut out = blob[..layout.data_at].to_vec();
    let mut moved: Vec<(&serial::Slot, usize, usize)> = Vec::with_capacity(order.len());

    for slot in &order {
        while !(out.len() - layout.data_at).is_multiple_of(align) {
            out.push(0);
        }

        let start = out.len() - layout.data_at;
        let body = match swaps.get(&slot.path_id) {
            Some(fresh) => fresh.as_slice(),
            None => {
                let from = layout.data_at + slot.start;
                &blob[from..from + slot.size]
            }
        };

        out.extend_from_slice(body);
        moved.push((slot, start, body.len()));
    }

    for (slot, start, size) in moved {
        slot.sits_at(&mut out, start, size)?;
    }

    layout.announce(&mut out)?;

    Ok(out)
}

fn alignment(slots: &[&serial::Slot]) -> usize {
    let mut unit = WIDEST;

    for slot in slots {
        while unit > NARROWEST && slot.start % unit != 0 {
            unit /= 2;
        }
    }

    unit
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::unity::{fake, text_asset};

    fn scripts(blob: &[u8]) -> BTreeMap<String, String> {
        text_asset::scripts_in(
            &serial::open(blob, "")
                .expect("a forged file has to open")
                .objects,
        )
        .into_iter()
        .map(|one| (one.stem, one.body))
        .collect()
    }

    #[test]
    fn rewriting_nothing_gives_the_very_same_bytes() {
        let blob = fake::forge(&[
            (11, "one", "Peter\nShe tilted her head.\n\n"),
            (22, "two", "Mary\nWait.\n\n"),
        ]);

        assert_eq!(
            rewrite(&blob, &BTreeMap::new()).unwrap(),
            blob,
            "an export that changed nothing must not touch a single byte"
        );
    }

    #[test]
    fn a_longer_script_moves_every_object_after_it_and_still_reads_back() {
        let blob = fake::forge(&[
            (11, "one", "Peter\nShe tilted her head.\n\n"),
            (22, "two", "Mary\nWait.\n\n"),
            (33, "three", "Walter\nGood morning.\n\n"),
        ]);

        let japanese = "Mary\n彼女は首をかたむけた、金の仮面が暗闇の中で光っていた。\n\n";
        let mut swaps = BTreeMap::new();
        swaps.insert(22, fake::a_text_asset("two", japanese));

        let fresh = rewrite(&blob, &swaps).unwrap();
        assert!(fresh.len() > blob.len(), "the file has to grow");

        let back = scripts(&fresh);
        assert_eq!(back.len(), 3, "no asset may be lost");
        assert_eq!(back["two"], japanese);
        assert_eq!(
            back["three"], "Walter\nGood morning.\n\n",
            "the object after the one that grew has to be found at its new place"
        );
        assert_eq!(back["one"], "Peter\nShe tilted her head.\n\n");
    }

    #[test]
    fn a_shorter_script_closes_the_gap_it_left() {
        let blob = fake::forge(&[
            (
                11,
                "one",
                "Peter\nShe tilted her head, the gold mask glinting.\n\n",
            ),
            (22, "two", "Mary\nWait.\n\n"),
        ]);

        let mut swaps = BTreeMap::new();
        swaps.insert(11, fake::a_text_asset("one", "Peter\nOh.\n\n"));

        let fresh = rewrite(&blob, &swaps).unwrap();
        assert!(fresh.len() < blob.len());

        let back = scripts(&fresh);
        assert_eq!(back["one"], "Peter\nOh.\n\n");
        assert_eq!(back["two"], "Mary\nWait.\n\n");
    }

    #[test]
    fn a_build_from_before_large_files_is_rewritten_by_its_own_widths() {
        let blob = fake::forge_as(
            17,
            &[
                (11, "one", "Peter\nShe tilted her head.\n\n"),
                (22, "two", "Mary\nWait.\n\n"),
                (33, "three", "Walter\nGood morning.\n\n"),
            ],
        );

        assert_eq!(
            rewrite(&blob, &BTreeMap::new()).unwrap(),
            blob,
            "an export that changed nothing must not touch a single byte here either"
        );

        let japanese = "Mary\n彼女は首をかたむけた。\n\n";
        let mut swaps = BTreeMap::new();
        swaps.insert(22, fake::a_text_asset("two", japanese));

        let fresh = rewrite(&blob, &swaps).unwrap();
        let back = scripts(&fresh);

        assert_eq!(
            back.len(),
            3,
            "an offset written eight bytes wide would run over the size and type beside it, and \
             the objects after it would be lost"
        );
        assert_eq!(back["two"], japanese);
        assert_eq!(back["three"], "Walter\nGood morning.\n\n");
        assert_eq!(back["one"], "Peter\nShe tilted her head.\n\n");
    }

    #[test]
    fn every_object_still_lands_where_the_file_wants_it() {
        let blob = fake::forge(&[(11, "one", "a\nbb\n\n"), (22, "two", "c\nddd\n\n")]);
        let mut swaps = BTreeMap::new();
        swaps.insert(11, fake::a_text_asset("one", "a\nbbbbbbbbbbbbbbbbbbb\n\n"));

        let fresh = rewrite(&blob, &swaps).unwrap();
        let layout = serial::layout(&fresh).unwrap();

        for slot in &layout.slots {
            assert!(
                slot.start.is_multiple_of(WIDEST),
                "the file this came from spaced its objects {WIDEST} bytes apart, so writing one \
                 back on a narrower step would move every object after it: {} sits at {}",
                slot.path_id,
                slot.start
            );
        }
    }
}
