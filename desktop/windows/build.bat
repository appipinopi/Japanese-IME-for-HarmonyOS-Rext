@echo off
rem Build the Windows version (TSF text service DLL + settings app) with MSVC (Visual Studio 2022 Build Tools).
rem Both x64 and x86 DLLs are needed (64-bit and 32-bit apps each load their own). The settings app is x64 only.
rem   build.bat         release build into desktop\build\dist (no input log code at all). package.ps1 makes the zip from dist only.
rem   build.bat debug   debug build WITH the input log (SHUNTI_INPUT_LOG) into desktop\build\dist_debug. Never distribute it.
rem
rem --- Rust switch: NOT done yet on Windows (2026-11-28, phase3 C-desktop) ---
rem This script still compiles desktop\core .cpp files and entry engine.cpp in C++.
rem Do not remove them yet: the Windows shell (text_service.cpp etc.) uses the
rem stateful desktop/core classes (Composer / Converter / Learning / UserDict /
rem SentenceFlow / StyleLog, text::from_utf8 / to_utf8) directly, and Rust only
rem exports the stateless C ABI (kkc_* 52 engine functions + rext_* 25 small
rem utilities in librext_kkc). There is no Rust C ABI for those classes, so
rem dropping the .cpp files now would only produce unresolved symbols.
rem Linux (desktop\linux\CMakeLists.txt) has the same gap documented; only its
rem Fcitx-free host check links Rust today. Full mapping: rust\phase3\C-desktop.md.
rem Switch points when a stateful Rust C ABI exists (keep this list current):
rem   1. :build cl line: drop CORE .cpp files and engine.cpp from the compile list.
rem   2. :build engine_avx2 cl line: drop it too (Rust engine has its own SIMD;
rem      kkc_avx2_* entry points are only called from engine.cpp).
rem   3. :build link line: add the Rust import library (rext_kkc.lib) built by
rem      cargo for x86_64-pc-windows-msvc and i686-pc-windows-msvc, and ship
rem      rext_kkc.dll next to shunti_ime_x64.dll / shunti_ime_x86.dll.
rem   4. :settings kana_fix and voice_app lines: they reuse engine.obj /
rem      converter.obj / engine_avx2.obj from the x64 DLL objects; relink them
rem      against rext_kkc.lib instead.
rem   5. Decide the CRT mix first: this script builds with /MT while the Rust
rem      msvc target links the dynamic CRT by default. Strings returned by
rem      rext_* must be freed with rext_free_string either way.
rem Until then this file is intentionally unchanged apart from this note,
rem because there is no MSVC environment here to verify a Windows build.
setlocal
set HERE=%~dp0
set ROOT=%HERE%..\..
set DIST=%HERE%..\build\dist
set DEFS=
set SUFFIX=
if /I "%~1"=="debug" (
  set DIST=%HERE%..\build\dist_debug
  set DEFS=/DSHUNTI_INPUT_LOG
  set SUFFIX=_debug
)
set VCVARS=C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build
set CORE=%ROOT%\desktop\core
if not exist "%DIST%" mkdir "%DIST%"

rem --- Rust switch (phase 3, 2026-11-28) ---------------------------------------
rem The conversion engine and the desktop\core logic now live in Rust
rem (rust\crates\rext-kkc, a cdylib exporting kkc_* 52 + rext_* 25).
rem Set REXT_RUST=1 to switch the main IME DLL below to link rext_kkc instead
rem of compiling desktop\core\*.cpp + entry engine.cpp (+ engine_avx2.cpp).
rem Default is 0 (the legacy C++ build, unchanged) because:
rem   - this switch is NOT verified with MSVC (no MSVC in the dev environment
rem     where the Rust port was done; see rust\phase3\C-desktop.md);
rem   - the Windows shell (text_service.cpp etc.) still uses the stateful C++
rem     classes Composer / Converter / Learning / UserDict / SentenceFlow /
rem     StyleLog, for which librext_kkc has NO C ABI yet (rext_* covers only
rem     stateless utilities, kkc_* only the engine). With REXT_RUST=1 the DLL
rem     link therefore fails with unresolved externals until a handle-based
rem     ABI for those classes is added. The mapping table is in
rem     rust\phase3\C-desktop.md.
rem Requires: cargo in PATH and rustup targets x86_64-pc-windows-msvc and
rem i686-pc-windows-msvc installed.
set REXT_RUST=0
if "%REXT_RUST%"=="1" call :rust || exit /b 1

call :build x64 vcvars64.bat || exit /b 1
call :build x86 vcvarsamd64_x86.bat || exit /b 1
call :settings || exit /b 1

rem dictionary and model (copied only when changed)
xcopy /D /Y /Q "%ROOT%\entry\src\main\resources\rawfile\kkc_lex.bin" "%DIST%\" >nul
xcopy /D /Y /Q "%ROOT%\entry\src\main\resources\rawfile\kkc_model.bin" "%DIST%\" >nul
xcopy /D /Y /Q "%ROOT%\entry\src\main\resources\rawfile\kkc_variants.tsv" "%DIST%\" >nul
xcopy /D /Y /Q "%ROOT%\entry\src\main\resources\rawfile\kkc_register.bin" "%DIST%\" >nul
rem light / high conversion models (models\, fetched by tools\fetch_ai_assets.py; optional)
if exist "%ROOT%\models\kkc_model_light.bin" xcopy /D /Y /Q "%ROOT%\models\kkc_model_light.bin" "%DIST%\" >nul
if exist "%ROOT%\models\kkc_model_s2.bin" copy /Y "%ROOT%\models\kkc_model_s2.bin" "%DIST%\" >nul
if exist "%ROOT%\models\kkc_model_m5.bin" copy /Y "%ROOT%\models\kkc_model_m5.bin" "%DIST%\" >nul
rem clean set (PureXS + clean dictionary, 2026-11-12): model, dictionary and variants go together
if exist "%ROOT%\models\kkc_model_pure.bin" xcopy /D /Y /Q "%ROOT%\models\kkc_model_pure.bin" "%DIST%\" >nul
if exist "%ROOT%\models\kkc_lex_clean.bin" xcopy /D /Y /Q "%ROOT%\models\kkc_lex_clean.bin" "%DIST%\" >nul
if exist "%ROOT%\models\kkc_variants_clean.tsv" xcopy /D /Y /Q "%ROOT%\models\kkc_variants_clean.tsv" "%DIST%\" >nul
rem voice input parts (optional, not in git): models\voice_trie.bin (辞書の読みの発音の木) and models\voice_e1.bin (音響モデル)
if exist "%ROOT%\models\voice_trie.bin" xcopy /D /Y /Q "%ROOT%\models\voice_trie.bin" "%DIST%\" >nul
if exist "%ROOT%\models\voice_e1.bin" xcopy /D /Y /Q "%ROOT%\models\voice_e1.bin" "%DIST%\" >nul
copy /Y "%HERE%install.ps1" "%DIST%\" >nul
copy /Y "%HERE%install.bat" "%DIST%\" >nul
copy /Y "%HERE%uninstall.bat" "%DIST%\" >nul
copy /Y "%HERE%README.txt" "%DIST%\" >nul
copy /Y "%HERE%LICENSE.txt" "%DIST%\" >nul
rem キー配列の表 (desktop\layouts\*.tsv -> layout_<名前>.tsv。インストール先の DLL の隣に置く)
for %%f in ("%ROOT%\desktop\layouts\*.tsv") do copy /Y "%%f" "%DIST%\layout_%%~nxf" >nul
echo built: %DIST%
exit /b 0

:build
setlocal
call "%VCVARS%\%2" >nul || exit /b 1
set OBJ=%HERE%..\build\obj_%1%SUFFIX%
if not exist "%OBJ%" mkdir "%OBJ%"
rem Rust switch for this arch: x64 -> x86_64-pc-windows-msvc, x86 -> i686-pc-windows-msvc.
rem CORE_AND_ENGINE is the legacy C++ core + engine list; empty under REXT_RUST=1.
rem The import library name (rext_kkc.dll.lib vs rext_kkc.lib) is not verified
rem under MSVC, so fall back to the other name when the first does not exist.
set RUSTTARGET=x86_64-pc-windows-msvc
if "%1"=="x86" set RUSTTARGET=i686-pc-windows-msvc
set CORE_AND_ENGINE="%CORE%\text.cpp" "%CORE%\romaji.cpp" "%CORE%\store.cpp" "%CORE%\settings.cpp" "%CORE%\converter.cpp" "%CORE%\style.cpp" "%CORE%\composer.cpp" "%CORE%\flow.cpp" "%CORE%\layout.cpp" "%CORE%\typo_alts.cpp" "%CORE%\datetime.cpp" "%ROOT%\entry\src\main\cpp\engine.cpp"
set REXTLIB=%ROOT%\rust\target\%RUSTTARGET%\release\rext_kkc.dll.lib
if not exist "%REXTLIB%" set REXTLIB=%ROOT%\rust\target\%RUSTTARGET%\release\rext_kkc.lib
set REXTLINK=
if "%REXT_RUST%"=="1" set CORE_AND_ENGINE=
if "%REXT_RUST%"=="1" set REXTLINK="%REXTLIB%"
rc /nologo %DEFS% /fo "%OBJ%\shunti_ime.res" "%HERE%shunti_ime.rc" || exit /b 1
cl /nologo /c /O2 /MT /EHsc /std:c++17 /utf-8 /W3 /GS /guard:cf /DNOMINMAX /DUNICODE /D_UNICODE /D_CRT_SECURE_NO_WARNINGS %DEFS% ^
  /I"%ROOT%\entry\src\main\cpp" /Fo"%OBJ%\\" ^
  "%HERE%dllmain.cpp" "%HERE%globals.cpp" "%HERE%text_service.cpp" "%HERE%cand_window.cpp" "%HERE%display_attr.cpp" "%HERE%langbar.cpp" ^
  "%HERE%input_log.cpp" "%HERE%strings.cpp" %CORE_AND_ENGINE% || exit /b 1
rem 変換エンジンの AVX2 版の計算だけは /arch:AVX2 で (使うのは CPU が対応しているときだけ。engine_simd.h を参照)
rem REXT_RUST=1: engine_avx2.cpp is inside the Rust engine (rext-core), do not compile it
if not "%REXT_RUST%"=="1" cl /nologo /c /O2 /MT /EHsc /std:c++17 /utf-8 /W3 /GS /guard:cf /arch:AVX2 %DEFS% /Fo"%OBJ%\\" ^
  "%ROOT%\entry\src\main\cpp\engine_avx2.cpp" || exit /b 1
link /nologo /DLL /guard:cf /DEF:"%HERE%shunti_ime.def" /OUT:"%DIST%\shunti_ime_%1.dll" "%OBJ%\*.obj" "%OBJ%\shunti_ime.res" %REXTLINK% ^
  ole32.lib oleaut32.lib uuid.lib advapi32.lib user32.lib gdi32.lib shell32.lib dwmapi.lib gdiplus.lib || exit /b 1
if "%REXT_RUST%"=="1" copy /Y "%ROOT%\rust\target\%RUSTTARGET%\release\rext_kkc.dll" "%DIST%\" >nul
del "%DIST%\shunti_ime_%1.exp" "%DIST%\shunti_ime_%1.lib" 2>nul
exit /b 0

:settings
rem NOTE (Rust switch): settings_app / setup below still compile desktop\core
rem (release_info, text, store, flow, settings) directly. Those are NOT switched:
rem Settings / StyleLog / store file IO have no C ABI in librext_kkc yet, and
rem this path is unverified under MSVC. They keep the legacy C++ objects in
rem both modes. kana_fix and voice_app link the x64 DLL objects (engine.obj
rem etc.), which do not exist under REXT_RUST=1, so they are skipped then.
rem settings app: plain Win32 controls (common controls v6 for the list view)
setlocal
call "%VCVARS%\vcvars64.bat" >nul || exit /b 1
set OBJ=%HERE%..\build\obj_settings%SUFFIX%
if not exist "%OBJ%" mkdir "%OBJ%"
rc /nologo %DEFS% /fo "%OBJ%\settings_app.res" "%HERE%settings_app.rc" || exit /b 1
cl /nologo /O2 /MT /EHsc /std:c++17 /utf-8 /W3 /GS /guard:cf /DNOMINMAX /DUNICODE /D_UNICODE /D_CRT_SECURE_NO_WARNINGS %DEFS% /Fo"%OBJ%\\" ^
  "%HERE%settings_app.cpp" "%HERE%strings.cpp" "%HERE%update.cpp" "%CORE%\release_info.cpp" "%CORE%\text.cpp" "%CORE%\store.cpp" "%CORE%\flow.cpp" "%CORE%\settings.cpp" /Fe"%DIST%\shunti_settings.exe" ^
  /link /SUBSYSTEM:WINDOWS /guard:cf "%OBJ%\settings_app.res" comctl32.lib comdlg32.lib shell32.lib ole32.lib user32.lib gdi32.lib winhttp.lib ^
  /MANIFEST:EMBED "/MANIFESTDEPENDENCY:type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'" || exit /b 1
rem one-shot learned.json repair run by install.ps1 (uses the x64 objects of the DLL)
set X64=%HERE%..\build\obj_x64%SUFFIX%
if not "%REXT_RUST%"=="1" cl /nologo /O2 /MT /EHsc /std:c++17 /utf-8 /W3 /GS /guard:cf /DNOMINMAX /DUNICODE /D_UNICODE /D_CRT_SECURE_NO_WARNINGS /I"%ROOT%\entry\src\main\cpp" /Fo"%OBJ%\\" ^
  "%HERE%kana_fix.cpp" "%X64%\text.obj" "%X64%\store.obj" "%X64%\converter.obj" "%X64%\style.obj" "%X64%\engine.obj" "%X64%\engine_avx2.obj" /Fe"%DIST%\shunti_kana_fix.exe" ^
  /link /SUBSYSTEM:WINDOWS /guard:cf shell32.lib ole32.lib || exit /b 1
rem web installer: downloads the latest Windows release from GitHub and runs install.ps1
cl /nologo /O2 /MT /EHsc /std:c++17 /utf-8 /W3 /GS /guard:cf /DNOMINMAX /DUNICODE /D_UNICODE /D_CRT_SECURE_NO_WARNINGS /Fo"%OBJ%\\" ^
  "%HERE%setup.cpp" "%HERE%strings.cpp" "%HERE%update.cpp" "%CORE%\release_info.cpp" "%CORE%\text.cpp" /Fe"%HERE%..\build\shunti-IME-setup.exe" ^
  /link /SUBSYSTEM:WINDOWS /guard:cf "%OBJ%\settings_app.res" shell32.lib user32.lib winhttp.lib ^
  /MANIFEST:EMBED "/MANIFESTDEPENDENCY:type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'" || exit /b 1
rem handwriting pad: Windows の日本語の手書き認識 (Windows.UI.Input.Inking、C++/WinRT) を呼ぶ小さな窓。モデルは持たない
rc /nologo %DEFS% /fo "%OBJ%\hand_app.res" "%HERE%hand_app.rc" || exit /b 1
cl /nologo /O2 /MT /EHsc /std:c++17 /utf-8 /W3 /GS /guard:cf /DNOMINMAX /DUNICODE /D_UNICODE /D_CRT_SECURE_NO_WARNINGS %DEFS% /Fo"%OBJ%\\" ^
  "%HERE%hand_app.cpp" "%HERE%strings.cpp" /Fe"%DIST%\shunti_hand.exe" ^
  /link /SUBSYSTEM:WINDOWS /guard:cf "%OBJ%\hand_app.res" windowsapp.lib user32.lib gdi32.lib || exit /b 1
rem voice input: 録音してこの PC の中で認識する小さな窓 (voice_app.cpp・voice_core.h)。変換は IME と同じエンジン
rc /nologo %DEFS% /fo "%OBJ%\voice_app.res" "%HERE%voice_app.rc" || exit /b 1
if not "%REXT_RUST%"=="1" cl /nologo /O2 /MT /EHsc /std:c++17 /utf-8 /W3 /GS /guard:cf /DNOMINMAX /DUNICODE /D_UNICODE /D_CRT_SECURE_NO_WARNINGS /D_USE_MATH_DEFINES %DEFS% /I"%ROOT%\entry\src\main\cpp" /Fo"%OBJ%\\" ^
  "%HERE%voice_app.cpp" "%HERE%strings.cpp" "%X64%\engine.obj" "%X64%\engine_avx2.obj" /Fe"%DIST%\shunti_voice.exe" ^
  /link /SUBSYSTEM:WINDOWS /guard:cf "%OBJ%\voice_app.res" user32.lib gdi32.lib winmm.lib shell32.lib ole32.lib || exit /b 1
rem clipboard history: 常駐してコピーを記録し、Zori-chan の右クリック・ショートカットで履歴の窓を出す (clip_app.cpp)
rc /nologo %DEFS% /fo "%OBJ%\clip_app.res" "%HERE%clip_app.rc" || exit /b 1
cl /nologo /O2 /MT /EHsc /std:c++17 /utf-8 /W3 /GS /guard:cf /DNOMINMAX /DUNICODE /D_UNICODE /D_CRT_SECURE_NO_WARNINGS %DEFS% /Fo"%OBJ%\\" ^
  "%HERE%clip_app.cpp" "%HERE%strings.cpp" /Fe"%DIST%\shunti_clip.exe" ^
  /link /SUBSYSTEM:WINDOWS /guard:cf "%OBJ%\clip_app.res" user32.lib gdi32.lib gdiplus.lib shell32.lib shlwapi.lib ole32.lib advapi32.lib || exit /b 1
exit /b 0

rem --- Rust library (only called when REXT_RUST=1) ------------------------------
:rust
setlocal
where cargo >nul 2>nul || (echo cargo not found in PATH ^(needed for REXT_RUST=1^) & exit /b 1)
cargo build --release --manifest-path "%ROOT%\rust\Cargo.toml" -p rext-kkc --target x86_64-pc-windows-msvc || exit /b 1
cargo build --release --manifest-path "%ROOT%\rust\Cargo.toml" -p rext-kkc --target i686-pc-windows-msvc || exit /b 1
exit /b 0
