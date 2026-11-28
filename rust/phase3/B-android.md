# 第3弾 B. Android 配線（担当: build.gradle.kts のみ）

日付: 2026-11-28 / 作業場所: `~/workspace/rext-rust-work/repo`（元の GitHub へは push していない）
変更したファイル: `android/app/build.gradle.kts` のみ。`jni.cpp`・Kotlin ファイル・Rust 本体は変更していない。
新規作成: このファイル `rust/phase3/B-android.md`。`RUST_PORT_STATUS.md` は編集していない。

## 1. 変更内容（build.gradle.kts）

現行は `buildKkc` が NDK clang で `engine.cpp + jni.cpp` を 1 つの `libkkc.so` にしていた。これを次の 2 タスクに置き換えた（`preBuild` が `buildKkc` と `copyAiAssets` に依存する点、`sourceSets` が `build/kkc/jniLibs` を取り込む点は不変）。

- `buildRustKkc`（新規、Exec）: `rust/` で
  `cargo build --release --target aarch64-linux-android -p rext-kkc` を実行し、できた
  `rust/target/aarch64-linux-android/release/librext_kkc.so` を `build/kkc/jniLibs/arm64-v8a/` にコピーする。
  - 入力: `rust/crates/`・`rust/Cargo.toml`・`rust/Cargo.lock`。出力: cargo の `.so` と jniLibs 側コピーの両方を宣言。
  - リンカ指定: リポジトリに `.cargo/config.toml` は作らない（NDK の絶対パスはマシンごとに違うため）。
    代わりに Gradle が知っている SDK の場所から
    `CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=<sdk>/ndk/27.2.12479018/toolchains/llvm/prebuilt/<host>/bin/aarch64-linux-android26-clang`
    を環境変数で渡す。`doFirst` でその clang の存在をチェックし、無ければ NDK バージョン名入りの `GradleException` で止める。
  - cargo の実行ファイルは `~/.cargo/bin/cargo` があればそれ、無ければ PATH 上の `cargo`（Gradle デーモンの PATH に cargo が無い環境への配慮）。
  - 事前条件（1 回だけ）: `rustup target add aarch64-linux-android`。未導入なら cargo がエラーで止まる。
- `buildKkc`（変更、Exec）: コンパイル対象を `jni.cpp` **だけ**にし、`engine.cpp` を外した。
  `-L build/kkc/jniLibs/arm64-v8a -lrext_kkc` で Rust ライブラリにリンクし、`dependsOn(buildRustKkc)` を付けた。
  追加で `-Wl,--no-undefined` を入れ、Rust 側に無い `kkc_*` があれば実行時ではなくリンク時に落とすようにした。
  入力には `jni.cpp`・`engine.h`・`ai_learn.h`・`phrase_ends.h`・`desktop/core/pos_classes.h` と `librext_kkc.so` を宣言（従来の入力から `engine.cpp` だけを外した形）。
  その他のフラグ（`--target=aarch64-linux-android26`・`-O3 -std=c++17 -fPIC`・`-static-libstdc++`・`-landroid -llog`・`-s` 等）は現行を踏襲。
- NDK ホスト判定（win/mac/linux）とツールチェーンの bin パスは、両タスクで使うためタスク定義の前に `ndkOs` / `ndkHost` / `ndkBin` として切り出した（ロジック自体は現行のまま）。

## 2. 構成の理由（2 層にした理由）

採用: **2 層** = `librext_kkc.so`（Rust、`kkc_*`）＋ `libkkc.so`（`jni.cpp` だけをコンパイルした薄い JNI 層、`librext_kkc` にリンク）。

- `Engine.kt` は `companion object { init { System.loadLibrary("kkc") } }` で `libkkc.so` しか読み込まない（実読で確認、変更なしで矛盾しない）。
  `libkkc.so` が `librext_kkc.so` への `DT_NEEDED` を持てば、Bionic のリンカが同じ APK 内 jniLibs（`lib/arm64-v8a/`）から依存ライブラリを自動で読み込むため、`Engine.kt` に `loadLibrary("rext_kkc")` を足す必要はない。両 `.so` は同じ `build/kkc/jniLibs/arm64-v8a/` に置かれ、`packaging.jniLibs.useLegacyPackaging = false` のまま APK に同梱される。
- `jni.cpp` は捨てられない: JNI シンボル（後述 19 本）、`AAssetManager` 経由の辞書・モデル読み込み、`kkc_ai::*`（`ai_learn.h` の inline）と `kkc_phrase_ends`（`phrase_ends.h` の inline）、`pos_classes.h` の表参照はすべて `jni.cpp` 側にあり、Rust 側は `kkc_*` の C ABI しか提供しない。したがって JNI 層は残し、エンジン実体だけを差し替えるのが最小構成になる。
- 却下した案: Rust の出力名を `libkkc.so` に変える案は、JNI シンボルが Rust 側に無いため成立しない（`UnsatisfiedLinkError` になる）。`jni.cpp` を Rust 側 cdylib に静的リンクする案は、ビルド系が cargo と NDK clang に二重化して複雑になる割に利点がない。

## 3. JNI シンボル照合結果（実測）

実測方法: ホスト向け release ビルド済み `rust/target/release/librext_kkc.so`（x86-64）に対して
`nm -D --defined-only` で輸出を取得し、`engine.h` の宣言、`jni.cpp` の呼び出しと照合した（シンボル名の集合の照合であり、ABI はターゲット非依存）。

- Rust 輸出（実測）: `kkc_*` **52 本** ＋ `rext_*` **25 本** ＝ 77 本。
- `engine.h` 宣言（関数 52 本）との照合: **52/52 完全一致**。engine.h にあって Rust に無いもの 0 件、逆も 0 件。
- `jni.cpp` が必要とする `kkc_*` は **25 本**で、全て Rust 輸出に存在した（不足 0 件、修正案なし）:
  - `jni.cpp` から直接呼ぶ 17 本: `kkc_open`, `kkc_close`, `kkc_convert`, `kkc_complete`, `kkc_last_times`,
    `kkc_last_suggest`, `kkc_last_segments`, `kkc_last_paths`, `kkc_last_segment_pos`, `kkc_path_pos`,
    `kkc_set_threads`, `kkc_set_keys`, `kkc_set_suggest`, `kkc_set_char_filter`, `kkc_user_clear`,
    `kkc_user_add_pos`, `kkc_user_add_like`
  - ヘッダ経由で追加で要る 8 本（`ai_learn.h` の `kkc_ai::*` inline から）: `kkc_romance_enable`,
    `kkc_romance_config`, `kkc_romance_learn`, `kkc_romance_size`, `kkc_romance_get`, `kkc_romance_set`,
    `kkc_romance_reset`, `kkc_model_tag`
- シンボル名だけでなく、jni.cpp が使う範囲では引数の並びも `engine.h` 宣言と Rust 側
  `rext-core/src/engine.rs` の `ffi` 実装で一致していることを目視確認した（特に引数の多い
  `kkc_convert`、`kkc_user_add_pos`（`groups`/`ngroups` 付き）、`kkc_user_add_like`、`kkc_last_paths`）。
- 注意（不足ではない）:
  - `kkc_phrase_ends` は輸出シンボルではない。`phrase_ends.h` の `inline` 関数で、中で `kkc_last_segments` と
    `kkc_last_segment_pos`（どちらも輸出あり）を呼ぶ。jni.cpp をコンパイルすれば解決する。
  - `kkc_ai` は名前空間、`kkc_engine` は型名、`kkc_lex` / `kkc_model` は資産ファイル名であり、関数ではない。
  - JNI 側の関数（`Java_com_shunti_japaneseime_Engine_native*`）は jni.cpp 内に **19 本**あり、
    `Engine.kt` の `external fun` **19 本**と 1 対 1 で対応する（今回の変更で不変）。
- 結論: **ズレなし。jni.cpp の修正は不要**（別担当への申し送りもなし）。

## 4. 検証できた範囲 / できなかった範囲

できた:
- JNI シンボル照合（上記、nm 実測ベース）。
- Android ターゲット向け Rust コードのコンパイル確認: この環境で `rustup target add aarch64-linux-android`
  （導入成功）を実行し、`CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=rust-lld cargo build --release --target aarch64-linux-android -p rext-kkc`
  を試行。Rust コード自体は aarch64 向けにコンパイルできたが、**リンクは失敗**した
  （`rust-lld: error: unable to find library -ldl / -llog / -lunwind / -lm / -lc`）。
  これは NDK の sysroot が無いためで、Gradle 側で NDK clang をリンカに指定する設計の根拠になった。
  NDK がある環境ではこのリンクが通る見込みだが、ここでは未検証。
- Gradle 文法: gradle / kotlinc / java がこの環境に無く（`java: command not found`、wrapper の実行も不可）、
  **Gradle の実ビルド・構文チェックは未検証**。代わりに自己レビューを行った:
  既存コードの書き方（`tasks.registering(Exec::class)`、`layout.buildDirectory` の Provider、`GradleException` の使用）
  を踏襲し、新規 API は `Exec` の `environment()` / `workingDir()` と `java.nio.file.Files.copy` のみに絞った。
  Provider を `inputs.file()` / `outputs.files()` に渡す箇所は Gradle の標準的な使い方の範囲だが、実地の確認はできていない。

できなかった（未検証を明記）:
- **Gradle 実ビルド未検証**（この環境に Android SDK / NDK / Java が無いため）。`./gradlew assembleDebug` 等は一切実行していない。
- APK への 2 つの `.so` の同梱、`DT_NEEDED` 経由での `librext_kkc.so` の実機ロード、実機での変換動作はすべて未検証。
- NDK clang をリンカにした aarch64 版 `librext_kkc.so` のリンク成功も未検証（上記の通り、この環境ではリンク前段まで）。

<!-- CARGO_TEST_RESULT -->
