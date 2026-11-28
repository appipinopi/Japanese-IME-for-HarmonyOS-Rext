//! ローマ字 → かな (HarmonyOS 版 ime/JapaneseConverter.ets・Android 版 JapaneseConverter.kt の移植。決まりは向こうと同じ)。
//! 元: desktop/core/romaji.cpp / romaji.h。
//! 表は HarmonyOS 版と同じもの (desktop/core/tables.h の ROMAJI_TABLE) に、PC のキーボードでよく打つ綴り
//! (ca・qa・kwa・tsa・wha・n' など) と z で始まる記号 (zh → ← など) を足したもの。
//! tables.rs は別担当の移植待ちのため、ROMAJI_TABLE はこのファイル内に保持する。

use std::collections::{HashMap, HashSet};

use crate::text::{is_hira, to_hiragana};
use crate::{u16s, U16};

const CONSONANTS: &str = "bcdfghjklmnpqrstvwxyz";

fn has(s: &[u16], c: u16) -> bool {
    s.contains(&c)
}

/// ローマ字 → かなの基本表 (desktop/core/tables.h の ROMAJI_TABLE と同じ)
static ROMAJI_TABLE: &[(&str, &str)] = &[
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

/// PC のキーボードでよく打つ綴り (HarmonyOS 版の表に無いもの)
static PC_EXTRA: &[(&str, &str)] = &[
    ("ca", "か"), ("ci", "し"), ("cu", "く"), ("ce", "せ"), ("co", "こ"),
    ("cya", "ちゃ"), ("cyi", "ちぃ"), ("cyu", "ちゅ"), ("cye", "ちぇ"), ("cyo", "ちょ"),
    // 'q' で始まる綴り (qa→くぁ 等) は置かない (2026-11-27 ユーザー決定で
    // HarmonyOS 版 JapaneseConverter.ets に統一)。HarmonyOS 版の表にも q 行は
    // 無く、'q' は接頭辞にもならないため process_buffered のフォールバックで
    // 打ったまま確定する (q → "q"、qa → "q"+"あ")。golden.json の romaji が正。
    ("kwa", "くぁ"), ("gwa", "ぐぁ"),
    ("tsa", "つぁ"), ("tsi", "つぃ"), ("tse", "つぇ"), ("tso", "つぉ"),
    ("wha", "うぁ"), ("whi", "うぃ"), ("whu", "う"), ("whe", "うぇ"), ("who", "うぉ"),
    ("wu", "う"), ("wyi", "ゐ"), ("wye", "ゑ"),
    ("fya", "ふゃ"), ("fyu", "ふゅ"), ("fyo", "ふょ"), ("vyu", "ゔゅ"),
    ("xka", "ヵ"), ("xke", "ヶ"), ("lka", "ヵ"), ("lke", "ヶ"),
    ("zh", "←"), ("zj", "↓"), ("zk", "↑"), ("zl", "→"),
    ("z-", "〜"), ("z.", "…"), ("z,", "‥"), ("z/", "・"), ("z[", "『"), ("z]", "』"),
];

/// かな → ローマ字で、表の中のいちばん短い綴りより、よく打つ綴りを使うもの (key=かな, kana=綴り)
static REVERSE_PREFER: &[(&str, &str)] = &[
    ("し", "shi"), ("ち", "chi"), ("つ", "tsu"), ("ふ", "fu"), ("じ", "ji"),
    ("しゃ", "sha"), ("しゅ", "shu"), ("しょ", "sho"), ("ちゃ", "cha"), ("ちゅ", "chu"), ("ちょ", "cho"),
    ("じゃ", "ja"), ("じゅ", "ju"), ("じょ", "jo"), ("ん", "nn"),
];

/// `Romaji::process` の結果
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Result {
    /// かなに決まった分
    pub committed: U16,
    /// まだ決まらないローマ字
    pub pending: U16,
}

/// ローマ字 → かな変換器 (元の Romaji クラス相当)
#[derive(Debug, Clone)]
pub struct Romaji {
    table: HashMap<U16, U16>,
    prefixes: HashSet<U16>,
    /// 足した綴りの、自分より長い綴りの頭になっているもの (拡張ローマ字で続きを待つ)
    longer: HashSet<U16>,
    /// かな → いちばん短い綴り
    reverse: HashMap<U16, U16>,
    max_kana: usize,
    buffer: U16,
    extended: bool,
    /// clear_table の後
    bare: bool,
}

impl Default for Romaji {
    fn default() -> Self {
        Self::new()
    }
}

impl Romaji {
    pub fn new() -> Self {
        let mut r = Romaji {
            table: HashMap::new(),
            prefixes: HashSet::new(),
            longer: HashSet::new(),
            reverse: HashMap::new(),
            max_kana: 1,
            buffer: Vec::new(),
            extended: false,
            bare: false,
        };
        for &(k, v) in ROMAJI_TABLE.iter().chain(PC_EXTRA.iter()) {
            r.add_initial(&u16s(k), &u16s(v));
        }
        for &(kana, spelling) in REVERSE_PREFER {
            r.reverse.insert(u16s(kana), u16s(spelling));
        }
        r.prefixes.insert(u16s("n"));
        r.prefixes.insert(u16s("n'"));
        r
    }

    /// コンストラクタ専用の登録 (元の Romaji() 内の add ラムダ)。
    /// 逆引き (いちばん短い綴り) と max_kana も更新する。longer は更新しない。
    fn add_initial(&mut self, k: &[u16], v: &[u16]) {
        self.table.insert(k.to_vec(), v.to_vec());
        for i in 1..=k.len() {
            self.prefixes.insert(k[..i].to_vec());
        }
        // 記号の綴りは逆引きに使わない
        if k.iter().any(|&c| !(b'a' as u16..=b'z' as u16).contains(&c)) {
            return;
        }
        let replace = match self.reverse.get(v) {
            None => true,
            Some(cur) => k.len() < cur.len(),
        };
        if replace {
            self.reverse.insert(v.to_vec(), k.to_vec());
        }
        if v.len() > self.max_kana {
            self.max_kana = v.len();
        }
    }

    /// 表に綴りを足す・上書きする (詳細設定のローマ字の表)
    pub fn add_pair(&mut self, key0: &[u16], kana: &[u16]) {
        let mut key: U16 = key0
            .iter()
            .map(|&c| {
                if (b'A' as u16..=b'Z' as u16).contains(&c) {
                    c + 32
                } else {
                    c
                }
            })
            .collect();
        if key.is_empty() || kana.is_empty() {
            return;
        }
        key.shrink_to_fit();
        self.table.insert(key.clone(), kana.to_vec());
        for i in 1..=key.len() {
            self.prefixes.insert(key[..i].to_vec());
        }
        for i in 1..key.len() {
            self.longer.insert(key[..i].to_vec());
        }
        if kana.len() > self.max_kana {
            self.max_kana = kana.len();
        }
    }

    /// 拡張ローマ字 (AZIK など、子音 + 子音の綴りを足した表)。オンのとき、子音を 2 つ重ねた「っ」(kk) と、n の後の子音で
    /// 「ん」にする決まり (nk → んk) より、表の綴り (kk = きん・nk = にん) とその続きを先に見る
    pub fn set_extended(&mut self, on: bool) {
        self.extended = on;
    }

    pub fn extended(&self) -> bool {
        self.extended
    }

    /// 元の表を空にする (gACT10 のように、ふつうのローマ字と全く違う綴りの表だけで打つとき)。
    /// 子音を重ねた「っ」・n の決まりも使わない。かな → ローマ字 (F9・F10) は元の表のまま
    pub fn clear_table(&mut self) {
        self.table.clear();
        self.prefixes.clear();
        self.longer.clear();
        self.max_kana = 1;
        self.bare = true;
    }

    /// 溜めているローマ字 (まだかなに決まっていない分)
    pub fn pending(&self) -> &[u16] {
        &self.buffer
    }

    /// key が溜めている分に続けてローマ字として意味を持つか (z の後の記号など)
    pub fn accepts(&self, key: u16) -> bool {
        let key = lower(key);
        let mut cand = self.buffer.clone();
        cand.push(key);
        self.prefixes.contains(&cand)
    }

    /// s (小文字のローマ字) が表のどれかの綴りの頭になっているか
    pub fn is_prefix(&self, s: &[u16]) -> bool {
        self.prefixes.contains(s)
    }

    /// 溜めている分の前に字を戻す (打ち間違いを消して子音だけが残ったとき、それを続きのローマ字として使う)
    pub fn unread(&mut self, s: &[u16]) {
        let mut b = s.to_vec();
        b.extend_from_slice(&self.buffer);
        self.buffer = b;
    }

    pub fn backspace(&mut self) {
        self.buffer.pop();
    }

    pub fn reset(&mut self) {
        self.buffer.clear();
    }

    /// 1 キー (小文字の a-z・-・' など) を足す
    pub fn process(&mut self, key: u16) -> Result {
        let key = lower(key);
        self.buffer.push(key);
        if self.buffer.len() >= 2 {
            let c0 = self.buffer[0];
            let consonants = u16s(CONSONANTS);
            if !self.bare
                && c0 == self.buffer[1]
                && c0 != b'n' as u16
                && has(&consonants, c0)
                && !(self.extended && self.prefixes.contains(&self.buffer))
            {
                self.buffer.drain(..1);
                let sub = self.process_buffered();
                let mut committed = u16s("っ");
                committed.extend_from_slice(&sub.committed);
                return Result {
                    committed,
                    pending: sub.pending,
                };
            }
        }
        self.process_buffered()
    }

    fn process_buffered(&mut self) -> Result {
        if self.buffer.is_empty() {
            return Result::default();
        }
        if self.buffer == u16s("n") && !self.bare {
            return Result {
                committed: Vec::new(),
                pending: u16s("n"),
            };
        }
        // n' → ん (ACT・DvorakJP のように表に n' があるときは表のとおり: n' = ない)
        if !self.bare
            && self.buffer.len() >= 2
            && self.buffer[0] == b'n' as u16
            && self.buffer[1] == b'\'' as u16
            && !(self.extended && self.table.contains_key(&u16s("n'")))
        {
            self.buffer.drain(..2);
            let sub = self.process_buffered();
            let mut committed = u16s("ん");
            committed.extend_from_slice(&sub.committed);
            return Result {
                committed,
                pending: sub.pending,
            };
        }
        if !self.bare
            && self.buffer.len() >= 2
            && self.buffer[0] == b'n' as u16
            && !has(&u16s("aiueoyn"), self.buffer[1])
            && !(self.extended && self.prefixes.contains(&self.buffer))
        {
            self.buffer.drain(..1);
            let sub = self.process_buffered();
            let mut committed = u16s("ん");
            committed.extend_from_slice(&sub.committed);
            return Result {
                committed,
                pending: sub.pending,
            };
        }
        let hit = self.table.get(&self.buffer).cloned();
        // 拡張ローマ字で、それだけで決まる綴りが長い綴りの頭も兼ねるとき (gACT10 の r = っ と rg = ぃ) は続きを待つ。
        // 続かなかったら下の「頭のいちばん長い綴り」で出る
        if let Some(kana) = hit {
            if !(self.extended && self.longer.contains(&self.buffer)) {
                self.buffer.clear();
                return Result {
                    committed: kana,
                    pending: Vec::new(),
                };
            }
        }
        if self.prefixes.contains(&self.buffer) {
            return Result {
                committed: Vec::new(),
                pending: self.buffer.clone(),
            };
        }
        for i in (1..self.buffer.len()).rev() {
            let hit = self.table.get(&self.buffer[..i]).cloned();
            if let Some(kana) = hit {
                self.buffer.drain(..i);
                let sub = self.process_buffered();
                let mut committed = kana;
                committed.extend_from_slice(&sub.committed);
                return Result {
                    committed,
                    pending: sub.pending,
                };
            }
        }
        let emitted = self.buffer[..1].to_vec();
        self.buffer.drain(..1);
        if !self.buffer.is_empty() {
            let sub = self.process_buffered();
            let mut committed = emitted;
            committed.extend_from_slice(&sub.committed);
            return Result {
                committed,
                pending: sub.pending,
            };
        }
        Result {
            committed: emitted,
            pending: Vec::new(),
        }
    }

    /// 溜めている分を出して空にする (n は ん、それ以外は打ったまま)
    pub fn flush(&mut self) -> U16 {
        let p = std::mem::take(&mut self.buffer);
        if p == u16s("n") && !self.bare {
            return u16s("ん");
        }
        if self.extended && !p.is_empty() {
            // 続きを待っていた綴り (r = っ など) と、その後に残った分
            if let Some(kana) = self.table.get(&p).cloned() {
                return kana;
            }
            for i in (1..p.len()).rev() {
                let hit = self.table.get(&p[..i]).cloned();
                if let Some(kana) = hit {
                    self.buffer = p[i..].to_vec();
                    let mut out = kana;
                    out.extend_from_slice(&self.flush());
                    return out;
                }
            }
        }
        p
    }

    /// かな → ローマ字 (F9・F10 で英字に戻すとき、打ったキーが分からない場合に使う)
    pub fn to_romaji(&self, kana0: &[u16]) -> U16 {
        let kana = to_hiragana(kana0);
        let consonants = u16s(CONSONANTS);
        let mut out: U16 = Vec::new();
        let mut sokuon = false;
        let mut i = 0;
        while i < kana.len() {
            let c = kana[i];
            if c == u16s("っ")[0] && i + 1 < kana.len() && is_hira(kana[i + 1]) {
                sokuon = true;
                i += 1;
                continue;
            }
            if c == u16s("ー")[0] {
                out.extend_from_slice(&u16s("-"));
                i += 1;
                continue;
            }
            let mut piece: U16 = Vec::new();
            let mut len = 0;
            for n in (1..=self.max_kana.min(kana.len() - i)).rev() {
                if let Some(spelling) = self.reverse.get(&kana[i..i + n]) {
                    piece = spelling.clone();
                    len = n;
                    break;
                }
            }
            if len == 0 {
                piece = kana[i..i + 1].to_vec();
                len = 1;
            }
            if c == u16s("ん")[0]
                && (i + 1 >= kana.len()
                    || !has(&u16s("あいうえおやゆよなにぬねの"), kana[i + 1]))
            {
                piece = u16s("n");
            }
            if sokuon && !piece.is_empty() && has(&consonants, piece[0]) {
                out.push(piece[0]);
            } else if sokuon {
                out.extend_from_slice(&u16s("xtu"));
            }
            sokuon = false;
            out.extend_from_slice(&piece);
            i += len;
        }
        out
    }
}

fn lower(c: u16) -> u16 {
    if (b'A' as u16..=b'Z' as u16).contains(&c) {
        c + 32
    } else {
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::from_u16s;

    fn run(r: &mut Romaji, input: &str) -> String {
        let mut out: U16 = Vec::new();
        for c in input.encode_utf16() {
            out.extend_from_slice(&r.process(c).committed);
        }
        out.extend_from_slice(&r.flush());
        from_u16s(&out)
    }

    fn run_fresh(input: &str) -> String {
        run(&mut Romaji::new(), input)
    }

    #[test]
    fn basic_words() {
        // desktop/tests/core_test.cpp のゴールデン
        assert_eq!(run_fresh("konnnichiha"), "こんにちは");
        assert_eq!(run_fresh("kanji"), "かんじ");
        assert_eq!(run_fresh("cocoa"), "ここあ");
        assert_eq!(run_fresh("kyakkasyouchi"), "きゃっかしょうち");
    }

    #[test]
    fn sokuon_doubled_consonant() {
        assert_eq!(run_fresh("kitte"), "きって");
        assert_eq!(run_fresh("kkanzh"), "っかん←");
        assert_eq!(run_fresh("gakkou"), "がっこう");
        // 子音の重ねでないものは促音にならない
        assert_eq!(run_fresh("aa"), "ああ");
    }

    #[test]
    fn n_rules() {
        assert_eq!(run_fresh("kan'i"), "かんい"); // n' → ん
        assert_eq!(run_fresh("hon"), "ほん"); // 文末の n は flush で ん
        assert_eq!(run_fresh("kan"), "かん");
        assert_eq!(run_fresh("kani"), "かに"); // 母音が続けば な行
        assert_eq!(run_fresh("kanni"), "かんい"); // nn → ん が両方の n を消費し、残りは い (元の表駆動の挙動)
        assert_eq!(run_fresh("kannni"), "かんに"); // n を 1 つ足すと ん + に
        assert_eq!(run_fresh("kinyou"), "きにょう"); // ny は拗音
    }

    #[test]
    fn pending_and_flush() {
        let mut r = Romaji::new();
        let res = r.process(b'k' as u16);
        assert_eq!(from_u16s(&res.committed), "");
        assert_eq!(from_u16s(&res.pending), "k");
        assert_eq!(from_u16s(r.pending()), "k");
        // 未確定の子音は flush で打ったまま出る
        assert_eq!(from_u16s(&r.flush()), "k");
        assert_eq!(from_u16s(r.pending()), "");
        // n だけは flush で ん になる
        let mut r = Romaji::new();
        r.process(b'n' as u16);
        assert_eq!(from_u16s(&r.flush()), "ん");
        // 途中まで打った綴りはそのまま
        let mut r = Romaji::new();
        r.process(b's' as u16);
        r.process(b'h' as u16);
        assert_eq!(from_u16s(&r.flush()), "sh");
    }

    #[test]
    fn pc_extra_and_symbols() {
        assert_eq!(run_fresh("z/"), "・");
        assert_eq!(run_fresh("zh"), "←");
        assert_eq!(run_fresh("z["), "『");
        assert_eq!(run_fresh("ca"), "か");
        assert_eq!(run_fresh("tsa"), "つぁ");
        assert_eq!(run_fresh("wha"), "うぁ");
        assert_eq!(run_fresh("-"), "ー");
    }

    #[test]
    fn backspace_reset_unread() {
        let mut r = Romaji::new();
        r.process(b'k' as u16);
        r.process(b'y' as u16);
        r.backspace();
        assert_eq!(from_u16s(r.pending()), "k");
        r.backspace();
        r.backspace(); // 空でも何もしない
        assert_eq!(from_u16s(r.pending()), "");
        r.process(b's' as u16);
        r.reset();
        assert_eq!(from_u16s(r.pending()), "");
        // unread: 戻した字が続きと合わせて決まる
        let mut r = Romaji::new();
        r.process(b'a' as u16);
        r.unread(&u16s("k"));
        let res = r.process(b'i' as u16);
        assert_eq!(from_u16s(&res.committed), "き");
    }

    #[test]
    fn accepts_and_is_prefix() {
        let r = Romaji::new();
        assert!(r.is_prefix(&u16s("k")));
        assert!(r.is_prefix(&u16s("ky")));
        assert!(r.is_prefix(&u16s("n'")));
        assert!(!r.is_prefix(&u16s("kk")));
        assert!(!r.is_prefix(&u16s("qz")));
        let mut r = Romaji::new();
        assert!(r.accepts(b'z' as u16));
        r.process(b'z' as u16);
        assert!(r.accepts(b'/' as u16)); // z の後の記号
        assert!(r.accepts(b'H' as u16)); // 大文字は小文字化して見る
        assert!(r.accepts(b'a' as u16)); // za = ざ があるので続く
        assert!(!r.accepts(b'q' as u16));
    }

    #[test]
    fn uppercase_input() {
        assert_eq!(run_fresh("KA"), "か");
        assert_eq!(run_fresh("KanJi"), "かんじ");
    }

    #[test]
    fn unknown_sequence_emitted_as_is() {
        // 表に無い並びは打ったまま出る
        assert_eq!(run_fresh("qz"), "qz");
        // 頭の綴りだけ決めて残りを処理する (k + 決まらない続き)
        assert_eq!(run_fresh("kakz"), "かkz");
    }

    #[test]
    fn to_romaji_cases() {
        let r = Romaji::new();
        // desktop/tests/core_test.cpp のゴールデン
        assert_eq!(from_u16s(&r.to_romaji(&u16s("とうきょうでしゅっぱつ"))), "toukyoudeshuppatsu");
        assert_eq!(from_u16s(&r.to_romaji(&u16s("かんい"))), "kanni");
        // ん の後が母音・や行・な行でなければ n、続くときは nn
        assert_eq!(from_u16s(&r.to_romaji(&u16s("かんじ"))), "kanji");
        assert_eq!(from_u16s(&r.to_romaji(&u16s("かに"))), "kani");
        // 促音は子音の重ね、長音は -
        assert_eq!(from_u16s(&r.to_romaji(&u16s("がっこう"))), "gakkou");
        assert_eq!(from_u16s(&r.to_romaji(&u16s("らーめん"))), "ra-men");
        // カタカナ入力はひらがな化してから
        assert_eq!(from_u16s(&r.to_romaji(&u16s("トウキョウ"))), "toukyou");
        // よく打つ綴り (REVERSE_PREFER)
        assert_eq!(from_u16s(&r.to_romaji(&u16s("しちつふじ"))), "shichitsufuji");
    }

    #[test]
    fn add_pair_overrides() {
        let mut r = Romaji::new();
        r.add_pair(&u16s("ka"), &u16s("蚊"));
        assert_eq!(run(&mut r, "ka"), "蚊");
        // 大文字キーは小文字化される
        let mut r = Romaji::new();
        r.add_pair(&u16s("XA"), &u16s("ぁ"));
        assert_eq!(run(&mut r, "xa"), "ぁ");
        // 空の組は無視
        let mut r = Romaji::new();
        r.add_pair(&u16s(""), &u16s("あ"));
        r.add_pair(&u16s("zz"), &u16s(""));
        assert!(!r.is_prefix(&u16s("zz")));
    }

    #[test]
    fn extended_waits_for_longer() {
        // 拡張: r = っ だが rg = ぃ があるので r だけでは決めない (続きを待つ)
        let mut r = Romaji::new();
        r.set_extended(true);
        r.add_pair(&u16s("r"), &u16s("っ"));
        r.add_pair(&u16s("rg"), &u16s("ぃ"));
        let res = r.process(b'r' as u16);
        assert_eq!(from_u16s(&res.committed), "");
        assert_eq!(from_u16s(&res.pending), "r");
        // 続かなかったら flush で r の分が出る
        assert_eq!(from_u16s(&r.flush()), "っ");
        // 続いたら長い方
        let mut r = Romaji::new();
        r.set_extended(true);
        r.add_pair(&u16s("r"), &u16s("っ"));
        r.add_pair(&u16s("rg"), &u16s("ぃ"));
        r.process(b'r' as u16);
        let res = r.process(b'g' as u16);
        assert_eq!(from_u16s(&res.committed), "ぃ");
    }

    #[test]
    fn extended_table_beats_sokuon_and_n() {
        // 拡張では kk = きん (表) が子音重ねの っ より先、nk = にん が ん+k より先
        let mut r = Romaji::new();
        r.set_extended(true);
        r.add_pair(&u16s("kk"), &u16s("きん"));
        r.add_pair(&u16s("nk"), &u16s("にん"));
        assert_eq!(run(&mut r, "kk"), "きん");
        let mut r = Romaji::new();
        r.set_extended(true);
        r.add_pair(&u16s("kk"), &u16s("きん"));
        r.add_pair(&u16s("nk"), &u16s("にん"));
        assert_eq!(run(&mut r, "nk"), "にん");
        // 表に無い重ねは通常どおり っ になる
        let mut r = Romaji::new();
        r.set_extended(true);
        assert_eq!(run(&mut r, "kka"), "っか");
    }

    #[test]
    fn clear_table_bare() {
        // 元の表を空にすると、ふつうのローマ字・っ・ん の決まりが効かない
        let mut r = Romaji::new();
        r.clear_table();
        r.add_pair(&u16s("ka"), &u16s("た")); // gACT10 の例 (core_test.cpp)
        assert_eq!(run(&mut r, "ka"), "た");
        let mut r = Romaji::new();
        r.clear_table();
        // n は ん にならず、process の時点で打ったまま確定する (flush には残らない)
        let res = r.process(b'n' as u16);
        assert_eq!(from_u16s(&res.committed), "n");
        assert_eq!(from_u16s(&r.flush()), "");
        // かな → ローマ字は元の表のまま
        assert_eq!(from_u16s(&r.to_romaji(&u16s("あ"))), "a");
    }

    #[test]
    fn extended_n_prime_in_table() {
        // 拡張で表に n' があるときは n' の決まりより表が先 (ACT: n' = ない)
        let mut r = Romaji::new();
        r.set_extended(true);
        r.clear_table();
        r.add_pair(&u16s("n'"), &u16s("ない"));
        r.add_pair(&u16s("nn"), &u16s("ん"));
        assert_eq!(run(&mut r, "n'nn"), "ないん");
    }
}
