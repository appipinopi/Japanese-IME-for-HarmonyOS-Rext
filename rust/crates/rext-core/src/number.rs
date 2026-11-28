//! 数字の書き換え (元: entry/src/main/ets/ime/NumberFormatter.ets、
//! android の NumberFormatter.kt、desktop/core/datetime.cpp の number_forms)。
//!
//! 3 実装に重複していた数字の形作りをここに 1 本化する:
//! - [`number_forms`]: C++ 版 number_forms そのもの (和暦・桁区切り・万の混ぜ書き・
//!   漢数字・丸数字)。桁数の下限は無い (1 桁でも漢数字などを出す)。
//! - [`predict`]: スマホ版 NumberFormatter.predict (4 桁以上だけ、桁区切り・
//!   万混ぜ・漢数字の 3 形)。
//! どちらも文字列のまま扱うので大きい数でも桁が落ちない。datetime.rs は
//! number_forms を再実装せず、ここのものを呼ぶ。

use crate::{u16s, U16};

const DIGIT_KANJI: [&str; 10] = ["〇", "一", "二", "三", "四", "五", "六", "七", "八", "九"];
const SMALL_UNITS: [&str; 4] = ["", "十", "百", "千"]; // 4 桁の組の中の位
const BIG_UNITS: [&str; 5] = ["", "万", "億", "兆", "京"]; // 4 桁の組ごと
const MAX_DIGITS: usize = 20;

/// 全角の数字も半角にした数字の列 (ASCII 数字だけの U16)。数字だけでなければ None。
/// 元の normalizeDigits と同じ。
pub fn normalize_digits(s: &[u16]) -> Option<U16> {
    if s.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(s.len());
    for &c in s {
        if (0x30..=0x39).contains(&c) {
            out.push(c);
        } else if (0xFF10..=0xFF19).contains(&c) {
            out.push(0x30 + (c - 0xFF10));
        } else {
            return None;
        }
    }
    Some(out)
}

/// 桁区切り (1234567 → 1,234,567)。入力は半角化済みの数字列。
pub fn add_commas(digits: &[u16]) -> U16 {
    let mut out = Vec::new();
    let n = digits.len();
    for (i, &c) in digits.iter().enumerate() {
        if i > 0 && (n - i) % 3 == 0 {
            out.push(b',' as u16);
        }
        out.push(c);
    }
    out
}

/// 右から 4 桁ずつ。[0] がいちばん下の組。
fn groups_of(s: &[u16]) -> Vec<U16> {
    let mut g = Vec::new();
    let mut end = s.len();
    while end > 0 {
        let start = end.saturating_sub(4);
        g.push(s[start..end].to_vec());
        end = start;
    }
    g
}

/// 先行ゼロを落とす (全部ゼロなら 1 桁残す)。
fn strip_zeros(s: &[u16]) -> &[u16] {
    let mut t = s;
    while t.len() > 1 && t[0] == b'0' as u16 {
        t = &t[1..];
    }
    t
}

/// 1〜4 桁の組を千百十の漢数字に (千・百・十の前の一は書かない)。
fn group_kanji(g: &[u16]) -> U16 {
    let mut out = Vec::new();
    for (i, &c) in g.iter().enumerate() {
        let d = (c - b'0' as u16) as usize;
        let place = g.len() - 1 - i;
        if d == 0 {
            continue;
        }
        if d != 1 || place == 0 {
            out.extend(u16s(DIGIT_KANJI[d]));
        }
        out.extend(u16s(SMALL_UNITS[place]));
    }
    out
}

/// 漢数字 (12345678 → 千二百三十四万五千六百七十八、0 → 〇)。
/// 入力は半角化済みの数字列。
pub fn to_kanji(digits: &[u16]) -> U16 {
    let s = strip_zeros(digits);
    if s.iter().all(|&c| c == b'0' as u16) {
        return u16s(DIGIT_KANJI[0]);
    }
    let gs = groups_of(s);
    let mut out = Vec::new();
    for i in (0..gs.len()).rev() {
        let k = group_kanji(&gs[i]);
        if !k.is_empty() {
            out.extend(k);
            out.extend(u16s(BIG_UNITS[i]));
        }
    }
    out
}

/// 万の混ぜ書き (12345678 → 1234万5678、1000000 → 100万)。
/// 組の中は算用数字、0 の組は飛ばし、下の組は 4 桁に 0 埋めする。
pub fn to_man_mixed(digits: &[u16]) -> U16 {
    let s = strip_zeros(digits);
    let gs = groups_of(s);
    let mut out: U16 = Vec::new();
    for i in (0..gs.len()).rev() {
        if gs[i].iter().all(|&c| c == b'0' as u16) {
            continue;
        }
        if out.is_empty() {
            // いちばん上の組は自然な幅 (先行ゼロを落とす)
            out.extend_from_slice(strip_zeros(&gs[i]));
        } else {
            for _ in 0..(4 - gs[i].len()) {
                out.push(b'0' as u16);
            }
            out.extend_from_slice(&gs[i]);
        }
        out.extend(u16s(BIG_UNITS[i]));
    }
    out
}

/// 年の和暦 (令和・平成・昭和)。1926 年より前は空。
/// 元は datetime.cpp の era_year (number_forms が使うのでここに置く)。
pub fn era_year(y: i64) -> U16 {
    fn num(v: i64) -> U16 {
        u16s(&v.to_string())
    }
    if y >= 2019 {
        if y == 2019 {
            u16s("令和元年")
        } else {
            let mut o = u16s("令和");
            o.extend(num(y - 2018));
            o.extend(u16s("年"));
            o
        }
    } else if y >= 1989 {
        if y == 1989 {
            u16s("平成元年")
        } else {
            let mut o = u16s("平成");
            o.extend(num(y - 1988));
            o.extend(u16s("年"));
            o
        }
    } else if y >= 1926 {
        if y == 1926 {
            u16s("昭和元年")
        } else {
            let mut o = u16s("昭和");
            o.extend(num(y - 1925));
            o.extend(u16s("年"));
            o
        }
    } else {
        Vec::new()
    }
}

fn circled(v: i64) -> u16 {
    // 白丸 0〜50
    if v == 0 {
        0x24EA
    } else if v <= 20 {
        0x2460 + (v as u16) - 1
    } else if v <= 35 {
        0x3251 + (v as u16) - 21
    } else {
        0x32B1 + (v as u16) - 36
    }
}

fn black_circled(v: i64) -> u16 {
    // 黒丸に白抜き 1〜20
    if v <= 10 {
        0x2776 + (v as u16) - 1
    } else {
        0x24EB + (v as u16) - 11
    }
}

/// 数字だけの読みの書き換え (和暦・桁区切り・万の混ぜ書き・漢数字・丸数字)。
/// 数字だけでなければ空、0 で始まるもの (番号らしい) も空。
/// 元: desktop/core/datetime.cpp の number_forms。
pub fn number_forms(reading: &[u16]) -> Vec<U16> {
    if reading.is_empty() || reading.len() > MAX_DIGITS {
        return Vec::new();
    }
    let d = match normalize_digits(reading) {
        Some(d) => d,
        None => return Vec::new(),
    };
    if d[0] == b'0' as u16 && d.len() > 1 {
        return Vec::new(); // 0 で始まるのは番号
    }
    let mut out: Vec<U16> = Vec::new();
    let mut push = |s: U16| {
        if !s.is_empty() && s != reading && !out.contains(&s) {
            out.push(s);
        }
    };
    let mut v: i64 = -1;
    if d.len() <= 4 {
        v = 0;
        for &c in &d {
            v = v * 10 + (c - b'0' as u16) as i64;
        }
    }
    if d.len() == 4 && (1926..=2100).contains(&v) {
        push(era_year(v));
    }
    if d.len() >= 4 {
        push(add_commas(&d));
        push(to_man_mixed(&d));
    }
    if d == u16s("0") {
        push(u16s("〇"));
    } else {
        push(to_kanji(&d));
    }
    if (0..=50).contains(&v) {
        push(vec![circled(v)]);
    }
    if (1..=20).contains(&v) {
        push(vec![black_circled(v)]);
    }
    out
}

/// スマホ版 NumberFormatter.predict: 数字だけの読み (4 桁以上) に対する
/// 桁区切り・万の混ぜ書き・漢数字。数字でない・短すぎる・長すぎる・
/// 0 で始まる (番号らしい) なら空。
pub fn predict(composing: &[u16]) -> Vec<U16> {
    let digits = match normalize_digits(composing) {
        Some(d) => d,
        None => return Vec::new(),
    };
    if digits.len() < 4 || digits.len() > MAX_DIGITS {
        return Vec::new();
    }
    if digits[0] == b'0' as u16 {
        return Vec::new();
    }
    let mut out: Vec<U16> = Vec::new();
    let mut push = |s: U16| {
        if !s.is_empty() && s != composing && !out.contains(&s) {
            out.push(s);
        }
    };
    push(add_commas(&digits));
    push(to_man_mixed(&digits));
    push(to_kanji(&digits));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::from_u16s;

    fn strs(v: Vec<U16>) -> Vec<String> {
        v.iter().map(|s| from_u16s(s)).collect()
    }

    #[test]
    fn predict_basic() {
        assert_eq!(
            strs(predict(&u16s("1000000"))),
            vec!["1,000,000", "100万", "百万"]
        );
        assert_eq!(
            strs(predict(&u16s("12345678"))),
            vec!["12,345,678", "1234万5678", "千二百三十四万五千六百七十八"]
        );
    }

    #[test]
    fn predict_rejects() {
        assert!(predict(&u16s("123")).is_empty()); // 4 桁未満
        assert!(predict(&u16s("0123")).is_empty()); // 0 始まり
        assert!(predict(&u16s("12a4")).is_empty()); // 数字でない
        assert!(predict(&u16s("")).is_empty());
        assert!(predict(&u16s("123456789012345678901")).is_empty()); // 21 桁
    }

    #[test]
    fn predict_fullwidth() {
        // 全角入力では半角の数字そのものも形の 1 つになる (元の push の
        // 比較は元の読みとの比較なので、C++ 版 number_forms でも同じ)
        assert_eq!(
            strs(predict(&u16s("１０００"))),
            vec!["1,000", "1000", "千"]
        );
        // 半角なら元の数字と同形の万混ぜは落ちる
        assert_eq!(strs(predict(&u16s("1000"))), vec!["1,000", "千"]);
    }

    #[test]
    fn man_mixed_zero_padding() {
        // 下の組は 4 桁に 0 埋め (1234万0056)
        assert_eq!(from_u16s(&to_man_mixed(&u16s("12340056"))), "1234万0056");
        assert_eq!(from_u16s(&to_man_mixed(&u16s("1000000"))), "100万");
        assert_eq!(from_u16s(&to_man_mixed(&u16s("10000"))), "1万");
    }

    #[test]
    fn kanji_groups() {
        assert_eq!(from_u16s(&to_kanji(&u16s("0"))), "〇");
        assert_eq!(from_u16s(&to_kanji(&u16s("10"))), "十");
        assert_eq!(from_u16s(&to_kanji(&u16s("1000"))), "千");
        assert_eq!(from_u16s(&to_kanji(&u16s("10000"))), "一万");
        assert_eq!(from_u16s(&to_kanji(&u16s("100000000"))), "一億");
        assert_eq!(from_u16s(&to_kanji(&u16s("2026"))), "二千二十六");
    }

    #[test]
    fn number_forms_small() {
        // 1 桁でも漢数字・丸数字が出る (C++ 版 number_forms)
        assert_eq!(strs(number_forms(&u16s("5"))), vec!["五", "⑤", "❺"]);
        assert_eq!(strs(number_forms(&u16s("0"))), vec!["〇", "⓪"]);
        assert_eq!(strs(number_forms(&u16s("21"))), vec!["二十一", "㉑"]);
    }

    #[test]
    fn number_forms_year() {
        let f = strs(number_forms(&u16s("2026")));
        assert_eq!(f[0], "令和8年"); // 和暦が先頭
        assert!(f.contains(&"2,026".to_string()));
        assert!(f.contains(&"二千二十六".to_string()));
        assert_eq!(from_u16s(&era_year(1989)), "平成元年");
        assert_eq!(from_u16s(&era_year(1926)), "昭和元年");
        assert!(era_year(1925).is_empty());
    }

    #[test]
    fn number_forms_rejects() {
        assert!(number_forms(&u16s("007")).is_empty());
        assert!(number_forms(&u16s("あ1")).is_empty());
        assert!(number_forms(&u16s("")).is_empty());
    }

    #[test]
    fn normalize_digits_works() {
        assert_eq!(normalize_digits(&u16s("１２3")), Some(u16s("123")));
        assert_eq!(normalize_digits(&u16s("1.5")), None);
    }
}
