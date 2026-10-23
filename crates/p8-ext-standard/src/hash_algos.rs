//! The hash algorithms PHP has that no RustCrypto crate gives as PHP
//! does: tiger (3 and 4 passes, PHP's byte order), snefru, haval (3-5
//! passes, every output size), murmur3 (a, c, f), crc32 (bzip2's) and
//! crc32c. Each hashes a whole message.
//!
//! Reference: $PHP_SRC_PATH/ext/hash/hash_tiger.c, hash_snefru.c,
//! hash_haval.c, hash_murmur.c (murmur/PMurHash*.c), hash_crc32.c

use crate::hash_tables::{SNEFRU, TIGER};

// --- tiger -----------------------------------------------------------------

fn tiger_round(a: &mut u64, b: &mut u64, c: &mut u64, x: u64, mul: u64) {
    *c ^= x;
    let cb = c.to_le_bytes();
    let t = |i: usize, byte: u8| TIGER[i * 256 + usize::from(byte)];
    *a = a.wrapping_sub(t(0, cb[0]) ^ t(1, cb[2]) ^ t(2, cb[4]) ^ t(3, cb[6]));
    *b = b.wrapping_add(t(3, cb[1]) ^ t(2, cb[3]) ^ t(1, cb[5]) ^ t(0, cb[7]));
    *b = b.wrapping_mul(mul);
}

fn tiger_pass(a: &mut u64, b: &mut u64, c: &mut u64, x: &[u64; 8], mul: u64) {
    tiger_round(a, b, c, x[0], mul);
    tiger_round(b, c, a, x[1], mul);
    tiger_round(c, a, b, x[2], mul);
    tiger_round(a, b, c, x[3], mul);
    tiger_round(b, c, a, x[4], mul);
    tiger_round(c, a, b, x[5], mul);
    tiger_round(a, b, c, x[6], mul);
    tiger_round(b, c, a, x[7], mul);
}

fn tiger_schedule(x: &mut [u64; 8]) {
    x[0] = x[0].wrapping_sub(x[7] ^ 0xA5A5_A5A5_A5A5_A5A5);
    x[1] ^= x[0];
    x[2] = x[2].wrapping_add(x[1]);
    x[3] = x[3].wrapping_sub(x[2] ^ ((!x[1]) << 19));
    x[4] ^= x[3];
    x[5] = x[5].wrapping_add(x[4]);
    x[6] = x[6].wrapping_sub(x[5] ^ ((!x[4]) >> 23));
    x[7] ^= x[6];
    x[0] = x[0].wrapping_add(x[7]);
    x[1] = x[1].wrapping_sub(x[0] ^ ((!x[7]) << 19));
    x[2] ^= x[1];
    x[3] = x[3].wrapping_add(x[2]);
    x[4] = x[4].wrapping_sub(x[3] ^ ((!x[2]) >> 23));
    x[5] ^= x[4];
    x[6] = x[6].wrapping_add(x[5]);
    x[7] = x[7].wrapping_sub(x[6] ^ 0x0123_4567_89AB_CDEF);
}

fn tiger_compress(extra_passes: usize, block: &[u8], state: &mut [u64; 3]) {
    let mut x = [0u64; 8];
    for (i, w) in x.iter_mut().enumerate() {
        *w = u64::from_le_bytes(block[i * 8..i * 8 + 8].try_into().expect("8 bytes"));
    }
    let [mut a, mut b, mut c] = *state;
    tiger_pass(&mut a, &mut b, &mut c, &x, 5);
    tiger_schedule(&mut x);
    tiger_pass(&mut c, &mut a, &mut b, &x, 7);
    tiger_schedule(&mut x);
    tiger_pass(&mut b, &mut c, &mut a, &x, 9);
    for _ in 0..extra_passes {
        tiger_schedule(&mut x);
        tiger_pass(&mut a, &mut b, &mut c, &x, 9);
        (a, b, c) = (c, a, b);
    }
    state[0] ^= a;
    state[1] = b.wrapping_sub(state[1]);
    state[2] = c.wrapping_add(state[2]);
}

/// tigerBITS,PASSES: `bits` of the digest, 3 or 4 passes.
pub(crate) fn tiger(data: &[u8], bits: usize, passes: usize) -> Vec<u8> {
    let extra = passes - 3;
    let mut state = [0x0123_4567_89AB_CDEF, 0xFEDC_BA98_7654_3210, 0xF096_A5B4_C3B2_E187];
    let (blocks, rest) = data.as_chunks::<64>();
    for block in blocks {
        tiger_compress(extra, block, &mut state);
    }
    let mut tail = rest.to_vec();
    tail.push(0x01);
    while tail.len() % 64 != 56 {
        tail.push(0);
    }
    tail.extend_from_slice(&((data.len() as u64) << 3).to_le_bytes());
    for block in tail.as_chunks::<64>().0 {
        tiger_compress(extra, block, &mut state);
    }
    state.iter().flat_map(|w| w.to_le_bytes()).take(bits / 8).collect()
}

// --- snefru ----------------------------------------------------------------

fn snefru_rounds(state: &mut [u32; 16]) {
    const SHIFTS: [u32; 4] = [16, 8, 16, 24];
    let mut b = *state;
    for index in 0..8 {
        let t0 = &SNEFRU[2 * index];
        let t1 = &SNEFRU[2 * index + 1];
        for &shift in &SHIFTS {
            for i in 0..16 {
                // round(L = B[i-1], C = B[i], N = B[i+1]) with tables t0 t0 t1 t1 ...
                let t = if (i / 2) % 2 == 0 { t0 } else { t1 };
                let sbe = t[(b[i] & 0xff) as usize];
                b[(i + 15) % 16] ^= sbe;
                b[(i + 1) % 16] ^= sbe;
            }
            for w in &mut b {
                *w = w.rotate_right(shift);
            }
        }
    }
    for i in 0..8 {
        state[i] ^= b[15 - i];
    }
}

fn snefru_block(state: &mut [u32; 16], block: &[u8]) {
    for j in 0..8 {
        state[8 + j] = u32::from_be_bytes(block[j * 4..j * 4 + 4].try_into().expect("4 bytes"));
    }
    snefru_rounds(state);
    state[8..].fill(0);
}

/// snefru (snefru256 is the same algorithm).
pub(crate) fn snefru(data: &[u8]) -> Vec<u8> {
    let mut state = [0u32; 16];
    let (blocks, rest) = data.as_chunks::<32>();
    for block in blocks {
        snefru_block(&mut state, block);
    }
    if !rest.is_empty() {
        let mut last = rest.to_vec();
        last.resize(32, 0);
        snefru_block(&mut state, &last);
    }
    let bits = (data.len() as u64) << 3;
    state[14] = (bits >> 32) as u32;
    state[15] = bits as u32;
    snefru_rounds(&mut state);
    state[..8].iter().flat_map(|w| w.to_be_bytes()).collect()
}

// --- haval -----------------------------------------------------------------

const HAVAL_D0: [u32; 8] = [0x243F6A88, 0x85A308D3, 0x13198A2E, 0x03707344, 0xA4093822, 0x299F31D0, 0x082EFA98, 0xEC4E6C89];
#[rustfmt::skip]
const HAVAL_K: [[u32; 32]; 4] = [
    [0x452821E6, 0x38D01377, 0xBE5466CF, 0x34E90C6C, 0xC0AC29B7, 0xC97C50DD, 0x3F84D5B5, 0xB5470917,
     0x9216D5D9, 0x8979FB1B, 0xD1310BA6, 0x98DFB5AC, 0x2FFD72DB, 0xD01ADFB7, 0xB8E1AFED, 0x6A267E96,
     0xBA7C9045, 0xF12C7F99, 0x24A19947, 0xB3916CF7, 0x0801F2E2, 0x858EFC16, 0x636920D8, 0x71574E69,
     0xA458FEA3, 0xF4933D7E, 0x0D95748F, 0x728EB658, 0x718BCD58, 0x82154AEE, 0x7B54A41D, 0xC25A59B5],
    [0x9C30D539, 0x2AF26013, 0xC5D1B023, 0x286085F0, 0xCA417918, 0xB8DB38EF, 0x8E79DCB0, 0x603A180E,
     0x6C9E0E8B, 0xB01E8A3E, 0xD71577C1, 0xBD314B27, 0x78AF2FDA, 0x55605C60, 0xE65525F3, 0xAA55AB94,
     0x57489862, 0x63E81440, 0x55CA396A, 0x2AAB10B6, 0xB4CC5C34, 0x1141E8CE, 0xA15486AF, 0x7C72E993,
     0xB3EE1411, 0x636FBC2A, 0x2BA9C55D, 0x741831F6, 0xCE5C3E16, 0x9B87931E, 0xAFD6BA33, 0x6C24CF5C],
    [0x7A325381, 0x28958677, 0x3B8F4898, 0x6B4BB9AF, 0xC4BFE81B, 0x66282193, 0x61D809CC, 0xFB21A991,
     0x487CAC60, 0x5DEC8032, 0xEF845D5D, 0xE98575B1, 0xDC262302, 0xEB651B88, 0x23893E81, 0xD396ACC5,
     0x0F6D6FF3, 0x83F44239, 0x2E0B4482, 0xA4842004, 0x69C8F04A, 0x9E1F9B5E, 0x21C66842, 0xF6E96C9A,
     0x670C9C61, 0xABD388F0, 0x6A51A0D2, 0xD8542F68, 0x960FA728, 0xAB5133A3, 0x6EEF0B6C, 0x137A3BE4],
    [0xBA3BF050, 0x7EFB2A98, 0xA1F1651D, 0x39AF0176, 0x66CA593E, 0x82430E88, 0x8CEE8619, 0x456F9FB4,
     0x7D84A5C3, 0x3B8B5EBE, 0xE06F75D8, 0x85C12073, 0x401A449F, 0x56C16AA6, 0x4ED3AA62, 0x363F7706,
     0x1BFEDF72, 0x429B023D, 0x37D0D724, 0xD00A1248, 0xDB0FEAD3, 0x49F1C09B, 0x075372C9, 0x80991B7B,
     0x25D479D8, 0xF6E8DEF7, 0xE3FE501A, 0xB6794C3B, 0x976CE0BD, 0x04C006BA, 0xC1A94FB6, 0x409F60C4],
];
#[rustfmt::skip]
const HAVAL_I: [[usize; 32]; 4] = [
    [5, 14, 26, 18, 11, 28, 7, 16, 0, 23, 20, 22, 1, 10, 4, 8, 30, 3, 21, 9, 17, 24, 29, 6, 19, 12, 15, 13, 2, 25, 31, 27],
    [19, 9, 4, 20, 28, 17, 8, 22, 29, 14, 25, 12, 24, 30, 16, 26, 31, 15, 7, 3, 1, 0, 18, 27, 13, 6, 21, 10, 23, 11, 5, 2],
    [24, 4, 0, 14, 2, 7, 28, 23, 26, 6, 30, 20, 18, 25, 19, 3, 22, 11, 31, 21, 8, 27, 12, 9, 1, 29, 5, 15, 17, 10, 16, 13],
    [27, 3, 21, 26, 17, 11, 20, 29, 19, 0, 12, 7, 13, 8, 31, 10, 5, 9, 14, 30, 18, 6, 28, 24, 2, 23, 16, 22, 4, 1, 25, 15],
];

/// The boolean functions F1..F5, arguments as PHP's macros take them (x6..x0).
fn haval_f(n: usize, [x6, x5, x4, x3, x2, x1, x0]: [u32; 7]) -> u32 {
    match n {
        1 => (x1 & x4) ^ (x2 & x5) ^ (x3 & x6) ^ (x0 & x1) ^ x0,
        2 => (x1 & x2 & x3) ^ (x2 & x4 & x5) ^ (x1 & x2) ^ (x1 & x4) ^ (x2 & x6) ^ (x3 & x5) ^ (x4 & x5) ^ (x0 & x2) ^ x0,
        3 => (x1 & x2 & x3) ^ (x1 & x4) ^ (x2 & x5) ^ (x3 & x6) ^ (x0 & x3) ^ x0,
        4 => {
            (x1 & x2 & x3) ^ (x2 & x4 & x5) ^ (x3 & x4 & x6) ^ (x1 & x4) ^ (x2 & x6) ^ (x3 & x4) ^ (x3 & x5) ^ (x3 & x6) ^ (x4 & x5) ^ (x4 & x6) ^ (x0 & x4) ^ x0
        }
        _ => (x1 & x4) ^ (x2 & x5) ^ (x3 & x6) ^ (x0 & x1 & x2 & x3) ^ (x0 & x5) ^ x0,
    }
}

/// Which of the M0..M6 word orders feed each round's function (x6..x0),
/// per pass count, as PHP's PHP_{3,4,5}HAVALTransform pass them.
#[rustfmt::skip]
const HAVAL_ORDER: [&[[usize; 7]]; 3] = [
    &[[1, 0, 3, 5, 6, 2, 4], [4, 2, 1, 0, 5, 3, 6], [6, 1, 2, 3, 4, 5, 0]],
    &[[2, 6, 1, 4, 5, 3, 0], [3, 5, 2, 0, 1, 6, 4], [1, 4, 3, 6, 0, 2, 5], [6, 4, 0, 5, 2, 1, 3]],
    &[[3, 4, 1, 0, 5, 2, 6], [6, 2, 1, 0, 3, 4, 5], [2, 6, 0, 4, 3, 1, 5], [1, 5, 3, 2, 0, 4, 6], [2, 5, 0, 6, 4, 3, 1]],
];

fn haval_transform(passes: usize, state: &mut [u32; 8], block: &[u8]) {
    let mut x = [0u32; 32];
    for (i, w) in x.iter_mut().enumerate() {
        *w = u32::from_le_bytes(block[i * 4..i * 4 + 4].try_into().expect("4 bytes"));
    }
    let mut e = *state;
    // M_k[i] = (k - i) mod 8 (PHP's M0..M7 tables).
    let m = |k: usize, i: usize| (k + 8 - i % 8) % 8;
    for (round, order) in HAVAL_ORDER[passes - 3].iter().enumerate() {
        for i in 0..32 {
            let f = haval_f(round + 1, std::array::from_fn(|k| e[m(order[k], i)]));
            let word = if round == 0 { x[i] } else { x[HAVAL_I[round - 1][i]].wrapping_add(HAVAL_K[round - 1][i]) };
            e[7 - (i % 8)] = f.rotate_right(7).wrapping_add(e[m(7, i)].rotate_right(11)).wrapping_add(word);
        }
    }
    for (s, v) in state.iter_mut().zip(e) {
        *s = s.wrapping_add(v);
    }
}

/// havalBITS,PASSES.
pub(crate) fn haval(data: &[u8], bits: u32, passes: usize) -> Vec<u8> {
    let mut state = HAVAL_D0;
    let mut msg = data.to_vec();
    let count = (data.len() as u64) << 3;
    msg.push(0x01);
    while msg.len() % 128 != 118 {
        msg.push(0);
    }
    msg.push(0x01 | ((passes as u8 & 0x07) << 3) | ((bits as u8 & 0x03) << 6));
    msg.push((bits >> 2) as u8);
    msg.extend_from_slice(&count.to_le_bytes());
    for block in msg.as_chunks::<128>().0 {
        haval_transform(passes, &mut state, block);
    }
    let s = &mut state;
    match bits {
        128 => {
            s[3] = s[3].wrapping_add((s[7] & 0xFF000000) | (s[6] & 0x00FF0000) | (s[5] & 0x0000FF00) | (s[4] & 0x000000FF));
            s[2] = s[2].wrapping_add((((s[7] & 0x00FF0000) | (s[6] & 0x0000FF00) | (s[5] & 0x000000FF)) << 8) | ((s[4] & 0xFF000000) >> 24));
            s[1] = s[1].wrapping_add((((s[7] & 0x0000FF00) | (s[6] & 0x000000FF)) << 16) | (((s[5] & 0xFF000000) | (s[4] & 0x00FF0000)) >> 16));
            s[0] = s[0].wrapping_add(((s[7] & 0x000000FF) << 24) | (((s[6] & 0xFF000000) | (s[5] & 0x00FF0000) | (s[4] & 0x0000FF00)) >> 8));
        }
        160 => {
            s[4] = s[4].wrapping_add(((s[7] & 0xFE000000) | (s[6] & 0x01F80000) | (s[5] & 0x0007F000)) >> 12);
            s[3] = s[3].wrapping_add(((s[7] & 0x01F80000) | (s[6] & 0x0007F000) | (s[5] & 0x00000FC0)) >> 6);
            s[2] = s[2].wrapping_add((s[7] & 0x0007F000) | (s[6] & 0x00000FC0) | (s[5] & 0x0000003F));
            s[1] = s[1].wrapping_add(((s[7] & 0x00000FC0) | (s[6] & 0x0000003F) | (s[5] & 0xFE000000)).rotate_right(25));
            s[0] = s[0].wrapping_add(((s[7] & 0x0000003F) | (s[6] & 0xFE000000) | (s[5] & 0x01F80000)).rotate_right(19));
        }
        192 => {
            s[5] = s[5].wrapping_add(((s[7] & 0xFC000000) | (s[6] & 0x03E00000)) >> 21);
            s[4] = s[4].wrapping_add(((s[7] & 0x03E00000) | (s[6] & 0x001F0000)) >> 16);
            s[3] = s[3].wrapping_add(((s[7] & 0x001F0000) | (s[6] & 0x0000FC00)) >> 10);
            s[2] = s[2].wrapping_add(((s[7] & 0x0000FC00) | (s[6] & 0x000003E0)) >> 5);
            s[1] = s[1].wrapping_add((s[7] & 0x000003E0) | (s[6] & 0x0000001F));
            s[0] = s[0].wrapping_add(((s[7] & 0x0000001F) | (s[6] & 0xFC000000)).rotate_right(26));
        }
        224 => {
            s[6] = s[6].wrapping_add(s[7] & 0x0000000F);
            s[5] = s[5].wrapping_add((s[7] >> 4) & 0x0000001F);
            s[4] = s[4].wrapping_add((s[7] >> 9) & 0x0000000F);
            s[3] = s[3].wrapping_add((s[7] >> 13) & 0x0000001F);
            s[2] = s[2].wrapping_add((s[7] >> 18) & 0x0000000F);
            s[1] = s[1].wrapping_add((s[7] >> 22) & 0x0000001F);
            s[0] = s[0].wrapping_add((s[7] >> 27) & 0x0000001F);
        }
        _ => {}
    }
    state.iter().flat_map(|w| w.to_le_bytes()).take(bits as usize / 8).collect()
}

// --- murmur3 ---------------------------------------------------------------

fn fmix32(mut h: u32) -> u32 {
    h ^= h >> 16;
    h = h.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2_ae35);
    h ^ (h >> 16)
}

fn fmix64(mut k: u64) -> u64 {
    k ^= k >> 33;
    k = k.wrapping_mul(0xff51_afd7_ed55_8ccd);
    k ^= k >> 33;
    k = k.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    k ^ (k >> 33)
}

/// A little-endian word from up to `n` bytes (the tail of a message).
fn tail_word(bytes: &[u8]) -> u64 {
    bytes.iter().rev().fold(0, |acc, &b| (acc << 8) | u64::from(b))
}

/// murmur3a: MurmurHash3_x86_32.
pub(crate) fn murmur3a(data: &[u8], seed: u32) -> Vec<u8> {
    const C1: u32 = 0xcc9e_2d51;
    const C2: u32 = 0x1b87_3593;
    let mut h = seed;
    let (words, rest) = data.as_chunks::<4>();
    for c in words {
        let k = u32::from_le_bytes(*c).wrapping_mul(C1).rotate_left(15).wrapping_mul(C2);
        h = (h ^ k).rotate_left(13).wrapping_mul(5).wrapping_add(0xe654_6b64);
    }
    if !rest.is_empty() {
        h ^= (tail_word(rest) as u32).wrapping_mul(C1).rotate_left(15).wrapping_mul(C2);
    }
    fmix32(h ^ data.len() as u32).to_be_bytes().to_vec()
}

/// murmur3c: MurmurHash3_x86_128.
pub(crate) fn murmur3c(data: &[u8], seed: u32) -> Vec<u8> {
    const C: [u32; 4] = [0x239b_961b, 0xab0e_9789, 0x38b3_4ae5, 0xa1e3_8b93];
    const R: [u32; 4] = [15, 16, 17, 18];
    const H_ROT: [u32; 4] = [19, 17, 15, 13];
    const H_ADD: [u32; 4] = [0x561c_cd1b, 0x0bca_a747, 0x96cd_1c35, 0x32ac_3b17];
    let mut h = [seed; 4];
    let mix = |i: usize, k: u32| k.wrapping_mul(C[i]).rotate_left(R[i]).wrapping_mul(C[(i + 1) % 4]);
    let (blocks, rest) = data.as_chunks::<16>();
    for c in blocks {
        let k: [u32; 4] = std::array::from_fn(|i| u32::from_le_bytes(c[i * 4..i * 4 + 4].try_into().expect("4 bytes")));
        for i in 0..4 {
            h[i] ^= mix(i, k[i]);
            h[i] = h[i].rotate_left(H_ROT[i]).wrapping_add(h[(i + 1) % 4]).wrapping_mul(5).wrapping_add(H_ADD[i]);
        }
    }
    for i in (0..4).rev() {
        let part = rest.get(i * 4..rest.len().min(i * 4 + 4)).unwrap_or(&[]);
        if !part.is_empty() {
            h[i] ^= mix(i, tail_word(part) as u32);
        }
    }
    let len = data.len() as u32;
    for x in &mut h {
        *x ^= len;
    }
    let sum = |h: &mut [u32; 4]| {
        h[0] = h[0].wrapping_add(h[1]).wrapping_add(h[2]).wrapping_add(h[3]);
        h[1] = h[1].wrapping_add(h[0]);
        h[2] = h[2].wrapping_add(h[0]);
        h[3] = h[3].wrapping_add(h[0]);
    };
    sum(&mut h);
    for x in &mut h {
        *x = fmix32(*x);
    }
    sum(&mut h);
    h.iter().flat_map(|w| w.to_be_bytes()).collect()
}

/// murmur3f: MurmurHash3_x64_128.
pub(crate) fn murmur3f(data: &[u8], seed: u64) -> Vec<u8> {
    const C1: u64 = 0x87c3_7b91_1142_53d5;
    const C2: u64 = 0x4cf5_ad43_2745_937f;
    let (mut h1, mut h2) = (seed, seed);
    let (blocks, rest) = data.as_chunks::<16>();
    for c in blocks {
        let k1 = u64::from_le_bytes(c[..8].try_into().expect("8 bytes"));
        let k2 = u64::from_le_bytes(c[8..].try_into().expect("8 bytes"));
        h1 ^= k1.wrapping_mul(C1).rotate_left(31).wrapping_mul(C2);
        h1 = h1.rotate_left(27).wrapping_add(h2).wrapping_mul(5).wrapping_add(0x52dc_e729);
        h2 ^= k2.wrapping_mul(C2).rotate_left(33).wrapping_mul(C1);
        h2 = h2.rotate_left(31).wrapping_add(h1).wrapping_mul(5).wrapping_add(0x3849_5ab5);
    }
    if rest.len() > 8 {
        h2 ^= tail_word(&rest[8..]).wrapping_mul(C2).rotate_left(33).wrapping_mul(C1);
    }
    if !rest.is_empty() {
        h1 ^= tail_word(&rest[..rest.len().min(8)]).wrapping_mul(C1).rotate_left(31).wrapping_mul(C2);
    }
    let len = data.len() as u64;
    h1 ^= len;
    h2 ^= len;
    h1 = h1.wrapping_add(h2);
    h2 = h2.wrapping_add(h1);
    h1 = fmix64(h1);
    h2 = fmix64(h2);
    h1 = h1.wrapping_add(h2);
    h2 = h2.wrapping_add(h1);
    [h1.to_be_bytes(), h2.to_be_bytes()].concat()
}

// --- crc32 (bzip2) and crc32c -----------------------------------------------

/// "crc32": bzip2's CRC (MSB first), its bytes as PHP stores them.
pub(crate) fn crc32_bzip2(data: &[u8]) -> Vec<u8> {
    let mut crc = !0u32;
    for &b in data {
        crc ^= u32::from(b) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 { (crc << 1) ^ 0x04C1_1DB7 } else { crc << 1 };
        }
    }
    (!crc).to_le_bytes().to_vec()
}

/// "crc32c": Castagnoli's CRC (reflected).
pub(crate) fn crc32c(data: &[u8]) -> Vec<u8> {
    let mut crc = !0u32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0x82F6_3B78 } else { crc >> 1 };
        }
    }
    (!crc).to_be_bytes().to_vec()
}
