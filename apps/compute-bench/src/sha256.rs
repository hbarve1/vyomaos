// Pure-Rust SHA-256 (FIPS 180-4) — no external dependencies.
//
// This implementation deliberately avoids any `std` features beyond what
// wasm32-wasip2 / wasm64-wasip2 provides.  It is intentionally simple and
// correct rather than high-performance; the benchmark workload is intentionally
// CPU-bound to stress deterministic compute, not memory bandwidth.

// ── SHA-256 round constants ───────────────────────────────────────────────────

#[rustfmt::skip]
const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5,
    0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
    0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc,
    0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
    0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
    0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3,
    0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5,
    0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
    0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

// ── Initial hash values (first 32 bits of fractional parts of sqrt(2..19)) ───

const H0: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
    0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

// ── Public API ────────────────────────────────────────────────────────────────

/// Compute SHA-256 of a 32-byte input and return the 32-byte digest.
///
/// Accepting exactly 32 bytes simplifies the padding logic for the benchmark
/// (one 512-bit block with 256 bits of data and 256 bits of padding).
pub fn hash(input: &[u8; 32]) -> [u8; 32] {
    // ── Pad message to one 512-bit (64-byte) block ────────────────────────────
    // Layout: input (32 B) | 0x80 | zeros | big-endian bit-length (8 B)
    // Bit-length of 32-byte message = 256 = 0x0000_0000_0000_0100
    let mut block = [0u8; 64];
    block[..32].copy_from_slice(input);
    block[32] = 0x80; // padding start bit
    // Bytes 33..55 remain zero (already zero-initialised)
    // Big-endian bit length = 32 * 8 = 256
    let bit_len: u64 = 256;
    block[56..64].copy_from_slice(&bit_len.to_be_bytes());

    // ── Prepare message schedule W[0..64] ────────────────────────────────────
    let mut w = [0u32; 64];
    for i in 0..16 {
        w[i] = u32::from_be_bytes([
            block[i * 4],
            block[i * 4 + 1],
            block[i * 4 + 2],
            block[i * 4 + 3],
        ]);
    }
    for i in 16..64 {
        let s0 = w[i - 15].rotate_right(7)
            ^ w[i - 15].rotate_right(18)
            ^ (w[i - 15] >> 3);
        let s1 = w[i - 2].rotate_right(17)
            ^ w[i - 2].rotate_right(19)
            ^ (w[i - 2] >> 10);
        w[i] = w[i - 16]
            .wrapping_add(s0)
            .wrapping_add(w[i - 7])
            .wrapping_add(s1);
    }

    // ── Compression ───────────────────────────────────────────────────────────
    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = H0;

    for i in 0..64 {
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = (e & f) ^ ((!e) & g);
        let temp1 = h
            .wrapping_add(s1)
            .wrapping_add(ch)
            .wrapping_add(K[i])
            .wrapping_add(w[i]);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let temp2 = s0.wrapping_add(maj);

        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(temp1);
        d = c;
        c = b;
        b = a;
        a = temp1.wrapping_add(temp2);
    }

    // ── Produce digest ────────────────────────────────────────────────────────
    let digest_words = [
        H0[0].wrapping_add(a),
        H0[1].wrapping_add(b),
        H0[2].wrapping_add(c),
        H0[3].wrapping_add(d),
        H0[4].wrapping_add(e),
        H0[5].wrapping_add(f),
        H0[6].wrapping_add(g),
        H0[7].wrapping_add(h),
    ];

    let mut out = [0u8; 32];
    for (i, word) in digest_words.iter().enumerate() {
        out[i * 4..(i + 1) * 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}
