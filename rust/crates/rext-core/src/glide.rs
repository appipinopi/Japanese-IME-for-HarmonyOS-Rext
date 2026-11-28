//! グライド入力 (QWERTY をなぞる) のデコーダ — `entry/src/main/cpp/glide.cpp` (595行),
//! `glide.h` (71行), `glide_nn.cpp` (175行), `glide_nn.h` (30行) の Rust 移植。
//!
//! 構成:
//! - [`Decoder`]: 読みの辞書の木をローマ字の綴り (同じキーが続くのは 1 つ) でたどる
//!   点ごとのビームサーチ ([`Decoder::decode`]) と、ニューラルネットの log p を
//!   辞書の木に沿って解く CTC ビームサーチ ([`Decoder::decode_ctc`])。
//! - [`Net`]: `glide_nn.bin` ("GNN1" + d + float32 の重み) の推論。
//! - [`GlideWords`]: `glide_en.tsv` / `glide_zh.tsv` (キー TAB 表記|表記 TAB コスト, UTF-8) の保持と検索。
//! - [`ffi`]: `engine.h` の glide 系 extern "C" 関数 5 本の輸出。
//!
//! エンジン連携について: C++ では `kkc_glide_prepare` / `kkc_glide_decode` が
//! `engine.cpp` の `glide_build` 経由でエンジン本体の辞書 (Lex) から読み一覧を取り、木を作る。
//! Rust 側も同じ形で、内部関数 [`ffi::engine_lexicon_words`] が
//! `engine::kkc_engine::glide_lexicon_words` (engine.rs 側の公開 API) を呼ぶ。
//! それ以外の経路 (TSV で渡される語一覧からの木の自前構築、拼音候補、ネット) は
//! このファイル内で完結する。
//!
//! 移植上の注意: C++ の `Search` 構造体にあった DP (`visit`/`rec`/`apply_all`/`step`) は
//! `Decoder::decode` からは呼ばれない到達不能コード (実際に使われるのは `Beam`) のため、
//! `Beam` とその土台 (`segpre`, `kc`/`kcpre`) のみを移植した。

use std::collections::{BTreeMap, HashMap};
use std::sync::OnceLock;

pub const NOC: i16 = i16::MAX;
const NSLOT: usize = 320;
const INF: f32 = 1e30;

/// Kana: 読みはひらがな、綴りの表でキーに直す。Letters: 読みが英字そのまま (英語・ピンイン)。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Kana,
    Letters,
}

/// キーの文字と中心 (キーの幅を 1 にした座標)。
#[derive(Clone, Copy, Debug)]
pub struct Key {
    pub c: u8,
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Debug)]
pub struct Result {
    pub reading: Vec<u16>,
    pub cost: f32,
    pub path: f32,
}

/// `glide.h` の `Params` と同じ既定値。
#[derive(Clone, Debug)]
pub struct Params {
    pub spacing: f32,
    pub max_pts: i32,
    pub sigma_key: f32,
    pub sigma_seg: f32,
    pub lex_w: f32,
    pub word_pen: f32,
    pub beam: f32,
    pub abs_bound: f32,
    pub max_words: i32,
    pub same_key_pen: f32,
    pub dir_w: f32,
    pub beam_size: i32,
    pub nbest: i32,
    pub max_expand: u64,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            spacing: 0.25,
            max_pts: 128,
            sigma_key: 0.45,
            sigma_seg: 0.45,
            lex_w: 1.0 / 1000.0,
            word_pen: 8.0,
            beam: 20.0,
            abs_bound: 60.0,
            max_words: 3,
            same_key_pen: 0.5,
            dir_w: 4.0,
            beam_size: 3000,
            nbest: 10,
            max_expand: 2_000_000,
        }
    }
}

// ---------------------------------------------------------------- 綴りの表

fn collapse(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        if o.chars().last() != Some(c) {
            o.push(c);
        }
    }
    o
}

type SpellMap = BTreeMap<Vec<u16>, Vec<String>>;

fn spell_add(m: &mut SpellMap, key: &[u16], rs: &[&str]) {
    let v = m.entry(key.to_vec()).or_default();
    for r in rs {
        let c = collapse(r);
        if !v.contains(&c) {
            v.push(c);
        }
    }
}

fn u16c(c: char) -> Vec<u16> {
    let mut buf = [0u16; 2];
    c.encode_utf16(&mut buf).to_vec()
}

/// `glide.cpp` の `Spell` と同じ表 (かな 1 字か 2 字 -> 綴り。綴りは同じ字の連続を 1 つにまとめた後)。
fn spell() -> &'static SpellMap {
    static SP: OnceLock<SpellMap> = OnceLock::new();
    SP.get_or_init(|| {
        let mut m: SpellMap = BTreeMap::new();
        let rows: [(&str, &str); 14] = [
            ("あいうえお", ""),
            ("かきくけこ", "k"),
            ("さしすせそ", "s"),
            ("たちつてと", "t"),
            ("なにぬねの", "n"),
            ("はひふへほ", "h"),
            ("まみむめも", "m"),
            ("らりるれろ", "r"),
            ("がぎぐげご", "g"),
            ("ざじずぜぞ", "z"),
            ("だぢづでど", "d"),
            ("ばびぶべぼ", "b"),
            ("ぱぴぷぺぽ", "p"),
            ("ぁぃぅぇぉ", "x"),
        ];
        let vs = ['a', 'i', 'u', 'e', 'o'];
        for (kana, cons) in rows {
            for (i, ch) in kana.chars().enumerate() {
                let r = format!("{}{}", cons, vs[i]);
                spell_add(&mut m, &u16c(ch), &[&r]);
            }
        }
        let singles: [(&str, &[&str]); 32] = [
            ("し", &["shi"]),
            ("ち", &["chi"]),
            ("つ", &["tsu"]),
            ("ふ", &["fu"]),
            ("じ", &["ji"]),
            ("ぁ", &["la"]),
            ("ぃ", &["li"]),
            ("ぅ", &["lu"]),
            ("ぇ", &["le"]),
            ("ぉ", &["lo"]),
            ("や", &["ya"]),
            ("ゆ", &["yu"]),
            ("よ", &["yo"]),
            ("わ", &["wa"]),
            ("を", &["wo"]),
            ("ん", &["n"]),
            ("ー", &["-"]),
            ("ゔ", &["vu"]),
            ("っ", &[""]),
            ("ゃ", &["xya"]),
            ("ゅ", &["xyu"]),
            ("ょ", &["xyo"]),
            ("しゃ", &["sha"]),
            ("しゅ", &["shu"]),
            ("しょ", &["sho"]),
            ("ちゃ", &["cha", "cya"]),
            ("ちゅ", &["chu", "cyu"]),
            ("ちょ", &["cho", "cyo"]),
            ("じゃ", &["ja", "jya"]),
            ("じゅ", &["ju", "jyu"]),
            ("じょ", &["jo", "jyo"]),
            ("しぇ", &["she", "sye"]),
        ];
        for (k, rs) in singles {
            spell_add(&mut m, &crate::u16s(k), rs);
        }
        spell_add(&mut m, &crate::u16s("ちぇ"), &["che", "tye"]);
        spell_add(&mut m, &crate::u16s("じぇ"), &["je", "zye"]);
        spell_add(&mut m, &crate::u16s("ふぁ"), &["fa"]);
        spell_add(&mut m, &crate::u16s("ふぃ"), &["fi"]);
        spell_add(&mut m, &crate::u16s("ふぇ"), &["fe"]);
        spell_add(&mut m, &crate::u16s("ふぉ"), &["fo"]);
        spell_add(&mut m, &crate::u16s("てぃ"), &["thi"]);
        spell_add(&mut m, &crate::u16s("でぃ"), &["dhi"]);
        spell_add(&mut m, &crate::u16s("うぃ"), &["wi"]);
        spell_add(&mut m, &crate::u16s("うぇ"), &["we"]);
        spell_add(&mut m, &crate::u16s("ゔぁ"), &["va"]);
        spell_add(&mut m, &crate::u16s("ゔぃ"), &["vi"]);
        spell_add(&mut m, &crate::u16s("ゔぇ"), &["ve"]);
        spell_add(&mut m, &crate::u16s("ゔぉ"), &["vo"]);
        // 拗音 (きゃ など)
        let yoon: [(char, &str); 12] = [
            ('き', "ky"),
            ('し', "sy"),
            ('ち', "ty"),
            ('に', "ny"),
            ('ひ', "hy"),
            ('み', "my"),
            ('り', "ry"),
            ('ぎ', "gy"),
            ('じ', "zy"),
            ('び', "by"),
            ('ぴ', "py"),
            ('ぢ', "dy"),
        ];
        let sm = ['ゃ', 'ゅ', 'ょ'];
        let yv = ['a', 'u', 'o'];
        for (base, pre) in yoon {
            for i in 0..3 {
                let mut k = u16c(base);
                k.extend(u16c(sm[i]));
                let r = format!("{}{}", pre, yv[i]);
                spell_add(&mut m, &k, &[&r]);
            }
        }
        m
    })
}

/// 試験用: 読みをローマ字の綴り (まとめる前) にする。
/// `rng` があり、綴りが複数あって `rng() % 10 < 3` のとき 2 番目以降から選ぶ (C++ の `romanize` と同じ)。
/// 綴りの表に無い字があれば空文字列を返す。
pub fn romanize(rd: &[u16], rng: Option<&mut dyn FnMut() -> u32>) -> String {
    let sp = spell();
    let mut rng = rng;
    let mut o = String::new();
    let mut i = 0;
    while i < rd.len() {
        let (v, n) = if i + 1 < rd.len() {
            match sp.get(&rd[i..i + 2]) {
                Some(v) => (v, 2),
                None => match sp.get(&rd[i..i + 1]) {
                    Some(v) => (v, 1),
                    None => return String::new(),
                },
            }
        } else {
            match sp.get(&rd[i..i + 1]) {
                Some(v) => (v, 1),
                None => return String::new(),
            }
        };
        let mut pick = 0usize;
        if let Some(r) = rng.as_deref_mut() {
            if v.len() > 1 && r() % 10 < 3 {
                pick = 1 + (r() as usize % (v.len() - 1));
            }
        }
        o += &v[pick];
        i += n;
    }
    o
}

// ---------------------------------------------------------------- デコーダ

#[derive(Clone, Copy)]
struct Node {
    ch: u16,
    cost: i16,
    minsub: i16,
    first: i32,
    next: i32,
}

impl Default for Node {
    fn default() -> Self {
        Node { ch: 0, cost: NOC, minsub: NOC, first: -1, next: -1 }
    }
}

fn slot(c: u16) -> i32 {
    if (0x3040..0x3100).contains(&(c as i32)) {
        c as i32 - 0x3040
    } else if c < 128 {
        192 + c as i32
    } else {
        -1
    }
}

pub struct Decoder {
    node_: Vec<Node>,
    keys_: Vec<Key>,
    kidx_: [i32; 128],
    mode_: Mode,
    sp1_: Vec<Vec<Vec<i8>>>,
    has2_: Vec<u8>,
    vowel_: Vec<u8>,
    sp2_: HashMap<u32, Vec<Vec<i8>>>,
    two_: HashMap<i32, Vec<i32>>,
    pub last_expanded: u64,
}

impl Default for Decoder {
    fn default() -> Self {
        Decoder {
            node_: Vec::new(),
            keys_: Vec::new(),
            kidx_: [-1; 128],
            mode_: Mode::Kana,
            sp1_: Vec::new(),
            has2_: Vec::new(),
            vowel_: Vec::new(),
            sp2_: HashMap::new(),
            two_: HashMap::new(),
            last_expanded: 0,
        }
    }
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn nodes(&self) -> usize {
        self.node_.len()
    }

    /// words: 読み (ひらがな・ー) と辞書のコスト。同じ読みが複数あれば小さい方を使う。
    pub fn build(&mut self, words0: &[(Vec<u16>, i32)], mode: Mode) {
        self.mode_ = mode;
        let mut words: Vec<(Vec<u16>, i32)> = words0.to_vec();
        words.sort();
        self.node_ = vec![Node::default()];
        for (s, cost) in &words {
            let mut n = 0usize;
            for &ch in s {
                let mut c = self.node_[n].first;
                let mut found: i32 = -1;
                while c >= 0 {
                    if self.node_[c as usize].ch == ch {
                        found = c;
                        break;
                    }
                    c = self.node_[c as usize].next;
                }
                if found < 0 {
                    let id = self.node_.len() as i32;
                    self.node_.push(Node { ch, ..Node::default() });
                    if self.node_[n].first < 0 {
                        self.node_[n].first = id;
                    } else {
                        let mut last = self.node_[n].first;
                        while self.node_[last as usize].next >= 0 {
                            last = self.node_[last as usize].next;
                        }
                        self.node_[last as usize].next = id;
                    }
                    found = id;
                }
                n = found as usize;
            }
            let cl = (*cost).clamp(0, NOC as i32 - 1) as i16;
            if cl < self.node_[n].cost {
                self.node_[n].cost = cl;
            }
        }
        // 子は親より後ろに作ったので、後ろから親へ部分木の最小コストを送る
        let mut par = vec![-1i32; self.node_.len()];
        for i in 0..self.node_.len() {
            let mut c = self.node_[i].first;
            while c >= 0 {
                par[c as usize] = i as i32;
                c = self.node_[c as usize].next;
            }
        }
        for i in (0..self.node_.len()).rev() {
            self.node_[i].minsub = self.node_[i].minsub.min(self.node_[i].cost);
            if par[i] >= 0 {
                let p = par[i] as usize;
                let ms = self.node_[i].minsub;
                if ms < self.node_[p].minsub {
                    self.node_[p].minsub = ms;
                }
            }
        }
    }

    pub fn set_keys(&mut self, keys: &[Key]) {
        self.keys_ = keys.to_vec();
        self.kidx_ = [-1; 128];
        for (i, k) in keys.iter().enumerate() {
            self.kidx_[(k.c & 127) as usize] = i as i32;
        }
        self.sp1_ = vec![Vec::new(); NSLOT];
        self.has2_ = vec![0; NSLOT];
        self.sp2_.clear();
        let kidx = self.kidx_;
        let conv = |r: &str, o: &mut Vec<i8>| -> bool {
            o.clear();
            for c in r.bytes() {
                let k = kidx[(c & 127) as usize];
                if k < 0 {
                    return false;
                }
                o.push(k as i8);
            }
            true
        };
        let mut o: Vec<i8> = Vec::new();
        if self.mode_ == Mode::Kana {
            for (k, rs) in spell().iter() {
                for r in rs {
                    if !conv(r, &mut o) {
                        continue;
                    }
                    if r.starts_with('x') || r.starts_with('l') {
                        continue;
                    }
                    if k.len() == 1 {
                        let s = slot(k[0]);
                        if s >= 0 {
                            self.sp1_[s as usize].push(o.clone());
                        }
                    } else {
                        let s0 = slot(k[0]);
                        let s1 = slot(k[1]);
                        if s0 >= 0 && s1 >= 0 {
                            self.sp2_
                                .entry(((s0 as u32) << 10) | (s1 as u32))
                                .or_default()
                                .push(o.clone());
                            self.has2_[s0 as usize] = 1;
                        }
                    }
                }
            }
        } else {
            for c in 0..128usize {
                if self.kidx_[c] >= 0 {
                    self.sp1_[192 + c].push(vec![self.kidx_[c] as i8]);
                }
            }
        }
        self.vowel_ = vec![0; self.keys_.len()];
        for v in "aiueon-".bytes() {
            if self.kidx_[v as usize] >= 0 {
                self.vowel_[self.kidx_[v as usize] as usize] = 1;
            }
        }
        // 2 字の綴りの後ろの字になる子を節ごとに
        self.two_.clear();
        let mut second = vec![0u8; NSLOT];
        for key in self.sp2_.keys() {
            second[(key & 1023) as usize] = 1;
        }
        for i in 0..self.node_.len() {
            let s1 = slot(self.node_[i].ch);
            if s1 < 0 || self.has2_[s1 as usize] == 0 {
                continue;
            }
            let mut c = self.node_[i].first;
            while c >= 0 {
                let s2 = slot(self.node_[c as usize].ch);
                if s2 >= 0
                    && second[s2 as usize] != 0
                    && self.sp2_.contains_key(&(((s1 as u32) << 10) | (s2 as u32)))
                {
                    self.two_.entry(i as i32).or_default().push(c);
                }
                c = self.node_[c as usize].next;
            }
        }
    }
}

// ---------------------------------------------------------------- ビームサーチ (C++ の Search + Beam)

struct Search<'a> {
    d: &'a Decoder,
    p: &'a Params,
    n: usize,
    k: usize,
    wpt: f32,
    px: Vec<f32>,
    py: Vec<f32>,
    tx: Vec<f32>,
    ty: Vec<f32>,
    kc: Vec<f32>,
    kcpre: Vec<f32>,
    segc: Vec<Vec<f32>>, // (a * K + b) -> a-b の線からのずれの累積 (空 = まだ)
    best: f32,
}

impl<'a> Search<'a> {
    fn segpre(&mut self, a: usize, b: usize) -> &[f32] {
        let idx = a * self.k + b;
        if !self.segc[idx].is_empty() {
            return &self.segc[idx];
        }
        let (ax, ay) = (self.d.keys_[a].x, self.d.keys_[a].y);
        let (bx, by) = (self.d.keys_[b].x, self.d.keys_[b].y);
        let (vx, vy) = (bx - ax, by - ay);
        let vv = vx * vx + vy * vy;
        let inv = self.wpt / (self.p.sigma_seg * self.p.sigma_seg);
        let mut s = vec![0.0f32; self.n];
        let mut acc = 0.0f32;
        for t in 0..self.n {
            let mut u = if vv > 0.0 {
                ((self.px[t] - ax) * vx + (self.py[t] - ay) * vy) / vv
            } else {
                0.0
            };
            u = u.clamp(0.0, 1.0);
            let dx = self.px[t] - (ax + u * vx);
            let dy = self.py[t] - (ay + u * vy);
            acc += (dx * dx + dy * dy) * inv;
            if vv > 0.0 {
                let cs = (self.tx[t] * vx + self.ty[t] * vy) / vv.sqrt();
                acc += self.p.dir_w * self.wpt * (1.0 - cs) * 0.5;
            }
            s[t] = acc;
        }
        self.segc[idx] = s;
        &self.segc[idx]
    }
}

/// 綴りへの参照 (C++ の `const std::vector<int8_t>*` の代わり)。
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum SpRef {
    One(usize, usize), // (slot, 綴りの番号)
    Two(u32, usize),   // (sp2 のキー, 綴りの番号)
}

impl SpRef {
    fn id(self) -> u64 {
        match self {
            SpRef::One(s, i) => (s as u64) * 1_000_003 + i as u64,
            SpRef::Two(k, i) => (1u64 << 40) | (k as u64) * 1_000_003 + i as u64,
        }
    }
}

fn spell_of(d: &Decoder, sp: SpRef) -> &[i8] {
    match sp {
        SpRef::One(s, i) => &d.sp1_[s][i],
        SpRef::Two(k, i) => &d.sp2_[&k][i],
    }
}

#[derive(Clone, Copy)]
struct Tok {
    cost: f32,
    lex: f32,
    node: i32,
    hist: i32,
    sp: SpRef,
    pos: i8,
    a: i8,
    words: i8,
    c1: u16,
    c2: u16,
}

#[derive(Clone, Copy)]
struct Hist {
    parent: i32,
    c1: u16,
    c2: u16,
}

struct Beam {
    cur: Vec<Tok>,
    nxt: Vec<Tok>,
    hist: Vec<Hist>,
    idx: HashMap<u64, i32>,
    fin: HashMap<i32, (f32, f32)>, // 履歴 -> (合計, なぞり)
    pushed: u64,
    t: usize,
}

impl Beam {
    fn lex_lb(&self, s: &Search, node: i32) -> f32 {
        let ms = s.d.node_[node as usize].minsub;
        if ms == NOC {
            INF
        } else {
            s.p.lex_w * ms as f32 + s.p.word_pen
        }
    }

    fn push(&mut self, k: Tok) {
        self.pushed += 1;
        let key = ((k.node as u32 as u64) << 32)
            ^ (k.sp.id().wrapping_mul(0x9E3779B97F4A7C15))
            ^ (((k.pos as u8) as u64) << 8)
            ^ ((k.a as u8) as u64)
            ^ (((k.words as u8) as u64) << 16);
        match self.idx.get(&key) {
            None => {
                self.idx.insert(key, self.nxt.len() as i32);
                self.nxt.push(k);
            }
            Some(&i) => {
                let o = &mut self.nxt[i as usize];
                if k.cost + k.lex < o.cost + o.lex {
                    *o = k;
                }
            }
        }
    }

    /// 節 n にキー a で着いた (履歴 h)。子へ向かうトークンを作る。最後の点なら結果に。
    #[allow(clippy::too_many_arguments)]
    fn arrive(
        &mut self,
        s: &mut Search,
        n: i32,
        a: i8,
        cost: f32,
        lex: f32,
        words: i8,
        h: i32,
        depth: i32,
    ) {
        if depth > 8 {
            return;
        }
        let (ncost, nfirst) = {
            let nd = &s.d.node_[n as usize];
            (nd.cost, nd.first)
        };
        if n != 0 && ncost != NOC {
            let l2 = lex + s.p.lex_w * ncost as f32 + s.p.word_pen;
            if self.t == s.n - 1 {
                let tot = cost + l2;
                match self.fin.get(&h) {
                    Some(&(old, _)) if old <= tot => {}
                    _ => {
                        self.fin.insert(h, (tot, cost));
                    }
                }
            } else if (words as i32) + 1 < s.p.max_words {
                self.arrive(s, 0, a, cost, l2, words + 1, h, depth + 1);
            }
        }
        if self.t == s.n - 1 {
            return;
        }
        let mut c = nfirst;
        while c >= 0 {
            let ch = s.d.node_[c as usize].ch;
            let next = s.d.node_[c as usize].next;
            let s1 = slot(ch);
            if s1 >= 0 {
                let cnt = s.d.sp1_[s1 as usize].len();
                for i in 0..cnt {
                    self.start(s, c, SpRef::One(s1 as usize, i), a, cost, lex, words, h, ch, 0, depth);
                }
                if s.d.has2_[s1 as usize] != 0 {
                    if let Some(tw) = s.d.two_.get(&c) {
                        let tw = tw.clone();
                        for g2 in tw {
                            let s2 = slot(s.d.node_[g2 as usize].ch);
                            let key = ((s1 as u32) << 10) | (s2 as u32);
                            if let Some(v) = s.d.sp2_.get(&key) {
                                let cnt2 = v.len();
                                let ch2 = s.d.node_[g2 as usize].ch;
                                for i in 0..cnt2 {
                                    self.start(s, g2, SpRef::Two(key, i), a, cost, lex, words, h, ch, ch2, depth);
                                }
                            }
                        }
                    }
                }
            }
            c = next;
        }
    }

    /// 綴り sp で節 n へ向かうトークン。最後のキーと同じキーで始まる字は、点を使わずに済ませる。
    #[allow(clippy::too_many_arguments)]
    fn start(
        &mut self,
        s: &mut Search,
        n: i32,
        sp: SpRef,
        a: i8,
        cost0: f32,
        lex: f32,
        words: i8,
        h: i32,
        c1: u16,
        c2: u16,
        depth: i32,
    ) {
        let spv: Vec<i8> = spell_of(s.d, sp).to_vec();
        // っ の後ろは子音
        if self.hist[h as usize].c1 == 'っ' as u16
            && self.hist[h as usize].c2 == 0
            && (spv.is_empty() || s.d.vowel_[spv[0] as usize] != 0)
        {
            return;
        }
        // ん・ー の後ろと先頭に っ は来ない
        if c1 == 'っ' as u16
            && (h == 0
                || self.hist[h as usize].c1 == 'ん' as u16
                || self.hist[h as usize].c1 == 'っ' as u16
                || self.hist[h as usize].c1 == 'ー' as u16)
        {
            return;
        }
        let mut cost = cost0;
        let mut pos = 0usize;
        while pos < spv.len() && spv[pos] == a {
            pos += 1;
            cost += s.p.same_key_pen;
        }
        if cost + lex + self.lex_lb(s, n) > s.best + s.p.beam {
            return;
        }
        if pos == spv.len() {
            // っ・同じキーだけの綴り
            if a < 0 {
                return;
            }
            self.hist.push(Hist { parent: h, c1, c2 });
            let nh = self.hist.len() as i32 - 1;
            self.arrive(s, n, a, cost, lex, words, nh, depth + 1);
            return;
        }
        self.push(Tok { cost, lex, node: n, hist: h, sp, pos: pos as i8, a, words, c1, c2 });
    }

    fn step(&mut self, s: &mut Search, k: Tok) {
        let t = self.t;
        let spv: Vec<i8> = spell_of(s.d, k.sp).to_vec();
        let b = spv[k.pos as usize];
        // 線の上を進む
        let sc = if k.a < 0 {
            s.kc[b as usize * s.n + t] * s.wpt
        } else {
            let sv = s.segpre(k.a as usize, b as usize);
            if t == 0 {
                sv[0]
            } else {
                sv[t] - sv[t - 1]
            }
        };
        {
            let mut tk = k;
            tk.cost += sc;
            if tk.cost + tk.lex + self.lex_lb(s, tk.node) <= s.best + s.p.beam {
                self.push(tk);
            }
        }
        // キー b に着く
        let c2 = k.cost + s.kc[b as usize * s.n + t];
        if c2 + k.lex + self.lex_lb(s, k.node) > s.best + s.p.beam {
            return;
        }
        let mut pos = k.pos as usize + 1;
        let mut c2x = c2;
        while pos < spv.len() && spv[pos] == b {
            pos += 1;
            c2x += s.p.same_key_pen;
        }
        if pos < spv.len() {
            let mut tk = k;
            tk.cost = c2x;
            tk.pos = pos as i8;
            tk.a = b;
            self.push(tk);
            return;
        }
        self.hist.push(Hist { parent: k.hist, c1: k.c1, c2: k.c2 });
        let nh = self.hist.len() as i32 - 1;
        self.arrive(s, k.node, b, c2x, k.lex, k.words, nh, 0);
    }

    fn run(&mut self, s: &mut Search) {
        self.hist.push(Hist { parent: -1, c1: 0, c2: 0 });
        s.best = INF;
        self.t = 0;
        self.nxt.clear();
        self.idx.clear();
        self.arrive(s, 0, -1, 0.0, 0.0, 0, 0, 0);
        for t in 0..s.n {
            self.t = t;
            std::mem::swap(&mut self.cur, &mut self.nxt);
            self.nxt.clear();
            self.idx.clear();
            let mut b0 = INF;
            for k in &self.cur {
                b0 = b0.min(k.cost + k.lex + self.lex_lb(s, k.node));
            }
            s.best = b0;
            if self.cur.len() as i32 > s.p.beam_size {
                // C++ の nth_element + resize と同じ (上位 beam_size 件を残す)
                let bs = s.p.beam_size as usize;
                let costs: Vec<f32> = self
                    .cur
                    .iter()
                    .map(|k| k.cost + k.lex + self.lex_lb(s, k.node))
                    .collect();
                let mut idxs: Vec<usize> = (0..self.cur.len()).collect();
                idxs.select_nth_unstable_by(bs, |&a, &b| {
                    costs[a].partial_cmp(&costs[b]).unwrap_or(std::cmp::Ordering::Equal)
                });
                idxs.truncate(bs);
                let old = std::mem::take(&mut self.cur);
                self.cur = idxs.into_iter().map(|i| old[i]).collect();
            }
            let cur = std::mem::take(&mut self.cur);
            for k in &cur {
                let k = *k;
                self.step(s, k);
            }
            self.cur = cur;
        }
    }

    fn text(&self, mut h: i32) -> Vec<u16> {
        let mut r = Vec::new();
        while h > 0 {
            let e = self.hist[h as usize];
            if e.c2 != 0 {
                r.push(e.c2);
            }
            r.push(e.c1);
            h = e.parent;
        }
        r.reverse();
        r
    }
}

impl Decoder {
    /// pts: 軌跡 (x, y の並び、キーの座標と同じ単位)
    pub fn decode(&mut self, pts: &[f32], p: &Params) -> Vec<Result> {
        let mut out = Vec::new();
        let n0 = pts.len() / 2;
        if n0 < 2 || self.keys_.is_empty() || self.node_.is_empty() {
            return out;
        }
        // 等間隔に取り直す
        let mut cum = vec![0.0f32; n0];
        for i in 1..n0 {
            cum[i] = cum[i - 1]
                + (pts[2 * i] - pts[2 * i - 2]).hypot(pts[2 * i + 1] - pts[2 * i - 1]);
        }
        let len = cum[n0 - 1];
        if len < 0.5 {
            return out; // なぞりでなくタップ
        }
        let n = ((len / p.spacing) as i32 + 1).clamp(12, p.max_pts) as usize;
        let wpt = (len / (n - 1) as f32) / 0.5;
        let mut s = Search {
            d: &*self,
            p,
            n,
            k: self.keys_.len(),
            wpt,
            px: vec![0.0; n],
            py: vec![0.0; n],
            tx: vec![0.0; n],
            ty: vec![0.0; n],
            kc: Vec::new(),
            kcpre: Vec::new(),
            segc: Vec::new(),
            best: INF,
        };
        {
            let mut i = 0usize;
            for t in 0..n {
                let at = len * t as f32 / (n - 1) as f32;
                while i + 1 < n0 - 1 && cum[i + 1] < at {
                    i += 1;
                }
                let seg = cum[i + 1] - cum[i];
                let u = if seg > 0.0 {
                    ((at - cum[i]) / seg).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                s.px[t] = pts[2 * i] + u * (pts[2 * i + 2] - pts[2 * i]);
                s.py[t] = pts[2 * i + 1] + u * (pts[2 * i + 3] - pts[2 * i + 1]);
            }
            for t in 0..n {
                let a0 = t.saturating_sub(1);
                let a1 = (t + 1).min(n - 1);
                let vx = s.px[a1] - s.px[a0];
                let vy = s.py[a1] - s.py[a0];
                let l = vx.hypot(vy);
                if l > 0.0 {
                    s.tx[t] = vx / l;
                    s.ty[t] = vy / l;
                }
            }
            let kk = s.k;
            s.segc = vec![Vec::new(); kk * kk];
            s.kc = vec![0.0; kk * n];
            s.kcpre = vec![0.0; kk * n];
            let inv = 1.0 / (p.sigma_key * p.sigma_key);
            for k in 0..kk {
                let mut acc = 0.0f32;
                for t in 0..n {
                    let dx = s.px[t] - self.keys_[k].x;
                    let dy = s.py[t] - self.keys_[k].y;
                    s.kc[k * n + t] = (dx * dx + dy * dy) * inv;
                    acc += (dx * dx + dy * dy) * inv * wpt;
                    s.kcpre[k * n + t] = acc;
                }
            }
        }
        let mut b = Beam {
            cur: Vec::new(),
            nxt: Vec::new(),
            hist: Vec::new(),
            idx: HashMap::new(),
            fin: HashMap::new(),
            pushed: 0,
            t: 0,
        };
        b.run(&mut s);
        self.last_expanded = b.pushed;
        let mut by: HashMap<Vec<u16>, (f32, f32)> = HashMap::new();
        for (h, v) in &b.fin {
            let r = b.text(*h);
            match by.get(&r) {
                Some(&(old, _)) if old <= v.0 => {}
                _ => {
                    by.insert(r, *v);
                }
            }
        }
        for (r, v) in by {
            out.push(Result { reading: r, cost: v.0, path: v.1 });
        }
        out.sort_by(|a, b| a.cost.partial_cmp(&b.cost).unwrap_or(std::cmp::Ordering::Equal));
        if out.len() as i32 > p.nbest {
            out.truncate(p.nbest.max(0) as usize);
        }
        out
    }

    /// ネットワーク (glide_nn) の log p [frames × (1 + キーの数)] を、辞書の木に沿った
    /// CTC のビームサーチで語にする (Letters の形だけ)。
    /// score = CTC の log p + lm_w × log p(語) + len_w × 字数、cost = −score。
    pub fn decode_ctc(
        &self,
        lp: &[f32],
        frames: i32,
        nkeys: i32,
        lm_w: f32,
        len_w: f32,
        beam: i32,
        nbest: i32,
    ) -> Vec<Result> {
        #[derive(Clone)]
        struct Hyp {
            node: i32,
            pb: f32,
            pnb: f32,
            s: Vec<u16>,
        }
        fn lse(a: f32, b: f32) -> f32 {
            let (a, b) = if a < b { (b, a) } else { (a, b) };
            if b <= -1e29 {
                a
            } else {
                a + (b - a).exp().ln_1p()
            }
        }
        const NEG: f32 = -1e30;
        let w = 1 + nkeys;
        let key_of = |d: &Decoder, nd: i32| -> i32 {
            let c = d.node_[nd as usize].ch;
            if c < 128 {
                d.kidx_[c as usize]
            } else {
                -1
            }
        };
        if frames <= 0 || nkeys <= 0 || self.node_.is_empty() {
            return Vec::new();
        }
        let mut cur: Vec<Hyp> = vec![Hyp { node: 0, pb: 0.0, pnb: NEG, s: Vec::new() }];
        let mut at: HashMap<i32, usize> = HashMap::new();
        // 同じキーの字が続く所 (hello の ll) はなぞりでは見えないので、子へ放出なしで進める
        fn close(d: &Decoder, key_of: &dyn Fn(&Decoder, i32) -> i32, hs: &mut Vec<Hyp>, at: &mut HashMap<i32, usize>) {
            let mut i = 0;
            while i < hs.len() {
                let nd = hs[i].node;
                if nd != 0 {
                    let k = key_of(d, nd);
                    let mut c = d.node_[nd as usize].first;
                    while c >= 0 {
                        if key_of(d, c) == k {
                            let (pb, pnb) = (hs[i].pb, hs[i].pnb);
                            match at.get(&c) {
                                Some(&j) => {
                                    let (opb, opnb) = (hs[j].pb, hs[j].pnb);
                                    hs[j].pb = lse(opb, pb);
                                    hs[j].pnb = lse(opnb, pnb);
                                }
                                None => {
                                    let mut s = hs[i].s.clone();
                                    s.push(d.node_[c as usize].ch);
                                    at.insert(c, hs.len());
                                    hs.push(Hyp { node: c, pb, pnb, s });
                                }
                            }
                        }
                        c = d.node_[c as usize].next;
                    }
                }
                i += 1;
            }
        }
        for t in 0..frames as usize {
            let r = &lp[t * w as usize..t * w as usize + w as usize];
            let mut nxt: Vec<Hyp> = Vec::new();
            at.clear();
            fn add(at: &mut HashMap<i32, usize>, nxt: &mut Vec<Hyp>, nd: i32, s: Vec<u16>, pb: f32, pnb: f32) {
                match at.get(&nd) {
                    Some(&j) => {
                        let (opb, opnb) = (nxt[j].pb, nxt[j].pnb);
                        nxt[j].pb = lse(opb, pb);
                        nxt[j].pnb = lse(opnb, pnb);
                    }
                    None => {
                        at.insert(nd, nxt.len());
                        nxt.push(Hyp { node: nd, pb, pnb, s });
                    }
                }
            }
            for h in &cur {
                let tot = lse(h.pb, h.pnb);
                add(&mut at, &mut nxt, h.node, h.s.clone(), tot + r[0], NEG); // blank
                let k = if h.node != 0 { key_of(self, h.node) } else { -1 };
                if k >= 0 {
                    add(&mut at, &mut nxt, h.node, h.s.clone(), NEG, h.pnb + r[1 + k as usize]);
                }
                let mut c = self.node_[h.node as usize].first;
                while c >= 0 {
                    let kc = key_of(self, c);
                    if kc >= 0 && kc != k {
                        let mut s = h.s.clone();
                        s.push(self.node_[c as usize].ch);
                        add(&mut at, &mut nxt, c, s, NEG, tot + r[1 + kc as usize]);
                    }
                    c = self.node_[c as usize].next;
                }
            }
            close(self, &key_of, &mut nxt, &mut at);
            let rank = |d: &Decoder, h: &Hyp| lse(h.pb, h.pnb) - lm_w * d.node_[h.node as usize].minsub as f32 / 500.0;
            if nxt.len() as i32 > beam {
                let b = beam as usize;
                nxt.select_nth_unstable_by(b, |a, bb| {
                    rank(self, bb).partial_cmp(&rank(self, a)).unwrap_or(std::cmp::Ordering::Equal)
                });
                nxt.truncate(b);
            }
            cur = nxt;
            at.clear();
            for (i, h) in cur.iter().enumerate() {
                at.insert(h.node, i);
            }
        }
        let mut out = Vec::new();
        for h in &cur {
            let c = self.node_[h.node as usize].cost;
            if h.node == 0 || c == NOC {
                continue;
            }
            let sc = lse(h.pb, h.pnb) - lm_w * c as f32 / 500.0 + len_w * h.s.len() as f32;
            out.push(Result { reading: h.s.clone(), cost: -sc, path: lse(h.pb, h.pnb) });
        }
        out.sort_by(|a, b| a.cost.partial_cmp(&b.cost).unwrap_or(std::cmp::Ordering::Equal));
        if out.len() as i32 > nbest {
            out.truncate(nbest.max(0) as usize);
        }
        out
    }
}

// ---------------------------------------------------------------- ネットワーク (glide_nn)

fn gelu(x: f32) -> f32 {
    0.5 * x * (1.0 + libm_erf(x * 0.70710678))
}

/// C++ `std::erf` (glibc erff、~1 ulp) に合わせた erf。不完全ガンマ関数の
/// 級数 (x² < 1.5) と連分数 (それ以上) を f64 で評価し、精度 ~1e-14 を得る。
/// 以前の Abramowitz–Stegun 7.1.26 は多項式自体の誤差が最大 1.5e-7 あった
/// (engine_simd.h の AVX2 版と同じ近似。スカラー C++ 版は std::erf)。
/// engine.rs の erf_f64 と同じ実装。
fn libm_erf(x: f32) -> f32 {
    let x = x as f64;
    if x == 0.0 {
        return 0.0;
    }
    let t = x * x;
    const LGAMMA_HALF: f64 = 0.5723649429247001; // ln Γ(1/2) = ln √π
    let p = if t < 1.5 {
        let mut ap = 0.5f64;
        let mut sum = 2.0f64;
        let mut del = sum;
        for _ in 0..1000 {
            ap += 1.0;
            del *= t / ap;
            sum += del;
            if del.abs() < sum.abs() * 1e-15 {
                break;
            }
        }
        sum * (-t + 0.5 * t.ln() - LGAMMA_HALF).exp()
    } else {
        let mut b = t + 0.5;
        let mut c = 1e300f64;
        let mut d = 1.0 / b;
        let mut h = d;
        for i in 1..1000 {
            let an = -(i as f64) * (i as f64 - 0.5);
            b += 2.0;
            d = an * d + b;
            if d.abs() < 1e-300 {
                d = 1e-300;
            }
            c = b + an / c;
            if c.abs() < 1e-300 {
                c = 1e-300;
            }
            d = 1.0 / d;
            let del = d * c;
            h *= del;
            if (del - 1.0).abs() < 1e-15 {
                break;
            }
        }
        1.0 - (-t + 0.5 * t.ln() - LGAMMA_HALF).exp() * h
    };
    (if x < 0.0 { -p } else { p }) as f32
}

fn sigm(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

fn logsig(x: f32) -> f32 {
    if x >= 0.0 {
        -(-x).exp().ln_1p()
    } else {
        x - x.exp().ln_1p()
    }
}

fn layernorm(x: &mut [f32], w: &[f32], b: &[f32]) {
    let d = x.len();
    let mut m = 0.0f32;
    for v in x.iter() {
        m += v;
    }
    m /= d as f32;
    let mut v = 0.0f32;
    for e in x.iter() {
        v += (e - m) * (e - m);
    }
    let r = 1.0 / (v / d as f32 + 1e-5).sqrt();
    for i in 0..d {
        x[i] = (x[i] - m) * r * w[i] + b[i];
    }
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    let mut s = [0.0f32; 8];
    let n = a.len();
    let mut i = 0;
    while i + 8 <= n {
        for j in 0..8 {
            s[j] += a[i + j] * b[i + j];
        }
        i += 8;
    }
    let mut r = ((s[0] + s[1]) + (s[2] + s[3])) + ((s[4] + s[5]) + (s[6] + s[7]));
    while i < n {
        r += a[i] * b[i];
        i += 1;
    }
    r
}

/// y[o] = b[o] + Σ W[o][i] x[i]
fn linear(w: &[f32], b: &[f32], x: &[f32], y: &mut [f32], nin: usize, nout: usize) {
    for o in 0..nout {
        y[o] = b[o] + dot(&w[o * nin..o * nin + nin], &x[..nin]);
    }
}

#[derive(Clone, Copy, Default)]
struct BlkOff {
    dw: usize,
    dwb: usize,
    lnw: usize,
    lnb: usize,
    p1: usize,
    p1b: usize,
    p2: usize,
    p2b: usize,
    s1: usize,
    s1b: usize,
    s2: usize,
    s2b: usize,
    dil: i32,
}

/// グライドのキーの確率を出す小さいネットワーク (`glide_nn.h` の `Net`)。
#[derive(Clone)]
pub struct Net {
    d_: i32,
    w_: Vec<f32>,
    stem_: usize,
    stemb_: usize,
    b_: [BlkOff; 5],
    down_: usize,
    downb_: usize,
    lnw_: usize,
    lnb_: usize,
    head_: usize,
    headb_: usize,
}

impl Default for Net {
    fn default() -> Self {
        Net {
            d_: 0,
            w_: Vec::new(),
            stem_: 0,
            stemb_: 0,
            b_: [BlkOff::default(); 5],
            down_: 0,
            downb_: 0,
            lnw_: 0,
            lnb_: 0,
            head_: 0,
            headb_: 0,
        }
    }
}

impl Net {
    pub const T: usize = 64;
    pub const TF: usize = 32;

    pub fn new() -> Self {
        Self::default()
    }

    pub fn ok(&self) -> bool {
        self.d_ > 0
    }

    /// glide_nn.bin の中身 ("GNN1" + d + float32 の重み)。読めなければ false。
    pub fn load(&mut self, data: &[u8]) -> bool {
        self.d_ = 0;
        if data.len() < 8 || &data[0..4] != b"GNN1" {
            return false;
        }
        let d = i32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        if d <= 0 {
            return false;
        }
        let dd = d as usize;
        let (h, s) = (4 * dd, dd / 8);
        let per_blk = dd * 7 + dd + dd + dd + h * dd + h + dd * h + dd + s * dd + s + dd * s + dd;
        let total = dd * 8 * 3 + dd + 5 * per_blk + dd * dd * 2 + dd + dd + dd + 65 * dd + 65;
        if data.len() != 8 + total * 4 {
            return false;
        }
        let mut w = vec![0.0f32; total];
        for i in 0..total {
            w[i] = f32::from_le_bytes([
                data[8 + 4 * i],
                data[8 + 4 * i + 1],
                data[8 + 4 * i + 2],
                data[8 + 4 * i + 3],
            ]);
        }
        self.w_ = w;
        let mut p = 0usize;
        let mut take = |k: usize| {
            let q = p;
            p += k;
            q
        };
        self.stem_ = take(dd * 8 * 3);
        self.stemb_ = take(dd);
        let dil = [1, 2, 3, 5, 8];
        for i in 0..5 {
            let b = &mut self.b_[i];
            b.dw = take(dd * 7);
            b.dwb = take(dd);
            b.lnw = take(dd);
            b.lnb = take(dd);
            b.p1 = take(h * dd);
            b.p1b = take(h);
            b.p2 = take(dd * h);
            b.p2b = take(dd);
            b.s1 = take(s * dd);
            b.s1b = take(s);
            b.s2 = take(dd * s);
            b.s2b = take(dd);
            b.dil = dil[i];
        }
        self.down_ = take(dd * dd * 2);
        self.downb_ = take(dd);
        self.lnw_ = take(dd);
        self.lnb_ = take(dd);
        self.head_ = take(65 * dd);
        self.headb_ = take(65);
        self.d_ = d;
        true
    }

    /// pts: 軌跡 (x, y の並び、0〜1)。64 点に取り直してから流す。keys: キーの中心 (x, y の並び、0〜1)。
    /// 返り値: [32 × (1 + K)] の log p (0 番が blank)。
    pub fn run(&self, pts: &[f32], keys: &[f32]) -> Vec<f32> {
        let (t_, tf) = (Self::T, Self::TF);
        let d = self.d_ as usize;
        let (h, s_) = (4 * d, d / 8);
        let k = keys.len() / 2;
        let n = pts.len() / 2;
        // 64 点に取り直す (点の番号で線形補間)
        let mut p = [[0.0f32; 2]; Self::T];
        for t in 0..t_ {
            let u = if n > 1 {
                t as f32 * (n - 1) as f32 / (t_ - 1) as f32
            } else {
                0.0
            };
            let i = (u as i32).min((n as i32 - 2).max(0)) as usize;
            let f = if n > 1 { u - i as f32 } else { 0.0 };
            for c in 0..2 {
                p[t][c] = if n > 1 {
                    pts[2 * i + c] * (1.0 - f) + pts[2 * (i + 1) + c] * f
                } else {
                    pts[c]
                };
            }
        }
        // 8 次元の特徴
        let mut x = vec![0.0f32; d * t_]; // [D][T]
        let mut fe = [[0.0f32; Self::T]; 8];
        let mut ang = [0.0f32; Self::T];
        for t in 0..t_ {
            let mut v = [0.0f32; 2];
            let mut a = [0.0f32; 2];
            for c in 0..2 {
                v[c] = if t == 0 {
                    p[1][c] - p[0][c]
                } else if t == t_ - 1 {
                    p[t_ - 1][c] - p[t_ - 2][c]
                } else {
                    (p[t + 1][c] - p[t - 1][c]) / 2.0
                };
                a[c] = if t == 0 || t == t_ - 1 {
                    0.0
                } else {
                    p[t + 1][c] - 2.0 * p[t][c] + p[t - 1][c]
                };
                v[c] *= 16.0;
                a[c] *= 64.0;
            }
            let sp = (v[0] * v[0] + v[1] * v[1]).sqrt();
            ang[t] = v[1].atan2(v[0]);
            let mut da = 0.0f32;
            if t > 0 {
                let tw = 2.0 * std::f32::consts::PI;
                da = ang[t] - ang[t - 1] + std::f32::consts::PI;
                da = da - tw * (da / tw).floor() - std::f32::consts::PI;
            }
            let cu = if sp > 0.05 {
                (da / 0.5).clamp(-2.0, 2.0)
            } else {
                0.0
            };
            fe[0][t] = p[t][0] * 2.0 - 1.0;
            fe[1][t] = p[t][1] * 2.0 - 1.0;
            fe[2][t] = v[0];
            fe[3][t] = v[1];
            fe[4][t] = a[0];
            fe[5][t] = a[1];
            fe[6][t] = sp;
            fe[7][t] = cu;
        }
        let w = &self.w_;
        // stem: Conv1d(8, D, 3, padding 1)
        for o in 0..d {
            for t in 0..t_ {
                let mut sm = w[self.stemb_ + o];
                for i in 0..8 {
                    for kk in 0..3 {
                        let tt = t as i32 + kk - 1;
                        if tt >= 0 && (tt as usize) < t_ {
                            sm += w[self.stem_ + (o * 8 + i) * 3 + kk as usize] * fe[i][tt as usize];
                        }
                    }
                }
                x[o * t_ + t] = sm;
            }
        }
        let mut hh = vec![0.0f32; t_ * d];
        let mut hid = vec![0.0f32; h];
        let mut col = vec![0.0f32; d];
        let mut se = vec![0.0f32; s_];
        let mut sc = vec![0.0f32; d];
        for bi in 0..5 {
            let b = self.b_[bi];
            for c in 0..d {
                for t in 0..t_ {
                    let mut sm = w[b.dwb + c];
                    for kk in 0..7 {
                        let tt = t as i32 + (kk - 3) * b.dil;
                        if tt >= 0 && (tt as usize) < t_ {
                            sm += w[b.dw + c * 7 + kk as usize] * x[c * t_ + tt as usize];
                        }
                    }
                    hh[t * d + c] = sm;
                }
            }
            sc.iter_mut().for_each(|v| *v = 0.0);
            for t in 0..t_ {
                let r = &mut hh[t * d..t * d + d];
                layernorm(r, &w[b.lnw..b.lnw + d], &w[b.lnb..b.lnb + d]);
                let rcopy: Vec<f32> = r.to_vec();
                linear(&w[b.p1..], &w[b.p1b..], &rcopy, &mut hid, d, h);
                for v in hid.iter_mut() {
                    *v = gelu(*v);
                }
                let hidc = hid.clone();
                linear(&w[b.p2..], &w[b.p2b..], &hidc, r, h, d);
                for c in 0..d {
                    sc[c] += r[c];
                }
            }
            for c in 0..d {
                sc[c] /= t_ as f32;
            }
            let scc = sc.clone();
            linear(&w[b.s1..], &w[b.s1b..], &scc, &mut se, d, s_);
            for v in se.iter_mut() {
                *v = v.max(0.0);
            }
            let sec = se.clone();
            linear(&w[b.s2..], &w[b.s2b..], &sec, &mut sc, s_, d);
            for c in 0..d {
                sc[c] = sigm(sc[c]);
            }
            for t in 0..t_ {
                for c in 0..d {
                    x[c * t_ + t] += hh[t * d + c] * sc[c];
                }
            }
        }
        // down: Conv1d(D, D, 2, stride 2) → ln → head
        const NB: usize = 8;
        let mut phi = vec![0.0f32; k * NB * NB];
        for kk in 0..k {
            for u in 0..NB {
                for v in 0..NB {
                    phi[kk * 64 + u * NB + v] = (std::f32::consts::PI * keys[2 * kk] * u as f32).cos()
                        * (std::f32::consts::PI * keys[2 * kk + 1] * v as f32).cos();
                }
            }
        }
        let mut out = vec![0.0f32; tf * (1 + k)];
        let mut hd = [0.0f32; 65];
        let mut z = vec![0.0f32; k];
        let mut pair = vec![0.0f32; 2 * d];
        for t in 0..tf {
            for i in 0..d {
                pair[2 * i] = x[i * t_ + 2 * t];
                pair[2 * i + 1] = x[i * t_ + 2 * t + 1];
            }
            for o in 0..d {
                col[o] = w[self.downb_ + o] + dot(&w[self.down_ + o * d * 2..self.down_ + o * d * 2 + 2 * d], &pair);
            }
            layernorm(&mut col, &w[self.lnw_..self.lnw_ + d], &w[self.lnb_..self.lnb_ + d]);
            let colc = col.clone();
            linear(&w[self.head_..], &w[self.headb_..], &colc, &mut hd, d, 65);
            let mut mx = -1e30f32;
            for kk in 0..k {
                let mut sm = 0.0f32;
                for j in 0..64 {
                    sm += hd[1 + j] * phi[kk * 64 + j];
                }
                z[kk] = sm;
                mx = mx.max(sm);
            }
            let mut se2 = 0.0f32;
            for kk in 0..k {
                se2 += (z[kk] - mx).exp();
            }
            let lse = mx + se2.ln();
            let ll = logsig(hd[0]);
            let r = &mut out[t * (1 + k)..t * (1 + k) + 1 + k];
            r[0] = logsig(-hd[0]);
            for kk in 0..k {
                r[1 + kk] = ll + z[kk] - lse;
            }
        }
        out
    }
}

// ---------------------------------------------------------------- 語の一覧 (TSV)

/// `engine.cpp` の `glide_utf8` と同じ寛容な UTF-8 デコード (不正な並びでも止まらない)。
fn glide_utf8(p: &[u8], o: &mut Vec<u16>) {
    let mut i = 0;
    while i < p.len() {
        let mut c = p[i] as u32;
        i += 1;
        let mut more: i32 = if c >= 0xF0 {
            3
        } else if c >= 0xE0 {
            2
        } else if c >= 0xC0 {
            1
        } else {
            0
        };
        if more > 0 {
            c &= 0x3F >> more;
        }
        while more > 0 && i < p.len() {
            c = (c << 6) | (p[i] as u32 & 0x3F);
            i += 1;
            more -= 1;
        }
        if c >= 0x10000 {
            c -= 0x10000;
            o.push((0xD800 + (c >> 10)) as u16);
            o.push((0xDC00 + (c & 0x3FF)) as u16);
        } else {
            o.push(c as u16);
        }
    }
}

fn atoi(s: &[u8]) -> i32 {
    let mut i = 0;
    while i < s.len() && (s[i] as char).is_whitespace() {
        i += 1;
    }
    let mut neg = false;
    if i < s.len() && (s[i] == b'-' || s[i] == b'+') {
        neg = s[i] == b'-';
        i += 1;
    }
    let mut v: i64 = 0;
    while i < s.len() && s[i].is_ascii_digit() {
        v = v * 10 + (s[i] - b'0') as i64;
        i += 1;
    }
    (if neg { -v } else { v }) as i32
}

/// なぞりの語の一覧 (rawfile の glide_en.tsv・glide_zh.tsv: キー TAB 表記|表記 TAB コスト、UTF-8)。
/// 表記は 1 本の文字列 (blob) に詰め、キーの順に並べた番号を二分探索で引く (`engine.cpp` の `GlideWords`)。
#[derive(Default)]
pub struct GlideWords {
    pub words: Vec<(Vec<u16>, i32)>,
    blob: Vec<u16>,
    so: Vec<u32>,
    sl: Vec<u32>,
    order: Vec<u32>,
}

impl GlideWords {
    pub fn parse(tsv: &[u8]) -> GlideWords {
        let mut w = GlideWords::default();
        let mut p = 0usize;
        let end = tsv.len();
        while p < end {
            let nl = match tsv[p..end].iter().position(|&c| c == b'\n') {
                Some(rel) => p + rel,
                None => end,
            };
            let line = &tsv[p..nl];
            if let Some(t1) = line.iter().position(|&c| c == b'\t') {
                if let Some(t2r) = line[t1 + 1..].iter().position(|&c| c == b'\t') {
                    let t2 = t1 + 1 + t2r;
                    // キーは C++ では char を char16_t へ広げたもの (ASCII 前提)
                    let k: Vec<u16> = line[..t1].iter().map(|&b| b as u16).collect();
                    w.words.push((k, atoi(&line[t2 + 1..])));
                    w.so.push(w.blob.len() as u32);
                    glide_utf8(&line[t1 + 1..t2], &mut w.blob);
                    w.sl.push(w.blob.len() as u32 - w.so[w.so.len() - 1]);
                }
            }
            p = nl + 1;
        }
        w.order = (0..w.words.len() as u32).collect();
        let words_ref = &w.words;
        w.order.sort_by(|&a, &b| words_ref[a as usize].0.cmp(&words_ref[b as usize].0));
        w
    }

    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    fn surfaces(&self, row: u32) -> Vec<Vec<u16>> {
        let mut r = Vec::new();
        let p = self.so[row as usize] as usize;
        let e = p + self.sl[row as usize] as usize;
        let mut i = p;
        while i < e {
            let mut j = i;
            while j < e && self.blob[j] != '|' as u16 {
                j += 1;
            }
            r.push(self.blob[i..j].to_vec());
            i = j + 1;
        }
        if e == p {
            // 空の表記欄は C++ では 1 つの空の表記になるが、呼び出し側で空は捨てられる
        }
        r
    }

    /// キーに完全一致する行の表記一覧。
    pub fn find(&self, k: &[u16]) -> Vec<Vec<u16>> {
        let it = self.order.partition_point(|&i| self.words[i as usize].0.as_slice() < k);
        if it >= self.order.len() {
            return Vec::new();
        }
        let row = self.order[it];
        if self.words[row as usize].0.as_slice() != k {
            return Vec::new();
        }
        self.surfaces(row)
    }

    fn lower_bound(&self, x: &[u16]) -> usize {
        self.order.partition_point(|&i| self.words[i as usize].0.as_slice() < x)
    }
}

/// 中国語をキーで 1 字ずつ打つときの候補 (`engine.cpp` の `kkc_zh_candidates` の中身)。
/// 返り値: (表記, 消費する打った字の数) の並び。
/// 並び: 拼音全体と同じキーの語 → 拼音で始まるキーの語 (続きの予測) → 前の方だけと同じキーの語 (長い順)。
pub fn zh_candidates(w: &GlideWords, typed: &[u8], maxout: i32, cap: i32) -> Vec<(Vec<u16>, i32)> {
    let mut out: Vec<(Vec<u16>, i32)> = Vec::new();
    if typed.is_empty() || w.words.is_empty() || maxout <= 0 {
        return out;
    }
    let t: Vec<u16> = typed.iter().map(|&b| b as u16).collect();
    let n = typed.len() as i32;
    let mut at = 0i32; // out に書いた UTF-16 の字数 (区切り込み)
    let mut seen: Vec<Vec<u16>> = Vec::new();
    fn put(
        out: &mut Vec<(Vec<u16>, i32)>,
        seen: &mut Vec<Vec<u16>>,
        at: &mut i32,
        maxout: i32,
        cap: i32,
        s: &[u16],
        u: i32,
    ) {
        if out.len() as i32 >= maxout || s.is_empty() || *at + s.len() as i32 + 1 > cap {
            return;
        }
        if seen.iter().any(|x| x.as_slice() == s) {
            return;
        }
        seen.push(s.to_vec());
        *at += s.len() as i32 + 1;
        out.push((s.to_vec(), u));
    }
    // 1. 拼音全体
    for s in w.find(&t) {
        put(&mut out, &mut seen, &mut at, maxout, cap, &s, n);
    }
    // 2. 続きの予測: キーが t で始まる行のうちコストの低いもの
    if n >= 2 {
        let mut cs: Vec<(i32, u32)> = Vec::new();
        let mut scanned = 0;
        let mut it = w.lower_bound(&t);
        while it < w.order.len() && scanned < 5000 {
            let row = w.order[it];
            let key = &w.words[row as usize].0;
            if key.len() < t.len() || key[..t.len()] != t[..] {
                break;
            }
            if key.len() != t.len() {
                cs.push((w.words[row as usize].1, row));
            }
            it += 1;
            scanned += 1;
        }
        cs.sort();
        for i in 0..cs.len().min(6) {
            let sv = w.surfaces(cs[i].1);
            if !sv.is_empty() {
                put(&mut out, &mut seen, &mut at, maxout, cap, &sv[0], n);
            }
        }
    }
    // 3. 前の方だけ (長い順)
    let mut taken = 0;
    let mut l = n - 1;
    while l >= 1 && (out.len() as i32) < maxout {
        let sv = w.find(&t[..l as usize]);
        if !sv.is_empty() {
            let lim = if taken == 0 { 10 } else { 4 };
            for i in 0..sv.len().min(lim) {
                let s = sv[i].clone();
                put(&mut out, &mut seen, &mut at, maxout, cap, &s, l);
            }
            taken += 1;
            if taken >= 3 {
                break;
            }
        }
        l -= 1;
    }
    out
}

// ---------------------------------------------------------------- FFI (engine.h の glide 系)

pub mod ffi {
    use super::*;
    use std::os::raw::{c_char, c_int, c_void};
    use std::sync::{Mutex, OnceLock};

    /// 縦の縮め: 行の間隔はキーの横の間隔より広いので、縦のずれを軽く見る (`engine.cpp` の GLIDE_YS)。
    const GLIDE_YS: f32 = 0.75;

    struct State {
        words: [GlideWords; 3],
        dec: [Decoder; 3],
        built: [bool; 3],
        keysig: [String; 3],
        owner: usize,
        net: Net,
    }

    fn state() -> &'static Mutex<State> {
        static ST: OnceLock<Mutex<State>> = OnceLock::new();
        ST.get_or_init(|| {
            Mutex::new(State {
                words: std::array::from_fn(|_| GlideWords::default()),
                dec: std::array::from_fn(|_| Decoder::default()),
                built: [false; 3],
                keysig: std::array::from_fn(|_| String::new()),
                owner: 0,
                net: Net::default(),
            })
        })
    }

    /// エンジン連携: エンジン本体 (`engine.rs`) の辞書から、グライド用の
    /// 語一覧 (読みと最小コスト) を取り出す。条件は C++ `glide_build` と同じ
    /// (engine.rs の `Lex::glide_words` に実装)。
    /// - mode 0 (かな): ひらがな・ー のみ、長さ 1〜12、最小コスト 7000 以下。
    /// - mode 1 (英字): TSV の語が無いときの代替 (辞書の英字の語、a-z のみ、長さ 2〜20)。
    /// - mode 2: C++ では常に TSV が必要で、エンジンからは作らない (`None`)。
    ///
    /// `e` は `kkc_open` が返した `kkc_engine` へのポインタ (C 側では
    /// `void*` として渡ってくる)。それ以外の FFI 経路 (TSV の語からの
    /// 木構築・拼音候補・ネット) はこの関数に依存しない。
    fn engine_lexicon_words(e: *mut c_void, mode: i32) -> Option<Vec<(Vec<u16>, i32)>> {
        if e.is_null() {
            return None;
        }
        // SAFETY: 呼び出し側 (kkc_glide_prepare / kkc_glide_decode) は
        // engine.h の契約どおり kkc_open 由来のエンジンポインタを渡す。
        let eng = unsafe { &*(e as *const crate::engine::kkc_engine) };
        eng.glide_lexicon_words(mode)
    }

    /// `engine.cpp` の `glide_build` に対応。木が要るモードだけ作る。
    fn glide_build(st: &mut State, e: *mut c_void, mode: i32) {
        if st.owner != e as usize {
            st.dec = std::array::from_fn(|_| Decoder::default());
            st.built = [false; 3];
            st.keysig = std::array::from_fn(|_| String::new());
            st.owner = e as usize;
        }
        let m = mode as usize;
        if st.built[m] {
            return;
        }
        if mode != 0 && !st.words[m].is_empty() {
            let words = st.words[m].words.clone();
            st.dec[m].build(&words, Mode::Letters);
            st.built[m] = true;
            return;
        }
        if mode == 2 {
            return; // 拼音の一覧が無い
        }
        if let Some(words) = engine_lexicon_words(e, mode) {
            st.dec[m].build(&words, if mode == 0 { Mode::Kana } else { Mode::Letters });
            st.built[m] = true;
        }
    }

    /// 辞書の読みの木を作る。返り値は木の節の数。
    #[no_mangle]
    pub extern "C" fn kkc_glide_prepare(e: *mut c_void, mode: c_int) -> c_int {
        if e.is_null() || !(0..=2).contains(&mode) {
            return 0;
        }
        let mut st = state().lock().unwrap();
        glide_build(&mut st, e, mode);
        st.dec[mode as usize].nodes() as c_int
    }

    /// 英字・拼音のなぞりの語の一覧を渡す (glide_en.tsv / glide_zh.tsv の中身)。返り値は語の数。
    #[no_mangle]
    pub extern "C" fn kkc_glide_set_words(mode: c_int, tsv: *const c_char, n: c_int) -> c_int {
        if !(1..=2).contains(&mode) || tsv.is_null() || n <= 0 {
            return 0;
        }
        let bytes = unsafe { std::slice::from_raw_parts(tsv as *const u8, n as usize) };
        let w = GlideWords::parse(bytes);
        let cnt = w.words.len() as c_int;
        let mut st = state().lock().unwrap();
        st.words[mode as usize] = w;
        st.built[mode as usize] = false;
        cnt
    }

    /// 中国語をキーで打つ: 拼音 typed の候補 (0 区切り) と、候補ごとに消費する打った字の数 use。
    #[no_mangle]
    pub extern "C" fn kkc_zh_candidates(
        typed: *const c_char,
        n: c_int,
        maxout: c_int,
        out: *mut u16,
        cap: c_int,
        use_: *mut i32,
    ) -> c_int {
        if typed.is_null() || n <= 0 || maxout <= 0 || out.is_null() || use_.is_null() {
            return 0;
        }
        let bytes = unsafe { std::slice::from_raw_parts(typed as *const u8, n as usize) };
        let st = state().lock().unwrap();
        let res = zh_candidates(&st.words[2], bytes, maxout, cap);
        let mut at = 0usize;
        for (i, (s, u)) in res.iter().enumerate() {
            unsafe {
                for &c in s {
                    *out.add(at) = c;
                    at += 1;
                }
                *out.add(at) = 0;
                at += 1;
                *use_.add(i) = *u;
            }
        }
        res.len() as c_int
    }

    /// 英字のなぞりのネットワーク (glide_nn.bin の中身)。読めれば 1。
    #[no_mangle]
    pub extern "C" fn kkc_glide_set_net(data: *const c_void, n: c_int) -> c_int {
        if data.is_null() || n <= 0 {
            return 0;
        }
        let bytes = unsafe { std::slice::from_raw_parts(data as *const u8, n as usize) };
        let mut net = Net::default();
        if net.load(bytes) {
            state().lock().unwrap().net = net;
            1
        } else {
            0
        }
    }

    /// なぞりのデコード。読み (mode>=1 で語一覧があれば表記) の候補を良い順に out (0 区切り) へ。
    #[no_mangle]
    #[allow(clippy::too_many_arguments)]
    pub extern "C" fn kkc_glide_decode(
        e: *mut c_void,
        mode: c_int,
        keys: *const c_char,
        kxy: *const f32,
        nkeys: c_int,
        pts: *const f32,
        npts: c_int,
        nbest: c_int,
        out: *mut u16,
        cap: c_int,
        costs: *mut f32,
    ) -> c_int {
        if e.is_null()
            || !(0..=2).contains(&mode)
            || nkeys <= 0
            || npts < 2
            || keys.is_null()
            || kxy.is_null()
            || pts.is_null()
            || out.is_null()
            || costs.is_null()
        {
            return 0;
        }
        let m = mode as usize;
        let key_bytes = unsafe { std::slice::from_raw_parts(keys as *const u8, nkeys as usize) };
        let kxy_s = unsafe { std::slice::from_raw_parts(kxy, 2 * nkeys as usize) };
        let pts_s = unsafe { std::slice::from_raw_parts(pts, 2 * npts as usize) };
        let mut st = state().lock().unwrap();
        glide_build(&mut st, e, mode);
        if !st.built[m] {
            return 0;
        }
        // キー配列の署名 (C++ と同じ作り: 文字 + 座標を 100 倍して整数化した並び)
        let mut sig = String::new();
        for &b in key_bytes {
            sig.push(b as char);
        }
        for i in 0..2 * nkeys as usize {
            sig += &format!("{},", (kxy_s[i] * 100.0) as i32);
        }
        if sig != st.keysig[m] {
            let ks: Vec<Key> = (0..nkeys as usize)
                .map(|i| Key { c: key_bytes[i], x: kxy_s[2 * i], y: kxy_s[2 * i + 1] * GLIDE_YS })
                .collect();
            st.dec[m].set_keys(&ks);
            st.keysig[m] = sig;
        }
        let mut p = Params::default();
        p.abs_bound = 200.0;
        if mode >= 1 {
            p.max_words = 1;
            p.sigma_key = 0.7;
            p.sigma_seg = 0.7;
            p.lex_w = 0.003;
        }
        p.nbest = nbest;
        let mut res: Vec<Result> = Vec::new();
        if mode >= 1 && st.net.ok() {
            // ネットワーク: キーボードの字の範囲を 0〜1 にする
            let (mut x0, mut x1, mut y0, mut y1) = (1e9f32, -1e9f32, 1e9f32, -1e9f32);
            for i in 0..nkeys as usize {
                x0 = x0.min(kxy_s[2 * i]);
                x1 = x1.max(kxy_s[2 * i]);
                y0 = y0.min(kxy_s[2 * i + 1]);
                y1 = y1.max(kxy_s[2 * i + 1]);
            }
            let rh = ((y1 - y0) / 2.0).max(0.5);
            x0 -= 0.5;
            x1 += 0.5;
            y0 -= rh / 2.0;
            y1 += rh / 2.0;
            let mut kn = vec![0.0f32; 2 * nkeys as usize];
            let mut pn = vec![0.0f32; 2 * npts as usize];
            for i in 0..nkeys as usize {
                kn[2 * i] = (kxy_s[2 * i] - x0) / (x1 - x0);
                kn[2 * i + 1] = (kxy_s[2 * i + 1] - y0) / (y1 - y0);
            }
            for i in 0..npts as usize {
                pn[2 * i] = (pts_s[2 * i] - x0) / (x1 - x0);
                pn[2 * i + 1] = (pts_s[2 * i + 1] - y0) / (y1 - y0);
            }
            let lp = st.net.run(&pn, &kn);
            res = st.dec[m].decode_ctc(&lp, Net::TF as i32, nkeys, 1.0, 0.0, 100, nbest);
        }
        if res.is_empty() {
            let mut pp = pts_s.to_vec();
            for i in (1..pp.len()).step_by(2) {
                pp[i] *= GLIDE_YS;
            }
            res = st.dec[m].decode(&pp, &p);
        }
        // 出す数: 英字・かなは nbest まで、中国語は 3 倍まで
        let maxout = if mode == 2 { 3 * nbest } else { nbest };
        let mut n = 0i32;
        let mut at = 0i32;
        let mut seen: Vec<Vec<u16>> = Vec::new();
        for (ri, r) in res.iter().enumerate() {
            let sv: Vec<Vec<u16>> = if mode >= 1 && !st.words[m].is_empty() {
                st.words[m].find(&r.reading)
            } else {
                Vec::new()
            };
            if sv.is_empty() {
                // put(r.reading, r.cost)
                if n < maxout
                    && at + r.reading.len() as i32 + 1 <= cap
                    && !seen.contains(&r.reading)
                {
                    seen.push(r.reading.clone());
                    unsafe {
                        for &c in &r.reading {
                            *out.add(at as usize) = c;
                            at += 1;
                        }
                        *out.add(at as usize) = 0;
                        at += 1;
                        *costs.add(n as usize) = r.cost;
                    }
                    n += 1;
                }
                continue;
            }
            let lim = if mode == 2 && ri > 0 {
                sv.len().min(3)
            } else {
                sv.len()
            };
            for (k, s) in sv.iter().take(lim).enumerate() {
                if n >= maxout || at + s.len() as i32 + 1 > cap || seen.contains(s) {
                    continue;
                }
                seen.push(s.clone());
                unsafe {
                    for &c in s {
                        *out.add(at as usize) = c;
                        at += 1;
                    }
                    *out.add(at as usize) = 0;
                    at += 1;
                    *costs.add(n as usize) = r.cost + 0.5 * k as f32;
                }
                n += 1;
            }
        }
        n
    }
}

// ---------------------------------------------------------------- テスト

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap as Map;

    /// QWERTY 配列 (キーの幅 1、行の間隔 1)。
    fn qwerty() -> (Vec<Key>, Map<u8, (f32, f32)>) {
        let rows: [(&str, f32, f32); 3] = [
            ("qwertyuiop", 0.0, 0.0),
            ("asdfghjkl", 0.5, 1.0),
            ("zxcvbnm", 1.0, 2.0),
        ];
        let mut ks = Vec::new();
        let mut mp = Map::new();
        for (r, off, y) in rows {
            for (i, c) in r.bytes().enumerate() {
                let x = off + i as f32;
                ks.push(Key { c, x, y });
                mp.insert(c, (x, y));
            }
        }
        (ks, mp)
    }

    /// キーの中心を順に結ぶ軌跡 (区間ごとに等分点を入れる)。
    fn trace(mp: &Map<u8, (f32, f32)>, s: &str) -> Vec<f32> {
        let cs: Vec<(f32, f32)> = s.bytes().map(|c| mp[&c]).collect();
        let mut v = Vec::new();
        for w in cs.windows(2) {
            for i in 0..6 {
                let u = i as f32 / 6.0;
                v.push(w[0].0 + u * (w[1].0 - w[0].0));
                v.push(w[0].1 + u * (w[1].1 - w[0].1));
            }
        }
        if let Some(&last) = cs.last() {
            v.push(last.0);
            v.push(last.1);
        }
        v
    }

    fn w(s: &str) -> Vec<u16> {
        crate::u16s(s)
    }

    #[test]
    fn spell_and_romanize() {
        let sp = spell();
        let shi = &sp[&w("し")];
        assert!(shi.iter().any(|r| r == "si"), "し row: {:?}", shi);
        assert!(shi.iter().any(|r| r == "shi"), "し shi: {:?}", shi);
        // 既定は綴り表の先頭 (し = si)。rng で変化形を選ぶと shi になる (C++ の romanize と同じ)
        assert_eq!(romanize(&w("しんぶん"), None), "sinbun");
        let mut zero = || 0u32;
        assert_eq!(romanize(&w("しんぶん"), Some(&mut zero)), "shinbun");
        assert_eq!(romanize(&w("きょう"), None), "kyou");
        // っ は空の綴り (なぞりでは見えない)
        assert_eq!(romanize(&w("がっこう"), None), "gakou");
        assert_eq!(romanize(&w("漢"), None), "");
    }

    #[test]
    fn kana_glide_basic() {
        let (ks, mp) = qwerty();
        let mut d = Decoder::new();
        d.build(
            &[
                (w("あした"), 100),
                (w("あさ"), 500),
                (w("あめ"), 400),
                (w("かぜ"), 300),
                (w("すし"), 600),
                (w("あい"), 700),
            ],
            Mode::Kana,
        );
        d.set_keys(&ks);
        let pts = trace(&mp, "ashita");
        let res = d.decode(&pts, &Params::default());
        assert!(!res.is_empty(), "候補が空");
        assert_eq!(res[0].reading, w("あした"), "1位: {:?}", res.iter().map(|r| crate::from_u16s(&r.reading)).collect::<Vec<_>>());
    }

    #[test]
    fn kana_glide_sokuon_same_key() {
        // がっこう と がこう はキーの並びが同じ (っ は見えない)。辞書コストで がっこう が勝つはず
        let (ks, mp) = qwerty();
        let mut d = Decoder::new();
        d.build(&[(w("がっこう"), 100), (w("がこう"), 900), (w("がく"), 800)], Mode::Kana);
        d.set_keys(&ks);
        let pts = trace(&mp, "gakou");
        let res = d.decode(&pts, &Params::default());
        assert!(!res.is_empty());
        assert_eq!(res[0].reading, w("がっこう"));
    }

    #[test]
    fn letters_glide_basic() {
        let (ks, mp) = qwerty();
        let mut d = Decoder::new();
        d.build(
            &[
                (w("hello"), 100),
                (w("world"), 200),
                (w("help"), 300),
                (w("held"), 400),
            ],
            Mode::Letters,
        );
        d.set_keys(&ks);
        // ll は同じキーが続くので軌跡は helo
        let pts = trace(&mp, "helo");
        let res = d.decode(&pts, &Params::default());
        assert!(!res.is_empty());
        assert_eq!(res[0].reading, w("hello"));
    }

    #[test]
    fn ctc_decode_hello() {
        // a-z のキーで、合成した log p から hello を復元する (ll の同キー処理込み)
        let mut ks = Vec::new();
        for i in 0..26u8 {
            ks.push(Key { c: b'a' + i, x: (i % 10) as f32, y: (i / 10) as f32 });
        }
        let mut d = Decoder::new();
        d.build(&[(w("hello"), 100), (w("world"), 200), (w("help"), 300)], Mode::Letters);
        d.set_keys(&ks);
        let frames_spec: [Option<u8>; 11] = [
            None,
            Some(b'h'),
            None,
            Some(b'e'),
            None,
            Some(b'l'),
            None,
            Some(b'l'),
            None,
            Some(b'o'),
            None,
        ];
        let width = 27usize;
        let mut lp = vec![-30.0f32; frames_spec.len() * width];
        for (t, f) in frames_spec.iter().enumerate() {
            match f {
                None => lp[t * width] = -0.01,
                Some(c) => lp[t * width + 1 + (c - b'a') as usize] = -0.01,
            }
        }
        let res = d.decode_ctc(&lp, frames_spec.len() as i32, 26, 1.0, 0.0, 100, 10);
        assert!(!res.is_empty(), "CTC 候補が空");
        assert_eq!(res[0].reading, w("hello"));
    }

    #[test]
    fn net_load_zero_weights() {
        // d=8 の形だけ作った零重み: head が 0 なので blank/キーは一様になるはず
        let dd = 8usize;
        let (h, s) = (4 * dd, dd / 8);
        let per_blk = dd * 7 + dd + dd + dd + h * dd + h + dd * h + dd + s * dd + s + dd * s + dd;
        let total = dd * 8 * 3 + dd + 5 * per_blk + dd * dd * 2 + dd + dd + dd + 65 * dd + 65;
        let mut data = vec![0u8; 8 + total * 4];
        data[0..4].copy_from_slice(b"GNN1");
        data[4..8].copy_from_slice(&(dd as i32).to_le_bytes());
        let mut net = Net::new();
        assert!(net.load(&data));
        assert!(net.ok());
        // 壊れた入力は弾く
        let mut bad = data.clone();
        bad[0] = b'X';
        assert!(!net.load(&bad));
        assert!(!net.load(&data[..data.len() - 4]));
        assert!(net.load(&data));
        let keys = [0.2f32, 0.3, 0.7, 0.3];
        let mut pts = Vec::new();
        for i in 0..10 {
            pts.push(i as f32 / 9.0);
            pts.push(0.3);
        }
        let out = net.run(&pts, &keys);
        assert_eq!(out.len(), Net::TF * 3);
        for t in 0..Net::TF {
            assert!((out[t * 3] - (-0.6931472f32)).abs() < 1e-4, "blank logp {}", out[t * 3]);
            assert!((out[t * 3 + 1] - (-1.3862944f32)).abs() < 1e-4, "key logp {}", out[t * 3 + 1]);
        }
    }

    #[test]
    fn net_load_real_bin() {
        // 実データ (entry/src/main/resources/rawfile/glide_nn.bin, GNN1 / d=128) があれば検証する
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../entry/src/main/resources/rawfile/glide_nn.bin");
        let data = match std::fs::read(path) {
            Ok(d) => d,
            Err(_) => return, // 実データが無い環境では飛ばす
        };
        let mut net = Net::new();
        assert!(net.load(&data), "実 glide_nn.bin が読めない");
        let (ks, mp) = qwerty();
        // 0〜1 に正規化したキーと軌跡
        let (mut x0, mut x1, mut y0, mut y1) = (1e9f32, -1e9f32, 1e9f32, -1e9f32);
        for k in &ks {
            x0 = x0.min(k.x);
            x1 = x1.max(k.x);
            y0 = y0.min(k.y);
            y1 = y1.max(k.y);
        }
        x0 -= 0.5;
        x1 += 0.5;
        y0 -= 0.5;
        y1 += 0.5;
        let mut kn = Vec::new();
        for k in &ks {
            kn.push((k.x - x0) / (x1 - x0));
            kn.push((k.y - y0) / (y1 - y0));
        }
        let raw = trace(&mp, "helo");
        let mut pn = Vec::new();
        for i in 0..raw.len() / 2 {
            pn.push((raw[2 * i] - x0) / (x1 - x0));
            pn.push((raw[2 * i + 1] - y0) / (y1 - y0));
        }
        let out = net.run(&pn, &kn);
        assert_eq!(out.len(), Net::TF * (1 + ks.len()));
        for t in 0..Net::TF {
            let mut sum = 0.0f32;
            for v in &out[t * (1 + ks.len())..(t + 1) * (1 + ks.len())] {
                assert!(v.is_finite() && *v <= 0.01, "logp 異常: {}", v);
                sum += v.exp();
            }
            assert!((sum - 1.0).abs() < 1e-2, "frame {} の確率和 {}", t, sum);
        }
    }

    #[test]
    fn tsv_parse_and_find() {
        let tsv = "the\tthe\t1389\nits\tits|it's\t3041\n";
        let gw = GlideWords::parse(tsv.as_bytes());
        assert_eq!(gw.words.len(), 2);
        assert_eq!(gw.find(&w("its")), vec![w("its"), w("it's")]);
        assert_eq!(gw.find(&w("the")), vec![w("the")]);
        assert!(gw.find(&w("xyz")).is_empty());
    }

    #[test]
    fn zh_candidates_via_ffi() {
        use std::os::raw::c_char;
        let tsv = "ni\t你|呢\t100\nnihao\t你好|您好\t90\nnihaoma\t你好吗\t95\nhao\t好\t80\n";
        let n = ffi::kkc_glide_set_words(2, tsv.as_ptr() as *const c_char, tsv.len() as i32);
        assert_eq!(n, 4);
        // 全体一致 → 前方一致の予測 → 前方だけ
        let typed = "nihao";
        let mut out = vec![0u16; 256];
        let mut use_ = vec![0i32; 32];
        let k = ffi::kkc_zh_candidates(
            typed.as_ptr() as *const c_char,
            typed.len() as i32,
            10,
            out.as_mut_ptr(),
            256,
            use_.as_mut_ptr(),
        );
        assert!(k >= 2, "候補数 {}", k);
        let mut cands: Vec<(String, i32)> = Vec::new();
        let mut at = 0usize;
        for i in 0..k as usize {
            let mut s = Vec::new();
            while out[at] != 0 {
                s.push(out[at]);
                at += 1;
            }
            at += 1;
            cands.push((crate::from_u16s(&s), use_[i]));
        }
        assert_eq!(cands[0], ("你好".to_string(), 5), "候補: {:?}", cands);
        assert!(cands.iter().any(|(s, u)| s == "你" && *u == 2), "前方だけ候補: {:?}", cands);
        // 前方一致の予測 (nih → 你好、消費 3)
        let typed2 = "nih";
        let k2 = ffi::kkc_zh_candidates(
            typed2.as_ptr() as *const c_char,
            3,
            10,
            out.as_mut_ptr(),
            256,
            use_.as_mut_ptr(),
        );
        assert!(k2 >= 1);
        let mut s0 = Vec::new();
        let mut at = 0usize;
        while out[at] != 0 {
            s0.push(out[at]);
            at += 1;
        }
        assert_eq!(crate::from_u16s(&s0), "你好");
        assert_eq!(use_[0], 3);
    }

    #[test]
    fn erf_matches_std_values() {
        // glide_nn.cpp の std::erf と同じ値になること (旧 A&S 近似は最大 1.5e-7 ずれた)
        let cases: [(f32, f32); 5] = [
            (0.5, 0.5204999),
            (1.0, 0.8427008),
            (-1.5, -0.9661051),
            (2.0, 0.9953223),
            (3.0, 0.9999779),
        ];
        for (x, want) in cases {
            assert!((libm_erf(x) - want).abs() < 2e-7, "erf({x}) = {} != {want}", libm_erf(x));
        }
        assert_eq!(libm_erf(0.0), 0.0);
        // gelu(1) = 0.5 * (1 + erf(1/√2))
        assert!((gelu(1.0) - 0.8413447).abs() < 1e-6, "gelu(1) = {}", gelu(1.0));
    }

    #[test]
    fn ffi_english_glide_decode() {
        use std::os::raw::{c_char, c_void};
        let tsv = "hello\thello\t100\nworld\tworld\t200\nhelp\thelp\t300\nheld\theld\t400\n";
        let n = ffi::kkc_glide_set_words(1, tsv.as_ptr() as *const c_char, tsv.len() as i32);
        assert_eq!(n, 4);
        let engine = 1usize as *mut c_void; // TSV の語で完結する経路ではエンジン本体を参照しない
        let nodes = ffi::kkc_glide_prepare(engine, 1);
        assert!(nodes > 0, "節の数 {}", nodes);
        let (ks, mp) = qwerty();
        let key_str: Vec<u8> = ks.iter().map(|k| k.c).collect();
        let mut kxy = Vec::new();
        for k in &ks {
            kxy.push(k.x);
            kxy.push(k.y);
        }
        let pts = trace(&mp, "helo");
        let mut out = vec![0u16; 512];
        let mut costs = vec![0.0f32; 16];
        let got = ffi::kkc_glide_decode(
            engine,
            1,
            key_str.as_ptr() as *const c_char,
            kxy.as_ptr(),
            ks.len() as i32,
            pts.as_ptr(),
            (pts.len() / 2) as i32,
            5,
            out.as_mut_ptr(),
            512,
            costs.as_mut_ptr(),
        );
        assert!(got >= 1, "候補数 {}", got);
        let mut first = Vec::new();
        let mut at = 0usize;
        while out[at] != 0 {
            first.push(out[at]);
            at += 1;
        }
        assert_eq!(crate::from_u16s(&first), "hello");
    }
}
