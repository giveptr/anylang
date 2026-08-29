use crate::engine::layout::Boxed;
use crate::engine::rpg_maker::{Gathered, text};
use crate::engine::{Offer, TranslationUnit, hand_written};
use serde_json::Value;
use std::cmp::Reverse;
use std::collections::HashMap;
use std::ops::{Range, RangeInclusive};

const MESSAGE_LINE: i64 = 401;
const SCROLL_LINE: i64 = 405;
const MESSAGE_HEAD: i64 = 101;
const CHOICES: i64 = 102;
const CHOICE_BRANCH: i64 = 402;
const CHOICES_END: i64 = 404;
const NAME_CHANGE: [i64; 3] = [320, 324, 325];
const MOVE_ROUTE: RangeInclusive<i64> = 1..=46;

const TEXT_FIELDS: [&str; 11] = [
    "name",
    "nickname",
    "profile",
    "description",
    "displayName",
    "message1",
    "message2",
    "message3",
    "message4",
    "gameTitle",
    "currencyUnit",
];

const TEXT_LISTS: [&str; 11] = [
    "elements",
    "variables",
    "switches",
    "etypes",
    "skillTypes",
    "weaponTypes",
    "armorTypes",
    "equipTypes",
    "basic",
    "commands",
    "params",
];

pub const LOOKED_UP_BY_NAME: [&str; 2] = ["elements", "equipTypes"];

const NOT_SHOWN: [&str; 6] = [
    "volume",
    "x",
    "switchId",
    "frames",
    "tilesetNames",
    "parentId",
];

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Step {
    Key(String),
    Index(usize),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Spot(pub Vec<Step>);

impl Spot {
    pub fn key(&self, key: &str) -> Self {
        let mut steps = self.0.clone();
        steps.push(Step::Key(key.to_string()));
        Spot(steps)
    }

    pub fn index(&self, index: usize) -> Self {
        let mut steps = self.0.clone();
        steps.push(Step::Index(index));
        Spot(steps)
    }

    pub fn reach<'a>(&self, root: &'a mut Value) -> Option<&'a mut String> {
        let mut node = root;
        for step in &self.0 {
            node = match step {
                Step::Key(key) => node.as_object_mut()?.get_mut(key)?,
                Step::Index(index) => node.as_array_mut()?.get_mut(*index)?,
            };
        }

        match node {
            Value::String(text) => Some(text),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fix {
    Js,
    Token,
    Raw,
    Packed,
}

pub fn fits_raw(translation: &str) -> bool {
    !translation.contains(['<', '>'])
}

#[derive(Debug, Clone)]
pub enum Slot {
    Whole(Vec<Spot>),
    Lines(Vec<Spot>, String),
    Inside(Spot, Range<usize>, Fix),
}

pub type Splices = Vec<(Range<usize>, String)>;

pub struct Writes<'a> {
    pub whole: Vec<(&'a Spot, String)>,
    pub inside: Vec<(&'a Spot, Splices)>,
}

pub fn written<'a>(
    units: &[TranslationUnit],
    slots: &'a [Slot],
    mut answer: impl FnMut(&TranslationUnit, &Slot) -> Option<String>,
    mut fixed: impl FnMut(Fix, &str, &str) -> Option<String>,
) -> Writes<'a> {
    let mut whole = Vec::new();
    let mut inside: Vec<(&'a Spot, Splices)> = Vec::new();
    let mut homes: HashMap<&'a Spot, usize> = HashMap::new();

    for (unit, slot) in units.iter().zip(slots) {
        let Some(translation) = answer(unit, slot) else {
            continue;
        };

        match slot {
            Slot::Whole(spots) => {
                for spot in spots {
                    whole.push((spot, translation.clone()));
                }
            }
            Slot::Lines(spots, was) => {
                let lines = Boxed::read(&text::RE_MARK, was).laid_over(&translation, spots.len());

                for (spot, line) in spots.iter().zip(lines) {
                    whole.push((spot, line));
                }
            }
            Slot::Inside(spot, at, fix) => {
                let Some(text) = fixed(*fix, &translation, &unit.text) else {
                    continue;
                };

                let home = *homes.entry(spot).or_insert_with(|| {
                    inside.push((spot, Vec::new()));
                    inside.len() - 1
                });
                inside[home].1.push((at.clone(), text));
            }
        }
    }

    Writes { whole, inside }
}

pub fn splice(target: &mut String, mut edits: Splices) -> u32 {
    let mut wrote = 0;
    edits.sort_by_key(|(at, _)| Reverse(at.start));

    for (at, text) in edits {
        if target.is_char_boundary(at.start) && target.is_char_boundary(at.end) {
            target.replace_range(at, &text);
            wrote += 1;
        }
    }

    wrote
}

enum Part {
    Body(Range<usize>),
    Value(Range<usize>),
}

fn note_parts(note: &str) -> Vec<Part> {
    let mut found = Vec::new();
    let mut at = 0;

    while let Some(step) = note[at..].find('<') {
        let opened = at + step;
        let Some(step) = note[opened..].find('>') else {
            break;
        };
        let shut = opened + step;
        let inside = &note[opened + 1..shut];

        let (name, value) = match inside.find(':') {
            Some(mark) => (&inside[..mark], Some(opened + mark + 2..shut)),
            None => (inside, None),
        };

        if name.is_empty() || name.contains(['/', '<', '\n', '\r']) {
            at = opened + 1;
            continue;
        }

        if let Some(value) = value {
            found.push(Part::Value(value));
            at = shut + 1;
            continue;
        }

        let closing = format!("</{name}>");
        match note[shut + 1..].find(&closing) {
            Some(step) => {
                let body = shut + 1..shut + 1 + step;
                if text::has_words(&note[body.clone()]) {
                    found.push(Part::Body(body.clone()));
                }
                at = body.end + closing.len();
            }
            None => at = shut + 1,
        }
    }

    found
}

fn settings_only(body: &str) -> bool {
    let mut seen = false;

    for line in body.lines().map(str::trim).filter(|one| !one.is_empty()) {
        let Some((key, _)) = line.split_once(':') else {
            return false;
        };

        let key = key.trim();
        let keyed = key.starts_with(|one: char| one.is_ascii_alphabetic())
            && key
                .chars()
                .all(|one| one.is_ascii_alphanumeric() || one == ' ' || one == '_');

        if !keyed {
            return false;
        }

        seen = true;
    }

    seen
}

pub struct Found {
    pub text: String,
    pub slot: Slot,
    pub listed: bool,
}

impl Found {
    pub fn plain(text: String, slot: Slot) -> Self {
        Self {
            text,
            slot,
            listed: false,
        }
    }

    pub fn doubted(text: String, slot: Slot) -> Self {
        Self {
            text,
            slot,
            listed: true,
        }
    }
}

pub trait Dialect {
    fn extra(&self, list: &[Value], index: usize, at: &Spot, found: &mut Vec<Found>) -> usize;

    fn doubtful(&self, _text: &str) -> bool {
        false
    }

    fn registers(&self, _name: &str) -> bool {
        false
    }
}

pub fn run(root: &Value, dialect: &dyn Dialect) -> (Vec<TranslationUnit>, Vec<Slot>) {
    let mut harvest = Harvest {
        taken: Gathered::default(),
        dialect,
    };
    harvest.walk(root, &mut Vec::new());

    harvest.taken.done()
}

struct Harvest<'a> {
    taken: Gathered<Slot>,
    dialect: &'a dyn Dialect,
}

impl Harvest<'_> {
    fn push(&mut self, text: &str, slot: Slot) {
        self.put(text, slot, false);
    }

    fn looked_up(&self, value: &str) -> bool {
        value
            .split(['\n', ',', ':'])
            .any(|token| self.dialect.registers(token.trim()))
    }

    fn names_only(&self, body: &str) -> bool {
        let mut seen = false;

        for line in body.lines().map(str::trim).filter(|one| !one.is_empty()) {
            if !self.dialect.registers(line) {
                return false;
            }

            seen = true;
        }

        seen
    }

    fn put(&mut self, text: &str, slot: Slot, listed: bool) {
        if !text::has_words(text) {
            return;
        }

        let offer = Offer::default().or_listed(listed || self.dialect.doubtful(text));

        self.taken.take(text, slot, offer);
    }

    fn walk(&mut self, node: &Value, at: &mut Vec<Step>) {
        if let Some(list) = command_list(node) {
            self.commands(list, &Spot(at.clone()));
            return;
        }

        match node {
            Value::Object(fields) => {
                let shown = !fields.keys().any(|key| NOT_SHOWN.contains(&key.as_str()));

                for (key, child) in fields {
                    at.push(Step::Key(key.clone()));
                    self.field(shown, key, child, at);
                    at.pop();
                }
            }
            Value::Array(items) => {
                for (index, child) in items.iter().enumerate() {
                    at.push(Step::Index(index));
                    self.walk(child, at);
                    at.pop();
                }
            }
            _ => {}
        }
    }

    fn field(&mut self, shown: bool, key: &str, child: &Value, at: &mut Vec<Step>) {
        match child {
            Value::String(note) if key == "note" => {
                let here = Spot(at.clone());
                for part in note_parts(note) {
                    match part {
                        Part::Body(block) => {
                            let body = &note[block.clone()];
                            let keyed = settings_only(body) || self.names_only(body);
                            self.put(
                                body,
                                Slot::Inside(here.clone(), block.clone(), Fix::Raw),
                                keyed,
                            );
                        }
                        Part::Value(block) => {
                            let value = &note[block.clone()];
                            if hand_written(value) {
                                let keyed = self.looked_up(value);
                                self.put(value, Slot::Inside(here.clone(), block, Fix::Raw), keyed);
                            }
                        }
                    }
                }
            }
            Value::String(text) if shown && TEXT_FIELDS.contains(&key) => {
                self.push(text, Slot::Whole(vec![Spot(at.clone())]));
            }
            Value::Array(items) if TEXT_LISTS.contains(&key) => {
                let here = Spot(at.clone());
                let named = LOOKED_UP_BY_NAME.contains(&key);
                for (index, item) in items.iter().enumerate() {
                    match item.as_str() {
                        Some(text) => self.put(text, Slot::Whole(vec![here.index(index)]), named),
                        None => {
                            at.push(Step::Index(index));
                            self.walk(item, at);
                            at.pop();
                        }
                    }
                }
            }
            Value::Object(fields) if key == "messages" => {
                let here = Spot(at.clone());
                for (name, value) in fields {
                    if let Some(text) = value.as_str() {
                        self.push(text, Slot::Whole(vec![here.key(name)]));
                    }
                }
            }
            _ => self.walk(child, at),
        }
    }

    fn commands(&mut self, list: &[Value], at: &Spot) {
        let mut index = 0;

        while index < list.len() {
            let code = code_of(&list[index]);

            match code {
                MESSAGE_LINE | SCROLL_LINE => {
                    let (text, spots, next) = run_of(list, index, code, at);
                    let boxed = Boxed::read(&text::RE_MARK, &text);

                    if let Some(said) = boxed.asked() {
                        self.push(&said, Slot::Lines(spots, text));
                    }

                    index = next;
                    continue;
                }
                MESSAGE_HEAD => {
                    if let Some(name) = param(&list[index], 4).and_then(Value::as_str) {
                        let spot = at.index(index).key("parameters").index(4);
                        self.push(name, Slot::Whole(vec![spot]));
                    }
                }
                CHOICES => self.choices(list, index, at),
                code if NAME_CHANGE.contains(&code) => {
                    if let Some(text) = param(&list[index], 1).and_then(Value::as_str) {
                        let spot = at.index(index).key("parameters").index(1);
                        self.push(text, Slot::Whole(vec![spot]));
                    }
                }
                _ => {
                    let mut found = Vec::new();
                    let next = self.dialect.extra(list, index, at, &mut found);
                    for one in found {
                        self.put(&one.text, one.slot, one.listed);
                    }
                    index = next;
                    continue;
                }
            }

            index += 1;
        }
    }

    fn choices(&mut self, list: &[Value], index: usize, at: &Spot) {
        let indent = list[index].get("indent").and_then(Value::as_i64);
        let Some(choices) = param(&list[index], 0).and_then(Value::as_array) else {
            return;
        };

        for (choice, value) in choices.iter().enumerate() {
            let Some(text) = value.as_str() else { continue };

            let mut spots = vec![at.index(index).key("parameters").index(0).index(choice)];
            spots.extend(branches(list, index, indent, choice, at));

            self.push(text, Slot::Whole(spots));
        }
    }
}

fn branches(
    list: &[Value],
    from: usize,
    indent: Option<i64>,
    choice: usize,
    at: &Spot,
) -> Vec<Spot> {
    let mut found = Vec::new();

    for (index, command) in list.iter().enumerate().skip(from + 1) {
        let same_level = command.get("indent").and_then(Value::as_i64) == indent;
        let code = code_of(command);

        if same_level && code == CHOICES_END {
            break;
        }
        if !same_level || code != CHOICE_BRANCH {
            continue;
        }
        if param(command, 0).and_then(Value::as_u64) != Some(choice as u64) {
            continue;
        }
        if param(command, 1).and_then(Value::as_str).is_some() {
            found.push(at.index(index).key("parameters").index(1));
        }
    }

    found
}

fn run_of(list: &[Value], from: usize, code: i64, at: &Spot) -> (String, Vec<Spot>, usize) {
    let mut lines = Vec::new();
    let mut spots = Vec::new();
    let mut index = from;

    while index < list.len() && code_of(&list[index]) == code {
        if let Some(text) = param(&list[index], 0).and_then(Value::as_str) {
            lines.push(text.to_string());
            spots.push(at.index(index).key("parameters").index(0));
        }
        index += 1;
    }

    (lines.join("\n"), spots, index)
}

pub fn code_of(command: &Value) -> i64 {
    command.get("code").and_then(Value::as_i64).unwrap_or(0)
}

pub fn param(command: &Value, index: usize) -> Option<&Value> {
    command
        .get("parameters")
        .and_then(Value::as_array)
        .and_then(|params| params.get(index))
}

pub fn command_list(node: &Value) -> Option<&Vec<Value>> {
    let list = node.as_array()?;
    let first = list.first()?.as_object()?;
    let code = first.get("code")?.as_i64()?;

    let event_command =
        !MOVE_ROUTE.contains(&code) && first.get("parameters").is_some_and(Value::is_array);

    event_command.then_some(list)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parted(note: &str) -> Vec<(&'static str, &str)> {
        note_parts(note)
            .into_iter()
            .map(|part| match part {
                Part::Body(at) => ("body", &note[at]),
                Part::Value(at) => ("value", &note[at]),
            })
            .collect()
    }

    #[test]
    fn a_note_gives_up_its_words_by_byte_and_never_cuts_a_character_in_half() {
        let note = "<\u{8aac}\u{660e}:\u{3053}\u{308c}\u{306f}\u{5263}>\n\
                    <desc>\u{62bc}\u{3057}\u{5165}\u{308c}\u{306e}\u{4e00}\u{624b}</desc>\n\
                    <hide>";

        assert_eq!(
            parted(note),
            [
                ("value", "\u{3053}\u{308c}\u{306f}\u{5263}"),
                (
                    "body",
                    "\u{62bc}\u{3057}\u{5165}\u{308c}\u{306e}\u{4e00}\u{624b}"
                ),
            ],
            "a tag with a colon gives up what follows it, a tag with a closing partner gives up \
             what it wraps, and a tag with neither gives up nothing"
        );

        for part in note_parts(note) {
            let at = match part {
                Part::Body(at) | Part::Value(at) => at,
            };

            assert!(
                note.is_char_boundary(at.start) && note.is_char_boundary(at.end),
                "these ranges are handed to replace_range on the same string, and one landing \
                 inside a multi-byte character would take the whole export down"
            );
        }
    }

    #[test]
    fn the_parts_of_a_note_never_overlap() {
        let note = "<a:\u{4e00}><b:\u{4e8c}><c>\u{4e09}</c><d:\u{56db}>";
        let mut ends = 0;

        for part in note_parts(note) {
            let at = match part {
                Part::Body(at) | Part::Value(at) => at,
            };

            assert!(
                at.start >= ends,
                "a translation is written back range by range, so two parts sharing a byte would \
                 leave the second one pointing into a string the first already moved"
            );
            ends = at.end;
        }
    }
}
