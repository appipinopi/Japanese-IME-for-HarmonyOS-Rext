# rext_* 追加輸出（ArkTS / Kotlin 小物ロジック用）

`librext_kkc.so` は engine.h の `kkc_*` ABI（52本）に加え、小物ロジック用の
`rext_*` 関数 **25本** を輸出する。実装は `src/rext_exports.rs` で、本体は
すべて rext-core の既存モジュール（新規ロジックは無い）。これにより
ArkTS の `JapaneseConverter.ets` / `NumberFormatter.ets` /
`DateTimePredictor.ets` / `Brackets.ets` / `Pos.ets` と、Kotlin の同名
クラスに重複していた小物ロジックを、NAPI / JNI 経由の呼び出しに置き換えられる。

## 共通規約（kkc_* と一貫）

- **文字コード**: 入出力とも UTF-8・NUL 終端の `char*`。
  kkc_* のパス引数（`kkc_open_files`）や `kkc_glide_set_words` と同じ表現。
  rext-core 内部の UTF-16 への変換は輸出層で行うので、呼び出し側は意識しない。
- **所有権と解放**: 戻り値の `char*` は Rust 側ヒープ確保で、所有権は
  呼び出し側へ移る。**必ず `rext_free_string(ptr)` でちょうど 1 回解放**すること。
  `free()` / `delete[]` で解放してはいけない（アロケータが違う）。
  `rext_free_string(NULL)` は何もしない（安全）。
- **NULL 安全**: 入力ポインタが NULL の場合、文字列関数は NULL を、
  スカラ関数は 0 を返す。落ちない。
- **候補リストの表現**: 複数候補を返す関数は、候補を `'\n'` 区切りで
  連結した 1 本の文字列を返す（候補自体に改行は含まれない）。
  候補が無い場合は NULL ではなく **確保済みの空文字列 `""`** を返す。
- **失敗の表現**: 単一値を返す関数で結果が無い場合（数字でない入力、
  未知の括弧、和暦の範囲外の年）は空文字列ではなく **NULL** を返す。

## 関数一覧

| 関数 | 引数 → 戻り値 | 対応する rext-core |
|---|---|---|
| `rext_free_string` | `char*` → void | （解放専用） |
| `rext_romaji_to_kana` | `const char*` → `char*` | `romaji::Romaji::process`+`flush`（無状態で一括変換。"neko"→"ねこ"、"kan"→"かん"） |
| `rext_kana_to_romaji` | `const char*` → `char*` | `romaji::Romaji::to_romaji`（"とうきょう"→"toukyou"） |
| `rext_to_katakana` | `const char*` → `char*` | `text::to_katakana` |
| `rext_to_hiragana` | `const char*` → `char*` | `text::to_hiragana` |
| `rext_to_halfwidth_kana` | `const char*` → `char*` | `text::to_halfwidth_kana` |
| `rext_to_fullwidth_ascii` | `const char*` → `char*` | `text::to_fullwidth_ascii` |
| `rext_to_halfwidth_ascii` | `const char*` → `char*` | `text::to_halfwidth_ascii` |
| `rext_number_add_commas` | `const char*` → `char*`（数字でなければ NULL） | `number::normalize_digits` + `number::add_commas`（全角数字も可） |
| `rext_number_to_kanji` | `const char*` → `char*`（同上） | `number::to_kanji` |
| `rext_number_to_man_mixed` | `const char*` → `char*`（同上） | `number::to_man_mixed`（"12345678"→"1234万5678"） |
| `rext_number_era_year` | `int64 year` → `char*`（1926年より前は NULL） | `number::era_year`（2026→"令和8年"） |
| `rext_number_forms` | `const char*` → `char*`（候補リスト） | `number::number_forms`（和暦・桁区切り・万混ぜ・漢数字・丸数字） |
| `rext_number_predict` | `const char*` → `char*`（候補リスト） | `number::predict`（スマホ版 NumberFormatter.predict、4桁以上のみ） |
| `rext_datetime_predict` | `const char*, int64 now_unix` → `char*`（候補リスト） | `datetime::datetime_predict`（ローカル時刻は UTC+9 固定） |
| `rext_datetime_predict_with_offset` | `const char*, int64 now_unix, int64 offset_seconds` → `char*`（候補リスト） | `datetime::datetime_predict_with_offset` |
| `rext_bracket_closing_for` | `const char*` → `char*`（括弧でなければ NULL） | `brackets::closing_for` |
| `rext_bracket_opening_for` | `const char*` → `char*`（同上） | `brackets::opening_for` |
| `rext_bracket_auto_close` | `const char*` → `char*`（＜ < ‘ と未知は NULL） | `brackets::auto_close` |
| `rext_bracket_variants` | `const char*` → `char*`（候補リスト） | `brackets::bracket_variants` |
| `rext_bracket_opens` | （なし） → `char*`（24件の候補リスト） | `brackets::opens` |
| `rext_bracket_closes` | （なし） → `char*`（24件の候補リスト） | `brackets::closes` |
| `rext_pos_class_count` | （なし） → `int32`（2672） | `pos::POS_CLASS_COUNT` |
| `rext_pos_class` | `int32 lid` → `uint32`（コードポイント '0'..'4'、範囲外は '0'=48） | `pos::pos_class`（品詞表は `pos`/`tables` の生成表） |
| `rext_pos_group` | `int32 lid` → `uint32`（'N'/'V'/'P'…、不明は 0） | `pos::group_of_lid` |

## 呼び出し手順の要点

### HarmonyOS（ArkTS = NAPI 経由）

1. `entry/src/main/cpp/napi_kkc.cpp` に、kkc_* と同じ要領でラッパを足す:
   引数は `napi_get_value_string_utf8` で UTF-8 として取り、
   戻り値は `napi_create_string_utf8` で ArkTS 文字列にしてから
   `rext_free_string` で解放する。
2. `entry/src/main/cpp/types/libkkc/index.d.ts` に型宣言を足す
   （例: `export const romajiToKana: (s: string) => string;`）。
3. ArkTS 側（`JapaneseConverter.ets` 等）はラッパを呼ぶだけに置き換える。

```cpp
// napi_kkc.cpp 側ラッパの形（抜粋）
static napi_value RomajiToKana(napi_env env, napi_callback_info info) {
    // ... argc/argv 取得、buf に UTF-8 入力を読む ...
    char *out = rext_romaji_to_kana(buf);
    napi_value ret;
    napi_create_string_utf8(env, out ? out : "", NAPI_AUTO_LENGTH, &ret);
    rext_free_string(out);  // NULL でも安全
    return ret;
}
```

### Android（Kotlin = JNI 経由）

1. `jni.cpp` に `JNIEXPORT jstring JNICALL Java_..._romajiToKana(...)` を足す:
   入力は `GetStringUTFChars`、戻り値は `NewStringUTF(out)` の後に
   `rext_free_string(out)`。
2. Kotlin 側は `external fun romajiToKana(s: String): String` を宣言し、
   既存の `Engine.kt` と同じ `System.loadLibrary` 済みライブラリから呼ぶ。
   候補リスト関数の戻り値は `split('\n')` して使う（空文字列なら候補なし）。

## 未対応（理由）

- `romaji::Romaji` の状態あり操作（1キーずつの `process`/`pending`/
  `backspace`、カスタム表の `add_pair`、拡張ローマ字）: ハンドル型の
  ABI 設計が別途必要。今回の小物置換の対象は無状態の一括変換のみ。
- `pos::words` / `pos::head_lid`: 入力が構造体配列（pathPos の結果）で、
  文字列規約に乗らないため未輸出。
- `flow::SentenceFlow`（文脈予測）: 状態あり＋ファイル保存先が OS 依存。
  RUST_PORT_STATUS の残件 4/5 と同じ理由で次段階。
- `text::japanese_symbol` 等の 1 字スカラ関数は需要が出たら追加する。

## 検証

- `cargo test -p rext-kkc`: rext_exports テスト **7 passed / 0 failed**
  （各 rext_* を直接呼び、期待値一致と NULL 安全を確認）。
  ワークスペース全体でも rext-core の 139 件を含め全件合格を確認
  （他担当が並行追加中の golden 突き合わせテストは実行タイミングで増減する）。
- `cargo build --release` 成功。`nm -D librext_kkc.so`:
  `rext_*` 25本を輸出、`kkc_*` は従来どおり 52本。
- ホスト側検証: C プログラムから `librext_kkc.so` を直接リンクし
  全 rext_* を呼ぶチェック 30/30 合格（napi_kkc.cpp / jni.cpp と
  同じリンク形態）。
