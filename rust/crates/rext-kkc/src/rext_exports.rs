//! rext_*: small-utility exports for the ArkTS (NAPI) / Kotlin (JNI) layers.
//!
//! The kkc_* ABI (engine.h) only exposes the conversion engine itself. The
//! per-platform utility logic that used to be duplicated in ArkTS/Kotlin
//! (JapaneseConverter, NumberFormatter, DateTimePredictor, Brackets, Pos)
//! already lives once in rext-core; this module re-exports it over a C ABI
//! so napi_kkc.cpp / jni.cpp can forward to it.
//!
//! Conventions (consistent with the kkc_* ABI in engine.rs / glide.rs):
//! - Text in/out is UTF-8, NUL-terminated `char*` — the same representation
//!   kkc_open_files / kkc_glide_set_words already use for `c_char` strings.
//!   (rext-core works in UTF-16 internally; conversion happens here.)
//! - Every returned `char*` is heap-allocated by Rust and ownership passes
//!   to the caller, who must release it exactly once with
//!   [`rext_free_string`]. Never free it with free()/delete[].
//! - Candidate-list functions return the candidates joined by '\n'
//!   (candidates never contain '\n'). An empty list is an allocated empty
//!   string "", not NULL.
//! - NULL input is always safe: string functions return NULL, scalar
//!   functions return 0. `rext_free_string(NULL)` is a no-op.
//! - Single-value functions that can fail (invalid digits, unknown
//!   bracket, out-of-era year) return NULL instead of an empty string.

use std::ffi::{CStr, CString};
use std::os::raw::c_char;

use rext_core::{brackets, datetime, from_u16s, number, pos, text, u16s, U16};

/// Read a NUL-terminated UTF-8 input string. None on NULL.
/// Lossy conversion mirrors kkc_open_files' handling of c_char paths.
unsafe fn read_input(p: *const c_char) -> Option<String> {
    if p.is_null() {
        None
    } else {
        Some(unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
    }
}

/// Hand a Rust string to the caller as an owned C string.
fn out(s: String) -> *mut c_char {
    match CString::new(s) {
        Ok(cs) => cs.into_raw(),
        // Interior NUL cannot occur for our generated values; fail safe.
        Err(_) => std::ptr::null_mut(),
    }
}

fn out_u16(v: &[u16]) -> *mut c_char {
    out(from_u16s(v))
}

/// Join a candidate list with '\n' (empty list -> allocated "").
fn out_list(v: &[U16]) -> *mut c_char {
    let joined = v
        .iter()
        .map(|s| from_u16s(s))
        .collect::<Vec<_>>()
        .join("\n");
    out(joined)
}

/// Release a string returned by any rext_* function. NULL is a no-op.
/// Must be called exactly once per returned pointer.
#[no_mangle]
pub extern "C" fn rext_free_string(p: *mut c_char) {
    if !p.is_null() {
        unsafe { drop(CString::from_raw(p)) };
    }
}

// ---------------------------------------------------------------- romaji

/// Romaji input (whole string, stateless) -> kana.
/// Feeds every char through a fresh rext_core::romaji::Romaji and appends
/// the flush() result, e.g. "neko" -> "ねこ", "kan" -> "かん".
#[no_mangle]
pub extern "C" fn rext_romaji_to_kana(input: *const c_char) -> *mut c_char {
    let Some(s) = (unsafe { read_input(input) }) else {
        return std::ptr::null_mut();
    };
    let mut r = rext_core::romaji::Romaji::new();
    let mut kana: U16 = Vec::new();
    for c in u16s(&s) {
        kana.extend_from_slice(&r.process(c).committed);
    }
    kana.extend_from_slice(&r.flush());
    out_u16(&kana)
}

/// Kana -> romaji (rext_core::romaji::Romaji::to_romaji),
/// e.g. "とうきょう" -> "toukyou", "がっこう" -> "gakkou".
#[no_mangle]
pub extern "C" fn rext_kana_to_romaji(input: *const c_char) -> *mut c_char {
    let Some(s) = (unsafe { read_input(input) }) else {
        return std::ptr::null_mut();
    };
    let r = rext_core::romaji::Romaji::new();
    out_u16(&r.to_romaji(&u16s(&s)))
}

// ------------------------------------------------------------------ text

/// Hiragana -> katakana (rext_core::text::to_katakana).
#[no_mangle]
pub extern "C" fn rext_to_katakana(input: *const c_char) -> *mut c_char {
    let Some(s) = (unsafe { read_input(input) }) else {
        return std::ptr::null_mut();
    };
    out_u16(&text::to_katakana(&u16s(&s)))
}

/// Katakana -> hiragana (rext_core::text::to_hiragana).
#[no_mangle]
pub extern "C" fn rext_to_hiragana(input: *const c_char) -> *mut c_char {
    let Some(s) = (unsafe { read_input(input) }) else {
        return std::ptr::null_mut();
    };
    out_u16(&text::to_hiragana(&u16s(&s)))
}

/// Kana -> halfwidth kana (rext_core::text::to_halfwidth_kana).
#[no_mangle]
pub extern "C" fn rext_to_halfwidth_kana(input: *const c_char) -> *mut c_char {
    let Some(s) = (unsafe { read_input(input) }) else {
        return std::ptr::null_mut();
    };
    out_u16(&text::to_halfwidth_kana(&u16s(&s)))
}

/// ASCII -> fullwidth ASCII (rext_core::text::to_fullwidth_ascii).
#[no_mangle]
pub extern "C" fn rext_to_fullwidth_ascii(input: *const c_char) -> *mut c_char {
    let Some(s) = (unsafe { read_input(input) }) else {
        return std::ptr::null_mut();
    };
    out_u16(&text::to_fullwidth_ascii(&u16s(&s)))
}

/// Fullwidth ASCII -> ASCII (rext_core::text::to_halfwidth_ascii).
#[no_mangle]
pub extern "C" fn rext_to_halfwidth_ascii(input: *const c_char) -> *mut c_char {
    let Some(s) = (unsafe { read_input(input) }) else {
        return std::ptr::null_mut();
    };
    out_u16(&text::to_halfwidth_ascii(&u16s(&s)))
}

// ---------------------------------------------------------------- number

/// Digits with thousands separators (rext_core::number::add_commas after
/// normalize_digits; fullwidth digits accepted). NULL if not all digits.
#[no_mangle]
pub extern "C" fn rext_number_add_commas(input: *const c_char) -> *mut c_char {
    let Some(s) = (unsafe { read_input(input) }) else {
        return std::ptr::null_mut();
    };
    match number::normalize_digits(&u16s(&s)) {
        Some(d) => out_u16(&number::add_commas(&d)),
        None => std::ptr::null_mut(),
    }
}

/// Digits -> kanji numerals (rext_core::number::to_kanji).
/// NULL if not all digits.
#[no_mangle]
pub extern "C" fn rext_number_to_kanji(input: *const c_char) -> *mut c_char {
    let Some(s) = (unsafe { read_input(input) }) else {
        return std::ptr::null_mut();
    };
    match number::normalize_digits(&u16s(&s)) {
        Some(d) => out_u16(&number::to_kanji(&d)),
        None => std::ptr::null_mut(),
    }
}

/// Digits -> mixed man-style (rext_core::number::to_man_mixed),
/// e.g. "12345678" -> "1234万5678". NULL if not all digits.
#[no_mangle]
pub extern "C" fn rext_number_to_man_mixed(input: *const c_char) -> *mut c_char {
    let Some(s) = (unsafe { read_input(input) }) else {
        return std::ptr::null_mut();
    };
    match number::normalize_digits(&u16s(&s)) {
        Some(d) => out_u16(&number::to_man_mixed(&d)),
        None => std::ptr::null_mut(),
    }
}

/// Gregorian year -> Japanese era year (rext_core::number::era_year),
/// e.g. 2026 -> "令和8年". NULL before 1926 (no era defined).
#[no_mangle]
pub extern "C" fn rext_number_era_year(year: i64) -> *mut c_char {
    let v = number::era_year(year);
    if v.is_empty() {
        std::ptr::null_mut()
    } else {
        out_u16(&v)
    }
}

/// All rewritten forms of a digit-only reading, '\n'-joined
/// (rext_core::number::number_forms: era year, commas, man-mixed, kanji,
/// circled numbers). Empty string when there are no forms.
#[no_mangle]
pub extern "C" fn rext_number_forms(input: *const c_char) -> *mut c_char {
    let Some(s) = (unsafe { read_input(input) }) else {
        return std::ptr::null_mut();
    };
    out_list(&number::number_forms(&u16s(&s)))
}

/// Smartphone NumberFormatter.predict equivalent, '\n'-joined
/// (rext_core::number::predict; digit readings of 4+ digits only).
#[no_mangle]
pub extern "C" fn rext_number_predict(input: *const c_char) -> *mut c_char {
    let Some(s) = (unsafe { read_input(input) }) else {
        return std::ptr::null_mut();
    };
    out_list(&number::predict(&u16s(&s)))
}

// -------------------------------------------------------------- datetime

/// Date/time candidates for a reading, '\n'-joined
/// (rext_core::datetime::datetime_predict; local time = UTC+9).
/// now_unix is seconds since the UNIX epoch.
#[no_mangle]
pub extern "C" fn rext_datetime_predict(input: *const c_char, now_unix: i64) -> *mut c_char {
    let Some(s) = (unsafe { read_input(input) }) else {
        return std::ptr::null_mut();
    };
    out_list(&datetime::datetime_predict(&u16s(&s), now_unix))
}

/// Same as rext_datetime_predict with an explicit UTC offset in seconds
/// (rext_core::datetime::datetime_predict_with_offset).
#[no_mangle]
pub extern "C" fn rext_datetime_predict_with_offset(
    input: *const c_char,
    now_unix: i64,
    offset_seconds: i64,
) -> *mut c_char {
    let Some(s) = (unsafe { read_input(input) }) else {
        return std::ptr::null_mut();
    };
    out_list(&datetime::datetime_predict_with_offset(
        &u16s(&s),
        now_unix,
        offset_seconds,
    ))
}

// -------------------------------------------------------------- brackets

/// Closing bracket for an opening bracket (rext_core::brackets::closing_for).
/// NULL if the input is not a known opening bracket.
#[no_mangle]
pub extern "C" fn rext_bracket_closing_for(input: *const c_char) -> *mut c_char {
    let Some(s) = (unsafe { read_input(input) }) else {
        return std::ptr::null_mut();
    };
    match brackets::closing_for(&u16s(&s)) {
        Some(v) => out_u16(&v),
        None => std::ptr::null_mut(),
    }
}

/// Opening bracket for a closing bracket (rext_core::brackets::opening_for).
/// NULL if the input is not a known closing bracket.
#[no_mangle]
pub extern "C" fn rext_bracket_opening_for(input: *const c_char) -> *mut c_char {
    let Some(s) = (unsafe { read_input(input) }) else {
        return std::ptr::null_mut();
    };
    match brackets::opening_for(&u16s(&s)) {
        Some(v) => out_u16(&v),
        None => std::ptr::null_mut(),
    }
}

/// Closing bracket to insert automatically for an opening bracket
/// (rext_core::brackets::auto_close; NULL for ＜ < ‘ and unknown input).
#[no_mangle]
pub extern "C" fn rext_bracket_auto_close(input: *const c_char) -> *mut c_char {
    let Some(s) = (unsafe { read_input(input) }) else {
        return std::ptr::null_mut();
    };
    match brackets::auto_close(&u16s(&s)) {
        Some(v) => out_u16(&v),
        None => std::ptr::null_mut(),
    }
}

/// Bracket/symbol candidates for a reading, '\n'-joined
/// (rext_core::brackets::bracket_variants).
#[no_mangle]
pub extern "C" fn rext_bracket_variants(input: *const c_char) -> *mut c_char {
    let Some(s) = (unsafe { read_input(input) }) else {
        return std::ptr::null_mut();
    };
    out_list(&brackets::bracket_variants(&u16s(&s)))
}

/// All opening brackets (24), '\n'-joined (rext_core::brackets::opens).
#[no_mangle]
pub extern "C" fn rext_bracket_opens() -> *mut c_char {
    out_list(&brackets::opens())
}

/// All closing brackets (24), '\n'-joined (rext_core::brackets::closes).
#[no_mangle]
pub extern "C" fn rext_bracket_closes() -> *mut c_char {
    out_list(&brackets::closes())
}

// ------------------------------------------------------------------- pos

/// Number of POS ids in the tables (rext_core::pos::POS_CLASS_COUNT = 2672).
#[no_mangle]
pub extern "C" fn rext_pos_class_count() -> i32 {
    pos::POS_CLASS_COUNT as i32
}

/// Bunsetsu class of a dictionary left-id, as a code point:
/// '0'..'4' (rext_core::pos::pos_class). Out-of-range ids give '0' (48).
#[no_mangle]
pub extern "C" fn rext_pos_class(lid: i32) -> u32 {
    pos::pos_class(lid) as u32
}

/// Coarse POS group of a dictionary left-id, as a code point
/// (rext_core::pos::group_of_lid): 'N','V','P', ...; 0 when unknown.
#[no_mangle]
pub extern "C" fn rext_pos_group(lid: i32) -> u32 {
    pos::group_of_lid(lid) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    /// Call a rext_* string function, copy the result out, free it.
    unsafe fn call(
        f: unsafe extern "C" fn(*const c_char) -> *mut c_char,
        s: &str,
    ) -> Option<String> {
        let cs = CString::new(s).unwrap();
        let p = f(cs.as_ptr());
        if p.is_null() {
            return None;
        }
        let r = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
        rext_free_string(p);
        Some(r)
    }

    #[test]
    fn romaji_round_trip() {
        unsafe {
            assert_eq!(call(rext_romaji_to_kana, "neko").as_deref(), Some("ねこ"));
            assert_eq!(call(rext_romaji_to_kana, "kan").as_deref(), Some("かん"));
            assert_eq!(
                call(rext_romaji_to_kana, "toukyou").as_deref(),
                Some("とうきょう")
            );
            assert_eq!(
                call(rext_kana_to_romaji, "とうきょう").as_deref(),
                Some("toukyou")
            );
            assert_eq!(
                call(rext_kana_to_romaji, "がっこう").as_deref(),
                Some("gakkou")
            );
        }
    }

    #[test]
    fn text_conversions() {
        unsafe {
            assert_eq!(call(rext_to_katakana, "ねこ").as_deref(), Some("ネコ"));
            assert_eq!(call(rext_to_hiragana, "ネコ").as_deref(), Some("ねこ"));
            assert_eq!(
                call(rext_to_halfwidth_kana, "ネコ").as_deref(),
                Some("ﾈｺ")
            );
            assert_eq!(
                call(rext_to_fullwidth_ascii, "abc123").as_deref(),
                Some("ａｂｃ１２３")
            );
            assert_eq!(
                call(rext_to_halfwidth_ascii, "ａｂｃ１２３").as_deref(),
                Some("abc123")
            );
        }
    }

    #[test]
    fn number_functions() {
        unsafe {
            assert_eq!(
                call(rext_number_add_commas, "1234567").as_deref(),
                Some("1,234,567")
            );
            assert_eq!(
                call(rext_number_add_commas, "１２３４").as_deref(),
                Some("1,234")
            );
            assert_eq!(call(rext_number_add_commas, "12a"), None);
            assert_eq!(
                call(rext_number_to_kanji, "12345678").as_deref(),
                Some("千二百三十四万五千六百七十八")
            );
            assert_eq!(
                call(rext_number_to_man_mixed, "12345678").as_deref(),
                Some("1234万5678")
            );
            let era = rext_number_era_year(2026);
            assert!(!era.is_null());
            assert_eq!(CStr::from_ptr(era).to_str().unwrap(), "令和8年");
            rext_free_string(era);
            assert!(rext_number_era_year(1900).is_null());
            assert_eq!(
                call(rext_number_predict, "1000000").as_deref(),
                Some("1,000,000\n100万\n百万")
            );
            let forms = call(rext_number_forms, "2026").unwrap();
            assert!(forms.contains("令和8年"), "forms={forms}");
            assert_eq!(call(rext_number_forms, "abc").as_deref(), Some(""));
        }
    }

    #[test]
    fn datetime_functions() {
        // 2026-10-21 (水) 12:34 JST — same fixed instant as rext-core tests.
        const NOW: i64 = 1792553640;
        unsafe fn call_now(
            f: unsafe extern "C" fn(*const c_char, i64) -> *mut c_char,
            s: &str,
        ) -> Option<String> {
            let cs = CString::new(s).unwrap();
            let p = f(cs.as_ptr(), NOW);
            if p.is_null() {
                return None;
            }
            let r = CStr::from_ptr(p).to_string_lossy().into_owned();
            rext_free_string(p);
            Some(r)
        }
        unsafe {
        let today = call_now(rext_datetime_predict, "きょう").unwrap();
        let lines: Vec<&str> = today.split('\n').collect();
        assert_eq!(lines[0], "2026年10月21日");
        assert_eq!(lines[1], "2026年10月21日(水)");
        // 末尾は数字 8 桁 (現行 DateTimePredictor.ets の dateForms 12 形目。
        // 2026-11-28 ユーザー再決定で現行 .ets を正とした)
        assert_eq!(*lines.last().unwrap(), "20261021");
        assert_eq!(
            call_now(rext_datetime_predict, "ことし").as_deref(),
            Some("2026年\n令和8年")
        );
        assert_eq!(call_now(rext_datetime_predict, "あいうえお").as_deref(), Some(""));
        // UTC では 03:34 で同日なので先頭は同じ日付。
        let cs = CString::new("きょう").unwrap();
        let p = rext_datetime_predict_with_offset(cs.as_ptr(), NOW, 0);
        assert!(!p.is_null());
        let utc = CStr::from_ptr(p).to_string_lossy().into_owned();
        rext_free_string(p);
        assert!(utc.starts_with("2026年10月21日"), "utc={utc}");
        }
    }

    #[test]
    fn bracket_functions() {
        unsafe {
            assert_eq!(call(rext_bracket_closing_for, "「").as_deref(), Some("」"));
            assert_eq!(call(rext_bracket_opening_for, "」").as_deref(), Some("「"));
            assert_eq!(call(rext_bracket_closing_for, "x"), None);
            assert_eq!(call(rext_bracket_auto_close, "「").as_deref(), Some("」"));
            assert_eq!(call(rext_bracket_auto_close, "<"), None);
            let opens = {
                let p = rext_bracket_opens();
                assert!(!p.is_null());
                let s = CStr::from_ptr(p).to_string_lossy().into_owned();
                rext_free_string(p);
                s
            };
            assert_eq!(opens.split('\n').count(), 24);
            assert_eq!(opens.split('\n').next(), Some("（"));
            let pairs = call(rext_bracket_variants, "かっこ").unwrap();
            assert_eq!(pairs.split('\n').count(), 24);
            assert_eq!(pairs.split('\n').next(), Some("（）"));
        }
    }

    #[test]
    fn pos_functions() {
        assert_eq!(rext_pos_class_count(), 2672);
        assert_eq!(rext_pos_class(0), '0' as u32);
        assert_eq!(rext_pos_class(-1), '0' as u32);
        assert_eq!(rext_pos_class(99999), '0' as u32);
        assert_eq!(rext_pos_group(0), 'O' as u32);
        assert_eq!(rext_pos_group(-1), 0);
        assert_eq!(rext_pos_group(99999), 0);
    }

    #[test]
    fn null_safety() {
        unsafe {
            assert!(rext_romaji_to_kana(std::ptr::null()).is_null());
            assert!(rext_kana_to_romaji(std::ptr::null()).is_null());
            assert!(rext_to_katakana(std::ptr::null()).is_null());
            assert!(rext_number_add_commas(std::ptr::null()).is_null());
            assert!(rext_number_forms(std::ptr::null()).is_null());
            assert!(rext_datetime_predict(std::ptr::null(), 0).is_null());
            assert!(rext_bracket_closing_for(std::ptr::null()).is_null());
            assert!(rext_bracket_variants(std::ptr::null()).is_null());
            rext_free_string(std::ptr::null_mut()); // must not crash
        }
    }
}
