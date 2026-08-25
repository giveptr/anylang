use crate::engine::unity::dotnet::Assemblies;
use crate::engine::unity::serial::{Container, Object, Value};
use crate::engine::unity::{Harvest, Known, blob, layout, localization, naming, serial};
use crate::engine::{Offer, sheet};
use anyhow::{Context, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

pub const NAME: &str = "mono_behaviour";
pub const KIND: &str = "MonoBehaviour";

const LOOSE: &str = "loose";
const BUCKET: i64 = 64;

fn bucket(path_id: i64) -> i64 {
    path_id.wrapping_div_euclid(BUCKET) * BUCKET
}

#[derive(Clone)]
pub struct Piece {
    pub id: String,
    pub field: String,
    pub at: Option<usize>,
    pub text: String,
}

const REGISTRY: &str = "references.RefIds";

fn each_text(value: &mut Value, path: String, out: &mut impl FnMut(&str, &mut Value)) {
    if path
        .strip_prefix(REGISTRY)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('['))
    {
        return;
    }

    match value {
        Value::Bytes(_) => out(&path, value),
        Value::Tree(kids) => {
            for (name, kid) in kids {
                let deep = match path.is_empty() {
                    true => name.clone(),
                    false => format!("{path}.{name}"),
                };

                each_text(kid, deep, out);
            }
        }
        Value::List(items) => {
            for (which, item) in items.iter_mut().enumerate() {
                each_text(item, format!("{path}[{which}]"), out);
            }
        }
        Value::Number(_) | Value::Real(_) => {}
    }
}

fn tree_pieces(object: &Object) -> Option<Vec<Piece>> {
    let mut value = object.value()?;
    let mut out = Vec::new();

    each_text(&mut value, String::new(), &mut |path, value| {
        let Some(text) = value.text() else {
            return;
        };

        if !text.is_empty() {
            out.push(Piece {
                id: path.to_string(),
                field: naming::stem(path),
                at: None,
                text,
            });
        }
    });

    (!out.is_empty()).then_some(out)
}

fn rebuilt(object: &Object, lines: &BTreeMap<String, String>) -> Result<Vec<u8>> {
    let mut value = object
        .value()
        .with_context(|| format!("object {} no longer reads by its type tree", object.path_id))?;

    let mut moved = false;

    each_text(&mut value, String::new(), &mut |path, value| {
        let Some(text) = value.text() else {
            return;
        };

        if let Some(said) = lines.get(path)
            && said != &text
        {
            *value = Value::Bytes(said.as_bytes().to_vec());
            moved = true;
        }
    });

    match moved {
        true => object.written(&value),
        false => Ok(object.body()?.into_owned()),
    }
}

pub fn shared_ids(nodes: &[&Container]) -> BTreeSet<i64> {
    let mut times: BTreeMap<i64, usize> = BTreeMap::new();
    for one in nodes {
        for object in &one.objects {
            if object.class_id == serial::MONO_BEHAVIOUR {
                *times.entry(object.path_id).or_default() += 1;
            }
        }
    }

    times
        .into_iter()
        .filter(|(_, seen)| *seen > 1)
        .map(|(path_id, _)| path_id)
        .collect()
}

const THEIRS: [&str; 4] = ["UnityEngine.", "Unity.", "TMPro.", "Cinemachine."];
const THEIR_WORDS: [&str; 2] = ["UnityEngine.Localization.", "UnityEngine.UIElements."];
const SHOWN: [&str; 2] = ["m_Text", "m_text"];

fn wiring(kind: Option<&str>, field: &str) -> bool {
    let Some(kind) = kind else {
        return false;
    };

    if THEIR_WORDS.iter().any(|one| kind.starts_with(one)) {
        return false;
    }
    if !THEIRS.iter().any(|one| kind.starts_with(one)) {
        return false;
    }

    let leaf = field.rsplit('.').next().unwrap_or(field);

    !SHOWN.contains(&leaf)
}

pub fn id_of(object: &Object, node: usize, shared: &BTreeSet<i64>) -> String {
    if shared.contains(&object.path_id) {
        format!("{}#{node}", object.path_id)
    } else {
        object.path_id.to_string()
    }
}

pub fn take(
    holder: &str,
    container: &Container,
    node: usize,
    nodes: usize,
    shared: &BTreeSet<i64>,
    known: &Known,
) -> Result<Vec<Harvest>> {
    let mut piled: BTreeMap<PathBuf, Vec<sheet::Line>> = BTreeMap::new();

    for object in &container.objects {
        let kind = known.classes.of(container, object);
        let pieces = pieces_in(container, object, kind, &known.assemblies, &known.books);
        if pieces.is_empty() {
            continue;
        }

        let held = id_of(object, node, shared);

        let mut byfield: BTreeMap<String, Vec<sheet::Line>> = BTreeMap::new();
        for one in pieces {
            let spot = format!("{held}/{}", one.id);
            byfield
                .entry(one.field)
                .or_default()
                .push(sheet::Line::read(spot, one.text));
        }

        let under = match kind {
            Some(kind) => PathBuf::from(NAME)
                .join(holder)
                .join(naming::named(kind, object.path_id)),
            None => PathBuf::from(NAME).join(holder),
        };

        let floor = bucket(object.path_id);
        let named = match nodes > 1 {
            false => format!("{floor}.{}", sheet::SUFFIX),
            true => format!("{floor}#{node}.{}", sheet::SUFFIX),
        };

        for (field, mut lines) in byfield {
            if wiring(kind, &field) {
                for line in &mut lines {
                    line.offer = Offer::Locked;
                }
            }

            piled
                .entry(
                    under
                        .join(naming::named(&field, object.path_id))
                        .join(&named),
                )
                .or_default()
                .extend(lines);
        }
    }

    Harvest::sheets(piled)
}

pub fn put_back(
    object: &Object,
    pieces: &[Piece],
    lines: &BTreeMap<String, String>,
) -> Result<Vec<u8>> {
    if pieces.iter().any(|one| one.at.is_none()) {
        return rebuilt(object, lines);
    }

    let swaps: Vec<(usize, usize, String)> = pieces
        .iter()
        .filter_map(|one| {
            let done = lines.get(&one.id)?;
            let at = one.at?;

            (*done != one.text).then(|| (at, one.text.len(), done.clone()))
        })
        .collect();

    let body = object.body()?;
    if swaps.is_empty() {
        return Ok(body.into_owned());
    }

    blob::splice(&body, &swaps).with_context(|| format!("object {}", object.path_id))
}

pub fn pieces_in(
    container: &Container,
    object: &Object,
    kind: Option<&str>,
    known: &Assemblies,
    books: &localization::Collections,
) -> Vec<Piece> {
    if object.class_id != serial::MONO_BEHAVIOUR
        || localization::sheet_of(container, object, kind, known, books).is_some()
        || books.shares(container, object)
    {
        return Vec::new();
    }

    let Ok(body) = object.body() else {
        return Vec::new();
    };

    if let Some(kind) = kind
        && let Ok(spots) = layout::strings_in(known, kind, &body)
    {
        return spots
            .into_iter()
            .filter(|spot| !spot.text.is_empty())
            .map(|spot| Piece {
                field: naming::stem(&spot.path),
                id: spot.path,
                at: Some(spot.at),
                text: spot.text,
            })
            .collect();
    }

    if let Some(pieces) = tree_pieces(object) {
        return pieces;
    }

    blob::strands(&body)
        .into_iter()
        .enumerate()
        .map(|(which, strand)| Piece {
            id: which.to_string(),
            field: LOOSE.to_string(),
            at: Some(strand.at),
            text: strand.text,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::sheet;
    use crate::engine::unity::{fake, mono_script, pictures};
    use std::path::Path;

    const HEADER: usize = 28;

    fn texts(body: &[u8]) -> Vec<String> {
        blob::strands(body)
            .into_iter()
            .map(|one| one.text)
            .collect()
    }

    fn packed(pieces: &[&str]) -> Vec<u8> {
        let mut out = vec![0u8; HEADER];
        out.extend_from_slice(&fake::strings(pieces));

        out
    }

    fn pieces_of(object: &Object) -> Vec<Piece> {
        pieces_in(
            &fake::container("sharedassets0.assets", Vec::new(), &[]),
            object,
            None,
            &nothing_known().assemblies,
            &localization::Collections::default(),
        )
    }

    fn nothing_known() -> Known {
        Known {
            assemblies: Assemblies::read(Path::new("/nowhere")),
            classes: mono_script::Names::default(),
            named: pictures::Named::default(),
            books: localization::Collections::default(),
        }
    }

    fn taken(pile: &str, container: &Container) -> Vec<Harvest> {
        take(pile, container, 0, 1, &BTreeSet::new(), &nothing_known()).expect("sheets")
    }

    fn taken_across(pile: &str, every: &[&Container], shared: &BTreeSet<i64>) -> Vec<PathBuf> {
        let known = nothing_known();

        every
            .iter()
            .enumerate()
            .flat_map(|(node, one)| {
                take(pile, one, node, every.len(), shared, &known).expect("sheets")
            })
            .map(|one| one.at)
            .collect()
    }

    #[test]
    fn with_no_layout_to_read_by_every_string_is_offered_and_none_is_judged() {
        let object = Object::forged(
            serial::MONO_BEHAVIOUR,
            11,
            packed(&["Wait for me.", "scene_042_intro", "unity", "LOAD GAME"]),
        );

        assert_eq!(
            pieces_of(&object)
                .into_iter()
                .map(|one| one.text)
                .collect::<Vec<_>>(),
            ["Wait for me.", "scene_042_intro", "unity", "LOAD GAME"],
            "a guess may not judge what is worth translating"
        );
    }

    #[test]
    fn a_translated_sheet_goes_back_in_and_the_rest_stays_put() {
        let object = Object::forged(
            serial::MONO_BEHAVIOUR,
            11,
            packed(&["Wait for me.", "scene_042_intro", "Is everything ok?"]),
        );

        let mut lines = BTreeMap::new();
        lines.insert("0".to_string(), "待ってくれ。".to_string());
        lines.insert("2".to_string(), "だいじょうぶ？".to_string());

        let body =
            put_back(&object, &pieces_of(&object), &lines).expect("a body that goes back together");

        assert_eq!(
            texts(&body),
            ["待ってくれ。", "scene_042_intro", "だいじょうぶ？"],
            "a line nobody staged comes through byte for byte"
        );
    }

    #[test]
    fn a_sheet_holding_nothing_leaves_the_body_byte_for_byte() {
        let body = packed(&["Wait for me."]);
        let object = Object::forged(serial::MONO_BEHAVIOUR, 1, body.clone());

        assert_eq!(
            put_back(&object, &pieces_of(&object), &BTreeMap::new()).unwrap(),
            body,
            "a sheet the reader cleared has to leave the object exactly as the game shipped it, \
             or clearing a translation still counts as changing the game"
        );
    }

    #[test]
    fn nothing_but_a_mono_behaviour_is_ever_read_this_way() {
        let body = packed(&["Wait for me."]);
        assert!(!pieces_of(&Object::forged(serial::MONO_BEHAVIOUR, 7, body.clone())).is_empty());
        assert!(
            pieces_of(&Object::forged(serial::TEXT_ASSET, 7, body)).is_empty(),
            "a text asset has a reader of its own"
        );
        assert!(
            pieces_of(&Object::forged(serial::MONO_BEHAVIOUR, 7, vec![0; HEADER])).is_empty(),
            "an object with no string in it must not become an empty sheet"
        );
    }

    #[test]
    fn a_field_unity_owns_is_wiring_unless_it_is_the_one_field_unity_draws() {
        for (kind, field) in [
            (
                "UnityEngine.UI.Button",
                "m_OnClick.m_PersistentCalls.m_Calls.m_MethodName",
            ),
            (
                "UnityEngine.UI.Button",
                "m_OnClick.m_PersistentCalls.m_Calls.m_TargetAssemblyTypeName",
            ),
            (
                "UnityEngine.UI.Button",
                "m_AnimationTriggers.m_NormalTrigger",
            ),
            (
                "UnityEngine.EventSystems.StandaloneInputModule",
                "m_HorizontalAxis",
            ),
            ("UnityEngine.Rendering.Universal.PostProcessData", "m_Name"),
            ("TMPro.TMP_Settings", "m_defaultFontAssetPath"),
        ] {
            assert!(
                wiring(Some(kind), field),
                "{kind}.{field} is how Unity wires a scene together, and a rail that sorts by how \
                 much text a row holds would bury every line a player reads under it"
            );
        }

        for (kind, field) in [
            ("UnityEngine.UI.Text", "m_Text"),
            ("UnityEngine.UI.Dropdown", "m_Options.m_Options.m_Text"),
            ("TMPro.TextMeshProUGUI", "m_text"),
        ] {
            assert!(
                !wiring(Some(kind), field),
                "{kind}.{field} is the field Unity puts on the screen"
            );
        }

        for (kind, field) in [("MapTrigger", "mapName"), ("DialogueText2", "dialogues")] {
            assert!(
                !wiring(Some(kind), field),
                "{kind} is the game's own class, and nobody but the game knows what {field} holds"
            );
        }

        for (kind, field) in [
            (
                "UnityEngine.Localization.Tables.StringTable",
                "m_TableData.m_Localized",
            ),
            (
                "UnityEngine.UIElements.VisualTreeAsset",
                "m_VisualElementAssets.m_Properties",
            ),
        ] {
            assert!(
                !wiring(Some(kind), field),
                "{kind} is Unity's, but holding a game's words is the whole job of it: the \
                 localization reader takes the tables it can parse, and whatever falls through to \
                 here is still text somebody has to be offered"
            );
        }

        assert!(
            !wiring(None, "m_MethodName"),
            "a class no assembly names could be anything at all"
        );
    }

    #[test]
    fn each_sheet_lands_under_the_class_its_object_belongs_to() {
        let container = fake::container(
            "sharedassets0.assets",
            vec![
                Object::forged(serial::MONO_BEHAVIOUR, 11, packed(&["Wait for me."])),
                Object::forged(
                    serial::MONO_BEHAVIOUR,
                    22,
                    packed(&["Morning.", "Hi there."]),
                ),
                Object::forged(serial::MONO_BEHAVIOUR, 33, vec![0; HEADER]),
            ],
            &[],
        );

        let found = taken("sharedassets0.assets", &container);

        assert_eq!(
            found.len(),
            1,
            "objects sharing one field pile into the same sheet"
        );
        assert_eq!(
            found[0].at,
            PathBuf::from("mono_behaviour/sharedassets0.assets/loose/0.sheet"),
            "an object whose class we cannot read is given no class folder: we do not invent a \
             name for it"
        );
        assert_eq!(
            found[0].lines, 3,
            "an object with no string in it gives nothing"
        );

        let spots: Vec<String> = sheet::lines(&found[0].body)
            .expect("a sheet we just wrote")
            .into_keys()
            .collect();
        assert!(
            spots
                .iter()
                .all(|spot| spot.starts_with("11/") || spot.starts_with("22/")),
            "every line has to name the object it came out of, or it cannot be put back: {spots:?}"
        );
        assert!(
            !spots.iter().any(|spot| spot.starts_with("33/")),
            "the object holding no string is nowhere in the sheet: {spots:?}"
        );
    }

    fn sheets_of(ids: &[i64], said: &str) -> Vec<Harvest> {
        let container = fake::container(
            "sharedassets0.assets",
            ids.iter()
                .map(|id| Object::forged(serial::MONO_BEHAVIOUR, *id, packed(&[said])))
                .collect(),
            &[],
        );

        taken("sharedassets0.assets", &container)
    }

    #[test]
    fn every_id_falls_in_one_range_however_wide_a_range_is() {
        for id in [
            i64::MIN,
            i64::MIN + 1,
            -129,
            -128,
            -70,
            -1,
            0,
            1,
            63,
            64,
            34347,
            i64::MAX,
        ] {
            let floor = bucket(id);

            assert!(
                floor <= id,
                "a range starts at or below the id it holds: {id}"
            );
            assert!(
                id.wrapping_sub(floor) < BUCKET,
                "no id sits further than one range past its start: {id}"
            );
            assert_eq!(
                bucket(floor),
                floor,
                "a range's own start has to fall inside it, or the ranges would not tile: {id}"
            );
        }
    }

    #[test]
    fn a_field_is_split_by_object_id_and_nothing_else() {
        let named = |said: &str| {
            let mut out: Vec<PathBuf> = sheets_of(&[1, 60, 70, 130, 200], said)
                .into_iter()
                .map(|one| one.at)
                .collect();
            out.sort();

            out
        };

        let under = PathBuf::from("mono_behaviour/sharedassets0.assets/loose");
        let mut want = vec![
            under.join("0.sheet"),
            under.join("64.sheet"),
            under.join("128.sheet"),
            under.join("192.sheet"),
        ];
        want.sort();

        assert_eq!(
            named("Hi."),
            want,
            "ids 1 and 60 fall in one range of 64, the other three each open their own"
        );

        assert_eq!(
            named("Hi."),
            named(&"y".repeat(4000)),
            "a line growing longer must never move an object into another sheet: the store keys \
             every translation by the sheet it came from, so a moved object is an orphaned \
             translation"
        );
    }

    #[test]
    fn no_object_is_dropped_or_written_twice_by_the_split() {
        let ids: Vec<i64> = (1..=200).collect();
        let found = sheets_of(&ids, "Wait for me.");

        assert!(found.len() > 1, "200 ids cannot share one range of 64");

        let mut whose: Vec<String> = Vec::new();
        for one in &found {
            for spot in sheet::lines(&one.body)
                .expect("a sheet we just wrote")
                .into_keys()
            {
                whose.push(
                    spot.split_once('/')
                        .expect("a line in a sheet names the object it came out of")
                        .0
                        .to_string(),
                );
            }
        }

        let mut once = whose.clone();
        once.sort();
        once.dedup();

        assert_eq!(whose.len(), 200, "no object may be dropped by the split");
        assert_eq!(once.len(), 200, "no object may be written into two sheets");
    }

    #[test]
    fn one_path_id_used_by_two_nodes_lands_in_two_sheets() {
        let node = |text: &str| {
            fake::container(
                "CAB-one",
                vec![Object::forged(serial::MONO_BEHAVIOUR, 7, packed(&[text]))],
                &[],
            )
        };

        let first = node("Morning.");
        let second = node("Evening.");
        let every = vec![&first, &second];
        let shared = shared_ids(&every);

        assert!(
            shared.contains(&7),
            "a path id is only unique inside one node of a bundle"
        );

        assert_eq!(
            taken_across("extra.bundle", &every, &shared),
            vec![
                PathBuf::from("mono_behaviour/extra.bundle/loose/0#0.sheet"),
                PathBuf::from("mono_behaviour/extra.bundle/loose/0#1.sheet"),
            ],
            "or one node's text would be written into the other node's object"
        );
    }

    #[test]
    fn two_nodes_with_different_ids_in_one_range_keep_their_own_sheets() {
        let node = |id: i64, text: &str| {
            fake::container(
                "CAB-one",
                vec![Object::forged(serial::MONO_BEHAVIOUR, id, packed(&[text]))],
                &[],
            )
        };

        let first = node(5, "Morning.");
        let second = node(50, "Evening.");
        let every = vec![&first, &second];
        let shared = shared_ids(&every);

        assert!(
            shared.is_empty(),
            "no id is used twice, so sharing cannot be what keeps the nodes apart"
        );

        assert_eq!(
            taken_across("extra.bundle", &every, &shared),
            vec![
                PathBuf::from("mono_behaviour/extra.bundle/loose/0#0.sheet"),
                PathBuf::from("mono_behaviour/extra.bundle/loose/0#1.sheet"),
            ],
            "ids 5 and 50 share the range of 64, so without the node in the name the second \
             node's sheet would overwrite the first's"
        );
    }

    fn talker() -> fake::Kind {
        fake::Kind::Struct(vec![
            ("m_GameObject", fake::Kind::Pointer),
            ("m_Enabled", fake::Kind::Number(4)),
            ("m_Script", fake::Kind::Pointer),
            ("m_Name", fake::Kind::Text),
            (
                "lines",
                fake::Kind::List(Box::new(fake::Kind::Struct(vec![
                    ("who", fake::Kind::Text),
                    ("said", fake::Kind::Text),
                ]))),
            ),
            (
                "references",
                fake::Kind::Struct(vec![
                    ("version", fake::Kind::Number(4)),
                    (
                        "RefIds",
                        fake::Kind::List(Box::new(fake::Kind::Struct(vec![
                            ("rid", fake::Kind::Number(8)),
                            ("type", fake::Kind::Struct(vec![("asm", fake::Kind::Text)])),
                        ]))),
                    ),
                ]),
            ),
        ])
    }

    fn talking(said: &[(&str, &str)]) -> fake::Val {
        fake::Val::Struct(vec![
            fake::Val::Pointer(0, 0),
            fake::Val::Number(1),
            fake::Val::Pointer(1, 5),
            fake::Val::Text("Talker".to_string()),
            fake::Val::List(
                said.iter()
                    .map(|(who, line)| {
                        fake::Val::Struct(vec![
                            fake::Val::Text((*who).to_string()),
                            fake::Val::Text((*line).to_string()),
                        ])
                    })
                    .collect(),
            ),
            fake::Val::Struct(vec![
                fake::Val::Number(2),
                fake::Val::List(vec![fake::Val::Struct(vec![
                    fake::Val::Number(7),
                    fake::Val::Struct(vec![fake::Val::Text("Assembly-CSharp".to_string())]),
                ])]),
            ]),
        ])
    }

    const SAID: [(&str, &str); 2] = [("Mary", "She tilted her head."), ("Peter", "Wait.")];

    fn a_talker_holding(body: Vec<u8>) -> Container {
        serial::open(
            &fake::forge_trees(&[(21, serial::MONO_BEHAVIOUR, talker(), body)]),
            "",
        )
        .expect("a forged container opens")
    }

    fn a_talker(said: &[(&str, &str)]) -> Container {
        a_talker_holding(fake::body_of(&talker(), &talking(said)))
    }

    fn read_by_tree(container: &Container) -> Vec<Piece> {
        pieces_in(
            container,
            &container.objects[0],
            Some("Talker"),
            &Assemblies::default(),
            &localization::Collections::default(),
        )
    }

    #[test]
    fn a_class_no_assembly_describes_is_still_read_field_by_field_from_its_type_tree() {
        let container = a_talker(&SAID);
        let pieces = read_by_tree(&container);

        assert!(
            pieces.iter().all(|one| one.at.is_none()),
            "a tree says what a field is called but never where it sits, so these can only be \
             written back by rebuilding the object"
        );
        assert_eq!(
            pieces
                .iter()
                .map(|one| (one.field.as_str(), one.text.as_str()))
                .collect::<Vec<_>>(),
            [
                ("m_Name", "Talker"),
                ("lines.who", "Mary"),
                ("lines.said", "She tilted her head."),
                ("lines.who", "Peter"),
                ("lines.said", "Wait."),
            ],
            "without this the whole object comes out as one nameless heap called loose"
        );
        assert!(
            !pieces
                .iter()
                .any(|one| one.text.contains("Assembly-CSharp")),
            "the reference registry is Unity's own bookkeeping; the field reader consumes it \
             without offering it, and reading by tree has to do the same"
        );
    }

    #[test]
    fn a_sheet_numbered_by_a_reader_that_is_no_longer_used_moves_nothing() {
        let container = a_talker(&SAID);
        let object = &container.objects[0];
        let pieces = read_by_tree(&container);

        let mut lines = BTreeMap::new();
        for which in 0..pieces.len() + 2 {
            lines.insert(which.to_string(), "\u{4f55}\u{304b}".to_string());
        }

        assert_eq!(
            put_back(object, &pieces, &lines).expect("a write"),
            object.body().expect("its body").into_owned(),
            "strands and a type tree once both numbered their pieces 0, 1, 2: a sheet taken by \
             one reader must never be written back by the other, or a line lands in a field \
             nobody translated"
        );
    }

    #[test]
    fn a_line_read_from_a_type_tree_is_written_back_by_rebuilding_the_object() {
        let container = a_talker(&SAID);
        let object = &container.objects[0];
        let pieces = read_by_tree(&container);

        let said = pieces
            .iter()
            .find(|one| one.text == "She tilted her head.")
            .expect("the line");

        let mut lines = BTreeMap::new();
        lines.insert(said.id.clone(), "\u{5f7c}\u{5973}\u{306f}\u{9996}\u{3092}\u{304b}\u{305f}\u{3080}\u{3051}\u{305f}\u{3002}".to_string());

        let body = put_back(object, &pieces, &lines).expect("a write");
        let fresh = a_talker_holding(body);

        assert_eq!(
            read_by_tree(&fresh)
                .iter()
                .map(|one| one.text.clone())
                .collect::<Vec<_>>(),
            [
                "Talker",
                "Mary",
                "\u{5f7c}\u{5973}\u{306f}\u{9996}\u{3092}\u{304b}\u{305f}\u{3080}\u{3051}\u{305f}\u{3002}",
                "Peter",
                "Wait."
            ],
            "the line that changed comes back changed and every line around it is untouched"
        );
    }
}
