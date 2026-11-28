# Rust移植 進捗 (RUST_PORT_STATUS)

対象: 処理系をすべてRustへ。作業場所はこのコピー内のみ。元のGitHubへはpushしない。
方針: 単一のRustワークスペース `rust/` に統一する。
- `crates/rext-core`: ロジック本体（プラットフォーム非依存）
- `crates/rext-kkc`: cdylib。`entry/src/main/cpp/engine.h` の extern "C" `kkc_*` ABIをそのまま輸出する。
  → HarmonyOSの `napi_kkc.cpp` と Androidの `jni.cpp` は橋渡し層としてほぼ無変更でRust製 .so に差し替え可能。

## 対象外（明記）
- `tools/` 配下のPython/JS: 辞書ビルド・評価用のビルド時ツールであり、実行時処理系ではないため今回は対象外。
- UI・OS API依存コード: HarmonyOSの `entry/src/main/ets/components/`・`pages/`、Androidの View 類（KeyboardView等）、
  desktop/windows・desktop/linux のOS固有シェル。これらは各OSの必須言語（ArkTS/Kotlin/C++）に残る。
  ただしその下の「ロジック」は rext-core に寄せる（下記 ArkTS/Kotlin 節）。

## モジュール対応表と状態
| Rust | 元 | 状態 |
|---|---|---|
| text.rs | desktop/core/text.cpp (130行) | 移植中 (担当A) |
| romaji.rs | desktop/core/romaji.cpp (165行) + tables.h | 移植中 (担当A) |
| datetime.rs | desktop/core/datetime.cpp (549行) / DateTimePredictor.ets | 移植中 (担当B) |
| number.rs | NumberFormatter.ets/.kt + datetime.cpp の number_forms | 移植中 (担当B) |
| brackets.rs | Brackets.ets/.kt | 移植中 (担当B) |
| pos.rs | Pos.ets/.kt, pos_classes.h | 移植中 (担当B) |
| flow.rs | desktop/core/flow.cpp (264行) / SentenceFlow | 移植中 (担当B) |
| typo_alts.rs | desktop/core/typo_alts.cpp (119行) | 移植中 (担当C) |
| layout.rs | desktop/core/layout.cpp (709行) | 移植中 (担当C) |
| style.rs | desktop/core/style.cpp (185行) | 移植中 (担当C) |
| settings.rs | desktop/core/settings.cpp (183行) | 移植中 (担当C) |
| tables.rs | desktop/core/tables.h (279行) | 移植中 (担当C) |
| engine.rs (+ffi) | entry/src/main/cpp/engine.cpp (4902行), ai_learn.h, phrase_ends.h, engine_simd.h | 移植中 (担当D・本丸) |
| composer.rs | desktop/core/composer.cpp (2306行) | 移植中 (担当E) |
| store.rs | desktop/core/store.cpp (1164行) | 移植中 (担当E) |
| converter.rs | desktop/core/converter.cpp (718行) | 移植中 (担当E) |
| glide.rs (+ffi) | entry/src/main/cpp/glide.cpp (595行), glide_nn.cpp (175行) | 移植中 (担当F) |

## ArkTS / Kotlin ロジックの扱い
HarmonyOSのNAPI型定義 `entry/src/main/cpp/types/libkkc/index.d.ts` が示すとおり、変換本体は既に
ネイティブ側（→今回Rust化）にあり、ArkTS側 `ime/*.ets` の残りは次の3種に分ける:
1. Rust化済み機能の呼び出しに置換できるもの: JapaneseConverter.ets（ローマ字→かな＝rext-core::romaji）、
   KanaKanjiConverter.ets（変換本体＝kkc ABI）、NumberFormatter/DateTimePredictor/Brackets/Pos/SentenceFlow
   （＝rext-core の各モジュール。NAPIに小物関数の輸出を追加すれば呼べる）。
2. 端末内永続化＋ロジック混在: SettingsStore/UserDictStore/UnknownWordLearner/InputLog/Learning系。
   ロジック部分は rext-core::store に対応物があるが、保存先が各OSのファイルAPIに依存するため、
   呼び出し置換にはNAPI側の追加輸出が要る（未実施・理由は本節）。
3. UI制御・OS API依存で残置: KeyboardController.ets (5169行、入力メソッドフレームワークとの制御が本体)、
   AiConverter.ets（推論資産の読み込みとOS連携を含む）、NextWordPredictor/PredictiveConversion等の予測系
   （rext-core::engine の complete/romance と接続する追加NAPIが要るため次段階）。
Android Kotlin も同型: Engine.kt は既にJNI越しに kkc ABI を呼んでおり、Rust製 .so にそのまま差し替わる。
InputHandler.kt / KkcIme.kt は Android InputMethodService 依存のため残置し、上記1.の小物ロジックは
JNI輸出の追加が次段階。

## テスト結果
（各担当の実測をここへ集約する）

## 統合検証（2026-11-27、コーディネータ実測）
- `cargo test`（ワークスペース全体）: **139 passed / 0 failed**
  内訳: text 8, romaji 15, datetime 12, number 9, brackets 4, pos 5, flow 10,
  layout 9, settings 8, typo_alts 8, style 6, tables 3, engine 10, glide 10,
  composer 10, converter 5, store 7、その他（lib等）
  ※ 途中、他担当の隔離検証で flow/settings の書込み系テストが失敗した件は /tmp (512MB tmpfs) 満杯が
  原因と特定し、掃除後の全体実行で全件合格を確認した。
- `cargo build --release`: 成功。`librext_kkc.so` 1.5MB。
- 輸出シンボル: `nm -D` で **kkc_* 52本**。engine.h が宣言する関数52本と完全一致
  （engine 由来47 + glide 由来5: kkc_glide_prepare/decode/set_words/set_net, kkc_zh_candidates）。

## モジュール別 完了状態（2026-11-27 時点）
完了（実処理＋テスト）: text, romaji, datetime, number, brackets, pos, flow, typo_alts,
layout, style, settings, tables, engine, glide, composer, store, converter（全17モジュールに実装あり、
合計 約21,500行のRust）。

## 残っているもの（ファイル単位・正直な一覧）
1. glide.rs: mode 0（かな）のFFI経路はエンジン辞書の読み木に未接続（内部フック
   `ffi::engine_lexicon_words` が None を返す間は 0 を返す）。mode 1/2・拼音・NNは動作。
   engine.rs 側に辞書読み一覧の公開APIを足して接続するのが次作業。
2. engine.rs の既知の未移植・差異:
   - タイポエッジの「2語で覆う組」の生成
   - latin_edges の mid2 チャンク連結（minecraft+あぷり型）
   - date_candidates の時刻がUTC計算（C++はlocaltime。端末TZで日付候補がずれうる）
   - gelu の erf 近似（誤差<1.2e-7、スコア末桁差の可能性）
   - kkc_set_threads は受理のみ（計算は単一スレッド、結果同一・速度のみ差）
   - ai_learn.h / phrase_ends.h のヘッダ内関数（kkc_* ABI外）は未実装
3. composer.rs: prime_alts（layout/typo_alts接続）、update_live の pre_r 使い回し最適化、
   type_kana の U+E000/E001 の拗・修の完全対応、Learning への lid resolver 自動配線。
   また composer 内に暫定の SimpleRomaji を内蔵しており、romaji.rs への一本化が残る。
4. flow.rs: forget_all / forget_recent は store 側の学習データ操作と接続が必要（未接続）。
   recent_add はコールバック受け口のみ。
5. ArkTS/Kotlin の呼び出し置換: napi_kkc.cpp / jni.cpp 経由でエンジン本体はRust .so に
   差し替わる構成まで完了。ただし ArkTS ime/*.ets・Kotlin の小物ロジック
   （JapaneseConverter, NumberFormatter 等）を rext-core 呼び出しに置き換えるには
   NAPI/JNI への小物関数の追加輸出が別途必要で未実施。KeyboardController.ets、
   InputHandler.kt 等のUI制御・OS API依存は残置（冒頭の方針どおり）。
6. desktop のOS固有シェル（desktop/windows, desktop/linux のC++）は残置。
   desktop/core のロジックは全て rext-core に移植済みだが、シェル側からのリンク切替は未実施。
7. 既存テストとの突き合わせ: desktop/tests/core_test.cpp のゴールデン（romaji の
   to_romaji 等）は一部をユニットテストに取り込み済み。全量の自動突き合わせ
   （android golden.json、tools/ime-eval 一式）は未実施。

## 第2弾（残件解消、2026-11-27）

### 1. glide mode 0 FFI接続（解消）
- engine.rs: `Lex::glide_words(mode)` と公開API `kkc_engine::glide_lexicon_words()` を追加。
  mode 0 は C++ `glide_build` と同条件（ひらがな・ーのみ、長さ1〜12、最小コスト≦7000）で読みを列挙。
  mode 1 は latin_words、mode 2 は C++ 同様 None（TSV必須）。
- glide.rs: `ffi::engine_lexicon_words` を実接続。テスト: 小辞書で mode0=8語、実エンジンFFIで
  QWERTYなぞり "ashita" → 1位「あした」を確認。

### 2. engine細部（全項目解消）
- (a) タイポエッジの2語カバー組: C++ の2語ブロックを移植（分割点・安い6件からの組生成・最良パス選択・bloom打ち切り）。
- (b) latin_edges の mid2 チャンク連結: C++ 第2ブロック全体を移植（最大3チャンク延長、lone-n未確定扱い等を含む）。
- (c) date_candidates のローカルタイム化: TZ解釈を依存クレートなしで実装（TZ環境変数→無ければ /etc/localtime、
  TZif v1/v2+ と POSIX TZ 文字列の両対応）。UTC分解16箇所を全て置換。テストはTZ固定で検証
  （UTC/JST-9/Asia-Tokyo/EST5/EDT規則で時刻と日付のずれを確認）。近似: TZif の最初の遷移より前は最初の標準時種別で処理。
- (d) gelu/erf: 元C++スカラー版は std::erf、AVX2版は A&S 7.1.26（誤差1.5e-7）。Rust側は engine/glide とも
  不完全ガンマ級数＋連分数の実装（~1e-14）に置換し std::erf に実質一致させた。

### 3. rext_* 追加輸出（解消）
- `crates/rext-kkc/src/rext_exports.rs` 新規: C ABI **rext_* 25本**（解放1、ローマ字2、テキスト変換5、
  数値6、日時2、括弧6、品詞3）。戻り文字列は `rext_free_string` で解放、候補リストは '\n' 区切り、NULL安全。
- 使い方: `crates/rext-kkc/REXT_EXPORTS.md`（関数一覧・規約・NAPI/JNI経由の呼び出し手順）。
- ホスト検証: 単体テスト7件合格、Cプログラムから cdylib 直接リンクで 30/30 合格。
  HarmonyOS/Android実機ビルドはこの環境では未実施（検証範囲はホストまで）。
- 未輸出: Romaji の状態あり操作（ハンドル型ABIが別途必要）、pos::words/head_lid（構造体配列で文字列規約外）、
  flow::SentenceFlow（状態あり＋保存先OS依存）。

### 4. 既存テストとの突き合わせ（仕組み作成＋実測）
- `crates/rext-core/tests/golden_match.rs` 新規（統合テスト8本、golden.json を実行時に読む）。
  詳細: `crates/rext-core/tests/GOLDEN_MATCH_REPORT.md`。
- 実測 506チェック: 完全一致 377件（74.5%）。内訳: numbers 26/26、core_test由来 115/115、
  romaji 155/158打鍵（98.1%）、dates 完全一致 81/207（39.1%）。
- dates の不一致126件は値の誤り0件: golden候補は全件・同順序でRust側に含まれる（包含では 207/207）。
  差は候補数の仕様差（HarmonyOS版 日付5形・時刻3形 vs Rust=desktop由来 12形・4形）。どちらに揃えるかは仕様判断待ち。
- romaji の不一致3打鍵は `q`/`qa` のみ: HarmonyOS版は表に無いキーを即確定、Rust(desktop表)は `qa→くぁ` で pending 保持。これも仕様差。
- 対象外: core_test のエンジン必須部分 約65件（kkc_lex.bin/kkc_model.bin がリポジトリ内に無い）、
  同梱レイアウト部分 約30件（材料はあるが未自動化＝次作業候補）。
- ime-eval: run_number.js / run_datetime.js は ALL PASS（対象は元ArkTS実装）。エンジン系32本は
  辞書 rawfile/{dict,global_dict}.json 不在と相対import解決不可で実行不能。

### 第2弾 統合検証（コーディネータ実測）
- `cargo test`（全体、3回連続実行とも）: **161 passed / 0 failed**
  内訳: rext-core 146（第1弾139＋第2弾の新規7）、golden_match 8、rext-kkc 7。
  ※ 作業中に golden_romaji の間欠失敗が1回だけ報告された（core側 HashMap 走査順依存の疑い）。
  統合検証の3回連続では再現しなかったが、根治はしていないので留意。
- `cargo build --release`: 成功。`librext_kkc.so` 1.7MB（rext_* 追加で第1弾の1.5MBから増加）。
- 輸出シンボル: kkc_* **52本**（不変）＋ rext_* **25本**。

### 第2弾後の残件（更新）
- 上記「残っているもの」の 1（glide接続）と 2（engine細部4点）は解消。5 は輸出まで完了し、
  .ets/.kt 側の呼び出し置換と実機ビルドが残る。7 は上記のとおり実施済み（仕様差2件と対象外2件が残る）。
- 引き続き残るもの: 2 の kkc_set_threads（受理のみ）・ai_learn.h/phrase_ends.h のヘッダ内関数、
  3（composer の prime_alts 等と SimpleRomaji 一本化）、4（flow の forget 系の store 接続）、
  6（desktopシェル側のリンク切替）、dates候補数と romaji `q` の仕様差の判断、core_test レイアウト約30件の自動化。

## 親エージェントによる再検証と修正（2026-11-27 22:16）
- 再実行では当初 `cargo test --release` が 138 passed / 1 failed だった（統合報告の139/0と相違）。
  失敗は `composer::tests::romaji_basic`: SimpleRomaji の逆引きが「ねこ」→"neco"（こ の綴りが
  同長の別綴り "co" と HashMap 走査順で競合）。romaji.rs の REVERSE_PREFER と同方式で
  標準綴りを明示優先するよう composer.rs を修正。
- 修正後の実測: `cargo test` **139 passed / 0 failed**、`cargo build --release` 成功、
  `target/release/librext_kkc.so` 1.5MB、輸出 kkc_* 52本を再確認。

## 仕様差をHarmonyOS版へ統一（2026-11-27、ユーザー決定「現行のHarmonyOS版に揃える」）

第2弾で仕様判断待ちだった2件を、HarmonyOS版（golden.json = HarmonyOS版ロジックの
生成物、および entry/src/main/ets/ime/ のソース）を正として rext-core 側を変更した。
変更は rext-core のみ。.ets / 元C++ は無変更。元のGitHubへはpushしていない。

### 1. dates（日時候補）: datetime.rs の生成規則を変更
- `date_forms`: desktop由来の12形 → HarmonyOS版の規則に変更。並びは
  [西暦 年月日, 西暦 年月日(曜日), 月日, 西暦 Y/M/D, 和暦 年月日（令和開始前は無し＝4形）]。
  出さなくなった形: 月日(曜日)、月日(曜日名)、月/日、月/日(曜日)、年-月-日、数字8桁、
  和暦(曜日)。単なる件数切り詰めではなく生成リスト自体をHarmonyOS版の規則に差し替えた。
  ※現行 .ets の dateForms は後からPC版と同内容（12形）に変わっているが、golden.json と
  ユーザー決定が示すHarmonyOS版の候補仕様（5形）を正とした。
- `time_forms`: 4形 → 3形 [H:MM, H時M分, 午前/午後h時M分]。「M月D日 H:MM」を廃止。
- 影響で更新したテスト（根拠コメント付き）: datetime.rs の today_forms / time_forms_now、
  golden_match.rs の core_test_datetime（きょう の期待値を5形に変更、件数チェック 12→5、
  いま[3] を削除。desktop/tests/core_test.cpp の12形期待値とは意図的に diverge する）、
  rext-kkc rext_exports の きょう 末尾候補（"20261021"→"令和8年10月21日"）。
- 実測: golden dates **81/207 → 207/207 完全一致**（部分一致も207/207）。

### 2. romaji の `q`: romaji.rs の表を変更
- PC_EXTRA から q 行（qa→くぁ, qi→くぃ, qu→く, qe→くぇ, qo→くぉ）を削除。
  HarmonyOS版 JapaneseConverter.ets の表にも q 行は無く、これで 'q' は接頭辞集合にも
  入らなくなるため、process_buffered のフォールバック（表に無いキーを打ったまま確定）
  がHarmonyOS版と同じく効く: q → committed "q"、qa → "q" 確定済み + "あ"。
- 実測: golden romaji **155/158 → 158/158 打鍵完全一致**、語単位 **31/33 → 33/33**。
  golden_match.rs 側の固定値も「全件一致」に更新した。
- 対象外として残したもの: composer.rs 内蔵 SimpleRomaji の表（desktop由来の別実装で
  golden の対象外。今回の決定は romaji.rs に適用）。

### 統合検証（実測）
- golden_match 合計チェック: 498件（core_test datetime の期待値整理で 506→498）全件一致。
  内訳: numbers 26/26、dates 207/207、romaji 158/158＋語33/33、core_test由来 107/107。
- `cargo test`（ワークスペース全体）: **161 passed / 0 failed**（変更前と同数。3回連続実行とも）。
- `cargo build --release`: 成功。librext_kkc.so 1.7MB。
- golden_romaji の間欠失敗: 本作業では再現しなかった（変更前1回＋変更後3回連続の
  全体実行でいずれも全件合格）。根治確認ではないため留意は継続する。

## datesを現行.ets（12形）へ再統一（2026-11-28、ユーザー再決定）

経緯（正直な記録）: 上記「仕様差をHarmonyOS版へ統一」節で dates を 5 形へ寄せた
根拠は「golden.json = HarmonyOS版ロジックの生成物であり 5 形が現行HarmonyOS版の
仕様」という理解だった。しかし現行 `entry/src/main/ets/ime/DateTimePredictor.ets`
の `dateForms` は後から PC 版と同内容（基本9形＋和暦2形＋数字8桁 = 最大12形、
`timeForms` は4形）に更新されており、5 形は golden.json 生成時点の古い
HarmonyOSロジックだった。つまり 5 形化は「現行 .ets に揃えた」のではなく
「古い golden に揃えた」もので、現行 .ets からは離れていた。この誤認を報告し、
ユーザーが「現行 .ets（12形）に揃え直す」と再決定したため、本節で巻き戻した。
romaji の `q` は現行 .ets（JapaneseConverter.ets）の表にも q 行が無いことを
再確認済みのため、5形化と同時に行った変更をそのまま維持した（対象外）。

変更内容（rext-core 側のみ。.ets / 元C++ / golden.json は無変更、GitHubへ未push）:
- `crates/rext-core/src/datetime.rs`:
  - `date_forms` を現行 .ets `dateForms` の規則に一致させた: 基本9形
    [年月日, 年月日(曜日), 月日, 月日(曜日), 月日(曜日名), 月/日, 月/日(曜日),
    Y/M/D, Y-M-D] + 和暦2形 [和暦年月日, 和暦年月日(曜日)]（令和開始前は無し）
    + 数字8桁（月・日0詰め）= 最大12形。desktop/core/datetime.cpp の
    date_forms とも同内容であることを確認済み。
  - `time_forms` を現行 .ets `timeForms` の4形に戻した:
    [H:MM, H時M分, 午前/午後h時M分, M月D日 H:MM]。
  - 単体テスト today_forms / time_forms_now の期待値を12形・4形に復元。
- `crates/rext-core/tests/golden_match.rs`:
  - `core_test_datetime`: きょう の期待値を core_test.cpp どおり12形に復元
    （index 0〜11 + 件数チェック 5→12）、「いま[3] = 10月17日 14:05」を復活。
  - `golden_dates`: golden.json は古い5形ロジックの生成物であり現行 .ets が
    正であるため、完全一致は要求しない形に変更。合格条件は「包含」
    （ゴールデンの候補が全件・同順序のまま部分列として Rust の候補に
    含まれること）。完全一致の担保は現行 .ets / core_test.cpp と同内容の
    core_test_datetime と datetime.rs 単体テストで行う旨をコメントに明記。
- `crates/rext-kkc/src/rext_exports.rs`: きょう の末尾候補の期待値を
  数字8桁 "20261021" に戻した。

実測（変更前 → 変更後）:
- golden dates 完全一致: **207/207 → 81/207（39.1%）**（想定内。golden が古い
  5形生成物のため下がるのが正しい状態）
- golden dates 包含（同順序部分列）: **207/207**（変更前後とも。集合包含も 207/207）
- core_test_datetime（現行 .ets 由来の完全一致チェック）: **51/51**
  （5形前提に整理していた 43 件から、12形の期待値復元で 51 件に増加）
- golden numbers 26/26、golden romaji 打鍵 158/158・語 33/33（いずれも不変）
- `cargo test`（ワークスペース全体）: **161 passed / 0 failed**
- `cargo build --release`: 成功。librext_kkc.so 1.7MB。
- 詳細は crates/rext-core/tests/GOLDEN_MATCH_REPORT.md の 2026-11-28 追記にも記録。

## 第3弾「配線」（2026-11-28）

各OSのビルドを C++エンジン→Rustライブラリへ切り替える配線と、ArkTS/Kotlin 小物ロジックの
rext_* 呼び出しへの置き換えを実施。詳細は rust/phase3/ の担当別記録
（A-harmonyos.md / B-android.md / C-desktop.md / D-bridges.md）。元のGitHubへは未push。
環境の線引き: この環境には DevEco Studio・HarmonyOS SDK・Android SDK/NDK・Fcitx5開発
ファイルが無い。実機ビルドと .ets/.kt のコンパイルは未検証（下記に明記）。

### A. HarmonyOS配線
- entry/src/main/cpp/CMakeLists.txt: add_library(kkc SHARED napi_kkc.cpp) のみに変更し、
  engine.cpp / glide.cpp / glide_nn.cpp のコンパイルを停止（ファイルは残置）。napi_kkc.cpp は橋渡しとして残す。
- entry/src/main/cpp/rext_kkc.cmake（新規）: Rust .so の探索・ELFアーキテクチャ検査・リンク・
  出力先コピーを担当。方式は (a) 事前ビルド済み .so の取り込みを既定に採用
  （hvigor は CMake を回すだけで Rust ツールチェーンを保証せず、cargo カスタムコマンドは
  DevEco 機に rustup/ohosターゲット/リンカ設定まで要求して壊れやすいため）。
  (b) は REXT_KKC_BUILD_WITH_CARGO=ON のオプションとして実装（既定OFF）。
  パス・ターゲットは -D > 環境変数 > 既定 の優先順で上書き可能。
  DT_NEEDED に絶対パスが埋まる問題（実測）への対策として -L + -lrext_kkc でリンク。
- 検証: rustup target add aarch64-unknown-linux-ohos 成功、rustc コンパイルは全クレート通過。
  ただし ohos向け最終リンクは OHOS NDK リンカが無く失敗（.so 生成は未達、リンカ指定が別作業）。
  CMake ハーネス検証（ダミーライブラリ）は全6ケース合格。DevEco/hvigor 実ビルド・実機は未検証。

### B. Android配線
- android/app/build.gradle.kts: buildKkc を2タスクに再構成。
  buildRustKkc（新規）: cargo build --release --target aarch64-linux-android -p rext-kkc を実行し
  librext_kkc.so を jniLibs へコピー（リンカは Gradle が知る NDK clang のパスを環境変数で渡す）。
  buildKkc（変更）: jni.cpp のみのコンパイルにし、-lrext_kkc リンク + --no-undefined を追加。
  構成は2層（librext_kkc.so + 薄い libkkc.so）。Engine.kt は loadLibrary("kkc") のみで、
  DT_NEEDED 経由で依存が解決されるため変更不要。
- JNIシンボル照合（nm 実測）: engine.h 宣言52本と Rust 輸出は 52/52 完全一致。
  jni.cpp が必要とする kkc_* 25本は全て存在（ズレなし、jni.cpp の修正不要）。
  JNI 関数19本と Engine.kt の external fun 19本も1対1で対応。
- 検証: aarch64-linux-android ターゲット追加と Rust コードのコンパイルは確認。
  リンクは NDK sysroot が無く失敗（NDK clang をリンカにする設計の根拠）。
  Gradle 実ビルドは Java/SDK/NDK が無く未検証。

### C. デスクトップ配線
- desktop/linux/CMakeLists.txt（全面改訂）: core の .cpp 11本 + engine.cpp のコンパイルを廃止し、
  CMake カスタムコマンドで cargo build -p rext-kkc を呼んでリンクする方式に切替
  （事前ビルド済みバイナリがリポジトリに無いため再現性優先）。SONAME が無いと DT_NEEDED に
  相対パスが残る問題への対策として RUSTFLAGS で soname を付与。
  find_package(Fcitx5Core) は QUIET 化し、無い場合はホスト検証のみビルドする分岐。
- desktop/linux/tests/rext_host_check.cpp（新規）: Fcitx 非依存のホスト検証。
  実測: cmake 構成成功 → rext_host_check **54 checks, 0 failures**（romaji/text/datetime 12形/
  number/brackets/pos/NULL安全。日時は現行 .ets と同じ12形を確認）→ ctest 1/1 Passed。
  コーディネータも再実行して 54/54 を確認済み。
- desktop/windows/build.bat: REXT_RUST スイッチ（既定0）を追加し、=1 で core/engine を外して
  Rust ライブラリをリンクする配線を実装（settings_app 等は対象外と明示）。
  既定を切り替えなかった理由: MSVC 未検証 + 状態あり ABI 不在で、唯一動く Windows ビルドを壊すため。
- 対応表の結論: エンジン本体（kkc_*）と無状態の小物（rext_*）は対応あり。
  Composer/Converter/Learning/UserDict/SentenceFlow/Layout/Settings 等の状態ありクラスには
  C ABI が無く、現行シェル（shunti.cpp / text_service.cpp）はリンク切替だけではリンクできない。
  シェルの書き換えにはハンドル型 ABI の設計が前提で次段階。

### D. ArkTS/Kotlin 小物ロジックの置き換え
- napi_kkc.cpp: rext_* ラッパ24本を既存パターンで追加し登録。index.d.ts に型を末尾追加のみ（既存型は無変更）。
  設計: rext_* 用Cヘッダが無く現行ビルドが Rust をリンクしないため、追加分は weak 宣言とし、
  未リンク時は空を返して .ets 側の純実装フォールバックに落ちる（モジュール読み込みは壊れない）。
- jni.cpp: Engine 用 JNI は無変更のまま、末尾に rext ブリッジ16本（Number/DateTime/Brackets/Pos）を追加。
- .ets/.kt: NumberFormatter・Brackets・Pos・DateTimePredictor をネイティブ先行 + 従来実装を
  predictPure 等として残置するフォールバック方式に置き換え。
  SentenceFlow は状態あり + 保存先OS依存で輸出が無いため置き換えなし（理由を明記）。
- ホスト検証: モックヘッダで両 .cpp のコンパイル成功。weak 機構の実動テストで
  number_predict / era_year / datetime（現行 .ets と同じ12形・同じ並び）/ pos_class_count=2672 等を確認。
- 未検証: .ets/.kt のコンパイル（SDK・kotlinc 無し）と実機での weak 解決・候補表示。
  注意: Kotlin の DateTimePredictor はネイティブ有効時に候補が従来の純実装より増える
  （HarmonyOS版=12形の移植のため）。戻したい場合は predictPure を正にすれば従来挙動になる。
  Android のローマ字・text系7本は受け手（JapaneseConverter.kt）が担当範囲外のため未ブリッジ。

### 第3弾 統合検証（コーディネータ実測）
- cargo test（全体）: **161 passed / 0 failed**（146 + golden 8 + rext-kkc 7。第1・2弾から無傷）
- cargo build --release: 成功。librext_kkc.so 1.7MB。輸出 kkc_* 52本 + rext_* 25本（不変）。
- desktop/linux の rext_host_check を再実行し 54 checks, 0 failures を確認。

### 第3弾後の残件
- 実環境での確認: DevEco/hvigor ビルドと HAP 同梱、Gradle 実ビルドと APK 同梱・実機ロード、
  Fcitx5 実ビルド、Windows 実ビルド（いずれもこの環境に SDK/ツールチェーンが無いため未検証）。
- ohos / android ターゲット向け Rust .so の最終リンク（NDK リンカ指定が要る）。
- 状態ありクラス（Composer 等）向けハンドル型 C ABI の設計と、デスクトップシェルの書き換え。
- .ets/.kt のコンパイル確認と実機確認、Android の JapaneseConverter.kt への橋渡し。
