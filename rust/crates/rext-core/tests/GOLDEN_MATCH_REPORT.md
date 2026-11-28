# ゴールデン突き合わせ レポート (担当C・2026-11-27 実測)

RUST_PORT_STATUS「残っているもの」7 の自動化。仕組みは新規統合テスト
`rust/crates/rext-core/tests/golden_match.rs`（既存 .rs は無変更）。
実行: `cargo test -p rext-core --test golden_match -- --nocapture`
（不一致一覧は標準出力に全件出る）。

## 突き合わせの仕組み

- `android/app/src/test/resources/golden.json` を**実行時に読む**（埋め込みコピーではない）。
  これは HarmonyOS 版ロジックを Node で実行して生成した正解で、Kotlin
  `GoldenTest.kt` が NumberFormatter.predict / JapaneseConverter.processKey /
  DateTimePredictor.predict と突き合わせているもの。同じ入力を rext-core の
  公開 API（`number::predict` / `romaji::Romaji::process` / `datetime::datetime_predict`）
  に通して比較した。日時候補の `now` は golden の [年,月,日,時,分] を
  JST (UTC+9) の UNIX 時刻に変換して渡した（Rust 側既定オフセットが +9 のため）。
- `desktop/tests/core_test.cpp` のゴールデンは、エンジン辞書なしで検証できる
  節（romaji / text / datetime / number_forms / settings・shortcuts / layout chord）
  の期待値ペアをテスト内に転記して突き合わせた。

## 実測結果（チェック単位）

| 対象 | 総件数 | 完全一致 | 一致率 | 備考 |
|---|---|---|---|---|
| golden numbers | 26 | 26 | 100% | 候補リスト全体（順序込み）で一致 |
| golden romaji（打鍵単位） | 158 | 155 | 98.1% | 語単位では 33 語中 31 語（93.9%） |
| golden dates（完全一致） | 207 | 81 | 39.1% | 下記「原因切り分け」参照 |
| golden dates（部分一致: golden ⊆ Rust） | 207 | 207 | 100% | golden の候補が欠けた件は 0 |
| core_test romaji/text | 10 | 10 | 100% | to_romaji・半角カナ含む |
| core_test datetime | 51 | 51 | 100% | 2026-10-17 14:05 固定、候補の index 指定まで一致 |
| core_test number_forms | 16 | 16 | 100% | 丸数字・万混ぜ書き・拒否ケース含む |
| core_test settings/shortcuts | 18 | 18 | 100% | typo/ctx 保存読直し + 既定ショートカット 17 件 |
| core_test layout chord | 20 | 20 | 100% | 同時押し・親指・連続シフト等 |
| **合計** | **506** | **377** | **74.5%** | dates を部分一致で数えると 503/506 = 99.4% |

テスト結果: `cargo test -p rext-core --test golden_match` = **8 passed / 0 failed**
（初回実測時。後述の他担当の編集中エラーにより、現時点の再実行は lib の
コンパイルで止まる）。

## 不一致の内訳と原因切り分け

### 1. golden dates: 126 件が「完全一致でない」— 候補の出し方の差（値の誤りではない）
- 内訳: 日付読み 99 件（11 読み × 9 時点）+ 時刻読み 27 件（3 読み × 9 時点）。
  年読み 63 件と空・非該当 18 件は完全一致。
- 原因: HarmonyOS 版 DateTimePredictor は日付で 5 形・時刻で 3 形しか返さないが、
  Rust `datetime` は desktop 版 `date_forms` 由来の 12 形・4 形を返す
  （例: きょう → golden 5 件に対し Rust は「3月1日(日)」「3/1」「2020-3-1」
  「20200301」等を追加で返す。時刻は「3月1日 1:00」が追加）。
  golden の候補は**全件・同順序のまま Rust の候補に含まれており**（部分一致
  207/207）、日付・曜日・和暦の値そのものに誤りは 1 件もなかった。
  「どちらに揃えるか」は仕様判断（候補を絞るか否か）であり、移植ミスではない。

### 2. golden romaji: 3 打鍵が不一致 — 'q' の扱いの仕様差
- `q`（1 打鍵目）: golden は表に無いキーをそのまま確定（committed="q", pending=""）。
  Rust は 'q' を pending に保持する。
- `qa`（2 打鍵目）: golden は "q" 確定済み + "あ"。Rust は desktop 由来の表に
  `qa → くぁ` があるため "くぁ" を確定する。
- それ以外の 155 打鍵（拗音・促音・撥音・外来語表記・大文字混在含む）は完全一致。
  テストでは「不一致はこの 3 件だけ」を固定し、それ以外の退行で落とすようにした。

## 対象外（ホスト検証不能・未自動化）

- **core_test.cpp のエンジン必須部分（約 65 件規模）**: `main` 後半の
  Converter/Composer/Learning/AI 学習の全チェックは引数 `kkc_lex.bin` /
  `kkc_model.bin` が必要だが、この 2 ファイルはリポジトリ内に存在しない
  （`entry/src/main/resources/rawfile/` にあるのは kkc_register.bin 等のみ）。
  よってホストでは突き合わせ不能。辞書・モデルが用意されれば converter の
  公開 API で追加可能。
- **core_test.cpp の同梱レイアウト部分（約 30 件規模）**: `desktop/layouts/*.tsv`
  の全件読込確認、AZIK / ACT / DvorakJP / SKY / gACT10 の romaji_pairs 経由の
  変換、前置シフト、letters の Shift 行。ファイルも Rust API（Layout /
  Romaji::add_pair）も揃っているので検証可能だが、今回は未自動化（次作業候補）。
- golden.json 自体に対象外はない（numbers 26 + dates 207 + romaji 33 語 =
  全 266 ケースを投入した）。

## tools/ime-eval の扱い（Node v24.22.1 で実測）

前提: スクリプト群は `npx tsc` で ArkTS をトランスパイルする方式。
素の状態では `npx tsc` が無関係な `tsc@2.0.4` パッケージを掴んで失敗する。
`/tmp/tsproj` に `typescript@5` を入れ、その cwd から実行すると動いた
（リポジトリには何も入れていない）。

- **実行できた（対象は元の ArkTS 実装そのもの。Rust の突き合わせではない）**:
  - `run_number.js` → **ALL PASS**
  - `run_datetime.js` → **ALL PASS**
  - これは「golden の生成元が今のホストでも再現する」確認にはなるが、
    Rust 実装の検証は上記 golden_match が担う。
- **実行不能（エンジン系 run.js / run_test*.js / run_vocab*.js / run_all.js 等 32 本)**:
  1. `KanaKanjiConverter.ets` の単体トランスパイルでは相対 import
     （`./AiConverter` 等）が解決できず `Cannot find module './AiConverter'` で停止。
  2. 仮に解決しても、辞書 `entry/src/main/resources/rawfile/{dict,global_dict}.json`
     がリポジトリに存在せず、変換器を開けない。
  よって変換精度系の評価（TRAIN/TEST/VOCAB コーパス）はこのホスト・この
  リポジトリ状態では実行不能。Rust 側にも対応する辞書資産が無いため、
  かな→漢字変換の精度突き合わせは今回の対象外。

## 他担当由来のブロッカー（切り分け済み）

- 本レポート作成時点の再実行では `cargo test`（ワークスペース全体・
  golden_match 単体とも）が **lib のコンパイルエラー**で止まる:
  `crates/rext-core/src/engine.rs:2861` で `u16` に `is_ascii_alphabetic()`
  を呼んでいる（E0599）。engine.rs は担当Dが並行編集中のファイルで、
  本担当の変更（tests/ 配下の新規ファイルのみ）とは無関係。
  エラー発生直前の実行では golden_match は 8/8 合格しており、上記の
  実測値はその実行で得たもの。engine.rs 側の修正が入れば再実行できる。

## 追記 (2026-11-28 再実測: dates を現行 .ets へ再統一)

経緯: 本レポートの後、2026-11-27 に「HarmonyOS 版 = golden の 5 形」と誤認して
Rust を 5 形へ寄せた (dates 完全一致 207/207 になった) が、現行
`entry/src/main/ets/ime/DateTimePredictor.ets` の dateForms は実際には 12 形
(PC 版と同内容) であることが判明。ユーザーが「現行 .ets (12 形) に揃え直す」
と再決定し、Rust を現行 .ets の規則に戻した。golden.json は古い 5 形ロジックの
生成物なので、dates の完全一致が下がるのは正しい状態であり、golden_dates は
包含 (同順序部分列) を合格条件に変更した。完全一致の担保は現行 .ets /
core_test.cpp と同内容の core_test_datetime で行う。

再実測 (2026-11-28、`cargo test --test golden_match -- --nocapture`):
- golden dates: 完全一致 **81/207 (39.1%)**、包含 (同順序部分列) **207/207**
- golden numbers: 26/26、golden romaji: 打鍵 **158/158**・語 33/33 (q 統一後)
- core_test datetime: **51/51** (12 形の期待値を復元した総数)
- `cargo test` 全体: **161 passed / 0 failed**、`cargo build --release`: 成功
