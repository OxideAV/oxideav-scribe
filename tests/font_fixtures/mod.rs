//! Shared download-cache-verify helper for the round-5 font fixtures.
//!
//! Two large fonts (~30 MB combined) are too big to vendor in-tree;
//! they're hosted at `samples.oxideav.org/fonts/` and fetched on first
//! use into `target/test-fixtures/fonts/`. Subsequent test runs read
//! from the cache and skip the network entirely.
//!
//! Mirrors the `oxideav-msmpeg4/tests/microsoft_fixtures.rs` pattern
//! (per the round-5 task specification — DO NOT reinvent the wheel).
//!
//! ## Network gating
//!
//! Set `OXIDEAV_NETWORK_TESTS=1` to allow first-time downloads. Without
//! the flag — and without a populated cache — `load_fixture` prints a
//! skip message via `eprintln!` and returns `None`. The test should
//! then return success; we never fail tests for missing fixtures.
//!
//! ## SHA-256 verification
//!
//! Every load (cached or freshly downloaded) is hashed and compared
//! against the pinned digest. A stale cache is removed and re-fetched
//! (under the network-gating policy above); a fresh download whose
//! hash doesn't match panics, anchoring any regression to a specific
//! upstream blob.

#![allow(dead_code)] // each test binary uses a different subset

use std::fs;
use std::io::Read;
use std::path::PathBuf;

/// One pinned font fixture.
pub struct FontFixture {
    /// File name on the CDN and on disk under
    /// `target/test-fixtures/fonts/`.
    pub name: &'static str,
    /// Source URL.
    pub url: &'static str,
    /// Pinned SHA-256 (lowercase hex). If the upstream blob changes
    /// we fail loudly so the regression is anchored to a specific
    /// binary.
    pub sha256: &'static str,
    /// Pinned content-length.
    pub bytes: u64,
}

pub const NOTO_SANS_CJK_MEDIUM_TTC: FontFixture = FontFixture {
    name: "NotoSansCJK-Medium.ttc",
    url: "https://samples.oxideav.org/fonts/NotoSansCJK-Medium.ttc",
    sha256: "fad9e409749406abc69ac32e1d013451a610d43c8afdd2ca238295c223e5567e",
    bytes: 19_215_012,
};

pub const NOTO_COLOR_EMOJI_TTF: FontFixture = FontFixture {
    name: "NotoColorEmoji.ttf",
    url: "https://samples.oxideav.org/fonts/NotoColorEmoji.ttf",
    sha256: "c2f19f6a404baa7da7a710b018c2892d7b51386983ddca146811f76aea0b6861",
    bytes: 10_589_456,
};

fn fixture_cache_dir() -> PathBuf {
    let target_dir = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let crate_dir = std::env::var("CARGO_MANIFEST_DIR")
                .map(PathBuf::from)
                .expect("CARGO_MANIFEST_DIR set during cargo test");
            crate_dir.join("..").join("..").join("target")
        });
    let dir = target_dir.join("test-fixtures").join("fonts");
    fs::create_dir_all(&dir).expect("create font test-fixtures dir");
    dir
}

fn cache_path(name: &str) -> PathBuf {
    fixture_cache_dir().join(name)
}

fn network_tests_enabled() -> bool {
    matches!(
        std::env::var("OXIDEAV_NETWORK_TESTS").ok().as_deref(),
        Some("1") | Some("true") | Some("TRUE") | Some("yes") | Some("YES")
    )
}

/// Fetch (or load from cache + verify) a fixture. Returns `None` if
/// the fixture is neither cached nor allowed to be downloaded — the
/// caller should print its own skip message and return success.
pub fn load_fixture(fix: &FontFixture) -> Option<Vec<u8>> {
    let path = cache_path(fix.name);
    if let Ok(bytes) = fs::read(&path) {
        if bytes.len() as u64 == fix.bytes && sha256_hex(&bytes) == fix.sha256 {
            eprintln!("[{}] using cached {}", fix.name, path.display());
            return Some(bytes);
        }
        eprintln!(
            "[{}] cached {} is stale (len {} / sha256 {}), re-downloading",
            fix.name,
            path.display(),
            bytes.len(),
            sha256_hex(&bytes)
        );
        let _ = fs::remove_file(&path);
    }
    if !network_tests_enabled() {
        eprintln!(
            "[{}] OXIDEAV_NETWORK_TESTS not set and no cached fixture at {} — skipping",
            fix.name,
            path.display()
        );
        return None;
    }
    eprintln!("[{}] downloading {}", fix.name, fix.url);
    let resp = match ureq::get(fix.url).call() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[{}] download failed ({e}) — skipping", fix.name);
            return None;
        }
    };
    let mut buf = Vec::with_capacity(fix.bytes as usize);
    if let Err(e) = resp.into_body().into_reader().read_to_end(&mut buf) {
        eprintln!("[{}] body read failed ({e}) — skipping", fix.name);
        return None;
    }
    if buf.len() as u64 != fix.bytes {
        eprintln!(
            "[{}] downloaded size {} != expected {} — skipping",
            fix.name,
            buf.len(),
            fix.bytes
        );
        return None;
    }
    let got = sha256_hex(&buf);
    if got != fix.sha256 {
        panic!(
            "{}: sha256 mismatch:\n  expected {}\n  got      {}\n\
             Either the upstream blob changed (update sha256 in font_fixtures.rs) \
             or the download was corrupted.",
            fix.name, fix.sha256, got,
        );
    }
    let tmp = path.with_extension("tmp");
    if let Err(e) = fs::write(&tmp, &buf).and_then(|_| fs::rename(&tmp, &path)) {
        eprintln!(
            "[{}] cache write to {} failed ({e})",
            fix.name,
            path.display()
        );
    } else {
        eprintln!("[{}] cached at {}", fix.name, path.display());
    }
    Some(buf)
}

// ---------------------------------------------------------------------------
// Inline SHA-256 (FIPS 180-4 byte-oriented reference). Same implementation
// as oxideav-msmpeg4/tests/microsoft_fixtures.rs and
// oxideav-mod/tests/rhmst_url_regression.rs — duplicated to avoid a
// `sha2` dev-dependency.
// ---------------------------------------------------------------------------

pub fn sha256_hex(bytes: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let bit_len = (bytes.len() as u64) * 8;
    let mut msg = Vec::with_capacity(bytes.len() + 72);
    msg.extend_from_slice(bytes);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let mj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(mj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }
    let mut out = String::with_capacity(64);
    for word in h {
        for byte in word.to_be_bytes() {
            out.push_str(&format!("{:02x}", byte));
        }
    }
    out
}
