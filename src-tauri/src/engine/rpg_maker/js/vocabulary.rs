use crate::engine::rpg_maker::harvest::LOOKED_UP_BY_NAME;
use crate::engine::rpg_maker::js::{DATA, SCRIPTS};
use crate::walk;
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

const NOT_WORDS: [&str; 22] = [
    "number",
    "boolean",
    "combo",
    "file",
    "actor",
    "class",
    "skill",
    "item",
    "weapon",
    "armor",
    "enemy",
    "troop",
    "state",
    "animation",
    "tileset",
    "common_event",
    "switch",
    "variable",
    "icon",
    "color",
    "location",
    "select",
];

static RE_LITERAL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"'((?:[^'\\\n]|\\.)*)'|"((?:[^"\\\n]|\\.)*)""#)
        .expect("RE_LITERAL is a valid pattern")
});

static READ_ONCE: LazyLock<Mutex<HashMap<PathBuf, Arc<Vocabulary>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn forget() {
    if let Ok(mut held) = READ_ONCE.lock() {
        held.clear();
    }
}

#[derive(Debug, Default)]
pub struct Vocabulary {
    prose_args: HashMap<String, HashMap<String, Vec<(String, Holds)>>>,
    params: HashMap<String, Vec<(String, Holds)>>,
    fields: HashMap<(String, String), Vec<(String, Holds)>>,
    identifiers: HashSet<String>,
    registered: HashSet<String>,
}

impl Vocabulary {
    pub fn shared(root: &Path) -> Arc<Vocabulary> {
        if let Some(found) = READ_ONCE.lock().expect("vocabulary lock").get(root) {
            return Arc::clone(found);
        }

        let built = Arc::new(Vocabulary::read(root));

        Arc::clone(
            READ_ONCE
                .lock()
                .expect("vocabulary lock")
                .entry(root.to_path_buf())
                .or_insert(built),
        )
    }

    pub fn read(root: &Path) -> Self {
        let mut found = Vocabulary::default();

        found.plugins(&root.join(SCRIPTS).join("plugins"));
        found.data(&root.join(DATA));
        found.assets(root);

        found
    }

    pub fn args_of(&self, plugin: &str, command: &str) -> &[(String, Holds)] {
        self.prose_args
            .get(stem_of(plugin))
            .and_then(|commands| commands.get(command))
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub fn params_of(&self, plugin: &str) -> &[(String, Holds)] {
        self.params
            .get(stem_of(plugin))
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub fn fields_of(&self, plugin: &str, named: &str) -> &[(String, Holds)] {
        self.fields
            .get(&(stem_of(plugin).to_string(), named.to_string()))
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub fn registered(&self, name: &str) -> bool {
        self.registered.contains(name)
    }

    pub fn identifier(&self, token: &str) -> bool {
        self.identifiers.contains(token)
    }

    pub fn ships(&self, text: &str) -> bool {
        let bare = text.trim();
        let last = bare.rsplit(['/', '\\']).next().unwrap_or(bare);
        let stem = last.rsplit_once('.').map_or(last, |(stem, _)| stem);

        !bare.is_empty()
            && (self.identifier(bare) || self.identifier(last) || self.identifier(stem))
    }

    fn plugins(&mut self, dir: &Path) {
        for at in walk::files_now(dir) {
            let Some(plugin) = at
                .file_stem()
                .and_then(OsStr::to_str)
                .filter(|_| at.extension().is_some_and(|kind| kind == "js"))
            else {
                continue;
            };

            let Ok(source) = fs::read_to_string(&at) else {
                continue;
            };

            let declared = declarations(&source);
            for (command, arg) in declared.args {
                self.insert(plugin, command, arg);
            }
            if !declared.params.is_empty() {
                self.params.insert(plugin.to_string(), declared.params);
            }
            self.fields.extend(
                declared
                    .fields
                    .into_iter()
                    .map(|(named, held)| ((plugin.to_string(), named), held)),
            );

            for found in RE_LITERAL.captures_iter(&source) {
                if let Some(text) = found.get(1).or_else(|| found.get(2)) {
                    self.identifiers.insert(text.as_str().to_string());
                }
            }
        }
    }

    fn data(&mut self, dir: &Path) {
        for at in walk::files_now(dir) {
            if let Ok(body) = fs::read_to_string(&at) {
                keys_in(&body, &mut self.identifiers);
            }
        }

        self.registry(&dir.join("System.json"));
    }

    fn registry(&mut self, at: &Path) {
        let Ok(body) = fs::read_to_string(at) else {
            return;
        };
        let body = body.strip_prefix('\u{feff}').unwrap_or(&body);
        let Ok(root) = serde_json::from_str::<serde_json::Value>(body) else {
            return;
        };

        for list in LOOKED_UP_BY_NAME {
            let Some(names) = root.get(list).and_then(serde_json::Value::as_array) else {
                continue;
            };

            for name in names.iter().filter_map(serde_json::Value::as_str) {
                let name = name.trim();
                if !name.is_empty() {
                    self.registered.insert(name.to_string());
                }
            }
        }
    }

    fn assets(&mut self, root: &Path) {
        for at in walk::outside(root, &[DATA, SCRIPTS]) {
            if let Some(stem) = at.file_stem().and_then(OsStr::to_str) {
                self.identifiers.insert(stem.to_string());
            }
        }
    }

    fn insert(&mut self, plugin: &str, command: String, arg: (String, Holds)) {
        let args = self
            .prose_args
            .entry(plugin.to_string())
            .or_default()
            .entry(command)
            .or_default();

        if !args.iter().any(|(named, _)| *named == arg.0) {
            args.push(arg);
        }
    }

    #[cfg(test)]
    fn commands(&self) -> usize {
        self.prose_args.values().map(HashMap::len).sum()
    }

    #[cfg(test)]
    pub fn of(words: &[&str]) -> Self {
        Self {
            identifiers: words.iter().map(|word| (*word).to_string()).collect(),
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub fn registering(names: &[&str]) -> Self {
        Self {
            registered: names.iter().map(|name| (*name).to_string()).collect(),
            ..Self::default()
        }
    }
}

fn stem_of(plugin: &str) -> &str {
    plugin.rsplit('/').next().unwrap_or(plugin)
}

fn keys_in(body: &str, into: &mut HashSet<String>) {
    let bytes = body.as_bytes();
    let mut at = 0;

    while at < bytes.len() {
        if bytes[at] != b'"' {
            at += 1;
            continue;
        }

        let opened = at + 1;
        let mut closed = opened;
        let mut plain = true;

        while closed < bytes.len() && bytes[closed] != b'"' {
            if bytes[closed] == b'\\' {
                plain = false;
                closed += 1;
            }
            closed += 1;
        }

        if closed >= bytes.len() {
            return;
        }

        let mut after = closed + 1;
        while after < bytes.len() && bytes[after].is_ascii_whitespace() {
            after += 1;
        }

        if plain && after < bytes.len() && bytes[after] == b':' {
            into.insert(body[opened..closed].to_string());
        }

        at = closed + 1;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Holds {
    Prose,
    Note,
    Plain,
    List(Box<Holds>),
    Fields(String),
}

impl Holds {
    pub fn unpacked(said: &str) -> Option<String> {
        serde_json::from_str(said).ok()
    }

    pub fn packed(text: &str) -> String {
        serde_json::to_string(text).unwrap_or_else(|_| text.to_string())
    }
}

const CODE_FIELDS: [&str; 5] = ["script", "js", "eval", "formula", "code"];

const DRAWN_PARTS: [&str; 6] = ["skeleton", "bone", "slot", "skin", "track", "attachment"];

fn listed_in(known: &[&str], bare: &str) -> bool {
    known.contains(&bare) || known.contains(&bare.trim_end_matches('s'))
}

pub fn keyed(name: &str) -> bool {
    let head = name.split_once(':').map_or(name, |(head, _)| head);
    let bare = head.trim().to_ascii_lowercase();

    listed_in(&NOT_WORDS, &bare)
        || listed_in(&DRAWN_PARTS, &bare)
        || listed_in(&DRAWN_PARTS, bare.trim_end_matches("name"))
}

pub fn coded(name: &str) -> bool {
    let (head, kind) = name.split_once(':').unwrap_or((name, ""));

    matches!(kind, "func" | "css" | "eval" | "e")
        || head.ends_with("JS")
        || CODE_FIELDS
            .iter()
            .any(|known| head.eq_ignore_ascii_case(known))
}

fn holds(kind: &str) -> Option<Holds> {
    if let Some(inner) = kind.strip_suffix("[]") {
        return holds(inner).map(|one| Holds::List(Box::new(one)));
    }

    if let Some(named) = kind
        .strip_prefix("struct<")
        .and_then(|rest| rest.strip_suffix('>'))
    {
        return Some(Holds::Fields(named.to_string()));
    }

    match kind {
        "multiline_string" => Some(Holds::Prose),
        "note" => Some(Holds::Note),
        _ if NOT_WORDS.contains(&kind) => None,
        _ => Some(Holds::Plain),
    }
}

fn struct_named(line: &str) -> Option<&str> {
    line.trim_start()
        .strip_prefix("/*~struct~")?
        .split(':')
        .next()
        .map(str::trim)
}

#[derive(Default)]
struct Declared {
    args: Vec<(String, (String, Holds))>,
    params: Vec<(String, Holds)>,
    fields: HashMap<String, Vec<(String, Holds)>>,
}

fn declarations(source: &str) -> Declared {
    let mut found = Declared::default();
    let mut command: Option<String> = None;
    let mut arg: Option<String> = None;
    let mut param: Option<String> = None;
    let mut inside: Option<String> = None;

    let settle =
        |found: &mut Declared, param: &mut Option<String>, inside: &Option<String>, kind: &str| {
            let Some(named) = param.take() else {
                return;
            };
            let Some(shape) = holds(kind) else {
                return;
            };

            match inside {
                Some(block) => found
                    .fields
                    .entry(block.clone())
                    .or_default()
                    .push((named, shape)),
                None => found.params.push((named, shape)),
            }
        };

    for line in source.lines() {
        if let Some(named) = struct_named(line) {
            settle(&mut found, &mut param, &inside, "");
            inside = Some(named.to_string());
            command = None;
            arg = None;
            continue;
        }

        if line.trim_start().starts_with("/*:") {
            settle(&mut found, &mut param, &inside, "");
            inside = None;
            continue;
        }

        let Some((tag, rest)) = tag_of(line) else {
            continue;
        };

        match tag {
            "command" => {
                settle(&mut found, &mut param, &inside, "");
                command = Some(rest.to_string());
                arg = None;
            }
            "arg" => {
                settle(&mut found, &mut param, &inside, "");
                arg = Some(rest.to_string());
            }
            "param" => {
                settle(&mut found, &mut param, &inside, "");
                param = Some(rest.to_string());
                arg = None;
            }
            "type" => {
                let kind = rest.split_whitespace().next().unwrap_or_default();

                if let (Some(named), Some(held)) = (command.as_ref(), arg.take())
                    && let Some(shape) = holds(kind)
                    && matches!(shape, Holds::Prose | Holds::Note)
                {
                    found.args.push((named.clone(), (held, shape)));
                }

                settle(&mut found, &mut param, &inside, kind);
            }
            _ => {}
        }
    }

    settle(&mut found, &mut param, &inside, "");

    found
}

fn tag_of(line: &str) -> Option<(&str, &str)> {
    let rest = line
        .trim_start()
        .strip_prefix('*')?
        .trim_start()
        .strip_prefix('@')?;

    let (tag, rest) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));

    Some((tag, rest.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::rpg_maker::js::fixture::{put, sandbox};

    const HEADER: &str = r"
/*:
 * @target MZ
 * @plugindesc Shows text as a picture.
 *
 * @param fontSize
 * @type number
 * @default 28
 *
 * @command set
 * @text Set Text Picture
 * @desc Sets the text of the picture.
 *
 * @arg text
 * @type multiline_string
 * @text Text
 *
 * @command clear
 * @text Clear
 *
 * @arg pictureId
 * @type number
 * @text Picture
 *
 * @arg easing
 * @type select
 * @option linear
 * @text Easing
 *
 * @command play
 * @text Play
 *
 * @arg id
 * @type text
 * @text Video id
 *
 * @arg onEnd
 * @type text
 * @text Script to run
 */
";

    #[test]
    fn only_arguments_declared_as_a_block_of_prose_are_taken() {
        let found = declarations(HEADER).args;

        assert_eq!(
            found,
            vec![("set".to_string(), ("text".to_string(), Holds::Prose))],
            "a number is a setting and a select is a preset: translating either breaks the plugin"
        );
    }

    #[test]
    fn a_setting_picked_from_a_list_the_plugin_reads_back_is_never_offered_as_words() {
        for kind in ["select", "combo"] {
            assert!(
                holds(kind).is_none(),
                "a plugin compares {kind} against the option it shipped, so translating the value \
                 stops the match and the setting quietly turns itself off"
            );
        }

        assert_eq!(holds("multiline_string"), Some(Holds::Prose));
        assert_eq!(holds("note"), Some(Holds::Note));
        assert_eq!(holds("string"), Some(Holds::Plain));
    }

    #[test]
    fn a_plugin_level_param_is_not_mistaken_for_a_command_argument() {
        let header = r"
 * @param message
 * @type multiline_string
 * @command run
 * @arg amount
 * @type note
";

        let found = declarations(header);

        assert_eq!(
            found.args,
            vec![("run".to_string(), ("amount".to_string(), Holds::Note))],
            "@param configures the plugin, it is never sent as a command argument"
        );
        assert_eq!(
            found.params,
            vec![("message".to_string(), Holds::Prose)],
            "and it is still offered on its own, or the settings a reader sees in the editor stay \
             in the language the plugin shipped in"
        );
    }

    #[test]
    fn the_same_command_declared_twice_is_only_listed_once() {
        let twice = format!("{HEADER}\n{HEADER}");
        let mut words = Vocabulary::default();

        for (command, arg) in declarations(&twice).args {
            words.insert("P", command, arg);
        }

        assert_eq!(
            words.args_of("P", "set"),
            [("text".to_string(), Holds::Prose)]
        );
        assert_eq!(words.commands(), 1);
    }

    #[test]
    fn a_folder_with_no_plugins_simply_offers_nothing() {
        let words = Vocabulary::read(Path::new("/nowhere/at/all"));

        assert!(words.args_of("TextPicture", "set").is_empty());
        assert!(!words.identifier("anything"));
    }

    #[test]
    fn only_the_names_on_the_left_of_a_colon_are_keys() {
        let mut found = HashSet::new();
        keys_in(
            r#"{"00intro": [{"code": 401, "parameters": ["Bob: hello", "a\"b"]}], "note": ""}"#,
            &mut found,
        );

        let mut names: Vec<&str> = found.iter().map(String::as_str).collect();
        names.sort_unstable();

        assert_eq!(
            names,
            ["00intro", "code", "note", "parameters"],
            "a line of dialogue may hold a colon, and it is still not a key"
        );
    }

    #[test]
    fn a_folder_is_only_ever_read_once_until_the_game_is_let_go() {
        let at = sandbox();
        put(at.path(), "data/System.json", "{}");

        let first = Vocabulary::shared(at.path());
        let again = Vocabulary::shared(at.path());

        assert!(
            Arc::ptr_eq(&first, &again),
            "every command opens the game afresh, so a second read would be paid on every click"
        );

        forget();
        let fresh = Vocabulary::shared(at.path());

        assert!(
            !Arc::ptr_eq(&first, &fresh),
            "a game that was let go is read again next time, or every game ever opened stays in \
             memory for the life of the app"
        );
    }

    #[test]
    fn the_names_a_note_is_checked_against_are_the_ones_never_translated() {
        let at = sandbox();
        let root = at.path();

        put(
            root,
            "data/System.json",
            r#"{"elements": ["", "物理", "魔力"], "skillTypes": ["", "剣術"]}"#,
        );

        let words = Vocabulary::read(root);

        assert!(words.registered("魔力"));
        assert!(
            !words.registered("剣術"),
            "the battle menu prints a skill type, so it is translated and a note naming it has to \
             be translated to match: only a list held back may answer for a note"
        );
    }

    #[test]
    fn the_game_tells_which_of_its_own_words_are_identifiers() {
        let at = sandbox();
        let root = at.path();

        put(
            root,
            "js/plugins/Saba_Tachie.js",
            "case 'showName': $gameTemp.tachieName = args[1]; break;",
        );
        put(
            root,
            "data/Scenario.json",
            r#"{"00intro_01": [{"code": 0, "indent": 0, "parameters": []}]}"#,
        );
        put(root, "img/pictures/menu_lil.png", "");

        let words = Vocabulary::read(root);

        assert!(
            words.identifier("showName"),
            "a plugin compares its own keywords, so they appear in its source"
        );
        assert!(
            words.identifier("00intro_01"),
            "a scenario id is a key of a data file: translating it stops the lookup"
        );
        assert!(
            words.identifier("menu_lil"),
            "an argument that names a shipped file is a file name"
        );
        assert!(
            !words.identifier("Old Storyteller"),
            "the game never uses a speaker name as an identifier"
        );
    }
}
