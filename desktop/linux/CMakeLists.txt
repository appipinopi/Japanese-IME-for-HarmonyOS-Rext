# Linux 版 (Fcitx5 の入力メソッドのアドオン)。変換エンジンと共通ロジックは Rust 化済みで、
# このファイルは desktop/core の .cpp と entry の engine.cpp をコンパイルせず、
# Rust ライブラリ librext_kkc (kkc_* = engine.h 互換 ABI 52本 + rext_* 小物 25本) をリンクする。
#   cmake -B build -DCMAKE_INSTALL_PREFIX=/usr -DCMAKE_BUILD_TYPE=Release && cmake --build build -j && sudo cmake --install build
# 辞書とモデル (entry/src/main/resources/rawfile/kkc_*.bin) は share/shunti-ime に入る (python tools/fetch_ai_assets.py で取得)。
#
# Rust の取り込み方 (選択と理由): 事前ビルド済みバイナリはリポジトリに無いため、
# CMake のカスタムコマンドで cargo build を呼ぶ方式にした。再現性 (ソースから必ず作れる) と
# 簡潔さ (ExternalProject の superbuild を持ち込まない) の両立のため。cargo が PATH に必要。
# CMAKE_BUILD_TYPE=Release のとき --release、それ以外は debug プロファイルを使う。
#
# 既知の不足 (正直な現状。詳細は rust/phase3/C-desktop.md の対応表):
#   src/shunti.cpp / shunti.h は desktop/core の状態ありクラス (Composer / Converter /
#   Learning / UserDict / SentenceFlow / StyleLog) と自由関数 (text::from_utf8 等) を
#   直接使っている。これらには Rust 側の C ABI がまだ無い (rext_* は無状態の小物のみ、
#   kkc_* は変換エンジン本体のみ)。そのため Fcitx5 がある環境でも、現状の shunti.cpp は
#   リンク時に未定義シンボルになる。シェル側を kkc_*/rext_* のみに書き換える作業は
#   今回の範囲外で、不足は対応表に明示してある。Fcitx5 が無い環境では shunti 本体は
#   作らず、Rust リンクのホスト検証 (rext_host_check) だけをビルドする。
cmake_minimum_required(VERSION 3.16)
project(fcitx5-shunti VERSION 0.7.7 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)   # Fcitx5 5.1 の見出し (std::span など) が C++20 を要る
set(CMAKE_CXX_STANDARD_REQUIRED ON)
if(NOT CMAKE_BUILD_TYPE)
  set(CMAKE_BUILD_TYPE Release)
endif()

include(GNUInstallDirs)

set(ROOT ${CMAKE_CURRENT_SOURCE_DIR}/../..)

# ---------------------------------------------------------------- Rust (librext_kkc)
option(REXT_BUILD_RUST "cargo で librext_kkc をビルドする" ON)
if(REXT_BUILD_RUST)
  find_program(REXT_CARGO cargo REQUIRED)
  if(CMAKE_BUILD_TYPE STREQUAL "Release")
    set(REXT_PROFILE release)
    set(REXT_CARGO_FLAGS --release)
  else()
    set(REXT_PROFILE debug)
    set(REXT_CARGO_FLAGS "")
  endif()
  set(REXT_LIB_DIR ${CMAKE_CURRENT_BINARY_DIR}/rust)
  set(REXT_LIB ${REXT_LIB_DIR}/librext_kkc.so)
  file(GLOB_RECURSE REXT_SOURCES CONFIGURE_DEPENDS
    ${ROOT}/rust/crates/rext-core/src/*.rs
    ${ROOT}/rust/crates/rext-kkc/src/*.rs)
  # SONAME を付ける: 付けないと消費側の DT_NEEDED にリンク時の相対パス
  # (rust/librext_kkc.so) が残り、RPATH が効かず実行時に見つからない。
  add_custom_command(OUTPUT ${REXT_LIB}
    COMMAND ${CMAKE_COMMAND} -E env
            "RUSTFLAGS=-C link-arg=-Wl,-soname,librext_kkc.so"
            ${REXT_CARGO} build ${REXT_CARGO_FLAGS}
            --manifest-path ${ROOT}/rust/Cargo.toml -p rext-kkc
    COMMAND ${CMAKE_COMMAND} -E make_directory ${REXT_LIB_DIR}
    COMMAND ${CMAKE_COMMAND} -E copy_if_different
            ${ROOT}/rust/target/${REXT_PROFILE}/librext_kkc.so ${REXT_LIB}
    DEPENDS ${REXT_SOURCES} ${ROOT}/rust/Cargo.toml ${ROOT}/rust/Cargo.lock
            ${ROOT}/rust/crates/rext-kkc/Cargo.toml ${ROOT}/rust/crates/rext-core/Cargo.toml
            ${CMAKE_CURRENT_LIST_FILE}
    COMMENT "cargo build librext_kkc (${REXT_PROFILE})"
    VERBATIM)
  add_custom_target(rext_kkc_build DEPENDS ${REXT_LIB})
  add_library(rext_kkc SHARED IMPORTED GLOBAL)
  set_target_properties(rext_kkc PROPERTIES IMPORTED_LOCATION ${REXT_LIB})
  add_dependencies(rext_kkc rext_kkc_build)
else()
  message(FATAL_ERROR "REXT_BUILD_RUST=OFF は未対応 (事前ビルドの librext_kkc を置く場所が無い)")
endif()

# ---------------------------------------------------------------- ホスト検証 (Fcitx5 非依存)
# desktop/tests/core_test.cpp のうち Rust 輸出に置換可能なチェックを
# librext_kkc へ直接リンクして実行する。ctest で走る。
option(REXT_BUILD_HOST_CHECK "Rust リンクのホスト検証をビルドする" ON)
if(REXT_BUILD_HOST_CHECK)
  enable_testing()
  add_executable(rext_host_check tests/rext_host_check.cpp)
  target_link_libraries(rext_host_check PRIVATE rext_kkc)
  # librext_kkc.so には SONAME (librext_kkc.so) を付けているため DT_NEEDED は
  # その名前になるが、所在はビルドツリー内では RPATH で明示する必要がある。
  set_target_properties(rext_host_check PROPERTIES BUILD_RPATH "${REXT_LIB_DIR}")
  add_test(NAME rext_host_check COMMAND rext_host_check)
endif()

# ---------------------------------------------------------------- Fcitx5 アドオン本体
# 旧構成は find_package(Fcitx5Core REQUIRED) で、開発ファイルが無い環境では
# 構成そのものが止まっていた。ホスト検証を先に進められるよう、既定では
# QUIET にして「無ければ shunti 本体をスキップ」を選べるようにした。
# パッケージング等で従来どおり構成失敗させたいときは -DSHUNTI_REQUIRE_FCITX=ON。
option(SHUNTI_REQUIRE_FCITX "Fcitx5Core が無ければ構成を失敗させる (旧挙動)" OFF)
find_package(Fcitx5Core QUIET)
find_package(Threads QUIET)
if(NOT Fcitx5Core_FOUND)
  if(SHUNTI_REQUIRE_FCITX)
    message(FATAL_ERROR "Fcitx5Core が見つかりません (SHUNTI_REQUIRE_FCITX=ON)")
  endif()
  message(WARNING "Fcitx5Core が見つからないため shunti アドオン本体はビルドしません。"
                  "Rust リンクのホスト検証 (rext_host_check) のみビルドします")
else()
  message(WARNING "shunti.cpp は desktop/core の状態ありクラス (Composer 等) を使っており、"
                  "Rust 側にその C ABI はまだ無いため、リンクは未定義シンボルで失敗する見込みです。"
                  "対応表は rust/phase3/C-desktop.md を参照")
  # 旧構成との差分: ${CORE}/*.cpp (11本) と entry/src/main/cpp/engine.cpp は
  # コンパイルしない。代わりに rext_kkc (kkc_* / rext_*) をリンクする。
  add_library(shunti MODULE src/shunti.cpp)
  target_include_directories(shunti PRIVATE ${ROOT}/entry/src/main/cpp)
  target_link_libraries(shunti PRIVATE Fcitx5::Core Threads::Threads rext_kkc)
  target_compile_definitions(shunti PRIVATE SHUNTI_VERSION="${PROJECT_VERSION}")   # 更新の確認で今の版と比べる
  set_target_properties(shunti PROPERTIES PREFIX "")

  configure_file(shunti-addon.conf.in ${CMAKE_CURRENT_BINARY_DIR}/shunti-addon.conf @ONLY)
  install(TARGETS shunti DESTINATION "${FCITX_INSTALL_ADDONDIR}")
  install(FILES ${CMAKE_CURRENT_BINARY_DIR}/shunti-addon.conf RENAME shunti.conf DESTINATION "${FCITX_INSTALL_PKGDATADIR}/addon")
  install(FILES shunti.conf DESTINATION "${FCITX_INSTALL_PKGDATADIR}/inputmethod")
  install(FILES icons/shunti-ime.png DESTINATION "${CMAKE_INSTALL_DATADIR}/icons/hicolor/64x64/apps")
  # ライセンスの表記は Windows 版と同じもの (見出しだけ Linux 版に)
  file(READ ${ROOT}/desktop/windows/LICENSE.txt LICENSE_TEXT)
  string(REPLACE "(Windows 版)" "(Linux 版)" LICENSE_TEXT "${LICENSE_TEXT}")
  file(WRITE ${CMAKE_CURRENT_BINARY_DIR}/copyright "${LICENSE_TEXT}")
  install(FILES ${CMAKE_CURRENT_BINARY_DIR}/copyright README.md DESTINATION "${CMAKE_INSTALL_DATADIR}/doc/fcitx5-shunti")
  foreach(f kkc_lex.bin kkc_model.bin kkc_variants.tsv kkc_register.bin)
    if(EXISTS ${ROOT}/entry/src/main/resources/rawfile/${f})
      install(FILES ${ROOT}/entry/src/main/resources/rawfile/${f} DESTINATION "${CMAKE_INSTALL_DATADIR}/shunti-ime")
    else()
      message(WARNING "${f} がありません (python tools/fetch_ai_assets.py で取得してください)。変換はできません")
    endif()
  endforeach()
  # 軽量・高精度の変換モデル (設定で選ぶ。無ければ標準だけ)
  foreach(f kkc_model_light.bin)
    if(EXISTS ${ROOT}/models/${f})
      install(FILES ${ROOT}/models/${f} DESTINATION "${CMAKE_INSTALL_DATADIR}/shunti-ime")
    endif()
  endforeach()
endif()

# .deb (Debian・Ubuntu 向け): cpack -G DEB
set(CPACK_PACKAGE_NAME "fcitx5-shunti")
set(CPACK_PACKAGE_VERSION ${PROJECT_VERSION})
set(CPACK_PACKAGE_CONTACT "shuntilettuce")
set(CPACK_PACKAGE_DESCRIPTION_SUMMARY "shunti IME: AI kana-kanji conversion (shuntelligence) for Fcitx5")
set(CPACK_DEBIAN_PACKAGE_DEPENDS "fcitx5")
set(CPACK_DEBIAN_PACKAGE_SHLIBDEPS ON)
set(CPACK_DEBIAN_PACKAGE_SECTION "utils")
set(CPACK_DEBIAN_FILE_NAME DEB-DEFAULT)
include(CPack)
