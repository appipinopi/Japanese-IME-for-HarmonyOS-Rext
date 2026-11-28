//! キー配列 (元の desktop/core/layout.cpp + layout.h の移植)。
//! PC の物理キーの並びを別の配列として読む。大西配列・Dvorak (英字の入れ替え)、JIS かな (かなを直接)、
//! NICOLA・薙刀式 (同時押し) を、同じ形の表 (layouts/*.tsv) で書く。
//!
//! 表の形 (1 行 1 つ、# で始まる行は説明。キーは US 配列の物理位置の名前):
//!   #! name 配列の名前 / #! kind letters|kana|chord / #! author 作者 / #! url 公式ページ
//!   a<TAB>出る字              単打
//!   S-a<TAB>出る字            Shift と
//!   a+j<TAB>出る字            2 キーの同時押し (順番は問わない)
//!   a+j+k<TAB>出る字          3 キーの同時押し
//!   space+a / lthumb+a / rthumb+a<TAB>出る字   シフト (押している間ずっと効く)
//!   d<TAB>{p1}  と  p1>e<TAB>出る字           前置シフト (月配列など)
//! #! kind romaji はローマ字の表に綴りを足す・上書きする (綴り<TAB>かな。AZIK など)。
//!   「#! keys」の行の後は letters と同じ形のキーの読み替え、「#! base empty」は元のローマ字の表を使わない。
//! {bs} {enter} {left} {right} は打鍵として返す。

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::{u16s, U16};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    #[default]
    Letters,
    Kana,
    Chord,
    Romaji,
}

/// 判定の結果: 出す字 (text) か、打鍵 (key: "bs" など)
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Out {
    pub text: U16,
    pub key: String,
    /// text を出した打鍵の単位 (表の書き方: a・S-a・a+j・lthumb+a・p1>e)。打ち間違いの直しに使う
    pub unit: String,
}

const SHIFTS: [&str; 3] = ["space", "lthumb", "rthumb"];

fn is_shift_key(k: &str) -> bool {
    SHIFTS.contains(&k)
}

fn split(s: &str, sep: char) -> Vec<String> {
    s.split(sep).map(|x| x.to_string()).collect()
}

/// キーを名前順に並べて + でつなぐ (同時押しは順番を問わない)
fn chord_key(mut keys: Vec<String>) -> String {
    keys.sort();
    keys.join("+")
}

/// 10 進の整数 (元の to_int と同じ: 空白を飛ばし、符号と数字だけ読む)
fn to_int(s: &str) -> i32 {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() && b[i] == b' ' {
        i += 1;
    }
    let mut sign = 1i32;
    if i < b.len() && b[i] == b'-' {
        sign = -1;
        i += 1;
    }
    let mut v = 0i32;
    while i < b.len() && b[i].is_ascii_digit() {
        v = v * 10 + (b[i] - b'0') as i32;
        i += 1;
    }
    sign * v
}

#[derive(Debug, Clone)]
struct Pending {
    key: String,
    shift: bool,
    t: u64,
}

/// 同時押しの配列で、まだ決まっていない打鍵 (押した順)
#[derive(Debug, Clone, Default)]
struct Ev {
    key: String,
    shift: bool,
    is_shift: bool, // 押している間効くシフト (space・lthumb・rthumb)
    down: u64,
    up: u64,
    released: bool,
    decided: bool,
    used: bool,
    shift_id: i32, // シフトなら、shifts_ の記録の番号
    out: Vec<Out>,
}

/// シフト (space・親指) を押した記録。組んだあとも押している間は残す (連続シフト)
#[derive(Debug, Clone)]
struct ShiftRec {
    id: i32,
    key: String,
    down: u64,
    up: u64,
    released: bool,
    used: bool,
}

fn ev_at(evs: &[Ev], sh: &[Ev], n_ev: usize, j: usize) -> Ev {
    if j < n_ev {
        evs[j].clone()
    } else {
        sh[j - n_ev].clone()
    }
}

fn ev_end(e: &Ev, now: u64) -> u64 {
    if e.released {
        e.up
    } else {
        now
    }
}

fn ev_dur(e: &Ev, now: u64) -> f64 {
    ev_end(e, now).wrapping_sub(e.down) as f64
}

fn ev_overlap(a: &Ev, b: &Ev, now: u64) -> f64 {
    let s = a.down.max(b.down);
    let en = ev_end(a, now).min(ev_end(b, now));
    if en > s {
        (en - s) as f64
    } else {
        0.0
    }
}

/// a から見た b との重なりの割合 (b がまだ押されているなら a の押していた時間に対して、両方離していれば短い方に対して)
fn ev_ratio(a: &Ev, b: &Ev, now: u64) -> f64 {
    let base = if b.released {
        ev_dur(a, now).min(ev_dur(b, now))
    } else {
        ev_dur(a, now)
    };
    if base > 0.0 {
        ev_overlap(a, b, now) / base
    } else {
        0.0
    }
}

#[derive(Debug, Clone)]
pub struct Layout {
    /// 1 つだけ押していて、ほかと重ならないまま この時間がたてば単打に決める
    pub window_ms: u64,
    /// 同時押し (chord の配列) は、押していた時間の重なりで決める: 2 つのキーの重なりが、短い方
    /// (まだ押している相手なら先に離した方) の押していた時間のこの割合以上なら組
    pub overlap_ratio: f64,
    /// 親指キー: 0 = 無変換・変換、1 = 無変換・空白 (空白が右親指)、2 = 空白・変換 (空白が左親指)
    pub thumbs: i32,
    /// 連続シフト: 親指・空白を押したまま続けて打てば、続けてシフト
    pub continuous: bool,
    /// 親指キーを単独で押したら、本来のキー (空白・変換・無変換) として働く
    pub thumb_tap: bool,
    /// 押しっぱなしで繰り返す
    pub repeat: bool,

    evs_: VecDeque<Ev>,
    shifts_: Vec<ShiftRec>,
    next_shift_id_: i32,
    kind_: Kind,
    name_: String,
    author_: String,
    url_: String,
    single_: BTreeMap<String, U16>,
    shifted_: BTreeMap<String, U16>,
    chords_: BTreeMap<String, U16>, // キーは キーを名前順に + でつないだもの
    chord_keys_: BTreeSet<String>,  // 同時押しに出てくるキー (それ以外は待たずに出す)
    shift_keys_: BTreeSet<String>,  // 押している間効くシフト (表に出てくるもの)
    pending_: Vec<Pending>,
    held_: BTreeSet<String>, // いま押されているキー
    shift_used_: bool,       // シフトを押している間に、ほかのキーと組んだ (離したときに単打で出さない)
    romaji_: Vec<(U16, U16)>, // kind romaji の綴り
    romaji_base_empty_: bool,
    prefixed_: BTreeMap<String, U16>, // 前置シフトの面: キーは "p1>e"
    prefix_: String,                  // 打った前置 ("p1")。空 = なし
}

impl Default for Layout {
    fn default() -> Self {
        Layout {
            window_ms: 100,
            overlap_ratio: 0.5,
            thumbs: 0,
            continuous: true,
            thumb_tap: true,
            repeat: true,
            evs_: VecDeque::new(),
            shifts_: Vec::new(),
            next_shift_id_: 0,
            kind_: Kind::Letters,
            name_: String::new(),
            author_: String::new(),
            url_: String::new(),
            single_: BTreeMap::new(),
            shifted_: BTreeMap::new(),
            chords_: BTreeMap::new(),
            chord_keys_: BTreeSet::new(),
            shift_keys_: BTreeSet::new(),
            pending_: Vec::new(),
            held_: BTreeSet::new(),
            shift_used_: false,
            romaji_: Vec::new(),
            romaji_base_empty_: false,
            prefixed_: BTreeMap::new(),
            prefix_: String::new(),
        }
    }
}

impl Layout {
    pub fn new() -> Layout {
        Layout::default()
    }

    /// 表 (TSV) を読む。失敗したら理由を Err で返す (元の load の err 引数)
    pub fn load(&mut self, tsv: &str) -> Result<(), String> {
        *self = Layout::default();
        let mut keys_section = false;
        let mut line_no = 0usize;
        for raw in tsv.split('\n') {
            line_no += 1;
            let mut line = raw;
            if line.ends_with('\r') {
                line = &line[..line.len() - 1];
            }
            let line_owned: String;
            if line_no == 1 {
                if let Some(stripped) = line.strip_prefix('\u{FEFF}') {
                    line_owned = stripped.to_string();
                    line = &line_owned;
                }
            }
            if line.is_empty() {
                continue;
            }
            if let Some(rest0) = line.strip_prefix("#!") {
                // 見出し: #! name 大西配列
                let rest = rest0.trim_start_matches(' ');
                let (k, v) = match rest.find(' ') {
                    Some(sp) => (&rest[..sp], &rest[sp + 1..]),
                    None => (rest, ""),
                };
                match k {
                    "name" => self.name_ = v.to_string(),
                    "author" => self.author_ = v.to_string(),
                    "url" => self.url_ = v.to_string(),
                    "kind" => {
                        self.kind_ = match v {
                            "kana" => Kind::Kana,
                            "chord" => Kind::Chord,
                            "romaji" => Kind::Romaji,
                            _ => Kind::Letters,
                        }
                    }
                    "window" => {
                        self.window_ms = to_int(v).clamp(20, 400) as u64;
                    }
                    "keys" => keys_section = true, // kind romaji: ここからキーの読み替え
                    "base" => self.romaji_base_empty_ = v == "empty", // kind romaji: 元のローマ字の表を使わない (gACT10)
                    _ => {}
                }
                continue;
            }
            if line.starts_with('#') {
                continue;
            }
            let tab = match line.find('\t') {
                Some(t) if t > 0 => t,
                _ => return Err(format!("行 {}: キーと字の間にタブがありません", line_no)),
            };
            let keys = &line[..tab];
            let out = u16s(&line[tab + 1..]);
            if self.kind_ == Kind::Romaji && keys_section {
                // キーの読み替え (単打と Shift だけ)
                if keys.len() > 2 && keys.starts_with("S-") {
                    self.shifted_.insert(keys[2..].to_string(), out);
                } else {
                    self.single_.insert(keys.to_string(), out);
                }
                continue;
            }
            if self.kind_ == Kind::Romaji {
                // ローマ字の表の綴り (; : なども綴りの字。同時押し・Shift の書き方は使わない)
                self.romaji_.push((u16s(keys), out));
                continue;
            }
            if keys.len() > 3 && keys.starts_with('p') && keys.contains('>') {
                // 前置シフトの面
                self.prefixed_.insert(keys.to_string(), out);
                continue;
            }
            if keys.len() > 2 && keys.starts_with("S-") {
                self.shifted_.insert(keys[2..].to_string(), out);
                continue;
            }
            if !keys.contains('+') || keys == "+" {
                self.single_.insert(keys.to_string(), out);
                continue;
            }
            let ks = split(keys, '+');
            for k in &ks {
                if is_shift_key(k) {
                    self.shift_keys_.insert(k.clone());
                } else {
                    self.chord_keys_.insert(k.clone());
                }
            }
            self.chords_.insert(chord_key(ks), out);
        }
        if self.loaded() {
            Ok(())
        } else {
            Err("キーの行がありません".to_string())
        }
    }

    pub fn loaded(&self) -> bool {
        !self.single_.is_empty() || !self.chords_.is_empty() || !self.romaji_.is_empty()
    }

    /// kind romaji: ローマ字の表に足す綴り (綴り, かな)。キーは読み替えない (handles は いつも false)
    pub fn romaji_pairs(&self) -> &[(U16, U16)] {
        &self.romaji_
    }

    /// 「#! base empty」: 元のローマ字の表を空にして、この表の綴りだけで打つ (gACT10)
    pub fn romaji_base_empty(&self) -> bool {
        self.romaji_base_empty_
    }

    pub fn kind(&self) -> Kind {
        self.kind_
    }

    pub fn name(&self) -> &str {
        &self.name_
    }

    pub fn author(&self) -> &str {
        &self.author_
    }

    pub fn url(&self) -> &str {
        &self.url_
    }

    /// 物理キーの名前を、親指キーの割り当てに合わせて読み替える (空なら この配列では受け持たない)
    pub fn map_key(&self, k: &str) -> String {
        if self.thumbs == 0
            || !(self.shift_keys_.contains("lthumb") || self.shift_keys_.contains("rthumb"))
        {
            return k.to_string(); // 親指の配列でなければそのまま
        }
        if self.thumbs == 1 {
            // 無変換・空白: 空白が右親指、変換キーはふつうに
            if k == "space" {
                return "rthumb".to_string();
            }
            if k == "rthumb" {
                return String::new();
            }
        } else if self.thumbs == 2 {
            // 空白・変換: 空白が左親指、無変換キーはふつうに
            if k == "space" {
                return "lthumb".to_string();
            }
            if k == "lthumb" {
                return String::new();
            }
        }
        k.to_string()
    }

    /// この配列が受け持つキーか (受け持たないキーは今までどおり扱う)
    pub fn handles(&self, key: &str) -> bool {
        if self.kind_ == Kind::Romaji {
            return self.single_.contains_key(key) || self.shifted_.contains_key(key);
        }
        if self.single_.contains_key(key)
            || self.shifted_.contains_key(key)
            || self.chord_keys_.contains(key)
            || self.shift_keys_.contains(key)
        {
            return true;
        }
        for k in self.prefixed_.keys() {
            // 前置シフトの面にしか無いキー (p1>e の e)
            if let Some(gt) = k.find('>') {
                if &k[gt + 1..] == key {
                    return true;
                }
            }
        }
        false
    }

    pub fn reset(&mut self) {
        self.evs_.clear();
        self.shifts_.clear();
        self.prefix_.clear();
        self.pending_.clear();
        self.held_.clear();
        self.shift_used_ = false;
    }

    /// {p1} は出さずに前置シフトにする
    fn emit(&mut self, s: &[u16], unit: &str) -> Vec<Out> {
        let mut out: Vec<Out> = Vec::new();
        if s.len() == 4 && s[0] == b'{' as u16 && s[1] == b'p' as u16 && s[3] == b'}' as u16 {
            // 前置シフトのキー: 次のキーをその面で読む
            self.prefix_ = format!("p{}", (s[2] as u8) as char);
            return out;
        }
        // {bs} などは打鍵に、それ以外は字に (つながった字は 1 つにまとめる)
        let mut i = 0usize;
        let mut text: U16 = Vec::new();
        while i < s.len() {
            if s[i] == b'{' as u16 {
                if let Some(rel) = s[i..].iter().position(|&c| c == b'}' as u16) {
                    let e = i + rel;
                    if !text.is_empty() {
                        out.push(Out {
                            text: std::mem::take(&mut text),
                            key: String::new(),
                            unit: String::new(),
                        });
                    }
                    let k: String = s[i + 1..e].iter().map(|&c| (c as u8) as char).collect();
                    out.push(Out {
                        text: Vec::new(),
                        key: k,
                        unit: String::new(),
                    });
                    i = e + 1;
                    continue;
                }
            }
            text.push(s[i]);
            i += 1;
        }
        if !text.is_empty() {
            out.push(Out {
                text,
                key: String::new(),
                unit: String::new(),
            });
        }
        for o in out.iter_mut() {
            if !o.text.is_empty() {
                o.unit = unit.to_string();
            }
        }
        out
    }

    fn lookup1(&self, key: &str, shift: bool) -> Out {
        if shift {
            if let Some(v) = self.shifted_.get(key) {
                return Out {
                    text: v.clone(),
                    key: String::new(),
                    unit: String::new(),
                };
            }
            if self.kind_ == Kind::Letters || self.kind_ == Kind::Romaji {
                // 英字の配列で Shift の行が無い英字: 大文字
                if let Some(v) = self.single_.get(key) {
                    if v.len() == 1 && (b'a' as u16..=b'z' as u16).contains(&v[0]) {
                        return Out {
                            text: vec![v[0] - 32],
                            key: String::new(),
                            unit: String::new(),
                        };
                    }
                }
            }
        }
        match self.single_.get(key) {
            Some(v) => Out {
                text: v.clone(),
                key: String::new(),
                unit: String::new(),
            },
            None => Out::default(),
        }
    }

    /// 押している間効くシフト (space・lthumb・rthumb) のうち押されているもの
    fn shift_held(&self) -> String {
        for s in SHIFTS {
            if self.held_.contains(s) && self.shift_keys_.contains(s) {
                return s.to_string();
            }
        }
        String::new()
    }

    fn flush_pending(&mut self) -> Vec<Out> {
        let mut out: Vec<Out> = Vec::new();
        if self.pending_.is_empty() {
            return out;
        }
        let pending = std::mem::take(&mut self.pending_);
        let sh = self.shift_held();
        if !sh.is_empty() {
            // シフトを押している間: シフト+キー (無ければ単打)
            for p in &pending {
                let ck = chord_key(vec![sh.clone(), p.key.clone()]);
                match self.chords_.get(&ck).cloned() {
                    Some(v) => {
                        let e = self.emit(&v, &ck);
                        out.extend(e);
                        self.shift_used_ = true;
                    }
                    None => {
                        let txt = self.lookup1(&p.key, p.shift).text;
                        let unit = unit_name(p.shift, &p.key);
                        let e = self.emit(&txt, &unit);
                        out.extend(e);
                    }
                }
            }
            return out;
        }
        // 同時押しの組 (いちばん長い組から。3 キー → 2 キー → 単打)
        let mut i = 0usize;
        while i < pending.len() {
            let mut done = false;
            let mut len = 3usize.min(pending.len() - i);
            while len >= 2 {
                let sub: Vec<String> = pending[i..i + len].iter().map(|p| p.key.clone()).collect();
                let ck = chord_key(sub);
                if let Some(v) = self.chords_.get(&ck).cloned() {
                    let e = self.emit(&v, &ck);
                    out.extend(e);
                    i += len;
                    done = true;
                    break;
                }
                len -= 1;
            }
            if !done {
                let txt = self.lookup1(&pending[i].key, pending[i].shift).text;
                let unit = unit_name(pending[i].shift, &pending[i].key);
                let e = self.emit(&txt, &unit);
                out.extend(e);
                i += 1;
            }
        }
        out
    }

    /// 押した (t はミリ秒)。決まった分を返す。同時押しの配列は、組になるか分かるまで待つ (空を返す)
    pub fn down(&mut self, key: &str, shift: bool, t: u64) -> Vec<Out> {
        let mut out: Vec<Out> = Vec::new();
        if self.held_.contains(key) {
            // 押しっぱなしの繰り返し: 単打の繰り返し (同時押しの配列でも、組にしない)
            if !self.repeat {
                return out;
            }
            if !is_shift_key(key) && self.kind_ != Kind::Chord {
                let txt = self.lookup1(key, shift).text;
                return self.emit(&txt, &unit_name(shift, key));
            }
            if !is_shift_key(key) && self.kind_ == Kind::Chord {
                for e in self.evs_.iter() {
                    if e.key == key && !e.decided {
                        return out; // まだ決まっていない: 繰り返さない
                    }
                }
                let txt = self.lookup1(key, shift).text;
                return self.emit(&txt, &unit_name(shift, key));
            }
            return out;
        }
        if !self.prefix_.is_empty() && !is_shift_key(key) {
            // 前置シフトの次のキー
            let pk = format!("{}>{}", self.prefix_, key);
            self.prefix_.clear();
            self.held_.insert(key.to_string());
            if let Some(v) = self.prefixed_.get(&pk).cloned() {
                return self.emit(&v, &pk);
            }
            self.held_.remove(key); // その面に無いキー: ふつうに
        }
        if self.kind_ == Kind::Chord {
            return self.chord_down(key, shift, t);
        }
        // 時間切れの待ちを先に出す (このキーを押す前に決まっていた分)
        if !self.pending_.is_empty() && t.wrapping_sub(self.pending_[0].t) >= self.window_ms {
            out = self.flush_pending();
        }
        self.held_.insert(key.to_string());
        if is_shift_key(key) && self.shift_keys_.contains(key) {
            // シフトを押した: 押しっぱなしで効く
            self.shift_used_ = false;
            // 先に押していて待っているキー (時間内) はシフトと組む (NICOLA: 文字キーと親指は順番を問わない)
            let e = self.flush_pending();
            out.extend(e);
            return out;
        }
        let chordable = self.chord_keys_.contains(key) || !self.shift_held().is_empty();
        if self.kind_ != Kind::Chord || !chordable {
            let p = self.flush_pending();
            out.extend(p);
            let sh = self.shift_held();
            if !sh.is_empty() {
                let ck = chord_key(vec![sh, key.to_string()]);
                if let Some(v) = self.chords_.get(&ck).cloned() {
                    self.shift_used_ = true;
                    let e = self.emit(&v, &ck);
                    out.extend(e);
                    return out;
                }
            }
            let txt = self.lookup1(key, shift).text;
            let e = self.emit(&txt, &unit_name(shift, key));
            out.extend(e);
            return out;
        }
        if !self.shift_held().is_empty() {
            // シフトを押している間は待たずに決まる
            self.pending_.push(Pending {
                key: key.to_string(),
                shift,
                t,
            });
            let e = self.flush_pending();
            out.extend(e);
            return out;
        }
        self.pending_.push(Pending {
            key: key.to_string(),
            shift,
            t,
        });
        if self.pending_.len() >= 3 {
            // 3 キーまで。そこで決める
            let e = self.flush_pending();
            out.extend(e);
        }
        out
    }

    /// 離した (t はミリ秒)
    pub fn up(&mut self, key: &str, t: u64) -> Vec<Out> {
        let mut out: Vec<Out> = Vec::new();
        if !self.held_.contains(key) {
            return out;
        }
        if self.kind_ == Kind::Chord {
            return self.chord_up(key, t);
        }
        if is_shift_key(key) && self.shift_keys_.contains(key) {
            let e = self.flush_pending(); // シフトを離す前に押していたキーはシフトと組む
            self.held_.remove(key);
            out.extend(e);
            if !self.shift_used_ {
                // シフトを単独で押して離した: そのキーの単打 (空白など)
                match self.single_.get(key).cloned() {
                    Some(v) => {
                        let s = self.emit(&v, key);
                        out.extend(s);
                    }
                    None => {
                        if key == "space" {
                            out.push(Out {
                                text: Vec::new(),
                                key: "space".to_string(),
                                unit: String::new(),
                            });
                        } else if key == "lthumb" || key == "rthumb" {
                            // 親指キーを単独で: 無変換・変換キーとして
                            out.push(Out {
                                text: Vec::new(),
                                key: if key == "lthumb" {
                                    "muhenkan".to_string()
                                } else {
                                    "henkan".to_string()
                                },
                                unit: String::new(),
                            });
                        }
                    }
                }
            }
            self.shift_used_ = false;
            return out;
        }
        // 待っているキーを離した: そこまでで決める
        let was_pending = self.pending_.iter().any(|p| p.key == key);
        self.held_.remove(key);
        if was_pending {
            out = self.flush_pending();
        }
        out
    }

    /// 待っているキーがあれば、時間切れ (now - 押した時刻 >= window) で単打として出す
    pub fn tick(&mut self, now: u64) -> Vec<Out> {
        if self.kind_ == Kind::Chord {
            return self.resolve(now);
        }
        if !self.pending_.is_empty()
            && now.wrapping_sub(self.pending_[0].t) >= self.window_ms
            && self.shift_held().is_empty()
        {
            return self.flush_pending();
        }
        Vec::new()
    }

    pub fn waiting(&self) -> bool {
        !self.pending_.is_empty() || !self.evs_.is_empty()
    }

    // ---------------------------------------------------------------- 同時押しの配列: 押していた時間の重なりで決める

    fn chord_down(&mut self, key: &str, shift: bool, t: u64) -> Vec<Out> {
        self.held_.insert(key.to_string());
        let mut e = Ev {
            key: key.to_string(),
            shift,
            is_shift: is_shift_key(key) && self.shift_keys_.contains(key),
            down: t,
            shift_id: -1,
            ..Default::default()
        };
        if e.is_shift {
            e.shift_id = self.next_shift_id_;
            self.next_shift_id_ += 1;
            self.shifts_.push(ShiftRec {
                id: e.shift_id,
                key: key.to_string(),
                down: t,
                up: 0,
                released: false,
                used: false,
            });
        }
        self.evs_.push_back(e);
        self.resolve(t)
    }

    fn chord_up(&mut self, key: &str, t: u64) -> Vec<Out> {
        self.held_.remove(key);
        for e in self.evs_.iter_mut() {
            if e.key == key && !e.released {
                e.released = true;
                e.up = t;
                break;
            }
        }
        for r in self.shifts_.iter_mut() {
            if r.key == key && !r.released {
                r.released = true;
                r.up = t;
            }
        }
        self.resolve(t)
    }

    fn absorb(
        &mut self,
        i: usize,
        with: &[usize],
        n_ev: usize,
        sh: &[Ev],
        out_text: &[u16],
        unit: &str,
    ) {
        let mut first = i; // 出す位置は組の中でいちばん先に押したキー
        for &j in with {
            if j < n_ev {
                first = first.min(j);
            }
        }
        self.evs_[i].decided = true;
        for &j in with {
            if j >= n_ev {
                // シフト: 使った印 (単打を出さない)。evs_ の方も決まりにして後ろの字を待たせない
                let id = sh[j - n_ev].shift_id;
                for r in self.shifts_.iter_mut() {
                    if r.id == id {
                        r.used = true;
                    }
                }
                for x in self.evs_.iter_mut() {
                    if x.shift_id == id {
                        x.used = true;
                        x.decided = true;
                    }
                }
            } else {
                self.evs_[j].decided = true;
            }
        }
        let outs = self.emit(out_text, unit);
        self.evs_[first].out = outs;
        if first != i {
            self.evs_[i].out.clear();
        }
    }

    fn resolve(&mut self, now: u64) -> Vec<Out> {
        let mut i = 0usize;
        while i < self.evs_.len() {
            let idx = i;
            i += 1;
            let e = self.evs_[idx].clone();
            if e.decided || e.is_shift {
                continue;
            }
            let long_alone = !e.released && now.wrapping_sub(e.down) >= self.window_ms;
            if !e.released && !long_alone {
                continue;
            }
            // 組になれる相手: まだ決まっていない字と、シフトの記録 (組んだ後も押している間は使える)。重なりが十分なもの
            let n_ev = self.evs_.len();
            // シフトの記録を、相手の候補として evs_ の後ろに一時的に並べる (番号 >= evs_.len() はシフトの記録)
            let sh: Vec<Ev> = self
                .shifts_
                .iter()
                .map(|r| Ev {
                    key: r.key.clone(),
                    is_shift: true,
                    down: r.down,
                    up: r.up,
                    released: r.released,
                    shift_id: r.id,
                    ..Default::default()
                })
                .collect();
            let evs_snap: Vec<Ev> = self.evs_.iter().cloned().collect();
            let mut strong: Vec<usize> = Vec::new();
            for j in 0..n_ev + sh.len() {
                if j == idx {
                    continue;
                }
                let x = ev_at(&evs_snap, &sh, n_ev, j);
                if j < n_ev && x.is_shift {
                    continue; // シフトは記録の方で見る
                }
                if !x.is_shift && x.decided {
                    continue;
                }
                if j >= n_ev {
                    let used = self
                        .shifts_
                        .iter()
                        .find(|r| r.id == x.shift_id)
                        .map_or(false, |r| r.used);
                    if used
                        && !(self.continuous
                            && (if e.released {
                                !x.released || x.up >= e.up
                            } else {
                                long_alone
                            }))
                    {
                        continue;
                    }
                }
                // 字どうしは押した時刻が近いときだけ (片方を長く押したまま、ずっと後でもう片方を押したのは組にしない)
                if !x.is_shift
                    && (x.down as i128 - e.down as i128).abs() > (self.window_ms * 3 / 2) as i128
                {
                    continue;
                }
                if ev_ratio(&e, &x, now) >= self.overlap_ratio
                    || (long_alone && !x.released && x.down <= e.down && x.is_shift)
                {
                    strong.push(j);
                }
            }
            if long_alone && strong.is_empty() {
                // 1 つだけ長く押している: まだほかと重なっていなければ単打に決める (重なっている相手が後から来たら待つ)
                let mut any = false;
                for j in 0..n_ev {
                    any = any
                        || (j != idx
                            && !evs_snap[j].is_shift
                            && !evs_snap[j].decided
                            && ev_overlap(&e, &evs_snap[j], now) > 0.0);
                }
                for x in &sh {
                    any = any || ev_overlap(&e, x, now) > 0.0;
                }
                if any {
                    continue;
                }
            } else if long_alone {
                continue; // 重なっている相手がいる: 離すまで待つ
            }
            let mut done = false;
            // 3 キー
            'outer3: for a in 0..strong.len() {
                for b in a + 1..strong.len() {
                    let ck = chord_key(vec![
                        e.key.clone(),
                        ev_at(&evs_snap, &sh, n_ev, strong[a]).key,
                        ev_at(&evs_snap, &sh, n_ev, strong[b]).key,
                    ]);
                    if let Some(v) = self.chords_.get(&ck).cloned() {
                        self.absorb(idx, &[strong[a], strong[b]], n_ev, &sh, &v, &ck);
                        done = true;
                        break 'outer3;
                    }
                }
            }
            // 2 キー (重なりの大きい相手から)
            if !done {
                strong.sort_by(|&a, &b| {
                    let ra = ev_ratio(&e, &ev_at(&evs_snap, &sh, n_ev, a), now);
                    let rb = ev_ratio(&e, &ev_at(&evs_snap, &sh, n_ev, b), now);
                    rb.partial_cmp(&ra).unwrap_or(std::cmp::Ordering::Equal)
                });
                for a in 0..strong.len() {
                    let ck = chord_key(vec![e.key.clone(), ev_at(&evs_snap, &sh, n_ev, strong[a]).key]);
                    if let Some(v) = self.chords_.get(&ck).cloned() {
                        self.absorb(idx, &[strong[a]], n_ev, &sh, &v, &ck);
                        done = true;
                        break;
                    }
                }
            }
            if !done {
                self.evs_[idx].decided = true;
                let txt = self.lookup1(&e.key, e.shift).text;
                let outs = self.emit(&txt, &unit_name(e.shift, &e.key));
                self.evs_[idx].out = outs;
            }
        }
        // シフト: 離して、重なっていた字が全部決まったら。どの字とも組まなかったら単打 (空白・無変換・変換)
        for idx in 0..self.evs_.len() {
            let s = self.evs_[idx].clone();
            if !s.is_shift || s.decided || !s.released {
                continue; // まだ押している・使った (決まり済み) シフトは見ない
            }
            let mut wait = false;
            for c in self.evs_.iter() {
                wait = wait || (!c.is_shift && !c.decided && ev_overlap(&s, c, now) > 0.0);
            }
            if wait {
                continue;
            }
            self.evs_[idx].decided = true;
            if self.shifts_.iter().any(|r| r.id == s.shift_id && r.used) {
                self.evs_[idx].used = true;
            }
            if !self.evs_[idx].used {
                match self.single_.get(&s.key).cloned() {
                    Some(v) => {
                        let o = self.emit(&v, &s.key);
                        self.evs_[idx].out = o;
                    }
                    None => {
                        if s.key == "space" {
                            self.evs_[idx].out = vec![Out {
                                text: Vec::new(),
                                key: "space".to_string(),
                                unit: String::new(),
                            }];
                        } else if !self.thumb_tap {
                            // 親指キーの単独打鍵は何もしない
                        } else if (s.key == "rthumb" && self.thumbs == 1)
                            || (s.key == "lthumb" && self.thumbs == 2)
                        {
                            // 空白を親指にしたとき
                            self.evs_[idx].out = vec![Out {
                                text: Vec::new(),
                                key: "space".to_string(),
                                unit: String::new(),
                            }];
                        } else if s.key == "lthumb" || s.key == "rthumb" {
                            self.evs_[idx].out = vec![Out {
                                text: Vec::new(),
                                key: if s.key == "lthumb" {
                                    "muhenkan".to_string()
                                } else {
                                    "henkan".to_string()
                                },
                                unit: String::new(),
                            }];
                        }
                    }
                }
            }
        }
        self.shifts_.retain(|r| {
            if !r.released || now.wrapping_sub(r.up) < 1000 {
                return true;
            }
            for c in self.evs_.iter() {
                if !c.is_shift && !c.decided && c.down <= r.up {
                    return true;
                }
            }
            false
        });
        // 決まったものを押した順に出す (前が決まっていなければ待つ)
        let mut out: Vec<Out> = Vec::new();
        while let Some(front) = self.evs_.front() {
            if !front.decided {
                break;
            }
            let e = self.evs_.pop_front().unwrap();
            out.extend(e.out);
        }
        out
    }

    // ---------------------------------------------------------------- 打ち間違いの直し (2026-11-15)

    /// 単位 (表の書き方) の出す字。前置シフト・打鍵 ({bs}) を含む単位は空
    pub fn unit_text(&self, unit: &str) -> U16 {
        let mut t: U16 = Vec::new();
        if unit.is_empty() {
            return t;
        }
        if unit.contains('>') {
            if let Some(v) = self.prefixed_.get(unit) {
                t = v.clone();
            }
        } else if unit.len() > 2 && unit.starts_with("S-") {
            t = self.lookup1(&unit[2..], true).text;
        } else if unit.contains('+') && unit != "+" {
            if let Some(v) = self.chords_.get(unit) {
                t = v.clone();
            }
        } else if self.kind_ == Kind::Romaji && !self.single_.contains_key(unit) {
            if unit.len() == 1 {
                t = vec![unit.as_bytes()[0] as u16]; // 読み替えないキー: 打ったままローマ字に
            }
        } else {
            t = self.lookup1(unit, false).text;
        }
        if t.contains(&(b'{' as u16)) {
            t.clear(); // 前置シフト・打鍵 ({bs}) を含む単位は直しに使わない
        }
        t
    }

    pub fn has_key_map(&self) -> bool {
        !self.single_.is_empty() || !self.shifted_.is_empty()
    }

    /// 1 つの単位の言い換え (隣のキー・別の面)
    pub fn unit_alts(&self, unit: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        if let Some(gt) = unit.find('>') {
            // 前置シフトの面: 前置なし・別の前置・隣のキー
            let (pre, k) = (&unit[..gt], &unit[gt + 1..]);
            self.alt_add(&mut out, unit, k.to_string());
            let keys: Vec<String> = self.prefixed_.keys().cloned().collect();
            for kv in &keys {
                if let Some(g2) = kv.find('>') {
                    if &kv[g2 + 1..] == k {
                        self.alt_add(&mut out, unit, kv.clone());
                    }
                }
            }
            for nb in phys_neighbors(k) {
                self.alt_add(&mut out, unit, format!("{}>{}", pre, nb));
            }
            return out;
        }
        if unit.len() > 2 && unit.starts_with("S-") {
            // Shift の付け忘れ・付けすぎ
            let k = &unit[2..];
            self.alt_add(&mut out, unit, k.to_string());
            for nb in phys_neighbors(k) {
                self.alt_add(&mut out, unit, format!("S-{}", nb));
            }
            return out;
        }
        if unit.contains('+') && unit != "+" {
            let parts = split(unit, '+');
            let (mut chars, mut shs): (Vec<String>, Vec<String>) = (Vec::new(), Vec::new());
            for p in &parts {
                if is_shift_key(p) {
                    shs.push(p.clone());
                } else {
                    chars.push(p.clone());
                }
            }
            if !shs.is_empty() && chars.len() == 1 {
                // 親指・空白のシフト: 付け忘れ・別の親指
                self.alt_add(&mut out, unit, chars[0].clone());
                let sh_keys: Vec<String> = self.shift_keys_.iter().cloned().collect();
                for sh in &sh_keys {
                    self.alt_add(&mut out, unit, chord_key(vec![sh.clone(), chars[0].clone()]));
                }
            }
            if shs.is_empty() && chars.len() >= 2 {
                // 字どうしの同時押し: 1 つ押しそこねた
                for i in 0..chars.len() {
                    let sub: Vec<String> = chars
                        .iter()
                        .enumerate()
                        .filter(|(j, _)| *j != i)
                        .map(|(_, c)| c.clone())
                        .collect();
                    self.alt_add(
                        &mut out,
                        unit,
                        if sub.len() == 1 {
                            sub[0].clone()
                        } else {
                            chord_key(sub)
                        },
                    );
                }
            }
            for i in 0..parts.len() {
                // どれか 1 つが隣のキー
                if is_shift_key(&parts[i]) {
                    continue;
                }
                for nb in phys_neighbors(&parts[i]) {
                    let mut q = parts.clone();
                    q[i] = nb;
                    self.alt_add(&mut out, unit, chord_key(q));
                }
            }
            return out;
        }
        // 単打: Shift・親指シフト・前置シフトの付け忘れ、隣のキー
        if self.shifted_.contains_key(unit) {
            self.alt_add(&mut out, unit, format!("S-{}", unit));
        }
        let sh_keys: Vec<String> = self.shift_keys_.iter().cloned().collect();
        for sh in &sh_keys {
            self.alt_add(&mut out, unit, chord_key(vec![sh.clone(), unit.to_string()]));
        }
        let keys: Vec<String> = self.prefixed_.keys().cloned().collect();
        for kv in &keys {
            if let Some(g2) = kv.find('>') {
                if &kv[g2 + 1..] == unit {
                    self.alt_add(&mut out, unit, kv.clone());
                }
            }
        }
        for nb in phys_neighbors(unit) {
            self.alt_add(&mut out, unit, nb);
        }
        out
    }

    fn alt_add(&self, out: &mut Vec<String>, unit: &str, u: String) {
        if u != unit && !self.unit_text(&u).is_empty() && !out.contains(&u) {
            out.push(u);
        }
    }

    /// 打鍵の単位の並びを 1 か所だけ直した並び (単位ごとの出す字の並びで返す):
    /// 消す・隣の物理キー (同じ面)・同じキーの別の面 (Shift・親指・前置シフトの付け外し)・入れ替え・
    /// 同時押しの組み直し (組みそこね・組みすぎ)・抜けた 1 打 (英字の配列だけ)
    pub fn typo_variants(&self, units: &[String]) -> Vec<Vec<U16>> {
        let mut out: Vec<Vec<U16>> = Vec::new();
        let n = units.len();
        let mut texts: Vec<U16> = Vec::new();
        for u in units {
            texts.push(self.unit_text(u));
            if texts.last().unwrap().is_empty() {
                return out; // 表で引けない単位がある: 直さない
            }
        }
        let plain = |u: &str| {
            !u.is_empty()
                && !u.contains('>')
                && (!u.contains('+') || u == "+")
                && !u.starts_with("S-")
        };
        for i in 0..n {
            if n > 1 {
                // 余分な 1 打
                let mut v = texts.clone();
                v.remove(i);
                out.push(v);
            }
            for a in self.unit_alts(&units[i]) {
                // 隣のキー・別の面
                let mut v = texts.clone();
                v[i] = self.unit_text(&a);
                out.push(v);
            }
            if i + 1 < n && texts[i] != texts[i + 1] {
                // 入れ替え
                let mut v = texts.clone();
                v.swap(i, i + 1);
                out.push(v);
            }
            if i + 1 < n && plain(&units[i]) && plain(&units[i + 1]) && units[i] != units[i + 1] {
                // 同時押しのつもりが 2 つの単打に
                let ck = chord_key(vec![units[i].clone(), units[i + 1].clone()]);
                if let Some(val) = self.chords_.get(&ck) {
                    if !val.contains(&(b'{' as u16)) {
                        let mut v = texts.clone();
                        v[i] = val.clone();
                        v.remove(i + 1);
                        out.push(v);
                    }
                }
            }
            if !units[i].contains('>') && units[i] != "+" {
                let parts = split(&units[i], '+');
                if parts.len() == 2 && !is_shift_key(&parts[0]) && !is_shift_key(&parts[1]) {
                    // 続けて打った 2 つの単打が重なって同時押しに
                    let a = self.unit_text(&parts[0]);
                    let b = self.unit_text(&parts[1]);
                    if !a.is_empty() && !b.is_empty() {
                        for ord in 0..2 {
                            let mut v = texts.clone();
                            v[i] = if ord == 1 { b.clone() } else { a.clone() };
                            v.insert(i + 1, if ord == 1 { a.clone() } else { b.clone() });
                            out.push(v);
                        }
                    }
                }
            }
        }
        if self.kind_ == Kind::Letters || self.kind_ == Kind::Romaji {
            // 抜けた 1 打 (英字の配列): ローマ字で抜けやすい字 (a i u e o n x t s r k h y) を出すキーと、前の打鍵の重ね
            let mut ins: Vec<String> = Vec::new();
            for c in "aiueonxtsrkhy".chars() {
                let want: U16 = vec![c as u16];
                let mut found = String::new();
                for (k, v) in self.single_.iter() {
                    if *v == want {
                        found = k.clone();
                        break;
                    }
                }
                if found.is_empty()
                    && self.kind_ == Kind::Romaji
                    && !self.single_.contains_key(&c.to_string())
                {
                    found = c.to_string();
                }
                if !found.is_empty() {
                    ins.push(found);
                }
            }
            for i in 0..=n {
                for u in &ins {
                    let mut v = texts.clone();
                    let ut = self.unit_text(u);
                    v.insert(i, ut);
                    out.push(v);
                }
                if i > 0 {
                    let mut v = texts.clone();
                    let prev = texts[i - 1].clone();
                    v.insert(i, prev);
                    out.push(v);
                }
            }
        }
        out
    }

    /// 表にある打鍵の単位の全部 (ベンチで読みを打鍵に直すのに使う)
    pub fn all_units(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for k in self.single_.keys() {
            out.push(k.clone());
        }
        for k in self.shifted_.keys() {
            out.push(format!("S-{}", k));
        }
        for k in self.chords_.keys() {
            out.push(k.clone());
        }
        for k in self.prefixed_.keys() {
            out.push(k.clone());
        }
        out
    }
}

fn unit_name(shift: bool, key: &str) -> String {
    format!("{}{}", if shift { "S-" } else { "" }, key)
}

/// 物理キーの段 (US 配列の位置。数字の段は字を打たない配列が多いので見ない)
const PHYS_ROWS: [&str; 3] = ["qwertyuiop[]", "asdfghjkl;'", "zxcvbnm,./"];

/// 隣の物理キー: 同じ段の左右と、上の段の同じ位置と右・下の段の左と同じ位置
fn phys_neighbors(k: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if k.chars().count() != 1 {
        return out;
    }
    let kc = k.chars().next().unwrap();
    for r in 0..3 {
        let row: Vec<char> = PHYS_ROWS[r].chars().collect();
        let j = match row.iter().position(|&c| c == kc) {
            Some(p) => p as i64,
            None => continue,
        };
        let put = |out: &mut Vec<String>, rr: usize, c: i64| {
            let o: Vec<char> = PHYS_ROWS[rr].chars().collect();
            if c >= 0 && (c as usize) < o.len() {
                out.push(o[c as usize].to_string());
            }
        };
        put(&mut out, r, j - 1);
        put(&mut out, r, j + 1);
        if r > 0 {
            put(&mut out, r - 1, j);
            put(&mut out, r - 1, j + 1);
        }
        if r < 2 {
            put(&mut out, r + 1, j - 1);
            put(&mut out, r + 1, j);
        }
    }
    out
}

/// US 配列の物理位置の名前 (Windows のスキャンコードから)。分からないキーは空
pub fn key_name_from_scancode(sc: u32, extended: bool) -> String {
    if extended {
        return String::new();
    }
    let s = match sc {
        0x02 => "1",
        0x03 => "2",
        0x04 => "3",
        0x05 => "4",
        0x06 => "5",
        0x07 => "6",
        0x08 => "7",
        0x09 => "8",
        0x0A => "9",
        0x0B => "0",
        0x0C => "-",
        0x0D => "=",
        0x10 => "q",
        0x11 => "w",
        0x12 => "e",
        0x13 => "r",
        0x14 => "t",
        0x15 => "y",
        0x16 => "u",
        0x17 => "i",
        0x18 => "o",
        0x19 => "p",
        0x1A => "[",
        0x1B => "]",
        0x1E => "a",
        0x1F => "s",
        0x20 => "d",
        0x21 => "f",
        0x22 => "g",
        0x23 => "h",
        0x24 => "j",
        0x25 => "k",
        0x26 => "l",
        0x27 => ";",
        0x28 => "'",
        0x29 => "`",
        0x2B => "\\",
        0x2C => "z",
        0x2D => "x",
        0x2E => "c",
        0x2F => "v",
        0x30 => "b",
        0x31 => "n",
        0x32 => "m",
        0x33 => ",",
        0x34 => ".",
        0x35 => "/",
        0x39 => "space",
        0x73 => "ro",     // JIS の \ _ (ろ)
        0x7D => "yen",    // JIS の ¥ |
        0x7B => "lthumb", // 無変換
        0x79 => "rthumb", // 変換
        _ => return String::new(),
    };
    s.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::from_u16s;

    fn texts(outs: &[Out]) -> Vec<String> {
        outs.iter().map(|o| from_u16s(&o.text)).collect()
    }

    #[test]
    fn load_letters_and_single() {
        let mut l = Layout::new();
        l.load("#! name テスト配列\n#! kind letters\na\tあ\nS-a\tぁ\nq\tた\n")
            .unwrap();
        assert_eq!(l.name(), "テスト配列");
        assert_eq!(l.kind(), Kind::Letters);
        assert!(l.loaded());
        assert!(l.handles("a"));
        assert!(!l.handles("z"));
        let out = l.down("a", false, 0);
        assert_eq!(texts(&out), vec!["あ"]);
        assert_eq!(out[0].unit, "a");
        let out = l.down("a", true, 10);
        assert_eq!(texts(&out), vec!["ぁ"]);
        assert_eq!(out[0].unit, "S-a");
    }

    #[test]
    fn load_errors() {
        let mut l = Layout::new();
        assert!(l.load("タブ無し\n").is_err());
        let mut l2 = Layout::new();
        assert_eq!(l2.load("# コメントだけ\n").unwrap_err(), "キーの行がありません");
    }

    #[test]
    fn romaji_kind_pairs_and_keys_section() {
        let mut l = Layout::new();
        l.load("#! name AZIK風\n#! kind romaji\nka\tかん\n#! keys\nq\tk\n")
            .unwrap();
        assert_eq!(l.kind(), Kind::Romaji);
        assert_eq!(l.romaji_pairs().len(), 1);
        assert_eq!(from_u16s(&l.romaji_pairs()[0].1), "かん");
        assert!(l.handles("q")); // #! keys で読み替えるキーだけ
        assert!(!l.romaji_base_empty());
        let mut l2 = Layout::new();
        l2.load("#! kind romaji\n#! base empty\nka\tかん\n").unwrap();
        assert!(l2.romaji_base_empty());
    }

    #[test]
    fn prefixed_shift() {
        let mut l = Layout::new();
        l.load("#! kind kana\nd\t{p1}\np1>e\tね\ne\tえ\n").unwrap();
        assert!(l.handles("e"));
        assert_eq!(texts(&l.down("d", false, 0)), Vec::<String>::new()); // {p1} は出さない
        let out = l.down("e", false, 10);
        assert_eq!(texts(&out), vec!["ね"]);
        assert_eq!(out[0].unit, "p1>e");
    }

    #[test]
    fn key_event_token() {
        let mut l = Layout::new();
        l.load("#! kind kana\na\tあ{bs}\n").unwrap();
        let out = l.down("a", false, 0);
        assert_eq!(out.len(), 2);
        assert_eq!(from_u16s(&out[0].text), "あ");
        assert_eq!(out[1].key, "bs");
    }

    #[test]
    fn thumb_shift_and_tap() {
        let mut l = Layout::new();
        l.load("#! kind kana\na\tあ\nlthumb+a\tぁ\nspace\t \n").unwrap();
        assert_eq!(texts(&l.down("lthumb", false, 0)), Vec::<String>::new());
        assert_eq!(texts(&l.down("a", false, 10)), vec!["ぁ"]);
        assert_eq!(texts(&l.up("a", 20)), Vec::<String>::new());
        assert_eq!(texts(&l.up("lthumb", 30)), Vec::<String>::new()); // 組んだので単打は出ない
        assert_eq!(texts(&l.down("lthumb", false, 100)), Vec::<String>::new());
        let out = l.up("lthumb", 110);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].key, "muhenkan");
        let mut l2 = Layout::new();
        l2.load("#! kind kana\nlthumb+a\tぁ\n").unwrap();
        assert_eq!(l2.map_key("space"), "space"); // thumbs=0 はそのまま
        l2.thumbs = 1;
        assert_eq!(l2.map_key("space"), "rthumb");
        assert_eq!(l2.map_key("rthumb"), "");
    }

    #[test]
    fn chord_kind_overlap() {
        let mut l = Layout::new();
        l.load("#! kind chord\na\tあ\ns\tさ\na+s\tざ\n").unwrap();
        assert_eq!(texts(&l.down("a", false, 0)), Vec::<String>::new());
        assert_eq!(texts(&l.down("s", false, 10)), Vec::<String>::new());
        // 先に離した時点で、重なりが十分なら組として確定する (元の resolve の決まり)
        let out = l.up("a", 100);
        assert_eq!(texts(&out), vec!["ざ"]);
        assert_eq!(texts(&l.up("s", 110)), Vec::<String>::new());
        let mut l2 = Layout::new();
        l2.load("#! kind chord\na\tあ\ns\tさ\na+s\tざ\n").unwrap();
        l2.down("a", false, 0);
        let out = l2.tick(200); // window (100ms) を過ぎたら単打
        assert_eq!(texts(&out), vec!["あ"]);
    }

    #[test]
    fn typo_variants_letters() {
        let mut l = Layout::new();
        let mut tsv = String::from("#! kind letters\n");
        for c in "abcdefghijklmnopqrstuvwxyz".chars() {
            tsv += &format!("{}\t{}\n", c, c);
        }
        l.load(&tsv).unwrap();
        let units: Vec<String> = vec!["k".into(), "a".into()];
        let vars = l.typo_variants(&units);
        assert!(!vars.is_empty());
        let as_str: Vec<String> = vars.iter().map(|v| from_u16s(&v.concat())).collect();
        assert!(as_str.contains(&"a".to_string())); // 余分な 1 打を消す
        assert!(as_str.contains(&"ak".to_string())); // 入れ替え
        assert!(as_str.contains(&"kaa".to_string())); // 抜けた 1 打 (a を足す)
        let bad: Vec<String> = vec!["k".into(), "1".into()];
        assert!(l.typo_variants(&bad).is_empty());
        let alts = l.unit_alts("k");
        assert!(alts.contains(&"j".to_string()));
        assert!(alts.contains(&"l".to_string()));
        assert!(l.all_units().contains(&"k".to_string()));
        assert_eq!(l.unit_text("k"), crate::u16s("k"));
    }

    #[test]
    fn scancode_names() {
        assert_eq!(key_name_from_scancode(0x10, false), "q");
        assert_eq!(key_name_from_scancode(0x39, false), "space");
        assert_eq!(key_name_from_scancode(0x7B, false), "lthumb");
        assert_eq!(key_name_from_scancode(0x10, true), "");
        assert_eq!(key_name_from_scancode(0xFF, false), "");
    }
}
