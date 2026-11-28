//! store: 学習 (Learning)・ユーザー辞書 (UserDict)・書き方の記録 (StyleLog) の永続化。
//! 元: desktop/core/store.cpp (1164行) + store.h (211行) + json.h (175行)。
//! ファイル形式は元の JSON/TSV と互換 (learned.json / user_dict.json / style.json / recent_learn.tsv)。
//!
//! 依存先の吸収 (他担当が並行実装中のため、このファイル内で完結させる):
//! - text.h の変換 (to_utf8/from_utf8/to_hiragana/to_halfwidth_ascii/to_fullwidth_ascii) は
//!   crate::u16s / crate::from_u16s と本ファイル内の小関数で代替 (text.rs は未実装スタブ)。
//! - style.h の StyleModel::Count は本ファイルの `StyleCount` で代替 (style.rs は未実装スタブ)。
//! - pos_classes.h の POS_GROUPS は生成表を本ファイルに埋め込んだ (pos.rs は未実装スタブ)。

use crate::{from_u16s, u16s, U16};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------- text helpers (text.h 相当)
pub(crate) fn to_utf8(s: &[u16]) -> String { from_u16s(s) }
pub(crate) fn from_utf8(s: &str) -> U16 { u16s(s) }
pub(crate) fn to_hiragana(s: &[u16]) -> U16 {
    s.iter().map(|&c| if (0x30A1..=0x30F6).contains(&c) { c - 0x60 } else { c }).collect()
}
pub(crate) fn to_halfwidth_ascii(s: &[u16]) -> U16 {
    s.iter().map(|&c| {
        if c == 0x3000 { b' ' as u16 } else if c > 0xFF00 && c < 0xFF5F { c - 0xFEE0 }
        else if c == 0xFFE5 { b'\\' as u16 } else { c }
    }).collect()
}
pub(crate) fn to_fullwidth_ascii(s: &[u16]) -> U16 {
    s.iter().map(|&c| {
        if c == b' ' as u16 { 0x3000 } else if c > 0x20 && c < 0x7F { c + 0xFEE0 } else { c }
    }).collect()
}

/// POS_GROUPS (pos_classes.h、生成表)。lid -> 粗い品詞グループ文字。範囲外は 0。
pub const POS_GROUPS: &str = "OOIIIIIIIIIIDDDDDDDDDDDDDDDDDXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSNNNNNNNNNNNNNNNnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaIICCCFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFKKKKKKKKKKKKKKKKRRRRRRRRRRRRR";
pub const POS_CLASS_COUNT: usize = 2672;
pub(crate) fn group_of_lid(lid: i32) -> u8 {
    if lid >= 0 && (lid as usize) < POS_CLASS_COUNT {
        POS_GROUPS.as_bytes()[lid as usize]
    } else { 0 }
}

// ---------------------------------------------------------------- json.h 相当
pub mod json {
    #[derive(Clone, Debug, PartialEq)]
    pub enum Value {
        Null,
        Bool(bool),
        Number(f64),
        String(String),
        Array(Vec<Value>),
        Object(Vec<(String, Value)>),
    }
    impl Value {
        pub fn get_str(&self, key: &str) -> String { self.get_str_or(key, "") }
        pub fn get_str_or(&self, key: &str, dflt: &str) -> String {
            if let Value::Object(o) = self {
                for (k, v) in o { if k == key { if let Value::String(s) = v { return s.clone(); } } }
            }
            dflt.to_string()
        }
        pub fn as_obj(&self) -> Option<&Vec<(String, Value)>> {
            if let Value::Object(o) = self { Some(o) } else { None }
        }
        pub fn as_arr(&self) -> Option<&Vec<Value>> {
            if let Value::Array(a) = self { Some(a) } else { None }
        }
        pub fn as_num(&self) -> Option<f64> {
            if let Value::Number(n) = self { Some(*n) } else { None }
        }
    }
    pub struct Parser { s: Vec<char>, i: usize }
    impl Parser {
        pub fn new(text: &str) -> Self { Parser { s: text.chars().collect(), i: 0 } }
        pub fn parse(&mut self) -> Option<Value> { self.ws(); self.value() }
        fn ws(&mut self) { while self.i < self.s.len() && matches!(self.s[self.i], ' ' | '\t' | '\n' | '\r') { self.i += 1; } }
        fn lit(&mut self, w: &str) -> bool {
            let wv: Vec<char> = w.chars().collect();
            if self.i + wv.len() <= self.s.len() && self.s[self.i..self.i + wv.len()] == wv[..] { self.i += wv.len(); true } else { false }
        }
        fn string(&mut self) -> Option<String> {
            if self.i >= self.s.len() || self.s[self.i] != '"' { return None; }
            self.i += 1;
            let mut o = String::new();
            while self.i < self.s.len() {
                let c = self.s[self.i]; self.i += 1;
                match c {
                    '"' => return Some(o),
                    '\\' => {
                        if self.i >= self.s.len() { return None; }
                        let e = self.s[self.i]; self.i += 1;
                        match e {
                            'n' => o.push('\n'), 'r' => o.push('\r'), 't' => o.push('\t'),
                            'u' => {
                                if self.i + 4 > self.s.len() { return None; }
                                let hex: String = self.s[self.i..self.i + 4].iter().collect();
                                self.i += 4;
                                let mut cp = u32::from_str_radix(&hex, 16).ok()?;
                                // サロゲートペア
                                if (0xD800..0xDC00).contains(&cp) && self.i + 6 <= self.s.len()
                                    && self.s[self.i] == '\\' && self.s[self.i + 1] == 'u' {
                                    let hex2: String = self.s[self.i + 2..self.i + 6].iter().collect();
                                    if let Ok(lo) = u32::from_str_radix(&hex2, 16) {
                                        if (0xDC00..0xE000).contains(&lo) {
                                            cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                                            self.i += 6;
                                        }
                                    }
                                }
                                o.push(char::from_u32(cp).unwrap_or('\u{FFFD}'));
                            }
                            other => o.push(other),
                        }
                    }
                    other => o.push(other),
                }
            }
            None
        }
        fn value(&mut self) -> Option<Value> {
            self.ws();
            if self.i >= self.s.len() { return None; }
            match self.s[self.i] {
                '{' => {
                    self.i += 1; self.ws();
                    let mut obj = Vec::new();
                    if self.i < self.s.len() && self.s[self.i] == '}' { self.i += 1; return Some(Value::Object(obj)); }
                    loop {
                        self.ws();
                        let k = self.string()?;
                        self.ws();
                        if self.i >= self.s.len() || self.s[self.i] != ':' { return None; }
                        self.i += 1;
                        let v = self.value()?;
                        obj.push((k, v));
                        self.ws();
                        if self.i >= self.s.len() { return None; }
                        if self.s[self.i] == ',' { self.i += 1; continue; }
                        if self.s[self.i] == '}' { self.i += 1; return Some(Value::Object(obj)); }
                        return None;
                    }
                }
                '[' => {
                    self.i += 1; self.ws();
                    let mut arr = Vec::new();
                    if self.i < self.s.len() && self.s[self.i] == ']' { self.i += 1; return Some(Value::Array(arr)); }
                    loop {
                        let v = self.value()?;
                        arr.push(v);
                        self.ws();
                        if self.i >= self.s.len() { return None; }
                        if self.s[self.i] == ',' { self.i += 1; continue; }
                        if self.s[self.i] == ']' { self.i += 1; return Some(Value::Array(arr)); }
                        return None;
                    }
                }
                '"' => self.string().map(Value::String),
                _ => {
                    if self.lit("true") { return Some(Value::Bool(true)); }
                    if self.lit("false") { return Some(Value::Bool(false)); }
                    if self.lit("null") { return Some(Value::Null); }
                    let st = self.i;
                    while self.i < self.s.len() && matches!(self.s[self.i], '-' | '+' | '.' | 'e' | 'E' | '0'..='9') { self.i += 1; }
                    if st == self.i { return None; }
                    let txt: String = self.s[st..self.i].iter().collect();
                    txt.parse::<f64>().ok().map(Value::Number)
                }
            }
        }
    }
    pub fn quote(s: &str) -> String {
        let mut o = String::from("\"");
        for c in s.chars() {
            match c {
                '"' => o.push_str("\\\""),
                '\\' => o.push_str("\\\\"),
                '\n' => o.push_str("\\n"),
                '\r' => o.push_str("\\r"),
                '\t' => o.push_str("\\t"),
                c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
                c => o.push(c),
            }
        }
        o.push('"');
        o
    }
}

// ---------------------------------------------------------------- ファイル読み書き
pub fn read_file(p: &Path) -> Option<String> {
    std::fs::read(p).ok().map(|b| String::from_utf8_lossy(&b).into_owned())
}
/// 読めなかった (壊れた) ファイルを別名で残す (learned.json.broken-<秒>)。
pub fn keep_broken(p: &Path) {
    let t = UNIX_EPOCH.elapsed().map(|d| d.as_secs()).unwrap_or(0);
    let mut np = p.as_os_str().to_owned();
    np.push(format!(".broken-{}", t));
    let _ = std::fs::rename(p, PathBuf::from(np));
}
fn read_or_missing(p: &Path) -> Option<String> {
    if !p.exists() { return Some(String::new()); }
    read_file(p)
}
pub fn write_file_atomic(p: &Path, data: &str) -> bool {
    if let Some(parent) = p.parent() { if !parent.as_os_str().is_empty() { let _ = std::fs::create_dir_all(parent); } }
    let mut tmp = p.as_os_str().to_owned();
    tmp.push(format!(".tmp{}", std::process::id()));
    let tmp = PathBuf::from(tmp);
    if std::fs::write(&tmp, data).is_err() { return false; }
    if std::fs::rename(&tmp, p).is_err() { let _ = std::fs::remove_file(&tmp); return false; }
    true
}
fn stamp_of(p: &Path) -> Option<SystemTime> { std::fs::metadata(p).ok().and_then(|m| m.modified().ok()) }
fn now_sec() -> i64 { UNIX_EPOCH.elapsed().map(|d| d.as_secs() as i64).unwrap_or(0) }

// ---------------------------------------------------------------- 最近の学習の控え (recent_learn.tsv)
pub const RECENT_FILE: &str = "recent_learn.tsv";
const RECENT_MAX_BYTES: u64 = 256 * 1024 * 1024;

fn recent_trim(p: &Path) {
    if let Some(s) = read_file(p) {
        if (s.len() as u64) < RECENT_MAX_BYTES / 2 { return; }
        let start = s.len() - (RECENT_MAX_BYTES / 2) as usize;
        if let Some(e) = s[start..].find('\n') { write_file_atomic(p, &s[start + e + 1..]); }
    }
}
pub fn recent_add(dir: &Path, fields: &[String]) {
    let mut line = now_sec().to_string();
    for f in fields {
        if f.contains('\t') || f.contains('\n') { return; }
        line.push('\t'); line.push_str(f);
    }
    line.push('\n');
    let p = dir.join(RECENT_FILE);
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        let _ = f.write_all(line.as_bytes());
    }
    if let Ok(m) = std::fs::metadata(&p) { if m.len() >= RECENT_MAX_BYTES { recent_trim(&p); } }
}
pub fn recent_drop(dir: &Path, kind: &str) {
    let p = dir.join(RECENT_FILE);
    let Some(s) = read_file(&p) else { return };
    let mark = format!("\t{}\t", kind);
    let mut kept = String::new();
    for chunk in s.split_inclusive('\n') {
        let line = chunk.strip_suffix('\n').unwrap_or(chunk);
        let drop_it = match line.find('\t') {
            Some(tab) => line[tab..].starts_with(&mark),
            None => false,
        };
        if !drop_it { kept.push_str(chunk); }
    }
    write_file_atomic(&p, &kept);
}
pub fn recent_cut(dir: &Path, since: i64, kinds: &str) -> i32 {
    let p = dir.join(RECENT_FILE);
    let Some(s) = read_file(&p) else { return 0 };
    let mut kept = String::new();
    let mut n = 0;
    for chunk in s.split_inclusive('\n') {
        let line = chunk.strip_suffix('\n').unwrap_or(chunk);
        let mut t: i64 = 0;
        for c in line.chars() { if c.is_ascii_digit() { t = t * 10 + (c as i64 - '0' as i64); } else { break; } }
        let kind_hit = match line.find('\t') {
            Some(tab) => {
                let rest = &line[tab + 1..];
                let kc = rest.chars().next();
                match kc {
                    Some(k) => kinds.contains(k) && (rest.len() == 1 || rest.as_bytes()[1] == b'\t'),
                    None => false,
                }
            }
            None => false,
        };
        if t >= since && kind_hit { n += 1; } else { kept.push_str(chunk); }
    }
    if n > 0 { write_file_atomic(&p, &kept); }
    n
}

// ---------------------------------------------------------------- 学習 (Learning)
pub type Counts = Vec<(U16, i32)>;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Undo {
    pub reading: U16,
    pub surface: U16,
    pub count: i32,
    pub day: i32,
    pub hidden: bool,
    pub left: U16,
    pub add: i32,
    pub evicted: U16,
    pub evicted_count: i32,
    pub snap: bool,
    pub ctx: Vec<(U16, Counts)>,
    pub pos: Vec<(U16, i32)>,
}

#[derive(Clone, Debug)]
struct Entry {
    reading: U16,
    counts: Counts,
    day: i32,
    ctx: Vec<(U16, Counts)>,
    pos: Vec<(U16, i32)>,
}

#[derive(Clone, Debug)]
pub struct Strong {
    pub reading: U16,
    pub surface: U16,
    pub count: i32,
    pub lid: i32,
}

pub struct Learning {
    file: PathBuf,
    stamp: Option<SystemTime>,
    loaded: bool,
    entries: Vec<Entry>,
    idx: HashMap<U16, usize>,
    version: u64,
    resolver: Option<std::rc::Rc<dyn Fn(&[u16], &[u16]) -> i32>>,
}

impl Learning {
    pub const MAX_READINGS: usize = 20000;
    pub const MAX_SURFACES: usize = 5;
    pub const TOP_MIN_COUNT: i32 = 2;
    pub const SAME_CTX_MIN: i32 = 2;
    pub const ANY_CTX_MIN: i32 = 6;
    pub const CTX_CHARS: usize = 2;
    pub const MAX_CONTEXTS: usize = 8;

    pub fn new(file: PathBuf) -> Self {
        Learning { file, stamp: None, loaded: false, entries: Vec::new(), idx: HashMap::new(), version: 0, resolver: None }
    }
    pub fn set_lid_resolver(&mut self, f: std::rc::Rc<dyn Fn(&[u16], &[u16]) -> i32>) { self.resolver = Some(f); }
    pub fn file(&self) -> &Path { &self.file }

    pub fn today() -> i32 {
        (UNIX_EPOCH.elapsed().map(|d| d.as_secs()).unwrap_or(0) / 86400) as i32
    }

    fn refresh(&mut self) {
        let t = stamp_of(&self.file);
        let exists = t.is_some();
        if self.loaded && (!exists || t == self.stamp) { return; }
        let Some(s) = read_or_missing(&self.file) else { return };
        self.loaded = true;
        self.stamp = t;
        self.entries.clear();
        self.version += 1;
        if !exists && s.is_empty() { self.reindex(); return; }
        let Some(v) = json::Parser::new(&s).parse() else { keep_broken(&self.file); self.reindex(); return };
        let Some(obj) = v.as_obj() else { keep_broken(&self.file); self.reindex(); return };
        let mut weighted = false;
        for (rk, rv) in obj {
            if rk == "@w" { weighted = true; continue; }
            let Some(ro) = rv.as_obj() else { continue };
            let mut e = Entry { reading: from_utf8(rk), counts: Vec::new(), day: 0, ctx: Vec::new(), pos: Vec::new() };
            for (ck, cv) in ro {
                if ck == "@c" {
                    if let Some(co) = cv.as_obj() {
                        for (lk, lv) in co {
                            let mut cs: Counts = Vec::new();
                            if let Some(lo) = lv.as_obj() {
                                for (x, xv) in lo { if let Some(n) = xv.as_num() { cs.push((from_utf8(x), n as i32)); } }
                            }
                            if !cs.is_empty() { e.ctx.push((from_utf8(lk), cs)); }
                        }
                    }
                    continue;
                }
                if ck == "@p" {
                    if let Some(po) = cv.as_obj() {
                        for (x, xv) in po { if let Some(n) = xv.as_num() { e.pos.push((from_utf8(x), n as i32)); } }
                    }
                    continue;
                }
                let Some(n) = cv.as_num() else { continue };
                if ck == "@day" { e.day = n as i32; } else { e.counts.push((from_utf8(ck), n as i32)); }
            }
            if !e.counts.is_empty() { self.entries.push(e); }
        }
        if !weighted {
            for e in &mut self.entries { for c in &mut e.counts { if c.1 > 0 { c.1 *= 2; } } }
        }
        self.reindex();
    }

    fn reindex(&mut self) {
        self.idx.clear();
        for (i, e) in self.entries.iter().enumerate() { self.idx.insert(e.reading.clone(), i); }
    }
    fn find(&self, reading: &[u16]) -> Option<&Entry> { self.idx.get(reading).map(|&i| &self.entries[i]) }
    fn find_mut(&mut self, reading: &[u16]) -> Option<&mut Entry> {
        let i = *self.idx.get(reading)?;
        Some(&mut self.entries[i])
    }

    fn save(&mut self) {
        if !self.loaded { return; }
        self.version += 1;
        let mut o = String::from("{\"@w\":{\"@w\":2}");
        for e in &self.entries {
            o.push(',');
            o.push_str(&json::quote(&to_utf8(&e.reading)));
            o.push_str(":{");
            for (i, c) in e.counts.iter().enumerate() {
                if i > 0 { o.push(','); }
                o.push_str(&json::quote(&to_utf8(&c.0)));
                o.push(':');
                o.push_str(&c.1.to_string());
            }
            if e.day > 0 { o.push_str(",\"@day\":"); o.push_str(&e.day.to_string()); }
            if !e.pos.is_empty() {
                o.push_str(",\"@p\":{");
                for (i, p) in e.pos.iter().enumerate() {
                    if i > 0 { o.push(','); }
                    o.push_str(&json::quote(&to_utf8(&p.0)));
                    o.push(':');
                    o.push_str(&p.1.to_string());
                }
                o.push('}');
            }
            if !e.ctx.is_empty() {
                o.push_str(",\"@c\":{");
                for (i, x) in e.ctx.iter().enumerate() {
                    if i > 0 { o.push(','); }
                    o.push_str(&json::quote(&to_utf8(&x.0)));
                    o.push_str(":{");
                    for (k, c) in x.1.iter().enumerate() {
                        if k > 0 { o.push(','); }
                        o.push_str(&json::quote(&to_utf8(&c.0)));
                        o.push(':');
                        o.push_str(&c.1.to_string());
                    }
                    o.push('}');
                }
                o.push('}');
            }
            o.push('}');
        }
        o.push('}');
        if write_file_atomic(&self.file, &o) { self.stamp = stamp_of(&self.file); }
    }

    fn left_key(left: &[u16]) -> U16 {
        if left.len() <= Self::CTX_CHARS { return left.to_vec(); }
        let mut st = left.len() - Self::CTX_CHARS;
        if (left[st] & 0xFC00) == 0xDC00 { st -= 1; }
        left[st..].to_vec()
    }
    fn note(e: &mut Entry, key: &[u16], surface: &[u16], add: i32) {
        let pos = e.ctx.iter().position(|x| x.0 == key);
        let mut idx = match pos {
            None => {
                if add < 0 { return; }
                e.ctx.push((key.to_vec(), Vec::new()));
                if e.ctx.len() > Self::MAX_CONTEXTS { e.ctx.remove(0); }
                e.ctx.len() - 1
            }
            Some(p) => {
                if add > 0 { let x = e.ctx.remove(p); e.ctx.push(x); e.ctx.len() - 1 } else { p }
            }
        };
        {
            let cs = &mut e.ctx[idx].1;
            match cs.iter_mut().find(|x| x.0 == surface) {
                None => { if add > 0 { cs.push((surface.to_vec(), add)); } }
                Some(c) => { c.1 += add; if c.1 <= 0 { let p = cs.iter().position(|x| x.0 == surface).unwrap(); cs.remove(p); } }
            }
        }
        if e.ctx[idx].1.is_empty() { e.ctx.remove(idx); idx = usize::MAX; }
        let _ = idx;
    }
    fn drop_ctx(e: &mut Entry, surface: &[u16]) {
        for x in &mut e.ctx { x.1.retain(|c| c.0 != surface); }
        e.ctx.retain(|x| !x.1.is_empty());
    }

    pub fn note_commit(&mut self, reading: &[u16], surface: &[u16], ctx: &[u16]) {
        let c: U16 = ctx.iter().map(|&ch| if ch == b'\t' as u16 || ch == b'\n' as u16 || ch == b'\r' as u16 { b' ' as u16 } else { ch }).collect();
        if let Some(dir) = self.file.parent() {
            recent_add(dir, &["C".into(), to_utf8(reading), to_utf8(surface), to_utf8(&c)]);
        }
    }

    pub fn record(&mut self, reading: &[u16], surface: &[u16], add: i32, left: &[u16], lid: i32) -> Undo {
        if reading.is_empty() || surface.is_empty() { return Undo::default(); }
        self.refresh();
        let mut undo = Undo { reading: reading.to_vec(), surface: surface.to_vec(), ..Default::default() };
        undo.left = Self::left_key(left);
        undo.add = add;
        undo.snap = true;
        if let Some(e) = self.find(reading) {
            undo.day = e.day;
            for c in &e.counts { if c.0 == surface { undo.count = c.1; } }
            undo.ctx = e.ctx.clone();
            undo.pos = e.pos.clone();
        }
        if let Some(dir) = self.file.parent() {
            let lf: U16 = undo.left.iter().map(|&ch| if ch == b'\t' as u16 || ch == b'\n' as u16 || ch == b'\r' as u16 { b' ' as u16 } else { ch }).collect();
            recent_add(dir, &["L".into(), to_utf8(reading), to_utf8(surface), undo.count.to_string(), undo.day.to_string(), add.to_string(), to_utf8(&lf)]);
        }
        if self.find(reading).is_none() {
            if self.entries.len() >= Self::MAX_READINGS {
                let t = Self::today();
                let score = |x: &Entry| {
                    let m = x.counts.iter().map(|c| c.1).max().unwrap_or(0);
                    (m / 2).min(20) * 30 - if x.day > 0 { t - x.day } else { 365 }
                };
                if let Some((wi, _)) = self.entries.iter().enumerate().min_by_key(|(_, x)| score(x)) {
                    self.entries.remove(wi);
                    self.reindex();
                }
            }
            self.entries.push(Entry { reading: reading.to_vec(), counts: Vec::new(), day: 0, ctx: Vec::new(), pos: Vec::new() });
            let n = self.entries.len() - 1;
            self.idx.insert(reading.to_vec(), n);
        }
        let evicted;
        {
            let e = self.find_mut(reading).unwrap();
            e.day = Self::today();
            Self::note(e, &undo.left.clone(), surface, add);
            if lid >= 0 {
                match e.pos.iter_mut().find(|x| x.0 == surface) {
                    Some(p) => p.1 = lid,
                    None => e.pos.push((surface.to_vec(), lid)),
                }
            }
            if let Some(c) = e.counts.iter_mut().find(|c| c.0 == surface) {
                c.1 = if c.1 < 0 { add } else { c.1 + add };
                evicted = None;
            } else {
                let npos = e.counts.iter().filter(|c| c.1 > 0).count();
                let mut ev: Option<(U16, i32)> = None;
                if npos >= Self::MAX_SURFACES {
                    if let Some(w) = e.counts.iter().filter(|c| c.1 > 0).min_by_key(|c| c.1) {
                        ev = Some((w.0.clone(), w.1));
                    }
                    if let Some((g, _)) = &ev {
                        Self::drop_ctx(e, g);
                        let g2 = g.clone();
                        e.pos.retain(|x| x.0 != g2);
                        e.counts.retain(|c| &c.0 != g);
                    }
                }
                e.counts.push((surface.to_vec(), add));
                evicted = ev;
            }
        }
        if let Some((g, gc)) = evicted { undo.evicted = g; undo.evicted_count = gc; }
        self.save();
        undo
    }

    pub fn confirm(&mut self, reading: &[u16], surface: &[u16], left: &[u16], lid: i32) -> Undo {
        self.refresh();
        let Some(e) = self.find(reading) else { return Undo::default() };
        for c in &e.counts { if c.0 == surface && c.1 < 0 { return Undo::default(); } }
        self.record(reading, surface, 1, left, lid)
    }

    pub fn restore(&mut self, us: &[Undo]) {
        self.refresh();
        let mut changed = false;
        for u in us {
            if u.reading.is_empty() { continue; }
            if self.find(&u.reading).is_none() {
                if u.count == 0 { continue; }
                self.entries.push(Entry { reading: u.reading.clone(), counts: Vec::new(), day: 0, ctx: Vec::new(), pos: Vec::new() });
                let n = self.entries.len() - 1;
                self.idx.insert(u.reading.clone(), n);
            }
            let mut remove_entry = false;
            {
                let e = self.find_mut(&u.reading).unwrap();
                let it = e.counts.iter().position(|c| c.0 == u.surface);
                if let Some(p) = it { if e.counts[p].1 < 0 && !u.hidden { continue; } }
                match it {
                    Some(p) => { if u.count != 0 { e.counts[p].1 = u.count; } else { e.counts.remove(p); } }
                    None => { if u.count != 0 { e.counts.push((u.surface.clone(), u.count)); } }
                }
                if !u.evicted.is_empty() { e.counts.push((u.evicted.clone(), u.evicted_count)); }
                e.day = u.day;
                if u.snap { e.ctx = u.ctx.clone(); e.pos = u.pos.clone(); }
                else if u.add != 0 { let lk = u.left.clone(); Self::note(e, &lk, &u.surface.clone(), -u.add); }
                changed = true;
                remove_entry = e.counts.is_empty();
            }
            if remove_entry {
                if let Some(&i) = self.idx.get(&u.reading) { self.entries.remove(i); self.reindex(); }
            }
        }
        if changed { self.save(); }
    }

    pub fn demote(&mut self, reading: &[u16], surface: &[u16], left: &[u16], n: i32) -> Undo {
        if reading.is_empty() || surface.is_empty() || n <= 0 { return Undo::default(); }
        self.refresh();
        let mut undo = Undo { reading: reading.to_vec(), surface: surface.to_vec(), ..Default::default() };
        let mut remove_entry = false;
        {
            let Some(e) = self.find_mut(reading) else { return Undo::default() };
            let Some(p) = e.counts.iter().position(|c| c.0 == surface) else { return Undo::default() };
            if e.counts[p].1 <= 0 { return Undo::default(); }
            undo.count = e.counts[p].1;
            undo.day = e.day;
            undo.left = Self::left_key(left);
            let cxp = e.ctx.iter().position(|x| x.0 == undo.left);
            if let Some(cxp) = cxp {
                if let Some(c) = e.ctx[cxp].1.iter().find(|x| x.0 == surface) {
                    undo.add = -n.min(c.1);
                    let (lk, sf, ad) = (undo.left.clone(), surface.to_vec(), undo.add);
                    Self::note(e, &lk, &sf, ad);
                }
            }
            // note で e の借用が続くため再取得
            let e = self.find_mut(reading).unwrap();
            let p = e.counts.iter().position(|c| c.0 == surface).unwrap();
            e.counts[p].1 -= n;
            if e.counts[p].1 <= 0 { e.counts.remove(p); remove_entry = e.counts.is_empty(); }
        }
        if remove_entry {
            if let Some(&i) = self.idx.get(reading) { self.entries.remove(i); self.reindex(); }
        }
        self.save();
        undo
    }

    pub fn hide(&mut self, reading: &[u16], surface: &[u16], keep_recent: bool) {
        if reading.is_empty() || surface.is_empty() { return; }
        self.refresh();
        let mut prev = 0;
        let mut day = 0;
        if let Some(e) = self.find(reading) {
            day = e.day;
            for c in &e.counts { if c.0 == surface { prev = c.1; } }
        }
        if keep_recent {
            if let Some(dir) = self.file.parent() {
                recent_add(dir, &["H".into(), to_utf8(reading), to_utf8(surface), prev.to_string(), day.to_string()]);
            }
        }
        if self.find(reading).is_none() {
            self.entries.push(Entry { reading: reading.to_vec(), counts: Vec::new(), day: 0, ctx: Vec::new(), pos: Vec::new() });
            let n = self.entries.len() - 1;
            self.idx.insert(reading.to_vec(), n);
        }
        let e = self.find_mut(reading).unwrap();
        Self::drop_ctx(e, surface);
        match e.counts.iter_mut().find(|c| c.0 == surface) {
            Some(c) => c.1 = -1,
            None => e.counts.push((surface.to_vec(), -1)),
        }
        self.save();
    }

    pub fn hidden(&mut self, reading: &[u16], surface: &[u16]) -> bool {
        self.refresh();
        match self.find(reading) {
            Some(e) => e.counts.iter().any(|c| c.0 == surface && c.1 < 0),
            None => false,
        }
    }

    pub fn forget(&mut self, reading: &[u16], surface: &[u16]) {
        self.refresh();
        let Some(&i) = self.idx.get(reading) else { return };
        let e = &mut self.entries[i];
        let Some(p) = e.counts.iter().position(|c| c.0 == surface) else { return };
        e.counts.remove(p);
        Self::drop_ctx(e, surface);
        let empty = e.counts.is_empty();
        if empty { self.entries.remove(i); self.reindex(); }
        self.save();
    }

    pub fn apply_order(&mut self, reading: &[u16], cands: &[U16]) -> Vec<U16> {
        self.refresh();
        let Some(e) = self.find(reading) else { return cands.to_vec() };
        let counts = e.counts.clone();
        let count = |s: &[u16]| counts.iter().find(|c| c.0 == s).map(|c| c.1).unwrap_or(0);
        let mut boosted: Vec<U16> = Vec::new();
        let mut rest: Vec<U16> = Vec::new();
        for c in cands {
            let n = count(c);
            if n < 0 { continue; }
            if n >= Self::TOP_MIN_COUNT { boosted.push(c.clone()); } else { rest.push(c.clone()); }
        }
        boosted.sort_by(|a, b| count(b).cmp(&count(a))); // stable sort 相当 (Vec::sort_by は安定)
        boosted.extend(rest);
        let english = |s: &[u16]| s.len() >= 2 && s.iter().all(|&c| (b'a' as u16..=b'z' as u16).contains(&c) || c == b'-' as u16);
        let kana = reading.iter().any(|&c| c >= 0x80);
        if kana && boosted.len() >= 2 && english(&boosted[0]) { boosted.swap(0, 1); }
        boosted
    }

    pub fn completions(&mut self, prefix: &[u16], max: usize) -> Vec<U16> {
        let mut out: Vec<U16> = Vec::new();
        if prefix.is_empty() { return out; }
        self.refresh();
        let mut all: Vec<(U16, i32)> = Vec::new();
        for e in &self.entries {
            if e.reading.len() > prefix.len() && e.reading[..prefix.len()] == *prefix {
                for c in &e.counts { if c.1 >= 4 { all.push(c.clone()); } }
            }
        }
        all.sort_by(|a, b| b.1.cmp(&a.1));
        for c in all {
            if out.len() >= max { break; }
            if !out.contains(&c.0) { out.push(c.0); }
        }
        out
    }

    pub fn counts(&mut self, reading: &[u16]) -> Counts {
        self.refresh();
        self.find(reading).map(|e| e.counts.clone()).unwrap_or_default()
    }

    fn lid_in(&self, e: &Entry, surface: &[u16]) -> i32 {
        for p in &e.pos { if p.0 == surface { return p.1; } }
        match &self.resolver { Some(f) => f(&e.reading, surface), None => -1 }
    }
    pub fn lid_of(&mut self, reading: &[u16], surface: &[u16]) -> i32 {
        self.refresh();
        // resolver は &self を要求するため clone してから呼ぶ
        let (entry, resolver) = (self.find(reading).cloned(), self.resolver.clone());
        match entry {
            Some(e) => {
                for p in &e.pos { if p.0 == surface { return p.1; } }
                match resolver { Some(f) => f(&e.reading, surface), None => -1 }
            }
            None => match resolver { Some(f) => f(reading, surface), None => -1 },
        }
    }

    pub fn top(&mut self, reading: &[u16], left: &[u16], lid: i32) -> U16 {
        self.refresh();
        let Some(e) = self.find(reading).cloned() else { return Vec::new() };
        let want = group_of_lid(lid);
        let lid_of_s = |s: &[u16]| -> i32 {
            for p in &e.pos { if p.0 == s { return p.1; } }
            -1 // resolver は top の中では使わない (元の lid_in は使うが、借用制約のため記録済み品詞のみ。差異は報告に明記)
        };
        let same = |s: &[u16]| {
            if want == 0 { return true; }
            let g = group_of_lid(lid_of_s(s));
            g == 0 || g == want
        };
        let key = Self::left_key(left);
        let mut here: Option<&Counts> = None;
        let mut all = 0i32;
        for x in &e.ctx { if x.0 == key { here = Some(&x.1); for c in &x.1 { all += c.1; } } }
        let mut total = 0i32;
        let mut total_all = 0i32;
        for c in &e.counts { if c.1 > 0 { total_all += c.1; if same(&c.0) { total += c.1; } } }
        let mut best: U16 = Vec::new();
        let mut n = Self::TOP_MIN_COUNT - 1;
        for c in &e.counts {
            if c.1 <= n { continue; }
            let mut mine = 0;
            if let Some(h) = here { for x in h { if x.0 == c.0 { mine = x.1; } } }
            let against = all - mine;
            let dominant = (total_all - c.1) * 4 <= c.1 || (want != 0 && same(&c.0) && (total - c.1) * 4 <= c.1);
            if mine - against >= Self::SAME_CTX_MIN || (c.1 >= Self::ANY_CTX_MIN && against == 0 && dominant) {
                n = c.1;
                best = c.0.clone();
            }
        }
        best
    }

    pub fn strong(&mut self, min: i32) -> Vec<Strong> {
        self.refresh();
        let entries = self.entries.clone();
        let mut out = Vec::new();
        for e in &entries {
            let lids: Vec<i32> = e.counts.iter().map(|c| if c.1 > 0 { self.lid_in_static(e, &c.0) } else { -1 }).collect();
            for (i, c) in e.counts.iter().enumerate() {
                if c.1 < min { continue; }
                let g = group_of_lid(lids[i]);
                let mut total = 0;
                for (k, o) in e.counts.iter().enumerate() {
                    if o.1 > 0 && (g == 0 || group_of_lid(lids[k]) == 0 || group_of_lid(lids[k]) == g) { total += o.1; }
                }
                if (total - c.1) * 4 <= c.1 { out.push(Strong { reading: e.reading.clone(), surface: c.0.clone(), count: c.1, lid: lids[i] }); }
            }
        }
        out
    }
    fn lid_in_static(&self, e: &Entry, surface: &[u16]) -> i32 {
        for p in &e.pos { if p.0 == surface { return p.1; } }
        match &self.resolver { Some(f) => f(&e.reading, surface), None => -1 }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.idx.clear();
        self.loaded = true;
        self.save();
        let mut pk = self.file.as_os_str().to_owned();
        pk.push(".pre-kana");
        let _ = std::fs::remove_file(PathBuf::from(pk));
    }

    pub fn release_kana(&mut self, kana_first: &dyn Fn(&[u16]) -> bool) -> i32 {
        self.refresh();
        let mut fixed = 0;
        for ei in 0..self.entries.len() {
            let (reading, mut best, mut kana) = {
                let e = &self.entries[ei];
                if e.reading.len() > 5 { continue; }
                let mut best = 0; let mut kana = 0;
                for c in &e.counts {
                    if c.0 == e.reading { kana = c.1; } else { best = best.max(c.1); }
                }
                (e.reading.clone(), best, kana)
            };
            let _ = (&mut best, &mut kana);
            if best < Self::TOP_MIN_COUNT || kana > best || !kana_first(&reading) { continue; }
            let e = &mut self.entries[ei];
            let mut gone: Vec<U16> = Vec::new();
            for c in &mut e.counts { if c.0 != e.reading && c.1 > 0 { c.1 = 0; gone.push(c.0.clone()); } }
            for g in gone { Self::drop_ctx(e, &g); }
            fixed += 1;
        }
        if fixed > 0 { self.save(); }
        fixed
    }

    pub fn version(&mut self) -> u64 { self.refresh(); self.version }

    pub fn last_day(&mut self, reading: &[u16]) -> i32 {
        self.refresh();
        self.find(reading).map(|e| e.day).unwrap_or(0)
    }

    /// 使う人の印: 表記 -> 1 + (段階-1)*4 + 時期。
    pub fn marks(&mut self) -> Vec<(U16, i32)> {
        self.refresh();
        #[derive(Default)]
        struct Acc { n: i32, day: i32 }
        let mut by: HashMap<U16, Acc> = HashMap::new();
        for e in &self.entries {
            let mut kana = 0;
            for c in &e.counts { if c.0 == e.reading { kana = kana.max(c.1); } }
            for c in &e.counts {
                let n = c.1 - kana;
                if n <= 0 { continue; }
                let mut r = e.reading.clone();
                let mut s = c.0.clone();
                while !r.is_empty() && !s.is_empty() && r[r.len() - 1] == s[s.len() - 1]
                    && (0x3041..=0x3096).contains(&r[r.len() - 1]) { r.pop(); s.pop(); }
                if s.is_empty() { continue; }
                let a = by.entry(s).or_default();
                a.n += (n + 1) / 2;
                a.day = a.day.max(e.day);
            }
        }
        let t = Self::today();
        let mut out = Vec::new();
        for (s, a) in by {
            let c = if a.n == 1 { 1 } else if a.n == 2 { 2 } else if a.n <= 4 { 3 } else if a.n <= 9 { 4 } else if a.n <= 19 { 5 } else { 6 };
            let age = if a.day > 0 { t - a.day } else { 9999 };
            let r = if age <= 7 { 0 } else if age <= 30 { 1 } else if age <= 183 { 2 } else { 3 };
            out.push((s, 1 + (c - 1) * 4 + r));
        }
        out
    }
}

// ---------------------------------------------------------------- 書き方の記録 (StyleLog)
/// style.h の StyleModel::Count 相当 (w = [丁寧, 崩れ] の重みつき回数、n = 回数)。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StyleCount {
    pub w: [f32; 2],
    pub n: i32,
}

pub struct StyleLog {
    file: PathBuf,
    stamp: Option<SystemTime>,
    loaded: bool,
    m: HashMap<U16, StyleCount>,
    version: u64,
}

impl StyleLog {
    pub const MAX_SURFACES: usize = 20000;
    pub fn new(file: PathBuf) -> Self {
        StyleLog { file, stamp: None, loaded: false, m: HashMap::new(), version: 0 }
    }
    fn refresh(&mut self) {
        let t = stamp_of(&self.file);
        let exists = t.is_some();
        if self.loaded && (!exists || t == self.stamp) { return; }
        let Some(s) = read_or_missing(&self.file) else { return };
        self.loaded = true;
        self.stamp = t;
        self.m.clear();
        self.version += 1;
        if !exists && s.is_empty() { return; }
        let Some(v) = json::Parser::new(&s).parse() else { keep_broken(&self.file); return };
        let Some(obj) = v.as_obj() else { keep_broken(&self.file); return };
        let mut sv: Option<&Vec<(String, json::Value)>> = None;
        for (k, val) in obj { if k == "s" { sv = val.as_obj(); } }
        if let Some(sv) = sv {
            for (k, val) in sv {
                let Some(a) = val.as_arr() else { continue };
                if a.len() != 3 { continue; }
                let c = StyleCount { w: [a[0].as_num().unwrap_or(0.0) as f32, a[1].as_num().unwrap_or(0.0) as f32], n: a[2].as_num().unwrap_or(0.0) as i32 };
                if c.n > 0 { self.m.insert(from_utf8(k), c); }
            }
            return;
        }
        // 古い形 (アプリ -> 表記 -> 回数): 合計を半々として読む
        for (_, val) in obj {
            let Some(inner) = val.as_obj() else { continue };
            for (k, cv) in inner {
                let Some(n) = cv.as_num() else { continue };
                if n <= 0.0 { continue; }
                let e = self.m.entry(from_utf8(k)).or_default();
                e.n += n as i32;
                e.w[0] = e.n as f32 * 0.70710678;
                e.w[1] = e.n as f32 * 0.70710678;
            }
        }
    }
    fn save(&mut self) {
        if !self.loaded { return; }
        self.version += 1;
        let mut o = String::from("{\"v\":2,\"s\":{");
        let mut keys: Vec<&U16> = self.m.keys().collect();
        keys.sort();
        for (i, k) in keys.iter().enumerate() {
            if i > 0 { o.push(','); }
            let c = self.m[*k];
            o.push_str(&json::quote(&to_utf8(k)));
            o.push_str(&format!(":[{:.3},{:.3},{}]", c.w[0], c.w[1], c.n));
        }
        o.push_str("}}");
        if write_file_atomic(&self.file, &o) { self.stamp = stamp_of(&self.file); }
    }
    pub fn record(&mut self, surface: &[u16], w: [f32; 2]) {
        if surface.is_empty() { return; }
        self.refresh();
        if !self.m.contains_key(surface) && self.m.len() >= Self::MAX_SURFACES {
            if let Some(low) = self.m.iter().min_by_key(|(_, v)| v.n).map(|(k, _)| k.clone()) { self.m.remove(&low); }
        }
        let mut full = false;
        {
            let k = self.m.entry(surface.to_vec()).or_default();
            if k.n >= 1_000_000 { full = true; } else { k.w[0] += w[0]; k.w[1] += w[1]; k.n += 1; }
        }
        if full { return; }
        if let Some(dir) = self.file.parent() {
            recent_add(dir, &["S".into(), to_utf8(surface), format!("{:.3}", w[0]), format!("{:.3}", w[1])]);
        }
        self.save();
    }
    pub fn unrecord(&mut self, items: &[(U16, [f32; 2])]) {
        self.refresh();
        let mut changed = false;
        for (s, w) in items {
            let Some(k) = self.m.get_mut(s) else { continue };
            changed = true;
            k.n -= 1;
            if k.n <= 0 { self.m.remove(s); continue; }
            k.w[0] = (k.w[0] - w[0]).max(0.0);
            k.w[1] = (k.w[1] - w[1]).max(0.0);
        }
        if changed { self.save(); }
    }
    pub fn counts(&mut self) -> &HashMap<U16, StyleCount> { self.refresh(); &self.m }
    pub fn version(&mut self) -> u64 { self.refresh(); self.version }
    pub fn clear(&mut self) { self.refresh(); self.m.clear(); self.save(); }
}

// ---------------------------------------------------------------- ユーザー辞書 (UserDict)
/// 英字の語の綴り (英字・数字を小文字に、間の空白は除く)。英字・数字・空白以外を含めば空。
pub fn latin_key(w: &[u16]) -> U16 {
    let mut k = Vec::new();
    for &c in w {
        if (b'a' as u16..=b'z' as u16).contains(&c) || (b'0' as u16..=b'9' as u16).contains(&c) { k.push(c); }
        else if (b'A' as u16..=b'Z' as u16).contains(&c) { k.push(c + 32); }
        else if c != b' ' as u16 { return Vec::new(); }
    }
    k
}

#[derive(Clone, Debug, PartialEq)]
pub struct DictEntry {
    pub reading: U16,
    pub word: U16,
    pub pos: String,
    pub group: String,
    pub pack: String,
}
impl Default for DictEntry {
    fn default() -> Self { DictEntry { reading: Vec::new(), word: Vec::new(), pos: "noun".into(), group: String::new(), pack: String::new() } }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DictFormat { MsIme, Csv, Mozc }

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DictForm {
    pub reading: U16,
    pub surface: U16,
    pub tmpl_reading: U16,
    pub tmpl_surface: U16,
    pub whole: bool,
    pub bonus: i32, // -1 = Converter の既定
    pub lid: i32,   // -1 = 分からない
}

struct Godan { last: u16, r: &'static str, s: &'static str, sufs: &'static str }
const GODAN: &[Godan] = &[
    Godan { last: 'う' as u16, r: "かう", s: "買う", sufs: "わいうえおっ" },
    Godan { last: 'く' as u16, r: "かく", s: "書く", sufs: "かきくけこい" },
    Godan { last: 'ぐ' as u16, r: "およぐ", s: "泳ぐ", sufs: "がぎぐげごい" },
    Godan { last: 'す' as u16, r: "はなす", s: "話す", sufs: "さしすせそ" },
    Godan { last: 'つ' as u16, r: "まつ", s: "待つ", sufs: "たちつてとっ" },
    Godan { last: 'ぬ' as u16, r: "しぬ", s: "死ぬ", sufs: "なにぬねのん" },
    Godan { last: 'ぶ' as u16, r: "あそぶ", s: "遊ぶ", sufs: "ばびぶべぼん" },
    Godan { last: 'む' as u16, r: "よむ", s: "読む", sufs: "まみむめもん" },
    Godan { last: 'る' as u16, r: "はしる", s: "走る", sufs: "らりるれろっ" },
];
const ICHIDAN: &[&str] = &["", "る", "れ", "ろ", "よ"];
const ADJ: &[&str] = &["い", "く", "かっ", "けれ", "かろ", "き", "さ", "そう"];
fn godan_of(last: u16) -> Option<&'static Godan> { GODAN.iter().find(|g| g.last == last) }
fn ends_with(s: &[u16], t: &[u16]) -> bool { s.len() >= t.len() && &s[s.len() - t.len()..] == t }

pub struct UserDict {
    file: PathBuf,
    stamp: Option<SystemTime>,
    loaded: bool,
    entries: Vec<DictEntry>,
    version: u32,
}

impl UserDict {
    pub fn new(file: PathBuf) -> Self {
        UserDict { file, stamp: None, loaded: false, entries: Vec::new(), version: 0 }
    }
    /// ファイルが変わっていれば読み直す。変わったら true。
    pub fn refresh(&mut self) -> bool {
        let t = stamp_of(&self.file);
        let exists = t.is_some();
        if self.loaded && (!exists || t == self.stamp) { return false; }
        let Some(s) = read_or_missing(&self.file) else { return false };
        self.loaded = true;
        self.stamp = t;
        self.entries.clear();
        self.version += 1;
        if !exists && s.is_empty() { return true; }
        let Some(v) = json::Parser::new(&s).parse() else { keep_broken(&self.file); return true };
        let Some(arr) = v.as_arr() else { keep_broken(&self.file); return true };
        for o in arr {
            if o.as_obj().is_none() { continue; }
            let e = DictEntry {
                reading: from_utf8(&o.get_str("r")),
                word: from_utf8(&o.get_str("w")),
                pos: o.get_str_or("p", "noun"),
                group: o.get_str("g"),
                pack: o.get_str("k"),
            };
            if !e.reading.is_empty() && !e.word.is_empty() { self.entries.push(e); }
        }
        true
    }
    pub fn entries(&self) -> &[DictEntry] { &self.entries }
    pub fn version(&self) -> u32 { self.version }

    fn reading_key(r: &[u16]) -> U16 { to_halfwidth_ascii(r) }
    pub fn lookup(&self, reading: &[u16]) -> Vec<U16> {
        let k = Self::reading_key(reading);
        self.entries.iter().filter(|e| Self::reading_key(&e.reading) == k).map(|e| e.word.clone()).collect()
    }

    pub fn pos_from_label(label: &[u16]) -> (String, String) {
        let l = to_utf8(label);
        let has = |s: &str| l.contains(s);
        if has("短縮") { return ("abbrev".into(), "".into()); }
        if has("顔文字") { return ("kaomoji".into(), "".into()); }
        if has("人名") || l == "姓" || l == "名" { return ("person".into(), "".into()); }
        if has("地名") { return ("place".into(), "".into()); }
        if has("組織") || has("固有名詞") { return ("org".into(), "".into()); }
        if has("感動詞") { return ("interjection".into(), "".into()); }
        if has("形容動詞") { return ("noun".into(), "".into()); }
        if has("一段") { return ("verb".into(), "ichidan".into()); }
        if has("動詞") { return ("verb".into(), "godan".into()); }
        if has("形容詞") { return ("adjective".into(), "".into()); }
        if has("副詞") { return ("adverb".into(), "".into()); }
        ("noun".into(), "".into())
    }

    fn ms_label(e: &DictEntry) -> U16 {
        match e.pos.as_str() {
            "person" => u16s("人名"),
            "place" => u16s("地名その他"),
            "org" => u16s("固有名詞"),
            "abbrev" => u16s("短縮よみ"),
            "kaomoji" => u16s("顔文字"),
            "verb" => u16s(if e.group == "ichidan" { "動詞一段" } else { "動詞五段" }),
            "adjective" => u16s("形容詞"),
            "adverb" => u16s("副詞"),
            "interjection" => u16s("感動詞"),
            _ => u16s("名詞"),
        }
    }
    fn mozc_label(e: &DictEntry) -> U16 {
        if e.pos == "place" { return u16s("地名"); }
        if e.pos == "org" { return u16s("組織"); }
        if e.pos == "verb" {
            if e.group == "ichidan" { return u16s("動詞一段"); }
            const ROW: &[(u16, &str)] = &[
                ('う' as u16, "ワ"), ('く' as u16, "カ"), ('ぐ' as u16, "ガ"), ('す' as u16, "サ"), ('つ' as u16, "タ"),
                ('ぬ' as u16, "ナ"), ('ぶ' as u16, "バ"), ('む' as u16, "マ"), ('る' as u16, "ラ"),
            ];
            if let Some(last) = e.reading.last() {
                for (c, row) in ROW { if last == c { return u16s(&format!("動詞{}行五段", row)); } }
            }
            return u16s("動詞ラ行五段");
        }
        Self::ms_label(e)
    }

    pub fn export_text(&self) -> U16 {
        let mut o = u16s("!Microsoft IME Dictionary Tool\r\n!shunti IME から書き出し\r\n");
        for e in &self.entries {
            o.extend(&e.reading); o.push(b'\t' as u16); o.extend(&e.word); o.push(b'\t' as u16);
            o.extend(Self::ms_label(e)); o.extend(u16s("\r\n"));
        }
        o
    }

    /// 書き出しのバイト列と語数。(bytes, count)
    pub fn export_bytes(&self, f: DictFormat, pack: Option<&str>) -> (Vec<u8>, i32) {
        let mut count = 0;
        let want = |e: &DictEntry| pack.is_none() || e.pack == pack.unwrap();
        if f == DictFormat::MsIme {
            let mut o = u16s("!Microsoft IME Dictionary Tool\r\n!shunti IME から書き出し\r\n");
            if let Some(p) = pack { if !p.is_empty() { o.extend(u16s(&format!("!パック: {}\r\n", p))); } }
            for e in &self.entries {
                if !want(e) { continue; }
                o.extend(&e.reading); o.push(b'\t' as u16); o.extend(&e.word); o.push(b'\t' as u16);
                o.extend(Self::ms_label(e)); o.extend(u16s("\r\n"));
                count += 1;
            }
            let mut b: Vec<u8> = vec![0xFF, 0xFE];
            for c in o { b.push((c & 0xFF) as u8); b.push((c >> 8) as u8); }
            return (b, count);
        }
        fn csv(s: &[u16]) -> String {
            let t = to_utf8(s);
            if !t.contains([',', '"', '\r', '\n']) { return t; }
            let mut q = String::from("\"");
            for c in t.chars() { if c == '"' { q.push('"'); } q.push(c); }
            q.push('"');
            q
        }
        let mut o: Vec<u8> = if f == DictFormat::Csv { vec![0xEF, 0xBB, 0xBF] } else { Vec::new() };
        for e in &self.entries {
            if !want(e) { continue; }
            let line = if f == DictFormat::Csv {
                format!("{},{},{}\r\n", csv(&e.reading), csv(&e.word), to_utf8(&Self::ms_label(e)))
            } else {
                format!("{}\t{}\t{}\t\n", to_utf8(&e.reading), to_utf8(&e.word), to_utf8(&Self::mozc_label(e)))
            };
            o.extend(line.as_bytes());
            count += 1;
        }
        (o, count)
    }

    pub fn packs(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for e in &self.entries { if !e.pack.is_empty() && !out.contains(&e.pack) { out.push(e.pack.clone()); } }
        out
    }

    fn pair_key(r: &[u16], w: &[u16]) -> U16 {
        let mut k = r.to_vec(); k.push(0); k.extend(w); k
    }
    pub fn move_to_pack(&mut self, items: &[(U16, U16)], pack: &str) -> i32 {
        self.refresh();
        let want: HashSet<U16> = items.iter().map(|(r, w)| Self::pair_key(r, w)).collect();
        let mut n = 0;
        for e in &mut self.entries {
            if e.pack != pack && want.contains(&Self::pair_key(&e.reading, &e.word)) { e.pack = pack.to_string(); n += 1; }
        }
        if n > 0 { self.save(); }
        n
    }
    pub fn rename_pack(&mut self, from: &str, to: &str) {
        self.refresh();
        if from.is_empty() || to.is_empty() || from == to { return; }
        let mut any = false;
        for e in &mut self.entries { if e.pack == from { e.pack = to.to_string(); any = true; } }
        if any { self.save(); }
    }
    pub fn remove_pack(&mut self, name: &str, with_words: bool) -> i32 {
        self.refresh();
        if name.is_empty() { return 0; }
        let n = self.entries.iter().filter(|e| e.pack == name).count() as i32;
        if n == 0 { return 0; }
        if with_words { self.entries.retain(|e| e.pack != name); }
        else { for e in &mut self.entries { if e.pack == name { e.pack.clear(); } } }
        self.save();
        n
    }

    pub fn import_text(&mut self, text: &[u16], pack: &str) -> (i32, Vec<String>) {
        self.refresh();
        let mut have: HashSet<U16> = self.entries.iter().map(|e| Self::pair_key(&e.reading, &e.word)).collect();
        let mut ok = 0;
        let mut bad: Vec<String> = Vec::new();
        let text_s = to_utf8(text);
        for (ln, raw_line) in text_s.split('\n').enumerate() {
            let line_no = ln + 1;
            let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
            let line = line.strip_prefix('\u{FEFF}').unwrap_or(line);
            if line.is_empty() || line.starts_with('!') || line.starts_with('#') { continue; }
            let line16 = from_utf8(line);
            let f: Vec<U16> = if line.contains('\t') {
                line.split('\t').map(from_utf8).collect()
            } else {
                // CSV ("" で囲んだ欄対応)
                let mut fields: Vec<U16> = Vec::new();
                let mut cur: Vec<char> = Vec::new();
                let mut q = false;
                let chars: Vec<char> = line.chars().collect();
                let mut k = 0;
                while k < chars.len() {
                    let c = chars[k];
                    if q {
                        if c == '"' && k + 1 < chars.len() && chars[k + 1] == '"' { cur.push(c); k += 1; }
                        else if c == '"' { q = false; }
                        else { cur.push(c); }
                    } else if c == '"' && cur.is_empty() { q = true; }
                    else if c == ',' { fields.push(u16s(&cur.iter().collect::<String>())); cur.clear(); }
                    else { cur.push(c); }
                    k += 1;
                }
                fields.push(u16s(&cur.iter().collect::<String>()));
                fields
            };
            let _ = line16;
            if f.len() < 2 { bad.push(format!("{} 行目: 読みと語句がありません", line_no)); continue; }
            let empty: U16 = Vec::new();
            let pg = Self::pos_from_label(if f.len() >= 3 { &f[2] } else { &empty });
            match self.make_entry(&f[0], &f[1], &pg.0, &pg.1, pack) {
                Ok(en) => {
                    if !have.insert(Self::pair_key(&en.reading, &en.word)) { continue; }
                    self.entries.push(en);
                    ok += 1;
                }
                Err(err) => {
                    if bad.len() < 1000 { bad.push(format!("{} 行目 ({}): {}", line_no, to_utf8(&f[0]), err)); }
                }
            }
        }
        if ok > 0 { self.save(); }
        (ok, bad)
    }

    fn make_entry(&self, reading0: &[u16], word0: &[u16], pos: &str, group: &str, pack: &str) -> Result<DictEntry, String> {
        fn trim(s: &[u16]) -> U16 {
            let mut v = s.to_vec();
            while matches!(v.last(), Some(&c) if c == b' ' as u16 || c == 0x3000) { v.pop(); }
            let mut i = 0;
            while i < v.len() && (v[i] == b' ' as u16 || v[i] == 0x3000) { i += 1; }
            v[i..].to_vec()
        }
        let mut r = to_hiragana(&trim(reading0));
        let w = trim(word0);
        let mut p = pos.to_string();
        if p == "latin" || (r.is_empty() && latin_key(&w).len() >= 2) {
            if latin_key(&w).len() < 2 {
                return Err("英字の語は、英字・数字 (と語の間の空白) で 2 字以上入れてください (例: ShuntiLettuce)".into());
            }
            r = latin_key(&w);
            p = "latin".into();
        }
        if r.is_empty() || w.is_empty() { return Err("読みと単語を入れてください".into()); }
        for &c in &r {
            if (0x4E00..=0x9FFF).contains(&c) || c == 0x3005 { return Err("読みに漢字は使えません。ひらがなで入れてください".into()); }
            if c < 0x20 { return Err("読みに使えない字があります".into()); }
        }
        if p == "verb" {
            if group == "ichidan" {
                if !ends_with(&r, &u16s("る")) || !ends_with(&w, &u16s("る")) {
                    return Err("一段の動詞は「る」で終わる形で入れてください (例: たべる / 食べる)".into());
                }
            } else if godan_of(*r.last().unwrap()).is_none() || w.last() != r.last() {
                return Err("動詞は終止形で、読みと単語の最後を同じかなにしてください (例: ぐぐる / ググる)".into());
            }
        } else if p == "adjective" {
            if !ends_with(&r, &u16s("い")) || !ends_with(&w, &u16s("い")) {
                return Err("形容詞は「い」で終わる形で入れてください (例: えもい / エモい)".into());
            }
        }
        Ok(DictEntry { reading: r, word: w, pos: p.clone(), group: if p == "verb" { group.to_string() } else { String::new() }, pack: pack.to_string() })
    }

    /// 登録。成功なら空文字列、失敗なら理由。
    pub fn add(&mut self, reading: &[u16], word: &[u16], pos: &str, group: &str, pack: &str) -> String {
        self.refresh();
        let en = match self.make_entry(reading, word, pos, group, pack) { Ok(e) => e, Err(e) => return e };
        for e in &self.entries { if e.reading == en.reading && e.word == en.word { return "もう登録してあります".into(); } }
        self.entries.push(en);
        self.save();
        String::new()
    }

    pub fn remove_many(&mut self, items: &[(U16, U16)]) -> i32 {
        self.refresh();
        let want: HashSet<U16> = items.iter().map(|(r, w)| Self::pair_key(r, w)).collect();
        let before = self.entries.len();
        self.entries.retain(|e| !want.contains(&Self::pair_key(&e.reading, &e.word)));
        let n = (before - self.entries.len()) as i32;
        if n > 0 { self.save(); }
        n
    }
    pub fn remove(&mut self, reading: &[u16], word: &[u16]) {
        self.refresh();
        let before = self.entries.len();
        self.entries.retain(|e| !(e.reading == reading && e.word == word));
        if self.entries.len() != before { self.save(); }
    }

    fn save(&mut self) {
        if !self.loaded { return; }
        self.version += 1;
        let mut o = String::from("[\n");
        o.reserve(self.entries.len() * 96 + 16);
        for (i, e) in self.entries.iter().enumerate() {
            if i > 0 { o.push_str(",\n"); }
            o.push_str("  {\"r\":");
            o.push_str(&json::quote(&to_utf8(&e.reading)));
            o.push_str(",\"w\":");
            o.push_str(&json::quote(&to_utf8(&e.word)));
            o.push_str(",\"p\":");
            o.push_str(&json::quote(&e.pos));
            o.push_str(",\"g\":");
            o.push_str(&json::quote(&e.group));
            if !e.pack.is_empty() { o.push_str(",\"k\":"); o.push_str(&json::quote(&e.pack)); }
            o.push('}');
        }
        o.push_str(if self.entries.is_empty() { "]\n" } else { "\n]\n" });
        if write_file_atomic(&self.file, &o) { self.stamp = stamp_of(&self.file); }
    }

    pub fn engine_forms(&self) -> Vec<DictForm> {
        let mut out: Vec<DictForm> = Vec::new();
        for e in &self.entries {
            let (r, w) = (&e.reading, &e.word);
            let mut stem = |tr: &[u16], ts: &[u16], suf: &[u16], out: &mut Vec<DictForm>| {
                let mut nr = r[..r.len() - 1].to_vec(); nr.extend(suf);
                let mut nw = w[..w.len() - 1].to_vec(); nw.extend(suf);
                let mut ntr = tr[..tr.len() - 1].to_vec(); ntr.extend(suf);
                let mut nts = ts[..ts.len() - 1].to_vec(); nts.extend(suf);
                out.push(DictForm { reading: nr, surface: nw, tmpl_reading: ntr, tmpl_surface: nts, whole: false, bonus: -1, lid: -1 });
            };
            if e.pos == "verb" {
                if e.group == "ichidan" {
                    for s in ICHIDAN { stem(&u16s("たべる"), &u16s("食べる"), &u16s(s), &mut out); }
                } else if let Some(g) = godan_of(*r.last().unwrap_or(&0)) {
                    for ch in g.sufs.chars() { stem(&u16s(g.r), &u16s(g.s), &[ch as u16], &mut out); }
                }
            } else if e.pos == "adjective" {
                for s in ADJ { stem(&u16s("たかい"), &u16s("高い"), &u16s(s), &mut out); }
            } else {
                let t: (&str, &str) = match e.pos.as_str() {
                    "person" => ("やまだ", "山田"),
                    "place" | "org" => ("とうきょう", "東京"),
                    "adverb" => ("とても", "とても"),
                    "interjection" => ("こんにちは", "こんにちは"),
                    _ => ("ねこ", "猫"),
                };
                let whole = e.pos == "abbrev";
                out.push(DictForm { reading: r.clone(), surface: w.clone(), tmpl_reading: u16s(t.0), tmpl_surface: u16s(t.1), whole, bonus: -1, lid: -1 });
                let rh = to_halfwidth_ascii(r);
                let rf = to_fullwidth_ascii(r);
                if rh != *r { out.push(DictForm { reading: rh.clone(), surface: w.clone(), tmpl_reading: u16s(t.0), tmpl_surface: u16s(t.1), whole, bonus: -1, lid: -1 }); }
                if rf != *r && rf != rh { out.push(DictForm { reading: rf, surface: w.clone(), tmpl_reading: u16s(t.0), tmpl_surface: u16s(t.1), whole, bonus: -1, lid: -1 }); }
            }
        }
        out
    }

    /// 英字の語: (小文字・空白なしの綴り, 表記)。元の実装は版ごとにキャッシュするが、
    /// ここでは毎回作る (結果は同じ)。
    pub fn latin_words(&self) -> Vec<(U16, U16)> {
        let mut out = Vec::new();
        for e in &self.entries {
            let k = latin_key(&e.word);
            if k.len() < 2 { continue; }
            out.push((k, e.word.clone()));
        }
        out
    }
    pub fn latin_find(&self, key: &[u16]) -> U16 {
        for (k, w) in self.latin_words() { if k == key { return w; } }
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tmpdir(name: &str) -> PathBuf {
        // /tmp は小さい tmpfs で他作業と取り合いになるため HOME 配下を使う
        let base = std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| std::env::temp_dir());
        let d = base.join(format!(".rext-store-test-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }
    #[test]
    fn json_roundtrip() {
        let v = json::Parser::new("{\"a\":1,\"b\":[true,\"x\\n\"],\"c\":{\"d\":-2.5}}").parse().unwrap();
        let o = v.as_obj().unwrap();
        assert_eq!(o[0].1.as_num(), Some(1.0));
        assert_eq!(json::quote("a\"b\\c\n"), "\"a\\\"b\\\\c\\n\"");
        let v2 = json::Parser::new("[1,2,]").parse();
        assert!(v2.is_none() || v2 == Some(json::Value::Array(vec![json::Value::Number(1.0), json::Value::Number(2.0)])));
    }
    #[test]
    fn learning_record_and_persist() {
        let d = tmpdir("learn");
        let f = d.join("learned.json");
        let mut l = Learning::new(f.clone());
        let u = l.record(&u16s("ねこ"), &u16s("猫"), 2, &u16s(""), -1);
        assert_eq!(u.count, 0);
        assert_eq!(l.counts(&u16s("ねこ")), vec![(u16s("猫"), 2)]);
        // 別インスタンスが同じファイルを読む (形式互換)
        let mut l2 = Learning::new(f.clone());
        assert_eq!(l2.counts(&u16s("ねこ")), vec![(u16s("猫"), 2)]);
        let raw = read_file(&f).unwrap();
        assert!(raw.contains("\"@w\""), "raw={}", raw);
        // confirm は学習のある読みだけ
        assert_eq!(l2.confirm(&u16s("いぬ"), &u16s("犬"), &u16s(""), -1).reading.len(), 0);
        l2.confirm(&u16s("ねこ"), &u16s("猫"), &u16s(""), -1);
        assert_eq!(l2.counts(&u16s("ねこ"))[0].1, 3);
        // apply_order: TOP_MIN_COUNT 以上が先頭へ、hidden は除外
        l2.record(&u16s("ねこ"), &u16s("猫"), 2, &u16s(""), -1);
        let ordered = l2.apply_order(&u16s("ねこ"), &[u16s("ねこ"), u16s("猫")]);
        assert_eq!(ordered[0], u16s("猫"));
        l2.hide(&u16s("ねこ"), &u16s("猫"), false);
        assert!(l2.hidden(&u16s("ねこ"), &u16s("猫")));
        let ordered = l2.apply_order(&u16s("ねこ"), &[u16s("猫"), u16s("ねこ")]);
        assert_eq!(ordered, vec![u16s("ねこ")]);
        // restore で hidden を戻す
        l2.restore(&[Undo { reading: u16s("ねこ"), surface: u16s("猫"), count: 5, hidden: true, snap: false, ..Default::default() }]);
        assert!(!l2.hidden(&u16s("ねこ"), &u16s("猫")));
        // demote
        let du = l2.demote(&u16s("ねこ"), &u16s("猫"), &u16s(""), 2);
        assert_eq!(du.count, 5);
        assert_eq!(l2.counts(&u16s("ねこ"))[0].1, 3);
        // completions は 4 以上だけ
        assert_eq!(l2.completions(&u16s("ね"), 5), Vec::<U16>::new());
        l2.record(&u16s("ねこ"), &u16s("猫"), 2, &u16s(""), -1);
        assert_eq!(l2.completions(&u16s("ね"), 5), vec![u16s("猫")]);
        let _ = std::fs::remove_dir_all(&d);
    }
    #[test]
    fn learning_top_and_strong() {
        let d = tmpdir("top");
        let mut l = Learning::new(d.join("learned.json"));
        for _ in 0..3 { l.record(&u16s("か"), &u16s("化"), 2, &u16s(""), -1); }
        // ANY_CTX_MIN=6 かつ他表記の票なし -> top
        assert_eq!(l.top(&u16s("か"), &u16s(""), -1), u16s("化"));
        let st = l.strong(4);
        assert_eq!(st.len(), 1);
        assert_eq!(st[0].surface, u16s("化"));
        let _ = std::fs::remove_dir_all(&d);
    }
    #[test]
    fn recent_ops() {
        let d = tmpdir("recent");
        recent_add(&d, &["L".into(), "a".into()]);
        recent_add(&d, &["C".into(), "b".into()]);
        recent_add(&d, &["L".into(), "tab\tx".into()]); // タブ入りは捨てる
        let s = read_file(&d.join(RECENT_FILE)).unwrap();
        assert_eq!(s.lines().count(), 2);
        assert_eq!(recent_cut(&d, 0, "C"), 1);
        recent_drop(&d, "L");
        let s = read_file(&d.join(RECENT_FILE)).unwrap();
        assert_eq!(s.lines().count(), 0);
        let _ = std::fs::remove_dir_all(&d);
    }
    #[test]
    fn user_dict_ops() {
        let d = tmpdir("dict");
        let f = d.join("user.json");
        let mut ud = UserDict::new(f.clone());
        assert_eq!(ud.add(&u16s("ぐぐる"), &u16s("ググる"), "verb", "godan", ""), "");
        assert_eq!(ud.add(&u16s("ねこ"), &u16s("猫又"), "noun", "", "pack1"), "");
        assert!(!ud.add(&u16s("あたら"), &u16s("当たら"), "verb", "godan", "").is_empty()); // 五段の行に無い読み終わり
        assert_eq!(ud.add(&u16s("たべる"), &u16s("食べる"), "verb", "ichidan", ""), "");
        assert_eq!(ud.lookup(&u16s("ぐぐる")), vec![u16s("ググる")]);
        assert_eq!(ud.packs(), vec!["pack1".to_string()]);
        let forms = ud.engine_forms();
        assert!(forms.iter().any(|x| x.reading == u16s("ぐぐり") && x.surface == u16s("ググり")));
        assert!(forms.iter().any(|x| x.reading == u16s("たべ") && x.surface == u16s("食べ")));
        // 書き出し/取り込み
        let txt = ud.export_text();
        let mut ud2 = UserDict::new(d.join("user2.json"));
        let (n, bad) = ud2.import_text(&txt, "");
        assert_eq!(bad.len(), 0);
        assert_eq!(n, 3);
        let (bytes, cnt) = ud.export_bytes(DictFormat::Csv, None);
        assert_eq!(cnt, 3);
        assert!(bytes.starts_with(&[0xEF, 0xBB, 0xBF]));
        // パック操作
        assert_eq!(ud.move_to_pack(&[(u16s("ねこ"), u16s("猫又"))], "pack2"), 1);
        assert_eq!(ud.remove_pack("pack2", false), 1);
        assert_eq!(ud.remove(&u16s("ねこ"), &u16s("猫又")), ());
        assert_eq!(ud.lookup(&u16s("ねこ")).len(), 0);
        // latin
        assert_eq!(ud.add(&u16s(""), &u16s("Shunti Lettuce"), "latin", "", ""), "");
        assert_eq!(ud.latin_find(&u16s("shuntilettuce")), u16s("Shunti Lettuce"));
        let _ = std::fs::remove_dir_all(&d);
    }
    #[test]
    fn style_log_ops() {
        let d = tmpdir("style");
        let f = d.join("style.json");
        let mut sl = StyleLog::new(f.clone());
        sl.record(&u16s("出来ない"), [0.7, 0.7]);
        sl.record(&u16s("出来ない"), [0.7, 0.7]);
        {
            let c = sl.counts();
            assert_eq!(c[&u16s("出来ない")].n, 2);
        }
        let mut sl2 = StyleLog::new(f.clone());
        assert_eq!(sl2.counts()[&u16s("出来ない")].n, 2);
        sl2.unrecord(&[(u16s("出来ない"), [0.7, 0.7])]);
        assert_eq!(sl2.counts()[&u16s("出来ない")].n, 1);
        let _ = std::fs::remove_dir_all(&d);
    }
    #[test]
    fn latin_key_fn() {
        assert_eq!(latin_key(&u16s("Open AI2")), u16s("openai2"));
        assert_eq!(latin_key(&u16s("ねこ")), Vec::<u16>::new());
    }
}
