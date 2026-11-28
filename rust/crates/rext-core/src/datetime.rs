//! 日付・時刻の予測変換 (元: desktop/core/datetime.cpp + datetime.h、
//! entry/src/main/ets/ime/DateTimePredictor.ets、android の DateTimePredictor.kt)。
//! 候補の形 (日の形・時刻の形) は現行 HarmonyOS 版
//! (entry/src/main/ets/ime/DateTimePredictor.ets の dateForms / timeForms:
//! 日 最大 12 形・時刻 4 形。desktop/core/datetime.cpp と同内容) に統一した
//! (2026-11-28 ユーザー再決定。経緯は RUST_PORT_STATUS 参照: 一時、古い
//! golden.json の生成規則 (日 5 形・時刻 3 形) に寄せたが、現行 .ets が正)。
//! それ以外の規則 (読みの表・月日の解釈・和暦変換等) は従来どおり各版共通
//! のものを使う。
//!
//! 純粋関数 (reading, now): 現在時刻 (UNIX 時刻) を外から渡すので決定的に
//! 試験できる。結果は一時的な値で、呼び出し側は変換の学習に入れないこと。
//!
//! ローカル時刻について: 元実装 (C++ localtime_r / ETS・Kotlin のローカル
//! カレンダー) は OS のローカル時刻前提だった。Rust 移植では std::time を
//! 直接読まず、UTC からのオフセット秒を引数で受ける形にした
//! ([`datetime_predict_with_offset`])。既定値 [`DEFAULT_TZ_OFFSET_SECONDS`]
//! は主な利用環境の UTC+9 (日本、夏時間なし)。夏時間のある地域では
//! 呼び出し側がその時点のオフセットを渡す必要がある。

use crate::{u16s, U16};

/// 既定のローカル時刻オフセット (UTC+9、日本)。
pub const DEFAULT_TZ_OFFSET_SECONDS: i64 = 9 * 3600;

const DOW: &str = "日月火水木金土"; // 日〜土 (wday と同じ並び)
const DOW_FULL: [&str; 7] = ["日曜日", "月曜日", "火曜日", "水曜日", "木曜日", "金曜日", "土曜日"];

// ---------------------------------------------------------------- 暦の計算 (1970-01-01 からの日数)
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64; // [0, 399]
    let mp = if m > 2 { m - 3 } else { m + 9 } as u64;
    let doy = (153 * mp + 2) / 5 + (d as u64) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe as i64 - 719468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let yy = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let dd = doy - (153 * mp + 2) / 5 + 1;
    let mm = if mp < 10 { mp + 3 } else { mp - 9 };
    (yy + if mm <= 2 { 1 } else { 0 }, mm as i64, dd as i64)
}

fn wday_of_days(days: i64) -> i64 {
    ((days + 4) % 7 + 7) % 7 // 0=日 .. 6=土
}

fn days_in_month(y: i64, m: i64) -> i64 {
    const DM: [i64; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    if m == 2 && ((y % 4 == 0 && y % 100 != 0) || y % 400 == 0) {
        29
    } else {
        DM[(m - 1) as usize]
    }
}

/// UNIX 時刻 + オフセット秒 → (年, 月, 日, 時, 分)。元の local_breakdown の置き換え。
fn local_breakdown(now_unix: i64, offset_seconds: i64) -> (i64, i64, i64, i64, i64) {
    let t = now_unix + offset_seconds;
    let days = t.div_euclid(86400);
    let secs = t.rem_euclid(86400);
    let (y, mo, d) = civil_from_days(days);
    (y, mo, d, secs / 3600, (secs % 3600) / 60)
}

// ---------------------------------------------------------------- 数の読み (算用数字とかな)
struct Piece {
    k: &'static str,
    kind: char, // 'd' 数字、's' 小さい単位、'b' 大きい単位
    v: i64,
}
struct Sok {
    k: &'static str,
    v: i64,
    unit: i64,
}

// 長い方から (貪欲に当てる)。並びは C++ 版 PIECES のまま
const PIECES: &[Piece] = &[
    Piece { k: "きゅう", kind: 'd', v: 9 },
    Piece { k: "じゅう", kind: 's', v: 10 },
    Piece { k: "ひゃく", kind: 's', v: 100 },
    Piece { k: "びゃく", kind: 's', v: 100 },
    Piece { k: "ぴゃく", kind: 's', v: 100 },
    Piece { k: "ちょう", kind: 'b', v: 1_000_000_000_000 },
    Piece { k: "いち", kind: 'd', v: 1 },
    Piece { k: "さん", kind: 'd', v: 3 },
    Piece { k: "よん", kind: 'd', v: 4 },
    Piece { k: "ろく", kind: 'd', v: 6 },
    Piece { k: "なな", kind: 'd', v: 7 },
    Piece { k: "しち", kind: 'd', v: 7 },
    Piece { k: "はち", kind: 'd', v: 8 },
    Piece { k: "れい", kind: 'd', v: 0 },
    Piece { k: "ぜろ", kind: 'd', v: 0 },
    Piece { k: "いっ", kind: 'd', v: 1 },
    Piece { k: "ろっ", kind: 'd', v: 6 },
    Piece { k: "はっ", kind: 'd', v: 8 },
    Piece { k: "せん", kind: 's', v: 1000 },
    Piece { k: "ぜん", kind: 's', v: 1000 },
    Piece { k: "まん", kind: 'b', v: 10000 },
    Piece { k: "おく", kind: 'b', v: 100_000_000 },
    Piece { k: "に", kind: 'd', v: 2 },
    Piece { k: "し", kind: 'd', v: 4 },
    Piece { k: "ご", kind: 'd', v: 5 },
    Piece { k: "く", kind: 'd', v: 9 },
];
const SOKUON: &[Sok] = &[
    Sok { k: "じゅっ", v: 10, unit: 10 },
    Sok { k: "じっ", v: 10, unit: 10 },
    Sok { k: "ひゃっ", v: 100, unit: 100 },
    Sok { k: "せっ", v: 1000, unit: 1000 },
    Sok { k: "いっ", v: 1, unit: 1 },
    Sok { k: "ろっ", v: 6, unit: 1 },
    Sok { k: "はっ", v: 8, unit: 1 },
];

fn starts_with(s: &[u16], i: usize, k: &str) -> bool {
    let ku = u16s(k);
    i + ku.len() <= s.len() && s[i..i + ku.len()] == ku[..]
}
fn ends_with(s: &[u16], k: &str) -> bool {
    let ku = u16s(k);
    s.len() >= ku.len() && s[s.len() - ku.len()..] == ku[..]
}

fn num(v: i64) -> U16 {
    u16s(&v.to_string())
}
fn num2(v: i64) -> U16 {
    if v < 10 {
        let mut o = u16s("0");
        o.push(b'0' as u16 + v as u16);
        o
    } else {
        num(v)
    }
}

/// かなの数 (2 字以上)。元の parse_kana と同じ決まり。
fn parse_kana(r: &[u16]) -> Option<i64> {
    if r.len() < 2 {
        return None;
    }
    let sok = SOKUON.iter().find(|s| ends_with(r, s.k));
    let body: &[u16] = match sok {
        Some(s) => &r[..r.len() - u16s(s.k).len()],
        None => r,
    };
    let mut toks: Vec<(char, i64)> = Vec::new();
    let mut i = 0usize;
    let mut prev: &str = "";
    while i < body.len() {
        let mut hit = false;
        for p in PIECES {
            if !starts_with(body, i, p.k) {
                continue;
            }
            if (p.k == "ぜん" || p.k == "びゃく") && prev != "さん" && prev != "なん" {
                return None;
            }
            if p.k == "ぴゃく" && prev != "ろっ" && prev != "はっ" {
                return None;
            }
            toks.push((p.kind, p.v));
            prev = p.k;
            i += u16s(p.k).len();
            hit = true;
            break;
        }
        if !hit {
            return None;
        }
    }
    if let Some(s) = sok {
        if s.unit > 1 && s.v == s.unit {
            toks.push(('s', s.unit));
        } else {
            toks.push(('d', s.v));
        }
    }
    if toks.is_empty() {
        return None;
    }
    let mut total: i64 = 0;
    let mut group: i64 = 0;
    let mut last_small: i64 = 100000;
    let mut has_digit = false;
    let mut digit: i64 = 0;
    for &(kind, v) in &toks {
        match kind {
            'd' => {
                if has_digit {
                    return None;
                }
                has_digit = true;
                digit = v;
            }
            's' => {
                if v >= last_small {
                    return None;
                }
                group += if has_digit { digit } else { 1 } * v;
                has_digit = false;
                last_small = v;
            }
            _ => {
                // 'b'
                group += if has_digit { digit } else { 0 };
                if group == 0 {
                    return None;
                }
                total += group * v;
                group = 0;
                has_digit = false;
                last_small = 100000;
            }
        }
    }
    group += if has_digit { digit } else { 0 };
    let value = total + group;
    if value == 0 && !(toks.len() == 1 && toks[0].0 == 'd' && toks[0].1 == 0) {
        return None;
    }
    Some(value)
}

fn single_kana(c: u16) -> i64 {
    match c {
        0x306B => 2, // に
        0x3057 => 4, // し
        0x3054 => 5, // ご
        0x304F => 9, // く
        _ => -1,
    }
}

/// 数の読み 1 語 (算用数字 or かな)。読めたら値。
fn parse_number_seg(seg: &[u16]) -> Option<i64> {
    if seg.is_empty() {
        return None;
    }
    let arabic = seg
        .iter()
        .all(|&c| (0x30..=0x39).contains(&c) || (0xFF10..=0xFF19).contains(&c));
    if arabic {
        let mut x: i64 = 0;
        for &c in seg {
            x = x * 10
                + if c >= 0xFF10 {
                    (c - 0xFF10) as i64
                } else {
                    (c - 0x30) as i64
                };
        }
        return Some(x);
    }
    if seg.len() == 1 {
        let s = single_kana(seg[0]);
        return if s >= 0 { Some(s) } else { None };
    }
    parse_kana(seg)
}

// ---------------------------------------------------------------- 和暦
/// 日付の形に添える和暦 (令和だけ)。対象外は空。
fn wareki(y: i64, m: i64, d: i64) -> U16 {
    if y > 2019 || (y == 2019 && (m > 5 || (m == 5 && d >= 1))) {
        let r = y - 2018;
        if r == 1 {
            u16s("令和元年")
        } else {
            let mut o = u16s("令和");
            o.extend(num(r));
            o.extend(u16s("年"));
            o
        }
    } else {
        Vec::new()
    }
}

// ---------------------------------------------------------------- 形を作る
fn dow_char(w: i64) -> u16 {
    DOW.encode_utf16().nth(w as usize).unwrap()
}

/// 日の形 (現行 HarmonyOS 版 DateTimePredictor.ets の dateForms と同じ
/// 規則・並び。2026-11-28 ユーザー再決定で現行 .ets を正とした)。
/// 基本 9 形 + 和暦 2 形 (令和開始前は無し) + 数字 8 桁 = 最大 12 形。
/// desktop 版 date_forms (datetime.cpp) とも同内容。
fn date_forms(days: i64) -> Vec<U16> {
    let (y, m, d) = civil_from_days(days);
    let w = wday_of_days(days);
    let (yy, mm, dd) = (num(y), num(m), num(d));
    let dow1 = vec![dow_char(w)];
    let dowf = u16s(DOW_FULL[w as usize]);
    let cat = |parts: &[U16]| -> U16 { parts.concat() };
    let mut out = vec![
        cat(&[yy.clone(), u16s("年"), mm.clone(), u16s("月"), dd.clone(), u16s("日")]),
        cat(&[yy.clone(), u16s("年"), mm.clone(), u16s("月"), dd.clone(), u16s("日("), dow1.clone(), u16s(")")]),
        cat(&[mm.clone(), u16s("月"), dd.clone(), u16s("日")]),
        cat(&[mm.clone(), u16s("月"), dd.clone(), u16s("日("), dow1.clone(), u16s(")")]),
        cat(&[mm.clone(), u16s("月"), dd.clone(), u16s("日("), dowf, u16s(")")]),
        cat(&[mm.clone(), u16s("/"), dd.clone()]),
        cat(&[mm.clone(), u16s("/"), dd.clone(), u16s("("), dow1.clone(), u16s(")")]),
        cat(&[yy.clone(), u16s("/"), mm.clone(), u16s("/"), dd.clone()]),
        cat(&[yy.clone(), u16s("-"), mm.clone(), u16s("-"), dd.clone()]),
    ];
    let w_ = wareki(y, m, d);
    if !w_.is_empty() {
        out.push(cat(&[w_.clone(), mm.clone(), u16s("月"), dd.clone(), u16s("日")]));
        out.push(cat(&[w_, mm.clone(), u16s("月"), dd.clone(), u16s("日("), dow1, u16s(")")]));
    }
    // 数字 8 桁 (20261021。月・日は 0 詰め)
    out.push(cat(&[yy, num2(m), num2(d)]));
    out
}

/// <数>がつ<数>にち の日付に曜日をつけた形 (4 通り)。
fn date_dow_forms(y: i64, m: i64, d: i64) -> Vec<U16> {
    let days = days_from_civil(y, m, d);
    let w = wday_of_days(days);
    let (yy, mm, dd) = (num(y), num(m), num(d));
    let dow1 = vec![dow_char(w)];
    let dowf = u16s(DOW_FULL[w as usize]);
    vec![
        [mm.clone(), u16s("月"), dd.clone(), u16s("日("), dow1.clone(), u16s(")")].concat(),
        [mm.clone(), u16s("月"), dd.clone(), u16s("日("), dowf, u16s(")")].concat(),
        [mm.clone(), u16s("/"), dd.clone(), u16s("("), dow1.clone(), u16s(")")].concat(),
        [yy, u16s("年"), mm, u16s("月"), dd, u16s("日("), dow1, u16s(")")].concat(),
    ]
}

/// 時刻の形 (現行 HarmonyOS 版 DateTimePredictor.ets の timeForms と同じ
/// 4 形。2026-11-28 ユーザー再決定)。4 形目は「M月D日 H:MM」。
fn time_forms(_y: i64, m: i64, d: i64, hh: i64, mi: i64) -> Vec<U16> {
    let h12 = if hh % 12 == 0 { 12 } else { hh % 12 };
    let mm = num2(mi);
    let ampm = if hh < 12 { "午前" } else { "午後" };
    vec![
        [num(hh), u16s(":"), mm.clone()].concat(),
        [num(hh), u16s("時"), num(mi), u16s("分")].concat(),
        [u16s(ampm), num(h12), u16s("時"), num(mi), u16s("分")].concat(),
        [num(m), u16s("月"), num(d), u16s("日 "), num(hh), u16s(":"), mm].concat(),
    ]
}

fn year_offset_forms(y: i64) -> Vec<U16> {
    let mut out = vec![[num(y), u16s("年")].concat()];
    let w = wareki(y, 6, 15);
    if !w.is_empty() {
        out.push(w);
    }
    out
}

fn month_forms(y: i64, m: i64) -> Vec<U16> {
    vec![
        [num(y), u16s("年"), num(m), u16s("月")].concat(),
        [num(m), u16s("月")].concat(),
    ]
}

fn week_range_forms(monday: i64) -> Vec<U16> {
    let (_y1, m1, d1) = civil_from_days(monday);
    let (_y2, m2, d2) = civil_from_days(monday + 6);
    vec![
        [num(m1), u16s("月"), num(d1), u16s("日〜"), num(m2), u16s("月"), num(d2), u16s("日")].concat(),
        [num(m1), u16s("/"), num(d1), u16s("〜"), num(m2), u16s("/"), num(d2)].concat(),
    ]
}

/// 曜日の読み (にちようび…どようび / にちよう…どよう) -> 0=日..6=土。無ければ -1。
fn weekday_of(s: &[u16]) -> i64 {
    const WDR: [&str; 7] = ["にち", "げつ", "か", "すい", "もく", "きん", "ど"];
    for (w, k) in WDR.iter().enumerate() {
        if s == u16s(&format!("{k}ようび")).as_slice() || s == u16s(&format!("{k}よう")).as_slice() {
            return w as i64;
        }
    }
    -1
}

struct OffsetEntry {
    k: &'static str,
    off: i64,
}
const DAY_OFFSETS: &[OffsetEntry] = &[
    OffsetEntry { k: "きょう", off: 0 },
    OffsetEntry { k: "ほんじつ", off: 0 },
    OffsetEntry { k: "あした", off: 1 },
    OffsetEntry { k: "あす", off: 1 },
    OffsetEntry { k: "みょうにち", off: 1 },
    OffsetEntry { k: "きのう", off: -1 },
    OffsetEntry { k: "さくじつ", off: -1 },
    OffsetEntry { k: "あさって", off: 2 },
    OffsetEntry { k: "みょうごにち", off: 2 },
    OffsetEntry { k: "しあさって", off: 3 },
    OffsetEntry { k: "おととい", off: -2 },
    OffsetEntry { k: "いっさくじつ", off: -2 },
    OffsetEntry { k: "さきおととい", off: -3 },
];
const YEAR_OFFSETS: &[OffsetEntry] = &[
    OffsetEntry { k: "ことし", off: 0 },
    OffsetEntry { k: "こんねん", off: 0 },
    OffsetEntry { k: "らいねん", off: 1 },
    OffsetEntry { k: "みょうねん", off: 1 },
    OffsetEntry { k: "きょねん", off: -1 },
    OffsetEntry { k: "さくねん", off: -1 },
    OffsetEntry { k: "さらいねん", off: 2 },
];
const MONTH_OFFSETS: &[OffsetEntry] = &[
    OffsetEntry { k: "こんげつ", off: 0 },
    OffsetEntry { k: "らいげつ", off: 1 },
    OffsetEntry { k: "せんげつ", off: -1 },
];
const WEEKDAY_WEEKS: &[OffsetEntry] = &[
    OffsetEntry { k: "さらいしゅう", off: 2 },
    OffsetEntry { k: "らいしゅう", off: 1 },
    OffsetEntry { k: "こんしゅう", off: 0 },
    OffsetEntry { k: "せんしゅう", off: -1 },
];
const WEEK_RANGES: &[OffsetEntry] = &[
    OffsetEntry { k: "らいしゅう", off: 1 },
    OffsetEntry { k: "こんしゅう", off: 0 },
    OffsetEntry { k: "せんしゅう", off: -1 },
];
const TIME_READINGS: [&str; 3] = ["いま", "げんざい", "ただいま"];

fn find_offset(t: &[OffsetEntry], s: &[u16]) -> Option<i64> {
    t.iter().find(|e| s == u16s(e.k).as_slice()).map(|e| e.off)
}

const ERAS: [(&str, i64); 3] = [("れいわ", 2018), ("へいせい", 1988), ("しょうわ", 1925)];

fn find_sub(hay: &[u16], needle: &[u16]) -> Option<usize> {
    if needle.len() > hay.len() {
        return None;
    }
    (0..=hay.len() - needle.len()).find(|&i| hay[i..i + needle.len()] == needle[..])
}

/// <数>がつ<数>にち [<数>ねん 付きも]。読めたら (年, 月, 日, 年付きか)。
fn parse_month_day(r: &[u16]) -> Option<(i64, i64, i64, bool)> {
    let gatsu = u16s("がつ");
    let g = find_sub(r, &gatsu)?;
    let before = &r[..g];
    let after = &r[g + 2..];
    let (mut year, mut hy) = (0i64, false);
    let m: i64;
    let nen = u16s("ねん");
    let npos = find_sub(before, &nen);
    if let Some(n) = npos {
        let yr = &before[..n];
        let mo = &before[n + 2..];
        let mut yv: Option<i64> = None;
        for (k, base) in ERAS {
            if !starts_with(yr, 0, k) {
                continue;
            }
            let nseg = &yr[u16s(k).len()..];
            let nv = if nseg == u16s("がん").as_slice() {
                1
            } else {
                parse_number_seg(nseg)?
            };
            yv = Some(base + nv);
            hy = true;
            break;
        }
        if !hy {
            let v = parse_number_seg(yr)?;
            if v < 1 {
                return None;
            }
            yv = Some(v);
            hy = true;
        }
        year = yv.unwrap();
        let mv = parse_number_seg(mo)?;
        if !(1..=12).contains(&mv) {
            return None;
        }
        m = mv;
    } else {
        let mv = parse_number_seg(before)?;
        if !(1..=12).contains(&mv) {
            return None;
        }
        m = mv;
    }
    const SP: [(&str, i64); 14] = [
        ("ついたち", 1),
        ("ふつか", 2),
        ("みっか", 3),
        ("よっか", 4),
        ("いつか", 5),
        ("むいか", 6),
        ("なのか", 7),
        ("ようか", 8),
        ("ここのか", 9),
        ("とおか", 10),
        ("じゅうよっか", 14),
        ("はつか", 20),
        ("にじゅうよっか", 24),
        ("みそか", 30),
    ];
    for (k, d) in SP {
        if after == u16s(k).as_slice() {
            return Some((year, m, d, hy));
        }
    }
    if ends_with(after, "にち") {
        let body = &after[..after.len() - 2];
        if let Some(dv) = parse_number_seg(body) {
            if (1..=31).contains(&dv) {
                return Some((year, m, dv, hy));
            }
        }
    }
    None
}

/// 読み reading に対する日付・時刻の候補 (now_unix = UNIX 時刻)。
/// ローカル時刻は既定オフセット UTC+9 で扱う。オフセットを変える場合は
/// [`datetime_predict_with_offset`] を使う。当たらなければ空。
pub fn datetime_predict(reading: &[u16], now_unix: i64) -> Vec<U16> {
    datetime_predict_with_offset(reading, now_unix, DEFAULT_TZ_OFFSET_SECONDS)
}

/// [`datetime_predict`] のオフセット指定版 (秒)。元実装の localtime の代わり。
pub fn datetime_predict_with_offset(reading: &[u16], now_unix: i64, offset_seconds: i64) -> Vec<U16> {
    let (y, mo, d, hh, mi) = local_breakdown(now_unix, offset_seconds);
    let today = days_from_civil(y, mo, d);
    let wday = wday_of_days(today);

    // 数字 8 桁の日付 (20261018 → 2026年10月18日(日) など)。ありえない日付は出さない
    if reading.len() == 8 {
        let mut v = [0i64; 8];
        let mut k = 0usize;
        for &c in reading {
            if (0x30..=0x39).contains(&c) {
                v[k] = (c - 0x30) as i64;
                k += 1;
            } else if (0xFF10..=0xFF19).contains(&c) {
                v[k] = (c - 0xFF10) as i64;
                k += 1;
            } else {
                break;
            }
        }
        if k == 8 {
            let (yy, mm, dd) = (v[0] * 1000 + v[1] * 100 + v[2] * 10 + v[3], v[4] * 10 + v[5], v[6] * 10 + v[7]);
            if (1900..=2100).contains(&yy) && (1..=12).contains(&mm) && (1..=31).contains(&dd) {
                let days = days_from_civil(yy, mm, dd);
                if civil_from_days(days) == (yy, mm, dd) {
                    let mut out = date_forms(days);
                    out.extend(crate::number::number_forms(reading));
                    return out;
                }
            }
        }
    }
    // 数字の書き換えは number.rs の単一実装を使う (ここでは再実装しない)
    {
        let nf = crate::number::number_forms(reading);
        if !nf.is_empty() {
            return nf;
        }
    }
    if let Some(off) = find_offset(DAY_OFFSETS, reading) {
        return date_forms(today + off);
    }
    if let Some(off) = find_offset(YEAR_OFFSETS, reading) {
        return year_offset_forms(y + off);
    }
    if TIME_READINGS.iter().any(|t| reading == u16s(t).as_slice()) {
        return time_forms(y, mo, d, hh, mi);
    }
    if reading == u16s("ようび").as_slice() || reading == u16s("なんようび").as_slice() {
        return vec![
            u16s(DOW_FULL[wday as usize]),
            vec![dow_char(wday)],
            [u16s("("), vec![dow_char(wday)], u16s(")")].concat(),
        ];
    }
    if let Some(off) = find_offset(WEEK_RANGES, reading) {
        return week_range_forms(today - (wday + 6) % 7 + off * 7);
    }
    if let Some(off) = find_offset(MONTH_OFFSETS, reading) {
        let (mut tm, mut ty) = (mo + off, y);
        if tm <= 0 {
            tm += 12;
            ty -= 1;
        }
        if tm > 12 {
            tm -= 12;
            ty += 1;
        }
        return month_forms(ty, tm);
    }
    // こんしゅうのげつようび・らいしゅうのきんようび … (の の有無どちらも、さらいしゅう も)
    for wk in WEEKDAY_WEEKS {
        let ku = u16s(wk.k);
        if reading.len() <= ku.len() || reading[..ku.len()] != ku[..] {
            continue;
        }
        let mut rest = &reading[ku.len()..];
        if starts_with(rest, 0, "の") {
            rest = &rest[1..];
        }
        let w = weekday_of(rest);
        if w < 0 {
            continue;
        }
        let mon_off = (wday + 6) % 7;
        let target = (w + 6) % 7;
        return date_forms(today - mon_off + target + wk.off * 7);
    }
    // 曜日だけ (げつようび …): 今日か次のその曜日
    {
        let w = weekday_of(reading);
        if w >= 0 {
            return date_forms(today + (w - wday + 7) % 7);
        }
    }
    // <数>がつ<数>にち [<数>ねん 付きも]: 日付に曜日をつける
    if let Some((yv, mv, dv, hy)) = parse_month_day(reading) {
        let mut year = if hy { yv } else { y };
        if !hy && today - days_from_civil(year, mv, dv) >= 60 {
            year = y + 1;
        }
        if dv > days_in_month(year, mv) {
            return Vec::new();
        }
        return date_dow_forms(year, mv, dv);
    }
    // <数>ねん (西暦 -> 和暦) / れいわ<数>ねん … (和暦 -> 西暦)
    if ends_with(reading, "ねん") {
        let body = &reading[..reading.len() - 2];
        for (k, base) in ERAS {
            if !starts_with(body, 0, k) {
                continue;
            }
            let nseg = &body[u16s(k).len()..];
            let nv = if nseg == u16s("がん").as_slice() {
                1
            } else {
                match parse_number_seg(nseg) {
                    Some(v) => v,
                    None => return Vec::new(),
                }
            };
            let yy = base + nv;
            if yy < 1 {
                return Vec::new();
            }
            return vec![[num(yy), u16s("年")].concat()];
        }
        if let Some(yy) = parse_number_seg(body) {
            if (1926..=2100).contains(&yy) {
                let w = crate::number::era_year(yy);
                if !w.is_empty() {
                    return vec![w];
                }
            }
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::from_u16s;

    // 2026-10-21 (水) 12:34 JST
    const NOW: i64 = 1792553640;

    fn strs(v: Vec<U16>) -> Vec<String> {
        v.iter().map(|s| from_u16s(s)).collect()
    }
    fn first(reading: &str, now: i64) -> String {
        from_u16s(&datetime_predict(&u16s(reading), now)[0])
    }

    #[test]
    fn civil_roundtrip() {
        for (y, m, d) in [(1970, 1, 1), (2026, 10, 21), (2000, 2, 29), (1900, 3, 1), (2019, 5, 1)] {
            assert_eq!(civil_from_days(days_from_civil(y, m, d)), (y, m, d));
        }
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(wday_of_days(0), 4); // 1970-01-01 は木曜
    }

    #[test]
    fn today_forms() {
        // 形と並びは現行 HarmonyOS 版 DateTimePredictor.ets の dateForms
        // (最大 12 形、desktop 版と同内容) に統一 (2026-11-28 ユーザー再決定)。
        let f = strs(datetime_predict(&u16s("きょう"), NOW));
        assert_eq!(
            f,
            vec![
                "2026年10月21日",
                "2026年10月21日(水)",
                "10月21日",
                "10月21日(水)",
                "10月21日(水曜日)",
                "10/21",
                "10/21(水)",
                "2026/10/21",
                "2026-10-21",
                "令和8年10月21日",
                "令和8年10月21日(水)",
                "20261021",
            ]
        );
        assert_eq!(first("あした", NOW), "2026年10月22日");
        assert_eq!(first("きのう", NOW), "2026年10月20日");
        assert_eq!(first("しあさって", NOW), "2026年10月24日");
    }

    #[test]
    fn time_forms_now() {
        // 現行 HarmonyOS 版 timeForms の 4 形 (2026-11-28 ユーザー再決定)。
        assert_eq!(
            strs(datetime_predict(&u16s("いま"), NOW)),
            vec!["12:34", "12時34分", "午後12時34分", "10月21日 12:34"]
        );
    }

    #[test]
    fn year_and_month_offsets() {
        assert_eq!(strs(datetime_predict(&u16s("ことし"), NOW)), vec!["2026年", "令和8年"]);
        assert_eq!(strs(datetime_predict(&u16s("らいねん"), NOW)), vec!["2027年", "令和9年"]);
        assert_eq!(
            strs(datetime_predict(&u16s("こんげつ"), NOW)),
            vec!["2026年10月", "10月"]
        );
        assert_eq!(first("せんげつ", NOW), "2026年9月");
    }

    #[test]
    fn weekday_readings() {
        assert_eq!(
            strs(datetime_predict(&u16s("ようび"), NOW)),
            vec!["水曜日", "水", "(水)"]
        );
        // 曜日だけ: 今日か次のその曜日 (水曜から月曜は 2026-10-26)
        assert_eq!(first("げつようび", NOW), "2026年10月26日");
        assert_eq!(first("すいようび", NOW), "2026年10月21日");
        // こんしゅうのきんようび / らいしゅうのきんようび
        assert_eq!(first("こんしゅうのきんようび", NOW), "2026年10月23日");
        assert_eq!(first("らいしゅうきんようび", NOW), "2026年10月30日");
    }

    #[test]
    fn week_range() {
        assert_eq!(
            strs(datetime_predict(&u16s("こんしゅう"), NOW)),
            vec!["10月19日〜10月25日", "10/19〜10/25"]
        );
    }

    #[test]
    fn eight_digit_date() {
        let f = strs(datetime_predict(&u16s("20261018"), NOW));
        assert_eq!(f[0], "2026年10月18日");
        assert_eq!(f[1], "2026年10月18日(日)");
        // 後ろに number_forms が付く
        assert!(f.contains(&"20,261,018".to_string()));
        // ありえない日付は日付にせず数字の形だけ
        let bad = strs(datetime_predict(&u16s("20260230"), NOW));
        assert_eq!(bad[0], "20,260,230");
        assert!(!bad.iter().any(|s| s.contains("年2月")));
    }

    #[test]
    fn month_day_reading() {
        assert_eq!(
            strs(datetime_predict(&u16s("じゅうがつにじゅういちにち"), NOW)),
            vec!["10月21日(水)", "10月21日(水曜日)", "10/21(水)", "2026年10月21日(水)"]
        );
        // 特殊な日の読み (はつか = 20 日)
        assert_eq!(
            datetime_predict(&u16s("じゅうがつはつか"), NOW)[0],
            u16s("10月20日(火)")
        );
        // 2月30日は出さない
        assert!(datetime_predict(&u16s("にがつさんじゅうにち"), NOW).is_empty());
    }

    #[test]
    fn era_conversions() {
        assert_eq!(strs(datetime_predict(&u16s("れいわはちねん"), NOW)), vec!["2026年"]);
        assert_eq!(strs(datetime_predict(&u16s("れいわがんねん"), NOW)), vec!["2019年"]);
        assert_eq!(
            strs(datetime_predict(&u16s("にせんにじゅうろくねん"), NOW)),
            vec!["令和8年"]
        );
        assert_eq!(
            strs(datetime_predict(&u16s("へいせいさんじゅういちねん"), NOW)),
            vec!["2019年"]
        );
    }

    #[test]
    fn offset_changes_date() {
        // 2026-10-21 00:30 JST = 2026-10-20 15:30 UTC
        let now = 1792510200;
        assert_eq!(first("きょう", now), "2026年10月21日"); // 既定 UTC+9
        let utc = datetime_predict_with_offset(&u16s("きょう"), now, 0);
        assert_eq!(from_u16s(&utc[0]), "2026年10月20日");
    }

    #[test]
    fn no_match() {
        assert!(datetime_predict(&u16s("あいうえお"), NOW).is_empty());
        assert!(datetime_predict(&u16s(""), NOW).is_empty());
    }

    #[test]
    fn kana_number_parse() {
        assert_eq!(parse_kana(&u16s("にじゅういち")), Some(21));
        assert_eq!(parse_kana(&u16s("さんびゃく")), Some(300));
        assert_eq!(parse_kana(&u16s("ろっぴゃく")), Some(600));
        assert_eq!(parse_kana(&u16s("いっせん")), Some(1000));
        assert_eq!(parse_kana(&u16s("ごまん")), Some(50000));
        assert_eq!(parse_kana(&u16s("びゃく")), None); // さん/なん の前置きが無い
    }
}
