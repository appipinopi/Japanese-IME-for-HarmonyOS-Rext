//! 物理キーの打ち間違いの直し (元の desktop/core/typo_alts.cpp + typo_alts.h の移植)。
//! 打鍵の単位の打ち直し (Layout::typo_variants) を読みに直し、変換に渡す「読みの打ち直し」を作る。
//!
//! 依存: crate::romaji::Romaji は元の romaji.h の形 (process / flush / reset / is_prefix) を前提に呼ぶ。

use std::collections::BTreeSet;
use std::sync::OnceLock;

use crate::layout::Layout;
use crate::romaji::Romaji;
use crate::U16;

/// 拗 (U+E000) の印
pub const YOU: u16 = 0xE000;
/// 修 (U+E001) の印
pub const SHU: u16 = 0xE001;

const DAKU_FROM: &str = "かきくけこさしすせそたちつてとはひふへほうカキクケコサシスセソタチツテトハヒフヘホウ";
const DAKU_TO: &str = "がぎぐげござじずぜぞだぢづでどばびぶべぼゔガギグゲゴザジズゼゾダヂヅデドバビブベボヴ";
const HAN_FROM: &str = "はひふへほハヒフヘホ";
const HAN_TO: &str = "ぱぴぷぺぽパピプペポ";
const SMALL_FROM: &str = "あいうえおやゆよつわかけアイウエオヤユヨツワカケ";
const SMALL_TO: &str = "ぁぃぅぇぉゃゅょっゎゕゖァィゥェォャュョッヮヵヶ";

fn find_in(table: &str, c: u16) -> Option<usize> {
    table.encode_utf16().position(|x| x == c)
}

fn at_in(table: &str, idx: usize) -> u16 {
    table.encode_utf16().nth(idx).unwrap()
}

/// かな入力の 拗 (U+E000: 小さく)・修 (U+E001: 濁音 → 半濁音 → 元、小さく → 元)・゛・゜ を前の字 prev に付ける。
/// 拗・修は前の字が変わらなくても true (使った)、゛・゜ は付けられたときだけ true
/// (付けられなければ呼ぶ側がそのまま入れる)
pub fn kana_modify(prev: &mut u16, c: u16) -> bool {
    if c == YOU {
        if let Some(at) = find_in(SMALL_FROM, *prev) {
            *prev = at_in(SMALL_TO, at);
        } else if let Some(at) = find_in(SMALL_TO, *prev) {
            *prev = at_in(SMALL_FROM, at);
        }
        return true;
    }
    if c == SHU {
        if let Some(at) = find_in(DAKU_FROM, *prev) {
            *prev = at_in(DAKU_TO, at); // か -> が
        } else if let Some(at) = find_in(DAKU_TO, *prev) {
            // が -> か、ば -> ぱ
            let plain = at_in(DAKU_FROM, at);
            *prev = match find_in(HAN_FROM, plain) {
                Some(h) => at_in(HAN_TO, h),
                None => plain,
            };
        } else if let Some(at) = find_in(HAN_TO, *prev) {
            *prev = at_in(HAN_FROM, at); // ぱ -> は
        } else if let Some(at) = find_in(SMALL_FROM, *prev) {
            *prev = at_in(SMALL_TO, at); // や -> ゃ
        } else if let Some(at) = find_in(SMALL_TO, *prev) {
            *prev = at_in(SMALL_FROM, at);
        }
        return true;
    }
    if c == '゛' as u16 || c == '゜' as u16 {
        let (from, to) = if c == '゛' as u16 {
            (DAKU_FROM, DAKU_TO)
        } else {
            (HAN_FROM, HAN_TO)
        };
        if let Some(at) = find_in(from, *prev) {
            *prev = at_in(to, at);
            return true;
        }
    }
    false
}

/// 打鍵の単位ごとの出した字を読みに直す。letters = 字をローマ字 (rom の表) に流す、false = かな入力の決まり
pub fn decode_unit_texts(outs: &[U16], letters: bool, rom: &mut Romaji) -> U16 {
    let mut kana: U16 = Vec::new();
    if letters {
        // Composer の replay と同じ: ローマ字の字は表に、それ以外は打ちかけを出してからそのまま
        rom.reset();
        for o in outs {
            for &key in o {
                let lk = if (b'A' as u16..=b'Z' as u16).contains(&key) {
                    key + 32
                } else {
                    key
                };
                if key == lk
                    && ((b'a' as u16..=b'z' as u16).contains(&lk)
                        || lk == '-' as u16
                        || lk == '\'' as u16
                        || rom.is_prefix(&[lk]))
                {
                    let committed = rom.process(lk).committed;
                    kana.extend_from_slice(&committed);
                } else {
                    let flushed = rom.flush();
                    kana.extend_from_slice(&flushed);
                    kana.push(key);
                }
            }
        }
        let flushed = rom.flush();
        kana.extend_from_slice(&flushed);
        return kana;
    }
    for o in outs {
        for &c in o {
            let m = c == YOU || c == SHU || c == '゛' as u16 || c == '゜' as u16;
            if m && !kana.is_empty() {
                let mut last = kana[kana.len() - 1];
                if kana_modify(&mut last, c) {
                    let n = kana.len();
                    kana[n - 1] = last;
                    continue;
                }
            }
            if c == YOU || c == SHU {
                continue;
            }
            kana.push(c);
        }
    }
    kana
}

/// 読みの打ち直し。orig = 打ったとおりの単位ごとの字、alts = 1 か所を直した並び、all = 今の読み全体、
/// off = 変換する所の始まり (all の後ろの部分)。記録を読みに直した形が all と字数で合い、直す所の前後 1 字まで同じ
/// ものだけ (句読点・数字など、組み立てで形の変わる字は直さない所にあってよい)。合わなければ false。
/// 返り値: (成否, 直した読みの並び) — 元の out 引数
pub fn alt_readings(
    orig: &[U16],
    alts: &[Vec<U16>],
    letters: bool,
    rom: &mut Romaji,
    all: &[u16],
    off: usize,
) -> (bool, Vec<U16>) {
    let mut out: Vec<U16> = Vec::new();
    let n = all.len();
    if off > n {
        return (false, out);
    }
    let d0 = decode_unit_texts(orig, letters, rom);
    if d0.len() != n {
        return (false, out);
    }
    let diff: Vec<bool> = (0..n).map(|i| d0[i] != all[i]).collect();
    let kana_only = |s: &[u16]| {
        s.iter()
            .all(|&c| (0x3041..=0x3096).contains(&c) || (0x30A1..=0x30FA).contains(&c) || c == 'ー' as u16)
    };
    let reading = &all[off..];
    let mut seen: BTreeSet<U16> = BTreeSet::new();
    for a in alts {
        let d = decode_unit_texts(a, letters, rom);
        let m = d.len();
        let mut p = 0usize;
        while p < n && p < m && d[p] == d0[p] {
            p += 1;
        }
        let mut q = 0usize;
        while q < n - p && q < m - p && d[m - 1 - q] == d0[n - 1 - q] {
            q += 1;
        }
        if (p == n && m == n) || p < off {
            continue;
        }
        let mut ok = true;
        let start = if p > 0 { p - 1 } else { 0 };
        let end = n.min(n - q + 1);
        for i in start..end {
            if diff[i] {
                ok = false;
                break;
            }
        }
        let mid = &d[p..m - q];
        if !ok || !kana_only(mid) {
            continue;
        }
        let mut alt: U16 = Vec::new();
        alt.extend_from_slice(&all[off..p]);
        alt.extend_from_slice(mid);
        alt.extend_from_slice(&all[n - q..]);
        if alt != reading && seen.insert(alt.clone()) {
            out.push(alt);
        }
    }
    (true, out)
}

/// 配列を使わない QWERTY のローマ字の単位の表 (打ったキーがそのまま単位。隣は QWERTY の物理キー)
pub fn qwerty_layout() -> &'static Layout {
    static L: OnceLock<Layout> = OnceLock::new();
    L.get_or_init(|| {
        let mut x = Layout::new();
        // 綴りも読み替えも無い: キーがそのまま単位
        let _ = x.load("#! name QWERTY のローマ字\n#! kind romaji\n");
        x
    })
}

/// 打ったキーの並び (ローマ字) を単位にする。大文字・かな・記号の混じるものは false。
/// 返り値: (成否, 単位の並び) — 元の units 引数
pub fn units_from_keys(keys: &[u16]) -> (bool, Vec<String>) {
    let mut units: Vec<String> = Vec::new();
    for &c in keys {
        let ok = (b'a' as u16..=b'z' as u16).contains(&c)
            || c == '-' as u16
            || c == '\'' as u16
            || c == ';' as u16
            || c == ':' as u16
            || c == ',' as u16
            || c == '.' as u16
            || c == '/' as u16
            || c == '[' as u16
            || c == ']' as u16
            || (b'0' as u16..=b'9' as u16).contains(&c);
        if !ok {
            return (false, Vec::new());
        }
        units.push(((c as u8) as char).to_string());
    }
    let ok = !units.is_empty();
    (ok, units)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{from_u16s, u16s};

    fn m(prev: &str, c: u16) -> (String, bool) {
        let mut p = prev.encode_utf16().next().unwrap();
        let r = kana_modify(&mut p, c);
        (from_u16s(&[p]), r)
    }

    #[test]
    fn kana_modify_you() {
        assert_eq!(m("あ", YOU), ("ぁ".into(), true));
        assert_eq!(m("ぁ", YOU), ("あ".into(), true)); // 小さく → 元
        assert_eq!(m("か", YOU), ("ゕ".into(), true));
        assert_eq!(m("な", YOU), ("な".into(), true)); // 変わらなくても true (使った)
    }

    #[test]
    fn kana_modify_shu_cycle() {
        assert_eq!(m("か", SHU), ("が".into(), true));
        assert_eq!(m("が", SHU), ("か".into(), true));
        assert_eq!(m("は", SHU), ("ば".into(), true));
        assert_eq!(m("ば", SHU), ("ぱ".into(), true)); // 濁音 → 半濁音
        assert_eq!(m("ぱ", SHU), ("は".into(), true)); // 半濁音 → 元
        assert_eq!(m("や", SHU), ("ゃ".into(), true));
        assert_eq!(m("ゃ", SHU), ("や".into(), true));
    }

    #[test]
    fn kana_modify_dakuten_marks() {
        assert_eq!(m("か", '゛' as u16), ("が".into(), true));
        assert_eq!(m("は", '゜' as u16), ("ぱ".into(), true));
        assert_eq!(m("あ", '゛' as u16), ("あ".into(), false)); // 付けられなければ false
        assert_eq!(m("か", '゜' as u16), ("か".into(), false));
    }

    #[test]
    fn decode_kana_mode() {
        // かな入力: 拗・修・゛・゜ を前の字に付ける。付けられない ゛ はそのまま残る
        let outs: Vec<U16> = vec![u16s("か"), vec![SHU], u16s("や"), vec![YOU], u16s("あ"), vec!['゛' as u16]];
        let mut rom = Romaji::default();
        assert_eq!(from_u16s(&decode_unit_texts(&outs, false, &mut rom)), "がゃあ゛");
        // 前の字が無い 拗・修 は捨てる
        let outs2: Vec<U16> = vec![vec![YOU], u16s("あ")];
        assert_eq!(from_u16s(&decode_unit_texts(&outs2, false, &mut rom)), "あ");
    }

    #[test]
    fn decode_letters_mode() {
        let mut rom = Romaji::default();
        let outs: Vec<U16> = vec![u16s("k"), u16s("a"), u16s("n"), u16s("a")];
        assert_eq!(from_u16s(&decode_unit_texts(&outs, true, &mut rom)), "かな");
        // 大文字はローマ字に流さず、打ちかけを出してからそのまま
        let outs2: Vec<U16> = vec![u16s("K"), u16s("a")];
        assert_eq!(from_u16s(&decode_unit_texts(&outs2, true, &mut rom)), "Kあ");
    }

    #[test]
    fn alt_readings_basic() {
        let mut rom = Romaji::default();
        let orig: Vec<U16> = vec![u16s("k"), u16s("a"), u16s("n"), u16s("a")];
        let alt: Vec<U16> = vec![u16s("s"), u16s("a"), u16s("n"), u16s("a")];
        let (ok, out) = alt_readings(&orig, &[alt], true, &mut rom, &u16s("かな"), 0);
        assert!(ok);
        assert_eq!(out.len(), 1);
        assert_eq!(from_u16s(&out[0]), "さな");
        // 同じ読みになる直しは出さない
        let (ok2, out2) = alt_readings(&orig, &[orig.clone()], true, &mut rom, &u16s("かな"), 0);
        assert!(ok2);
        assert!(out2.is_empty());
        // 記録の読みと字数が合わなければ false
        let (ok3, _) = alt_readings(&orig, &[], true, &mut rom, &u16s("かなx"), 0);
        assert!(!ok3);
    }

    #[test]
    fn units_from_keys_cases() {
        let (ok, units) = units_from_keys(&u16s("kana;"));
        assert!(ok);
        assert_eq!(units, vec!["k", "a", "n", "a", ";"]);
        assert!(!units_from_keys(&u16s("Kana")).0); // 大文字は false
        assert!(!units_from_keys(&u16s("かな")).0); // かなは false
        assert!(!units_from_keys(&u16s("")).0); // 空は false
    }

    #[test]
    fn qwerty_layout_is_romaji_kind() {
        let l = qwerty_layout();
        assert_eq!(l.kind(), crate::layout::Kind::Romaji);
        assert_eq!(l.name(), "QWERTY のローマ字");
        // 読み替えないキー: 単位の字は打ったまま
        assert_eq!(l.unit_text("k"), u16s("k"));
    }
}
