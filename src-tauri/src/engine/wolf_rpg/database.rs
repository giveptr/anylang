use crate::engine::wolf_rpg::coder::{self, Reader};
use crate::engine::wolf_rpg::held::{self, Held, Kind, Piece, Said};

pub const SUFFIX: &str = "dat";
pub const PLAN: &str = "project";

const MAGIC: [u8; 9] = [0x57, 0x00, 0x00, 0x4F, 0x4C, 0x00, 0x46, 0x4D, 0x00];
const UTF8_AT: usize = 5;
const FROM: usize = 1;
const VERSION_AT: usize = 10;
const HEAD: usize = 11;
const PACKED: u8 = 0xC4;

const TYPE_MAGIC: [u8; 4] = [0xFE, 0xFF, 0xFF, 0xFF];
const NAMED_TYPE: u32 = 0x0001_D4C0;
const SAID_FROM: u32 = 0x07D0;

pub const SYSTEM: &str = "SysDatabase";
const MAP_SETTINGS: usize = 0;

pub struct Counted {
    pub named: String,
    pub fields: Vec<String>,
    pub entries: Vec<Said>,
}

pub fn plan(raw: &[u8]) -> Result<Vec<Counted>, String> {
    let mut reader = Reader::over(raw, 0);
    let count = reader.count()?;
    let mut types = Vec::with_capacity(count);

    for which in 0..count {
        types.push(planned(&mut reader).map_err(|why| format!("type {which} of the plan: {why}"))?);
    }

    reader.ended()?;

    Ok(types)
}

pub fn places(raw: &[u8], plan: &[Counted]) -> Result<Held, String> {
    let counted = plan
        .get(MAP_SETTINGS)
        .ok_or_else(|| "this plan names no types at all".to_string())?;

    let pieces = counted
        .entries
        .iter()
        .enumerate()
        .map(|(which, said)| Piece {
            spot: format!("t{MAP_SETTINGS}/d{which}"),
            kind: Kind::Value,
            said: vec![said.clone()],
        })
        .collect();

    Ok(Held {
        plain: raw.to_vec(),
        shape: held::Shape::Plain,
        pieces,
    })
}

fn planned(reader: &mut Reader) -> Result<Counted, String> {
    let named = reader.said()?.0;

    let count = reader.count()?;
    let mut fields = Vec::with_capacity(count);
    for _ in 0..count {
        fields.push(reader.said()?.0);
    }

    let count = reader.count()?;
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        let (text, at) = reader.said()?;
        entries.push(Said { text, at });
    }

    reader.past_said()?;

    let listed = reader.count()?;
    if listed < fields.len() {
        return Err(format!(
            "{} fields were named and only {listed} were typed",
            fields.len()
        ));
    }
    reader.skip(listed)?;

    let count = reader.count()?;
    reader.past_saids(count)?;

    reader.past_said_lists()?;
    reader.past_word_lists()?;

    let count = reader.count()?;
    for _ in 0..count {
        reader.word()?;
    }

    Ok(Counted {
        named,
        fields,
        entries,
    })
}

pub fn read(raw: &[u8], plan: &[Counted]) -> Result<Held, String> {
    coder::spelled(&MAGIC, UTF8_AT, raw, FROM)?;
    let version = coder::byte_at(raw, VERSION_AT)?;
    let (plain, shape) = held::opened(raw, HEAD, version == PACKED)?;

    let mut reader = Reader::over(&plain, HEAD);
    let mut pieces = Vec::new();

    let count = reader.count()?;
    if count != plan.len() {
        return Err(format!(
            "the plan names {} types and the data holds {count}",
            plan.len()
        ));
    }

    for (which, counted) in plan.iter().enumerate() {
        one(&mut reader, which, counted, &mut pieces)
            .map_err(|why| format!("type {which} of the data: {why}"))?;
    }

    let closing = reader.byte()?;
    if closing != version {
        return Err(format!(
            "a database ends with the {version:#04x} it opened with and this one with \
             {closing:#04x}"
        ));
    }

    reader.ended()?;

    Ok(Held {
        plain,
        shape,
        pieces,
    })
}

fn one(
    reader: &mut Reader,
    which: usize,
    counted: &Counted,
    pieces: &mut Vec<Piece>,
) -> Result<(), String> {
    reader.expect(&TYPE_MAGIC, "the head of a database type")?;

    let named = reader.word()?;
    let fields = reader.count()?;
    if fields > counted.fields.len() {
        return Err(format!(
            "the plan names {} fields and the data holds {fields}",
            counted.fields.len()
        ));
    }
    if named == NAMED_TYPE {
        reader.past_said()?;
    }

    let mut saids = 0;
    let mut field_of = vec![usize::MAX; fields];
    for field in 0..fields {
        let told = reader.word()?;
        if told < SAID_FROM {
            continue;
        }

        let slot = (told - SAID_FROM) as usize;
        if slot >= fields {
            return Err(format!(
                "field {field} is stored past the end of its own type"
            ));
        }
        field_of[slot] = field;
        saids += 1;
    }

    if field_of[..saids].contains(&usize::MAX) {
        return Err("this type leaves a gap where one of its written fields is stored".to_string());
    }

    let listed = reader.word()? as usize;
    let entries = counted.entries.len().min(listed);
    let numbers = fields - saids;

    let mut rows: Vec<Vec<Piece>> = Vec::with_capacity(entries);

    for entry in 0..entries {
        for _ in 0..numbers {
            reader.word()?;
        }

        let mut row = Vec::with_capacity(saids);
        for &field in &field_of[..saids] {
            let (text, at) = reader.said()?;

            row.push(Piece {
                spot: format!("t{which}/d{entry}/f{field}"),
                kind: Kind::Value,
                said: vec![Said { text, at }],
            });
        }

        rows.push(row);
    }

    for at in naming(&rows, &counted.entries) {
        for row in rows.iter_mut() {
            if let Some(one) = row.get_mut(at) {
                one.kind = Kind::Naming;
            }
        }
    }

    pieces.extend(rows.into_iter().flatten());

    Ok(())
}

fn matching(rows: &[Vec<Piece>], entries: &[Said], at: usize) -> usize {
    let mut matched = 0;

    for (entry, row) in rows.iter().enumerate() {
        let (Some(one), Some(name)) = (row.get(at), entries.get(entry)) else {
            return 0;
        };

        let said = one.said[0].text.trim();
        if said.is_empty() {
            continue;
        }

        if said != name.text.trim() {
            return 0;
        }

        matched += 1;
    }

    matched
}

fn naming(rows: &[Vec<Piece>], entries: &[Said]) -> Vec<usize> {
    let Some(first) = rows.first() else {
        return Vec::new();
    };

    let mut best = 0;
    let mut told = Vec::new();

    for at in 0..first.len() {
        let matched = matching(rows, entries, at);

        if matched == 0 || matched < best {
            continue;
        }

        if matched > best {
            best = matched;
            told.clear();
        }

        told.push(at);
    }

    told
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::wolf_rpg::reached::Reached;
    use crate::engine::wolf_rpg::{fixture, harvest};

    #[test]
    fn the_places_a_plan_names_its_maps_by_are_read_out_of_it_and_written_back_into_it() {
        let south = "\u{5357}\u{5730}\u{533a}\u{30b3}\u{30f3}\u{30d3}\u{30cb}";
        let park = "\u{4e2d}\u{592e}\u{516c}\u{5712}\u{5357}\u{9580}";

        let (plan_raw, _) = fixture::database(&[fixture::Type {
            name: "\u{30de}\u{30c3}\u{30d7}\u{8a2d}\u{5b9a}",
            fields: &["\u{540d}"],
            words: &[0],
            entries: &[&[" "], &[" "]],
            rows: &[south, park],
            named_by: None,
        }]);

        let types = plan(&plan_raw).expect("a plan");
        let held = places(&plan_raw, &types).expect("the places a plan names");

        assert_eq!(
            held.pieces
                .iter()
                .map(|one| (one.spot.as_str(), one.said[0].text.as_str()))
                .collect::<Vec<_>>(),
            [("t0/d0", south), ("t0/d1", park)],
            "the banner a game raises over a map spells the name out of its plan and nowhere \
             else, so a reader that never opens the plan leaves every place name behind"
        );

        let mut reached = Reached::new();
        harvest::homes_in(&held.pieces, "BasicData/SysDatabase.project", &mut reached);

        assert!(
            !reached.a_name(south),
            "a map is one label among thousands, so letting its name stand as the key for that \
             spelling would take every line the game happens to spell the same way out of the \
             reader's sight and slave it to whatever the map was called"
        );

        let edits = vec![(
            held.pieces[0].said[0].at.clone(),
            coder::line("South District Store"),
        )];
        let body = held::wrapped(&held, edits).expect("a plan written back");
        let again = plan(&body).expect("a plan this reader can still take in");

        assert_eq!(
            (
                again[0].entries[0].text.as_str(),
                again[0].entries[1].text.as_str()
            ),
            ("South District Store", park),
            "a place name is stored the length of it and then the letters, so laying a longer \
             wording in has to move everything after it: the plan has to read back whole, with \
             the name beside it untouched"
        );
    }

    #[test]
    fn every_written_field_of_every_entry_comes_out_under_the_field_it_belongs_to() {
        let (plan_raw, data) = fixture::database(&[fixture::Type {
            name: "\u{30a2}\u{30a4}\u{30c6}\u{30e0}",
            fields: &["\u{540d}\u{524d}", "\u{5024}\u{6bb5}", "\u{8aac}\u{660e}"],
            words: &[0, 2],
            entries: &[
                &["Green Tea", "Heals 30 HP."],
                &["Luxury Tea", "Heals 100 HP."],
            ],
            rows: &[],
            named_by: None,
        }]);

        let plan = plan(&plan_raw).expect("a plan");
        assert_eq!(
            plan[0].fields,
            ["\u{540d}\u{524d}", "\u{5024}\u{6bb5}", "\u{8aac}\u{660e}"],
            "a command reaches a field by the name the plan gives it, and the plan is a file no \
             translation is ever written back into, so the reader has to carry those names out"
        );
        assert_eq!(plan[0].entries.len(), 2);

        let held = read(&data, &plan).expect("a database");

        assert_eq!(
            held.pieces
                .iter()
                .map(|one| (one.spot.as_str(), one.said[0].text.as_str()))
                .collect::<Vec<_>>(),
            [
                ("t0/d0/f0", "Green Tea"),
                ("t0/d0/f2", "Heals 30 HP."),
                ("t0/d1/f0", "Luxury Tea"),
                ("t0/d1/f2", "Heals 100 HP."),
            ],
            "the numbered field between them is not a line, and the spot has to skip it or a \
             description would land in a price"
        );
    }

    #[test]
    fn the_field_a_row_is_named_by_is_told_apart_from_the_words_that_only_look_like_it() {
        let key = "\u{30df}\u{30ca}";
        let plus = "\u{30df}\u{30ca}2";

        let (plan_raw, data) = fixture::database(&[fixture::Type {
            name: "\u{30ad}\u{30e3}\u{30e9}\u{7acb}\u{3061}\u{7d75}\u{8a2d}\u{5b9a}",
            fields: &[
                "\u{30ad}\u{30e3}\u{30e9}\u{540d}",
                "\u{30c6}\u{30ad}\u{30b9}\u{30c8}\u{8868}\u{8a18}\u{540d}",
            ],
            words: &[0, 1],
            entries: &[&[key, key], &[plus, key]],
            rows: &[],
            named_by: Some(0),
        }]);

        let plan = plan(&plan_raw).expect("a plan");
        let held = read(&data, &plan).expect("a database");

        assert_eq!(
            held.pieces
                .iter()
                .map(|one| (one.spot.as_str(), one.kind.clone()))
                .collect::<Vec<(&str, Kind)>>(),
            [
                ("t0/d0/f0", Kind::Naming),
                ("t0/d0/f1", Kind::Value),
                ("t0/d1/f0", Kind::Naming),
                ("t0/d1/f1", Kind::Value),
            ],
            "both fields spell the name of the first row, and only the one that spells every \
             row's name is the one the game finds a row by"
        );
    }

    #[test]
    fn a_row_the_author_named_by_hand_does_not_hide_the_field_the_rest_are_named_by() {
        let (plan_raw, data) = fixture::database(&[fixture::Type {
            name: "\u{30ad}\u{30e3}\u{30e9}\u{7acb}\u{3061}\u{7d75}\u{8a2d}\u{5b9a}",
            fields: &["\u{30ad}\u{30e3}\u{30e9}\u{540d}"],
            words: &[0],
            entries: &[&["\u{30b5}\u{30ad}"], &[""], &["\u{30a2}\u{30ad}\u{30e9}"]],
            rows: &[],
            named_by: Some(0),
        }]);

        let mut plan = plan(&plan_raw).expect("a plan");
        plan[0].entries[1].text = "\u{4ee5}\u{4e0b}\u{30b2}\u{30b9}\u{30c8}\u{67a0}".to_string();

        let held = read(&data, &plan).expect("a database");

        assert!(
            held.pieces.iter().all(|one| one.kind == Kind::Naming),
            "the author typed a divider in over one blank row, and one row named by hand is no \
             reason to stop treating the field as what the other rows are found by"
        );
    }

    #[test]
    fn a_type_whose_rows_are_named_by_hand_hands_no_field_over_as_a_key() {
        let (plan_raw, data) = fixture::database(&[fixture::Type {
            name: "\u{4ef2}\u{9593}\u{306e}\u{96d1}\u{8ac7}",
            fields: &["\u{30bb}\u{30ea}\u{30d5}"],
            words: &[0],
            entries: &[&["\u{304a}\u{306f}\u{306a}\u{3057}"], &["\u{3044}\u{3084}"]],
            rows: &[],
            named_by: None,
        }]);

        let plan = plan(&plan_raw).expect("a plan");
        let held = read(&data, &plan).expect("a database");

        assert!(
            held.pieces.iter().all(|one| one.kind == Kind::Value),
            "the author typed these row names in by hand, so nothing in the file itself is the \
             name a reach spells out, and every field is a line to read"
        );
    }

    #[test]
    fn a_plan_and_a_data_file_that_disagree_are_refused_rather_than_read_askew() {
        let (plan_raw, data) = fixture::database(&[fixture::Type {
            name: "One",
            fields: &["a"],
            words: &[0],
            entries: &[&["x"]],
            rows: &[],
            named_by: None,
        }]);

        let mut plan = plan(&plan_raw).expect("a plan");
        plan.push(Counted {
            named: "Two".to_string(),
            fields: vec!["a".to_string()],
            entries: vec![Said {
                text: "row0".to_string(),
                at: 0..8,
            }],
        });

        assert!(read(&data, &plan).is_err());
    }
}
