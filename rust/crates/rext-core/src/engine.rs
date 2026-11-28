//! Rust port of `entry/src/main/cpp/engine.cpp` (4902 lines) + `engine.h` ABI.
//!
//! Ported: lexicon format KKL2 v1 (16-byte entries) and v2 (10-byte packed
//! entries, prefix-compressed readings in blocks of 16); model format KKM1
//! v2..=7 (fp32 tables, v4 int8 char tables, v5 int8 big linears + kW,
//! v3 cost gate, v6 user/style mark tables, v7 user-vector weights);
//! lattice construction (dictionary / kana / katakana / passthrough /
//! number / special / abbreviation / user-dictionary / typed-latin edges,
//! katakana & kanji continuation edges, typo edges); the Transformer
//! scorer (pre-LN encoder, span-q / word-k, cost gate, Romance overlay);
//! K-best Viterbi (`nbest`); cascade mode (use_model=2); suggestions;
//! personalization (variants / marks / style); completion; Romance
//! per-user model (enable/config/learn/get/set/reset).
//!
//! SIMD note: the C++ x86 AVX2+FMA path (`engine_simd.h` /
//! `engine_avx2.cpp`) is replaced by scalar code performing the same
//! arithmetic in the same order of accumulation per output element
//! (plain dot products); results are mathematically equivalent, only
//! slower. The int8-quantized linear path is likewise replaced by the
//! fp32 path (the C++ code itself falls back to fp32 off x86).
//!
//! TODO (known gaps vs. engine.cpp, do not claim parity for these):
//! - Thread pool: `kkc_set_threads` is accepted and stored; all compute
//!   is single-threaded scalar (same results, no parallel speedup).
//! - The lexicon/model bytes are copied into owned buffers instead of
//!   borrowed (mmap) as in C++; observable behaviour is unchanged.

use std::collections::{HashMap, HashSet};
use std::ffi::CStr;
use std::os::raw::{c_char, c_void};
use std::time::Instant;

// ------------------------------------------------------------------ types
pub type Ustr = Vec<char>;

fn is_kana(c: char) -> bool {
    ('\u{3041}'..='\u{3096}').contains(&c) || c == '\u{30fc}' || c == '\u{3094}'
}
fn kata(c: char) -> char {
    if ('\u{3041}'..='\u{3096}').contains(&c) {
        char::from_u32(c as u32 + 0x60).unwrap()
    } else {
        c
    }
}
fn to_kata(s: &[char]) -> Ustr {
    s.iter().map(|&c| kata(c)).collect()
}
fn from16(p: &[u16]) -> Ustr {
    let mut o = Vec::with_capacity(p.len());
    let mut i = 0;
    while i < p.len() {
        let c = p[i] as u32;
        if (0xD800..0xDC00).contains(&c) && i + 1 < p.len() && (0xDC00..0xE000).contains(&(p[i + 1] as u32)) {
            let v = 0x10000 + ((c - 0xD800) << 10) + (p[i + 1] as u32 - 0xDC00);
            o.push(char::from_u32(v).unwrap_or('\u{FFFD}'));
            i += 2;
        } else {
            o.push(char::from_u32(c).unwrap_or('\u{FFFD}'));
            i += 1;
        }
    }
    o
}
fn to16(s: &[char], o: &mut Vec<u16>) {
    for &c in s {
        let v = c as u32;
        if v >= 0x10000 {
            let v = v - 0x10000;
            o.push((0xD800 + (v >> 10)) as u16);
            o.push((0xDC00 + (v & 0x3FF)) as u16);
        } else {
            o.push(v as u16);
        }
    }
}
fn l16len(s: &[char]) -> usize {
    s.iter().map(|&c| if (c as u32) >= 0x10000 { 2 } else { 1 }).sum()
}
fn u(s: &str) -> Ustr {
    s.chars().collect()
}
fn starts_with(s: &[char], i: usize, k: &[char]) -> bool {
    i + k.len() <= s.len() && &s[i..i + k.len()] == k
}
fn ends_with(s: &[char], k: &[char]) -> bool {
    s.len() >= k.len() && &s[s.len() - k.len()..] == k
}

const K_DICT: u8 = 0;
const K_KANA: u8 = 1;
const K_KATA: u8 = 2;
const K_PASS: u8 = 3;
const K_NUM: u8 = 4;
const K_TYPO: u8 = 5;
const K_LATIN: u8 = 6;
const ENG_COST: i32 = 32000;
const ENG_LATIN_COST: i32 = 9000;
const LATIN_WORD_BONUS: i32 = 500;
const LATIN_BIAS: i32 = 3000;
const LATIN_RAW_COST: i32 = 6000;
const LATIN_CASE_ALT: i32 = 1500;

#[derive(Clone, Debug)]
struct Edge {
    s: i32,
    e: i32,
    surf: Ustr,
    lid: i32,
    rid: i32,
    cost: i32,
    kind: u8,
    mark: i32,
}
impl Edge {
    fn new(s: i32, e: i32, surf: Ustr, lid: i32, rid: i32, cost: i32, kind: u8) -> Self {
        Edge { s, e, surf, lid, rid, cost, kind, mark: 0 }
    }
}

#[derive(Clone, Copy, Default)]
struct Ent {
    soff: u32,
    slen: u16,
    lid: u16,
    rid: u16,
    cost: i32,
}

#[derive(Clone)]
struct LatinWord {
    surf: Ustr,
    lid: i32,
    rid: i32,
    cost: i32,
}

#[derive(Clone, Copy, Default)]
struct SegInfo {
    e: i32,
    l16: i32,
    lid: i32,
    rid: i32,
}

#[derive(Clone)]
struct PathInfo {
    c: f64,
    w: Vec<(i32, Ustr)>, // (reading end, surface)
}

#[derive(Clone)]
struct UserWord {
    r: Ustr,
    s: Ustr,
    lid: i32,
    rid: i32,
    cost: i32,
    whole: bool,
}

#[derive(Default)]
struct UserList {
    v: Vec<UserWord>,
    by_r: HashMap<Ustr, Vec<u32>>,
    maxlen: usize,
}
impl UserList {
    fn clear(&mut self) {
        self.v.clear();
        self.by_r.clear();
        self.maxlen = 0;
    }
    fn push(&mut self, w: UserWord) {
        if !w.r.is_empty() {
            self.by_r.entry(w.r.clone()).or_default().push(self.v.len() as u32);
            self.maxlen = self.maxlen.max(w.r.len());
        }
        self.v.push(w);
    }
}

// ------------------------------------------------------------------ binary reader (engine.cpp Reader: arrays are 8-byte aligned)
struct Reader<'a> {
    p: &'a [u8],
    off: usize,
    ok: bool,
}
impl<'a> Reader<'a> {
    fn arr(&mut self, nbytes: usize) -> &'a [u8] {
        self.off = (self.off + 7) & !7usize;
        if self.off + nbytes > self.p.len() {
            self.ok = false;
            return &[];
        }
        let r = &self.p[self.off..self.off + nbytes];
        self.off += nbytes;
        r
    }
    fn i32s(&mut self, n: usize) -> Vec<i32> {
        self.arr(n * 4).chunks_exact(4).map(|c| i32::from_le_bytes(c.try_into().unwrap())).collect()
    }
    fn i16s(&mut self, n: usize) -> Vec<i16> {
        self.arr(n * 2).chunks_exact(2).map(|c| i16::from_le_bytes(c.try_into().unwrap())).collect()
    }
    fn i8s(&mut self, n: usize) -> Vec<i8> {
        self.arr(n).iter().map(|&b| b as i8).collect()
    }
    fn u32s(&mut self, n: usize) -> Vec<u32> {
        self.arr(n * 4).chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect()
    }
    fn u16s(&mut self, n: usize) -> Vec<u16> {
        self.arr(n * 2).chunks_exact(2).map(|c| u16::from_le_bytes(c.try_into().unwrap())).collect()
    }
    fn f32s(&mut self, n: usize) -> Vec<f32> {
        self.arr(n * 4).chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect()
    }
    fn val_i32(&mut self) -> i32 {
        if self.off + 4 > self.p.len() {
            self.ok = false;
            return 0;
        }
        let v = i32::from_le_bytes(self.p[self.off..self.off + 4].try_into().unwrap());
        self.off += 4;
        v
    }
    fn val_i64(&mut self) -> i64 {
        if self.off + 8 > self.p.len() {
            self.ok = false;
            return 0;
        }
        let v = i64::from_le_bytes(self.p[self.off..self.off + 8].try_into().unwrap());
        self.off += 8;
        v
    }
    fn val_f32(&mut self) -> f32 {
        if self.off + 4 > self.p.len() {
            self.ok = false;
            return 0.0;
        }
        let v = f32::from_le_bytes(self.p[self.off..self.off + 4].try_into().unwrap());
        self.off += 4;
        v
    }
    fn val_u32(&mut self) -> u32 {
        self.val_i32() as u32
    }
}

// ------------------------------------------------------------------ lexicon (KKL2)
struct Lex {
    noun: i32,
    num_arabic: i32,
    num_kanji: i32,
    max_word: i32,
    oov: i32,
    kata: i32,
    kata_per: i32,
    pass: i32,
    nconn: i32,
    nread: i32,
    nent: i32,
    ver: i32,
    conn: Vec<i32>, // nconn*nconn (conn16 widened)
    r_off: Vec<u32>,
    e_first: Vec<u32>,
    ents: Vec<Ent>, // v2 packed entries decoded to the v1 shape
    r_blob: Vec<u16>,
    s_blob: Vec<u16>,
    typo: bool,
    latin: bool,
    latin_words: HashMap<String, LatinWord>,
    head_idx: HashMap<char, Vec<(u32, u32)>>,
    head_built: bool,
    abbr_lid: i32,
    abbr_rid: i32,
    bloom: Vec<u64>,
    pbloom: Vec<u64>,
    pbits: usize,
}
const RB: i32 = 16;
const MAXR: usize = 256;

impl Lex {
    fn conn(&self, a: i32, b: i32) -> i32 {
        if a < 0 || b < 0 || a >= self.nconn || b >= self.nconn {
            return 0;
        }
        self.conn[(a as usize) * (self.nconn as usize) + b as usize]
    }
    fn ent(&self, j: u32) -> Ent {
        self.ents[j as usize]
    }
    fn surf(&self, en: &Ent) -> Ustr {
        from16(&self.s_blob[en.soff as usize..en.soff as usize + en.slen as usize])
    }
    // v2: buf holds reading k-1; produce reading k. Returns length.
    fn next_reading(&self, k: i32, buf: &mut [u16; MAXR]) -> usize {
        let p = self.r_off[k as usize] as usize;
        let len = (self.r_off[k as usize + 1] - self.r_off[k as usize]) as usize;
        if k % RB == 0 {
            buf[..len].copy_from_slice(&self.r_blob[p..p + len]);
            return len;
        }
        let pre = self.r_blob[p] as usize;
        let n = len - 1;
        buf[pre..pre + n].copy_from_slice(&self.r_blob[p + 1..p + 1 + n]);
        pre + n
    }
    fn reading(&self, i: i32, buf: &mut [u16; MAXR]) -> usize {
        if self.ver == 1 {
            let a = self.r_off[i as usize] as usize;
            let b = self.r_off[i as usize + 1] as usize;
            let n = (b - a).min(MAXR);
            buf[..n].copy_from_slice(&self.r_blob[a..a + n]);
            return n;
        }
        let mut n = 0;
        for k in (i - i % RB)..=i {
            n = self.next_reading(k, buf);
        }
        n
    }
    fn reading_vec(&self, i: i32) -> Vec<u16> {
        let mut buf = [0u16; MAXR];
        let n = self.reading(i, &mut buf);
        buf[..n].to_vec()
    }
    fn cmp(r: &[u16], k: &[u16]) -> i32 {
        let m = r.len().min(k.len());
        for i in 0..m {
            if r[i] != k[i] {
                return r[i] as i32 - k[i] as i32;
            }
        }
        r.len() as i32 - k.len() as i32
    }
    fn lower_bound(&self, k: &[u16]) -> i32 {
        if self.ver == 1 {
            let (mut lo, mut hi) = (0i32, self.nread);
            while lo < hi {
                let mid = (lo + hi) >> 1;
                let a = self.r_off[mid as usize] as usize;
                let b = self.r_off[mid as usize + 1] as usize;
                if Self::cmp(&self.r_blob[a..b], k) < 0 {
                    lo = mid + 1;
                } else {
                    hi = mid;
                }
            }
            return lo;
        }
        let nb = (self.nread + RB - 1) / RB;
        let (mut lo, mut hi) = (0i32, nb);
        while lo < hi {
            let mid = (lo + hi) >> 1;
            let h = mid * RB;
            let a = self.r_off[h as usize] as usize;
            let b = self.r_off[h as usize + 1] as usize;
            if Self::cmp(&self.r_blob[a..b], k) < 0 {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        if lo == 0 {
            return 0;
        }
        let mut buf = [0u16; MAXR];
        let end = self.nread.min(lo * RB);
        let mut i = (lo - 1) * RB;
        let mut n = self.next_reading(i, &mut buf);
        i += 1;
        while i < end {
            n = self.next_reading(i, &mut buf);
            if Self::cmp(&buf[..n], k) >= 0 {
                return i;
            }
            i += 1;
        }
        end
    }
    fn each_reading<F: FnMut(&[u16])>(&self, mut f: F) {
        let mut buf = [0u16; MAXR];
        for i in 0..self.nread {
            if self.ver == 1 {
                let a = self.r_off[i as usize] as usize;
                let b = self.r_off[i as usize + 1] as usize;
                f(&self.r_blob[a..b]);
            } else {
                let n = self.next_reading(i, &mut buf);
                f(&buf[..n]);
            }
        }
    }
    fn hash16(k: &[u16]) -> u64 {
        let mut h: u64 = 1469598103934665603;
        for &c in k {
            h ^= c as u64;
            h = h.wrapping_mul(1099511628211);
        }
        h ^ (h >> 29)
    }
    fn build_bloom(&mut self) {
        if !self.bloom.is_empty() || self.nread <= 0 {
            return;
        }
        let bits = self.nread as usize * 12;
        let mut bloom = vec![0u64; (bits + 63) / 64];
        self.each_reading(|r| {
            let h = Self::hash16(r);
            let h2 = (h >> 32) | 1;
            for j in 0..4u64 {
                let b = (h.wrapping_add(j.wrapping_mul(h2)) % bits as u64) as usize;
                bloom[b >> 6] |= 1u64 << (b & 63);
            }
        });
        self.bloom = bloom;
        // prefix bloom
        let mut total = 0usize;
        self.each_reading(|r| total += r.len());
        let pbits = (total * 6).max(64);
        let mut pb = vec![0u64; (pbits + 63) / 64];
        self.each_reading(|r| {
            let mut h: u64 = 1469598103934665603;
            for &c in r {
                h ^= c as u64;
                h = h.wrapping_mul(1099511628211);
                let x = h ^ (h >> 29);
                let x2 = (x >> 32) | 1;
                for t in 0..3u64 {
                    let b = (x.wrapping_add(t.wrapping_mul(x2)) % pbits as u64) as usize;
                    pb[b >> 6] |= 1u64 << (b & 63);
                }
            }
        });
        self.pbits = pbits;
        self.pbloom = pb;
    }
    fn maybe_head(&self, h: u64) -> bool {
        if self.pbloom.is_empty() {
            return true;
        }
        let h2 = (h >> 32) | 1;
        for j in 0..3u64 {
            let b = (h.wrapping_add(j.wrapping_mul(h2)) % self.pbits as u64) as usize;
            if (self.pbloom[b >> 6] >> (b & 63)) & 1 == 0 {
                return false;
            }
        }
        true
    }
    fn maybe_hash(&self, h: u64) -> bool {
        if self.bloom.is_empty() {
            return true;
        }
        let bits = (self.bloom.len() * 64).min(self.nread as usize * 12);
        let h2 = (h >> 32) | 1;
        for j in 0..4u64 {
            let b = (h.wrapping_add(j.wrapping_mul(h2)) % bits as u64) as usize;
            if (self.bloom[b >> 6] >> (b & 63)) & 1 == 0 {
                return false;
            }
        }
        true
    }
    fn maybe(&self, k: &[u16]) -> bool {
        self.maybe_hash(Self::hash16(k))
    }
    fn find(&self, k: &[u16]) -> Option<(u32, u32)> {
        if !self.maybe(k) {
            return None;
        }
        let lo = self.lower_bound(k);
        if lo >= self.nread {
            return None;
        }
        let r = self.reading_vec(lo);
        if r.len() != k.len() || r != k {
            return None;
        }
        Some((self.e_first[lo as usize], self.e_first[lo as usize + 1]))
    }
    /// Word list for the glide decoder, mirroring `engine.cpp`'s
    /// `glide_build` (used by `glide::ffi::engine_lexicon_words`).
    /// - mode 0 (kana): readings of hiragana + ー only, length 1..=12,
    ///   whose cheapest entry costs at most 7000; the cost kept is that
    ///   minimum entry cost.
    /// - mode 1 (letters): the latin words of the lexicon (a-z only,
    ///   length 2..=20) with their latin-word cost.
    /// - mode 2: never built from the engine (a TSV word list is
    ///   required), so `None`.
    pub fn glide_words(&self, mode: i32) -> Option<Vec<(Vec<u16>, i32)>> {
        match mode {
            0 => {
                let hira = |c: u16| (0x3041..=0x3094).contains(&c) || c == 0x30FC;
                let mut words: Vec<(Vec<u16>, i32)> = Vec::new();
                let mut buf = [0u16; MAXR];
                for i in 0..self.nread {
                    // Sequential walk like C++ (v2 readings are prefix
                    // differences against the previous reading).
                    let rn = if self.ver == 1 || i % RB == 0 {
                        self.reading(i, &mut buf)
                    } else {
                        self.next_reading(i, &mut buf)
                    };
                    if !(1..=12).contains(&rn) {
                        continue;
                    }
                    if !buf[..rn].iter().all(|&c| hira(c)) {
                        continue;
                    }
                    let mut best = i32::MAX;
                    for j in self.e_first[i as usize]..self.e_first[i as usize + 1] {
                        best = best.min(self.ent(j).cost);
                    }
                    if best > 7000 {
                        continue;
                    }
                    words.push((buf[..rn].to_vec(), best));
                }
                Some(words)
            }
            1 => {
                // The latin-word table built at open time; fall back to
                // deriving it from the entries when it was not built
                // (model without a latin kind), exactly like open does.
                let mut words: Vec<(Vec<u16>, i32)> = Vec::new();
                if !self.latin_words.is_empty() {
                    for (k, w) in &self.latin_words {
                        if k.len() >= 2 && k.len() <= 20 && k.bytes().all(|c| c.is_ascii_lowercase()) {
                            words.push((k.bytes().map(|c| c as u16).collect(), w.cost));
                        }
                    }
                } else {
                    let mut map: HashMap<String, i32> = HashMap::new();
                    for j in 0..self.nent as u32 {
                        let en = self.ent(j);
                        if en.slen < 2 {
                            continue;
                        }
                        let s = &self.s_blob[en.soff as usize..en.soff as usize + en.slen as usize];
                        let mut lower = String::new();
                        let mut ok = true;
                        for &c in s {
                            if (b'a' as u16..=b'z' as u16).contains(&c) || (b'0' as u16..=b'9' as u16).contains(&c) {
                                lower.push(c as u8 as char);
                            } else if (b'A' as u16..=b'Z' as u16).contains(&c) {
                                lower.push((c + 32) as u8 as char);
                            } else {
                                ok = false;
                                break;
                            }
                        }
                        if !ok || lower.len() < 2 || lower.len() > 20 || !lower.bytes().all(|c| c.is_ascii_lowercase()) {
                            continue;
                        }
                        let cost = if en.cost >= ENG_COST { ENG_LATIN_COST } else { en.cost };
                        match map.get(&lower) {
                            Some(&old) if old <= cost => {}
                            _ => {
                                map.insert(lower, cost);
                            }
                        }
                    }
                    for (k, cost) in map {
                        words.push((k.bytes().map(|c| c as u16).collect(), cost));
                    }
                }
                Some(words)
            }
            _ => None,
        }
    }
}

fn load_lex(data: &[u8]) -> Option<Lex> {
    if data.len() < 4 || &data[0..4] != b"KKL2" {
        return None;
    }
    let mut r = Reader { p: data, off: 4, ok: true };
    let ver = r.val_i32();
    if ver != 1 && ver != 2 {
        return None;
    }
    let noun = r.val_i32();
    let num_arabic = r.val_i32();
    let num_kanji = r.val_i32();
    let max_word = r.val_i32();
    let oov = r.val_i32();
    let kata = r.val_i32();
    let kata_per = r.val_i32();
    let pass = r.val_i32();
    let nconn = r.val_i32();
    let conn16 = r.val_i32();
    let nread = r.val_i32();
    let nent = r.val_i32();
    let nr16 = r.val_i64();
    let ns16 = r.val_i64();
    if !r.ok || nconn < 0 || nread < 0 || nent < 0 {
        return None;
    }
    let nc = (nconn as usize) * (nconn as usize);
    let conn: Vec<i32> = if conn16 != 0 {
        r.i16s(nc).into_iter().map(|v| v as i32).collect()
    } else {
        r.i32s(nc)
    };
    let r_off = r.u32s(nread as usize + 1);
    let e_first = r.u32s(nread as usize + 1);
    let ents: Vec<Ent> = if ver == 1 {
        let raw = r.arr(nent as usize * 16);
        raw.chunks_exact(16)
            .map(|c| Ent {
                soff: u32::from_le_bytes(c[0..4].try_into().unwrap()),
                slen: u16::from_le_bytes(c[4..6].try_into().unwrap()),
                lid: u16::from_le_bytes(c[6..8].try_into().unwrap()),
                rid: u16::from_le_bytes(c[8..10].try_into().unwrap()),
                cost: i32::from_le_bytes(c[12..16].try_into().unwrap()),
            })
            .collect()
    } else {
        let raw = r.arr(nent as usize * 10);
        raw.chunks_exact(10)
            .map(|c| {
                let a = u16::from_le_bytes(c[0..2].try_into().unwrap());
                let b = u16::from_le_bytes(c[2..4].try_into().unwrap());
                Ent {
                    soff: a as u32 | (((b & 0x3FF) as u32) << 16),
                    slen: b >> 10,
                    lid: u16::from_le_bytes(c[4..6].try_into().unwrap()),
                    rid: u16::from_le_bytes(c[6..8].try_into().unwrap()),
                    cost: i16::from_le_bytes(c[8..10].try_into().unwrap()) as i32,
                }
            })
            .collect()
    };
    let r_blob = r.u16s(nr16.max(0) as usize);
    let s_blob = r.u16s(ns16.max(0) as usize);
    if !r.ok {
        return None;
    }
    Some(Lex {
        noun, num_arabic, num_kanji, max_word, oov, kata, kata_per, pass, nconn, nread, nent, ver,
        conn, r_off, e_first, ents, r_blob, s_blob,
        typo: false, latin: false, latin_words: HashMap::new(), head_idx: HashMap::new(),
        head_built: false, abbr_lid: -1, abbr_rid: -1, bloom: Vec::new(), pbloom: Vec::new(), pbits: 0,
    })
}

// ------------------------------------------------------------------ model (KKM1)
#[derive(Clone)]
enum Tab {
    F(Vec<f32>),
    Q { q: Vec<i8>, s: Vec<f32> },
}
impl Tab {
    fn row_vec(&self, i: usize, d: usize) -> Vec<f32> {
        match self {
            Tab::F(f) => f[i * d..(i + 1) * d].to_vec(),
            Tab::Q { q, s } => {
                let k = s[i];
                (0..d).map(|j| k * q[i * d + j] as f32).collect()
            }
        }
    }
}

#[derive(Clone, Default)]
struct LayerW {
    ln1w: Vec<f32>, ln1b: Vec<f32>, inw: Vec<f32>, inb: Vec<f32>,
    outw: Vec<f32>, outb: Vec<f32>, ln2w: Vec<f32>, ln2b: Vec<f32>,
    l1w: Vec<f32>, l1b: Vec<f32>, l2w: Vec<f32>, l2b: Vec<f32>,
}

#[derive(Default)]
struct Model {
    n_kind: i32,
    v: i32,
    d: i32,
    layers: i32,
    heads: i32,
    ff: i32,
    dk: i32,
    max_len: i32,
    n_seg: i32,
    ctx: i32,
    draft_max: i32,
    draft: i32,
    beta: f32,
    gamma: f32,
    vocab: HashMap<char, i32>,
    emb: Option<Tab>,
    pa: Option<Tab>,
    pb: Option<Tab>,
    pc: Option<Tab>,
    kw: Option<Vec<f32>>,
    seg: Vec<f32>,
    pos: Vec<f32>,
    l: Vec<LayerW>,
    normw: Vec<f32>,
    normb: Vec<f32>,
    end: Vec<f32>,
    q1w: Vec<f32>, q1b: Vec<f32>, q2w: Vec<f32>, q2b: Vec<f32>,
    kkind: Vec<f32>, klen: Vec<f32>, k1b: Vec<f32>, k2w: Vec<f32>, k2b: Vec<f32>,
    gw: Option<Vec<f32>>,
    gb: Vec<f32>,
    n_user: i32,
    n_style: i32,
    kuser: Option<Vec<f32>>,
    kstyle: Option<Vec<f32>>,
    d_uvec: i32,
    wu: Option<Vec<f32>>,
}
impl Model {
    fn vid(&self, c: char) -> i32 {
        *self.vocab.get(&c).unwrap_or(&1) // UNK = 1
    }
}

fn load_model(data: &[u8]) -> Option<Model> {
    if data.len() < 4 || &data[0..4] != b"KKM1" {
        return None;
    }
    let mut r = Reader { p: data, off: 4, ok: true };
    let ver = r.val_i32();
    if !(2..=7).contains(&ver) {
        return None;
    }
    let mut m = Model::default();
    m.v = r.val_i32();
    m.d = r.val_i32();
    m.layers = r.val_i32();
    m.heads = r.val_i32();
    m.ff = r.val_i32();
    m.dk = r.val_i32();
    m.max_len = r.val_i32();
    m.n_seg = r.val_i32();
    m.ctx = r.val_i32();
    m.draft_max = r.val_i32();
    let draft_raw = r.val_i32();
    m.n_kind = if (draft_raw >> 8) != 0 { draft_raw >> 8 } else { 5 };
    m.draft = draft_raw & 0xFF;
    m.beta = r.val_f32();
    m.gamma = r.val_f32();
    let nch = r.val_i32();
    for i in 0..nch {
        let c = r.val_u32();
        if let Some(ch) = char::from_u32(c) {
            m.vocab.insert(ch, 4 + i);
        }
    }
    if !r.ok || m.d <= 0 || m.dk <= 0 || m.v < 0 {
        return None;
    }
    let (d, ff, dk, v) = (m.d as usize, m.ff as usize, m.dk as usize, m.v as usize);
    let mut tab = |r: &mut Reader| -> Tab {
        if ver >= 4 {
            let s = r.f32s(v);
            let q = r.i8s(v * d);
            Tab::Q { q, s }
        } else {
            Tab::F(r.f32s(v * d))
        }
    };
    m.emb = Some(tab(&mut r));
    m.seg = r.f32s((m.n_seg.max(0) as usize) * d);
    m.pos = r.f32s((m.max_len.max(0) as usize) * d);
    for _ in 0..m.layers {
        let mut w = LayerW::default();
        w.ln1w = r.f32s(d);
        w.ln1b = r.f32s(d);
        let mut big = |r: &mut Reader, in_: usize, out: usize| -> Vec<f32> {
            if ver < 5 {
                return r.f32s(in_ * out);
            }
            let sc = r.f32s(out);
            let q = r.i8s(in_ * out);
            if !r.ok {
                return Vec::new();
            }
            let mut f = vec![0f32; in_ * out];
            for i in 0..in_ {
                for o in 0..out {
                    f[i * out + o] = sc[o] * q[i * out + o] as f32;
                }
            }
            f
        };
        w.inw = big(&mut r, d, 3 * d);
        w.inb = r.f32s(3 * d);
        w.outw = big(&mut r, d, d);
        w.outb = r.f32s(d);
        w.ln2w = r.f32s(d);
        w.ln2b = r.f32s(d);
        w.l1w = big(&mut r, d, ff);
        w.l1b = r.f32s(ff);
        w.l2w = big(&mut r, ff, d);
        w.l2b = r.f32s(d);
        m.l.push(w);
    }
    m.normw = r.f32s(d);
    m.normb = r.f32s(d);
    m.end = r.f32s(d);
    m.q1w = r.f32s(d * 3 * d);
    m.q1b = r.f32s(d);
    m.q2w = r.f32s(dk * d);
    m.q2b = r.f32s(dk);
    if ver >= 5 {
        m.kw = Some(r.f32s(3 * d * d));
    } else {
        m.pa = Some(tab(&mut r));
        m.pb = Some(tab(&mut r));
        m.pc = Some(tab(&mut r));
    }
    m.kkind = r.f32s((m.n_kind.max(0) as usize) * d);
    m.klen = r.f32s(17 * d);
    m.k1b = r.f32s(d);
    m.k2w = r.f32s(dk * d);
    m.k2b = r.f32s(dk);
    if ver >= 3 {
        m.gw = Some(r.f32s(dk * dk));
        m.gb = r.f32s(dk);
    }
    if ver >= 6 {
        m.n_user = r.val_i32();
        m.n_style = r.val_i32();
        if m.n_user > 0 {
            m.kuser = Some(r.f32s(m.n_user as usize * d));
        }
        if m.n_style > 0 {
            m.kstyle = Some(r.f32s(m.n_style as usize * d));
        }
    }
    if ver >= 7 {
        m.d_uvec = r.val_i32();
        if m.d_uvec > 0 {
            m.wu = Some(r.f32s(m.d_uvec as usize * dk));
        }
    }
    if !r.ok {
        return None;
    }
    Some(m)
}

// ------------------------------------------------------------------ numbers (engine.cpp: init_numbers/parse/kanji/...)
struct Piece {
    k: Ustr,
    kind: u8, // b'd' digit, b's' small unit, b'b' big unit
    v: i64,
}
fn pieces() -> Vec<Piece> {
    let mut p: Vec<Piece> = [
        ("いち", b'd', 1), ("に", b'd', 2), ("さん", b'd', 3), ("よん", b'd', 4), ("し", b'd', 4), ("ご", b'd', 5),
        ("ろく", b'd', 6), ("なな", b'd', 7), ("しち", b'd', 7), ("はち", b'd', 8), ("きゅう", b'd', 9), ("く", b'd', 9),
        ("れい", b'd', 0), ("ぜろ", b'd', 0), ("いっ", b'd', 1), ("ろっ", b'd', 6), ("はっ", b'd', 8),
        ("じゅう", b's', 10), ("ひゃく", b's', 100), ("びゃく", b's', 100), ("ぴゃく", b's', 100), ("せん", b's', 1000),
        ("ぜん", b's', 1000), ("まん", b'b', 10000), ("おく", b'b', 100000000), ("ちょう", b'b', 1000000000000),
    ]
    .iter()
    .map(|(k, kind, v)| Piece { k: u(k), kind: *kind, v: *v })
    .collect();
    p.sort_by(|a, b| b.k.len().cmp(&a.k.len())); // stable, like std::stable_sort by size desc
    p
}
fn sokuon() -> Vec<(Ustr, i64, i64)> {
    [("いっ", 1, 1), ("ろっ", 6, 1), ("はっ", 8, 1), ("じゅっ", 10, 10), ("じっ", 10, 10), ("ひゃっ", 100, 100), ("せっ", 1000, 1000)]
        .iter()
        .map(|(k, v, un)| (u(k), *v, *un))
        .collect()
}
const KANJI_DIGITS: &str = "〇一二三四五六七八九";

fn parse_num_full(r: &[char]) -> Option<(i64, bool)> {
    // engine.cpp parse(): returns (value, sokuon)
    if r.len() < 2 {
        return None;
    }
    let sok_list = sokuon();
    let mut sok: Option<&(Ustr, i64, i64)> = None;
    for s in &sok_list {
        if ends_with(r, &s.0) {
            sok = Some(s);
            break;
        }
    }
    let body: &[char] = match sok {
        Some(s) => &r[..r.len() - s.0.len()],
        None => r,
    };
    let pcs = pieces();
    let mut toks: Vec<(u8, i64)> = Vec::new();
    let mut i = 0usize;
    let mut prev: Ustr = Vec::new();
    while i < body.len() {
        let mut hit = false;
        for p in &pcs {
            if starts_with(body, i, &p.k) {
                if (p.k == u("ぜん") || p.k == u("びゃく")) && prev != u("さん") && prev != u("なん") {
                    return None;
                }
                if p.k == u("ぴゃく") && prev != u("ろっ") && prev != u("はっ") {
                    return None;
                }
                toks.push((p.kind, p.v));
                prev = p.k.clone();
                i += p.k.len();
                hit = true;
                break;
            }
        }
        if !hit {
            return None;
        }
    }
    if let Some(s) = sok {
        if s.2 > 1 && s.1 == s.2 {
            toks.push((b's', s.2));
        } else {
            toks.push((b'd', s.1));
        }
    }
    if toks.is_empty() {
        return None;
    }
    let (mut total, mut group, mut last_small) = (0i64, 0i64, 100000i64);
    let (mut has_digit, mut digit) = (false, 0i64);
    for t in &toks {
        if t.0 == b'd' {
            if has_digit {
                return None;
            }
            has_digit = true;
            digit = t.1;
        } else if t.0 == b's' {
            if t.1 >= last_small {
                return None;
            }
            group += if has_digit { digit } else { 1 } * t.1;
            has_digit = false;
            last_small = t.1;
        } else {
            group += if has_digit { digit } else { 0 };
            if group == 0 {
                return None;
            }
            total += group * t.1;
            group = 0;
            has_digit = false;
            last_small = 100000;
        }
    }
    group += if has_digit { digit } else { 0 };
    let value = total + group;
    if value == 0 && !(toks.len() == 1 && toks[0].0 == b'd' && toks[0].1 == 0) {
        return None;
    }
    Some((value, sok.is_some()))
}
fn kanji_num(n0: i64) -> Ustr {
    let kd: Vec<char> = KANJI_DIGITS.chars().collect();
    if n0 == 0 {
        return u("〇");
    }
    let mut n = n0;
    let bigs = [1000000000000i64, 100000000, 10000, 1];
    let names = ["兆", "億", "万", ""];
    let units = [1000i64, 100, 10, 1];
    let un = ["千", "百", "十", ""];
    let mut out = Vec::new();
    for b in 0..4 {
        let mut g = n / bigs[b];
        n %= bigs[b];
        if g == 0 {
            continue;
        }
        let mut s = Vec::new();
        for uu in 0..4 {
            let d = g / units[uu];
            g %= units[uu];
            if d != 0 {
                if !(d == 1 && units[uu] > 1) {
                    s.push(kd[d as usize]);
                }
                s.extend(un[uu].chars());
            }
        }
        out.extend(s);
        out.extend(names[b].chars());
    }
    out
}
fn digits_str(v: i64, full: bool) -> Ustr {
    v.to_string()
        .chars()
        .map(|c| if full { char::from_u32(0xFF10 + (c as u32 - '0' as u32)).unwrap() } else { c })
        .collect()
}
fn with_commas(d: &[char]) -> Ustr {
    let mut o = Vec::new();
    for i in 0..d.len() {
        if i > 0 && (d.len() - i) % 3 == 0 {
            o.push(',');
        }
        o.push(d[i]);
    }
    o
}
fn unit_mixed(v: i64, comma: bool) -> Ustr {
    let us: [(i64, &str); 4] = [(1000000000000, "兆"), (100000000, "億"), (10000, "万"), (1, "")];
    let mut o = Vec::new();
    for (uval, name) in us {
        let g = (v / uval) % if uval == 1000000000000 { 1000000 } else { 10000 };
        if g == 0 {
            continue;
        }
        let d = digits_str(g, false);
        o.extend(if comma { with_commas(&d) } else { d });
        o.extend(name.chars());
    }
    o
}
fn number_continues(r: &[char], s: usize, e: usize) -> bool {
    for p in pieces() {
        if p.kind == b'd' || !starts_with(r, e, &p.k) {
            continue;
        }
        let mut sub: Ustr = r[s..e].to_vec();
        sub.extend(p.k.iter());
        if parse_num_full(&sub).is_some() {
            return true;
        }
    }
    false
}
fn boundaries(r: &[char], s: usize) -> Vec<usize> {
    let pcs = pieces();
    let mut out = Vec::new();
    let mut i = s;
    while i < r.len() {
        let mut hit = false;
        for p in &pcs {
            if starts_with(r, i, &p.k) {
                i += p.k.len();
                out.push(i);
                hit = true;
                break;
            }
        }
        if !hit {
            for k in sokuon() {
                if starts_with(r, i, &k.0) {
                    out.push(i + k.0.len());
                }
            }
            break;
        }
    }
    out
}
fn parse_num1(r: &[char]) -> Option<i64> {
    if r.len() == 1 {
        for (c, v) in [('に', 2), ('し', 4), ('ご', 5), ('く', 9)] {
            if r[0] == c {
                return Some(v);
            }
        }
        return None;
    }
    match parse_num_full(r) {
        Some((v, false)) => Some(v),
        _ => None,
    }
}

fn number_edges(l: &Lex, r: &[char], out: &mut Vec<Edge>) {
    let (base, per_char) = (4000i32, -150i32);
    for (sg, sc) in [(u("まいなす"), '-'), (u("ぷらす"), '+')] {
        let mut from = 0usize;
        while from + sg.len() <= r.len() {
            let found = (from..=r.len() - sg.len()).find(|&i| starts_with(r, i, &sg));
            let Some(i) = found else { break };
            let s = i + sg.len();
            if s < r.len() {
                if let Some(v) = parse_num1(&r[s..s + 1]) {
                    let mut surf = vec![sc];
                    surf.extend(digits_str(v, false));
                    out.push(Edge::new(i as i32, (s + 1) as i32, surf, l.num_arabic, l.num_arabic, 3500, K_NUM));
                }
            }
            from = i + 1;
        }
    }
    for s in 0..r.len() {
        for e in boundaries(r, s) {
            if e - s < 2 || e - s > 24 {
                continue;
            }
            let Some((v, _sk)) = parse_num_full(&r[s..e]) else { continue };
            if number_continues(r, s, e) {
                continue;
            }
            let c = (base + per_char * (e - s) as i32).max(3000);
            let big = v >= 10000;
            let d = digits_str(v, false);
            out.push(Edge::new(s as i32, e as i32, d.clone(), l.num_arabic, l.num_arabic, if big { c + 300 } else { c }, K_NUM));
            if v >= 1000 {
                out.push(Edge::new(s as i32, e as i32, with_commas(&d), l.num_arabic, l.num_arabic, if big { c + 400 } else { c + 150 }, K_NUM));
            }
            if big {
                let m = unit_mixed(v, false);
                let mc = unit_mixed(v, true);
                out.push(Edge::new(s as i32, e as i32, m.clone(), l.num_arabic, l.num_arabic, c - 300, K_NUM));
                if mc != m {
                    out.push(Edge::new(s as i32, e as i32, mc, l.num_arabic, l.num_arabic, c - 100, K_NUM));
                }
            }
            out.push(Edge::new(s as i32, e as i32, digits_str(v, true), l.num_arabic, l.num_arabic, c + 2000, K_NUM));
            out.push(Edge::new(s as i32, e as i32, kanji_num(v), l.num_kanji, l.num_kanji, c + 500, K_NUM));
            for (sg, sc) in [(u("まいなす"), '-'), (u("ぷらす"), '+')] {
                let k = sg.len();
                if s >= k && r[s - k..s] == sg[..] {
                    let mut surf = vec![sc];
                    surf.extend(digits_str(v, false));
                    out.push(Edge::new((s - k) as i32, e as i32, surf, l.num_arabic, l.num_arabic, c + 500, K_NUM));
                }
            }
        }
    }
    // decimals: number + てん + digit-readings
    let fig: Vec<(Ustr, char)> = [
        ("れい", '0'), ("ぜろ", '0'), ("いち", '1'), ("に", '2'), ("さん", '3'), ("よん", '4'), ("ご", '5'),
        ("ろく", '6'), ("なな", '7'), ("はち", '8'), ("きゅう", '9'),
    ]
    .iter()
    .map(|(k, c)| (u(k), *c))
    .collect();
    let ten = u("てん");
    let mut p = 0usize;
    while p + 2 <= r.len() {
        let found = (p..=r.len() - 2).find(|&i| starts_with(r, i, &ten));
        let Some(pp) = found else { break };
        p = pp;
        let mut frac: Ustr = Vec::new();
        let mut e = p + 2;
        loop {
            let mut hit = false;
            for f in &fig {
                if starts_with(r, e, &f.0) {
                    frac.push(f.1);
                    e += f.0.len();
                    hit = true;
                    break;
                }
            }
            if !hit {
                break;
            }
        }
        if !frac.is_empty() {
            for s in 0..p {
                let head = &r[s..p];
                let v = if head == u("れい") || head == u("ぜろ") {
                    0
                } else {
                    match parse_num1(head) {
                        Some(v) if !number_continues(r, s, p) => v,
                        _ => continue,
                    }
                };
                let mut surf = digits_str(v, false);
                surf.push('.');
                surf.extend(frac.iter());
                out.push(Edge::new(s as i32, e as i32, surf, l.num_arabic, l.num_arabic, 3000, K_NUM));
                break;
            }
        }
        p += 1;
    }
}

fn special_edges(l: &Lex, r: &[char], out: &mut Vec<Edge>) {
    let cost = 4500i32;
    for (f, s) in [(u("ひとり"), u("1人")), (u("ふたり"), u("2人")), (u("はたち"), u("20歳"))] {
        let mut from = 0usize;
        while from + f.len() <= r.len() {
            let Some(i) = (from..=r.len() - f.len()).find(|&i| starts_with(r, i, &f)) else { break };
            out.push(Edge::new(i as i32, (i + f.len()) as i32, s.clone(), l.noun, l.noun, cost + 2000, K_NUM));
            from = i + 1;
        }
    }
    let days: Vec<(Ustr, i64)> = [
        ("ついたち", 1), ("ふつか", 2), ("みっか", 3), ("よっか", 4), ("いつか", 5), ("むいか", 6), ("なのか", 7),
        ("ようか", 8), ("ここのか", 9), ("とおか", 10), ("はつか", 20),
    ]
    .iter()
    .map(|(k, v)| (u(k), *v))
    .collect();
    for w in &days {
        let mut from = 0usize;
        while from + w.0.len() <= r.len() {
            let Some(e0) = (from..=r.len() - w.0.len()).find(|&i| starts_with(r, i, &w.0)) else { break };
            let e = e0 + w.0.len();
            let mut starts: Vec<(usize, i64)> = vec![(e0, w.1)];
            if w.1 < 10 {
                for s in e0.saturating_sub(12)..e0 {
                    if let Some((t, false)) = parse_num_full(&r[s..e0]) {
                        if t % 10 == 0 && t > 0 && t <= 30 {
                            starts.push((s, t + w.1));
                        }
                    }
                }
            }
            for sv in starts {
                if sv.1 > 31 {
                    continue;
                }
                let span = (e - sv.0) as i32;
                let mut a = digits_str(sv.1, false);
                a.push('日');
                out.push(Edge::new(sv.0 as i32, e as i32, a, l.noun, l.noun, cost + 200 * span, K_NUM));
                let mut b = digits_str(sv.1, true);
                b.push('日');
                out.push(Edge::new(sv.0 as i32, e as i32, b, l.noun, l.noun, cost + 2000 + 200 * span, K_NUM));
                let mut c = kanji_num(sv.1);
                c.push('日');
                out.push(Edge::new(sv.0 as i32, e as i32, c, l.noun, l.noun, cost + 300 + 200 * span, K_NUM));
            }
            from = e0 + 1;
        }
    }
}

fn pict_word(l: &Lex, en: &Ent) -> bool {
    if en.cost < 8000 || en.cost > 9399 || (en.cost >= 9000 && en.cost < 9300) {
        return false;
    }
    let p = &l.s_blob[en.soff as usize..en.soff as usize + en.slen as usize];
    let mut pict = false;
    for &c in p {
        if (0x3041..=0x30FF).contains(&c) || (0x4E00..=0x9FFF).contains(&c) {
            return false;
        }
        if (0xD800..=0xDBFF).contains(&c) || (0x2600..=0x27BF).contains(&c) || (0x2B00..=0x2BFF).contains(&c) {
            pict = true;
        }
    }
    en.cost >= 9300 || pict
}
fn symbol_word(s: &[char]) -> bool {
    let mut sym = false;
    for &c in s {
        if (0x2190..=0x21FF).contains(&(c as u32))
            || (0x25A0..=0x27BF).contains(&(c as u32))
            || (0x2900..=0x2BFF).contains(&(c as u32))
            || (0x1F000..=0x1FAFF).contains(&(c as u32))
        {
            sym = true;
        } else if c != '\u{FE0F}' && c != '\u{200D}' {
            return false;
        }
    }
    sym
}
fn has_latin_word(s: &[char]) -> bool {
    let is_lat = |c: char| c.is_ascii_alphabetic();
    let mut i = 0;
    while i < s.len() {
        if !is_lat(s[i]) {
            i += 1;
            continue;
        }
        let mut j = i;
        let mut lower = false;
        while j < s.len() && (is_lat(s[j]) || s[j].is_ascii_digit()) {
            lower |= s[j].is_ascii_lowercase();
            j += 1;
        }
        if lower || j - i > 5 {
            return true;
        }
        i = j;
    }
    false
}

// ------------------------------------------------------------------ abbreviations (あいえむいー -> IME)
fn letters() -> Vec<(Ustr, char)> {
    [
        ("えー", 'A'), ("えい", 'A'), ("びー", 'B'), ("びい", 'B'), ("びぃ", 'B'), ("しー", 'C'), ("でぃー", 'D'), ("でー", 'D'),
        ("いー", 'E'), ("えふ", 'F'), ("じー", 'G'), ("えいち", 'H'), ("えっち", 'H'), ("あい", 'I'), ("じぇー", 'J'), ("じぇい", 'J'),
        ("けー", 'K'), ("けい", 'K'), ("える", 'L'), ("えむ", 'M'), ("えぬ", 'N'), ("おー", 'O'), ("ぴー", 'P'), ("きゅー", 'Q'),
        ("きゅう", 'Q'), ("あーる", 'R'), ("えす", 'S'), ("てぃー", 'T'), ("てぃ", 'T'), ("てー", 'T'), ("ゆー", 'U'), ("ぶい", 'V'),
        ("ゔぃ", 'V'), ("ぶぃ", 'V'), ("だぶりゅー", 'W'), ("だぶる", 'W'), ("だぶるー", 'W'), ("えっくす", 'X'), ("わい", 'Y'), ("ぜっと", 'Z'),
    ]
    .iter()
    .map(|(k, c)| (u(k), *c))
    .collect()
}
const ABBR_MIN: usize = 3;
const ABBR_MAX: usize = 8;
const ABBR_COST: i32 = 7120;
fn abbr_spans(r: &[char], s: usize, cur: &mut Ustr, out: &mut Vec<(usize, Ustr)>, ls: &[(Ustr, char)]) {
    if cur.len() >= ABBR_MIN {
        out.push((s, cur.clone()));
    }
    if cur.len() >= ABBR_MAX {
        return;
    }
    for lt in ls {
        if starts_with(r, s, &lt.0) {
            cur.push(lt.1);
            abbr_spans(r, s + lt.0.len(), cur, out, ls);
            cur.pop();
        }
    }
}
fn abbr_edges(l: &Lex, r: &[char], out: &mut Vec<Edge>) {
    if l.abbr_lid < 0 {
        return;
    }
    let ls = letters();
    for s in 0..r.len() {
        let mut sp = Vec::new();
        let mut cur = Vec::new();
        abbr_spans(r, s, &mut cur, &mut sp, &ls);
        for p in sp {
            out.push(Edge::new(s as i32, p.0 as i32, p.1, l.abbr_lid, l.abbr_rid, ABBR_COST, K_DICT));
        }
    }
}
fn whole_abbr(r: &[char]) -> Ustr {
    let ls = letters();
    let mut sp = Vec::new();
    let mut cur = Vec::new();
    abbr_spans(r, 0, &mut cur, &mut sp, &ls);
    let mut best: Ustr = Vec::new();
    for p in sp {
        if p.0 == r.len() && (best.is_empty() || p.1.len() < best.len()) {
            best = p.1;
        }
    }
    best
}

// ------------------------------------------------------------------ math expressions (mathx)
mod mathx {
    use super::*;
    pub fn vars() -> Vec<(Ustr, Ustr)> {
        [("えっくす", "x"), ("わい", "y"), ("ぜっと", "z"), ("えー", "a"), ("びー", "b"), ("しー", "c"), ("えぬ", "n"),
         ("えむ", "m"), ("けー", "k"), ("てぃー", "t"), ("ぴー", "p"), ("きゅー", "q"), ("あーる", "r"), ("ぱい", "π"),
         ("しーた", "θ"), ("あるふぁ", "α"), ("べーた", "β")]
        .iter().map(|(a, b)| (u(a), u(b))).collect()
    }
    pub fn ops() -> Vec<(Ustr, Ustr)> {
        [("ぷらす", "+"), ("たす", "+"), ("まいなす", "-"), ("ひく", "-"), ("かける", "×"), ("わる", "÷"), ("いこーる", "="),
         ("のっといこーる", "≠"), ("だいなりいこーる", "≧"), ("しょうなりいこーる", "≦"), ("だいなり", ">"), ("しょうなり", "<")]
        .iter().map(|(a, b)| (u(a), u(b))).collect()
    }
    pub const SYMBOLS: &[(&str, &str)] = &[
        ("かける", "×"), ("わる", "÷"), ("たす", "+"), ("にじょう", "²"), ("さんじょう", "³"), ("じじょう", "²"), ("へいほうこん", "√"),
    ];
    fn nums(r: &[char], i: usize, out: &mut Vec<(usize, Ustr)>) {
        let hi = r.len().min(i + 12);
        for e in (i + 1..=hi).rev() {
            if let Some(v) = parse_num1(&r[i..e]) {
                out.push((e, digits_str(v, false)));
                return;
            }
        }
    }
    fn terms(r: &[char], i: usize, out: &mut Vec<(usize, Ustr)>, depth: i32) {
        let vs = vars();
        let mut base: Vec<(usize, Ustr)> = Vec::new();
        let mut n0 = Vec::new();
        nums(r, i, &mut n0);
        for a in &n0 {
            base.push(a.clone());
            for v in &vs {
                if starts_with(r, a.0, &v.0) {
                    let mut s = a.1.clone();
                    s.extend(v.1.iter());
                    base.push((a.0 + v.0.len(), s));
                }
            }
            if starts_with(r, a.0, &u("ぶんの")) {
                let mut n1 = Vec::new();
                nums(r, a.0 + 3, &mut n1);
                for b in n1 {
                    let mut s = b.1.clone();
                    s.push('/');
                    s.extend(a.1.iter());
                    base.push((b.0, s));
                }
            }
        }
        for v in &vs {
            if starts_with(r, i, &v.0) {
                base.push((i + v.0.len(), v.1.clone()));
            }
        }
        let n0b = base.len();
        for k in 0..n0b {
            let mut cur = vec![base[k].clone()];
            for _ in 0..2 {
                let mut nxt = Vec::new();
                for c in &cur {
                    if !c.1.is_empty() && !c.1[c.1.len() - 1].is_ascii_digit() {
                        for v in &vs {
                            if starts_with(r, c.0, &v.0) {
                                let mut s = c.1.clone();
                                s.extend(v.1.iter());
                                nxt.push((c.0 + v.0.len(), s));
                            }
                        }
                    }
                }
                base.extend(nxt.iter().cloned());
                cur = nxt;
            }
        }
        for rt in [u("るーと"), u("へいほうこん")] {
            if depth < 2 && starts_with(r, i, &rt) {
                let mut t = Vec::new();
                terms(r, i + rt.len(), &mut t, depth + 1);
                for x in t {
                    let mut s = u("√");
                    s.extend(x.1.iter());
                    base.push((x.0, s));
                }
            }
        }
        let nb = base.len();
        for k in 0..nb {
            if starts_with(r, base[k].0, &u("のへいほうこん")) {
                let mut s = u("√");
                s.extend(base[k].1.iter());
                base.push((base[k].0 + 7, s));
            }
        }
        let sup: Vec<char> = "⁰¹²³⁴⁵⁶⁷⁸⁹".chars().collect();
        for b in &base {
            out.push(b.clone());
            for j in [b.0, b.0 + if starts_with(r, b.0, &u("の")) { 1 } else { 0 }] {
                if starts_with(r, j, &u("じじょう")) {
                    let mut s = b.1.clone();
                    s.push('²');
                    out.push((j + 4, s));
                }
                let mut ex = Vec::new();
                nums(r, j, &mut ex);
                for e in ex {
                    if !starts_with(r, e.0, &u("じょう")) {
                        continue;
                    }
                    let mut s = b.1.clone();
                    for c in &e.1 {
                        s.push(sup[(*c as u32 - '0' as u32) as usize]);
                    }
                    out.push((e.0 + 3, s));
                }
            }
        }
    }
    pub fn whole(r: &[char]) -> Ustr {
        fn go(r: &[char], i: usize, acc: Ustr, mathy: bool, best: &mut Ustr) {
            let mut t = Vec::new();
            terms(r, i, &mut t, 0);
            for x in t {
                let mut a = acc.clone();
                a.extend(x.1.iter());
                let m = mathy || x.1.iter().any(|&c| "/√⁰¹²³⁴⁵⁶⁷⁸⁹".contains(c));
                if x.0 == r.len() {
                    if m && (best.is_empty() || a.len() < best.len()) {
                        *best = a;
                    }
                    continue;
                }
                for o in ops() {
                    if starts_with(r, x.0, &o.0) {
                        let mut a2 = a.clone();
                        a2.extend(o.1.iter());
                        go(r, x.0 + o.0.len(), a2, true, best);
                    }
                }
            }
        }
        let mut best = Vec::new();
        go(r, 0, Vec::new(), false, &mut best);
        best
    }
}

// ------------------------------------------------------------------ romaji tables for typo edges (typo::init)
struct Romaji {
    r2k: HashMap<String, Ustr>,
    k2r: HashMap<char, String>,
    k2r2: HashMap<Ustr, String>,
}
fn romaji() -> Romaji {
    let mut r2k: HashMap<String, Ustr> = HashMap::new();
    let mut k2r: HashMap<char, String> = HashMap::new();
    let mut k2r2: HashMap<Ustr, String> = HashMap::new();
    let vv = ['a', 'i', 'u', 'e', 'o'];
    let rows: Vec<(Ustr, &str)> = [
        ("あいうえお", ""), ("かきくけこ", "k"), ("さしすせそ", "s"), ("たちつてと", "t"), ("なにぬねの", "n"),
        ("はひふへほ", "h"), ("まみむめも", "m"), ("らりるれろ", "r"), ("がぎぐげご", "g"), ("ざじずぜぞ", "z"),
        ("だぢづでど", "d"), ("ばびぶべぼ", "b"), ("ぱぴぷぺぽ", "p"), ("ぁぃぅぇぉ", "x"),
    ]
    .iter().map(|(a, b)| (u(a), *b)).collect();
    for rw in &rows {
        for i in 0..5 {
            let k = format!("{}{}", rw.1, vv[i]);
            r2k.insert(k.clone(), vec![rw.0[i]]);
            k2r.insert(rw.0[i], k);
        }
    }
    let extra: Vec<(Ustr, &str)> = [
        ("や", "ya"), ("ゆ", "yu"), ("よ", "yo"), ("わ", "wa"), ("を", "wo"), ("ゃ", "xya"), ("ゅ", "xyu"), ("ょ", "xyo"),
        ("っ", "xtu"), ("ゎ", "xwa"), ("ー", "-"), ("し", "shi"), ("ち", "chi"), ("つ", "tsu"), ("ふ", "fu"), ("じ", "ji"), ("ゔ", "vu"),
    ]
    .iter().map(|(a, b)| (u(a), *b)).collect();
    for x in &extra {
        r2k.insert(x.1.to_string(), x.0.clone());
    }
    for (c, s) in [('や', "ya"), ('ゆ', "yu"), ('よ', "yo"), ('わ', "wa"), ('を', "wo"), ('ゃ', "xya"), ('ゅ', "xyu"), ('ょ', "xyo"), ('ゎ', "xwa"), ('ー', "-"), ('ゔ', "vu")] {
        k2r.insert(c, s.to_string());
    }
    let yoon: Vec<(char, &str)> = [
        ('き', "ky"), ('し', "sy"), ('ち', "ty"), ('に', "ny"), ('ひ', "hy"), ('み', "my"), ('り', "ry"), ('ぎ', "gy"),
        ('じ', "zy"), ('び', "by"), ('ぴ', "py"), ('ぢ', "dy"),
    ]
    .to_vec();
    let small = ['ゃ', 'ゅ', 'ょ'];
    let yv = ['a', 'u', 'o'];
    for y in &yoon {
        for i in 0..3 {
            let k2 = vec![y.0, small[i]];
            let r = format!("{}{}", y.1, yv[i]);
            r2k.insert(r.clone(), k2.clone());
            k2r2.insert(k2, r);
        }
    }
    let more: Vec<(&str, Ustr)> = [
        ("sha", u("しゃ")), ("shu", u("しゅ")), ("sho", u("しょ")), ("cha", u("ちゃ")), ("chu", u("ちゅ")), ("cho", u("ちょ")),
        ("ja", u("じゃ")), ("ju", u("じゅ")), ("jo", u("じょ")), ("she", u("しぇ")), ("che", u("ちぇ")), ("je", u("じぇ")),
        ("fa", u("ふぁ")), ("fi", u("ふぃ")), ("fe", u("ふぇ")), ("fo", u("ふぉ")), ("thi", u("てぃ")), ("dhi", u("でぃ")),
        ("wi", u("うぃ")), ("we", u("うぇ")), ("tsa", u("つぁ")), ("tso", u("つぉ")), ("va", u("ゔぁ")), ("vi", u("ゔぃ")),
        ("ve", u("ゔぇ")), ("vo", u("ゔぉ")),
    ]
    .to_vec();
    for x in &more {
        r2k.insert(x.0.to_string(), x.1.clone());
        if x.1.len() == 2 && !k2r2.contains_key(&x.1) {
            k2r2.insert(x.1.clone(), x.0.to_string());
        }
    }
    Romaji { r2k, k2r, k2r2 }
}
fn is_vowel(c: char) -> bool {
    matches!(c, 'a' | 'i' | 'u' | 'e' | 'o')
}
fn to_romaji(rm: &Romaji, k: &[char]) -> Option<String> {
    let mut rom = String::new();
    let mut i = 0;
    while i < k.len() {
        if i + 1 < k.len() {
            if let Some(v) = rm.k2r2.get(&k[i..i + 2]) {
                rom.push_str(v);
                i += 2;
                continue;
            }
        }
        if k[i] == 'ん' {
            rom.push_str("nn");
            i += 1;
            continue;
        }
        if k[i] == 'っ' {
            let mut nx = String::new();
            if i + 1 < k.len() {
                if i + 2 <= k.len() {
                    if let Some(v) = rm.k2r2.get(&k[i + 1..(i + 2).min(k.len())]) {
                        if k[i + 1..].len() >= 2 {
                            nx = v.clone();
                        }
                    }
                }
                if nx.is_empty() {
                    if let Some(v) = rm.k2r.get(&k[i + 1]) {
                        nx = v.clone();
                    }
                }
            }
            if !nx.is_empty() && !is_vowel(nx.chars().next().unwrap()) && !nx.starts_with('n') && !nx.starts_with('-') {
                rom.push(nx.chars().next().unwrap());
            } else {
                rom.push_str("xtu");
            }
            i += 1;
            continue;
        }
        match rm.k2r.get(&k[i]) {
            Some(v) => {
                rom.push_str(v);
                i += 1;
            }
            None => return None,
        }
    }
    Some(rom)
}
fn to_kana(rm: &Romaji, r: &str) -> Option<Ustr> {
    let rb: Vec<char> = r.chars().collect();
    let mut out: Ustr = Vec::new();
    let mut i = 0;
    while i < rb.len() {
        let c = rb[i];
        if c == 'n' {
            if i + 1 < rb.len() && rb[i + 1] == 'n' {
                let nv = i + 2 < rb.len() && (is_vowel(rb[i + 2]) || rb[i + 2] == 'y');
                out.push('ん');
                i += if nv { 1 } else { 2 };
                continue;
            }
            if i + 1 == rb.len() || (!is_vowel(rb[i + 1]) && rb[i + 1] != 'y') {
                out.push('ん');
                i += 1;
                continue;
            }
        }
        if i + 1 < rb.len() && c == rb[i + 1] && !is_vowel(c) && c != 'n' && c != '-' {
            out.push('っ');
            i += 1;
            continue;
        }
        let mut hit = false;
        for len in (1..=3usize).rev() {
            if i + len > rb.len() {
                continue;
            }
            let key: String = rb[i..i + len].iter().collect();
            if let Some(v) = rm.r2k.get(&key) {
                out.extend(v.iter());
                i += len;
                hit = true;
                break;
            }
        }
        if !hit {
            return None;
        }
    }
    Some(out)
}
fn neighbors(c: char) -> Vec<char> {
    let rows = ["qwertyuiop", "asdfghjkl", "zxcvbnm"];
    let mut out = Vec::new();
    for rr in 0..3 {
        let row: Vec<char> = rows[rr].chars().collect();
        for j in 0..row.len() {
            if row[j] != c {
                continue;
            }
            if j > 0 {
                out.push(row[j - 1]);
            }
            if j + 1 < row.len() {
                out.push(row[j + 1]);
            }
            if rr > 0 {
                let o: Vec<char> = rows[rr - 1].chars().collect();
                if j < o.len() {
                    out.push(o[j]);
                }
                if j + 1 < o.len() {
                    out.push(o[j + 1]);
                }
            }
            if rr < 2 {
                let o: Vec<char> = rows[rr + 1].chars().collect();
                if j >= 1 && j - 1 < o.len() {
                    out.push(o[j - 1]);
                }
                if j < o.len() {
                    out.push(o[j]);
                }
            }
        }
    }
    out
}
fn kana_alts(c: char) -> Vec<char> {
    const FLICK_INV: &str = "あいうえおかがきぎけげただつづとど いきぎ うつづ えけげ おとど かあいえきくけこさしせなぬの がぎぐげござじぜ きいし ぎじ くぬ けえせ げぜ この さかきけしすせそはふほ ざがぎげじずぜぞばぱぶぷぼぽ しき じぎ すふ ずぶぷ せけ ぜげ そほ ぞぼぽ たあうおちつてとなにねまむも だぢづでど ちに つうむ てね とおも なかがくぐこごただちぢてでにぬねのはばぱひびぴへべぺやゆよ にちぢひびぴ ぬくぐゆ ねてでへべぺ のこごよ はさすそなにねひふへほらるろ ばざずぞびぶべぼ ぱぴぷぺぽ ひに ふする ぶず へね ほそろ ぼぞ まただつづとどみむめもや むつづ もとど やなぬのまみめゆよらりれわん ゆぬん よの らはばぱふぶぷほぼぽやりるれろ るふぶぷ ろほぼぽ わやゆよをん んゆ";
    let daku: Vec<Ustr> = ["かが", "きぎ", "くぐ", "けげ", "こご", "さざ", "しじ", "すず", "せぜ", "そぞ", "ただ", "ちぢ", "つづ", "てで", "とど", "はばぱ", "ひびぴ", "ふぶぷ", "へべぺ", "ほぼぽ", "うゔ"].iter().map(|s| u(s)).collect();
    let small: Vec<Ustr> = ["あぁ", "いぃ", "うぅ", "えぇ", "おぉ", "つっ", "やゃ", "ゆゅ", "よょ", "わゎ"].iter().map(|s| u(s)).collect();
    let mut out: Vec<char> = Vec::new();
    let mut add = |x: char, out: &mut Vec<char>| {
        if x != c && !out.contains(&x) {
            out.push(x);
        }
    };
    for grp in FLICK_INV.split(' ') {
        let g: Vec<char> = grp.chars().collect();
        if !g.is_empty() && g[0] == c {
            for &x in &g[1..] {
                add(x, &mut out);
            }
        }
    }
    for g in daku.iter().chain(small.iter()) {
        if g.contains(&c) {
            for &x in g {
                add(x, &mut out);
            }
        }
    }
    out
}
fn particles_only(s: &[char]) -> bool {
    s.iter().all(|c| "のにをがとはでへもや".contains(*c))
}

// ------------------------------------------------------------------ typo edges (S7)
const TYPO_COST: i32 = 3000;
const TYPO_MAX: usize = 32;
const MAX_WORD_KANA: i32 = 8;
const TYPO_KEEP: f64 = 2000.0;

use std::sync::Mutex;
static VOICE: Mutex<(Vec<Ustr>, Vec<i32>, Ustr)> = Mutex::new((Vec::new(), Vec::new(), Vec::new()));

struct TypoCtx {
    n: i32,
    by_end: Vec<Vec<usize>>,
    by_start: Vec<Vec<usize>>,
    f: Vec<f64>,
    b: Vec<f64>,
    lmin: Vec<f64>,
    rmin: Vec<f64>,
    best: f64,
}
fn build_typo_ctx(l: &Lex, e: &[Edge], n: i32) -> TypoCtx {
    let mut c = TypoCtx {
        n,
        by_end: vec![Vec::new(); (n + 1) as usize],
        by_start: vec![Vec::new(); (n + 1) as usize],
        f: vec![1e18; e.len()],
        b: vec![1e18; e.len()],
        lmin: vec![1e18; (n + 1) as usize],
        rmin: vec![1e18; (n + 1) as usize],
        best: 1e18,
    };
    for (i, ed) in e.iter().enumerate() {
        c.by_end[ed.e as usize].push(i);
        c.by_start[ed.s as usize].push(i);
    }
    for pos in 0..=n {
        let starts = c.by_start[pos as usize].clone();
        for i in starts {
            let mut best = if pos == 0 { l.conn(0, e[i].lid) as f64 } else { 1e18 };
            for &j in &c.by_end[pos as usize] {
                best = best.min(c.f[j] + l.conn(e[j].rid, e[i].lid) as f64);
            }
            if best < 1e17 {
                c.f[i] = best + e[i].cost as f64;
            }
        }
    }
    for pos in (0..=n).rev() {
        let ends = c.by_end[pos as usize].clone();
        for i in ends {
            let mut best = if pos == n { l.conn(e[i].rid, 0) as f64 } else { 1e18 };
            for &k in &c.by_start[pos as usize] {
                best = best.min(c.b[k] + l.conn(e[i].rid, e[k].lid) as f64);
            }
            if best < 1e17 {
                c.b[i] = best + e[i].cost as f64;
            }
        }
    }
    for &j in &c.by_end[n as usize] {
        c.best = c.best.min(c.f[j] + l.conn(e[j].rid, 0) as f64);
    }
    c.lmin[0] = 0.0;
    c.rmin[n as usize] = 0.0;
    for pos in 1..=n {
        for &j in &c.by_end[pos as usize] {
            let v = c.f[j];
            if v < c.lmin[pos as usize] {
                c.lmin[pos as usize] = v;
            }
        }
    }
    for pos in 0..n {
        for &k in &c.by_start[pos as usize] {
            let v = c.b[k];
            if v < c.rmin[pos as usize] {
                c.rmin[pos as usize] = v;
            }
        }
    }
    c
}
impl TypoCtx {
    fn through(&self, l: &Lex, base: &[Edge], s: i32, e: i32, lid: i32, rid: i32, cost: i32) -> f64 {
        let mut left = if s == 0 { l.conn(0, lid) as f64 } else { 1e18 };
        for &j in &self.by_end[s as usize] {
            left = left.min(self.f[j] + l.conn(base[j].rid, lid) as f64);
        }
        let mut right = if e == self.n { l.conn(rid, 0) as f64 } else { 1e18 };
        for &k in &self.by_start[e as usize] {
            right = right.min(l.conn(rid, base[k].lid) as f64 + self.b[k]);
        }
        if left >= 1e17 || right >= 1e17 {
            return 1e18;
        }
        left + cost as f64 + right
    }
}

struct AltReading {
    a: Ustr,
    p: i32,
    q: i32,
}
fn prep_alts(r: &[char], alts: &[Ustr]) -> Vec<AltReading> {
    let n = r.len() as i32;
    let mut out = Vec::new();
    for a in alts {
        let m = a.len() as i32;
        let mut p = 0;
        while p < n && p < m && a[p as usize] == r[p as usize] {
            p += 1;
        }
        let mut q = 0;
        while q < n - p && q < m - p && a[(m - 1 - q) as usize] == r[(n - 1 - q) as usize] {
            q += 1;
        }
        if p == n && m == n {
            continue;
        }
        out.push(AltReading { a: a.clone(), p, q });
    }
    out
}
fn window_alts(alts: &[AltReading], n: i32, w: i32, we: i32) -> Vec<Ustr> {
    let mut out = Vec::new();
    for x in alts {
        if x.p < w || n - x.q > we {
            continue;
        }
        let len = (we - w) + x.a.len() as i32 - n;
        if len < 1 {
            continue;
        }
        let s: Ustr = x.a[w as usize..w as usize + len as usize].to_vec();
        if s.iter().all(|&c| is_kana(c)) {
            out.push(s);
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn typo_add_edges(
    l: &Lex, r: &[char], off: i32, base: &[Edge], ctx: &TypoCtx, out: &mut Vec<Edge>,
    rom_in: Option<&str>, seg_alts: Option<&[Ustr]>,
) {
    if r.len() < 2 {
        return;
    }
    let rm = romaji();
    let mut vars: Vec<String> = Vec::new();
    let mut rom = String::new();
    if let Some(ri) = rom_in {
        rom = ri.to_string();
    } else if seg_alts.is_none() {
        match to_romaji(&rm, r) {
            Some(v) => rom = v,
            None => return,
        }
    }
    if !rom.is_empty() {
        let rb: Vec<char> = rom.chars().collect();
        let big_r = rb.len() as i32;
        for i in 0..big_r {
            let mut v: String = rb[..i as usize].iter().collect();
            v.extend(rb[i as usize + 1..].iter());
            vars.push(v);
            for c in neighbors(rb[i as usize]) {
                let mut v = rom.clone();
                v.replace_range(i as usize..i as usize + 1, &c.to_string());
                vars.push(v);
            }
            if i + 1 < big_r && rb[i as usize] != rb[i as usize + 1] {
                let mut vc: Vec<char> = rb.clone();
                vc.swap(i as usize, i as usize + 1);
                vars.push(vc.iter().collect());
            }
        }
        for i in 0..=big_r {
            let mut ins = "aiueonxtsrkhy".to_string();
            if i > 0 {
                ins.push(rb[i as usize - 1]);
            }
            for c in ins.chars() {
                let mut v: String = rb[..i as usize].iter().collect();
                v.push(c);
                v.extend(rb[i as usize..].iter());
                vars.push(v);
            }
        }
    }
    let mut kvars: Vec<Ustr> = Vec::new();
    for i in 0..r.len() {
        let c = r[i];
        if i + 1 < r.len() && c != r[i + 1] {
            let mut v = r.to_vec();
            v.swap(i, i + 1);
            kvars.push(v);
        }
        for a in kana_alts(c) {
            let mut v = r.to_vec();
            v[i] = a;
            kvars.push(v);
        }
        if r.len() > 2 {
            let mut v = r.to_vec();
            v.remove(i);
            kvars.push(v);
        }
    }
    for i in 0..=r.len() {
        for m in ['ー', 'っ', 'ん'] {
            let mut v = r.to_vec();
            v.insert(i, m);
            kvars.push(v);
        }
    }
    // voice alts
    let mut vextra: HashMap<Ustr, i32> = HashMap::new();
    {
        let (alts, costs, full) = VOICE.lock().unwrap().clone();
        if !alts.is_empty() && off as usize + r.len() <= full.len() {
            let n = full.len() as i32;
            let (w0, w1) = (off, off + r.len() as i32);
            for (ai, a) in alts.iter().enumerate() {
                let m = a.len() as i32;
                let mut p = 0;
                while p < n && p < m && full[p as usize] == a[p as usize] {
                    p += 1;
                }
                let mut q = 0;
                while q < n - p && q < m - p && full[(n - 1 - q) as usize] == a[(m - 1 - q) as usize] {
                    q += 1;
                }
                if p < w0 || n - q > w1 || w1 - w0 + m - n <= 0 {
                    continue;
                }
                let seg: Ustr = a[w0 as usize..(w0 + (w1 - w0 + m - n)) as usize].to_vec();
                vextra.insert(seg.clone(), costs[ai] - TYPO_COST);
                kvars.push(seg);
            }
        }
    }
    let mut base_keys: HashSet<Ustr> = HashSet::new();
    for ed in base {
        let mut k = ed.surf.clone();
        k.push(char::from_u32(0x110000 + ed.s as u32).unwrap_or('\u{FFFD}'));
        k.push(char::from_u32(0x110000 + ed.e as u32).unwrap_or('\u{FFFD}'));
        base_keys.insert(k);
    }
    #[derive(Clone)]
    struct Cand {
        s: i32, e: i32, surf: Ustr, lid: i32, rid: i32, cost: i32, total: f64,
    }
    let mut cand: HashMap<Ustr, Cand> = HashMap::new();
    let n1 = r.len() as i32;
    let mut seen_k: HashSet<Ustr> = HashSet::new();
    let mut all: Vec<Ustr> = Vec::new();
    let mut tried: HashSet<String> = HashSet::new();
    for v in &vars {
        if tried.insert(v.clone()) {
            if let Some(k2) = to_kana(&rm, v) {
                if k2 != r && seen_k.insert(k2.clone()) {
                    all.push(k2);
                }
            }
        }
    }
    if let Some(sa) = seg_alts {
        for v in sa {
            if *v != r && seen_k.insert(v.clone()) {
                all.push(v.clone());
            }
        }
    }
    for v in kvars {
        if seen_k.insert(v.clone()) {
            all.push(v);
        }
    }
    let mut add = |cand: &mut HashMap<Ustr, Cand>, s: i32, e: i32, sf: Ustr, lid: i32, rid: i32, c: i32| {
        let mut bk = sf.clone();
        bk.push(char::from_u32(0x110000 + (s + off) as u32).unwrap_or('\u{FFFD}'));
        bk.push(char::from_u32(0x110000 + (e + off) as u32).unwrap_or('\u{FFFD}'));
        if base_keys.contains(&bk) {
            return;
        }
        let mut ck = sf.clone();
        ck.push(char::from_u32(0x110000 + s as u32).unwrap_or('\u{FFFD}'));
        ck.push(char::from_u32(0x110000 + e as u32).unwrap_or('\u{FFFD}'));
        if let Some(old) = cand.get(&ck) {
            if c >= old.cost {
                return;
            }
        }
        if ctx.lmin[(s + off) as usize] + c as f64 + ctx.rmin[(e + off) as usize] - 2000.0 > ctx.best + TYPO_KEEP {
            return;
        }
        let t = ctx.through(l, base, s + off, e + off, lid, rid, c);
        if t > ctx.best + TYPO_KEEP {
            return;
        }
        cand.insert(ck, Cand { s, e, surf: sf, lid, rid, cost: c, total: t });
    };
    for kv in &all {
        let k2 = kv;
        let tc = TYPO_COST + vextra.get(kv).copied().unwrap_or(0);
        let n2 = k2.len() as i32;
        let mut p = 0;
        while p < n1 && p < n2 && r[p as usize] == k2[p as usize] {
            p += 1;
        }
        let mut q = 0;
        while q < n1 - p && q < n2 - p && r[(n1 - 1 - q) as usize] == k2[(n2 - 1 - q) as usize] {
            q += 1;
        }
        if !vextra.contains_key(kv)
            && particles_only(&r[p as usize..(n1 - q) as usize])
            && particles_only(&k2[p as usize..(n2 - q) as usize])
        {
            continue;
        }
        for s2 in (p - MAX_WORD_KANA + 1).max(0)..=p.min(n2 - 1) {
            let emin = (n2 - q).max(s2 + 2);
            let emax = n2.min(s2 + MAX_WORD_KANA);
            if emin > emax {
                continue;
            }
            // prefix bloom walk like Head::to
            let mut h: u64 = 1469598103934665603;
            let mut ecur = s2;
            let mut advance = |l: &Lex, upto: i32, h: &mut u64, ecur: &mut i32| -> bool {
                while *ecur < upto {
                    *h ^= k2[*ecur as usize] as u64;
                    *h = h.wrapping_mul(1099511628211);
                    *ecur += 1;
                }
                l.maybe_head(*h ^ (*h >> 29))
            };
            for e2 in emin..=emax {
                if !advance(l, e2, &mut h, &mut ecur) {
                    break;
                }
                let (s, e) = (s2, e2 - n2 + n1);
                if e <= s || e > n1 || e < n1 - q {
                    continue;
                }
                if ctx.lmin[(s + off) as usize] + tc as f64 + ctx.rmin[(e + off) as usize] - 2000.0 > ctx.best + TYPO_KEEP {
                    continue;
                }
                let key16: Vec<u16> = k2[s2 as usize..e2 as usize].iter().map(|&c| c as u16).collect();
                let Some((a, b)) = l.find(&key16) else { continue };
                for j in a..b {
                    let en = l.ent(j);
                    if en.cost >= ENG_COST || pict_word(l, &en) {
                        continue;
                    }
                    let c0 = en.cost + tc;
                    if ctx.lmin[(s + off) as usize] + c0 as f64 + ctx.rmin[(e + off) as usize] - 2000.0 > ctx.best + TYPO_KEEP {
                        continue;
                    }
                    if ctx.through(l, base, s + off, e + off, en.lid as i32, en.rid as i32, c0) > ctx.best + TYPO_KEEP {
                        continue;
                    }
                    let sf = l.surf(&en);
                    add(&mut cand, s, e, sf, en.lid as i32, en.rid as i32, en.cost + tc);
                }
            }
        }
        // Two-word coverings (engine.cpp typo::add_edges): when the
        // changed span straddles two dictionary words (つぉあ -> と|される),
        // join the pair into one K_TYPO edge. The split point m lies in
        // or at the edge of the changed span [p, n2-q); the first word
        // may be a single kana (と・を …). Pairs are chosen by the cost
        // of the best full path through the joined edge, cheapest 3 per
        // (s2, e2), among the cheapest 6 entries of each word.
        {
            let look = |a0: i32, a1: i32| -> Option<(u32, u32)> {
                if a1 <= a0 {
                    return None;
                }
                let key16: Vec<u16> = k2[a0 as usize..a1 as usize].iter().map(|&c| c as u16).collect();
                l.find(&key16)
            };
            let cheapest_m = |a: u32, b: u32| -> Vec<Ent> {
                let mut v: Vec<Ent> = Vec::new();
                for j in a..b {
                    let en = l.ent(j);
                    if en.cost < ENG_COST {
                        v.push(en);
                    }
                }
                if v.len() > 6 {
                    v.select_nth_unstable_by(5, |x, y| x.cost.cmp(&y.cost));
                    v.truncate(6);
                }
                v.sort_by(|x, y| x.cost.cmp(&y.cost)); // stable, [0] is cheapest
                v
            };
            let (lo, hi) = (p, n2 - q);
            for m in lo.max(1)..=hi.min(n2 - 1) {
                for s2 in (m - MAX_WORD_KANA).max(0)..=lo.min(m - 1) {
                    let Some((a1, b1)) = look(s2, m) else { continue };
                    let c1 = cheapest_m(a1, b1);
                    if c1.is_empty() {
                        continue;
                    }
                    // prefix bloom walk for the second word, like Head::to
                    let mut h: u64 = 1469598103934665603;
                    let mut ecur = m;
                    for e2 in hi.max(m + 1)..=n2.min(m + MAX_WORD_KANA) {
                        while ecur < e2 {
                            h ^= k2[ecur as usize] as u64;
                            h = h.wrapping_mul(1099511628211);
                            ecur += 1;
                        }
                        if !l.maybe_head(h ^ (h >> 29)) {
                            break;
                        }
                        if m == lo && e2 == hi && lo == hi {
                            continue;
                        }
                        let (s, e) = (s2, e2 - n2 + n1);
                        if e <= s || e > n1 || e < n1 - q {
                            continue;
                        }
                        if ctx.lmin[(s + off) as usize] + c1[0].cost as f64 + tc as f64 + ctx.rmin[(e + off) as usize] - 2000.0
                            > ctx.best + TYPO_KEEP
                        {
                            continue;
                        }
                        let Some((a2, b2)) = look(m, e2) else { continue };
                        let c2 = cheapest_m(a2, b2);
                        if c2.is_empty() {
                            continue;
                        }
                        // Pairs: keep, per surface, the pair whose best
                        // full path (through the joined edge) is cheapest.
                        struct Pair {
                            t: f64,
                            c: i32,
                            lid: i32,
                            rid: i32,
                            sf: Ustr,
                        }
                        let mut ps: Vec<Pair> = Vec::new();
                        let lr = ctx.lmin[(s + off) as usize] + ctx.rmin[(e + off) as usize] - 2000.0 + tc as f64;
                        'pairs: for x in &c1 {
                            if lr + x.cost as f64 + c2[0].cost as f64 > ctx.best + TYPO_KEEP {
                                break 'pairs; // c1 is cost-sorted
                            }
                            for y in &c2 {
                                if lr + x.cost as f64 + y.cost as f64 > ctx.best + TYPO_KEEP {
                                    break;
                                }
                                let c = x.cost + y.cost + l.conn(x.rid as i32, y.lid as i32) + tc;
                                let t = ctx.through(l, base, s + off, e + off, x.lid as i32, y.rid as i32, c);
                                if t > ctx.best + TYPO_KEEP {
                                    continue;
                                }
                                let mut sf = l.surf(x);
                                sf.extend(l.surf(y));
                                match ps.iter_mut().find(|p2| p2.sf == sf) {
                                    Some(p2) => {
                                        if t < p2.t {
                                            *p2 = Pair { t, c, lid: x.lid as i32, rid: y.rid as i32, sf };
                                        }
                                    }
                                    None => ps.push(Pair { t, c, lid: x.lid as i32, rid: y.rid as i32, sf }),
                                }
                            }
                        }
                        ps.sort_by(|x, y| x.t.partial_cmp(&y.t).unwrap_or(std::cmp::Ordering::Equal));
                        for pr in ps.into_iter().take(3) {
                            add(&mut cand, s, e, pr.sf, pr.lid, pr.rid, pr.c);
                        }
                    }
                }
            }
        }
    }
    // misread marker entries (reading prefixed with U+0001)
    for s in 0..n1 {
        for e in s + 2..=(n1.min(s + MAX_WORD_KANA)) {
            if ctx.lmin[(s + off) as usize] + TYPO_COST as f64 + ctx.rmin[(e + off) as usize] - 2000.0 > ctx.best + TYPO_KEEP {
                continue;
            }
            let mut mk: Vec<u16> = vec![1];
            mk.extend(r[s as usize..e as usize].iter().map(|&c| c as u16));
            let Some((a, b)) = l.find(&mk) else { continue };
            for j in a..b {
                let en = l.ent(j);
                let sf = l.surf(&en);
                add(&mut cand, s, e, sf, en.lid as i32, en.rid as i32, en.cost + TYPO_COST);
            }
        }
    }
    let mut cs: Vec<Cand> = cand.into_values().collect();
    cs.sort_by(|x, y| {
        x.total.partial_cmp(&y.total).unwrap_or(std::cmp::Ordering::Equal)
            .then(x.s.cmp(&y.s)).then(x.e.cmp(&y.e)).then(x.surf.cmp(&y.surf))
    });
    cs.truncate(TYPO_MAX);
    for c in cs {
        out.push(Edge::new(c.s + off, c.e + off, c.surf, c.lid, c.rid, c.cost, K_TYPO));
    }
}

// ------------------------------------------------------------------ lattice construction (edges())
fn edges(l: &Lex, r: &[char], user: Option<&UserList>, alts: Option<&[Ustr]>) -> Vec<Edge> {
    let mut out: Vec<Edge> = Vec::new();
    let n = r.len() as i32;
    for s in 0..n {
        if !is_kana(r[s as usize]) {
            out.push(Edge::new(s, s + 1, vec![r[s as usize]], l.noun, l.noun, l.pass, K_PASS));
            continue;
        }
        let mut key: Vec<u16> = Vec::new();
        for e in s + 1..=n.min(s + l.max_word) {
            if !is_kana(r[(e - 1) as usize]) {
                break;
            }
            key.push(r[(e - 1) as usize] as u16);
            if let Some((a, b)) = l.find(&key) {
                for j in a..b {
                    let en = l.ent(j);
                    if en.cost >= ENG_COST || pict_word(l, &en) {
                        continue;
                    }
                    let sf = l.surf(&en);
                    if sf.len() > (e - s) as usize + 1 && sf.iter().any(|&ch| ch == '、' || ch == '。' || ch == '…') {
                        continue;
                    }
                    if sf.iter().all(|&ch| ch.is_ascii_digit()) && number_continues(r, s as usize, e as usize) {
                        continue;
                    }
                    if (s != 0 || e != n) && symbol_word(&sf) {
                        continue;
                    }
                    let c = if has_latin_word(&sf) { en.cost - LATIN_WORD_BONUS } else { en.cost };
                    out.push(Edge::new(s, e, sf, en.lid as i32, en.rid as i32, c, K_DICT));
                }
            }
            if e - s == 1 {
                out.push(Edge::new(s, e, vec![r[s as usize]], l.noun, l.noun, l.oov, K_KANA));
                if r[s as usize] != 'ー' {
                    out.push(Edge::new(s, e, vec![kata(r[s as usize])], l.noun, l.noun, l.kata + l.kata_per, K_KATA));
                }
            } else if e - s <= 12 {
                out.push(Edge::new(s, e, to_kata(&r[s as usize..e as usize]), l.noun, l.noun, l.kata + l.kata_per * (e - s), K_KATA));
            }
        }
    }
    // latin typed as-is (iphone/github on an alphanumeric keyboard)
    {
        let al = |c: char| c.is_ascii_alphanumeric();
        let mut s = 0i32;
        while s < n {
            if !al(r[s as usize]) {
                s += 1;
                continue;
            }
            let mut e = s;
            let mut letter = false;
            while e < n
                && (al(r[e as usize])
                    || ((r[e as usize] == '.' || r[e as usize] == '-' || r[e as usize] == '&' || r[e as usize] == '+')
                        && e + 1 < n
                        && al(r[(e + 1) as usize])))
            {
                letter = letter || r[e as usize].is_ascii_alphabetic();
                e += 1;
            }
            if letter && e - s >= 2 && e - s <= 30 {
                let key: Vec<u16> = r[s as usize..e as usize]
                    .iter()
                    .filter(|&&c| c != ' ')
                    .map(|&c| (if c.is_ascii_uppercase() { c as u32 + 32 } else { c as u32 }) as u16)
                    .collect();
                if let Some((a, b)) = l.find(&key) {
                    for j in a..b {
                        let en = l.ent(j);
                        if en.cost >= ENG_COST || pict_word(l, &en) {
                            continue;
                        }
                        out.push(Edge::new(s, e, l.surf(&en), en.lid as i32, en.rid as i32, en.cost, K_DICT));
                    }
                }
            }
            s = e;
        }
    }
    // user dictionary
    if let Some(user) = user {
        if !user.by_r.is_empty() {
            let mut hits: Vec<(u32, i32)> = Vec::new();
            for s in 0..n {
                for len in 1..=(user.maxlen as i32).min(n - s) {
                    let sub = &r[s as usize..(s + len) as usize];
                    if let Some(ks) = user.by_r.get(sub) {
                        for &k in ks {
                            hits.push((k, s));
                        }
                    }
                }
            }
            hits.sort();
            for (k, s) in hits {
                let w = &user.v[k as usize];
                if w.whole {
                    if s == 0 && w.r.len() as i32 == n {
                        out.push(Edge::new(0, n, w.s.clone(), w.lid, w.rid, w.cost, K_DICT));
                    }
                    continue;
                }
                out.push(Edge::new(s, s + w.r.len() as i32, w.s.clone(), w.lid, w.rid, w.cost, K_DICT));
            }
        }
    }
    // digit runs typed on a number pad
    {
        let mut s = 0i32;
        while s < n {
            if !r[s as usize].is_ascii_digit() {
                s += 1;
                continue;
            }
            let mut e = s;
            while e < n && r[e as usize].is_ascii_digit() {
                e += 1;
            }
            if e - s <= 16 && (r[s as usize] != '0' || e - s == 1) {
                let mut v: i64 = 0;
                for i in s..e {
                    v = v * 10 + (r[i as usize] as i64 - '0' as i64);
                }
                let c = 3000;
                out.push(Edge::new(s, e, r[s as usize..e as usize].to_vec(), l.num_arabic, l.num_arabic, c, K_NUM));
                if v >= 1000 && !(e - s == 4 && starts_with(r, e as usize, &u("ねん"))) {
                    out.push(Edge::new(s, e, with_commas(&r[s as usize..e as usize]), l.num_arabic, l.num_arabic, c + 600, K_NUM));
                }
                if v >= 10000 {
                    let m = unit_mixed(v, false);
                    let mc = unit_mixed(v, true);
                    out.push(Edge::new(s, e, m.clone(), l.num_arabic, l.num_arabic, c + 700, K_NUM));
                    if mc != m {
                        out.push(Edge::new(s, e, mc, l.num_arabic, l.num_arabic, c + 800, K_NUM));
                    }
                }
            }
            s = e;
        }
    }
    let mut num: Vec<Edge> = Vec::new();
    number_edges(l, r, &mut num);
    special_edges(l, r, &mut num);
    abbr_edges(l, r, &mut num);
    for ed in num {
        if (ed.s..ed.e).all(|i| is_kana(r[i as usize])) {
            out.push(ed);
        }
    }
    if l.typo {
        let ctx = build_typo_ctx(l, &out, n);
        let mut ty: Vec<Edge> = Vec::new();
        let pa = alts.map(|a| prep_alts(r, a));
        let mut win = |w: i32, we: i32, out_ref: &Vec<Edge>, ty: &mut Vec<Edge>| {
            if let Some(pa) = &pa {
                let sa = window_alts(pa, n, w, we);
                typo_add_edges(l, &r[w as usize..we as usize], w, out_ref, &ctx, ty, None, Some(&sa));
            } else {
                typo_add_edges(l, &r[w as usize..we as usize], w, out_ref, &ctx, ty, None, None);
            }
        };
        let mut s = 0i32;
        while s < n {
            if !is_kana(r[s as usize]) {
                s += 1;
                continue;
            }
            let mut e = s;
            while e < n && is_kana(r[e as usize]) {
                e += 1;
            }
            let (w_sz, step) = (12i32, 6i32);
            if e - s <= w_sz {
                win(s, e, &out, &mut ty);
            } else {
                let mut w = s;
                loop {
                    let we = e.min(w + w_sz);
                    win(w, we, &out, &mut ty);
                    if we == e {
                        break;
                    }
                    w += step;
                }
            }
            s = e;
        }
        ty.sort_by(|x, y| x.s.cmp(&y.s).then(x.e.cmp(&y.e)).then(x.surf.cmp(&y.surf)).then(x.cost.cmp(&y.cost)));
        ty.dedup_by(|a, b| a.s == b.s && a.e == b.e && a.surf == b.surf);
        out.extend(ty);
    }
    out
}

fn kata_continuation_edges(l: &Lex, ctx: &[char], r: &[char], out: &mut Vec<Edge>) {
    let is_katach = |c: char| (0x30A1..=0x30FA).contains(&(c as u32)) || c == 'ー';
    let mut t = 0usize;
    while t < ctx.len() && t < 12 && is_katach(ctx[ctx.len() - 1 - t]) {
        t += 1;
    }
    if t < 2 {
        return;
    }
    let n = r.len();
    let mut best = 0usize;
    for j in 1..=n.min(12) {
        if !is_kana(r[j - 1]) {
            break;
        }
        let tail_r = to_kata(&r[..j]);
        for k in 2..=t {
            let mut word: Ustr = ctx[ctx.len() - k..].to_vec();
            word.extend(tail_r.iter());
            let key: Vec<u16> = word
                .iter()
                .map(|&c| if (0x30A1..=0x30F6).contains(&(c as u32)) { (c as u32 - 0x60) as u16 } else { c as u16 })
                .collect();
            let Some((a, b)) = l.find(&key) else { continue };
            let mut hit = false;
            for q in a..b {
                let en = l.ent(q);
                hit = en.cost < ENG_COST && l.surf(&en) == word;
                if hit {
                    break;
                }
            }
            if hit {
                best = j;
                break;
            }
        }
    }
    if best > 0 {
        out.push(Edge::new(0, best as i32, to_kata(&r[..best]), l.noun, l.noun, 0, K_KATA));
    }
}

fn kanji_continuation_edges(l: &mut Lex, ctx: &[char], r: &[char], out: &mut Vec<Edge>) {
    let is_kanji = |c: char| (0x4E00..=0x9FFF).contains(&(c as u32)) || c == '々';
    if ctx.is_empty() || !is_kanji(*ctx.last().unwrap()) || r.is_empty() {
        return;
    }
    if !l.head_built {
        l.head_built = true;
        let mut idx: HashMap<char, Vec<(u32, u32)>> = HashMap::new();
        for i in 0..l.nread {
            for j in l.e_first[i as usize]..l.e_first[i as usize + 1] {
                let en = l.ent(j);
                if en.slen < 1 || en.cost >= 7000 {
                    continue;
                }
                let q = &l.s_blob[en.soff as usize..en.soff as usize + en.slen as usize];
                if q.iter().any(|&c| is_kanji(char::from_u32(c as u32).unwrap_or('\u{FFFD}'))) {
                    idx.entry(char::from_u32(q[0] as u32).unwrap_or('\u{FFFD}')).or_default().push((j, i as u32));
                }
            }
        }
        l.head_idx = idx;
    }
    let n = r.len() as i32;
    let mut best_j = 0i32;
    let mut best: Option<Edge> = None;
    for k in 1..=ctx.len().min(4) {
        let p: Ustr = ctx[ctx.len() - k..].to_vec();
        if !p.iter().any(|&c| is_kanji(c)) {
            continue;
        }
        let Some(list) = l.head_idx.get(&p[0]).cloned() else { continue };
        let mut complete = false;
        for (j, _ri) in &list {
            let en = l.ent(*j);
            if en.slen as usize != k || en.cost >= 6000 {
                continue;
            }
            let q = &l.s_blob[en.soff as usize..en.soff as usize + en.slen as usize];
            if q.iter().zip(p.iter()).all(|(&a, &b)| a == b as u16) {
                complete = true;
                break;
            }
        }
        if complete {
            continue;
        }
        for (j, ri) in &list {
            let en = l.ent(*j);
            if en.slen as usize <= k {
                continue;
            }
            let q = &l.s_blob[en.soff as usize..en.soff as usize + en.slen as usize];
            if !q[..k].iter().zip(p.iter()).all(|(&a, &b)| a == b as u16) {
                continue;
            }
            let rd = l.reading_vec(*ri as i32);
            let rn = rd.len() as i32;
            for jj in (1..=n.min(rn - k as i32)).rev() {
                let tail = (0..jj).all(|x| rd[(rn - jj + x) as usize] == r[x as usize] as u16);
                if !tail {
                    continue;
                }
                let cost = en.cost.min(3000);
                if jj > best_j || (jj == best_j && best.as_ref().map_or(true, |b| cost < b.cost)) {
                    best_j = jj;
                    best = Some(Edge::new(
                        0, jj,
                        from16(&q[k..]),
                        en.lid as i32, en.rid as i32, cost, K_DICT,
                    ));
                }
                break;
            }
        }
    }
    if best_j > 0 {
        if let Some(b) = best {
            out.push(b);
        }
    }
}

// ------------------------------------------------------------------ latin edges (kkc_set_keys)
fn small_romaji(k: &[u16]) -> bool {
    k.iter().any(|&c| c == b'l' as u16 || c == b'x' as u16)
}
fn unromaji(k: &[u16]) -> bool {
    let cons = |c: u16| (b'a' as u16..=b'z' as u16).contains(&c) && ![b'a' as u16, b'i' as u16, b'u' as u16, b'e' as u16, b'o' as u16].contains(&c);
    for i in 0..k.len() {
        if (b'A' as u16..=b'Z' as u16).contains(&k[i]) {
            return true;
        }
        if i + 1 < k.len() && cons(k[i]) && cons(k[i + 1]) {
            let (a, b) = (k[i], k[i + 1]);
            if a == b || a == b'n' as u16 || b == b'y' as u16
                || (b == b'h' as u16 && [b's' as u16, b'c' as u16, b'd' as u16, b't' as u16, b'w' as u16].contains(&a))
                || (a == b't' as u16 && b == b's' as u16)
                || (b == b'w' as u16 && [b'k' as u16, b'g' as u16].contains(&a))
                || a == b'x' as u16 || a == b'l' as u16
            {
                continue;
            }
            return true;
        }
    }
    false
}
#[allow(clippy::too_many_arguments)]
fn latin_edges(
    l: &Lex, keys: &[u16], span_end: &[i32], n: i32, r: &[char],
    user_latin: &HashMap<String, LatinWord>, out: &mut Vec<Edge>,
    left_latin: bool, right_latin: bool, mid2: bool,
) {
    let nkeys = keys.len() as i32;
    // whole-chunk suppression
    let mut whole_end = vec![-1i32; n as usize];
    {
        let mut a = 0i32;
        while a < n {
            if r[a as usize] == ' ' {
                a += 1;
                continue;
            }
            let mut b = a;
            let mut left = false;
            while b < n && r[b as usize] != ' ' {
                if (r[b as usize] as u32) < 0x80 && r[b as usize].is_alphabetic() {
                    left = true;
                }
                b += 1;
            }
            let ka = if a == 0 { 0 } else { span_end[(a - 1) as usize] };
            let kb = if b == n { nkeys } else { span_end[(b - 1) as usize] };
            if left && kb - ka >= 2 {
                let mut lower = String::new();
                let mut ok = true;
                for i in ka..kb {
                    let c = keys[i as usize];
                    if (b'a' as u16..=b'z' as u16).contains(&c) {
                        lower.push(c as u8 as char);
                    } else if (b'A' as u16..=b'Z' as u16).contains(&c) {
                        lower.push((c + 32) as u8 as char);
                    } else {
                        ok = false;
                        break;
                    }
                }
                if ok && (l.latin_words.contains_key(&lower) || user_latin.contains_key(&lower)) {
                    for i in a..b {
                        whole_end[i as usize] = b;
                    }
                }
            }
            a = b;
        }
    }
    let inside_whole = |s: i32, e: i32| -> bool {
        if whole_end[s as usize] < 0 {
            return false;
        }
        let b = whole_end[s as usize];
        let start = s == 0 || r[(s - 1) as usize] == ' ';
        !(start && e == b)
    };
    let mut add = |ks: i32, ke: i32, s: i32, e: i32, out: &mut Vec<Edge>| {
        if inside_whole(s, e) {
            return;
        }
        if ke - ks < 2 {
            if ke - ks != 1 {
                return;
            }
            let c = keys[ks as usize];
            let sp = (if s > 0 { r[(s - 1) as usize] == ' ' } else { left_latin })
                || (if e < n { r[e as usize] == ' ' } else { right_latin });
            if (sp && [b'i' as u16, b'a' as u16, b'I' as u16, b'A' as u16].contains(&c))
                || (b'A' as u16..=b'Z' as u16).contains(&c)
            {
                out.push(Edge::new(s, e, vec![char::from_u32(c as u32).unwrap()], l.noun, l.noun, LATIN_RAW_COST, K_LATIN));
            }
            return;
        }
        let mut lower = String::new();
        let mut spaced = false;
        for i in ks..ke {
            let c = keys[i as usize];
            if (b'a' as u16..=b'z' as u16).contains(&c) {
                lower.push(c as u8 as char);
            } else if (b'A' as u16..=b'Z' as u16).contains(&c) {
                lower.push((c + 32) as u8 as char);
            } else if c == b' ' as u16 && i > ks && i + 1 < ke {
                spaced = true;
            } else {
                return;
            }
        }
        if lower.len() < 2 {
            return;
        }
        if let Some(uw) = user_latin.get(&lower) {
            out.push(Edge::new(s, e, uw.surf.clone(), uw.lid, uw.rid, uw.cost, K_LATIN));
            return;
        }
        if spaced {
            return;
        }
        let latin_side = |i: i32| -> bool {
            if i < 0 {
                return left_latin;
            }
            if i >= n {
                return right_latin;
            }
            r[i as usize] == ' ' || ((r[i as usize] as u32) < 0x80 && r[i as usize].is_alphabetic())
        };
        let english = latin_side(s - 1) || latin_side(e);
        let kana_span = r[s as usize..e as usize].iter().all(|&c| is_kana(c)) && !english;
        let camel = |w: &[char]| -> bool {
            let (mut up, mut low) = (false, false);
            for (i, &c) in w.iter().enumerate() {
                if c.is_ascii_uppercase() {
                    if i > 0 {
                        up = true;
                    }
                } else if c.is_ascii_lowercase() {
                    low = true;
                }
            }
            up && low
        };
        let upper = keys[ks as usize..ke as usize].iter().any(|&c| (b'A' as u16..=b'Z' as u16).contains(&c));
        let lx = small_romaji(&keys[ks as usize..ke as usize]);
        let mut it = l.latin_words.get(&lower);
        if let Some(w) = it {
            if kana_span && !camel(&w.surf) && !lx {
                if !upper {
                    return;
                }
                it = None;
            }
        }
        if let Some(w) = it {
            let mut surf = w.surf.clone();
            let common = {
                let mut k16: Vec<u16> = vec![2];
                k16.extend(lower.bytes().map(|b| b as u16));
                l.find(&k16).is_some()
            };
            if upper || (english && (!camel(&surf) || common)) {
                surf = keys[ks as usize..ke as usize].iter().map(|&c| char::from_u32(c as u32).unwrap()).collect();
            }
            out.push(Edge::new(s, e, surf.clone(), w.lid, w.rid, w.cost + LATIN_BIAS, K_LATIN));
            if mid2 && !r[s as usize..e as usize].iter().all(|&c| is_kana(c)) {
                let typed: Ustr = keys[ks as usize..ke as usize].iter().map(|&c| char::from_u32(c as u32).unwrap()).collect();
                if typed != surf {
                    out.push(Edge::new(s, e, typed, w.lid, w.rid, w.cost + LATIN_BIAS + LATIN_CASE_ALT, K_LATIN));
                }
            }
            return;
        }
        if r[s as usize..e as usize].iter().all(|&c| is_kana(c)) && !unromaji(&keys[ks as usize..ke as usize]) {
            return;
        }
        let typed: Ustr = keys[ks as usize..ke as usize].iter().map(|&c| char::from_u32(c as u32).unwrap()).collect();
        out.push(Edge::new(s, e, typed, l.noun, l.noun, LATIN_RAW_COST + LATIN_BIAS, K_LATIN));
    };
    for s in 0..n {
        let ks = if s == 0 { 0 } else { span_end[(s - 1) as usize] };
        for e in s + 1..=n {
            let ke = span_end[(e - 1) as usize];
            add(ks, ke, s, e, out);
            if e == n && nkeys > ke {
                add(ks, nkeys, s, e, out);
            }
        }
    }
    // A latin word ending in the middle of a kana chunk
    // (minecraftapuri -> minecraft ends on the 't' of た, 'a' starts the
    // next word): join the latin word with a dictionary word that starts
    // with the leftover key(s) re-read as kana (あぷり -> アプリ) into one
    // edge. With mid2 (kkc_set_latin_mid2) the leftover keys may be more
    // than one vowel and may extend into the following chunks, re-read
    // with to_kana (IDha -> ID + は, jsonni -> json + に).
    {
        // Chunks: [cs[i], ce[i]) are the reading chars determined by one
        // key span (consecutive chars sharing the same span_end step).
        let mut cs: Vec<i32> = Vec::new();
        let mut ce: Vec<i32> = Vec::new();
        for e in 1..=n {
            if e < n && span_end[e as usize] == span_end[(e - 1) as usize] {
                continue;
            }
            cs.push(if ce.is_empty() { 0 } else { *ce.last().unwrap() });
            ce.push(e);
        }
        const VOWELS: [u16; 5] = [b'a' as u16, b'i' as u16, b'u' as u16, b'e' as u16, b'o' as u16];
        const KANA_V: [u16; 5] = ['あ' as u16, 'い' as u16, 'う' as u16, 'え' as u16, 'お' as u16];
        let rm = romaji();
        for ci in 0..cs.len() {
            let (c0, e) = (cs[ci], ce[ci]);
            if e >= n && !mid2 {
                continue;
            }
            let kend = span_end[(e - 1) as usize];
            let kb = if c0 == 0 { 0 } else { span_end[(c0 - 1) as usize] };
            if kend - kb < 2 || (!mid2 && !r[c0 as usize..e as usize].iter().all(|&c| is_kana(c))) {
                continue;
            }
            for p in (if mid2 { kb + 1 } else { kend - 1 })..kend {
                let mut rest: Vec<u16> = Vec::new();
                let mut e1 = e;
                let mut cj = ci as i32;
                if !mid2 {
                    let mut vk = keys[(kend - 1) as usize];
                    if (b'A' as u16..=b'Z' as u16).contains(&vk) {
                        vk += 32;
                    }
                    let Some(vp) = VOWELS.iter().position(|&v| v == vk) else { continue };
                    rest.push(KANA_V[vp]);
                } else {
                    let mut ok = false;
                    for ext in 0..3 {
                        if ext > 0 {
                            cj += 1;
                            if cj as usize >= cs.len() {
                                break;
                            }
                            if !r[cs[cj as usize] as usize..ce[cj as usize] as usize].iter().all(|&c| is_kana(c)) {
                                break;
                            }
                            e1 = ce[cj as usize];
                        }
                        // A lone trailing 'n' is undecided (json|n|i
                        // needs the next chunk's i).
                        let kto = span_end[(e1 - 1) as usize];
                        let mut rk = String::new();
                        let mut bad = false;
                        for i in p..kto {
                            let mut c = keys[i as usize];
                            if (b'A' as u16..=b'Z' as u16).contains(&c) {
                                c += 32;
                            }
                            if !((b'a' as u16..=b'z' as u16).contains(&c) || c == b'-' as u16) {
                                bad = true;
                                break;
                            }
                            rk.push(c as u8 as char);
                        }
                        if bad {
                            break;
                        }
                        let rkb: Vec<char> = rk.chars().collect();
                        let lone_n = !rkb.is_empty()
                            && rkb[rkb.len() - 1] == 'n'
                            && (rkb.len() < 2 || rkb[rkb.len() - 2] != 'n');
                        rest.clear();
                        let Some(kk) = (if lone_n { None } else { to_kana(&rm, &rk) }) else { continue };
                        rest = kk.iter().map(|&c| c as u16).collect();
                        ok = !rest.is_empty()
                            && rest.iter().all(|&ch| (0x3041..=0x30FC).contains(&ch));
                        if ok {
                            break;
                        }
                    }
                    if !ok {
                        continue;
                    }
                }
                // Start of the latin word: without mid2, any char before
                // the chunk; with mid2, chunk starts only (plus this
                // chunk's own start).
                let ns = if mid2 { ci as i32 + 1 } else { c0 };
                for si in 0..ns {
                    let s = if mid2 { cs[si as usize] } else { si };
                    let ks = if s == 0 { 0 } else { span_end[(s - 1) as usize] };
                    if ks >= p {
                        continue;
                    }
                    // Only dictionary/user latin words, or words typed
                    // with Shift capitals (an unmistakable latin intent).
                    let mut lower = String::new();
                    let mut up = false;
                    let mut letters = true;
                    for i in ks..p {
                        let c = keys[i as usize];
                        up |= (b'A' as u16..=b'Z' as u16).contains(&c);
                        letters &= (b'a' as u16..=b'z' as u16).contains(&c) || (b'A' as u16..=b'Z' as u16).contains(&c);
                        lower.push(if (b'A' as u16..=b'Z' as u16).contains(&c) {
                            (c + 32) as u8 as char
                        } else if c < 0x80 {
                            c as u8 as char
                        } else {
                            '?'
                        });
                    }
                    if mid2 && !letters {
                        continue;
                    }
                    if p - ks < 2 && !(mid2 && up) {
                        continue;
                    }
                    if mid2 {
                        let vowel_only = p == kend - 1
                            && rest.len() == 1
                            && KANA_V.contains(&rest[0]);
                        if !up && !vowel_only && r[s as usize..e as usize].iter().all(|&c| is_kana(c)) {
                            continue; // まっちゃん -> まcc半 にしない
                        }
                        let nk = keys[p as usize];
                        if (b'A' as u16..=b'Z' as u16).contains(&nk) {
                            continue; // inside a capitalised word
                        }
                    }
                    if mid2 && s == c0 && !up {
                        continue;
                    }
                    if !l.latin_words.contains_key(&lower) && !user_latin.contains_key(&lower) && !up {
                        continue;
                    }
                    let before = out.len();
                    add(ks, p, s, e, out);
                    if out.len() == before {
                        continue;
                    }
                    let words: Vec<Edge> = out.drain(before..).collect();
                    let mut key: Vec<u16> = rest.clone();
                    // Without mid2 the leftover vowel alone is not a
                    // word (e2 starts at e1+1); with mid2 it may be
                    // (ID + は).
                    for e2 in (if mid2 { e1 } else { e1 + 1 })..=n.min(e1 + l.max_word) {
                        if e2 > e1 {
                            if !is_kana(r[(e2 - 1) as usize]) {
                                break;
                            }
                            key.push(r[(e2 - 1) as usize] as u16);
                        }
                        if key.len() as i32 > l.max_word {
                            break;
                        }
                        let Some((a, b)) = l.find(&key) else { continue };
                        for j in a..b {
                            let en = l.ent(j);
                            if en.cost >= ENG_COST || pict_word(l, &en) {
                                continue;
                            }
                            if mid2 {
                                let sf = l.surf(&en);
                                // Rare kana (変体仮名・ゐゑゟ・ヰヱ・ヷ..ヺ)
                                // are not joined.
                                if sf.iter().any(|&c| {
                                    (c as u32) >= 0x1B000
                                        || matches!(c, 'ゐ' | 'ゑ' | 'ゟ' | 'ヰ' | 'ヱ')
                                        || ('ヷ'..='ヺ').contains(&c)
                                }) {
                                    continue;
                                }
                                // A word made of the leftover kana alone
                                // must be a single particle kana.
                                if e2 == e1
                                    && (sf.len() != 1 || !"はがをにへとでやのもかねよ".contains(sf[0]))
                                {
                                    continue;
                                }
                            }
                            for w in &words {
                                let mut sf = w.surf.clone();
                                sf.extend(l.surf(&en));
                                out.push(Edge::new(s, e2, sf, w.lid, en.rid as i32, w.cost + en.cost, K_LATIN));
                            }
                        }
                    }
                }
            }
        }
    }
}

// ------------------------------------------------------------------ best path / scorer
fn best_path(l: &Lex, e: &[Edge], n: i32) -> Ustr {
    let m = e.len();
    if m == 0 {
        return Vec::new();
    }
    let mut order: Vec<usize> = (0..m).collect();
    order.sort_by_key(|&i| e[i].s);
    let mut ptr = vec![0usize; (n + 2) as usize];
    for ed in e {
        ptr[(ed.e + 1) as usize] += 1;
    }
    for p in 0..=(n as usize) {
        ptr[p + 1] += ptr[p];
    }
    let mut by_end = vec![0usize; m];
    let mut fill = ptr.clone();
    for i in 0..m {
        by_end[fill[e[i].e as usize]] = i;
        fill[e[i].e as usize] += 1;
    }
    let mut best = vec![1e18f64; m];
    let mut prev = vec![-1i32; m];
    for &i in &order {
        let s = e[i].s;
        if s == 0 {
            best[i] = l.conn(0, e[i].lid) as f64 + e[i].cost as f64;
            continue;
        }
        for q in ptr[s as usize]..ptr[s as usize + 1] {
            let j = by_end[q];
            if best[j] >= 1e17 {
                continue;
            }
            let v = best[j] + l.conn(e[j].rid, e[i].lid) as f64 + e[i].cost as f64;
            if v < best[i] {
                best[i] = v;
                prev[i] = j as i32;
            }
        }
    }
    let (mut bi, mut bv) = (-1i32, 1e18f64);
    for q in ptr[n as usize]..ptr[n as usize + 1] {
        let j = by_end[q];
        let v = best[j] + l.conn(e[j].rid, 0) as f64;
        if v < bv {
            bv = v;
            bi = j as i32;
        }
    }
    let mut path = Vec::new();
    while bi >= 0 {
        path.push(bi as usize);
        bi = prev[bi as usize];
    }
    let mut out = Vec::new();
    for &i in path.iter().rev() {
        out.extend(e[i].surf.iter());
    }
    out
}

// Scalar linear: Y[t][o] = sum_i X[t][i]*WT[i][o] + B[o]. This is the
// scalar equivalent of engine_simd.h's AVX2 kernels (same math).
fn lin(x: &[f32], t: usize, in_: usize, wt: &[f32], ldw: usize, b: Option<&[f32]>, y: &mut [f32], ldy: usize) {
    for tt in 0..t {
        for o in 0..ldy {
            let mut s = b.map_or(0.0, |b| b[o]);
            for i in 0..in_ {
                s += x[tt * in_ + i] * wt[i * ldw + o];
            }
            y[tt * ldy + o] = s;
        }
    }
}
fn gelu(x: f32) -> f32 {
    0.5 * x * (1.0 + erf(x * 0.70710678118654752))
}
// erf matching C++ std::erf (glibc erff, correctly rounded to ~1 ulp):
// Rust std has no erf, so evaluate the incomplete-gamma form in f64 —
// series for x² < 1.5, continued fraction (Lentz) above — which is
// accurate to ~1e-14, far below f32 resolution. (The previous
// Numerical-Recipes / Abramowitz-Stegun polynomials have up to
// ~1.2e-7 / 1.5e-7 absolute error, the same order as the AVX2 path in
// engine_simd.h; the scalar C++ path uses std::erf.)
fn erf(x: f32) -> f32 {
    erf_f64(x as f64) as f32
}
fn erf_f64(x: f64) -> f64 {
    if x == 0.0 {
        return 0.0;
    }
    let t = x * x;
    // ln Γ(1/2) = ln √π
    const LGAMMA_HALF: f64 = 0.5723649429247001;
    let p = if t < 1.5 {
        // series for P(1/2, t)
        let mut ap = 0.5f64;
        let mut sum = 2.0f64; // 1/a
        let mut del = sum;
        for _ in 0..1000 {
            ap += 1.0;
            del *= t / ap;
            sum += del;
            if del.abs() < sum.abs() * 1e-15 {
                break;
            }
        }
        sum * (-t + 0.5 * t.ln() - LGAMMA_HALF).exp()
    } else {
        // continued fraction for Q(1/2, t); erf = 1 - Q
        let mut b = t + 0.5; // t + 1 - a
        let mut c = 1e300f64;
        let mut d = 1.0 / b;
        let mut h = d;
        for i in 1..1000 {
            let an = -(i as f64) * (i as f64 - 0.5);
            b += 2.0;
            d = an * d + b;
            if d.abs() < 1e-300 {
                d = 1e-300;
            }
            c = b + an / c;
            if c.abs() < 1e-300 {
                c = 1e-300;
            }
            d = 1.0 / d;
            let del = d * c;
            h *= del;
            if (del - 1.0).abs() < 1e-15 {
                break;
            }
        }
        1.0 - (-t + 0.5 * t.ln() - LGAMMA_HALF).exp() * h
    };
    if x < 0.0 {
        -p
    } else {
        p
    }
}
fn layernorm(x: &[f32], t: usize, d: usize, w: &[f32], b: &[f32], y: &mut [f32]) {
    for tt in 0..t {
        let v = &x[tt * d..(tt + 1) * d];
        let mut m = 0f32;
        for &z in v {
            m += z;
        }
        m /= d as f32;
        let mut s = 0f32;
        for &z in v {
            let z = z - m;
            s += z * z;
        }
        let r = 1.0 / (s / d as f32 + 1e-5).sqrt();
        for i in 0..d {
            y[tt * d + i] = (v[i] - m) * r * w[i] + b[i];
        }
    }
}

// ------------------------------------------------------------------ Romance (per-user small model)
#[derive(Default)]
struct Romance {
    on: bool,
    du: i32,
    dk: i32,
    h: i32, // 32
    v: Vec<f32>,
    w: Vec<f32>,
    w0: Vec<f32>,
    a: Vec<f32>,
    a0: Vec<f32>,
    ab: Vec<f32>, // MLP bias 'a' in C++ (named ab to avoid clash)
    c: Vec<f32>,
    cap: f32,
    lr_v: f32,
    lr_w: f32,
    lr_net: f32,
    l2: f32,
    updates: i32,
    s: Vec<f32>,
    e: Vec<Edge>,
    n: i32,
    q: Vec<f32>,
    k: Vec<f32>,
}
impl Romance {
    fn size(&self) -> usize {
        self.v.len() + self.w.len() + self.a.len() + self.ab.len() + self.c.len()
    }
    fn init(&mut self, du_: i32, dk_: i32, wm: Option<&[f32]>) {
        self.du = if wm.is_some() { du_ } else { 0 };
        self.dk = dk_;
        self.h = 32;
        self.cap = 2.0;
        self.lr_v = 0.05;
        self.lr_w = 0.002;
        self.lr_net = 0.01;
        self.l2 = 0.001;
        self.v = vec![0.0; self.du as usize];
        self.w0 = wm.map(|w| w[..(self.du as usize) * (dk_ as usize)].to_vec()).unwrap_or_default();
        self.w = self.w0.clone();
        self.a = vec![0.0; (self.h as usize) * (dk_ as usize)];
        let mut x: u64 = 0x9E3779B97F4A7C15;
        let sd = 1.0 / (dk_ as f32).sqrt();
        for w in self.a.iter_mut() {
            let mut next = || {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                x
            };
            let u1 = ((next() >> 11) as f64 + 0.5) / 9007199254740992.0;
            let u2 = (next() >> 11) as f64 / 9007199254740992.0;
            *w = ((-2.0 * u1.ln()).sqrt() * (6.283185307179586 * u2).cos()) as f32 * sd;
        }
        self.a0 = self.a.clone();
        self.ab = vec![0.0; self.h as usize];
        self.c = vec![0.0; self.h as usize];
        self.updates = 0;
    }
    fn prepare(&mut self) {
        self.s = vec![0.0; self.dk as usize];
        for i in 0..self.du as usize {
            if self.v[i] == 0.0 {
                continue;
            }
            for j in 0..self.dk as usize {
                self.s[j] += self.v[i] * self.w[i * self.dk as usize + j];
            }
        }
    }
    fn extra(&self, q: &[f32], kk: &[f32], sc: f32, hid: &mut [f32]) -> f32 {
        let (mut t, mut o) = (0f32, 0f32);
        for j in 0..self.dk as usize {
            t += self.s[j] * kk[j];
        }
        for h in 0..self.h as usize {
            let mut p = 0f32;
            for j in 0..self.dk as usize {
                p += self.a[h * self.dk as usize + j] * q[j] * kk[j];
            }
            p = p * sc + self.ab[h];
            hid[h] = p;
            if p > 0.0 {
                o += self.c[h] * p;
            }
        }
        t * sc + self.cap * (o / self.cap).tanh()
    }
}

// ------------------------------------------------------------------ engine state
#[derive(Clone, Default)]
struct VarForm {
    other: Ustr,
    lemma: i32,
    cls: i32,
    kanji: bool,
}

pub struct kkc_engine {
    lex: Lex,
    m: Model,
    ro: Romance,
    own_lex: Vec<u8>,
    own_model: Vec<u8>,
    model_tag: u32,
    threads: i32,
    times: [f64; 5],
    typo_bias: f32,
    typo_promote: f64,
    latin_promote: f64,
    typo_margin: f64,
    suggest_mode: bool,
    latin_mid2: bool,
    latin: bool,
    char_filter: i32,
    suggest: Vec<(i32, Ustr)>,
    typo_spans: Vec<i32>,
    kcache: HashMap<Ustr, i32>,
    kstore: Vec<f32>,
    last_u: Vec<f32>,
    last_segs: Vec<SegInfo>,
    last_paths: Vec<PathInfo>,
    last_cost: [f64; 2],
    user: UserList,
    var: HashMap<Ustr, VarForm>,
    umarks: HashMap<Ustr, i32>,
    fam: HashMap<i64, i32>,
    cls_n: [[i32; 2]; 8],
    user_latin: HashMap<String, LatinWord>,
    keys: Vec<u16>,
    span_end: Vec<i32>,
    alts: Vec<Ustr>,
    alts_n: i32,
    rctx: Vec<u16>,
    prow: [HashMap<i32, Vec<f32>>; 3],
    h_out: Vec<f32>,
}

impl kkc_engine {
    /// Glide word list taken from this engine's lexicon
    /// (`engine.cpp`'s `glide_build`); see [`Lex::glide_words`].
    pub fn glide_lexicon_words(&self, mode: i32) -> Option<Vec<(Vec<u16>, i32)>> {
        self.lex.glide_words(mode)
    }
}

fn encode(e: &mut kkc_engine, tok: &[i32], seg: &[i32]) {
    let (d, heads, ff) = (e.m.d as usize, e.m.heads as usize, e.m.ff as usize);
    let t = tok.len();
    let dh = d / heads.max(1) as usize;
    let mut x = vec![0f32; t * d];
    let mut xn = vec![0f32; t * d];
    let mut qkv = vec![0f32; t * 3 * d];
    let mut att = vec![0f32; t * d];
    let mut tmp = vec![0f32; t * d];
    let mut fb = vec![0f32; t * ff.max(1)];
    let mut ebuf = vec![0f32; d];
    let emb = e.m.emb.clone().unwrap();
    for tt in 0..t {
        let a: Vec<f32> = emb.row_vec(tok[tt] as usize, d);
        for i in 0..d {
            x[tt * d + i] = a[i]
                + e.m.seg[(seg[tt] as usize) * d + i]
                + e.m.pos[tt * d + i];
        }
    }
    let scale = 1.0 / (dh as f32).sqrt();
    for li in 0..e.m.layers as usize {
        let w = e.m.l[li].clone();
        layernorm(&x, t, d, &w.ln1w, &w.ln1b, &mut xn);
        lin(&xn, t, d, &w.inw, 3 * d, Some(&w.inb), &mut qkv, 3 * d);
        let mut sc = vec![0f32; t];
        for hh in 0..heads {
            for tt in 0..t {
                let mut mx = -1e30f32;
                for uu in 0..t {
                    let mut s = 0f32;
                    for i in 0..dh {
                        s += qkv[tt * 3 * d + hh * dh + i] * qkv[uu * 3 * d + d + hh * dh + i];
                    }
                    sc[uu] = s * scale;
                    mx = mx.max(sc[uu]);
                }
                let mut z = 0f32;
                for uu in 0..t {
                    sc[uu] = (sc[uu] - mx).exp();
                    z += sc[uu];
                }
                for i in 0..dh {
                    att[tt * d + hh * dh + i] = 0.0;
                }
                for uu in 0..t {
                    let p = sc[uu] / z;
                    for i in 0..dh {
                        att[tt * d + hh * dh + i] += p * qkv[uu * 3 * d + 2 * d + hh * dh + i];
                    }
                }
            }
        }
        lin(&att, t, d, &w.outw, d, Some(&w.outb), &mut tmp, d);
        for i in 0..x.len() {
            x[i] += tmp[i];
        }
        layernorm(&x, t, d, &w.ln2w, &w.ln2b, &mut xn);
        lin(&xn, t, d, &w.l1w, ff, Some(&w.l1b), &mut fb, ff);
        for v in fb.iter_mut() {
            *v = gelu(*v);
        }
        lin(&fb, t, ff, &w.l2w, d, Some(&w.l2b), &mut tmp, d);
        for i in 0..x.len() {
            x[i] += tmp[i];
        }
    }
    let mut h = vec![0f32; t * d];
    layernorm(&x, t, d, &e.m.normw, &e.m.normb, &mut h);
    e.h_out = h;
}

fn scores(e: &mut kkc_engine, rstart: i32, n: i32, ed: &[Edge]) -> Vec<f32> {
    let (d, dk) = (e.m.d as usize, e.m.dk as usize);
    let h = e.h_out.clone();
    let nu = n as usize;
    let mut bd = vec![0f32; (nu + 1) * d];
    for p in 0..=nu {
        for i in 0..d {
            let left = h[(rstart as usize + p - 1) * d + i];
            let right = if p < nu { h[(rstart as usize + p) * d + i] } else { e.m.end[i] };
            bd[p * d + i] = (left + right) * 0.5;
        }
    }
    let mut qa = vec![0f32; (nu + 1) * d];
    let mut qb = vec![0f32; (nu + 1) * d];
    let mut qc = vec![0f32; nu.max(1) * d];
    lin(&bd, nu + 1, d, &e.m.q1w, d, None, &mut qa, d);
    lin(&bd, nu + 1, d, &e.m.q1w[d * d..], d, None, &mut qb, d);
    if nu > 0 {
        lin(&h[rstart as usize * d..], nu, d, &e.m.q1w[2 * d * d..], d, None, &mut qc, d);
    }
    let mut cq = vec![0f64; (nu + 1) * d];
    for i in 0..nu {
        for j in 0..d {
            cq[(i + 1) * d + j] = cq[i * d + j] + qc[i * d + j] as f64;
        }
    }
    // span q
    let mut span_id: HashMap<i32, i32> = HashMap::new();
    let mut qstore: Vec<f32> = Vec::new();
    let mut span_q = |e: &kkc_engine, span_id: &mut HashMap<i32, i32>, qstore: &mut Vec<f32>, s: i32, en: i32| -> i32 {
        let key = s * 4096 + en;
        if let Some(&id) = span_id.get(&key) {
            return id;
        }
        let inv = 1.0 / (en - s).max(1) as f32;
        let mut hid = vec![0f32; d];
        for j in 0..d {
            hid[j] = qa[s as usize * d + j] + qb[en as usize * d + j]
                + (cq[en as usize * d + j] - cq[s as usize * d + j]) as f32 * inv
                + e.m.q1b[j];
        }
        for v in hid.iter_mut() {
            *v = gelu(*v);
        }
        let id = (qstore.len() / dk) as i32;
        qstore.resize(qstore.len() + dk, 0.0);
        let (qs, qe) = (qstore.len() - dk, qstore.len());
        let mut outv = vec![0f32; dk];
        lin(&hid, 1, d, &e.m.q2w, dk, Some(&e.m.q2b), &mut outv, dk);
        qstore[qs..qe].copy_from_slice(&outv);
        span_id.insert(key, id);
        id
    };
    if e.kcache.len() > 30000 {
        e.kcache.clear();
        e.kstore.clear();
    }
    let mut u = vec![0f32; ed.len()];
    let sc = 1.0 / (dk as f32).sqrt();
    let mut gq: Vec<f32> = Vec::new();
    let mut gq_done: Vec<bool> = Vec::new();
    let ro_on = e.ro.on;
    if ro_on {
        e.ro.prepare();
        e.ro.q = vec![0.0; ed.len() * dk];
        e.ro.k = vec![0.0; ed.len() * dk];
    }
    let mut tbuf = vec![0f32; 2 * d];
    for i in 0..ed.len() {
        let a = span_q(e, &mut span_id, &mut qstore, ed[i].s, ed[i].e);
        // word k (cached across keystrokes by surface+kind+mark)
        let b = {
            let mut key = ed[i].surf.clone();
            key.push(char::from_u32(0x110000 + ed[i].kind as u32).unwrap_or('\u{FFFD}'));
            if ed[i].mark != 0 {
                key.push(char::from_u32(0x120000 + ed[i].mark as u32).unwrap_or('\u{FFFD}'));
            }
            if let Some(&id) = e.kcache.get(&key) {
                id
            } else {
                let edge = ed[i].clone();
                let nc = edge.surf.len();
                let inv = 1.0 / nc.max(1) as f32;
                let kk: Vec<f32> = e.m.kkind[(edge.kind as usize) * d..(edge.kind as usize + 1) * d].to_vec();
                let kl: Vec<f32> = e.m.klen[(nc.min(16)) * d..(nc.min(16) + 1) * d].to_vec();
                let mut hid = vec![0f32; d];
                if let Some(kw) = &e.m.kw {
                    let kw = kw.clone();
                    let emb = e.m.emb.clone().unwrap();
                    for part in 0..3usize {
                        // prow computed lazily in word_k below via closure substitute:
                        let _ = part;
                        let _ = &kw;
                        let _ = &emb;
                    }
                    // v5: sum over chars of prow(0), plus prow(1, first), prow(2, last)
                    let mut prow = |e: &mut kkc_engine, part: usize, v: i32| -> Vec<f32> {
                        if let Some(cached) = e.prow[part].get(&v) {
                            return cached.clone();
                        }
                        let kw = e.m.kw.as_ref().unwrap();
                        let emb = e.m.emb.clone().unwrap();
                        let mut tb = vec![0f32; d];
                        let er: Vec<f32> = emb.row_vec(v as usize, d);
                        let mut outv = vec![0f32; d];
                        let w0 = &kw[part * d * d..];
                        for ii in 0..d {
                            let xv = er[ii];
                            if xv == 0.0 {
                                continue;
                            }
                            for j in 0..d {
                                outv[j] += xv * w0[ii * d + j];
                            }
                        }
                        e.prow[part].insert(v, outv.clone());
                        outv
                    };
                    for &c in &edge.surf {
                        let p = prow(e, 0, e.m.vid(c));
                        for j in 0..d {
                            hid[j] += p[j];
                        }
                    }
                    let pb = prow(e, 1, e.m.vid(*edge.surf.first().unwrap()));
                    let pc = prow(e, 2, e.m.vid(*edge.surf.last().unwrap()));
                    for j in 0..d {
                        hid[j] = hid[j] * inv + pb[j] + kk[j] + pc[j] + kl[j] + e.m.k1b[j];
                    }
                    if let Some(kuser) = &e.m.kuser {
                        let um = (edge.mark % 32).min(e.m.n_user - 1) as usize;
                        let sm = (edge.mark / 32) as usize;
                        for j in 0..d {
                            hid[j] += kuser[um * d + j];
                        }
                        if let Some(kstyle) = &e.m.kstyle {
                            let sm = sm.min((e.m.n_style - 1) as usize);
                            for j in 0..d {
                                hid[j] += kstyle[sm * d + j];
                            }
                        }
                    }
                } else {
                    let pa = e.m.pa.clone().unwrap();
                    let pb = e.m.pb.clone().unwrap();
                    let pc = e.m.pc.clone().unwrap();
                    for &c in &edge.surf {
                        let p: Vec<f32> = pa.row_vec(e.m.vid(c) as usize, d);
                        for j in 0..d {
                            hid[j] += p[j];
                        }
                    }
                    let pbv: Vec<f32> = pb.row_vec(e.m.vid(*edge.surf.first().unwrap()) as usize, d);
                    let mut tbuf2 = vec![0f32; d];
                    let pcv: Vec<f32> = pc.row_vec(e.m.vid(*edge.surf.last().unwrap()) as usize, d);
                    for j in 0..d {
                        hid[j] = hid[j] * inv + pbv[j] + kk[j] + pcv[j] + kl[j] + e.m.k1b[j];
                    }
                }
                for v in hid.iter_mut() {
                    *v = gelu(*v);
                }
                let id = (e.kstore.len() / dk) as i32;
                e.kstore.resize(e.kstore.len() + dk, 0.0);
                let mut outv = vec![0f32; dk];
                lin(&hid, 1, d, &e.m.k2w, dk, Some(&e.m.k2b), &mut outv, dk);
                let (ks, ke) = (e.kstore.len() - dk, e.kstore.len());
                e.kstore[ks..ke].copy_from_slice(&outv);
                e.kcache.insert(key, id);
                id
            }
        };
        let q: Vec<f32> = qstore[(a as usize) * dk..(a as usize + 1) * dk].to_vec();
        let k: Vec<f32> = e.kstore[(b as usize) * dk..(b as usize + 1) * dk].to_vec();
        let mut s = 0f32;
        for j in 0..dk {
            s += q[j] * k[j];
        }
        let mut ui = s * sc;
        if let Some(gw) = &e.m.gw {
            if a as usize >= gq_done.len() {
                gq_done.resize(a as usize + 1, false);
                gq.resize((a as usize + 1) * dk, 0.0);
            }
            if !gq_done[a as usize] {
                let mut outv = vec![0f32; dk];
                lin(&q, 1, dk, gw, dk, Some(&e.m.gb), &mut outv, dk);
                gq[a as usize * dk..(a as usize + 1) * dk].copy_from_slice(&outv);
                gq_done[a as usize] = true;
            }
            let mut t = 0f32;
            for j in 0..dk {
                t += gq[a as usize * dk + j] * k[j];
            }
            let g = 2.0 / (1.0 + (-t * sc).exp());
            ui -= e.m.beta * (g - 1.0) * ed[i].cost as f32 / 1000.0;
        }
        if ro_on {
            let mut rh = vec![0f32; e.ro.h as usize];
            ui += e.ro.extra(&q, &k, sc, &mut rh);
            e.ro.q[i * dk..(i + 1) * dk].copy_from_slice(&q);
            e.ro.k[i * dk..(i + 1) * dk].copy_from_slice(&k);
        }
        u[i] = ui;
    }
    u
}

struct NbestOut {
    res: Vec<Ustr>,
    segs: Vec<SegInfo>,
    paths: Vec<PathInfo>,
    top2: [f64; 2],
}
fn nbest(l: &Lex, e: &[Edge], n: i32, u: Option<&[f32]>, beta: f64, gamma: f64, k: i32, maxout: i32, want_segs: bool) -> NbestOut {
    let m = e.len();
    let mut by_start: Vec<Vec<usize>> = vec![Vec::new(); (n + 1) as usize];
    let mut by_end: Vec<Vec<usize>> = vec![Vec::new(); (n + 1) as usize];
    for (i, ed) in e.iter().enumerate() {
        by_start[ed.s as usize].push(i);
        by_end[ed.e as usize].push(i);
    }
    #[derive(Clone, Copy)]
    struct Cand {
        c: f64,
        j: i32,
        r: i32,
    }
    let mut best: Vec<Vec<Cand>> = vec![Vec::new(); m];
    let mut has = vec![false; m];
    for pos in 0..n {
        for &i in &by_start[pos as usize] {
            let ed = &e[i];
            let own = match u {
                Some(u) => beta * ed.cost as f64 - 1000.0 * u[i] as f64,
                None => beta * ed.cost as f64,
            };
            let mut cands: Vec<Cand> = Vec::new();
            if pos == 0 {
                cands.push(Cand { c: gamma * l.conn(0, ed.lid) as f64 + own, j: -1, r: 0 });
            } else {
                for &j in &by_end[pos as usize] {
                    if !has[j] {
                        continue;
                    }
                    let t = gamma * l.conn(e[j].rid, ed.lid) as f64 + own;
                    for r in 0..best[j].len() {
                        cands.push(Cand { c: best[j][r].c + t, j: j as i32, r: r as i32 });
                    }
                }
                if cands.is_empty() {
                    continue;
                }
            }
            cands.sort_by(|a, b| a.c.partial_cmp(&b.c).unwrap_or(std::cmp::Ordering::Equal)); // stable
            cands.truncate(k.max(0) as usize);
            best[i] = cands;
            has[i] = true;
        }
    }
    let mut fin: Vec<Cand> = Vec::new();
    for &j in &by_end[n as usize] {
        for r in 0..best[j].len() {
            fin.push(Cand { c: best[j][r].c + gamma * l.conn(e[j].rid, 0) as f64, j: j as i32, r: r as i32 });
        }
    }
    fin.sort_by(|a, b| a.c.partial_cmp(&b.c).unwrap_or(std::cmp::Ordering::Equal));
    let mut out = NbestOut { res: Vec::new(), segs: Vec::new(), paths: Vec::new(), top2: [0.0, 1e18] };
    let mut seen: HashSet<Ustr> = HashSet::new();
    for f in &fin {
        let mut path: Vec<usize> = Vec::new();
        let (mut j, mut r) = (f.j, f.r);
        while j >= 0 {
            path.push(j as usize);
            let c = best[j as usize][r as usize];
            j = c.j;
            r = c.r;
        }
        let mut surf: Ustr = Vec::new();
        for &i in path.iter().rev() {
            surf.extend(e[i].surf.iter());
        }
        if !seen.insert(surf.clone()) {
            continue;
        }
        if want_segs {
            let mut pi = PathInfo { c: f.c, w: Vec::new() };
            for &i in path.iter().rev() {
                pi.w.push((e[i].e, e[i].surf.clone()));
            }
            out.paths.push(pi);
        }
        if out.res.is_empty() && want_segs {
            out.segs = path
                .iter()
                .rev()
                .map(|&i| SegInfo { e: e[i].e, l16: l16len(&e[i].surf) as i32, lid: e[i].lid, rid: e[i].rid })
                .collect();
        }
        if out.res.len() < 2 {
            out.top2[out.res.len()] = f.c;
        }
        out.res.push(surf);
        if out.res.len() >= maxout.max(0) as usize {
            break;
        }
    }
    out
}

// ------------------------------------------------------------------ personalize (M5)
const VAR_PENALTY: i32 = 300;
fn count_bucket(n: i32) -> i32 {
    if n <= 0 { 0 } else if n == 1 { 1 } else if n == 2 { 2 } else if n <= 4 { 3 } else if n <= 9 { 4 } else if n <= 19 { 5 } else { 6 }
}
fn rate_bucket(s: f64) -> i32 {
    if s < 0.2 { 1 } else if s < 0.4 { 2 } else if s < 0.6 { 3 } else if s < 0.8 { 4 } else { 5 }
}
fn personalize(e: &mut kkc_engine, edges: &mut Vec<Edge>) {
    if !e.var.is_empty() {
        let key = |x: &Edge| -> Ustr {
            let mut k = x.surf.clone();
            k.push(char::from_u32(0x110000 + x.s as u32).unwrap_or('\u{FFFD}'));
            k.push(char::from_u32(0x110000 + x.e as u32).unwrap_or('\u{FFFD}'));
            k
        };
        let mut have: HashSet<Ustr> = edges.iter().map(|x| key(x)).collect();
        let n0 = edges.len();
        let mut add: Vec<Edge> = Vec::new();
        for i in 0..n0 {
            if edges[i].kind == K_TYPO || edges[i].kind == K_LATIN {
                continue;
            }
            if let Some(v) = e.var.get(&edges[i].surf) {
                let mut ne = edges[i].clone();
                ne.surf = v.other.clone();
                ne.kind = K_DICT;
                ne.cost = edges[i].cost + VAR_PENALTY;
                if have.insert(key(&ne)) {
                    add.push(ne);
                }
            }
        }
        edges.extend(add);
    }
    let (var, umarks, fam, cls_n) = (e.var.clone(), e.umarks.clone(), e.fam.clone(), e.cls_n);
    for x in edges.iter_mut() {
        let um = umarks.get(&x.surf).copied().unwrap_or(0);
        let (mut f, mut c) = (0i32, 0i32);
        if let Some(vf) = var.get(&x.surf) {
            if let Some(&cnt) = fam.get(&(vf.lemma as i64 * 2 + if vf.kanji { 1 } else { 0 })) {
                f = count_bucket(cnt);
            }
            if vf.cls > 0 && vf.cls < 8 {
                let a = cls_n[vf.cls as usize][vf.kanji as usize];
                let b = cls_n[vf.cls as usize][(!vf.kanji) as usize];
                if a + b >= 5 {
                    c = rate_bucket(a as f64 / (a + b) as f64);
                }
            }
        }
        x.mark = um + 32 * (f * 6 + c);
    }
}

// ------------------------------------------------------------------ numeric surface rewrites
fn with_commas_in(t: &[char]) -> Ustr {
    let mut out = Vec::new();
    let mut changed = false;
    let mut i = 0;
    while i < t.len() {
        if !t[i].is_ascii_digit() {
            out.push(t[i]);
            i += 1;
            continue;
        }
        let mut j = i;
        while j < t.len() && t[j].is_ascii_digit() {
            j += 1;
        }
        let run = &t[i..j];
        let edge = (i > 0 && (t[i - 1] == ',' || t[i - 1] == '.'))
            || (j < t.len() && (t[j] == ',' || t[j] == '.' || t[j] == '年'));
        if run.len() >= 4 && !edge {
            out.extend(with_commas(run));
            changed = true;
        } else {
            out.extend(run.iter());
        }
        i = j;
    }
    if changed { out } else { Vec::new() }
}
fn kanjize(top: &[char]) -> Ustr {
    let dig = |c: char| -> i64 {
        if c.is_ascii_digit() {
            c as i64 - '0' as i64
        } else if ('０'..='９').contains(&c) {
            c as i64 - '０' as i64
        } else {
            -1
        }
    };
    let num = |ds: &[char]| -> Ustr {
        if ds == u("0") {
            return u("〇");
        }
        let dd: Vec<char> = "〇一二三四五六七八九".chars().collect();
        let big = ["", "万", "億", "兆"];
        let mut out = Vec::new();
        let n = ds.len() as i32;
        for g in (0..=(n - 1) / 4).rev() {
            let mut v = 0i64;
            for i in (n - 4 * (g + 1)).max(0)..n - 4 * g {
                v = v * 10 + dig(ds[i as usize]);
            }
            if v == 0 {
                continue;
            }
            let (th, hu, te, on) = (v / 1000, v / 100 % 10, v / 10 % 10, v % 10);
            if th > 0 {
                if th > 1 {
                    out.push(dd[th as usize]);
                }
                out.push('千');
            }
            if hu > 0 {
                if hu > 1 {
                    out.push(dd[hu as usize]);
                }
                out.push('百');
            }
            if te > 0 {
                if te > 1 {
                    out.push(dd[te as usize]);
                }
                out.push('十');
            }
            if on > 0 {
                out.push(dd[on as usize]);
            }
            out.extend(big[g as usize].chars());
        }
        out
    };
    let mut out = Vec::new();
    let mut changed = false;
    let mut i = 0;
    while i < top.len() {
        if dig(top[i]) < 0 {
            out.push(top[i]);
            i += 1;
            continue;
        }
        let mut j = i;
        while j < top.len() && dig(top[j]) >= 0 {
            j += 1;
        }
        let ds = &top[i..j];
        let pv = if i > 0 { top[i - 1] } else { '\0' };
        let nx = if j < top.len() { top[j] } else { '\0' };
        let skip = ds.len() > 16
            || (ds.len() > 1 && dig(ds[0]) == 0)
            || pv == '.' || nx == '.' || pv == ',' || nx == ','
            || pv.is_ascii_alphabetic() || nx.is_ascii_alphabetic();
        if skip {
            out.extend(ds.iter());
        } else {
            let d2: Ustr = ds.iter().map(|&c| char::from_u32('0' as u32 + dig(c) as u32).unwrap()).collect();
            out.extend(num(&d2));
            changed = true;
        }
        i = j;
    }
    if changed { out } else { Vec::new() }
}
fn arabicize(r: &[char], top: &[char], segs: &[SegInfo]) -> Ustr {
    let counters: Vec<(Ustr, Ustr)> = [
        ("がつ", "月"), ("にち", "日"), ("じ", "時"), ("ふん", "分"), ("ぷん", "分"), ("びょう", "秒"),
        ("ねん", "年"), ("えん", "円"), ("にん", "人"), ("かい", "回階"), ("こ", "個"), ("ほん", "本"),
        ("ぼん", "本"), ("ぽん", "本"), ("まい", "枚"), ("さい", "歳才"), ("けん", "件軒"), ("ばい", "倍"),
        ("ぱーせんと", "パ%"), ("ぺーじ", "ペ頁"), ("めーとる", "メm"), ("きろ", "キk"), ("せんち", "センc"),
        ("ぐらむ", "グg"), ("どる", "ド"), ("しゅうかん", "週"), ("かげつ", "かカ ヶヵ箇"), ("ひき", "匹"),
        ("びき", "匹"), ("ぴき", "匹"), ("さつ", "冊"), ("とう", "頭"), ("わ", "羽話"), ("てん", "点"),
    ]
    .iter().map(|(a, b)| (u(a), u(b))).collect();
    let numk = u("〇一二三四五六七八九十百千万億兆");
    let days: Vec<(Ustr, i64)> = [
        ("ついたち", 1), ("ふつか", 2), ("みっか", 3), ("よっか", 4), ("いつか", 5), ("むいか", 6), ("なのか", 7),
        ("ようか", 8), ("ここのか", 9), ("とおか", 10), ("じゅうよっか", 14), ("はつか", 20), ("にじゅうよっか", 24),
    ]
    .iter().map(|(a, b)| (u(a), *b)).collect();
    let mut heads: Vec<(usize, usize)> = Vec::new();
    let (mut rs, mut ss) = (0usize, 0usize);
    for g in segs {
        heads.push((rs, ss));
        let mut l = 0;
        while ss < top.len() && l < g.l16 {
            l += if (top[ss] as u32) >= 0x10000 { 2 } else { 1 };
            ss += 1;
        }
        rs = g.e as usize;
    }
    let mut out: Ustr = Vec::new();
    let mut changed = false;
    let mut cur = 0usize;
    for h in &heads {
        let (r0, s0) = *h;
        if s0 < cur || s0 >= top.len() {
            continue;
        }
        if !numk.contains(&top[s0]) {
            continue;
        }
        if s0 > 0 && top[s0 - 1] == '月' {
            let mut hit = false;
            for d in &days {
                let mut kd = kanji_num(d.1);
                kd.push('日');
                if starts_with(r, r0, &d.0) && top[s0..].starts_with(&kd) {
                    out.extend(top[cur..s0].iter());
                    out.extend(digits_str(d.1, false));
                    out.push('日');
                    cur = s0 + kd.len();
                    changed = true;
                    hit = true;
                    break;
                }
            }
            if hit {
                continue;
            }
        }
        let (mut v, mut pe, mut kj) = (0i64, 0usize, Vec::new());
        for b in boundaries(r, r0) {
            let sub = &r[r0..b];
            let x = match parse_num1(sub) {
                Some(x) => Some(x),
                None => parse_num_full(sub).map(|(x, _)| x),
            };
            let Some(x) = x else { continue };
            let k = kanji_num(x);
            if !top[s0..].starts_with(&k) {
                continue;
            }
            if s0 + k.len() < top.len() && numk.contains(&top[s0 + k.len()]) {
                continue;
            }
            v = x;
            pe = b;
            kj = k;
        }
        if pe == 0 || v < 2 {
            continue;
        }
        let nx = if s0 + kj.len() < top.len() { top[s0 + kj.len()] } else { '\0' };
        let mut ok = v >= 100 && pe - r0 >= 3 && !(0x4E00..=0x9FFF).contains(&(nx as u32));
        if !ok {
            for c in &counters {
                if starts_with(r, pe, &c.0) && s0 + kj.len() < top.len() && c.1.contains(&top[s0 + kj.len()]) {
                    ok = true;
                    break;
                }
            }
        }
        if !ok {
            continue;
        }
        out.extend(top[cur..s0].iter());
        out.extend(if v >= 10000 { unit_mixed(v, false) } else { digits_str(v, false) });
        cur = s0 + kj.len();
        changed = true;
    }
    if !changed {
        return Vec::new();
    }
    out.extend(top[cur..].iter());
    out
}
fn is_emoji(s: &[char]) -> bool {
    let mut hit = false;
    for &c in s {
        let v = c as u32;
        if (0x3040..=0x9FFF).contains(&v) || (v < 0x2000 && c != '\u{200D}' && c != '\u{20E3}') {
            return false;
        }
        if (0x1F000..=0x1FAFF).contains(&v) || (0x2600..=0x27BF).contains(&v) || (0x2B00..=0x2BFF).contains(&v) {
            hit = true;
        }
    }
    hit
}

// ------------------------------------------------------------------ date/time candidates
// civil_from_unix は UTC 分解。地方時は local_from_unix (下記 TZ 解釈) を使う。
#[derive(Clone, Copy)]
struct Tm {
    year: i64, // full year
    mon: i64,  // 1..12
    mday: i64,
    hour: i64,
    min: i64,
    wday: i64, // 0 = Sunday
}
fn civil_from_unix(t: i64) -> Tm {
    let days = t.div_euclid(86400);
    let secs = t.rem_euclid(86400);
    // Howard Hinnant's civil_from_days
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    Tm {
        year: if m <= 2 { y + 1 } else { y },
        mon: m,
        mday: d,
        hour: secs / 3600,
        min: secs % 3600 / 60,
        wday: (days + 4).rem_euclid(7),
    }
}
fn unix_from_civil(year: i64, mon: i64, mday: i64, hour: i64) -> i64 {
    let y = if mon <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = if mon > 2 { mon - 3 } else { mon + 9 };
    let doy = (153 * mp + 2) / 5 + mday - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    (era * 146097 + doe - 719468) * 86400 + hour * 3600
}
// ------------------------------------------------------------------ local time (C++ localtime 相当)
// TZ は環境変数 TZ (無ければ /etc/localtime) から取る。外部クレート無しで、
// TZif ファイル (zoneinfo) と POSIX TZ 文字列 (JST-9 / EST5EDT,M3.2.0/2,M11.1.0/2 等)
// の両方を自前で解釈する。
struct TzRule {
    // Mm.w.d (月・週・曜日 0=日曜) / Jn (1..365, うるう日を数えない) / n (0..365)
    kind: u8, // 0=M, 1=Jn, 2=n
    m: i64,
    w: i64,
    d: i64,
    time: i64, // 地方時での秒 (既定 2:00。負や 24 時超もありうる)
}
struct PosixTz {
    std_off: i64, // UTC からのずれ (東が正)
    dst: Option<(i64, TzRule, TzRule)>, // (DST のずれ, 開始, 終了)
}
struct Tzif {
    trans: Vec<(i64, usize)>, // (時刻, ttinfo 番号)
    offs: Vec<i64>,
    isdst: Vec<bool>,
    footer: Option<PosixTz>,
}
enum Tz {
    Fixed(i64),
    Posix(PosixTz),
    File(Tzif),
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let yy = if m <= 2 { y - 1 } else { y };
    let era = yy.div_euclid(400);
    let yoe = yy.rem_euclid(400);
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    era * 146097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719468
}

fn parse_tz_rule(s: &[u8], i: &mut usize) -> Option<TzRule> {
    let mut r = TzRule { kind: 0, m: 0, w: 0, d: 0, time: 7200 };
    if *i >= s.len() {
        return None;
    }
    if s[*i] == b'M' {
        *i += 1;
        let mut num = |s: &[u8], i: &mut usize| -> Option<i64> {
            let mut v: i64 = 0;
            let mut any = false;
            while *i < s.len() && s[*i].is_ascii_digit() {
                v = v * 10 + (s[*i] - b'0') as i64;
                *i += 1;
                any = true;
            }
            if any { Some(v) } else { None }
        };
        r.kind = 0;
        r.m = num(s, i)?;
        if *i < s.len() && s[*i] == b'.' {
            *i += 1;
        } else {
            return None;
        }
        r.w = num(s, i)?;
        if *i < s.len() && s[*i] == b'.' {
            *i += 1;
        } else {
            return None;
        }
        r.d = num(s, i)?;
    } else if s[*i] == b'J' {
        *i += 1;
        let mut v: i64 = 0;
        let mut any = false;
        while *i < s.len() && s[*i].is_ascii_digit() {
            v = v * 10 + (s[*i] - b'0') as i64;
            *i += 1;
            any = true;
        }
        if !any {
            return None;
        }
        r.kind = 1;
        r.d = v;
    } else if s[*i].is_ascii_digit() {
        let mut v: i64 = 0;
        while *i < s.len() && s[*i].is_ascii_digit() {
            v = v * 10 + (s[*i] - b'0') as i64;
            *i += 1;
        }
        r.kind = 2;
        r.d = v;
    } else {
        return None;
    }
    if *i < s.len() && s[*i] == b'/' {
        *i += 1;
        r.time = parse_tz_hms(s, i, true)?;
    }
    Some(r)
}

/// [+-]hh[:mm[:ss]] を秒に。POSIX の符号 (地方時 + これ = UTC) のまま返す。
fn parse_tz_hms(s: &[u8], i: &mut usize, signed: bool) -> Option<i64> {
    let mut sign = 1i64;
    if signed && *i < s.len() && (s[*i] == b'-' || s[*i] == b'+') {
        if s[*i] == b'-' {
            sign = -1;
        }
        *i += 1;
    }
    let mut parts = [0i64; 3];
    for k in 0..3 {
        let mut v: i64 = 0;
        let mut any = false;
        while *i < s.len() && s[*i].is_ascii_digit() {
            v = v * 10 + (s[*i] - b'0') as i64;
            *i += 1;
            any = true;
        }
        if !any {
            if k == 0 {
                return None;
            }
            break;
        }
        parts[k] = v;
        if *i < s.len() && s[*i] == b':' {
            *i += 1;
        } else {
            break;
        }
    }
    Some(sign * (parts[0] * 3600 + parts[1] * 60 + parts[2]))
}

fn parse_posix_tz(s: &[u8]) -> Option<PosixTz> {
    let mut i = 0usize;
    let name = |s: &[u8], i: &mut usize| -> bool {
        if *i < s.len() && s[*i] == b'<' {
            while *i < s.len() && s[*i] != b'>' {
                *i += 1;
            }
            if *i < s.len() {
                *i += 1;
                return true;
            }
            return false;
        }
        let st = *i;
        while *i < s.len() && !s[*i].is_ascii_digit() && s[*i] != b'-' && s[*i] != b'+' && s[*i] != b',' {
            *i += 1;
        }
        *i - st >= 3
    };
    if !name(s, &mut i) {
        return None;
    }
    // POSIX のオフセットは「地方時に足すと UTC」なので東のずれは符号反転
    let std_off = -parse_tz_hms(s, &mut i, true)?;
    if i >= s.len() {
        return Some(PosixTz { std_off, dst: None });
    }
    if !name(s, &mut i) {
        return None;
    }
    let dst_off = if i < s.len() && s[i] != b',' {
        -parse_tz_hms(s, &mut i, true)?
    } else {
        std_off + 3600
    };
    let (start, end) = if i < s.len() && s[i] == b',' {
        i += 1;
        let st = parse_tz_rule(s, &mut i)?;
        if i < s.len() && s[i] == b',' {
            i += 1;
        } else {
            return None;
        }
        let en = parse_tz_rule(s, &mut i)?;
        (st, en)
    } else {
        // 規則が無ければ米国の既定 (glibc と同じ)
        (
            TzRule { kind: 0, m: 3, w: 2, d: 0, time: 7200 },
            TzRule { kind: 0, m: 11, w: 1, d: 0, time: 7200 },
        )
    };
    Some(PosixTz { std_off, dst: Some((dst_off, start, end)) })
}

/// 規則が地方時で発火する UTC 時刻 (year 年)。off は発火前のずれ。
fn tz_rule_unix(r: &TzRule, year: i64, off: i64) -> i64 {
    let day = match r.kind {
        0 => {
            // その月の第 w 週の曜日 d (w=5 は最終週)
            let first = days_from_civil(year, r.m, 1);
            let wd1 = (first + 4).rem_euclid(7); // 0=日曜
            let mut dom = 1 + (r.d - wd1).rem_euclid(7) + (r.w - 1) * 7;
            let next = days_from_civil(year, r.m + 1, 1);
            if first + dom - 1 >= next {
                dom -= 7;
            }
            first + dom - 1
        }
        1 => {
            // Jn: 2/29 を数えない
            let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
            let mut doy = r.d - 1;
            if leap && r.d > 59 {
                doy += 1;
            }
            days_from_civil(year, 1, 1) + doy
        }
        _ => days_from_civil(year, 1, 1) + r.d,
    };
    day * 86400 + r.time - off
}

fn posix_offset_at(tz: &PosixTz, t: i64) -> i64 {
    let Some((dst_off, start, end)) = &tz.dst else {
        return tz.std_off;
    };
    // 前後 1 年分も含めて発火時刻を並べ、t 以前の最後の発火で決める
    let y = civil_from_unix(t).year;
    let mut ev: Vec<(i64, i64)> = Vec::new();
    for yy in (y - 1)..=(y + 1) {
        ev.push((tz_rule_unix(start, yy, tz.std_off), *dst_off));
        ev.push((tz_rule_unix(end, yy, *dst_off), tz.std_off));
    }
    ev.sort();
    let mut off = tz.std_off;
    for (tt, o) in ev {
        if tt <= t {
            off = o;
        } else {
            break;
        }
    }
    off
}

fn parse_tzif(data: &[u8]) -> Option<Tzif> {
    if data.len() < 44 || &data[0..4] != b"TZif" {
        return None;
    }
    let ver = data[4];
    let cnt = |o: usize| -> u32 { u32::from_be_bytes(data[o..o + 4].try_into().unwrap()) };
    let counts = |o: usize| -> (usize, usize, usize, usize, usize, usize) {
        (
            cnt(o + 20) as usize, // isutcnt
            cnt(o + 24) as usize, // isstdcnt
            cnt(o + 28) as usize, // leapcnt
            cnt(o + 32) as usize, // timecnt
            cnt(o + 36) as usize, // typecnt
            cnt(o + 40) as usize, // charcnt
        )
    };
    let block = |o: usize, wide: bool, c: &(usize, usize, usize, usize, usize, usize)| -> Option<(Tzif, usize)> {
        let (isut, isstd, leap, tc, ty, ch) = *c;
        let tsz = if wide { 8 } else { 4 };
        let mut p = o;
        let mut trans = Vec::new();
        for _ in 0..tc {
            if p + tsz > data.len() {
                return None;
            }
            let v = if wide {
                i64::from_be_bytes(data[p..p + 8].try_into().unwrap())
            } else {
                i32::from_be_bytes(data[p..p + 4].try_into().unwrap()) as i64
            };
            p += tsz;
            trans.push(v);
        }
        if p + tc + ty * 6 + ch > data.len() {
            return None;
        }
        let idx: Vec<usize> = (0..tc).map(|k| data[p + k] as usize).collect();
        p += tc;
        let mut offs = Vec::new();
        let mut isdst = Vec::new();
        for _ in 0..ty {
            offs.push(i32::from_be_bytes(data[p..p + 4].try_into().unwrap()) as i64);
            isdst.push(data[p + 4] != 0);
            p += 6;
        }
        p += ch + leap * (tsz + 4) + isstd + isut;
        let pairs: Vec<(i64, usize)> = trans.into_iter().zip(idx).filter(|&(_, i)| i < offs.len()).collect();
        Some((Tzif { trans: pairs, offs, isdst, footer: None }, p))
    };
    if ver == 0 {
        let c = counts(0);
        let (z, _) = block(44, false, &c)?;
        return Some(z);
    }
    // v2+: 第 1 ブロックを飛ばして第 2 ブロックを読む
    let c1 = counts(0);
    let (_, used) = block(44, false, &c1)?;
    if used + 44 > data.len() {
        return None;
    }
    let c2 = counts(used);
    let (mut z, used2) = block(used + 44, true, &c2)?;
    // 末尾の POSIX TZ 文字列 (改行で囲まれた部分) があれば将来分に使う
    if used2 < data.len() {
        let rest = &data[used2..];
        if rest.first() == Some(&b'\n') {
            if let Some(end) = rest[1..].iter().position(|&b| b == b'\n') {
                z.footer = parse_posix_tz(&rest[1..1 + end]);
            }
        }
    }
    Some(z)
}

fn tzif_offset_at(z: &Tzif, t: i64) -> i64 {
    if z.offs.is_empty() {
        return 0;
    }
    if z.trans.is_empty() {
        return z.offs[0];
    }
    if t >= z.trans[z.trans.len() - 1].0 {
        if let Some(f) = &z.footer {
            return posix_offset_at(f, t);
        }
    }
    let idx = match z.trans.binary_search_by(|p| p.0.cmp(&t)) {
        Ok(k) => z.trans[k].1,
        Err(0) => {
            // 最初の発火より前: glibc と同様、標準時らしい最初の種別を使う
            let mut pick = z.trans[0].1;
            for k in 0..z.offs.len() {
                if !z.isdst[k] {
                    pick = k;
                    break;
                }
            }
            pick
        }
        Err(k) => z.trans[k - 1].1,
    };
    z.offs[idx]
}

fn load_tz() -> Tz {
    match std::env::var("TZ") {
        Ok(tzv) => {
            let name = tzv.strip_prefix(':').unwrap_or(&tzv);
            if name.is_empty() {
                return Tz::Fixed(0); // TZ="" は UTC (POSIX)
            }
            let path = if name.starts_with('/') {
                name.to_string()
            } else {
                format!("/usr/share/zoneinfo/{name}")
            };
            if let Ok(data) = std::fs::read(&path) {
                if let Some(z) = parse_tzif(&data) {
                    return Tz::File(z);
                }
            }
            if let Some(p) = parse_posix_tz(name.as_bytes()) {
                return Tz::Posix(p);
            }
            Tz::Fixed(0)
        }
        Err(_) => {
            if let Ok(data) = std::fs::read("/etc/localtime") {
                if let Some(z) = parse_tzif(&data) {
                    return Tz::File(z);
                }
            }
            Tz::Fixed(0)
        }
    }
}

/// TZ 環境変数の値ごとに 1 度だけ解釈して使い回す。
fn tz_offset_at(t: i64) -> i64 {
    use std::sync::Mutex;
    static CACHE: Mutex<Option<(String, Tz)>> = Mutex::new(None);
    let key = std::env::var("TZ").unwrap_or_default();
    let mut g = CACHE.lock().unwrap();
    if g.as_ref().map_or(true, |(k, _)| *k != key) {
        *g = Some((key, load_tz()));
    }
    let tz = &g.as_ref().unwrap().1;
    match tz {
        Tz::Fixed(o) => *o,
        Tz::Posix(p) => posix_offset_at(p, t),
        Tz::File(z) => tzif_offset_at(z, t),
    }
}

/// C++ の localtime と同じ地方時の分解。
fn local_from_unix(t: i64) -> Tm {
    civil_from_unix(t + tz_offset_at(t))
}

fn now_unix() -> i64 {
    if let Ok(s) = std::env::var("KKC_NOW") {
        let v: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(v) = v.parse::<i64>() {
            return v;
        }
    }
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}
fn date_candidates(r: &[char]) -> Vec<Ustr> {
    let mut out: Vec<Ustr> = Vec::new();
    let now = now_unix();
    let num = |v: i64| digits_str(v, false);
    let two = |v: i64| -> Ustr { vec![char::from_u32('0' as u32 + (v / 10) as u32).unwrap(), char::from_u32('0' as u32 + (v % 10) as u32).unwrap()] };
    let wd: Vec<char> = "日月火水木金土".chars().collect();
    let era = |t: &Tm| -> Ustr {
        let y = t.year;
        if y > 2019 || (y == 2019 && t.mon >= 5) {
            let mut s = u("令和");
            if y == 2019 {
                s.push('元');
            } else {
                s.extend(num(y - 2018));
            }
            s.push('年');
            return s;
        }
        Vec::new()
    };
    let days_tbl: Vec<(Ustr, i64)> = [
        ("きょう", 0), ("ほんじつ", 0), ("あした", 1), ("あす", 1), ("みょうにち", 1), ("あさって", 2),
        ("みょうごにち", 2), ("しあさって", 3), ("やのあさって", 3), ("きのう", -1), ("さくじつ", -1), ("おととい", -2),
        ("おとつい", -2), ("いっさくじつ", -2), ("さきおととい", -3),
    ]
    .iter().map(|(a, b)| (u(a), *b)).collect();
    for d in &days_tbl {
        if r != d.0.as_slice() {
            continue;
        }
        let t = local_from_unix(now + d.1 * 86400);
        let (y, m, dd) = (t.year, t.mon, t.mday);
        let mut a = num(y);
        a.push('/');
        a.extend(num(m));
        a.push('/');
        a.extend(num(dd));
        out.push(a);
        let mut a = num(y);
        a.push('年');
        a.extend(num(m));
        a.push('月');
        a.extend(num(dd));
        a.push('日');
        out.push(a);
        let mut a = num(m);
        a.push('/');
        a.extend(num(dd));
        out.push(a);
        let mut a = num(m);
        a.push('月');
        a.extend(num(dd));
        a.push('日');
        out.push(a.clone());
        let mut a2 = a;
        a2.push('(');
        a2.push(wd[t.wday as usize]);
        a2.push(')');
        out.push(a2);
        let mut a = kanji_num(m);
        a.push('月');
        a.extend(kanji_num(dd));
        a.push('日');
        out.push(a);
        let e = era(&t);
        if !e.is_empty() {
            let mut a = e;
            a.extend(num(m));
            a.push('月');
            a.extend(num(dd));
            a.push('日');
            out.push(a);
        }
        let mut a = num(y);
        a.push('/');
        a.extend(two(m));
        a.push('/');
        a.extend(two(dd));
        out.push(a);
        return out;
    }
    if r == u("いま") || r == u("げんざい") {
        let t = local_from_unix(now);
        let mut a = two(t.hour);
        a.push(':');
        a.extend(two(t.min));
        out.push(a);
        let mut a = num(t.hour);
        a.push('時');
        a.extend(num(t.min));
        a.push('分');
        out.push(a);
        let h12 = t.hour % 12;
        let mut a = u(if t.hour < 12 { "午前" } else { "午後" });
        a.extend(num(h12));
        a.push('時');
        a.extend(num(t.min));
        a.push('分');
        out.push(a);
        return out;
    }
    for (k, off) in [(u("ことし"), 0), (u("ほんねん"), 0), (u("こんねん"), 0), (u("らいねん"), 1), (u("みょうねん"), 1), (u("きょねん"), -1), (u("さくねん"), -1)] {
        if r == k.as_slice() {
            let mut t = local_from_unix(now);
            t.year += off;
            t.mon = 12;
            let mut a = num(t.year);
            a.push('年');
            out.push(a);
            let e = era(&t);
            if !e.is_empty() {
                out.push(e);
            }
            return out;
        }
    }
    for (k, off) in [(u("こんげつ"), 0), (u("らいげつ"), 1), (u("せんげつ"), -1)] {
        if r == k.as_slice() {
            let t = local_from_unix(now);
            let mut m = t.mon + off;
            let mut y = t.year;
            if m == 0 {
                m = 12;
                y -= 1;
            }
            if m == 13 {
                m = 1;
                y += 1;
            }
            let mut a = num(m);
            a.push('月');
            out.push(a);
            let mut a = num(y);
            a.push('年');
            a.extend(num(m));
            a.push('月');
            out.push(a);
            return out;
        }
    }
    // relative dates below share put_date
    let put_date = |out: &mut Vec<Ustr>, tt: i64| {
        let t = local_from_unix(tt);
        let (y, m, dd) = (t.year, t.mon, t.mday);
        let mut a = num(m);
        a.push('/');
        a.extend(num(dd));
        a.push('(');
        a.push(wd[t.wday as usize]);
        a.push(')');
        out.push(a);
        let mut a = num(m);
        a.push('月');
        a.extend(num(dd));
        a.push('日');
        a.push('(');
        a.push(wd[t.wday as usize]);
        a.push(')');
        out.push(a);
        let mut a = num(y);
        a.push('/');
        a.extend(num(m));
        a.push('/');
        a.extend(num(dd));
        out.push(a);
        let mut a = num(y);
        a.push('年');
        a.extend(num(m));
        a.push('月');
        a.extend(num(dd));
        a.push('日');
        out.push(a);
    };
    let day_num = |w: &[char]| -> Option<i64> {
        for (k, v) in [
            ("ついたち", 1), ("ふつか", 2), ("みっか", 3), ("よっか", 4), ("いつか", 5), ("むいか", 6), ("なのか", 7),
            ("ようか", 8), ("ここのか", 9), ("とおか", 10), ("じゅうよっか", 14), ("はつか", 20), ("にじゅうよっか", 24),
        ] {
            if w == u(k).as_slice() {
                return Some(v);
            }
        }
        if ends_with(w, &u("にち")) {
            if let Some(v) = parse_num1(&w[..w.len() - 2]) {
                if (1..=31).contains(&v) {
                    return Some(v);
                }
            }
        }
        None
    };
    // weekday phrases
    {
        let t0 = local_from_unix(now);
        let wdr: Vec<Ustr> = ["にち", "げつ", "か", "すい", "もく", "きん", "ど"].iter().map(|s| u(s)).collect();
        let weekday = |rest: &[char], short_ok: bool| -> i64 {
            for w in 0..7i64 {
                let k = &wdr[w as usize];
                let mut a = k.clone();
                a.extend(u("ようび"));
                let mut b = k.clone();
                b.extend(u("よう"));
                if rest == a.as_slice() || rest == b.as_slice() || (short_ok && rest == k.as_slice() && k.len() >= 2) {
                    return w;
                }
            }
            -1
        };
        for (k, off) in [(u("さらいしゅう"), 14), (u("らいしゅう"), 7), (u("こんしゅう"), 0), (u("せんせんしゅう"), -14), (u("せんしゅう"), -7)] {
            if !starts_with(r, 0, &k) {
                continue;
            }
            let mut rest = &r[k.len()..];
            if starts_with(rest, 0, &u("の")) {
                rest = &rest[1..];
            }
            let w = weekday(rest, true);
            if w < 0 {
                continue;
            }
            let mon_off = (t0.wday + 6) % 7;
            let target = (w + 6) % 7;
            put_date(&mut out, now + (-mon_off + target + off) * 86400);
            return out;
        }
        for (k, sgn) in [(u("このつぎの"), 1), (u("つぎの"), 1), (u("こんどの"), 1), (u("きたる"), 1), (u("このまえの"), -1), (u("まえの"), -1), (u("さきの"), -1), (u("さる"), -1)] {
            if !starts_with(r, 0, &k) {
                continue;
            }
            let w = weekday(&r[k.len()..], true);
            if w < 0 {
                continue;
            }
            let mut d = (w - t0.wday + 7) % 7;
            if sgn > 0 {
                if d == 0 {
                    d = 7;
                }
            } else {
                let back = (t0.wday - w + 7) % 7;
                d = -(if back == 0 { 7 } else { back });
            }
            put_date(&mut out, now + d * 86400);
            return out;
        }
        let w = weekday(r, false);
        if w >= 0 && r.len() >= 4 {
            put_date(&mut out, now + (w - t0.wday + 7) % 7 * 86400);
            return out;
        }
        for (k, off) in [(u("らいしゅうまつ"), 7), (u("こんしゅうまつ"), 0), (u("しゅうまつ"), 0), (u("せんしゅうまつ"), -7)] {
            if r == k.as_slice() {
                let mon_off = (t0.wday + 6) % 7;
                let sat = now + (-mon_off + 5 + off) * 86400;
                put_date(&mut out, sat);
                let ts = local_from_unix(sat);
                let tsu = local_from_unix(sat + 86400);
                let mut a = num(ts.mon);
                a.push('/');
                a.extend(num(ts.mday));
                a.push('〜');
                a.extend(num(tsu.mon));
                a.push('/');
                a.extend(num(tsu.mday));
                out.insert(1, a);
                return out;
            }
        }
        for (k, off) in [(u("らいげつまつ"), 1), (u("こんげつまつ"), 0), (u("げつまつ"), 0), (u("せんげつまつ"), -1)] {
            if r == k.as_slice() {
                // last day of the (offset) month: first of next month minus a day
                let t = local_from_unix(now);
                let mut y = t.year;
                let mut m = t.mon + off + 1;
                while m > 12 {
                    m -= 12;
                    y += 1;
                }
                while m < 1 {
                    m += 12;
                    y -= 1;
                }
                put_date(&mut out, unix_from_civil(y, m, 1, 12) - 86400);
                return out;
            }
        }
        if r == u("ねんまつ") || r == u("らいねんまつ") {
            let t = local_from_unix(now);
            let y = t.year + if r == u("ねんまつ") { 0 } else { 1 };
            put_date(&mut out, unix_from_civil(y, 12, 31, 12));
            return out;
        }
    }
    // N days / weeks later/before
    for (k, sgn) in [(u("ご"), 1), (u("あと"), 1), (u("まえ"), -1)] {
        if !ends_with(r, &k) {
            continue;
        }
        let body = &r[..r.len() - k.len()];
        if let Some(d) = day_num(body) {
            if body.len() >= 3 {
                put_date(&mut out, now + d * sgn * 86400);
                return out;
            }
        }
        if ends_with(body, &u("しゅうかん")) {
            let wn = &body[..body.len() - 5];
            let v = parse_num1(wn).or_else(|| parse_num_full(wn).map(|(v, _)| v));
            if let Some(v) = v {
                if (1..=52).contains(&v) {
                    put_date(&mut out, now + v * 7 * sgn * 86400);
                    return out;
                }
            }
        }
    }
    // N months / years / hours / minutes later/before
    for (k, sgn) in [(u("ご"), 1), (u("あと"), 1), (u("まえ"), -1)] {
        if !ends_with(r, &k) {
            continue;
        }
        let body = &r[..r.len() - k.len()];
        let numof = |x: &[char]| -> Option<i64> {
            parse_num1(x).or_else(|| parse_num_full(x).map(|(v, _)| v)).or(if x == u("いっ") { Some(1) } else { None })
        };
        if let Some(pos) = body.windows(3).position(|w| w == u("かげつ")) {
            if let Some(v) = numof(&body[..pos]) {
                if (1..=36).contains(&v) {
                    let t = local_from_unix(now);
                    let tt = unix_from_civil(t.year, t.mon + v * sgn, t.mday.min(28), 12);
                    // normalize month overflow like mktime (approx: civil fn handles mon>12 via era math on mon<=2 branch only;
                    // do explicit normalization)
                    let mut y = t.year;
                    let mut m = t.mon + v * sgn;
                    while m > 12 {
                        m -= 12;
                        y += 1;
                    }
                    while m < 1 {
                        m += 12;
                        y -= 1;
                    }
                    let tt = unix_from_civil(y, m, t.mday.min(28), 12);
                    let _ = tt;
                    let uu = local_from_unix(unix_from_civil(y, m, t.mday.min(28), 12));
                    let mut a = num(uu.mon);
                    a.push('月');
                    out.push(a);
                    let mut a = num(uu.year);
                    a.push('年');
                    a.extend(num(uu.mon));
                    a.push('月');
                    out.push(a);
                    put_date(&mut out, unix_from_civil(y, m, t.mday.min(28), 12));
                    return out;
                }
            }
        }
        if ends_with(body, &u("ねん")) {
            if let Some(v) = numof(&body[..body.len() - 2]) {
                if (1..=30).contains(&v) {
                    let t = local_from_unix(now);
                    let y = t.year + v * sgn;
                    let mut a = num(y);
                    a.push('年');
                    out.push(a);
                    if y >= 2019 {
                        let mut a = u("令和");
                        if y == 2019 {
                            a.push('元');
                        } else {
                            a.extend(num(y - 2018));
                        }
                        a.push('年');
                        out.push(a);
                    }
                    put_date(&mut out, unix_from_civil(y, t.mon, t.mday, 12));
                    return out;
                }
            }
        }
        let mut mins: i64 = -1;
        if ends_with(body, &u("じかん")) {
            if let Some(v) = numof(&body[..body.len() - 3]) {
                if (1..=48).contains(&v) {
                    mins = v * 60;
                }
            }
        } else if ends_with(body, &u("ふん")) || ends_with(body, &u("ぷん")) {
            if let Some(v) = numof(&body[..body.len() - 2]) {
                if (1..=600).contains(&v) {
                    mins = v;
                }
            }
        }
        if mins > 0 {
            let t = local_from_unix(now + mins * 60 * sgn);
            let mut a = num(t.hour);
            a.push(':');
            a.extend(two(t.min));
            out.push(a);
            let mut a = num(t.hour);
            a.push('時');
            a.extend(num(t.min));
            a.push('分');
            out.push(a);
            return out;
        }
    }
    // month + day (らいげつついたち)
    for (k, off) in [(u("さらいげつ"), 2), (u("らいげつ"), 1), (u("こんげつ"), 0), (u("せんげつ"), -1)] {
        if !starts_with(r, 0, &k) {
            continue;
        }
        let mut rest = &r[k.len()..];
        if starts_with(rest, 0, &u("の")) {
            rest = &rest[1..];
        }
        if rest.is_empty() {
            continue;
        }
        if let Some(d) = day_num(rest) {
            let t = local_from_unix(now);
            let mut y = t.year;
            let mut m = t.mon + off;
            while m > 12 {
                m -= 12;
                y += 1;
            }
            while m < 1 {
                m += 12;
                y -= 1;
            }
            put_date(&mut out, unix_from_civil(y, m, d, 12));
            return out;
        }
    }
    // era <-> western year
    if ends_with(r, &u("ねん")) {
        let eras: Vec<(Ustr, i64, &str)> = [
            (u("れいわ"), 2018, "令和"), (u("へいせい"), 1988, "平成"), (u("しょうわ"), 1925, "昭和"),
            (u("たいしょう"), 1911, "大正"), (u("めいじ"), 1867, "明治"),
        ]
        .to_vec();
        for (i, er) in eras.iter().enumerate() {
            let _ = i;
            if !starts_with(r, 0, &er.0) {
                continue;
            }
            let body = &r[er.0.len()..r.len() - 2];
            let v = if body == u("がん") { Some(1) } else { parse_num1(body) };
            if let Some(v) = v {
                if (1..=64).contains(&v) {
                    let mut a = num(er.1 + v);
                    a.push('年');
                    out.push(a);
                    let mut a = u(er.2);
                    if v == 1 {
                        a.push('元');
                    } else {
                        a.extend(num(v));
                    }
                    a.push('年');
                    a.push('(');
                    a.extend(num(er.1 + v));
                    a.push('年');
                    a.push(')');
                    out.push(a);
                    return out;
                }
            }
        }
        if let Some(y) = parse_num1(&r[..r.len() - 2]) {
            if (1868..=2100).contains(&y) {
                let (name, base): (&str, i64) = if y >= 2019 {
                    ("令和", 2018)
                } else if y >= 1989 {
                    ("平成", 1988)
                } else if y >= 1926 {
                    ("昭和", 1925)
                } else if y >= 1912 {
                    ("大正", 1911)
                } else {
                    ("明治", 1867)
                };
                let mut e = u(name);
                if y == base + 1 {
                    e.push('元');
                } else {
                    e.extend(num(y - base));
                }
                let mut a = e.clone();
                a.push('年');
                out.push(a);
                let mut a = num(y);
                a.push('年');
                a.push('(');
                a.extend(e);
                a.push('年');
                a.push(')');
                out.push(a);
                return out;
            }
        }
    }
    // clock time
    {
        let mut t: &[char] = r;
        let mut ampm = 0;
        if starts_with(t, 0, &u("ごぜん")) {
            ampm = 1;
            t = &t[3..];
        } else if starts_with(t, 0, &u("ごご")) {
            ampm = 2;
            t = &t[2..];
        }
        let hour = |x: &[char]| -> Option<i64> {
            if x == u("よ") {
                return Some(4);
            }
            if x == u("く") {
                return Some(9);
            }
            if let Some(v) = parse_num1(x) {
                return Some(v);
            }
            if ends_with(x, &u("よ")) {
                let mut y = x.to_vec();
                y.push('ん');
                return parse_num1(&y);
            }
            None
        };
        let mut j: Option<usize> = None;
        if t.len() >= 2 {
            for q in (1..t.len()).rev() {
                if t[q] == 'じ' && hour(&t[..q]).is_some() {
                    j = Some(q);
                    break;
                }
            }
        }
        if let Some(j) = j {
            if j > 0 {
                let hs = &t[..j];
                let rest = &t[j + 1..];
                if let Some(h) = hour(hs) {
                    let mut mi = 0i64;
                    let mut ok = true;
                    if !rest.is_empty() {
                        if rest == u("はん") {
                            mi = 30;
                        } else if ends_with(rest, &u("ふん")) || ends_with(rest, &u("ぷん")) {
                            let ms = &rest[..rest.len() - 2];
                            match parse_num1(ms).or_else(|| parse_num_full(ms).map(|(v, _)| v)).or(if ms == u("いっ") { Some(1) } else { None }) {
                                Some(v) if v < 60 => mi = v,
                                _ => ok = false,
                            }
                        } else {
                            ok = false;
                        }
                    }
                    if ok && (0..=24).contains(&h) && (ampm != 0 || !rest.is_empty() || hs.len() >= 2) {
                        let mut hh = h;
                        if ampm == 2 && hh < 12 {
                            hh += 12;
                        }
                        let mut a = num(hh);
                        a.push(':');
                        a.extend(two(mi));
                        out.push(a);
                        let mut a = num(hh);
                        a.push('時');
                        if mi != 0 {
                            a.extend(num(mi));
                            a.push('分');
                        }
                        out.push(a);
                        if ampm != 0 {
                            let mut a = u(if ampm == 1 { "午前" } else { "午後" });
                            a.extend(num(h));
                            a.push('時');
                            if mi != 0 {
                                a.extend(num(mi));
                                a.push('分');
                            }
                            out.push(a);
                        }
                        return out;
                    }
                }
            }
        }
    }
    out
}

// ------------------------------------------------------------------ convert
const CASCADE_PER_CHAR: f64 = 300.0;
const CASCADE_K: i32 = 3;
const SHORT_READING: i32 = 3;
const SHORT_KEEP_MODEL: i32 = 3;
const CASCADE_MAX_LEN: i32 = 16;

fn put_at(res: &mut Vec<Ustr>, c: &Ustr, rank: usize, maxout: i32) {
    if let Some(pos) = res.iter().position(|x| x == c) {
        if pos <= rank {
            return;
        }
        res.remove(pos);
    }
    let at = rank.min(res.len());
    res.insert(at, c.clone());
    if res.len() > maxout.max(0) as usize {
        res.truncate(maxout.max(0) as usize);
    }
}

fn convert_impl(e: &mut kkc_engine, ctx16: &[u16], kana16: &[u16], maxout: i32, use_model_in: i32) -> Result<Vec<Ustr>, i32> {
    let mut use_model = use_model_in;
    let t_start = Instant::now();
    let r = from16(kana16);
    let n = r.len() as i32;
    // one-shot keys / alts / right-ctx
    let mut keys: Vec<u16> = Vec::new();
    let mut span_end: Vec<i32> = Vec::new();
    if e.lex.latin && e.span_end.len() as i32 == n {
        keys = std::mem::take(&mut e.keys);
        span_end = std::mem::take(&mut e.span_end);
    }
    e.keys.clear();
    e.span_end.clear();
    let has_alts = e.alts_n == n;
    let alts: Vec<Ustr> = if has_alts { std::mem::take(&mut e.alts) } else { Vec::new() };
    e.alts.clear();
    e.alts_n = -1;
    let rctx = std::mem::take(&mut e.rctx);
    let ctx = from16(ctx16);
    let mut ed = edges(&e.lex, &r, Some(&e.user), if has_alts { Some(&alts) } else { None });
    kata_continuation_edges(&e.lex, &ctx, &r, &mut ed);
    kanji_continuation_edges(&mut e.lex, &ctx, &r, &mut ed);
    if !keys.is_empty() {
        let mut lat: Vec<Edge> = Vec::new();
        let latin_ch = |c: u16| c == b' ' as u16 || (c < 0x80 && (c as u8).is_ascii_alphabetic());
        let left_latin = !ctx16.is_empty() && latin_ch(ctx16[ctx16.len() - 1]);
        let right_latin = !rctx.is_empty() && latin_ch(rctx[0]);
        if e.latin {
            latin_edges(&e.lex, &keys, &span_end, n, &r, &e.user_latin, &mut lat, left_latin, right_latin, e.latin_mid2);
        }
        if e.lex.typo {
            let tctx = build_typo_ctx(&e.lex, &ed, n);
            let mut ty: Vec<Edge> = Vec::new();
            let w_sz = 6i32;
            let mut i = 0i32;
            while i < n {
                if is_kana(r[i as usize]) {
                    i += 1;
                    continue;
                }
                let mut j = i;
                while j < n && !is_kana(r[j as usize]) {
                    j += 1;
                }
                let w = (i - w_sz).max(0);
                let we = (j + w_sz).min(n);
                let ks = if w == 0 { 0 } else { span_end[(w - 1) as usize] };
                let ke = if we == n { keys.len() as i32 } else { span_end[(we - 1) as usize] };
                let mut rom = String::new();
                let mut ok = ke - ks >= 2;
                for k in ks..ke {
                    let mut c = keys[k as usize];
                    if (b'A' as u16..=b'Z' as u16).contains(&c) {
                        c += 32;
                    }
                    if (b'a' as u16..=b'z' as u16).contains(&c) || c == b'-' as u16 {
                        rom.push(c as u8 as char);
                    } else {
                        ok = false;
                        break;
                    }
                }
                if has_alts {
                    let pa = prep_alts(&r, &alts);
                    let sa = window_alts(&pa, n, w, we);
                    if we - w >= 2 {
                        typo_add_edges(&e.lex, &r[w as usize..we as usize], w, &ed, &tctx, &mut ty, if ok { Some(&rom) } else { None }, Some(&sa));
                    }
                } else if ok {
                    typo_add_edges(&e.lex, &r[w as usize..we as usize], w, &ed, &tctx, &mut ty, Some(&rom), None);
                }
                i = j;
            }
            lat.extend(ty);
        }
        ed.extend(lat);
    }
    if e.m.n_user > 0 || e.m.n_style > 0 {
        personalize(e, &mut ed);
    }
    e.times[0] = t_start.elapsed().as_secs_f64() * 1000.0;
    let mut res: Vec<Ustr> = Vec::new();
    e.suggest.clear();
    e.typo_spans.clear();
    e.last_u.clear();
    e.last_segs.clear();
    e.ro.e.clear();
    e.last_paths.clear();
    let mut done = false;
    if use_model == 2 && n >= CASCADE_MAX_LEN {
        use_model = 1;
    }
    if use_model == 0 || n == 0 || use_model == 2 {
        for i in 1..5 {
            e.times[i] = 0.0;
        }
        let t0 = Instant::now();
        let k0 = if use_model == 2 && n > 0 { CASCADE_K } else { 10 };
        let edf: Vec<Edge> = ed.iter().filter(|x| x.kind != K_TYPO).cloned().collect();
        let strip = edf.len() != ed.len();
        let use_ed = if strip { &edf } else { &ed };
        let nb = nbest(&e.lex, use_ed, n, None, 1.0, 1.0, k0, if k0 == 10 { maxout } else { CASCADE_K }, true);
        res = nb.res;
        e.last_segs = nb.segs;
        e.last_paths = nb.paths;
        e.last_cost = nb.top2;
        done = use_model != 2 || n == 0 || (e.last_cost[1] - e.last_cost[0]) >= CASCADE_PER_CHAR * n as f64;
        if done && use_model == 2 {
            for c in &res {
                if has_latin_word(c) {
                    done = false;
                    break;
                }
            }
        }
        if done && k0 != 10 {
            let nb = nbest(&e.lex, use_ed, n, None, 1.0, 1.0, 10, maxout, true);
            res = nb.res;
            e.last_segs = nb.segs;
            e.last_paths = nb.paths;
            e.last_cost = nb.top2;
        }
        e.times[4] = t0.elapsed().as_secs_f64() * 1000.0;
    }
    if !done {
        let t0 = Instant::now();
        let mut c = ctx.clone();
        if c.len() as i32 > e.m.ctx {
            c = c[c.len() - e.m.ctx as usize..].to_vec();
        }
        let mut tok: Vec<i32> = Vec::new();
        let mut seg: Vec<i32> = Vec::new();
        for &ch in &c {
            tok.push(e.m.vid(ch));
            seg.push(0);
        }
        tok.push(2); // SEP
        seg.push(0);
        let rstart = tok.len() as i32;
        for &ch in &r {
            tok.push(e.m.vid(ch));
            seg.push(1);
        }
        if e.m.draft != 0 {
            let dr0 = best_path(&e.lex, &ed, n);
            let room = e.m.max_len - tok.len() as i32 - 1;
            let lim = e.m.draft_max.min(room.max(0));
            let dr: Ustr = dr0[..dr0.len().min(lim.max(0) as usize)].to_vec();
            if room > 0 {
                tok.push(2);
                seg.push(2);
                for &ch in &dr {
                    tok.push(e.m.vid(ch));
                    seg.push(2);
                }
            }
        }
        if e.m.n_seg > 3 && !rctx.is_empty() {
            let room = e.m.max_len - tok.len() as i32 - 1;
            if room > 0 {
                let rc = from16(&rctx[..rctx.len().min(room as usize)]);
                tok.push(2);
                seg.push(3);
                for &ch in &rc {
                    tok.push(e.m.vid(ch));
                    seg.push(3);
                }
            }
        }
        if tok.len() as i32 > e.m.max_len {
            return Err(-3);
        }
        e.times[1] = t0.elapsed().as_secs_f64() * 1000.0;
        let t0 = Instant::now();
        encode(e, &tok, &seg);
        e.times[2] = t0.elapsed().as_secs_f64() * 1000.0;
        let t0 = Instant::now();
        e.last_u = scores(e, rstart, n, &ed);
        if e.typo_bias != 0.0 {
            for (i, x) in ed.iter().enumerate() {
                if x.kind == K_TYPO {
                    e.last_u[i] -= e.typo_bias;
                }
            }
        }
        if e.latin_promote > 0.0 {
            let pen = (e.latin_promote * n.clamp(1, 12) as f64 / 1000.0) as f32;
            for (i, x) in ed.iter().enumerate() {
                if x.kind == K_LATIN {
                    e.last_u[i] -= pen;
                }
            }
        }
        if e.ro.on {
            e.ro.e = ed.clone();
            e.ro.n = n;
        }
        e.times[3] = t0.elapsed().as_secs_f64() * 1000.0;
        let t0 = Instant::now();
        let has_typo = ed.iter().any(|x| x.kind == K_TYPO);
        if e.suggest_mode && has_typo {
            let nb = nbest(&e.lex, &ed, n, Some(&e.last_u), e.m.beta as f64, e.m.gamma as f64, 1, 1, true);
            let ty = nb.res;
            let tsegs = nb.segs;
            if !ty.is_empty() {
                let mut ci = 0usize;
                let mut s0 = 0i32;
                for sg in &tsegs {
                    let mut sf: Ustr = Vec::new();
                    let mut u16n = 0;
                    while ci < ty[0].len() && u16n < sg.l16 {
                        sf.push(ty[0][ci]);
                        u16n += if (ty[0][ci] as u32) > 0xFFFF { 2 } else { 1 };
                        ci += 1;
                    }
                    for x in &ed {
                        if x.kind == K_TYPO && x.s == s0 && x.e == sg.e && x.surf == sf {
                            e.typo_spans.push(s0);
                            e.typo_spans.push(sg.e);
                            break;
                        }
                    }
                    s0 = sg.e;
                }
            }
            let mut lit = e.last_u.clone();
            for (i, x) in ed.iter().enumerate() {
                if x.kind == K_TYPO {
                    lit[i] = -1e4;
                }
            }
            let nb = nbest(&e.lex, &ed, n, Some(&lit), e.m.beta as f64, e.m.gamma as f64, 10, maxout, true);
            res = nb.res;
            e.last_segs = nb.segs;
            e.last_paths = nb.paths;
            e.last_cost = nb.top2;
            e.typo_margin = (e.last_cost[0] - nb.top2[0]) / n.clamp(1, 12) as f64;
            // NOTE: C++ computes typo_margin against the typo-path cost (tyc);
            // nb.top2 here is the literal path's, so recompute properly below.
            let tyc0 = {
                let nb2 = nbest(&e.lex, &ed, n, Some(&e.last_u), e.m.beta as f64, e.m.gamma as f64, 1, 1, false);
                nb2.top2[0]
            };
            e.typo_margin = (e.last_cost[0] - tyc0) / n.clamp(1, 12) as f64;
            if !ty.is_empty() && !res.is_empty() && ty[0] != res[0] {
                if e.typo_margin >= e.typo_promote {
                    res.retain(|x| *x != ty[0]);
                    res.insert(0, ty[0].clone());
                    e.suggest.push((8, ty[0].clone())); // KKC_SUGGEST_TYPO_TOP
                } else {
                    e.suggest.push((1, ty[0].clone())); // KKC_SUGGEST_TYPO
                }
            }
        } else {
            let nb = nbest(&e.lex, &ed, n, Some(&e.last_u), e.m.beta as f64, e.m.gamma as f64, 10, maxout, true);
            res = nb.res;
            e.last_segs = nb.segs;
            e.last_paths = nb.paths;
            e.last_cost = nb.top2;
        }
        if !res.is_empty() {
            let ar = arabicize(&r, &res[0], &e.last_segs);
            let arc = with_commas_in(if ar.is_empty() { &res[0] } else { &ar });
            if !ar.is_empty() {
                put_at(&mut res, &ar, 0, maxout);
            }
            let kj = kanjize(if res.is_empty() { &[] } else { &res[0] });
            if !kj.is_empty() && !res[..res.len().min(3)].contains(&kj) {
                let rank = res.len().min(2); put_at(&mut res, &kj, rank, maxout);
            }
            if !arc.is_empty() && !res[..res.len().min(3)].contains(&arc) {
                put_at(&mut res, &arc, 2, maxout);
            }
            if res.len() > maxout.max(0) as usize {
                res.truncate(maxout.max(0) as usize);
            }
        }
        if n <= SHORT_READING && !res.is_empty() {
            let mut whole: Vec<(i32, Ustr)> = Vec::new();
            for x in &ed {
                if x.s == 0 && x.e == n {
                    whole.push((e.lex.conn(0, x.lid) + x.cost + e.lex.conn(x.rid, 0), x.surf.clone()));
                }
            }
            whole.sort_by(|a, b| a.0.cmp(&b.0));
            let dict = nbest(&e.lex, &ed, n, None, 1.0, 1.0, 10, maxout, false).res;
            let mut merged: Vec<Ustr> = Vec::new();
            let mut seen: HashSet<Ustr> = HashSet::new();
            let mut add = |merged: &mut Vec<Ustr>, seen: &mut HashSet<Ustr>, s: &Ustr| {
                if (merged.len() as i32) < maxout && seen.insert(s.clone()) {
                    merged.push(s.clone());
                }
            };
            for i in 0..res.len().min(SHORT_KEEP_MODEL as usize) {
                let s = res[i].clone();
                add(&mut merged, &mut seen, &s);
            }
            for w in &whole {
                add(&mut merged, &mut seen, &w.1);
            }
            for s in res.clone() {
                add(&mut merged, &mut seen, &s);
            }
            for s in &dict {
                add(&mut merged, &mut seen, s);
            }
            res = merged;
        }
        e.times[4] = t0.elapsed().as_secs_f64() * 1000.0;
    }
    // math expression / whole-number / english / emoji / kaomoji insertions
    {
        let mx = mathx::whole(&r);
        if !mx.is_empty() {
            put_at(&mut res, &mx, 1, maxout);
        }
        if use_model != 0 && n >= 3 {
            let (mut ne, mut v) = (0usize, 0i64);
            for b in boundaries(&r, 0) {
                if b >= 2 {
                    if let Some((x, false)) = parse_num_full(&r[..b]) {
                        ne = b;
                        v = x;
                    }
                }
            }
            if ne > 0 && v >= 100 && n as usize - ne <= 3 {
                let dict = nbest(&e.lex, &ed, n, None, 1.0, 1.0, 10, 1, false).res;
                if !dict.is_empty() && !dict[0].is_empty() && ('1'..='9').contains(&dict[0][0]) {
                    let d0 = dict[0].clone();
                    put_at(&mut res, &d0, 0, maxout);
                    let dc = with_commas_in(&d0);
                    if !dc.is_empty() {
                        put_at(&mut res, &dc, 2, maxout);
                    }
                }
            }
        }
        if let Some((a, b)) = e.lex.find(kana16) {
            for j in a..b {
                let en = e.lex.ent(j);
                if en.cost < ENG_COST {
                    continue;
                }
                let sf = e.lex.surf(&en);
                if e.suggest_mode {
                    e.suggest.push((4, sf)); // KKC_SUGGEST_ENGLISH
                } else {
                    put_at(&mut res, &sf, 3, maxout);
                }
                if j - a >= 1 {
                    break;
                }
            }
        }
        for (k, s) in mathx::SYMBOLS {
            if r == u(k) {
                put_at(&mut res, &u(s), 4, maxout);
            }
        }
        // emoji (cost 8000s): up to 2 at rank 4/5
        if let Some((a, b)) = e.lex.find(kana16) {
            let mut em: Vec<(i32, Ustr)> = Vec::new();
            for j in a..b {
                let en = e.lex.ent(j);
                if en.cost < 8000 || en.cost >= 9000 {
                    continue;
                }
                let sf = e.lex.surf(&en);
                if is_emoji(&sf) {
                    em.push((en.cost, sf));
                }
            }
            em.sort_by(|a, b| a.0.cmp(&b.0));
            let bare = |t: &[char]| -> Ustr { t.iter().filter(|&&c| c != '\u{FE0F}').cloned().collect() };
            let mut pick: Vec<Ustr> = Vec::new();
            for x in &em {
                let k = bare(&x.1);
                if pick.iter().any(|p| bare(p) == k) {
                    continue;
                }
                let mut best = x.1.clone();
                for y in &em {
                    if bare(&y.1) == k && y.1.len() > best.len() {
                        best = y.1.clone();
                    }
                }
                pick.push(best);
                if pick.len() == 2 {
                    break;
                }
            }
            for i in (0..pick.len()).rev() {
                let p = pick[i].clone();
                put_at(&mut res, &p, 3, maxout);
            }
        }
        // kaomoji (cost 9300..9399)
        if let Some((a, b)) = e.lex.find(kana16) {
            let mut km: Vec<(i32, Ustr)> = Vec::new();
            for j in a..b {
                let en = e.lex.ent(j);
                if en.cost < 9300 || en.cost > 9399 {
                    continue;
                }
                let sf = e.lex.surf(&en);
                if sf.iter().all(|&c| !is_kana(c) && !(0x4E00..=0x9FFF).contains(&(c as u32))) {
                    km.push((en.cost, sf));
                }
            }
            km.sort_by(|a, b| a.0.cmp(&b.0));
            if r == u("かおもじ") {
                let m = km.len().min(6);
                for i in (0..m).rev() {
                    let s = km[i].1.clone();
                    put_at(&mut res, &s, 1, maxout);
                }
            } else if !km.is_empty() {
                let s = km[0].1.clone();
                put_at(&mut res, &s, 5, maxout);
            }
        }
    }
    // whole-reading abbreviation at rank <= 1
    if n >= 2 * ABBR_MIN as i32 && e.lex.abbr_lid >= 0 {
        let ab = whole_abbr(&r);
        if !ab.is_empty() {
            put_at(&mut res, &ab, 1, maxout);
        }
    }
    // zip code -> address
    {
        let mut z: Ustr = Vec::new();
        let mut ok = true;
        for &c in &r {
            if c.is_ascii_digit() {
                z.push(c);
            } else if !(z.len() == 3 && (c == '-' || c == 'ー' || c == '−')) {
                z.clear();
                ok = false;
                break;
            }
        }
        let _ = ok;
        let mut addr: Vec<(i32, Ustr)> = Vec::new();
        if z.len() == 7 && (r.len() == 7 || r.len() == 8) {
            let k16: Vec<u16> = z.iter().map(|&c| c as u16).collect();
            if let Some((a, b)) = e.lex.find(&k16) {
                for j in a..b {
                    let en = e.lex.ent(j);
                    addr.push((en.cost, e.lex.surf(&en)));
                }
            }
            addr.sort_by(|a, b| a.0.cmp(&b.0));
        }
        if !addr.is_empty() && e.suggest_mode {
            for x in addr.iter().take(3) {
                e.suggest.push((3, x.1.clone())); // KKC_SUGGEST_ADDRESS
            }
            addr.clear();
        }
        if !addr.is_empty() {
            let mut hy: Ustr = z[..3].to_vec();
            hy.push('-');
            hy.extend(z[3..].iter());
            let mut add: Vec<Ustr> = addr.iter().map(|x| x.1.clone()).collect();
            add.push(hy.clone());
            let mut mark = u("〒");
            mark.extend(hy.iter());
            add.push(mark);
            let mut at = res.len().min(1);
            for c in &add {
                if let Some(pos) = res.iter().position(|x| x == c) {
                    if pos < at {
                        continue;
                    }
                    res.remove(pos);
                }
                let ins = at.min(res.len());
                res.insert(ins, c.clone());
                at += 1;
            }
            if res.len() > maxout.max(0) as usize {
                res.truncate(maxout.max(0) as usize);
            }
        }
    }
    // lowercase latin typed: restore dictionary casing in the top candidate
    if !res.is_empty() {
        let mut top = res[0].clone();
        let low = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit();
        let mut i0 = 0i32;
        while i0 < n {
            if !low(r[i0 as usize]) {
                i0 += 1;
                continue;
            }
            let mut i1 = i0;
            let mut letter = false;
            while i1 < n && low(r[i1 as usize]) {
                letter = letter || r[i1 as usize].is_ascii_lowercase();
                i1 += 1;
            }
            if letter && i1 - i0 >= 2 && (i1 == n || !r[i1 as usize].is_ascii_uppercase()) {
                let k16: Vec<u16> = r[i0 as usize..i1 as usize].iter().map(|&c| c as u16).collect();
                let (mut bc, mut best) = (5001i32, Vec::new());
                if let Some((a, b)) = e.lex.find(&k16) {
                    for j in a..b {
                        let en = e.lex.ent(j);
                        if en.cost < bc {
                            bc = en.cost;
                            best = e.lex.surf(&en);
                        }
                    }
                }
                let typed = &r[i0 as usize..i1 as usize];
                let alpha = |c: char| c.is_ascii_alphanumeric();
                let mut at = top.windows(typed.len()).position(|w| w == typed);
                while let Some(p) = at {
                    if (p > 0 && alpha(top[p - 1])) || (p + typed.len() < top.len() && alpha(top[p + typed.len()])) {
                        at = top[p + 1..].windows(typed.len()).position(|w| w == typed).map(|q| q + p + 1);
                    } else {
                        break;
                    }
                }
                if !best.is_empty() {
                    if let Some(p) = at {
                        let mut nt: Ustr = top[..p].to_vec();
                        nt.extend(best.iter());
                        nt.extend(top[p + typed.len()..].iter());
                        top = nt;
                    }
                }
            }
            i0 = i1;
        }
        if top != res[0] {
            res.retain(|x| *x != top);
            res.insert(0, top);
            if res.len() > maxout.max(0) as usize {
                res.truncate(maxout.max(0) as usize);
            }
        }
    }
    // whole reading is latin letters: append typed/capitalized/upper/fullwidth forms
    if n >= 1
        && r.iter().all(|&c| c.is_ascii_alphanumeric())
        && r.iter().any(|&c| c.is_ascii_alphabetic())
    {
        let lo: Ustr = r.iter().map(|&c| if c.is_ascii_uppercase() { (c as u32 + 32) as u8 as char } else { c }).collect();
        let up: Ustr = r.iter().map(|&c| if c.is_ascii_lowercase() { (c as u32 - 32) as u8 as char } else { c }).collect();
        let mut cap = lo.clone();
        if !cap.is_empty() && cap[0].is_ascii_lowercase() {
            cap[0] = (cap[0] as u32 - 32) as u8 as char;
        }
        let fw: Ustr = r.iter().map(|&c| char::from_u32(c as u32 + 0xFEE0).unwrap()).collect();
        for c in [r.clone(), lo, cap, up, fw] {
            if !res.contains(&c) {
                res.push(c);
            }
        }
        if res.len() > maxout.max(0) as usize {
            res.truncate(maxout.max(0) as usize);
        }
    }
    // whole reading is digits: typed form first, then comma/unit/kanji/fullwidth
    if n > 0 && n <= 16 && r.iter().all(|&c| c.is_ascii_digit()) && (r[0] != '0' || n == 1) {
        let mut v: i64 = 0;
        for &c in &r {
            v = v * 10 + (c as i64 - '0' as i64);
        }
        let mut add: Vec<Ustr> = vec![r.clone()];
        if v >= 1000 {
            add.push(with_commas(&r));
        }
        if v >= 10000 {
            add.push(unit_mixed(v, false));
            add.push(unit_mixed(v, true));
        }
        add.push(kanji_num(v));
        add.push(digits_str(v, true));
        let mut at = 0usize;
        for c in &add {
            if let Some(pos) = res.iter().position(|x| x == c) {
                if pos < at {
                    continue;
                }
                res.remove(pos);
            }
            let ins = at.min(res.len());
            res.insert(ins, c.clone());
            at += 1;
        }
        if res.len() > maxout.max(0) as usize {
            res.truncate(maxout.max(0) as usize);
        }
    }
    // date/time candidates from rank 3 (or suggestions)
    {
        let mut dc = date_candidates(&r);
        if e.suggest_mode {
            for c in &dc {
                e.suggest.push((2, c.clone())); // KKC_SUGGEST_DATE
            }
            dc.clear();
        }
        let mut at = res.len().min(2);
        for c in &dc {
            if let Some(pos) = res.iter().position(|x| x == c) {
                res.remove(pos);
            }
            let ins = at.min(res.len());
            res.insert(ins, c.clone());
            at += 1;
        }
        if res.len() > maxout.max(0) as usize {
            res.truncate(maxout.max(0) as usize);
        }
    }
    // char filter (tofu)
    {
        let cf = e.char_filter;
        let tofu = |c: &[char]| -> bool {
            c.iter().any(|&ch| {
                let v = ch as u32;
                ((cf & 1) != 0 && (0x1B000..=0x1B16F).contains(&v))
                    || ((cf & 2) != 0 && (0x13000..=0x1345F).contains(&v))
                    || ((cf & 4) != 0 && (0x1F000..=0x1F0FF).contains(&v) && v != 0x1F004 && v != 0x1F0CF)
            })
        };
        res.retain(|c| !tofu(c));
        e.suggest.retain(|x| !tofu(&x.1));
    }
    // suggest mode: move purely symbolic candidates to suggestions
    if e.suggest_mode && r.iter().any(|&c| (0x3041..=0x30FA).contains(&(c as u32))) {
        let unit_marks = u("℃°%％‰℉Ω㎏㎞㎝㎜㎡㎥¥￥$＄€£");
        let symbolic = |c: &[char]| -> bool {
            if !c.is_empty() && c.iter().all(|&ch| unit_marks.contains(&ch)) {
                return false;
            }
            let mut letters = 0usize;
            for &ch in c {
                let v = ch as u32;
                if (0x3041..=0x3096).contains(&v) || (0x30A1..=0x30FA).contains(&v) || (0x4E00..=0x9FFF).contains(&v)
                    || ch.is_ascii_digit() || (0xFF10..=0xFF19).contains(&v)
                {
                    return false;
                }
                if ch.is_ascii_alphabetic() {
                    letters += 1;
                }
            }
            !c.is_empty() && letters * 2 <= c.len()
        };
        let orig = res.clone();
        let (mut keep, mut moved) = (Vec::new(), Vec::new());
        for c in orig {
            if symbolic(&c) {
                moved.push(c);
            } else {
                keep.push(c);
            }
        }
        if !moved.is_empty() && !keep.is_empty() {
            res = keep;
            for c in moved {
                e.suggest.push((7, c)); // KKC_SUGGEST_SYMBOL
            }
        }
    }
    // numeric + 度 -> ℃ variant right after
    for i in 0..res.len().min(3) {
        let c = res[i].clone();
        let Some(p) = c.iter().position(|&x| x == '度') else { continue };
        if p == 0 || !(c[p - 1].is_ascii_digit() || (0xFF10..=0xFF19).contains(&(c[p - 1] as u32))) {
            continue;
        }
        let mut v = c;
        v[p] = '℃';
        if !res.contains(&v) {
            res.insert(i + 1, v);
        }
        break;
    }
    // AFTER suggestions (emoji/kaomoji for words of the top candidate)
    if e.suggest_mode && !e.last_segs.is_empty() && !res.is_empty() {
        let top0 = res[0].clone();
        let mut spans: Vec<(i32, i32)> = Vec::new();
        let mut surfs: Vec<(usize, usize)> = Vec::new();
        let (mut rs, mut ss) = (0i32, 0usize);
        for sg in &e.last_segs {
            let mut se = ss;
            let mut l = 0;
            while se < top0.len() && l < sg.l16 {
                l += if (top0[se] as u32) >= 0x10000 { 2 } else { 1 };
                se += 1;
            }
            spans.push((rs, sg.e));
            surfs.push((ss, se));
            rs = sg.e;
            ss = se;
        }
        let (mut em, mut km): (Vec<Ustr>, Vec<Ustr>) = (Vec::new(), Vec::new());
        for k in (0..spans.len()).rev() {
            if em.len() >= 8 && km.len() >= 6 {
                break;
            }
            let (a0, b0) = spans[k];
            let sf = &top0[surfs[k].0..surfs[k].1];
            let content = sf.iter().any(|&c| (0x4E00..=0x9FFF).contains(&(c as u32)) || (0x30A1..=0x30FA).contains(&(c as u32)));
            if !content || b0 - a0 < 2 {
                continue;
            }
            let k16: Vec<u16> = r[a0 as usize..b0 as usize].iter().map(|&c| c as u16).collect();
            let Some((la, lb)) = e.lex.find(&k16) else { continue };
            let (mut ce, mut ck): (Vec<(i32, Ustr)>, Vec<(i32, Ustr)>) = (Vec::new(), Vec::new());
            for j in la..lb {
                let en = e.lex.ent(j);
                let x = e.lex.surf(&en);
                if en.cost >= 8000 && en.cost < 9000 && is_emoji(&x) {
                    ce.push((en.cost, x));
                } else if en.cost >= 9300 && en.cost <= 9399 && b0 - a0 >= 3
                    && x.iter().all(|&c| !is_kana(c) && !(0x4E00..=0x9FFF).contains(&(c as u32)))
                {
                    ck.push((en.cost, x));
                }
            }
            ce.sort_by(|a, b| a.0.cmp(&b.0));
            ck.sort_by(|a, b| a.0.cmp(&b.0));
            for x in ce {
                if em.len() < 8 && !em.contains(&x.1) {
                    em.push(x.1);
                }
            }
            for x in ck {
                if km.len() < 6 && !km.contains(&x.1) {
                    km.push(x.1);
                }
            }
        }
        for x in em {
            e.suggest.push((9, x)); // KKC_SUGGEST_AFTER
        }
        for x in km {
            e.suggest.push((9, x));
        }
    }
    // suggestion ordering: typo, date, address, mood emoji, symbol, english...
    {
        let pri = |k: i32| -> i32 {
            const P: [i32; 10] = [9, 0, 1, 2, 5, 9, 3, 4, 0, 9];
            if (0..10).contains(&k) { P[k as usize] } else { 9 }
        };
        e.suggest.sort_by(|a, b| pri(a.0).cmp(&pri(b.0))); // stable
    }
    Ok(res)
}

// ------------------------------------------------------------------ open
fn open_engine(lex_data: &[u8], model_data: &[u8]) -> Option<Box<kkc_engine>> {
    let mut lex = load_lex(lex_data)?;
    let m = load_model(model_data)?;
    let mut model_tag: u32 = 2166136261;
    {
        let mut mix = |c: u8| {
            model_tag ^= c as u32;
            model_tag = model_tag.wrapping_mul(16777619);
        };
        for k in 0..8 {
            mix((model_data.len() as u64 >> (8 * k)) as u8);
        }
        for k in 0..model_data.len().min(4096) {
            mix(model_data[k]);
        }
        let mut k = 4096;
        while k < model_data.len() {
            mix(model_data[k]);
            k += 1021;
        }
    }
    lex.typo = m.n_kind > K_TYPO as i32;
    if lex.typo {
        lex.build_bloom();
    }
    lex.latin = m.n_kind > K_LATIN as i32;
    if lex.latin {
        let mut words: HashMap<String, LatinWord> = HashMap::new();
        for j in 0..lex.nent as u32 {
            let en = lex.ent(j);
            if en.slen < 2 {
                continue;
            }
            let s = &lex.s_blob[en.soff as usize..en.soff as usize + en.slen as usize];
            let mut lower = String::new();
            let mut ok = true;
            for &c in s {
                if (b'a' as u16..=b'z' as u16).contains(&c) || (b'0' as u16..=b'9' as u16).contains(&c) {
                    lower.push(c as u8 as char);
                } else if (b'A' as u16..=b'Z' as u16).contains(&c) {
                    lower.push((c + 32) as u8 as char);
                } else {
                    ok = false;
                    break;
                }
            }
            if !ok {
                continue;
            }
            let cost = if en.cost >= ENG_COST { ENG_LATIN_COST } else { en.cost };
            let w = LatinWord { surf: from16(s), lid: en.lid as i32, rid: en.rid as i32, cost };
            match words.get(&lower) {
                Some(old) if old.cost <= cost => {}
                _ => {
                    words.insert(lower, w);
                }
            }
        }
        lex.latin_words = words;
    }
    {
        let k: Vec<u16> = u("じーでぃーぴー").iter().map(|&c| c as u16).collect();
        if let Some((a, b)) = lex.find(&k) {
            for j in a..b {
                let en = lex.ent(j);
                if en.slen == 3 && lex.s_blob[en.soff as usize] == b'G' as u16 {
                    lex.abbr_lid = en.lid as i32;
                    lex.abbr_rid = en.rid as i32;
                    break;
                }
            }
        }
    }
    Some(Box::new(kkc_engine {
        lex,
        m,
        ro: Romance { h: 32, cap: 2.0, lr_v: 0.05, lr_w: 0.002, lr_net: 0.01, l2: 0.001, ..Default::default() },
        own_lex: Vec::new(),
        own_model: Vec::new(),
        model_tag,
        threads: 1,
        times: [0.0; 5],
        typo_bias: 0.0,
        typo_promote: 200.0,
        latin_promote: 0.0,
        typo_margin: 0.0,
        suggest_mode: false,
        latin_mid2: false,
        latin: true,
        char_filter: 7,
        suggest: Vec::new(),
        typo_spans: Vec::new(),
        kcache: HashMap::new(),
        kstore: Vec::new(),
        last_u: Vec::new(),
        last_segs: Vec::new(),
        last_paths: Vec::new(),
        last_cost: [0.0, 1e18],
        user: UserList::default(),
        var: HashMap::new(),
        umarks: HashMap::new(),
        fam: HashMap::new(),
        cls_n: [[0; 2]; 8],
        user_latin: HashMap::new(),
        keys: Vec::new(),
        span_end: Vec::new(),
        alts: Vec::new(),
        alts_n: -1,
        rctx: Vec::new(),
        prow: [HashMap::new(), HashMap::new(), HashMap::new()],
        h_out: Vec::new(),
    }))
}

// ------------------------------------------------------------------ user dictionary
fn user_add_like_impl(
    e: &mut kkc_engine, r16: &[u16], s16: &[u16], tr16: &[u16], ts16: &[u16], bonus: i32,
) -> i32 {
    if r16.is_empty() || s16.is_empty() {
        return 0;
    }
    let Some((a, b)) = e.lex.find(tr16) else { return 0 };
    let want = from16(ts16);
    let mut best: Option<Ent> = None;
    for j in a..b {
        let en = e.lex.ent(j);
        if e.lex.surf(&en) == want && best.map_or(true, |bb| en.cost < bb.cost) {
            best = Some(en);
        }
    }
    let Some(best) = best else { return 0 };
    e.user.push(UserWord {
        r: from16(r16),
        s: from16(s16),
        lid: best.lid as i32,
        rid: best.rid as i32,
        cost: best.cost - bonus,
        whole: false,
    });
    {
        let mut lower = String::new();
        let mut latin = true;
        for &c in s16 {
            if (b'a' as u16..=b'z' as u16).contains(&c) || (b'0' as u16..=b'9' as u16).contains(&c) {
                lower.push(c as u8 as char);
            } else if (b'A' as u16..=b'Z' as u16).contains(&c) {
                lower.push((c + 32) as u8 as char);
            } else if c != b' ' as u16 {
                latin = false;
                break;
            }
        }
        if latin && lower.len() >= 2 {
            e.user_latin.insert(lower, LatinWord {
                surf: from16(s16),
                lid: best.lid as i32,
                rid: best.rid as i32,
                cost: best.cost - bonus,
            });
        }
    }
    1
}

// ------------------------------------------------------------------ path tracing (path_variants / path_pos)
// Finds the min-cost path through the lattice whose surfaces concatenate
// to `text`. Returns the edge indices.
fn trace_path(e: &kkc_engine, r: &[char], t: &[char], ed: &[Edge]) -> Option<Vec<usize>> {
    let n = r.len();
    let g = t.len();
    // state: (edge idx, text end pos) -> (cost, prev state)
    let mut by_start: Vec<Vec<usize>> = vec![Vec::new(); n + 1];
    for (i, x) in ed.iter().enumerate() {
        by_start[x.s as usize].push(i);
    }
    use std::collections::HashMap as Map;
    let mut dp: Map<(usize, usize), (f64, Option<(usize, usize)>)> = Map::new();
    let mut order: Vec<(usize, usize)> = Vec::new(); // (reading pos, text pos) frontier processing by reading pos
    for p in 0..n {
        // collect states ending at reading pos p from dp keys plus start
        let mut states: Vec<(Option<usize>, usize, f64)> = Vec::new(); // (edge, text pos, cost so far incl conn to next added later)
        if p == 0 {
            states.push((None, 0, 0.0));
        }
        for ((ei, te), (cost, _)) in dp.iter() {
            if ed[*ei].e as usize == p {
                states.push((Some(*ei), *te, *cost));
            }
        }
        for (pe, tp, cost) in states {
            for &i in &by_start[p] {
                let x = &ed[i];
                if x.surf.is_empty() || tp + x.surf.len() > g {
                    continue;
                }
                if t[tp..tp + x.surf.len()] != x.surf[..] {
                    continue;
                }
                let conn = match pe {
                    None => e.lex.conn(0, x.lid),
                    Some(pe) => e.lex.conn(ed[pe].rid, x.lid),
                };
                let nc = cost + conn as f64 + x.cost as f64;
                let key = (i, tp + x.surf.len());
                let prev = pe.map(|pe| (pe, tp));
                if dp.get(&key).map_or(true, |(c, _)| nc < *c) {
                    dp.insert(key, (nc, prev));
                    order.push(key);
                }
            }
        }
    }
    let mut best: Option<((usize, usize), f64)> = None;
    for ((ei, te), (cost, _)) in dp.iter() {
        if ed[*ei].e as usize == n && *te == g {
            let total = cost + e.lex.conn(ed[*ei].rid, 0) as f64;
            if best.map_or(true, |(_, c)| total < c) {
                best = Some(((*ei, *te), total));
            }
        }
    }
    let (mut key, _) = best?;
    let mut path = Vec::new();
    loop {
        path.push(key.0);
        match dp[&key].1 {
            Some(prev) => key = prev,
            None => break,
        }
    }
    path.reverse();
    Some(path)
}

// ------------------------------------------------------------------ completion
fn proper_noun_lid(lid: i32) -> bool {
    (1920..=1923).contains(&lid) || lid == 1929
}
const NG_PREDICT: &[(&str, &str)] = &[
    ("エッチ", "えっち"), ("えっち", "えっち"), ("セックス", "せっくす"), ("セックル", "せっくる"), ("オナ", "おな"),
    ("パイズリ", "ぱいずり"), ("フェラ", "ふぇら"), ("チンコ", "ちんこ"), ("ちんこ", "ちんこ"), ("チンポ", "ちんぽ"),
    ("ちんぽ", "ちんぽ"), ("マンコ", "まんこ"), ("まんこ", "まんこ"), ("勃起", "ぼっき"), ("射精", "しゃせい"),
    ("中出し", "なかだし"), ("手コキ", "てこき"), ("痴漢", "ちかん"), ("痴女", "ちじょ"), ("強姦", "ごうかん"),
    ("レイプ", "れいぷ"), ("乳首", "ちくび"), ("おっぱい", "おっぱい"), ("巨乳", "きょにゅう"), ("爆乳", "ばくにゅう"),
    ("貧乳", "ひんにゅう"), ("淫", "いん"), ("童貞", "どうてい"), ("処女", "しょじょ"), ("風俗", "ふうぞく"),
    ("ソープ嬢", "そーぷじょう"), ("デリヘル", "でりへる"), ("ラブホ", "らぶほ"), ("アダルト", "あだると"), ("エロ", "えろ"),
    ("ヌード", "ぬーど"), ("全裸", "ぜんら"), ("陰毛", "いんもう"), ("陰部", "いんぶ"), ("性器", "せいき"),
    ("性交", "せいこう"), ("売春", "ばいしゅん"), ("援交", "えんこう"), ("援助交際", "えんじょこうさい"), ("姦", "かん"),
    ("肉便器", "にくべんき"), ("精液", "せいえき"), ("愛液", "あいえき"), ("潮吹き", "しおふき"), ("クンニ", "くんに"),
    ("アナル", "あなる"), ("素股", "すまた"), ("乱交", "らんこう"), ("ビッチ", "びっち"), ("熟女", "じゅくじょ"),
    ("人妻", "ひとづま"), ("ハメ撮り", "はめどり"), ("夜這い", "よばい"), ("獣姦", "じゅうかん"), ("ロリコン", "ろりこん"),
    ("うんこ", "うんこ"), ("ウンコ", "うんこ"), ("死ね", "しね"), ("キチガイ", "きちがい"), ("気違い", "きちがい"),
    ("基地外", "きちがい"), ("ガイジ", "がいじ"), ("池沼", "ちしょう"), ("土人", "どじん"),
];
fn ng_predict(sf: &[char], pre: &[u16]) -> bool {
    let s: String = from16(&to16_owned(sf)).iter().collect();
    let p: String = from16(pre).iter().collect();
    for w in NG_PREDICT {
        if s.contains(w.0) && !p.contains(w.1) {
            return true;
        }
    }
    false
}
fn to16_owned(s: &[char]) -> Vec<u16> {
    let mut o = Vec::new();
    to16(s, &mut o);
    o
}
const KKC_FRAG: i32 = 4500;
fn complete_impl(e: &kkc_engine, prefix: &[u16], max_extra: i32, maxout: i32) -> Option<Vec<Ustr>> {
    let np = prefix.len() as i32;
    if np <= 0 {
        return Some(Vec::new());
    }
    let l = &e.lex;
    let lo = l.lower_bound(prefix);
    let mut ne_lid = 0i32;
    {
        let ne: [u16; 1] = [0x306D];
        if let Some((a, b)) = l.find(&ne) {
            let mut best = i32::MAX;
            for j in a..b {
                let en = l.ent(j);
                if en.slen == 1 && l.s_blob[en.soff as usize] == 0x306D && en.cost < best {
                    best = en.cost;
                    ne_lid = en.lid as i32;
                }
            }
        }
    }
    let mut found: Vec<(i32, u32)> = Vec::new();
    let mut rbuf = [0u16; MAXR];
    let mut scanned = 0;
    let mut i = lo;
    while i < l.nread && scanned < 50000 {
        let rn = if l.ver == 1 || i == lo {
            l.reading(i, &mut rbuf)
        } else {
            l.next_reading(i, &mut rbuf)
        };
        let rr = &rbuf[..rn];
        if rn < np as usize || &rr[..np as usize] != prefix {
            break;
        }
        if rn != np as usize && rn <= (np + max_extra) as usize {
            for j in l.e_first[i as usize]..l.e_first[i as usize + 1] {
                let en = l.ent(j);
                if en.cost < ENG_COST && l.conn(en.rid as i32, ne_lid) <= KKC_FRAG && !proper_noun_lid(en.lid as i32) {
                    found.push((en.cost, j));
                }
            }
        }
        i += 1;
        scanned += 1;
    }
    found.sort_by(|a, b| a.0.cmp(&b.0)); // stable
    let all: HashSet<Ustr> = found.iter().map(|c| l.surf(&l.ent(c.1))).collect();
    let mut outv: Vec<Ustr> = Vec::new();
    let mut seen: HashSet<Ustr> = HashSet::new();
    for c in &found {
        if outv.len() >= maxout.max(0) as usize {
            break;
        }
        let en = l.ent(c.1);
        let sf = l.surf(&en);
        if sf.is_empty() {
            continue;
        }
        let last = *sf.last().unwrap();
        if ['ゃ', 'ゅ', 'ょ', 'っ'].contains(&last) {
            continue;
        }
        if sf.len() >= 2 && (ends_with(&sf, &u("けれ")) || ends_with(&sf, &u("かろ"))) {
            continue;
        }
        if sf.len() >= 3 && ends_with(&sf, &u("しから")) {
            continue;
        }
        if en.lid != 2591 && en.lid != 2593 {
            let mut with_u = sf.clone();
            with_u.push('う');
            if all.contains(&with_u) {
                continue;
            }
        }
        if ng_predict(&sf, prefix) {
            continue;
        }
        if !seen.insert(sf.clone()) {
            continue;
        }
        outv.push(sf);
    }
    Some(outv)
}

// ------------------------------------------------------------------ romance learn
fn lse2(a: f64, b: f64) -> f64 {
    if a == f64::NEG_INFINITY {
        return b;
    }
    if b == f64::NEG_INFINITY {
        return a;
    }
    if a > b {
        a + (b - a).exp().ln_1p()
    } else {
        b + (a - b).exp().ln_1p()
    }
}
fn romance_learn_impl(e: &mut kkc_engine, text16: &[u16]) -> (i32, f32) {
    if !e.ro.on {
        return (-1, 0.0);
    }
    let ed = e.ro.e.clone();
    let n = e.ro.n;
    let m = ed.len();
    let dk = e.ro.dk;
    if m == 0 || e.last_u.len() != m || e.ro.q.len() != m * dk as usize {
        return (-1, 0.0);
    }
    let t = from16(text16);
    let nt = t.len() as i32;
    let (beta, gamma) = (e.m.beta as f64, e.m.gamma as f64);
    let lex = &e.lex;
    let tr = |rid: i32, lid: i32| -> f64 { -gamma * lex.conn(rid, lid) as f64 / 1000.0 };
    let mut w = vec![0f64; m];
    let mut by_start: Vec<Vec<usize>> = vec![Vec::new(); (n + 1) as usize];
    let mut by_end: Vec<Vec<usize>> = vec![Vec::new(); (n + 1) as usize];
    for i in 0..m {
        w[i] = e.last_u[i] as f64 - beta * ed[i].cost as f64 / 1000.0;
        by_start[ed[i].s as usize].push(i);
        by_end[ed[i].e as usize].push(i);
    }
    let ni = f64::NEG_INFINITY;
    let mut al = vec![ni; m];
    let mut be = vec![ni; m];
    for pos in 0..n {
        for &i in &by_start[pos as usize] {
            let mut a = ni;
            if pos == 0 {
                a = tr(0, ed[i].lid);
            } else {
                for &j in &by_end[pos as usize] {
                    if al[j] > ni {
                        a = lse2(a, al[j] + tr(ed[j].rid, ed[i].lid));
                    }
                }
            }
            if a > ni {
                al[i] = a + w[i];
            }
        }
    }
    for pos in (1..=n).rev() {
        for &i in &by_end[pos as usize] {
            let mut b = ni;
            if pos == n {
                b = tr(ed[i].rid, 0);
            } else {
                for &j in &by_start[pos as usize] {
                    if be[j] > ni {
                        b = lse2(b, tr(ed[i].rid, ed[j].lid) + w[j] + be[j]);
                    }
                }
            }
            be[i] = b;
        }
    }
    let mut logz = ni;
    for &i in &by_end[n as usize] {
        logz = lse2(logz, al[i] + be[i]);
    }
    if logz == ni {
        return (-1, 0.0);
    }
    // gold path marginals: state = (edge, text start)
    let mut ga: Vec<Vec<(i32, f64)>> = vec![Vec::new(); m];
    let mut gb: Vec<Vec<(i32, f64)>> = vec![Vec::new(); m];
    for pos in 0..n {
        for &i in &by_start[pos as usize] {
            let s = &ed[i].surf;
            if s.is_empty() {
                continue;
            }
            let mut starts: Vec<i32> = Vec::new();
            let mut from = 0usize;
            while from + s.len() <= t.len() {
                let Some(p) = (from..=t.len() - s.len()).find(|&p| t[p..p + s.len()] == s[..]) else { break };
                starts.push(p as i32);
                from = p + 1;
            }
            for tt in starts {
                let mut a = ni;
                if pos == 0 {
                    if tt == 0 {
                        a = tr(0, ed[i].lid);
                    }
                } else {
                    for &j in &by_end[pos as usize] {
                        for ja in &ga[j] {
                            if ja.0 + ed[j].surf.len() as i32 == tt {
                                a = lse2(a, ja.1 + tr(ed[j].rid, ed[i].lid));
                            }
                        }
                    }
                }
                if a > ni {
                    ga[i].push((tt, a + w[i]));
                }
            }
        }
    }
    for pos in (1..=n).rev() {
        for &i in &by_end[pos as usize] {
            for ia in ga[i].clone() {
                let tt = ia.0;
                let te = tt + ed[i].surf.len() as i32;
                let mut b = ni;
                if pos == n {
                    if te == nt {
                        b = tr(ed[i].rid, 0);
                    }
                } else {
                    for &j in &by_start[pos as usize] {
                        for jb in &gb[j] {
                            if jb.0 == te {
                                b = lse2(b, tr(ed[i].rid, ed[j].lid) + w[j] + jb.1);
                            }
                        }
                    }
                }
                if b > ni {
                    gb[i].push((tt, b));
                }
            }
        }
    }
    let gbv = |gb: &[Vec<(i32, f64)>], i: usize, tt: i32| -> f64 {
        gb[i].iter().find(|x| x.0 == tt).map(|x| x.1).unwrap_or(ni)
    };
    let mut logzg = ni;
    for &i in &by_end[n as usize] {
        for ia in &ga[i] {
            logzg = lse2(logzg, ia.1 + gbv(&gb, i, ia.0));
        }
    }
    if logzg == ni {
        return (0, 0.0);
    }
    let loss = (logz - logzg) as f32;
    let sc = 1.0 / (dk as f32).sqrt();
    let h = e.ro.h as usize;
    let dku = dk as usize;
    let mut gvec = vec![0f32; dku];
    let mut da = vec![0f32; e.ro.a.len()];
    let mut dab = vec![0f32; h];
    let mut dc = vec![0f32; h];
    let mut hid = vec![0f32; h];
    for i in 0..m {
        let mut mg = 0f64;
        for ia in &ga[i] {
            let b = gbv(&gb, i, ia.0);
            if b > ni {
                mg += (ia.1 + b - logzg).exp();
            }
        }
        let mm = if al[i] > ni && be[i] > ni { (al[i] + be[i] - logz).exp() } else { 0.0 };
        let g = (mm - mg) as f32;
        if g.abs() < 1e-6 {
            continue;
        }
        let q: Vec<f32> = e.ro.q[i * dku..(i + 1) * dku].to_vec();
        let k: Vec<f32> = e.ro.k[i * dku..(i + 1) * dku].to_vec();
        for j in 0..dku {
            gvec[j] += g * k[j] * sc;
        }
        e.ro.extra(&q, &k, sc, &mut hid);
        let mut o = 0f64;
        for hh in 0..h {
            if hid[hh] > 0.0 {
                o += e.ro.c[hh] as f64 * hid[hh] as f64;
            }
        }
        let th = (o / e.ro.cap as f64).tanh();
        let do_ = g * (1.0 - th * th) as f32;
        for hh in 0..h {
            if hid[hh] <= 0.0 {
                continue;
            }
            dc[hh] += do_ * hid[hh];
            let dp = do_ * e.ro.c[hh];
            if dp == 0.0 {
                continue;
            }
            dab[hh] += dp;
            for j in 0..dku {
                da[hh * dku + j] += dp * q[j] * k[j] * sc;
            }
        }
    }
    fn clip(x: &mut [f32]) {
        let s: f64 = x.iter().map(|&y| y as f64 * y as f64).sum();
        if s > 1.0 {
            let f = (1.0 / s.sqrt()) as f32;
            for y in x.iter_mut() {
                *y *= f;
            }
        }
    }
    let du = e.ro.du as usize;
    let mut dv = vec![0f32; du];
    let mut dw = vec![0f32; e.ro.w.len()];
    for i in 0..du {
        for j in 0..dku {
            dv[i] += e.ro.w[i * dku + j] * gvec[j];
            dw[i * dku + j] = e.ro.v[i] * gvec[j];
        }
    }
    clip(&mut dv);
    clip(&mut dw);
    clip(&mut da);
    clip(&mut dab);
    clip(&mut dc);
    let (lr_v, lr_w, lr_net, l2) = (e.ro.lr_v, e.ro.lr_w, e.ro.lr_net, e.ro.l2);
    for i in 0..du {
        e.ro.v[i] -= lr_v * (dv[i] + l2 * e.ro.v[i]);
    }
    for i in 0..e.ro.w.len() {
        let w0 = e.ro.w0[i];
        e.ro.w[i] -= lr_w * (dw[i] + l2 * (e.ro.w[i] - w0));
    }
    for i in 0..e.ro.a.len() {
        let a0 = e.ro.a0[i];
        e.ro.a[i] -= lr_net * (da[i] + l2 * (e.ro.a[i] - a0));
    }
    for hh in 0..h {
        e.ro.ab[hh] -= lr_net * (dab[hh] + l2 * e.ro.ab[hh]);
        e.ro.c[hh] -= lr_net * (dc[hh] + l2 * e.ro.c[hh]);
    }
    e.ro.updates += 1;
    e.ro.e.clear();
    (1, loss)
}

// ------------------------------------------------------------------ FFI (engine.h ABI)
// All kkc_* functions from engine.h except the glide family
// (kkc_glide_* / kkc_zh_candidates), which live in glide.rs by design.
pub mod ffi {
    use super::*;
    use std::os::raw::{c_char, c_void};

    type E = kkc_engine;

    unsafe fn eng<'a>(e: *mut E) -> Option<&'a mut E> {
        if e.is_null() {
            None
        } else {
            Some(&mut *e)
        }
    }
    unsafe fn sl<'a, T>(p: *const T, n: i32) -> &'a [T] {
        if p.is_null() || n <= 0 {
            &[]
        } else {
            std::slice::from_raw_parts(p, n as usize)
        }
    }
    unsafe fn slz<'a, T>(p: *const T, n: usize) -> &'a [T] {
        if p.is_null() {
            &[]
        } else {
            std::slice::from_raw_parts(p, n)
        }
    }
    fn write_cands(res: &[Ustr], out: *mut u16, cap: i32) -> i32 {
        let mut buf: Vec<u16> = Vec::new();
        for s in res {
            to16(s, &mut buf);
            buf.push(0);
        }
        if buf.len() as i32 > cap {
            return -1;
        }
        if !out.is_null() && !buf.is_empty() {
            unsafe { std::ptr::copy_nonoverlapping(buf.as_ptr(), out, buf.len()) };
        }
        res.len() as i32
    }

    #[no_mangle]
    pub extern "C" fn kkc_open(lex: *const c_void, lex_size: usize, model: *const c_void, model_size: usize) -> *mut E {
        if lex.is_null() || model.is_null() {
            return std::ptr::null_mut();
        }
        let l = unsafe { slz(lex as *const u8, lex_size) };
        let m = unsafe { slz(model as *const u8, model_size) };
        match open_engine(l, m) {
            Some(b) => Box::into_raw(b),
            None => std::ptr::null_mut(),
        }
    }

    #[no_mangle]
    pub extern "C" fn kkc_open_files(lex_path: *const c_char, model_path: *const c_char) -> *mut E {
        if lex_path.is_null() || model_path.is_null() {
            return std::ptr::null_mut();
        }
        let lp = unsafe { CStr::from_ptr(lex_path) }.to_string_lossy().into_owned();
        let mp = unsafe { CStr::from_ptr(model_path) }.to_string_lossy().into_owned();
        let (Ok(a), Ok(b)) = (std::fs::read(lp), std::fs::read(mp)) else {
            return std::ptr::null_mut();
        };
        match open_engine(&a, &b) {
            Some(mut bx) => {
                bx.own_lex = a;
                bx.own_model = b;
                Box::into_raw(bx)
            }
            None => std::ptr::null_mut(),
        }
    }

    #[no_mangle]
    pub extern "C" fn kkc_close(e: *mut E) {
        if !e.is_null() {
            unsafe { drop(Box::from_raw(e)) };
        }
    }

    #[no_mangle]
    pub extern "C" fn kkc_set_threads(e: *mut E, n: i32) {
        if let Some(e) = unsafe { eng(e) } {
            e.threads = n.max(1);
        }
    }

    #[no_mangle]
    pub extern "C" fn kkc_set_typo(e: *mut E, on: i32, bias: f32) {
        if let Some(e) = unsafe { eng(e) } {
            e.lex.typo = on != 0 && e.m.n_kind > K_TYPO as i32;
            if e.lex.typo {
                e.lex.build_bloom();
            }
            e.typo_bias = bias;
        }
    }

    #[no_mangle]
    pub extern "C" fn kkc_set_suggest(e: *mut E, on: i32) {
        if let Some(e) = unsafe { eng(e) } {
            e.suggest_mode = on != 0;
        }
    }

    #[no_mangle]
    pub extern "C" fn kkc_set_char_filter(e: *mut E, mask: i32) {
        if let Some(e) = unsafe { eng(e) } {
            e.char_filter = mask;
        }
    }

    #[no_mangle]
    pub extern "C" fn kkc_set_latin(e: *mut E, on: i32) {
        if let Some(e) = unsafe { eng(e) } {
            e.latin = on != 0;
        }
    }

    #[no_mangle]
    pub extern "C" fn kkc_set_latin_mid2(e: *mut E, on: i32) {
        if let Some(e) = unsafe { eng(e) } {
            e.latin_mid2 = on != 0;
        }
    }

    #[no_mangle]
    pub extern "C" fn kkc_latin_word(e: *mut E, key: *const u16, nk: i32, out: *mut u16, cap: i32) -> i32 {
        let Some(e) = (unsafe { eng(e) }) else { return 0 };
        if !e.latin || nk <= 0 {
            return 0;
        }
        let key = unsafe { sl(key, nk) };
        let mut k = String::new();
        for &c in key {
            if c >= 0x80 {
                return 0;
            }
            k.push(if (b'A' as u16..=b'Z' as u16).contains(&c) { (c + 32) as u8 as char } else { c as u8 as char });
        }
        match e.lex.latin_words.get(&k) {
            Some(w) => {
                let s16 = to16_owned(&w.surf);
                if s16.len() as i32 > cap || out.is_null() {
                    return 0;
                }
                unsafe { std::ptr::copy_nonoverlapping(s16.as_ptr(), out, s16.len()) };
                s16.len() as i32
            }
            None => 0,
        }
    }

    #[no_mangle]
    pub extern "C" fn kkc_set_typo_promote(e: *mut E, per_char: f64) {
        if let Some(e) = unsafe { eng(e) } {
            e.typo_promote = per_char;
        }
    }

    #[no_mangle]
    pub extern "C" fn kkc_last_typo_margin(e: *mut E) -> f64 {
        if e.is_null() {
            0.0
        } else {
            unsafe { (*e).typo_margin }
        }
    }

    #[no_mangle]
    pub extern "C" fn kkc_last_suggest(e: *mut E, out: *mut u16, cap: i32, kinds: *mut i32, kcap: i32) -> i32 {
        let Some(e) = (unsafe { eng(e) }) else { return 0 };
        let mut buf: Vec<u16> = Vec::new();
        let mut n = 0;
        for x in &e.suggest {
            if n >= kcap {
                break;
            }
            to16(&x.1, &mut buf);
            buf.push(0);
            if !kinds.is_null() {
                unsafe { *kinds.add(n as usize) = x.0 };
            }
            n += 1;
        }
        if buf.len() as i32 > cap {
            return -1;
        }
        if !out.is_null() && !buf.is_empty() {
            unsafe { std::ptr::copy_nonoverlapping(buf.as_ptr(), out, buf.len()) };
        }
        n
    }

    #[no_mangle]
    pub extern "C" fn kkc_convert(
        e: *mut E, ctx: *const u16, nctx: i32, kana: *const u16, nk: i32,
        maxout: i32, use_model: i32, out: *mut u16, cap: i32,
    ) -> i32 {
        let Some(e) = (unsafe { eng(e) }) else { return -2 };
        let ctxs = unsafe { sl(ctx, nctx) }.to_vec();
        let kanas = unsafe { sl(kana, nk) }.to_vec();
        match convert_impl(e, &ctxs, &kanas, maxout, use_model) {
            Ok(res) => write_cands(&res, out, cap),
            Err(code) => code,
        }
    }

    #[no_mangle]
    pub extern "C" fn kkc_set_keys(e: *mut E, keys: *const u16, nkeys: i32, span_end: *const i32, nk: i32) {
        let Some(e) = (unsafe { eng(e) }) else { return };
        let se = unsafe { sl(span_end, nk) };
        for i in 0..nk.max(0) as usize {
            let prev = if i > 0 { se[i - 1] } else { 0 };
            if se[i] < prev || se[i] > nkeys {
                e.keys.clear();
                e.span_end.clear();
                return;
            }
        }
        e.keys = unsafe { sl(keys, nkeys) }.to_vec();
        e.span_end = se.to_vec();
    }

    #[no_mangle]
    pub extern "C" fn kkc_set_alt_readings(e: *mut E, buf: *const u16, lens: *const i32, count: i32, nk: i32) {
        let Some(e) = (unsafe { eng(e) }) else { return };
        e.alts.clear();
        e.alts_n = -1;
        if count < 0 || nk <= 0 {
            return;
        }
        let lens = unsafe { sl(lens, count) };
        let total: i32 = lens.iter().sum();
        let all = unsafe { sl(buf, total) };
        let mut at = 0usize;
        for &ln in lens {
            if ln < 0 {
                e.alts.clear();
                return;
            }
            e.alts.push(from16(&all[at..at + ln as usize]));
            at += ln as usize;
        }
        e.alts_n = nk;
    }

    #[no_mangle]
    pub extern "C" fn kkc_set_voice_alts(
        _e: *mut E, full: *const u16, nfull: i32, buf: *const u16, lens: *const i32, costs: *const i32, count: i32,
    ) {
        let mut g = VOICE.lock().unwrap();
        g.0.clear();
        g.1.clear();
        g.2.clear();
        if count <= 0 || nfull <= 0 {
            return;
        }
        g.2 = from16(unsafe { sl(full, nfull) });
        let lens = unsafe { sl(lens, count) };
        let costs = unsafe { sl(costs, count) };
        let total: i32 = lens.iter().sum();
        let all = unsafe { sl(buf, total) };
        let mut at = 0usize;
        for (i, &ln) in lens.iter().enumerate() {
            if ln < 0 {
                break;
            }
            g.0.push(from16(&all[at..at + ln as usize]));
            g.1.push(costs[i]);
            at += ln as usize;
        }
    }

    #[no_mangle]
    pub extern "C" fn kkc_set_right_ctx(e: *mut E, text: *const u16, n: i32) {
        if let Some(e) = unsafe { eng(e) } {
            e.rctx = unsafe { sl(text, n) }.to_vec();
        }
    }

    #[no_mangle]
    pub extern "C" fn kkc_has_right_ctx(e: *mut E) -> i32 {
        if e.is_null() {
            0
        } else {
            unsafe { if (*e).m.n_seg > 3 { 1 } else { 0 } }
        }
    }

    #[no_mangle]
    pub extern "C" fn kkc_last_margin(e: *mut E) -> f64 {
        if e.is_null() {
            0.0
        } else {
            unsafe { (*e).last_cost[1] - (*e).last_cost[0] }
        }
    }

    #[no_mangle]
    pub extern "C" fn kkc_last_times(e: *mut E, t5: *mut f64) {
        if t5.is_null() {
            return;
        }
        for i in 0..5 {
            unsafe { *t5.add(i) = if e.is_null() { 0.0 } else { (*e).times[i] } };
        }
    }

    #[no_mangle]
    pub extern "C" fn kkc_last_scores(e: *mut E, u: *mut f32, cap: i32) -> i32 {
        let Some(e) = (unsafe { eng(e) }) else { return -2 };
        let n = e.last_u.len() as i32;
        if n > cap {
            return -1;
        }
        if !u.is_null() && n > 0 {
            unsafe { std::ptr::copy_nonoverlapping(e.last_u.as_ptr(), u, n as usize) };
        }
        n
    }

    #[no_mangle]
    pub extern "C" fn kkc_last_segments(e: *mut E, ends: *mut i32, lens: *mut i32, cap: i32) -> i32 {
        let Some(e) = (unsafe { eng(e) }) else { return -2 };
        let n = e.last_segs.len() as i32;
        if n > cap {
            return -1;
        }
        for i in 0..n as usize {
            if !ends.is_null() {
                unsafe { *ends.add(i) = e.last_segs[i].e };
            }
            if !lens.is_null() {
                unsafe { *lens.add(i) = e.last_segs[i].l16 };
            }
        }
        n
    }

    #[no_mangle]
    pub extern "C" fn kkc_last_paths(
        e: *mut E, pmax: i32, costs: *mut f64, counts: *mut i32, ends: *mut i32, lens: *mut i32, wcap: i32,
        text: *mut u16, tcap: i32,
    ) -> i32 {
        let Some(e) = (unsafe { eng(e) }) else { return 0 };
        let (mut np, mut nw, mut nt) = (0i32, 0i32, 0i32);
        for p in &e.last_paths {
            if np >= pmax {
                break;
            }
            if nw + p.w.len() as i32 > wcap {
                break;
            }
            let need: i32 = p.w.iter().map(|w| l16len(&w.1) as i32).sum();
            if nt + need > tcap {
                break;
            }
            if !costs.is_null() {
                unsafe { *costs.add(np as usize) = p.c };
            }
            if !counts.is_null() {
                unsafe { *counts.add(np as usize) = p.w.len() as i32 };
            }
            for w in &p.w {
                let s16 = to16_owned(&w.1);
                if !text.is_null() {
                    unsafe { std::ptr::copy_nonoverlapping(s16.as_ptr(), text.add(nt as usize), s16.len()) };
                }
                nt += s16.len() as i32;
                if !ends.is_null() {
                    unsafe { *ends.add(nw as usize) = w.0 };
                }
                if !lens.is_null() {
                    unsafe { *lens.add(nw as usize) = s16.len() as i32 };
                }
                nw += 1;
            }
            np += 1;
        }
        np
    }

    #[no_mangle]
    pub extern "C" fn kkc_last_segment_pos(e: *mut E, lids: *mut i32, rids: *mut i32, cap: i32) -> i32 {
        let Some(e) = (unsafe { eng(e) }) else { return -2 };
        let n = e.last_segs.len() as i32;
        if n > cap {
            return -1;
        }
        for i in 0..n as usize {
            if !lids.is_null() {
                unsafe { *lids.add(i) = e.last_segs[i].lid };
            }
            if !rids.is_null() {
                unsafe { *rids.add(i) = e.last_segs[i].rid };
            }
        }
        n
    }

    #[no_mangle]
    pub extern "C" fn kkc_user_clear(e: *mut E) {
        if let Some(e) = unsafe { eng(e) } {
            e.user.clear();
            e.user_latin.clear();
        }
    }

    #[no_mangle]
    pub extern "C" fn kkc_set_variants(e: *mut E, tsv: *const u16, n: i32) -> i32 {
        let Some(e) = (unsafe { eng(e) }) else { return -1 };
        e.var.clear();
        let all = from16(unsafe { sl(tsv, n) });
        let mut lem: HashMap<Ustr, i32> = HashMap::new();
        let mut line: Ustr = Vec::new();
        let mut flush = |e: &mut E, line: &Ustr, lem: &mut HashMap<Ustr, i32>| {
            let mut col: Vec<Ustr> = vec![Vec::new()];
            for &ch in line {
                if ch == '\t' {
                    col.push(Vec::new());
                } else {
                    col.last_mut().unwrap().push(ch);
                }
            }
            if col.len() >= 4 && !col[0].is_empty() && !col[1].is_empty() {
                let next = lem.len() as i32;
                let id = *lem.entry(col[2].clone()).or_insert(next);
                let mut cl = 0i32;
                for &ch in &col[3] {
                    if ch.is_ascii_digit() {
                        cl = cl * 10 + (ch as i32 - '0' as i32);
                    }
                }
                e.var.insert(col[0].clone(), VarForm { other: col[1].clone(), lemma: id, cls: cl, kanji: false });
                e.var.insert(col[1].clone(), VarForm { other: col[0].clone(), lemma: id, cls: cl, kanji: true });
            }
        };
        for &ch in &all {
            if ch == '\n' {
                flush(e, &line, &mut lem);
                line.clear();
            } else if ch != '\r' {
                line.push(ch);
            }
        }
        if !line.is_empty() {
            flush(e, &line, &mut lem);
        }
        e.kcache.clear();
        e.kstore.clear();
        e.var.len() as i32
    }

    #[no_mangle]
    pub extern "C" fn kkc_personal(e: *mut E) -> i32 {
        if e.is_null() {
            0
        } else {
            unsafe { if (*e).m.n_user > 0 || (*e).m.n_style > 0 { 1 } else { 0 } }
        }
    }

    #[no_mangle]
    pub extern "C" fn kkc_typo_spans(e: *mut E, out: *mut i32, cap: i32) -> i32 {
        let Some(e) = (unsafe { eng(e) }) else { return 0 };
        let mut n = 0;
        for &v in &e.typo_spans {
            if n >= cap {
                break;
            }
            if !out.is_null() {
                unsafe { *out.add(n as usize) = v };
            }
            n += 1;
        }
        n / 2
    }

    #[no_mangle]
    pub extern "C" fn kkc_path_variants(
        e: *mut E, reading: *const u16, nr: i32, text: *const u16, nt: i32, out: *mut u16, cap: i32,
    ) -> i32 {
        let Some(e) = (unsafe { eng(e) }) else { return 0 };
        if e.var.is_empty() || nr <= 0 || nt <= 0 {
            return 0;
        }
        let r = from16(unsafe { sl(reading, nr) });
        let t = from16(unsafe { sl(text, nt) });
        let mut ed = edges(&e.lex, &r, Some(&e.user), None);
        personalize(e, &mut ed);
        let Some(path) = trace_path(e, &r, &t, &ed) else { return 0 };
        let words_surf: Vec<Ustr> = path.iter().map(|&i| ed[i].surf.clone()).collect();
        let mut words: Vec<Ustr> = Vec::new();
        let mut i = 0;
        while i < words_surf.len() {
            let mut len = 0;
            for l in (1..=3.min(words_surf.len() - i)).rev() {
                let mut tt: Ustr = Vec::new();
                for q in 0..l {
                    tt.extend(words_surf[i + q].iter());
                }
                if e.var.contains_key(&tt) {
                    words.push(tt);
                    len = l;
                    break;
                }
            }
            i += if len > 0 { len } else { 1 };
        }
        let mut w = 0i32;
        for it in &words {
            let s16 = to16_owned(it);
            if w + s16.len() as i32 + 1 > cap {
                break;
            }
            if !out.is_null() {
                unsafe {
                    std::ptr::copy_nonoverlapping(s16.as_ptr(), out.add(w as usize), s16.len());
                    *out.add((w + s16.len() as i32) as usize) = 0;
                }
            }
            w += s16.len() as i32 + 1;
        }
        w
    }

    #[no_mangle]
    pub extern "C" fn kkc_path_pos(
        e: *mut E, reading: *const u16, nr: i32, text: *const u16, nt: i32,
        ends: *mut i32, l16: *mut i32, lids: *mut i32, rids: *mut i32, cap: i32,
    ) -> i32 {
        let Some(e) = (unsafe { eng(e) }) else { return 0 };
        if nr <= 0 || nt <= 0 {
            return 0;
        }
        let r = from16(unsafe { sl(reading, nr) });
        let t = from16(unsafe { sl(text, nt) });
        let ed = edges(&e.lex, &r, Some(&e.user), None);
        let Some(path) = trace_path(e, &r, &t, &ed) else { return 0 };
        if path.len() as i32 > cap {
            return -1;
        }
        for (q, &pi) in path.iter().enumerate() {
            let x = &ed[pi];
            if !ends.is_null() {
                unsafe { *ends.add(q) = x.e };
            }
            if !l16.is_null() {
                unsafe { *l16.add(q) = l16len(&x.surf) as i32 };
            }
            if !lids.is_null() {
                unsafe { *lids.add(q) = x.lid };
            }
            if !rids.is_null() {
                unsafe { *rids.add(q) = x.rid };
            }
        }
        path.len() as i32
    }

    #[no_mangle]
    pub extern "C" fn kkc_user_add_pos(
        e: *mut E, r: *const u16, nr: i32, s: *const u16, ns: i32, bonus: i32, flags: i32, lid: i32,
        groups: *const c_char, ngroups: i32,
    ) -> i32 {
        let Some(e) = (unsafe { eng(e) }) else { return 0 };
        if lid < 0 {
            let rs = unsafe { sl(r, nr) }.to_vec();
            let ss = unsafe { sl(s, ns) }.to_vec();
            let ok = user_add_like_impl(e, &rs, &ss, &rs.clone(), &ss.clone(), bonus);
            if ok != 0 && (flags & 1) != 0 {
                e.user.v.last_mut().unwrap().whole = true;
            }
            return ok;
        }
        if nr <= 0 || ns <= 0 {
            return 0;
        }
        let rs = unsafe { sl(r, nr) }.to_vec();
        let ss = unsafe { sl(s, ns) }.to_vec();
        let Some((a, b)) = e.lex.find(&rs) else { return 0 };
        let want = from16(&ss);
        let groups = unsafe { sl(groups as *const u8, ngroups) };
        let mut best: Option<Ent> = None;
        for j in a..b {
            let en = e.lex.ent(j);
            let same = if !groups.is_empty() && lid < ngroups && (en.lid as i32) < ngroups {
                groups[en.lid as usize] == groups[lid as usize]
            } else {
                en.lid as i32 == lid
            };
            if same && e.lex.surf(&en) == want && best.map_or(true, |bb| en.cost < bb.cost) {
                best = Some(en);
            }
        }
        let Some(best) = best else { return 0 };
        e.user.push(UserWord {
            r: from16(&rs),
            s: want,
            lid: best.lid as i32,
            rid: best.rid as i32,
            cost: best.cost - bonus,
            whole: false,
        });
        if (flags & 1) != 0 {
            e.user.v.last_mut().unwrap().whole = true;
        }
        1
    }

    #[no_mangle]
    pub extern "C" fn kkc_variant_info(e: *mut E, s: *const u16, n: i32, out3: *mut i32) -> i32 {
        let Some(e) = (unsafe { eng(e) }) else { return 0 };
        if out3.is_null() {
            return 0;
        }
        let key = from16(unsafe { sl(s, n) });
        match e.var.get(&key) {
            Some(v) => {
                unsafe {
                    *out3 = v.lemma;
                    *out3.add(1) = v.cls;
                    *out3.add(2) = if v.kanji { 1 } else { 0 };
                }
                1
            }
            None => 0,
        }
    }

    #[no_mangle]
    pub extern "C" fn kkc_set_marks(e: *mut E, surfs: *const u16, n: i32, marks: *const i32, nm: i32) {
        let Some(e) = (unsafe { eng(e) }) else { return };
        e.umarks.clear();
        let all = from16(unsafe { sl(surfs, n) });
        let marks = unsafe { sl(marks, nm) };
        let mut cur: Ustr = Vec::new();
        let mut k = 0usize;
        for &ch in &all {
            if ch == '\0' {
                if !cur.is_empty() && k < marks.len() && marks[k] > 0 {
                    e.umarks.insert(cur.clone(), marks[k].min(31));
                }
                if k < marks.len() || !cur.is_empty() {
                    k += 1;
                }
                cur.clear();
            } else {
                cur.push(ch);
            }
        }
    }

    #[no_mangle]
    pub extern "C" fn kkc_set_style(e: *mut E, fam: *const i32, nf: i32, cls: *const i32) {
        let Some(e) = (unsafe { eng(e) }) else { return };
        e.fam.clear();
        let fam = unsafe { sl(fam, nf * 3) };
        for i in 0..nf.max(0) as usize {
            e.fam.insert(fam[3 * i] as i64 * 2 + if fam[3 * i + 1] != 0 { 1 } else { 0 }, fam[3 * i + 2]);
        }
        let cls = unsafe { sl(cls, 16) };
        for c in 0..8 {
            for k in 0..2 {
                e.cls_n[c][k] = if cls.is_empty() { 0 } else { cls[c * 2 + k] };
            }
        }
    }

    #[no_mangle]
    pub extern "C" fn kkc_user_add_like2(
        e: *mut E, r: *const u16, nr: i32, s: *const u16, ns: i32, tr: *const u16, ntr: i32,
        ts: *const u16, nts: i32, bonus: i32, flags: i32,
    ) -> i32 {
        let Some(e) = (unsafe { eng(e) }) else { return 0 };
        let rs = unsafe { sl(r, nr) }.to_vec();
        let ss = unsafe { sl(s, ns) }.to_vec();
        let trs = unsafe { sl(tr, ntr) }.to_vec();
        let tss = unsafe { sl(ts, nts) }.to_vec();
        let ok = user_add_like_impl(e, &rs, &ss, &trs, &tss, bonus);
        if ok != 0 && (flags & 1) != 0 {
            e.user.v.last_mut().unwrap().whole = true;
        }
        ok
    }

    #[no_mangle]
    pub extern "C" fn kkc_user_add_like(
        e: *mut E, r: *const u16, nr: i32, s: *const u16, ns: i32, tr: *const u16, ntr: i32,
        ts: *const u16, nts: i32, bonus: i32,
    ) -> i32 {
        let Some(e) = (unsafe { eng(e) }) else { return 0 };
        let rs = unsafe { sl(r, nr) }.to_vec();
        let ss = unsafe { sl(s, ns) }.to_vec();
        let trs = unsafe { sl(tr, ntr) }.to_vec();
        let tss = unsafe { sl(ts, nts) }.to_vec();
        user_add_like_impl(e, &rs, &ss, &trs, &tss, bonus)
    }

    #[no_mangle]
    pub extern "C" fn kkc_complete(
        e: *mut E, prefix: *const u16, np: i32, max_extra: i32, maxout: i32, out: *mut u16, cap: i32,
    ) -> i32 {
        let Some(e) = (unsafe { eng(e) }) else { return 0 };
        if np <= 0 {
            return 0;
        }
        let pre = unsafe { sl(prefix, np) }.to_vec();
        match complete_impl(e, &pre, max_extra, maxout) {
            Some(res) => write_cands(&res, out, cap),
            None => 0,
        }
    }

    #[no_mangle]
    pub extern "C" fn kkc_romance_enable(e: *mut E, on: i32) -> i32 {
        let Some(e) = (unsafe { eng(e) }) else { return 0 };
        if on != 0 && e.ro.dk != e.m.dk {
            let wu = e.m.wu.clone();
            e.ro.init(e.m.d_uvec, e.m.dk, wu.as_deref());
        }
        e.ro.on = on != 0;
        e.ro.e.clear();
        if e.ro.on {
            e.ro.size() as i32
        } else {
            0
        }
    }

    #[no_mangle]
    pub extern "C" fn kkc_romance_config(e: *mut E, lr_v: f32, lr_w: f32, lr_net: f32, l2: f32, cap: f32) {
        if let Some(e) = unsafe { eng(e) } {
            e.ro.lr_v = lr_v;
            e.ro.lr_w = lr_w;
            e.ro.lr_net = lr_net;
            e.ro.l2 = l2;
            e.ro.cap = cap;
        }
    }

    #[no_mangle]
    pub extern "C" fn kkc_romance_learn(e: *mut E, text: *const u16, nt: i32, loss: *mut f32) -> i32 {
        let Some(e) = (unsafe { eng(e) }) else { return -1 };
        let t = unsafe { sl(text, nt) }.to_vec();
        let (rc, ls) = romance_learn_impl(e, &t);
        if rc == 1 && !loss.is_null() {
            unsafe { *loss = ls };
        }
        rc
    }

    #[no_mangle]
    pub extern "C" fn kkc_romance_size(e: *mut E) -> i32 {
        if e.is_null() {
            0
        } else {
            unsafe { (*e).ro.size() as i32 }
        }
    }

    #[no_mangle]
    pub extern "C" fn kkc_romance_get(e: *mut E, out: *mut f32, cap: i32) -> i32 {
        let Some(e) = (unsafe { eng(e) }) else { return -1 };
        if cap < e.ro.size() as i32 {
            return -1;
        }
        let mut p = 0usize;
        for part in [&e.ro.v, &e.ro.w, &e.ro.a, &e.ro.ab, &e.ro.c] {
            if !out.is_null() && !part.is_empty() {
                unsafe { std::ptr::copy_nonoverlapping(part.as_ptr(), out.add(p), part.len()) };
            }
            p += part.len();
        }
        p as i32
    }

    #[no_mangle]
    pub extern "C" fn kkc_romance_set(e: *mut E, inp: *const f32, n: i32) -> i32 {
        let Some(e) = (unsafe { eng(e) }) else { return 0 };
        if n != e.ro.size() as i32 || n == 0 {
            return 0;
        }
        let all = unsafe { sl(inp, n) };
        let mut p = 0usize;
        for part in [&mut e.ro.v, &mut e.ro.w, &mut e.ro.a, &mut e.ro.ab, &mut e.ro.c] {
            let len = part.len();
            part.copy_from_slice(&all[p..p + len]);
            p += len;
        }
        n
    }

    #[no_mangle]
    pub extern "C" fn kkc_romance_updates(e: *mut E) -> i32 {
        if e.is_null() {
            0
        } else {
            unsafe { (*e).ro.updates }
        }
    }

    #[no_mangle]
    pub extern "C" fn kkc_romance_reset(e: *mut E) {
        let Some(e) = (unsafe { eng(e) }) else { return };
        if e.ro.dk == 0 {
            return;
        }
        let wu = e.m.wu.clone();
        e.ro.init(e.m.d_uvec, e.m.dk, wu.as_deref());
        e.ro.e.clear();
    }

    #[no_mangle]
    pub extern "C" fn kkc_model_tag(e: *mut E) -> u32 {
        if e.is_null() {
            0
        } else {
            unsafe { (*e).model_tag }
        }
    }
}

// ------------------------------------------------------------------ tests
#[cfg(test)]
mod tests {
    use super::*;

    struct Bin {
        v: Vec<u8>,
    }
    impl Bin {
        fn align(&mut self) {
            while self.v.len() % 8 != 0 {
                self.v.push(0);
            }
        }
        fn i32(&mut self, x: i32) {
            self.v.extend(x.to_le_bytes());
        }
        fn i64(&mut self, x: i64) {
            self.v.extend(x.to_le_bytes());
        }
        fn f32(&mut self, x: f32) {
            self.v.extend(x.to_le_bytes());
        }
        fn tag(&mut self, s: &[u8; 4]) {
            self.v.extend(s);
        }
        fn arr_i16(&mut self, a: &[i16]) {
            self.align();
            for x in a {
                self.v.extend(x.to_le_bytes());
            }
        }
        fn arr_u32(&mut self, a: &[u32]) {
            self.align();
            for x in a {
                self.v.extend(x.to_le_bytes());
            }
        }
        fn arr_u16(&mut self, a: &[u16]) {
            self.align();
            for x in a {
                self.v.extend(x.to_le_bytes());
            }
        }
        fn arr_f32(&mut self, a: &[f32]) {
            self.align();
            for x in a {
                self.v.extend(x.to_le_bytes());
            }
        }
        fn arr_bytes(&mut self, a: &[u8]) {
            self.align();
            self.v.extend(a);
        }
    }

    // readings (sorted) -> (surface, lid, rid, cost)
    fn lex_data() -> Vec<(Vec<u16>, Vec<(Vec<u16>, u16, u16, i32)>)> {
        let w = |s: &str| -> Vec<u16> { s.encode_utf16().collect() };
        vec![
            (w("あい"), vec![(w("愛"), 1, 1, 100)]),
            (w("あした"), vec![(w("明日"), 1, 1, 120)]),
            (w("こん"), vec![(w("今"), 1, 1, 200), (w("根"), 1, 1, 300)]),
            (w("こんにちは"), vec![(w("こんにちは"), 1, 1, 150)]),
            (w("こんばんは"), vec![(w("こんばんは"), 1, 1, 160)]),
            (w("じーでぃーぴー"), vec![(w("GDP"), 1, 1, 400)]),
            (w("ずーむ"), vec![(w("Zoom"), 1, 1, 100)]),
            (w("はしら"), vec![(w("走ら"), 1, 1, 250)]),
        ]
    }

    fn build_lex(ver: i32) -> Vec<u8> {
        build_lex_from(lex_data(), ver)
    }

    fn build_lex_from(data: Vec<(Vec<u16>, Vec<(Vec<u16>, u16, u16, i32)>)>, ver: i32) -> Vec<u8> {
        let nread = data.len();
        let mut s_blob: Vec<u16> = Vec::new();
        let mut ents: Vec<(u32, u16, u16, u16, i32)> = Vec::new();
        let mut e_first: Vec<u32> = vec![0];
        for (_, ws) in &data {
            for (sf, lid, rid, cost) in ws {
                ents.push((s_blob.len() as u32, sf.len() as u16, *lid, *rid, *cost));
                s_blob.extend(sf);
            }
            e_first.push(ents.len() as u32);
        }
        // readings blob
        let (r_blob, r_off): (Vec<u16>, Vec<u32>) = if ver == 1 {
            let mut b = Vec::new();
            let mut o = vec![0u32];
            for (r, _) in &data {
                b.extend(r);
                o.push(b.len() as u32);
            }
            (b, o)
        } else {
            let mut b: Vec<u16> = Vec::new();
            let mut o = vec![0u32];
            let mut prev: Vec<u16> = Vec::new();
            for (i, (r, _)) in data.iter().enumerate() {
                if i % 16 == 0 {
                    b.extend(r);
                } else {
                    let mut pre = 0usize;
                    while pre < prev.len() && pre < r.len() && prev[pre] == r[pre] {
                        pre += 1;
                    }
                    b.push(pre as u16);
                    b.extend(&r[pre..]);
                }
                o.push(b.len() as u32);
                prev = r.clone();
            }
            (b, o)
        };
        let mut bin = Bin { v: Vec::new() };
        bin.tag(b"KKL2");
        for x in [
            ver, 1, 2, 3, 5, 5000, 4000, 100, 3000, 4, 1, nread as i32, ents.len() as i32,
        ] {
            bin.i32(x);
        }
        bin.i64(r_blob.len() as i64);
        bin.i64(s_blob.len() as i64);
        bin.arr_i16(&vec![0i16; 16]);
        bin.arr_u32(&r_off);
        bin.arr_u32(&e_first);
        if ver == 1 {
            let mut raw = Vec::new();
            for e in &ents {
                raw.extend(e.0.to_le_bytes());
                raw.extend(e.1.to_le_bytes());
                raw.extend(e.2.to_le_bytes());
                raw.extend(e.3.to_le_bytes());
                raw.extend(0u16.to_le_bytes());
                raw.extend(e.4.to_le_bytes());
            }
            bin.arr_bytes(&raw);
        } else {
            let mut raw = Vec::new();
            for e in &ents {
                raw.extend(((e.0 & 0xFFFF) as u16).to_le_bytes());
                let b = ((e.0 >> 16) & 0x3FF) as u16 | (e.1 << 10);
                raw.extend(b.to_le_bytes());
                raw.extend(e.2.to_le_bytes());
                raw.extend(e.3.to_le_bytes());
                raw.extend((e.4 as i16).to_le_bytes());
            }
            bin.arr_bytes(&raw);
        }
        bin.arr_u16(&r_blob);
        bin.arr_u16(&s_blob);
        bin.v
    }

    fn build_model() -> Vec<u8> {
        // v2, d=4, dk=4, layers=0, n_kind=6 (typo-capable), draft=1
        let chars: Vec<char> = "あいこんはしらぐ今愛走グ明日にちばん".chars().collect();
        let v = 4 + chars.len();
        let (d, dk) = (4usize, 4usize);
        let mut seed: u64 = 12345;
        let mut rnd = move || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            ((seed >> 33) as f32 / (1u64 << 31) as f32 - 0.5) * 0.2
        };
        let mut vec_of = |n: usize| -> Vec<f32> { (0..n).map(|_| rnd()).collect() };
        let mut bin = Bin { v: Vec::new() };
        bin.tag(b"KKM1");
        for x in [2, v as i32, d as i32, 0, 1, 4, dk as i32, 32, 4, 40, 16, 1 + (7 << 8)] {
            bin.i32(x);
        }
        bin.f32(1.0);
        bin.f32(1.0);
        bin.i32(chars.len() as i32);
        // vocab chars are a plain u32 array (val reads, no alignment)
        for c in &chars {
            bin.v.extend((*c as u32).to_le_bytes());
        }
        bin.arr_f32(&vec_of(v * d)); // emb
        bin.arr_f32(&vec_of(4 * d)); // seg
        bin.arr_f32(&vec_of(32 * d)); // pos
        bin.arr_f32(&vec![1.0; d]); // normw
        bin.arr_f32(&vec_of(d)); // normb
        bin.arr_f32(&vec_of(d)); // end
        bin.arr_f32(&vec_of(d * 3 * d)); // q1w
        bin.arr_f32(&vec_of(d)); // q1b
        bin.arr_f32(&vec_of(dk * d)); // q2w
        bin.arr_f32(&vec_of(dk)); // q2b
        bin.arr_f32(&vec_of(v * d)); // Pa
        bin.arr_f32(&vec_of(v * d)); // Pb
        bin.arr_f32(&vec_of(v * d)); // Pc
        bin.arr_f32(&vec_of(7 * d)); // Kkind
        bin.arr_f32(&vec_of(17 * d)); // Klen
        bin.arr_f32(&vec_of(d)); // k1b
        bin.arr_f32(&vec_of(dk * d)); // k2w
        bin.arr_f32(&vec_of(dk)); // k2b
        bin.v
    }

    fn open_test(ver: i32) -> Box<kkc_engine> {
        open_engine(&build_lex(ver), &build_model()).expect("open")
    }
    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }
    fn conv(e: &mut kkc_engine, kana: &str, use_model: i32) -> Vec<String> {
        let res = convert_impl(e, &[], &w(kana), 10, use_model).expect("convert");
        res.iter().map(|s| s.iter().collect::<String>()).collect()
    }

    #[test]
    fn lex_v1_convert_dict_cost() {
        let mut e = open_test(1);
        let res = conv(&mut e, "あい", 0);
        assert_eq!(res[0], "愛");
        let res = conv(&mut e, "こん", 0);
        assert_eq!(res[0], "今");
        assert!(res.contains(&"根".to_string()));
        // segments of the top candidate
        assert_eq!(e.last_segs.len(), 1);
        assert_eq!(e.last_segs[0].e, 2);
    }

    #[test]
    fn lex_v2_convert_matches_v1() {
        let mut e = open_test(2);
        let res = conv(&mut e, "あい", 0);
        assert_eq!(res[0], "愛");
        let res = conv(&mut e, "こんにちは", 0);
        assert_eq!(res[0], "こんにちは");
    }

    #[test]
    fn number_edges_convert() {
        let mut e = open_test(1);
        let res = conv(&mut e, "にせん", 0);
        assert!(res.iter().any(|s| s == "2000" || s == "二千"), "got {res:?}");
    }

    #[test]
    fn complete_prefix() {
        let e = open_test(1);
        let res = complete_impl(&e, &w("こん"), 3, 10).unwrap();
        let ss: Vec<String> = res.iter().map(|s| s.iter().collect()).collect();
        assert!(ss.contains(&"こんにちは".to_string()), "got {ss:?}");
        assert!(ss.contains(&"こんばんは".to_string()), "got {ss:?}");
    }

    #[test]
    fn user_add_like_and_convert() {
        let mut e = open_test(1);
        let ok = user_add_like_impl(&mut e, &w("ぐぐら"), &w("ググら"), &w("はしら"), &w("走ら"), 500);
        assert_eq!(ok, 1);
        let res = conv(&mut e, "ぐぐら", 0);
        assert_eq!(res[0], "ググら");
        // representative not in the lexicon -> 0
        let ok = user_add_like_impl(&mut e, &w("xxx"), &w("yyy"), &w("ないよみ"), &w("無い"), 0);
        assert_eq!(ok, 0);
    }

    #[test]
    fn model_convert_and_scores() {
        let mut e = open_test(1);
        let res = conv(&mut e, "あい", 1);
        assert!(!res.is_empty());
        assert!(!e.last_u.is_empty());
        assert!(!e.last_segs.is_empty());
        assert!(e.model_tag != 0);
    }

    #[test]
    fn latin_word_lookup() {
        let mut e = open_test(1);
        // via FFI
        let key = w("zoom");
        let mut out = [0u16; 16];
        let n = ffi::kkc_latin_word(&mut *e, key.as_ptr(), 4, out.as_mut_ptr(), 16);
        assert_eq!(n, 4);
        assert_eq!(String::from_utf16_lossy(&out[..4]), "Zoom");
    }

    #[test]
    fn romance_learn_cycle() {
        let mut e = open_test(1);
        let sz = ffi::kkc_romance_enable(&mut *e, 1);
        assert!(sz > 0);
        let _ = conv(&mut e, "あい", 1);
        let top: Vec<u16> = {
            let res = convert_impl(&mut e, &[], &w("あい"), 10, 1).unwrap();
            to16_owned(&res[0])
        };
        let mut loss = 0f32;
        let rc = ffi::kkc_romance_learn(&mut *e, top.as_ptr(), top.len() as i32, &mut loss);
        assert_eq!(rc, 1, "loss={loss}");
        assert_eq!(ffi::kkc_romance_updates(&mut *e), 1);
        let sz2 = ffi::kkc_romance_size(&mut *e);
        assert_eq!(sz2, sz);
        let mut buf = vec![0f32; sz2 as usize];
        assert_eq!(ffi::kkc_romance_get(&mut *e, buf.as_mut_ptr(), sz2), sz2);
        ffi::kkc_romance_reset(&mut *e);
        assert_eq!(ffi::kkc_romance_updates(&mut *e), 0);
    }

    #[test]
    fn ffi_convert_and_paths() {
        let mut e = open_test(1);
        let kana = w("あした");
        let mut out = [0u16; 256];
        let n = ffi::kkc_convert(&mut *e, [].as_ptr(), 0, kana.as_ptr(), kana.len() as i32, 10, 0, out.as_mut_ptr(), 256);
        assert!(n >= 1, "n={n}");
        let first: Vec<u16> = out.iter().take_while(|&&c| c != 0).cloned().collect();
        assert_eq!(String::from_utf16_lossy(&first), "明日");
        let mut ends = [0i32; 8];
        let mut lens = [0i32; 8];
        let m = ffi::kkc_last_segments(&mut *e, ends.as_mut_ptr(), lens.as_mut_ptr(), 8);
        assert_eq!(m, 1);
        assert_eq!(ends[0], 3);
        // path_pos: reading あした -> 明日
        let text = w("明日");
        let (mut e2, mut l16, mut lid, mut rid) = ([0i32; 8], [0i32; 8], [0i32; 8], [0i32; 8]);
        let pc = ffi::kkc_path_pos(
            &mut *e, kana.as_ptr(), kana.len() as i32, text.as_ptr(), text.len() as i32,
            e2.as_mut_ptr(), l16.as_mut_ptr(), lid.as_mut_ptr(), rid.as_mut_ptr(), 8,
        );
        assert_eq!(pc, 1);
        assert_eq!(e2[0], 3);
    }

    #[test]
    fn glide_lexicon_words_from_engine() {
        let e = open_test(1);
        // mode 0: 全読みが ひらがな・ー で長さ・コスト条件を満たす
        let w0 = e.glide_lexicon_words(0).expect("mode0");
        assert_eq!(w0.len(), 8, "mode0 words: {:?}", w0);
        assert!(w0.contains(&(w("あした"), 120)));
        assert!(w0.contains(&(w("あい"), 100)));
        // mode 1: 辞書の英字の語 (GDP / Zoom) だけ
        let w1 = e.glide_lexicon_words(1).expect("mode1");
        assert_eq!(w1.len(), 2, "mode1 words: {:?}", w1);
        assert!(w1.contains(&(w("gdp"), 400)));
        assert!(w1.contains(&(w("zoom"), 100)));
        // mode 2 はエンジンからは作らない
        assert!(e.glide_lexicon_words(2).is_none());
    }

    #[test]
    fn glide_mode0_via_ffi_with_small_lexicon() {
        // 小さな辞書をロードしたエンジンで kkc_glide_prepare/decode (mode 0)
        let mut e = open_test(1);
        let ep = &mut *e as *mut kkc_engine as *mut std::os::raw::c_void;
        let nodes = crate::glide::ffi::kkc_glide_prepare(ep, 0);
        assert!(nodes > 0, "mode0 の木の節の数 {nodes}");
        // QWERTY で "ashita" と なぞる -> あした
        let rows: [(&str, f32, f32); 3] = [("qwertyuiop", 0.0, 0.0), ("asdfghjkl", 0.5, 1.0), ("zxcvbnm", 1.0, 2.0)];
        let mut key_str: Vec<u8> = Vec::new();
        let mut kxy: Vec<f32> = Vec::new();
        let mut mp: std::collections::HashMap<u8, (f32, f32)> = std::collections::HashMap::new();
        for (r, off, y) in rows {
            for (i, c) in r.bytes().enumerate() {
                key_str.push(c);
                kxy.push(off + i as f32);
                kxy.push(y);
                mp.insert(c, (off + i as f32, y));
            }
        }
        let cs: Vec<(f32, f32)> = "ashita".bytes().map(|c| mp[&c]).collect();
        let mut pts: Vec<f32> = Vec::new();
        for win in cs.windows(2) {
            for i in 0..6 {
                let t = i as f32 / 6.0;
                pts.push(win[0].0 + t * (win[1].0 - win[0].0));
                pts.push(win[0].1 + t * (win[1].1 - win[0].1));
            }
        }
        pts.push(cs[cs.len() - 1].0);
        pts.push(cs[cs.len() - 1].1);
        let mut out = vec![0u16; 512];
        let mut costs = vec![0.0f32; 16];
        let got = crate::glide::ffi::kkc_glide_decode(
            ep, 0, key_str.as_ptr() as *const std::os::raw::c_char, kxy.as_ptr(), key_str.len() as i32,
            pts.as_ptr(), (pts.len() / 2) as i32, 5, out.as_mut_ptr(), 512, costs.as_mut_ptr(),
        );
        assert!(got >= 1, "mode0 候補数 {got}");
        let first: Vec<u16> = out.iter().take_while(|&&c| c != 0).cloned().collect();
        assert_eq!(String::from_utf16_lossy(&first), "あした");
    }

    #[test]
    fn typo_two_word_covering_pair() {
        // 直した読み あいうえ が 1 語では覆えず、あい|うえ の 2 語で覆う組
        let data = vec![
            (w("あい"), vec![(w("愛"), 1, 1, 100)]),
            (w("うえ"), vec![(w("上"), 1, 1, 100)]),
        ];
        let lex = load_lex(&build_lex_from(data, 1)).expect("lex");
        let r = u("あきうえ"); // き が打ち間違い (正しくは い)
        let base: Vec<Edge> = (0..4).map(|i| Edge::new(i, i + 1, vec![r[i as usize]], 1, 1, 5000, K_KANA)).collect();
        let ctx = build_typo_ctx(&lex, &base, 4);
        let mut out: Vec<Edge> = Vec::new();
        typo_add_edges(&lex, &r, 0, &base, &ctx, &mut out, None, Some(&[u("あいうえ")]));
        let hit = out.iter().find(|e| e.s == 0 && e.e == 4 && e.surf == u("愛上"));
        assert!(hit.is_some(), "2語の組のエッジが無い: {:?}", out.iter().map(|e| (e.s, e.e, e.surf.iter().collect::<String>(), e.cost, e.kind)).collect::<Vec<_>>());
        let hit = hit.unwrap();
        assert_eq!(hit.kind, K_TYPO);
        assert_eq!(hit.cost, 100 + 100 + TYPO_COST); // conn(1,1) = 0
    }

    #[test]
    fn latin_mid_chunk_joining() {
        // 英字の語 "ab" (辞書表記 aB) が かなチャンク「ば」の途中で終わり、
        // 残りの母音 a で始まる辞書語 あぷ (アプ) とつないだエッジを作る
        let data = vec![(w("あぷ"), vec![(w("アプ"), 1, 1, 200)])];
        let mut lex = load_lex(&build_lex_from(data, 1)).expect("lex");
        lex.latin_words.insert("ab".to_string(), LatinWord { surf: u("aB"), lid: 1, rid: 1, cost: 100 });
        let r = u("あばぷり");
        let keys: Vec<u16> = "abapuri".bytes().map(|c| c as u16).collect();
        let span_end = vec![1, 3, 5, 7];
        for mid2 in [false, true] {
            let mut out: Vec<Edge> = Vec::new();
            latin_edges(&lex, &keys, &span_end, 4, &r, &HashMap::new(), &mut out, false, false, mid2);
            let hit = out.iter().find(|e| e.s == 0 && e.e == 3 && e.surf == u("aBアプ"));
            assert!(hit.is_some(), "mid2={mid2} で連結エッジが無い: {:?}", out.iter().map(|e| (e.s, e.e, e.surf.iter().collect::<String>())).collect::<Vec<_>>());
            assert_eq!(hit.unwrap().cost, 100 + LATIN_BIAS + 200);
            assert_eq!(hit.unwrap().kind, K_LATIN);
        }
    }

    #[test]
    fn date_candidates_local_time_tz() {
        // TZ を固定して検証する (KKC_NOW で今を固定)
        let saved_now = std::env::var("KKC_NOW").ok();
        let saved_tz = std::env::var("TZ").ok();
        let now = unix_from_civil(2026, 1, 1, 0) + 1800; // 2026-01-01 00:30 UTC
        std::env::set_var("KKC_NOW", now.to_string());
        let first = |reading: &str, tz: &str| -> String {
            std::env::set_var("TZ", tz);
            let v = date_candidates(&u(reading));
            assert!(!v.is_empty(), "{reading} の候補が空 (TZ={tz})");
            v[0].iter().collect()
        };
        assert_eq!(first("いま", "UTC0"), "00:30");
        assert_eq!(first("いま", "JST-9"), "09:30"); // POSIX 形式 (符号反転)
        assert_eq!(first("いま", "EST5"), "19:30"); // 前日の夜
        assert_eq!(first("いま", "EST5EDT,M3.2.0/2,M11.1.0/2"), "19:30"); // 1月は標準時
        if std::path::Path::new("/usr/share/zoneinfo/Asia/Tokyo").exists() {
            assert_eq!(first("いま", "Asia/Tokyo"), "09:30"); // TZif ファイル経由
        }
        // 7月は夏時間 (EDT = UTC-4)
        std::env::set_var("KKC_NOW", (unix_from_civil(2026, 7, 1, 0) + 1800).to_string());
        assert_eq!(first("いま", "EST5EDT,M3.2.0/2,M11.1.0/2"), "20:30");
        assert_eq!(first("いま", "EST5"), "19:30");
        // 日付も TZ でずれる
        std::env::set_var("KKC_NOW", now.to_string());
        assert_eq!(first("きょう", "EST5"), "2025/12/31");
        assert_eq!(first("きょう", "JST-9"), "2026/1/1");
        match saved_now {
            Some(v) => std::env::set_var("KKC_NOW", v),
            None => std::env::remove_var("KKC_NOW"),
        }
        match saved_tz {
            Some(v) => std::env::set_var("TZ", v),
            None => std::env::remove_var("TZ"),
        }
    }

    #[test]
    fn erf_and_gelu_match_std_values() {
        // std::erf (glibc) の既知の値と f64 で 1e-12 まで一致すること
        let cases: [(f64, f64); 7] = [
            (0.0, 0.0),
            (0.25, 0.2763263901682369),
            (0.5, 0.5204998778130465),
            (1.0, 0.8427007929497149),
            (-1.5, -0.9661051464753108),
            (2.0, 0.9953222650189527),
            (3.0, 0.9999779095030014),
        ];
        for (x, want) in cases {
            assert!((erf_f64(x) - want).abs() < 1e-12, "erf({x}) = {} != {want}", erf_f64(x));
        }
        // gelu は f32 でも std::erf 版と 1e-6 以内
        assert!((gelu(1.0) - 0.84134475).abs() < 1e-6, "gelu(1) = {}", gelu(1.0));
        assert!((gelu(-2.0) - -0.04550026).abs() < 1e-6, "gelu(-2) = {}", gelu(-2.0));
        assert_eq!(gelu(0.0), 0.0);
    }

    #[test]
    fn variants_roundtrip() {
        let mut e = open_test(1);
        let tsv = w("かな\t漢字\t見出し\t1\n");
        let n = ffi::kkc_set_variants(&mut *e, tsv.as_ptr(), tsv.len() as i32);
        assert_eq!(n, 2);
        let s = w("漢字");
        let mut out3 = [0i32; 3];
        assert_eq!(ffi::kkc_variant_info(&mut *e, s.as_ptr(), s.len() as i32, out3.as_mut_ptr()), 1);
        assert_eq!(out3[2], 1);
    }
}
