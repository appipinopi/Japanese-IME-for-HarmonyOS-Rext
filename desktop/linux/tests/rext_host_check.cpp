// Rust ライブラリ (librext_kkc) のホスト検証。Fcitx5 非依存。
//
// 目的: desktop/tests/core_test.cpp のうち、Fcitx に依存せず、かつ Rust 側の
// 輸出 (kkc_* 52本 / rext_* 25本) に置換可能なチェックを、C++ から
// librext_kkc を直接リンクして実行する。期待値は core_test.cpp の
// ゴールデンと一致させてある (romaji / text / datetime / number)。
// 辞書・モデル (kkc_lex.bin / kkc_model.bin) はリポジトリ内に無いため、
// エンジン変換そのもの (kkc_convert 等) はここでは検証できない。
// その代わり kkc_open_files が存在しないファイルに対して安全に NULL を
// 返すことと、kkc_close(NULL) が安全なことを確認する。
//
// 戻り文字列はすべて rext_free_string で解放する (REXT_EXPORTS.md の規約)。
// 失敗があれば FAIL を出して終了コード 1。

#include <cstdio>
#include <cstring>
#include <string>
#include <vector>

extern "C" {
// rext_* (crates/rext-kkc/src/rext_exports.rs)
char* rext_romaji_to_kana(const char*);
char* rext_kana_to_romaji(const char*);
char* rext_to_katakana(const char*);
char* rext_to_hiragana(const char*);
char* rext_to_halfwidth_kana(const char*);
char* rext_to_fullwidth_ascii(const char*);
char* rext_to_halfwidth_ascii(const char*);
char* rext_number_add_commas(const char*);
char* rext_number_to_kanji(const char*);
char* rext_number_to_man_mixed(const char*);
char* rext_number_era_year(long long);
char* rext_number_forms(const char*);
char* rext_number_predict(const char*);
char* rext_datetime_predict(const char*, long long);
char* rext_datetime_predict_with_offset(const char*, long long, long long);
char* rext_bracket_closing_for(const char*);
char* rext_bracket_opening_for(const char*);
char* rext_bracket_auto_close(const char*);
char* rext_bracket_variants(const char*);
char* rext_bracket_opens();
char* rext_bracket_closes();
int rext_pos_class_count();
unsigned rext_pos_class(int);
unsigned rext_pos_group(int);
void rext_free_string(char*);
// kkc_* (engine.h の一部。辞書が無い条件での安全確認用)
struct kkc_engine;
kkc_engine* kkc_open_files(const char*, const char*);
void kkc_close(kkc_engine*);
}

static int failures = 0;
static int checks = 0;

// rext_* の戻り文字列を取り出して解放する。NULL はそのまま返す印に
// has_value=false。
struct Owned {
    bool has_value = false;
    std::string value;
};
static Owned take(char* p) {
    Owned o;
    if (p) {
        o.has_value = true;
        o.value = p;
        rext_free_string(p);
    }
    return o;
}

static void expect(const char* name, const Owned& got, const char* want) {
    checks++;
    if (got.has_value && got.value == want) {
        printf("ok   %s: %s\n", name, got.value.c_str());
        return;
    }
    printf("FAIL %s: got [%s] want [%s]\n", name, got.has_value ? got.value.c_str() : "(null)", want);
    failures++;
}

static void expect_null(const char* name, char* p) {
    checks++;
    if (!p) {
        printf("ok   %s: NULL\n", name);
        return;
    }
    printf("FAIL %s: got [%s] want NULL\n", name, p);
    rext_free_string(p);
    failures++;
}

static std::vector<std::string> split_list(const Owned& o) {
    std::vector<std::string> v;
    if (!o.has_value || o.value.empty()) return v;
    size_t at = 0;
    while (true) {
        size_t nl = o.value.find('\n', at);
        if (nl == std::string::npos) {
            v.push_back(o.value.substr(at));
            break;
        }
        v.push_back(o.value.substr(at, nl - at));
        at = nl + 1;
    }
    return v;
}

static void expect_at(const char* name, const std::vector<std::string>& v, size_t i, const char* want) {
    checks++;
    std::string got = i < v.size() ? v[i] : std::string();
    if (got == want) {
        printf("ok   %s: %s\n", name, got.c_str());
        return;
    }
    printf("FAIL %s: got [%s] want [%s]\n", name, got.c_str(), want);
    failures++;
}

static void expect_count(const char* name, const std::vector<std::string>& v, size_t want) {
    checks++;
    if (v.size() == want) {
        printf("ok   %s: %zu\n", name, v.size());
        return;
    }
    printf("FAIL %s: got %zu want %zu\n", name, v.size(), want);
    failures++;
}

static void expect_contains(const char* name, const std::vector<std::string>& v, const char* want) {
    checks++;
    for (auto& s : v)
        if (s == want) {
            printf("ok   %s: %s\n", name, want);
            return;
        }
    printf("FAIL %s: [%s] not in list\n", name, want);
    failures++;
}

int main() {
    // ---- ローマ字 (core_test.cpp の romaji 節と同じ期待値)
    expect("romaji konnnichiha", take(rext_romaji_to_kana("konnnichiha")), "こんにちは");
    expect("romaji kanji", take(rext_romaji_to_kana("kanji")), "かんじ");
    expect("romaji n'a", take(rext_romaji_to_kana("kan'i")), "かんい");
    expect("romaji tte", take(rext_romaji_to_kana("kitte")), "きって");
    expect("romaji ca", take(rext_romaji_to_kana("cocoa")), "ここあ");
    expect("romaji end n", take(rext_romaji_to_kana("hon")), "ほん");
    expect("to_romaji", take(rext_kana_to_romaji("とうきょうでしゅっぱつ")), "toukyoudeshuppatsu");
    expect("to_romaji n", take(rext_kana_to_romaji("かんい")), "kanni");

    // ---- テキスト変換 (core_test.cpp の halfwidth kana と同じ期待値を含む)
    expect("halfwidth kana", take(rext_to_halfwidth_kana("ガッコウ　パン")), "ｶﾞｯｺｳ ﾊﾟﾝ");
    expect("to katakana", take(rext_to_katakana("ねこ")), "ネコ");
    expect("to hiragana", take(rext_to_hiragana("ネコ")), "ねこ");
    expect("to fullwidth ascii", take(rext_to_fullwidth_ascii("abc123")), "ａｂｃ１２３");
    expect("to halfwidth ascii", take(rext_to_halfwidth_ascii("ａｂｃ１２３")), "abc123");

    // ---- 日付・時刻 (core_test.cpp と同じ固定時刻: 2026-10-17 (土) 14:05 JST)
    // rext_datetime_predict はローカル時刻 = UTC+9 固定なので、
    // now_unix は 2026-10-17 14:05 +09:00 = 1792213500。
    const long long now = 1792213500LL;
    auto ky = split_list(take(rext_datetime_predict("きょう", now)));
    expect_at("datetime きょう[0]", ky, 0, "2026年10月17日");
    expect_at("datetime きょう[1]", ky, 1, "2026年10月17日(土)");
    expect_at("datetime きょう[2]", ky, 2, "10月17日");
    expect_at("datetime きょう[9]", ky, 9, "令和8年10月17日");
    expect_at("datetime きょう[11]", ky, 11, "20261017");
    expect_count("datetime きょう count", ky, 12);
    auto asita = split_list(take(rext_datetime_predict("あした", now)));
    expect_at("datetime あした[0]", asita, 0, "2026年10月18日");
    auto ima = split_list(take(rext_datetime_predict("いま", now)));
    expect_at("datetime いま[0]", ima, 0, "14:05");
    expect_at("datetime いま[1]", ima, 1, "14時5分");
    expect_at("datetime いま[2]", ima, 2, "午後2時5分");
    expect_at("datetime いま[3]", ima, 3, "10月17日 14:05");
    // offset 明示版 (UTC): 同日 05:05
    auto ima_utc = split_list(take(rext_datetime_predict_with_offset("いま", now, 0)));
    expect_at("datetime いま UTC[0]", ima_utc, 0, "5:05");

    // ---- 数字 (core_test.cpp の number 節と同じ期待値を含む)
    auto nf2026 = split_list(take(rext_number_forms("2026")));
    expect_at("number 2026 era", nf2026, 0, "令和8年");
    expect_contains("number 2026 kanji", nf2026, "二千二十六");
    expect_contains("number 2026 commas", nf2026, "2,026");
    auto nf20 = split_list(take(rext_number_forms("20")));
    expect_contains("number 20 kanji", nf20, "二十");
    expect_contains("number 20 circled", nf20, "⑳");
    expect_contains("number 20 circled filled", nf20, "⓴");
    expect("number forms reject", take(rext_number_forms("abc")), "");
    expect("number add_commas", take(rext_number_add_commas("1234567")), "1,234,567");
    expect("number add_commas fullwidth", take(rext_number_add_commas("１２３４")), "1,234");
    expect_null("number add_commas reject", rext_number_add_commas("12a"));
    expect("number to_kanji", take(rext_number_to_kanji("12345678")), "千二百三十四万五千六百七十八");
    expect("number to_man_mixed", take(rext_number_to_man_mixed("12345678")), "1234万5678");
    expect("number era_year", take(rext_number_era_year(2026)), "令和8年");
    expect_null("number era_year before 1926", rext_number_era_year(1900));
    expect("number predict", take(rext_number_predict("1000000")), "1,000,000\n100万\n百万");

    // ---- 括弧
    expect("bracket closing", take(rext_bracket_closing_for("「")), "」");
    expect("bracket opening", take(rext_bracket_opening_for("」")), "「");
    expect_null("bracket closing unknown", rext_bracket_closing_for("x"));
    expect("bracket auto_close", take(rext_bracket_auto_close("「")), "」");
    expect_null("bracket auto_close <", rext_bracket_auto_close("<"));
    expect_count("bracket opens", split_list(take(rext_bracket_opens())), 24);
    expect_count("bracket closes", split_list(take(rext_bracket_closes())), 24);
    expect_count("bracket variants", split_list(take(rext_bracket_variants("かっこ"))), 24);

    // ---- 品詞
    checks++;
    if (rext_pos_class_count() == 2672 && rext_pos_class(0) == '0' && rext_pos_group(0) == 'O' &&
        rext_pos_class(-1) == '0' && rext_pos_group(-1) == 0)
        printf("ok   pos class/group\n");
    else {
        printf("FAIL pos class/group: count %d\n", rext_pos_class_count());
        failures++;
    }

    // ---- NULL 安全
    expect_null("null romaji", rext_romaji_to_kana(nullptr));
    expect_null("null number_forms", rext_number_forms(nullptr));
    rext_free_string(nullptr);  // 落ちないこと
    checks++;
    printf("ok   free_string(NULL)\n");

    // ---- kkc_* (辞書が無い条件: 安全に失敗することだけ確認。変換は未検証)
    checks++;
    kkc_engine* e = kkc_open_files("/nonexistent/kkc_lex.bin", "/nonexistent/kkc_model.bin");
    if (!e)
        printf("ok   kkc_open_files missing files -> NULL\n");
    else {
        printf("FAIL kkc_open_files missing files -> non-NULL\n");
        failures++;
        kkc_close(e);
    }
    kkc_close(nullptr);  // 落ちないこと
    checks++;
    printf("ok   kkc_close(NULL)\n");

    printf("rext_host_check: %d checks, %d failures\n", checks, failures);
    return failures ? 1 : 0;
}
