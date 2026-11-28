//! 文の流れの予測 (元: desktop/core/flow.cpp + flow.h、
//! entry/src/main/ets/ime/SentenceFlow.ets、android の SentenceFlow.kt)。
//! 食い違いは C++ 版を正とした (保存形式も C++ 版の TSV)。
//!
//! 入力欄に出た文字を句読点・改行・送信で区切った節の並びとして覚え、
//! 節 -> 句読点・節 -> 次の節 の流れのうち 2 回以上あったものを、確定の後に
//! 出す。端末の中だけに保存する (TSV: キー\t続き\t回数)。
//!
//! C++ 版との差:
//! - bump のたびの recent_add (学習の控えへの追記) は store.rs (担当外) の
//!   機能なので、ここでは任意のコールバック [`SentenceFlow::set_recent_hook`]
//!   に逃がした。未設定なら何もしない (流れの学習そのものには影響しない)。
//! - forget_all / forget_recent は Learning・StyleLog (store.rs) が要るため
//!   未移植。forget_ai_learn だけはファイル削除のみなので移植した。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::{from_u16s, u16s, U16};

// 句読点・改行の集合 (元の BOUNDARY: "。．！？!?、，\n")
fn is_boundary(c: u16) -> bool {
    matches!(
        c,
        0x3002 | 0xFF0E | 0xFF01 | 0xFF1F | 0x21 | 0x3F | 0x3001 | 0xFF0C | 0x0A
    )
}
const MAX_CLAUSE: usize = 40;
const MAX_KEYS: usize = 10000; // 以前は 1500 (2026-10-24)
const MIN_COUNT: i32 = 2;

/// 前後の空白 (半角・全角・タブ) を落とす。元の trim と同じ字だけ。
fn trim(s: &[u16]) -> U16 {
    let is_ws = |c: u16| c == 0x20 || c == 0x3000 || c == 0x09;
    let mut a = 0;
    let mut b = s.len();
    while a < b && is_ws(s[a]) {
        a += 1;
    }
    while b > a && is_ws(s[b - 1]) {
        b -= 1;
    }
    s[a..b].to_vec()
}

/// 10 進の整数 (元の to_int: 先頭の空白を飛ばし、符号と数字だけ読む)。
fn to_int(s: &str) -> i32 {
    let b = s.as_bytes();
    let mut i = 0;
    let mut sign = 1i32;
    let mut v = 0i32;
    while i < b.len() && b[i] == b' ' {
        i += 1;
    }
    if i < b.len() && b[i] == b'-' {
        sign = -1;
        i += 1;
    }
    while i < b.len() && b[i].is_ascii_digit() {
        v = v * 10 + (b[i] - b'0') as i32;
        i += 1;
    }
    sign * v
}

fn prefixed(prefix: u16, s: &[u16]) -> U16 {
    let mut o = vec![prefix];
    o.extend_from_slice(s);
    o
}

/// 一時ファイルに書いてから置き換える (元の write_file_atomic の簡略版)。
fn write_file_atomic(p: &Path, data: &str) -> bool {
    use std::io::Write;
    if let Some(parent) = p.parent() {
        if !parent.as_os_str().is_empty() {
            let _ = std::fs::create_dir_all(parent);
        }
    }
    let mut tmp_os = p.as_os_str().to_owned();
    tmp_os.push(format!(".tmp{}", std::process::id()));
    let tmp = PathBuf::from(tmp_os);
    let ok = (|| -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(data.as_bytes())?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, p)
    })()
    .is_ok();
    if !ok {
        let _ = std::fs::remove_file(&tmp);
    }
    ok
}

/// 節 -> 句読点・次の節 の学習と予測。元: SentenceFlow クラス。
pub struct SentenceFlow {
    table: BTreeMap<U16, BTreeMap<U16, i32>>, // "P" + 節 -> 句読点、"N" + 節 -> 次の節、"S" + 表記 -> "1"
    buf: U16,
    prev: U16,
    path: Option<PathBuf>,
    stamp: Option<SystemTime>,
    dirty: bool,
    unread: bool, // ファイルはあるのに読めなかった: 上書きしない
    recent_hook: Option<Box<dyn Fn(&Path, &[u16], &[u16])>>,
}

impl Default for SentenceFlow {
    fn default() -> Self {
        Self::new()
    }
}

impl SentenceFlow {
    pub fn new() -> Self {
        SentenceFlow {
            table: BTreeMap::new(),
            buf: Vec::new(),
            prev: Vec::new(),
            path: None,
            stamp: None,
            dirty: false,
            unread: false,
            recent_hook: None,
        }
    }

    /// bump のたびに呼ばれる任意の控えコールバック (引数: 保存先の親
    /// ディレクトリ、キー、続き)。元の recent_add の受け口。
    pub fn set_recent_hook(&mut self, f: Box<dyn Fn(&Path, &[u16], &[u16])>) {
        self.recent_hook = Some(f);
    }

    /// 学習の中身 (テスト・移行用)。
    pub fn table(&self) -> &BTreeMap<U16, BTreeMap<U16, i32>> {
        &self.table
    }

    /// 学習の中身を丸ごと差し替える (スマホ版 setLearned 相当)。
    pub fn set_table(&mut self, t: BTreeMap<U16, BTreeMap<U16, i32>>) {
        self.table = t;
        self.dirty = true;
    }

    pub fn load(&mut self, path: &Path) {
        self.path = Some(path.to_path_buf());
        self.table.clear();
        self.stamp = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        self.unread = false;
        let data = match std::fs::read(path) {
            Ok(d) => d,
            Err(_) => {
                self.unread = path.exists();
                return;
            }
        };
        // 元の from_utf8 は不正な並びを U+FFFD にする。lossy 変換で寄せる
        let text = String::from_utf8_lossy(&data);
        for line in text.split('\n') {
            let t1 = match line.find('\t') {
                Some(t) => t,
                None => continue,
            };
            let t2 = match line[t1 + 1..].find('\t') {
                Some(t) => t1 + 1 + t,
                None => continue,
            };
            let key = u16s(&line[..t1]);
            let val = u16s(&line[t1 + 1..t2]);
            let count = to_int(&line[t2 + 1..]).max(1);
            self.table.entry(key).or_default().insert(val, count);
        }
    }

    pub fn save(&mut self) {
        if !self.dirty {
            return;
        }
        let path = match &self.path {
            Some(p) => p.clone(),
            None => return,
        };
        if self.unread {
            return;
        }
        self.dirty = false;
        let mut out = String::new();
        for (k, vs) in &self.table {
            for (v, n) in vs {
                out.push_str(&from_u16s(k));
                out.push('\t');
                out.push_str(&from_u16s(v));
                out.push('\t');
                out.push_str(&n.to_string());
                out.push('\n');
            }
        }
        write_file_atomic(&path, &out);
        self.stamp = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
    }

    /// ほかのプロセス・設定の画面がファイルを変えていたら読み直す。
    fn refresh(&mut self) {
        if self.dirty {
            return;
        }
        let path = match &self.path {
            Some(p) => p.clone(),
            None => return,
        };
        if let Ok(m) = std::fs::metadata(&path) {
            if let Ok(t) = m.modified() {
                if Some(t) != self.stamp {
                    self.load(&path);
                }
            }
        }
    }

    pub fn clear(&mut self) {
        self.table.clear();
        self.dirty = true;
        self.save();
        self.reset();
    }

    /// 数えた分を 1 つ戻す (時間を指定した消去)。
    pub fn unbump(&mut self, key: &[u16], v: &[u16]) {
        let remove_key = match self.table.get_mut(key) {
            Some(m) => match m.get_mut(v) {
                Some(n) => {
                    *n -= 1;
                    if *n <= 0 {
                        m.remove(v);
                    }
                    m.is_empty()
                }
                None => return,
            },
            None => return,
        };
        if remove_key {
            self.table.remove(key);
        }
        self.dirty = true;
    }

    /// 別の欄・カーソルを動かした: 途中の節はつながらない。
    pub fn reset(&mut self) {
        self.buf.clear();
        self.prev.clear();
    }

    /// 送信・改行: 文が終わった。
    pub fn boundary(&mut self) {
        self.close(&[]);
        self.prev.clear();
    }

    /// 確定した文字を 1 字消した。
    pub fn backspace(&mut self) {
        if !self.buf.is_empty() {
            self.buf.pop();
        } else {
            self.reset();
        }
    }

    /// 入力欄に文字が出た。
    pub fn feed(&mut self, text: &[u16]) {
        for &ch in text {
            if is_boundary(ch) {
                let punct = if ch == 0x0A { Vec::new() } else { vec![ch] };
                self.close(&punct);
            } else {
                self.buf.push(ch);
                if self.buf.len() > MAX_CLAUSE * 2 {
                    let keep = MAX_CLAUSE * 2;
                    self.buf = self.buf[self.buf.len() - keep..].to_vec();
                }
            }
        }
    }

    fn close(&mut self, punct: &[u16]) {
        self.refresh();
        let c = trim(&self.buf);
        self.buf.clear();
        if c.is_empty() || c.len() > MAX_CLAUSE {
            if c.len() > MAX_CLAUSE {
                self.prev.clear();
            }
            return;
        }
        if !punct.is_empty() {
            let key = prefixed(b'P' as u16, &c);
            self.bump(&key, punct);
        }
        if !self.prev.is_empty() {
            let key = prefixed(b'N' as u16, &self.prev);
            self.bump(&key, &c);
        }
        self.prev = c;
        self.dirty = true;
    }

    fn bump(&mut self, key: &[u16], v: &[u16]) {
        if !self.table.contains_key(key) {
            if self.table.len() >= MAX_KEYS {
                // いちばん使われていないキーを 1 つ捨てる
                let mut worst: Option<U16> = None;
                let mut wn = i32::MAX;
                for (k, vs) in &self.table {
                    let n: i32 = vs.values().sum();
                    if n < wn {
                        wn = n;
                        worst = Some(k.clone());
                    }
                }
                if let Some(w) = worst {
                    self.table.remove(&w);
                }
            }
            self.table.insert(key.to_vec(), BTreeMap::new());
        }
        *self.table.get_mut(key).unwrap().entry(v.to_vec()).or_insert(0) += 1;
        if let (Some(path), Some(hook)) = (&self.path, &self.recent_hook) {
            if let Some(parent) = path.parent() {
                hook(parent, key, v);
            }
        }
    }

    fn top(&self, key: &[u16], n: usize) -> Vec<U16> {
        let m = match self.table.get(key) {
            Some(m) => m,
            None => return Vec::new(),
        };
        let mut v: Vec<(i32, U16)> = m
            .iter()
            .filter(|(_, &count)| count >= MIN_COUNT)
            .map(|(val, &count)| (count, val.clone()))
            .collect();
        v.sort_by(|a, b| b.0.cmp(&a.0)); // 安定ソート (同数はキーの並び = 元の std::map 順)
        v.into_iter().take(n).map(|(_, val)| val).collect()
    }

    /// 確定の後の帯に出す続き (先頭ほど確か)。
    pub fn predict(&self) -> Vec<U16> {
        let cur = trim(&self.buf);
        if !cur.is_empty() {
            return self.top(&prefixed(b'P' as u16, &cur), 2);
        }
        let mut out = Vec::new();
        if self.prev.is_empty() {
            return out;
        }
        for nx in self.top(&prefixed(b'N' as u16, &self.prev), 3) {
            let p = self.top(&prefixed(b'P' as u16, &nx), 1);
            let mut s = nx;
            if let Some(first) = p.into_iter().next() {
                s.extend(first);
            }
            out.push(s);
        }
        out
    }

    /// 提案を選んだ回数 (よく選ぶ提案を前に。同じ表に "S" + 表記 で入れる)。
    pub fn suggest_use(&self, t: &[u16]) -> i32 {
        self.table
            .get(&prefixed(b'S' as u16, t))
            .and_then(|m| m.get(&u16s("1")))
            .copied()
            .unwrap_or(0)
    }

    pub fn record_suggest(&mut self, t: &[u16]) {
        self.refresh();
        let key = prefixed(b'S' as u16, t);
        self.bump(&key, &u16s("1"));
        self.dirty = true;
        self.save();
    }
}

/// AI学習で育てた中身 (どのモデルの分も) だけを消す
/// (ai_learn.bin と ai_learn_<モデルの印>.bin)。元: flow.cpp の forget_ai_learn。
pub fn forget_ai_learn(dir: &Path) {
    let _ = std::fs::remove_file(dir.join("ai_learn.bin"));
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if n.starts_with("ai_learn_") && n.len() > 4 && n.ends_with(".bin") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::from_u16s;

    fn strs(v: Vec<U16>) -> Vec<String> {
        v.iter().map(|s| from_u16s(s)).collect()
    }

    fn feed2(f: &mut SentenceFlow, text: &str) {
        f.feed(&u16s(text));
        f.feed(&u16s(text));
    }

    #[test]
    fn punct_prediction_needs_two() {
        let mut f = SentenceFlow::new();
        f.feed(&u16s("お疲れ様です。"));
        f.feed(&u16s("お疲れ様です"));
        assert!(f.predict().is_empty()); // 1 回だけは出さない
        f.reset();
        f.feed(&u16s("お疲れ様です。"));
        f.feed(&u16s("お疲れ様です"));
        assert_eq!(strs(f.predict()), vec!["。"]);
    }

    #[test]
    fn next_clause_prediction() {
        let mut f = SentenceFlow::new();
        feed2(&mut f, "お疲れ様です。本日の進捗報告です。");
        f.reset();
        f.feed(&u16s("お疲れ様です。"));
        // 句読点の直後: 次の節 + その句読点
        assert_eq!(strs(f.predict()), vec!["本日の進捗報告です。"]);
    }

    #[test]
    fn boundary_and_reset_break_chain() {
        let mut f = SentenceFlow::new();
        feed2(&mut f, "お疲れ様です。本日の進捗報告です。");
        f.reset();
        f.feed(&u16s("お疲れ様です"));
        f.boundary(); // 句読点なしで文が終わった
        assert!(f.predict().is_empty());
    }

    #[test]
    fn backspace_edits_buf() {
        let mut f = SentenceFlow::new();
        feed2(&mut f, "お疲れ様です。");
        f.feed(&u16s("お疲れ様ですあ"));
        f.backspace(); // 余分な 1 字を消すと節がそろう
        assert_eq!(strs(f.predict()), vec!["。"]);
    }

    #[test]
    fn long_clause_not_learned() {
        let mut f = SentenceFlow::new();
        let long = "あ".repeat(41) + "。";
        feed2(&mut f, &long);
        f.feed(&u16s(&"あ".repeat(41)));
        assert!(f.predict().is_empty());
    }

    #[test]
    fn trim_fullwidth_space() {
        let mut f = SentenceFlow::new();
        feed2(&mut f, "　お疲れ様です　。");
        f.feed(&u16s("お疲れ様です"));
        assert_eq!(strs(f.predict()), vec!["。"]);
    }

    #[test]
    fn suggest_counts() {
        let mut f = SentenceFlow::new();
        assert_eq!(f.suggest_use(&u16s("候補")), 0);
        f.record_suggest(&u16s("候補"));
        f.record_suggest(&u16s("候補"));
        assert_eq!(f.suggest_use(&u16s("候補")), 2);
        f.unbump(&prefixed(b'S' as u16, &u16s("候補")), &u16s("1"));
        assert_eq!(f.suggest_use(&u16s("候補")), 1);
    }

    #[test]
    fn tsv_roundtrip() {
        let dir = std::env::temp_dir().join(format!("rext-flow-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("flow.tsv");
        let mut f = SentenceFlow::new();
        f.load(&path);
        feed2(&mut f, "お疲れ様です。本日の進捗報告です。");
        f.save();
        let raw = std::fs::read_to_string(&path).unwrap();
        // TSV 形式: キー\t続き\t回数 (元の形式と互換)
        assert!(raw.contains("Pお疲れ様です\t。\t2\n"), "raw={raw:?}");
        assert!(raw.contains("Nお疲れ様です\t本日の進捗報告です\t2\n"), "raw={raw:?}");
        // 読み直しても予測できる
        let mut g = SentenceFlow::new();
        g.load(&path);
        g.feed(&u16s("お疲れ様です。"));
        assert_eq!(strs(g.predict()), vec!["本日の進捗報告です。"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tsv_load_skips_bad_lines() {
        let dir = std::env::temp_dir().join(format!("rext-flow-test2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("flow.tsv");
        std::fs::write(&path, "Pあ\t。\t3\n壊れた行\nPい\t、\tx\n").unwrap();
        let mut f = SentenceFlow::new();
        f.load(&path);
        assert_eq!(f.table()[&prefixed(b'P' as u16, &u16s("あ"))][&u16s("。")], 3);
        // 回数が読めない行は 1 に丸める (元の max(1, to_int) と同じ)
        assert_eq!(f.table()[&prefixed(b'P' as u16, &u16s("い"))][&u16s("、")], 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn forget_ai_learn_removes_only_ai_files() {
        let dir = std::env::temp_dir().join(format!("rext-flow-test3-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for n in ["ai_learn.bin", "ai_learn_m5.bin", "flow.tsv", "ai_learn.txt"] {
            std::fs::write(dir.join(n), "x").unwrap();
        }
        forget_ai_learn(&dir);
        assert!(!dir.join("ai_learn.bin").exists());
        assert!(!dir.join("ai_learn_m5.bin").exists());
        assert!(dir.join("flow.tsv").exists());
        assert!(dir.join("ai_learn.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
