use crate::engine::wolf_rpg::coder::{self, Reader};
use crate::engine::wolf_rpg::held::{Held, Kind, Piece, Said, Shape};

pub const NAME: &str = "Game";

const MAGIC: [u8; 9] = [0x57, 0x00, 0x00, 0x4F, 0x4C, 0x00, 0x46, 0x4D, 0x00];
const UTF8_AT: usize = 8;
const FROM: usize = 1;
const HEAD: usize = 10;

const STAMP: &str = "0000-0000";
const WITH_PLUS: u32 = 9;
const WITH_MESSAGES: u32 = 9;
const WITH_MORE: u32 = 13;

const TITLE: &str = "title";
const PLUS: &str = "titlePlus";
const OPENING: &str = "startUpMsg";
const HEADLINE: &str = "titleMsg";

pub const SUB_FONTS: usize = 3;

pub fn spelled(raw: &[u8]) -> Result<(), String> {
    coder::spelled(&MAGIC, UTF8_AT, raw, FROM)
}

pub fn read(raw: &[u8]) -> Result<Held, String> {
    spelled(raw)?;
    let mut reader = Reader::over(raw, HEAD);

    let count = reader.count()?;
    reader.skip(count)?;

    let strings = reader.word()?;

    let mut told: Vec<(&str, Said)> = Vec::new();
    let mut say = |name: &'static str, reader: &mut Reader| -> Result<(), String> {
        let (text, at) = reader.said()?;
        told.push((name, Said { text, at }));

        Ok(())
    };

    say(TITLE, &mut reader)?;

    let (stamp, _) = reader.said()?;
    if stamp != STAMP {
        return Err(format!("Game.dat is stamped {stamp:?} and not {STAMP:?}"));
    }

    let count = reader.count()?;
    reader.skip(count)?;

    let mut faces = Vec::with_capacity(1 + SUB_FONTS);
    for _ in 0..1 + SUB_FONTS {
        let (text, at) = reader.said()?;
        faces.push(Said { text, at });
    }

    reader.past_said()?;

    if strings >= WITH_PLUS {
        say(PLUS, &mut reader)?;
    }

    if strings > WITH_MESSAGES {
        reader.past_said()?;
        reader.past_said()?;
        say(OPENING, &mut reader)?;
        say(HEADLINE, &mut reader)?;
    }

    if strings > WITH_MORE {
        reader.past_said()?;
    }

    let size = reader.offset();
    let stamped = reader.word()? as usize;
    if stamped != raw.len() - 1 {
        return Err(format!(
            "Game.dat says it holds {} byte(s) where {} were read, a layout this reader does \
             not know",
            stamped + 1,
            raw.len()
        ));
    }
    reader.word()?;
    let words = reader.count()?;
    reader.skip(words * 2)?;

    let first = reader.offset();
    reader.word()?;
    let second = reader.offset();
    reader.word()?;

    let mut pieces = Vec::with_capacity(told.len() + 1);
    for (name, said) in told {
        pieces.push(Piece {
            spot: name.to_string(),
            kind: Kind::Title,
            said: vec![said],
        });
    }
    pieces.push(Piece {
        spot: "fonts".to_string(),
        kind: Kind::Font,
        said: faces,
    });

    Ok(Held {
        plain: raw.to_vec(),
        shape: Shape::Measured {
            size,
            offsets: [first, second],
        },
        pieces,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::wolf_rpg::{fixture, game, harvest, held};
    use std::collections::BTreeMap;

    #[test]
    fn the_title_and_the_fonts_the_engine_asks_for_are_both_found() {
        let raw = fixture::game("The Long Road Home", " + DLC", "Pixelify Sans");

        let held = read(&raw).expect("Game.dat");
        let named: Vec<&str> = held.pieces.iter().map(|one| one.spot.as_str()).collect();

        assert_eq!(named, [TITLE, PLUS, OPENING, HEADLINE, "fonts"]);
        assert_eq!(held.pieces[0].said[0].text, "The Long Road Home");
        assert_eq!(held.pieces[1].said[0].text, " + DLC");
        assert_eq!(
            held.fonts()
                .iter()
                .map(|one| one.text.as_str())
                .collect::<Vec<_>>(),
            ["Pixelify Sans", "", "", ""],
            "the main face comes first and the three it falls back to follow"
        );
    }

    #[test]
    fn a_longer_title_leaves_the_file_saying_its_own_new_size() {
        let raw = fixture::game("Short", "", "MS Gothic");
        let read = read(&raw).expect("Game.dat");

        let said = BTreeMap::from([(format!("{TITLE}/s0"), "A Rather Longer Title".to_string())]);

        let edits = harvest::changed(
            &read,
            &harvest::sift(&read.pieces, "", &Default::default()),
            &said,
            &Default::default(),
            &Default::default(),
        );
        let out = held::wrapped(&read, edits).expect("a whole Game.dat");

        assert!(out.len() > raw.len());
        assert_eq!(
            game::read(&out).expect("it still reads").pieces[0].said[0].text,
            "A Rather Longer Title",
            "the engine reads its own size out of this file, so a title that grows has to \
             leave every number after it true"
        );
    }

    #[test]
    fn an_older_game_dat_naming_fewer_strings_is_read_without_reaching_past_what_it_holds() {
        for (told, named) in [
            (8u32, vec![TITLE, "fonts"]),
            (9, vec![TITLE, PLUS, "fonts"]),
            (13, vec![TITLE, PLUS, OPENING, HEADLINE, "fonts"]),
            (14, vec![TITLE, PLUS, OPENING, HEADLINE, "fonts"]),
        ] {
            let raw = fixture::told_by(told, "A Title", " + DLC", &["MS Gothic"]);
            let held =
                read(&raw).unwrap_or_else(|why| panic!("a Game.dat of {told} strings: {why}"));

            assert_eq!(
                held.pieces
                    .iter()
                    .map(|one| one.spot.as_str())
                    .collect::<Vec<&str>>(),
                named,
                "this count is how the file says which of its strings are there at all, so \
                 reading one that is not written turns the next number into nonsense"
            );
        }
    }

    #[test]
    fn a_file_that_is_not_game_dat_is_turned_away() {
        assert!(read(b"nothing of the sort").is_err());

        let mut wrong = fixture::game("Title", "", "Font");
        let at = wrong
            .windows(STAMP.len())
            .position(|found| found == STAMP.as_bytes())
            .expect("the stamp");
        wrong[at] = b'9';

        assert!(
            read(&wrong).is_err(),
            "the stamp is how this file says it is Game.dat and not something else the same size"
        );
    }
}
