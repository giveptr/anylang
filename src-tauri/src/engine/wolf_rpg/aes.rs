const SBOX: [u8; 256] = [
    0x63, 0x7c, 0x77, 0x7b, 0xf2, 0x6b, 0x6f, 0xc5, 0x30, 0x01, 0x67, 0x2b, 0xfe, 0xd7, 0xab, 0x76,
    0xca, 0x82, 0xc9, 0x7d, 0xfa, 0x59, 0x47, 0xf0, 0xad, 0xd4, 0xa2, 0xaf, 0x9c, 0xa4, 0x72, 0xc0,
    0xb7, 0xfd, 0x93, 0x26, 0x36, 0x3f, 0xf7, 0xcc, 0x34, 0xa5, 0xe5, 0xf1, 0x71, 0xd8, 0x31, 0x15,
    0x04, 0xc7, 0x23, 0xc3, 0x18, 0x96, 0x05, 0x9a, 0x07, 0x12, 0x80, 0xe2, 0xeb, 0x27, 0xb2, 0x75,
    0x09, 0x83, 0x2c, 0x1a, 0x1b, 0x6e, 0x5a, 0xa0, 0x52, 0x3b, 0xd6, 0xb3, 0x29, 0xe3, 0x2f, 0x84,
    0x53, 0xd1, 0x00, 0xed, 0x20, 0xfc, 0xb1, 0x5b, 0x6a, 0xcb, 0xbe, 0x39, 0x4a, 0x4c, 0x58, 0xcf,
    0xd0, 0xef, 0xaa, 0xfb, 0x43, 0x4d, 0x33, 0x85, 0x45, 0xf9, 0x02, 0x7f, 0x50, 0x3c, 0x9f, 0xa8,
    0x51, 0xa3, 0x40, 0x8f, 0x92, 0x9d, 0x38, 0xf5, 0xbc, 0xb6, 0xda, 0x21, 0x10, 0xff, 0xf3, 0xd2,
    0xcd, 0x0c, 0x13, 0xec, 0x5f, 0x97, 0x44, 0x17, 0xc4, 0xa7, 0x7e, 0x3d, 0x64, 0x5d, 0x19, 0x73,
    0x60, 0x81, 0x4f, 0xdc, 0x22, 0x2a, 0x90, 0x88, 0x46, 0xee, 0xb8, 0x14, 0xde, 0x5e, 0x0b, 0xdb,
    0xe0, 0x32, 0x3a, 0x0a, 0x49, 0x06, 0x24, 0x5c, 0xc2, 0xd3, 0xac, 0x62, 0x91, 0x95, 0xe4, 0x79,
    0xe7, 0xc8, 0x37, 0x6d, 0x8d, 0xd5, 0x4e, 0xa9, 0x6c, 0x56, 0xf4, 0xea, 0x65, 0x7a, 0xae, 0x08,
    0xba, 0x78, 0x25, 0x2e, 0x1c, 0xa6, 0xb4, 0xc6, 0xe8, 0xdd, 0x74, 0x1f, 0x4b, 0xbd, 0x8b, 0x8a,
    0x70, 0x3e, 0xb5, 0x66, 0x48, 0x03, 0xf6, 0x0e, 0x61, 0x35, 0x57, 0xb9, 0x86, 0xc1, 0x1d, 0x9e,
    0xe1, 0xf8, 0x98, 0x11, 0x69, 0xd9, 0x8e, 0x94, 0x9b, 0x1e, 0x87, 0xe9, 0xce, 0x55, 0x28, 0xdf,
    0x8c, 0xa1, 0x89, 0x0d, 0xbf, 0xe6, 0x42, 0x68, 0x41, 0x99, 0x2d, 0x0f, 0xb0, 0x54, 0xbb, 0x16,
];

const RCON: [u8; 11] = [
    0x8d, 0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80, 0x1b, 0x36,
];

pub const BLOCK: usize = 16;
const ROUNDS: usize = 10;
const WORDS: usize = 4;
const SCHEDULE: usize = BLOCK * (ROUNDS + 1);

fn schedule(key: &[u8; BLOCK]) -> [u8; SCHEDULE] {
    let mut out = [0u8; SCHEDULE];
    out[..BLOCK].copy_from_slice(key);

    for i in WORDS..WORDS * (ROUNDS + 1) {
        let back = (i - 1) * 4;
        let mut hold = [out[back], out[back + 1], out[back + 2], out[back + 3]];

        if i % WORDS == 0 {
            hold.rotate_left(1);

            hold[0] = SBOX[hold[0] as usize] ^ RCON[i / WORDS];
            hold[1] = SBOX[hold[1] as usize] >> 4;
            hold[2] = !SBOX[hold[2] as usize];
            hold[3] = SBOX[hold[3] as usize].rotate_right(7);
        }

        let here = i * 4;
        let from = (i - WORDS) * 4;

        for n in 0..4 {
            out[here + n] = out[from + n] ^ hold[n];
        }
    }

    out
}

fn laid(state: &mut [u8; BLOCK], round: usize, keys: &[u8; SCHEDULE]) {
    for i in 0..BLOCK {
        state[i] ^= keys[round * BLOCK + i];
    }
}

fn subbed(state: &mut [u8; BLOCK]) {
    for one in state.iter_mut() {
        *one = SBOX[*one as usize];
    }
}

fn shifted(state: &mut [u8; BLOCK]) {
    let was = *state;

    state[1] = was[5];
    state[5] = was[9];
    state[9] = was[13];
    state[13] = was[1];

    state[2] = was[10];
    state[10] = was[2];
    state[6] = was[14];
    state[14] = was[6];

    state[3] = was[15];
    state[15] = was[11];
    state[11] = was[7];
    state[7] = was[3];
}

fn twice(x: u8) -> u8 {
    (x << 1) ^ (((x >> 7) & 1) * 0x1b)
}

fn mixed(state: &mut [u8; BLOCK]) {
    for column in state.as_chunks_mut::<4>().0 {
        let first = column[0];
        let all = column[1] ^ column[0] ^ column[2] ^ column[3];

        column[0] ^= all ^ twice(column[1] ^ column[0]);
        column[1] ^= all ^ twice(column[2] ^ column[1]);
        column[2] ^= all ^ twice(column[2] ^ column[3]);
        column[3] ^= all ^ twice(column[3] ^ first);
    }
}

fn sealed(state: &mut [u8; BLOCK], keys: &[u8; SCHEDULE]) {
    laid(state, 0, keys);

    for round in 1..ROUNDS {
        subbed(state);
        shifted(state);
        mixed(state);
        laid(state, round, keys);
    }

    subbed(state);
    shifted(state);
    laid(state, ROUNDS, keys);
}

pub fn streamed(body: &mut [u8], key: &[u8; BLOCK], iv: &[u8; BLOCK], from: u64) {
    if body.is_empty() {
        return;
    }

    let keys = schedule(key);
    let wide = BLOCK as u64;

    let mut walking = u128::from_be_bytes(*iv).wrapping_add(u128::from(from / wide));
    let mut at = (from % wide) as usize;

    let mut state = walking.to_be_bytes();
    sealed(&mut state, &keys);
    walking = walking.wrapping_add(1);

    for one in body.iter_mut() {
        if at == BLOCK {
            state = walking.to_be_bytes();
            sealed(&mut state, &keys);
            walking = walking.wrapping_add(1);

            at = 0;
        }

        *one ^= state[at];
        at += 1;
    }
}

pub fn counted(body: &mut [u8], key: &[u8; BLOCK], iv: &[u8; BLOCK]) {
    streamed(body, key, iv, 0);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn standard(key: &[u8; BLOCK]) -> [u8; SCHEDULE] {
        let mut out = [0u8; SCHEDULE];
        out[..BLOCK].copy_from_slice(key);

        for i in WORDS..WORDS * (ROUNDS + 1) {
            let back = (i - 1) * 4;
            let mut hold = [out[back], out[back + 1], out[back + 2], out[back + 3]];

            if i % WORDS == 0 {
                hold.rotate_left(1);

                for one in hold.iter_mut() {
                    *one = SBOX[*one as usize];
                }

                hold[0] ^= RCON[i / WORDS];
            }

            let here = i * 4;
            let from = (i - WORDS) * 4;

            for n in 0..4 {
                out[here + n] = out[from + n] ^ hold[n];
            }
        }

        out
    }

    #[test]
    fn the_key_schedule_wolf_leans_on_bends_three_bytes_of_every_fourth_word() {
        let key = b"fc06fabb4e6db2f3";
        let ours = schedule(key);
        let theirs = standard(key);

        assert_eq!(&ours[..BLOCK], key, "the first round key is the key itself");
        assert_eq!(
            ours[BLOCK], theirs[BLOCK],
            "the byte the standard salts with Rcon is left alone"
        );

        let bent = (BLOCK + 1..BLOCK + 4)
            .filter(|at| ours[*at] != theirs[*at])
            .count();

        assert_eq!(bent, 3, "the other three bytes of that word are bent");
        assert_ne!(ours[SCHEDULE - 1], theirs[SCHEDULE - 1]);
    }

    #[test]
    fn each_block_of_a_counter_run_gets_a_keystream_of_its_own() {
        let mut run = vec![0u8; 32];
        counted(&mut run, b"fc06fabb4e6db2f3", b"9ef324770d3be914");

        assert_ne!(
            run[..BLOCK],
            run[BLOCK..],
            "each block gets its own keystream"
        );
    }

    #[test]
    fn a_run_started_partway_along_lands_on_the_same_keystream_the_whole_run_would_have() {
        let mut whole = vec![0u8; 96];
        counted(&mut whole, b"fc06fabb4e6db2f3", b"9ef324770d3be914");

        for from in [0u64, 1, 15, 16, 17, 40, 64, 95] {
            let mut part = vec![0u8; whole.len() - from as usize];
            streamed(&mut part, b"fc06fabb4e6db2f3", b"9ef324770d3be914", from);

            assert_eq!(
                part,
                whole[from as usize..],
                "a newer archive is unsealed one read at a time out of the middle of two runs the \
                 counter walks straight through, so starting partway along has to land on the very \
                 byte of the keystream that byte would have got"
            );
        }
    }

    #[test]
    fn the_counter_walked_far_enough_to_carry_out_of_its_own_last_byte_keeps_walking() {
        let once = [0u8; BLOCK];
        let mut first = vec![0u8; BLOCK];
        let mut far = vec![0u8; BLOCK];

        streamed(&mut first, b"fc06fabb4e6db2f3", &once, 0);
        streamed(&mut far, b"fc06fabb4e6db2f3", &once, 256 * BLOCK as u64);

        assert_ne!(
            first, far,
            "the walk the reference does carries out of the last byte into the one before it, so a \
             counter kept in one byte would wrap after 256 blocks and hand this read the very \
             keystream the file opened with"
        );
    }
}
