use crate::engine::rpg_maker::harvest::{self, Dialect, Fix, Found, Slot, Spot, code_of, param};
use crate::engine::rpg_maker::js::vocabulary::{Holds, Vocabulary};
use crate::engine::rpg_maker::js::{DATA, script};
use crate::engine::rpg_maker::text;
use crate::engine::{Applied, Parsed, TranslationUnit, symbolic};
use anyhow::Result;
use serde_json::Value;
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::ops::Range;
use std::path::Path;

const VARIABLES: i64 = 122;
const SCRIPT_OPERAND: i64 = 4;
const PLUGIN_CALL: i64 = 356;
const PLUGIN_COMMAND: i64 = 357;
const SCRIPT_HEAD: i64 = 355;
const SCRIPT_LINE: i64 = 655;

pub fn translatable(at: &Path) -> bool {
    const NO_TEXT: [&str; 3] = ["Animations.json", "Tilesets.json", "MapInfos.json"];

    at.parent().is_some_and(|up| up.ends_with(DATA))
        && at
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|name| name.ends_with(".json") && !NO_TEXT.contains(&name))
}

pub struct DataFile {
    root: Result<Value, String>,
    units: Vec<TranslationUnit>,
    slots: Vec<Slot>,
}

pub fn parse(body: &str, words: &Vocabulary) -> DataFile {
    let bare = body.strip_prefix('\u{feff}').unwrap_or(body);
    let Ok(root) = serde_json::from_str::<Value>(bare) else {
        return DataFile {
            root: Err(body.to_string()),
            units: Vec::new(),
            slots: Vec::new(),
        };
    };

    let (units, slots) = harvest::run(&root, &Js(words));

    DataFile {
        root: Ok(root),
        units,
        slots,
    }
}

impl Parsed for DataFile {
    fn units(&self) -> &[TranslationUnit] {
        &self.units
    }

    fn render(self: Box<Self>, translations: &BTreeMap<u32, String>) -> Result<(String, Applied)> {
        let this = *self;
        let mut applied = Applied::default();

        let mut root = match this.root {
            Ok(root) => root,
            Err(opened) => return Ok((opened, applied)),
        };

        let held = harvest::written(
            &this.units,
            &this.slots,
            |unit, _| unit.answer(translations).cloned(),
            fixed,
        );

        for (spot, line) in held.whole {
            if let Some(target) = spot.reach(&mut root) {
                *target = line;
                applied.lines += 1;
            }
        }

        for (spot, edits) in held.inside {
            if let Some(target) = spot.reach(&mut root) {
                applied.lines += harvest::splice(target, edits);
            }
        }

        Ok((serde_json::to_string(&root)?, applied))
    }
}

fn fixed(fix: Fix, translation: &str, source: &str) -> Option<String> {
    match fix {
        Fix::Js => Some(script::js_escape(translation)),
        Fix::Token => Some(one_token(translation, source)),
        Fix::Raw => harvest::fits_raw(translation).then(|| translation.to_string()),
        Fix::Packed => {
            Some(serde_json::to_string(translation).unwrap_or_else(|_| translation.to_string()))
        }
    }
}

struct Js<'a>(&'a Vocabulary);

impl Dialect for Js<'_> {
    fn doubtful(&self, text: &str) -> bool {
        text::listed_line(text)
    }

    fn registers(&self, name: &str) -> bool {
        self.0.registered(name)
    }

    fn extra(&self, list: &[Value], index: usize, at: &Spot, found: &mut Vec<Found>) -> usize {
        match code_of(&list[index]) {
            SCRIPT_HEAD => return self.script(list, index, at, found),
            VARIABLES => self.variable(&list[index], index, at, found),
            PLUGIN_CALL => self.call(&list[index], index, at, found),
            PLUGIN_COMMAND => self.plugin(&list[index], index, at, found),
            _ => {}
        }

        index + 1
    }
}

impl Js<'_> {
    fn script(&self, list: &[Value], from: usize, at: &Spot, found: &mut Vec<Found>) -> usize {
        let mut lines: Vec<(usize, String)> = Vec::new();
        let mut index = from;

        while index < list.len() && (index == from || code_of(&list[index]) == SCRIPT_LINE) {
            if let Some(line) = param(&list[index], 0).and_then(Value::as_str) {
                lines.push((index, line.to_string()));
            }
            index += 1;
        }

        let joined = lines
            .iter()
            .map(|(_, line)| line.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        for stored in script::stored_text(&joined) {
            let mut start = 0;
            for (which, line) in &lines {
                let past = start + line.len();
                if stored.at.start >= start && stored.at.end <= past {
                    let slot = Slot::Inside(
                        at.index(*which).key("parameters").index(0),
                        stored.at.start - start..stored.at.end - start,
                        Fix::Js,
                    );

                    found.push(match stored.beside {
                        true => Found::doubted(stored.text, slot),
                        false => Found::plain(stored.text, slot),
                    });
                    break;
                }
                start = past + 1;
            }
        }

        index
    }

    fn variable(&self, command: &Value, index: usize, at: &Spot, found: &mut Vec<Found>) {
        if param(command, 3).and_then(Value::as_i64) != Some(SCRIPT_OPERAND) {
            return;
        }

        let Some(source) = param(command, 4).and_then(Value::as_str) else {
            return;
        };

        let spot = at.index(index).key("parameters").index(4);
        for stored in script::literals(source) {
            found.push(Found::plain(
                stored.text,
                Slot::Inside(spot.clone(), stored.at, Fix::Js),
            ));
        }
    }

    fn call(&self, command: &Value, index: usize, at: &Spot, found: &mut Vec<Found>) {
        let Some(line) = param(command, 0).and_then(Value::as_str) else {
            return;
        };

        let Some(said) = argument(line, self.0) else {
            return;
        };

        let spot = at.index(index).key("parameters").index(0);
        found.push(Found::plain(
            line[said.clone()].to_string(),
            Slot::Inside(spot, said, Fix::Token),
        ));
    }

    fn plugin(&self, command: &Value, index: usize, at: &Spot, found: &mut Vec<Found>) {
        let name = param(command, 0)
            .and_then(Value::as_str)
            .unwrap_or_default();
        let called = param(command, 1)
            .and_then(Value::as_str)
            .unwrap_or_default();

        let wanted = self.0.args_of(name, called);
        if wanted.is_empty() {
            return;
        }

        let Some(args) = param(command, 3).and_then(Value::as_object) else {
            return;
        };

        let at = at.index(index).key("parameters").index(3);
        for (arg, shape) in wanted {
            let Some(said) = args.get(arg).and_then(Value::as_str) else {
                continue;
            };

            match shape {
                Holds::Note => {
                    if let Some(inner) = Holds::unpacked(said) {
                        found.push(Found::plain(
                            inner,
                            Slot::Inside(at.key(arg), 0..said.len(), Fix::Packed),
                        ));
                    }
                }
                _ => found.push(Found::plain(
                    said.to_string(),
                    Slot::Whole(vec![at.key(arg)]),
                )),
            }
        }
    }
}

fn tokens_of(line: &str) -> Vec<Range<usize>> {
    let mut found = Vec::new();
    let mut start = 0;

    for (at, _) in line.match_indices(' ') {
        found.push(start..at);
        start = at + 1;
    }
    found.push(start..line.len());

    found
}

fn setting(token: &str) -> bool {
    token.is_empty() || token.parse::<f64>().is_ok_and(f64::is_finite)
}

fn argument(line: &str, words: &Vocabulary) -> Option<Range<usize>> {
    let tokens = tokens_of(line);
    let args = tokens.get(1..)?;

    let mut first = 0;
    while first < args.len() {
        let token = &line[args[first].clone()];
        if !setting(token) && !words.identifier(token) {
            break;
        }
        first += 1;
    }

    let mut last = args.len();
    while last > first && setting(&line[args[last - 1].clone()]) {
        last -= 1;
    }

    if last <= first {
        return None;
    }

    let names_something = |at: &Range<usize>| {
        let token = &line[at.clone()];
        setting(token) || words.identifier(token) || symbolic(token)
    };

    if args[first..last].iter().all(names_something) {
        return None;
    }

    Some(args[first].start..args[last - 1].end)
}

fn one_token(translation: &str, source: &str) -> String {
    if source.contains(' ') {
        return translation.to_string();
    }

    translation.replace(' ', "\u{a0}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::rpg_maker::js::fixture::{put, sandbox};
    use serde_json::json;

    const TELLER: &str = "/*:
 * @command tell
 * @arg body
 * @type note
 *
 * @arg aside
 * @type multiline_string
 */";

    fn commanding(value: Value) -> DataFile {
        let at = sandbox();
        put(at.path(), "js/plugins/Teller.js", TELLER);
        parse(&value.to_string(), &Vocabulary::read(at.path()))
    }

    fn told(body: Value) -> Value {
        json!({ "events": [null, { "pages": [{ "list": [
            { "code": 357, "indent": 0, "parameters": ["Teller", "tell", "", body] },
        ] }] }] })
    }

    fn sheet(value: Value) -> DataFile {
        parse(&value.to_string(), &Vocabulary::default())
    }

    fn knowing(words: &[&str], value: Value) -> DataFile {
        parse(&value.to_string(), &Vocabulary::of(words))
    }

    fn registering(names: &[&str], value: Value) -> DataFile {
        parse(&value.to_string(), &Vocabulary::registering(names))
    }

    fn texts(sheet: &DataFile) -> Vec<&str> {
        sheet
            .units()
            .iter()
            .map(|unit| unit.text.as_str())
            .collect()
    }

    fn listed(sheet: &DataFile, text: &str) -> bool {
        !sheet
            .units()
            .iter()
            .find(|unit| unit.text == text)
            .unwrap_or_else(|| panic!("{text} was not picked up at all"))
            .offer
            .asked()
    }

    fn rendered(sheet: DataFile, pairs: &[(u32, &str)]) -> Value {
        let map: BTreeMap<u32, String> = pairs
            .iter()
            .map(|(id, text)| (*id, text.to_string()))
            .collect();

        serde_json::from_str(&Box::new(sheet).render(&map).expect("renders").0)
            .expect("render produces valid JSON")
    }

    fn message(lines: &[&str]) -> Value {
        let mut list = vec![json!({
            "code": 101, "indent": 0,
            "parameters": ["", 0, 0, 2, "Elena"],
        })];
        for line in lines {
            list.push(json!({ "code": 401, "indent": 0, "parameters": [line] }));
        }
        list.push(json!({ "code": 0, "indent": 0, "parameters": [] }));

        json!({ "events": [null, { "pages": [{ "list": list }] }] })
    }

    #[test]
    fn a_message_split_across_commands_is_one_unit_to_translate() {
        let sheet = sheet(message(&["The door is", "locked tight."]));

        assert_eq!(
            texts(&sheet),
            vec!["Elena", "The door is\nlocked tight."],
            "the model must see the whole sentence, not one row at a time"
        );
    }

    #[test]
    fn a_translated_message_goes_back_one_line_per_command() {
        let sheet = sheet(message(&["The door is", "locked tight."]));
        let out = rendered(sheet, &[(1, "扉は\nかたく閉ざされている。")]);

        let list = &out["events"][1]["pages"][0]["list"];
        assert_eq!(list[1]["parameters"][0], "扉は");
        assert_eq!(list[2]["parameters"][0], "かたく閉ざされている。");
    }

    #[test]
    fn a_choice_and_its_branch_label_are_kept_in_step() {
        let sheet = sheet(json!({ "events": [null, { "pages": [{ "list": [
                { "code": 102, "indent": 0, "parameters": [["Yes", "No"], 1, 0, 2, 0] },
                { "code": 402, "indent": 0, "parameters": [0, "Yes"] },
                { "code": 402, "indent": 0, "parameters": [1, "No"] },
                { "code": 404, "indent": 0, "parameters": [] },
            ] }] }] }));

        assert_eq!(texts(&sheet), vec!["Yes", "No"]);

        let out = rendered(sheet, &[(0, "はい"), (1, "いいえ")]);
        let list = &out["events"][1]["pages"][0]["list"];

        assert_eq!(list[0]["parameters"][0][0], "はい");
        assert_eq!(list[0]["parameters"][0][1], "いいえ");
        assert_eq!(
            list[1]["parameters"][1], "はい",
            "the branch label must follow the choice or the editor drifts"
        );
        assert_eq!(list[2]["parameters"][1], "いいえ");
    }

    #[test]
    fn plugin_notes_are_never_offered_for_translation() {
        let sheet = sheet(json!([null, {
            "id": 1,
            "name": "Iron Key",
            "description": "Opens the north gate.",
            "note": "<WordWrap>\n<Custom Plugin Param: 3>",
        }]));

        assert_eq!(texts(&sheet), vec!["Iron Key", "Opens the north gate."]);
        assert!(
            !texts(&sheet).iter().any(|text| text.contains("Plugin")),
            "the note box holds plugin configuration, translating it breaks the game"
        );
    }

    #[test]
    fn the_same_string_in_two_places_is_asked_about_once_for_each_place() {
        let sheet = sheet(json!([null,
            { "id": 1, "name": "Potion", "description": "Heals." },
            { "id": 2, "name": "Potion", "description": "Heals." },
        ]));

        assert_eq!(
            texts(&sheet),
            vec!["Potion", "Heals.", "Potion", "Heals."],
            "the same words in two places can read two ways, so each place is asked about on its \
             own rather than sharing one answer"
        );

        let out = rendered(sheet, &[(0, "ポーション")]);
        assert_eq!(out[1]["name"], "ポーション");
        assert_eq!(
            out[2]["name"], "Potion",
            "the second one holds its own line and waits for its own answer"
        );
    }

    #[test]
    fn nothing_outside_the_translated_strings_is_ever_touched() {
        let raw = json!({
            "autoplayBgm": false,
            "tilesetId": 3,
            "data": [1, 0, null, 2048, -1],
            "scrollSpeed": 0.5,
            "note": "<Custom Param: 7>",
            "events": [null, { "id": 1, "x": 12, "pages": [{ "list": [
                { "code": 401, "indent": 0, "parameters": ["Hello"] },
                { "code": 355, "indent": 0, "parameters": ["$gameVariables.setValue(3, 1)"] },
            ] }] }],
        })
        .to_string();

        let sheet = || parse(&raw, &Vocabulary::default());
        let (untouched, applied) = Box::new(sheet()).render(&BTreeMap::new()).expect("renders");

        assert_eq!(applied.lines, 0);
        assert_eq!(
            serde_json::from_str::<Value>(&untouched).unwrap(),
            serde_json::from_str::<Value>(&raw).unwrap(),
            "an export that translated nothing must change nothing"
        );

        let out = rendered(sheet(), &[(0, "こんにちは")]);
        let list = &out["events"][1]["pages"][0]["list"];

        assert_eq!(list[0]["parameters"][0], "こんにちは");
        assert_eq!(
            list[1]["parameters"][0], "$gameVariables.setValue(3, 1)",
            "script calls are game logic and must survive untouched"
        );
        assert_eq!(out["scrollSpeed"], 0.5, "numbers must not drift");
        assert_eq!(out["data"], json!([1, 0, null, 2048, -1]));
        assert_eq!(out["note"], "<Custom Param: 7>");
    }

    #[test]
    fn exporting_twice_gives_the_same_file_both_times() {
        let raw = json!([null, { "id": 1, "name": "Potion", "description": "Heals." }]).to_string();

        let once = Box::new(parse(&raw, &Vocabulary::default()))
            .render(&BTreeMap::from([(0, "ポーション".to_string())]))
            .expect("renders")
            .0;
        let twice = Box::new(parse(&once, &Vocabulary::default()))
            .render(&BTreeMap::new())
            .expect("renders")
            .0;

        assert_eq!(
            once, twice,
            "a second export must not keep rewriting the file"
        );
    }

    #[test]
    fn a_move_route_list_is_a_different_code_space_and_is_never_entered() {
        let route = json!([
            { "code": 41, "parameters": ["$TREASURE-1@(13)", 0] },
            { "code": 45, "parameters": ["this.requestBalloon(1)"] },
            { "code": 0 },
        ]);

        assert!(harvest::command_list(&route).is_none());
        assert!(harvest::command_list(&json!([{ "code": 0, "parameters": [] }])).is_some());
        assert!(
            harvest::command_list(&json!([{ "code": 22, "dataId": 3, "value": 1 }])).is_none(),
            "a trait carries a code too, and its numbers are not commands"
        );

        let sheet = sheet(json!({ "events": [null, { "pages": [{
                "moveRoute": { "list": route },
                "list": [{ "code": 401, "indent": 0, "parameters": ["Hello."] }],
            }] }] }));

        assert_eq!(texts(&sheet), vec!["Hello."]);
    }

    #[test]
    fn system_terms_and_type_names_are_picked_up() {
        let sheet = sheet(json!({
            "gameTitle": "Serpent",
            "currencyUnit": "G",
            "skillTypes": ["", "Magic"],
            "terms": {
                "basic": ["Level", "Lv"],
                "commands": ["Fight", null, "Attack"],
                "params": ["Max HP"],
                "messages": { "actionFailure": "There was no effect!" },
            },
        }));

        let found = texts(&sheet);
        for wanted in [
            "Serpent",
            "G",
            "Magic",
            "Level",
            "Fight",
            "Max HP",
            "There was no effect!",
        ] {
            assert!(found.contains(&wanted), "{wanted} was not picked up");
        }
    }

    #[test]
    fn a_variable_or_switch_name_is_offered_like_any_other_word() {
        let sheet = sheet(json!({
            "variables": ["", "撃破回数（X07)"],
            "switches": ["", "鹵獲フラグ"],
        }));

        for named in ["撃破回数（X07)", "鹵獲フラグ"] {
            assert!(
                texts(&sheet).contains(&named),
                "a records screen prints these, so leaving them out left japanese on the screen"
            );
            assert!(
                !listed(&sheet, named),
                "the engine reaches a variable by its number: only a plugin can look one up by \
                 name, and a game that never does is a game this cannot break"
            );
        }
    }

    #[test]
    fn an_element_name_the_notes_spell_out_is_listed_and_never_sent_to_the_model() {
        let sheet = sheet(json!({
            "elements": ["", "物理", "魔力"],
            "skillTypes": ["", "魔法"],
        }));

        assert!(
            listed(&sheet, "魔力"),
            "a note reading <elementDamageBonus:魔力:50> is looked up by name in this list, so \
             translating it leaves the plugin with a name it cannot find"
        );
        assert!(
            !listed(&sheet, "魔法"),
            "a skill type is read off the battle menu, not out of a note"
        );
    }

    #[test]
    fn a_word_that_is_both_an_element_and_a_term_is_held_back_only_where_it_is_looked_up() {
        let both = sheet(json!({
            "elements": ["", "魔力"],
            "terms": { "params": ["魔力"] },
        }));
        let reversed = sheet(json!({
            "terms": { "params": ["魔力"] },
            "elements": ["", "魔力"],
        }));

        for sheet in [&both, &reversed] {
            let held: Vec<bool> = sheet
                .units()
                .iter()
                .filter(|unit| unit.text == "魔力")
                .map(|unit| !unit.offer.asked())
                .collect();

            assert_eq!(held.len(), 2, "each list holds a line of its own");
            assert_eq!(
                held.iter().filter(|one| **one).count(),
                1,
                "a note reading <elementDamageBonus:魔力:50> looks the element up by name, so \
                 that one is held back, while the term is read off the battle menu and is not, \
                 and which list the harvest reaches first may not decide either"
            );
        }
    }

    #[test]
    fn a_data_file_that_cannot_be_read_is_handed_back_untouched() {
        let broken = "{\"name\": 薬草}";

        assert!(parse(broken, &Vocabulary::default()).units().is_empty());
        assert_eq!(
            Box::new(parse(broken, &Vocabulary::default()))
                .render(&BTreeMap::new())
                .expect("no change")
                .0,
            broken,
            "a file we could not read is a file we must not rewrite"
        );
    }

    #[test]
    fn a_file_saved_with_a_byte_order_mark_still_gives_up_its_text() {
        let raw = format!(
            "\u{feff}{}",
            json!([null, { "id": 1, "name": "Potion", "description": "Heals." }])
        );
        let sheet = parse(&raw, &Vocabulary::default());

        assert_eq!(texts(&sheet), vec!["Potion", "Heals."]);
    }

    #[test]
    fn every_data_file_is_read_but_the_ones_known_to_hold_no_text() {
        let under_data = |name: &str| Path::new("game").join("data").join(name);

        for name in [
            "System.json",
            "Items.json",
            "Map001.json",
            "Map042.json",
            "Scenario.json",
        ] {
            assert!(
                translatable(&under_data(name)),
                "{name} may hold text: a plugin names its own files"
            );
        }
        for name in ["MapInfos.json", "Animations.json", "Tilesets.json"] {
            assert!(
                !translatable(&under_data(name)),
                "{name} holds no player-facing text"
            );
        }
        assert!(
            !translatable(&under_data("Scenario.rar")),
            "not a data file at all"
        );
        assert!(
            !translatable(Path::new("game/js/Map001.json")),
            "only the data folder holds the database the engine reads"
        );
    }

    #[test]
    fn a_set_value_wrapped_across_script_lines_is_still_found() {
        let raw = json!({
            "events": [null, { "id": 1, "x": 0, "pages": [{ "list": [
                { "code": 355, "indent": 0, "parameters": ["$gameVariables.setValue(21,"] },
                { "code": 655, "indent": 0, "parameters": ["\"The mask glinted in the dark.\")"] },
                { "code": 655, "indent": 0, "parameters": ["$gameSwitches.setValue(3, true)"] },
            ] }] }],
        })
        .to_string();

        let sheet = parse(&raw, &Vocabulary::default());
        let said: Vec<&str> = sheet.units().iter().map(|one| one.text.as_str()).collect();

        assert_eq!(
            said,
            ["The mask glinted in the dark."],
            "the call opens on the 355 line and its string sits on the 655 line"
        );

        let mut done = BTreeMap::new();
        done.insert(0, "仮面が暗闇の中で光っていた。".to_string());
        let (written, applied) = Box::new(parse(&raw, &Vocabulary::default()))
            .render(&done)
            .expect("renders");

        assert_eq!(applied.lines, 1);
        let back: Value = serde_json::from_str(&written).unwrap();
        assert_eq!(
            back["events"][1]["pages"][0]["list"][1]["parameters"][0],
            "\"仮面が暗闇の中で光っていた。\")",
            "the translation lands on the continuation line that holds the string"
        );
    }

    #[test]
    fn a_row_of_neighbouring_cells_is_listed_while_a_sentence_beside_it_is_still_asked() {
        let scene = json!({ "events": [null, { "pages": [{ "list": [
            { "code": 355, "indent": 0, "parameters": ["$gameVariables.setValue(11,\"あ\");"] },
            { "code": 655, "indent": 0, "parameters": ["$gameVariables.setValue(12,\"い\");"] },
            { "code": 655, "indent": 0, "parameters": ["$gameVariables.setValue(13,\"う\");"] },
            { "code": 655, "indent": 0, "parameters": ["$gameVariables.setValue(40,\"The mask glinted in the dark.\");"] },
        ] }] }] });

        let held = sheet(scene);

        for cell in ["あ", "い", "う"] {
            assert!(
                listed(&held, cell),
                "a game that paints a line one glyph per variable fills a run of neighbouring \
                 slots, and {cell} on its own is a picture key rather than anything a translator \
                 can read or a model can answer"
            );
        }

        assert!(
            !listed(&held, "The mask glinted in the dark."),
            "the sentence sits alone at slot 40 with neither neighbour written, which is what \
             tells a stored line apart from a cell in a row"
        );
    }

    #[test]
    fn a_plugin_argument_holding_a_script_is_listed_rather_than_asked() {
        let at = sandbox();
        put(at.path(), "js/plugins/Teller.js", TELLER);

        let said = "AudioManager.playSe({name: 'heal', pan: 0});";
        let scene = json!({ "events": [null, { "pages": [{ "list": [
            { "code": 357, "indent": 0, "parameters": ["Teller", "tell", "", { "aside": said }] },
        ] }] }] });

        let held = parse(&scene.to_string(), &Vocabulary::read(at.path()));

        assert!(
            listed(&held, said),
            "multiline_string only says how tall the box in the editor is, so an argument \
             declared with it can hold the script the event runs; the plugin list has read the \
             shape of a line for this since it was written and the data files never asked"
        );
    }

    #[test]
    fn a_plugin_file_that_keeps_its_lists_under_its_own_keys_still_gives_them_up() {
        let scene = json!({
            "00intro_01": [
                { "code": 356, "indent": 0, "parameters": ["Tachie showLeft 99 0 0 100"] },
                { "code": 101, "indent": 0, "parameters": ["", 0, 0, 2] },
                { "code": 401, "indent": 0, "parameters": ["The lamp went out before he answered."] },
                { "code": 0, "indent": 0, "parameters": [] },
            ],
        });

        let sheet = knowing(&["showLeft"], scene.clone());

        assert_eq!(
            texts(&sheet),
            vec!["The lamp went out before he answered."],
            "a scenario plugin keys its command lists by scene name, not \"list\""
        );

        let out = rendered(sheet, &[(0, "彼が答える前にランプが消えた。")]);
        assert_eq!(
            out["00intro_01"][2]["parameters"][0],
            "彼が答える前にランプが消えた。"
        );
        assert_eq!(
            out["00intro_01"][0]["parameters"][0], "Tachie showLeft 99 0 0 100",
            "every argument of that call is a setting the plugin compares"
        );
    }

    #[test]
    fn a_record_is_recognised_by_its_fields_wherever_it_lives() {
        let sheet = sheet(json!({ "quests": [null, {
                "id": 1,
                "name": "The Missing Key",
                "description": "Find the key the smith lost.",
                "note": "<Category: side>",
                "iconIndex": 84,
            }] }));

        assert_eq!(
            texts(&sheet),
            vec!["The Missing Key", "Find the key the smith lost."],
            "a plugin invents its own file names, so the fields have to be what is recognised"
        );
    }

    #[test]
    fn the_name_a_map_shows_on_screen_is_offered_and_the_names_the_editor_uses_are_not() {
        let sheet = sheet(json!({
            "displayName": "Castle Throne Room",
            "bgm": { "name": "Theme6", "pan": 0, "pitch": 100, "volume": 90 },
            "parallaxName": "BlueSky",
            "note": "<Light: 3>",
            "events": [null, {
                "id": 1, "name": "EV001_door", "note": "", "x": 12, "y": 8,
                "pages": [{ "list": [
                    { "code": 401, "indent": 0, "parameters": ["It will not budge."] },
                ] }],
            }],
        }));

        assert_eq!(
            texts(&sheet),
            vec!["Castle Throne Room", "It will not budge."],
            "an audio file, a parallax and an event name are all read back by name at runtime"
        );
    }

    #[test]
    fn a_common_event_keeps_the_name_plugins_call_it_by_and_a_troop_keeps_the_one_it_shows() {
        let common = sheet(json!([null, {
            "id": 1, "name": "call_shop", "switchId": 0, "trigger": 0,
            "list": [{ "code": 401, "indent": 0, "parameters": ["Welcome."] }],
        }]));

        assert_eq!(
            texts(&common),
            vec!["Welcome."],
            "a common event is called by name by plugins, and the name is never drawn"
        );

        let troop = sheet(json!([null, {
            "id": 1, "name": "Slime*2", "members": [{ "enemyId": 1, "x": 0, "y": 0 }],
            "pages": [{ "list": [{ "code": 0, "indent": 0, "parameters": [] }] }],
        }]));

        assert_eq!(texts(&troop), vec!["Slime*2"]);
    }

    #[test]
    fn a_free_text_plugin_call_gives_up_its_words_and_keeps_its_settings() {
        let sheet = knowing(
            &["showName", "showLeft", "00intro_01"],
            json!({ "one": [
                { "code": 356, "indent": 0, "parameters": ["Tachie showName Old\u{2008}Storyteller"] },
                { "code": 356, "indent": 0, "parameters": ["D_TEXT \\C[10]The Hall of Echoes 24"] },
                { "code": 356, "indent": 0, "parameters": ["Scenario 00intro_01"] },
                { "code": 356, "indent": 0, "parameters": ["Tachie showLeft 99 0 0 100"] },
            ] }),
        );

        assert_eq!(
            texts(&sheet),
            vec!["Old\u{2008}Storyteller", "\\C[10]The Hall of Echoes"],
            "the scenario id is a key of a data file and the font size is a number"
        );

        let out = rendered(sheet, &[(0, "語り部の老人"), (1, "\\C[10]響きの広間")]);
        let list = &out["one"];

        assert_eq!(
            list[0]["parameters"][0], "Tachie showName 語り部の老人",
            "the engine splits a plugin call on spaces, so a name that was one argument stays one"
        );
        assert_eq!(
            list[1]["parameters"][0], "D_TEXT \\C[10]響きの広間 24",
            "this call was already several arguments, and the size has to stay last"
        );
        assert_eq!(list[2]["parameters"][0], "Scenario 00intro_01");
        assert_eq!(list[3]["parameters"][0], "Tachie showLeft 99 0 0 100");
    }

    #[test]
    fn a_call_whose_every_argument_names_something_offers_nothing() {
        let sheet = knowing(
            &["set", "edit", "play", "sironuki", "def"],
            json!({ "one": [
                { "code": 356, "indent": 0, "parameters": ["particle set sironuki_12 event:40 sironuki def 8"] },
                { "code": 356, "indent": 0, "parameters": ["particle edit event:12 def"] },
                { "code": 356, "indent": 0, "parameters": ["particle play rial_3"] },
            ] }),
        );

        assert!(
            texts(&sheet).is_empty(),
            "a call is settings when not one of its arguments could be read by a player"
        );
    }

    #[test]
    fn a_number_between_two_names_is_read_as_a_setting_and_not_as_a_word() {
        let held = sheet(json!({ "one": [
            { "code": 356, "indent": 0, "parameters": ["D_TEXT showLeft 55 showRight"] },
        ] }));

        assert!(
            texts(&held).is_empty(),
            "the trims only reach the ends of a call, so a number of more than one digit \
             sitting between two names is the one place the whole call turns on reading it as a \
             setting: one digit alone would be listed and prove nothing"
        );
    }

    #[test]
    fn a_sentence_that_opens_on_a_label_keeps_the_label() {
        let sheet = knowing(
            &["D_TEXT"],
            json!({ "one": [
                { "code": 356, "indent": 0, "parameters": ["D_TEXT Reward: 100 spirit stones 24"] },
            ] }),
        );

        assert_eq!(
            texts(&sheet),
            vec!["Reward: 100 spirit stones"],
            "one argument reading like a setting may not cut the sentence it opens"
        );
    }

    #[test]
    fn a_sentence_stored_by_control_variables_is_found_too() {
        let raw = json!({ "events": [null, { "pages": [{ "list": [
            { "code": 122, "indent": 0, "parameters": [7, 7, 0, 4, "'Day 1'"] },
            { "code": 122, "indent": 0, "parameters": [8, 8, 0, 4, "$gameParty.gold()"] },
            { "code": 122, "indent": 0, "parameters": [9, 9, 0, 0, 42] },
        ] }] }] });

        let sheet = sheet(raw.clone());
        assert_eq!(
            texts(&sheet),
            vec!["Day 1"],
            "operand 4 is a script, and the game shows what it stores with \\V[n]"
        );

        let out = rendered(sheet, &[(0, "1日目")]);
        let list = &out["events"][1]["pages"][0]["list"];
        assert_eq!(list[0]["parameters"][4], "'1日目'");
        assert_eq!(list[1]["parameters"][4], "$gameParty.gold()");
    }

    #[test]
    fn a_note_gives_up_what_a_paired_tag_wraps_and_keeps_the_tag_itself() {
        let sheet = sheet(json!([null, {
            "id": 1,
            "name": "Iron Shield",
            "description": "Dented but sturdy.",
            "note": "<ORDER 3>\n<Help Description>\nA shield that has seen better days.\n</Help Description>",
        }]));

        assert_eq!(
            texts(&sheet),
            vec![
                "Iron Shield",
                "Dented but sturdy.",
                "\nA shield that has seen better days.\n"
            ],
            "a script reads the tag name and prints what it wraps"
        );

        let out = rendered(sheet, &[(2, "\n盾です。\n")]);
        assert_eq!(
            out[1]["note"], "<ORDER 3>\n<Help Description>\n盾です。\n</Help Description>",
            "only the words between the tags may move"
        );
    }

    #[test]
    fn a_paired_tag_wrapping_only_settings_is_listed_and_never_sent_to_the_model() {
        for body in [
            "\nid: 1\nname: 'Petals2'\nopacity: 180\nmove x: -1.5\nmove y: -0.5\n",
            "\nitem 43: 1\ngold: 10\n",
        ] {
            let sheet = sheet(json!([null, {
                "id": 1,
                "note": format!("<overlay>{body}</overlay>"),
            }]));

            assert!(
                listed(&sheet, body),
                "a plugin reads {body:?} back line by line with a regex, so sending it to the \
                 model costs a request and a mistranslated value takes the effect it configures \
                 out of the game"
            );
        }
    }

    #[test]
    fn a_paired_tag_whose_lines_all_open_with_a_key_is_listed_even_when_a_value_holds_words() {
        let keyed = sheet(json!([null, {
            "id": 1,
            "note": "<quest>\ntitle: 古い剣を探せ\n</quest>",
        }]));
        assert!(
            listed(&keyed, "\ntitle: 古い剣を探せ\n"),
            "the shape says machine even though this value reads like a title: an unsure guess \
             lands on listed, where the reader still sees the line and can hand the words in, \
             because guessing the other way sends fog settings to the model and breaks the \
             weather"
        );

        let prose = sheet(json!([null, {
            "id": 1,
            "note": "<desc>\n雨の日の話。\n</desc>",
        }]));
        assert!(
            !listed(&prose, "\n雨の日の話。\n"),
            "a body with no keys in it is the prose the pairing rule exists for"
        );
    }

    #[test]
    fn a_note_holding_only_settings_still_offers_nothing() {
        for note in [
            "<ORDER 3>",
            "<WordWrap>\n<Custom Param: 7>",
            "<light red_light_s, 128>",
            "<characterName:!$Overworld_Worldtree>",
            "<ExtraDamageBuff:a.atk * 2 - b.def>",
            "<PsensorF:2 Ld100>",
        ] {
            let sheet = sheet(json!([null, { "id": 1, "note": note }]));
            assert!(texts(&sheet).is_empty(), "{note} configures a script");
        }
    }

    #[test]
    fn a_note_tag_spelled_in_the_games_own_script_is_a_label_to_translate() {
        let sheet = sheet(json!([null, {
            "id": 1,
            "note": "<characterName:mains>\n<srpgClass:帝国軍>\n<srpgMove:3>",
        }]));

        assert_eq!(
            texts(&sheet),
            vec!["帝国軍"],
            "a plugin prints this one beside the unit, and the two around it are machinery"
        );
        assert!(
            !listed(&sheet, "帝国軍"),
            "no list of the game's names holds it"
        );
    }

    #[test]
    fn a_note_tag_naming_something_the_game_registers_is_held_back() {
        let sheet = registering(
            &["物理", "魔力", "魔力範囲"],
            json!([null, {
                "id": 1,
                "note": "<elementDamageBonus:魔力:50\n魔力範囲:50>\n<ExtendDesc:【攻撃属性：魔力】>",
            }]),
        );

        assert!(
            listed(&sheet, "魔力:50\n魔力範囲:50"),
            "the plugin reads this back with elements.indexOf, so translating it loses the id"
        );
        assert!(
            !listed(&sheet, "【攻撃属性：魔力】"),
            "a description that merely mentions an element is still a description"
        );
    }

    #[test]
    fn a_translated_note_tag_lands_back_between_its_own_angle_brackets() {
        let sheet = sheet(json!([null, {
            "id": 1,
            "note": "<characterName:mains>\n<srpgClass:帝国軍>\n<srpgMove:3>",
        }]));

        assert_eq!(
            rendered(sheet, &[(0, "Imperial Army")])[1]["note"],
            json!("<characterName:mains>\n<srpgClass:Imperial Army>\n<srpgMove:3>"),
            "the tag name and the settings around it have to survive untouched"
        );
    }

    #[test]
    fn a_note_tag_and_the_body_of_a_paired_tag_both_come_through() {
        let sheet = sheet(json!([null, {
            "id": 1,
            "note": "<srpgClass:帝国軍>\n<Desc>雨の日の話。</Desc>",
        }]));

        assert_eq!(texts(&sheet), vec!["帝国軍", "雨の日の話。"]);
    }

    #[test]
    fn a_note_argument_is_read_unpacked_and_written_back_packed() {
        let scene = told(json!({ "body": "\"雨の日の話。\"", "aside": "そして夏が来た。" }));

        assert_eq!(
            texts(&commanding(scene.clone())),
            vec!["雨の日の話。", "そして夏が来た。"],
            "the quotes a note is stored with are not something a reader translates"
        );

        let out = rendered(
            commanding(scene),
            &[(0, "A rainy day."), (1, "Then summer came.")],
        );
        let held = &out["events"][1]["pages"][0]["list"][0]["parameters"][3];

        assert_eq!(
            held["body"],
            json!("\"A rainy day.\""),
            "a plugin calls JSON.parse on a note, so it has to go back quoted"
        );
        assert_eq!(
            held["aside"],
            json!("Then summer came."),
            "a multiline_string was never packed, so packing it now would show the quotes"
        );
    }

    #[test]
    fn a_plugin_command_naming_the_folder_it_loads_from_still_gives_up_its_words() {
        let at = sandbox();
        put(at.path(), "js/plugins/Toriacontan/Teller.js", TELLER);

        let scene = json!({ "events": [null, { "pages": [{ "list": [
            {
                "code": 357, "indent": 0,
                "parameters": ["Toriacontan/Teller", "tell", "", { "aside": "そして夏が来た。" }],
            },
        ] }] }] });

        assert_eq!(
            texts(&parse(&scene.to_string(), &Vocabulary::read(at.path()))),
            vec!["そして夏が来た。"],
            "the engine strips the folder off before it looks the command up, so where the \
             plugin file was filed cannot decide whether a line reaches the reader"
        );
    }

    #[test]
    fn a_translation_that_would_cut_a_note_tag_short_is_refused() {
        let note = "<srpgClass:帝国軍>\n<srpgMove:3>";
        let scene = || json!([null, { "id": 1, "note": note }]);

        for broken in ["帝国 > 軍", "Army <Imperial>"] {
            let out = rendered(sheet(scene()), &[(0, broken)]);

            assert_eq!(
                out[1]["note"],
                json!(note),
                "the engine reads a note with /<([^<>:]+):?([^>]*)>/, so {broken:?} would end the \
                 tag early and leave the rest as loose text"
            );
        }

        let out = rendered(sheet(scene()), &[(0, "Imperial Army")]);
        assert_eq!(
            out[1]["note"],
            json!("<srpgClass:Imperial Army>\n<srpgMove:3>"),
            "a translation carrying no brackets goes in as it is"
        );
    }

    #[test]
    fn a_message_line_gives_up_the_row_the_engine_prints_and_no_other() {
        let sheet = sheet(json!({ "events": [null, { "pages": [{ "list": [
                { "code": 401, "indent": 0, "parameters": ["Leave it to me", "Miss Hero!"] },
                { "code": 0, "indent": 0, "parameters": [] },
            ] }] }] }));

        assert_eq!(
            texts(&sheet),
            vec!["Leave it to me"],
            "the interpreter adds parameters[0] of each 401 and never looks at the rest"
        );
    }
}
