//! composer: PC 版の入力の状態と操作 (Windows の TSF・Linux の Fcitx5 から同じものを使う)。
//! 元: desktop/core/composer.cpp (2306行) + composer.h (353行)。
//!
//! ハードウェアキーボードで打つ PC の IME の決まり (MS-IME・Google 日本語入力と同じ操作):
//! ローマ字で打つ → 空白 (変換キー) で AI 変換 → 文節ごとに直す → Enter で確定。
//! F6〜F10 でひらがな・カタカナ・半角カナ・全角英数・半角英数。無変換は カタカナ→半角カナ→ひらがな。
//!
//! 依存先の吸収 (他担当が並行実装中のため、このファイル内で完結させる):
//! - Converter は本ファイルの `Convert` trait 越しに使う (crate::converter::Converter に impl 済み)。
//! テストではフェイクを差し込める。
//! - Romaji (romaji.rs は未実装スタブ) は本ファイル内の `SimpleRomaji` で代替する。標準的な綴り
//! (訓令・ヘボン混在、促音・撥音・拗音・小書き) を生成表で持つが、元の tables.h の全綴り
//! (z 記号 zh→← 等) は未収録 (未移植点として報告)。
//! - text.h の変換は本ファイル内の小関数で代替 (text.rs は未実装スタブ)。
//! - SentenceFlow (flow.rs は未実装スタブ) は `Flow` trait で差し替え可能にした。
//! - datetime_predict (datetime.rs) は `datetime_hook` (関数) で差し替え可能にした (既定は無し)。
//! - typo_alts.h の kana_modify は本ファイル内に移植。Layout の打ち直し (prime_alts) は
//! layout_typo フックが無い場合の QWERTY 経路が Layout 依存のため未移植 (報告に明記)。
//! - StyleLog / Learning / UserDict は crate::store の実装を使う。

use crate::converter::{group_phrases, Conversion, Converter, Phrase, TreeNode, TreeWord, Word};
use crate::store::{DictForm, Learning, StyleCount, StyleLog, Undo, UserDict};
use crate::{from_u16s, u16s, U16};
use std::collections::HashMap;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------- text.h 相当の小関数
pub fn to_katakana(s: &[u16]) -> U16 {
s.iter().map(|&c| if (0x3041..=0x3096).contains(&c) { c + 0x60} else { c}).collect()
}
pub fn to_hiragana(s: &[u16]) -> U16 {
s.iter().map(|&c| if (0x30A1..=0x30F6).contains(&c) { c - 0x60} else { c}).collect()
}
pub fn to_fullwidth_ascii(s: &[u16]) -> U16 {
s.iter().map(|&c| if c == b' ' as u16 { 0x3000} else if c > 0x20 && c < 0x7F { c + 0xFEE0} else { c}).collect()
}
pub fn to_halfwidth_ascii(s: &[u16]) -> U16 {
s.iter().map(|&c| {
if c == 0x3000 { b' ' as u16} else if c > 0xFF00 && c < 0xFF5F { c - 0xFEE0}
else if c == 0xFFE5 { b'\\' as u16} else { c}
}).collect()
}
pub fn to_halfwidth_kana(s: &[u16]) -> U16 {
const FULL: &str = "ァアィイゥウェエォオカキクケコサシスセソタチッツテトナニヌネノハヒフヘホマミムメモャヤュユョヨラリルレロワヲンーヮヰヱヵヶ。、「」・";
const HALF: &str = "ｧｱｨｲｩｳｪｴｫｵｶｷｸｹｺｻｼｽｾｿﾀﾁｯﾂﾃﾄﾅﾆﾇﾈﾉﾊﾋﾌﾍﾎﾏﾐﾑﾒﾓｬﾔｭﾕｮﾖﾗﾘﾙﾚﾛﾜｦﾝｰﾜｲｴｶｹ｡､｢｣･";
const VOICED: &str = "ガギグゲゴザジズゼゾダヂヅデドバビブベボ";
const SEMI: &str = "パピプペポ";
let full: Vec<u16> = u16s(FULL);
let half: Vec<u16> = u16s(HALF);
let voiced: Vec<u16> = u16s(VOICED);
let semi: Vec<u16> = u16s(SEMI);
let idx = |v: &[u16], c: u16| v.iter().position(|&x| x == c).map(|p| p as i32).unwrap_or(-1);
let mut o = Vec::new();
for c in to_katakana(s) {
let i = idx(&full, c);
if i >= 0 { o.push(half[i as usize]); continue;}
if idx(&voiced, c) >= 0 { let j = idx(&full, c - 1); if j >= 0 { o.push(half[j as usize]); o.push(0xFF9E); continue;}}
if idx(&semi, c) >= 0 { let j = idx(&full, c - 2); if j >= 0 { o.push(half[j as usize]); o.push(0xFF9F); continue;}}
if c == 0x30F4 { o.extend(u16s("ｳﾞ")); continue;}
if c == 0x30F7 { o.extend(u16s("ﾜﾞ")); continue;}
o.extend(to_halfwidth_ascii(&[c]));
}
o
}
pub fn japanese_symbol(c: u16) -> u16 {
match c as u8 as char {
',' => '、' as u16,
'.' => '。' as u16,
'[' => '「' as u16,
']' => '」' as u16,
'/' => '・' as u16,
'-' => 'ー' as u16,
'\\' => 0xFFE5,
_ => if c > 0x20 && c < 0x7F { c + 0xFEE0} else { c},
}
}
fn is_hira(c: u16) -> bool { (0x3041..=0x3096).contains(&c)}
fn is_digit(c: u16) -> bool { (0x30..=0x39).contains(&c)}
fn is_upper(c: u16) -> bool { (0x41..=0x5A).contains(&c)}
fn is_lower(c: u16) -> bool { (0x61..=0x7A).contains(&c)}
fn tail(s: &[u16], n: usize) -> U16 { if s.len() > n { s[s.len() - n..].to_vec()} else { s.to_vec()}}
fn contains(v: &[U16], s: &[u16]) -> bool { v.iter().any(|x| x == s)}
fn push_unique(v: &mut Vec<U16>, s: &[u16]) { if!s.is_empty() &&!contains(v, s) { v.push(s.to_vec());}}
fn lower(s: &[u16]) -> U16 { s.iter().map(|&c| if is_upper(c) { c + 32} else { c}).collect()}
fn upper(s: &[u16]) -> U16 { s.iter().map(|&c| if is_lower(c) { c - 32} else { c}).collect()}
fn capital(s: &[u16]) -> U16 {
let mut s = lower(s);
if!s.is_empty() && is_lower(s[0]) { s[0] -= 32;}
s
}
fn replace_all(mut s: U16, from: &[u16], to: &[u16]) -> U16 {
if from.is_empty() { return s;}
let mut i = 0;
while i + from.len() <= s.len() {
if s[i..i + from.len()] == *from {
let mut ns: U16 = s[..i].to_vec();
ns.extend(to);
ns.extend(&s[i + from.len()..]);
s = ns;
i += to.len();
} else { i += 1;}
}
s
}
fn cat(a: &[u16], b: &[u16]) -> U16 { let mut v = a.to_vec(); v.extend(b); v}

/// typo_alts.h の kana_modify: 濁点・半濁点・拗音化などを前のかなに付ける。
pub fn kana_modify(prev: &mut u16, c: u16) -> bool {
// 濁点 (゛ = 0x309B / U+E001 系は呼び出し側で正規化済みの想定。ここでは ゛゜ と 309B/309C を扱う)
let voiced = u16s("かきくけこさしすせそたちつてとはひふへほ");
let semi = u16s("はひふへほ");
if c == 0x309B || c == '゛' as u16 {
if voiced.contains(prev) { *prev += 1; return true;}
if *prev == 'う' as u16 { *prev = 'ゔ' as u16; return true;}
return false;
}
if c == 0x309C || c == '゜' as u16 {
if semi.contains(prev) { *prev += 2; return true;}
return false;
}
false
}

// ---------------------------------------------------------------- 挨拶・括弧 (composer.cpp の自由関数)
/// 読み reading とその表記 surf で、挨拶の「今日は」をかな書きに直す。
pub fn greeting_fix(reading: &[u16], surf: &[u16]) -> U16 {
let r = from_u16s(reading);
if r.contains("こんにちは") &&!r.contains("きょうは") {
return replace_all(surf.to_vec(), &u16s("今日は"), &u16s("こんにちは"));
}
surf.to_vec()
}
fn prefer_greetings(target: &[u16], cands: &mut Vec<U16>) {
if cands.is_empty() { return;}
let top = cands[0].clone();
let mut trimmed = target.to_vec();
while matches!(trimmed.last(), Some(&c) if u16s("、。！？!?ー〜").contains(&c)) { trimmed.pop();}
let t = from_u16s(target);
let rules: [(&str, &str, bool); 2] = [
("今日は", "こんにちは", t.contains("こんにちは") &&!t.contains("きょうは")),
("今晩は", "こんばんは", from_u16s(&trimmed) == "こんばんは"),
];
for (kanji, kana, ok) in rules {
if!ok { continue;}
if!from_u16s(&top).contains(kanji) { continue;}
let fixed = replace_all(top.clone(), &u16s(kanji), &u16s(kana));
cands.retain(|c| *c!= fixed);
cands.insert(0, fixed);
return;
}
}
const BRACKETS: &[(&str, &str)] = &[
("（", "）"), ("(", ")"), ("「", "」"), ("『", "』"), ("", ""), ("［", "］"), ("[", "]"),
("｛", "｝"), ("{", "}"), ("〔", "〕"), ("〈", "〉"), ("《", "》"), ("〖", "〗"), ("〘", "〙"),
("〚", "〛"), ("｢", "｣"), ("＜", "＞"), ("<", ">"), ("«", "»"), ("‹", "›"), ("“", "”"),
("‘", "’"), ("〝", "〟"), ("｟", "｠"),
];
pub fn bracket_variants(reading: &[u16]) -> Vec<U16> {
let opens: Vec<U16> = BRACKETS.iter().map(|b| u16s(b.0)).collect();
let closes: Vec<U16> = BRACKETS.iter().map(|b| u16s(b.1)).collect();
let mut out: Vec<U16> = Vec::new();
if reading == u16s("かっこ") {
for b in BRACKETS { out.push(cat(&u16s(b.0), &u16s(b.1)));}
return out;
}
if reading == u16s("かっこひらき") { return opens;}
if reading == u16s("かっことじ") { return closes;}
const MARKS: &[&[&str]] = &[
&["・", "／", "/"], &["、", "，", ","], &["。", "．", "."], &["ー", "－", "-", "―", "‐"], &["～", "〜", "~"],
&["￥", "＼", "\\"], &["＊", "*", "※", "×"], &["＂", "\"", "“", "”"], &["＇", "'", "‘", "’"],
];
if reading.len() == 1 &&!contains(&opens, reading) &&!contains(&closes, reading) {
let mut marks: Vec<U16> = Vec::new();
for g in MARKS {
let m: Vec<U16> = g.iter().map(|x| u16s(x)).collect();
if contains(&m, reading) { marks = m; break;}
}
let ch = reading[0];
let half = if (0xFF01..=0xFF5E).contains(&ch) { ch - 0xFEE0} else { ch};
if marks.is_empty() && half > 0x20 && half < 0x7F &&!is_digit(half)
&&!((half | 0x20) >= b'a' as u16 && (half | 0x20) <= b'z' as u16) {
marks = vec![vec![half + 0xFEE0], vec![half]];
}
if!marks.is_empty() {
out.push(reading.to_vec());
for m in marks { if m!= reading { out.push(m);}}
return out;
}
}
for side in [&opens, &closes] {
if!contains(side, reading) { continue;}
out.push(reading.to_vec());
for c in side.iter() { if *c!= reading { out.push(c.clone());}}
break;
}
out
}
fn add_brackets(reading: &[u16], cands: &mut Vec<U16>) {
let br = bracket_variants(reading);
if br.is_empty() { return;}
if br[0] == reading {
let mut nb = br;
for c in cands.iter() { if!contains(&nb, c) { nb.push(c.clone());}}
*cands = nb;
return;
}
let mut at = 1.min(cands.len());
for b in br {
if let Some(p) = cands.iter().position(|c| *c == b) {
if p < at { continue;}
cands.remove(p);
}
cands.insert(at, b);
at += 1;
}
}

// ---------------------------------------------------------------- Romaji (代替実装)
#[derive(Clone, Debug)]
pub struct RomajiResult { pub committed: U16, pub pending: U16}

#[derive(Clone)]
pub struct SimpleRomaji {
table: HashMap<String, String>,
prefixes: std::collections::HashSet<String>,
reverse: HashMap<String, String>,
buffer: String,
extended: bool,
bare: bool,
}
impl Default for SimpleRomaji { fn default() -> Self { Self::new()}}
impl SimpleRomaji {
pub fn new() -> Self {
let mut table: HashMap<String, String> = HashMap::new();
let mut add = |k: &str, v: &str| { table.insert(k.to_string(), v.to_string());};
// 母音・基本行
let rows: &[(&str, [&str; 5])] = &[
("", ["あ", "い", "う", "え", "お"]),
("k", ["か", "き", "く", "け", "こ"]), ("g", ["が", "ぎ", "ぐ", "げ", "ご"]),
("s", ["さ", "し", "す", "せ", "そ"]), ("z", ["ざ", "じ", "ず", "ぜ", "ぞ"]),
("t", ["た", "ち", "つ", "て", "と"]), ("d", ["だ", "ぢ", "づ", "で", "ど"]),
("n", ["な", "に", "ぬ", "ね", "の"]), ("h", ["は", "ひ", "ふ", "へ", "ほ"]),
("b", ["ば", "び", "ぶ", "べ", "ぼ"]), ("p", ["ぱ", "ぴ", "ぷ", "ぺ", "ぽ"]),
("m", ["ま", "み", "む", "め", "も"]), ("r", ["ら", "り", "る", "れ", "ろ"]),
("w", ["わ", "ゐ", "う", "ゑ", "を"]),
];
let vs = ["a", "i", "u", "e", "o"];
for (cons, kanas) in rows {
for (i, v) in vs.iter().enumerate() { add(&format!("{}{}", cons, v), kanas[i]);}
}
for (k, v) in [
("si", "し"), ("ti", "ち"), ("tu", "つ"), ("hu", "ふ"), ("fu", "ふ"), ("zi", "じ"), ("ji", "じ"),
("di", "ぢ"), ("du", "づ"), ("ya", "や"), ("yu", "ゆ"), ("yo", "よ"), ("wa", "わ"), ("wo", "を"),
("nn", "ん"), ("n'", "ん"), ("-", "ー"),
("xa", "ぁ"), ("xi", "ぃ"), ("xu", "ぅ"), ("xe", "ぇ"), ("xo", "ぉ"),
("la", "ぁ"), ("li", "ぃ"), ("lu", "ぅ"), ("le", "ぇ"), ("lo", "ぉ"),
("xya", "ゃ"), ("xyu", "ゅ"), ("xyo", "ょ"), ("lya", "ゃ"), ("lyu", "ゅ"), ("lyo", "ょ"),
("xtu", "っ"), ("ltu", "っ"), ("ltsu", "っ"), ("xwa", "ゎ"), ("lwa", "ゎ"),
] { add(k, v);}
// 拗音
let yoon: &[(&str, &str)] = &[
("ky", "き"), ("gy", "ぎ"), ("sy", "し"), ("sh", "し"), ("zy", "じ"), ("j", "じ"), ("jy", "じ"),
("ty", "ち"), ("ch", "ち"), ("cy", "ち"), ("dy", "ぢ"), ("ny", "に"), ("hy", "ひ"),
("by", "び"), ("py", "ぴ"), ("my", "み"), ("ry", "り"),
];
for (base, kana) in yoon {
for (v, small) in [("a", "ゃ"), ("u", "ゅ"), ("o", "ょ"), ("e", "ぇ")] {
add(&format!("{}{}", base, v), &format!("{}{}", kana, small));
}
}
for (k, v) in [
("sha", "しゃ"), ("shu", "しゅ"), ("sho", "しょ"), ("she", "しぇ"),
("cha", "ちゃ"), ("chu", "ちゅ"), ("cho", "ちょ"), ("che", "ちぇ"),
("tsa", "つぁ"), ("tsi", "つぃ"), ("tse", "つぇ"), ("tso", "つぉ"),
("fa", "ふぁ"), ("fi", "ふぃ"), ("fe", "ふぇ"), ("fo", "ふぉ"),
("va", "ゔぁ"), ("vi", "ゔぃ"), ("vu", "ゔ"), ("ve", "ゔぇ"), ("vo", "ゔぉ"),
("qa", "くぁ"), ("qi", "くぃ"), ("qu", "く"), ("qe", "くぇ"), ("qo", "くぉ"),
("kwa", "くぁ"), ("gwa", "ぐぁ"), ("wha", "うぁ"), ("whi", "うぃ"), ("whe", "うぇ"), ("who", "うぉ"),
("ca", "か"), ("cu", "く"), ("co", "こ"), ("tha", "てゃ"), ("thu", "てゅ"), ("tho", "てょ"),
] { add(k, v);}
let mut prefixes = std::collections::HashSet::new();
for k in table.keys() { for i in 1..k.len() { prefixes.insert(k[..i].to_string());}}
let mut reverse: HashMap<String, String> = HashMap::new();
for (k, v) in &table {
match reverse.get(v) { Some(old) if old.len() <= k.len() => {}, _ => { reverse.insert(v.clone(), k.clone());}}
}
// 同じ長さの別綴り (か=ka/ca, く=ku/cu, こ=ko/co 等) は HashMap の走査順で決まってしまうため、
// romaji.rs の REVERSE_PREFER と同じく標準綴りを明示的に優先する (逆引きが "neco" になる不具合の修正)
for (kana, spelling) in [
("か", "ka"), ("く", "ku"), ("こ", "ko"),
("し", "shi"), ("ち", "chi"), ("つ", "tsu"), ("ふ", "fu"), ("じ", "ji"),
("しゃ", "sha"), ("しゅ", "shu"), ("しょ", "sho"), ("ちゃ", "cha"), ("ちゅ", "chu"), ("ちょ", "cho"),
("じゃ", "ja"), ("じゅ", "ju"), ("じょ", "jo"),
] { reverse.insert(kana.to_string(), spelling.to_string()); }
SimpleRomaji { table, prefixes, reverse, buffer: String::new(), extended: false, bare: false}
}
pub fn pending(&self) -> U16 { u16s(&self.buffer)}
pub fn reset(&mut self) { self.buffer.clear();}
pub fn backspace(&mut self) { self.buffer.pop();}
pub fn is_prefix(&self, s: &[u16]) -> bool { self.prefixes.contains(&from_u16s(s))}
pub fn accepts(&self, key: u16) -> bool {
let mut b = self.buffer.clone();
b.push(char::from_u32(key as u32).unwrap_or('\0'));
self.prefixes.contains(&b) || self.table.contains_key(&b)
}
pub fn unread(&mut self, s: &[u16]) { self.buffer = format!("{}{}", from_u16s(s), self.buffer);}
pub fn set_extended(&mut self, on: bool) { self.extended = on;}
pub fn extended(&self) -> bool { self.extended}
pub fn clear_table(&mut self) { self.table.clear(); self.prefixes.clear(); self.bare = true;}
pub fn add_pair(&mut self, key: &[u16], kana: &[u16]) {
let k = from_u16s(key);
for i in 1..k.len() { self.prefixes.insert(k[..i].to_string());}
self.table.insert(k, from_u16s(kana));
}
fn try_commit(&mut self) -> String {
// buffer の頭から確定できる分を返す
let mut out = String::new();
loop {
if self.buffer.is_empty() { break;}
let b = self.buffer.clone();
// 促音: 子音の重なり (n・母音を除く)
if b.len() >= 2 {
let c0 = b.as_bytes()[0] as char;
let c1 = b.as_bytes()[1] as char;
if c0 == c1 &&!"aiueon".contains(c0) &&!self.bare {
out.push('っ');
self.buffer = b[1..].to_string();
continue;
}
}
// 撥音: n + (母音・y 以外) / nn / n'
if b.starts_with('n') &&!self.bare {
if b.len() == 1 { break;} // まだ決まらない
let c1 = b.as_bytes()[1] as char;
if c1 == '\'' { out.push('ん'); self.buffer = b[2.min(b.len())..].to_string(); continue;}
if!"aiueoy".contains(c1) {
if!(c1 == 'n' && b.len() >= 3 && "aiueoy".contains(b.as_bytes()[2] as char)) {
out.push('ん');
self.buffer = b[1..].to_string();
continue;
}
}
}
// 表の最長一致。ただし buffer 全体がより長い綴りの頭なら待つ
if self.prefixes.contains(&b) &&!self.extended_done(&b) { break;}
let mut matched: Option<(usize, String)> = None;
for len in (1..=b.len().min(4)).rev() {
let head = &b[..len];
if let Some(v) = self.table.get(head) {
// 残りが空で、この綴り自体がより長い綴りの頭なら待つ (拡張でない通常時)
if len == b.len() && self.prefixes.contains(head) && self.has_longer(head) { break;}
matched = Some((len, v.clone()));
break;
}
}
match matched {
Some((len, v)) => { out.push_str(&v); self.buffer = b[len..].to_string();}
None => break,
}
}
out
}
fn extended_done(&self, _b: &str) -> bool { false}
fn has_longer(&self, head: &str) -> bool {
self.table.keys().any(|k| k.len() > head.len() && k.starts_with(head))
}
pub fn process(&mut self, key: u16) -> RomajiResult {
self.buffer.push(char::from_u32(key as u32).unwrap_or('\0'));
let committed = self.try_commit();
RomajiResult { committed: u16s(&committed), pending: self.pending()}
}
pub fn flush(&mut self) -> U16 {
let mut out = String::new();
loop {
if self.buffer.is_empty() { break;}
let b = self.buffer.clone();
if b == "n" { out.push('ん'); self.buffer.clear(); break;}
let c = self.try_commit();
if!c.is_empty() { out.push_str(&c); continue;}
if b.starts_with('n') && b.len() >= 2 {
let c1 = b.as_bytes()[1] as char;
if!"aiueoy".contains(c1) { out.push('ん'); self.buffer = b[1..].to_string(); continue;}
}
// 決まらない分は打ったまま 1 字出す
let ch = b.chars().next().unwrap();
out.push(ch);
self.buffer = b[ch.len_utf8()..].to_string();
}
u16s(&out)
}
pub fn to_romaji(&self, kana: &[u16]) -> U16 {
let s = from_u16s(kana);
let mut out = String::new();
let chars: Vec<char> = s.chars().collect();
let mut i = 0;
while i < chars.len() {
// 拗音など 2 字
if i + 1 < chars.len() {
let two: String = chars[i..i + 2].iter().collect();
if let Some(r) = self.reverse.get(&two) { out.push_str(r); i += 2; continue;}
}
let one: String = chars[i..i + 1].iter().collect();
if one == "っ" { // 次の子音を重ねる形はここでは xtu で代表させる
out.push_str("xtu");
i += 1;
continue;
}
if one == "ん" { out.push('n'); i += 1; continue;}
match self.reverse.get(&one) { Some(r) => out.push_str(r), None => out.push_str(&one)}
i += 1;
}
u16s(&out)
}
}

// ---------------------------------------------------------------- 型 (composer.h)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
Char, Space, Enter, Backspace, Delete, Escape, Left, Right, Up, Down, Home, End, Tab, PageUp, PageDown,
F6, F7, F8, F9, F10, Henkan, Muhenkan,
}
#[derive(Clone, Debug, Default)]
pub struct KeyEvent {
pub key: Key,
pub ch: u16,
pub shift: bool,
pub ctrl: bool,
pub alt: bool,
pub raw: bool,
}
impl Default for Key {
fn default() -> Self { Key::Char}
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpanKind { Input, Converted, Focused, Corrected}
#[derive(Clone, Debug, PartialEq)]
pub struct Span { pub start: i32, pub len: i32, pub kind: SpanKind}

#[derive(Clone, Debug, Default)]
pub struct View {
pub text: U16,
pub spans: Vec<Span>,
pub typed: U16,
pub typed_fixed: bool,
pub typed_mark: Vec<bool>,
pub caret: i32,
pub anchor: i32,
pub cand_open: bool,
pub cands: Vec<U16>,
pub cand_notes: Vec<U16>,
pub cand_sel: i32,
pub sugg: Vec<U16>,
pub sugg_text: Vec<U16>,
pub sugg_kinds: Vec<i32>,
pub sugg_sel: i32,
pub post: Vec<U16>,
pub tree: Vec<TreeNode>,
pub tree_rs: i32,
pub tree_re: i32,
pub cand_short: i32,
pub cand_scores: Vec<f32>,
pub cand_fork: i32,
}
impl View {
fn new() -> Self { View { cand_sel: -1, sugg_sel: -1, tree_rs: -1, tree_re: -1, cand_fork: -1,..Default::default()}}
}

#[derive(Clone, Debug, Default)]
pub struct LogEvent {
pub kind: String,
pub reading: U16,
pub surface: U16,
pub ctx: U16,
pub cands: Vec<U16>,
pub index: i32,
pub segs: Vec<U16>,
pub extra: String,
}

#[derive(Clone, Debug)]
pub struct ComposerOptions {
pub live: bool,
pub live_commit: bool,
pub live_display: bool,
pub live_wait: bool,
pub always_cands: bool,
pub cand_tree: bool,
pub follow: bool,
pub space_fullwidth: bool,
pub space_literal: bool,
pub punct_commit: bool,
pub typo_keys: bool,
pub caps_alpha: bool,
pub punct: i32,
pub digits_fullwidth: bool,
pub half_symbols: U16,
pub numpad: i32,
pub private_mode: bool,
pub learn_log: bool,
pub secure_field: bool,
pub ctx_max: usize,
pub rctx_max: usize,
}
impl Default for ComposerOptions {
fn default() -> Self {
ComposerOptions {
live: true, live_commit: true, live_display: false, live_wait: false, always_cands: true,
cand_tree: false, follow: true, space_fullwidth: true, space_literal: false, punct_commit: false,
typo_keys: true, caps_alpha: false, punct: 0, digits_fullwidth: false, half_symbols: Vec::new(),
numpad: 0, private_mode: false, learn_log: false, secure_field: false, ctx_max: 40, rctx_max: 80,
}
}
}

/// 変換の差し替え口 (converter.h の Converter の公開 API 相当)。
pub trait Convert {
fn convert(&mut self, ctx: &[u16], kana: &[u16], max: i32) -> Conversion;
fn set_right_ctx(&mut self, text: &[u16]);
fn set_keys(&mut self, keys: &[u16], span_end: &[i32], reading_len: usize);
fn set_alt_readings(&mut self, alts: &[U16], reading_len: usize);
fn has_right_ctx(&self) -> bool;
fn set_suggest(&mut self, on: bool);
fn set_typo(&mut self, on: bool, bias: f32);
fn set_latin(&mut self, on: bool);
fn personal(&self) -> bool;
fn set_variants(&mut self, tsv_utf8: &str);
fn set_marks(&mut self, marks: &[(U16, i32)]);
fn set_style_counts(&mut self, counts: &HashMap<U16, StyleCount>);
fn set_restyle(&mut self, on: bool);
fn style_weight(&self, ctx: &[u16]) -> [f32; 2];
fn typo_spans(&self) -> Vec<(i32, i32)>;
fn path_variants(&self, reading: &[u16], text: &[u16]) -> Vec<U16>;
fn path_pos(&self, reading: &[u16], text: &[u16]) -> Vec<Word>;
fn latin_word(&self, key: &[u16]) -> U16;
fn complete(&mut self, prefix: &[u16], max: i32, max_extra: i32) -> Vec<U16>;
fn set_user_words(&mut self, forms: &[DictForm], bonus: i32) -> i32;
fn ai_learn(&mut self, ctx: &[u16], reading: &[u16], text: &[u16]) -> f32;
}
impl Convert for Converter {
fn convert(&mut self, ctx: &[u16], kana: &[u16], max: i32) -> Conversion { Converter::convert(self, ctx, kana, max)}
fn set_right_ctx(&mut self, text: &[u16]) { Converter::set_right_ctx(self, text)}
fn set_keys(&mut self, keys: &[u16], span_end: &[i32], reading_len: usize) { Converter::set_keys(self, keys, span_end, reading_len)}
fn set_alt_readings(&mut self, alts: &[U16], reading_len: usize) { Converter::set_alt_readings(self, alts, reading_len)}
fn has_right_ctx(&self) -> bool { Converter::has_right_ctx(self)}
fn set_suggest(&mut self, on: bool) { Converter::set_suggest(self, on)}
fn set_typo(&mut self, on: bool, bias: f32) { Converter::set_typo(self, on, bias)}
fn set_latin(&mut self, on: bool) { Converter::set_latin(self, on)}
fn personal(&self) -> bool { Converter::personal(self)}
fn set_variants(&mut self, tsv_utf8: &str) { Converter::set_variants(self, tsv_utf8)}
fn set_marks(&mut self, marks: &[(U16, i32)]) { Converter::set_marks(self, marks)}
fn set_style_counts(&mut self, counts: &HashMap<U16, StyleCount>) { Converter::set_style_counts(self, counts)}
fn set_restyle(&mut self, on: bool) { Converter::set_restyle(self, on)}
fn style_weight(&self, ctx: &[u16]) -> [f32; 2] { Converter::style_weight(self, ctx)}
fn typo_spans(&self) -> Vec<(i32, i32)> { Converter::typo_spans(self)}
fn path_variants(&self, reading: &[u16], text: &[u16]) -> Vec<U16> { Converter::path_variants(self, reading, text)}
fn path_pos(&self, reading: &[u16], text: &[u16]) -> Vec<Word> { Converter::path_pos(self, reading, text)}
fn latin_word(&self, key: &[u16]) -> U16 { Converter::latin_word(self, key)}
fn complete(&mut self, prefix: &[u16], max: i32, max_extra: i32) -> Vec<U16> { Converter::complete(self, prefix, max, max_extra)}
fn set_user_words(&mut self, forms: &[DictForm], bonus: i32) -> i32 { Converter::set_user_words(self, forms, bonus)}
fn ai_learn(&mut self, ctx: &[u16], reading: &[u16], text: &[u16]) -> f32 { Converter::ai_learn(self, ctx, reading, text)}
}

/// 文の流れの予測 (flow.h の SentenceFlow 相当) の差し替え口。
pub trait Flow {
fn reset(&mut self);
fn boundary(&mut self);
fn backspace(&mut self);
fn feed(&mut self, text: &[u16]);
fn predict(&self) -> Vec<U16>;
fn suggest_use(&self, t: &[u16]) -> i32;
fn record_suggest(&mut self, t: &[u16]);
fn save(&mut self);
}

/// キー配列の打ち間違いの直し (呼ぶ側が返す)。
#[derive(Clone, Debug, Default)]
pub struct LayoutTypo {
pub orig: Vec<U16>,
pub alts: Vec<Vec<U16>>,
pub letters: bool,
pub qwerty: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Seg {
pub reading: U16,
pub surface: U16,
pub tail_r: U16,
pub tail_s: U16,
pub cands: Vec<U16>,
pub loaded: bool,
pub sel: i32,
pub changed: bool,
pub limit: i32,
pub tree_k: i32,
pub scores: Vec<f32>,
pub fork_idx: i32,
pub merged: Vec<Seg>,
pub own_reading: U16,
pub own_tail_r: U16,
pub own_tail_s: U16,
}
impl Seg {
pub fn new(reading: U16, surface: U16) -> Self {
Seg { reading, surface, fork_idx: -1,..Default::default()}
}
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum State { Idle, Input, Convert}

#[derive(Clone)]
struct Retype {
reading: U16,
surface: U16,
left: U16,
chars: i32,
erased: bool,
at: Instant,
}
impl Default for Retype {
fn default() -> Self { Retype { reading: Vec::new(), surface: Vec::new(), left: Vec::new(), chars: 0, erased: false, at: Instant::now()}}
}

/// 決まり文句の予測 (tables.h の PREDICTIVE_PHRASES)。
const PREDICTIVE_PHRASES: &[(&str, &str, usize)] = &[
("あけましておめでとうございます", "あけましておめでとうございます", 4),
    ("おめでとうございます", "おめでとうございます", 6),
    ("たんじょうびおめでとうございます", "誕生日おめでとうございます", 8),
    ("よろしくおねがいします", "よろしくお願いします", 5),
    ("よろしくおねがいいたします", "よろしくお願いいたします", 5),
    ("どうぞよろしくおねがいします", "どうぞよろしくお願いします", 5),
    ("こんごともよろしくおねがいします", "今後ともよろしくお願いします", 6),
    ("なにとぞよろしくおねがいもうしあげます", "何卒よろしくお願い申し上げます", 5),
    ("おつかれさまでした", "お疲れ様でした", 6),
    ("おつかれさまです", "お疲れ様です", 6),
    ("ありがとうございます", "ありがとうございます", 6),
    ("ありがとうございました", "ありがとうございました", 6),
    ("いつもありがとうございます", "いつもありがとうございます", 5),
    ("おせわになっております", "お世話になっております", 5),
    ("おせわになりました", "お世話になりました", 5),
    ("たいへんおせわになりました", "大変お世話になりました", 6),
    ("ごぶさたしております", "ご無沙汰しております", 5),
    ("しつれいします", "失礼します", 5),
    ("しつれいいたします", "失礼いたします", 5),
    ("かしこまりました", "かしこまりました", 6),
    ("しょうちいたしました", "承知いたしました", 6),
    ("おまたせしました", "お待たせしました", 5),
    ("おげんきですか", "お元気ですか", 5),
    ("おはようございます", "おはようございます", 6),
    ("たいへんもうしわけございません", "大変申し訳ございません", 6),
    ("おてすうをおかけしますが", "お手数をおかけしますが", 5),
    ("ごかくにんよろしくおねがいします", "ご確認よろしくお願いします", 6),
    ("よいおとしを", "よいお年を", 4),
];

const K_CANDIDATES: i32 = 30;

pub struct Composer {
    pub options: ComposerOptions,
    pub page: i32, // Composer::PAGE 相当 (候補の窓の 1 ページの数)
    pub on_log: Option<Box<dyn FnMut(&LogEvent)>>,
    pub layout_typo: Option<Box<dyn FnMut(&mut LayoutTypo) -> bool>>,
    /// 日付・時刻の候補 (datetime_predict 相当。(読み, UNIX 秒) -> 候補)。既定は無し。
    pub datetime_hook: Option<Box<dyn Fn(&[u16], i64) -> Vec<U16>>>,
    pub flow: Option<Box<dyn Flow>>,

    conv: Option<Box<dyn Convert>>,
    learning: Option<Learning>,
    dict: Option<UserDict>,
    style_log: Option<StyleLog>,
    app: String,
    context_provider: Option<Box<dyn Fn() -> U16>>,

    state: State,
    ctx: U16,
    history: U16,
    new_learn: Vec<Undo>,
    last_learn: Vec<Undo>,
    last_style: Vec<(U16, [f32; 2])>,
    last_commit: (U16, U16),
    retype: Retype,
    recent: Vec<(U16, U16)>,
    commit: U16,
    view_: View,

    kana: U16,
    caret: usize,
    romaji: SimpleRomaji,
    raw: U16,
    raw_valid: bool,
    alpha_run: bool,

    live_key: U16,
    live_read: U16,
    live_pending: bool,
    live_cands: Vec<U16>,
    live_paths: Vec<Vec<TreeWord>>,
    live_sugg: Vec<U16>,
    live_sugg_kinds: Vec<i32>,
    typo_top: U16,
    typo_spans: Vec<(i32, i32)>,
    live_pred: Vec<U16>,
    live_phrase: U16,
    restore: U16,
    after_emoji: Vec<U16>,
    sugg_typo: Vec<U16>,
    sugg_set: Vec<U16>,
    date_cands: Vec<U16>,
    sugg_focus: i32,
    live_top: U16,
    prev_segs: HashMap<usize, U16>,
    frozen: Vec<Seg>,
    keep_: usize,
    calibrated: bool,

    segs: Vec<Seg>,
    base: Vec<(U16, U16)>,
    whole: Vec<U16>,
    first_cands: Vec<U16>,
    tree: Vec<TreeNode>,
    last_tree: Vec<TreeNode>,
    learn_version: u64,
    marks_learn: u64,
    marks_style: u64,
    marks_app: String,
    marks_on: bool,
    tree_off: i32,
    live_words: Vec<Word>,
    pred_tab: Vec<i32>,
    first_via: String,
    focus: usize,
    cand_open: bool,
    dict_version: u32,
    last_f: Key,
    f_cycle: i32,
    froze_r: usize,
    froze_s: usize,
    post: Vec<U16>,
}

impl Composer {
    pub const KEEP: usize = 8;
    pub const KEEP_MAX: usize = 32;

    pub fn new(conv: Option<Box<dyn Convert>>, learning: Option<Learning>, dict: Option<UserDict>) -> Self {
        let mut c = Composer {
            options: ComposerOptions::default(),
            page: 9,
            on_log: None,
            layout_typo: None,
            datetime_hook: None,
            flow: None,
            conv, learning, dict,
            style_log: None,
            app: String::new(),
            context_provider: None,
            state: State::Idle,
            ctx: Vec::new(),
            history: Vec::new(),
            new_learn: Vec::new(),
            last_learn: Vec::new(),
            last_style: Vec::new(),
            last_commit: (Vec::new(), Vec::new()),
            retype: Retype::default(),
            recent: Vec::new(),
            commit: Vec::new(),
            view_: View::new(),
            kana: Vec::new(),
            caret: 0,
            romaji: SimpleRomaji::new(),
            raw: Vec::new(),
            raw_valid: true,
            alpha_run: false,
            live_key: Vec::new(),
            live_read: Vec::new(),
            live_pending: false,
            live_cands: Vec::new(),
            live_paths: Vec::new(),
            live_sugg: Vec::new(),
            live_sugg_kinds: Vec::new(),
            typo_top: Vec::new(),
            typo_spans: Vec::new(),
            live_pred: Vec::new(),
            live_phrase: Vec::new(),
            restore: Vec::new(),
            after_emoji: Vec::new(),
            sugg_typo: Vec::new(),
            sugg_set: Vec::new(),
            date_cands: Vec::new(),
            sugg_focus: -1,
            live_top: Vec::new(),
            prev_segs: HashMap::new(),
            frozen: Vec::new(),
            keep_: Self::KEEP,
            calibrated: false,
            segs: Vec::new(),
            base: Vec::new(),
            whole: Vec::new(),
            first_cands: Vec::new(),
            tree: Vec::new(),
            last_tree: Vec::new(),
            learn_version: 0,
            marks_learn: 0,
            marks_style: 0,
            marks_app: String::new(),
            marks_on: false,
            tree_off: 0,
            live_words: Vec::new(),
            pred_tab: Vec::new(),
            first_via: String::new(),
            focus: 0,
            cand_open: false,
            dict_version: 0,
            last_f: Key::Char,
            f_cycle: 0,
            froze_r: 0,
            froze_s: 0,
            post: Vec::new(),
        };
        // 品詞の記録の無い学習の表記は、読みと表記だけで網をたどって品詞を決める。
        // Rust 版: Converter が trait object のため、resolver は Composer の生成後に
        // set_lid_resolver_via_converter で付ける形にした (元のコンストラクタ内蔵とは差異。報告に明記)。
        let _ = &mut c;
        c
    }

    pub fn learning_mut(&mut self) -> Option<&mut Learning> { self.learning.as_mut() }
    pub fn dict_mut(&mut self) -> Option<&mut UserDict> { self.dict.as_mut() }
    pub fn set_style_log(&mut self, s: StyleLog) { self.style_log = Some(s); }
    pub fn set_app(&mut self, app: &str) { self.app = app.to_string(); }
    pub fn set_context_provider(&mut self, f: Box<dyn Fn() -> U16>) { self.context_provider = Some(f); }
    pub fn raw_keys(&self) -> &[u16] { &self.raw }
    pub fn raw_keys_valid(&self) -> bool { self.raw_valid }
    pub fn reset_history(&mut self) {
        self.history.clear();
        self.post.clear();
        self.last_commit = (Vec::new(), Vec::new());
        self.retype = Retype::default();
        if let Some(f) = &mut self.flow { f.reset(); }
    }
    pub fn undoable(&self) -> &(U16, U16) { &self.last_commit }
    pub fn add_romaji(&mut self, key: &[u16], kana: &[u16]) { self.romaji.add_pair(key, kana); }
    pub fn reset_romaji(&mut self) { self.romaji = SimpleRomaji::new(); }
    pub fn set_romaji_extended(&mut self, on: bool) { self.romaji.set_extended(on); }
    pub fn clear_romaji_table(&mut self) { self.romaji.clear_table(); }
    pub fn clear_recent(&mut self) { self.recent.clear(); }
    pub fn keep(&self) -> usize { self.keep_ }
    pub fn set_keep(&mut self, n: usize) { self.keep_ = n; self.calibrated = true; }
    pub fn composing(&self) -> bool { self.state != State::Idle }
    pub fn converting(&self) -> bool { self.state == State::Convert }
    pub fn view(&self) -> &View { &self.view_ }
    pub fn take_commit(&mut self) -> U16 { std::mem::take(&mut self.commit) }
    pub fn live_waiting(&self) -> bool { self.live_pending }
    fn quiet(&self) -> bool { self.options.private_mode || self.options.secure_field }
    fn log(&mut self, e: LogEvent) {
        if let Some(f) = &mut self.on_log { f(&e); }
    }

    // ---------------------------------------------------------------- キー
    pub fn will_handle(&self, ev: &KeyEvent) -> bool {
        if ev.ctrl && !ev.alt && self.state == State::Convert && ev.key == Key::Delete { return true; }
        if ev.ctrl || ev.alt { return false; }
        if self.state == State::Idle {
            return ev.key == Key::Char || ev.key == Key::Space || (ev.key == Key::Tab && !ev.shift && !self.post.is_empty());
        }
        true
    }

    pub fn press(&mut self, ev: &KeyEvent) -> bool {
        if !self.will_handle(ev) { return false; }
        self.last_commit = (Vec::new(), Vec::new());
        let mut handled = false;
        match self.state {
            State::Idle => {
                if ev.key == Key::Tab {
                    let t = self.post[0].clone();
                    self.commit_text(&t);
                    handled = true;
                } else if ev.key == Key::Space {
                    let mut full = ev.shift != self.options.space_fullwidth;
                    if self.options.space_literal && !self.history.is_empty() {
                        full = *self.history.last().unwrap() >= 0x80;
                    }
                    self.commit_text(&u16s(if full { "　" } else { " " }));
                    handled = true;
                } else if ev.key == Key::Char {
                    self.start_input();
                    self.type_char(ev.ch, ev.raw);
                    handled = true;
                }
            }
            State::Input => handled = self.press_input(ev),
            State::Convert => handled = self.press_convert(ev),
        }
        if ev.key != Key::F9 && ev.key != Key::F10 && ev.key != Key::Muhenkan { self.last_f = Key::Char; }
        if ev.key == Key::Char && self.options.punct_commit { self.commit_at_punct(); }
        if self.state == State::Input { self.after_input(ev.key == Key::Char); }
        if self.state == State::Convert && self.options.always_cands && !self.segs.is_empty() {
            let f = self.focus;
            if !self.segs[f].loaded { self.load_cands(f); }
            self.cand_open = true;
        }
        self.rebuild_view();
        handled
    }

    pub fn passthrough(&mut self, ev: &KeyEvent) {
        self.last_commit = (Vec::new(), Vec::new());
        if ev.key == Key::Backspace && !self.retype.surface.is_empty() && !self.retype.erased {
            self.retype.chars -= 1;
            if self.retype.chars <= 0 { self.retype.erased = true; }
        } else {
            self.retype = Retype::default();
        }
        if self.flow.is_none() || self.state != State::Idle { return; }
        {
            let f = self.flow.as_mut().unwrap();
            if ev.key == Key::Backspace { f.backspace(); }
            else if ev.key == Key::Enter { f.boundary(); }
            else { f.reset(); }
            f.save();
        }
        self.post.clear();
        self.rebuild_view();
    }

    fn start_input(&mut self) {
        self.post.clear();
        let c = match &self.context_provider { Some(f) => f(), None => Vec::new() };
        let src = if c.is_empty() { self.history.clone() } else { c };
        self.ctx = tail(&src, self.options.ctx_max);
        self.state = State::Input;
        self.kana.clear();
        self.caret = 0;
        self.romaji.reset();
        self.raw.clear();
        self.raw_valid = true;
        self.alpha_run = false;
        self.live_key.clear();
        self.live_cands.clear();
        self.live_read.clear();
        self.live_pending = false;
        self.prev_segs.clear();
    }

    fn insert(&mut self, t: &[u16]) {
        let at = self.caret;
        self.kana.splice(at..at, t.iter().copied());
        self.caret += t.len();
    }

    pub fn type_kana(&mut self, s: &[u16]) {
        if s.is_empty() { return; }
        self.last_commit = (Vec::new(), Vec::new());
        if self.state == State::Convert {
            let mut carry = self.ctx.clone();
            for x in &self.segs { carry.extend(&x.surface); }
            self.commit_all();
            self.start_input();
            self.ctx = tail(&carry, self.options.ctx_max);
        } else if self.state == State::Idle {
            self.start_input();
        }
        self.flush_romaji();
        self.raw_valid = false;
        for &c in s {
            let is_mod = c == 0xE000 || c == 0xE001 || c == '゛' as u16 || c == '゜' as u16;
            if is_mod && self.caret > 0 {
                let mut prev = self.kana[self.caret - 1];
                // E000/E001 (拗・修) は未移植: 濁点・半濁点のみ kana_modify で扱う
                let mc = if c == '゛' as u16 { 0x309B } else if c == '゜' as u16 { 0x309C } else { c };
                if kana_modify(&mut prev, mc) { self.kana[self.caret - 1] = prev; continue; }
            }
            if c == 0xE000 || c == 0xE001 { continue; }
            self.insert(&[c]);
        }
        self.after_input(true);
        self.rebuild_view();
    }

    fn flush_romaji(&mut self) {
        if !self.romaji.pending().is_empty() {
            let f = self.romaji.flush();
            self.insert(&f);
        }
    }

    fn type_char(&mut self, c: u16, raw: bool) {
        if self.raw_valid && self.caret == self.kana.len() { self.raw.push(c); } else { self.raw_valid = false; }
        if raw {
            self.flush_romaji();
            let full = self.options.numpad == 2 || (self.options.numpad == 1 && self.options.digits_fullwidth);
            let ch = if full && c > 0x20 && c < 0x7F { c + 0xFEE0 } else { c };
            self.insert(&[ch]);
            return;
        }
        let prev = if self.caret > 0 { self.kana[self.caret - 1] } else { 0 };
        if is_upper(c) && !self.options.caps_alpha {
            self.flush_romaji();
            self.insert(&[c]);
            return;
        }
        if is_upper(c) {
            self.flush_romaji();
            self.alpha_run = true;
            self.insert(&[c]);
            return;
        }
        if self.alpha_run && c != b' ' as u16 && c < 0x7F {
            self.flush_romaji();
            self.insert(&[c]);
            return;
        }
        if self.romaji.pending().is_empty() && is_digit(prev) && [b'-' as u16, b',' as u16, b'.' as u16, b':' as u16, b'/' as u16].contains(&c) {
            self.insert(&[c]);
            return;
        }
        if c == b'-' as u16 && self.romaji.pending().is_empty() && self.options.half_symbols.contains(&(b'-' as u16)) {
            self.insert(&u16s("-"));
            return;
        }
        if is_lower(c) || self.romaji.accepts(c) || c == b'-' as u16 {
            if self.romaji.pending().is_empty() && is_lower(c) {
                let mut n = 0usize;
                while n < 3 && self.caret > n && is_lower(self.kana[self.caret - 1 - n]) { n += 1; }
                let mut m = n;
                while m > 0 {
                    let back: U16 = self.kana[self.caret - m..self.caret].to_vec();
                    let sokuon = m == 1 && back[0] == c && c != b'n' as u16 && !u16s("aiueo").contains(&c);
                    let mut bc = back.clone();
                    bc.push(c);
                    if !self.romaji.is_prefix(&bc) && !sokuon { m -= 1; continue; }
                    self.kana.drain(self.caret - m..self.caret);
                    self.caret -= m;
                    self.romaji.unread(&back);
                    break;
                }
            }
            let r = self.romaji.process(c);
            self.insert(&r.committed);
            return;
        }
        self.flush_romaji();
        if (c == 0x309B || c == 0x309C) && self.caret > 0 {
            let mut p = self.kana[self.caret - 1];
            if kana_modify(&mut p, c) { self.kana[self.caret - 1] = p; return; }
        }
        if c > 0x20 && c < 0x7F && !is_digit(c) && self.options.half_symbols.contains(&c) {
            self.insert(&[c]);
        } else if is_digit(c) {
            self.insert(&[if self.options.digits_fullwidth { c + 0xFEE0 } else { c }]);
        } else if c == b',' as u16 {
            self.insert(&u16s(if self.options.punct == 1 || self.options.punct == 3 { "，" } else { "、" }));
        } else if c == b'.' as u16 {
            self.insert(&u16s(if self.options.punct == 1 || self.options.punct == 2 { "．" } else { "。" }));
        } else {
            self.insert(&[japanese_symbol(c)]);
        }
    }

    fn press_input(&mut self, ev: &KeyEvent) -> bool {
        if self.sugg_focus >= 0 {
            let n = self.live_sugg.len() as i32;
            match ev.key {
                Key::Right | Key::Down => { self.sugg_focus = (self.sugg_focus + 1) % n.max(1); return true; }
                Key::Left => { self.sugg_focus = (self.sugg_focus + n - 1) % n.max(1); return true; }
                Key::Tab => {
                    self.sugg_focus = if ev.shift { (self.sugg_focus + n - 1) % n.max(1) } else { (self.sugg_focus + 1) % n.max(1) };
                    return true;
                }
                Key::Enter | Key::Space => { let f = self.sugg_focus; self.select_suggestion(f); return true; }
                Key::Escape | Key::Up => { self.sugg_focus = -1; return true; }
                _ => { self.sugg_focus = -1; }
            }
        }
        match ev.key {
            Key::Char => { self.type_char(ev.ch, ev.raw); true }
            Key::Space if !self.options.space_literal => { self.convert_via_space(); true }
            Key::Space => {
                self.flush_romaji();
                let at = self.caret;
                self.kana.insert(at, b' ' as u16);
                self.caret += 1;
                if self.raw_valid && self.caret == self.kana.len() { self.raw.push(b' ' as u16); } else { self.raw_valid = false; }
                true
            }
            Key::Henkan => { self.convert_via_space(); true }
            Key::Down | Key::Tab => {
                self.flush_romaji();
                self.update_live();
                if !self.live_cands.is_empty() {
                    let mut sel = 0i32;
                    self.pred_tab.clear();
                    if ev.key == Key::Tab {
                        for (i, x) in self.live_cands.iter().enumerate() {
                            if contains(&self.live_pred, x) { self.pred_tab.push(i as i32); }
                        }
                    }
                    if !self.pred_tab.is_empty() { sel = self.pred_tab[0]; }
                    let cands = self.live_cands.clone();
                    let surface = cands[sel as usize].clone();
                    self.enter_single(&surface, cands, sel);
                    self.first_cands = self.live_cands.clone();
                    self.first_via = "live".into();
                }
                true
            }
            Key::Enter => {
                self.flush_romaji();
                if ev.shift && !self.live_sugg.is_empty() {
                    self.update_live();
                    if !self.live_sugg.is_empty() { self.select_suggestion(0); return true; }
                }
                self.commit_shown();
                true
            }
            Key::Escape => {
                let mut r = self.frozen_reading();
                r.extend(&self.kana);
                r.extend(self.romaji.pending());
                self.log(LogEvent { kind: "clear".into(), reading: r, ctx: self.ctx.clone(), ..Default::default() });
                if !self.cancel_reconvert() { self.to_idle(); }
                true
            }
            Key::Backspace => {
                if self.romaji.pending().is_empty() && self.caret == 0 { self.unfreeze_last(); }
                if self.caret == self.kana.len() && (if self.romaji.pending().is_empty() { self.delete_last_char() } else { self.undo_last_key() }) {
                    // 打ち直しで戻せた
                } else if !self.romaji.pending().is_empty() {
                    self.romaji.backspace();
                    if self.raw_valid && !self.raw.is_empty() { self.raw.pop(); }
                } else if self.caret > 0 {
                    self.kana.remove(self.caret - 1);
                    self.caret -= 1;
                    self.raw_valid = false;
                }
                if self.kana.is_empty() && self.romaji.pending().is_empty() && self.frozen.is_empty() { self.to_idle(); }
                true
            }
            Key::Delete => {
                self.flush_romaji();
                if self.caret < self.kana.len() { self.kana.remove(self.caret); self.raw_valid = false; }
                if self.kana.is_empty() && self.frozen.is_empty() { self.to_idle(); }
                true
            }
            Key::Left | Key::Right => {
                if self.options.space_literal && !ev.shift {
                    self.flush_romaji();
                    self.update_live();
                    self.convert_all();
                    if ev.key == Key::Left && !self.segs.is_empty() { self.focus = self.segs.len() - 1; }
                    return true;
                }
                if ev.key == Key::Right {
                    self.flush_romaji();
                    if self.caret < self.kana.len() { self.caret += 1; }
                    return true;
                }
                self.flush_romaji();
                if self.caret == 0 { self.unfreeze_last(); }
                if self.caret > 0 { self.caret -= 1; }
                true
            }
            Key::Home => {
                self.flush_romaji();
                while self.unfreeze_last() {}
                self.caret = 0;
                true
            }
            Key::End => {
                self.flush_romaji();
                self.caret = self.kana.len();
                true
            }
            Key::F6 | Key::F7 | Key::F8 | Key::F9 | Key::F10 | Key::Muhenkan => {
                self.flush_romaji();
                if self.kana.is_empty() && self.frozen.is_empty() { self.to_idle(); return true; }
                if self.kana.is_empty() { self.unfreeze_last(); }
                let kana = self.kana.clone();
                self.enter_single(&kana.clone(), Vec::new(), 0);
                self.first_via = "fkey".into();
                self.transform(ev.key);
                true
            }
            _ => true,
        }
    }

    fn convert_via_space(&mut self) {
        self.flush_romaji();
        self.update_live();
        if (!self.typo_top.is_empty() || !self.sugg_typo.is_empty()) && !self.live_cands.is_empty() {
            let cands = self.live_cands.clone();
            let top = cands[0].clone();
            self.enter_single(&top, cands, 0);
            self.first_cands = self.live_cands.clone();
            self.first_via = "space".into();
            return;
        }
        self.convert_all();
    }

    fn set_sel(&mut self, i: i32) {
        let f = self.focus;
        if !self.segs[f].loaded { self.load_cands(f); }
        if self.segs[f].cands.is_empty() { return; }
        let mut i = i;
        {
            let s = &self.segs[f];
            if i >= 0 && i / self.page >= (s.cands.len() as i32 - 1) / self.page {
                // 最後のページ: 続きを読む (load_more は &mut が要るので下で)
            }
        }
        if i >= 0 && i / self.page >= (self.segs[f].cands.len() as i32 - 1) / self.page { self.load_more(f); }
        if i < 0 { i = 0; }
        let n = self.segs[f].cands.len() as i32;
        let old = self.segs[f].sel;
        let sel = ((i % n) + n) % n;
        self.segs[f].sel = sel;
        self.segs[f].surface = self.segs[f].cands[sel as usize].clone();
        self.segs[f].changed = true;
        self.cand_open = true;
        // after_pick (借用回避のため clone して処理し、必要な変更を戻す簡易版)
        self.after_pick(f, old);
    }

    fn press_convert(&mut self, ev: &KeyEvent) -> bool {
        match ev.key {
            Key::Char => {
                if self.cand_open && (b'1' as u16..=b'9' as u16).contains(&ev.ch) {
                    let f = self.focus;
                    let idx = (self.segs[f].sel / self.page) * self.page + (ev.ch - b'1' as u16) as i32;
                    if idx < self.segs[f].cands.len() as i32 {
                        self.set_sel(idx);
                        if !self.options.always_cands { self.cand_open = false; }
                    }
                    return true;
                }
                let mut carry = self.ctx.clone();
                for x in &self.segs { carry.extend(&x.surface); }
                self.commit_all();
                self.start_input();
                self.ctx = tail(&carry, self.options.ctx_max);
                self.type_char(ev.ch, ev.raw);
                true
            }
            Key::Tab => {
                if !self.pred_tab.is_empty() {
                    let f = self.focus;
                    let cur = self.segs[f].sel;
                    let m = self.pred_tab.len() as i32;
                    let k = self.pred_tab.iter().position(|&x| x == cur).map(|p| p as i32).unwrap_or(0);
                    let nk = ((if ev.shift { k - 1 } else { k + 1 }) % m + m) % m;
                    let target = self.pred_tab[nk as usize];
                    self.set_sel(target);
                    return true;
                }
                let cur = self.segs[self.focus].sel;
                self.set_sel(if ev.shift { cur - 1 } else { cur + 1 });
                true
            }
            Key::Space if self.options.space_literal => {
                self.commit_all();
                // 何も打っていないときの空白として処理し直す
                let ev2 = KeyEvent { key: Key::Space, ..Default::default() };
                self.press(&ev2);
                true
            }
            Key::Space | Key::Henkan | Key::Down => {
                self.pred_tab.clear();
                let cur = self.segs[self.focus].sel;
                self.set_sel(if ev.shift && ev.key != Key::Down { cur - 1 } else { cur + 1 });
                true
            }
            Key::Up => { let cur = self.segs[self.focus].sel; self.set_sel(cur - 1); true }
            Key::PageDown => {
                if self.cand_open {
                    let f = self.focus;
                    let t = ((self.segs[f].sel / self.page + 1) * self.page).min(self.segs[f].cands.len() as i32 - 1);
                    self.set_sel(t);
                }
                true
            }
            Key::PageUp => {
                if self.cand_open {
                    let f = self.focus;
                    let t = ((self.segs[f].sel / self.page - 1) * self.page).max(0);
                    self.set_sel(t);
                }
                true
            }
            Key::Enter => { self.commit_all(); true }
            Key::Escape => {
                if self.cand_open && !self.options.always_cands { self.cand_open = false; }
                else if !self.cancel_reconvert() { self.back_to_input(); }
                true
            }
            Key::Backspace => { self.back_to_input(); true }
            Key::Delete => {
                if ev.ctrl {
                    self.hide_focus_candidate();
                }
                true
            }
            Key::Left | Key::Right => {
                if ev.shift { self.resize_focus(if ev.key == Key::Left { -1 } else { 1 }); return true; }
                self.cand_open = false;
                if ev.key == Key::Left && self.focus > 0 { self.focus -= 1; }
                if ev.key == Key::Right && self.focus + 1 < self.segs.len() { self.focus += 1; }
                true
            }
            Key::Home => { self.cand_open = false; self.focus = 0; true }
            Key::End => { self.cand_open = false; self.focus = self.segs.len().saturating_sub(1); true }
            Key::F6 | Key::F7 | Key::F8 | Key::F9 | Key::F10 | Key::Muhenkan => { self.transform(ev.key); true }
            _ => true,
        }
    }

    fn hide_focus_candidate(&mut self) {
        if self.learning.is_none() { return; }
        let f = self.focus;
        if !self.segs[f].loaded { self.load_cands(f); }
        if self.segs[f].cands.is_empty() { return; }
        let sel = self.segs[f].sel.clamp(0, self.segs[f].cands.len() as i32 - 1);
        let x = self.segs[f].cands[sel as usize].clone();
        let reading = self.segs[f].reading.clone();
        if x == reading || x == to_katakana(&reading) { return; }
        if !self.quiet() {
            let keep = !self.options.private_mode;
            self.learning.as_mut().unwrap().hide(&reading, &x, keep);
        }
        let left = self.left_context(f);
        let cands = self.segs[f].cands.clone();
        self.log(LogEvent { kind: "hide".into(), reading: reading.clone(), surface: x.clone(), ctx: left, cands, index: sel, ..Default::default() });
        let s = &mut self.segs[f];
        if let Some(at) = s.cands.iter().position(|c| *c == x) {
            if at < s.scores.len() { s.scores.remove(at); }
            if (at as i32) < s.tree_k { s.tree_k -= 1; }
            if at as i32 == s.fork_idx { s.fork_idx = -1; } else if (at as i32) < s.fork_idx { s.fork_idx -= 1; }
        }
        s.cands.retain(|c| *c != x);
        self.cand_open = true;
        let t = s.sel.min(s.cands.len() as i32 - 1);
        if t >= 0 { self.set_sel(t); }
    }
}

impl Composer {
    // ---------------------------------------------------------------- 変換
    fn sync_personal(&mut self) {
        if self.conv.is_none() { return; }
        let on = !self.options.private_mode;
        let lv = if on { self.learning.as_mut().map(|l| l.version()).unwrap_or(0) } else { 0 };
        let sv = if on { self.style_log.as_mut().map(|s| s.version()).unwrap_or(0) } else { 0 };
        if on == self.marks_on && lv == self.marks_learn && sv == self.marks_style && self.app == self.marks_app { return; }
        let marks = if on { self.learning.as_mut().map(|l| l.marks()).unwrap_or_default() } else { Vec::new() };
        let counts: HashMap<U16, StyleCount> = if on {
            self.style_log.as_mut().map(|s| s.counts().clone()).unwrap_or_default()
        } else { HashMap::new() };
        let conv = self.conv.as_mut().unwrap();
        if conv.personal() { conv.set_marks(&marks); }
        conv.set_style_counts(&counts);
        conv.set_restyle(on && self.style_log.is_some());
        self.marks_on = on;
        self.marks_learn = lv;
        self.marks_style = sv;
        self.marks_app = self.app.clone();
    }

    fn sync_user_dict(&mut self) {
        self.sync_personal();
        if self.dict.is_none() || self.conv.is_none() { return; }
        self.dict.as_mut().unwrap().refresh();
        let lv = if !self.options.private_mode { self.learning.as_mut().map(|l| l.version()).unwrap_or(0) } else { 0 };
        let dv = self.dict.as_ref().unwrap().version();
        if dv != self.dict_version || lv != self.learn_version {
            let mut forms: Vec<DictForm> = self.dict.as_ref().unwrap().engine_forms();
            if lv > 0 {
                // 一度呼んで版だけ合わせる元の二段呼びは、Rust 版では strong を足してから 1 回だけ呼ぶ
                let strongs = self.learning.as_mut().unwrap().strong(4);
                for st in strongs {
                    let mut r = st.reading.clone();
                    let mut s = st.surface.clone();
                    while !r.is_empty() && !s.is_empty() && *r.last().unwrap() == *s.last().unwrap()
                        && (0x3041..=0x3096).contains(r.last().unwrap()) { r.pop(); s.pop(); }
                    if r.is_empty() || s.is_empty() || r == s { continue; }
                    if !self.writable(&st.reading, &st.surface) { continue; }
                    if r.len() < 2 && st.lid < 0 { continue; }
                    forms.push(DictForm {
                        reading: r, surface: s, tmpl_reading: Vec::new(), tmpl_surface: Vec::new(),
                        whole: false, bonus: if st.count >= 3 { 1000 } else { 600 }, lid: st.lid,
                    });
                }
            }
            self.conv.as_mut().unwrap().set_user_words(&forms, 800);
            self.dict_version = dv;
            self.learn_version = lv;
            self.live_key.clear();
        }
    }

    fn writable(&self, reading: &[u16], surface: &[u16]) -> bool {
        match &self.conv {
            Some(c) => !c.path_pos(reading, surface).is_empty() || surface == to_halfwidth_kana(reading),
            None => false,
        }
    }

    fn convert_phrases(&mut self, ctx: &[u16], reading: &[u16], whole: Option<&mut Vec<U16>>) -> Vec<Seg> {
        self.prime_keys(reading);
        let mut c = match &mut self.conv {
            Some(cv) => cv.convert(ctx, reading, K_CANDIDATES),
            None => Conversion::default(),
        };
        self.last_tree = std::mem::take(&mut c.tree);
        if let Some(w) = whole {
            *w = c.cands.clone();
            prefer_greetings(reading, w);
        }
        let mut words = c.words.clone();
        if words.is_empty() {
            words.push(Word { reading: reading.to_vec(), surface: if c.cands.is_empty() { reading.to_vec() } else { c.cands[0].clone() }, ..Default::default() });
        }
        let mut left = ctx.to_vec();
        let mut wi = 0usize;
        let mut out = Vec::new();
        for p in group_phrases(&words) {
            let plid = if wi < words.len() { words[wi].lid } else { -1 };
            let mut r = 0usize;
            while r < p.reading.len() && wi < words.len() { r += words[wi].reading.len(); wi += 1; }
            let mut s = Seg::new(p.reading.clone(), greeting_fix(&p.reading, &p.surface));
            if p.head_r < p.reading.len() && p.head_s < p.surface.len() {
                s.tail_r = p.reading[p.head_r..].to_vec();
                s.tail_s = p.surface[p.head_s..].to_vec();
            }
            if let Some(l) = &mut self.learning {
                let t = l.top(&p.reading, &left, plid);
                if !t.is_empty() && t != s.surface && self.writable(&p.reading, &t) { s.surface = t; }
            }
            left.extend(&s.surface);
            out.push(s);
        }
        out
    }

    pub fn convert_all(&mut self) {
        self.flush_romaji();
        if self.kana.is_empty() && !self.unfreeze_last() { self.to_idle(); return; }
        self.sync_user_dict();
        let mut segs: Vec<Seg> = std::mem::take(&mut self.frozen);
        let kana = self.kana.clone();
        let ctx = self.conv_ctx();
        let mut whole: Vec<U16> = Vec::new();
        let mut rest = self.convert_phrases(&ctx, &kana, Some(&mut whole));
        self.whole = whole;
        self.tree = std::mem::take(&mut self.last_tree);
        self.tree_off = 0;
        for s in &segs { self.tree_off += s.reading.len() as i32; }
        // 打っている間の一覧の 1 位が AI の 1 位と違うときは、その文を 1 つの文節で出す
        if !self.live_cands.is_empty() {
            let key = cat(&self.conv_ctx(), &[1]);
            let key = cat(&key, &kana);
            if key == self.live_key {
                let live: Vec<U16> = self.live_cands.iter().filter(|x| !contains(&self.live_pred, x)).cloned().collect();
                let mut joined: U16 = Vec::new();
                for s in &rest { joined.extend(&s.surface); }
                if !live.is_empty() && !self.whole.is_empty() && live[0] != self.whole[0] && live[0] != joined {
                    rest = vec![Seg::new(kana.clone(), live[0].clone())];
                    self.tree.clear();
                }
                if !live.is_empty() { self.whole = live; }
            }
        }
        // 固定した前の方を、後ろの文脈で変換し直す
        if self.conv.as_ref().map(|c| c.has_right_ctx()).unwrap_or(false) && !segs.is_empty()
            && segs.iter().all(|s| !s.changed) {
            let mut fr_r: U16 = Vec::new();
            let mut fr_s: U16 = Vec::new();
            let mut rest_s: U16 = Vec::new();
            for s in &segs { fr_r.extend(&s.reading); fr_s.extend(&s.surface); }
            for s in &rest { rest_s.extend(&s.surface); }
            let rctx: U16 = rest_s[..rest_s.len().min(self.options.rctx_max)].to_vec();
            if let Some(cv) = &mut self.conv { cv.set_right_ctx(&rctx); }
            let ctx2 = tail(&self.ctx.clone(), self.options.ctx_max);
            let again = self.convert_phrases(&ctx2, &fr_r, None);
            let mut asurf: U16 = Vec::new();
            for s in &again { asurf.extend(&s.surface); }
            if !again.is_empty() && asurf != fr_s {
                segs = again;
                self.tree_off = 0;
                for s in &segs { self.tree_off += s.reading.len() as i32; }
            }
        }
        let first_new = segs.len();
        let rest_len = rest.len();
        segs.extend(rest);
        self.segs = segs;
        // 消した候補が 1 位にいたら候補を読み直す
        for i in first_new..self.segs.len() {
            let (rd, sf) = (self.segs[i].reading.clone(), self.segs[i].surface.clone());
            let hidden = self.learning.as_mut().map(|l| l.hidden(&rd, &sf)).unwrap_or(false);
            if hidden {
                self.segs[i].surface.clear();
                self.load_cands(i);
                if !self.segs[i].cands.is_empty() && self.segs[i].cands[0].is_empty() { self.segs[i].cands.remove(0); }
                self.segs[i].surface = if self.segs[i].cands.is_empty() { self.segs[i].reading.clone() } else { self.segs[i].cands[0].clone() };
                self.segs[i].sel = 0;
            }
        }
        self.first_cands = self.whole.clone();
        if self.segs.len() > rest_len {
            let mut fs: U16 = Vec::new();
            for i in 0..self.segs.len() - rest_len { fs.extend(&self.segs[i].surface); }
            for x in &mut self.first_cands { *x = cat(&fs, x); }
        }
        self.first_via = "space".into();
        self.base = self.segs.iter().map(|s| (s.reading.clone(), s.surface.clone())).collect();
        if self.segs.len() != 1 { self.whole.clear(); }
        self.focus = 0;
        self.cand_open = false;
        self.state = State::Convert;
    }

    fn enter_single(&mut self, surface: &[u16], cands: Vec<U16>, sel: i32) {
        let mut s = Seg::new(self.kana.clone(), surface.to_vec());
        let mut cands = cands;
        if !cands.is_empty() {
            push_unique(&mut cands, &self.kana.clone());
            push_unique(&mut cands, &to_katakana(&self.kana));
            s.cands = cands;
            s.loaded = true;
            s.sel = sel;
            self.cand_open = true;
        } else {
            self.cand_open = false;
        }
        let mut segs: Vec<Seg> = std::mem::take(&mut self.frozen);
        segs.push(s);
        self.segs = segs;
        self.tree.clear();
        self.whole.clear();
        self.focus = self.segs.len() - 1;
        self.state = State::Convert;
    }

    fn left_context(&self, seg: usize) -> U16 {
        let mut c = self.ctx.clone();
        for i in 0..seg.min(self.segs.len()) { c.extend(&self.segs[i].surface); }
        tail(&c, self.options.ctx_max)
    }

    fn load_cands(&mut self, i: usize) {
        let (reading, tail_r, tail_s, surface) = {
            let s = &self.segs[i];
            (s.reading.clone(), s.tail_r.clone(), s.tail_s.clone(), s.surface.clone())
        };
        let mut list: Vec<U16> = Vec::new();
        if let Some(d) = &self.dict { for w in d.lookup(&reading) { push_unique(&mut list, &w); } }
        if self.segs.len() == 1 && !self.whole.is_empty() {
            for c in self.whole.clone() { push_unique(&mut list, &c); }
        } else if self.conv.is_some() {
            let head: U16 = reading[..reading.len() - tail_r.len()].to_vec();
            let ctx = self.left_context(i);
            let c1 = self.conv.as_mut().unwrap().convert(&ctx, &head, K_CANDIDATES);
            for c in &c1.cands { push_unique(&mut list, &cat(c, &tail_s)); }
            if !tail_r.is_empty() { push_unique(&mut list, &cat(&head, &tail_s)); }
            if !tail_r.is_empty() {
                let c2 = self.conv.as_mut().unwrap().convert(&ctx, &reading, 5);
                for c in &c2.cands { push_unique(&mut list, c); }
            }
        }
        add_brackets(&reading, &mut list);
        if let Some(l) = &mut self.learning { list = l.apply_order(&reading, &list); }
        list.retain(|x| *x != surface);
        list.insert(0, surface.clone());
        push_unique(&mut list, &reading);
        push_unique(&mut list, &to_katakana(&reading));
        // 候補の木 (cand_tree): 木に出ている表記を先頭に寄せる
        let mut tree_k = 0i32;
        let mut fork_idx = -1i32;
        let mut scores: Vec<f32> = vec![-1.0; list.len()];
        if self.options.cand_tree && !self.tree.is_empty() {
            let mut r = -self.tree_off;
            for k in 0..i { r += self.segs[k].reading.len() as i32; }
            let re = r + reading.len() as i32;
            'outer: for n in self.tree.clone() {
                for w in &n.main {
                    if w.rs != r || w.re != re { continue; }
                    let score_of = |x: &[u16]| -> f32 {
                        if *x == w.surface { return w.s; }
                        for a in &w.alts { if a.0 == x { return a.1; } }
                        -1.0
                    };
                    let mut top: Vec<(U16, f32)> = vec![(list[0].clone(), score_of(&list[0]))];
                    let has = |top: &Vec<(U16, f32)>, x: &[u16]| top.iter().any(|t| t.0 == x);
                    if !has(&top, &w.surface) { top.push((w.surface.clone(), w.s)); }
                    for a in &w.alts {
                        if a.1 >= 0.03 && top.len() < 5 && !has(&top, &a.0) { top.push(a.clone()); }
                    }
                    if !n.fork.is_empty() && w.surface == n.main[0].surface {
                        let mut fs: U16 = Vec::new();
                        let mut sc = 1.0f32;
                        for fw in &n.fork { fs.extend(&fw.surface); sc = sc.min(fw.s); }
                        if !has(&top, &fs) {
                            let mut at = 1usize;
                            while at < top.len() && top[at].1 >= sc { at += 1; }
                            top.insert(at, (fs, sc));
                            fork_idx = at as i32;
                        }
                    }
                    let rest: Vec<U16> = list.iter().filter(|x| !has(&top, x)).cloned().collect();
                    tree_k = top.len() as i32;
                    scores = top.iter().map(|t| t.1).collect();
                    list = top.into_iter().map(|t| t.0).collect();
                    list.extend(rest);
                    break 'outer;
                }
            }
        }
        scores.resize(list.len(), -1.0);
        let s = &mut self.segs[i];
        s.scores = scores;
        s.cands = list;
        s.sel = 0;
        s.loaded = true;
        s.tree_k = tree_k;
        s.fork_idx = fork_idx;
    }

    fn load_more(&mut self, i: usize) {
        if self.conv.is_none() { return; }
        let (limit, reading, tail_r, tail_s) = {
            let s = &self.segs[i];
            if s.limit >= 2000 { return; }
            (s.limit.max(K_CANDIDATES) * 4, s.reading.clone(), s.tail_r.clone(), s.tail_s.clone())
        };
        self.segs[i].limit = limit;
        let head: U16 = reading[..reading.len() - tail_r.len()].to_vec();
        let ctx = self.left_context(i);
        let mut more: Vec<U16> = Vec::new();
        if self.segs.len() == 1 && !self.whole.is_empty() {
            let c = self.conv.as_mut().unwrap().convert(&ctx, &reading, limit);
            more.extend(c.cands);
        } else {
            let c = self.conv.as_mut().unwrap().convert(&ctx, &head, limit);
            for x in &c.cands { more.push(cat(x, &tail_s)); }
        }
        let comp = self.conv.as_mut().unwrap().complete(&head, limit, 0);
        for w in comp { more.push(cat(&w, &tail_s)); }
        let s = &mut self.segs[i];
        for c in more { push_unique(&mut s.cands, &c); }
    }

    fn resize_focus(&mut self, delta: i32) {
        self.cand_open = false;
        let f = self.focus;
        let mut rest: U16 = Vec::new();
        for i in f + 1..self.segs.len() { rest.extend(&self.segs[i].reading); }
        let cur_len = self.segs[f].reading.len() as i32;
        let len = cur_len + delta;
        if len < 1 || len > cur_len + rest.len() as i32 { return; }
        let all = cat(&self.segs[f].reading.clone(), &rest);
        let mine: U16 = all[..len as usize].to_vec();
        let rest2: U16 = all[len as usize..].to_vec();
        let ctx = self.left_context(f);
        let c = match &mut self.conv { Some(cv) => cv.convert(&ctx, &mine, K_CANDIDATES), None => Conversion::default() };
        let mut ns = Seg::new(mine.clone(), if c.cands.is_empty() { mine.clone() } else { c.cands[0].clone() });
        let ph = group_phrases(&c.words);
        if ph.len() == 1 && ph[0].reading == mine && ph[0].head_r < mine.len() && ph[0].head_s < ph[0].surface.len() {
            ns.tail_r = mine[ph[0].head_r..].to_vec();
            ns.tail_s = ph[0].surface[ph[0].head_s..].to_vec();
        }
        ns.changed = true;
        self.segs.truncate(f);
        self.segs.push(ns);
        if !rest2.is_empty() {
            let ctx2 = self.left_context(f + 1);
            let after = self.convert_phrases(&ctx2, &rest2, None);
            self.segs.extend(after);
        }
        self.whole.clear();
    }

    fn alpha_source(&self, reading: &[u16]) -> U16 {
        if self.segs.len() == 1 && self.raw_valid && !self.raw.is_empty() { return self.raw.clone(); }
        self.romaji.to_romaji(reading)
    }

    fn transform(&mut self, f: Key) {
        let idx = self.focus;
        let reading = self.segs[idx].reading.clone();
        let out: U16 = match f {
            Key::F6 => to_hiragana(&reading),
            Key::F7 => { self.segs[idx].changed = true; to_katakana(&reading) }
            Key::F8 => to_halfwidth_kana(&reading),
            Key::Muhenkan => {
                self.f_cycle = if self.last_f == f { (self.f_cycle + 1) % 3 } else { 0 };
                let o = match self.f_cycle { 0 => to_katakana(&reading), 1 => to_halfwidth_kana(&reading), _ => to_hiragana(&reading) };
                if self.f_cycle == 0 { self.segs[idx].changed = true; }
                o
            }
            Key::F9 | Key::F10 => {
                self.f_cycle = if self.last_f == f { (self.f_cycle + 1) % 3 } else { 0 };
                let mut a = to_halfwidth_ascii(&self.alpha_source(&reading));
                a = match self.f_cycle { 0 => lower(&a), 1 => upper(&a), _ => capital(&a) };
                for c in &mut a { if *c == 'ー' as u16 { *c = b'-' as u16; } }
                if f == Key::F9 { to_fullwidth_ascii(&a) } else { a }
            }
            _ => return,
        };
        self.last_f = f;
        let s = &mut self.segs[idx];
        s.surface = out;
        s.cands.clear();
        s.loaded = false;
    }

    fn back_to_input(&mut self) {
        let mut shown: U16 = Vec::new();
        let mut reading: U16 = Vec::new();
        for s in &self.segs { shown.extend(&s.surface); reading.extend(&s.reading); }
        let fc = self.first_cands.clone();
        self.log(LogEvent { kind: "cancel".into(), reading: reading.clone(), surface: shown, ctx: self.ctx.clone(), cands: fc, index: -1, ..Default::default() });
        self.kana = reading;
        self.caret = self.kana.len();
        self.segs.clear();
        self.whole.clear();
        self.cand_open = false;
        self.state = State::Input;
        self.live_key.clear();
        self.prev_segs.clear();
    }

    // ---------------------------------------------------------------- 文節の選び直し・木の二股
    fn after_pick(&mut self, i: usize, old: i32) {
        let (fork_idx, sel) = (self.segs[i].fork_idx, self.segs[i].sel);
        if fork_idx >= 0 && old == fork_idx && sel != fork_idx { self.fork_unmerge(i); }
        if fork_idx >= 0 && sel == fork_idx && old != fork_idx && !self.fork_merge(i) {
            let old_surface = self.segs[i].cands[old as usize].clone();
            self.segs[i].sel = old;
            self.segs[i].surface = old_surface;
            return;
        }
        if sel != old { self.follow_after(i); }
    }

    fn fork_merge(&mut self, i: usize) -> bool {
        let mut st = -self.tree_off;
        for k in 0..i { st += self.segs[k].reading.len() as i32; }
        for n in self.tree.clone() {
            if n.fork.is_empty() || n.main.is_empty() || n.main[0].rs != st { continue; }
            let b = n.main.last().unwrap().re;
            let mut j = i + 1;
            let mut e = st + self.segs[i].reading.len() as i32;
            while j < self.segs.len() && e < b { e += self.segs[j].reading.len() as i32; j += 1; }
            if e != b { return false; }
            let merged: Vec<Seg> = self.segs[i + 1..j].to_vec();
            let s = &mut self.segs[i];
            s.own_reading = s.reading.clone();
            s.own_tail_r = s.tail_r.clone();
            s.own_tail_s = s.tail_s.clone();
            for m in &merged { let mr = m.reading.clone(); s.reading.extend(&mr); }
            s.merged = merged;
            s.tail_r.clear();
            s.tail_s.clear();
            self.segs.drain(i + 1..j);
            self.whole.clear();
            return true;
        }
        false
    }

    fn fork_unmerge(&mut self, i: usize) {
        if self.segs[i].merged.is_empty() && self.segs[i].own_reading.is_empty() { return; }
        let rest: Vec<Seg> = std::mem::take(&mut self.segs[i].merged);
        let s = &mut self.segs[i];
        s.reading = std::mem::take(&mut s.own_reading);
        s.tail_r = std::mem::take(&mut s.own_tail_r);
        s.tail_s = std::mem::take(&mut s.own_tail_s);
        for (k, seg) in rest.into_iter().enumerate() { self.segs.insert(i + 1 + k, seg); }
    }

    fn follow_after(&mut self, i: usize) {
        if !self.options.follow || self.conv.is_none() || i + 1 >= self.segs.len() { return; }
        let head_reading = |s: &Seg| -> U16 { s.reading[..s.reading.len() - s.tail_r.len().min(s.reading.len())].to_vec() };
        let mut end = i + 1;
        while end < self.segs.len() && !self.segs[end].changed && self.segs[end].own_reading.is_empty() { end += 1; }
        if end == i + 1 { return; }
        let mut same = self.base.len() == self.segs.len();
        for k in 0..=i { if same { same = self.base[k].0 == self.segs[k].reading && self.base[k].1 == self.segs[k].surface; } }
        let mut rest: U16 = Vec::new();
        for k in i + 1..end { rest.extend(&self.segs[k].reading); }
        if same {
            for k in i + 1..end {
                if self.base[k].0 != self.segs[k].reading || self.base[k].1 == self.segs[k].surface { continue; }
                let t = &mut self.segs[k];
                t.surface = self.base[k].1.clone();
                t.cands.clear();
                t.scores.clear();
                t.loaded = false;
                t.sel = 0;
            }
            return;
        }
        let keep_tree = std::mem::take(&mut self.last_tree);
        let ctx = self.left_context(i + 1);
        let again = self.convert_phrases(&ctx, &rest, None);
        self.last_tree = keep_tree;
        let picked_head = head_reading(&self.segs[i]);
        let mut k = i + 1;
        for a in again {
            if k >= end || a.reading != self.segs[k].reading { break; }
            let t_reading = self.segs[k].reading.clone();
            let t_surface = self.segs[k].surface.clone();
            let t_head = head_reading(&self.segs[k]);
            k += 1;
            if a.surface == t_surface || t_head == picked_head { continue; }
            let t = &mut self.segs[k - 1];
            let _ = t_reading;
            t.surface = a.surface;
            t.tail_r = a.tail_r;
            t.tail_s = a.tail_s;
            t.cands.clear();
            t.scores.clear();
            t.loaded = false;
            t.sel = 0;
            t.tree_k = 0;
            t.fork_idx = -1;
        }
    }
}

/// 文の語のうち、読みの位置 r0 から始まる文節の頭の語の品詞 (接頭詞は飛ばす)。無ければ -1。
fn head_lid(ws: &[Word], r0: usize) -> i32 {
    let mut r = 0usize;
    for i in 0..ws.len() {
        if r == r0 {
            let mut k = i;
            while k + 1 < ws.len() && crate::converter::pos_class(ws[k].lid) == b'2' { k += 1; }
            return ws[k].lid;
        }
        if r > r0 { return -1; }
        r += ws[i].reading.len();
    }
    -1
}

fn settle_spaces(mut s: U16) -> U16 {
    let mut i = 0;
    while i < s.len() {
        if s[i] != b' ' as u16 && s[i] != 0x3000 { i += 1; continue; }
        let mut a = i;
        let mut b = i + 1;
        while a > 0 && (s[a - 1] == b' ' as u16 || s[a - 1] == 0x3000) { a -= 1; }
        while b < s.len() && (s[b] == b' ' as u16 || s[b] == 0x3000) { b += 1; }
        if !(a == 0 && b == s.len()) {
            let hp = a > 0 && s[a - 1] < 0x80;
            let hn = b < s.len() && s[b] < 0x80;
            let fp = a > 0 && s[a - 1] >= 0x80;
            let fnext = b < s.len() && s[b] >= 0x80;
            s[i] = if hp || hn { b' ' as u16 } else if (fp && (fnext || b == s.len())) || (a == 0 && fnext) { 0x3000 } else { b' ' as u16 };
        }
        i += 1;
    }
    s
}

fn camel(w: &[u16]) -> bool {
    let mut up = false;
    let mut low = false;
    for (i, &c) in w.iter().enumerate() {
        if is_upper(c) { if i > 0 { up = true; } }
        else if is_lower(c) { low = true; }
    }
    up && low
}

impl Composer {
    // ---------------------------------------------------------------- 候補・提案の選択
    fn sugg_note(&self, x: &[u16]) -> U16 {
        if (!self.typo_top.is_empty() && x == self.typo_top) || contains(&self.sugg_typo, x) { return u16s("もしかして"); }
        if contains(&self.date_cands, x) { return u16s("日付"); }
        if !contains(&self.sugg_set, x) { return Vec::new(); }
        let mut k = 0;
        for (i, s) in self.live_sugg.iter().enumerate() { if s == x { k = self.live_sugg_kinds.get(i).copied().unwrap_or(0); break; } }
        if k == 2 { return u16s("日付"); }
        if k == 3 { return u16s("住所"); }
        if k == 4 { return u16s("英語"); }
        let mut emoji = false;
        let mut letters = false;
        for &c in x {
            emoji = emoji || (0xD800..=0xDBFF).contains(&c) || (0x2600..=0x27BF).contains(&c);
            letters = letters || is_lower(c) || is_upper(c) || c == 'ω' as u16 || c == '∀' as u16 || c == '´' as u16 || c == '｀' as u16;
        }
        if emoji { return u16s("絵文字"); }
        if letters || x.len() >= 3 { return u16s("顔文字"); }
        u16s("記号")
    }

    fn add_datetime(&mut self, reading: &[u16], list: &mut Vec<U16>) {
        let Some(hook) = &self.datetime_hook else { return };
        let now = UNIX_EPOCH.elapsed().map(|d| d.as_secs() as i64).unwrap_or(0);
        for d in hook(reading, now) {
            if contains(list, &d) { continue; }
            list.push(d.clone());
            self.date_cands.push(d.clone());
            self.sugg_set.push(d);
        }
    }

    pub fn select_suggestion(&mut self, index: i32) {
        if self.state != State::Input || index < 0 || index as usize >= self.live_sugg.len() { return; }
        self.flush_romaji();
        let idx = index as usize;
        let mut t = self.frozen_surface();
        if self.live_sugg_kinds.get(idx).copied().unwrap_or(0) == 6 { t.extend(&self.live_top); }
        t.extend(&self.live_sugg[idx]);
        let mut reading = self.frozen_reading();
        reading.extend(&self.kana);
        let sugg = self.live_sugg.clone();
        self.log(LogEvent { kind: "commit".into(), reading, surface: t.clone(), ctx: self.ctx.clone(), cands: sugg.clone(), index, extra: "suggest".into(), ..Default::default() });
        if !self.quiet() {
            if let Some(f) = &mut self.flow { let s = sugg[idx].clone(); f.record_suggest(&s); }
        }
        self.commit_text(&t);
        self.to_idle();
        self.rebuild_view();
    }

    pub fn select_candidate(&mut self, index: i32) {
        if self.state == State::Input {
            if self.live_pending || index < 0 || index as usize >= self.live_cands.len() { return; }
            let c = self.live_cands[index as usize].clone();
            self.romaji.reset();
            let kana = self.kana.clone();
            let cands = self.live_cands.clone();
            self.log(LogEvent { kind: "commit".into(), reading: kana.clone(), surface: c.clone(), ctx: self.ctx.clone(), cands, index, segs: vec![c.clone()], extra: "live,mouse".into(), ..Default::default() });
            if !self.quiet() && !contains(&self.sugg_set, &c) && !self.is_completion(&kana.clone(), &c) {
                self.learn_picked_words(&c.clone());
                if self.learning.is_some() && self.conv.is_some() {
                    let ctx = self.conv_ctx();
                    let pw = self.conv.as_ref().unwrap().path_pos(&kana, &c);
                    let lid = head_lid(&pw, 0);
                    let u = self.learning.as_mut().unwrap().record(&kana, &c, 2, &ctx, lid);
                    self.new_learn.push(u);
                }
            }
            if !self.quiet() && contains(&self.sugg_set, &c) {
                if let Some(f) = &mut self.flow { f.record_suggest(&c); }
            }
            let mut t = self.frozen_surface();
            t.extend(&c);
            self.commit_text(&t);
            self.to_idle();
            self.rebuild_view();
            return;
        }
        if self.state != State::Convert { return; }
        let f = self.focus;
        if index < 0 || index as usize >= self.segs[f].cands.len() { return; }
        let old = self.segs[f].sel;
        self.segs[f].sel = index;
        self.segs[f].surface = self.segs[f].cands[index as usize].clone();
        self.segs[f].changed = true;
        self.after_pick(f, old);
        let (rd, sf, cands) = (self.segs[f].reading.clone(), self.segs[f].surface.clone(), self.segs[f].cands.clone());
        let left = self.left_context(f);
        self.log(LogEvent { kind: "bseg".into(), reading: rd, surface: sf, ctx: left, cands, index, extra: "mouse".into(), ..Default::default() });
        if !self.options.always_cands { self.cand_open = false; }
        self.rebuild_view();
    }

    pub fn select_post(&mut self, index: i32) {
        if self.state != State::Idle || index < 0 || index as usize >= self.post.len() { return; }
        let t = self.post[index as usize].clone();
        self.commit_text(&t);
        self.rebuild_view();
    }

    // ---------------------------------------------------------------- 打っている間の変換
    fn after_input(&mut self, typed: bool) {
        self.live_pending = typed && self.options.live_wait && self.options.live_display;
        if !self.live_pending { self.update_live(); }
    }
    pub fn flush_live(&mut self) -> bool {
        if !self.live_pending { return false; }
        self.live_pending = false;
        if self.state == State::Input {
            self.update_live();
            self.rebuild_view();
        }
        true
    }

    fn update_live(&mut self) {
        if !(self.options.live || self.options.live_display) || self.state != State::Input || self.conv.is_none() || self.kana.is_empty() {
            self.live_cands.clear();
            self.live_key.clear();
            return;
        }
        let key = cat(&cat(&self.conv_ctx(), &[1]), &self.kana);
        if key == self.live_key { return; }
        self.sync_user_dict();
        if !self.calibrated { self.calibrate(); }
        // 注: 元の「前の結果の語の使い回し (pre_r)」と句読点での使い回しは未移植 (報告に明記)。
        // 常に全体を変換するので、候補の正しさは変わらない。
        let kana = self.kana.clone();
        let ctx = self.conv_ctx();
        self.prime_keys(&kana);
        let mut c = self.conv.as_mut().unwrap().convert(&ctx, &kana, K_CANDIDATES);
        self.typo_spans = self.conv.as_ref().unwrap().typo_spans();
        self.live_paths = c.paths.clone();
        self.apply_learned_words(&mut c, &ctx);
        self.live_words = c.words.clone();
        let mut key = key;
        if self.options.live_commit && self.caret == self.kana.len() && !self.alpha_run && self.live_commit(&c) {
            if !self.tail_after_freeze(&mut c) {
                self.update_live();
                return;
            }
            self.live_paths = c.paths.clone();
            self.live_words = c.words.clone();
            key = cat(&cat(&self.conv_ctx(), &[1]), &self.kana);
            let fr = self.froze_r as i32;
            self.typo_spans.retain(|sp| sp.0 >= fr);
            for sp in &mut self.typo_spans { sp.0 -= fr; sp.1 -= fr; }
        }
        self.live_key = key;
        self.live_read = self.kana.clone();
        self.sugg_focus = -1;
        self.live_sugg.clear();
        self.live_sugg_kinds.clear();
        self.typo_top.clear();
        self.after_emoji.clear();
        for i in 0..c.sugg.len() {
            let kind = c.sugg_kinds.get(i).copied().unwrap_or(0);
            if kind == 8 { self.typo_top = c.sugg[i].clone(); continue; }
            if kind == 9 { self.after_emoji.push(c.sugg[i].clone()); continue; }
            if !c.cands.is_empty() && c.sugg[i] == c.cands[0] { continue; }
            self.live_sugg.push(c.sugg[i].clone());
            self.live_sugg_kinds.push(kind);
        }
        if self.flow.is_some() && self.live_sugg.len() > 1 {
            let mut ord: Vec<usize> = (0..self.live_sugg.len()).collect();
            let kinds = self.live_sugg_kinds.clone();
            let sugg = self.live_sugg.clone();
            let flow = self.flow.as_ref().unwrap();
            let first_of = |k: i32| kinds.iter().position(|&x| x == k).unwrap_or(usize::MAX);
            ord.sort_by(|&x, &y| {
                let (gx, gy) = (first_of(kinds[x]), first_of(kinds[y]));
                if gx != gy { return gx.cmp(&gy); }
                flow.suggest_use(&sugg[y]).cmp(&flow.suggest_use(&sugg[x]))
            });
            self.live_sugg = ord.iter().map(|&i| sugg[i].clone()).collect();
            self.live_sugg_kinds = ord.iter().map(|&i| kinds[i]).collect();
        }
        let kana_now = self.kana.clone();
        let mut cands = c.cands.clone();
        add_brackets(&kana_now, &mut cands);
        if let Some(l) = &mut self.learning { cands = l.apply_order(&kana_now, &cands); }
        prefer_greetings(&kana_now, &mut cands);
        if self.dict.is_some() && cands.len() > 1 {
            let entries: Vec<(U16, U16, String)> = self.dict.as_ref().unwrap().entries().iter()
                .map(|e| (e.reading.clone(), e.word.clone(), e.pos.clone())).collect();
            for (r, w, pos) in entries {
                if pos == "abbrev" && r == kana_now && cands[0] == w {
                    cands.rotate_left(1);
                    break;
                }
            }
        }
        self.live_top = if cands.is_empty() { kana_now.clone() } else { cands[0].clone() };
        // 予測: 決まり文句は先頭、読みの続く語は 1 位の直後に 3 つまで
        let mut phrase: Option<U16> = None;
        for (r, t, minp) in PREDICTIVE_PHRASES {
            let rv = u16s(r);
            if kana_now.len() >= *minp && kana_now.len() < rv.len() && rv[..kana_now.len()] == kana_now[..] {
                phrase = Some(u16s(t));
                break;
            }
        }
        self.live_pred.clear();
        self.live_phrase = phrase.clone().unwrap_or_default();
        if kana_now.len() >= 2 {
            let mut comp: Vec<U16> = Vec::new();
            if let Some(l) = &mut self.learning { for x in l.completions(&kana_now, 2) { push_unique(&mut comp, &x); } }
            for x in self.conv.as_mut().unwrap().complete(&kana_now, 4, 6) { push_unique(&mut comp, &x); }
            let mut at = 1.min(cands.len());
            let mut inserted = 0;
            for x in comp {
                if inserted >= 3 { break; }
                if contains(&cands, &x) || phrase.as_deref() == Some(&x[..]) { continue; }
                if self.learning.as_mut().map(|l| l.hidden(&kana_now, &x)).unwrap_or(false) { continue; }
                cands.insert(at, x.clone());
                at += 1;
                self.live_pred.push(x);
                inserted += 1;
            }
        }
        if let Some(t) = &phrase {
            cands.retain(|x| x != t);
            cands.insert(0, t.clone());
        }
        if self.dict.is_some() {
            let mut user = self.dict.as_ref().unwrap().lookup(&kana_now);
            if self.learning.is_some() && user.len() > 1 { user = self.learning.as_mut().unwrap().apply_order(&kana_now, &user); }
            for w in user.iter().rev() {
                cands.retain(|x| x != w);
                cands.insert(0, w.clone());
            }
        }
        if cands.is_empty() { cands.push(kana_now.clone()); }
        self.sugg_set.clear();
        self.sugg_typo.clear();
        self.date_cands.clear();
        let mut at = 2usize;
        for si in 0..self.live_sugg.len() {
            let s = self.live_sugg[si].clone();
            if contains(&cands, &s) { continue; }
            if self.live_sugg_kinds[si] == 1 { self.sugg_typo.push(s.clone()); }
            if at > cands.len() { at = cands.len(); }
            cands.insert(at, s.clone());
            at += 2;
            self.sugg_set.push(s);
        }
        self.add_datetime(&kana_now.clone(), &mut cands);
        if self.dict.is_some() && self.raw_valid && !self.raw.is_empty() && self.frozen.is_empty() {
            let k = crate::store::latin_key(&self.raw);
            let mut hit: U16 = Vec::new();
            if k.len() >= 2 { hit = self.dict.as_ref().unwrap().latin_find(&k); }
            if hit.is_empty() && k.len() >= 2 {
                let w = self.conv.as_ref().unwrap().latin_word(&k);
                if camel(&w) { hit = w; }
            }
            if !hit.is_empty() && !contains(&cands, &hit) {
                cands.insert(1.min(cands.len()), hit);
            }
        }
        self.live_cands = cands;
    }

    fn live_commit(&mut self, c: &Conversion) -> bool {
        if c.cands.is_empty() || c.words.len() < 2 {
            self.prev_segs.clear();
            return false;
        }
        let top = c.cands[0].clone();
        let (mut cum_r, mut cum_s) = (0usize, 0usize);
        let (mut freeze_end, mut freeze_len) = (0usize, 0usize);
        let mut segs: HashMap<usize, U16> = HashMap::new();
        for i in 0..c.words.len() - 1 {
            if self.learning.as_mut().map(|l| l.hidden(&c.words[i].reading, &c.words[i].surface)).unwrap_or(false) { break; }
            cum_r += c.words[i].reading.len();
            cum_s += c.words[i].surface.len();
            let surf: U16 = top[..cum_s.min(top.len())].to_vec();
            segs.insert(cum_r, surf.clone());
            let punct = u16s("、。！？!?").contains(&self.kana[cum_r - 1]);
            let stable = self.prev_segs.get(&cum_r).map(|v| *v == surf).unwrap_or(false);
            if cum_r + self.keep_ <= self.kana.len() && (stable || punct) {
                freeze_end = cum_r;
                freeze_len = cum_s;
            }
        }
        self.prev_segs = segs;
        if freeze_end == 0 { return false; }
        self.froze_r = freeze_end;
        self.froze_s = freeze_len;
        let mut fw: Vec<Word> = Vec::new();
        let mut rr = 0usize;
        for w in &c.words {
            if rr >= freeze_end { break; }
            fw.push(w.clone());
            rr += w.reading.len();
        }
        let mut fs: U16 = Vec::new();
        for p in group_phrases(&fw) {
            let mut s = Seg::new(p.reading.clone(), greeting_fix(&p.reading, &p.surface));
            if p.head_r < p.reading.len() && p.head_s < p.surface.len() {
                s.tail_r = p.reading[p.head_r..].to_vec();
                s.tail_s = p.surface[p.head_s..].to_vec();
            }
            fs.extend(&s.surface);
            self.frozen.push(s);
        }
        let cs: Vec<U16> = c.cands[..c.cands.len().min(5)].to_vec();
        let reading: U16 = self.kana[..freeze_end].to_vec();
        let rest: U16 = self.kana[freeze_end..].to_vec();
        self.log(LogEvent { kind: "auto".into(), reading, surface: fs, ctx: self.ctx.clone(), cands: cs, index: -1, extra: format!("hold,{}", from_u16s(&rest)), ..Default::default() });
        self.kana.drain(..freeze_end);
        self.caret -= self.caret.min(freeze_end);
        self.prev_segs.clear();
        self.live_key.clear();
        true
    }

    fn tail_after_freeze(&self, c: &mut Conversion) -> bool {
        if c.cands.is_empty() || self.froze_s > c.cands[0].len() { return false; }
        let head: U16 = c.cands[0][..self.froze_s].to_vec();
        fn strip(v: &mut Vec<U16>, kinds: Option<&mut Vec<i32>>, froze_s: usize, head: &[u16]) {
            let mut out: Vec<U16> = Vec::new();
            let mut ok: Vec<i32> = Vec::new();
            for i in 0..v.len() {
                if v[i].len() <= froze_s || v[i][..froze_s] != *head { continue; }
                out.push(v[i][froze_s..].to_vec());
                if let Some(k) = &kinds { if i < k.len() { ok.push(k[i]); } }
            }
            *v = out;
            if let Some(k) = kinds { *k = ok; }
        }
        strip(&mut c.cands, None, self.froze_s, &head);
        strip(&mut c.sugg, Some(&mut c.sugg_kinds), self.froze_s, &head);
        let mut r = 0usize;
        let mut i = 0usize;
        while i < c.words.len() && r < self.froze_r { r += c.words[i].reading.len(); i += 1; }
        if c.cands.is_empty() || r != self.froze_r { return false; }
        c.words.drain(..i);
        let mut paths: Vec<Vec<TreeWord>> = Vec::new();
        for p in &c.paths {
            let mut q: Vec<TreeWord> = Vec::new();
            let mut cross = false;
            for w in p {
                if w.re <= self.froze_r as i32 { continue; }
                if w.rs < self.froze_r as i32 { cross = true; break; }
                let mut w2 = w.clone();
                w2.rs -= self.froze_r as i32;
                w2.re -= self.froze_r as i32;
                q.push(w2);
            }
            if !cross { paths.push(q); }
        }
        c.paths = paths;
        c.tree.clear();
        true
    }

    fn apply_learned_words(&mut self, c: &mut Conversion, ctx: &[u16]) {
        if self.learning.is_none() || c.cands.is_empty() || c.words.is_empty() { return; }
        let mut out: Vec<Word> = Vec::new();
        let mut left = ctx.to_vec();
        let mut sentence: U16 = Vec::new();
        let mut changed = false;
        let mut i = 0usize;
        for p in group_phrases(&c.words) {
            let first = i;
            let mut r = 0usize;
            while r < p.reading.len() { r += c.words[i].reading.len(); i += 1; }
            let t = self.learning.as_mut().unwrap().top(&p.reading, &left, c.words[first].lid);
            if !t.is_empty() && t != p.surface && self.writable(&p.reading, &t) {
                let mut w = c.words[first].clone();
                w.reading = p.reading.clone();
                w.surface = t.clone();
                w.rid = c.words[i - 1].rid;
                w.pass = false;
                out.push(w);
                changed = true;
            } else {
                out.extend(c.words[first..i].iter().cloned());
            }
            let s = if t.is_empty() { &p.surface } else { &t };
            left.extend(s);
            sentence.extend(s);
        }
        if !changed { return; }
        c.cands.retain(|x| *x != sentence);
        c.cands.insert(0, sentence);
        c.words = out;
    }

    fn calibrate(&mut self) {
        self.calibrated = true;
        if self.conv.is_none() { return; }
        let r = u16s("きょうはとてもいいてんきなのでこうえんまでさんぽにいってみようとおもいます");
        const BUDGET_MS: f64 = 40.0;
        let mut best = f64::INFINITY;
        for i in 0..3 {
            let t0 = Instant::now();
            self.conv.as_mut().unwrap().convert(&[], &r, K_CANDIDATES);
            let ms = t0.elapsed().as_secs_f64() * 1000.0;
            if i > 0 { best = best.min(ms); }
        }
        let n = if best > 0.0 && best.is_finite() { (r.len() as f64 * BUDGET_MS / best) as usize } else { Self::KEEP_MAX };
        self.keep_ = n.clamp(Self::KEEP, Self::KEEP_MAX);
        self.log(LogEvent { kind: "diag".into(), extra: format!("keep={},ms36={:.1}", self.keep_, best), index: -1, ..Default::default() });
    }

    fn frozen_reading(&self) -> U16 {
        let mut r = Vec::new();
        for s in &self.frozen { r.extend(&s.reading); }
        r
    }
    fn frozen_surface(&self) -> U16 {
        let mut r = Vec::new();
        for s in &self.frozen { r.extend(&s.surface); }
        r
    }
    fn conv_ctx(&self) -> U16 { tail(&cat(&self.ctx, &self.frozen_surface()), self.options.ctx_max) }

    fn has_latin_word(&self, raw: &[u16]) -> bool {
        let low: U16 = raw.iter().map(|&c| if is_upper(c) { c + 32 } else { c }).collect();
        for i in 0..low.len() {
            for n in 4..=(low.len() - i).min(24) {
                let k: U16 = low[i..i + n].to_vec();
                if k.iter().any(|&c| !(is_lower(c) || is_digit(c))) { break; }
                if self.dict.as_ref().map(|d| !d.latin_find(&k).is_empty()).unwrap_or(false) { return true; }
                let w = self.conv.as_ref().map(|c| c.latin_word(&k)).unwrap_or_default();
                if camel(&w) { return true; }
            }
        }
        false
    }

    fn prime_keys(&mut self, reading: &[u16]) {
        // 注: prime_alts (キー配列の打ち直し) は Layout 依存のため未移植 (報告に明記)。
        if self.conv.is_none() || !self.raw_valid || self.raw.is_empty() || reading.is_empty() { return; }
        let ext = self.romaji.extended();
        let has_alpha = reading.iter().any(|&ch| ch < 0x80 && is_lower(ch | 0x20));
        if !has_alpha && (ext || (!self.has_latin_word(&self.raw.clone()) && !self.raw.contains(&(b'l' as u16)) && !self.raw.contains(&(b'x' as u16)))) {
            return;
        }
        let (sim, span) = self.simulate_raw();
        if sim.len() < reading.len() || sim[sim.len() - reading.len()..] != *reading { return; }
        let off = sim.len() - reading.len();
        let ks = if off == 0 { 0 } else { span[off - 1] };
        let mut sp: Vec<i32> = span[off..].to_vec();
        for x in &mut sp { *x -= ks; }
        let keys: U16 = self.raw[ks as usize..].to_vec();
        self.conv.as_mut().unwrap().set_keys(&keys, &sp, reading.len());
    }

    fn simulate_raw(&self) -> (U16, Vec<i32>) {
        let mut rom = self.romaji.clone();
        rom.reset();
        let mut sim: U16 = Vec::new();
        let mut span: Vec<i32> = Vec::new();
        let mut k = 0i32;
        for &key in &self.raw {
            let lk = if is_upper(key) { key + 32 } else { key };
            let romaji_key = is_lower(lk) || lk == b'-' as u16 || lk == b'\'' as u16 || rom.is_prefix(&[lk]);
            if !romaji_key {
                let f = rom.flush();
                for ch in f { sim.push(ch); span.push(k); }
                k += 1;
                sim.push(key);
                span.push(k);
                continue;
            }
            k += 1;
            let r = rom.process(lk);
            let end = if rom.pending().is_empty() { k } else { k - 1 };
            for ch in r.committed { sim.push(ch); span.push(end); }
        }
        let f = rom.flush();
        for ch in f { sim.push(ch); span.push(k); }
        (sim, span)
    }

    fn replay(keys: &[u16], romaji: &SimpleRomaji) -> (U16, SimpleRomaji) {
        let mut rom = romaji.clone();
        rom.reset();
        let mut kana: U16 = Vec::new();
        for &key in keys {
            let lk = if is_upper(key) { key + 32 } else { key };
            if is_lower(lk) || lk == b'-' as u16 || lk == b'\'' as u16 || rom.is_prefix(&[lk]) {
                if key != lk {
                    let f = rom.flush();
                    kana.extend(f);
                    kana.push(key);
                    continue;
                }
                let r = rom.process(lk);
                kana.extend(r.committed);
            } else {
                let f = rom.flush();
                kana.extend(f);
                kana.push(key);
            }
        }
        (kana, rom)
    }

    fn undo_last_key(&mut self) -> bool {
        if !self.raw_valid || self.raw.is_empty() { return false; }
        let fr = self.frozen_reading();
        let (all, rom) = Self::replay(&self.raw.clone(), &self.romaji);
        if all != cat(&fr, &self.kana) || rom.pending() != self.romaji.pending() { return false; }
        let shorter_keys: U16 = self.raw[..self.raw.len() - 1].to_vec();
        let (shorter, rom2) = Self::replay(&shorter_keys, &self.romaji);
        if shorter.len() < fr.len() || shorter[..fr.len()] != fr[..] { return false; }
        self.raw.pop();
        self.kana = shorter[fr.len()..].to_vec();
        self.caret = self.kana.len();
        self.romaji = rom2;
        true
    }

    fn delete_last_char(&mut self) -> bool {
        if !self.raw_valid || self.raw.is_empty() || self.kana.is_empty() { return false; }
        let fr = self.frozen_reading();
        let (all, rom) = Self::replay(&self.raw.clone(), &self.romaji);
        if all != cat(&fr, &self.kana) || !rom.pending().is_empty() { return false; }
        let target: U16 = all[..all.len() - 1].to_vec();
        for n in (0..self.raw.len()).rev() {
            let head_keys: U16 = self.raw[..n].to_vec();
            let (head, r) = Self::replay(&head_keys, &self.romaji);
            if !r.pending().is_empty() || head.len() > target.len() || target[..head.len()] != head[..] { continue; }
            let mut rest = r.to_romaji(&target[head.len()..]);
            for extra in 0..2 {
                if extra == 1 { rest.push(b'n' as u16); }
                let keys = cat(&head_keys, &rest);
                let (got, r2) = Self::replay(&keys, &self.romaji);
                if got != target || !r2.pending().is_empty() || got.len() < fr.len() { continue; }
                self.raw = keys;
                self.kana = target[fr.len()..].to_vec();
                self.caret = self.kana.len();
                self.romaji = r2;
                return true;
            }
            return false;
        }
        false
    }

    fn unfreeze_last(&mut self) -> bool {
        let Some(last) = self.frozen.pop() else { return false };
        let r = last.reading;
        self.kana.splice(0..0, r.iter().copied());
        self.caret += r.len();
        self.live_key.clear();
        self.prev_segs.clear();
        true
    }

    // ---------------------------------------------------------------- 確定
    fn is_completion(&self, reading: &[u16], surface: &[u16]) -> bool {
        reading == self.kana && (contains(&self.live_pred, surface) || (!self.live_phrase.is_empty() && surface == self.live_phrase))
    }
    fn confirm_learned(&mut self, reading: &[u16], surface: &[u16], left: &[u16], lid: i32) {
        if self.learning.is_some() && !self.quiet() && !contains(&self.sugg_set, surface) && !self.is_completion(reading, surface) {
            let u = self.learning.as_mut().unwrap().confirm(reading, surface, left, lid);
            self.new_learn.push(u);
        }
    }
    fn confirm_frozen(&mut self) -> U16 {
        let mut left = self.ctx.clone();
        let frozen = self.frozen.clone();
        for s in &frozen {
            self.confirm_learned(&s.reading.clone(), &s.surface.clone(), &left.clone(), -1);
            left.extend(&s.surface);
        }
        left
    }
    fn confirm_live(&mut self, top: &[u16]) {
        let left0 = self.confirm_frozen();
        let mut left = left0;
        self.confirm_learned(&self.kana.clone(), top, &left.clone(), -1);
        let mut joined: U16 = Vec::new();
        for w in &self.live_words { joined.extend(&w.surface); }
        if joined != top { return; }
        let ph = group_phrases(&self.live_words);
        if ph.len() < 2 { return; }
        let words = self.live_words.clone();
        let mut r0 = 0usize;
        for p in ph {
            let lid = head_lid(&words, r0);
            self.confirm_learned(&p.reading.clone(), &p.surface.clone(), &left.clone(), lid);
            left.extend(&p.surface);
            r0 += p.reading.len();
        }
    }
    fn learn_picked_words(&mut self, picked: &[u16]) {
        if self.learning.is_none() || self.conv.is_none() { return; }
        for path in self.live_paths.clone() {
            let mut joined: U16 = Vec::new();
            for w in &path { joined.extend(&w.surface); }
            if joined != picked { continue; }
            let pw = self.conv.as_ref().unwrap().path_pos(&self.kana, picked);
            let (mut a, mut b) = (0usize, 0usize);
            let (mut ea, mut start) = (0usize, 0usize);
            let mut eb = 0i32;
            let (mut sa, mut sb): (U16, U16) = (Vec::new(), Vec::new());
            let mut left = self.conv_ctx();
            while a < self.live_words.len() && b < path.len() {
                ea += self.live_words[a].reading.len();
                sa.extend(&self.live_words[a].surface);
                a += 1;
                eb = path[b].re;
                sb.extend(&path[b].surface);
                b += 1;
                while ea != eb as usize {
                    if ea < eb as usize {
                        if a >= self.live_words.len() { return; }
                        ea += self.live_words[a].reading.len();
                        sa.extend(&self.live_words[a].surface);
                        a += 1;
                    } else {
                        if b >= path.len() { return; }
                        eb = path[b].re;
                        sb.extend(&path[b].surface);
                        b += 1;
                    }
                }
                if sa != sb && ea - start < self.kana.len() {
                    let sub: U16 = self.kana[start..ea].to_vec();
                    let lid = head_lid(&pw, start);
                    let u = self.learning.as_mut().unwrap().record(&sub, &sb, 2, &left, lid);
                    self.new_learn.push(u);
                }
                left.extend(&sb);
                start = ea;
                sa.clear();
                sb.clear();
            }
            return;
        }
    }

    fn commit_shown(&mut self) {
        if self.options.live_display || self.options.space_literal { self.update_live(); }
        if self.live_shown() {
            let t = self.live_top.clone();
            let kana = self.kana.clone();
            let cands = self.live_cands.clone();
            self.log(LogEvent { kind: "commit".into(), reading: kana, surface: t.clone(), ctx: self.ctx.clone(), cands, index: 0, extra: "live".into(), ..Default::default() });
            self.confirm_live(&t.clone());
            let mut full = self.frozen_surface();
            full.extend(&t);
            self.commit_text(&full);
            self.to_idle();
            return;
        }
        let kana = self.kana.clone();
        self.log(LogEvent { kind: "raw".into(), reading: kana.clone(), surface: kana.clone(), ctx: self.ctx.clone(), index: -1, ..Default::default() });
        self.confirm_frozen();
        let ctx = self.conv_ctx();
        self.confirm_learned(&kana.clone(), &kana.clone(), &ctx, -1);
        let mut full = self.frozen_surface();
        full.extend(&kana);
        self.commit_text(&full);
        self.to_idle();
    }

    fn commit_at_punct(&mut self) {
        if self.state != State::Input || self.kana.is_empty() || self.caret != self.kana.len() || !self.romaji.pending().is_empty() { return; }
        if !u16s("、。，．！？!?").contains(self.kana.last().unwrap()) { return; }
        if self.options.live_display || self.options.space_literal { self.update_live(); }
        if !self.live_shown() {
            self.convert_all();
            if self.state == State::Convert { self.commit_all(); return; }
        }
        self.commit_shown();
    }

    fn commit_all(&mut self) {
        let mut t: U16 = Vec::new();
        let mut reading: U16 = Vec::new();
        let mut seg_surfaces: Vec<U16> = Vec::new();
        let mut changed = 0;
        let pw: Vec<Word> = if self.learning.is_some() && !self.quiet() && self.conv.is_some() {
            let (mut fr, mut ft): (U16, U16) = (Vec::new(), Vec::new());
            for s in &self.segs { fr.extend(&s.reading); ft.extend(&s.surface); }
            self.conv.as_ref().unwrap().path_pos(&fr, &ft)
        } else { Vec::new() };
        let segs = self.segs.clone();
        for s in &segs {
            let left = cat(&self.ctx, &t);
            let lid = if pw.is_empty() { -1 } else { head_lid(&pw, reading.len()) };
            t.extend(&s.surface);
            reading.extend(&s.reading);
            seg_surfaces.push(s.surface.clone());
            if s.changed {
                changed += 1;
                if !self.quiet() && !contains(&self.sugg_set, &s.surface) && !self.is_completion(&s.reading, &s.surface) {
                    if let Some(l) = &mut self.learning {
                        let u = l.record(&s.reading, &s.surface, 2, &left, lid);
                        self.new_learn.push(u);
                    }
                    if contains(&self.sugg_set, &s.surface) {
                        if let Some(f) = &mut self.flow { f.record_suggest(&s.surface); }
                    }
                }
                self.log(LogEvent { kind: "bseg".into(), reading: s.reading.clone(), surface: s.surface.clone(), cands: s.cands.clone(), index: s.sel, ..Default::default() });
            } else {
                self.confirm_learned(&s.reading.clone(), &s.surface.clone(), &left, lid);
            }
        }
        if self.on_log.is_some() {
            let idx = self.first_cands.iter().position(|c| *c == t).map(|p| p as i32).unwrap_or(-1);
            self.log(LogEvent {
                kind: "commit".into(), reading: reading.clone(), surface: t.clone(), ctx: self.ctx.clone(),
                cands: self.first_cands.clone(), index: idx, segs: seg_surfaces,
                extra: format!("{},changed={}", self.first_via, changed), ..Default::default()
            });
        }
        self.kana = reading;
        self.commit_text(&t);
        self.to_idle();
    }

    fn commit_text(&mut self, t0: &[u16]) {
        let t = if self.options.space_literal { settle_spaces(t0.to_vec()) } else { t0.to_vec() };
        self.commit.extend(&t);
        let mut full_reading = self.frozen_reading();
        full_reading.extend(&self.kana);
        if !t.is_empty() && !self.kana.is_empty() && t != self.kana {
            self.last_commit = (t.clone(), full_reading.clone());
        } else {
            self.last_commit = (Vec::new(), Vec::new());
        }
        self.last_learn = std::mem::take(&mut self.new_learn);
        self.last_style.clear();
        {
            let now = Instant::now();
            let reading = if self.kana.is_empty() { Vec::new() } else { full_reading.clone() };
            if self.learning.is_some() && !self.quiet() && self.retype.erased
                && now.duration_since(self.retype.at).as_secs() <= 10 && !t.is_empty()
                && reading == self.retype.reading && t != self.retype.surface {
                let (rr, rs, rl) = (self.retype.reading.clone(), self.retype.surface.clone(), self.retype.left.clone());
                let u = self.learning.as_mut().unwrap().demote(&rr, &rs, &rl, 2);
                if !u.reading.is_empty() { self.last_learn.push(u); }
            }
            self.retype = Retype::default();
            if !t.is_empty() && !reading.is_empty() {
                let chars = t.iter().filter(|&&c| (c & 0xFC00) != 0xDC00).count() as i32;
                self.retype = Retype { reading, surface: t.clone(), left: self.ctx.clone(), chars, erased: false, at: now };
            }
        }
        self.history = tail(&cat(&self.history, &t), self.options.ctx_max);
        if self.style_log.is_some() && self.conv.is_some() && !self.quiet() && !t.is_empty() && !self.kana.is_empty() {
            let w8 = self.conv.as_ref().unwrap().style_weight(&self.ctx);
            let vars = self.conv.as_ref().unwrap().path_variants(&full_reading, &t);
            for w in vars {
                self.style_log.as_mut().unwrap().record(&w, w8);
                self.last_style.push((w, w8));
            }
        }
        if self.learning.is_some() && self.options.learn_log && !self.quiet() && !t.is_empty() && !self.kana.is_empty() {
            let ctx = self.ctx.clone();
            self.learning.as_mut().unwrap().note_commit(&full_reading, &t, &ctx);
        }
        if self.conv.is_some() && !self.quiet() && !t.is_empty() && !self.kana.is_empty() && t != self.kana {
            let ctx = self.ctx.clone();
            self.conv.as_mut().unwrap().ai_learn(&ctx, &full_reading, &t);
        }
        if !t.is_empty() && !self.kana.is_empty() && !self.quiet() {
            self.recent.push((t.clone(), full_reading.clone()));
            if self.recent.len() > 200 { self.recent.remove(0); }
        }
        if self.flow.is_some() && !self.quiet() {
            let f = self.flow.as_mut().unwrap();
            f.feed(&t);
            self.post = f.predict();
            f.save();
        }
        self.after_emoji.clear();
    }

    // ---------------------------------------------------------------- 再変換・取り消し
    pub fn reading_of(&self, text: &[u16]) -> U16 {
        if text.is_empty() { return Vec::new(); }
        let kana_only = text.iter().all(|&c| is_hira(c) || (0x309D..=0x309E).contains(&c) || (0x30A1..=0x30FE).contains(&c));
        if kana_only { return to_hiragana(text); }
        for (s, r) in self.recent.iter().rev() { if s == text { return r.clone(); } }
        Vec::new()
    }
    pub fn start_reconvert(&mut self, reading: &[u16], original: &[u16]) -> bool {
        if reading.is_empty() || self.state != State::Idle { return false; }
        self.start_input();
        self.insert(reading);
        self.raw_valid = false;
        self.convert_all();
        self.restore = original.to_vec();
        self.rebuild_view();
        true
    }
    fn cancel_reconvert(&mut self) -> bool {
        if self.restore.is_empty() { return false; }
        let r = std::mem::take(&mut self.restore);
        self.commit.extend(&r);
        self.history = tail(&cat(&self.history, &r), self.options.ctx_max);
        self.to_idle();
        true
    }
    pub fn undo_commit(&mut self) -> bool {
        if self.last_commit.0.is_empty() || self.state != State::Idle { return false; }
        let (surface, reading) = self.last_commit.clone();
        self.last_commit = (Vec::new(), Vec::new());
        self.retype = Retype::default();
        if self.history.len() >= surface.len() && self.history[self.history.len() - surface.len()..] == surface[..] {
            self.history.truncate(self.history.len() - surface.len());
        }
        let hist = self.history.clone();
        self.log(LogEvent { kind: "undo".into(), reading: reading.clone(), surface: surface.clone(), ctx: hist, index: -1, ..Default::default() });
        let mut us = std::mem::take(&mut self.last_learn);
        us.reverse();
        if let Some(l) = &mut self.learning { l.restore(&us); }
        if let Some(s) = &mut self.style_log { let items = std::mem::take(&mut self.last_style); s.unrecord(&items); }
        self.last_style.clear();
        if self.recent.last().map(|x| x.0 == surface).unwrap_or(false) { self.recent.pop(); }
        if let Some(f) = &mut self.flow { for _ in 0..surface.len() { f.backspace(); } }
        self.start_reconvert(&reading, &surface)
    }
    pub fn romaji_reading(&self, text: &[u16]) -> U16 {
        let mut rom = self.romaji.clone();
        rom.reset();
        let mut s: U16 = Vec::new();
        for &c in text {
            if c == b' ' as u16 || c == b'\t' as u16 { continue; }
            let lk = if is_upper(c) { c + 32 } else { c };
            if !is_lower(lk) && lk != b'-' as u16 && lk != b'\'' as u16 { return Vec::new(); }
            s.push(lk);
        }
        let mut out: U16 = Vec::new();
        for i in 0..s.len() {
            out.extend(rom.process(s[i]).committed);
            if s[i] == b'n' as u16 && i + 2 < s.len() && s[i + 1] == b'n' as u16 && u16s("aiueoy").contains(&s[i + 2]) {
                out.extend(rom.process(b'\'' as u16).committed);
            }
        }
        out.extend(rom.flush());
        if out.iter().any(|&c| is_lower(c)) { return Vec::new(); }
        out
    }

    // ---------------------------------------------------------------- 終了・見せ方
    fn to_idle(&mut self) {
        self.state = State::Idle;
        self.restore.clear();
        self.pred_tab.clear();
        self.kana.clear();
        self.caret = 0;
        self.romaji.reset();
        self.segs.clear();
        self.whole.clear();
        self.first_cands.clear();
        self.first_via.clear();
        self.cand_open = false;
        self.alpha_run = false;
        self.live_key.clear();
        self.live_cands.clear();
        self.live_read.clear();
        self.live_pending = false;
        self.live_sugg.clear();
        self.sugg_focus = -1;
        self.live_sugg_kinds.clear();
        self.prev_segs.clear();
        self.frozen.clear();
        self.date_cands.clear();
    }
    pub fn finish(&mut self) {
        if self.state == State::Input {
            self.flush_romaji();
            if (self.options.live_display || self.options.space_literal) && !self.kana.is_empty() { self.update_live(); }
            if self.live_shown() {
                let t = self.live_top.clone();
                let kana = self.kana.clone();
                let cands = self.live_cands.clone();
                self.log(LogEvent { kind: "commit".into(), reading: kana, surface: t.clone(), ctx: self.ctx.clone(), cands, index: 0, extra: "live".into(), ..Default::default() });
                self.confirm_live(&t.clone());
                let mut full = self.frozen_surface();
                full.extend(&t);
                self.commit_text(&full);
            } else {
                self.confirm_frozen();
                let ctx = self.conv_ctx();
                let kana = self.kana.clone();
                self.confirm_learned(&kana.clone(), &kana, &ctx, -1);
                let mut full = self.frozen_surface();
                full.extend(&kana);
                self.commit_text(&full);
            }
            self.to_idle();
        } else if self.state == State::Convert {
            self.commit_all();
        }
        self.rebuild_view();
    }
    pub fn cancel(&mut self) {
        self.to_idle();
        self.rebuild_view();
    }
    fn live_shown(&self) -> bool {
        (self.options.live_display || self.options.space_literal) && self.state == State::Input && !self.kana.is_empty()
            && self.caret == self.kana.len() && !self.alpha_run && !self.live_top.is_empty()
            && self.live_key == cat(&cat(&self.conv_ctx(), &[1]), &self.kana)
    }
    fn live_stale(&self) -> bool {
        self.live_pending && self.options.live_display && self.state == State::Input && self.caret == self.kana.len()
            && !self.alpha_run && !self.live_top.is_empty() && !self.live_read.is_empty()
            && self.kana.len() >= self.live_read.len() && self.kana[..self.live_read.len()] == self.live_read[..]
    }

    fn rebuild_view(&mut self) {
        let mut v = View::new();
        v.sugg_sel = self.sugg_focus;
        if self.state == State::Idle { v.post = self.post.clone(); }
        if self.state == State::Input {
            let p = self.romaji.pending();
            let fz = self.frozen_surface();
            let f = fz.len() as i32;
            if f > 0 { v.spans.push(Span { start: 0, len: f, kind: SpanKind::Converted }); }
            if self.live_shown() {
                let top = if self.options.space_literal { settle_spaces(self.live_top.clone()) } else { self.live_top.clone() };
                v.text = cat(&cat(&fz, &top), &p);
                v.caret = v.text.len() as i32;
                if !self.live_top.is_empty() {
                    let n = self.live_top.len() as i32;
                    let (mut a, mut b) = (0i32, n);
                    let mut corrected = false;
                    if !self.typo_top.is_empty() && self.live_top == self.typo_top {
                        let mut lit: U16 = Vec::new();
                        for x in &self.live_cands { if *x != self.typo_top { lit = x.clone(); break; } }
                        if !lit.is_empty() {
                            while a < n && (a as usize) < lit.len() && self.live_top[a as usize] == lit[a as usize] { a += 1; }
                            let mut q = 0;
                            while q < n - a && (q as usize) < lit.len() - a as usize
                                && self.live_top[(n - 1 - q) as usize] == lit[lit.len() - 1 - q as usize] { q += 1; }
                            b = n - q;
                            if a >= b { a = 0; b = n; }
                            corrected = true;
                        }
                    }
                    if corrected {
                        if a > 0 { v.spans.push(Span { start: f, len: a, kind: SpanKind::Converted }); }
                        v.spans.push(Span { start: f + a, len: b - a, kind: SpanKind::Corrected });
                        if b < n { v.spans.push(Span { start: f + b, len: n - b, kind: SpanKind::Converted }); }
                    } else {
                        v.spans.push(Span { start: f, len: n, kind: SpanKind::Converted });
                    }
                }
                if !p.is_empty() { v.spans.push(Span { start: f + self.live_top.len() as i32, len: p.len() as i32, kind: SpanKind::Input }); }
            } else if self.live_stale() {
                let top = if self.options.space_literal { settle_spaces(self.live_top.clone()) } else { self.live_top.clone() };
                let mut rest: U16 = self.kana[self.live_read.len()..].to_vec();
                rest.extend(&p);
                v.text = cat(&cat(&fz, &top), &rest);
                v.caret = v.text.len() as i32;
                v.spans.push(Span { start: f, len: top.len() as i32, kind: SpanKind::Converted });
                if !rest.is_empty() { v.spans.push(Span { start: f + top.len() as i32, len: rest.len() as i32, kind: SpanKind::Input }); }
            } else {
                let mut inp: U16 = self.kana[..self.caret].to_vec();
                inp.extend(&p);
                inp.extend(&self.kana[self.caret..]);
                v.text = cat(&fz, &inp);
                v.caret = f + (self.caret + p.len()) as i32;
                if !inp.is_empty() { v.spans.push(Span { start: f, len: inp.len() as i32, kind: SpanKind::Input }); }
            }
            if !self.live_cands.is_empty() && self.options.live {
                v.cand_open = true;
                v.cands = self.live_cands.clone();
                v.cand_notes = self.live_cands.iter().map(|x| self.sugg_note(x)).collect();
                v.cand_sel = -1;
            }
            // 提案の行 (打っている間だけ)
            v.sugg = self.live_sugg.clone();
            v.sugg_text = self.live_sugg.iter().map(|s| cat(&self.frozen_surface(), s)).collect();
            v.sugg_kinds = self.live_sugg_kinds.clone();
        } else if self.state == State::Convert {
            for i in 0..self.segs.len() {
                let st = v.text.len() as i32;
                v.text.extend(&self.segs[i].surface);
                let len = self.segs[i].surface.len() as i32;
                if len > 0 {
                    v.spans.push(Span { start: st, len, kind: if i == self.focus { SpanKind::Focused } else { SpanKind::Converted } });
                }
                if i == self.focus { v.anchor = st; v.caret = st + len; }
            }
            if self.cand_open && !self.segs.is_empty() {
                v.cand_open = true;
                v.cands = self.segs[self.focus].cands.clone();
                if self.segs.len() == 1 {
                    v.cand_notes = v.cands.iter().map(|x| self.sugg_note(x)).collect();
                }
                v.cand_sel = self.segs[self.focus].sel;
                v.cand_short = self.segs[self.focus].tree_k;
                v.cand_scores = self.segs[self.focus].scores.clone();
                v.cand_scores.resize(v.cands.len(), -1.0);
                v.cand_fork = self.segs[self.focus].fork_idx;
            }
            if self.options.cand_tree && self.tree.is_empty() {
                let mut r = 0i32;
                for sg in &self.segs {
                    let mut n = TreeNode::new();
                    let mut w = TreeWord::default();
                    w.surface = sg.surface.clone();
                    w.rs = r;
                    r += sg.reading.len() as i32;
                    w.re = r;
                    w.s = -1.0;
                    n.main.push(w);
                    v.tree.push(n);
                }
                let mut fpos = 0i32;
                for i in 0..self.focus { fpos += self.segs[i].reading.len() as i32; }
                v.tree_rs = fpos;
                v.tree_re = fpos + self.segs[self.focus].reading.len() as i32;
            }
            if self.options.cand_tree && !self.tree.is_empty() {
                v.tree = self.tree.clone();
                let mut pre: Vec<TreeNode> = Vec::new();
                let mut q = -self.tree_off;
                for sg in &self.segs {
                    if q >= 0 { break; }
                    let mut n = TreeNode::new();
                    let mut w = TreeWord::default();
                    w.surface = sg.surface.clone();
                    w.rs = q;
                    q += sg.reading.len() as i32;
                    w.re = q;
                    w.s = -1.0;
                    n.main.push(w);
                    pre.push(n);
                }
                for (idx, n) in pre.into_iter().enumerate() { v.tree.insert(idx, n); }
                let mut r = -self.tree_off;
                for i in 0..self.segs.len() {
                    let rs = r;
                    let re = r + self.segs[i].reading.len() as i32;
                    r = re;
                    if i == self.focus { v.tree_rs = rs; v.tree_re = re; }
                    if !self.segs[i].own_reading.is_empty() { continue; }
                    for n in &mut v.tree {
                        for w in &mut n.main {
                            if w.rs == rs && w.re == re && w.surface != self.segs[i].surface {
                                let pos = w.alts.iter().position(|a| a.0 == self.segs[i].surface);
                                let s = pos.map(|p| w.alts[p].1).unwrap_or(0.0);
                                if let Some(p) = pos { w.alts.remove(p); }
                                let old_surf = std::mem::replace(&mut w.surface, self.segs[i].surface.clone());
                                let old_s = std::mem::replace(&mut w.s, s);
                                w.alts.insert(0, (old_surf, old_s));
                            }
                        }
                    }
                }
            }
        }
        if self.state != State::Idle && (!self.kana.is_empty() || !self.romaji.pending().is_empty()) {
            let pending = self.romaji.pending();
            v.typed = cat(&self.kana, &pending);
            v.typed_fixed = self.live_shown() && !self.typo_top.is_empty() && self.live_top == self.typo_top;
            let mut off = 0usize;
            if self.raw_valid && !self.raw.is_empty() {
                let (sim, _span) = self.simulate_raw();
                let mut rom2 = self.romaji.clone();
                let tailv = cat(&self.kana, &rom2.flush());
                if sim.len() >= tailv.len() && sim[sim.len() - tailv.len()..] == tailv[..] {
                    off = sim.len() - tailv.len();
                    let mut nt: U16 = sim[..off].to_vec();
                    nt.extend(&v.typed);
                    v.typed = nt;
                }
            }
            if v.typed_fixed && !self.typo_spans.is_empty() {
                v.typed_mark = vec![false; v.typed.len()];
                for (a, b) in self.typo_spans.clone() {
                    if a < 0 || b <= a || b as usize > self.kana.len() { continue; }
                    for k in off + a as usize..off + b as usize {
                        if k < v.typed_mark.len() { v.typed_mark[k] = true; }
                    }
                }
            }
        }
        self.view_ = v;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use crate::converter::{Conversion, Word};

    #[derive(Default)]
    struct FakeConv {
        map: HashMap<String, Vec<String>>,
    }
    impl FakeConv {
        fn new() -> Self {
            let mut map = HashMap::new();
            map.insert("ねこ".to_string(), vec!["猫".to_string(), "ねこ".to_string(), "ネコ".to_string()]);
            map.insert("きょう".to_string(), vec!["今日".to_string(), "京".to_string()]);
            map.insert("は".to_string(), vec!["は".to_string(), "葉".to_string()]);
            map.insert("きょうは".to_string(), vec!["今日は".to_string()]);
            FakeConv { map }
        }
    }
    impl Convert for FakeConv {
        fn convert(&mut self, _ctx: &[u16], kana: &[u16], _max: i32) -> Conversion {
            let k = from_u16s(kana);
            let cands: Vec<U16> = self.map.get(&k).cloned().unwrap_or_else(|| vec![k.clone()]).iter().map(|s| u16s(s)).collect();
            // きょうは は 2 語に分ける
            let words = if k == "きょうは" {
                vec![
                    Word { reading: u16s("きょう"), surface: u16s("今日"), lid: 100, rid: 100, pass: false },
                    Word { reading: u16s("は"), surface: u16s("は"), lid: 300, rid: 300, pass: false },
                ]
            } else {
                vec![Word { reading: kana.to_vec(), surface: cands[0].clone(), lid: 100, rid: 100, pass: false }]
            };
            Conversion { cands, words, ..Default::default() }
        }
        fn set_right_ctx(&mut self, _t: &[u16]) {}
        fn set_keys(&mut self, _k: &[u16], _s: &[i32], _n: usize) {}
        fn set_alt_readings(&mut self, _a: &[U16], _n: usize) {}
        fn has_right_ctx(&self) -> bool { false }
        fn set_suggest(&mut self, _o: bool) {}
        fn set_typo(&mut self, _o: bool, _b: f32) {}
        fn set_latin(&mut self, _o: bool) {}
        fn personal(&self) -> bool { false }
        fn set_variants(&mut self, _t: &str) {}
        fn set_marks(&mut self, _m: &[(U16, i32)]) {}
        fn set_style_counts(&mut self, _c: &HashMap<U16, StyleCount>) {}
        fn set_restyle(&mut self, _o: bool) {}
        fn style_weight(&self, _c: &[u16]) -> [f32; 2] { [0.5, 0.5] }
        fn typo_spans(&self) -> Vec<(i32, i32)> { Vec::new() }
        fn path_variants(&self, _r: &[u16], _t: &[u16]) -> Vec<U16> { Vec::new() }
        fn path_pos(&self, r: &[u16], t: &[u16]) -> Vec<Word> {
            vec![Word { reading: r.to_vec(), surface: t.to_vec(), lid: 100, rid: 100, pass: false }]
        }
        fn latin_word(&self, _k: &[u16]) -> U16 { Vec::new() }
        fn complete(&mut self, _p: &[u16], _m: i32, _e: i32) -> Vec<U16> { Vec::new() }
        fn set_user_words(&mut self, _f: &[DictForm], _b: i32) -> i32 { 0 }
        fn ai_learn(&mut self, _c: &[u16], _r: &[u16], _t: &[u16]) -> f32 { -1.0 }
    }

    fn key(k: Key) -> KeyEvent { KeyEvent { key: k, ..Default::default() } }
    fn chr(c: char) -> KeyEvent { KeyEvent { key: Key::Char, ch: c as u16, ..Default::default() } }
    fn type_str(c: &mut Composer, s: &str) { for ch in s.chars() { c.press(&chr(ch)); } }

    fn composer() -> Composer {
        Composer::new(Some(Box::new(FakeConv::new())), None, None)
    }

    #[test]
    fn romaji_basic() {
        let mut r = SimpleRomaji::new();
        let mut out = Vec::new();
        for ch in "neko".chars() { out.extend(r.process(ch as u16).committed); }
        out.extend(r.flush());
        assert_eq!(out, u16s("ねこ"));
        let mut r = SimpleRomaji::new();
        let mut out = Vec::new();
        for ch in "kyouha".chars() { out.extend(r.process(ch as u16).committed); }
        out.extend(r.flush());
        assert_eq!(out, u16s("きょうは"));
        let mut r = SimpleRomaji::new();
        let mut out = Vec::new();
        for ch in "gakkou".chars() { out.extend(r.process(ch as u16).committed); }
        out.extend(r.flush());
        assert_eq!(out, u16s("がっこう"));
        let mut r = SimpleRomaji::new();
        let mut out = Vec::new();
        for ch in "kanji".chars() { out.extend(r.process(ch as u16).committed); }
        out.extend(r.flush());
        assert_eq!(out, u16s("かんじ"));
        assert_eq!(r.to_romaji(&u16s("ねこ")), u16s("neko"));
    }

    #[test]
    fn type_convert_commit() {
        let mut c = composer();
        c.options.live = false;
        type_str(&mut c, "neko");
        assert!(c.composing());
        assert_eq!(c.view().text, u16s("ねこ"));
        c.press(&key(Key::Space));
        assert!(c.converting());
        assert_eq!(c.view().text, u16s("猫"));
        // 候補を下げる
        c.press(&key(Key::Space));
        assert_eq!(c.view().text, u16s("ねこ"));
        c.press(&key(Key::Up));
        assert_eq!(c.view().text, u16s("猫"));
        c.press(&key(Key::Enter));
        assert!(!c.composing());
        assert_eq!(c.take_commit(), u16s("猫"));
    }

    #[test]
    fn phrases_and_focus() {
        let mut c = composer();
        c.options.live = false;
        type_str(&mut c, "kyouha");
        c.press(&key(Key::Space));
        assert!(c.converting());
        assert_eq!(c.view().text, u16s("今日は"));
        // 文節は 1 つ (は が付属語として付く) のはず
        c.press(&key(Key::Enter));
        assert_eq!(c.take_commit(), u16s("今日は"));
    }

    #[test]
    fn backspace_and_cancel() {
        let mut c = composer();
        c.options.live = false;
        type_str(&mut c, "neko");
        c.press(&key(Key::Backspace));
        assert_eq!(c.view().text, u16s("ね"));
        c.press(&key(Key::Escape));
        assert!(!c.composing());
        assert_eq!(c.take_commit(), Vec::<u16>::new());
    }

    #[test]
    fn fkeys_transform() {
        let mut c = composer();
        c.options.live = false;
        type_str(&mut c, "neko");
        c.press(&key(Key::Space));
        c.press(&key(Key::F7));
        assert_eq!(c.view().text, u16s("ネコ"));
        c.press(&key(Key::F6));
        assert_eq!(c.view().text, u16s("ねこ"));
        c.press(&key(Key::F9));
        assert_eq!(c.view().text, u16s("ｎｅｋｏ"));
        c.press(&key(Key::F10));
        assert_eq!(c.view().text, u16s("neko"));
        c.press(&key(Key::F10));
        assert_eq!(c.view().text, u16s("NEKO"));
    }

    #[test]
    fn live_candidates_and_enter() {
        let mut c = composer();
        type_str(&mut c, "neko");
        // ライブ変換の候補が出ている
        assert!(c.view().cand_open);
        assert_eq!(c.view().cands[0], u16s("猫"));
        // Tab で候補に入り Enter で確定
        c.press(&key(Key::Tab));
        assert!(c.converting());
        c.press(&key(Key::Enter));
        assert_eq!(c.take_commit(), u16s("猫"));
    }

    #[test]
    fn learning_applied() {
        let base = std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| std::env::temp_dir());
        let d = base.join(format!(".rext-comp-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let mut learning = Learning::new(d.join("learned.json"));
        learning.record(&u16s("ねこ"), &u16s("猫"), 2, &u16s(""), -1);
        let mut c = Composer::new(Some(Box::new(FakeConv::new())), Some(learning), None);
        type_str(&mut c, "neko");
        assert_eq!(c.view().cands[0], u16s("猫"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn greeting_and_brackets() {
        assert_eq!(greeting_fix(&u16s("こんにちは"), &u16s("今日は")), u16s("こんにちは"));
        assert_eq!(greeting_fix(&u16s("きょうは"), &u16s("今日は")), u16s("今日は"));
        let br = bracket_variants(&u16s("かっこ"));
        assert_eq!(br[0], u16s("（）"));
        let br2 = bracket_variants(&u16s("「"));
        assert_eq!(br2[0], u16s("「"));
        assert!(br2.len() > 5);
    }

    #[test]
    fn reconvert_and_undo() {
        let mut c = composer();
        c.options.live = false;
        type_str(&mut c, "neko");
        c.press(&key(Key::Space));
        c.press(&key(Key::Enter));
        assert_eq!(c.take_commit(), u16s("猫"));
        assert_eq!(c.reading_of(&u16s("猫")), u16s("ねこ"));
        assert!(c.start_reconvert(&u16s("ねこ"), &u16s("猫")));
        assert!(c.converting());
        c.press(&key(Key::Escape)); // 再変換をやめる -> 元の文が確定に戻る
        assert_eq!(c.take_commit(), u16s("猫"));
    }

    #[test]
    fn idle_space_and_post() {
        let mut c = composer();
        c.press(&key(Key::Space));
        assert_eq!(c.take_commit(), u16s("　"));
        let ev = KeyEvent { key: Key::Space, shift: true, ..Default::default() };
        c.press(&ev);
        assert_eq!(c.take_commit(), u16s(" "));
    }
}
