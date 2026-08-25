use std::fs::File;
use std::io::{BufReader, Cursor, Read, Seek, SeekFrom};
use std::ops::Range;
use std::path::Path;

const MAGIC: [u8; 7] = *b"RGSSAD\0";
const VERSION: u8 = 3;
const SEED: Range<usize> = 8..12;

#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    pub at: usize,
    pub size: usize,
    pub key: u32,
    pub told: Range<usize>,
}

const LONGEST_NAME: usize = 4096;

pub fn entries(bytes: &[u8]) -> Result<Vec<Entry>, String> {
    listed(&mut Cursor::new(bytes))
}

pub fn opened(at: &Path) -> Result<Vec<Entry>, String> {
    let file = File::open(at).map_err(|why| format!("reading {}: {why}", at.display()))?;

    listed(&mut BufReader::new(file))
}

pub fn read(at: &Path, entry: &Entry) -> Result<Vec<u8>, String> {
    let mut file = File::open(at).map_err(|why| format!("opening {}: {why}", at.display()))?;
    file.seek(SeekFrom::Start(entry.at as u64))
        .map_err(|why| format!("reaching {} inside {}: {why}", entry.name, at.display()))?;

    let mut raw = vec![0u8; entry.size];
    file.read_exact(&mut raw)
        .map_err(|why| format!("reading {} out of {}: {why}", entry.name, at.display()))?;

    Ok(turned(&raw, entry.key))
}

pub fn listed(source: &mut impl Read) -> Result<Vec<Entry>, String> {
    let mut head = [0u8; SEED.end];
    source
        .read_exact(&mut head)
        .map_err(|_| "no RGSSAD header".to_string())?;

    if head[..MAGIC.len()] != MAGIC {
        return Err("no RGSSAD header".to_string());
    }

    match head[MAGIC.len()] {
        VERSION => {}
        other => return Err(format!("RGSSAD version {other} is not {VERSION}")),
    }

    let key = word(&head, SEED.start)
        .ok_or_else(|| "no key".to_string())?
        .wrapping_mul(9)
        .wrapping_add(3);

    let mut found = Vec::new();
    let mut at = SEED.end;

    loop {
        let told = at;
        let mut block = [0u8; 16];
        source
            .read_exact(&mut block)
            .map_err(|_| "a half written entry".to_string())?;
        at += block.len();

        let mut taken = [0u32; 4];
        for (which, slot) in taken.iter_mut().enumerate() {
            *slot =
                word(&block, which * 4).ok_or_else(|| "a half written entry".to_string())? ^ key;
        }

        let [offset, size, own, length] = taken;
        if offset == 0 {
            return Ok(found);
        }

        let length = length as usize;
        if length > LONGEST_NAME {
            return Err(format!(
                "a name of {length} bytes is longer than any file in an archive is named"
            ));
        }

        let mut spelled = vec![0u8; length];
        source
            .read_exact(&mut spelled)
            .map_err(|_| "a name past the end".to_string())?;
        for (which, byte) in spelled.iter_mut().enumerate() {
            *byte ^= key.to_le_bytes()[which % 4];
        }
        at += length;

        let name = String::from_utf8_lossy(&spelled).replace('\\', "/");

        found.push(Entry {
            name,
            at: offset as usize,
            size: size as usize,
            key: own,
            told: told..at,
        });
    }
}

pub fn body(bytes: &[u8], entry: &Entry) -> Vec<u8> {
    let end = entry.at.saturating_add(entry.size).min(bytes.len());
    let held = bytes.get(entry.at..end).unwrap_or_default();

    turned(held, entry.key)
}

pub fn head(bytes: &[u8], entry: &Entry, most: usize) -> (Vec<u8>, bool) {
    let want = entry.size.min(most);
    let end = entry.at.saturating_add(want).min(bytes.len());
    let held = bytes.get(entry.at..end).unwrap_or_default();

    let whole = held.len() == entry.size;
    (turned(held, entry.key), whole)
}

pub fn patched(bytes: &[u8], edits: &[(&Entry, Vec<u8>)]) -> Result<Vec<u8>, String> {
    let key = word(bytes, SEED.start)
        .ok_or_else(|| "no key".to_string())?
        .wrapping_mul(9)
        .wrapping_add(3);

    let mut out = bytes.to_vec();

    for (entry, body) in edits {
        let at = out.len();
        if at
            .checked_add(body.len())
            .is_none_or(|end| end > u32::MAX as usize)
        {
            return Err("the archive cannot grow past 4 GiB".to_string());
        }

        out.extend_from_slice(&turned(body, entry.key));

        for (which, value) in [at as u32, body.len() as u32].into_iter().enumerate() {
            let told = entry.told.start + 4 * which;
            let Some(slot) = out.get_mut(told..told.saturating_add(4)) else {
                return Err(format!("{} is listed past the archive", entry.name));
            };

            slot.copy_from_slice(&(value ^ key).to_le_bytes());
        }
    }

    Ok(out)
}

pub fn turned(body: &[u8], key: u32) -> Vec<u8> {
    let mut out = body.to_vec();
    let mut key = key;

    for block in out.chunks_mut(4) {
        for (which, byte) in block.iter_mut().enumerate() {
            *byte ^= key.to_le_bytes()[which];
        }
        key = key.wrapping_mul(7).wrapping_add(3);
    }

    out
}

fn word(bytes: &[u8], at: usize) -> Option<u32> {
    let taken: [u8; 4] = bytes.get(at..at + 4)?.try_into().ok()?;

    Some(u32::from_le_bytes(taken))
}

#[cfg(test)]
pub fn packed(files: &[(&str, &[u8])]) -> Vec<u8> {
    tests::packed(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    pub fn packed(files: &[(&str, &[u8])]) -> Vec<u8> {
        let seed = 0x1234_5678u32;
        let key = seed.wrapping_mul(9).wrapping_add(3);

        let mut table = Vec::new();
        let mut bodies = Vec::new();
        let head =
            MAGIC.len() + 1 + 4 + files.iter().map(|(name, _)| 16 + name.len()).sum::<usize>() + 16;

        for (which, (name, body)) in files.iter().enumerate() {
            let own = 0xdead_0000 + which as u32;
            let at = head + bodies.len();

            for value in [at as u32, body.len() as u32, own, name.len() as u32] {
                table.extend_from_slice(&(value ^ key).to_le_bytes());
            }
            table.extend(
                name.as_bytes()
                    .iter()
                    .enumerate()
                    .map(|(step, byte)| byte ^ key.to_le_bytes()[step % 4]),
            );

            bodies.extend(turned(body, own));
        }

        for _ in 0..4 {
            table.extend_from_slice(&key.to_le_bytes());
        }

        let mut out = MAGIC.to_vec();
        out.push(VERSION);
        out.extend_from_slice(&seed.to_le_bytes());
        out.extend_from_slice(&table);
        out.extend_from_slice(&bodies);

        out
    }

    struct Counted<'a> {
        inner: Cursor<&'a [u8]>,
        read: usize,
    }

    impl Read for Counted<'_> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let many = self.inner.read(buf)?;
            self.read += many;

            Ok(many)
        }
    }

    #[test]
    fn the_table_is_read_without_pulling_the_whole_archive_along() {
        let body = vec![7u8; 4 * 1024 * 1024];
        let raw = packed(&[("Graphics/Pictures/title.png", &body)]);
        let mut held = Counted {
            inner: Cursor::new(&raw),
            read: 0,
        };

        let found = listed(&mut held).expect("an archive");

        assert_eq!(found.len(), 1, "the one file in it has to be listed");
        assert!(
            held.read < 4096,
            "the table sits at the head of the archive, so listing a game whose archive is \
             hundreds of megabytes may not read all of it: {} bytes read of {}",
            held.read,
            raw.len()
        );
    }

    #[test]
    fn an_archive_whose_last_file_was_cut_short_still_hands_over_everything_before_it() {
        let raw = packed(&[
            ("Data\\Map001.rvdata2", &[4, 8, 111, 1, 2, 3]),
            ("Data\\Map002.rvdata2", &[4, 8, 1, 2, 3, 4, 5, 6]),
        ]);
        let cut = &raw[..raw.len() - 3];

        let found = entries(cut).expect("a listing");
        assert_eq!(
            found.len(),
            2,
            "the game itself seeks to a file and reads what is there, so a download that lost \
             its last bytes is still an archive with everything before them in it"
        );
        assert_eq!(
            body(cut, &found[0]),
            [4, 8, 111, 1, 2, 3],
            "the file that is whole comes back whole"
        );
        assert_eq!(
            body(cut, &found[1]).len(),
            5,
            "and the one that was cut hands over what is left of it rather than nothing"
        );
    }

    #[test]
    fn every_file_comes_back_out_of_the_archive_it_went_into() {
        let raw = packed(&[
            ("Data\\Map001.rvdata2", &[4, 8, 111, 1, 2, 3]),
            ("Graphics\\Faces\\Actor1.png", &[137, 80, 78, 71]),
        ]);

        let found = entries(&raw).expect("an archive");
        let names: Vec<&str> = found.iter().map(|one| one.name.as_str()).collect();

        assert_eq!(
            names,
            ["Data/Map001.rvdata2", "Graphics/Faces/Actor1.png"],
            "a name is stored with backslashes and has to read as a path"
        );
        assert_eq!(body(&raw, &found[0]), [4, 8, 111, 1, 2, 3]);
        assert_eq!(body(&raw, &found[1]), [137, 80, 78, 71]);
    }

    #[test]
    fn a_longer_file_is_appended_and_only_its_own_entry_moves() {
        let raw = packed(&[
            ("Data\\Map001.rvdata2", &[4, 8, 1]),
            ("Data\\Map002.rvdata2", &[4, 8, 2]),
        ]);
        let found = entries(&raw).expect("an archive");

        let fatter = vec![4u8, 8, 9, 9, 9, 9, 9, 9, 9, 9];
        let fresh = patched(&raw, &[(&found[0], fatter.clone())]).expect("a patched archive");
        let after = entries(&fresh).expect("it still reads as an archive");

        assert_eq!(
            body(&fresh, &after[0]),
            fatter,
            "the sheet that grew has to come back whole"
        );
        assert_eq!(
            body(&fresh, &after[1]),
            [4, 8, 2],
            "its neighbour was never rewritten"
        );
        assert_eq!(
            after[1].at, found[1].at,
            "nothing before the appended bytes may shift"
        );
        assert_eq!(fresh.len(), raw.len() + fatter.len());

        let differs: Vec<usize> = (0..raw.len()).filter(|&at| fresh[at] != raw[at]).collect();
        assert!(
            !differs.is_empty(),
            "the entry has to point at the new bytes"
        );
        assert!(
            differs.iter().all(|at| found[0].told.contains(at)),
            "an export rewrites where one entry points and touches nothing else: {differs:?}"
        );
    }

    #[test]
    fn the_same_pass_that_reads_a_file_writes_it() {
        let plain = b"Marshal bytes go here".to_vec();
        let key = 0x0bad_f00d;

        assert_eq!(
            turned(&turned(&plain, key), key),
            plain,
            "the cipher is one xor, so export needs no second routine"
        );
    }

    #[test]
    fn an_archive_from_another_maker_is_refused_rather_than_guessed_at() {
        let mut older = MAGIC.to_vec();
        older.push(1);
        older.extend_from_slice(&[0; 8]);

        assert!(
            entries(&older).is_err(),
            "XP archives are a different cipher"
        );
        assert!(entries(b"not an archive at all").is_err());
    }
}
