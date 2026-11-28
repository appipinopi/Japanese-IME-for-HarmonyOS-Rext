//! ゴールデン突き合わせ (RUST_PORT_STATUS「残っているもの」7 の自動化)。
//!
//! 対象:
//! - `android/app/src/test/resources/golden.json` (HarmonyOS 版ロジックを Node で
//!   実行して生成。Kotlin `GoldenTest.kt` が NumberFormatter.predict /
//!   JapaneseConverter.processKey / DateTimePredictor.predict と突き合わせるもの)
//!   を実行時に読み、rext-core の公開 API (`number::predict`, `romaji::Romaji`,
//!   `datetime::datetime_predict`) と突き合わせる。
//! - `desktop/tests/core_test.cpp` のうちエンジン辞書 (kkc_lex.bin / kkc_model.bin)
//!   無しで検証できる期待値ペア (romaji / datetime / number_forms / text /
//!   layout の一部) を表として埋め込み、同じ公開 API で突き合わせる。
//!
//! 測り方 (数字を偽らないための約束):
//! - 「完全一致」= GoldenTest.kt と同じく候補リスト全体の一致 (順序込み)。
//! - dates については「包含 (ゴールデンの候補がすべて、同順序のまま Rust の
//!   候補に含まれる)」も数え、こちらを合格条件にする。理由: golden.json は
//!   古い HarmonyOS 版ロジック (日 5 形・時刻 3 形) の生成物であり、候補の
//!   出し方の正は現行 entry/src/main/ets/ime/DateTimePredictor.ets
//!   (日 最大 12 形・時刻 4 形) である (2026-11-28 ユーザー再決定)。
//!   そのため golden との完全一致は下がるのが正しい状態で、完全一致の
//!   担保は現行 .ets / desktop 版と同内容の core_test_datetime と
//!   datetime.rs の単体テストで行う。
//! - このテストは測定用: 不一致があっても assert で落とさず、件数・一致数・
//!   不一致一覧 (入力/期待/実際) を標準出力に残す (`-- --nocapture` で見る)。
//!   落とすのはゴールデンが読めない・API が壊れている場合だけ。
//!   実測値は tests/GOLDEN_MATCH_REPORT.md に記録する。

use rext_core::datetime::datetime_predict;
use rext_core::layout::Layout;
use rext_core::number::{number_forms, predict as number_predict};
use rext_core::romaji::Romaji;
use rext_core::text::to_halfwidth_kana;
use rext_core::{from_u16s, u16s};

// ---------------------------------------------------------------- 簡易 JSON
#[derive(Debug, Clone, PartialEq)]
enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

struct P<'a> {
    b: &'a [u8],
    i: usize,
}
impl<'a> P<'a> {
    fn ws(&mut self) {
        while self.i < self.b.len() && (self.b[self.i] as char).is_whitespace() {
            self.i += 1;
        }
    }
    fn val(&mut self) -> Json {
        self.ws();
        match self.b[self.i] {
            b'{' => {
                self.i += 1;
                let mut o = Vec::new();
                loop {
                    self.ws();
                    if self.b[self.i] == b'}' {
                        self.i += 1;
                        break;
                    }
                    let k = match self.val() {
                        Json::Str(s) => s,
                        _ => panic!("obj key"),
                    };
                    self.ws();
                    assert_eq!(self.b[self.i], b':');
                    self.i += 1;
                    let v = self.val();
                    o.push((k, v));
                    self.ws();
                    if self.b[self.i] == b',' {
                        self.i += 1;
                    }
                }
                Json::Obj(o)
            }
            b'[' => {
                self.i += 1;
                let mut a = Vec::new();
                loop {
                    self.ws();
                    if self.b[self.i] == b']' {
                        self.i += 1;
                        break;
                    }
                    a.push(self.val());
                    self.ws();
                    if self.b[self.i] == b',' {
                        self.i += 1;
                    }
                }
                Json::Arr(a)
            }
            b'"' => {
                self.i += 1;
                let mut s = String::new();
                loop {
                    match self.b[self.i] {
                        b'"' => {
                            self.i += 1;
                            break;
                        }
                        b'\\' => {
                            self.i += 1;
                            match self.b[self.i] {
                                b'n' => s.push('\n'),
                                b't' => s.push('\t'),
                                b'r' => s.push('\r'),
                                b'/' => s.push('/'),
                                b'\\' => s.push('\\'),
                                b'"' => s.push('"'),
                                b'u' => {
                                    let h = std::str::from_utf8(&self.b[self.i + 1..self.i + 5]).unwrap();
                                    let mut cp = u32::from_str_radix(h, 16).unwrap();
                                    self.i += 4;
                                    if (0xD800..0xDC00).contains(&cp) {
                                        // サロゲートペア
                                        assert_eq!(self.b[self.i + 1], b'\\');
                                        assert_eq!(self.b[self.i + 2], b'u');
                                        let h2 = std::str::from_utf8(&self.b[self.i + 3..self.i + 7]).unwrap();
                                        let lo = u32::from_str_radix(h2, 16).unwrap();
                                        cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                                        self.i += 6;
                                    }
                                    s.push(char::from_u32(cp).unwrap_or('\u{FFFD}'));
                                }
                                c => panic!("bad escape {}", c as char),
                            }
                            self.i += 1;
                        }
                        _ => {
                            // UTF-8 をそのまま通す
                            let start = self.i;
                            let len = utf8_len(self.b[self.i]);
                            self.i += len;
                            s.push_str(std::str::from_utf8(&self.b[start..self.i]).unwrap());
                        }
                    }
                }
                Json::Str(s)
            }
            b't' => {
                self.i += 4;
                Json::Bool(true)
            }
            b'f' => {
                self.i += 5;
                Json::Bool(false)
            }
            b'n' => {
                self.i += 4;
                Json::Null
            }
            _ => {
                let start = self.i;
                while self.i < self.b.len()
                    && matches!(self.b[self.i], b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E')
                {
                    self.i += 1;
                }
                Json::Num(std::str::from_utf8(&self.b[start..self.i]).unwrap().parse().unwrap())
            }
        }
    }
}
fn utf8_len(b: u8) -> usize {
    if b < 0x80 {
        1
    } else if b >> 5 == 0b110 {
        2
    } else if b >> 4 == 0b1110 {
        3
    } else {
        4
    }
}
impl Json {
    fn get(&self, k: &str) -> &Json {
        match self {
            Json::Obj(o) => &o.iter().find(|(n, _)| n == k).unwrap().1,
            _ => panic!("not obj"),
        }
    }
    fn arr(&self) -> &[Json] {
        match self {
            Json::Arr(a) => a,
            _ => panic!("not arr: {:?}", self),
        }
    }
    fn s(&self) -> &str {
        match self {
            Json::Str(s) => s,
            _ => panic!("not str: {:?}", self),
        }
    }
    fn n(&self) -> i64 {
        match self {
            Json::Num(n) => *n as i64,
            _ => panic!("not num"),
        }
    }
    fn strs(&self) -> Vec<String> {
        self.arr().iter().map(|x| x.s().to_string()).collect()
    }
}

// ---------------------------------------------------------------- 計測基盤
#[derive(Default)]
struct Stats {
    name: &'static str,
    total: usize,
    matched: usize,
    subset: usize, // dates 用: ゴールデン ⊆ Rust (完全一致を含む)
    misses: Vec<String>,
}
impl Stats {
    fn new(name: &'static str) -> Self {
        Stats { name, ..Default::default() }
    }
    fn check(&mut self, label: &str, got: &[String], want: &[String]) {
        self.total += 1;
        if got == want {
            self.matched += 1;
            self.subset += 1;
        } else {
            if want.iter().all(|w| got.contains(w)) {
                self.subset += 1;
            }
            self.misses.push(format!("  MISS {label}\n    want: {want:?}\n    got:  {got:?}"));
        }
    }
    fn report(&self) {
        println!(
            "[{}] total={} exact={} ({}%) subset={} ({}%) miss={}",
            self.name,
            self.total,
            self.matched,
            pct(self.matched, self.total),
            self.subset,
            pct(self.subset, self.total),
            self.total - self.matched
        );
        for m in &self.misses {
            println!("{m}");
        }
    }
}
fn pct(a: usize, b: usize) -> String {
    if b == 0 {
        return "-".into();
    }
    format!("{:.1}", a as f64 * 100.0 / b as f64)
}

fn repo_root() -> std::path::PathBuf {
    // rust/crates/rext-core -> リポジトリルート
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .unwrap()
        .to_path_buf()
}
fn load_golden() -> Json {
    let p = repo_root().join("android/app/src/test/resources/golden.json");
    let text = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("golden.json が読めない {p:?}: {e}"));
    let mut p = P { b: text.as_bytes(), i: 0 };
    p.val()
}
fn to_strs(v: &[rext_core::U16]) -> Vec<String> {
    v.iter().map(|x| from_u16s(x)).collect()
}

/// [y, m, d, hh, mm] (ローカル, JST 前提) -> datetime_predict 用 UNIX 時刻。
/// Rust 側の既定オフセットは UTC+9 なので、ローカル壁時計から 9 時間引く。
fn unix_of(y: i64, m: i64, d: i64, hh: i64, mi: i64) -> i64 {
    days_from_civil(y, m, d) * 86400 + hh * 3600 + mi * 60 - 9 * 3600
}
fn days_from_civil(y0: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y0 - 1 } else { y0 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

// ---------------------------------------------------------------- golden.json: numbers
#[test]
fn golden_numbers() {
    let g = load_golden();
    let mut st = Stats::new("golden numbers (NumberFormatter.predict vs number::predict)");
    for c in g.get("numbers").arr() {
        let input = c.arr()[0].s();
        let want = c.arr()[1].strs();
        let got = to_strs(&number_predict(&u16s(input)));
        st.check(&format!("predict({input:?})"), &got, &want);
    }
    st.report();
    assert_eq!(st.total, 26, "golden numbers の件数が想定と違う");
    // 完全一致を仕様として固定する (実測で全件一致を確認済み。不一致が出たら
    // ここで落ちて気づけるようにする)
    assert_eq!(st.matched, st.total, "numbers に不一致: {:?}", st.misses);
}

// ---------------------------------------------------------------- golden.json: romaji
#[test]
fn golden_romaji() {
    let g = load_golden();
    let mut keys = Stats::new("golden romaji keystrokes (JapaneseConverter.processKey vs Romaji::process)");
    let mut words = Stats::new("golden romaji words (全打鍵一致した語)");
    for c in g.get("romaji").arr() {
        let word = c.arr()[0].s().to_string();
        let want_steps = c.arr()[1].arr();
        let mut r = Romaji::new();
        let mut word_ok = true;
        for (k, ch) in word.chars().enumerate() {
            let res = r.process(ch as u16);
            let got = vec![from_u16s(&res.committed), from_u16s(&res.pending)];
            let want = want_steps[k].strs();
            let before = keys.matched;
            keys.check(&format!("{word} 打鍵{}", k + 1), &got, &want);
            if keys.matched == before {
                word_ok = false;
            }
        }
        words.total += 1;
        if word_ok {
            words.matched += 1;
        } else {
            words.misses.push(format!("  MISS word {word}"));
        }
    }
    words.subset = words.matched;
    keys.report();
    words.report();
    assert_eq!(words.total, 33);
    // 2026-11-27 ユーザー決定で HarmonyOS 版に統一済み: Rust 表から q 行
    // (qa->くぁ 等) を外し、'q' は HarmonyOS 版 JapaneseConverter と同じく
    // 表に無いキーとしてそのまま確定する。全打鍵の完全一致を固定する。
    assert_eq!(keys.matched, keys.total, "romaji に不一致: {:?}", keys.misses);
    assert_eq!(words.matched, 33);
}

// ---------------------------------------------------------------- golden.json: dates
/// want が got の中に同順序のまま (部分列として) すべて含まれるか。
fn is_subsequence(want: &[String], got: &[String]) -> bool {
    let mut it = got.iter();
    want.iter().all(|w| it.by_ref().any(|g| g == w))
}

#[test]
fn golden_dates() {
    let g = load_golden();
    let mut st = Stats::new("golden dates (DateTimePredictor.predict vs datetime_predict)");
    let mut subseq = 0usize;
    let mut subseq_misses: Vec<String> = Vec::new();
    for c in g.get("dates").arr() {
        let t = c.arr()[0].arr();
        let (y, mo, d, hh, mi) = (t[0].n(), t[1].n(), t[2].n(), t[3].n(), t[4].n());
        let reading = c.arr()[1].s();
        let want = c.arr()[2].strs();
        let got = to_strs(&datetime_predict(&u16s(reading), unix_of(y, mo, d, hh, mi)));
        let label = format!("predict({reading:?} @ {y}-{mo}-{d} {hh}:{mi:02})");
        st.check(&label, &got, &want);
        if is_subsequence(&want, &got) {
            subseq += 1;
        } else {
            subseq_misses.push(format!("  MISS {label}\n    want: {want:?}\n    got:  {got:?}"));
        }
    }
    st.report();
    println!("[golden dates 包含 (同順序部分列)] total={} subseq={}", st.total, subseq);
    for m in &subseq_misses {
        println!("{m}");
    }
    assert_eq!(st.total, 207);
    // golden.json は古い HarmonyOS 版ロジック (日 5 形・時刻 3 形) の生成物。
    // 候補の出し方の正は現行 DateTimePredictor.ets (日 最大 12 形・時刻 4 形)
    // で (2026-11-28 ユーザー再決定)、Rust はそちらに揃えているため、
    // golden との完全一致 (st.matched) は下がるのが正しい状態。ここでは
    // 完全一致を要求せず、「ゴールデンの候補が全件・同順序で Rust の候補に
    // 含まれる」こと (包含) を合格条件にする。完全一致の担保は現行 .ets /
    // desktop 版と同内容の core_test_datetime 側で行う。
    assert_eq!(subseq, st.total, "dates にゴールデン欠落 (包含不成立): {:?}", subseq_misses);
}

// ---------------------------------------------------------------- core_test.cpp: romaji / text
fn romaji_run(input: &str) -> String {
    let mut r = Romaji::new();
    let mut out = String::new();
    for ch in input.chars() {
        out.push_str(&from_u16s(&r.process(ch as u16).committed));
    }
    out.push_str(&from_u16s(&r.flush()));
    out
}

#[test]
fn core_test_romaji_text() {
    // desktop/tests/core_test.cpp「ローマ字」節 + halfwidth kana (期待値は同ファイルから転記)
    let mut st = Stats::new("core_test.cpp romaji/text");
    let cases: &[(&str, &str)] = &[
        ("konnnichiha", "こんにちは"),
        ("kanji", "かんじ"),
        ("kan'i", "かんい"),
        ("kitte", "きって"),
        ("z/", "・"),
        ("cocoa", "ここあ"),
        ("hon", "ほん"),
    ];
    for (input, want) in cases {
        st.check(&format!("romaji run({input})"), &[romaji_run(input)], &[want.to_string()]);
    }
    let r = Romaji::new();
    st.check(
        "to_romaji(とうきょうでしゅっぱつ)",
        &[from_u16s(&r.to_romaji(&u16s("とうきょうでしゅっぱつ")))],
        &["toukyoudeshuppatsu".to_string()],
    );
    st.check("to_romaji(かんい)", &[from_u16s(&r.to_romaji(&u16s("かんい")))], &["kanni".to_string()]);
    st.check(
        "halfwidth kana",
        &[from_u16s(&to_halfwidth_kana(&u16s("ガッコウ　パン")))],
        &["ｶﾞｯｺｳ ﾊﾟﾝ".to_string()],
    );
    st.report();
    assert_eq!(st.matched, st.total, "core_test romaji/text に不一致: {:?}", st.misses);
}

// ---------------------------------------------------------------- core_test.cpp: datetime
#[test]
fn core_test_datetime() {
    // 2026-10-17 (土) 14:05 固定。core_test.cpp「日付・時刻」節から転記。
    let now = unix_of(2026, 10, 17, 14, 5);
    let pred = |r: &str| to_strs(&datetime_predict(&u16s(r), now));
    let mut st = Stats::new("core_test.cpp datetime");
    // (読み, 何番目の候補か, 期待値)
    // ※ きょう の候補は現行 DateTimePredictor.ets の dateForms (最大 12 形、
    // desktop/tests/core_test.cpp と同内容) が正 (2026-11-28 ユーザー再決定)。
    // core_test.cpp の期待値をそのまま転記する。
    let cases: &[(&str, usize, &str)] = &[
        ("きょう", 0, "2026年10月17日"),
        ("きょう", 1, "2026年10月17日(土)"),
        ("きょう", 2, "10月17日"),
        ("きょう", 3, "10月17日(土)"),
        ("きょう", 4, "10月17日(土曜日)"),
        ("きょう", 5, "10/17"),
        ("きょう", 6, "10/17(土)"),
        ("きょう", 7, "2026/10/17"),
        ("きょう", 8, "2026-10-17"),
        ("きょう", 9, "令和8年10月17日"),
        ("きょう", 10, "令和8年10月17日(土)"),
        ("きょう", 11, "20261017"),
        ("あした", 0, "2026年10月18日"),
        ("しあさって", 0, "2026年10月20日"),
        ("さきおととい", 0, "2026年10月14日"),
        ("きのう", 0, "2026年10月16日"),
        ("げつようび", 0, "2026年10月19日"),
        ("かようび", 0, "2026年10月20日"),
        ("にちようび", 0, "2026年10月18日"),
        ("げつよう", 0, "2026年10月19日"),
        ("こんしゅうのげつようび", 0, "2026年10月12日"),
        ("らいしゅうのきんようび", 0, "2026年10月23日"),
        ("せんしゅうのすいようび", 0, "2026年10月7日"),
        ("さらいしゅうげつようび", 0, "2026年10月26日"),
        ("こんしゅう", 0, "10月12日〜10月18日"),
        ("こんしゅう", 1, "10/12〜10/18"),
        ("らいしゅう", 0, "10月19日〜10月25日"),
        ("せんしゅう", 0, "10月5日〜10月11日"),
        ("こんげつ", 0, "2026年10月"),
        ("こんげつ", 1, "10月"),
        ("らいげつ", 0, "2026年11月"),
        ("せんげつ", 0, "2026年9月"),
        ("じゅうがつにじゅうにち", 0, "10月20日(火)"),
        ("じゅうがつにじゅうにち", 1, "10月20日(火曜日)"),
        ("じゅうがつにじゅうにち", 2, "10/20(火)"),
        ("じゅうがつにじゅうにち", 3, "2026年10月20日(火)"),
        ("10がつ20にち", 0, "10月20日(火)"),
        ("にがつにじゅうにち", 0, "2月20日(土)"),
        ("2026ねん", 0, "令和8年"),
        ("20261018", 1, "2026年10月18日(日)"),
        ("れいわはちねん", 0, "2026年"),
        ("へいせいにねん", 0, "1990年"),
        ("しょうわごじゅうごねん", 0, "1980年"),
        ("いま", 0, "14:05"),
        ("いま", 1, "14時5分"),
        ("いま", 2, "午後2時5分"),
        ("いま", 3, "10月17日 14:05"),
    ];
    for (r, i, want) in cases {
        let v = pred(r);
        let got = v.get(*i).cloned().unwrap_or_default();
        st.check(&format!("datetime {r}[{i}]"), &[got], &[want.to_string()]);
    }
    // 件数・特殊ケース (きょう は現行 .ets の 12 形。2026-11-28 ユーザー再決定)
    st.check("datetime きょう count==12", &[pred("きょう").len().to_string()], &["12".to_string()]);
    let yb = pred("ようび");
    let joined = if yb.len() >= 3 { format!("{} {} {}", yb[0], yb[1], yb[2]) } else { String::new() };
    st.check("datetime ようび", &[joined], &["土曜日 土 (土)".to_string()]);
    st.check(
        "datetime にがつさんじゅうにち empty",
        &[pred("にがつさんじゅうにち").is_empty().to_string()],
        &["true".to_string()],
    );
    let v = pred("20260230");
    let has_date = v.iter().any(|s| s.contains('年') || s.contains('/'));
    st.check("datetime 20260230 not-a-date", &[(!has_date).to_string()], &["true".to_string()]);
    st.report();
    assert_eq!(st.matched, st.total, "core_test datetime に不一致: {:?}", st.misses);
}

// ---------------------------------------------------------------- core_test.cpp: number_forms
#[test]
fn core_test_number_forms() {
    // core_test.cpp「数字だけの読みの書き換え」節から転記 (has 判定は含むかで数える)
    let mut st = Stats::new("core_test.cpp number_forms");
    let forms = |r: &str| to_strs(&number_forms(&u16s(r)));
    st.check("number 2026 era [0]", &[forms("2026").first().cloned().unwrap_or_default()], &["令和8年".to_string()]);
    for (r, want) in [
        ("2026", "二千二十六"),
        ("2026", "2,026"),
        ("20", "二十"),
        ("20", "⑳"),
        ("20", "⓴"),
        ("50", "㊿"),
        ("21", "㉑"),
        ("36", "㊱"),
        ("0", "⓪"),
        ("12345678", "1234万5678"),
        ("1000000", "100万"),
        ("1000000", "百万"),
    ] {
        st.check(&format!("number {r} has {want}"), &[forms(r).contains(&want.to_string()).to_string()], &["true".to_string()]);
    }
    for r in ["01", "3a", "123456789012345678901"] {
        st.check(&format!("number {r} rejected"), &[forms(r).is_empty().to_string()], &["true".to_string()]);
    }
    st.report();
    assert_eq!(st.matched, st.total, "core_test number_forms に不一致: {:?}", st.misses);
}

// ---------------------------------------------------------------- core_test.cpp: settings / shortcuts
#[test]
fn core_test_settings_shortcuts() {
    use rext_core::settings::{
        build_default_shortcuts, default_shortcuts, load_settings, save_settings, typo_bias_of, Settings,
        CTX_LIMIT,
    };
    let mut st = Stats::new("core_test.cpp settings/shortcuts");
    // typo/ctx の保存・読直し (一時フォルダを使う)
    let dir = std::env::temp_dir().join("rext_golden_match_settings");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("knob_settings.json");
    let mut a = Settings::default();
    a.typo_level = 2;
    a.ctx_left = 10;
    a.ctx_right = 999;
    assert!(save_settings(&path, &a));
    let mut b = Settings::default();
    assert!(load_settings(&path, &mut b));
    let ok = b.typo_level == 2
        && b.ctx_left == 10
        && b.ctx_right == CTX_LIMIT
        && typo_bias_of(2) < 0.0
        && typo_bias_of(0) > 0.0
        && typo_bias_of(1) == 0.0;
    st.check("typo/ctx level saved", &[ok.to_string()], &["true".to_string()]);

    let has = |v: &[(String, String)], k: &str, f: &str| v.iter().any(|p| p.0 == k && p.1 == f);
    let idx = |v: &[(String, String)], k: &str, f: &str| v.iter().position(|p| p.0 == k && p.1 == f);
    fn sc(st: &mut Stats, name: &str, v: &[(String, String)], k: &str, f: &str, want: bool) {
        let found = v.iter().any(|p| p.0 == k && p.1 == f);
        st.check(name, &[found.to_string()], &[want.to_string()]);
    }
    macro_rules! sc {
        ($name:expr, $v:expr, $k:expr, $f:expr, $want:expr) => {
            sc(&mut st, $name, $v, $k, $f, $want)
        };
    }
    let msime = default_shortcuts("msime");
    sc!("shortcut msime Ctrl+U", &msime, "Ctrl+U", "hiragana", true);
    sc!("shortcut msime Ctrl+M", &msime, "Ctrl+M", "confirm", true);
    sc!("shortcut msime Ctrl+Space", &msime, "Ctrl+Space", "ime_toggle", true);
    sc!("shortcut msime Henkan on", &msime, "Henkan", "ime_on", true);
    sc!("shortcut msime Henkan reconvert", &msime, "Henkan", "reconvert", true);
    sc!("shortcut msime Muhenkan off", &msime, "Muhenkan", "ime_off", true);
    let order_ok = match (idx(&msime, "Henkan", "ime_on"), idx(&msime, "Henkan", "reconvert")) {
        (Some(i), Some(j)) => i < j,
        _ => false,
    };
    st.check("shortcut Henkan order", &[order_ok.to_string()], &["true".to_string()]);
    let atok = default_shortcuts("atok");
    sc!("shortcut atok Ctrl+@", &atok, "Ctrl+@", "han_alpha", true);
    sc!("shortcut atok Shift+Henkan", &atok, "Shift+Henkan", "reconvert", true);
    let kotoeri = default_shortcuts("kotoeri");
    sc!("shortcut kotoeri Ctrl+I", &kotoeri, "Ctrl+I", "seg_shrink", true);
    sc!("shortcut kotoeri Ctrl+Shift+R", &kotoeri, "Ctrl+Shift+R", "reconvert", true);
    let off = build_default_shortcuts("msime", "UIOPT", false, false, true);
    sc!("migrate no ctrl_space", &off, "Ctrl+Space", "ime_toggle", false);
    sc!("migrate no muhenkan_off", &off, "Muhenkan", "ime_off", false);
    sc!("migrate reconvert stays", &off, "Henkan", "reconvert", true);
    let custom = build_default_shortcuts("msime", "AB-C-", true, true, true);
    sc!("migrate custom ctrl", &custom, "Ctrl+A", "hiragana", true);
    sc!("migrate custom pos C", &custom, "Ctrl+C", "zen_alpha", true);
    let any_halfkana = custom.iter().any(|p| p.1 == "halfkana");
    st.check("migrate skip - (no halfkana)", &[(!any_halfkana).to_string()], &["true".to_string()]);
    st.report();
    assert_eq!(st.matched, st.total, "core_test settings/shortcuts に不一致: {:?}", st.misses);
}

// ---------------------------------------------------------------- core_test.cpp: layout (chord 節)
#[test]
fn core_test_layout_chord() {
    // core_test.cpp「配列」節の inline TSV 部分から転記。
    // (同梱 layouts/*.tsv の読込と AZIK は対象外としてレポートに明記する)
    fn texts(v: &[rext_core::layout::Out]) -> String {
        let mut t = String::new();
        for o in v {
            if o.key.is_empty() {
                t.push_str(&from_u16s(&o.text));
            } else {
                t.push_str(&format!("{{{}}}", o.key));
            }
        }
        t
    }
    let mut l = Layout::new();
    l.load("#! kind chord\na\tう\nk\tの\nlthumb+a\tを\nrthumb+a\tゔ\na+k\tA\nj+k+l\tX\nj\tj\nk+l\tY\nspace+a\tS\n")
        .expect("layout load");
    let mut got = String::new();
    let mut st = Stats::new("core_test.cpp layout chord");
    macro_rules! d {
        ($k:expr, $t:expr) => {
            got.push_str(&texts(&l.down($k, false, $t)))
        };
    }
    macro_rules! u {
        ($k:expr, $t:expr) => {
            got.push_str(&texts(&l.up($k, $t)))
        };
    }
    macro_rules! tk {
        ($t:expr) => {
            got.push_str(&texts(&l.tick($t)))
        };
    }
    macro_rules! check {
        ($name:expr, $want:expr) => {
            st.check($name, &[got.clone()], &[$want.to_string()]);
            got.clear();
            l.reset();
        };
    }
    d!("a", 0); d!("k", 20); u!("a", 80); u!("k", 85);
    check!("chord: 時間内の同時押し", "A");
    d!("a", 100); tk!(200); u!("a", 210);
    check!("chord: 時間切れは単打", "う");
    d!("a", 300); u!("a", 310);
    check!("chord: 離したら単打", "う");
    d!("a", 400); d!("lthumb", 410); u!("a", 460); u!("lthumb", 470);
    check!("thumb: 文字→親指 (順不同)", "を");
    d!("lthumb", 450); u!("lthumb", 460);
    check!("thumb: 単独は無変換", "{muhenkan}");
    d!("rthumb", 500); d!("a", 520); u!("a", 530); u!("rthumb", 540);
    check!("thumb: 親指→文字", "ゔ");
    d!("j", 600); d!("k", 605); d!("l", 610); u!("j", 670); u!("k", 672); u!("l", 674);
    check!("3 キー", "X");
    d!("space", 700); d!("a", 800); u!("a", 810); u!("space", 820);
    check!("space シフト", "S");
    d!("space", 900); u!("space", 950);
    check!("space 単独", "{space}");
    d!("a", 1000); d!("k", 1200); tk!(1400); u!("a", 1410); u!("k", 1410);
    check!("時間をおいた 2 打は別々", "うの");
    d!("a", 1500); d!("k", 1540); u!("a", 1570); u!("k", 1610);
    check!("速打ちの重なり (ロールオーバー) は別々", "うの");
    d!("a", 1600); d!("j", 1620); u!("j", 1640); tk!(1720);
    check!("押したままの字の中でタップ", "うj");
    u!("a", 1730);
    got.clear();
    l.reset();
    d!("a", 1650); d!("k", 1660); d!("a", 2200); d!("k", 2230); u!("a", 2300); u!("k", 2310);
    check!("組を押したままのリピートは重ならない", "A");
    d!("lthumb", 1700); d!("a", 1720); u!("a", 1780); d!("a", 1800); u!("a", 1860); u!("lthumb", 1900);
    check!("連続シフト", "をを");
    l.continuous = false;
    d!("lthumb", 2400); d!("a", 2420); u!("a", 2480); d!("a", 2500); u!("a", 2560); u!("lthumb", 2600);
    check!("連続シフトを切る: 2 字目は単打", "をう");
    l.continuous = true;
    l.thumbs = 1;
    st.check(
        "map_key space->rthumb / rthumb->none",
        &[format!("{}|{}", l.map_key("space"), l.map_key("rthumb"))],
        &["rthumb|".to_string()],
    );
    d!("rthumb", 2700); d!("a", 2710); u!("a", 2770); u!("rthumb", 2780);
    check!("空白を右親指に: 空白+字", "ゔ");
    d!("rthumb", 2800); u!("rthumb", 2850);
    check!("空白を右親指に: 単独は空白", "{space}");
    l.thumbs = 0;
    l.thumb_tap = false;
    d!("lthumb", 2900); u!("lthumb", 2950);
    check!("親指の単独打鍵を切る", "");
    l.thumb_tap = true;
    l.repeat = false;
    d!("j", 3000); tk!(3200); d!("j", 3500); d!("j", 3530); u!("j", 3600);
    check!("キーリピートを切る", "j");
    l.repeat = true;
    st.report();
    assert_eq!(st.matched, st.total, "core_test layout に不一致: {:?}", st.misses);
}
