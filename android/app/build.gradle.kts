import java.util.Properties

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

// shunti IME の Android 版 (AI 変換だけ)。外部ライブラリなし。
// - 変換エンジンは Rust 版 (../../rust の rext-kkc、kkc_* ABI は engine.h と互換) を使う (buildRustKkc + buildKkc、詳細は rust/phase3/B-android.md)
//   構成は 2 層: librext_kkc.so (Rust、kkc_* 52 本) + libkkc.so (jni.cpp だけをコンパイルした薄い JNI 層、librext_kkc にリンク)
//   Engine.kt は System.loadLibrary("kkc") のまま。libkkc.so の DT_NEEDED 経由で librext_kkc.so も同じ jniLibs から読み込まれる
// - 辞書とモデルも HarmonyOS 版と同じもの (../entry/src/main/resources/rawfile/kkc_*.bin。python tools/fetch_ai_assets.py で取得)
//   を APK に無圧縮で同梱する (copyAiAssets)。端末ではそのままメモリに写して使うので、インターネット権限は要らない
// - キー配置・記号・絵文字などの表は HarmonyOS 版のソースから生成したもの (node tools/gen_android_tables.mjs)
val repo = rootDir.parentFile
val engineDir = File(repo, "entry/src/main/cpp")
val rawfile = File(repo, "entry/src/main/resources/rawfile")
val ndkVer = "27.2.12479018"

android {
    namespace = "com.shunti.japaneseime"
    compileSdk = 36
    ndkVersion = ndkVer

    defaultConfig {
        // AppGallery では HarmonyOS 版 (bundleName com.shunti.japaneseime) と同じ名前を使えないので .android を足す。
        // コードの中の名前 (namespace・Kotlin のパッケージ・JNI の関数名) は com.shunti.japaneseime のまま
        applicationId = "com.shunti.japaneseime.android"
        minSdk = 26
        targetSdk = 36
        versionCode = 12
        versionName = "1.3.7"
        ndk { abiFilters += listOf("arm64-v8a") }
    }
    // リリースの署名: android/keystore.properties (git には入れない) があれば使う
    val ks = rootProject.file("keystore.properties")
    if (ks.exists()) {
        val p = Properties().apply { ks.inputStream().use { load(it) } }
        signingConfigs {
            create("release") {
                storeFile = rootProject.file(p.getProperty("storeFile"))
                storePassword = p.getProperty("storePassword")
                keyAlias = p.getProperty("keyAlias")
                keyPassword = p.getProperty("keyPassword")
            }
        }
    }
    buildTypes {
        release {
            isMinifyEnabled = false
            if (ks.exists()) signingConfig = signingConfigs.getByName("release")
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_11
        targetCompatibility = JavaVersion.VERSION_11
    }
    kotlinOptions {
        jvmTarget = "11"
    }
    // 辞書とモデルは圧縮しない (端末でそのままメモリに写して使う)
    androidResources {
        noCompress += listOf("bin", "ttf")
    }
    packaging {
        jniLibs { useLegacyPackaging = false }
    }
    sourceSets["main"].jniLibs.srcDir(layout.buildDirectory.dir("kkc/jniLibs"))
    sourceSets["main"].assets.srcDir(layout.buildDirectory.dir("kkc/assets"))
}

// NDK のツールチェーンの場所 (buildRustKkc のリンカと buildKkc の clang で共用)
val ndkOs = System.getProperty("os.name").lowercase()
val ndkHost = when {
    ndkOs.contains("win") -> "windows-x86_64"
    ndkOs.contains("mac") -> "darwin-x86_64"
    else -> "linux-x86_64"
}
val ndkBin = File(android.sdkDirectory, "ndk/$ndkVer/toolchains/llvm/prebuilt/$ndkHost/bin")
val rustDir = File(repo, "rust")
val jniLibsDir = layout.buildDirectory.dir("kkc/jniLibs/arm64-v8a")

// librext_kkc.so を cargo で作って jniLibs に取り込む。
// - 事前に 1 回だけ `rustup target add aarch64-linux-android` が要る (未導入だと cargo がエラーにする)
// - リンカは .cargo/config.toml には書かない (NDK の場所はマシンごとに違う)。Gradle が知っている SDK の場所から
//   CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER で渡す。NDK の clang をリンカに使うのは、Rust 標準ライブラリが
//   要る Android の sysroot (-lc / -lm / -ldl / -llog / -lunwind) を解決するため (rust-lld 単体ではリンクできない)
val buildRustKkc by tasks.registering(Exec::class) {
    val cargoOut = File(rustDir, "target/aarch64-linux-android/release/librext_kkc.so")
    val out = jniLibsDir.map { it.file("librext_kkc.so") }
    inputs.dir(File(rustDir, "crates"))
    inputs.files(File(rustDir, "Cargo.toml"), File(rustDir, "Cargo.lock"))
    outputs.files(cargoOut, out)
    workingDir(rustDir)
    val linker = File(ndkBin, if (ndkOs.contains("win")) "aarch64-linux-android26-clang.cmd" else "aarch64-linux-android26-clang")
    val cargoHome = File(System.getProperty("user.home"), ".cargo/bin/cargo")
    doFirst {
        if (!linker.exists()) throw GradleException("NDK の clang がありません ($linker)。NDK $ndkVer を入れてください")
        out.get().asFile.parentFile.mkdirs()
    }
    environment("CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER", linker.path)
    executable = if (cargoHome.exists()) cargoHome.path else "cargo"
    args("build", "--release", "--target", "aarch64-linux-android", "-p", "rext-kkc")
    doLast {
        // Gradle の Copy タスクに分けず、ここでそのまま写す (出力先は outputs で宣言済み)
        java.nio.file.Files.copy(
            cargoOut.toPath(), out.get().asFile.toPath(),
            java.nio.file.StandardCopyOption.REPLACE_EXISTING,
        )
    }
}

// libkkc.so (JNI 層だけ) を NDK の clang で作る。CMake は使わない (入っていなくてよい)
// engine.cpp はもうコンパイルしない。jni.cpp が要るのは engine.h / ai_learn.h / phrase_ends.h / pos_classes.h の
// ヘッダ (kkc_phrase_ends と kkc_ai::* はヘッダ内 inline) と、librext_kkc.so の kkc_* シンボルだけ。
// -Wl,--no-undefined で、Rust 側に無い kkc_* があればリンク時に落とす (実行時に落ちるのを防ぐ)
val buildKkc by tasks.registering(Exec::class) {
    dependsOn(buildRustKkc)
    val out = jniLibsDir.map { it.file("libkkc.so") }
    val jniCpp = file("src/main/cpp/jni.cpp")
    inputs.files(jniCpp, File(engineDir, "engine.h"), File(engineDir, "ai_learn.h"), File(engineDir, "phrase_ends.h"), File(engineDir, "../../../../desktop/core/pos_classes.h"))
    inputs.file(jniLibsDir.map { it.file("librext_kkc.so") })
    outputs.file(out)
    val exe = if (ndkOs.contains("win")) "clang++.exe" else "clang++"
    doFirst { out.get().asFile.parentFile.mkdirs() }
    executable = File(ndkBin, exe).path
    args(
        "--target=aarch64-linux-android26", "-O3", "-std=c++17", "-fPIC", "-ffp-contract=fast", "-fno-math-errno",
        "-fvisibility=hidden", "-DNDEBUG", "-I", engineDir.path, "-shared", "-static-libstdc++",
        jniCpp.path,
        "-L", out.get().asFile.parentFile.path, "-lrext_kkc",
        "-landroid", "-llog", "-Wl,--gc-sections", "-Wl,--no-undefined", "-s", "-o", out.get().asFile.path,
    )
}

// 辞書とモデルを HarmonyOS 版の rawfile から取り込む (変わったときだけ写す)
val copyAiAssets by tasks.registering(Copy::class) {
    val names = listOf("kkc_lex.bin", "kkc_model.bin")
    doFirst {
        val missing = names.filter { !File(rawfile, it).exists() }
        if (missing.isNotEmpty()) throw GradleException("辞書とモデルがありません ($missing)。リポジトリの直下で python tools/fetch_ai_assets.py を実行してください")
    }
    from(rawfile) { include(names) }
    // キーボード・候補のフォント (HarmonyOS 版と同じ 2 本。tools/build_ud_font.py で作る。JpFont.kt)
    from(rawfile) { include("font/BIZUDPGothic-Regular.ttf", "font/NotoSansJP-Fallback.ttf") }
    // 軽量の変換モデル (設定で選ぶ。リポジトリの models/、tools/fetch_ai_assets.py で取得。無ければ標準だけ)
    from(File(repo, "models")) { include("kkc_model_light.bin") }
    into(layout.buildDirectory.dir("kkc/assets"))
}

tasks.named("preBuild") { dependsOn(buildKkc, copyAiAssets) }

dependencies {
    // テストだけ (APK には入らない)。GoldenTest で HarmonyOS 版と答え合わせをする
    testImplementation("junit:junit:4.13.2")
    testImplementation("org.json:json:20240303")
}
