//! 書き方の癖 (元の desktop/core/style.cpp + style.h の移植。2026-10-28)。
//! モデルには読ませず、確定した書き方を数えて 1 位の書き方 (できない ↔ 出来ない) だけを入れ替える。
//! - 書き方の対の表 (kkc_variants.tsv): かなの形・漢字の形・見出し・品詞の種類
//! - 文の種類の判定器 (kkc_register.bin): 左の文脈から 丁寧 / 崩れ の確率 (字の 1〜3 字組の hash を引いて平均)
//! - 覚え方: 見出しごとの回数 + 3 × 品詞の種類の漢字率 (経験ベイズ)。回数は文の種類で重みづけ
//!   (覚えるとき A += r̂、使うとき a = A·r̂0 + 0.2 × 全体の回数。r̂ = r² を長さ 1 に)。見込みが 0.7 以上なら漢字・0.3 以下ならかな
//!
//! 依存: crate::converter::{Word, Conversion} は元の converter.h の形を前提に使う。
//! var_ は元の std::unordered_multimap の代わりに Vec<(キー, Form)> (挿入順) で持ち、equal_range 相当は全走査で再現する。

use std::collections::HashMap;

use crate::converter::Conversion;
use crate::{u16s, U16};

/// 書き方ごとの記録: 文の種類で重みづけした回数・回数
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Count {
    pub w: [f32; 2],
    pub n: i32,
}

#[derive(Debug, Clone)]
struct Form {
    other: U16,
    lemma: i32,
    cls: i32,
    kanji: bool,
}

const A: f32 = 3.0;
const T: f32 = 0.2;
const LAM: f32 = 0.2;

/// zlib.crc32 と同じ (判定器の学習は Python)
fn crc32(s: &[u8]) -> u32 {
    static TAB: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let tab = TAB.get_or_init(|| {
        let mut t = [0u32; 256];
        for i in 0..256u32 {
            let mut c = i;
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    0xEDB88320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
            t[i as usize] = c;
        }
        t
    });
    let mut c = 0xFFFFFFFFu32;
    for &b in s {
        c = tab[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    c ^ 0xFFFFFFFF
}

fn put_utf8(o: &mut Vec<u8>, c: u32) {
    if c < 0x80 {
        o.push(c as u8);
    } else if c < 0x800 {
        o.push((0xC0 | (c >> 6)) as u8);
        o.push((0x80 | (c & 0x3F)) as u8);
    } else if c < 0x10000 {
        o.push((0xE0 | (c >> 12)) as u8);
        o.push((0x80 | ((c >> 6) & 0x3F)) as u8);
        o.push((0x80 | (c & 0x3F)) as u8);
    } else {
        o.push((0xF0 | (c >> 18)) as u8);
        o.push((0x80 | ((c >> 12) & 0x3F)) as u8);
        o.push((0x80 | ((c >> 6) & 0x3F)) as u8);
        o.push((0x80 | (c & 0x3F)) as u8);
    }
}

/// UTF-16 → コードポイント列 (サロゲート対を合わせる。元の to32)
fn to32(s: &[u16]) -> Vec<u32> {
    let mut o: Vec<u32> = Vec::new();
    let mut i = 0usize;
    while i < s.len() {
        let mut c = s[i] as u32;
        if (0xD800..0xDC00).contains(&c) && i + 1 < s.len() {
            c = 0x10000 + ((c - 0xD800) << 10) + (s[i + 1] as u32 - 0xDC00);
            i += 1;
        }
        o.push(c);
        i += 1;
    }
    o
}

#[derive(Debug, Clone, Default)]
pub struct StyleModel {
    /// 同じ漢字に読みの違う対がある (内 = うち・ない、良い = いい・よい)。書き換えるときは語の読みで選ぶ (2026-11-15)。
    /// (キー, 対) の並び。元の std::unordered_multimap と違い挿入順を保つ
    var_: Vec<(U16, Form)>,
    w_: Vec<i8>, // 判定器 (H 行 × 2)
    scale_: f32,
    h_: u32,
    /// 見出し・品詞の種類ごとの [漢字/かな] の重みつき回数と回数
    lem_: HashMap<i32, [Count; 2]>,
    cls_: [[Count; 2]; 8],
}

impl StyleModel {
    pub fn new() -> StyleModel {
        StyleModel::default()
    }

    pub fn set_variants(&mut self, tsv_utf8: &str) {
        self.var_.clear();
        let mut lem: HashMap<U16, i32> = HashMap::new();
        for line0 in tsv_utf8.split('\n') {
            let line = line0.strip_suffix('\r').unwrap_or(line0);
            let col: Vec<&str> = line.split('\t').collect();
            if col.len() < 4 || col[0].is_empty() || col[1].is_empty() {
                continue;
            }
            let kana = u16s(col[0]);
            let kanji = u16s(col[1]);
            let key = u16s(col[2]);
            let next = lem.len() as i32;
            let id = *lem.entry(key).or_insert(next);
            let mut cl: i32 = 0;
            for ch in col[3].chars() {
                if ch.is_ascii_digit() {
                    cl = cl * 10 + (ch as i32 - '0' as i32);
                }
            }
            if !(0..=7).contains(&cl) {
                cl = 0;
            }
            self.var_.push((
                kana.clone(),
                Form {
                    other: kanji.clone(),
                    lemma: id,
                    cls: cl,
                    kanji: false,
                },
            ));
            self.var_.push((
                kanji,
                Form {
                    other: kana,
                    lemma: id,
                    cls: cl,
                    kanji: true,
                },
            ));
        }
    }

    /// 形: "SREG" 版 (1) H (u32) 尺度 (f32) と H × 2 の int8 (行ごとに 丁寧・崩れ)。
    /// 壊れていれば false (文の種類を見ずに数える)
    pub fn set_register(&mut self, bin: &[u8]) -> bool {
        self.w_.clear();
        self.h_ = 0;
        if bin.len() < 16 || &bin[0..4] != b"SREG" {
            return false;
        }
        let ver = u32::from_le_bytes(bin[4..8].try_into().unwrap());
        let h = u32::from_le_bytes(bin[8..12].try_into().unwrap());
        let sc = f32::from_le_bytes(bin[12..16].try_into().unwrap());
        if ver != 1 || h == 0 || bin.len() != 16 + (h as usize) * 2 {
            return false;
        }
        self.w_ = bin[16..].iter().map(|&b| b as i8).collect();
        self.h_ = h;
        self.scale_ = sc;
        true
    }

    pub fn ready(&self) -> bool {
        !self.var_.is_empty()
    }

    pub fn is_variant(&self, s: &[u16]) -> bool {
        self.var_.iter().any(|(k, _)| k == s)
    }

    /// 丁寧・崩れの確率 (判定器が無ければ半々)
    pub fn register_of(&self, ctx: &[u16]) -> [f32; 2] {
        if self.h_ == 0 || ctx.is_empty() {
            return [0.5, 0.5]; // 文脈が無いときは分からない
        }
        let mut s = to32(ctx);
        if s.len() > 80 {
            s = s.split_off(s.len() - 80);
        }
        let mut g: Vec<u32> = Vec::new();
        let len_key = format!("L{}", (s.len() / 10).min(8));
        g.push(crc32(len_key.as_bytes()) % self.h_);
        for n in 1..=3usize {
            if n > s.len() {
                break;
            }
            for i in 0..=s.len() - n {
                let mut u: Vec<u8> = Vec::new();
                for k in 0..n {
                    put_utf8(&mut u, s[i + k]);
                }
                g.push(crc32(&u) % self.h_);
            }
        }
        let mut z = [0f32; 2];
        for &x in &g {
            z[0] += self.w_[(x as usize) * 2] as f32;
            z[1] += self.w_[(x as usize) * 2 + 1] as f32;
        }
        let d = (z[1] - z[0]) * self.scale_ / (g.len() as f32);
        let p1 = 1.0f32 / (1.0f32 + (-d).exp());
        [1.0 - p1, p1]
    }

    /// 覚えるときの重み r̂
    pub fn weight_of(&self, ctx: &[u16]) -> [f32; 2] {
        let r = self.register_of(ctx);
        let a = r[0] * r[0];
        let b = r[1] * r[1];
        let n = (a * a + b * b).sqrt();
        [a / n, b / n]
    }

    pub fn set_counts(&mut self, counts: &HashMap<U16, Count>) {
        self.lem_.clear();
        self.cls_ = [[Count::default(); 2]; 8];
        for (s, c) in counts {
            // 元は var_ の find (最初に見つかった対) で振り分ける
            let f = match self.var_.iter().find(|(k, _)| k == s) {
                Some((_, f)) => f.clone(),
                None => continue,
            };
            let k = if f.kanji { 0 } else { 1 };
            for dst in [
                &mut self.lem_.entry(f.lemma).or_default()[k],
                &mut self.cls_[f.cls as usize][k],
            ] {
                dst.w[0] += c.w[0];
                dst.w[1] += c.w[1];
                dst.n += c.n;
            }
        }
    }

    /// その語を今の文脈で書くなら: 1 漢字・-1 かな・0 分からない
    pub fn pref(&self, surface: &[u16], r: &[f32; 2]) -> i32 {
        match self.var_.iter().find(|(k, _)| k == surface) {
            None => 0,
            Some((_, f)) => {
                let f = f.clone();
                self.pref_form(&f, r)
            }
        }
    }

    fn pref_form(&self, f: &Form, r: &[f32; 2]) -> i32 {
        let eff = |c: &Count| c.w[0] * r[0] + c.w[1] * r[1] + LAM * (c.n as f32);
        let (mut a1, mut b1) = (0f32, 0f32);
        if let Some(l) = self.lem_.get(&f.lemma) {
            a1 = eff(&l[0]);
            b1 = eff(&l[1]);
        }
        let ck = &self.cls_[f.cls as usize];
        let ak = eff(&ck[0]);
        let bk = eff(&ck[1]);
        let q = (ak + 1.0) / (ak + bk + 2.0);
        let pk = (a1 + A * q) / (a1 + b1 + A);
        if pk >= 0.5 + T {
            1
        } else if pk <= 0.5 - T {
            -1
        } else {
            0
        }
    }

    /// 1 位 (out.cands[0]、語の区切り out.words) の書き方を、覚えた書き方に入れ替えた候補を先頭に足す。足したら true
    pub fn restyle(&self, ctx: &[u16], out: &mut Conversion) -> bool {
        if self.var_.is_empty() || self.lem_.is_empty() || out.cands.is_empty() || out.words.is_empty()
        {
            return false;
        }
        let r = self.weight_of(ctx);
        let mut text: U16 = Vec::new();
        let mut words = Vec::new();
        let mut changed = false;
        // 網の語の区切りは書き方の対の表と合わないことがある (でき|ない と できない): 隣の語を 3 つまでつないで長い方から引く。
        // 漢字の形は、語の読みが対のかなの形と同じものだけ (読みの違う 内 (ない) を うち に替えていた。ユーザー 2026-11-15)
        let mut i = 0usize;
        while i < out.words.len() {
            let mut len = 1usize;
            let mut found: Option<Form> = None;
            let mut l = 3usize.min(out.words.len() - i);
            while l >= 1 && found.is_none() {
                let mut t: U16 = Vec::new();
                let mut rd: U16 = Vec::new();
                for k in 0..l {
                    t.extend_from_slice(&out.words[i + k].surface);
                    rd.extend_from_slice(&out.words[i + k].reading);
                }
                for (key, f) in self.var_.iter() {
                    if *key == t && (!f.kanji || f.other == rd) {
                        found = Some(f.clone());
                        len = l;
                        break;
                    }
                }
                l -= 1;
            }
            let mut w = out.words[i].clone();
            for k in 1..len {
                let src = out.words[i + k].clone();
                w.reading.extend_from_slice(&src.reading);
                w.surface.extend_from_slice(&src.surface);
                w.rid = src.rid;
            }
            if let Some(f) = &found {
                let p = self.pref_form(f, &r);
                if p != 0 && (p > 0) != f.kanji {
                    w.surface = f.other.clone();
                    changed = true;
                }
            }
            text.extend_from_slice(&w.surface);
            words.push(w);
            i += len;
        }
        if !changed {
            return false;
        }
        if let Some(pos) = out.cands.iter().position(|c| *c == text) {
            out.cands.remove(pos);
        }
        out.cands.insert(0, text);
        out.words = words;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::converter::Word;
    use crate::from_u16s;

    fn variants() -> StyleModel {
        let mut m = StyleModel::new();
        m.set_variants("できない\t出来ない\tできない\t0\n");
        m
    }

    #[test]
    fn variants_pairs() {
        let m = variants();
        assert!(m.ready());
        assert!(m.is_variant(&u16s("できない")));
        assert!(m.is_variant(&u16s("出来ない")));
        assert!(!m.is_variant(&u16s("する")));
    }

    #[test]
    fn register_broken_and_empty() {
        let mut m = StyleModel::new();
        assert!(!m.set_register(b"XXXX"));
        assert!(!m.set_register(b"SREG\x02\0\0\0\x01\0\0\0\0\0\0\0\0\0")); // 版が違う
        // 判定器が無ければ半々、重みは 1/√2 ずつ
        assert_eq!(m.register_of(&u16s("こんにちは")), [0.5, 0.5]);
        let w = m.weight_of(&u16s("こんにちは"));
        assert!((w[0] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        // 文脈が空でも半々
        assert_eq!(m.register_of(&[]), [0.5, 0.5]);
    }

    #[test]
    fn register_bin_roundtrip() {
        // H=4、全ての行が (0, 127): 崩れ寄りになるはず
        let mut bin: Vec<u8> = Vec::new();
        bin.extend_from_slice(b"SREG");
        bin.extend_from_slice(&1u32.to_le_bytes());
        bin.extend_from_slice(&4u32.to_le_bytes());
        bin.extend_from_slice(&1.0f32.to_le_bytes());
        for _ in 0..4 {
            bin.push(0);
            bin.push(127);
        }
        let mut m = StyleModel::new();
        assert!(m.set_register(&bin));
        let r = m.register_of(&u16s("あ"));
        assert!(r[1] > 0.99, "r={:?}", r);
        assert!((r[0] + r[1] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn crc32_known() {
        // zlib.crc32(b"123456789") = 0xCBF43926
        assert_eq!(crc32(b"123456789"), 0xCBF43926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn pref_and_restyle() {
        let mut m = variants();
        // 漢字で 10 回書いた記録 (丁寧・崩れ 半々ずつの重み)
        let mut counts: HashMap<U16, Count> = HashMap::new();
        counts.insert(
            u16s("出来ない"),
            Count {
                w: [5.0, 5.0],
                n: 10,
            },
        );
        m.set_counts(&counts);
        let r = [0.5, 0.5];
        assert_eq!(m.pref(&u16s("できない"), &r), 1); // 漢字で書く
        assert_eq!(m.pref(&u16s("出来ない"), &r), 1);
        assert_eq!(m.pref(&u16s("する"), &r), 0);
        // restyle: 1 位が かな なら漢字の候補を先頭に足す
        let mut conv = Conversion {
            cands: vec![u16s("できない")],
            words: vec![Word {
                reading: u16s("できない"),
                surface: u16s("できない"),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(m.restyle(&u16s(""), &mut conv));
        assert_eq!(from_u16s(&conv.cands[0]), "出来ない");
        assert_eq!(from_u16s(&conv.words[0].surface), "出来ない");
        // すでに漢字なら何もしない
        let mut conv2 = Conversion {
            cands: vec![u16s("出来ない")],
            words: vec![Word {
                reading: u16s("できない"),
                surface: u16s("出来ない"),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(!m.restyle(&u16s(""), &mut conv2));
    }

    #[test]
    fn pref_kana_side() {
        let mut m = variants();
        let mut counts: HashMap<U16, Count> = HashMap::new();
        counts.insert(
            u16s("できない"),
            Count {
                w: [5.0, 5.0],
                n: 10,
            },
        );
        m.set_counts(&counts);
        assert_eq!(m.pref(&u16s("出来ない"), &[0.5, 0.5]), -1); // かなで書く
    }
}
