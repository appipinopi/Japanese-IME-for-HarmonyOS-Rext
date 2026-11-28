//! 品詞の表 (元: desktop/core/pos_classes.h の POS_CLASSES・POS_GROUPS、
//! Pos.ets / Pos.kt)。辞書の左 ID ごとに 1 字。
//! pos_classes.h は Mozc の id.def からの生成物で、手で直さない決まりだった。
//! このファイルの表も生成値そのまま (テストで長さと代表値を固定する)。
//!
//! 文節の分類 (POS_CLASSES): 1 = 付属語 (前につく)、2 = 接頭 (次がつく)、
//! 3 = サ変接続の名詞、4 = サ変・スルの動詞 (3 の後ろならつく)、0 = 自立語。
//! 粗い品詞 (POS_GROUPS): N 名詞・n 名詞非自立・S 名詞接尾・V 動詞・
//! v 動詞非自立/接尾・A 形容詞・a 形容詞非自立/接尾・P 助詞・X 助動詞・
//! F 接頭詞・D 副詞・R 連体詞・C 接続詞・I 感動詞・K 記号・O それ以外。

use crate::U16;

/// 品詞 ID の総数 (表の長さ)。
pub const POS_CLASS_COUNT: usize = 2672;

/// 文節の分類の表 (左 ID ごとに 1 字)。
pub const POS_CLASSES: &str = concat!(
    "0000000000000000000000000000011111111111111111111111111111111111111111111111111111111111111111111111",
    "1111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111",
    "1111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111",
    "1111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111",
    "1111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111",
    "1111111111111111111111111111111111111111111111111111111111111111111111111111100000000000000000000000",
    "0000000000000000000000000004444444444440000000000000000000000000000000000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000000000000001111111111111111111111111111111111111111111",
    "1111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111",
    "1111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111",
    "1111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111",
    "1111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111",
    "1111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111",
    "1111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111",
    "1111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111",
    "1111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111",
    "1111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111",
    "1111111111111111111111111111111111111111133333333300000000000000000000000000000000000000000000000000",
    "0000000000000000000000000000000000001111111111111111111111111111111111111111111111111111111111111111",
    "1111111111111111111111111111111111111111110000000000000000000000000000000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001111",
    "1111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111",
    "1111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111000000000",
    "0000000000000000000000000000000000000000000000000000000000000000000000000111111111111111111111111111",
    "1111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111000002222",
    "222222222222222222222222222222222222222222200111111111112110000000000000",
);

/// 学習を分ける粗い品詞の表 (左 ID ごとに 1 字)。
pub const POS_GROUPS: &str = concat!(
    "OOIIIIIIIIIIDDDDDDDDDDDDDDDDDXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX",
    "XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX",
    "XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPP",
    "PPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPP",
    "PPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvv",
    "vvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvVVVVVVVVVVVVVVVVVVVVVVV",
    "VVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVV",
    "VVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVV",
    "VVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvv",
    "vvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvv",
    "vvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvv",
    "vvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvv",
    "vvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvv",
    "vvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvv",
    "vvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvv",
    "vvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvv",
    "vvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvv",
    "vvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvv",
    "vvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNN",
    "NNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSS",
    "SSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSNNNNNNNNNNNNNNNnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnn",
    "nnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnaaaa",
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaAAAAAAAAA",
    "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaIICCCFFFF",
    "FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFKKKKKKKKKKKKKKKKRRRRRRRRRRRRR",
);

/// 文節の分類: 1 付属語・2 接頭・3 サ変接続の名詞・4 サ変・スルの動詞・0 自立語。
/// 範囲外・負の ID は '0' (元: converter.cpp の pos_class)。
pub fn pos_class(lid: i32) -> char {
    if lid < 0 {
        return '0';
    }
    POS_CLASSES.as_bytes().get(lid as usize).map(|&b| b as char).unwrap_or('0')
}

/// 学習を分ける粗い品詞 (N 名詞・V 動詞・P 助詞・X 助動詞 …)。
/// 分からなければ '\0' (元: store.cpp の group_of_lid。ArkTS 版は空文字列、
/// Kotlin 版は 0 を返すので C++ に寄せた)。
pub fn group_of_lid(lid: i32) -> char {
    if lid < 0 {
        return '\0';
    }
    POS_GROUPS.as_bytes().get(lid as usize).map(|&b| b as char).unwrap_or('\0')
}

/// 文の中の語 (読み・表記・品詞の左 ID)。元: Pos.ets の PosWord。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PosWord {
    pub reading: U16,
    pub surface: U16,
    pub lid: i32,
}

/// 文の語のうち、読みの位置 r0 から始まる文節の頭の語の品詞 ID
/// (接頭詞 = 分類 '2' は飛ばす)。無ければ -1。
/// 元: composer.cpp の head_lid (Pos.ets / Pos.kt の headLid と同じ)。
pub fn head_lid(ws: &[PosWord], r0: usize) -> i32 {
    let mut r = 0usize;
    for (i, w) in ws.iter().enumerate() {
        if r == r0 {
            let mut k = i;
            while k + 1 < ws.len() && ws[k].lid >= 0 && pos_class(ws[k].lid) == '2' {
                k += 1;
            }
            return ws[k].lid;
        }
        if r > r0 {
            return -1;
        }
        r += w.reading.len();
    }
    -1
}

/// pathPos の結果 ([読みの終わり, 表記の長さ, 左 ID, 右 ID] の並び) を語に分ける。
/// 合わなければ空。元: Pos.ets の Pos.words。
pub fn words(reading: &[u16], text: &[u16], a: &[i32]) -> Vec<PosWord> {
    let mut out = Vec::new();
    let mut r0 = 0usize;
    let mut s0 = 0usize;
    let mut i = 0usize;
    while i + 3 < a.len() {
        let r1 = a[i] as usize;
        let s1 = s0 + a[i + 1] as usize;
        if r1 > reading.len() || s1 > text.len() || r1 < r0 {
            return Vec::new();
        }
        out.push(PosWord {
            reading: reading[r0..r1].to_vec(),
            surface: text[s0..s1].to_vec(),
            lid: a[i + 2],
        });
        r0 = r1;
        s0 = s1;
        i += 4;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::u16s;

    #[test]
    fn table_lengths() {
        assert_eq!(POS_CLASSES.len(), POS_CLASS_COUNT);
        assert_eq!(POS_GROUPS.len(), POS_CLASS_COUNT);
    }

    #[test]
    fn class_values() {
        assert_eq!(pos_class(-1), '0');
        assert_eq!(pos_class(0), '0');
        assert_eq!(pos_class(2672), '0');
        assert_eq!(pos_class(100000), '0');
        assert_eq!(pos_class(30), '1'); // 付属語の帯 (生成表の実データ)
        assert!(POS_CLASSES.contains('2') && POS_CLASSES.contains('3') && POS_CLASSES.contains('4'));
    }

    #[test]
    fn group_values() {
        assert_eq!(group_of_lid(-1), '\0');
        assert_eq!(group_of_lid(2672), '\0');
        assert_eq!(group_of_lid(0), 'O');
        assert_eq!(group_of_lid(2), 'I'); // 感動詞の帯
        assert_eq!(group_of_lid(13), 'D'); // 副詞の帯
    }

    #[test]
    fn head_lid_skips_prefix() {
        let prefix_id = (0..POS_CLASS_COUNT as i32).find(|&i| pos_class(i) == '2').unwrap();
        let noun_id = (0..POS_CLASS_COUNT as i32).find(|&i| group_of_lid(i) == 'N').unwrap();
        let ws = vec![
            PosWord { reading: u16s("お"), surface: u16s("お"), lid: prefix_id },
            PosWord { reading: u16s("かね"), surface: u16s("金"), lid: noun_id },
            PosWord { reading: u16s("を"), surface: u16s("を"), lid: -1 },
        ];
        assert_eq!(head_lid(&ws, 0), noun_id); // 接頭を飛ばす
        assert_eq!(head_lid(&ws, 1), noun_id);
        assert_eq!(head_lid(&ws, 3), -1);
        assert_eq!(head_lid(&ws, 2), -1); // 語の途中は合わない
        assert_eq!(head_lid(&[], 0), -1);
    }

    #[test]
    fn words_split() {
        let reading = u16s("きょうは");
        let text = u16s("今日は");
        let a = [3, 2, 100, 100, 4, 1, 200, 200];
        let ws = words(&reading, &text, &a);
        assert_eq!(ws.len(), 2);
        assert_eq!(ws[0].reading, u16s("きょう"));
        assert_eq!(ws[0].surface, u16s("今日"));
        assert_eq!(ws[1].lid, 200);
        assert!(words(&reading, &text, &[99, 1, 0, 0]).is_empty());
        assert!(words(&reading, &text, &[]).is_empty());
    }
}
