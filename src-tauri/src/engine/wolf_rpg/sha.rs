const START: [u64; 8] = [
    0x123456789abcdef0,
    0xfedcba9876543210,
    0x0f1e2d3c4b5a6978,
    0x89abcdef01234567,
    0x13579bdf02468ace,
    0xf0e1d2c3b4a59687,
    0x5a6b7c8d9e0f1a2b,
    0x1a2b3c4d5e6f7890,
];

const SALTING: u64 = 0x123456789abcdef0;

const ROUNDS: [u64; 80] = [
    0x428a2f98d728ae22,
    0x7137449123ef65cd,
    0xb5c0fbcfec4d3b2f,
    0xe9b5dba58189dbbc,
    0x3956c25bf348b538,
    0x59f111f1b605d019,
    0x923f82a4af194f9b,
    0xab1c5ed5da6d8118,
    0xd807aa98a3030242,
    0x12835b0145706fbe,
    0x243185be4ee4b28c,
    0x550c7dc3d5ffb4e2,
    0x72be5d74f27b896f,
    0x80deb1fe3b1696b1,
    0x9bdc06a725c71235,
    0xc19bf174cf692694,
    0xe49b69c19ef14ad2,
    0xefbe4786384f25e3,
    0x0fc19dc68b8cd5b5,
    0x240ca1cc77ac9c65,
    0x2de92c6f592b0275,
    0x4a7484aa6ea6e483,
    0x5cb0a9dcbd41fbd4,
    0x76f988da831153b5,
    0x983e5152ee66dfab,
    0xa831c66d2db43210,
    0xb00327c898fb213f,
    0xbf597fc7beef0ee4,
    0xc6e00bf33da88fc2,
    0xd5a79147930aa725,
    0x06ca6351e003826f,
    0x142929670a0e6e70,
    0x27b70a8546d22ffc,
    0x2e1b21385c26c926,
    0x4d2c6dfc5ac42aed,
    0x53380d139d95b3df,
    0x650a73548baf63de,
    0x766a0abb3c77b2a8,
    0x81c2c92e47edaee6,
    0x92722c851482353b,
    0xa2bfe8a14cf10364,
    0xa81a664bbc423001,
    0xc24b8b70d0f89791,
    0xc76c51a30654be30,
    0xd192e819d6ef5218,
    0xd69906245565a910,
    0xf40e35855771202a,
    0x106aa07032bbd1b8,
    0x19a4c116b8d2d0c8,
    0x1e376c085141ab53,
    0x2748774cdf8eeb99,
    0x34b0bcb5e19b48a8,
    0x391c0cb3c5c95a63,
    0x4ed8aa4ae3418acb,
    0x5b9cca4f7763e373,
    0x682e6ff3d6b2b8a3,
    0x748f82ee5defb2fc,
    0x78a5636f43172f60,
    0x84c87814a1f0ab72,
    0x8cc702081a6439ec,
    0x90befffa23631e28,
    0xa4506cebde82bde9,
    0xbef9a3f7b2c67915,
    0xc67178f2e372532b,
    0xca273eceea26619c,
    0xd186b8c721c0c207,
    0xeada7dd6cde0eb1e,
    0xf57d4f7fee6ed178,
    0x06f067aa72176fba,
    0x0a637dc5a2c898a6,
    0x113f9804bef90dae,
    0x1b710b35131c471b,
    0x28db77f523047d84,
    0x32caab7b40c72493,
    0x3c9ebe0a15c9bebc,
    0x431d67c49c100d4c,
    0x4cc5d4becb3e42b6,
    0x597f299cfc657e2a,
    0x5fcb6fab3ad6faec,
    0x6c44198c4a475817,
];

fn sig0(x: u64) -> u64 {
    x.rotate_right(1) ^ x.rotate_right(8) ^ (x >> 7)
}

fn sig1(x: u64) -> u64 {
    x.rotate_right(19) ^ x.rotate_right(61) ^ (x >> 6)
}

fn big0(x: u64) -> u64 {
    x.rotate_right(28) ^ x.rotate_right(34) ^ x.rotate_right(39)
}

fn big1(x: u64) -> u64 {
    x.rotate_right(14) ^ x.rotate_right(18) ^ x.rotate_right(41)
}

fn pick(x: u64, y: u64, z: u64) -> u64 {
    (x & y) ^ (!x & z)
}

fn most(x: u64, y: u64, z: u64) -> u64 {
    (x & y) ^ (x & z) ^ (y & z)
}

fn spread(said: &[u8]) -> Vec<u64> {
    let bits = said.len() as u64 * 8;
    let blocks = (895u64.wrapping_sub(bits) % 1024 + bits + 129) / 1024;

    let mut out = vec![0u64; (blocks * 16) as usize];
    let mut at = 0usize;

    for word in out.iter_mut() {
        let mut chunk = 0u64;

        for _ in 0..8 {
            chunk <<= 8;

            if at < said.len() {
                chunk |= u64::from(said[at]);
            } else if at == said.len() {
                chunk |= 0x80;
            }

            at += 1;
        }

        *word = chunk;
    }

    let last = out.len() - 1;
    out[last - 1] = 0;
    out[last] = bits;

    out
}

pub fn letters(said: &[u8]) -> String {
    let laid = spread(said);
    let mut hold = START;

    for block in laid.chunks(16) {
        let mut w = [0u64; 80];
        w[..16].copy_from_slice(block);

        for j in 16..80 {
            w[j] = w[j - 16]
                .wrapping_add(sig0(w[j - 15]))
                .wrapping_add(w[j - 7])
                .wrapping_add(sig1(w[j - 2]));
        }

        let mut s = hold;

        for j in 0..80 {
            let first = s[7]
                .wrapping_add(big1(s[4]))
                .wrapping_add((s[4] >> 3) ^ pick(s[4], s[5], s[6]))
                .wrapping_add(ROUNDS[j])
                .wrapping_add(w[j]);

            let second = big0(s[0]).wrapping_add(most(s[0], s[1], s[2]));

            s[7] = s[6];
            s[6] = s[5];
            s[5] = s[4];
            s[4] = s[3].wrapping_add(first);
            s[3] = s[2];
            s[2] = s[1];
            s[1] = s[0];
            s[0] = first.wrapping_add(second);
        }

        for (held, one) in hold.iter_mut().zip(s) {
            *held = held.wrapping_add(one ^ SALTING);
        }
    }

    hold.iter().map(|one| format!("{one:016x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_password_hashes_the_way_the_editor_does() {
        assert_eq!(
            letters(b"DBase4"),
            "5f12f1cb00dc69e5d2b3f7ef6575e4b2f0c6e0d42ff37fc441f4c810741f4b381b9cd7a637204b6d36d34376684c4b1a00fb2e66604564a4ad20375d631b5f52"
        );
    }

    #[test]
    fn a_salted_password_hashes_the_way_the_editor_does() {
        assert_eq!(
            letters(&[1, 2, 3, 4, b'D', b'B', b'a', b's', b'e', b'4']),
            "121dd6fcf2bc6b808fc29b4c09a13aae5bb8273e10363447538551c1f92e5541880820182b08c2ed508d6db9119a02578eeb7441e669fe8e03686f47e2f67bca"
        );
        assert_eq!(
            letters(b"abcdbasicD1"),
            "bc34488c89fbbdd8511649bf0b8e34acd5ad98d323e757a1899e92d683e1afe4a244127345658817b2ac1d6433797ebc354100b45237af1e7979ac22c770e4b0"
        );
    }

    #[test]
    fn a_password_long_enough_to_fill_a_second_block_carries_the_first_one_into_it() {
        assert_eq!(
            letters(&[1u8; 112]),
            "9133e837ceb27cb9967e4fd322c081d08bb5cc6c356837dfa5121c7a77932d4105160482b4333c8ab1e4cc4c773645e8bd3d949e14f091f497eed07051b255b3",
            "the editor folds each block into what the last one left, so a reader that hashed only \
             the first block, or started the second one over, would cut a key the game was never \
             sealed with"
        );
        assert_eq!(
            letters(&[0u8; 200]),
            "3cca254820f55cba93be5e9b5c8e8ce07537aa1310b7d0736d379d238807cf72cfbb415780a69ffe589c4faf0ef8ac0f1a5dd7bad17b17a2544a0860fe802027",
            "and the length written into the tail of the last block is the whole password, not \
             what is left of it"
        );
    }

    #[test]
    fn every_digest_is_hex_the_whole_way_and_long_enough_to_cut_a_key_and_a_nonce_from() {
        for said in [b"".as_slice(), &[0u8; 200], &[0xFFu8; 63], &[1u8; 64]] {
            let held = letters(said);

            assert!(
                held.len() >= 89,
                "the key is cut from letter 12 and the nonce from letter 73, so a digest shorter \
                 than 89 letters is a slice that panics: {}",
                held.len()
            );
            assert!(
                held.chars().all(|one| one.is_ascii_hexdigit()),
                "the editor salts and slices this digest as text, so a letter that is not a hex \
                 digit is a different key than the game was sealed with: {held}"
            );
        }
    }
}
