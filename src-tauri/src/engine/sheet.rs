use crate::engine::{Applied, Install, Offer, Parsed, TranslationUnit};
use crate::walk;
use anyhow::{Context, Result};
use serde_json::{Map, Value};
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::path::Path;

pub const SUFFIX: &str = "sheet";

const SAID: &str = "text";
const OFFER: &str = "offer";
const LISTED: &str = "listed";
const LOCKED: &str = "locked";

pub fn wants(path: &Path) -> bool {
    path.extension()
        .is_some_and(|kind| kind.eq_ignore_ascii_case(SUFFIX))
}

pub fn shown(name: &str) -> Cow<'_, str> {
    match name.rsplit_once('.') {
        Some((bare, kind)) if kind.eq_ignore_ascii_case(SUFFIX) => Cow::Borrowed(bare),
        _ => Cow::Borrowed(name),
    }
}

pub async fn staged(
    at: &Install<'_>,
    keyed: impl Fn(&Path) -> Option<String>,
    known: impl Fn(&str) -> bool,
) -> Result<BTreeMap<String, BTreeMap<String, String>>> {
    let mut held = BTreeMap::new();

    for one in walk::files(at.staged).await {
        let Some(named) = keyed(&one) else {
            continue;
        };

        if !known(&named) {
            at.progress.warn(
                at.doing,
                &format!("{named} is no longer in the game, so its lines stay out"),
            );
            continue;
        }

        let body = tokio::fs::read_to_string(&one)
            .await
            .with_context(|| format!("reading {}", one.display()))?;

        held.insert(named, lines(&body)?);
    }

    Ok(held)
}

pub struct Line {
    pub spot: String,
    pub said: String,
    pub offer: Offer,
}

impl Line {
    pub fn read(spot: String, said: String) -> Self {
        Self {
            spot,
            said,
            offer: Offer::Asked,
        }
    }
}

fn offer_of(held: &Map<String, Value>) -> Offer {
    match held.get(OFFER).and_then(Value::as_str) {
        Some(LISTED) => Offer::Listed,
        Some(LOCKED) => Offer::Locked,
        _ => Offer::Asked,
    }
}

fn said_of(body: &Value) -> Option<(String, Offer)> {
    match body {
        Value::String(said) => Some((said.clone(), Offer::Asked)),
        Value::Object(held) => Some((held.get(SAID)?.as_str()?.to_string(), offer_of(held))),
        _ => None,
    }
}

pub fn write(lines: impl IntoIterator<Item = (String, String)>) -> Result<String> {
    page(lines.into_iter().map(|(spot, said)| Line::read(spot, said)))
}

pub fn page(lines: impl IntoIterator<Item = Line>) -> Result<String> {
    let page: Map<String, Value> = lines
        .into_iter()
        .map(|line| {
            let named = match line.offer {
                Offer::Asked => None,
                Offer::Listed => Some(LISTED),
                Offer::Locked => Some(LOCKED),
            };

            let body = match named {
                None => Value::String(line.said),
                Some(named) => Value::Object(Map::from_iter([
                    (SAID.to_string(), Value::String(line.said)),
                    (OFFER.to_string(), Value::String(named.to_string())),
                ])),
            };

            (line.spot, body)
        })
        .collect();

    Ok(serde_json::to_string_pretty(&Value::Object(page))?)
}

pub fn lines(text: &str) -> Result<BTreeMap<String, String>> {
    Ok(serde_json::from_str::<Map<String, Value>>(text)
        .context("this sheet is not the JSON it was written as")?
        .into_iter()
        .filter_map(|(spot, body)| Some((spot, said_of(&body)?.0)))
        .collect())
}

struct Row {
    spot: String,
    offer: Offer,
}

pub struct Sheet {
    rows: Vec<Row>,
    units: Vec<TranslationUnit>,
}

pub fn read(text: &str, also_listed: impl Fn(&str) -> bool) -> Sheet {
    let page: Map<String, Value> = serde_json::from_str(text).unwrap_or_default();

    let mut rows = Vec::with_capacity(page.len());
    let mut units = Vec::with_capacity(page.len());

    for (spot, body) in page {
        let Some((said, offer)) = said_of(&body) else {
            continue;
        };

        units.push(TranslationUnit {
            id: units.len() as u32,
            offer: offer.or_listed(also_listed(&said)),
            text: said,
        });
        rows.push(Row { spot, offer });
    }

    Sheet { rows, units }
}

impl Parsed for Sheet {
    fn units(&self) -> &[TranslationUnit] {
        &self.units
    }

    fn render(self: Box<Self>, translations: &BTreeMap<u32, String>) -> Result<(String, Applied)> {
        let this = *self;
        let mut lines = Vec::with_capacity(this.units.len());
        let mut applied = Applied::default();

        for (unit, row) in this.units.into_iter().zip(this.rows) {
            let said = match unit.answer(translations) {
                Some(done) => {
                    applied.lines += 1;
                    done.clone()
                }
                None => unit.text,
            };

            lines.push(Line {
                spot: row.spot,
                said,
                offer: row.offer,
            });
        }

        Ok((page(lines)?, applied))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_the_format_itself_locks_never_carries_a_translation_out() {
        let held = page(vec![
            Line {
                spot: "0/note".to_string(),
                said: "Hello".to_string(),
                offer: Offer::Asked,
            },
            Line {
                spot: "0/eval".to_string(),
                said: "const x = 20;".to_string(),
                offer: Offer::Locked,
            },
        ])
        .expect("a sheet");

        let sheet = Box::new(read(&held, |_| false));
        let said = BTreeMap::from([
            (0, "the greeting".to_string()),
            (1, "a hand written script".to_string()),
        ]);

        let (out, applied) = sheet.render(&said).expect("a rendered sheet");
        let rows = lines(&out).expect("its lines");

        assert_eq!(
            rows.get("0/eval").map(String::as_str),
            Some("const x = 20;"),
            "the plugin itself declares this parameter as code, so not even a line somebody \
             settled by hand may reach it: what a reader types there is a guess, and a game that \
             no longer runs is not a translation"
        );
        assert_eq!(rows.get("0/note").map(String::as_str), Some("the greeting"));
        assert_eq!(
            applied.lines, 1,
            "and the count says one line went in, not two"
        );
    }

    #[test]
    fn the_order_a_file_lists_its_lines_in_is_the_order_they_are_offered_in() {
        let unsorted = write([
            ("9".to_string(), "written first".to_string()),
            ("1".to_string(), "written last".to_string()),
        ])
        .unwrap();
        assert_eq!(
            read(&unsorted, |_| false)
                .rows
                .iter()
                .map(|row| row.spot.as_str())
                .collect::<Vec<_>>(),
            ["9", "1"],
            "file order is the unit order even when the keys would sort the other way"
        );
    }

    #[test]
    fn a_sheet_keeps_every_line_where_it_was_found() {
        let rows = vec![
            ("list/1/parameters/0".to_string(), "The door is".to_string()),
            ("1/name".to_string(), "Hanako".to_string()),
        ];

        let page = write(rows).expect("a sheet");
        let sheet = read(&page, |_| false);

        assert_eq!(sheet.units().len(), 2);

        let which = sheet
            .units()
            .iter()
            .find(|unit| unit.text == "Hanako")
            .expect("the name is one of the lines")
            .id;

        let done = BTreeMap::from([(which, "\u{82b1}\u{5b50}".to_string())]);
        let (written, applied) = Box::new(read(&page, |_| false))
            .render(&done)
            .expect("renders");

        assert_eq!(applied.lines, 1);

        let back = lines(&written).expect("the rendered sheet reads");
        assert_eq!(back["1/name"], "\u{82b1}\u{5b50}");
        assert_eq!(
            back["list/1/parameters/0"], "The door is",
            "a line nobody translated keeps the words the game shipped"
        );
    }

    #[test]
    fn a_sheet_that_is_not_json_offers_nothing_rather_than_panicking() {
        for broken in ["not json at all", "half written {"] {
            assert!(read(broken, |_| false).units().is_empty());
            assert!(lines(broken).is_err());
        }
    }
}
