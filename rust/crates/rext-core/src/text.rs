//! 文字の変換 (UTF-8 と UTF-16、ひらがな・カタカナ・半角カナ・全角英数)。
//! 元: desktop/core/text.cpp / text.h (namespace shunti)。
//!
//! from_utf8 / to_utf8 相当は Rust 側では `crate::u16s` (str::encode_utf16) と
//! `crate::from_u16s` (String::from_utf16_lossy) を使うため、ここには置かない。
//! 挙動の違い: 元の from_utf8 は不正なバイト列を U+FFFD に置き換えつつ自前で
//! デコードするが、encode_utf16 は &str (妥当な UTF-8) を前提とする。
//! to_utf8 は元のものが孤立サロゲートをそのまま 3 バイトで出すのに対し、
//! from_utf16_lossy は U+FFFD に置き換える。

use crate::U16;

/// ひらがな (ぁ U+3041 〜 ゖ U+3096)
pub fn is_hira(c: u16) -> bool {
    (0x3041..=0x3096).contains(&c)
}

/// カタカナ (ァ U+30A1 〜 ヶ U+30F6)
pub fn is_kata(c: u16) -> bool {
    (0x30A1..=0x30F6).contains(&c)
}

/// かな漢字変換の読みになる字 (ひらがなと長音、ゔ U+3094)
pub fn is_reading_char(c: u16) -> bool {
    is_hira(c) || c == 0x30FC || c == 0x3094
}

/// ASCII の英数字
pub fn is_ascii_alnum(c: u16) -> bool {
    (b'0' as u16..=b'9' as u16).contains(&c)
        || (b'a' as u16..=b'z' as u16).contains(&c)
        || (b'A' as u16..=b'Z' as u16).contains(&c)
}

/// ひらがな → カタカナ (+0x60)。それ以外はそのまま
pub fn to_katakana(s: &[u16]) -> U16 {
    s.iter()
        .map(|&c| if is_hira(c) { c + 0x60 } else { c })
        .collect()
}

/// カタカナ → ひらがな (-0x60)。対象は ァ U+30A1 〜 ヶ U+30F6 の範囲のみ
pub fn to_hiragana(s: &[u16]) -> U16 {
    s.iter()
        .map(|&c| {
            if (0x30A1..=0x30F6).contains(&c) {
                c - 0x60
            } else {
                c
            }
        })
        .collect()
}

// 全角カタカナ (濁点・半濁点の無い形) と半角カナの組
const FULL: &str = "ァアィイゥウェエォオカキクケコサシスセソタチッツテトナニヌネノハヒフヘホマミムメモャヤュユョヨラリルレロワヲンーヮヰヱヵヶ。、「」・";
const HALF: &str = "ｧｱｨｲｩｳｪｴｫｵｶｷｸｹｺｻｼｽｾｿﾀﾁｯﾂﾃﾄﾅﾆﾇﾈﾉﾊﾋﾌﾍﾎﾏﾐﾑﾒﾓｬﾔｭﾕｮﾖﾗﾘﾙﾚﾛﾜｦﾝｰﾜｲｴｶｹ｡､｢｣･";
const VOICED: &str = "ガギグゲゴザジズゼゾダヂヅデドバビブベボ";
const SEMI: &str = "パピプペポ";

fn index_of(s: &[u16], c: u16) -> Option<usize> {
    s.iter().position(|&x| x == c)
}

/// 全角カタカナ・ひらがな・記号 → 半角カナ (ガ → ｶﾞ)
pub fn to_halfwidth_kana(s: &[u16]) -> U16 {
    let full = crate::u16s(FULL);
    let half = crate::u16s(HALF);
    let voiced = crate::u16s(VOICED);
    let semi = crate::u16s(SEMI);
    let mut o: U16 = Vec::new();
    for c in to_katakana(s) {
        if let Some(i) = index_of(&full, c) {
            o.push(half[i]);
            continue;
        }
        // 濁点: 清音 (c-1) が FULL にあれば 半角 + ﾞ
        if index_of(&voiced, c).is_some() {
            if let Some(i) = index_of(&full, c - 1) {
                o.push(half[i]);
                o.push(0xFF9E);
                continue;
            }
        }
        // 半濁点: 清音 (c-2) が FULL にあれば 半角 + ﾟ
        if index_of(&semi, c).is_some() {
            if let Some(i) = index_of(&full, c - 2) {
                o.push(half[i]);
                o.push(0xFF9F);
                continue;
            }
        }
        if c == 0x30F4 {
            // ヴ → ｳﾞ
            o.extend_from_slice(&crate::u16s("ｳﾞ"));
            continue;
        }
        if c == 0x30F7 {
            // ヷ → ﾜﾞ
            o.extend_from_slice(&crate::u16s("ﾜﾞ"));
            continue;
        }
        o.extend_from_slice(&to_halfwidth_ascii(&[c]));
    }
    o
}

/// ASCII → 全角 (空白は全角の空白)。ASCII 以外はそのまま
pub fn to_fullwidth_ascii(s: &[u16]) -> U16 {
    s.iter()
        .map(|&c| {
            if c == b' ' as u16 {
                0x3000
            } else if c > 0x20 && c < 0x7F {
                c + 0xFEE0
            } else {
                c
            }
        })
        .collect()
}

/// 全角英数記号 → ASCII。それ以外はそのまま
pub fn to_halfwidth_ascii(s: &[u16]) -> U16 {
    s.iter()
        .map(|&c| {
            if c == 0x3000 {
                b' ' as u16
            } else if c > 0xFF00 && c < 0xFF5F {
                c - 0xFEE0
            } else if c == 0xFFE5 {
                b'\\' as u16
            } else {
                c
            }
        })
        .collect()
}

/// 打った ASCII の記号を、日本語入力のときの字に (, → 、 . → 。 [ → 「 など。それ以外の記号は全角に)
pub fn japanese_symbol(c: u16) -> u16 {
    match c {
        x if x == b',' as u16 => 0x3001,  // 、
        x if x == b'.' as u16 => 0x3002,  // 。
        x if x == b'[' as u16 => 0x300C,  // 「
        x if x == b']' as u16 => 0x300D,  // 」
        x if x == b'/' as u16 => 0x30FB,  // ・
        x if x == b'-' as u16 => 0x30FC,  // ー
        x if x == b'\\' as u16 => 0xFFE5, // ￥
        _ => {
            if c > 0x20 && c < 0x7F {
                c + 0xFEE0
            } else {
                c
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{from_u16s, u16s};

    fn s(v: &[u16]) -> String {
        from_u16s(v)
    }

    #[test]
    fn char_classes() {
        assert!(is_hira(u16s("ぁ")[0]));
        assert!(is_hira(u16s("ゖ")[0]));
        assert!(!is_hira(u16s("ァ")[0]));
        assert!(!is_hira(u16s("ぁ")[0] - 1)); // ぀ は範囲外
        assert!(is_kata(u16s("ァ")[0]));
        assert!(is_kata(u16s("ヶ")[0]));
        assert!(!is_kata(u16s("ー")[0]));
        assert!(is_reading_char(u16s("あ")[0]));
        assert!(is_reading_char(u16s("ー")[0]));
        assert!(is_reading_char(u16s("ゔ")[0]));
        assert!(!is_reading_char(u16s("ア")[0]));
        assert!(is_ascii_alnum(b'0' as u16));
        assert!(is_ascii_alnum(b'z' as u16));
        assert!(is_ascii_alnum(b'A' as u16));
        assert!(!is_ascii_alnum(b'-' as u16));
        assert!(!is_ascii_alnum(u16s("あ")[0]));
    }

    #[test]
    fn katakana_hiragana_roundtrip() {
        assert_eq!(s(&to_katakana(&u16s("あいうえおゔゕゖ"))), "アイウエオヴヵヶ");
        assert_eq!(s(&to_hiragana(&u16s("アイウエオヴヵヶ"))), "あいうえおゔゕゖ");
        // 範囲外はそのまま: ー (U+30FC)・・(U+30FB) は変換されない
        assert_eq!(s(&to_hiragana(&u16s("カレー・"))), "かれー・");
        assert_eq!(s(&to_katakana(&u16s("漢字a1"))), "漢字a1");
        assert_eq!(s(&to_hiragana(&u16s("漢字a1"))), "漢字a1");
    }

    #[test]
    fn halfwidth_kana_basic() {
        // desktop/tests/core_test.cpp のゴールデン
        assert_eq!(s(&to_halfwidth_kana(&u16s("ガッコウ　パン"))), "ｶﾞｯｺｳ ﾊﾟﾝ");
        assert_eq!(s(&to_halfwidth_kana(&u16s("あいうえお"))), "ｱｲｳｴｵ");
        assert_eq!(s(&to_halfwidth_kana(&u16s("。「」・ー"))), "｡｢｣･ｰ");
        assert_eq!(s(&to_halfwidth_kana(&u16s("ヰヱヮ"))), "ｲｴﾜ");
    }

    #[test]
    fn halfwidth_kana_voiced_compose() {
        // 濁点・半濁点の合成 (清音 + ﾞ / ﾟ)
        assert_eq!(
            s(&to_halfwidth_kana(&u16s("ガギグゲゴザジズゼゾダヂヅデドバビブベボ"))),
            "ｶﾞｷﾞｸﾞｹﾞｺﾞｻﾞｼﾞｽﾞｾﾞｿﾞﾀﾞﾁﾞﾂﾞﾃﾞﾄﾞﾊﾞﾋﾞﾌﾞﾍﾞﾎﾞ"
        );
        assert_eq!(s(&to_halfwidth_kana(&u16s("パピプペポ"))), "ﾊﾟﾋﾟﾌﾟﾍﾟﾎﾟ");
        // ヴ・ヷ は特別扱い
        assert_eq!(s(&to_halfwidth_kana(&u16s("ヴヷ"))), "ｳﾞﾜﾞ");
        // ひらがなの濁点もまずカタカナ化されてから合成される
        assert_eq!(s(&to_halfwidth_kana(&u16s("がぱ"))), "ｶﾞﾊﾟ");
    }

    #[test]
    fn halfwidth_kana_fallback() {
        // 表に無い字は to_halfwidth_ascii 経由 (全角英数は半角に、漢字はそのまま)
        assert_eq!(s(&to_halfwidth_kana(&u16s("Ａ漢"))), "A漢");
    }

    #[test]
    fn fullwidth_halfwidth_ascii_conv() {
        assert_eq!(s(&to_fullwidth_ascii(&u16s("Az 09!~"))), "Ａｚ　０９！～");
        assert_eq!(s(&to_fullwidth_ascii(&u16s("あ"))), "あ");
        assert_eq!(s(&to_halfwidth_ascii(&u16s("Ａｚ　０９！～"))), "Az 09!~");
        // ￥ (U+FFE5) は範囲外の特別扱いで \ になる
        assert_eq!(s(&to_halfwidth_ascii(&u16s("￥"))), "\\");
        assert_eq!(s(&to_halfwidth_ascii(&u16s("あ"))), "あ");
    }

    #[test]
    fn japanese_symbol_map() {
        let f = |ch: char| japanese_symbol(ch.encode_utf16(&mut [0; 1])[0]);
        assert_eq!(f(','), u16s("、")[0]);
        assert_eq!(f('.'), u16s("。")[0]);
        assert_eq!(f('['), u16s("「")[0]);
        assert_eq!(f(']'), u16s("」")[0]);
        assert_eq!(f('/'), u16s("・")[0]);
        assert_eq!(f('-'), u16s("ー")[0]);
        assert_eq!(f('\\'), 0xFFE5);
        // それ以外の記号は全角に、英数字・空白・非 ASCII はそのまま
        assert_eq!(f('!'), u16s("！")[0]);
        assert_eq!(f('a'), u16s("ａ")[0]);
        assert_eq!(f(' '), b' ' as u16);
        assert_eq!(japanese_symbol(u16s("あ")[0]), u16s("あ")[0]);
    }

    #[test]
    fn utf16_helpers_roundtrip() {
        // from_utf8 / to_utf8 相当 (crate::u16s / crate::from_u16s)
        let v = u16s("かな漢字😀");
        assert_eq!(s(&v), "かな漢字😀");
        assert_eq!(v.len(), 6); // サロゲートペアで 2 単位
    }
}
