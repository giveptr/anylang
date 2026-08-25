use crate::canvas::Canvas;
use crate::engine::unity::atlas::{self, Found, Sheets, Spot, WIDEST};
use crate::engine::unity::serial;
use crate::engine::unity::serial::{Object, Value};
use anyhow::{Result, bail};

const PACKED: i64 = 0x1;
const TURNED: i64 = 0x3c;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Turn {
    Upright,
    Mirrored,
    Upended,
    Halfway,
    Odd,
}

impl Turn {
    fn of(settings: i64) -> Self {
        if settings & PACKED == 0 {
            return match settings & TURNED {
                0 => Self::Upright,
                _ => Self::Odd,
            };
        }

        match (settings & TURNED) >> 2 {
            0 => Self::Upright,
            1 => Self::Mirrored,
            2 => Self::Upended,
            3 => Self::Halfway,
            _ => Self::Odd,
        }
    }

    fn on(&self, held: &Canvas) -> Canvas {
        match self {
            Self::Upright | Self::Odd => held.clone(),
            Self::Mirrored => held.mirrored(),
            Self::Upended => held.flipped(),
            Self::Halfway => held.flipped().mirrored(),
        }
    }

    pub fn odd(&self) -> bool {
        matches!(self, Self::Odd)
    }
}

pub struct Cut {
    pub name: String,
    pub at: i64,
    pub of: i64,
    pub elsewhere: bool,
    pub alpha_apart: bool,
    pub from_x: usize,
    pub from_y: usize,
    pub wide: usize,
    pub high: usize,
    pub turn: Turn,
}

pub enum Adrift {
    Packed(String),
    Twice,
    Scaled,
    Bare,
    Odd,
}

pub struct Away {
    pub name: String,
    pub wide: usize,
    pub high: usize,
    pub why: Adrift,
}

enum Sits {
    At(Spot),
    Adrift(Adrift),
    Nowhere,
}

fn sits(value: &Value, sheets: &Sheets) -> Sits {
    let spot = match sheets.look(value) {
        Some(Found::Twice) => return Sits::Adrift(Adrift::Twice),
        Some(Found::Scaled) => return Sits::Adrift(Adrift::Scaled),
        Some(Found::Spot(held)) => held,
        None => {
            match value
                .field("m_RD")
                .and_then(atlas::spot_in)
                .filter(|held| held.of != 0)
            {
                Some(held) => held,
                None if atlas::packed(value) => {
                    return Sits::Adrift(Adrift::Packed(atlas::tagged(value)));
                }
                None => return Sits::Adrift(Adrift::Bare),
            }
        }
    };

    if spot.empty() {
        return Sits::Nowhere;
    }

    match spot.sound() {
        true => Sits::At(spot),
        false => Sits::Adrift(Adrift::Odd),
    }
}

pub fn cut_in(value: &Value, sheets: &Sheets, at: i64) -> Option<Cut> {
    let Sits::At(spot) = sits(value, sheets) else {
        return None;
    };
    let (from_x, from_y, wide, high) = spot.rect;

    Some(Cut {
        name: value
            .field("m_Name")
            .and_then(Value::text)
            .unwrap_or_default(),
        at,
        of: spot.of,
        elsewhere: spot.elsewhere,
        alpha_apart: spot.alpha_apart,
        from_x: from_x.round() as usize,
        from_y: from_y.round() as usize,
        wide: wide.round() as usize,
        high: high.round() as usize,
        turn: Turn::of(spot.settings),
    })
}

pub fn cut_of(object: &Object, sheets: &Sheets) -> Option<Cut> {
    if object.class_id != serial::SPRITE {
        return None;
    }

    cut_in(&object.value()?, sheets, object.path_id)
}

pub fn cuts_in(objects: &[Object], sheets: &Sheets) -> Vec<Cut> {
    objects
        .iter()
        .filter_map(|object| cut_of(object, sheets))
        .collect()
}

pub fn adrift(value: &Value, sheets: &Sheets) -> Option<Away> {
    let Sits::Adrift(why) = sits(value, sheets) else {
        return None;
    };

    let sized = |at: &str| {
        value
            .field("m_Rect")
            .and_then(|one| one.field(at))
            .and_then(Value::real)
            .filter(|held| (1.0..=WIDEST).contains(held))
            .map(|held| held.round() as usize)
            .unwrap_or(0)
    };

    Some(Away {
        name: value
            .field("m_Name")
            .and_then(Value::text)
            .unwrap_or_default(),
        wide: sized("width"),
        high: sized("height"),
        why,
    })
}

impl Cut {
    pub fn inside(&self, wide: usize, high: usize) -> bool {
        self.from_x + self.wide <= wide && self.from_y + self.high <= high
    }

    pub fn cut(&self, whole: &Canvas) -> Result<Canvas> {
        if !self.inside(whole.wide, whole.high) {
            bail!(
                "{} sits at {},{} and is {}x{}, which reaches past a picture of {}x{}",
                self.shown(),
                self.from_x,
                self.from_y,
                self.wide,
                self.high,
                whole.wide,
                whole.high
            );
        }

        let held = whole.cut(self.from_x, self.top_of(whole.high), self.wide, self.high)?;

        Ok(self.turn.on(&held))
    }

    pub fn paint(&self, whole: &mut Canvas, held: &Canvas) -> Result<()> {
        if self.turn.odd() {
            bail!(
                "{} is turned inside its atlas in a way this writer cannot put a picture back \
                 into",
                self.shown()
            );
        }
        if held.wide != self.wide || held.high != self.high {
            bail!(
                "{} is {}x{} in the game and the picture handed back is {}x{}",
                self.shown(),
                self.wide,
                self.high,
                held.wide,
                held.high
            );
        }
        if !self.inside(whole.wide, whole.high) {
            bail!(
                "{} does not sit inside a picture of {}x{}",
                self.shown(),
                whole.wide,
                whole.high
            );
        }

        let top = self.top_of(whole.high);

        whole.paint(self.from_x, top, &self.turn.on(held))
    }

    pub fn shown(&self) -> String {
        match self.name.is_empty() {
            true => format!("sprite {}", self.at),
            false => self.name.clone(),
        }
    }

    fn top_of(&self, high: usize) -> usize {
        high.saturating_sub(self.from_y + self.high)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::unity::{fake, serial};

    const PACKED_FLAG: u32 = 0x1;
    const TIGHT: u32 = 0x40;

    const KEY: fake::Key = fake::Key {
        guid: [1, 2, 3, 4],
        local: 21300000,
    };

    fn bare() -> Sheets {
        Sheets::default()
    }

    fn forged(name: &str, of: i64, rect: (f32, f32, f32, f32), settings: u32) -> Object {
        Object::forged(serial::SPRITE, 42, fake::a_sprite(name, of, rect, settings))
    }

    fn in_an_atlas(rect: (f32, f32, f32, f32), settings: u32) -> (Object, Sheets) {
        let objects = vec![
            Object::forged(
                serial::SPRITE,
                42,
                fake::a_packed_sprite("packed", 9, (rect.2, rect.3), KEY, ""),
            ),
            Object::forged(
                atlas::SPRITE_ATLAS,
                9,
                fake::an_atlas(
                    "one_Atlas",
                    &[fake::Entry {
                        key: KEY,
                        of: 7,
                        rect,
                        settings,
                        ..fake::entry()
                    }],
                ),
            ),
        ];

        let sheets = Sheets::read(&objects);
        let mut walk = objects.into_iter();

        (walk.next().expect("the sprite"), sheets)
    }

    fn dotted(wide: usize, high: usize) -> Canvas {
        let mut held = Canvas::blank(wide, high);

        for line in 0..high {
            for column in 0..wide {
                let at = (line * wide + column) * 4;
                held.pixels[at] = column as u8;
                held.pixels[at + 1] = line as u8;
                held.pixels[at + 3] = 255;
            }
        }

        held
    }

    #[test]
    fn a_sprite_names_the_texture_it_was_cut_out_of_and_where_it_sits() {
        let object = forged("button_yes", 7, (300.0, 0.0, 300.0, 300.0), 0);
        let held = cut_of(&object, &bare()).expect("a sprite this reader wrote itself");

        assert_eq!(held.name, "button_yes");
        assert_eq!(
            held.of, 7,
            "a sprite carries no pixels of its own: everything it shows lives in the texture it \
             points at, so losing that pointer loses the picture"
        );
        assert_eq!((held.from_x, held.from_y), (300, 0));
        assert_eq!((held.wide, held.high), (300, 300));
        assert_eq!(held.turn, Turn::Upright);
    }

    #[test]
    fn a_sprite_the_packer_moved_into_an_atlas_is_cut_out_of_the_atlas_instead() {
        let (object, sheets) = in_an_atlas((16.0, 8.0, 64.0, 32.0), 65);
        let held = cut_of(&object, &sheets).expect("a sprite the atlas answers for");

        assert_eq!(
            held.of, 7,
            "a packed sprite points at no texture of its own, and the atlas is the only place the \
             one it really draws from is written down"
        );
        assert_eq!((held.from_x, held.from_y), (16, 8));
        assert_eq!((held.wide, held.high), (64, 32));
        assert_eq!(held.turn, Turn::Upright);

        assert!(
            cut_of(&object, &bare()).is_none(),
            "without the atlas there is nothing to cut, and reading its own empty rect would \
             hand back a picture from the wrong place"
        );
    }

    #[test]
    fn what_the_atlas_says_outranks_what_the_sprite_kept_of_its_own() {
        let objects = vec![
            Object::forged(
                serial::SPRITE,
                42,
                fake::a_sprite_in_an_atlas("both", 5, (0.0, 0.0, 8.0, 8.0), 9, KEY),
            ),
            Object::forged(
                atlas::SPRITE_ATLAS,
                9,
                fake::an_atlas(
                    "one_Atlas",
                    &[fake::Entry {
                        key: KEY,
                        of: 7,
                        rect: (16.0, 8.0, 64.0, 32.0),
                        settings: 65,
                        ..fake::entry()
                    }],
                ),
            ),
        ];
        let sheets = Sheets::read(&objects);
        let held = cut_of(&objects[0], &sheets).expect("a sprite");

        assert_eq!(
            (held.of, held.from_x, held.from_y),
            (7, 16, 8),
            "a build may leave the loose texture in place beside the atlas, and the game draws \
             the atlas: showing the loose copy would let a reader paint a picture nobody sees"
        );
    }

    #[test]
    fn a_cut_is_taken_from_the_top_because_unity_counts_from_the_bottom() {
        let atlas = dotted(8, 8);
        let object = forged("low", 7, (0.0, 0.0, 4.0, 2.0), 0);
        let held = cut_of(&object, &bare()).expect("a sprite");

        let piece = held.cut(&atlas).expect("it cuts");

        assert_eq!((piece.wide, piece.high), (4, 2));
        assert_eq!(
            piece.pixels[1], 6,
            "a rect at y=0 is the bottom of the atlas, which is the last row of a picture read \
             top down: cutting from the top instead hands the reader the wrong sprite"
        );
    }

    #[test]
    fn what_is_painted_in_lands_exactly_where_the_cut_came_from() {
        let mut atlas = dotted(16, 8);
        let object = forged("piece", 7, (4.0, 2.0, 4.0, 4.0), 0);
        let held = cut_of(&object, &bare()).expect("a sprite");

        let was = held.cut(&atlas).expect("it cuts");
        let fresh = Canvas::of(4, 4, vec![200; 64]).expect("a picture");

        held.paint(&mut atlas, &fresh).expect("it paints");
        let back = held.cut(&atlas).expect("it cuts again");

        assert_eq!(
            back.pixels, fresh.pixels,
            "the sprite that goes in has to be the sprite that comes back out of the same rect"
        );
        assert_ne!(was.pixels, back.pixels);
    }

    #[test]
    fn a_sprite_of_the_wrong_size_or_off_the_edge_is_refused() {
        let mut atlas = dotted(8, 8);
        let object = forged("piece", 7, (4.0, 2.0, 4.0, 4.0), 0);
        let held = cut_of(&object, &bare()).expect("a sprite");

        assert!(
            held.paint(&mut atlas, &dotted(3, 4)).is_err(),
            "a sprite is a rect of a fixed size, and anything else would spill onto its neighbours"
        );

        let over = cut_of(&forged("over", 7, (6.0, 0.0, 4.0, 4.0), 0), &bare()).expect("a sprite");
        assert!(
            over.cut(&atlas).is_err(),
            "a rect this reader read wrongly reaches past the atlas, and guessing at it would \
             show a picture nobody drew"
        );
        assert!(!over.inside(8, 8));
    }

    #[test]
    fn a_sprite_the_packer_flipped_comes_back_the_way_the_game_draws_it() {
        let atlas = dotted(16, 8);

        for (settings, turn) in [
            (69, Turn::Mirrored),
            (73, Turn::Upended),
            (77, Turn::Halfway),
        ] {
            let (object, sheets) = in_an_atlas((0.0, 0.0, 4.0, 4.0), settings);
            let held = cut_of(&object, &sheets).expect("a sprite");
            assert_eq!(held.turn, turn, "settings {settings:#x}");

            let upright = cut_of(&forged("upright", 7, (0.0, 0.0, 4.0, 4.0), 0), &bare())
                .expect("the same rect, unturned");

            let shown = held.cut(&atlas).expect("it cuts");
            let raw = upright.cut(&atlas).expect("it cuts");

            assert_ne!(
                shown.pixels, raw.pixels,
                "the packer stored these pixels turned, so handing them over as they lie would \
                 show the reader a picture the game never draws"
            );
            assert_eq!(
                turn.on(&shown).pixels,
                raw.pixels,
                "and turning them back has to land on the very bytes the atlas holds"
            );
        }
    }

    #[test]
    fn a_flipped_sprite_goes_back_into_the_atlas_the_way_the_packer_stored_it() {
        for settings in [69, 73, 77] {
            let (object, sheets) = in_an_atlas((4.0, 2.0, 4.0, 4.0), settings);
            let held = cut_of(&object, &sheets).expect("a sprite");

            let mut atlas = dotted(16, 8);
            let fresh = dotted(4, 4);

            held.paint(&mut atlas, &fresh).expect("it paints");

            assert_eq!(
                held.cut(&atlas).expect("it cuts again").pixels,
                fresh.pixels,
                "settings {settings:#x}: what a reader hands in is what the game has to draw, so \
                 the flip on the way in and the flip on the way out have to be the same one"
            );
        }
    }

    #[test]
    fn a_sprite_the_packer_turned_a_quarter_is_marked_so_nobody_replaces_it_sideways() {
        let (object, sheets) = in_an_atlas((0.0, 0.0, 4.0, 4.0), 0x11);
        let held = cut_of(&object, &sheets).expect("a sprite the packer rotated");

        assert!(
            held.turn.odd(),
            "a quarter turn swaps the sides of the rect, so a picture painted upright into that \
             spot would show up sideways in the game"
        );
        assert!(
            held.paint(&mut dotted(16, 8), &dotted(4, 4)).is_err(),
            "and a pick made against it is turned away rather than painted"
        );
        assert_eq!(
            held.cut(&dotted(16, 8)).expect("it still cuts").wide,
            4,
            "the reader is still shown what is there, sideways and marked, rather than an empty \
             tile"
        );
    }

    #[test]
    fn a_tight_mesh_or_a_packed_flag_is_never_read_as_a_turn() {
        for settings in [TIGHT, PACKED_FLAG, TIGHT | PACKED_FLAG] {
            let held = cut_of(&forged("tight", 7, (0.0, 0.0, 4.0, 4.0), settings), &bare())
                .expect("a sprite the packer left upright");

            assert_eq!(
                held.turn,
                Turn::Upright,
                "2211 of the 2217 sprites in one game carry {settings:#x}: reading a tight mesh \
                 or a packed flag as a rotation would mark a whole game unreplaceable"
            );
        }

        let held = cut_of(&forged("odd", 7, (0.0, 0.0, 4.0, 4.0), 0x10), &bare())
            .expect("a sprite nobody packed, carrying rotation bits anyway");
        assert!(
            held.turn.odd(),
            "a sprite the packer never touched has no reason to carry a turn, so a turn on one is \
             a number this reader is reading wrongly"
        );
    }

    #[test]
    fn only_a_sprite_is_read_as_one() {
        let mut object = forged("piece", 7, (0.0, 0.0, 4.0, 4.0), 0);
        object.class_id = serial::TEXT_ASSET;

        assert!(cut_of(&object, &bare()).is_none());
    }

    #[test]
    fn a_sprite_nobody_named_is_still_one_this_reader_can_cut() {
        let object = forged("", 7, (0.0, 0.0, 4.0, 4.0), 0);
        let held = cut_of(&object, &bare()).expect("a sprite carrying no name of its own");

        assert_eq!(
            held.shown(),
            "sprite 42",
            "a name is only what the row is called: refusing the cut over it would take away a \
             picture that reads and replaces perfectly well"
        );
    }

    #[test]
    fn two_unnamed_sprites_cut_from_one_atlas_are_told_apart() {
        let body = fake::a_sprite("", 7, (0.0, 0.0, 4.0, 4.0), 0);
        let one = Object::forged(serial::SPRITE, 42, body.clone());
        let other = Object::forged(serial::SPRITE, 43, body);

        let one = cut_of(&one, &bare()).expect("a sprite");
        let other = cut_of(&other, &bare()).expect("the sprite beside it");

        assert_eq!(one.of, other.of, "both are cut from the same atlas texture");
        assert_ne!(
            one.shown(),
            other.shown(),
            "every line this reader drops or refuses names the sprite it is about, so two \
             nameless sprites sharing an atlas have to answer to different names or the reader \
             cannot tell which tile the complaint is for"
        );
    }

    #[test]
    fn a_sprite_the_game_gives_no_size_is_not_a_picture_at_all() {
        let hollow = forged("rig piece", 7, (343.0, 572.0, 0.0, 0.0), 0)
            .value()
            .expect("a sprite");

        assert!(cut_in(&hollow, &bare(), 42).is_none());
        assert!(
            adrift(&hollow, &bare()).is_none(),
            "16 sprites in one game are rig pieces the artist left at no size at all: the game \
             draws nothing for them and there is nothing to hand back, so they get no row, the \
             same way texture::unread gives none to a texture the game says is 0 by 0"
        );
    }

    #[test]
    fn a_sprite_with_nowhere_to_be_cut_from_says_which_of_the_ways_it_got_there() {
        let orphan = forged("orphan", 0, (0.0, 0.0, 8.0, 8.0), 0)
            .value()
            .expect("a sprite");
        assert!(matches!(
            adrift(&orphan, &bare()).expect("a row of its own").why,
            Adrift::Bare
        ));

        let (object, sheets) = in_an_atlas((16.0, 8.0, 64.0, 32.0), 65);
        let packed = object.value().expect("a sprite");
        assert!(
            matches!(
                adrift(&packed, &bare()).expect("a row of its own").why,
                Adrift::Packed(_)
            ),
            "the atlas naming this sprite lives in another container, and the reader has to hear \
             that rather than see an empty tile"
        );
        assert!(
            adrift(&packed, &sheets).is_none(),
            "once the atlas is there, the sprite is not adrift at all"
        );
    }

    #[test]
    fn a_rect_no_texture_could_hold_is_a_row_and_never_a_cut() {
        let wild = forged("wild", 7, (0.0, 0.0, 99_999.0, 8.0), 0)
            .value()
            .expect("a sprite");

        assert!(cut_in(&wild, &bare(), 42).is_none());
        assert!(
            matches!(
                adrift(&wild, &bare()).expect("a row of its own").why,
                Adrift::Odd
            ),
            "a rect wider than any texture is a number read out of the wrong four bytes, and the \
             reader hears that rather than losing the row"
        );
    }

    #[test]
    fn a_body_the_type_tree_reaches_past_the_end_of_is_refused() {
        let object = forged("piece", 7, (0.0, 0.0, 4.0, 4.0), 0);

        let mut body = object.body().expect("its body").into_owned();
        body.truncate(body.len() - 8);
        assert!(
            cut_of(&Object::forged(serial::SPRITE, 42, body), &bare()).is_none(),
            "a body that runs out before the shape does is one this reader cannot trust a single \
             number in"
        );

        let mut body = object.body().expect("its body").into_owned();
        body[0..4].copy_from_slice(&9999i32.to_le_bytes());
        assert!(
            cut_of(&Object::forged(serial::SPRITE, 42, body), &bare()).is_none(),
            "a length claiming more bytes than the object could hold is a number read out of the \
             wrong four bytes"
        );

        assert!(cut_of(&Object::forged(serial::SPRITE, 42, vec![0; 12]), &bare()).is_none());
    }
}
