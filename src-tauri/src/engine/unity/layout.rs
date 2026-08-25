use crate::engine::unity::dotnet;
use crate::engine::unity::dotnet::{Assemblies, Class, Field, Shape};
use anyhow::{Result, bail};
use std::str;

const POINTER: usize = 12;
const STEP: usize = 4;
const DEEPEST: usize = 8;

const BEHAVIOUR: [&str; 2] = ["UnityEngine.MonoBehaviour", "UnityEngine.ScriptableObject"];
const ROOT: [&str; 3] = ["System.Object", "System.ValueType", "System.Enum"];
const OBJECT: &str = "UnityEngine.Object";
const STRUCT: &str = "System.ValueType";
const KEYFRAME: usize = 28;

enum Native {
    Wide(usize),
    Curve,
}

const NATIVE: [(&str, Native); 3] = [
    ("UnityEngine.AnimationCurve", Native::Curve),
    ("UnityEngine.RectOffset", Native::Wide(16)),
    ("UnityEngine.Color32", Native::Wide(4)),
];

fn natively(name: &str) -> Option<&'static Native> {
    NATIVE
        .iter()
        .find(|(named, _)| *named == name)
        .map(|(_, how)| how)
}

pub struct Spot {
    pub path: String,
    pub at: usize,
    pub text: String,
}

pub struct Read {
    pub spots: Vec<Spot>,
    pub numbers: Vec<(String, i64)>,
    pub pointers: Vec<(String, i32, i64)>,
}

fn points(shape: &Shape) -> bool {
    match shape {
        Shape::Reference => true,
        Shape::List(inner) => points(inner),
        _ => false,
    }
}

pub fn descends(known: &Assemblies, name: &str, ancestor: &str) -> bool {
    let mut walking = name.to_string();

    for _ in 0..DEEPEST * 4 {
        if walking == ancestor {
            return true;
        }
        let Some(one) = known.named(&walking) else {
            return false;
        };
        if one.base.is_empty() {
            return false;
        }
        walking = one.base.clone();
    }

    false
}

fn packed(shape: &Shape) -> Option<usize> {
    match shape {
        Shape::Bool | Shape::Byte => Some(1),
        Shape::Short => Some(2),
        _ => None,
    }
}

pub fn strings_in(known: &Assemblies, class: &str, body: &[u8]) -> Result<Vec<Spot>> {
    read(known, class, body).map(|one| one.spots)
}

pub fn read(known: &Assemblies, class: &str, body: &[u8]) -> Result<Read> {
    let mut walk = Walk {
        known,
        body,
        at: 0,
        found: Vec::new(),
        counted: Vec::new(),
        aimed: Vec::new(),
        pointed: false,
        quiet: false,
    };

    walk.object(class)?;
    if walk.pointed {
        walk.registry()?;
    }

    if walk.at != body.len() {
        bail!(
            "{class} reads {} of {} byte(s): the layout does not fit the object",
            walk.at,
            body.len()
        );
    }

    Ok(Read {
        spots: walk.found,
        numbers: walk.counted,
        pointers: walk.aimed,
    })
}

fn fields_of(ladder: Vec<&Class>) -> impl Iterator<Item = &Field> {
    ladder.into_iter().rev().flat_map(|one| one.fields.iter())
}

struct Walk<'a> {
    known: &'a Assemblies,
    body: &'a [u8],
    at: usize,
    found: Vec<Spot>,
    counted: Vec<(String, i64)>,
    aimed: Vec<(String, i32, i64)>,
    pointed: bool,
    quiet: bool,
}

impl<'a> Walk<'a> {
    fn object(&mut self, class: &str) -> Result<()> {
        self.skip(POINTER)?;
        self.skip(1)?;
        self.align();
        self.skip(POINTER)?;
        self.text("m_Name")?;

        let (ladder, stopped) = self.ancestry(class)?;
        if !BEHAVIOUR.contains(&stopped.as_str()) {
            bail!("{class} does not come down from a class Unity serializes this way");
        }

        for field in fields_of(ladder) {
            self.deep(&field.shape, &field.name, 0)?;
        }

        Ok(())
    }

    fn ancestry(&self, class: &str) -> Result<(Vec<&'a Class>, String)> {
        let mut ladder = Vec::new();
        let mut walking = class.to_string();

        while !BEHAVIOUR.contains(&walking.as_str())
            && !ROOT.contains(&walking.as_str())
            && !walking.is_empty()
        {
            let Some(one) = self.known.named(&walking) else {
                bail!("{walking} is not in any assembly this game ships");
            };
            if ladder.len() > DEEPEST * 4 {
                bail!("{class} inherits in a circle");
            }
            ladder.push(one);
            walking = one.base.clone();
        }

        Ok((ladder, walking))
    }

    fn deep(&mut self, shape: &Shape, path: &str, depth: usize) -> Result<()> {
        if depth > DEEPEST {
            bail!("{path} nests deeper than Unity serializes");
        }

        if points(shape) {
            self.pointed = true;
        }

        match shape {
            Shape::Bool | Shape::Byte => {
                self.skip(1)?;
                self.align();
            }
            Shape::Short => {
                self.skip(2)?;
                self.align();
            }
            Shape::Int | Shape::Float => self.skip(4)?,
            Shape::Double => self.skip(8)?,
            Shape::Long => self.long(path)?,
            Shape::Reference => self.skip(8)?,
            Shape::Text => self.text(path)?,
            Shape::List(inner) => {
                let many = self.number()?;

                let element = match inner.as_ref() {
                    Shape::Named(name) => self
                        .known
                        .named(name)
                        .and_then(|one| one.enumeration.as_ref()),
                    _ => None,
                }
                .unwrap_or(inner);

                match packed(element) {
                    Some(wide) => self.skip(many * wide)?,
                    None => {
                        for which in 0..many {
                            self.deep(inner, &format!("{path}[{which}]"), depth + 1)?;
                        }
                    }
                }

                self.align();
            }
            Shape::Named(name) => self.named(name, path, depth)?,
            Shape::Unknown => bail!("{path} has a type this reader does not know"),
        }

        Ok(())
    }

    fn named(&mut self, name: &str, path: &str, depth: usize) -> Result<()> {
        if self.descends(name, OBJECT) {
            if !self.quiet
                && let Some(raw) = self.body.get(self.at..self.at + POINTER)
                && let (Ok(file), Ok(path_id)) = (raw[..4].try_into(), raw[4..].try_into())
            {
                self.aimed.push((
                    path.to_string(),
                    i32::from_le_bytes(file),
                    i64::from_le_bytes(path_id),
                ));
            }

            return self.skip(POINTER);
        }

        match natively(name) {
            Some(Native::Wide(wide)) => return self.skip(*wide),
            Some(Native::Curve) => return self.curve(),
            None => {}
        }

        let Some(one) = self.known.named(name) else {
            bail!("{path} is a {name}, which is not in any assembly this game ships");
        };

        if let Some(kind) = &one.enumeration {
            return self.deep(kind, path, depth + 1);
        }

        if !one.serializable && one.base != STRUCT {
            return Ok(());
        }

        let (ladder, _) = self.ancestry(name)?;
        for field in fields_of(ladder) {
            self.deep(&field.shape, &format!("{path}.{}", field.name), depth + 1)?;
        }

        Ok(())
    }

    fn registry(&mut self) -> Result<()> {
        self.quiet = true;

        let _version = self.number()?;
        let many = self.number()?;

        for _ in 0..many {
            self.skip(8)?;
            let class = self.said()?;
            let space = self.said()?;
            let _assembly = self.said()?;

            let named = dotnet::full_name(&space, &class);
            self.named(&named, &named, 0)?;
        }

        self.quiet = false;
        Ok(())
    }

    fn curve(&mut self) -> Result<()> {
        let many = self.number()?;
        self.skip(many * KEYFRAME)?;
        self.align();

        self.skip(4 * 3)
    }

    fn descends(&self, name: &str, ancestor: &str) -> bool {
        descends(self.known, name, ancestor)
    }

    fn skip(&mut self, many: usize) -> Result<()> {
        let past = self.at + many;
        if past > self.body.len() {
            bail!("a field reaches past the object");
        }

        self.at = past;
        Ok(())
    }

    fn align(&mut self) {
        self.at = self.at.next_multiple_of(STEP).min(self.body.len());
    }

    fn number(&mut self) -> Result<usize> {
        let raw = self
            .body
            .get(self.at..self.at + 4)
            .ok_or_else(|| anyhow::anyhow!("a count reaches past the object"))?;
        self.at += 4;

        let many = i32::from_le_bytes(raw.try_into()?);
        if many < 0 || many as usize > self.body.len() {
            bail!("a count of {many} is more than the object could hold");
        }

        Ok(many as usize)
    }

    fn long(&mut self, path: &str) -> Result<()> {
        let raw = self
            .body
            .get(self.at..self.at + 8)
            .ok_or_else(|| anyhow::anyhow!("a number reaches past the object"))?;
        let number = i64::from_le_bytes(raw.try_into()?);
        self.at += 8;

        if !self.quiet {
            self.counted.push((path.to_string(), number));
        }

        Ok(())
    }

    fn said(&mut self) -> Result<String> {
        let many = self.number()?;

        let raw = self
            .body
            .get(self.at..self.at + many)
            .ok_or_else(|| anyhow::anyhow!("a string reaches past the object"))?;
        let text = str::from_utf8(raw)
            .map_err(|_| anyhow::anyhow!("a string is not text after all"))?
            .to_string();

        self.at += many;
        self.align();

        Ok(text)
    }

    fn text(&mut self, path: &str) -> Result<()> {
        let at = self.at;
        let text = self
            .said()
            .map_err(|_| anyhow::anyhow!("{path} is not text after all"))?;

        if !self.quiet {
            self.found.push(Spot {
                path: path.to_string(),
                at,
                text,
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::unity::fake;
    use std::path::Path;

    fn head() -> Vec<u8> {
        let mut out = vec![0u8; POINTER + STEP + POINTER];
        out.extend_from_slice(&0u32.to_le_bytes());
        out
    }

    fn word(body: &mut Vec<u8>, said: &str) {
        body.extend_from_slice(&fake::string(said));
    }

    #[test]
    fn a_run_of_small_numbers_is_aligned_once_not_once_each() {
        let known = Assemblies::forged(vec![(
            "Holder",
            fake::class(
                BEHAVIOUR[0],
                vec![
                    ("raw", Shape::List(Box::new(Shape::Byte))),
                    ("said", Shape::Text),
                ],
            ),
        )]);

        let mut body = head();
        body.extend_from_slice(&3u32.to_le_bytes());
        body.extend_from_slice(&[1, 2, 3]);
        body.push(0);
        word(&mut body, "after the bytes");

        let found = strings_in(&known, "Holder", &body).expect("an exact walk");

        let said: Vec<&str> = found.iter().map(|one| one.text.as_str()).collect();

        assert_eq!(
            said,
            ["", "after the bytes"],
            "Unity pads a byte array once at its end: padding every element walks off the object"
        );
    }

    #[test]
    fn a_list_of_byte_backed_enums_is_packed_like_the_bytes_it_is() {
        let mut flags = fake::class("System.Enum", Vec::new());
        flags.enumeration = Some(Shape::Byte);

        let known = Assemblies::forged(vec![
            (
                "Holder",
                fake::class(
                    BEHAVIOUR[0],
                    vec![
                        ("flags", Shape::List(Box::new(Shape::Named("Flags".into())))),
                        ("said", Shape::Text),
                    ],
                ),
            ),
            ("Flags", flags),
        ]);

        let mut body = head();
        body.extend_from_slice(&3u32.to_le_bytes());
        body.extend_from_slice(&[1, 2, 3]);
        body.push(0);
        word(&mut body, "after the flags");

        let found = strings_in(&known, "Holder", &body).expect("an exact walk");
        let said: Vec<&str> = found.iter().map(|one| one.text.as_str()).collect();

        assert_eq!(
            said,
            ["", "after the flags"],
            "an enum stored as a byte is written like the byte itself, so its list is padded \
             once at the end and not once per element"
        );
    }

    #[test]
    fn a_walk_that_does_not_land_on_the_end_is_refused() {
        let known = Assemblies::read(Path::new("/nowhere"));

        assert!(
            strings_in(&known, "Whatever", &[0; 64]).is_err(),
            "stopping short of the end means the fields were read by the wrong widths, and \
             every string found along the way came out of the wrong bytes"
        );
        assert!(strings_in(&known, BEHAVIOUR[0], &[0; 64]).is_err());
    }

    #[test]
    fn the_fields_a_class_inherits_are_read_before_its_own() {
        let known = Assemblies::forged(vec![
            (
                "Parent",
                fake::class(BEHAVIOUR[0], vec![("from_parent", Shape::Text)]),
            ),
            (
                "Child",
                fake::class("Parent", vec![("of_its_own", Shape::Text)]),
            ),
        ]);

        let mut body = head();
        word(&mut body, "first");
        word(&mut body, "second");

        let found = strings_in(&known, "Child", &body).expect("a walk");
        let said: Vec<(&str, &str)> = found
            .iter()
            .map(|one| (one.path.as_str(), one.text.as_str()))
            .collect();

        assert_eq!(
            said,
            [
                ("m_Name", ""),
                ("from_parent", "first"),
                ("of_its_own", "second")
            ],
            "Unity writes a base class before the class that extends it"
        );
    }

    #[test]
    fn a_serialized_field_carries_what_its_own_base_class_holds() {
        let known = Assemblies::forged(vec![
            (
                "Holder",
                fake::class(BEHAVIOUR[0], vec![("kept", Shape::Named("Note".into()))]),
            ),
            ("Note", fake::class("Label", vec![("body", Shape::Text)])),
            (
                "Label",
                fake::class("System.Object", vec![("title", Shape::Text)]),
            ),
        ]);

        let mut body = head();
        word(&mut body, "a title");
        word(&mut body, "a body");

        let found = strings_in(&known, "Holder", &body).expect("a walk");

        assert_eq!(
            found
                .iter()
                .map(|one| one.path.as_str())
                .collect::<Vec<_>>(),
            ["m_Name", "kept.title", "kept.body"],
            "a field's type brings its whole ancestry, not just what it declares itself"
        );
    }

    #[test]
    fn a_class_unity_writes_natively_costs_what_the_engine_writes() {
        let known = Assemblies::forged(vec![
            (
                "Holder",
                fake::class(
                    BEHAVIOUR[0],
                    vec![
                        ("space", Shape::Named("UnityEngine.RectOffset".into())),
                        ("said", Shape::Text),
                    ],
                ),
            ),
            (
                "UnityEngine.RectOffset",
                fake::class("System.Object", Vec::new()),
            ),
        ]);

        let mut body = head();
        body.extend_from_slice(&[0; 16]);
        word(&mut body, "after the padding");

        let found = strings_in(&known, "Holder", &body).expect("a walk");

        assert_eq!(
            found.last().map(|one| one.text.as_str()),
            Some("after the padding"),
            "a native type has no managed fields, so only its own width places what follows"
        );
    }

    #[test]
    fn a_class_that_never_reaches_a_behaviour_is_refused() {
        let known = Assemblies::forged(vec![(
            "Loose",
            fake::class("System.Object", vec![("said", Shape::Text)]),
        )]);

        let mut body = head();
        word(&mut body, "anything");

        assert!(
            strings_in(&known, "Loose", &body).is_err(),
            "only a class Unity lays out this way may be read this way"
        );
    }

    #[test]
    fn a_field_unity_keeps_by_reference_is_an_id_and_the_objects_come_after() {
        let known = Assemblies::forged(vec![
            (
                "Table",
                fake::class(
                    "UnityEngine.ScriptableObject",
                    vec![
                        ("m_Metadata", Shape::List(Box::new(Shape::Reference))),
                        ("m_Line", Shape::Text),
                    ],
                ),
            ),
            (
                "Note",
                fake::class("System.Object", vec![("m_Said", Shape::Text)]),
            ),
        ]);

        let mut body = head();
        body.extend_from_slice(&1u32.to_le_bytes());
        body.extend_from_slice(&0x2929_2929_2929_2929u64.to_le_bytes());
        body.extend_from_slice(&fake::strings(&["Wait for me."]));

        body.extend_from_slice(&2u32.to_le_bytes());
        body.extend_from_slice(&1u32.to_le_bytes());
        body.extend_from_slice(&0x2929_2929_2929_2929u64.to_le_bytes());
        body.extend_from_slice(&fake::strings(&["Note", "", "Game"]));
        body.extend_from_slice(&fake::strings(&["a comment nobody translates"]));

        let found = strings_in(&known, "Table", &body).expect("the whole object reads");

        assert_eq!(
            found
                .iter()
                .map(|one| (one.path.as_str(), one.text.as_str()))
                .collect::<Vec<_>>(),
            [("m_Name", ""), ("m_Line", "Wait for me.")],
            "the id is eight bytes and the objects it points at are read but never offered: \
             their text is the game's own bookkeeping, not a line anybody reads"
        );
    }

    #[test]
    fn a_list_of_references_nobody_filled_still_leaves_its_two_counts_behind() {
        let known = Assemblies::forged(vec![(
            "Locale",
            fake::class(
                "UnityEngine.ScriptableObject",
                vec![
                    ("m_Metadata", Shape::List(Box::new(Shape::Reference))),
                    ("m_LocaleName", Shape::Text),
                ],
            ),
        )]);

        let mut body = head();
        body.extend_from_slice(&0u32.to_le_bytes());
        body.extend_from_slice(&fake::strings(&["French (fr)"]));
        body.extend_from_slice(&2u32.to_le_bytes());
        body.extend_from_slice(&0u32.to_le_bytes());

        let found = strings_in(&known, "Locale", &body).expect("the whole object reads");

        assert_eq!(
            found
                .iter()
                .map(|one| (one.path.as_str(), one.text.as_str()))
                .collect::<Vec<_>>(),
            [("m_Name", ""), ("m_LocaleName", "French (fr)")],
            "Unity writes the registry because the class declares a reference, not because \
             anything ended up in it: reading it only when a reference turned up leaves eight \
             bytes over and the whole object is refused"
        );
    }
}
