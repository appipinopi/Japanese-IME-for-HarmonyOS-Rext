# shuntelligence (AI 変換) の推論エンジン libkkc.so。napi_kkc.cpp = ArkTS との窓口 (橋渡し)。
# 変換エンジン本体は Rust 化され (rust/crates/rext-kkc)、engine.h の kkc_* ABI を輸出する
# librext_kkc.so としてリンクする。旧 C++ 実装 (engine.cpp / glide.cpp / glide_nn.cpp) は
# もうコンパイルしない (ファイル自体は参照用に残置)。取り込み方式・変数は rext_kkc.cmake、
# 選択理由と検証範囲は rust/phase3/A-harmonyos.md を参照。
cmake_minimum_required(VERSION 3.5.0)
project(kkc)
set(CMAKE_CXX_STANDARD 17)

# Rust 製 librext_kkc.so の取り込み (rext_kkc.cmake の rext_kkc_link_to を使う)。
# 既定は事前ビルド済み .so のリンク (方式a)。場所やターゲットは
# REXT_KKC_LIBRARY / REXT_KKC_LIB_DIR / REXT_RUST_TARGET 等で上書き可能。
include(${CMAKE_CURRENT_SOURCE_DIR}/rext_kkc.cmake)

add_library(kkc SHARED napi_kkc.cpp)
target_include_directories(kkc PRIVATE ${CMAKE_CURRENT_SOURCE_DIR})
# 開発用ビルドでも最適化する (-O0 だと行列計算が数倍遅く、打鍵ごとの変換が重くなる)。
# target の指定は CMAKE_CXX_FLAGS_DEBUG の -O0 より後ろに付くので、こちらが勝つ。
# (行列計算の本体は Rust 側へ移ったが、橋渡しの変換処理にも同じ方針を残す)
target_compile_options(kkc PRIVATE -O3 -ffp-contract=fast -fno-math-errno)
# napi_kkc.cpp が engine.h 経由で呼ぶ kkc_* (glide 分の kkc_glide_* を含む) は、
# リンク時に librext_kkc.so のシンボルで解決される。あわせて HAP 同梱のため
# librext_kkc.so を libkkc.so の出力先へ並べる (詳細は rext_kkc.cmake)。
target_link_libraries(kkc PUBLIC libace_napi.z.so librawfile.z.so)
rext_kkc_link_to(kkc)
