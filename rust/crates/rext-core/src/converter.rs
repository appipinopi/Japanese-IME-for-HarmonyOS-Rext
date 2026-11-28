//! converter: 変換エンジン (engine の kkc_* ABI) の窓口。
//! 元: desktop/core/converter.cpp (718行) + converter.h (146行)。
//! 差別語の候補を弾き、長すぎる読みは分けて変換し、1 位の候補を文節に分ける。
//!
//! 依存先の吸収:
//! - engine.h の kkc_* ABI は本ファイルの `Engine` trait として定義する (engine.rs は未実装スタブで
//!   公開関数が無いため)。実エンジン (rext-kkc 等) はこの trait を実装して `Converter::open_with` で差し込む。
//!   trait のメソッドは kkc_* 関数と 1:1 に対応する (convert の生出力は `RawConversion`)。
//! - pos_classes.h の POS_CLASSES / POS_GROUPS は生成表を本ファイルに埋め込んだ (pos.rs は未実装スタブ)。
//! - style.h の StyleModel (書き方の癖) は `StyleHook` trait で差し替え可能にした (style.rs は未実装スタブ)。
//!   既定は無効 (restyle しない)。
//! - tables.h の SLURS は本ファイルに埋め込んだ (tables.rs は未実装スタブ)。
//! - store.h の UserDict::Form は crate::store::DictForm を使う。

use crate::store::DictForm;
use crate::{u16s, U16};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub const POS_CLASSES: &str = "00000000000000000000000000000111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111110000000000000000000000000000000000000000000000000044444444444400000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111333333333000000000000000000000000000000000000000000000000000000000000000000000000000000000000001111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000011111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111100000000000000000000000000000000000000000000000000000000000000000000000000000000001111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111000002222222222222222222222222222222222222222222222200111111111112110000000000000";
pub const POS_GROUPS: &str = "OOIIIIIIIIIIDDDDDDDDDDDDDDDDDXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSNNNNNNNNNNNNNNNnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaIICCCFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFKKKKKKKKKKKKKKKKRRRRRRRRRRRRR";
pub const POS_CLASS_COUNT: usize = 2672;
pub fn pos_class(id: i32) -> u8 {
    if id >= 0 && (id as usize) < POS_CLASS_COUNT { POS_CLASSES.as_bytes()[id as usize] } else { b'0' }
}
pub fn pos_group(id: i32) -> u8 {
    if id >= 0 && (id as usize) < POS_CLASS_COUNT { POS_GROUPS.as_bytes()[id as usize] } else { 0 }
}

fn is_reading_char(c: u16) -> bool { (0x3041..=0x3096).contains(&c) || c == 0x30FC || c == 0x3094 }
fn is_punct(s: &[u16]) -> bool {
    !s.is_empty() && s.iter().all(|&c| u16s("、。，．！？!?…‥・,.").contains(&c))
}
/// 長すぎる読みを分ける位置 (真ん中より前の最後の句読点の後ろ。無ければ真ん中)。
fn split_point(kana: &[u16]) -> usize {
    let mid = (kana.len() / 2).min(120);
    let puncts = u16s("、。！？!?");
    let mut i = mid;
    while i > mid / 2 {
        if puncts.contains(&kana[i - 1]) { return i; }
        i -= 1;
    }
    mid
}

// ---------------------------------------------------------------- 差別語
/// 不適切な語も候補に出す (ユーザー 2026-10-13)。元の `allow_slurs` グローバル相当。
pub static mut ALLOW_SLURS: bool = true;
pub fn allow_slurs() -> bool { unsafe { ALLOW_SLURS } }
pub fn set_allow_slurs(on: bool) { unsafe { ALLOW_SLURS = on; } }

const SLURS: &[(&str, &[&str])] = &[
    ("ガイジ", &[]),
    ("害児", &[]),
    ("池沼", &[]),
    ("キチガイ", &[]),
    ("きちがい", &[]),
    ("気違い", &[]),
    ("気狂い", &[]),
    ("基地外", &[]),
    ("キチゲェ", &[]),
    ("チョン", &["マゲ", "ボ", "チョン", "ガー"]),
    ("チャンコロ", &[]),
    ("支那人", &[]),
    ("シナ人", &[]),
    ("土人", &[]),
    ("ニガー", &[]),
    ("ニガ―", &[]),
    ("黒んぼ", &[]),
    ("クロンボ", &[]),
    ("エタ非人", &[]),
    ("めくら", &[]),
    ("メクラ", &[]),
    ("つんぼ", &[]),
    ("ツンボ", &[]),
    ("知恵遅れ", &[]),
    ("片輪", &[]),
    ("かたわ", &[]),
    ("三国人", &[]),
    ("ジャップ", &[]),
    ("鮮人", &[]),
    ("ホモガキ", &[]),
];
pub fn is_slur(s: &[u16]) -> bool {
    for r in SLURS {
        let w = u16s(r.0);
        if w.is_empty() || s.len() < w.len() { continue; }
        let mut i = 0;
        while i + w.len() <= s.len() {
            if s[i..i + w.len()] == w[..] {
                let mut excused = false;
                for na in r.1 {
                    let nav = u16s(na);
                    if s.len() >= i + w.len() + nav.len() && s[i + w.len()..i + w.len() + nav.len()] == nav[..] { excused = true; }
                }
                if !excused { return true; }
            }
            i += 1;
        }
    }
    false
}

// ---------------------------------------------------------------- 型 (converter.h)
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Word {
    pub reading: U16,
    pub surface: U16,
    pub lid: i32,
    pub rid: i32,
    pub pass: bool,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Phrase {
    pub reading: U16,
    pub surface: U16,
    pub head_r: usize,
    pub head_s: usize,
}
#[derive(Clone, Debug, PartialEq)]
pub struct TreeWord {
    pub surface: U16,
    pub rs: i32,
    pub re: i32,
    pub s: f32,
    pub link: f32,
    pub alts: Vec<(U16, f32)>,
}
impl Default for TreeWord {
    fn default() -> Self { TreeWord { surface: Vec::new(), rs: 0, re: 0, s: 1.0, link: 1.0, alts: Vec::new() } }
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TreeNode {
    pub main: Vec<TreeWord>,
    pub fork: Vec<TreeWord>,
    pub p: f32,
}
impl TreeNode { pub fn new() -> Self { TreeNode { main: Vec::new(), fork: Vec::new(), p: 1.0 } } }

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Conversion {
    pub tree: Vec<TreeNode>,
    pub cands: Vec<U16>,
    pub words: Vec<Word>,
    pub paths: Vec<Vec<TreeWord>>,
    pub sugg: Vec<U16>,
    pub sugg_kinds: Vec<i32>, // engine.h の KKC_SUGGEST_*
}
pub const KKC_SUGGEST_TYPO: i32 = 1;
pub const KKC_SUGGEST_DATE: i32 = 2;
pub const KKC_SUGGEST_ADDRESS: i32 = 3;
pub const KKC_SUGGEST_ENGLISH: i32 = 4;
pub const KKC_SUGGEST_ALT: i32 = 5;
pub const KKC_SUGGEST_MOOD: i32 = 6;
pub const KKC_SUGGEST_SYMBOL: i32 = 7;
pub const KKC_SUGGEST_TYPO_TOP: i32 = 8;
pub const KKC_SUGGEST_AFTER: i32 = 9;

// ---------------------------------------------------------------- Engine trait (kkc_* ABI 相当)
/// kkc_convert の生出力 (0 区切りの候補列を Vec に直したもの + 直後の状態を読むための情報)。
#[derive(Clone, Debug, Default)]
pub struct RawConversion {
    /// 候補 (よい順)。kkc_convert の out バッファの中身。
    pub cands: Vec<U16>,
    /// 提案 (kkc_last_suggest)。
    pub sugg: Vec<U16>,
    pub sugg_kinds: Vec<i32>,
    /// 1 位の語の区切り (kkc_last_segments + kkc_last_segment_pos)。
    /// (読みの終わり位置, 表記の長さ, lid, rid)
    pub segments: Vec<(i32, i32, i32, i32)>,
    /// 上位の経路 (kkc_last_paths): (コスト, 語)。
    pub paths: Vec<(f64, Vec<TreeWord>)>,
}
/// convert が長すぎる読みで失敗した (kkc_convert の -3) ことを表す。
/// Engine::convert の返り値: None = -3 (長すぎる)、Some = それ以外 (候補 0 件でも Some)。
pub trait Engine {
    fn set_threads(&mut self, _n: i32) {}
    fn set_suggest(&mut self, _on: bool) {}
    fn set_latin(&mut self, _on: bool) {}
    fn set_latin_mid2(&mut self, _on: bool) {}
    fn set_typo(&mut self, _on: bool, _bias: f32) {}
    fn set_right_ctx(&mut self, _text: &[u16]) {}
    fn has_right_ctx(&self) -> bool { false }
    fn set_keys(&mut self, _keys: &[u16], _span_end: &[i32], _reading_len: usize) {}
    fn set_alt_readings(&mut self, _alts: &[U16], _reading_len: usize) {}
    /// kkc_convert 本体。mode は変換方式 (元の SHUNTI_KKC_MODE、既定 1)。
    fn convert(&mut self, ctx: &[u16], kana: &[u16], want: i32, mode: i32) -> Option<RawConversion>;
    fn latin_word(&self, _key: &[u16]) -> U16 { Vec::new() }
    fn personal(&self) -> bool { false }
    fn set_variants(&mut self, _tsv_u16: &[u16]) {}
    fn set_marks(&mut self, _marks: &[(U16, i32)]) {}
    fn typo_spans(&self) -> Vec<(i32, i32)> { Vec::new() }
    fn variant_info(&self, _surface: &[u16]) -> bool { false }
    fn path_variants(&self, _reading: &[u16], _text: &[u16]) -> Vec<U16> { Vec::new() }
    fn complete(&mut self, _prefix: &[u16], _max_extra: i32, _max: i32) -> Vec<U16> { Vec::new() }
    fn path_pos(&self, _reading: &[u16], _text: &[u16]) -> Vec<(i32, i32, i32, i32)> { Vec::new() }
    fn user_clear(&mut self) {}
    /// kkc_user_add_pos。成功で true。
    fn user_add_pos(&mut self, _r: &[u16], _s: &[u16], _bonus: i32, _whole: bool, _lid: i32, _pos_groups: &str, _pos_count: usize) -> bool { false }
    /// kkc_user_add_like2。成功で 1。
    fn user_add_like2(&mut self, _r: &[u16], _s: &[u16], _tr: &[u16], _ts: &[u16], _bonus: i32, _whole: bool) -> i32 { 0 }
    // AI学習 (kkc_romance_*)
    fn romance_enable(&mut self, _on: bool) -> i32 { 0 }
    fn romance_config(&mut self, _lr_v: f32, _lr_w: f32, _lr_net: f32, _l2: f32, _cap: f32) {}
    fn romance_size(&self) -> i32 { 0 }
    fn romance_get(&self, _cap: i32) -> Vec<f32> { Vec::new() }
    fn romance_set(&mut self, _v: &[f32]) {}
    fn romance_reset(&mut self) {}
    /// kkc_romance_learn: 学んだら Some(学ぶ前の損失)。
    fn romance_learn(&mut self, _text: &[u16]) -> Option<f32> { None }
    fn model_tag(&self) -> u32 { 0 }
}

/// 書き方の癖 (style.h の StyleModel) の差し替え口。Converter は 1 位の書き換えだけを使う。
pub trait StyleHook {
    fn set_variants(&mut self, _tsv_utf8: &str) {}
    fn set_register(&mut self, _bin: &[u8]) -> bool { false }
    fn set_counts(&mut self, _counts: &HashMap<U16, crate::store::StyleCount>) {}
    fn weight_of(&self, _ctx: &[u16]) -> [f32; 2] { [0.5, 0.5] }
    fn is_variant(&self, _surface: &[u16]) -> bool { false }
    fn restyle(&self, _ctx: &[u16], _out: &mut Conversion) -> bool { false }
}
pub struct NoStyle;
impl StyleHook for NoStyle {}

// ---------------------------------------------------------------- 候補の木 (build_tree)
const TREE_TEMP: f64 = 2500.0;

pub fn build_tree(raw: &[(f64, Vec<TreeWord>)], cuts: &[i32]) -> Vec<TreeNode> {
    let mut out: Vec<TreeNode> = Vec::new();
    if raw.is_empty() || raw[0].1.is_empty() { return out; }
    // 語を文節にまとめる
    let mut paths: Vec<(f64, Vec<TreeWord>)> = Vec::new();
    for (c, ws) in raw {
        let mut ch: Vec<TreeWord> = Vec::new();
        let mut cur = TreeWord::default();
        let mut open = false;
        for x in ws {
            if !open { cur = TreeWord::default(); cur.rs = x.rs; open = true; }
            cur.surface.extend(&x.surface);
            cur.re = x.re;
            if cuts.contains(&x.re) { ch.push(cur.clone()); open = false; }
        }
        if open { ch.push(cur); }
        paths.push((*c, ch));
    }
    let p_n = paths.len();
    let mut w: Vec<f64> = vec![0.0; p_n];
    let mut sum = 0.0;
    for i in 0..p_n { w[i] = (-(paths[i].0 - paths[0].0) / TREE_TEMP).exp(); sum += w[i]; }
    for x in &mut w { *x /= sum; }
    let has = |p: usize, t: &TreeWord| paths[p].1.iter().any(|x| x.rs == t.rs && x.re == t.re && x.surface == t.surface);
    let bounds = |p: usize| -> Vec<i32> { paths[p].1.iter().map(|x| x.re).collect() };
    let top = paths[0].1.clone();
    let b0 = bounds(0);
    let mut words = top.clone();
    for k in 0..words.len() {
        let t = words[k].clone();
        let mut s = 0.0f64;
        let mut both = 0.0f64;
        let mut prev = 0.0f64;
        let mut alt: Vec<(U16, f64)> = Vec::new();
        for p in 0..p_n {
            let h = has(p, &t);
            if h { s += w[p]; }
            if k > 0 && has(p, &top[k - 1]) { prev += w[p]; if h { both += w[p]; } }
            for x in &paths[p].1 {
                if x.rs != t.rs || x.re != t.re || x.surface == t.surface { continue; }
                match alt.iter_mut().find(|a| a.0 == x.surface) {
                    Some(a) => a.1 += w[p],
                    None => alt.push((x.surface.clone(), w[p])),
                }
            }
        }
        words[k].s = s as f32;
        words[k].link = if k == 0 { 1.0 } else if prev > 0.0 { (both / prev) as f32 } else { s as f32 };
        alt.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        words[k].alts = alt.iter().take(6).map(|a| (a.0.clone(), a.1 as f32)).collect();
    }
    #[derive(Clone)]
    struct Fork { a: i32, b: i32, p: usize, mass: f64, best: f64 }
    let mut forks: Vec<Fork> = Vec::new();
    let in_v = |v: &[i32], x: i32| v.contains(&x);
    for p in 1..p_n {
        let bp = bounds(p);
        let mut a = -1i32;
        let mut b = -1i32;
        for x in &bp { if !in_v(&b0, *x) { if a < 0 || *x < a { a = *x; } if *x > b { b = *x; } } }
        for x in &b0 { if !in_v(&bp, *x) && *x != *b0.last().unwrap() { if a < 0 || *x < a { a = *x; } if *x > b { b = *x; } } }
        if a < 0 { continue; }
        let mut lo = 0i32;
        let mut hi = *b0.last().unwrap();
        for x in &b0 {
            if *x < a && in_v(&bp, *x) { lo = lo.max(*x); }
            if *x > b && in_v(&bp, *x) { hi = hi.min(*x); }
        }
        let mut clash = false;
        for f in &mut forks {
            if !(hi <= f.a || lo >= f.b) {
                if lo == f.a && hi == f.b { f.mass += w[p]; }
                clash = true;
                break;
            }
        }
        if !clash { forks.push(Fork { a: lo, b: hi, p, mass: w[p], best: w[p] }); }
    }
    let mut k = 0usize;
    while k < words.len() {
        let f = forks.iter().find(|x| x.a == words[k].rs && x.mass >= 0.08 && x.best >= 0.06).cloned();
        let mut n = TreeNode::new();
        match f {
            None => { n.main.push(words[k].clone()); k += 1; out.push(n); }
            Some(f) => {
                while k < words.len() && words[k].re <= f.b { n.main.push(words[k].clone()); k += 1; }
                for mut x in paths[f.p].1.iter().cloned() {
                    if x.rs >= f.a && x.re <= f.b {
                        let mut s = 0.0;
                        for p in 0..p_n { if has(p, &x) { s += w[p]; } }
                        x.s = s as f32;
                        n.fork.push(x);
                    }
                }
                n.p = (1.0 - f.mass).max(0.0) as f32;
                out.push(n);
            }
        }
    }
    out
}

// ---------------------------------------------------------------- 文節へのまとめ (group_phrases)
pub fn group_phrases(words: &[Word]) -> Vec<Phrase> {
    let mut out: Vec<Phrase> = Vec::new();
    let mut in_tail = false;
    for i in 0..words.len() {
        let w = &words[i];
        let mut attach = false;
        let mut head = false;
        if i > 0 {
            let p = &words[i - 1];
            attach = pos_class(w.lid) == b'1' || is_punct(&w.surface)
                || (pos_class(w.lid) == b'4' && pos_class(p.rid) == b'3');
            head = !attach && (pos_class(p.rid) == b'2' || (p.pass && w.pass)) && !in_tail;
        }
        if (attach || head) && !out.is_empty() {
            let ph = out.last_mut().unwrap();
            ph.reading.extend(&w.reading);
            ph.surface.extend(&w.surface);
            if head { ph.head_r = ph.reading.len(); ph.head_s = ph.surface.len(); } else { in_tail = true; }
        } else {
            out.push(Phrase { reading: w.reading.clone(), surface: w.surface.clone(), head_r: w.reading.len(), head_s: w.surface.len() });
            in_tail = false;
        }
    }
    out
}

// ---------------------------------------------------------------- Converter
pub struct Converter {
    e: Option<Box<dyn Engine>>,
    user: Vec<DictForm>,
    user_bonus: i32,
    suggest: bool,
    latin: bool,
    mid2: bool,
    typo: bool,
    typo_bias: f32,
    variants: String,
    style: Box<dyn StyleHook>,
    restyle: bool,
    ai_base: PathBuf,
    ai_path: PathBuf,
    ai_seen: Option<SystemTime>,
    ai_ready: bool,
    ai_until: i64,
    ai_bytes: i64,
    ai_bulk_learned: i32,
    lex: Vec<u8>,
    model: Vec<u8>,
}
impl Default for Converter {
    fn default() -> Self {
        Converter {
            e: None, user: Vec::new(), user_bonus: 800, suggest: true, latin: true, mid2: false,
            typo: true, typo_bias: 0.0, variants: String::new(), style: Box::new(NoStyle), restyle: false,
            ai_base: PathBuf::new(), ai_path: PathBuf::new(), ai_seen: None, ai_ready: false,
            ai_until: 0, ai_bytes: 0, ai_bulk_learned: 0, lex: Vec::new(), model: Vec::new(),
        }
    }
}

fn unix_now() -> i64 { std::time::UNIX_EPOCH.elapsed().map(|d| d.as_secs() as i64).unwrap_or(0) }
fn log_bytes(dir: &Path) -> i64 {
    std::fs::metadata(dir.join(crate::store::RECENT_FILE)).map(|m| m.len() as i64).unwrap_or(0)
}

impl Converter {
    pub fn new() -> Self { Self::default() }
    pub fn ok(&self) -> bool { self.e.is_some() }

    /// 辞書とモデルで開く。実エンジンは呼び出し側が Engine 実装として渡す
    /// (元の open(const void*, ...) は kkc_open を呼ぶだけなので、生成は差し替え口にした)。
    /// 開けなければ今のエンジンのまま false。
    pub fn open_with(&mut self, mut engine: Box<dyn Engine>, lex: &[u8], model: &[u8], threads: i32) -> bool {
        self.lex = lex.to_vec();
        self.model = model.to_vec();
        if threads > 0 { engine.set_threads(threads); }
        engine.set_suggest(self.suggest);
        engine.set_latin(self.latin);
        engine.set_latin_mid2(self.mid2);
        if !self.typo || self.typo_bias != 0.0 { engine.set_typo(self.typo, self.typo_bias); }
        self.e = Some(engine);
        if !self.variants.is_empty() { let v = self.variants.clone(); self.set_variants(&v); }
        if !self.user.is_empty() { let u = self.user.clone(); let b = self.user_bonus; self.set_user_words(&u, b); }
        self.ai_ready = false;
        if !self.ai_base.as_os_str().is_empty() { let base = self.ai_base.clone(); self.set_ai_learn(&base); }
        self.e.is_some()
    }
    /// 元の open() の形 (バイト列だけでは実エンジンを作れないので、エンジン未登録なら false)。
    /// 実エンジンがある場合は呼び出し側で open_with を使うこと。
    pub fn open(&mut self, _lex: &[u8], _model: &[u8], _threads: i32) -> bool { self.e.is_some() }

    pub fn set_style_hook(&mut self, style: Box<dyn StyleHook>) { self.style = style; }

    // ---- AI学習 (kkc_romance_*)。保存形式: "SHA2" + until(i64) + bytes(i64) + 個数(i32) + float 列
    fn ai_config(e: &mut dyn Engine) { e.romance_config(0.2, 0.01, 0.05, 0.01, 2.0); }
    fn ai_read(path: &Path, e: &mut dyn Engine, until: &mut i64, bytes: &mut i64) -> bool {
        let Ok(data) = std::fs::read(path) else { return false };
        if data.len() < 24 || &data[0..4] != b"SHA2" { return false; }
        let u = i64::from_le_bytes(data[4..12].try_into().unwrap());
        let b = i64::from_le_bytes(data[12..20].try_into().unwrap());
        let n = i32::from_le_bytes(data[20..24].try_into().unwrap());
        if n != e.romance_size() || data.len() < 24 + (n as usize) * 4 { return false; }
        let mut v = Vec::with_capacity(n as usize);
        for i in 0..n as usize {
            v.push(f32::from_le_bytes(data[24 + i * 4..28 + i * 4].try_into().unwrap()));
        }
        e.romance_set(&v);
        *until = u;
        *bytes = b;
        true
    }
    fn ai_write(path: &Path, e: &mut dyn Engine, until: i64, bytes: i64) -> bool {
        let n = e.romance_size();
        if n <= 0 { return false; }
        let v = e.romance_get(n);
        if v.len() != n as usize { return false; }
        let mut data = Vec::with_capacity(24 + v.len() * 4);
        data.extend(b"SHA2");
        data.extend(until.to_le_bytes());
        data.extend(bytes.to_le_bytes());
        data.extend((n as i32).to_le_bytes());
        for x in &v { data.extend(x.to_le_bytes()); }
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        if std::fs::write(&tmp, &data).is_err() { return false; }
        std::fs::rename(&tmp, path).is_ok()
    }
    fn ai_step(e: &mut dyn Engine, ctx: &[u16], reading: &[u16], text: &[u16]) -> f32 {
        if reading.is_empty() || text.is_empty() || text == reading { return -1.0; }
        match e.convert(ctx, reading, 10, 1) {
            None => -1.0,
            Some(_) => e.romance_learn(text).unwrap_or(-1.0),
        }
    }
    fn ai_model_path(base: &Path, e: &dyn Engine) -> PathBuf {
        if base.as_os_str().is_empty() { return base.to_path_buf(); }
        let tag = format!("_{:08x}", e.model_tag());
        let stem = base.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        base.parent().map(|p| p.join(format!("{}{}.bin", stem, tag))).unwrap_or_else(|| base.to_path_buf())
    }

    pub fn set_ai_learn(&mut self, base: &Path) {
        if self.ai_ready && base == self.ai_base { return; }
        let path = match &self.e { Some(e) => Self::ai_model_path(base, e.as_ref()), None => base.to_path_buf() };
        self.ai_ready = self.e.is_some();
        self.ai_base = base.to_path_buf();
        self.ai_path = path.clone();
        self.ai_seen = None;
        self.ai_until = 0;
        self.ai_bytes = 0;
        let Some(e) = &mut self.e else { return };
        e.romance_enable(!path.as_os_str().is_empty());
        if path.as_os_str().is_empty() { return; }
        e.romance_reset();
        Self::ai_config(e.as_mut());
        // 前の 1 つだけの保存 (ai_learn.bin) を、個数が合えば今のモデルの名前に移す + 古いモデルの分は新しい 4 つまで
        Self::ai_adopt_and_prune(base, &path, e.as_mut());
        self.ai_sync();
        self.ai_start_bulk();
    }
    fn ai_adopt_and_prune(base: &Path, path: &Path, e: &mut dyn Engine) {
        if !path.exists() && base.exists() {
            let keep = e.romance_get(e.romance_size().max(0));
            let (mut u, mut b) = (0i64, 0i64);
            if Self::ai_read(base, e, &mut u, &mut b) { let _ = std::fs::rename(base, path); }
            if !keep.is_empty() { e.romance_set(&keep); }
        }
        if let Some(dir) = path.parent() {
            let mut others: Vec<(SystemTime, PathBuf)> = Vec::new();
            if let Ok(rd) = std::fs::read_dir(dir) {
                for ent in rd.flatten() {
                    let p = ent.path();
                    let name = p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                    if p == path || !name.starts_with("ai_learn_") || p.extension().map(|x| x != "bin").unwrap_or(true) { continue; }
                    let t = ent.metadata().and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
                    others.push((t, p));
                }
            }
            others.sort_by(|a, b| b.0.cmp(&a.0));
            for (_, p) in others.iter().skip(4) { let _ = std::fs::remove_file(p); }
        }
    }
    fn ai_sync(&mut self) {
        // 注: 元の実装は裏スレッド (ai_bulk) で学んでいる間は同期しない。Rust 版は bulk を同期実行に
        // したため ai_busy は常に false (下記 ai_start_bulk の注記参照)。
        let Some(e) = &mut self.e else { return };
        let t = std::fs::metadata(&self.ai_path).and_then(|m| m.modified()).ok();
        match t {
            None => {
                if self.ai_seen.is_some() { e.romance_reset(); }
                self.ai_seen = None;
                self.ai_until = 0;
                self.ai_bytes = 0;
            }
            Some(t) => {
                if self.ai_seen == Some(t) { return; }
                self.ai_seen = Some(t);
                let (mut u, mut b) = (0i64, 0i64);
                if !Self::ai_read(&self.ai_path.clone(), e.as_mut(), &mut u, &mut b) {
                    e.romance_reset();
                    u = 0; b = 0;
                }
                self.ai_until = u;
                self.ai_bytes = b;
            }
        }
    }
    fn ai_save(&mut self) {
        self.ai_until = unix_now();
        if let Some(dir) = self.ai_path.parent() { self.ai_bytes = log_bytes(dir); }
        let (path, until, bytes) = (self.ai_path.clone(), self.ai_until, self.ai_bytes);
        if let Some(e) = &mut self.e {
            if Self::ai_write(&path, e.as_mut(), until, bytes) {
                self.ai_seen = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
            }
        }
    }
    /// 元は別スレッド + 別エンジンでまとめて学ぶ。Rust 版 (trait 化) ではエンジンの複製が作れないため、
    /// 同期的に同じエンジンで学ぶ (結果は同じ、打鍵を止める点だけ違う。報告に明記)。
    fn ai_start_bulk(&mut self) {
        let Some(dir) = self.ai_path.parent().map(|p| p.to_path_buf()) else { return };
        let bytes = log_bytes(&dir);
        if bytes == 0 || bytes == self.ai_bytes || self.lex.is_empty() || self.model.is_empty() { return; }
        let lock = dir.join("ai_learn.lock");
        if let Ok(m) = std::fs::metadata(&lock) {
            let fresh = m.modified().map(|t| t.elapsed().map(|d| d.as_secs() < 1800).unwrap_or(false)).unwrap_or(false);
            if fresh { return; }
            let _ = std::fs::remove_file(&lock);
        }
        if std::fs::OpenOptions::new().write(true).create_new(true).open(&lock).is_err() { return; }
        let mut learned = 0i32;
        let mut u = self.ai_until;
        if let Some(e) = &mut self.e {
            for _pass in 0..4 {
                let nu = Self::ai_learn_log(e.as_mut(), &dir, u, &mut learned);
                if nu == u { break; }
                u = nu;
            }
            let b2 = log_bytes(&dir);
            Self::ai_write(&self.ai_path.clone(), e.as_mut(), u, b2);
        }
        self.ai_bulk_learned = learned;
        let _ = std::fs::remove_file(&lock);
    }
    fn ai_learn_log(e: &mut dyn Engine, dir: &Path, since: i64, learned: &mut i32) -> i64 {
        let Some(s) = crate::store::read_file(&dir.join(crate::store::RECENT_FILE)) else { return since };
        let mut last = since;
        for line in s.split('\n') {
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() != 5 || f[1] != "C" { continue; }
            let mut t: i64 = 0;
            for c in f[0].chars() { if c.is_ascii_digit() { t = t * 10 + (c as i64 - '0' as i64); } }
            if t <= since { continue; }
            if Self::ai_step(e, &u16s(f[4]), &u16s(f[2]), &u16s(f[3])) >= 0.0 { *learned += 1; }
            last = last.max(t);
        }
        last
    }
    pub fn ai_wait(&mut self) { /* 同期実行のため待つものは無い */ }
    pub fn ai_busy(&self) -> bool { false }
    pub fn ai_bulk_learned(&self) -> i32 { self.ai_bulk_learned }
    pub fn ai_file(&self) -> &Path { &self.ai_path }
    pub fn ai_learn(&mut self, ctx: &[u16], reading: &[u16], text: &[u16]) -> f32 {
        if self.e.is_none() || self.ai_path.as_os_str().is_empty() { return -1.0; }
        self.ai_sync();
        let loss = {
            let e = self.e.as_mut().unwrap();
            Self::ai_step(e.as_mut(), ctx, reading, text)
        };
        if loss >= 0.0 { self.ai_save(); }
        loss
    }

    // ---- 変換本体
    fn convert_once(&mut self, ctx: &[u16], kana: &[u16], max: i32, out: &mut Conversion) -> bool {
        *out = Conversion::default();
        let want = max * 2;
        let mode: i32 = std::env::var("SHUNTI_KKC_MODE").ok()
            .and_then(|m| m.chars().next()).filter(|c| c.is_ascii_digit()).map(|c| c as i32 - '0' as i32).unwrap_or(1);
        let Some(e) = &mut self.e else { return true };
        let Some(raw) = e.convert(ctx, kana, want, mode) else { return false }; // -3 = 長すぎる
        for c in &raw.cands {
            if out.cands.len() as i32 >= max { break; }
            if allow_slurs() || !is_slur(c) { out.cands.push(c.clone()); }
        }
        for (i, sg) in raw.sugg.iter().enumerate() {
            if allow_slurs() || !is_slur(sg) {
                out.sugg.push(sg.clone());
                out.sugg_kinds.push(raw.sugg_kinds.get(i).copied().unwrap_or(0));
            }
        }
        // 。。。・・・ は … と ... にした形も 1 位のすぐ後ろに
        if !out.cands.is_empty() {
            let mut t = out.cands[0].clone();
            let mut hit = false;
            for dots in [u16s("。。。"), u16s("・・・"), u16s("．．．")] {
                let mut replaced = Vec::new();
                let mut i = 0;
                while i < t.len() {
                    if i + 3 <= t.len() && t[i..i + 3] == dots[..] { replaced.push(u16s("…")[0]); i += 3; hit = true; }
                    else { replaced.push(t[i]); i += 1; }
                }
                t = replaced;
            }
            if hit && !out.cands.contains(&t) {
                out.cands.insert(1.min(out.cands.len()), t.clone());
                let mut a = Vec::new();
                let mut i = 0;
                while i < t.len() {
                    if t[i] == u16s("…")[0] { a.extend(u16s("...")); i += 1; } else { a.push(t[i]); i += 1; }
                }
                if !out.cands.contains(&a) { out.cands.insert(2.min(out.cands.len()), a); }
            }
        }
        // 差別語で 1 位が落ちたときは区切りを使わない
        if raw.cands.is_empty() || out.cands.is_empty() || out.cands[0] != raw.cands[0] { self.apply_restyle(ctx, out); return true; }
        let top = raw.cands[0].clone();
        let mut r0 = 0usize;
        let mut s0 = 0usize;
        let mut words: Vec<Word> = Vec::new();
        let mut valid = true;
        for (end, len, lid, rid) in &raw.segments {
            let (r1, s1) = (*end as usize, s0 + *len as usize);
            if r1 < r0 || r1 > kana.len() || s1 > top.len() { valid = false; break; }
            let mut w = Word { reading: kana[r0..r1].to_vec(), surface: top[s0..s1].to_vec(), lid: *lid, rid: *rid, pass: false };
            w.pass = !w.reading.is_empty() && w.reading == w.surface && !is_reading_char(w.reading[0]);
            words.push(w);
            r0 = r1;
            s0 = s1;
        }
        if !valid || r0 != kana.len() || s0 != top.len() { self.apply_restyle(ctx, out); return true; }
        out.words = words;
        out.paths = raw.paths.iter().map(|p| p.1.clone()).collect();
        if !raw.paths.is_empty() {
            let mut j: U16 = Vec::new();
            for w in &raw.paths[0].1 { j.extend(&w.surface); }
            if j == top {
                let mut cuts = Vec::new();
                let mut r = 0i32;
                for ph in group_phrases(&out.words) { r += ph.reading.len() as i32; cuts.push(r); }
                out.tree = build_tree(&raw.paths, &cuts);
            }
        }
        self.apply_restyle(ctx, out);
        true
    }
    fn apply_restyle(&mut self, ctx: &[u16], out: &mut Conversion) {
        if self.restyle { self.style.restyle(ctx, out); }
    }

    pub fn convert(&mut self, ctx: &[u16], kana: &[u16], max: i32) -> Conversion {
        let mut out = Conversion::default();
        if self.e.is_none() || kana.is_empty() { return out; }
        if !self.ai_path.as_os_str().is_empty() { self.ai_sync(); }
        if self.convert_once(ctx, kana, max, &mut out) { return out; }
        // 長すぎる: 前と後ろに分けて変換し、1 位どうしをつなぐ
        let sp = split_point(kana);
        let a = self.convert(ctx, &kana[..sp], 1);
        let sa = if a.cands.is_empty() { kana[..sp].to_vec() } else { a.cands[0].clone() };
        let mut ctx2 = ctx.to_vec();
        ctx2.extend(&sa);
        let b = self.convert(&ctx2, &kana[sp..], 1);
        let sb = if b.cands.is_empty() { kana[sp..].to_vec() } else { b.cands[0].clone() };
        let mut joined = sa.clone();
        joined.extend(&sb);
        out.cands.push(joined);
        let mut aw = a.words;
        if aw.is_empty() { aw.push(Word { reading: kana[..sp].to_vec(), surface: sa, ..Default::default() }); }
        let mut bw = b.words;
        if bw.is_empty() { bw.push(Word { reading: kana[sp..].to_vec(), surface: sb, ..Default::default() }); }
        out.words = aw;
        out.words.extend(bw);
        out
    }

    // ---- 1 回だけ効く設定・問い合わせ
    pub fn set_right_ctx(&mut self, text: &[u16]) { if let Some(e) = &mut self.e { e.set_right_ctx(text); } }
    pub fn set_keys(&mut self, keys: &[u16], span_end: &[i32], reading_len: usize) {
        if span_end.len() == reading_len { if let Some(e) = &mut self.e { e.set_keys(keys, span_end, reading_len); } }
    }
    pub fn set_alt_readings(&mut self, alts: &[U16], reading_len: usize) {
        if reading_len == 0 { return; }
        if let Some(e) = &mut self.e { e.set_alt_readings(alts, reading_len); }
    }
    pub fn has_right_ctx(&self) -> bool { self.e.as_ref().map(|e| e.has_right_ctx()).unwrap_or(false) }
    pub fn latin_word(&self, key: &[u16]) -> U16 {
        if key.is_empty() { return Vec::new(); }
        self.e.as_ref().map(|e| e.latin_word(key)).unwrap_or_default()
    }
    pub fn personal(&self) -> bool { self.e.as_ref().map(|e| e.personal()).unwrap_or(false) }
    pub fn set_variants(&mut self, tsv_utf8: &str) {
        self.variants = tsv_utf8.to_string();
        self.style.set_variants(tsv_utf8);
        if let Some(e) = &mut self.e { e.set_variants(&u16s(tsv_utf8)); }
    }
    pub fn set_marks(&mut self, marks: &[(U16, i32)]) { if let Some(e) = &mut self.e { e.set_marks(marks); } }
    pub fn typo_spans(&self) -> Vec<(i32, i32)> { self.e.as_ref().map(|e| e.typo_spans()).unwrap_or_default() }
    pub fn is_variant(&self, surface: &[u16]) -> bool {
        !surface.is_empty() && self.e.as_ref().map(|e| e.variant_info(surface)).unwrap_or(false)
    }
    pub fn path_variants(&self, reading: &[u16], text: &[u16]) -> Vec<U16> {
        if reading.is_empty() || text.is_empty() { return Vec::new(); }
        self.e.as_ref().map(|e| e.path_variants(reading, text)).unwrap_or_default()
    }
    pub fn set_typo(&mut self, on: bool, bias: f32) {
        if on == self.typo && bias == self.typo_bias { return; }
        self.typo = on;
        self.typo_bias = bias;
        if let Some(e) = &mut self.e { e.set_typo(on, bias); }
    }
    pub fn set_latin(&mut self, on: bool) {
        self.latin = on;
        if let Some(e) = &mut self.e { e.set_latin(on); }
    }
    pub fn set_latin_mid2(&mut self, on: bool) { self.mid2 = on; }
    pub fn set_suggest(&mut self, on: bool) {
        self.suggest = on;
        if let Some(e) = &mut self.e { e.set_suggest(on); }
    }
    pub fn set_register(&mut self, bin: &[u8]) -> bool { self.style.set_register(bin) }
    pub fn set_style_counts(&mut self, counts: &HashMap<U16, crate::store::StyleCount>) { self.style.set_counts(counts); }
    pub fn set_restyle(&mut self, on: bool) { self.restyle = on; }
    pub fn style_weight(&self, ctx: &[u16]) -> [f32; 2] { self.style.weight_of(ctx) }
    pub fn style_is_variant(&self, surface: &[u16]) -> bool { self.style.is_variant(surface) }

    pub fn complete(&mut self, prefix: &[u16], max: i32, max_extra: i32) -> Vec<U16> {
        if prefix.is_empty() { return Vec::new(); }
        let Some(e) = &mut self.e else { return Vec::new() };
        let mut out = e.complete(prefix, max_extra, max);
        if !allow_slurs() { out.retain(|c| !is_slur(c)); }
        out
    }
    pub fn path_pos(&self, reading: &[u16], text: &[u16]) -> Vec<Word> {
        if reading.is_empty() || text.is_empty() { return Vec::new(); }
        let Some(e) = &self.e else { return Vec::new() };
        let segs = e.path_pos(reading, text);
        let mut out = Vec::new();
        let (mut r0, mut s0) = (0usize, 0usize);
        for (end, len, lid, rid) in segs {
            let (r1, s1) = (end as usize, s0 + len as usize);
            if r1 > reading.len() || s1 > text.len() { return Vec::new(); }
            out.push(Word { reading: reading[r0..r1].to_vec(), surface: text[s0..s1].to_vec(), lid, rid, pass: false });
            r0 = r1;
            s0 = s1;
        }
        out
    }
    pub fn set_user_words(&mut self, forms: &[DictForm], bonus: i32) -> i32 {
        self.user = forms.to_vec();
        self.user_bonus = bonus;
        let Some(e) = &mut self.e else { return 0 };
        e.user_clear();
        let mut n = 0;
        for f in forms {
            let b = if f.bonus >= 0 { f.bonus } else { bonus };
            if f.lid >= 0 && e.user_add_pos(&f.reading, &f.surface, b, f.whole, f.lid, POS_GROUPS, POS_CLASS_COUNT) {
                n += 1;
                continue;
            }
            if f.lid >= 0 && f.reading.len() < 2 { continue; }
            let (tr, ts): (&[u16], &[u16]) = if f.tmpl_reading.is_empty() { (&f.reading, &f.surface) } else { (&f.tmpl_reading, &f.tmpl_surface) };
            let mut ok = e.user_add_like2(&f.reading, &f.surface, tr, ts, b, f.whole);
            if ok == 0 && f.tmpl_reading.is_empty() {
                ok = e.user_add_like2(&f.reading, &f.surface, &u16s("ねこ"), &u16s("猫"), b, false);
            }
            n += ok;
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::from_u16s;

    struct FakeEngine {
        // 読み -> 候補 (1 語として返す)
        map: HashMap<String, Vec<String>>,
        typo: Vec<(i32, i32)>,
        romance: Vec<f32>,
        user_added: Vec<(String, String)>,
    }
    impl FakeEngine {
        fn new() -> Self {
            let mut map = HashMap::new();
            map.insert("ねこ".to_string(), vec!["猫".to_string(), "ねこ".to_string()]);
            map.insert("きょうは".to_string(), vec!["今日は".to_string()]);
            FakeEngine { map, typo: Vec::new(), romance: vec![0.0; 4], user_added: Vec::new() }
        }
    }
    impl Engine for FakeEngine {
        fn convert(&mut self, _ctx: &[u16], kana: &[u16], want: i32, _mode: i32) -> Option<RawConversion> {
            if kana.len() > 10 { return None; } // 長すぎる (-3)
            let k = from_u16s(kana);
            let cands: Vec<U16> = self.map.get(&k).cloned().unwrap_or_else(|| vec![k.clone()])
                .iter().take(want.max(0) as usize).map(|s| u16s(s)).collect();
            let mut segments = Vec::new();
            if let Some(top) = cands.first() {
                segments.push((kana.len() as i32, top.len() as i32, 100, 100));
            }
            let paths = if cands.is_empty() { Vec::new() } else {
                vec![(0.0, vec![TreeWord { surface: cands[0].clone(), rs: 0, re: kana.len() as i32, ..Default::default() }])]
            };
            Some(RawConversion { cands, sugg: Vec::new(), sugg_kinds: Vec::new(), segments, paths })
        }
        fn typo_spans(&self) -> Vec<(i32, i32)> { self.typo.clone() }
        fn romance_size(&self) -> i32 { self.romance.len() as i32 }
        fn romance_get(&self, _cap: i32) -> Vec<f32> { self.romance.clone() }
        fn romance_set(&mut self, v: &[f32]) { self.romance = v.to_vec(); }
        fn romance_learn(&mut self, _text: &[u16]) -> Option<f32> { Some(0.25) }
        fn model_tag(&self) -> u32 { 0x1234 }
        fn user_add_like2(&mut self, r: &[u16], s: &[u16], _tr: &[u16], _ts: &[u16], _b: i32, _w: bool) -> i32 {
            self.user_added.push((from_u16s(r), from_u16s(s)));
            1
        }
        fn complete(&mut self, prefix: &[u16], _e: i32, _m: i32) -> Vec<U16> {
            if prefix == u16s("ね") { vec![u16s("猫")] } else { Vec::new() }
        }
    }

    #[test]
    fn slur_check() {
        assert!(is_slur(&u16s("ガイジ")));
        assert!(is_slur(&u16s("あガイジい")));
        assert!(!is_slur(&u16s("ねこ")));
        // チョン + マゲ は弾かない
        assert!(!is_slur(&u16s("チョンマゲ")));
        assert!(is_slur(&u16s("チョン")));
    }
    #[test]
    fn group_phrases_attach() {
        // 助詞 (lid の class が '1') は前の文節に付く
        let lid1 = (0..POS_CLASS_COUNT).find(|&i| pos_class(i as i32) == b'1').unwrap() as i32;
        let lid0 = (0..POS_CLASS_COUNT).find(|&i| pos_class(i as i32) == b'0').unwrap() as i32;
        let words = vec![
            Word { reading: u16s("ねこ"), surface: u16s("猫"), lid: lid0, rid: lid0, pass: false },
            Word { reading: u16s("は"), surface: u16s("は"), lid: lid1, rid: lid1, pass: false },
            Word { reading: u16s("かわいい"), surface: u16s("可愛い"), lid: lid0, rid: lid0, pass: false },
        ];
        let ph = group_phrases(&words);
        assert_eq!(ph.len(), 2);
        assert_eq!(ph[0].surface, u16s("猫は"));
        assert_eq!(ph[0].head_s, 1);
        assert_eq!(ph[1].surface, u16s("可愛い"));
    }
    #[test]
    fn build_tree_basic() {
        let w = |s: &str, rs: i32, re: i32| TreeWord { surface: u16s(s), rs, re, ..Default::default() };
        let raw = vec![
            (0.0, vec![w("猫", 0, 2), w("は", 2, 3)]),
            (100.0, vec![w("ねこ", 0, 2), w("は", 2, 3)]),
        ];
        let tree = build_tree(&raw, &[2, 3]);
        assert_eq!(tree.len(), 2);
        assert_eq!(tree[0].main[0].surface, u16s("猫"));
        assert!(tree[0].main[0].alts.iter().any(|a| a.0 == u16s("ねこ")));
        assert!(tree[0].main[0].s > 0.5);
    }
    #[test]
    fn convert_and_split() {
        let mut c = Converter::new();
        assert!(!c.ok());
        assert!(c.open_with(Box::new(FakeEngine::new()), b"lex", b"model", 1));
        let out = c.convert(&[], &u16s("ねこ"), 30);
        assert_eq!(out.cands[0], u16s("猫"));
        assert_eq!(out.words.len(), 1);
        // 長すぎる読みは分割して 1 位をつなぐ (11 字 -> 分割)
        let long = u16s("ねこねこねこねこねこね"); // 11
        let out2 = c.convert(&[], &long, 30);
        assert!(!out2.cands.is_empty());
        // user words
        let forms = vec![DictForm { reading: u16s("ねこ"), surface: u16s("猫"), tmpl_reading: u16s("ねこ"), tmpl_surface: u16s("猫"), whole: false, bonus: -1, lid: -1 }];
        assert_eq!(c.set_user_words(&forms, 800), 1);
        assert_eq!(c.complete(&u16s("ね"), 4, 6), vec![u16s("猫")]);
    }
    #[test]
    fn dots_rewrite() {
        struct Dots;
        impl Engine for Dots {
            fn convert(&mut self, _c: &[u16], k: &[u16], _w: i32, _m: i32) -> Option<RawConversion> {
                Some(RawConversion { cands: vec![u16s("あ。。。い")], segments: vec![(k.len() as i32, 5, 0, 0)], ..Default::default() })
            }
        }
        let mut c = Converter::new();
        c.open_with(Box::new(Dots), b"", b"", 0);
        let out = c.convert(&[], &u16s("あ"), 30);
        assert_eq!(out.cands[1], u16s("あ…い"));
        assert_eq!(out.cands[2], u16s("あ...い"));
    }
}
