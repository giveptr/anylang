use crate::engine::sheet;
use crate::engine::unity::dotnet::Assemblies;
use crate::engine::unity::layout::Read;
use crate::engine::unity::serial::{Container, Object, Value};
use crate::engine::unity::{Harvest, blob, layout, mono_script, naming, serial};
use anyhow::Result;
use std::collections::BTreeMap;
use std::mem;
use std::path::PathBuf;

pub const NAME: &str = "localization";
pub const KIND: &str = "StringTable";

const TABLE: &str = "UnityEngine.Localization.Tables.StringTable";
const SHARED: &str = "UnityEngine.Localization.Tables.SharedTableData";

const LOCALE_ID: &str = "m_LocaleId";
const CODE: &str = "m_Code";
const TITLE: &str = "m_Name";
const COLLECTION: &str = "m_TableCollectionName";
const POINTS_AT: &str = "m_SharedData";
const ENTRIES: &str = "m_TableData";
const SAID: &str = "m_Localized";
const KEYED: &str = "m_Id";

const NAME_FIELD: usize = 3;

fn under<'p>(path: &'p str, field: &str) -> Option<&'p str> {
    path.strip_suffix(field)?.strip_suffix('.')
}

type Whose = (String, i64);

struct Guess {
    at: Whose,
    script: Option<Whose>,
    named: String,
}

#[derive(Default)]
pub struct Collections {
    collections: BTreeMap<Whose, String>,
    guessed: Vec<Guess>,
}

impl Collections {
    pub fn learn(&mut self, container: &Container, known: &Assemblies) {
        for object in &container.objects {
            if object.class_id != serial::MONO_BEHAVIOUR {
                continue;
            }

            let at = (mono_script::file_key(&container.name), object.path_id);

            if let Some(named) = named_by_layout(object, known).or_else(|| named_by_tree(object)) {
                self.guessed.push(Guess {
                    at,
                    script: mono_script::points_to(container, object),
                    named,
                });
            }
        }
    }

    pub fn confirm(&mut self, names: &mono_script::Names) {
        for one in mem::take(&mut self.guessed) {
            let told = one.script.as_ref().and_then(|script| names.told(script));

            if told.is_none_or(|class| class == SHARED) {
                self.collections.insert(one.at, one.named);
            }
        }
    }

    pub fn shares(&self, container: &Container, object: &Object) -> bool {
        self.collections
            .contains_key(&(mono_script::file_key(&container.name), object.path_id))
    }

    fn of(&self, container: &Container, file: i32, path_id: i64) -> Option<&str> {
        let owner = mono_script::owner_of(container, file)?;

        self.collections.get(&(owner, path_id)).map(String::as_str)
    }

    fn by_read(&self, container: &Container, read: &Read) -> Option<&str> {
        let (_, file, path_id) = read
            .pointers
            .iter()
            .find(|(path, _, _)| path == POINTS_AT)?;

        self.of(container, *file, *path_id)
    }

    fn by_tree(&self, container: &Container, value: &Value) -> Option<&str> {
        let points = value.field(POINTS_AT)?;
        let file = i32::try_from(points.field("m_FileID")?.number()?).ok()?;

        self.of(container, file, points.field("m_PathID")?.number()?)
    }
}

fn named_by_layout(object: &Object, known: &Assemblies) -> Option<String> {
    known.named(SHARED)?;

    said(
        &layout::read(known, SHARED, &object.body().ok()?).ok()?,
        COLLECTION,
    )
}

fn named_by_tree(object: &Object) -> Option<String> {
    if !object.has(COLLECTION) {
        return None;
    }

    object.value()?.field(COLLECTION)?.text()
}

fn said(read: &Read, path: &str) -> Option<String> {
    read.spots
        .iter()
        .find(|one| one.path == path)
        .map(|one| one.text.clone())
}

struct Line {
    id: String,
    at: usize,
    text: String,
}

enum Body {
    Spliced(Vec<Line>),
    Rebuilt(Value),
}

pub struct Table {
    collection: String,
    locale: String,
    named: String,
    body: Body,
}

impl Table {
    pub fn sheet(&self) -> PathBuf {
        sheet_path(self)
    }
}

impl Body {
    fn said(&self) -> Vec<(String, String)> {
        match self {
            Body::Spliced(lines) => lines
                .iter()
                .map(|one| (one.id.clone(), one.text.clone()))
                .collect(),
            Body::Rebuilt(value) => entries_in(value).collect(),
        }
    }
}

fn entries_in(value: &Value) -> impl Iterator<Item = (String, String)> {
    value
        .field(ENTRIES)
        .map(Value::items)
        .unwrap_or_default()
        .iter()
        .filter_map(|one| {
            let id = one.field(KEYED).and_then(Value::number)?;
            let said = one.field(SAID).and_then(Value::text)?;

            (!said.is_empty()).then(|| (id.to_string(), said))
        })
}

fn sheet_path(table: &Table) -> PathBuf {
    PathBuf::from(NAME)
        .join(&table.collection)
        .join(&table.locale)
        .join(format!("{}.{}", table.named, sheet::SUFFIX))
}

fn lines_in(read: &Read) -> Vec<Line> {
    let ids: BTreeMap<&str, i64> = read
        .numbers
        .iter()
        .filter_map(|(path, id)| Some((under(path, KEYED)?, *id)))
        .collect();

    let mut out = Vec::new();
    for spot in &read.spots {
        let Some(stem) = under(&spot.path, SAID) else {
            continue;
        };
        let Some(id) = ids.get(stem) else {
            continue;
        };

        out.push(Line {
            id: id.to_string(),
            at: spot.at,
            text: spot.text.clone(),
        });
    }

    out
}

pub fn table_of(
    container: &Container,
    object: &Object,
    kind: Option<&str>,
    known: &Assemblies,
    books: &Collections,
) -> Option<Table> {
    if object.class_id != serial::MONO_BEHAVIOUR {
        return None;
    }

    let kind = kind?;
    if !layout::descends(known, kind, TABLE) {
        return None;
    }

    by_fields(container, object, kind, known, books).or_else(|| by_tree(container, object, books))
}

fn by_fields(
    container: &Container,
    object: &Object,
    kind: &str,
    known: &Assemblies,
    books: &Collections,
) -> Option<Table> {
    let read = layout::read(known, kind, &object.body().ok()?).ok()?;

    let locale = naming::as_filename(&said(&read, &format!("{LOCALE_ID}.{CODE}"))?)?;
    let named = said(&read, TITLE)
        .and_then(|name| naming::as_filename(&name))
        .unwrap_or_else(|| object.path_id.to_string());
    let collection = books
        .by_read(container, &read)
        .and_then(naming::as_filename)
        .unwrap_or_else(|| named.clone());

    let lines: Vec<Line> = lines_in(&read)
        .into_iter()
        .filter(|one| !one.text.is_empty())
        .collect();

    (!lines.is_empty()).then_some(Table {
        collection,
        locale,
        named,
        body: Body::Spliced(lines),
    })
}

fn by_tree(container: &Container, object: &Object, books: &Collections) -> Option<Table> {
    let value = object.value()?;

    let locale = naming::as_filename(&value.field(LOCALE_ID)?.field(CODE)?.text()?)?;
    let named = value
        .field(TITLE)
        .or_else(|| value.nth(NAME_FIELD))
        .and_then(Value::text)
        .and_then(|name| naming::as_filename(&name))
        .unwrap_or_else(|| object.path_id.to_string());
    let collection = books
        .by_tree(container, &value)
        .and_then(naming::as_filename)
        .unwrap_or_else(|| named.clone());

    entries_in(&value).next()?;

    Some(Table {
        collection,
        locale,
        named,
        body: Body::Rebuilt(value),
    })
}

pub fn take<'k>(
    container: &Container,
    kinds: impl Fn(&Object) -> Option<&'k str>,
    known: &Assemblies,
    books: &Collections,
) -> Result<Vec<Harvest>> {
    let mut out = Vec::new();

    for object in &container.objects {
        let Some(table) = table_of(container, object, kinds(object), known, books) else {
            continue;
        };

        let lines = table.body.said();

        out.push(Harvest {
            at: sheet_path(&table),
            lines: lines.len() as u32,
            body: sheet::write(lines)?,
        });
    }

    Ok(out)
}

pub fn sheet_of(
    container: &Container,
    object: &Object,
    kind: Option<&str>,
    known: &Assemblies,
    books: &Collections,
) -> Option<PathBuf> {
    Some(table_of(container, object, kind, known, books)?.sheet())
}

pub fn put_back(
    object: &Object,
    table: Table,
    lines: &BTreeMap<String, String>,
) -> Result<Vec<u8>> {
    match table.body {
        Body::Spliced(spliced) => {
            let swaps: Vec<(usize, usize, String)> = spliced
                .into_iter()
                .filter_map(|one| {
                    let said = lines.get(&one.id)?;

                    (said != &one.text).then(|| (one.at, one.text.len(), said.clone()))
                })
                .collect();

            let body = object.body()?;
            if swaps.is_empty() {
                return Ok(body.into_owned());
            }

            blob::splice(&body, &swaps)
        }
        Body::Rebuilt(mut value) => {
            let mut moved = false;

            if let Some(items) = value.field_mut(ENTRIES).map(Value::items_mut) {
                for one in items {
                    let Some(id) = one.field(KEYED).and_then(Value::number) else {
                        continue;
                    };
                    let Some(said) = lines.get(&id.to_string()) else {
                        continue;
                    };
                    if one.field(SAID).and_then(Value::text).as_ref() == Some(said) {
                        continue;
                    }

                    moved |= one.put(SAID, said);
                }
            }

            match moved {
                true => object.written(&value),
                false => Ok(object.body()?.into_owned()),
            }
        }
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::engine::unity::dotnet::Shape;
    use crate::engine::unity::fake;

    const DETAILED: &str = "UnityEngine.Localization.Tables.DetailedLocalizationTable`1";
    const ASSETS: &str = "UnityEngine.Localization.Tables.AssetTable";
    const HEAD: usize = 28;

    fn text(body: &mut Vec<u8>, said: &str) {
        body.extend_from_slice(&(said.len() as u32).to_le_bytes());
        body.extend_from_slice(said.as_bytes());
        while !body.len().is_multiple_of(4) {
            body.push(0);
        }
    }

    fn known() -> Assemblies {
        Assemblies::forged(vec![
            (
                "UnityEngine.Object",
                fake::class("System.Object", Vec::new()),
            ),
            (
                "UnityEngine.ScriptableObject",
                fake::class("UnityEngine.Object", Vec::new()),
            ),
            (
                DETAILED,
                fake::class(
                    "UnityEngine.ScriptableObject",
                    vec![
                        ("m_LocaleId", Shape::Named("Locale".to_string())),
                        ("m_SharedData", Shape::Named(SHARED.to_string())),
                        (
                            "m_TableData",
                            Shape::List(Box::new(Shape::Named("Entry".to_string()))),
                        ),
                    ],
                ),
            ),
            (TABLE, fake::class(DETAILED, Vec::new())),
            (ASSETS, fake::class(DETAILED, Vec::new())),
            ("Table", fake::class(TABLE, Vec::new())),
            (
                SHARED,
                fake::class(
                    "UnityEngine.ScriptableObject",
                    vec![(COLLECTION, Shape::Text)],
                ),
            ),
            (
                "Locale",
                fake::class("System.ValueType", vec![("m_Code", Shape::Text)]),
            ),
            (
                "Entry",
                fake::class(
                    "System.Object",
                    vec![("m_Id", Shape::Long), ("m_Localized", Shape::Text)],
                ),
            ),
        ])
    }

    fn a_table(file: i32, shared: i64) -> Object {
        let mut body = vec![0u8; HEAD];
        text(&mut body, "UI_GENERAL_en");
        text(&mut body, "en");
        body.extend_from_slice(&file.to_le_bytes());
        body.extend_from_slice(&shared.to_le_bytes());
        body.extend_from_slice(&2u32.to_le_bytes());
        body.extend_from_slice(&7i64.to_le_bytes());
        text(&mut body, "Start");
        body.extend_from_slice(&9i64.to_le_bytes());
        text(&mut body, "Load");

        Object::forged(serial::MONO_BEHAVIOUR, 42, body)
    }

    fn a_shared(name: &str) -> Object {
        let mut body = vec![0u8; HEAD];
        text(&mut body, "UI_GENERAL Shared Data");
        text(&mut body, name);

        Object::forged(serial::MONO_BEHAVIOUR, 900, body)
    }

    #[test]
    fn a_table_is_filed_under_the_collection_the_game_declares() {
        let mut books = Collections::default();
        books.learn(
            &fake::container("shared.bundle", vec![a_shared("UI_GENERAL")], &[]),
            &known(),
        );
        books.confirm(&mono_script::Names::default());

        let node = fake::container("english.bundle", vec![a_table(1, 900)], &["shared.bundle"]);

        assert_eq!(
            sheet_of(&node, &node.objects[0], Some("Table"), &known(), &books),
            Some(PathBuf::from(
                "localization/UI_GENERAL/en/UI_GENERAL_en.sheet"
            )),
            "Unity holds a collection of tables, one per locale, and every step here is a name \
             the game declares: the collection from its shared data, the locale from its \
             identifier, the asset by its own name"
        );
    }

    #[test]
    fn a_table_whose_shared_data_never_shipped_falls_back_to_its_own_name() {
        let node = fake::container("english.bundle", vec![a_table(1, 900)], &["missing.bundle"]);

        assert_eq!(
            sheet_of(
                &node,
                &node.objects[0],
                Some("Table"),
                &known(),
                &Collections::default()
            ),
            Some(PathBuf::from(
                "localization/UI_GENERAL_en/en/UI_GENERAL_en.sheet"
            )),
            "one shape whatever can be read, so the reader never meets two layouts for one thing"
        );
    }

    #[test]
    fn a_table_of_assets_is_refused_even_though_its_bytes_read_the_same() {
        let node = fake::container("english.bundle", vec![a_table(0, 900)], &[]);
        let object = &node.objects[0];

        assert!(
            table_of(
                &node,
                object,
                Some(ASSETS),
                &known(),
                &Collections::default()
            )
            .is_none(),
            "a string table and an asset table share a base and both hold TableEntryData, so \
             only the class tells them apart: claiming an asset table would offer its guids as \
             lines to translate"
        );
        assert!(
            table_of(&node, object, None, &known(), &Collections::default()).is_none(),
            "with no class to read there is no way to know which of the two it is"
        );
        assert!(
            table_of(
                &node,
                object,
                Some("Table"),
                &known(),
                &Collections::default()
            )
            .is_some(),
            "a game may name its own table whatever it likes as long as it comes down from the \
             package's own StringTable"
        );
    }

    #[test]
    fn the_keys_a_collection_shares_are_never_offered_as_lines() {
        let shared = fake::container("shared.bundle", vec![a_shared("UI_GENERAL")], &[]);
        let mut books = Collections::default();
        books.learn(&shared, &known());
        books.confirm(&mono_script::Names::default());

        assert!(
            books.shares(&shared, &shared.objects[0]),
            "shared data holds every key of a collection, which is machinery and not talk: \
             saying so is what keeps it out of the harvest"
        );

        let table = fake::container("english.bundle", vec![a_table(0, 900)], &[]);
        assert!(!books.shares(&table, &table.objects[0]));
    }

    #[test]
    fn an_object_that_merely_reads_like_shared_data_is_not_shared_data() {
        let node = fake::container(
            "shared.bundle",
            vec![
                Object::forged(serial::MONO_SCRIPT, 66, fake::a_mono_script("Talker")),
                Object::forged(
                    serial::MONO_BEHAVIOUR,
                    900,
                    fake::a_mono_behaviour(66, &["UI_GENERAL"]),
                ),
            ],
            &[],
        );

        let mut names = mono_script::Names::default();
        names.learn(&node);

        let mut books = Collections::default();
        books.learn(&node, &known());
        books.confirm(&names);

        assert!(
            !books.shares(&node, &node.objects[1]),
            "a game object small enough to parse under the shared-data layout is still the \
             game's own: calling it shared silently drops its text from the harvest"
        );
    }

    #[test]
    fn a_line_written_back_by_bytes_still_reads_as_the_table_it_was() {
        let node = fake::container("english.bundle", vec![a_table(0, 900)], &[]);
        let object = &node.objects[0];

        let mut lines = BTreeMap::new();
        lines.insert(
            "7".to_string(),
            "\u{306f}\u{3058}\u{3081}\u{308b}".to_string(),
        );

        let table = table_of(
            &node,
            object,
            Some("Table"),
            &known(),
            &Collections::default(),
        )
        .expect("a table to write into");
        let body = put_back(object, table, &lines).expect("a patched body");

        let again = fake::container(
            "english.bundle",
            vec![Object::forged(serial::MONO_BEHAVIOUR, 42, body)],
            &[],
        );
        let table = table_of(
            &again,
            &again.objects[0],
            Some("Table"),
            &known(),
            &Collections::default(),
        )
        .expect("it reads again");

        assert_eq!(
            table.body.said(),
            [
                (
                    "7".to_string(),
                    "\u{306f}\u{3058}\u{3081}\u{308b}".to_string()
                ),
                ("9".to_string(), "Load".to_string())
            ],
            "a longer line moves every byte after it, so the object only reads again if the \
             splice put the whole table back together"
        );
    }

    fn table_shape() -> fake::Kind {
        fake::Kind::Struct(vec![
            ("m_GameObject", fake::Kind::Pointer),
            ("m_Enabled", fake::Kind::Number(4)),
            ("m_Script", fake::Kind::Pointer),
            ("m_Name", fake::Kind::Text),
            (
                "m_LocaleId",
                fake::Kind::Struct(vec![("m_Code", fake::Kind::Text)]),
            ),
            ("m_SharedData", fake::Kind::Pointer),
            (
                "m_TableData",
                fake::Kind::List(Box::new(fake::Kind::Struct(vec![
                    ("m_Id", fake::Kind::Number(8)),
                    ("m_Localized", fake::Kind::Text),
                ]))),
            ),
        ])
    }

    fn shared_shape() -> fake::Kind {
        fake::Kind::Struct(vec![
            ("m_GameObject", fake::Kind::Pointer),
            ("m_Enabled", fake::Kind::Number(4)),
            ("m_Script", fake::Kind::Pointer),
            ("m_Name", fake::Kind::Text),
            ("m_TableCollectionName", fake::Kind::Text),
        ])
    }

    fn a_tree_table(named: &str, locale: &str, shared: i64, said: &[(i64, &str)]) -> fake::Val {
        fake::Val::Struct(vec![
            fake::Val::Pointer(0, 0),
            fake::Val::Number(1),
            fake::Val::Pointer(1, 55),
            fake::Val::Text(named.to_string()),
            fake::Val::Struct(vec![fake::Val::Text(locale.to_string())]),
            fake::Val::Pointer(0, shared),
            fake::Val::List(
                said.iter()
                    .map(|(id, text)| {
                        fake::Val::Struct(vec![
                            fake::Val::Number(*id),
                            fake::Val::Text((*text).to_string()),
                        ])
                    })
                    .collect(),
            ),
        ])
    }

    fn a_tree_shared(named: &str) -> fake::Val {
        fake::Val::Struct(vec![
            fake::Val::Pointer(0, 0),
            fake::Val::Number(1),
            fake::Val::Pointer(0, 66),
            fake::Val::Text("shared".to_string()),
            fake::Val::Text(named.to_string()),
        ])
    }

    fn script_shape() -> fake::Kind {
        fake::Kind::Struct(vec![
            ("m_Name", fake::Kind::Text),
            ("m_ClassName", fake::Kind::Text),
            ("m_Namespace", fake::Kind::Text),
        ])
    }

    fn a_tree_script(class: &str) -> fake::Val {
        let (space, name) = class.rsplit_once('.').unwrap_or(("", class));

        fake::Val::Struct(vec![
            fake::Val::Text(name.to_string()),
            fake::Val::Text(name.to_string()),
            fake::Val::Text(space.to_string()),
        ])
    }

    fn a_tree_container(said: &[(i64, &str)], class: &str) -> Container {
        let blob = fake::forge_trees(&[
            (
                66,
                serial::MONO_SCRIPT,
                script_shape(),
                fake::body_of(&script_shape(), &a_tree_script(class)),
            ),
            (
                77,
                serial::MONO_BEHAVIOUR,
                shared_shape(),
                fake::body_of(&shared_shape(), &a_tree_shared("Dialogue")),
            ),
            (
                11,
                serial::MONO_BEHAVIOUR,
                table_shape(),
                fake::body_of(&table_shape(), &a_tree_table("Dialogue_ja", "ja", 77, said)),
            ),
        ]);

        serial::open(&blob, "dialogue.bundle").expect("a forged container opens")
    }

    fn tree_books(node: &Container, known: &Assemblies) -> Collections {
        let mut names = mono_script::Names::default();
        names.learn(node);

        let mut books = Collections::default();
        books.learn(node, known);
        books.confirm(&names);

        books
    }

    #[test]
    fn a_table_a_game_ships_without_its_assemblies_is_still_read_by_its_own_type_tree() {
        let node = a_tree_container(&[(7, "Hello"), (9, "Wait.")], SHARED);
        let known = Assemblies::default();
        let books = tree_books(&node, &known);

        let out = take(&node, |_| Some(TABLE), &known, &books).expect("a harvest");

        assert_eq!(
            out.len(),
            1,
            "the shared data holds keys, not text, so only the table is offered"
        );
        assert_eq!(
            out[0].at,
            PathBuf::from("localization/Dialogue/ja/Dialogue_ja.sheet"),
            "an IL2CPP build ships no assemblies, so the collection and the locale can only come \
             from the type tree the container carries"
        );
        assert!(out[0].body.contains("Hello") && out[0].body.contains("Wait."));
    }

    #[test]
    fn a_class_of_the_games_own_is_not_shared_data_for_naming_one_field_alike() {
        let node = a_tree_container(&[(7, "Hello")], "Talker");
        let known = Assemblies::default();
        let books = tree_books(&node, &known);

        let object = node
            .objects
            .iter()
            .find(|one| one.path_id == 77)
            .expect("the object");

        assert!(
            !books.shares(&node, object),
            "a field name is not a class: saying it is would drop every line of the object from \
             the harvest, and nothing in the app could bring it back"
        );

        let out = take(&node, |_| Some(TABLE), &known, &books).expect("a harvest");
        assert_eq!(
            out[0].at,
            PathBuf::from("localization/Dialogue_ja/ja/Dialogue_ja.sheet"),
            "with no shared data to read the collection from, a table falls back to its own name"
        );
    }

    #[test]
    fn a_table_read_by_its_type_tree_is_written_back_through_it() {
        let node = a_tree_container(&[(7, "Hello"), (9, "Wait.")], SHARED);
        let known = Assemblies::default();
        let books = tree_books(&node, &known);

        let table = node
            .objects
            .iter()
            .find(|one| one.path_id == 11)
            .expect("the table");

        let mut lines = BTreeMap::new();
        lines.insert(
            "7".to_string(),
            "\u{3053}\u{3093}\u{306b}\u{3061}\u{306f}".to_string(),
        );

        let held = table_of(&node, table, Some(TABLE), &known, &books).expect("a table");
        let body = put_back(table, held, &lines).expect("a write");
        assert!(
            body.len() > table.body().expect("its body").len(),
            "a longer line grows the object"
        );

        let fresh = serial::open(
            &fake::forge_trees(&[(11, serial::MONO_BEHAVIOUR, table_shape(), body)]),
            "",
        )
        .expect("the rewritten object opens");

        let books = tree_books(&fresh, &known);
        let out = take(&fresh, |_| Some(TABLE), &known, &books).expect("a harvest");

        assert!(
            out[0]
                .body
                .contains("\u{3053}\u{3093}\u{306b}\u{3061}\u{306f}")
                && out[0].body.contains("Wait."),
            "the line that changed has to come back changed and the one beside it untouched"
        );
    }
}
