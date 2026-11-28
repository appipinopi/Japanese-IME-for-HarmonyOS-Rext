//! 静的テーブル類 (元の desktop/core/tables.h + pos_classes.h の移植。
//! 元は生成物: desktop/tools/gen_core_tables.mjs / gen_pos_classes.py。本ファイルも元の表から機械的に写した)

/// ローマ字 → かなの対 (key, kana)。元の ROMAJI_TABLE
pub static ROMAJI_TABLE: &[(&str, &str)] = &[
    ("a", "あ"),
    ("i", "い"),
    ("u", "う"),
    ("e", "え"),
    ("o", "お"),
    ("-", "ー"),
    ("ka", "か"),
    ("ki", "き"),
    ("ku", "く"),
    ("ke", "け"),
    ("ko", "こ"),
    ("kya", "きゃ"),
    ("kyi", "きぃ"),
    ("kyu", "きゅ"),
    ("kye", "きぇ"),
    ("kyo", "きょ"),
    ("sa", "さ"),
    ("si", "し"),
    ("su", "す"),
    ("se", "せ"),
    ("so", "そ"),
    ("shi", "し"),
    ("sha", "しゃ"),
    ("shu", "しゅ"),
    ("she", "しぇ"),
    ("sho", "しょ"),
    ("sya", "しゃ"),
    ("syi", "しぃ"),
    ("syu", "しゅ"),
    ("sye", "しぇ"),
    ("syo", "しょ"),
    ("ta", "た"),
    ("ti", "ち"),
    ("tu", "つ"),
    ("te", "て"),
    ("to", "と"),
    ("chi", "ち"),
    ("cha", "ちゃ"),
    ("chu", "ちゅ"),
    ("che", "ちぇ"),
    ("cho", "ちょ"),
    ("tsu", "つ"),
    ("tya", "ちゃ"),
    ("tyi", "ちぃ"),
    ("tyu", "ちゅ"),
    ("tye", "ちぇ"),
    ("tyo", "ちょ"),
    ("thi", "てぃ"),
    ("tha", "てゃ"),
    ("thu", "てゅ"),
    ("the", "てぇ"),
    ("tho", "てょ"),
    ("twu", "とぅ"),
    ("na", "な"),
    ("ni", "に"),
    ("nu", "ぬ"),
    ("ne", "ね"),
    ("no", "の"),
    ("nya", "にゃ"),
    ("nyi", "にぃ"),
    ("nyu", "にゅ"),
    ("nye", "にぇ"),
    ("nyo", "にょ"),
    ("nn", "ん"),
    ("ha", "は"),
    ("hi", "ひ"),
    ("hu", "ふ"),
    ("he", "へ"),
    ("ho", "ほ"),
    ("fu", "ふ"),
    ("fa", "ふぁ"),
    ("fi", "ふぃ"),
    ("fe", "ふぇ"),
    ("fo", "ふぉ"),
    ("hya", "ひゃ"),
    ("hyi", "ひぃ"),
    ("hyu", "ひゅ"),
    ("hye", "ひぇ"),
    ("hyo", "ひょ"),
    ("ma", "ま"),
    ("mi", "み"),
    ("mu", "む"),
    ("me", "め"),
    ("mo", "も"),
    ("mya", "みゃ"),
    ("myi", "みぃ"),
    ("myu", "みゅ"),
    ("mye", "みぇ"),
    ("myo", "みょ"),
    ("ya", "や"),
    ("yi", "い"),
    ("yu", "ゆ"),
    ("ye", "いぇ"),
    ("yo", "よ"),
    ("ra", "ら"),
    ("ri", "り"),
    ("ru", "る"),
    ("re", "れ"),
    ("ro", "ろ"),
    ("rya", "りゃ"),
    ("ryi", "りぃ"),
    ("ryu", "りゅ"),
    ("rye", "りぇ"),
    ("ryo", "りょ"),
    ("wa", "わ"),
    ("wi", "うぃ"),
    ("we", "うぇ"),
    ("wo", "を"),
    ("ga", "が"),
    ("gi", "ぎ"),
    ("gu", "ぐ"),
    ("ge", "げ"),
    ("go", "ご"),
    ("gya", "ぎゃ"),
    ("gyi", "ぎぃ"),
    ("gyu", "ぎゅ"),
    ("gye", "ぎぇ"),
    ("gyo", "ぎょ"),
    ("za", "ざ"),
    ("zi", "じ"),
    ("zu", "ず"),
    ("ze", "ぜ"),
    ("zo", "ぞ"),
    ("ji", "じ"),
    ("ja", "じゃ"),
    ("ju", "じゅ"),
    ("je", "じぇ"),
    ("jo", "じょ"),
    ("jya", "じゃ"),
    ("jyi", "じぃ"),
    ("jyu", "じゅ"),
    ("jye", "じぇ"),
    ("jyo", "じょ"),
    ("zya", "じゃ"),
    ("zyi", "じぃ"),
    ("zyu", "じゅ"),
    ("zye", "じぇ"),
    ("zyo", "じょ"),
    ("da", "だ"),
    ("di", "ぢ"),
    ("du", "づ"),
    ("de", "で"),
    ("do", "ど"),
    ("dya", "ぢゃ"),
    ("dyi", "ぢぃ"),
    ("dyu", "ぢゅ"),
    ("dye", "ぢぇ"),
    ("dyo", "ぢょ"),
    ("dhi", "でぃ"),
    ("dha", "でゃ"),
    ("dhu", "でゅ"),
    ("dhe", "でぇ"),
    ("dho", "でょ"),
    ("dwu", "どぅ"),
    ("ba", "ば"),
    ("bi", "び"),
    ("bu", "ぶ"),
    ("be", "べ"),
    ("bo", "ぼ"),
    ("bya", "びゃ"),
    ("byi", "びぃ"),
    ("byu", "びゅ"),
    ("bye", "びぇ"),
    ("byo", "びょ"),
    ("pa", "ぱ"),
    ("pi", "ぴ"),
    ("pu", "ぷ"),
    ("pe", "ぺ"),
    ("po", "ぽ"),
    ("pya", "ぴゃ"),
    ("pyi", "ぴぃ"),
    ("pyu", "ぴゅ"),
    ("pye", "ぴぇ"),
    ("pyo", "ぴょ"),
    ("va", "ゔぁ"),
    ("vi", "ゔぃ"),
    ("vu", "ゔ"),
    ("ve", "ゔぇ"),
    ("vo", "ゔぉ"),
    ("xa", "ぁ"),
    ("xi", "ぃ"),
    ("xu", "ぅ"),
    ("xe", "ぇ"),
    ("xo", "ぉ"),
    ("xya", "ゃ"),
    ("xyu", "ゅ"),
    ("xyo", "ょ"),
    ("xtu", "っ"),
    ("xtsu", "っ"),
    ("xwa", "ゎ"),
    ("la", "ぁ"),
    ("li", "ぃ"),
    ("lu", "ぅ"),
    ("le", "ぇ"),
    ("lo", "ぉ"),
    ("lya", "ゃ"),
    ("lyu", "ゅ"),
    ("lyo", "ょ"),
    ("ltu", "っ"),
    ("ltsu", "っ"),
];

/// 候補に含まれていたら弾く語 (not_after = その語の直後に来ていれば弾かない語)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlurRule { pub word: &'static str, pub not_after: &'static [&'static str] }
pub static SLURS: &[SlurRule] = &[
    SlurRule { word: "ガイジ", not_after: &[] },
    SlurRule { word: "害児", not_after: &[] },
    SlurRule { word: "池沼", not_after: &[] },
    SlurRule { word: "キチガイ", not_after: &[] },
    SlurRule { word: "きちがい", not_after: &[] },
    SlurRule { word: "気違い", not_after: &[] },
    SlurRule { word: "気狂い", not_after: &[] },
    SlurRule { word: "基地外", not_after: &[] },
    SlurRule { word: "キチゲェ", not_after: &[] },
    SlurRule { word: "チョン", not_after: &["マゲ", "ボ", "チョン", "ガー"] },
    SlurRule { word: "チャンコロ", not_after: &[] },
    SlurRule { word: "支那人", not_after: &[] },
    SlurRule { word: "シナ人", not_after: &[] },
    SlurRule { word: "土人", not_after: &[] },
    SlurRule { word: "ニガー", not_after: &[] },
    SlurRule { word: "ニガ―", not_after: &[] },
    SlurRule { word: "黒んぼ", not_after: &[] },
    SlurRule { word: "クロンボ", not_after: &[] },
    SlurRule { word: "エタ非人", not_after: &[] },
    SlurRule { word: "めくら", not_after: &[] },
    SlurRule { word: "メクラ", not_after: &[] },
    SlurRule { word: "つんぼ", not_after: &[] },
    SlurRule { word: "ツンボ", not_after: &[] },
    SlurRule { word: "知恵遅れ", not_after: &[] },
    SlurRule { word: "片輪", not_after: &[] },
    SlurRule { word: "かたわ", not_after: &[] },
    SlurRule { word: "三国人", not_after: &[] },
    SlurRule { word: "ジャップ", not_after: &[] },
    SlurRule { word: "鮮人", not_after: &[] },
    SlurRule { word: "ホモガキ", not_after: &[] },
];

/// 決まり文句の予測 (読みの頭を min_prefix 字以上打ったら、候補の先頭に出す)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Phrase2 { pub reading: &'static str, pub text: &'static str, pub min_prefix: i32 }
pub static PREDICTIVE_PHRASES: &[Phrase2] = &[
    Phrase2 { reading: "あけましておめでとうございます", text: "あけましておめでとうございます", min_prefix: 4 },
    Phrase2 { reading: "おめでとうございます", text: "おめでとうございます", min_prefix: 6 },
    Phrase2 { reading: "たんじょうびおめでとうございます", text: "誕生日おめでとうございます", min_prefix: 8 },
    Phrase2 { reading: "よろしくおねがいします", text: "よろしくお願いします", min_prefix: 5 },
    Phrase2 { reading: "よろしくおねがいいたします", text: "よろしくお願いいたします", min_prefix: 5 },
    Phrase2 { reading: "どうぞよろしくおねがいします", text: "どうぞよろしくお願いします", min_prefix: 5 },
    Phrase2 { reading: "こんごともよろしくおねがいします", text: "今後ともよろしくお願いします", min_prefix: 6 },
    Phrase2 { reading: "なにとぞよろしくおねがいもうしあげます", text: "何卒よろしくお願い申し上げます", min_prefix: 5 },
    Phrase2 { reading: "おつかれさまでした", text: "お疲れ様でした", min_prefix: 6 },
    Phrase2 { reading: "おつかれさまです", text: "お疲れ様です", min_prefix: 6 },
    Phrase2 { reading: "ありがとうございます", text: "ありがとうございます", min_prefix: 6 },
    Phrase2 { reading: "ありがとうございました", text: "ありがとうございました", min_prefix: 6 },
    Phrase2 { reading: "いつもありがとうございます", text: "いつもありがとうございます", min_prefix: 5 },
    Phrase2 { reading: "おせわになっております", text: "お世話になっております", min_prefix: 5 },
    Phrase2 { reading: "おせわになりました", text: "お世話になりました", min_prefix: 5 },
    Phrase2 { reading: "たいへんおせわになりました", text: "大変お世話になりました", min_prefix: 6 },
    Phrase2 { reading: "ごぶさたしております", text: "ご無沙汰しております", min_prefix: 5 },
    Phrase2 { reading: "しつれいします", text: "失礼します", min_prefix: 5 },
    Phrase2 { reading: "しつれいいたします", text: "失礼いたします", min_prefix: 5 },
    Phrase2 { reading: "かしこまりました", text: "かしこまりました", min_prefix: 6 },
    Phrase2 { reading: "しょうちいたしました", text: "承知いたしました", min_prefix: 6 },
    Phrase2 { reading: "おまたせしました", text: "お待たせしました", min_prefix: 5 },
    Phrase2 { reading: "おげんきですか", text: "お元気ですか", min_prefix: 5 },
    Phrase2 { reading: "おはようございます", text: "おはようございます", min_prefix: 6 },
    Phrase2 { reading: "たいへんもうしわけございません", text: "大変申し訳ございません", min_prefix: 6 },
    Phrase2 { reading: "おてすうをおかけしますが", text: "お手数をおかけしますが", min_prefix: 5 },
    Phrase2 { reading: "ごかくにんよろしくおねがいします", text: "ご確認よろしくお願いします", min_prefix: 6 },
    Phrase2 { reading: "よいおとしを", text: "よいお年を", min_prefix: 4 },
];

/// 品詞 ID の数 (元の POS_CLASS_COUNT)
pub const POS_CLASS_COUNT: usize = 2672;
/// 品詞 ID ごとの文節の分類: 1 = 付属語 (前につく)、2 = 接頭 (次がつく)、3 = サ変接続の名詞、
/// 4 = サ変・スルの動詞 (3 の後ろならつく)、0 = 自立語。元の POS_CLASSES (ID 番目の文字)
pub static POS_CLASSES: &str = "00000000000000000000000000000111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111110000000000000000000000000000000000000000000000000044444444444400000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111333333333000000000000000000000000000000000000000000000000000000000000000000000000000000000000001111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000011111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111100000000000000000000000000000000000000000000000000000000000000000000000000000000001111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111111000002222222222222222222222222222222222222222222222200111111111112110000000000000";
/// 学習を分ける粗い品詞 (N 名詞・n 名詞非自立・S 名詞接尾・V 動詞・v 動詞非自立/接尾・A 形容詞・
/// a 形容詞非自立/接尾・P 助詞・X 助動詞・F 接頭詞・D 副詞・R 連体詞・C 接続詞・I 感動詞・K 記号・O それ以外)。元の POS_GROUPS
pub static POS_GROUPS: &str = "OOIIIIIIIIIIDDDDDDDDDDDDDDDDDXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPPvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVVvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvvNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNNSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSSNNNNNNNNNNNNNNNnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaIICCCFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFKKKKKKKKKKKKKKKKRRRRRRRRRRRRR";

/// 品詞 ID の文節の分類 (範囲外は 0 = 自立語)
pub fn pos_class(id: usize) -> u8 {
    POS_CLASSES.as_bytes().get(id).map_or(b'0', |b| *b) - b'0'
}
/// 品詞 ID の粗い品詞 (範囲外は 'O')
pub fn pos_group(id: usize) -> char {
    POS_GROUPS.as_bytes().get(id).map_or(b'O', |b| *b) as char
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn romaji_table_shape() {
        assert_eq!(ROMAJI_TABLE.len(), 200);
        assert_eq!(ROMAJI_TABLE[0], ("a", "あ"));
        assert!(ROMAJI_TABLE.contains(&("shi", "し")));
        assert!(ROMAJI_TABLE.contains(&("xtsu", "っ")));
        for (k, v) in ROMAJI_TABLE { assert!(!k.is_empty() && !v.is_empty()); }
    }
    #[test]
    fn slurs_and_phrases() {
        assert_eq!(SLURS.len(), 30);
        let chon = SLURS.iter().find(|s| s.word == "チョン").unwrap();
        assert_eq!(chon.not_after, &["マゲ", "ボ", "チョン", "ガー"]);
        assert_eq!(PREDICTIVE_PHRASES.len(), 28);
        assert!(PREDICTIVE_PHRASES.iter().all(|p| p.min_prefix > 0));
    }
    #[test]
    fn pos_tables() {
        assert_eq!(POS_CLASSES.len(), POS_CLASS_COUNT);
        assert_eq!(POS_GROUPS.len(), POS_CLASS_COUNT);
        assert_eq!(pos_class(0), 0);
        assert_eq!(pos_class(29), 1); // 先頭の 0 の並びの直後
        assert_eq!(pos_class(99999), 0);
        assert_eq!(pos_group(0), 'O');
        assert_eq!(pos_group(99999), 'O');
    }
}
