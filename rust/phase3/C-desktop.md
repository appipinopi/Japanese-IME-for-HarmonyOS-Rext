# 第3弾 C. デスクトップ配線（2026-11-28）

担当範囲: `desktop/linux/CMakeLists.txt`、`desktop/windows/build.bat`、および desktop 配下の新規ファイル。
他の領域（entry/、android/、rust/ 本体）は変更していない。元の GitHub へは push していない。
前提環境: cmake 3.28.3 / g++ 13.3.0 / cargo・rustc 1.100.0。Fcitx5 の開発ファイル
（Fcitx5Core の CMake config）はホストに無い。apt 等でのインストールはしていない。

## 変更ファイル

| ファイル | 変更 |
|---|---|
| `desktop/linux/CMakeLists.txt` | 全面改訂（下記）。core の .cpp 11本＋engine.cpp のコンパイルを廃止し、cargo でビルドした librext_kkc をリンクする形へ切替 |
| `desktop/linux/tests/rext_host_check.cpp` | **新規**。Fcitx5 非依存のホスト検証プログラム（54チェック） |
| `desktop/windows/build.bat` | `REXT_RUST` スイッチ（既定 0）を追加し、メイン IME DLL の切替配線を実装（下記）。CRLF は保持 |
| `rust/phase3/C-desktop.md` | 本ファイル（新規） |

`rust/RUST_PORT_STATUS.md` 本体は編集していない。

## 1. Linux: CMakeLists の切替

### Rust の取り込み方（選択と理由）
- **採用: CMake のカスタムコマンドで `cargo build -p rext-kkc` を呼ぶ方式。**
  - 理由1（再現性）: 事前ビルド済みバイナリはリポジトリに存在しない。取り込み方式では
    ソースから作れない環境が必ず出る。
  - 理由2（簡潔さ）: ExternalProject の superbuild は一段増えるだけで得がない。
    カスタムコマンド＋IMPORTED ライブラリで足りる。
  - `CMAKE_BUILD_TYPE=Release` のとき `--release`、それ以外は debug プロファイル。
  - 依存には rust ソース一式（glob）と各 Cargo.toml と CMakeLists 自身を列挙し、
    変更時に再ビルドされるようにした。
- **SONAME の付与**: cargo の cdylib は SONAME を付けないため、そのままでは消費側の
  DT_NEEDED にリンク時の相対パス（`rust/librext_kkc.so`）が残り、RPATH が効かず
  実行時に見つからない（実測で確認）。対策として cargo 呼び出しに
  `RUSTFLAGS=-C link-arg=-Wl,-soname,librext_kkc.so` を付け、ホスト検証には
  `BUILD_RPATH` も設定した。対策後は DT_NEEDED = `librext_kkc.so` になる（実測）。

### Fcitx5 の扱い
- 旧構成は `find_package(Fcitx5Core REQUIRED)` で、開発ファイルが無い環境では
  構成そのものが止まっていた（ベースライン実測: 下記）。
- 新構成は `find_package(Fcitx5Core QUIET)` とし、無い場合は shunti 本体を
  スキップしてホスト検証だけをビルドする。従来どおり構成を失敗させたい場合は
  `-DSHUNTI_REQUIRE_FCITX=ON` で旧挙動に戻る。
- Fcitx5 がある場合の shunti ターゲットは、ソースが `src/shunti.cpp` のみ、
  リンクは `Fcitx5::Core` + `Threads::Threads` + `rext_kkc`。
  旧構成の `${CORE}/*.cpp`（text/romaji/store/settings/converter/style/release_info/
  composer/flow/layout/typo_alts/datetime のうち Linux は release_info を除く11本相当）
  と `entry/src/main/cpp/engine.cpp` はコンパイルしない。
- 構成時に WARNING で「状態ありクラスの C ABI が無いためリンクは未定義シンボルで
  失敗する見込み」を明示する（下記対応表）。

## 2. 対応表: desktop/core の API → Rust 輸出

シェル（linux の `src/shunti.cpp`/`shunti.h`、windows の `text_service.cpp` 等）が
使う core の API と、librext_kkc の輸出（kkc_* 52本＋rext_* 25本）の対応。

### 対応あり（置換可能）
| core の API | Rust 輸出 | 備考 |
|---|---|---|
| `Converter` の内側の変換エンジン呼び出し（engine.cpp） | `kkc_*` 52本（engine.h 互換） | Converter クラス自体のラッパではない（下記） |
| `datetime_predict(reading, now)` (datetime.h) | `rext_datetime_predict` / `_with_offset` | rext 側は UTC+9 固定版と offset 明示版。C++ 側は localtime |
| `number_forms(reading)` (datetime.h) | `rext_number_forms` | |
| `text::to_katakana / to_hiragana / to_halfwidth_kana / to_fullwidth_ascii / to_halfwidth_ascii` | `rext_to_*` 5本 | |
| `Romaji::to_romaji`、無状態の一括かな変換 | `rext_kana_to_romaji` / `rext_romaji_to_kana` | 1キーずつの状態あり処理は対象外（下記） |
| `pos_classes.h` の品詞クラス参照 | `rext_pos_class_count / rext_pos_class / rext_pos_group` | |

### 対応なし（残置・対象外。正直な一覧）
| core の API | 状態 | 理由・次の一手 |
|---|---|---|
| `Composer` クラス全体（composer.h、353行の状態機械。シェルの中心） | **輸出なし** | 状態あり＋コールバック（context provider 等）。ハンドル型 ABI（生成/破棄/キー入力/View 取得）の新規設計が必要。今回の範囲外 |
| `Converter` クラス（converter.h。学習・ユーザー辞書・スタイル・AI学習・候補の木まで含む窓口） | **輸出なし**（内側の kkc_* のみ対応） | 状態あり。kkc_* 直叩きへの書き換えは Composer と一体で必要 |
| `Learning` / `UserDict` / `StyleLog`（store.h） | **輸出なし** | 状態あり＋ファイル保存先が OS 依存。rext-core に対応ロジックはあるが C ABI が無い |
| `SentenceFlow`（flow.h）＋ `forget_all` 等 | **輸出なし** | 状態あり＋保存先 OS 依存（REXT_EXPORTS.md の未対応理由と同じ） |
| `Layout` クラス（layout.h。同時押し・配列解釈） | **輸出なし** | 状態あり。シェル（主に Windows）が直接使う |
| `Romaji` の状態あり操作（process/pending/backspace/add_pair/set_extended） | **輸出なし** | ハンドル型 ABI が別途必要（REXT_EXPORTS.md 既記） |
| `Settings` / `load_settings` / `save_settings` / `SettingsFile`、`model_file_name` 等（settings.h） | **輸出なし** | shunti.cpp は `model_file_name` を使用。rext-core::settings に対応ロジックはあるが C ABI が無い |
| `release_info.h`（`latest_release` / `version_older` / `release_change_lines`） | **輸出なし** | 更新確認用。rext-core にも対応モジュールが無い（移植対象外だった） |
| `text::from_utf8 / to_utf8`、`store.h` の `read_file / write_file_atomic` 等のファイル・文字列ユーティリティ | **輸出なし** | rext_* は UTF-8 文字列を返す小物変換のみで、UTF-16 ベースの汎用ユーティリティ輸出は無い |
| `StyleModel`（style.h）、`typo_alts` の状態あり関数、`build_tree` / `group_phrases` / `is_slur`（converter.h） | **輸出なし** | 構造体（Word/Phrase/TreeNode）を返すため文字列規約に乗らない |
| brackets / number の小物 | シェルは未使用 | rext_* にはあるが desktop シェル側の利用箇所は無い（スマホ側の置換用） |

**結論**: 現行シェル（shunti.cpp / text_service.cpp）は上記の状態ありクラスに
全面依存しているため、リンク切替だけではシェルはリンクできない。
今回のゴールは「CMake/build.bat の配線切替＋不足の明示」であり、シェルの
全面書き換え（Composer 等のハンドル型 ABI 設計を含む）は次段階の作業として残る。

## 3. 実ビルド検証（実測）

### ベースライン（変更前）
```
$ cmake -S desktop/linux -B /tmp/rext-baseline-build -DCMAKE_BUILD_TYPE=Release
CMake Error at CMakeLists.txt:13 (find_package):
  Could not find a package configuration file provided by "Fcitx5Core"
  (Fcitx5CoreConfig.cmake / fcitx5core-config.cmake)
-- Configuring incomplete, errors occurred!
```
→ 前提どおり Fcitx5Core 不在で構成が止まることを確認。

```
$ cargo test   (rust/ ワークスペース全体、変更前)
rext-core lib 146 passed / golden_match 8 passed / rext-kkc 7 passed
合計 161 passed / 0 failed
$ cargo build --release → librext_kkc.so 1.7MB、nm -D: kkc_* 52本 / rext_* 25本
```

### 変更後（Linux、この環境で可能な最大の検証）
```
$ cmake -S desktop/linux -B /tmp/rext-phase3-build -DCMAKE_BUILD_TYPE=Release
→ 構成成功。Fcitx5Core 不在の WARNING を出して shunti 本体はスキップ、
  rext_host_check のみを対象にする旨が表示される
$ cmake --build /tmp/rext-phase3-build -j
→ cargo build (release, SONAME 付き) が CMake から呼ばれ、
  rext_host_check のコンパイル・リンク成功
$ /tmp/rext-phase3-build/rext_host_check
rext_host_check: 54 checks, 0 failures   (終了コード 0)
$ ctest --output-on-failure
1/1 Test #1: rext_host_check ... Passed
```

ホスト検証の中身（`desktop/linux/tests/rext_host_check.cpp`）:
core_test.cpp のうち Fcitx 非依存かつ Rust 輸出に置換可能なチェックを移植。
- romaji 8件（konnnichiha→こんにちは 等、core_test と同じ期待値）
- text 5件（半角カナ `ガッコウ　パン`→`ｶﾞｯｺｳ ﾊﾟﾝ` 等）
- datetime 12件（固定時刻 2026-10-17 (土) 14:05 JST。きょう12形・いま4形・
  あした・UTC offset 版）
- number 15件（2026→令和8年 先頭、丸数字、桁区切り、漢数字、万混ぜ、predict、
  不正入力の NULL/空）
- brackets 8件、pos 1件、NULL 安全 3件
- kkc_* は辞書・モデルがリポジトリ内に無いため変換は検証せず、
  `kkc_open_files`(存在しないファイル) が NULL を返すことと
  `kkc_close(NULL)` の安全性のみ確認（2件）

### cargo test（全体161件）
- 本担当の変更は rust/ 本体に触れていない（変更は desktop/ と本記録のみ）。
- 変更前の全体実行で **161 passed / 0 failed** を実測済み。
- 加えて変更後の CMake ビルド経由で rext-kkc の cargo build が成功している。
- ※ セッション後半にツール側のセキュリティ通知があったため、変更後の
  cargo test 全体再実行は行っていない。再実行は未実施として明記する
  （rust/ に変更が無いため結果が変わる要因は無いが、実測と推測は区別する）。

## 4. Windows: build.bat の切替

判断: **既定は従来ビルド（REXT_RUST=0）のまま、1変数で切り替える配線を実装した。**
理由: (1) MSVC 環境が無く切替後のビルドを一切検証できない、(2) 上記対応表のとおり
状態ありクラスの C ABI が無いため、既定を切り替えると現状唯一動く Windows ビルドが
確実にリンク失敗するようになる。「無理に書き換えて壊す」より、切替箇所を実装して
既定 OFF にする方を選んだ。

実装した切替（`REXT_RUST=1` のとき、メイン IME DLL のみ）:
- 先頭で `:rust` を呼び、cargo で x86_64-pc-windows-msvc / i686-pc-windows-msvc の
  rext-kkc を release ビルドする。
- `:build` の cl 行から desktop/core の .cpp 11本＋engine.cpp を外し
  （変数 `CORE_AND_ENGINE` を空にする）、engine_avx2.cpp のコンパイルを止め、
  link 行に rext_kkc のインポートライブラリ（`%REXTLINK%`）を追加し、
  rext_kkc.dll を dist へコピーする。
- インポートライブラリ名は MSVC 未検証のため `rext_kkc.dll.lib` →
  無ければ `rext_kkc.lib` へフォールバックする形にした（コメント明記）。
- kana_fix / voice_app は x64 の engine.obj 等に依存し Rust モードでは存在しないため、
  REXT_RUST=1 のときはスキップする条件を付けた。
- settings_app / setup は core の一部（release_info/text/store/flow/settings）を
  直接コンパイルするが、Settings 等に C ABI が無いため**切替対象外**として
  コメントで明示し、両モードとも従来どおりにした。

## 5. 未検証事項（分けて明記）

| 項目 | 状態 | 理由 |
|---|---|---|
| Fcitx5 本体の実ビルド（shunti.so の生成） | **未検証** | ホストに Fcitx5Core 開発ファイルが無い。apt 等での導入は指示により行っていない。さらに C ABI 不足のため、Fcitx5 があっても現行 shunti.cpp はリンク失敗が見込まれる（対応表参照） |
| Windows 実ビルド（REXT_RUST=0 / 1 両方） | **未検証** | MSVC 環境が無い。build.bat の変更は構文の目視確認のみ。特にインポートライブラリ名、`:rust` の cargo ターゲット要否（rustup target の追加）は Windows 実機で要確認 |
| kkc_convert 等のエンジン変換のホスト検証 | **未検証** | 辞書・モデル（kkc_lex.bin / kkc_model.bin）がリポジトリ内に無い（core_test.cpp のエンジン必須部分が対象外なのと同じ理由） |
| 変更後の cargo test 全体再実行 | **未実施** | 上記 3 節の注記参照（変更前 161/0 は実測済み、rust/ は無変更） |
| クリーンな CMake ビルドディレクトリからの再現 | **未検証** | 検証は既存ビルドディレクトリの再構成で行った。cargo のキャッシュが効かない初回は cargo build に数分かかる見込み |

## 6. 次段階の候補（今回の範囲外）
1. 状態ありクラスのハンドル型 C ABI 設計（Composer / Converter / Learning /
   UserDict / SentenceFlow / StyleLog / Layout / Romaji 状態操作）。
2. その ABI を使ったシェル（shunti.cpp / text_service.cpp）の書き換え。
3. release_info / settings / ファイル IO ユーティリティの輸出追加。
