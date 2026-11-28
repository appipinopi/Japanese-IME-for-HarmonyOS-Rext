//! rext-core: Japanese IME processing core, ported to Rust from the
//! original C++ (HarmonyOS/desktop), ArkTS and Kotlin implementations.
//! Duplicate logic that existed per-platform lives here exactly once.
pub mod brackets;
pub mod composer;
pub mod converter;
pub mod datetime;
pub mod engine;
pub mod flow;
pub mod glide;
pub mod layout;
pub mod number;
pub mod pos;
pub mod romaji;
pub mod settings;
pub mod store;
pub mod style;
pub mod tables;
pub mod text;
pub mod typo_alts;

/// UTF-16 string type used across the original code bases (std::u16string / String in ETS/Kotlin).
pub type U16 = Vec<u16>;

pub fn u16s(s: &str) -> U16 { s.encode_utf16().collect() }
pub fn from_u16s(v: &[u16]) -> String { String::from_utf16_lossy(v) }
