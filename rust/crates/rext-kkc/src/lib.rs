//! rext-kkc: cdylib exporting the exact extern "C" kkc_* ABI declared in
//! entry/src/main/cpp/engine.h, so napi_kkc.cpp (HarmonyOS) and jni.cpp
//! (Android) can link against the Rust engine unchanged.
pub use rext_core::engine::ffi::*;
pub use rext_core::glide::ffi::*;

/// rext_* small-utility exports for ArkTS (NAPI) / Kotlin (JNI) callers.
/// See rext_exports.rs and REXT_EXPORTS.md.
pub mod rext_exports;
