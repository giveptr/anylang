use crate::engine::wolf_rpg::coder::{self, Reader};
use crate::engine::wolf_rpg::event;
use crate::engine::wolf_rpg::held::{self, Held, Piece};

pub const NAME: &str = "CommonEvent";

const MAGIC: [u8; 9] = [0x57, 0x00, 0x00, 0x4F, 0x4C, 0x00, 0x46, 0x43, 0x00];
const UTF8_AT: usize = 5;
const FROM: usize = 1;
const VERSION_AT: usize = 10;
const HEAD: usize = 11;

const PACKED: [u8; 2] = [0x93, 0xCC];
const OPENS: u8 = 0x8E;
const PARTS: u8 = 0x8F;
const NAMED: u8 = 0x91;
const EXTRA: u8 = 0x92;
const LEAST_END: u8 = 0x89;

const HEADER: usize = 7;
const MIDDLE: usize = 0x1D;
const SLOTS: usize = 100;

pub fn read(raw: &[u8]) -> Result<Held, String> {
    coder::spelled(&MAGIC, UTF8_AT, raw, FROM)?;
    let v35 = PACKED.contains(&coder::byte_at(raw, VERSION_AT)?);
    let (plain, shape) = held::opened(raw, HEAD, v35)?;

    let mut reader = Reader::over(&plain, HEAD);
    let mut pieces = Vec::new();

    let count = reader.count()?;
    for which in 0..count {
        one(&mut reader, which, v35, &mut pieces)
            .map_err(|why| format!("common event {which} could not be read: {why}"))?;
    }

    let closing = reader.byte()?;
    if closing < LEAST_END {
        return Err(format!(
            "the common events end with {closing:#04x}, which is below {LEAST_END:#04x}"
        ));
    }

    reader.ended()?;

    Ok(Held {
        plain,
        shape,
        pieces,
    })
}

fn one(
    reader: &mut Reader,
    which: usize,
    v35: bool,
    pieces: &mut Vec<Piece>,
) -> Result<(), String> {
    reader.marker(OPENS, "the head of a common event")?;

    reader.word()?;
    reader.word()?;
    reader.skip(HEADER)?;
    reader.past_said()?;

    event::commands(reader, v35, &format!("e{which}"), pieces)?;

    reader.past_said()?;
    reader.past_said()?;
    reader.marker(PARTS, "the settings of a common event")?;

    let count = reader.count()?;
    reader.past_saids(count)?;

    let count = reader.count()?;
    reader.skip(count)?;

    reader.past_said_lists()?;
    reader.past_word_lists()?;

    reader.skip(MIDDLE)?;
    reader.past_saids(SLOTS)?;

    reader.marker(NAMED, "the name a common event is called by")?;
    reader.past_said()?;

    let closing = reader.byte()?;
    if closing == NAMED {
        return Ok(());
    }
    if closing != EXTRA {
        return Err(format!(
            "a common event ends with {NAMED:#04x} or {EXTRA:#04x} and this one with \
             {closing:#04x}"
        ));
    }

    reader.past_said()?;
    reader.word()?;

    reader.marker(EXTRA, "the end of a common event")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::wolf_rpg::fixture;

    #[test]
    fn every_common_event_gives_up_its_lines_under_the_number_it_sits_at() {
        let raw = fixture::commons(&[
            &[fixture::command(
                101,
                &[],
                &["\u{6249}\u{306f}\u{9589}\u{307e}\u{3063}\u{3066}"],
            )],
            &[],
            &[fixture::command(122, &[3000001], &["Rest"])],
        ]);

        let held = read(&raw).expect("the common events");

        assert_eq!(
            held.pieces
                .iter()
                .map(|one| (one.spot.as_str(), one.said[0].text.as_str()))
                .collect::<Vec<_>>(),
            [
                ("e0/c0", "\u{6249}\u{306f}\u{9589}\u{307e}\u{3063}\u{3066}"),
                ("e2/c0", "Rest"),
            ],
            "an event holding nothing to translate still counts, or every later number slips"
        );
        assert_eq!(held.shape, held::Shape::Plain);
    }

    #[test]
    fn common_events_from_the_newest_editor_are_opened_out_and_read_with_the_tails_they_carry() {
        let raw = fixture::newest_commons(&[
            &[fixture::tailed(101, &[], &["\u{306f}\u{3044}"])],
            &[fixture::tailed(122, &[3000001], &["Rest"])],
        ]);

        let held = read(&raw).expect("the common events");

        assert_eq!(
            held.pieces
                .iter()
                .map(|one| (one.spot.as_str(), one.said[0].text.as_str()))
                .collect::<Vec<_>>(),
            [("e0/c0", "\u{306f}\u{3044}"), ("e1/c0", "Rest")],
            "this editor packs the file and writes a byte-counted tail on every command, and both \
             have to be read for the events after the first to land where they belong"
        );
        assert_eq!(held.shape, held::Shape::Packed { head: HEAD });
    }

    #[test]
    fn a_common_event_file_that_does_not_end_where_it_should_is_refused() {
        let mut raw = fixture::commons(&[&[fixture::command(101, &[], &["Hello"])]]);
        *raw.last_mut().expect("the terminator") = 0x01;

        assert!(read(&raw).is_err());
        assert!(read(b"not a wolf file").is_err());
    }
}
