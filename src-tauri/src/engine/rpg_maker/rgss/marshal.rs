use crate::engine::rpg_maker::harvest::Step;
use serde_json::{Map, Number, Value};
use std::cmp::Reverse;
use std::ops::Range;

const MAJOR: u8 = 4;
const MINOR: u8 = 8;

#[derive(Debug, Clone)]
pub struct Text {
    pub path: Vec<Step>,
    pub prefix: usize,
    pub at: Range<usize>,
    pub text: String,
}

#[derive(Debug)]
pub struct Sheet {
    pub view: Value,
    pub texts: Vec<Text>,
}

pub fn read(bytes: &[u8]) -> Result<Sheet, String> {
    let mut reader = Reader {
        bytes,
        at: 0,
        symbols: Vec::new(),
        links: Vec::new(),
        texts: Vec::new(),
        path: Vec::new(),
    };

    let major = reader.byte()?;
    let minor = reader.byte()?;
    if (major, minor) != (MAJOR, MINOR) {
        return Err(format!("marshal {major}.{minor} is not {MAJOR}.{MINOR}"));
    }

    let view = reader.object()?;

    if reader.at != bytes.len() {
        return Err(format!(
            "{} bytes past the end of the stream",
            bytes.len() - reader.at
        ));
    }

    Ok(Sheet {
        view,
        texts: reader.texts,
    })
}

pub fn long_bytes(value: i64) -> Vec<u8> {
    if value == 0 {
        return vec![0];
    }
    if (1..123).contains(&value) {
        return vec![(value + 5) as u8];
    }
    if (-123..0).contains(&value) {
        return vec![((value - 5) & 0xff) as u8];
    }

    let mut out = vec![0u8];
    let mut left = value;

    for taken in 1..=4i64 {
        out.push((left & 0xff) as u8);
        left >>= 8;

        if left == 0 {
            out[0] = taken as u8;
            break;
        }
        if left == -1 {
            out[0] = (-taken & 0xff) as u8;
            break;
        }
    }

    debug_assert!(out[0] != 0, "{value} does not fit Marshal's long format");

    out
}

pub fn spliced(bytes: &[u8], edits: &[(&Text, Vec<u8>)]) -> Vec<u8> {
    let mut order: Vec<&(&Text, Vec<u8>)> = edits.iter().collect();
    order.sort_by_key(|(text, _)| Reverse(text.prefix));

    let mut kept: Vec<&(&Text, Vec<u8>)> = Vec::with_capacity(order.len());
    let mut cut = bytes.len();

    for held in order {
        if held.0.at.end > cut {
            continue;
        }

        cut = held.0.prefix;
        kept.push(held);
    }

    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;

    for (text, said) in kept.iter().rev() {
        out.extend_from_slice(&bytes[at..text.prefix]);
        out.extend_from_slice(&long_bytes(said.len() as i64));
        out.extend_from_slice(said);
        at = text.at.end;
    }

    out.extend_from_slice(&bytes[at..]);

    out
}

struct Reader<'b> {
    bytes: &'b [u8],
    at: usize,
    symbols: Vec<String>,
    links: Vec<Value>,
    texts: Vec<Text>,
    path: Vec<Step>,
}

impl<'b> Reader<'b> {
    fn byte(&mut self) -> Result<u8, String> {
        let found = *self
            .bytes
            .get(self.at)
            .ok_or_else(|| "the stream ends mid value".to_string())?;
        self.at += 1;

        Ok(found)
    }

    fn long(&mut self) -> Result<i64, String> {
        let lead = self.byte()? as i8;

        if lead == 0 {
            return Ok(0);
        }

        if lead > 0 {
            if lead > 4 {
                return Ok(lead as i64 - 5);
            }

            let mut value = 0i64;
            for step in 0..lead {
                value |= (self.byte()? as i64) << (8 * step);
            }

            return Ok(value);
        }

        if lead < -4 {
            return Ok(lead as i64 + 5);
        }

        let taken = -(lead as i64);
        let mut value = -1i64;
        for step in 0..taken {
            value &= !(0xffi64 << (8 * step));
            value |= (self.byte()? as i64) << (8 * step);
        }

        Ok(value)
    }

    fn raw(&mut self) -> Result<(usize, Range<usize>), String> {
        let prefix = self.at;
        let size = self.long()?;
        if size < 0 {
            return Err(format!("a length of {size}"));
        }

        let at = self.at..self.at + size as usize;
        if at.end > self.bytes.len() {
            return Err("a length past the end of the stream".to_string());
        }
        self.at = at.end;

        Ok((prefix, at))
    }

    fn reserve(&mut self) -> usize {
        self.links.push(Value::Null);

        self.links.len() - 1
    }

    fn keep(&mut self, value: Value) -> Value {
        self.links.push(value.clone());

        value
    }

    fn symbol(&mut self) -> Result<String, String> {
        match self.byte()? {
            b'I' => {
                let name = self.symbol()?;
                self.fields(None)?;

                Ok(name)
            }
            b':' => {
                let (_, at) = self.raw()?;
                let name = String::from_utf8_lossy(&self.bytes[at]).into_owned();
                self.symbols.push(name.clone());

                Ok(name)
            }
            b';' => {
                let which = self.long()?;
                self.symbols
                    .get(which as usize)
                    .cloned()
                    .ok_or_else(|| format!("symbol {which} was never written"))
            }
            other => Err(format!("{} is not a symbol", other as char)),
        }
    }

    fn fields(&mut self, mut into: Option<&mut Map<String, Value>>) -> Result<(), String> {
        let mut left = self.long()?;

        while left > 0 {
            let name = self.symbol()?;
            let key = as_json(name.strip_prefix('@').unwrap_or(&name));

            self.path.push(Step::Key(key.clone()));
            let value = self.object();
            self.path.pop();

            let value = value?;
            if let Some(fields) = into.as_deref_mut() {
                fields.insert(key, value);
            }
            left -= 1;
        }

        Ok(())
    }

    fn object(&mut self) -> Result<Value, String> {
        match self.byte()? {
            b'0' => Ok(Value::Null),
            b'T' => Ok(Value::Bool(true)),
            b'F' => Ok(Value::Bool(false)),
            b'i' => Ok(Value::Number(self.long()?.into())),
            b':' | b';' => {
                self.at -= 1;
                let name = self.symbol()?;

                Ok(Value::String(name))
            }
            b'@' => {
                let which = self.long()?;
                self.links
                    .get(which as usize)
                    .cloned()
                    .ok_or_else(|| format!("object {which} was never written"))
            }
            b'I' => {
                let value = self.object()?;
                self.fields(None)?;

                Ok(value)
            }
            b'"' => {
                let (prefix, at) = self.raw()?;
                let text = String::from_utf8_lossy(&self.bytes[at.clone()]).into_owned();

                self.texts.push(Text {
                    path: self.path.clone(),
                    prefix,
                    at,
                    text: text.clone(),
                });

                Ok(self.keep(Value::String(text)))
            }
            b'f' => {
                let (_, at) = self.raw()?;
                let said = String::from_utf8_lossy(&self.bytes[at]).into_owned();
                let value = said
                    .parse::<f64>()
                    .ok()
                    .and_then(Number::from_f64)
                    .map_or(Value::Null, Value::Number);

                Ok(self.keep(value))
            }
            b'l' => {
                self.byte()?;
                let words = self.long()?;
                self.at += 2 * words.max(0) as usize;
                if self.at > self.bytes.len() {
                    return Err("a bignum past the end of the stream".to_string());
                }

                Ok(self.keep(Value::Null))
            }
            b'[' => {
                let slot = self.reserve();
                let mut items = Vec::new();

                for index in 0..self.long()?.max(0) as usize {
                    self.path.push(Step::Index(index));
                    let item = self.object();
                    self.path.pop();
                    items.push(item?);
                }

                let value = Value::Array(items);
                self.links[slot] = value.clone();

                Ok(value)
            }
            kind @ (b'{' | b'}') => {
                let slot = self.reserve();
                let mut fields = Map::new();

                for _ in 0..self.long()?.max(0) as usize {
                    let key = named(&self.object()?);

                    self.path.push(Step::Key(key.clone()));
                    let value = self.object();
                    self.path.pop();

                    fields.insert(key, value?);
                }

                if kind == b'}' {
                    self.object()?;
                }

                let value = Value::Object(fields);
                self.links[slot] = value.clone();

                Ok(value)
            }
            b'o' | b'S' => {
                let slot = self.reserve();
                self.symbol()?;

                let mut fields = Map::new();
                self.fields(Some(&mut fields))?;

                let value = Value::Object(fields);
                self.links[slot] = value.clone();

                Ok(value)
            }
            b'u' => {
                self.symbol()?;
                self.raw()?;

                Ok(self.keep(Value::Null))
            }
            b'U' => {
                let slot = self.reserve();
                self.symbol()?;
                let value = self.object()?;
                self.links[slot] = value.clone();

                Ok(value)
            }
            b'C' | b'e' => {
                self.symbol()?;

                self.object()
            }
            b'c' | b'm' | b'M' => {
                let (_, at) = self.raw()?;
                let name = String::from_utf8_lossy(&self.bytes[at]).into_owned();

                Ok(self.keep(Value::String(name)))
            }
            b'/' => {
                let (_, at) = self.raw()?;
                self.byte()?;
                let said = String::from_utf8_lossy(&self.bytes[at]).into_owned();

                Ok(self.keep(Value::String(said)))
            }
            b'd' => {
                let slot = self.reserve();
                self.symbol()?;
                let value = self.object()?;
                self.links[slot] = value.clone();

                Ok(value)
            }
            other => Err(format!(
                "{} at byte {} is not a value this reader knows",
                other as char,
                self.at - 1
            )),
        }
    }
}

fn as_json(ivar: &str) -> String {
    let mut out = String::with_capacity(ivar.len());
    let mut upper = false;

    for letter in ivar.chars() {
        if letter == '_' {
            upper = true;
            continue;
        }

        if upper {
            out.extend(letter.to_uppercase());
            upper = false;
        } else {
            out.push(letter);
        }
    }

    out
}

fn named(key: &Value) -> String {
    match key {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::rpg_maker::rgss::fixture::{hash, list, name, number, said, stream};

    #[test]
    fn two_edits_reaching_for_one_string_leave_the_stream_still_readable() {
        let raw = stream(&list(&[said("one"), said("two")]));
        let sheet = read(&raw).expect("a sheet");

        let first = sheet
            .texts
            .iter()
            .find(|one| one.text == "one")
            .expect("the first string");

        let fresh = spliced(
            &raw,
            &[(first, b"ichi".to_vec()), (first, b"hitotsu".to_vec())],
        );

        let after = read(&fresh).expect("it still reads");
        assert_eq!(
            after
                .texts
                .iter()
                .map(|one| one.text.as_str())
                .collect::<Vec<_>>(),
            ["ichi", "two"],
            "two edits aimed at one string would splice the second over bytes the first already \
             moved, and what comes out is a stream nothing can read: the second is turned away"
        );
    }

    #[test]
    fn a_hash_carrying_a_default_leaves_the_reader_standing_on_the_value_after_it() {
        let pairs = [(said("a"), number(1))];
        let after = said("the line after it");

        for (held, what) in [
            (hash(&pairs, None), "a plain hash"),
            (hash(&pairs, Some(number(9))), "a hash carrying a default"),
        ] {
            let sheet = read(&stream(&list(&[held, after.clone()]))).expect(what);
            let listed = sheet.view.as_array().expect("a list");

            assert_eq!(
                listed[0]["a"].as_i64(),
                Some(1),
                "{what} keeps its own pair"
            );
            assert_eq!(
                listed[1].as_str(),
                Some("the line after it"),
                "{what}: a default nobody steps over leaves the reader standing mid-stream, and \
                 every object after it is read out of the wrong bytes"
            );
        }
    }

    #[test]
    fn the_integer_encoding_is_the_one_ruby_writes() {
        for value in [
            0i64, 1, 122, 123, 255, 256, 401, -1, -123, -124, -300, 70000,
        ] {
            let sheet = read(&stream(&number(value))).expect("a number");

            assert_eq!(
                sheet.view.as_i64(),
                Some(value),
                "{value} did not survive the round trip"
            );
        }

        assert_eq!(long_bytes(0), vec![0]);
        assert_eq!(long_bytes(1), vec![6]);
        assert_eq!(long_bytes(122), vec![127]);
        assert_eq!(long_bytes(123), vec![1, 123]);
        assert_eq!(long_bytes(-1), vec![250]);
    }

    #[test]
    fn a_user_marshal_object_takes_its_link_slot_before_its_payload() {
        let mut body = vec![b'o'];
        body.extend_from_slice(&name("Holder"));
        body.extend_from_slice(&long_bytes(2));
        body.extend_from_slice(&name("@a"));
        body.push(b'U');
        body.extend_from_slice(&name("Color"));
        body.push(b'[');
        body.extend_from_slice(&long_bytes(1));
        body.extend_from_slice(&said("inner"));
        body.extend_from_slice(&name("@b"));
        body.push(b'@');
        body.extend_from_slice(&long_bytes(3));

        let sheet = read(&stream(&body)).expect("a stream ruby wrote");

        assert_eq!(
            sheet.view["b"], "inner",
            "ruby numbers the wrapper before its payload, so every later link counts it"
        );
    }

    #[test]
    fn a_linked_string_is_one_line_that_reads_back_in_both_places() {
        let mut body = vec![b'['];
        body.extend_from_slice(&long_bytes(2));
        body.push(b'I');
        body.extend_from_slice(&said("Hello"));
        body.extend_from_slice(&long_bytes(1));
        body.extend_from_slice(&name("E"));
        body.push(b'T');
        body.push(b'@');
        body.extend_from_slice(&long_bytes(1));

        let sheet = read(&stream(&body)).expect("a stream ruby wrote");

        assert_eq!(sheet.view[0], "Hello");
        assert_eq!(sheet.view[1], "Hello", "the link points back at the string");
        assert_eq!(
            sheet.texts.len(),
            1,
            "a link is not a second line to translate"
        );
    }

    #[test]
    fn an_event_command_reads_back_with_the_field_names_the_harvest_knows() {
        let mut body = vec![b'o'];
        body.extend_from_slice(&name("RPG::EventCommand"));
        body.extend_from_slice(&long_bytes(3));
        body.extend_from_slice(&name("@code"));
        body.extend_from_slice(&number(401));
        body.extend_from_slice(&name("@indent"));
        body.extend_from_slice(&number(0));
        body.extend_from_slice(&name("@parameters"));
        body.push(b'[');
        body.extend_from_slice(&long_bytes(1));
        body.extend_from_slice(&said("Hello"));

        let raw = stream(&body);
        let sheet = read(&raw).expect("an event command");

        assert_eq!(sheet.view["indent"], 0);
        assert_eq!(sheet.view["parameters"][0], "Hello");
        assert_eq!(sheet.texts.len(), 1, "only the line is text");
        assert_eq!(sheet.texts[0].text, "Hello");
        assert_eq!(
            sheet.texts[0].path,
            vec![Step::Key("parameters".to_string()), Step::Index(0)],
            "the path has to read like the one a JSON sheet gives"
        );
    }

    #[test]
    fn a_record_reads_with_the_field_names_the_json_generation_spells() {
        let mut body = vec![b'o'];
        body.extend_from_slice(&name("RPG::Map"));
        body.extend_from_slice(&long_bytes(2));
        body.extend_from_slice(&name("@display_name"));
        body.extend_from_slice(&said("Castle Town"));
        body.extend_from_slice(&name("@battleback1_name"));
        body.extend_from_slice(&said("Fort"));

        let sheet = read(&stream(&body)).expect("a map");

        assert_eq!(
            sheet.view["displayName"], "Castle Town",
            "RGSS writes @display_name where MV writes displayName, and one table has to serve both"
        );
        assert_eq!(sheet.view["battleback1Name"], "Fort");
        assert_eq!(as_json("message1"), "message1");
        assert_eq!(as_json("se"), "se");
    }

    #[test]
    fn a_translated_line_is_spliced_in_and_nothing_else_moves() {
        let mut body = vec![b'['];
        body.extend_from_slice(&long_bytes(2));
        body.extend_from_slice(&said("Hello"));
        body.extend_from_slice(&said("Bye"));

        let raw = stream(&body);
        let sheet = read(&raw).expect("two strings");

        let said = "\u{3053}\u{3093}\u{306b}\u{3061}\u{306f}".to_string();
        let fresh = spliced(&raw, &[(&sheet.texts[0], said.clone().into_bytes())]);
        let after = read(&fresh).expect("the spliced stream still reads");

        assert_eq!(after.texts[0].text, said, "the line took the translation");
        assert_eq!(after.texts[1].text, "Bye", "its neighbour is untouched");
        assert_eq!(after.view[1], "Bye");
    }

    #[test]
    fn a_length_that_no_longer_fits_one_byte_grows_its_prefix() {
        let raw = stream(&said("a"));
        let sheet = read(&raw).expect("one string");

        let long = "x".repeat(300);
        let fresh = spliced(&raw, &[(&sheet.texts[0], long.clone().into_bytes())]);

        assert_eq!(read(&fresh).expect("it reads").texts[0].text, long);
        assert_eq!(long_bytes(300), vec![2, 44, 1]);
    }

    #[test]
    fn a_stream_holding_bytes_past_the_end_is_refused() {
        let mut raw = stream(&[b'i', 5]);
        raw.push(b'i');

        assert!(read(&raw).is_err(), "a reader that stops early is wrong");
    }
}
