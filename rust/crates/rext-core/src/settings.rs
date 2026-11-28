//! PC 版の設定 (元の desktop/core/settings.cpp + settings.h の移植)。
//! 設定画面が書き、IME が読む (%APPDATA%\shunti IME\settings.json)。
//! IME はアプリごとのプロセスに読み込まれるので、入力を始めるたびにファイルの更新を見て読み直す。
//!
//! 元は json.h (小さな JSON) と store.h (read_file / keep_broken / write_file_atomic) を使うが、
//! この移植では settings.rs の中に対応する最小実装を持つ (他モジュールの移植を待たないため)。
//! 壊れたファイルは上書きで消さないように別名で残す (store.cpp の keep_broken と同じ)。

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// 文脈を読む字数の上限 (Windows は入力欄から左をこの字数まで読む)
pub const CTX_LIMIT: i32 = 80;

/// 型ごとの Ctrl+字 (ひらがな・カタカナ・半角カナ・全角英数・半角英数)
pub fn keymap_ctrl_chars(keymap: &str) -> &'static str {
    match keymap {
        "atok" => "UIOP@",
        "kotoeri" => "JK;L:",
        _ => "UIOPT",
    }
}

/// 既定のショートカットキーの一覧。ctrl は型ごとの 5 字、ctrl_space・muhenkan_off・reconvert が false の機能は
/// 入れない (古い settings.json を読み替えるときに、オフにしていた機能をそのままにするため)。
pub fn build_default_shortcuts(
    keymap: &str,
    ctrl: &str,
    ctrl_space: bool,
    muhenkan_off: bool,
    reconvert: bool,
) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    const F: [&str; 5] = ["hiragana", "katakana", "halfkana", "zen_alpha", "han_alpha"];
    let ctrl_b = ctrl.as_bytes();
    for i in 0..5 {
        if i >= ctrl_b.len() {
            break;
        }
        if ctrl_b[i] == b'-' {
            continue; // 割り当て無し
        }
        out.push((format!("Ctrl+{}", ctrl_b[i] as char), F[i].to_string()));
    }
    out.push(("Ctrl+M".into(), "confirm".into()));
    if keymap == "kotoeri" {
        out.push(("Ctrl+I".into(), "seg_shrink".into()));
        out.push(("Ctrl+Q".into(), "seg_shrink".into()));
        out.push(("Ctrl+O".into(), "seg_grow".into()));
        out.push(("Ctrl+W".into(), "seg_grow".into()));
    } else {
        out.push(("Ctrl+K".into(), "seg_shrink".into()));
        out.push(("Ctrl+L".into(), "seg_grow".into()));
    }
    out.push(("Ctrl+Delete".into(), "cand_delete".into()));
    out.push(("Ctrl+Backspace".into(), "undo_commit".into()));
    out.push(("Ctrl+Shift+F10".into(), "private_toggle".into()));
    if ctrl_space {
        out.push(("Ctrl+Space".into(), "ime_toggle".into()));
    }
    out.push(("Henkan".into(), "ime_on".into()));
    out.push(("Hiragana".into(), "ime_on".into()));
    if muhenkan_off {
        out.push(("Muhenkan".into(), "ime_off".into()));
    }
    if reconvert {
        if keymap == "atok" {
            out.push(("Shift+Henkan".into(), "reconvert".into()));
        } else if keymap == "kotoeri" {
            out.push(("Ctrl+Shift+R".into(), "reconvert".into()));
        } else {
            out.push(("Henkan".into(), "reconvert".into()));
        }
    }
    out
}

/// 既定のショートカットキー (型ごと)。Settings の既定・古い設定の移し替え・詳細設定の「初期状態に戻す」に使う
pub fn default_shortcuts(keymap: &str) -> Vec<(String, String)> {
    build_default_shortcuts(keymap, keymap_ctrl_chars(keymap), true, true, true)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// 候補の窓の色: auto (Windows に合わせる)・light・dark
    pub theme: String,
    /// 何も打っていないときの空白を全角に (Shift で逆)
    pub space_fullwidth: bool,
    /// 句読点: 0 = 、。 1 = ，． 2 = 、． 3 = ，。
    pub punct: i32,
    /// 数字を全角で入れる
    pub digits_fullwidth: bool,
    /// 打っている間も候補を出す
    pub live: bool,
    /// リアルタイム確定
    pub live_commit: bool,
    /// ライブ変換: 入力中の文字を、空白を押す前から変換して見せる
    pub live_display: bool,
    /// ライブ変換を打つ手が止まってから行う
    pub live_wait: bool,
    /// 大文字で打ち始めたら英字のまま。既定はオフ (2026-11-11)
    pub caps_alpha: bool,
    /// 確定した文を手元に残す (設定の「学習を始める」で同意した人だけ)
    pub learn_log: bool,
    /// 同意を聞く画面を出した (更新の後に 1 度だけ出す)
    pub consent_asked: bool,
    /// 入力中の字の上に読みの帯を出す
    pub typed_strip: bool,
    /// 打ち間違いの直しの強さ (詳細設定): 0 控えめ・1 普通・2 強め (typo_bias_of)
    pub typo_level: i32,
    /// 文脈を読む字数 (詳細設定、0〜CTX_LIMIT): 左 (確定した文)
    pub ctx_left: i32,
    /// 後ろ (固定した前の方を直すときの、後で打った文)
    pub ctx_right: i32,
    /// ローマ字の打ち間違いを直す (オフなら打ったとおり)
    pub typo_fix: bool,
    /// 英字の打ち分け (ローマ字で打った英単語を文脈で英字にする)
    pub latin_auto: bool,
    /// 空白キーで変換しない (空白も読みに入れて変換を続ける)
    pub space_literal: bool,
    /// 句読点 (、。！？) を打ったら確定する
    pub punct_commit: bool,
    /// クリップボードの履歴を残す
    pub clip_history: bool,
    /// AI学習: 確定した文から変換の点に上乗せする小さいモデルを育てる。既定はオフ
    pub ai_learn: bool,
    /// 新デザインの候補バー。既定はオフ
    pub cand_tree: bool,
    /// Ctrl+Space でも日本語入力のオン・オフ (shortcuts に移した。古い設定の読み替え用)
    pub ctrl_space: bool,
    /// 変換モデル: light (XS3・メモリの少ない PC 向け)・standard (S7)
    pub model: String,
    /// 辞書: standard・clean (モデルとは別に選ぶ。pure のモデルはいつも clean)
    pub dict: String,
    // ---- 詳細設定 (2026-10-15) ----
    /// 半角で入れる記号 (! ? ( ) @ など)
    pub half_symbols: String,
    /// テンキー: 0 = いつも半角、1 = 数字の設定に合わせる、2 = いつも全角
    pub numpad: i32,
    /// 候補の窓の 1 ページの数 (5〜9)
    pub page_size: i32,
    /// 無変換キーで日本語入力をオフ (shortcuts に移した。読み替え用)
    pub muhenkan_off: bool,
    /// 選んだ文字を変換キーで変換し直す (再変換。shortcuts に移した。読み替え用)
    pub reconvert: bool,
    /// 初めて使うソフトでの日本語入力
    pub app_default_open: bool,
    /// プライベート: 学習・文の流れ・提案の学習を残さない (Ctrl+Shift+F10 でも切り替え)
    pub private_mode: bool,
    /// 打っている間の Ctrl + 字 (- は割り当て無し。shortcuts に移した。古い設定の読み替え用)
    pub ctrl_keys: String,
    /// キー割り当ての型: msime / atok / kotoeri
    pub keymap: String,
    /// キー配列 (layouts/*.tsv)。空 = ふつうのローマ字入力
    pub layout: String,
    /// 1 つだけ押していて この時間ほかと重ならなければ単打に (ms)
    pub layout_window: i32,
    /// 同時押しとみなす重なり (短い方の押していた時間の %)
    pub layout_overlap: i32,
    /// 親指キー: 0 無変換・変換 / 1 無変換・空白 / 2 空白・変換
    pub layout_thumbs: i32,
    /// 連続シフト
    pub layout_continuous: bool,
    /// 親指キーの単独打鍵を本来のキーとして
    pub layout_thumb_tap: bool,
    /// 押しっぱなしで繰り返す
    pub layout_repeat: bool,
    /// ショートカットキー (既定も自分で追加も同じ一覧)。settings.json には "Ctrl+Shift+K=katakana" の形
    pub shortcuts: Vec<(String, String)>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: "auto".to_string(),
            space_fullwidth: true,
            punct: 0,
            digits_fullwidth: false,
            live: true,
            live_commit: true,
            live_display: false,
            live_wait: false,
            caps_alpha: false,
            learn_log: false,
            consent_asked: false,
            typed_strip: true,
            typo_level: 1,
            ctx_left: 40,
            ctx_right: 80,
            typo_fix: true,
            latin_auto: true,
            space_literal: false,
            punct_commit: false,
            clip_history: true,
            ai_learn: false,
            cand_tree: false,
            ctrl_space: false,
            model: "standard".to_string(),
            dict: "standard".to_string(),
            half_symbols: String::new(),
            numpad: 0,
            page_size: 9,
            muhenkan_off: true,
            reconvert: true,
            app_default_open: true,
            private_mode: false,
            ctrl_keys: "UIOPT".to_string(),
            keymap: "msime".to_string(),
            layout: String::new(),
            layout_window: 100,
            layout_overlap: 50,
            layout_thumbs: 0,
            layout_continuous: true,
            layout_thumb_tap: true,
            layout_repeat: true,
            shortcuts: default_shortcuts("msime"),
        }
    }
}

/// 打ち間違いの直しの強さの段 -> 打ち間違いの辺の点数から引く値 (控えめ 1・普通 0・強め -1)
pub fn typo_bias_of(level: i32) -> f32 {
    if level <= 0 {
        1.0
    } else if level >= 2 {
        -1.0
    } else {
        0.0
    }
}

/// 変換モデルの設定の値 -> モデルのファイル名 (辞書と同じフォルダ)。知らない値は standard
pub fn model_file_name(model: &str) -> &'static str {
    match model {
        "light" => "kkc_model_light.bin",
        "s2" => "kkc_model_s2.bin",   // 比較用の昔のモデル
        "m5" => "kkc_model_m5.bin",   // 正確 (M5、2026-11-11)
        "pure" => "kkc_model_pure.bin", // クリーン (PureXS + クリーン辞書、2026-11-12)
        // 高精度 (M4) は廃止: S7 (標準) の方が精度が上で、誤字も直せる (2026-10-14)
        _ => "kkc_model.bin",
    }
}

/// モデルと組になる辞書 (クリーン版 pure だけ別の辞書。2026-11-12)
pub fn lex_file_name(model: &str, dict: &str) -> &'static str {
    if model == "pure" || dict == "clean" {
        "kkc_lex_clean.bin"
    } else {
        "kkc_lex.bin"
    }
}

/// モデルと組になる書き方の対の表
pub fn variants_file_name(model: &str, dict: &str) -> &'static str {
    if model == "pure" || dict == "clean" {
        "kkc_variants_clean.tsv"
    } else {
        "kkc_variants.tsv"
    }
}

// ---------------------------------------------------------------- 小さな JSON (元の json.h の移植、設定ファイル用)

#[derive(Debug, Clone, PartialEq)]
enum Json {
    Null,
    Bool(bool),
    Number(f64),
    Str(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    fn get(&self, k: &str) -> Option<&Json> {
        match self {
            Json::Object(obj) => obj.iter().find(|(key, _)| key == k).map(|(_, v)| v),
            _ => None,
        }
    }
}

struct JsonParser<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> JsonParser<'a> {
    fn parse(&mut self) -> Option<Json> {
        self.ws();
        let v = self.value()?;
        self.ws();
        if self.i == self.s.len() {
            Some(v)
        } else {
            None
        }
    }

    fn ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn lit(&mut self, w: &str) -> bool {
        if self.s[self.i..].starts_with(w.as_bytes()) {
            self.i += w.len();
            true
        } else {
            false
        }
    }

    fn hex4(&mut self) -> Option<u32> {
        if self.i + 4 > self.s.len() {
            return None;
        }
        let mut v = 0u32;
        for _ in 0..4 {
            let c = self.s[self.i];
            self.i += 1;
            v <<= 4;
            match c {
                b'0'..=b'9' => v |= (c - b'0') as u32,
                b'a'..=b'f' => v |= (c - b'a' + 10) as u32,
                b'A'..=b'F' => v |= (c - b'A' + 10) as u32,
                _ => return None,
            }
        }
        Some(v)
    }

    fn string(&mut self) -> Option<String> {
        if self.i >= self.s.len() || self.s[self.i] != b'"' {
            return None;
        }
        self.i += 1;
        let mut o: Vec<u8> = Vec::new();
        while self.i < self.s.len() {
            let c = self.s[self.i];
            self.i += 1;
            if c == b'"' {
                // 元は UTF-8 バイト列のまま保持する。Rust の String にするので lossy に寄せる
                return Some(String::from_utf8_lossy(&o).into_owned());
            }
            if c != b'\\' {
                o.push(c);
                continue;
            }
            if self.i >= self.s.len() {
                return None;
            }
            let e = self.s[self.i];
            self.i += 1;
            match e {
                b'"' => o.push(b'"'),
                b'\\' => o.push(b'\\'),
                b'/' => o.push(b'/'),
                b'b' => o.push(0x08),
                b'f' => o.push(0x0C),
                b'n' => o.push(b'\n'),
                b'r' => o.push(b'\r'),
                b't' => o.push(b'\t'),
                b'u' => {
                    let mut v = self.hex4()?;
                    if (0xD800..0xDC00).contains(&v)
                        && self.i + 6 <= self.s.len()
                        && self.s[self.i] == b'\\'
                        && self.s[self.i + 1] == b'u'
                    {
                        self.i += 2;
                        let lo = self.hex4()?;
                        v = 0x10000 + ((v - 0xD800) << 10) + lo.wrapping_sub(0xDC00);
                    }
                    let mut buf = [0u8; 4];
                    let ch = char::from_u32(v).unwrap_or('\u{FFFD}');
                    o.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                }
                _ => return None,
            }
        }
        None
    }

    fn value(&mut self) -> Option<Json> {
        if self.i >= self.s.len() {
            return None;
        }
        match self.s[self.i] {
            b'{' => {
                self.i += 1;
                let mut obj: Vec<(String, Json)> = Vec::new();
                self.ws();
                if self.i < self.s.len() && self.s[self.i] == b'}' {
                    self.i += 1;
                    return Some(Json::Object(obj));
                }
                loop {
                    self.ws();
                    let k = self.string()?;
                    self.ws();
                    if self.i >= self.s.len() || self.s[self.i] != b':' {
                        return None;
                    }
                    self.i += 1;
                    self.ws();
                    let x = self.value()?;
                    obj.push((k, x));
                    self.ws();
                    if self.i >= self.s.len() {
                        return None;
                    }
                    if self.s[self.i] == b',' {
                        self.i += 1;
                        continue;
                    }
                    if self.s[self.i] == b'}' {
                        self.i += 1;
                        return Some(Json::Object(obj));
                    }
                    return None;
                }
            }
            b'[' => {
                self.i += 1;
                let mut arr: Vec<Json> = Vec::new();
                self.ws();
                if self.i < self.s.len() && self.s[self.i] == b']' {
                    self.i += 1;
                    return Some(Json::Array(arr));
                }
                loop {
                    self.ws();
                    let x = self.value()?;
                    arr.push(x);
                    self.ws();
                    if self.i >= self.s.len() {
                        return None;
                    }
                    if self.s[self.i] == b',' {
                        self.i += 1;
                        continue;
                    }
                    if self.s[self.i] == b']' {
                        self.i += 1;
                        return Some(Json::Array(arr));
                    }
                    return None;
                }
            }
            b'"' => self.string().map(Json::Str),
            _ => {
                if self.lit("true") {
                    return Some(Json::Bool(true));
                }
                if self.lit("false") {
                    return Some(Json::Bool(false));
                }
                if self.lit("null") {
                    return Some(Json::Null);
                }
                let st = self.i;
                while self.i < self.s.len()
                    && (matches!(self.s[self.i], b'-' | b'+' | b'.' | b'e' | b'E')
                        || self.s[self.i].is_ascii_digit())
                {
                    self.i += 1;
                }
                if st == self.i {
                    return None;
                }
                let txt = std::str::from_utf8(&self.s[st..self.i]).ok()?;
                Some(Json::Number(txt.parse::<f64>().unwrap_or(0.0)))
            }
        }
    }
}

/// 元の json::quote と同じエスケープ
fn json_quote(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.bytes() {
        match c {
            b'"' => o += "\\\"",
            b'\\' => o += "\\\\",
            b'\n' => o += "\\n",
            b'\r' => o += "\\r",
            b'\t' => o += "\\t",
            0..=0x1F => o += &format!("\\u{:04x}", c),
            _ => o.push(c as char),
        }
    }
    o + "\""
}

// ---------------------------------------------------------------- ファイル (元の store.h の該当部分)

fn read_file(p: &Path) -> Option<String> {
    fs::read(p).ok().map(|b| String::from_utf8_lossy(&b).into_owned())
}

/// 読めなかった (壊れた) ファイルを、上書きで消さないように別名で残す (learned.json.broken-<秒>)
fn keep_broken(p: &Path) {
    let t = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut name = p.as_os_str().to_owned();
    name.push(format!(".broken-{}", t));
    let _ = fs::rename(p, PathBuf::from(name));
}

/// 一時ファイルに書いてから置き換える
fn write_file_atomic(p: &Path, data: &str) -> bool {
    if let Some(parent) = p.parent() {
        if !parent.as_os_str().is_empty() {
            let _ = fs::create_dir_all(parent);
        }
    }
    let mut tmp_name = p.as_os_str().to_owned();
    tmp_name.push(format!(".tmp{}", std::process::id()));
    let tmp = PathBuf::from(tmp_name);
    if fs::write(&tmp, data).is_err() {
        return false;
    }
    match fs::rename(&tmp, p) {
        Ok(()) => true,
        Err(_) => {
            let _ = fs::remove_file(&tmp);
            false
        }
    }
}

// ---------------------------------------------------------------- 読み書き

/// 読む (無い・壊れていれば既定のまま)
pub fn load_settings(p: &Path, s: &mut Settings) -> bool {
    *s = Settings::default();
    let text = match read_file(p) {
        Some(t) => t,
        None => return false,
    };
    let v = match (JsonParser {
        s: text.as_bytes(),
        i: 0,
    })
    .parse()
    {
        Some(v @ Json::Object(_)) => v,
        _ => {
            keep_broken(p);
            return false;
        }
    };
    let str_of = |v: &Json, k: &str, out: &mut String| {
        if let Some(Json::Str(x)) = v.get(k) {
            *out = x.clone();
        }
    };
    let bool_of = |v: &Json, k: &str, out: &mut bool| {
        if let Some(Json::Bool(x)) = v.get(k) {
            *out = *x;
        }
    };
    let num_of = |v: &Json, k: &str| -> Option<f64> {
        match v.get(k) {
            Some(Json::Number(n)) => Some(*n),
            _ => None,
        }
    };
    str_of(&v, "theme", &mut s.theme);
    bool_of(&v, "space_fullwidth", &mut s.space_fullwidth);
    bool_of(&v, "digits_fullwidth", &mut s.digits_fullwidth);
    bool_of(&v, "live", &mut s.live);
    bool_of(&v, "live_commit", &mut s.live_commit);
    bool_of(&v, "live_display", &mut s.live_display);
    bool_of(&v, "live_wait", &mut s.live_wait);
    bool_of(&v, "cand_tree", &mut s.cand_tree);
    bool_of(&v, "space_literal", &mut s.space_literal);
    bool_of(&v, "punct_commit", &mut s.punct_commit);
    bool_of(&v, "clip_history", &mut s.clip_history);
    bool_of(&v, "ai_learn", &mut s.ai_learn);
    bool_of(&v, "latin_auto", &mut s.latin_auto);
    bool_of(&v, "typo_fix", &mut s.typo_fix);
    bool_of(&v, "learn_log", &mut s.learn_log);
    bool_of(&v, "consent_asked", &mut s.consent_asked);
    bool_of(&v, "typed_strip", &mut s.typed_strip);
    bool_of(&v, "caps_alpha", &mut s.caps_alpha);
    bool_of(&v, "ctrl_space", &mut s.ctrl_space);
    str_of(&v, "model", &mut s.model);
    str_of(&v, "dict", &mut s.dict);
    if let Some(n) = num_of(&v, "punct") {
        s.punct = (n as i32) & 3;
    }
    str_of(&v, "half_symbols", &mut s.half_symbols);
    if let Some(n) = num_of(&v, "typo_level") {
        s.typo_level = (n as i32).clamp(0, 2);
    }
    if let Some(n) = num_of(&v, "ctx_left") {
        s.ctx_left = (n as i32).clamp(0, CTX_LIMIT);
    }
    if let Some(n) = num_of(&v, "ctx_right") {
        s.ctx_right = (n as i32).clamp(0, CTX_LIMIT);
    }
    if let Some(n) = num_of(&v, "numpad") {
        s.numpad = (n as i32).clamp(0, 2);
    }
    if let Some(n) = num_of(&v, "page_size") {
        s.page_size = (n as i32).clamp(5, 9);
    }
    bool_of(&v, "muhenkan_off", &mut s.muhenkan_off);
    bool_of(&v, "reconvert", &mut s.reconvert);
    bool_of(&v, "private_mode", &mut s.private_mode);
    bool_of(&v, "app_default_open", &mut s.app_default_open);
    str_of(&v, "keymap", &mut s.keymap);
    match v.get("ctrl_keys") {
        Some(Json::Str(x)) => s.ctrl_keys = x.clone(),
        _ => s.ctrl_keys = keymap_ctrl_chars(&s.keymap).to_string(),
    }
    if s.ctrl_keys.len() != 5 {
        s.ctrl_keys = keymap_ctrl_chars(&s.keymap).to_string();
    }
    {
        let mut fixed = String::new();
        for ch in s.ctrl_keys.chars() {
            let up = if ('a'..='z').contains(&ch) {
                ch.to_ascii_uppercase()
            } else {
                ch
            };
            fixed.push(if ('A'..='Z').contains(&up) || up == '@' || up == ';' || up == ':' || up == '-' {
                up
            } else {
                '-'
            });
        }
        s.ctrl_keys = fixed;
    }
    str_of(&v, "layout", &mut s.layout);
    if let Some(n) = num_of(&v, "layout_window") {
        s.layout_window = (n as i32).clamp(20, 400);
    }
    if let Some(n) = num_of(&v, "layout_overlap") {
        s.layout_overlap = (n as i32).clamp(10, 95);
    }
    if let Some(n) = num_of(&v, "layout_thumbs") {
        s.layout_thumbs = (n as i32).clamp(0, 2);
    }
    bool_of(&v, "layout_continuous", &mut s.layout_continuous);
    bool_of(&v, "layout_thumb_tap", &mut s.layout_thumb_tap);
    bool_of(&v, "layout_repeat", &mut s.layout_repeat);
    // キー割り当ての一覧。shortcuts_v が 2 ならそのまま、それ以外 (古い設定) は既定を今の設定に合わせて作り、
    // その後ろに自分で足したものを付ける (移し替え)
    let mut listed: Vec<(String, String)> = Vec::new();
    if let Some(Json::Array(arr)) = v.get("shortcuts") {
        for e in arr {
            let es = match e {
                Json::Str(x) => x,
                _ => continue, // 読めない項目は飛ばす
            };
            if let Some(eq) = es.find('=') {
                if eq == 0 || eq + 1 == es.len() {
                    continue;
                }
                listed.push((es[..eq].to_string(), es[eq + 1..].to_string()));
            }
        }
    }
    if s.keymap != "atok" && s.keymap != "kotoeri" {
        s.keymap = "msime".to_string();
    }
    let shortcuts_v = num_of(&v, "shortcuts_v").map(|n| n as i32).unwrap_or(0);
    if shortcuts_v == 2 {
        s.shortcuts = listed;
    } else {
        let mut defs = build_default_shortcuts(
            &s.keymap,
            &s.ctrl_keys,
            s.ctrl_space,
            s.muhenkan_off,
            s.reconvert,
        );
        defs.extend(listed);
        s.shortcuts = defs;
    }
    true
}

/// 書く (一時ファイルから置き換える)
pub fn save_settings(p: &Path, s: &Settings) -> bool {
    let b = |x: bool| if x { "true" } else { "false" };
    let mut o = String::from("{\n");
    o += &format!("  \"theme\": {},\n", json_quote(&s.theme));
    o += &format!("  \"space_fullwidth\": {},\n", b(s.space_fullwidth));
    o += &format!("  \"punct\": {},\n", s.punct);
    o += &format!("  \"digits_fullwidth\": {},\n", b(s.digits_fullwidth));
    o += &format!("  \"live\": {},\n", b(s.live));
    o += &format!("  \"live_commit\": {},\n", b(s.live_commit));
    o += &format!("  \"live_display\": {},\n", b(s.live_display));
    o += &format!("  \"live_wait\": {},\n", b(s.live_wait));
    o += &format!("  \"cand_tree\": {},\n", b(s.cand_tree));
    o += &format!("  \"space_literal\": {},\n", b(s.space_literal));
    o += &format!("  \"punct_commit\": {},\n", b(s.punct_commit));
    o += &format!("  \"clip_history\": {},\n", b(s.clip_history));
    o += &format!("  \"ai_learn\": {},\n", b(s.ai_learn));
    o += &format!("  \"latin_auto\": {},\n", b(s.latin_auto));
    o += &format!("  \"typo_fix\": {},\n", b(s.typo_fix));
    o += &format!("  \"learn_log\": {},\n", b(s.learn_log));
    o += &format!("  \"consent_asked\": {},\n", b(s.consent_asked));
    o += &format!("  \"typed_strip\": {},\n", b(s.typed_strip));
    o += &format!("  \"caps_alpha\": {},\n", b(s.caps_alpha));
    o += &format!("  \"model\": {},\n", json_quote(&s.model));
    o += &format!("  \"dict\": {},\n", json_quote(&s.dict));
    o += &format!("  \"half_symbols\": {},\n", json_quote(&s.half_symbols));
    o += &format!("  \"numpad\": {},\n", s.numpad);
    o += &format!("  \"typo_level\": {},\n", s.typo_level);
    o += &format!("  \"ctx_left\": {},\n", s.ctx_left);
    o += &format!("  \"ctx_right\": {},\n", s.ctx_right);
    o += &format!("  \"page_size\": {},\n", s.page_size);
    o += &format!("  \"private_mode\": {},\n", b(s.private_mode));
    o += &format!("  \"app_default_open\": {},\n", b(s.app_default_open));
    o += &format!("  \"keymap\": {},\n", json_quote(&s.keymap));
    o += &format!("  \"layout\": {},\n", json_quote(&s.layout));
    o += &format!("  \"layout_window\": {},\n", s.layout_window);
    o += &format!("  \"layout_overlap\": {},\n", s.layout_overlap);
    o += &format!("  \"layout_thumbs\": {},\n", s.layout_thumbs);
    o += &format!("  \"layout_continuous\": {},\n", b(s.layout_continuous));
    o += &format!("  \"layout_thumb_tap\": {},\n", b(s.layout_thumb_tap));
    o += &format!("  \"layout_repeat\": {},\n", b(s.layout_repeat));
    o += "  \"shortcuts_v\": 2,\n";
    o += "  \"shortcuts\": [";
    for (i, (k, f)) in s.shortcuts.iter().enumerate() {
        if i > 0 {
            o += ", ";
        }
        o += &json_quote(&format!("{}={}", k, f));
    }
    o += "]\n";
    o += "}\n";
    write_file_atomic(p, &o)
}

/// ファイルが変わっていたら読み直すための控え
pub struct SettingsFile {
    path_: PathBuf,
    stamp_: Option<SystemTime>,
    loaded_: bool,
    s_: Settings,
}

impl SettingsFile {
    pub fn new(p: PathBuf) -> SettingsFile {
        SettingsFile {
            path_: p,
            stamp_: None,
            loaded_: false,
            s_: Settings::default(),
        }
    }

    /// 変わっていれば読み直して true
    pub fn refresh(&mut self) -> bool {
        let t = fs::metadata(&self.path_).and_then(|m| m.modified()).ok();
        let exists = t.is_some();
        if self.loaded_ && (!exists || t == self.stamp_) {
            return false;
        }
        if exists && read_file(&self.path_).is_none() {
            return false; // 読めないだけ: 前の設定のまま、次にまた読む
        }
        self.loaded_ = true;
        self.stamp_ = t;
        load_settings(&self.path_, &mut self.s_);
        true
    }

    pub fn get(&self) -> &Settings {
        &self.s_
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("rext-settings-test-{}-{}.json", std::process::id(), tag))
    }

    #[test]
    fn defaults_match_header() {
        let s = Settings::default();
        assert_eq!(s.theme, "auto");
        assert!(s.space_fullwidth);
        assert_eq!(s.punct, 0);
        assert!(s.live && s.live_commit && !s.live_display && !s.live_wait);
        assert!(!s.caps_alpha);
        assert_eq!(s.typo_level, 1);
        assert_eq!(s.ctx_left, 40);
        assert_eq!(s.ctx_right, 80);
        assert!(s.typo_fix && s.latin_auto && s.clip_history);
        assert_eq!(s.model, "standard");
        assert_eq!(s.dict, "standard");
        assert_eq!(s.page_size, 9);
        assert_eq!(s.ctrl_keys, "UIOPT");
        assert_eq!(s.keymap, "msime");
        assert_eq!(s.layout_window, 100);
        assert_eq!(s.layout_overlap, 50);
        assert_eq!(s.shortcuts, default_shortcuts("msime"));
    }

    #[test]
    fn default_shortcuts_keymaps() {
        let ms = default_shortcuts("msime");
        assert!(ms.contains(&("Ctrl+T".into(), "han_alpha".into())));
        assert!(ms.contains(&("Ctrl+K".into(), "seg_shrink".into())));
        assert!(ms.contains(&("Henkan".into(), "reconvert".into())));
        let atok = default_shortcuts("atok");
        assert!(atok.contains(&("Ctrl+O".into(), "halfkana".into())));
        assert!(atok.contains(&("Ctrl+@".into(), "han_alpha".into())));
        assert!(atok.contains(&("Shift+Henkan".into(), "reconvert".into())));
        let kot = default_shortcuts("kotoeri");
        assert!(kot.contains(&("Ctrl+J".into(), "hiragana".into())));
        assert!(kot.contains(&("Ctrl+I".into(), "seg_shrink".into())));
        assert!(kot.contains(&("Ctrl+Shift+R".into(), "reconvert".into())));
        // オフの機能は入れない
        let off = build_default_shortcuts("msime", "UIOP-", false, false, false);
        assert!(!off.iter().any(|(_, f)| f == "ime_toggle"));
        assert!(!off.iter().any(|(_, f)| f == "ime_off"));
        assert!(!off.iter().any(|(_, f)| f == "reconvert"));
        assert!(!off.iter().any(|(_, f)| f == "han_alpha")); // ctrl の 5 字目が '-'
    }

    #[test]
    fn file_names_and_bias() {
        assert_eq!(model_file_name("light"), "kkc_model_light.bin");
        assert_eq!(model_file_name("pure"), "kkc_model_pure.bin");
        assert_eq!(model_file_name("high"), "kkc_model.bin"); // 廃止した値は標準
        assert_eq!(model_file_name("standard"), "kkc_model.bin");
        assert_eq!(lex_file_name("pure", "standard"), "kkc_lex_clean.bin");
        assert_eq!(lex_file_name("standard", "clean"), "kkc_lex_clean.bin");
        assert_eq!(lex_file_name("standard", "standard"), "kkc_lex.bin");
        assert_eq!(variants_file_name("standard", "clean"), "kkc_variants_clean.tsv");
        assert_eq!(typo_bias_of(0), 1.0);
        assert_eq!(typo_bias_of(1), 0.0);
        assert_eq!(typo_bias_of(2), -1.0);
    }

    #[test]
    fn save_load_roundtrip() {
        let p = tmp_path("roundtrip");
        let mut s = Settings::default();
        s.theme = "dark".into();
        s.punct = 3;
        s.typo_level = 2;
        s.ctx_left = 10;
        s.keymap = "atok".into();
        s.ctrl_keys = "UIOP@".into();
        s.shortcuts = vec![("Ctrl+Q".into(), "katakana".into())];
        assert!(save_settings(&p, &s));
        let mut back = Settings::default();
        assert!(load_settings(&p, &mut back));
        assert_eq!(back.theme, "dark");
        assert_eq!(back.punct, 3);
        assert_eq!(back.typo_level, 2);
        assert_eq!(back.ctx_left, 10);
        assert_eq!(back.keymap, "atok");
        assert_eq!(back.ctrl_keys, "UIOP@");
        assert_eq!(back.shortcuts, s.shortcuts); // shortcuts_v=2 なのでそのまま
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn load_clamps_and_ctrl_keys_fix() {
        let p = tmp_path("clamp");
        fs::write(&p, "{\"punct\": 7, \"typo_level\": 9, \"page_size\": 3, \"ctx_left\": 999, \"layout_window\": 5, \"ctrl_keys\": \"uio1~\"}").unwrap();
        let mut s = Settings::default();
        assert!(load_settings(&p, &mut s));
        assert_eq!(s.punct, 3); // & 3
        assert_eq!(s.typo_level, 2);
        assert_eq!(s.page_size, 5);
        assert_eq!(s.ctx_left, CTX_LIMIT);
        assert_eq!(s.layout_window, 20);
        assert_eq!(s.ctrl_keys, "UIO--"); // 小文字は大文字に、英字・@;:・- 以外は割り当て不可で '-'
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn load_old_shortcuts_migration() {
        let p = tmp_path("old");
        // shortcuts_v が無い古い設定: 既定 (今の keymap・ctrl_keys・各フラグで作る) + 自分で足したもの
        fs::write(&p, "{\"keymap\": \"kotoeri\", \"shortcuts\": [\"Ctrl+Z=my_func\", \"壊れた項目\", \"=x\", \"y=\"]}").unwrap();
        let mut s = Settings::default();
        assert!(load_settings(&p, &mut s));
        let defs = build_default_shortcuts("kotoeri", "JK;L:", false, true, true);
        assert_eq!(s.shortcuts.len(), defs.len() + 1);
        assert_eq!(s.shortcuts[defs.len()], ("Ctrl+Z".into(), "my_func".into()));
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn load_missing_and_broken() {
        let p = tmp_path("missing");
        let _ = fs::remove_file(&p);
        let mut s = Settings::default();
        assert!(!load_settings(&p, &mut s));
        assert_eq!(s, Settings::default());
        // 壊れた JSON: 既定のまま + 元ファイルは .broken-<秒> に残る
        fs::write(&p, "{これは壊れている").unwrap();
        assert!(!load_settings(&p, &mut s));
        assert!(!p.exists());
    }

    #[test]
    fn settings_file_refresh() {
        let p = tmp_path("refresh");
        let _ = fs::remove_file(&p);
        let mut f = SettingsFile::new(p.clone());
        assert!(f.refresh()); // 初回は読む (ファイルが無くても既定で確定)
        assert!(!f.refresh()); // 変わっていない
        let mut s = Settings::default();
        s.theme = "light".into();
        assert!(save_settings(&p, &s));
        assert!(f.refresh());
        assert_eq!(f.get().theme, "light");
        assert!(!f.refresh());
        let _ = fs::remove_file(&p);
    }
}
