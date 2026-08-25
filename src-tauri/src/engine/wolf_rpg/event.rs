use crate::engine::wolf_rpg::coder::Reader;
use crate::engine::wolf_rpg::held::{Kind, Piece, Said};

pub const MESSAGE: u32 = 101;
pub const CHOICES: u32 = 102;
pub const STRING_CONDITION: u32 = 112;
pub const SET_STRING: u32 = 122;
pub const PICTURE: u32 = 150;
pub const MOVE: u32 = 201;
pub const DB_WRITE: u32 = 250;
pub const CALL_BY_NAME: u32 = 300;

const STEP_END: [u8; 2] = [0x01, 0x00];
const MOVED: u8 = 0x01;
const PLAIN: u8 = 0x00;
const MOVE_HEAD: usize = 6;

fn step(reader: &mut Reader) -> Result<(), String> {
    reader.byte()?;
    let count = reader.byte()? as usize;

    for _ in 0..count {
        reader.word()?;
    }

    reader.expect(&STEP_END, "the end of a move step")
}

pub fn route(reader: &mut Reader) -> Result<(), String> {
    let count = reader.count()?;

    for _ in 0..count {
        step(reader)?;
    }

    Ok(())
}

struct Command {
    pub code: u32,
    pub args: Vec<u32>,
    pub said: Vec<Said>,
}

fn command(reader: &mut Reader, v35: bool) -> Result<Command, String> {
    let count = reader.byte()?.wrapping_sub(1) as usize;
    let code = reader.word()?;

    let mut args = Vec::with_capacity(count);
    for _ in 0..count {
        args.push(reader.word()?);
    }

    reader.byte()?;

    let lines = reader.byte()? as usize;
    let mut said = Vec::with_capacity(lines);
    for _ in 0..lines {
        let (text, at) = reader.said()?;
        said.push(Said { text, at });
    }

    let at = reader.offset();
    match (reader.byte()?, code) {
        (MOVED, _) | (PLAIN, MOVE) => {
            reader.skip(MOVE_HEAD)?;
            route(reader)?;
        }
        (PLAIN, _) => {}
        (found, _) => {
            return Err(format!(
                "command {code} ends with {found:#04x} at {at}, and only {PLAIN:#04x} or \
                 {MOVED:#04x} belong there"
            ));
        }
    }

    if v35 {
        let trailing = reader.byte()? as usize;
        reader.skip(trailing)?;
    }

    Ok(Command { code, args, said })
}

pub fn commands(
    reader: &mut Reader,
    v35: bool,
    at: &str,
    pieces: &mut Vec<Piece>,
) -> Result<(), String> {
    let count = reader.count()?;

    for index in 0..count {
        let read = command(reader, v35)
            .map_err(|why| format!("command {index} of {at} could not be read: {why}"))?;

        if read.said.is_empty() {
            continue;
        }

        pieces.push(Piece {
            spot: format!("{at}/c{index}"),
            kind: Kind::Command {
                code: read.code,
                args: read.args,
            },
            said: read.said,
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::wolf_rpg::fixture;

    #[test]
    fn a_command_gives_up_its_code_its_numbers_and_every_line_it_carries() {
        let raw = fixture::command(101, &[0, 1], &["Hello", "there"]);
        let mut reader = Reader::over(&raw, 0);

        let read = command(&mut reader, false).expect("a message command");

        assert_eq!(read.code, 101);
        assert_eq!(read.args, vec![0, 1]);
        assert_eq!(
            read.said
                .iter()
                .map(|one| one.text.as_str())
                .collect::<Vec<_>>(),
            ["Hello", "there"]
        );
        assert!(
            reader.done(),
            "a command is read to its last byte or not at all"
        );
    }

    #[test]
    fn a_move_command_carries_a_route_no_translation_may_walk_past() {
        let raw = fixture::moved(&[(1, &[]), (2, &[7])]);
        let mut reader = Reader::over(&raw, 0);

        let read = command(&mut reader, false).expect("a move command");

        assert_eq!(read.code, MOVE);
        assert!(
            reader.done(),
            "the walk the author drew sits after the command, and stopping short of it would \
             read the next command out of the middle of a step"
        );
    }

    #[test]
    fn a_command_carrying_as_many_numbers_as_the_count_byte_can_hold_reads_every_one_of_them() {
        let raw = fixture::command(101, &[7; 255], &["Hello"]);
        let mut reader = Reader::over(&raw, 0);

        let read = command(&mut reader, false).expect("a command of 255 numbers");

        assert_eq!(
            read.args.len(),
            255,
            "the editor writes this count as one byte holding the number of numbers plus one, so \
             255 of them wraps the byte to zero, and reading zero leaves the parse a kilobyte \
             short of where the next command starts"
        );
        assert!(reader.done());
    }

    #[test]
    fn a_command_ending_in_a_byte_the_engine_never_writes_is_refused() {
        let mut raw = fixture::command(101, &[], &["Hello"]);
        *raw.last_mut().expect("the terminator") = 0x42;

        assert!(command(&mut Reader::over(&raw, 0), false).is_err());
    }

    #[test]
    fn the_newest_editor_writes_a_tail_on_every_command_and_it_is_read_past() {
        let mut raw = fixture::command(101, &[], &["Hello"]);
        raw.push(3);
        raw.extend_from_slice(&[9, 9, 9]);

        let mut reader = Reader::over(&raw, 0);
        assert_eq!(
            command(&mut reader, true).expect("a command").said[0].text,
            "Hello"
        );
        assert!(reader.done());

        let mut older = Reader::over(&raw, 0);
        command(&mut older, false).expect("a command an older game wrote");

        assert!(
            !older.done(),
            "an older game holds no tail, so reading one would swallow the head of whatever \
             command comes next"
        );
    }

    #[test]
    fn a_command_with_no_line_in_it_is_not_offered_as_a_place_to_translate() {
        let mut whole = 2u32.to_le_bytes().to_vec();
        whole.extend(fixture::command(121, &[2000299, 0], &[]));
        whole.extend(fixture::command(101, &[], &["Hello"]));

        let mut reader = Reader::over(&whole, 0);
        let mut found = Vec::new();

        commands(&mut reader, false, "e0/p0", &mut found).expect("both commands");

        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].spot, "e0/p0/c1",
            "the index is the one in the file"
        );
    }
}
