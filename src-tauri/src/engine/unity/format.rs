use crate::engine::unicode_escape;
use crate::engine::unity::naming;
use anyhow::{Result, bail};
use std::collections::BTreeMap;
use std::iter;

const KEY: &str = "key";
const LINE: &str = "line";
const HEADING: &str = "heading";

const JSON: &str = "json";
const TABLE: &str = "table";
const XML: &str = "xml";
const TEXT: &str = "text";

const LONGEST_MARK: usize = 32;

pub struct Piece {
    pub path: String,
    pub stem: String,
    pub text: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Wrap {
    Bare,
    Cell(char),
    Json,
    Xml,
    CData,
}

struct Span {
    from: usize,
    to: usize,
    wrap: Wrap,
    path: String,
    stem: String,
    text: String,
}

pub fn pieces(text: &str) -> Vec<Piece> {
    spans_in(text)
        .into_iter()
        .map(|one| Piece {
            path: one.path,
            stem: one.stem,
            text: one.text,
        })
        .collect()
}

pub fn put_back(text: &str, lines: &BTreeMap<String, String>) -> Result<String> {
    let spans = spans_in(text);
    let mut out = String::with_capacity(text.len());
    let mut cut = 0;

    for one in &spans {
        if one.from < cut {
            bail!("{} overlaps the piece before it", one.path);
        }

        out.push_str(&text[cut..one.from]);

        match lines.get(&one.path).filter(|done| **done != one.text) {
            Some(done) => match one.wrap {
                Wrap::Bare => out.push_str(done),
                Wrap::Cell(apart) => out.push_str(&as_cell(done, apart)),
                Wrap::Json => out.push_str(&as_json(done)),
                Wrap::Xml => out.push_str(&as_xml(done, &text[one.from..one.to])),
                Wrap::CData => out.push_str(&done.replace("]]>", "]]]]><![CDATA[>")),
            },
            None => out.push_str(&text[one.from..one.to]),
        }

        cut = one.to;
    }

    out.push_str(&text[cut..]);

    Ok(out)
}

fn spans_in(text: &str) -> Vec<Span> {
    let (shape, mut spans) = curly(text)
        .map(|spans| (JSON, spans))
        .or_else(|| table(text).map(|spans| (TABLE, spans)))
        .or_else(|| angled(text).map(|spans| (XML, spans)))
        .unwrap_or_else(|| (TEXT, lines_of(text)));

    for one in &mut spans {
        one.stem = format!("{shape}.{}", one.stem);
    }

    let mut seen: BTreeMap<String, u32> = BTreeMap::new();
    for one in &spans {
        *seen.entry(one.path.clone()).or_default() += 1;
    }
    seen.retain(|_, times| *times > 1);

    let mut nth: BTreeMap<String, u32> = BTreeMap::new();
    for one in &mut spans {
        if seen.contains_key(&one.path) {
            let which = nth.entry(one.path.clone()).or_default();
            one.path = format!("{}#{which}", one.path);
            *which += 1;
        }
    }

    spans
}

fn as_cell(text: &str, apart: char) -> String {
    let awkward =
        text.contains(apart) || text.contains('"') || text.contains('\n') || text.contains('\r');

    if !awkward {
        return text.to_string();
    }

    format!("\"{}\"", text.replace('"', "\"\""))
}

fn as_json(text: &str) -> String {
    let mut out = String::with_capacity(text.len());

    for letter in text.chars() {
        match letter {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ if letter.is_control() => out.push_str(&format!("\\u{:04x}", letter as u32)),
            _ => out.push(letter),
        }
    }

    out
}

fn from_json(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut walk = text.chars();

    while let Some(letter) = walk.next() {
        if letter != '\\' {
            out.push(letter);
            continue;
        }

        match walk.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('b') => out.push('\u{8}'),
            Some('f') => out.push('\u{c}'),
            Some('u') => match unicode_escape(walk.as_str()) {
                Some((found, taken)) => {
                    out.push(found);
                    walk.by_ref().nth(taken - 1);
                }
                None => {
                    let digits: String = walk.by_ref().take(4).collect();
                    out.push_str("\\u");
                    out.push_str(&digits);
                }
            },
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }

    out
}

fn flat(from: usize, to: usize, stem: &str, which: usize, text: &str) -> Span {
    Span {
        from,
        to,
        wrap: Wrap::Bare,
        path: format!("{stem}[{which}]"),
        stem: stem.to_string(),
        text: text.to_string(),
    }
}

fn lines_of(text: &str) -> Vec<Span> {
    let mut out = Vec::new();
    let mut at = 0;

    for line in text.split_inclusive('\n') {
        let said = line.trim_end_matches(['\n', '\r']);

        if !said.is_empty() {
            out.push(flat(at, at + said.len(), LINE, out.len(), said));
        }

        at += line.len();
    }

    out
}

fn angled(text: &str) -> Option<Vec<Span>> {
    if !text.trim_start().starts_with('<') {
        return None;
    }

    let raw = text.as_bytes();
    let mut out: Vec<Span> = Vec::new();
    let mut counted: BTreeMap<&str, usize> = BTreeMap::new();
    let mut open: Vec<&str> = Vec::new();
    let mut tags = 0;
    let mut at = 0;

    while let Some(step) = raw[at..].iter().position(|byte| *byte == b'<') {
        let mark = at + step;

        if let Some(&name) = open.last() {
            let body = &text[at..mark];
            let said = body.trim();

            if !said.is_empty() {
                let lead = at + (body.len() - body.trim_start().len());
                let which = counted.entry(name).or_insert(0);

                out.push(Span {
                    from: lead,
                    to: lead + said.len(),
                    wrap: Wrap::Xml,
                    path: format!("{name}[{which}]"),
                    stem: name.to_string(),
                    text: from_xml(said),
                });
                *which += 1;
            }

            if let Some(body) = text[mark..].strip_prefix("<![CDATA[")
                && let Some(end) = body.find("]]>")
                && !body[..end].trim().is_empty()
            {
                let from = mark + "<![CDATA[".len();
                let which = counted.entry(name).or_insert(0);

                out.push(Span {
                    from,
                    to: from + end,
                    wrap: Wrap::CData,
                    path: format!("{name}[{which}]"),
                    stem: name.to_string(),
                    text: body[..end].to_string(),
                });
                *which += 1;
            }
        }

        at = past_tag(text, mark, &mut open, &mut tags)?;
    }

    (tags > 0 && open.is_empty()).then_some(out)
}

fn past_tag<'t>(
    text: &'t str,
    mark: usize,
    open: &mut Vec<&'t str>,
    tags: &mut usize,
) -> Option<usize> {
    let rest = &text[mark..];

    for (head, tail) in [("<!--", "-->"), ("<![CDATA[", "]]>"), ("<?", "?>")] {
        if let Some(body) = rest.strip_prefix(head) {
            let end = body.find(tail)?;
            return Some(mark + head.len() + end + tail.len());
        }
    }

    if rest.starts_with("<!") {
        return Some(mark + rest.find('>')? + 1);
    }

    let end = tag_end(rest)?;
    let inside = &rest[1..end];
    let past = mark + end + 1;

    if let Some(closing) = inside.strip_prefix('/') {
        if open.last() != Some(&closing.trim()) {
            return None;
        }

        open.pop();
        return Some(past);
    }

    *tags += 1;

    if inside.ends_with('/') {
        return Some(past);
    }

    open.push(inside.split_whitespace().next()?);

    Some(past)
}

fn tag_end(rest: &str) -> Option<usize> {
    let mut quote = None;

    for (at, one) in rest.char_indices().skip(1) {
        match (quote, one) {
            (None, '"' | '\'') => quote = Some(one),
            (Some(open), _) if one == open => quote = None,
            (None, '>') => return Some(at),
            _ => {}
        }
    }

    None
}

fn as_xml(text: &str, shipped: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(at) = rest.find(['&', '<', '>']) {
        out.push_str(&rest[..at]);
        let after = &rest[at..];

        match after.as_bytes()[0] {
            b'<' => {
                out.push_str("&lt;");
                rest = &after[1..];
            }
            b'>' => {
                out.push_str("&gt;");
                rest = &after[1..];
            }
            _ => match reference_at(after).filter(|one| shipped.contains(*one)) {
                Some(reference) => {
                    out.push_str(reference);
                    rest = &after[reference.len()..];
                }
                None => {
                    out.push_str("&amp;");
                    rest = &after[1..];
                }
            },
        }
    }

    out.push_str(rest);
    out
}

fn reference_at(text: &str) -> Option<&str> {
    let rest = text.strip_prefix('&')?;
    let end = rest
        .char_indices()
        .take(LONGEST_MARK)
        .find(|(_, one)| *one == ';')?
        .0;
    let name = &rest[..end];

    let shaped = match name.strip_prefix('#') {
        Some(number) => match number.strip_prefix(['x', 'X']) {
            Some(hex) => !hex.is_empty() && hex.chars().all(|one| one.is_ascii_hexdigit()),
            None => !number.is_empty() && number.chars().all(|one| one.is_ascii_digit()),
        },
        None => {
            name.starts_with(|one: char| one.is_ascii_alphabetic())
                && name.chars().all(|one| one.is_ascii_alphanumeric())
        }
    };

    shaped.then(|| &text[..end + 2])
}

fn from_xml(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;

    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let after = &rest[at..];

        match after
            .char_indices()
            .take(LONGEST_MARK)
            .find(|(_, one)| *one == ';')
            .and_then(|(end, _)| named_mark(&after[1..end]).map(|found| (found, end)))
        {
            Some((found, end)) => {
                out.push(found);
                rest = &after[end + 1..];
            }
            None => {
                out.push('&');
                rest = &after[1..];
            }
        }
    }

    out.push_str(rest);
    out
}

fn named_mark(mark: &str) -> Option<char> {
    match mark {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => {
            let mark = mark.strip_prefix('#')?;
            let code = match mark.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => mark.parse().ok()?,
            };

            char::from_u32(code)
        }
    }
}

enum Frame {
    Object(Option<String>),
    Array(usize),
}

fn curly(text: &str) -> Option<Vec<Span>> {
    let head = text.trim_start();
    if !head.starts_with('{') && !head.starts_with('[') {
        return None;
    }

    let raw = text.as_bytes();
    let mut out: Vec<Span> = Vec::new();
    let mut stack: Vec<Frame> = Vec::new();
    let mut keys = 0;
    let mut at = 0;

    while at < raw.len() {
        match raw[at] {
            b'{' => {
                stack.push(Frame::Object(None));
                at += 1;
            }
            b'[' => {
                stack.push(Frame::Array(0));
                at += 1;
            }
            b'}' | b']' => {
                stack.pop();
                at += 1;
            }
            b',' => {
                match stack.last_mut() {
                    Some(Frame::Object(key)) => *key = None,
                    Some(Frame::Array(which)) => *which += 1,
                    None => {}
                }
                at += 1;
            }
            b'"' => {
                let from = at + 1;
                let mut walk = from;
                while walk < raw.len() {
                    match raw[walk] {
                        b'\\' => walk += 2,
                        b'"' => break,
                        _ => walk += 1,
                    }
                }
                if walk >= raw.len() || !text.is_char_boundary(from) || !text.is_char_boundary(walk)
                {
                    return None;
                }

                let inside = &text[from..walk];
                let after = raw[walk + 1..]
                    .iter()
                    .position(|byte| !byte.is_ascii_whitespace())
                    .map(|step| raw[walk + 1 + step]);

                let naming = after == Some(b':') && matches!(stack.last(), Some(Frame::Object(_)));

                if naming {
                    if !inside.is_empty() {
                        let mut one = flat(from, walk, KEY, keys, &from_json(inside));
                        one.wrap = Wrap::Json;
                        out.push(one);
                    }
                    keys += 1;
                    if let Some(Frame::Object(key)) = stack.last_mut() {
                        *key = Some(from_json(inside));
                    }
                } else if !inside.is_empty() {
                    let path = pointing(&stack);
                    out.push(Span {
                        from,
                        to: walk,
                        wrap: Wrap::Json,
                        stem: naming::leaf(&path),
                        path,
                        text: from_json(inside),
                    });
                }

                at = walk + 1;
            }
            _ => at += 1,
        }
    }

    (!out.is_empty()).then_some(out)
}

fn pointing(stack: &[Frame]) -> String {
    let mut out = String::new();

    for frame in stack {
        match frame {
            Frame::Object(Some(key)) => {
                if !out.is_empty() {
                    out.push('.');
                }
                out.push_str(key);
            }
            Frame::Object(None) => {}
            Frame::Array(which) => out.push_str(&format!("[{which}]")),
        }
    }

    if out.is_empty() {
        "value".to_string()
    } else {
        out
    }
}

#[derive(Default)]
struct Quotes {
    inside: bool,
    used: bool,
    closed: bool,
}

impl Quotes {
    fn step(&mut self, letter: char) -> bool {
        let opens = !self.used || self.closed;
        self.used = true;
        self.closed = false;

        if letter == '"' {
            match self.inside {
                true => {
                    self.inside = false;
                    self.closed = true;
                }
                false => self.inside = opens,
            }
        }

        !self.inside
    }

    fn apart(&mut self) {
        self.used = false;
        self.closed = false;
    }
}

fn cells(said: &str, apart: char) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut from = 0;
    let mut quotes = Quotes::default();
    let mut walk = 0;

    for letter in said.chars() {
        if quotes.step(letter) && letter == apart {
            out.push((from, walk));
            from = walk + letter.len_utf8();
            quotes.apart();
        }

        walk += letter.len_utf8();
    }
    out.push((from, walk));

    out
}

fn bare(cell: &str) -> String {
    let quoted = cell.len() >= 2 && cell.starts_with('"') && cell.ends_with('"');

    if quoted {
        cell[1..cell.len() - 1].replace("\"\"", "\"")
    } else {
        cell.to_string()
    }
}

fn rows_of(text: &str, apart: char) -> Vec<&str> {
    let mut out = Vec::new();
    let mut quotes = Quotes::default();
    let mut from = 0;

    for (at, letter) in text.char_indices() {
        if !quotes.step(letter) {
            continue;
        }

        match letter {
            '\n' => {
                out.push(&text[from..=at]);
                from = at + 1;
                quotes.apart();
            }
            _ if letter == apart => quotes.apart(),
            _ => {}
        }
    }

    if from < text.len() {
        out.push(&text[from..]);
    }

    out
}

fn table(text: &str) -> Option<Vec<Span>> {
    let apart = match text.split('\n').next()?.contains('\t') {
        true => '\t',
        false => ',',
    };

    let mut lines = rows_of(text, apart).into_iter();
    let first = lines.next()?;

    let mut heading: Vec<String> = cells(first.trim_end_matches(['\n', '\r']), apart)
        .into_iter()
        .map(|(from, to)| bare(&first[from..to]))
        .collect();

    if heading.len() < 2 {
        return None;
    }

    let mut named: BTreeMap<String, usize> = BTreeMap::new();
    for name in &mut heading {
        let times = named.entry(name.clone()).or_insert(0);
        *times += 1;
        if *times > 1 {
            *name = format!("{name}#{times}");
        }
    }

    let mut out = Vec::new();
    let mut rows = 0;
    let mut at = 0;

    for (which, line) in iter::once(first).chain(lines).enumerate() {
        let said = line.trim_end_matches(['\n', '\r']);
        if said.is_empty() {
            at += line.len();
            continue;
        }

        let found = cells(said, apart);
        if found.len() != heading.len() {
            return None;
        }
        rows += 1;

        for (column, (from, to)) in found.into_iter().enumerate() {
            let inside = bare(&said[from..to]);
            if inside.is_empty() {
                continue;
            }

            let (path, stem) = if which == 0 {
                (format!("{HEADING}[{column}]"), HEADING.to_string())
            } else {
                let name = heading.get(column).cloned().unwrap_or_default();
                (format!("row[{}].{name}", which - 1), name)
            };

            out.push(Span {
                from: at + from,
                to: at + to,
                wrap: Wrap::Cell(apart),
                path,
                stem,
                text: inside,
            });
        }

        at += line.len();
    }

    (rows > 1).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn said(text: &str) -> Vec<String> {
        pieces(text).into_iter().map(|one| one.text).collect()
    }

    fn stems(text: &str) -> Vec<String> {
        pieces(text).into_iter().map(|one| one.stem).collect()
    }

    fn paths(text: &str) -> Vec<String> {
        pieces(text).into_iter().map(|one| one.path).collect()
    }

    fn back(text: &str) -> String {
        put_back(text, &BTreeMap::new()).expect("a render")
    }

    fn kept(text: &str) -> String {
        let lines = pieces(text)
            .into_iter()
            .map(|one| (one.path, one.text))
            .collect();

        put_back(text, &lines).expect("a render")
    }

    #[test]
    fn a_line_handed_back_exactly_as_it_was_read_leaves_the_document_alone() {
        let text = concat!(
            "<NPC>\n",
            "  <Bio>Wait&nbsp;here &amp; rest</Bio>\n",
            "  <Note>&#x0001F600; ready</Note>\n",
            "</NPC>\n"
        );

        assert_eq!(
            said(text),
            ["Wait&nbsp;here & rest", "\u{1F600} ready"],
            "a mark this reader does not know stays the text it was, and one written out long \
             still names the letter it names"
        );
        assert_eq!(
            kept(text),
            text,
            "a staged sheet holds a row for every line whether anybody translated it or not, so \
             writing one back that nobody changed has to leave the file as it was: escaping it \
             again turns &nbsp; into &amp;nbsp; and the player reads the mark instead of the space"
        );
    }

    #[test]
    fn a_translated_line_keeps_the_entity_marks_the_document_shipped() {
        let text = concat!(
            "<NPC>\n",
            "  <Bio>Wait&nbsp;here &amp; rest</Bio>\n",
            "  <Note>literally &amp;nbsp; spelled out</Note>\n",
            "</NPC>\n"
        );

        let spans = pieces(text);
        let mut lines: BTreeMap<String, String> = BTreeMap::new();
        lines.insert(
            spans[0].path.clone(),
            "ここで&nbsp;待て & 休め &broken; loose".to_string(),
        );
        lines.insert(
            spans[1].path.clone(),
            "文字通りの &nbsp; のつづり".to_string(),
        );

        let out = put_back(text, &lines).expect("a render");

        assert!(
            out.contains("ここで&nbsp;待て &amp; 休め &amp;broken; loose"),
            "a mark the document shipped rides through untouched, while a bare ampersand and a \
             mark the model invented are escaped: {out}"
        );
        assert!(
            out.contains("文字通りの &amp;nbsp; のつづり"),
            "this document spelled the mark out as text, so a translation keeping it has to \
             stay spelled out: {out}"
        );
    }

    #[test]
    fn plain_text_gives_up_every_line_under_one_name() {
        let text = "Peter\nClass was different today.\n\nWalter\nGood morning.\n\n";

        assert_eq!(
            said(text),
            [
                "Peter",
                "Class was different today.",
                "Walter",
                "Good morning."
            ]
        );
        assert_eq!(paths(text)[1], "line[1]");
        assert_eq!(
            stems(text),
            ["text.line", "text.line", "text.line", "text.line"],
            "the format leads the name, so a column called line elsewhere stays apart"
        );
        assert_eq!(back(text), text);
    }

    #[test]
    fn json_names_a_value_after_the_keys_that_lead_to_it() {
        let text = r#"{"skeleton":{"spine":"4.1.08"},"bones":[{"name":"root"},{"name":"hip"}]}"#;

        assert_eq!(
            paths(text),
            [
                "key[0]",
                "key[1]",
                "skeleton.spine",
                "key[2]",
                "key[3]",
                "bones[0].name",
                "key[4]",
                "bones[1].name",
            ]
        );
        assert_eq!(
            stems(text),
            [
                "json.key",
                "json.key",
                "json.spine",
                "json.key",
                "json.key",
                "json.name",
                "json.key",
                "json.name"
            ],
            "the last step of the path is the group; the keys in the middle are data, \
             and a map keyed by data would spawn a group per row"
        );
        assert_eq!(back(text), text);
    }

    #[test]
    fn json_that_ends_mid_string_is_read_as_plain_text() {
        assert_eq!(
            stems("{\"open\": \"never closed"),
            ["text.line"],
            "a file that only looks like JSON has to fall back to being read line by line, or \
             the reader loses every word in it to a parse error"
        );
    }

    #[test]
    fn a_table_names_a_cell_after_its_column() {
        let text = "KEY,English,Francais\nFlag,Flag-US,Flag-FR\n";

        assert_eq!(
            paths(text),
            [
                "heading[0]",
                "heading[1]",
                "heading[2]",
                "row[0].KEY",
                "row[0].English",
                "row[0].Francais",
            ]
        );
        assert_eq!(
            stems(text),
            [
                "table.heading",
                "table.heading",
                "table.heading",
                "table.KEY",
                "table.English",
                "table.Francais"
            ],
            "one group per column is what lets a reader leave the key column alone"
        );
        assert_eq!(back(text), text);
    }

    #[test]
    fn a_quoted_cell_may_hold_the_separator_and_quotes_of_its_own() {
        let text = "KEY,English\nParagraph,\"how to \"\"do\"\" it, and why\"\n";

        assert_eq!(
            said(text)[3],
            "how to \"do\" it, and why",
            "the reader sees the text, not the escaping around it"
        );
        assert_eq!(back(text), text);
    }

    #[test]
    fn a_translation_landing_in_a_quoted_cell_is_wrapped_again() {
        let text = "KEY,English\nParagraph,\"one, two\"\n";
        let mut done = BTreeMap::new();
        done.insert("row[0].English".to_string(), "a \"b\", c".to_string());

        let out = put_back(text, &done).expect("a render");

        assert_eq!(
            out, "KEY,English\nParagraph,\"a \"\"b\"\", c\"\n",
            "a comma or a quote in the translation must not break the row"
        );
        assert_eq!(said(&out)[3], "a \"b\", c");
    }

    #[test]
    fn a_json_value_is_read_as_the_reader_sees_it_and_written_back_escaped() {
        let text = r#"{"say":"he said \"go\"\nnow"}"#;

        assert_eq!(
            said(text)[1],
            "he said \"go\"\nnow",
            "the translator has to see the text, not its escaping"
        );
        assert_eq!(back(text), text);

        let mut done = BTreeMap::new();
        done.insert(
            "say".to_string(),
            "彼は\"行け\"と言った\n今すぐ".to_string(),
        );

        let out = put_back(text, &done).expect("a render");
        assert_eq!(out, r#"{"say":"彼は\"行け\"と言った\n今すぐ"}"#);
        assert_eq!(
            said(&out)[1],
            "彼は\"行け\"と言った\n今すぐ",
            "and what was written has to read back the same"
        );
    }

    #[test]
    fn a_translation_that_grows_a_separator_is_wrapped_so_the_row_holds() {
        let text = "KEY,English\nGreeting,Hello there\n";
        let mut done = BTreeMap::new();
        done.insert(
            "row[0].English".to_string(),
            "おはよう, 元気ですか".to_string(),
        );

        let out = put_back(text, &done).expect("a render");

        assert_eq!(
            out, "KEY,English\nGreeting,\"おはよう, 元気ですか\"\n",
            "a comma in the translation would otherwise split the row into three cells"
        );
        assert_eq!(
            stems(&out),
            [
                "table.heading",
                "table.heading",
                "table.KEY",
                "table.English"
            ],
            "and the file still reads back as the same table"
        );
        assert_eq!(said(&out)[3], "おはよう, 元気ですか");
    }

    #[test]
    fn a_single_line_holding_a_comma_is_prose_not_a_table() {
        let text = "Hello, world";

        assert_eq!(
            stems(text),
            ["text.line"],
            "a heading with no rows under it is not a table, and splitting prose at its commas \
             hands the reader fragments"
        );
        assert_eq!(said(text), ["Hello, world"]);
        assert_eq!(back(text), text);

        assert_eq!(
            said("<a>Hello, world</a>"),
            ["Hello, world"],
            "and a one-line document is read as a document, not as a table"
        );
    }

    #[test]
    fn a_surrogate_pair_is_one_letter_and_survives_the_round_trip() {
        let text = r#"{"a":"\ud83d\ude00 hi"}"#;

        assert_eq!(
            said(text)[1],
            "😀 hi",
            "the two halves stand for one letter, not for twelve characters of backslash-u"
        );

        let mut done = BTreeMap::new();
        done.insert("a".to_string(), "😀 やあ".to_string());
        assert_eq!(
            put_back(text, &done).expect("a render"),
            r#"{"a":"😀 やあ"}"#,
            "what goes back is the letter itself, never a backslash-u spelled out as text"
        );

        assert_eq!(
            said(r#"{"a":"\ud83d alone"}"#)[1],
            "\\ud83d alone",
            "a half with no partner has no letter to become, so it is left as it was written"
        );
    }

    #[test]
    fn a_ragged_table_is_not_a_table() {
        assert_eq!(
            stems("KEY,English\nInfo,Hello,Bonjour\n"),
            ["text.line", "text.line"]
        );
    }

    #[test]
    fn a_translation_reaches_the_line_it_was_written_for() {
        let text = "Peter\nWait.\n\n";
        let mut done = BTreeMap::new();
        done.insert("line[1]".to_string(), "待って。".to_string());

        assert_eq!(
            put_back(text, &done).expect("a render"),
            "Peter\n待って。\n\n",
            "the name above the line is the speaker and not a line to translate, so writing the \
             answer one row out would put the words in the wrong mouth"
        );
    }

    #[test]
    fn nothing_at_all_is_read_without_panicking() {
        assert!(pieces("").is_empty());
        assert!(pieces("\n\n\n").is_empty());
        assert_eq!(back(""), "");
    }

    #[test]
    fn an_xml_document_gives_up_its_text_and_nothing_of_its_shape() {
        let text = concat!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n",
            "<NPC xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\n",
            "  <!-- Player -->\n",
            "  <ID>3</ID>\n",
            "  <Name>Ashley</Name>\n",
            "  <Bio>She said &quot;go&quot; &amp; left</Bio>\n",
            "  <Stats />\n",
            "</NPC>\n"
        );

        assert_eq!(
            said(text),
            ["3", "Ashley", "She said \"go\" & left"],
            "the declaration, the namespaces, the comment, the closing tags and an empty element \
             are shape, not text, and a reader asked to translate them can only break the file"
        );
        assert_eq!(stems(text), ["xml.ID", "xml.Name", "xml.Bio"]);
        assert_eq!(paths(text), ["ID[0]", "Name[0]", "Bio[0]"]);
        assert_eq!(back(text), text);
    }

    #[test]
    fn text_wrapped_in_cdata_is_still_text() {
        let text =
            "<NPC>\n  <Name>Ashley</Name>\n  <Bio><![CDATA[She said \"go\" <em>]]></Bio>\n</NPC>";

        assert_eq!(said(text), ["Ashley", "She said \"go\" <em>"]);
        assert_eq!(paths(text), ["Name[0]", "Bio[0]"]);
        assert_eq!(back(text), text);

        let done = BTreeMap::from([("Bio[0]".to_string(), "keep ]]> whole".to_string())]);
        assert_eq!(
            put_back(text, &done).expect("a render"),
            "<NPC>\n  <Name>Ashley</Name>\n  <Bio><![CDATA[keep ]]]]><![CDATA[> whole]]></Bio>\n</NPC>",
            "a translation that spells the section closer is split so the parser never sees it"
        );
    }

    #[test]
    fn two_columns_sharing_a_heading_keep_their_own_cells() {
        let text = "ID,Text,Text\n1,alpha,beta\n";

        assert!(paths(text).contains(&"row[0].Text".to_string()));
        assert!(paths(text).contains(&"row[0].Text#2".to_string()));

        let done = BTreeMap::from([
            ("row[0].Text".to_string(), "x".to_string()),
            ("row[0].Text#2".to_string(), "y".to_string()),
        ]);
        assert!(
            put_back(text, &done).expect("a render").contains("1,x,y"),
            "each cell answers to its own name even when two columns share a heading"
        );
    }

    #[test]
    fn a_translated_node_comes_back_escaped_and_its_neighbours_keep_their_bytes() {
        let text = "<NPC>\n  <Name>Ashley</Name>\n  <Bio>A &amp; B</Bio>\n</NPC>";
        let mut done = BTreeMap::new();
        done.insert("Name[0]".to_string(), "アシュリー & <b>".to_string());

        assert_eq!(
            put_back(text, &done).expect("a render"),
            "<NPC>\n  <Name>アシュリー &amp; &lt;b&gt;</Name>\n  <Bio>A &amp; B</Bio>\n</NPC>",
            "a translation lands as XML text, never as markup the parser would then read as tags"
        );
    }

    #[test]
    fn a_document_holding_no_text_at_all_is_still_read_as_a_document() {
        let text = concat!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n",
            "<PlayerData xmlns:xsd=\"http://www.w3.org/2001/XMLSchema\">\n",
            "</PlayerData>"
        );

        assert!(
            pieces(text).is_empty(),
            "there is nothing here to translate, and falling back to lines would hand the reader \
             the declaration and the root tag as if they were text"
        );
        assert_eq!(back(text), text);
    }

    #[test]
    fn a_document_that_does_not_close_its_tags_is_read_as_plain_lines_instead() {
        let text = "<NPC>\n  <Name>Ashley</Nam>\n";

        assert_eq!(
            stems(text),
            ["text.line", "text.line"],
            "a shape we cannot follow to the end must fall back, or every node after the break \
             would be filed under the wrong element"
        );
        assert_eq!(back(text), text);
    }

    #[test]
    fn a_tag_is_not_cut_short_by_an_angle_bracket_inside_an_attribute() {
        let text = "<a><b note=\"x > y\">one</b><b>two</b></a>";

        assert_eq!(said(text), ["one", "two"]);
        assert_eq!(
            paths(text),
            ["b[0]", "b[1]"],
            "two nodes of one name are told apart by the order they appear in"
        );
        assert_eq!(back(text), text);
    }

    #[test]
    fn two_values_sharing_a_path_get_a_spot_of_their_own() {
        let text = "[intro]\nx = \"one\"\ny = \"two\"\n";

        assert_eq!(said(text), ["one", "two"], "both lines reach the sheet");
        assert_eq!(
            paths(text),
            ["value#0", "value#1"],
            "one shared spot would write one translation into both"
        );

        let done = BTreeMap::from([("value#1".to_string(), "TWO".to_string())]);
        assert_eq!(
            put_back(text, &done).expect("a render"),
            "[intro]\nx = \"one\"\ny = \"TWO\"\n",
            "the answer lands only on the line it was made for"
        );

        let twice = r#"{"a":"one","a":"two"}"#;
        assert_eq!(
            paths(twice),
            ["key[0]", "a#0", "key[1]", "a#1"],
            "duplicate json keys stay two lines instead of collapsing into one"
        );
    }

    #[test]
    fn text_that_merely_opens_with_a_bracket_is_still_read_as_text() {
        assert_eq!(stems("{"), ["text.line"]);
        assert_eq!(
            said("[intro]\nWelcome to the village."),
            ["[intro]", "Welcome to the village."],
            "a bracket at the start must not hand the whole file to the json reader"
        );
        assert_eq!(
            back("[intro]\nWelcome to the village."),
            "[intro]\nWelcome to the village."
        );
    }

    #[test]
    fn a_cell_holding_a_line_break_does_not_split_the_row_it_sits_in() {
        let text = "KEY,English\nintro,\"line one\nline two\"\nnext,Hello\n";

        assert_eq!(
            said(text),
            [
                "KEY",
                "English",
                "intro",
                "line one\nline two",
                "next",
                "Hello"
            ],
            "a quoted cell may hold a newline, so rows cannot be cut on every newline"
        );
        assert_eq!(back(text), text);
    }

    #[test]
    fn a_quote_a_cell_never_opened_is_a_quote_and_not_a_wrapper() {
        let text = "KEY\tEnglish\nsize\tthe 12\" nails\nnext\tHello\n";

        assert_eq!(
            said(text),
            ["KEY", "English", "size", "the 12\" nails", "next", "Hello"],
            "a table that never quotes a cell still writes inches and feet: reading that quote \
             as an opening one swallows every row after it"
        );
        assert_eq!(back(text), text);
    }

    #[test]
    fn a_translation_with_a_line_break_leaves_the_file_a_table() {
        let text = "KEY,English\nintro,Hello\nnext,Bye\n";
        let mut done = BTreeMap::new();
        done.insert(
            "row[0].English".to_string(),
            "Good morning\nthere".to_string(),
        );

        let out = put_back(text, &done).expect("a render");

        assert_eq!(
            stems(&out),
            stems(text),
            "if a written newline made the file stop reading as a table every key would change \
             and every translation already saved against it would be orphaned"
        );
        assert!(said(&out).contains(&"Good morning\nthere".to_string()));
    }
}
