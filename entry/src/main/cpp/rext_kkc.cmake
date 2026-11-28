# Rust製エンジン librext_kkc.so の取り込み (第3弾 A: HarmonyOS配線)。
#
# 役割: engine.h の kkc_* ABI (52本) と rext_* (25本) を輸出する Rust cdylib
#       librext_kkc.so を見つけ (または任意で cargo でビルドし)、リンク用の関数
#       rext_kkc_link_to(<target>) を公開する。CMakeLists.txt は napi_kkc.cpp だけを
#       コンパイルし、この関数で Rust .so をリンクする。napi_kkc.cpp のコンパイル時は
#       従来どおり engine.h の宣言を使い、リンク時に Rust .so のシンボルで解決される。
#
# リンクは IMPORTED ターゲットの実パス指定ではなく -L<dir> + -lrext_kkc で行う。
# librext_kkc.so には SONAME が無く、実パスでリンクすると GNU ld が DT_NEEDED に
# ビルド機の絶対パスを埋めてしまう (ホスト検証で実測)。-l 経由なら DT_NEEDED は
# ファイル名 librext_kkc.so になり、HAP 内の同梱先から名前で解決できる。
# 場所の参照用に IMPORTED ターゲット rext_kkc_imported も作るが、リンクには使わない。
# なお target_link_directories を使うため CMake 3.13+ が必要 (DevEco 同梱の CMake は
# これを満たす想定。実機側での確認は rust/phase3/A-harmonyos.md の未検証事項を参照)。
#
# 方式は (a) 事前ビルド済み .so の imported library を既定とした。理由と (b) との比較は
# rust/phase3/A-harmonyos.md を参照。要点だけ書くと、hvigor の externalNativeOptions は
# CMake を OHOS ツールチェーンで回すだけで Rust ツールチェーンの存在を保証せず、
# (b) のカスタムコマンドは DevEco 機に rustup / ohos ターゲット / NDK リンカ設定まで
# 要求して壊れやすい。(a) なら CMake 側はファイルの有無とアーキテクチャの検査だけで済む。
# (b) も REXT_KKC_BUILD_WITH_CARGO=ON で選べるよう残してある (既定 OFF)。
#
# 上書き可能な変数 (優先順: -D の CMake 変数 > 環境変数 > 既定値):
#   REXT_KKC_LIBRARY          librext_kkc.so の場所そのもの (最優先)
#   REXT_KKC_LIB_DIR          librext_kkc.so の置きディレクトリ
#   REXT_RUST_WORKSPACE_DIR   Rust ワークスペース (既定: リポジトリ直下の rust/)
#   REXT_RUST_TARGET          Rust ターゲット (既定: aarch64-unknown-linux-ohos。
#                             entry/build-profile.json5 の abiFilters が arm64-v8a のみのため)
#   REXT_CARGO_EXECUTABLE     (b) で使う cargo (既定: 環境変数 CARGO、無ければ "cargo")
#   REXT_KKC_BUILD_WITH_CARGO (b) を有効化する ON/OFF (既定 OFF)
#   REXT_KKC_SKIP_ARCH_CHECK  ELF アーキテクチャ検査を無効化する ON/OFF (既定 OFF)

# --- 変数の解決 (CMake 変数が未定義のときだけ環境変数→既定値の順で埋める) ---
if(NOT DEFINED REXT_RUST_TARGET)
    if(DEFINED ENV{REXT_RUST_TARGET})
        set(REXT_RUST_TARGET "$ENV{REXT_RUST_TARGET}")
    else()
        set(REXT_RUST_TARGET "aarch64-unknown-linux-ohos")
    endif()
endif()
set(REXT_RUST_TARGET "${REXT_RUST_TARGET}" CACHE STRING "librext_kkc.so の Rust ターゲット")

if(NOT DEFINED REXT_RUST_WORKSPACE_DIR)
    if(DEFINED ENV{REXT_RUST_WORKSPACE_DIR})
        set(REXT_RUST_WORKSPACE_DIR "$ENV{REXT_RUST_WORKSPACE_DIR}")
    else()
        # このファイルは entry/src/main/cpp/ にあるので、リポジトリ直下は 4 つ上。
        get_filename_component(REXT_RUST_WORKSPACE_DIR
            "${CMAKE_CURRENT_LIST_DIR}/../../../../rust" ABSOLUTE)
    endif()
endif()
set(REXT_RUST_WORKSPACE_DIR "${REXT_RUST_WORKSPACE_DIR}" CACHE PATH "Rust ワークスペースの場所")

if(NOT DEFINED REXT_KKC_LIBRARY AND DEFINED ENV{REXT_KKC_LIBRARY})
    set(REXT_KKC_LIBRARY "$ENV{REXT_KKC_LIBRARY}")
endif()
set(REXT_KKC_LIBRARY "${REXT_KKC_LIBRARY}" CACHE FILEPATH "librext_kkc.so の場所 (最優先で使う)")

if(NOT DEFINED REXT_KKC_LIB_DIR AND DEFINED ENV{REXT_KKC_LIB_DIR})
    set(REXT_KKC_LIB_DIR "$ENV{REXT_KKC_LIB_DIR}")
endif()
set(REXT_KKC_LIB_DIR "${REXT_KKC_LIB_DIR}" CACHE PATH "librext_kkc.so の置きディレクトリ")

if(NOT DEFINED REXT_CARGO_EXECUTABLE)
    if(DEFINED ENV{CARGO})
        set(REXT_CARGO_EXECUTABLE "$ENV{CARGO}")
    else()
        set(REXT_CARGO_EXECUTABLE "cargo")
    endif()
endif()
set(REXT_CARGO_EXECUTABLE "${REXT_CARGO_EXECUTABLE}" CACHE FILEPATH "(b) で使う cargo")

option(REXT_KKC_BUILD_WITH_CARGO "(b) CMake から cargo build で librext_kkc.so を作る" OFF)
option(REXT_KKC_SKIP_ARCH_CHECK "librext_kkc.so の ELF アーキテクチャ検査を無効化" OFF)

# cargo の出力先 (ワークスペースの target/<target>/release/)。
set(REXT_KKC_CARGO_OUTPUT
    "${REXT_RUST_WORKSPACE_DIR}/target/${REXT_RUST_TARGET}/release/librext_kkc.so")

# --- .so の決定 ---
if(REXT_KKC_BUILD_WITH_CARGO)
    # (b): ビルド順序は custom command (cargo) → imported 配置 → kkc のリンク、の順に
    # add_dependencies で固定する。ohos ターゲットのリンクには OHOS NDK の clang を
    # リンカに指定する Rust 側設定 (rust/.cargo/config.toml 等) が別途要る点に注意。
    set(REXT_KKC_LIBRARY_RESOLVED "${REXT_KKC_CARGO_OUTPUT}")
    add_custom_command(
        OUTPUT "${REXT_KKC_LIBRARY_RESOLVED}"
        COMMAND "${REXT_CARGO_EXECUTABLE}" build --release -p rext-kkc
                --target "${REXT_RUST_TARGET}"
                --manifest-path "${REXT_RUST_WORKSPACE_DIR}/Cargo.toml"
        COMMENT "cargo build -p rext-kkc --release --target ${REXT_RUST_TARGET}"
        VERBATIM)
    add_custom_target(rext_kkc_cargo_build DEPENDS "${REXT_KKC_LIBRARY_RESOLVED}")
else()
    # (a): 候補を優先順に探す。
    set(_rext_candidates "")
    if(REXT_KKC_LIBRARY)
        list(APPEND _rext_candidates "${REXT_KKC_LIBRARY}")
    endif()
    if(REXT_KKC_LIB_DIR)
        list(APPEND _rext_candidates "${REXT_KKC_LIB_DIR}/librext_kkc.so")
    endif()
    list(APPEND _rext_candidates "${REXT_KKC_CARGO_OUTPUT}")

    set(REXT_KKC_LIBRARY_RESOLVED "")
    foreach(_cand IN LISTS _rext_candidates)
        if(EXISTS "${_cand}")
            set(REXT_KKC_LIBRARY_RESOLVED "${_cand}")
            break()
        endif()
    endforeach()
    if(NOT REXT_KKC_LIBRARY_RESOLVED)
        message(FATAL_ERROR
            "librext_kkc.so が見つかりません。先に Rust 側をビルドしてください:\n"
            "  cargo build --release -p rext-kkc --target ${REXT_RUST_TARGET} "
            "--manifest-path \"${REXT_RUST_WORKSPACE_DIR}/Cargo.toml\"\n"
            "または場所を -DREXT_KKC_LIBRARY=<path> / 環境変数 REXT_KKC_LIBRARY / "
            "-DREXT_KKC_LIB_DIR=<dir> で指定してください。探した候補: ${_rext_candidates}")
    endif()
endif()
message(STATUS "rext_kkc: ${REXT_KKC_LIBRARY_RESOLVED} (target=${REXT_RUST_TARGET}, cargo=${REXT_KKC_BUILD_WITH_CARGO})")

# --- アーキテクチャ検査 ---
# 間違ったアーキテクチャの .so (例: ホスト x86_64 用) を arm64-v8a の HAP に混ぜると
# リンクは通っても実機でロードできない。ELF ヘッダの e_machine を先に照合する。
# (b) の場合はビルド成果物ができる前は検査できないので、リンク後に目視確認すること。
if(NOT REXT_KKC_SKIP_ARCH_CHECK AND NOT REXT_KKC_BUILD_WITH_CARGO)
    set(_rext_expected_machine "")
    if(REXT_RUST_TARGET MATCHES "aarch64")
        set(_rext_expected_machine "b700")   # EM_AARCH64 (0xB7, リトルエンディアン)
    elseif(REXT_RUST_TARGET MATCHES "x86_64")
        set(_rext_expected_machine "3e00")   # EM_X86_64 (0x3E, リトルエンディアン)
    endif()
    if(_rext_expected_machine)
        file(READ "${REXT_KKC_LIBRARY_RESOLVED}" _rext_elf_hex HEX LIMIT 20)
        string(SUBSTRING "${_rext_elf_hex}" 36 4 _rext_machine)
        string(TOLOWER "${_rext_machine}" _rext_machine)
        if(NOT _rext_machine STREQUAL _rext_expected_machine)
            message(FATAL_ERROR
                "librext_kkc.so のアーキテクチャが REXT_RUST_TARGET=${REXT_RUST_TARGET} と合いません "
                "(e_machine=${_rext_machine}, 期待値=${_rext_expected_machine}): "
                "${REXT_KKC_LIBRARY_RESOLVED}\n"
                "正しいターゲットでビルドし直すか、REXT_KKC_LIBRARY で正しい .so を指定してください。"
                "検査を無効化するには -DREXT_KKC_SKIP_ARCH_CHECK=ON。")
        endif()
    endif()
endif()

# --- 場所の決定と参照用 IMPORTED ターゲット ---
get_filename_component(REXT_KKC_LIB_DIR_RESOLVED
    "${REXT_KKC_LIBRARY_RESOLVED}" DIRECTORY)
if(NOT TARGET rext_kkc_imported)
    add_library(rext_kkc_imported SHARED IMPORTED)
endif()
set_target_properties(rext_kkc_imported PROPERTIES
    IMPORTED_LOCATION "${REXT_KKC_LIBRARY_RESOLVED}")

# 対象ターゲットへ librext_kkc.so をリンクし、出力先へ .so をコピーする。
# コピーする理由: hvigor が HAP に同梱するネイティブライブラリは CMake の出力先から
# 集められる想定で、.so の実体はリポジトリ内の rust/target/ にあるため、そのままでは
# 同梱対象から漏れる可能性がある (DevEco での実ビルドで要確認。詳細は
# rust/phase3/A-harmonyos.md)。
function(rext_kkc_link_to _target)
    # SONAME 無しの .so を実パスでリンクすると DT_NEEDED に絶対パスが入るため、
    # -L + -l でファイル名だけを DT_NEEDED に残す (ヘッダの説明を参照)。
    target_link_directories(${_target} PRIVATE "${REXT_KKC_LIB_DIR_RESOLVED}")
    target_link_libraries(${_target} PRIVATE rext_kkc)
    if(REXT_KKC_BUILD_WITH_CARGO)
        # (b) では .so がリンク時までに出来ていることを依存関係で保証する。
        add_dependencies(${_target} rext_kkc_cargo_build)
    endif()
    add_custom_command(TARGET ${_target} POST_BUILD
        COMMAND ${CMAKE_COMMAND} -E copy_if_different
                "${REXT_KKC_LIBRARY_RESOLVED}" "$<TARGET_FILE_DIR:${_target}>/librext_kkc.so"
        COMMENT "librext_kkc.so を ${_target} の出力先へコピー (HAP 同梱用)"
        VERBATIM)
endfunction()
