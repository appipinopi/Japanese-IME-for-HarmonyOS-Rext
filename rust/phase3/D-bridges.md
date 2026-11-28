# 第3弾 D: ArkTS / Kotlin 小物ロジックの rext_* 置き換え（ブリッジ配線）

担当範囲: `napi_kkc.cpp` / `index.d.ts` / `jni.cpp` と、.ets 5 件・.kt 5 件の
小物ロジック（NumberFormatter / Brackets / Pos / SentenceFlow / DateTimePredictor）を、
Rust 版 rext-kkc の `rext_*` 25 本（`rust/crates/rext-kkc/REXT_EXPORTS.md`）への
呼び出しに置き換える配線。元の GitHub へは push していない。

前提（コーディネータ確認済み）: この環境に HarmonyOS SDK / Android SDK・NDK は無い。
.ets / .kt のコンパイルと実機確認はできない。ここに書く検証はホストで可能な範囲のみで、
.ets / .kt は**未検証（コンパイル未確認）**として扱うこと。

## 共通方針（壊れにくさ優先）

1. **weak シンボル**: `napi_kkc.cpp` / `jni.cpp` の追加分は、rext_* を
   `__attribute__((weak))` の `extern "C"` で宣言する（rext_* 用の C ヘッダは
   リポジトリに無いため、宣言は各 .cpp 内に置いた）。
   - Rust 版ライブラリ（kkc_* 52 本 + rext_* 25 本を出す librext_kkc）がリンク
     されていれば実物が呼ばれ、されていなければシンボルが NULL になり、ラッパは
     空文字列 / 空配列 / -1 を返すだけで、モジュール自体の読み込みは失敗しない。
   - 従来ビルド（C++ engine.cpp のみ）でも libkkc.so が壊れないための措置。
     実機リンクでの weak 解決は未検証（ホスト ELF では検証済み。下記）。
2. **所有権**: 戻り文字列は、NAPI 側は `napi_create_string_utf8` で ArkTS 文字列に
   した後に `rext_free_string` で解放、JNI 側は `NewStringUTF` の後に
   `rext_free_string` で解放（ちょうど 1 回。NULL でも安全）。
3. **候補リスト**: NAPI 側は `'\n'` 区切りの 1 本の文字列のまま返し、string[] への
   分割は ArkTS 側で行う（規約どおり）。JNI 側は C++ で分割して `Array<String>` で
   返す（既存の nativeConvert 等の配列返却スタイルに合わせた）。
4. **フォールバック**: .ets / .kt 側は全箇所「ネイティブを先に試し、未ロード・
   例外・空（結果なし）のときは元の純実装に落ちる」。純実装は削除せず
   `predictPure` / `variantsPure` 等に改名して残した。rext 側と純実装は同じ
   決まりのもの同士を対応させてあるので、空が「候補なし」か「リンクなし」か
   区別できない場合でも、純実装で確かめ直せば結果は一致する。

## ファイル別の変更

### entry/src/main/cpp/napi_kkc.cpp

- rext_* 24 本（free を除く全輸出）の weak 宣言を冒頭に追加（`#include <cstdint>` も追加）。
- ラッパ 24 本を既存パターン（`str_arg` で UTF-8 取得、結果を napi_value で返す）で追加し、
  `Init` の `desc[]` に `rextXxx` の名前で登録。既存の登録・関数は無変更。
  - 文字列 1 引数→文字列: romajiToKana / kanaToRomaji / toKatakana / toHiragana /
    toHalfwidthKana / toFullwidthAscii / toHalfwidthAscii / numberAddCommas /
    numberToKanji / numberToManMixed / bracketClosingFor / bracketOpeningFor /
    bracketAutoClose（共用ヘルパ `rext_str1`）
  - 候補リスト（'\n' 区切りのまま返す）: numberForms / numberPredict /
    bracketVariants（同上）、bracketOpens / bracketCloses（引数なし版ヘルパ）
  - numberEraYear: `napi_get_value_int64` で年を受ける
  - datetimePredict（UTC+9 固定版）/ datetimePredictWithOffset（オフセット指定版）
  - posClassCount（int32）/ posClass / posGroup（コードポイントを 1 字文字列で返す。
    0=不明 とリンクなしは空文字列）
  - 単一値関数が NULL を返す場合（数字でない等）は空文字列に直す（d.ts に明記）。

### entry/src/main/cpp/types/libkkc/index.d.ts

- 末尾に rext_* 24 本の型宣言を追加（既存の型は無変更）。
  候補リスト系は「'\n' 区切りの文字列を返す。使う側で split する」と明記。

### android/app/src/main/cpp/jni.cpp

- 既存の Engine 用 JNI は無変更。末尾に rext_* ブリッジ 16 本を追加。
- 配置先クラスは各 .kt の object に合わせ、JNI 名は
  `Java_com_shunti_japaneseime_<Object名>_native<名前>`、第 2 引数は object の
  インスタンスメソッドなので `jobject`（Engine の companion @JvmStatic = jclass とは違う）。
  - NumberFormatter（6）: nativeNumberAddCommas / nativeNumberToKanji /
    nativeNumberToManMixed / nativeNumberEraYear(jlong) / nativeNumberForms /
    nativeNumberPredict
  - DateTimePredictor（1）: nativeDatetimePredict(reading, nowUnix, offsetSeconds)
    → `rext_datetime_predict_with_offset` に接続（端末 TZ を使うため。UTC+9 固定版は
    Android ではブリッジしていない）
  - Brackets（6）: nativeBracketClosingFor / nativeBracketOpeningFor /
    nativeBracketAutoClose / nativeBracketVariants / nativeBracketOpens /
    nativeBracketCloses
  - Pos（3）: nativePosClassCount / nativePosClass / nativePosGroup
    （コードポイントを jint で返す。リンクなしは -1、不明は rext_* の 0）
- 入力文字列は `GetStringUTFChars`（変形 UTF-8 はサロゲートペアを化けさせる）を
  避け、`GetStringChars` から UTF-16→UTF-8 変換するヘルパ `rext_utf8_arg` を追加。
  出力は数字・かな・漢字・括弧など BMP の字だけなので `NewStringUTF` で問題ない
  （前提として記録する）。
- ローマ字変換・text 系（romajiToKana 等 7 本）は Android 側にブリッジしていない:
  対応する受け手は JapaneseConverter.kt で、本担当の対象ファイル外のため。
  HarmonyOS 側は napi に全 24 本を配線済み（受け手の JapaneseConverter.ets は対象外
  だが、libkkc の公開面として揃えた）。

### entry/src/main/ets/ime/NumberFormatter.ets

- `import kkc from 'libkkc.so'` を追加。`predict` は `kkc.rextNumberPredict` を
  先に試し（'\n' を split）、失敗時は従来実装（`predictPure` に改名）に落ちる。
- rext 側 predict は純実装と同一仕様（4 桁以上・0 始まり除外・桁区切り/万混ぜ/
  漢数字の順）であることを Rust ソースで確認済み。

### entry/src/main/ets/ime/Brackets.ets

- `import kkc` を追加。`bracketVariants` は `rextBracketVariants` を先に試し、
  従来実装は `bracketVariantsPure` として残した（記号の markVariants 含め rext 側に
  同一実装があることを Rust ソースで確認済み）。
- 新規 export `bracketClosingFor` / `bracketOpeningFor` / `bracketAutoClose` を追加
  （rext_bracket_* 優先、ALL_BRACKETS の純表がフォールバック。autoClose の
  ＜ < ‘ 除外は rext 側と同じ決まり）。既存の呼び出し側は変更していない。
- `ALL_BRACKETS` はフォールバック用に純データのまま残置。

### entry/src/main/ets/ime/Pos.ets

- `cls` / `group` は `rextPosClass` / `rextPosGroup` を先に試し、空・例外時は
  従来の `posTables()` の表に落ちる。
- `words` / `headLid` は**置き換えていない**: `pos::words` / `pos::head_lid` は
  入力が構造体配列で文字列規約に乗らず、rext_* に輸出が無い（REXT_EXPORTS.md
  未対応節）。純実装のまま。

### entry/src/main/ets/ime/DateTimePredictor.ets

- `import kkc` を追加。`predict(reading, now)` は
  `rextDatetimePredictWithOffset(reading, floor(now/1000), -getTimezoneOffset()*60)`
  を先に試し、失敗時は従来実装（`predictPure` に改名）に落ちる。
  UTC+9 固定版ではなくオフセット指定版を使い、端末の現地時刻で数える。
- 日時候補は現行 .ets の dateForms 12 形が正で、rext 側も 12 形を返す前提
  （ホスト実行で実際に 12 形返ることを確認済み。下記）。

### entry/src/main/ets/ime/SentenceFlow.ets

- **実装は置き換えていない**（冒頭コメントに理由を明記したのみ）。
  SentenceFlow は状態あり（節バッファ・学習表・保存先が OS 依存）で、rext_* に
  状態あり関数の輸出が無い（REXT_EXPORTS.md 未対応節。ハンドル型 ABI が別途必要）。
  feed / close / predict の各部もすべて内部状態に依存し、純ロジックとして
  切り出せる部分が無いため、全体を純実装のまま残すのが妥当と判断した。

### android/app/src/main/java/com/shunti/japaneseime/NumberFormatter.kt

- `init { runCatching { System.loadLibrary("kkc") } }`（Engine を経由しない単体
  利用でも JNI を引けるように。失敗しても純実装に落ちるだけ）と external fun
  6 本（object 内なので JNI は jobject 受け）を追加。
- `predict` は `nativeNumberPredict` を先に試し、従来実装は `predictPure` に改名して残置。

### android/app/src/main/java/com/shunti/japaneseime/Brackets.kt

- 同様に init + external fun 6 本を追加。`variants` はネイティブ先行、従来実装は
  `variantsPure` に改名して残置。
- 新規に `closingFor` / `openingFor` / `autoClose`（public）を追加
  （ネイティブ先行、ALL / PAIRS がフォールバック）。
- `ALL` / `PAIRS` は初期化時に組み立てる静的な組として純データのまま残置
  （PAIRS は InputHandler が引く既存 API）。

### android/app/src/main/java/com/shunti/japaneseime/Pos.kt

- init + external fun 3 本（nativePosClassCount / nativePosClass / nativePosGroup）
  を追加。`cls` / `group` はネイティブ（コードポイント、-1 はリンクなし）を先に
  試し、従来の `Engine.posTables()` の表に落ちる。
- `headLid` は輸出が無いため純実装のまま（.ets 側と同じ理由）。

### android/app/src/main/java/com/shunti/japaneseime/DateTimePredictor.kt

- init + external fun 1 本（nativeDatetimePredict）を追加。`predict` は
  `now.timeInMillis/1000` と `now.timeZone.getOffset(...)/1000` を渡してネイティブを
  先に試し、従来実装は `predictPure` に改名して残置。
- **注意（挙動差）**: rext 側は HarmonyOS 版（12 形・曜日・週・<数>がつ<数>にち等）の
  移植で、このファイルの従来の純実装（日付 5 形・時刻 3 形、トリガも一部のみ）より
  候補が多い。ネイティブ有効時は Android でも候補の種類が増える。統一が意図に
  合わない場合は predictPure を正に戻せば従来挙動に戻る。

### android/app/src/main/java/com/shunti/japaneseime/SentenceFlow.kt

- **実装は置き換えていない**（ドキュメントコメントに理由を明記したのみ）。
  理由は .ets 版と同じ（状態あり＋filesDir 保存で輸出なし）。

## 置き換えできなかった部分と理由（まとめ）

| 対象 | 理由 |
|---|---|
| SentenceFlow（.ets / .kt とも） | 状態あり＋保存先 OS 依存。rext_* に状態あり関数の輸出なし（REXT_EXPORTS.md 未対応節） |
| Pos.words / Pos.headLid（.ets）、Pos.headLid（.kt） | pos::words / head_lid は入力が構造体配列で未輸出 |
| Brackets の ALL_BRACKETS / ALL・PAIRS | 静的データ表。フォールバックとして残置が安全（rext_bracket_opens/closes のブリッジ自体は用意済み） |
| Android のローマ字・text 系 7 本 | 受け手の JapaneseConverter.kt が本担当の対象ファイル外 |
| JNI の datetime UTC+9 固定版 | Android は端末 TZ のオフセット指定版で足りるため未ブリッジ（NAPI には両方ある） |

## 検証範囲（ホストで実施したもの）

- `cargo test`（rust/ 全体）: **161 passed / 0 failed**（内訳 146 + 8 + 7 の 3 スイート。rext_exports の 7 件を含む）。
- `cargo build --release` 成功。`nm -D librext_kkc.so`: rext_* 25 本 / kkc_* 52 本を輸出。
- napi_kkc.cpp / jni.cpp: 実ヘッダが無いため、モックヘッダ（napi/native_api.h、
  rawfile/*.h、jni.h、android/asset_manager*.h を /tmp に自作）で
  `g++ -std=c++17 -c` の**オブジェクト生成まで成功**（追加分を含むファイル全体）。
  jni.o の新規 JNI シンボル 16 本、napi_kkc.o の rext_* weak 未定義シンボルを確認。
  警告 1 件（napi_kkc.cpp:184 のリテラル内 NUL）は既存コードのもので追加分ではない。
- weak 機構の実動テスト（ブリッジと同じ宣言・ガード・解放パターンの小プログラム）:
  - librext_kkc.so をリンクした場合: rext_number_predict("1000000") =
    "1,000,000\n100万\n百万"、rext_number_era_year(2026) = "令和8年"、
    rext_datetime_predict_with_offset("きょう", 2025-11-29 12:00 JST) = **12 形**
    （曜日・和暦・8 桁形を含む現行 .ets と同じ並び）、rext_pos_class_count() = 2672、
    rext_pos_class(0) = '0'、rext_free_string(NULL) 安全 — すべて期待どおり。
  - リンクしない場合: weak シンボルが NULL になり、ガードがフォールバック経路を
    取ることを確認（このとき実行ファイルに .so への DT_NEEDED が必要。実モジュールでは
    kkc_* への強い参照が同じライブラリを引く前提）。

## 未検証（できないこと）

- .ets / .kt のコンパイル（HarmonyOS SDK / Android SDK・kotlinc が無い）。
  特に ArkTS の厳格化（arkts-no-any 等）や、object 内 external fun の JNI 名解決は
  実ビルドで要確認。
- 実機・実ビルドでの weak シンボル解決、NAPI 登録名と ArkTS import の一致、
  JNI ブリッジ経由の候補の見た目。
- B 担当の jni.cpp kkc_* 照合で修正指摘が出た場合は統合時に扱う（本担当では
  Engine 用 JNI に手を入れていない）。
