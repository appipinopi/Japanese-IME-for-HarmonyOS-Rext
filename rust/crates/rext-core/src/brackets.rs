//! 括弧の候補と自動対応付け (元: entry/src/main/ets/ime/Brackets.ets、
//! android の Brackets.kt、desktop/core/composer.cpp の BRACKETS / MARKS /
//! bracket_variants、KeyboardController.ets の BRACKET_PAIRS)。
//! 並びは全実装で同一 (変えるときは全部そろえる)。食い違いは C++ 版を正とした。

use crate::{u16s, U16};

/// 括弧の全種類 (開き, 閉じ)。どの括弧も必ず打てるように、変換の候補はここから出す。
pub const ALL_BRACKETS: [(&str, &str); 24] = [
("（", "）"),
("(", ")"),
("「", "」"),
("『", "』"),
("\u{3010}", "\u{3011}"),
("［", "］"),
("[", "]"),
("｛", "｝"),
("{", "}"),
("〔", "〕"),
("〈", "〉"),
("《", "》"),
("〖", "〗"),
("〘", "〙"),
("〚", "〛"),
("｢", "｣"),
("＜", "＞"),
("<", ">"),
("«", "»"),
("‹", "›"),
("“", "”"),
("‘", "’"),
("〝", "〟"),
("｟", "｠"),
];

/// 開き括弧の一覧 (ALL_BRACKETS の並び)。
pub fn opens() -> Vec<U16> {
ALL_BRACKETS.iter().map(|p| u16s(p.0)).collect()
}

/// 閉じ括弧の一覧 (ALL_BRACKETS の並び)。
pub fn closes() -> Vec<U16> {
ALL_BRACKETS.iter().map(|p| u16s(p.1)).collect()
}

/// 開きに対応する閉じ (括弧でなければ None)。
pub fn closing_for(open: &[u16]) -> Option<U16> {
ALL_BRACKETS
.iter()
.find(|p| u16s(p.0) == open)
.map(|p| u16s(p.1))
}

/// 閉じに対応する開き (括弧でなければ None)。
pub fn opening_for(close: &[u16]) -> Option<U16> {
ALL_BRACKETS
.iter()
.find(|p| u16s(p.1) == close)
.map(|p| u16s(p.0))
}

/// 自動で閉じる対象か (開きを入れたら閉じも組で入れる括弧)。
/// ＜ < ‘ は不等号・アポストロフィにも使うので除く (元の BRACKET_PAIRS / PAIRS と同じ)。
pub fn auto_close(close_of_open: &[u16]) -> Option<U16> {
if close_of_open == u16s("＜").as_slice()
|| close_of_open == u16s("<").as_slice()
|| close_of_open == u16s("‘").as_slice()
{
return None;
}
closing_for(close_of_open)
}

// 記号 1 字の仲間 (全角・半角の両方と仲間の記号)。MS-IME と同じく ・ で ／ /、
// ￥ で ＼ \ (元の MARKS と同じ並び)。
const MARKS: &[&[&str]] = &[
&["・", "／", "/"],
&["、", "，", ","],
&["。", "．", "."],
&["ー", "－", "-", "―", "‐"],
&["～", "〜", "~"],
&["￥", "＼", "\\"],
&["＊", "*", "※", "×"],
&["＂", "\"", "“", "”"],
&["＇", "'", "‘", "’"],
];

/// 記号 1 字の候補 (打った字が先頭)。記号でなければ空。英数字は空。
fn mark_variants(reading: &[u16]) -> Vec<U16> {
if reading.len()!= 1 {
return Vec::new();
}
let mut group: Vec<U16> = Vec::new();
for g in MARKS {
let gu: Vec<U16> = g.iter().map(|s| u16s(s)).collect();
if gu.iter().any(|m| m[..] == reading[..]) {
group = gu;
break;
}
}
if group.is_empty() {
// ほかの記号は全角と半角 (！ と! など)。英数字は変換の候補に出るので入れない
let ch = reading[0];
let half = if (0xFF01..=0xFF5E).contains(&ch) { ch - 0xFEE0} else { ch};
let is_digit = (0x30..=0x39).contains(&half);
let is_alpha = (0x61..=0x7A).contains(&(half | 0x20));
if!(half > 0x20 && half < 0x7F &&!is_digit &&!is_alpha) {
return Vec::new();
}
group = vec![vec![half + 0xFEE0], vec![half]];
}
let mut out = vec![reading.to_vec()];
for m in group {
if m[..]!= reading[..] {
out.push(m);
}
}
out
}

/// 読みに合う括弧の候補。
/// 括弧 1 字 → 同じ側 (開き・閉じ) の全種類 (打った字が先頭)
/// かっこ → 全種類の組 / かっこひらき・かっことじ → 開き・閉じの全種類
/// 記号 1 字 → 全角・半角の両方と仲間の記号 (打った字が先頭)
/// 括弧でも記号でもない読みは空。元: composer.cpp の bracket_variants。
pub fn bracket_variants(reading: &[u16]) -> Vec<U16> {
let opens = opens();
let closes = closes();
if reading == u16s("かっこ").as_slice() {
return ALL_BRACKETS
.iter()
.map(|p| {
let mut o = u16s(p.0);
o.extend(u16s(p.1));
o
})
.collect();
}
if reading == u16s("かっこひらき").as_slice() {
return opens;
}
if reading == u16s("かっことじ").as_slice() {
return closes;
}
// 括弧 1 字でない記号は mark_variants (括弧は下で全種類を出す)
if reading.len() == 1
&&!opens.iter().any(|o| o[..] == reading[..])
&&!closes.iter().any(|c| c[..] == reading[..])
{
let marks = mark_variants(reading);
if!marks.is_empty() {
return marks;
}
}
for side in [&opens, &closes] {
if side.iter().any(|c| c[..] == reading[..]) {
let mut out = vec![reading.to_vec()];
for c in side {
if c[..]!= reading[..] {
out.push(c.clone());
}
}
return out;
}
}
Vec::new()
}

#[cfg(test)]
mod tests {
use super::*;
use crate::from_u16s;

fn strs(v: Vec<U16>) -> Vec<String> {
v.iter().map(|s| from_u16s(s)).collect()
}

#[test]
fn kakko_readings() {
let pairs = bracket_variants(&u16s("かっこ"));
assert_eq!(pairs.len(), 24);
assert_eq!(from_u16s(&pairs[0]), "（）");
assert_eq!(from_u16s(&pairs[2]), "「」");
assert_eq!(bracket_variants(&u16s("かっこひらき")).len(), 24);
assert_eq!(bracket_variants(&u16s("かっことじ")).len(), 24);
assert_eq!(from_u16s(&bracket_variants(&u16s("かっこひらき"))[0]), "（");
assert_eq!(from_u16s(&bracket_variants(&u16s("かっことじ"))[0]), "）");
}

#[test]
fn single_bracket_same_side_first() {
let v = bracket_variants(&u16s("「"));
assert_eq!(v.len(), 24);
assert_eq!(from_u16s(&v[0]), "「");
assert_eq!(from_u16s(&v[1]), "（"); // 打った字以外は ALL の並び
let c = bracket_variants(&u16s(")"));
assert_eq!(from_u16s(&c[0]), ")");
assert_eq!(from_u16s(&c[1]), "）");
}

#[test]
fn pairing() {
assert_eq!(closing_for(&u16s("「")), Some(u16s("」")));
assert_eq!(opening_for(&u16s("」")), Some(u16s("「")));
assert_eq!(closing_for(&u16s("あ")), None);
// 自動閉じ: ＜ < ‘ は対象外、それ以外は閉じが返る
assert_eq!(auto_close(&u16s("（")), Some(u16s("）")));
assert_eq!(auto_close(&u16s("＜")), None);
assert_eq!(auto_close(&u16s("<")), None);
assert_eq!(auto_close(&u16s("‘")), None);
assert_eq!(auto_close(&u16s("“")), Some(u16s("”")));
}

#[test]
fn marks_variants() {
assert_eq!(strs(bracket_variants(&u16s("・"))), vec!["・", "／", "/"]);
assert_eq!(strs(bracket_variants(&u16s("￥"))), vec!["￥", "＼", "\\"]);
assert_eq!(strs(bracket_variants(&u16s("!"))), vec!["!", "！"]);
assert_eq!(strs(bracket_variants(&u16s("！"))), vec!["！", "!"]);
// 英数字は空
assert!(bracket_variants(&u16s("a")).is_empty());
assert!(bracket_variants(&u16s("1")).is_empty());
assert!(bracket_variants(&u16s("あ")).is_empty());
}
}
