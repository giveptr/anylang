use crate::engine::unity::cursor::At;
use crate::engine::unity::serial;
use anyhow::{Context, Result, anyhow, bail};

pub const MAGIC: &[u8] = b"UnityFS";

const BLOCK: usize = 128 * 1024;
const COMPRESSION: u32 = 0x3f;
const LZMA: u32 = 1;
const LISTING_AT_END: u32 = 0x80;
const PAD_BEFORE_BLOCKS: u32 = 0x200;
const SERIALIZED: u32 = 4;

pub struct Node {
    pub name: String,
    pub body: Vec<u8>,
    flags: u32,
}

pub struct Bundle {
    pub nodes: Vec<Node>,
    pub crc: u32,
    pub revision: String,
    version: u32,
    unity: String,
    flags: u32,
    hash: Vec<u8>,
    how: u32,
    padded: bool,
}

pub struct Packed {
    pub bytes: Vec<u8>,
    pub crc: u32,
}

struct Listing {
    version: u32,
    unity: String,
    revision: String,
    flags: u32,
    hash: Vec<u8>,
    how: u32,
    plan: Vec<(usize, usize, u32)>,
    wanted: Vec<(usize, usize, u32, String)>,
    at: usize,
    padded: bool,
}

fn holds_objects(head: &[u8], size: usize, flags: u32) -> bool {
    flags & SERIALIZED != 0 || serial::announces_itself(head, size as u64)
}

fn head_at(raw: &[u8], held: &Listing, from: usize, want: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(want);
    let mut plain_at = 0usize;
    let mut packed_at = held.at;

    for (plain, packed, how) in &held.plan {
        let span = plain_at..plain_at + plain;
        let at = packed_at;
        plain_at = span.end;
        packed_at += packed;

        if span.end <= from {
            continue;
        }
        if out.len() >= want {
            break;
        }

        let Some(slice) = raw.get(at..at + packed) else {
            break;
        };
        let Ok(block) = unpack(how & COMPRESSION, slice, *plain) else {
            break;
        };
        let Some(rest) = block.get(from.saturating_sub(span.start)..) else {
            break;
        };

        let room = want - out.len();
        out.extend_from_slice(&rest[..rest.len().min(room)]);
    }

    out
}

pub fn open(raw: &[u8]) -> Result<Bundle> {
    let held = listing_of(raw)?;

    let keeping: Vec<usize> = (0..held.wanted.len())
        .filter(|which| {
            let (from, size, flags, _) = &held.wanted[*which];

            holds_objects(&head_at(raw, &held, *from, serial::HEAD), *size, *flags)
        })
        .collect();

    built(raw, held, &keeping, false)
}

pub fn read(raw: &[u8]) -> Result<Bundle> {
    let held = listing_of(raw)?;
    let keeping: Vec<usize> = (0..held.wanted.len()).collect();

    built(raw, held, &keeping, true)
}

fn built(raw: &[u8], held: Listing, keeping: &[usize], sealed: bool) -> Result<Bundle> {
    let (nodes, crc) = drawn(raw, &held, keeping, sealed)?;

    Ok(Bundle {
        nodes,
        crc,
        revision: held.revision,
        version: held.version,
        unity: held.unity,
        flags: held.flags,
        hash: held.hash,
        how: held.how,
        padded: held.padded,
    })
}

fn drawn(raw: &[u8], held: &Listing, keeping: &[usize], sealed: bool) -> Result<(Vec<Node>, u32)> {
    let whole: usize = held.plan.iter().map(|(plain, _, _)| plain).sum();

    let mut out = Vec::with_capacity(keeping.len());
    for which in keeping {
        let (from, size, flags, name) = &held.wanted[*which];
        from.checked_add(*size)
            .filter(|end| *end <= whole)
            .with_context(|| format!("{name} reaches past the bundle"))?;

        out.push(Node {
            name: name.clone(),
            body: Vec::with_capacity((*size).min(raw.len())),
            flags: *flags,
        });
    }

    let mut crc = crc32fast::Hasher::new();
    let mut at = At::new(raw.get(held.at..).unwrap_or_default());
    let mut seen = 0usize;

    for (plain, packed, how) in &held.plan {
        let span = seen..seen + plain;
        seen = span.end;

        let touching: Vec<usize> = (0..keeping.len())
            .filter(|which| {
                let (from, size, ..) = &held.wanted[keeping[*which]];
                *from < span.end && span.start < from + size
            })
            .collect();

        if !sealed && touching.is_empty() {
            at.take(*packed)?;
            continue;
        }

        let block = unpack(how & COMPRESSION, at.take(*packed)?, *plain)?;
        if block.len() != *plain {
            bail!(
                "a block unpacked to {} byte(s) where {plain} were promised",
                block.len()
            );
        }
        if sealed {
            crc.update(&block);
        }

        for which in touching {
            let (from, size, ..) = &held.wanted[keeping[which]];
            let want = (*from).max(span.start)..(from + size).min(span.end);

            out[which]
                .body
                .extend_from_slice(&block[want.start - span.start..want.end - span.start]);
        }
    }

    Ok((out, crc.finalize()))
}

fn listing_of(raw: &[u8]) -> Result<Listing> {
    match read_listing(raw, false) {
        Ok(held) => Ok(held),
        Err(why) => read_listing(raw, true).map_err(|_| why),
    }
}

fn read_listing(raw: &[u8], padded: bool) -> Result<Listing> {
    let mut at = At::new(raw);

    let signature = at.zero_ended()?;
    if signature.as_bytes() != MAGIC {
        bail!("{signature} is not a Unity bundle");
    }

    let version = at.big32()?;
    let unity = at.zero_ended()?;
    let revision = at.zero_ended()?;

    let _size = at.big64()?;
    let packed = at.big32()? as usize;
    let plain = at.big32()? as usize;
    let flags = at.big32()?;

    if version >= 7 || padded {
        at.align(16);
    }

    if flags & LISTING_AT_END != 0 {
        bail!(
            "this bundle keeps its block listing at the end, which this reader cannot handle yet"
        );
    }

    let listing = unpack(flags & COMPRESSION, at.take(packed)?, plain)?;

    if flags & PAD_BEFORE_BLOCKS != 0 {
        at.align(16);
    }

    let mut list = At::new(&listing);
    let hash = list.take(16)?.to_vec();

    let blocks = list.big32()?;
    let mut plan = Vec::with_capacity((blocks as usize).min(4096));
    for _ in 0..blocks {
        let plain = list.big32()? as usize;
        let packed = list.big32()? as usize;
        let how = list.big16()? as u32;
        plan.push((plain, packed, how));
    }

    let count = list.big32()?;
    let mut wanted = Vec::with_capacity((count as usize).min(4096));
    for _ in 0..count {
        let from = list.big64()? as usize;
        let size = list.big64()? as usize;
        let flags = list.big32()?;
        wanted.push((from, size, flags, list.zero_ended()?));
    }

    let how = plan
        .iter()
        .map(|(_, _, how)| how & COMPRESSION)
        .find(|how| *how != 0)
        .unwrap_or(0);

    Ok(Listing {
        version,
        unity,
        revision,
        flags,
        hash,
        how,
        plan,
        wanted,
        at: at.seen,
        padded,
    })
}

impl Bundle {
    #[cfg(test)]
    pub fn serialized(&self) -> Vec<usize> {
        self.nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| {
                let head = node.body.get(..serial::HEAD).unwrap_or(&node.body);

                holds_objects(head, node.body.len(), node.flags)
            })
            .map(|(which, _)| which)
            .collect()
    }

    pub fn pack(&self) -> Result<Packed> {
        let mut crc = crc32fast::Hasher::new();
        let mut squeezed = Vec::new();
        let mut sizes: Vec<(u32, u32, u32)> = Vec::new();

        self.each_block(|block| {
            crc.update(block);

            match worth_packing(written_as(self.how), block)? {
                Some(held) => {
                    sizes.push((block.len() as u32, held.len() as u32, written_as(self.how)));
                    squeezed.extend_from_slice(&held);
                }
                None => {
                    sizes.push((block.len() as u32, block.len() as u32, 0));
                    squeezed.extend_from_slice(block);
                }
            }

            Ok(())
        })?;

        let mut listing = Vec::new();
        listing.extend_from_slice(&self.hash);
        listing.extend_from_slice(&(sizes.len() as u32).to_be_bytes());

        for (plain, packed, how) in &sizes {
            listing.extend_from_slice(&plain.to_be_bytes());
            listing.extend_from_slice(&packed.to_be_bytes());
            listing.extend_from_slice(&(*how as u16).to_be_bytes());
        }

        listing.extend_from_slice(&(self.nodes.len() as u32).to_be_bytes());

        let mut at = 0i64;
        for node in &self.nodes {
            listing.extend_from_slice(&at.to_be_bytes());
            listing.extend_from_slice(&(node.body.len() as i64).to_be_bytes());
            listing.extend_from_slice(&node.flags.to_be_bytes());
            listing.extend_from_slice(node.name.as_bytes());
            listing.push(0);

            at += node.body.len() as i64;
        }

        let held = written_as(self.flags & COMPRESSION);
        let told = squeeze(held, &listing)?;

        let mut out = Vec::with_capacity(squeezed.len() + told.len() + 128);
        out.extend_from_slice(MAGIC);
        out.push(0);
        out.extend_from_slice(&self.version.to_be_bytes());
        for text in [&self.unity, &self.revision] {
            out.extend_from_slice(text.as_bytes());
            out.push(0);
        }

        let size_at = out.len();
        out.extend_from_slice(&0i64.to_be_bytes());
        out.extend_from_slice(&(told.len() as u32).to_be_bytes());
        out.extend_from_slice(&(listing.len() as u32).to_be_bytes());
        out.extend_from_slice(&((self.flags & !COMPRESSION) | held).to_be_bytes());

        if self.version >= 7 || self.padded {
            while !out.len().is_multiple_of(16) {
                out.push(0);
            }
        }

        out.extend_from_slice(&told);

        if self.flags & PAD_BEFORE_BLOCKS != 0 {
            while !out.len().is_multiple_of(16) {
                out.push(0);
            }
        }

        out.extend_from_slice(&squeezed);

        let whole = out.len() as i64;
        out[size_at..size_at + 8].copy_from_slice(&whole.to_be_bytes());

        Ok(Packed {
            bytes: out,
            crc: crc.finalize(),
        })
    }

    fn each_block(&self, mut take: impl FnMut(&[u8]) -> Result<()>) -> Result<()> {
        let mut held: Vec<u8> = Vec::with_capacity(BLOCK);
        let mut given = 0usize;

        for node in &self.nodes {
            let mut rest = node.body.as_slice();

            while !rest.is_empty() {
                if held.is_empty() && rest.len() >= BLOCK {
                    let (now, later) = rest.split_at(BLOCK);
                    take(now)?;
                    given += 1;
                    rest = later;
                    continue;
                }

                let (now, later) = rest.split_at((BLOCK - held.len()).min(rest.len()));
                held.extend_from_slice(now);
                rest = later;

                if held.len() == BLOCK {
                    take(&held)?;
                    given += 1;
                    held.clear();
                }
            }
        }

        if !held.is_empty() || given == 0 {
            take(&held)?;
        }

        Ok(())
    }
}

fn unpack(how: u32, packed: &[u8], plain: usize) -> Result<Vec<u8>> {
    match how {
        0 => Ok(packed.to_vec()),
        1 => unlzma(packed, plain),
        2 | 3 => unlz4(packed, plain),
        other => bail!(
            "this file is packed with bundle compression {other}, which this reader cannot unpack \
             yet"
        ),
    }
}

fn written_as(how: u32) -> u32 {
    match how {
        LZMA => 0,
        other => other,
    }
}

fn unlzma(packed: &[u8], plain: usize) -> Result<Vec<u8>> {
    let mut room: Vec<u8> = Vec::new();
    room.try_reserve_exact(plain).map_err(|_| {
        anyhow!(
            "a block of {} byte(s) says it unpacks to {plain}, which there is no room for",
            packed.len()
        )
    })?;

    lzma_rs::lzma_decompress_with_options(
        &mut &packed[..],
        &mut room,
        &lzma_rs::decompress::Options {
            unpacked_size: lzma_rs::decompress::UnpackedSize::UseProvided(Some(plain as u64)),
            memlimit: None,
            allow_incomplete: false,
        },
    )
    .map_err(|why| anyhow!("this LZMA block does not unpack: {why}"))?;

    if room.len() != plain {
        bail!(
            "an LZMA block said it holds {plain} byte(s) and gave up {}",
            room.len()
        );
    }

    Ok(room)
}

fn squeeze(how: u32, plain: &[u8]) -> Result<Vec<u8>> {
    match how {
        0 => Ok(plain.to_vec()),
        2 | 3 => lz4::block::compress(plain, None, false).context("packing an LZ4 block"),
        other => bail!("this writer cannot pack a bundle held with compression {other}"),
    }
}

fn worth_packing(how: u32, plain: &[u8]) -> Result<Option<Vec<u8>>> {
    if how == 0 {
        return Ok(None);
    }

    let held = squeeze(how, plain)?;

    Ok(Some(held).filter(|held| held.len() < plain.len()))
}

fn unlz4(packed: &[u8], plain: usize) -> Result<Vec<u8>> {
    let mut room: Vec<u8> = Vec::new();
    room.try_reserve_exact(plain).map_err(|_| {
        anyhow!(
            "a block of {} byte(s) says it unpacks to {plain}, which there is no room for",
            packed.len()
        )
    })?;
    room.resize(plain, 0);

    let filled = lz4::block::decompress_to_buffer(packed, Some(plain as i32), &mut room)
        .context("unpacking an LZ4 block")?;
    room.truncate(filled);

    Ok(room)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::unity::fake;

    #[test]
    fn a_block_unity_squeezed_with_lzma_is_read_and_written_back_plain() {
        let plain = b"Peter\nShe tilted her head, the gold mask glinting.\n\n".repeat(8);
        let packed = as_unity_lzma(&plain);

        assert_eq!(
            unpack(LZMA, &packed, plain.len()).expect("an LZMA block reads"),
            plain,
            "LZMA is what Unity reaches for by default when it builds an AssetBundle, so a game \
             holding one gives up no text at all until this reads"
        );
        assert!(
            unpack(LZMA, &packed, plain.len() + 1).is_err(),
            "a block that gives up fewer bytes than it promised is one this reader cannot trust"
        );
        assert!(
            unpack(9, b"", 0).is_err(),
            "and a way nobody named is refused"
        );

        assert_eq!(
            written_as(LZMA),
            0,
            "this writer cannot squeeze LZMA the way Unity does, so a bundle it rewrites carries \
             plain blocks instead: bigger on disk, and read by the flags the header now names"
        );
        assert_eq!(written_as(3), 3, "an LZ4 bundle is still written as LZ4");
    }

    fn as_unity_lzma(plain: &[u8]) -> Vec<u8> {
        let mut held = Vec::new();
        lzma_rs::lzma_compress(&mut &plain[..], &mut held).expect("it squeezes");

        let mut out = held[..5].to_vec();
        out.extend_from_slice(&held[13..]);

        out
    }

    #[test]
    fn only_a_unity_bundle_is_opened() {
        assert!(
            open(b"NotUnity\0").is_err(),
            "a game folder is mostly files that are not bundles, and reading one as a bundle \
             would take whatever followed as a block listing"
        );
    }

    fn holding(nodes: Vec<Node>, version: u32, flags: u32, how: u32) -> Bundle {
        let mut whole = crc32fast::Hasher::new();
        for node in &nodes {
            whole.update(&node.body);
        }

        Bundle {
            nodes,
            crc: whole.finalize(),
            version,
            unity: "5.x.x".to_string(),
            revision: "2021.3.1f1".to_string(),
            flags,
            hash: vec![0; 16],
            how,
            padded: false,
        }
    }

    fn forged(version: u32, flags: u32, how: u32) -> Bundle {
        holding(
            vec![
                Node {
                    name: "one.assets".to_string(),
                    body: vec![7u8; BLOCK + 2_000],
                    flags: SERIALIZED,
                },
                Node {
                    name: "two.resS".to_string(),
                    body: b"tiny".to_vec(),
                    flags: 0,
                },
            ],
            version,
            flags,
            how,
        )
    }

    #[test]
    fn a_packed_bundle_reads_back_node_for_node() {
        for (version, flags) in [(6u32, 0u32), (7, PAD_BEFORE_BLOCKS)] {
            for how in [0, 2, 3] {
                let bundle = forged(version, flags | how, how);
                let packed = bundle.pack().expect("a packed bundle");
                let out = &packed.bytes;

                let size_at =
                    MAGIC.len() + 1 + 4 + bundle.unity.len() + 1 + bundle.revision.len() + 1;
                let told = i64::from_be_bytes(out[size_at..size_at + 8].try_into().unwrap());
                assert_eq!(
                    told as usize,
                    out.len(),
                    "the backpatched size has to be the whole file"
                );

                let back = read(out).expect("a bundle this reader accepts");
                assert_eq!(back.version, version);
                assert_eq!(back.nodes.len(), bundle.nodes.len());
                for (before, after) in bundle.nodes.iter().zip(&back.nodes) {
                    assert_eq!(before.name, after.name);
                    assert_eq!(before.body, after.body, "{} came back changed", before.name);
                    assert_eq!(before.flags, after.flags);
                }
                assert_eq!(
                    back.crc, packed.crc,
                    "the check a bundle is sealed with is read out of it the same way it was \
                     written, or the seal beside it would be lifted to a number the game never \
                     computes"
                );
                assert_eq!(
                    back.crc, bundle.crc,
                    "packing changes how the bytes are held, never what they say, so the check \
                     may not move when nothing did"
                );
            }
        }
    }

    #[test]
    fn a_bundle_that_came_in_packed_does_not_go_out_loose() {
        let loose = forged(7, PAD_BEFORE_BLOCKS, 0).pack().expect("a bundle");
        let held = forged(7, PAD_BEFORE_BLOCKS | 3, 3)
            .pack()
            .expect("a bundle");

        assert!(
            held.bytes.len() < loose.bytes.len(),
            "a body of one repeated byte packs away to nothing, so writing it back loose would \
             mean every export grew the game by whatever its compression was worth"
        );
        assert_eq!(
            held.crc, loose.crc,
            "however it is held, the bundle says the same thing"
        );
    }

    #[test]
    fn only_the_node_holding_objects_is_unpacked_when_a_bundle_is_read_to_be_harvested() {
        let bundle = forged(7, PAD_BEFORE_BLOCKS | 3, 3);
        let packed = bundle.pack().expect("a bundle");

        assert_eq!(
            open(&packed.bytes)
                .expect("a bundle this reader accepts")
                .nodes
                .iter()
                .map(|node| node.name.as_str())
                .collect::<Vec<_>>(),
            ["one.assets"],
            "a resS beside a serialized file is bytes the harvest never looks at, and on a real \
             game it is most of the bundle: unpacking it costs seconds per read for nothing"
        );

        let whole = read(&packed.bytes).expect("a bundle this reader accepts");
        assert_eq!(
            whole.nodes.len(),
            2,
            "writing one back still needs every node it came with"
        );
        assert_eq!(
            whole
                .serialized()
                .into_iter()
                .map(|which| whole.nodes[which].name.as_str())
                .collect::<Vec<_>>(),
            ["one.assets"],
            "the write path has to pick the same nodes in the same order as the read path, or a \
             sheet staged against node 0 is written back into some other node"
        );
    }

    #[test]
    fn a_node_is_kept_for_what_it_holds_and_not_only_for_the_flag_it_was_given() {
        let bundle = holding(
            vec![
                Node {
                    name: "unflagged.assets".to_string(),
                    body: fake::forge(&[(11, "one", "Peter\nWait.\n\n")]),
                    flags: 0,
                },
                Node {
                    name: "two.resS".to_string(),
                    body: vec![7u8; 4_096],
                    flags: 0,
                },
            ],
            7,
            PAD_BEFORE_BLOCKS | 3,
            3,
        );
        let packed = bundle.pack().expect("a bundle");

        assert_eq!(
            open(&packed.bytes)
                .expect("a bundle")
                .nodes
                .iter()
                .map(|node| node.name.as_str())
                .collect::<Vec<_>>(),
            ["unflagged.assets"],
            "a serialized file is what it says it is in its own first bytes: leaning on the flag \
             alone would drop every line in a bundle that did not set it, and drop them in \
             silence"
        );
        assert_eq!(
            bundle
                .serialized()
                .into_iter()
                .map(|which| bundle.nodes[which].name.as_str())
                .collect::<Vec<_>>(),
            ["unflagged.assets"],
            "and the write path has to agree with the read path node for node"
        );
    }
}
