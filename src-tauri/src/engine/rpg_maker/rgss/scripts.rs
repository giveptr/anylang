use crate::engine::rpg_maker::harvest::Step;
use crate::engine::rpg_maker::rgss::marshal::{self, Text};
use crate::engine::rpg_maker::rgss::packed;
use crate::engine::rpg_maker::text;
use crate::engine::{Offer, sheet, symbolic};
use regex::Regex;
use serde_json::Value;
use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Range;
use std::sync::LazyLock;

pub const NAME: &str = "Scripts";
const VOCABULARY: &str = "Vocab";
const FONT: &str = "FONT";
const A_SENTENCE: usize = 8;

static RE_FORMAT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"%[-+ #0]*\d*(?:\.\d+)?[A-Za-z]").expect("RE_FORMAT is a valid pattern")
});

static RE_DECLARES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^[ \t]*(?:module|class)[ \t]+([A-Za-z_][\w:]*)")
        .expect("RE_DECLARES is a valid pattern")
});

static RE_TABLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^[ \t]*([A-Z][A-Za-z0-9_]*)[ \t]*=[ \t]*(\[|\{)")
        .expect("RE_TABLE is a valid pattern")
});

static RE_TERM: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?m)^[ \t]*([A-Z][A-Za-z0-9_]*)[ \t]*=[ \t]*(?:"((?:[^"\\\n]|\\.)*)"|'((?:[^'\\\n]|\\.)*)')"#)
        .expect("RE_TERM is a valid pattern")
});

struct Term {
    named: String,
    at: Range<usize>,
    quote: char,
    text: String,
    in_list: bool,
}

struct Vouched {
    term: Term,
    listed: bool,
}

fn tables(source: &str) -> Vec<Term> {
    let mut found = Vec::new();

    for head in RE_TABLE.captures_iter(source) {
        let named = head[1].to_string();
        let bytes = source.as_bytes();
        let mut holders: Vec<u8> = Vec::new();
        let mut at = head.get(2).expect("a bracket").start();
        let mut which = 0;

        while at < bytes.len() {
            match bytes[at] {
                b'"' | b'\'' => {
                    let quote = bytes[at] as char;
                    let opened = at + 1;
                    let mut shut = opened;
                    while shut < bytes.len() && bytes[shut] != quote as u8 {
                        shut += if bytes[shut] == b'\\' { 2 } else { 1 };
                    }
                    if shut >= bytes.len() {
                        break;
                    }

                    let text = unescaped(&source[opened..shut], quote);
                    if worth(&text) {
                        found.push(Term {
                            named: format!("{named}/{which}"),
                            at: opened..shut,
                            quote,
                            text,
                            in_list: holders.iter().all(|one| *one == b'['),
                        });
                    }
                    which += 1;
                    at = shut + 1;
                }
                b'#' => {
                    at = source[at..]
                        .find('\n')
                        .map_or(bytes.len(), |step| at + step)
                }
                b'[' | b'{' => {
                    holders.push(bytes[at]);
                    at += 1;
                }
                b']' | b'}' => {
                    holders.pop();
                    if holders.is_empty() {
                        break;
                    }
                    at += 1;
                }
                _ => at += 1,
            }
        }
    }

    found
}

pub fn lines_of(bytes: &[u8], known: &HashSet<String>) -> Result<Vec<sheet::Line>, String> {
    let held = marshal::read(bytes)?;
    let mut lines = Vec::new();

    for (which, named, source) in scripts(&held, bytes)? {
        for vouched in said_in(&named, &source) {
            let named_something = names_something(&vouched.term.text, known);

            lines.push(sheet::Line {
                spot: format!("{which}/{}", vouched.term.named),
                offer: Offer::default().or_listed(vouched.listed || named_something),
                said: vouched.term.text,
            });
        }
    }

    Ok(lines)
}

fn vocab_only(named: &str, source: &str) -> bool {
    let bare = source
        .lines()
        .map(|line| line.split('#').next().unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n");

    let mut declared = RE_DECLARES
        .captures_iter(&bare)
        .map(|found| found[1].to_string());

    match declared.next() {
        None => named.trim() == VOCABULARY,
        Some(first) => first == VOCABULARY && declared.all(|one| one == VOCABULARY),
    }
}

fn reads_as_prose(term: &Term) -> bool {
    let said = term.text.trim();
    let field = term.named.split('/').next().unwrap_or_default();

    !field.contains(FONT)
        && (said.contains(' ') || (!said.is_ascii() && said.chars().count() >= A_SENTENCE))
}

fn said_in(named: &str, source: &str) -> Vec<Vouched> {
    let vocabulary = vocab_only(named, source);

    let mut found: Vec<Vouched> = tables(source)
        .into_iter()
        .map(|term| {
            let vouched = vocabulary || (term.in_list && reads_as_prose(&term));

            Vouched {
                term,
                listed: !vouched,
            }
        })
        .collect();

    found.extend(terms(source).into_iter().map(|term| Vouched {
        term,
        listed: !vocabulary,
    }));

    let mut seen: HashMap<&str, u32> = HashMap::new();
    for vouched in &found {
        *seen.entry(&vouched.term.named).or_default() += 1;
    }
    let alone: HashSet<String> = seen
        .into_iter()
        .filter(|(_, times)| *times == 1)
        .map(|(named, _)| named.to_string())
        .collect();

    found.retain(|vouched| alone.contains(&vouched.term.named));

    found
}

fn scripts(sheet: &marshal::Sheet, bytes: &[u8]) -> Result<Vec<(usize, String, String)>, String> {
    let listed = sheet
        .view
        .as_array()
        .ok_or_else(|| "the script list is not a list".to_string())?;

    let mut found = Vec::new();

    for (which, one) in listed.iter().enumerate() {
        let named = one.get(1).and_then(Value::as_str).unwrap_or_default();

        let Some(text) = held(sheet, which) else {
            continue;
        };

        let Ok(source) = inflated(&bytes[text.at.clone()]) else {
            continue;
        };

        found.push((which, named.to_string(), source));
    }

    Ok(found)
}

fn held(sheet: &marshal::Sheet, which: usize) -> Option<&Text> {
    sheet
        .texts
        .iter()
        .find(|text| text.path == [Step::Index(which), Step::Index(2)])
}

pub fn rewritten(
    bytes: &[u8],
    mut edit: impl FnMut(usize, &str, &str) -> Option<String>,
) -> Result<(Vec<u8>, u32), String> {
    let sheet = marshal::read(bytes)?;
    let mut edits: Vec<(&Text, Vec<u8>)> = Vec::new();

    for (which, named, source) in scripts(&sheet, bytes)? {
        let Some(fresh) = edit(which, &named, &source).filter(|fresh| fresh != &source) else {
            continue;
        };

        let Some(text) = held(&sheet, which) else {
            continue;
        };

        edits.push((text, deflated(&fresh)?));
    }

    let written = edits.len() as u32;

    Ok((marshal::spliced(bytes, &edits), written))
}

pub fn spliced(bytes: &[u8], said: &BTreeMap<String, String>) -> Result<(Vec<u8>, u32), String> {
    rewritten(bytes, |which, named, source| {
        let mut fresh = source.to_string();
        let mut taken: Vec<Term> = said_in(named, source)
            .into_iter()
            .map(|vouched| vouched.term)
            .collect();
        taken.sort_by_key(|term| Reverse(term.at.start));

        for term in taken {
            let Some(now) = said.get(&format!("{which}/{}", term.named)) else {
                continue;
            };
            if now == &term.text {
                continue;
            }

            fresh.replace_range(term.at, &escaped(now, term.quote));
        }

        Some(fresh)
    })
}

pub fn listed(bytes: &[u8]) -> Result<Vec<(usize, String, String)>, String> {
    let sheet = marshal::read(bytes)?;

    scripts(&sheet, bytes)
}

pub fn sources(bytes: &[u8]) -> Result<Vec<(String, String)>, String> {
    Ok(listed(bytes)?
        .into_iter()
        .map(|(_, named, source)| (named, source))
        .collect())
}

fn terms(source: &str) -> Vec<Term> {
    RE_TERM
        .captures_iter(source)
        .filter_map(|found| {
            let (body, quote) = match found.get(2) {
                Some(body) => (body, '"'),
                None => (found.get(3)?, '\''),
            };

            let text = unescaped(body.as_str(), quote);

            worth(&text).then(|| Term {
                named: found[1].to_string(),
                at: body.range(),
                quote,
                text,
                in_list: false,
            })
        })
        .collect()
}

fn unescaped(body: &str, quote: char) -> String {
    let mut out = String::with_capacity(body.len());
    let mut letters = body.chars();

    while let Some(letter) = letters.next() {
        if letter != '\\' {
            out.push(letter);
            continue;
        }

        match letters.next() {
            Some('n') if quote == '"' => out.push('\n'),
            Some('t') if quote == '"' => out.push('\t'),
            Some(next) if quote == '\'' && next != '\'' && next != '\\' => {
                out.push('\\');
                out.push(next);
            }
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }

    out
}

fn escaped(said: &str, quote: char) -> String {
    let mut out = String::with_capacity(said.len() + 8);

    for letter in said.chars() {
        match letter {
            '\\' => out.push_str("\\\\"),
            '\n' if quote == '"' => out.push_str("\\n"),
            '\t' if quote == '"' => out.push_str("\\t"),
            other if other == quote => {
                out.push('\\');
                out.push(other);
            }
            other => out.push(other),
        }
    }

    out
}

fn names_something(said: &str, known: &HashSet<String>) -> bool {
    known.contains(said) || symbolic(said)
}

fn worth(said: &str) -> bool {
    text::has_words(&RE_FORMAT.replace_all(said, ""))
}

fn inflated(body: &[u8]) -> Result<String, String> {
    String::from_utf8(packed::opened(body)?).map_err(|why| format!("a script is not utf-8: {why}"))
}

fn deflated(source: &str) -> Result<Vec<u8>, String> {
    packed::shut(source.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::rpg_maker::rgss::fixture;

    fn nothing() -> HashSet<String> {
        HashSet::new()
    }

    fn sifted(raw: &[u8], listed: bool) -> Vec<(String, String)> {
        lines_of(raw, &nothing())
            .expect("its lines")
            .into_iter()
            .filter(|line| (line.offer != Offer::Asked) == listed)
            .map(|line| (line.spot, line.said))
            .collect()
    }

    fn offered(raw: &[u8]) -> Vec<(String, String)> {
        sifted(raw, false)
    }

    fn as_symbols(raw: &[u8]) -> Vec<(String, String)> {
        sifted(raw, true)
    }

    const VOCAB: &str = "module Vocab\n  ShopBuy = \"Buy\"\n  ExpTotal    = \"Total EXP: \"\n  \
                         PartyName = \"%s's Party\"\n  Leave = 'Bye'\n  \
                         Format = \"%3d:%02d\"\n  lower = \"not a term\"\nend\n";

    const BATTLE: &str = "module N03\n  FLOOR1_DATA = {\n  \
                          \"Sea_ex01\"       => [ [ 0, 120], [ 150, 150], false, -1],\n  \
                          \"\u{5168}Battlebacks1\" => [ [ 0, 120], [ 150, 150], true, 0],\n  }\n  \
                          ACTION = {\n  \
                          \"\u{6226}\u{95d8}\u{524d}\u{306e}\u{5473}\u{65b9}\u{914d}\u{7f6e}\" => \
                          [\"move\", -7, 180, [0, 0], \"\u{901a}\u{5e38}\u{79fb}\u{52d5}\"],\n  }\n\
                          end\n";

    #[test]
    fn a_lookup_table_is_never_offered_however_much_it_reads_like_prose() {
        let raw = fixture::scripts(&[("Vocab", VOCAB), ("SB\u{ff1a}\u{5168}\u{4f53}", BATTLE)]);

        let asked = offered(&raw);
        let held = as_symbols(&raw);

        assert!(
            asked.iter().all(|(at, _)| at.starts_with("0/")),
            "a hash is addressed by what is written in it, so no model may be asked for one: \
             {asked:?}"
        );
        assert!(
            held.iter().any(|(_, said)| said.contains("Battlebacks1")),
            "the key still reaches the reader, who may translate it knowing what it costs: \
             {held:?}"
        );
    }

    #[test]
    fn a_constant_assigned_twice_is_offered_for_neither_string() {
        let raw = fixture::scripts(&[(
            "Vocab",
            "module Vocab\n  Greet = \"Good morning\"\n  Greet = \"Good evening\"\n  \
             Leave = \"Goodbye\"\nend\n",
        )]);

        assert_eq!(
            offered(&raw),
            vec![("0/Leave".to_string(), "Goodbye".to_string())],
            "one spot naming two strings could only splice the same answer into both"
        );
        assert!(
            as_symbols(&raw).is_empty(),
            "and the pair may not sneak back in as a symbol"
        );
    }

    #[test]
    fn a_list_of_records_offers_its_prose_from_any_script() {
        let raw = fixture::scripts(&[(
            "\u{901a}\u{5e38}\u{30df}\u{30c3}\u{30b7}\u{30e7}\u{30f3}",
            "module SET_MISSION\n  MS_SETTING01 = [\n  \
             [101, \"Eliminate the Imperial Hideout\", 1, 200],\n  \
             [102, \"Assassinate the Monster Tamer\", 2, 300],\n  ]\nend\n",
        )]);

        assert_eq!(
            offered(&raw),
            vec![
                (
                    "0/MS_SETTING01/0".to_string(),
                    "Eliminate the Imperial Hideout".to_string()
                ),
                (
                    "0/MS_SETTING01/1".to_string(),
                    "Assassinate the Monster Tamer".to_string()
                ),
            ],
            "a record in a list is addressed by its number, so its prose is the player's to read"
        );
    }

    #[test]
    fn a_name_a_script_looks_itself_up_by_is_marked_and_never_thrown_away() {
        let raw = fixture::scripts(&[(
            "Vocab",
            "module Vocab\n  ShopBuy = \"Buy\"\n  Walk = 'ripple_walk'\nend\n",
        )]);

        assert_eq!(
            offered(&raw),
            [("0/ShopBuy".to_string(), "Buy".to_string())]
        );
        assert_eq!(
            as_symbols(&raw),
            [("0/Walk".to_string(), "ripple_walk".to_string())],
            "a name a script looks itself up by is the reader's to see and settle by hand: a \
             line thrown away here is one nobody ever learns is in the game"
        );

        let said = BTreeMap::from([("0/Walk".to_string(), "nami_aruki".to_string())]);
        let (out, written) = spliced(&raw, &said).expect("it splices");

        assert_eq!(written, 1);
        assert!(
            sources(&out).expect("its scripts")[0]
                .1
                .contains("nami_aruki"),
            "and what the reader settled by hand has to reach the game, or marking it was a lie"
        );
    }

    #[test]
    fn a_list_of_marks_or_short_keys_holds_no_prose() {
        let raw = fixture::scripts(&[(
            "Window_NameInput",
            "class Window_NameInput\n  \
             LATIN1 = [ 'A','B','C','a','b','c' ]\n  \
             LETTER_TABLE = [' A',' B',' C']\n  \
             COST_HP_KEYS = ['\u{6d88}\u{8cbb}HP', 'CostHP']\n  \
             NUMBER_FONT = ['Arial Black', 'VL Gothic']\nend\n",
        )]);

        assert!(
            offered(&raw).is_empty(),
            "a name-entry grid, an enemy letter, a note-tag key and a font are all addressed by \
             what they spell"
        );
        assert_eq!(
            as_symbols(&raw).len(),
            13,
            "none of them is thrown away either: every one is the reader's to take on"
        );
    }

    #[test]
    fn a_script_that_holds_more_than_the_vocabulary_is_left_alone_even_where_it_reopens_it() {
        let source =
            format!("{BATTLE}\nmodule Vocab\n  ObtainGold = \"Obtained %s \\\\G!\"\nend\n");
        let raw = fixture::scripts(&[("SB\u{ff1a}\u{30a2}\u{30af}", &source)]);

        assert!(
            offered(&raw).is_empty(),
            "reopening Vocab may not make the config around it speak, and the reopened term is \
             not worth the risk of guessing which is which"
        );
    }

    #[test]
    fn a_script_holding_nothing_but_the_vocabulary_speaks_however_it_is_named() {
        let raw = fixture::scripts(&[(
            "SB\u{ff1a}\u{30d0}\u{30b0}\u{56de}\u{907f}",
            "module Vocab\n  ObtainGold = \"Obtained %s \\\\G!\"\nend\n",
        )]);

        assert_eq!(
            offered(&raw),
            vec![("0/ObtainGold".to_string(), "Obtained %s \\G!".to_string())],
            "a game patch reopening Vocab in a script of its own is the shipped shape"
        );
    }

    #[test]
    fn a_vocabulary_table_offers_its_short_terms() {
        let raw = fixture::scripts(&[(
            "Vocab",
            "module Vocab\n  BASIC = [\"Level\", \"HP\", \"MP\"]\nend\n",
        )]);

        assert_eq!(
            offered(&raw),
            vec![
                ("0/BASIC/0".to_string(), "Level".to_string()),
                ("0/BASIC/1".to_string(), "HP".to_string()),
                ("0/BASIC/2".to_string(), "MP".to_string()),
            ],
            "a term the engine draws is a term however short it is"
        );
    }

    #[test]
    fn only_the_terms_a_script_declares_as_constants_are_offered() {
        let raw = fixture::scripts(&[("Vocab", VOCAB), ("Scene_Menu", "class Scene_Menu\nend\n")]);

        assert_eq!(
            offered(&raw),
            vec![
                ("0/ShopBuy".to_string(), "Buy".to_string()),
                ("0/ExpTotal".to_string(), "Total EXP: ".to_string()),
                ("0/PartyName".to_string(), "%s's Party".to_string()),
                ("0/Leave".to_string(), "Bye".to_string()),
            ],
            "a format string holds no words and a lowercase name is a local, not a term"
        );
    }

    #[test]
    fn a_translated_term_goes_back_packed_and_the_script_still_unpacks() {
        let raw = fixture::scripts(&[("Vocab", VOCAB)]);

        let said = BTreeMap::from([
            ("0/ShopBuy".to_string(), "\u{8cb7}\u{3046}".to_string()),
            (
                "0/ExpTotal".to_string(),
                "\u{7d4c}\u{9a13}\u{5024}: ".to_string(),
            ),
        ]);

        let (fresh, written) = spliced(&raw, &said).expect("a spliced list");
        assert_eq!(written, 1, "one script was rewritten");

        assert_eq!(
            offered(&fresh),
            vec![
                ("0/ShopBuy".to_string(), "\u{8cb7}\u{3046}".to_string()),
                (
                    "0/ExpTotal".to_string(),
                    "\u{7d4c}\u{9a13}\u{5024}: ".to_string()
                ),
                ("0/PartyName".to_string(), "%s's Party".to_string()),
                ("0/Leave".to_string(), "Bye".to_string()),
            ],
            "the keys hold still because they are the constant names, not offsets"
        );
    }

    #[test]
    fn a_single_quoted_term_reads_and_writes_its_backslashes_as_the_game_shows_them() {
        assert_eq!(unescaped(r"Wait\nhere", '\''), r"Wait\nhere");
        assert_eq!(unescaped(r"Wait\nhere", '"'), "Wait\nhere");
        assert_eq!(unescaped(r"It\'s", '\''), "It's");
        assert_eq!(escaped(r"Wait\nhere", '\''), r"Wait\\nhere");
        assert_eq!(escaped("Wait\nhere", '\''), "Wait\nhere");
    }

    #[test]
    fn a_quote_or_a_control_code_is_escaped_so_the_literal_still_closes_where_it_did() {
        let raw = fixture::scripts(&[(
            "Vocab",
            "module Vocab\n  Obtain = \"Obtained %s \\\\G!\"\n  Leave = 'Bye'\nend\n",
        )]);

        assert_eq!(
            offered(&raw),
            vec![
                ("0/Obtain".to_string(), "Obtained %s \\G!".to_string()),
                ("0/Leave".to_string(), "Bye".to_string()),
            ],
            "a term reads as the game prints it, not as ruby spells it"
        );

        let said = BTreeMap::from([
            ("0/Obtain".to_string(), "%s \\G を入手！".to_string()),
            ("0/Leave".to_string(), "it's over".to_string()),
        ]);

        let (fresh, written) = spliced(&raw, &said).expect("a spliced list");
        assert_eq!(written, 1);

        assert_eq!(
            offered(&fresh),
            vec![
                ("0/Obtain".to_string(), "%s \\G を入手！".to_string()),
                ("0/Leave".to_string(), "it's over".to_string()),
            ],
            "a control code the game needs survives the trip"
        );

        let after = marshal::read(&fresh).expect("a list");
        let source = inflated(&fresh[after.texts[1].at.clone()]).expect("it unpacks");

        assert!(
            source.contains(r#"Obtain = "%s \\G を入手！""#),
            "the backslash has to be doubled in the ruby source: {source}"
        );
        assert!(
            source.contains(r"Leave = 'it\'s over'"),
            "the apostrophe has to be escaped inside a single quoted term: {source}"
        );
    }

    #[test]
    fn a_script_list_with_no_vocabulary_in_it_asks_for_nothing() {
        let raw = fixture::scripts(&[("Scene_Map", "class Scene_Map\n  NAME = \"map\"\nend\n")]);

        assert!(
            offered(&raw).is_empty(),
            "a lone constant in a script of its own could be a key, so no model is asked for it"
        );
        assert_eq!(
            as_symbols(&raw),
            vec![("0/NAME".to_string(), "map".to_string())],
            "it is still there to be taken on by hand"
        );
    }
}
