use crate::engine::unity::serial::{Object, Value};
use std::collections::{BTreeMap, BTreeSet};

pub const SPRITE_ATLAS: i32 = 687078895;

pub const WIDEST: f64 = 32768.0;

#[derive(Clone, Copy, PartialEq)]
pub struct Spot {
    pub of: i64,
    pub elsewhere: bool,
    pub alpha_apart: bool,
    pub rect: (f64, f64, f64, f64),
    pub settings: i64,
}

impl Spot {
    pub fn empty(&self) -> bool {
        let (_, _, wide, high) = self.rect;

        wide < 1.0 || high < 1.0
    }

    pub fn sound(&self) -> bool {
        let (from_x, from_y, wide, high) = self.rect;

        (1.0..=WIDEST).contains(&wide)
            && (1.0..=WIDEST).contains(&high)
            && (0.0..=WIDEST).contains(&from_x)
            && (0.0..=WIDEST).contains(&from_y)
    }
}

pub fn spot_in(render: &Value) -> Option<Spot> {
    let rect = render.field("textureRect")?;
    let side = |at: &str| rect.field(at).and_then(Value::real);

    let pointed = |at: &str, part: &str| {
        render
            .field(at)
            .and_then(|one| one.field(part))
            .and_then(Value::number)
    };

    Some(Spot {
        of: pointed("texture", "m_PathID")?,
        elsewhere: pointed("texture", "m_FileID")? != 0,
        alpha_apart: pointed("alphaTexture", "m_PathID").unwrap_or(0) != 0,
        rect: (side("x")?, side("y")?, side("width")?, side("height")?),
        settings: render
            .field("settingsRaw")
            .and_then(Value::number)
            .unwrap_or(0),
    })
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    guid: [i64; 4],
    local: i64,
}

pub enum Found {
    Spot(Spot),
    Twice,
    Scaled,
}

#[derive(Default)]
pub struct Sheets {
    spots: BTreeMap<Key, Spot>,
    twice: BTreeSet<Key>,
    scaled: BTreeSet<Key>,
}

impl Sheets {
    pub fn read(objects: &[Object]) -> Self {
        let mut held = Self::default();

        for atlas in objects
            .iter()
            .filter(|one| one.class_id == SPRITE_ATLAS)
            .filter_map(Object::value)
        {
            for pair in atlas
                .field("m_RenderDataMap")
                .map(Value::items)
                .unwrap_or(&[])
            {
                let Some(key) = pair.field("first").and_then(keyed) else {
                    continue;
                };
                let Some(entry) = pair.field("second") else {
                    continue;
                };
                let Some(spot) = spot_in(entry) else {
                    continue;
                };

                if scaled_down(entry) {
                    held.scaled.insert(key);
                    continue;
                }

                match held.spots.get(&key) {
                    Some(had) if *had == spot => {}
                    Some(_) => {
                        held.twice.insert(key);
                    }
                    None => {
                        held.spots.insert(key, spot);
                    }
                }
            }
        }

        held
    }

    pub fn look(&self, sprite: &Value) -> Option<Found> {
        let key = key_in(sprite)?;

        if self.twice.contains(&key) {
            return Some(Found::Twice);
        }
        if self.scaled.contains(&key) {
            return Some(Found::Scaled);
        }

        self.spots.get(&key).copied().map(Found::Spot)
    }
}

pub fn packed(sprite: &Value) -> bool {
    let pointed = sprite
        .field("m_SpriteAtlas")
        .and_then(|one| one.field("m_PathID"))
        .and_then(Value::number)
        .unwrap_or(0);

    pointed != 0 || !tags_in(sprite).is_empty()
}

pub fn tagged(sprite: &Value) -> String {
    tags_in(sprite)
        .first()
        .and_then(Value::text)
        .unwrap_or_default()
}

fn tags_in(sprite: &Value) -> &[Value] {
    sprite.field("m_AtlasTags").map(Value::items).unwrap_or(&[])
}

fn key_in(sprite: &Value) -> Option<Key> {
    keyed(sprite.field("m_RenderDataKey")?)
}

fn keyed(pair: &Value) -> Option<Key> {
    let guid = pair.field("first")?;
    let mut held = [0i64; 4];
    for (which, one) in held.iter_mut().enumerate() {
        *one = guid.nth(which)?.number()?;
    }

    Some(Key {
        guid: held,
        local: pair.field("second")?.number()?,
    })
}

fn scaled_down(entry: &Value) -> bool {
    entry
        .field("downscaleMultiplier")
        .and_then(Value::real)
        .is_some_and(|held| held != 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::unity::{fake, serial};

    const KEY: fake::Key = fake::Key {
        guid: [11, 22, 33, 44],
        local: 21300000,
    };

    fn a_sprite(name: &str, key: fake::Key, atlas: i64) -> Object {
        Object::forged(
            serial::SPRITE,
            7,
            fake::a_packed_sprite(name, atlas, (64.0, 32.0), key, ""),
        )
    }

    fn an_atlas(name: &str, held: &[fake::Entry]) -> Object {
        Object::forged(SPRITE_ATLAS, 9, fake::an_atlas(name, held))
    }

    fn one_entry(key: fake::Key, of: i64, rect: (f32, f32, f32, f32)) -> fake::Entry {
        fake::Entry {
            key,
            of,
            rect,
            settings: 65,
            ..fake::entry()
        }
    }

    #[test]
    fn a_sprite_is_found_in_the_atlas_by_the_key_it_carries() {
        let objects = vec![
            a_sprite("packed", KEY, 9),
            an_atlas("one_Atlas", &[one_entry(KEY, 42, (16.0, 8.0, 64.0, 32.0))]),
        ];

        let sheets = Sheets::read(&objects);
        let value = objects[0].value().expect("a sprite");
        let Some(Found::Spot(held)) = sheets.look(&value) else {
            panic!(
                "the sprite carries no rect of its own once it is packed: the atlas is the only \
                 place its spot is written down"
            );
        };

        assert_eq!(held.of, 42);
        assert_eq!(held.rect, (16.0, 8.0, 64.0, 32.0));
        assert_eq!(held.settings, 65);
        assert!(held.sound() && !held.empty());

        let stranger = a_sprite(
            "stranger",
            fake::Key {
                local: 21300001,
                ..KEY
            },
            9,
        )
        .value()
        .expect("a sprite");
        assert!(
            sheets.look(&stranger).is_none(),
            "a key one number apart is another sprite, and handing it this rect would show a \
             reader somebody else's picture"
        );
    }

    #[test]
    fn a_sprite_two_atlases_claim_differently_is_left_alone() {
        let objects = vec![
            a_sprite("packed", KEY, 9),
            an_atlas("one_Atlas", &[one_entry(KEY, 42, (16.0, 8.0, 64.0, 32.0))]),
            an_atlas("two_Atlas", &[one_entry(KEY, 43, (0.0, 0.0, 64.0, 32.0))]),
        ];

        let sheets = Sheets::read(&objects);
        let value = objects[0].value().expect("a sprite");

        assert!(
            matches!(sheets.look(&value), Some(Found::Twice)),
            "a variant atlas holds the same sprites as its master at another size: picking one of \
             the two would draw over a picture the game keeps somewhere else"
        );
    }

    #[test]
    fn two_atlases_agreeing_on_a_sprite_are_not_a_disagreement() {
        let objects = vec![
            a_sprite("packed", KEY, 9),
            an_atlas("one_Atlas", &[one_entry(KEY, 42, (16.0, 8.0, 64.0, 32.0))]),
            an_atlas("two_Atlas", &[one_entry(KEY, 42, (16.0, 8.0, 64.0, 32.0))]),
        ];

        let sheets = Sheets::read(&objects);
        let value = objects[0].value().expect("a sprite");

        assert!(
            matches!(sheets.look(&value), Some(Found::Spot(_))),
            "one atlas listed in two places is still one answer, and refusing it would take a \
             picture away for nothing"
        );
    }

    #[test]
    fn a_sprite_the_atlas_keeps_at_another_scale_is_left_alone() {
        let objects = vec![
            a_sprite("packed", KEY, 9),
            an_atlas(
                "small_Atlas",
                &[fake::Entry {
                    downscale: 0.5,
                    ..one_entry(KEY, 42, (16.0, 8.0, 64.0, 32.0))
                }],
            ),
        ];

        let sheets = Sheets::read(&objects);
        let value = objects[0].value().expect("a sprite");

        assert!(
            matches!(sheets.look(&value), Some(Found::Scaled)),
            "a downscaled atlas writes the rect at the size the master atlas had, so cutting by \
             it would hand back pixels from the wrong place"
        );
    }

    #[test]
    fn a_sprite_whose_atlas_is_not_in_this_file_is_answered_by_nothing_here() {
        let objects = vec![a_sprite("packed", KEY, 9)];
        let sheets = Sheets::read(&objects);
        let value = objects[0].value().expect("a sprite");

        assert!(
            sheets.look(&value).is_none(),
            "the atlas may sit in another container, and there is nothing here to read it out of"
        );
        assert!(
            packed(&value),
            "the sprite still says it was packed, which is what tells the reader why it has no \
             picture"
        );
    }

    #[test]
    fn a_sprite_names_the_atlas_it_was_packed_into_even_with_no_atlas_to_read() {
        let held = Object::forged(
            serial::SPRITE,
            7,
            fake::a_packed_sprite("packed", 0, (64.0, 32.0), KEY, "Journal_Atlas"),
        )
        .value()
        .expect("a sprite");

        assert_eq!(
            tagged(&held),
            "Journal_Atlas",
            "the atlas picture is named after its atlas, so this tag is the one thread a reader \
             has back to the picture when the atlas itself is in another file"
        );
        assert!(
            packed(&held),
            "a tag and no pointer is how a sprite says it was packed by name alone"
        );
    }
}
