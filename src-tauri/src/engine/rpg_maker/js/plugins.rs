use crate::engine::rpg_maker::harvest::Step;
use crate::engine::rpg_maker::js::vocabulary::{Holds, Vocabulary, coded, keyed};
use crate::engine::rpg_maker::js::{SCRIPTS, script};
use crate::engine::rpg_maker::{Gathered, text};
use crate::engine::{Applied, Offer, Parsed, TranslationUnit, hand_written};
use anyhow::Result;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::ffi::OsStr;
use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

const NAME: &str = "plugins.js";

pub fn is_list(at: &Path) -> bool {
    at.file_name().and_then(OsStr::to_str) == Some(NAME)
        && at.parent().is_some_and(|up| up.ends_with(SCRIPTS))
}

pub struct PluginList {
    opened: String,
    listed: Option<Vec<Value>>,
    shut: String,
    units: Vec<TranslationUnit>,
    spots: Vec<Setting>,
    shapes: HashMap<(usize, String), Holds>,
    words: Arc<Vocabulary>,
}

struct Setting {
    which: usize,
    param: String,
    into: Vec<Step>,
}

struct Taken {
    text: String,
    into: Vec<Step>,
    offer: Offer,
}

pub fn parse(body: &str, words: &Arc<Vocabulary>) -> PluginList {
    let held = listing(body).and_then(|at| {
        let entries = serde_json::from_str::<Vec<Value>>(&body[at.clone()]).ok()?;
        Some((at, entries))
    });

    let Some((at, listed)) = held else {
        return PluginList {
            opened: body.to_string(),
            listed: None,
            shut: String::new(),
            units: Vec::new(),
            spots: Vec::new(),
            shapes: HashMap::new(),
            words: Arc::clone(words),
        };
    };

    let mut taken: Gathered<Setting> = Gathered::default();
    let mut shapes: HashMap<(usize, String), Holds> = HashMap::new();

    for (which, entry) in listed.iter().enumerate() {
        let name = entry
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let Some(held) = entry.get("parameters").and_then(Value::as_object) else {
            continue;
        };

        for (param, shape) in words.params_of(name) {
            let Some(said) = held.get(param).and_then(Value::as_str) else {
                continue;
            };

            let found = gathered(said, shape, words, Under::of(name, param));

            if !found.is_empty() {
                shapes.insert((which, param.clone()), shape.clone());
            }

            for one in found {
                taken.take(
                    &one.text,
                    Setting {
                        which,
                        param: param.clone(),
                        into: one.into,
                    },
                    one.offer,
                );
            }
        }
    }

    let (units, spots) = taken.done();

    PluginList {
        opened: body[..at.start].to_string(),
        listed: Some(listed),
        shut: body[at.end..].to_string(),
        units,
        spots,
        shapes,
        words: Arc::clone(words),
    }
}

#[derive(Clone, Copy)]
struct Under<'a> {
    plugin: &'a str,
    coded: bool,
    keyed: bool,
}

impl<'a> Under<'a> {
    fn of(plugin: &'a str, named: &str) -> Self {
        Self {
            plugin,
            coded: coded(named),
            keyed: keyed(named),
        }
    }

    fn inside(self, named: &str) -> Self {
        Self {
            plugin: self.plugin,
            coded: self.coded || coded(named),
            keyed: self.keyed || keyed(named),
        }
    }
}

fn walk(
    said: &str,
    shape: &Holds,
    words: &Vocabulary,
    under: Under<'_>,
    at: &mut Vec<Step>,
    word: &mut impl FnMut(&[Step], &str, &Holds, Under<'_>) -> Option<String>,
) -> Option<String> {
    match shape {
        Holds::List(inner) => {
            let Ok(mut items) = serde_json::from_str::<Vec<String>>(said) else {
                return None;
            };

            let mut changed = false;
            for (which, item) in items.iter_mut().enumerate() {
                at.push(Step::Index(which));
                let fresh = walk(item, inner, words, under, at, word);
                at.pop();

                if let Some(fresh) = fresh {
                    *item = fresh;
                    changed = true;
                }
            }

            changed.then(|| serde_json::to_string(&items).unwrap_or_else(|_| said.to_string()))
        }
        Holds::Fields(named) => {
            let Ok(mut held) = serde_json::from_str::<serde_json::Map<String, Value>>(said) else {
                return None;
            };

            let mut changes = Vec::new();
            for (field, inner) in words.fields_of(under.plugin, named) {
                let Some(was) = held.get(field).and_then(Value::as_str) else {
                    continue;
                };

                at.push(Step::Key(field.clone()));
                let fresh = walk(was, inner, words, under.inside(field), at, word);
                at.pop();

                if let Some(fresh) = fresh {
                    changes.push((field.clone(), fresh));
                }
            }

            let changed = !changes.is_empty();
            for (field, fresh) in changes {
                held.insert(field, Value::String(fresh));
            }

            changed.then(|| serde_json::to_string(&held).unwrap_or_else(|_| said.to_string()))
        }
        Holds::Note => {
            let body = Holds::unpacked(said)?;

            leaves(&body, shape, under, at, word).map(|fresh| Holds::packed(&fresh))
        }
        Holds::Prose | Holds::Plain => leaves(said, shape, under, at, word),
    }
}

fn leaves(
    text: &str,
    shape: &Holds,
    under: Under<'_>,
    at: &mut Vec<Step>,
    word: &mut impl FnMut(&[Step], &str, &Holds, Under<'_>) -> Option<String>,
) -> Option<String> {
    if !under.coded {
        return word(at, text, shape, under);
    }

    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    for (which, one) in script::literals(text).into_iter().enumerate() {
        at.push(Step::Index(which));
        let fresh = word(at, &one.text, shape, under);
        at.pop();

        if let Some(fresh) = fresh {
            edits.push((one.at, script::js_escape(&fresh)));
        }
    }

    if edits.is_empty() {
        return None;
    }

    let mut out = text.to_string();
    for (at, fresh) in edits.into_iter().rev() {
        out.replace_range(at, &fresh);
    }

    Some(out)
}

fn gathered(said: &str, shape: &Holds, words: &Vocabulary, under: Under<'_>) -> Vec<Taken> {
    let mut found = Vec::new();

    walk(
        said,
        shape,
        words,
        under,
        &mut Vec::new(),
        &mut |at, text, shape, under| {
            if under.coded {
                found.push(Taken {
                    text: text.to_string(),
                    into: at.to_vec(),
                    offer: Offer::default().or_listed(text::listed_line(text)),
                });
            } else if text::has_words(text) {
                let listed = (under.keyed && matches!(shape, Holds::Plain))
                    || a_name(words, shape, text)
                    || text::listed_line(text);

                found.push(Taken {
                    text: text.to_string(),
                    into: at.to_vec(),
                    offer: Offer::default().or_listed(listed),
                });
            }

            None
        },
    );

    found
}

fn a_name(words: &Vocabulary, shape: &Holds, text: &str) -> bool {
    if !matches!(shape, Holds::Plain) {
        return false;
    }

    words.ships(text) || (!hand_written(text) && !text.chars().any(char::is_whitespace))
}

fn listing(body: &str) -> Option<Range<usize>> {
    let opened = body.find('[')?;
    let shut = body.rfind(']')?;

    (opened < shut).then_some(opened..shut + 1)
}

impl Parsed for PluginList {
    fn units(&self) -> &[TranslationUnit] {
        &self.units
    }

    fn render(self: Box<Self>, translations: &BTreeMap<u32, String>) -> Result<(String, Applied)> {
        let this = *self;
        let mut applied = Applied::default();

        let Some(mut listed) = this.listed else {
            return Ok((this.opened, applied));
        };

        let mut edits: HashMap<(usize, String), HashMap<Vec<Step>, String>> = HashMap::new();

        for (unit, spot) in this.units.iter().zip(&this.spots) {
            let Some(translation) = unit.answer(translations) else {
                continue;
            };

            edits
                .entry((spot.which, spot.param.clone()))
                .or_default()
                .insert(spot.into.clone(), translation.clone());
        }

        for ((which, param), wanted) in edits {
            let Some(shape) = this.shapes.get(&(which, param.clone())) else {
                continue;
            };

            let Some(plugin) = listed
                .get(which)
                .and_then(|entry| entry.get("name"))
                .and_then(Value::as_str)
                .map(str::to_string)
            else {
                continue;
            };

            let Some(slot) = listed
                .get_mut(which)
                .and_then(|entry| entry.get_mut("parameters"))
                .and_then(|held| held.get_mut(&param))
            else {
                continue;
            };

            let Some(was) = slot.as_str() else {
                continue;
            };

            let fresh = walk(
                was,
                shape,
                &this.words,
                Under::of(&plugin, &param),
                &mut Vec::new(),
                &mut |at, _, _, _| {
                    let fresh = wanted.get(at)?;
                    applied.lines += 1;

                    Some(fresh.clone())
                },
            );

            if let Some(fresh) = fresh {
                *slot = Value::String(fresh);
            }
        }

        Ok((
            format!("{}{}{}", this.opened, rows(&listed)?, this.shut),
            applied,
        ))
    }
}

fn rows(listed: &[Value]) -> Result<String> {
    let mut out = String::from("[\n");

    for (which, entry) in listed.iter().enumerate() {
        if which > 0 {
            out.push_str(",\n");
        }
        out.push_str(&serde_json::to_string(entry)?);
    }

    out.push_str("\n]");

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::rpg_maker::js::fixture::{put, sandbox};
    use serde_json::json;

    const HEADER: &str = "/*:
 * @param textSrpgTurnEnd
 * @type string
 *
 * @param mapWidth
 * @type string
 *
 * @param helpWindow
 * @type multiline_string
 *
 * @param faceFile
 * @type file
 *
 * @param captions
 * @type struct<Caption>[]
 *
 * @param cheers
 * @type string[]
 *
 * @param credits
 * @type note
 */
/*~struct~Caption:
 * @param icon
 * @type number
 *
 * @param words
 * @type string
 */";

    fn packed(value: Value) -> String {
        serde_json::to_string(&value).expect("json")
    }

    fn listing_of(parameters: Value) -> String {
        format!(
            "// Generated by RPG Maker.\nvar $plugins =\n[\n{}\n];\n",
            json!({ "name": "SRPG_core_MZ", "status": true, "parameters": parameters })
        )
    }

    fn every() -> String {
        listing_of(json!({
            "textSrpgTurnEnd": "ターン終了",
            "mapWidth": "640",
            "helpWindow": "Pick a unit.",
            "faceFile": "actor1",
            "captions": packed(json!([packed(json!({ "icon": "2", "words": "毒" }))])),
            "cheers": packed(json!(["やった", "ok"])),
            "credits": packed(json!("作：だれか")),
        }))
    }

    fn read(body: &str) -> PluginList {
        let at = sandbox();
        put(at.path(), "js/plugins/SRPG_core_MZ.js", HEADER);
        parse(body, &Arc::new(Vocabulary::read(at.path())))
    }

    fn texts(sheet: &PluginList) -> Vec<&str> {
        sheet.units().iter().map(|one| one.text.as_str()).collect()
    }

    fn parameters(body: &str) -> Value {
        let listed: Value =
            serde_json::from_str(&body[listing(body).expect("an array")]).expect("valid json");

        listed[0]["parameters"].clone()
    }

    const OTHER: &str = "/*:
 * @param quests
 * @type struct<Quest>[]
 */
/*~struct~Quest:
 * @param title
 * @text Name
 *
 * @param tasks:strA
 * @type text[]
 *
 * @param PositionJS:func
 * @type note
 *
 * @param Prompt:eval
 * @type note
 */";

    const STATES: &str = "/*:
 * @param states
 * @type struct<StateConfig>[]
 */
/*~struct~StateConfig:
 * @param stateId
 * @type state
 *
 * @param description
 * @type note
 *
 * @param label
 * @type string
 */";

    const FOLDERED: &str = "/*:
 * @param Log Command
 * @desc Command name to display battle past log
 * @default Battle Log
 *
 * @param Past Log Window Params
 * @type struct<PastLogWindow>
 */
/*~struct~PastLogWindow:
 * @param Title
 * @type string
 */";

    const SPINE: &str = "/*:
 * @param skeleton
 * @type string
 *
 * @param animation
 * @type string[]
 *
 * @param boneName
 * @type string
 *
 * @param itemName
 * @type string
 *
 * @param label
 * @type string
 */";

    #[test]
    fn a_prose_box_under_a_setting_named_like_a_lookup_is_still_the_readers_to_translate() {
        let at = sandbox();
        put(
            at.path(),
            "js/plugins/KEN_BattleStateInformation.js",
            STATES,
        );

        let body = format!(
            "// Generated by RPG Maker.\nvar $plugins =\n[\n{}\n];\n",
            json!({
                "name": "KEN_BattleStateInformation",
                "status": true,
                "parameters": {
                    "states": packed(json!([packed(json!({
                        "stateId": "175",
                        "description": packed(json!(
                            "素早さが50％低下する。\n魔法攻撃を受けると\\C[24]着地\\C[0]する。"
                        )),
                        "label": "着地",
                    }))])),
                },
            })
        );

        let sheet = parse(&body, &Arc::new(Vocabulary::read(at.path())));
        let offered: Vec<(&str, Offer)> = sheet
            .units()
            .iter()
            .map(|one| (one.text.as_str(), one.offer))
            .collect();

        assert_eq!(
            offered
                .iter()
                .find(|(text, _)| text.contains('素'))
                .map(|(_, offer)| *offer),
            Some(Offer::Asked),
            "the list is named states, but the plugin itself declares this field a note: a box \
             the developer typed prose into for the player to read. The declared shape outranks \
             the name of the list it sits in, or every description a plugin draws stays foreign: \
             {offered:?}"
        );

        assert_eq!(
            offered
                .iter()
                .find(|(text, _)| *text == "着地")
                .map(|(_, offer)| *offer),
            Some(Offer::Listed),
            "a plain string beside it says nothing about being prose, so the list's name still \
             counts: {offered:?}"
        );

        assert!(
            !offered.iter().any(|(text, _)| *text == "175"),
            "and the state id the plugin looks up never leaves the file: {offered:?}"
        );
    }

    #[test]
    fn a_setting_named_after_a_thing_the_engine_looks_up_is_never_sent_off_to_be_translated() {
        let at = sandbox();
        put(at.path(), "js/plugins/NrSpineChoice.js", SPINE);

        let body = format!(
            "// Generated by RPG Maker.\nvar $plugins =\n[\n{}\n];\n",
            json!({
                "name": "NrSpineChoice",
                "status": true,
                "parameters": {
                    "skeleton": "\u{9078}\u{629e}\u{80a2}",
                    "animation": packed(json!([
                        "\u{30aa}\u{30fc}\u{30d7}\u{30cb}\u{30f3}\u{30b0}",
                        "\u{30aa}\u{30fc}\u{30d7}\u{30cb}\u{30f3}\u{30b0}2",
                    ])),
                    "boneName": "\u{30b2}\u{30fc}\u{30b8}M",
                    "itemName": "\u{30de}\u{30b9}\u{30bf}\u{30fc}\u{97f3}\u{91cf}",
                    "label": "\u{30bf}\u{30fc}\u{30f3}\u{7d42}\u{4e86}",
                }
            })
        );

        let sheet = parse(&body, &Arc::new(Vocabulary::read(at.path())));
        let offered: Vec<(&str, Offer)> = sheet
            .units()
            .iter()
            .map(|one| (one.text.as_str(), one.offer))
            .collect();

        for named in [
            "\u{30aa}\u{30fc}\u{30d7}\u{30cb}\u{30f3}\u{30b0}",
            "\u{30aa}\u{30fc}\u{30d7}\u{30cb}\u{30f3}\u{30b0}2",
            "\u{30b2}\u{30fc}\u{30b8}M",
        ] {
            assert_eq!(
                offered
                    .iter()
                    .find(|(text, _)| *text == named)
                    .map(|(_, offer)| *offer),
                Some(Offer::Listed),
                "the plugin calls this setting an animation, and an animation lives inside the \
                 skeleton by that exact name: translating it leaves the game asking for one that \
                 was never drawn. No file is named after it, so only the setting's own name says \
                 so: {offered:?}"
            );
        }

        for said in [
            "\u{30bf}\u{30fc}\u{30f3}\u{7d42}\u{4e86}",
            "\u{30de}\u{30b9}\u{30bf}\u{30fc}\u{97f3}\u{91cf}",
        ] {
            assert_eq!(
                offered
                    .iter()
                    .find(|(text, _)| *text == said)
                    .map(|(_, offer)| *offer),
                Some(Offer::Asked),
                "a setting beside it that names nothing is still the reader's to translate, and \
                 itemName ends the same way boneName does without meaning the same thing: \
                 trimming the ending off every name would take a menu label with it"
            );
        }
    }

    #[test]
    fn a_plugin_kept_in_a_folder_still_hands_over_every_setting_it_declared() {
        let at = sandbox();
        put(
            at.path(),
            "js/plugins/MokuseiPengin/MPP_SmoothBattleLog.js",
            FOLDERED,
        );

        let body = format!(
            "// Generated by RPG Maker.\nvar $plugins =\n[\n{}\n];\n",
            json!({
                "name": "MokuseiPengin/MPP_SmoothBattleLog",
                "status": true,
                "parameters": {
                    "Log Command": "戦闘ログ",
                    "Past Log Window Params": packed(json!({ "Title": "過去ログ" })),
                },
            })
        );

        let sheet = parse(&body, &Arc::new(Vocabulary::read(at.path())));
        let offered: Vec<(&str, Offer)> = sheet
            .units()
            .iter()
            .map(|one| (one.text.as_str(), one.offer))
            .collect();

        for said in ["戦闘ログ", "過去ログ"] {
            assert_eq!(
                offered
                    .iter()
                    .find(|(text, _)| *text == said)
                    .map(|(_, offer)| *offer),
                Some(Offer::Asked),
                "the listing names a plugin by the path it loads from, while the engine keys its \
                 settings by the bare file name: a folder in that path must not swallow the \
                 command the player reads: {offered:?}"
            );
        }
    }

    #[test]
    fn a_setting_naming_a_picture_the_game_ships_is_never_sent_off_to_be_translated() {
        let at = sandbox();
        put(at.path(), "js/plugins/SRPG_core_MZ.js", HEADER);
        put(at.path(), "img/system/\u{3064}\u{307e}\u{307f}.png", "");
        put(
            at.path(),
            "img/spines/UI/\u{30bf}\u{30a4}\u{30c8}\u{30eb}.json",
            "",
        );

        let body = listing_of(json!({
            "textSrpgTurnEnd": "\u{3064}\u{307e}\u{307f}",
            "helpWindow": "\u{30bf}\u{30fc}\u{30f3}\u{7d42}\u{4e86}",
            "cheers": packed(json!(["UI/\u{30bf}\u{30a4}\u{30c8}\u{30eb}"])),
        }));

        let sheet = parse(&body, &Arc::new(Vocabulary::read(at.path())));
        let offered: Vec<(&str, Offer)> = sheet
            .units()
            .iter()
            .map(|one| (one.text.as_str(), one.offer))
            .collect();

        for named in [
            "\u{3064}\u{307e}\u{307f}",
            "UI/\u{30bf}\u{30a4}\u{30c8}\u{30eb}",
        ] {
            assert_eq!(
                offered
                    .iter()
                    .find(|(text, _)| *text == named)
                    .map(|(_, offer)| *offer),
                Some(Offer::Listed),
                "this game ships a file by that name, so the setting points at an asset: \
                 translating it leaves the plugin looking up a name nothing answers to, and the \
                 game throws rather than draws. A game names its own files in its own alphabet, \
                 so reading the letters is what missed it: {offered:?}"
            );
        }

        assert_eq!(
            offered
                .iter()
                .find(|(text, _)| *text == "\u{30bf}\u{30fc}\u{30f3}\u{7d42}\u{4e86}")
                .map(|(_, offer)| *offer),
            Some(Offer::Asked),
            "and a line nothing in the game is filed under is still the reader's to translate"
        );
    }

    #[test]
    fn two_plugins_naming_a_struct_the_same_each_read_their_own_fields() {
        let at = sandbox();
        put(at.path(), "js/plugins/SRPG_core_MZ.js", HEADER);
        put(at.path(), "js/plugins/Journal.js", OTHER);
        let words = Arc::new(Vocabulary::read(at.path()));

        let body = format!(
            "// Generated by RPG Maker.\nvar $plugins =\n[\n{}\n];\n",
            json!({
                "name": "Journal",
                "status": true,
                "parameters": {
                    "quests": packed(json!([packed(json!({
                        "title": "\u{5e30}\u{308a}\u{9053}",
                        "tasks:strA": packed(json!(["\u{7bb1}\u{3092}\u{62fe}\u{3046}"])),
                        "PositionJS:func": packed(json!(
                            "const x = 20;\nreturn \"Show Battle Animations\";"
                        )),
                        "Prompt:eval": packed(json!(
                            "\"\u{540d}\u{524d}\u{3092}\u{5165}\u{529b}\u{3057}\u{3066}\u{304f}\u{3060}\u{3055}\u{3044}\""
                        )),
                    }))])),
                },
            })
        );

        let sheet = parse(&body, &words);
        let held = texts(&sheet);

        assert!(
            held.contains(&"\u{5e30}\u{308a}\u{9053}"),
            "a struct named Quest in one plugin has nothing to do with a Quest in another, and \
             reading one by the other's fields loses every line: {held:?}"
        );
        assert!(
            held.contains(&"\u{7bb1}\u{3092}\u{62fe}\u{3046}"),
            "a type this reader has never heard of is a string, the way the engine reads it too"
        );

        assert!(
            !held.iter().any(|one| one.contains("const x")),
            "the value of a func parameter is a function body, and offering it whole gives a \
             reader a line no translator can safely touch: {held:?}"
        );

        let drawn = sheet
            .units()
            .iter()
            .find(|one| one.text == "Show Battle Animations")
            .expect("a menu label a plugin builds inside a function is still read by a player");
        assert_eq!(
            drawn.offer,
            Offer::Asked,
            "pulled out of the code around it, the label is ordinary prose and there is nothing \
             left to hold it back for"
        );

        let expression = sheet
            .units()
            .iter()
            .find(|one| one.text.contains('\u{540d}'))
            .expect("an eval holding one quoted string is read the same way");
        assert_eq!(expression.offer, Offer::Asked);

        let wanted: BTreeMap<u32, String> = [
            (drawn.id, "\u{6226}\u{95d8}\u{6f14}\u{51fa}".to_string()),
            (expression.id, "\u{8a33}".to_string()),
        ]
        .into_iter()
        .collect();

        let (back, applied) = Box::new(parse(&body, &words))
            .render(&wanted)
            .expect("a rewritten list");

        assert_eq!(applied.lines, 2);
        assert!(
            back.contains("const x = 20;"),
            "the code around the label is the game's, and it comes back exactly as it went in: \
             {back}"
        );
        assert!(
            !back.contains("Show Battle Animations"),
            "and the label inside it took the translation: {back}"
        );
    }

    #[test]
    fn only_the_list_the_engine_loads_is_read_for_parameters() {
        assert!(is_list(Path::new("/game/js/plugins.js")));
        assert!(is_list(Path::new("www/js/plugins.js")));
        assert!(
            !is_list(Path::new("/game/js/plugins/SRPG_core_MZ.js")),
            "a plugin's own source is read for what it declares, never rewritten"
        );
        assert!(!is_list(Path::new("/game/data/plugins.js")));
    }

    #[test]
    fn a_setting_is_told_apart_from_a_word_the_player_reads() {
        let sheet = read(&every());
        let found = texts(&sheet);

        assert!(
            found.contains(&"ターン終了"),
            "a plain string somebody spelled out"
        );
        assert!(
            found.contains(&"Pick a unit."),
            "a multiline_string is prose either way"
        );
        assert!(!found.contains(&"640"), "a plain number is a setting");
        assert!(!found.contains(&"actor1"), "@type file names a picture");
        assert!(
            found.contains(&"ok"),
            "a plain string is still the reader's to see, whatever it turns out to be"
        );
        assert!(
            sheet
                .units()
                .iter()
                .find(|one| one.text == "ok")
                .expect("the line is there")
                .offer
                != Offer::Asked,
            "but one ascii word on its own is a keyword until it reads like a sentence, and a \
             model handed every one of them writes a game that looks itself up by the wrong name"
        );
    }

    #[test]
    fn a_word_inside_a_packed_list_or_struct_is_reached() {
        let sheet = read(&every());
        let found = texts(&sheet);

        assert!(
            found.contains(&"やった"),
            "string[] holds its items packed as json"
        );
        assert!(
            found.contains(&"毒"),
            "struct<Caption>[] packs a struct inside a list"
        );
        assert!(!found.contains(&"2"), "the struct calls its icon a number");
        assert!(
            found.contains(&"作：だれか"),
            "a note is packed as a json string, so its quotes are not part of what is read"
        );
    }

    #[test]
    fn a_translation_goes_back_packed_the_way_the_engine_expects() {
        let sheet = read(&every());
        let wanted: BTreeMap<u32, String> = sheet
            .units()
            .iter()
            .map(|one| (one.id, format!("<{}>", one.text)))
            .collect();

        let (back, applied) = Box::new(read(&every()))
            .render(&wanted)
            .expect("a rewritten list");

        assert_eq!(
            applied.lines, 6,
            "a line the reader settled by hand goes back whether it was marked a symbol or not"
        );

        let held = parameters(&back);
        assert_eq!(held["textSrpgTurnEnd"], "<ターン終了>");
        assert_eq!(
            held["mapWidth"], "640",
            "a setting is left exactly as it was"
        );
        assert_eq!(held["faceFile"], "actor1");
        assert_eq!(
            held["captions"],
            packed(json!([packed(json!({ "icon": "2", "words": "<毒>" }))])),
            "the struct is packed back inside the list, and the icon it never asked about stays"
        );
        assert_eq!(
            held["cheers"],
            packed(json!(["<やった>", "<ok>"])),
            "every item of the list is the reader's to settle, and what they settled is written"
        );
        assert_eq!(
            held["credits"],
            packed(json!("<作：だれか>")),
            "a note has to come back quoted or the plugin cannot read it"
        );
    }

    const CODED: &str = "/*:
 * @param LinesJS
 * @type string[]
 */";

    #[test]
    fn a_list_of_code_hands_its_wordings_back_where_the_reader_settled_them() {
        let at = sandbox();
        put(at.path(), "js/plugins/Coded.js", CODED);
        let words = Arc::new(Vocabulary::read(at.path()));

        let body = format!(
            "// Generated by RPG Maker.\nvar $plugins =\n[\n{}\n];\n",
            json!({
                "name": "Coded",
                "status": true,
                "parameters": {
                    "LinesJS": packed(json!([
                        "return \"Bring me the sword.\";",
                        "return \"And the shield.\";",
                    ])),
                },
            })
        );

        let sheet = parse(&body, &words);
        assert_eq!(
            texts(&sheet),
            ["Bring me the sword.", "And the shield."],
            "a coded parameter is walked for the wordings inside its code, and a list of code is \
             still a list"
        );

        let wanted: BTreeMap<u32, String> = sheet
            .units()
            .iter()
            .map(|one| (one.id, format!("<{}>", one.text)))
            .collect();
        let (back, applied) = Box::new(parse(&body, &words))
            .render(&wanted)
            .expect("a rewritten list");

        assert_eq!(
            applied.lines, 2,
            "the writer has to walk a list of code the same way the reader did, or the keys it \
             looks the translations up by are not the keys they were stored under"
        );
        assert_eq!(
            parameters(&back)["LinesJS"],
            packed(json!([
                "return \"<Bring me the sword.>\";",
                "return \"<And the shield.>\";",
            ])),
            "a translation the reader settled has to reach the game, and a coded list is where it \
             was quietly dropped before"
        );
    }

    #[test]
    fn a_list_left_alone_comes_back_exactly_as_it_was() {
        let body = every();
        let (back, applied) = Box::new(read(&body))
            .render(&BTreeMap::new())
            .expect("no change");

        assert_eq!(applied.lines, 0);
        assert_eq!(parameters(&back), parameters(&body));
        assert!(back.starts_with("// Generated by RPG Maker.\nvar $plugins =\n"));
        assert!(back.trim_end().ends_with("];"));
    }

    #[test]
    fn a_list_that_cannot_be_read_is_handed_back_untouched() {
        let at = sandbox();
        let broken = "var $plugins =\n[{\"name\": oops}];\n";
        let words = Arc::new(Vocabulary::read(at.path()));

        assert!(parse(broken, &words).units().is_empty());
        assert_eq!(
            Box::new(parse(broken, &words))
                .render(&BTreeMap::new())
                .expect("no change")
                .0,
            broken,
            "a file we could not read is a file we must not rewrite"
        );
    }
}
