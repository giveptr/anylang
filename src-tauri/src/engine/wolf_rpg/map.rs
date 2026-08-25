use crate::engine::wolf_rpg::coder::{self, Reader};
use crate::engine::wolf_rpg::event;
use crate::engine::wolf_rpg::held::{self, Held, Piece};

pub const SUFFIX: &str = "mps";

const MAGIC: [u8; 20] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x57, 0x4F, 0x4C, 0x46, 0x4D, 0x00,
    0x00, 0x00, 0x00, 0x00,
];
const UTF8_AT: usize = 16;
const VERSION_AT: usize = 20;

const PACKED_FROM: u32 = 0x65;
const LAYERED_FROM: u32 = 0x67;
const HEAD: usize = 25;
const LAYERS: usize = 3;
const NO_TILES: u32 = 0xFFFF_FFFF;

const EVENT: u8 = 0x6F;
const EVENT_MAGIC: [u8; 4] = [0x39, 0x30, 0x00, 0x00];
const PAGES_MAGIC: [u8; 4] = [0x00, 0x00, 0x00, 0x00];
const EVENT_END: u8 = 0x70;
const PAGE: u8 = 0x79;
const PAGE_END: u8 = 0x7A;
const END: u8 = 0x66;

const CONDITIONS: usize = 1 + 4 + 4 * 4 + 4 * 4;
const MOVEMENT: usize = 4;
const FEATURED: u32 = 3;

pub fn read(raw: &[u8]) -> Result<Held, String> {
    coder::spelled(&MAGIC, UTF8_AT, raw, 0)?;
    let version = coder::word_at(raw, VERSION_AT)?;
    let (plain, shape) = held::opened(raw, HEAD, version >= PACKED_FROM)?;

    let last = plain.len().checked_sub(1).ok_or("this map is empty")?;
    if plain[last] != END {
        return Err(format!(
            "a map ends with {END:#04x} and this one ends with {:#04x}",
            plain[last]
        ));
    }

    let mut reader = Reader::over(&plain, VERSION_AT);
    let mut pieces = Vec::new();

    reader.word()?;
    reader.byte()?;
    reader.past_said()?;

    reader.word()?;
    let width = reader.word()? as usize;
    let height = reader.word()? as usize;
    let events = reader.count()?;

    let mut layers = LAYERS;
    let mut v35 = false;
    if version >= LAYERED_FROM {
        reader.word()?;
        layers = reader.word()? as usize;
        v35 = true;
    }

    let mut found = 0;
    let closing = match reader.offset() == last {
        true => reader.byte()?,
        false => {
            let held = reader.offset();
            let marked = reader.word()? == NO_TILES
                && matches!(plain.get(reader.offset()), Some(&EVENT) | Some(&END));

            if !marked {
                reader.seek(held);
                let span = width
                    .checked_mul(height)
                    .and_then(|area| area.checked_mul(layers))
                    .and_then(|cells| cells.checked_mul(4))
                    .ok_or("this map claims more tiles than could ever be drawn")?;
                reader.skip(span)?;
            }

            loop {
                let marker = reader.byte()?;
                if marker != EVENT {
                    break marker;
                }

                one(&mut reader, found, v35, &mut pieces)?;
                found += 1;
            }
        }
    };

    if closing != END {
        return Err(format!(
            "a map ends with {END:#04x} and the events here run into {closing:#04x}"
        ));
    }

    if found != events {
        return Err(format!(
            "this map says it holds {events} events and {found} were read"
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
    reader.expect(&EVENT_MAGIC, "the head of a map event")?;

    reader.word()?;
    reader.past_said()?;
    reader.word()?;
    reader.word()?;
    let pages = reader.count()?;
    reader.expect(&PAGES_MAGIC, "the four zeroes before a page")?;

    let mut found = 0;
    let closing = loop {
        let marker = reader.byte()?;
        if marker != PAGE {
            break marker;
        }

        page(reader, &format!("e{which}/p{found}"), v35, pieces)?;
        found += 1;
    };

    if found != pages {
        return Err(format!(
            "event {which} says it holds {pages} pages and {found} were read"
        ));
    }

    if closing != EVENT_END {
        return Err(format!(
            "event {which} ends with {closing:#04x} and not {EVENT_END:#04x}"
        ));
    }

    Ok(())
}

fn page(reader: &mut Reader, at: &str, v35: bool, pieces: &mut Vec<Piece>) -> Result<(), String> {
    reader.word()?;
    reader.past_said()?;
    reader.skip(4)?;
    reader.skip(CONDITIONS)?;
    reader.skip(MOVEMENT)?;
    reader.byte()?;
    reader.byte()?;

    event::route(reader)?;
    event::commands(reader, v35, at, pieces)?;

    let features = reader.word()?;
    reader.skip(3)?;
    if features > FEATURED {
        reader.byte()?;
    }

    reader.marker(PAGE_END, &format!("the end of {at}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::wolf_rpg::fixture;

    #[test]
    fn a_map_gives_up_every_line_its_events_speak_and_says_where_each_one_sits() {
        let raw = fixture::map(&[
            &[
                fixture::command(
                    101,
                    &[],
                    &["\u{5263}\u{3092}\u{62bc}\u{3057}\u{5165}\u{308c}"],
                ),
                fixture::command(121, &[2000299, 0], &[]),
            ],
            &[fixture::command(
                102,
                &[0],
                &["\u{306f}\u{3044}", "\u{3044}\u{3044}\u{3048}"],
            )],
        ]);

        let held = read(&raw).expect("a map");
        let spots: Vec<&str> = held.pieces.iter().map(|one| one.spot.as_str()).collect();

        assert_eq!(
            spots,
            ["e0/p0/c0", "e1/p0/c0"],
            "one event to a page and one page to an event, and the numbers are the file's own"
        );
        assert_eq!(held.pieces[1].said.len(), 2, "both choices come out");
        assert_eq!(held.shape, held::Shape::Plain);
    }

    #[test]
    fn a_map_the_newest_editor_packed_is_opened_out_and_read_the_same_way() {
        let raw = fixture::packed_map(&[&[fixture::command(101, &[], &["Hello"])]]);

        let held = read(&raw).expect("a packed map");

        assert_eq!(held.shape, held::Shape::Packed { head: HEAD });
        assert_eq!(held.pieces[0].said[0].text, "Hello");
    }

    #[test]
    fn a_map_from_the_newest_editor_is_read_with_the_layer_count_and_the_tail_each_command_carries()
    {
        let raw = fixture::newest_map(&[&[
            fixture::tailed(101, &[], &["\u{5263}\u{3092}\u{62bc}\u{3057}"]),
            fixture::tailed(102, &[0], &["\u{306f}\u{3044}", "\u{3044}\u{3044}\u{3048}"]),
        ]]);

        let held = read(&raw).expect("a map the newest editor wrote");

        assert_eq!(
            held.pieces
                .iter()
                .map(|one| (one.spot.as_str(), one.said.len()))
                .collect::<Vec<(&str, usize)>>(),
            [("e0/p0/c0", 1), ("e0/p0/c1", 2)],
            "this editor writes a layer count the older one did not and a byte-counted tail on \
             every command, and a reader that walks past either lands in the middle of the next \
             command"
        );
    }

    #[test]
    fn a_map_whose_own_bookkeeping_is_not_utf8_still_gives_up_the_lines_a_player_reads() {
        let named = "\u{753b}\u{50cf}\u{6d88}\u{53bb}";
        let spot = |raw: &[u8], said: &str| {
            raw.windows(said.len())
                .position(|found| found == said.as_bytes())
                .expect("the line the fixture wrote")
        };

        let mut raw = fixture::map(&[&[fixture::command(101, &[], &["Hello"])]]);
        let at = spot(&raw, named);
        raw[at..at + named.len()].fill(0x82);

        assert_eq!(
            read(&raw).expect("a map").pieces[0].said[0].text,
            "Hello",
            "the name of an event is never carried over, so a stray byte in it is no reason to \
             refuse the whole map and leave a translator unable to touch it"
        );

        let mut raw = fixture::map(&[&[fixture::command(101, &[], &["Hello"])]]);
        let at = spot(&raw, "Hello");
        raw[at..at + 5].fill(0x82);

        assert!(
            read(&raw).is_err(),
            "a line that is carried over has to read as text, or a translation would be spliced \
             in beside bytes nobody could make sense of"
        );
    }

    #[test]
    fn a_map_that_is_not_one_is_turned_away_rather_than_read_as_rubbish() {
        assert!(read(b"PNG\r\n").is_err());

        let mut broken = fixture::map(&[&[fixture::command(101, &[], &["Hello"])]]);
        *broken.last_mut().expect("the terminator") = 0x00;
        assert!(
            read(&broken).is_err(),
            "a map that does not end where it should has been read wrong somewhere, and \
             splicing into it would hand the player a game that will not boot"
        );
    }
}
