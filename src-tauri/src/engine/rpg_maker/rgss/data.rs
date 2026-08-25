use crate::engine::rpg_maker::harvest::{self, Dialect, Fix, Found, Slot, Spot, Step};
use crate::engine::rpg_maker::rgss::marshal::{self, Text};
use crate::engine::rpg_maker::text;
use crate::engine::sheet;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

pub const SUFFIX: &str = "rvdata2";

struct Rgss;

impl Dialect for Rgss {
    fn doubtful(&self, text: &str) -> bool {
        text::symbolic_line(text)
    }

    fn extra(&self, _: &[Value], index: usize, _: &Spot, _: &mut Vec<Found>) -> usize {
        index + 1
    }
}

fn path_of(spot: &Spot) -> String {
    spot.0
        .iter()
        .map(|step| match step {
            Step::Key(key) => key.clone(),
            Step::Index(index) => index.to_string(),
        })
        .collect::<Vec<String>>()
        .join("/")
}

pub fn lines_of(bytes: &[u8]) -> Result<Vec<sheet::Line>, String> {
    let held = marshal::read(bytes)?;
    let (units, slots) = harvest::run(&held.view, &Rgss);

    Ok(units
        .into_iter()
        .zip(slots)
        .filter_map(|(unit, slot)| {
            Some(sheet::Line {
                spot: sheet_key(&slot)?,
                said: unit.text,
                offer: unit.offer,
            })
        })
        .collect())
}

pub fn spliced(bytes: &[u8], said: &BTreeMap<String, String>) -> Result<(Vec<u8>, u32), String> {
    let sheet = marshal::read(bytes)?;
    let (units, slots) = harvest::run(&sheet.view, &Rgss);

    let found: HashMap<&[Step], &Text> = sheet
        .texts
        .iter()
        .map(|text| (text.path.as_slice(), text))
        .collect();

    let held = harvest::written(
        &units,
        &slots,
        |unit, slot| {
            sheet_key(slot)
                .and_then(|key| said.get(&key))
                .filter(|now| *now != &unit.text)
                .cloned()
        },
        |fix, translation, _| {
            (fix == Fix::Raw && harvest::fits_raw(translation)).then(|| translation.to_string())
        },
    );

    let mut edits: Vec<(&Text, Vec<u8>)> = Vec::new();

    for (spot, row) in &held.whole {
        if let Some(text) = kept(&found, spot, row) {
            edits.push(text);
        }
    }

    for (spot, blocks) in held.inside {
        let Some(text) = found.get(spot.0.as_slice()) else {
            continue;
        };

        if bytes.get(text.at.clone()) != Some(text.text.as_bytes()) {
            continue;
        }

        let mut whole = text.text.clone();
        harvest::splice(&mut whole, blocks);

        if whole != text.text {
            edits.push((text, whole.into_bytes()));
        }
    }

    let written = edits.len() as u32;

    Ok((marshal::spliced(bytes, &edits), written))
}

fn kept<'t>(
    found: &HashMap<&[Step], &'t Text>,
    spot: &Spot,
    said: &str,
) -> Option<(&'t Text, Vec<u8>)> {
    let text = found.get(spot.0.as_slice())?;

    (text.text != said).then(|| (*text, said.as_bytes().to_vec()))
}

fn sheet_key(slot: &Slot) -> Option<String> {
    match slot {
        Slot::Whole(spots) | Slot::Lines(spots, _) => Some(path_of(spots.first()?)),
        Slot::Inside(spot, at, _) => Some(format!("{}@{}", path_of(spot), at.start)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Offer;
    use crate::engine::rpg_maker::rgss::fixture::{
        self, command, list, map, number, object, said, stream,
    };

    #[test]
    fn a_name_a_script_looks_up_in_the_system_list_reaches_the_sheet_already_held_back() {
        let raw = stream(&object(
            "RPG::System",
            &[(
                "elements",
                list(&[said(""), said("\u{7269}\u{7406}"), said("\u{9b54}\u{529b}")]),
            )],
        ));

        let lines = lines_of(&raw).expect("a sheet");

        assert_eq!(lines.len(), 2, "the blank slot at the front is no line");
        assert!(
            lines.iter().all(|line| line.offer == Offer::Listed),
            "the shared harvest already worked out that a script reaches these by name, and \
             handing the sheet a bare pair threw that away: the reader was shown a pile filter \
             with nothing behind it while the model was sent names that break the lookup"
        );
    }

    #[test]
    fn an_identifier_shaped_value_is_listed_while_speech_is_still_asked() {
        let raw = stream(&object(
            "RPG::Item",
            &[
                ("name", said("00makai_16")),
                (
                    "description",
                    said("\u{3088}\u{304f}\u{52b9}\u{304f}\u{85ac}\u{3002}"),
                ),
            ],
        ));

        let lines = lines_of(&raw).expect("a sheet");
        let offer_of = |what: &str| {
            lines
                .iter()
                .find(|line| line.said == what)
                .expect("the line is on the sheet")
                .offer
        };

        assert_eq!(
            offer_of("00makai_16"),
            Offer::Listed,
            "a VX Ace script can build a path or a lookup from this name, and the JS branch \
             already lists the same shape rather than asking a model to translate it"
        );
        assert_eq!(
            offer_of("\u{3088}\u{304f}\u{52b9}\u{304f}\u{85ac}\u{3002}"),
            Offer::Asked
        );
    }

    #[test]
    fn a_message_run_is_one_line_of_the_sheet_and_goes_back_row_by_row() {
        let raw = map(&[
            command(101, &[said("")]),
            command(401, &[said("The door is")]),
            command(401, &[said("locked tight.")]),
        ]);

        let lines = lines_of(&raw).expect("a sheet");
        assert_eq!(
            lines
                .iter()
                .map(|line| (line.spot.as_str(), line.said.as_str()))
                .collect::<Vec<_>>(),
            vec![("list/1/parameters/0", "The door is\nlocked tight.")],
            "the run reads as one line to translate, keyed by where its first row sits"
        );

        let mut said = BTreeMap::new();
        said.insert(
            "list/1/parameters/0".to_string(),
            "\u{6249}\u{306f}\n\u{9589}\u{3055}\u{308c}\u{3066}\u{3044}\u{308b}".to_string(),
        );

        let (fresh, written) = spliced(&raw, &said).expect("a spliced sheet");
        assert_eq!(written, 2, "one line to translate, two rows to write");

        let after = marshal::read(&fresh).expect("it still reads");
        let rows: Vec<&str> = after.texts.iter().map(|one| one.text.as_str()).collect();
        assert_eq!(
            rows,
            [
                "",
                "\u{6249}\u{306f}",
                "\u{9589}\u{3055}\u{308c}\u{3066}\u{3044}\u{308b}"
            ]
        );
    }

    #[test]
    fn a_note_holding_bytes_this_reader_cannot_spell_is_left_exactly_as_it_was() {
        let mut body = b"<Help>Hello there</Help> ".to_vec();
        body.push(0x83);
        body.extend_from_slice(b" tail");

        let raw = stream(&list(&[object(
            "RPG::Item",
            &[("note", fixture::tagged(b'"', &body))],
        )]));

        let lines = lines_of(&raw).expect("a sheet");
        let done: BTreeMap<String, String> = lines
            .iter()
            .map(|line| (line.spot.clone(), "\u{3053}\u{3093}".to_string()))
            .collect();

        let (fresh, written) = spliced(&raw, &done).expect("a spliced sheet");

        assert_eq!(
            written, 0,
            "nothing is written into a string this reader cannot spell back"
        );
        assert_eq!(
            fresh, raw,
            "the note was read through a lossy decode, so writing part of it back would put a \
             replacement mark where the game kept a byte, and the note is the game's own data"
        );
    }

    #[test]
    fn two_note_blocks_in_one_note_keep_their_own_lines() {
        let raw = stream(&list(&[object(
            "RPG::Item",
            &[(
                "note",
                said("<Help>Hello there</Help>\n<Desc>Goodbye now</Desc>"),
            )],
        )]));

        let lines = lines_of(&raw).expect("a sheet");
        assert_eq!(lines.len(), 2, "each block is its own line of the sheet");

        let done: BTreeMap<String, String> = lines
            .iter()
            .map(|line| (line.spot.clone(), format!("[{}]", line.said)))
            .collect();

        let (fresh, written) = spliced(&raw, &done).expect("a spliced sheet");
        assert_eq!(written, 1, "both blocks land in the one note string");

        let after = marshal::read(&fresh).expect("it still reads");
        let note = after
            .texts
            .iter()
            .map(|one| one.text.as_str())
            .find(|text| text.contains("Help"))
            .expect("the note");
        assert_eq!(
            note, "<Help>[Hello there]</Help>\n<Desc>[Goodbye now]</Desc>",
            "each block takes its own translation, not its neighbour's"
        );
    }

    #[test]
    fn a_sheet_with_nothing_said_about_it_comes_back_byte_for_byte() {
        let raw = map(&[command(401, &[said("Hello.")])]);

        let (fresh, written) = spliced(&raw, &BTreeMap::new()).expect("a sheet");

        assert_eq!(written, 0);
        assert_eq!(
            fresh, raw,
            "an export that translated nothing changes nothing"
        );
    }

    #[test]
    fn a_choice_and_the_branch_that_mirrors_it_both_take_the_translation() {
        let raw = map(&[
            command(102, &[list(&[said("Yes")])]),
            command(402, &[number(0), said("Yes")]),
        ]);

        let lines = lines_of(&raw).expect("a sheet");
        assert_eq!(
            lines.len(),
            1,
            "the branch label is the same line as the choice"
        );

        let mut said = BTreeMap::new();
        said.insert(lines[0].spot.clone(), "\u{306f}\u{3044}".to_string());

        let (fresh, written) = spliced(&raw, &said).expect("a spliced sheet");
        assert_eq!(
            written, 2,
            "the choice and its branch label both get written"
        );

        let after = marshal::read(&fresh).expect("it still reads");
        let rows: Vec<&str> = after.texts.iter().map(|one| one.text.as_str()).collect();
        assert_eq!(rows, ["\u{306f}\u{3044}", "\u{306f}\u{3044}"]);
    }
}
