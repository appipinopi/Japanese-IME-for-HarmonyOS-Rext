// 括弧の全種類 (開き, 閉じ)。どの括弧も必ず打てるように、変換の候補はここから出す。
// Android 版 Brackets.kt・Windows 版 desktop/core/composer.cpp の BRACKETS も同じ並び (変えるときは 3 つとも)。
//
// 第3弾 (Rust 化): bracketVariants と closingFor / openingFor / autoClose は、まず Rust 版
// rext-kkc の rext_bracket_* を libkkc.so 経由で呼ぶ (候補は '\n' 区切りで返るのでここで
// string[] に直す)。ネイティブが読み込めない・例外・結果なしのときは下の純実装に落ちる。
// ALL_BRACKETS の表そのものは純実装フォールバックとして残す (rext 側の表も同じ並び)。
import kkc from 'libkkc.so';
export const ALL_BRACKETS: string[][] = [
  ['（', '）'], ['(', ')'], ['「', '」'], ['『', '』'], ['【', '】'], ['［', '］'], ['[', ']'],
  ['｛', '｝'], ['{', '}'], ['〔', '〕'], ['〈', '〉'], ['《', '》'], ['〖', '〗'], ['〘', '〙'],
  ['〚', '〛'], ['｢', '｣'], ['＜', '＞'], ['<', '>'], ['«', '»'], ['‹', '›'], ['“', '”'],
  ['‘', '’'], ['〝', '〟'], ['｟', '｠'],
];

// 開き括弧 -> 対応する閉じ括弧。括弧でなければ空文字列
export function bracketClosingFor(open: string): string {
  try {
    const c = kkc.rextBracketClosingFor(open);
    if (c.length > 0) { return c; }
  } catch (e) {
    // 純実装に落ちる
  }
  return bracketClosingForPure(open);
}

function bracketClosingForPure(open: string): string {
  for (const p of ALL_BRACKETS) { if (p[0] === open) { return p[1]; } }
  return '';
}

// 閉じ括弧 -> 対応する開き括弧。括弧でなければ空文字列
export function bracketOpeningFor(close: string): string {
  try {
    const o = kkc.rextBracketOpeningFor(close);
    if (o.length > 0) { return o; }
  } catch (e) {
    // 純実装に落ちる
  }
  for (const p of ALL_BRACKETS) { if (p[1] === close) { return p[0]; } }
  return '';
}

// 開き括弧 -> 自動で閉じる対象ならその閉じ括弧、でなければ空文字列
// (＜ < ‘ は不等号・アポストロフィにも使うので自動では閉じない。rext_bracket_auto_close と同じ決まり)
export function bracketAutoClose(open: string): string {
  try {
    const c = kkc.rextBracketAutoClose(open);
    if (c.length > 0) { return c; }
    // 空は「自動で閉じない」と「リンクなし」の両方でありうるが、純実装も
    // 同じ決まりで判定するので、そのまま純実装の答えを採用してよい
  } catch (e) {
    // 純実装に落ちる
  }
  if (open === '＜' || open === '<' || open === '‘') { return ''; }
  return bracketClosingForPure(open);
}

// 読みに合う括弧の候補。
//   括弧 1 字 → 同じ側 (開き・閉じ) の全種類 (打った字が先頭)
//   かっこ → 全種類の組 / かっこひらき・かっことじ → 開き・閉じの全種類
// 括弧でない読みは空。
export function bracketVariants(reading: string): string[] {
  // Rust 版 (rext_bracket_variants) を先に試す。記号の仲間 (markVariants) まで
  // rext 側に同じ実装がある。空が返るのは「候補なし」か「リンクなし」で、
  // どちらも純実装で確かめ直せば結果は同じになる。
  try {
    const raw = kkc.rextBracketVariants(reading);
    if (raw.length > 0) { return raw.split('\n'); }
  } catch (e) {
    // libkkc.so が読めない環境では純実装に落ちる
  }
  return bracketVariantsPure(reading);
}

function bracketVariantsPure(reading: string): string[] {
  const opens: string[] = ALL_BRACKETS.map((p: string[]) => p[0]);
  const closes: string[] = ALL_BRACKETS.map((p: string[]) => p[1]);
  if (reading === 'かっこ') { return ALL_BRACKETS.map((p: string[]) => p[0] + p[1]); }
  if (reading === 'かっこひらき') { return opens; }
  if (reading === 'かっことじ') { return closes; }
  if (opens.includes(reading)) { return [reading, ...opens.filter((c: string) => c !== reading)]; }
  if (closes.includes(reading)) { return [reading, ...closes.filter((c: string) => c !== reading)]; }
  return markVariants(reading);
}

// 記号 1 字: 全角・半角の両方と仲間の記号 (打った字が先頭)。MS-IME と同じく・で ／ /、￥ で ＼ \ (ブログの報告 2026-11-02。
// Windows 版 composer.cpp の MARKS・Android 版 Brackets.kt と同じ)。英数字は空
const MARKS: string[][] = [
  ['・', '／', '/'], ['、', '，', ','], ['。', '．', '.'], ['ー', '－', '-', '―', '‐'], ['～', '〜', '~'],
  ['￥', '＼', '\\'], ['＊', '*', '※', '×'], ['＂', '"', '“', '”'], ['＇', '\'', '‘', '’'],
];

function markVariants(reading: string): string[] {
  if (reading.length !== 1) { return []; }
  let group: string[] = [];
  for (const g of MARKS) {
    if (g.includes(reading)) { group = g; break; }
  }
  if (group.length === 0) {
    const ch = reading.charCodeAt(0);
    const half = ch >= 0xFF01 && ch <= 0xFF5E ? ch - 0xFEE0 : ch;
    const isAlnum = (half >= 0x30 && half <= 0x39) || (half >= 0x41 && half <= 0x5A) || (half >= 0x61 && half <= 0x7A);
    if (half <= 0x20 || half > 0x7E || isAlnum) { return []; }
    group = [String.fromCharCode(half + 0xFEE0), String.fromCharCode(half)];
  }
  return [reading, ...group.filter((c: string) => c !== reading)];
}
