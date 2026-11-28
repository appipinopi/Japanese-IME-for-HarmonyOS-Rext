# 第3弾 A: HarmonyOS 配線 (CMake 切替)

担当範囲: `entry/src/main/cpp/CMakeLists.txt` と、その配下への新規ファイル追加のみ。
`napi_kkc.cpp` / `types/libkkc/index.d.ts` / Rust 本体 / `RUST_PORT_STATUS.md` は変更していない。
元の GitHub へは push していない。

## 変更ファイル

| ファイル | 変更 |
|---|---|
| `entry/src/main/cpp/CMakeLists.txt` | 変更。`add_library(kkc SHARED napi_kkc.cpp)` のみにし、C++ エンジン (engine.cpp / glide.cpp / glide_nn.cpp) のコンパイルを停止。代わりに `rext_kkc.cmake` を include し `rext_kkc_link_to(kkc)` で Rust 製 `librext_kkc.so` をリンク。従来の `-O3` 等のコンパイルオプションと `libace_napi.z.so` / `librawfile.z.so` のリンクは維持 |
| `entry/src/main/cpp/rext_kkc.cmake` | 新規。librext_kkc.so の探索・検証・(任意) cargo ビルド・リンク・出力先へのコピーを担う |
| `rust/phase3/A-harmonyos.md` | 新規 (この記録) |

旧 C++ ソース (engine.cpp 等) は参照用に残置し、削除はしていない。

## ビルド順序の成立条件

- コンパイル時: `napi_kkc.cpp` は従来どおり `engine.h` の `kkc_*` 宣言を使う。あわせて読む
`ai_learn.h` / `phrase_ends.h` はヘッダ内 inline 実装で、それらも `kkc_*` 呼び出しのみに依存する
(engine.cpp 由来の非 ABI シンボルには依存しないことを grep で確認済み)。
- リンク時: `kkc_*` 52本 (glide 分の `kkc_glide_*` を含む) と `rext_*` 25本は Rust cdylib
`librext_kkc.so` が輸出済み (第2弾で検証済み。本作業でもホスト.so の `nm -D` で再確認)。
よって宣言 (C++) → 実体 (Rust.so) で解決される。

## 取り込み方式の比較と選択

| | (a) 事前ビルド済み.so を取り込む | (b) CMake カスタムコマンドで cargo build |
|---|---|---|
| 仕組み | 先に `cargo build --release -p rext-kkc --target aarch64-unknown-linux-ohos` で作った.so を CMake が探索してリンク | `REXT_KKC_BUILD_WITH_CARGO=ON` のとき CMake が `cargo build --release --target <REXT_RUST_TARGET>` を実行し、その成果物をリンク |
| hvigor との相性 | hvigor の `externalNativeOptions` (abiFilters は arm64-v8a のみ) は CMake を回すだけ。Rust ツールチェーンの有無に影響されない | hvigor が回す CMake の内側から cargo を呼ぶため、DevEco 機に rustup・ohos ターゲット・NDK リンカ設定・PATH が全て揃っている必要があり、失敗点が増える |
| 再現性 |.so が成果物として固定され、CMake 側は存在とアーキテクチャの検査だけで済む | CMake の再実行のたびに cargo の増分状態に依存する |
| 弱点 |.so のビルドを開発者が先に済ませる手順が要る (未ビルド時は CMake が手順付きで FATAL_ERROR を出す) | 上記の環境依存。ohos ターゲットのリンクには Rust 側のリンカ設定 (OHOS NDK の clang 指定) が別途必須 |

選択理由: この環境でも DevEco 機でも、hvigor のネイティブビルドと喧嘩しないのは (a)。
(b) は捨てずにオプションとして実装してある (既定 OFF、ホストでは動作検証済み・下記)。

### 実装上の注意 (方式a の中での工夫)

- リンクは IMPORTED ターゲットの実パス指定ではなく `-L<dir>` + `-lrext_kkc` で行う。
理由: `librext_kkc.so` には SONAME が無く、実パスでリンクすると GNU ld が `DT_NEEDED` に
ビルド機の絶対パスを埋める (ホスト検証 case1 初版で実測)。`-l` 経由では `DT_NEEDED` が
ファイル名 `librext_kkc.so` になり、HAP 内の同梱先から名前で解決できる。
参照用に IMPORTED ターゲット `rext_kkc_imported` も作るがリンクには使わない。
- HAP 同梱のため、POST_BUILD で `librext_kkc.so` を `libkkc.so` の出力先へコピーする
(hvigor が CMake 出力先からネイティブライブラリを集める想定。実ビルドでの確認は未検証事項)。
- アーキテクチャ検査:.so の ELF `e_machine` と `REXT_RUST_TARGET` を照合し、不一致
(例: ホスト x86_64 用.so を arm64 の HAP に混ぜる) は configure 時に FATAL_ERROR にする。
無効化は `-DREXT_KKC_SKIP_ARCH_CHECK=ON`。

### 上書き可能な変数 (優先順: -D の CMake 変数 > 環境変数 > 既定値)

| 変数 | 既定値 | 用途 |
|---|---|---|
| `REXT_KKC_LIBRARY` | (なし) | librext_kkc.so の場所そのもの (最優先) |
| `REXT_KKC_LIB_DIR` | (なし) | librext_kkc.so の置きディレクトリ |
| `REXT_RUST_TARGET` | `aarch64-unknown-linux-ohos` | Rust ターゲット。探索先 `rust/target/<target>/release/` とアーキ検査に使う |
| `REXT_RUST_WORKSPACE_DIR` | リポジトリ直下の `rust/` | ワークスペースの場所 |
| `REXT_KKC_BUILD_WITH_CARGO` | OFF | 方式 (b) の有効化 |
| `REXT_CARGO_EXECUTABLE` | 環境変数 `CARGO`、無ければ `cargo` | (b) で使う cargo |
| `REXT_KKC_SKIP_ARCH_CHECK` | OFF | アーキテクチャ検査の無効化 |

開発者の手順 (方式a): Rust をビルド → DevEco で通常どおりビルド。場所が既定と違う場合のみ
環境変数 `REXT_KKC_LIBRARY` 等で知らせる (build-profile.json5 の `arguments` への記入でも可)。

## 検証できた範囲 (この環境: DevEco / HarmonyOS SDK なし、ホスト x86_64 Linux)

1. `rustup target add aarch64-unknown-linux-ohos`: **成功** (この環境に導入済みになった)。
2. `cargo build --release -p rext-kkc --target aarch64-unknown-linux-ohos`: **リンク段階で失敗**。
rustc のコンパイル自体は全クレート通ったが、最終リンクをホストの `/usr/bin/ld` (x86_64) が
担い `Relocations in generic ELF (EM: 183)` / `file in wrong format` で落ちた。
原因は OHOS NDK の clang をリンカに指定する設定が無いこと (この環境に NDK が無いため設定不能)。
→ ohos 向け.so の生成自体は未達。DevEco/NDK のある環境で
`CARGO_TARGET_AARCH64_UNKNOWN_LINUX_OHOS_LINKER` 等の設定が要る (Rust 側設定は本担当の範囲外)。
3. CMake 構成検証 (ハーネス `/tmp/rext-cmake-test`: `rext_kkc.cmake` を単独 include し、
engine.h の宣言だけを使うダミー共有ライブラリをリンク): **全6ケース合格**。
- case1 明示パス指定: configure/build 成功、`DT_NEEDED = librext_kkc.so` (ファイル名のみ)、
出力先への.so コピーを確認
- case2 環境変数 `REXT_KKC_LIBRARY`: 成功
- case3 既定探索 (`rust/target/x86_64-unknown-linux-gnu/release/` に実ビルドした.so): 成功
- case4 アーキ不一致 (ohos 指定 + x86_64.so): configure が正しく FATAL_ERROR
- case5.so 未存在: 手順 (cargo build コマンド) 付きで正しく FATAL_ERROR
- case6 方式 (b) (`REXT_KKC_BUILD_WITH_CARGO=ON`、ホストターゲット): cargo が CMake から
呼ばれてビルドされ、リンク成功
4. Rust 回帰: `cargo test` (ワークスペース全体) **161 passed / 0 failed** (146+8+7)、
`cargo build --release` (ホスト) **成功**、`librext_kkc.so` 生成を確認。本担当は Rust 本体を
変更していないので当然だが、壊していないことを実測で確認した。

## できなかった範囲・未検証事項 (できたと偽らない)

- **DevEco Studio / hvigor での実ビルドは未検証** (この環境に無い)。特に次は実環境で要確認:
- POST_BUILD コピーした `librext_kkc.so` が実際に HAP の `libs/arm64-v8a/` へ同梱されるか
- OHOS のリンカ (lld) でも `DT_NEEDED` がファイル名のみになるか (ホスト検証は GNU ld)
- `target_link_directories` を使うため CMake 3.13+ が必要。DevEco 同梱 CMake は満たす想定だが未確認
- `libace_napi.z.so` 等とのリンク順・実機での `libkkc.so` ロードと変換動作
- **実機 (HarmonyOS 端末) での動作は未検証**。
- ohos 向け `librext_kkc.so` そのものが未生成 (上記2)。配線は「正しい.so が
`rust/target/aarch64-unknown-linux-ohos/release/` に置かれれば取り込める」状態までで、
.so の生成には NDK リンカ設定という別作業が残る。
- ハーネス検証は本物の `napi_kkc.cpp` ではなく宣言のみのダミーで代替した
(`napi/native_api.h` 等の OHOS ヘッダがホストに無いため)。本物のコンパイルは DevEco 側でのみ可能。
