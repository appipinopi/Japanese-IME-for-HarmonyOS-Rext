package com.shunti.japaneseime

/**
 * 品詞の表 (desktop/core/pos_classes.h の POS_CLASSES・POS_GROUPS、辞書の左 ID ごとに 1 字。エンジンから 1 回だけ読む)。
 * 品詞ごとの学習 (PC 版と同じ。2026-11-09) で使う。品詞が分からない (-1) ときは表を読まない (単体試験でエンジンを読まずに済む)
 */
// 第3弾 (Rust 化): cls / group は Rust 版 rext_pos_* を先に試し、引けなければ上の表に落ちる。
// headLid は rext_* に輸出が無い (REXT_EXPORTS.md の未対応節) ので純実装のまま。
object Pos {
    init {
        runCatching { System.loadLibrary("kkc") }
    }

    // 第3弾 (Rust 化): rext_pos_* の JNI ブリッジ。コードポイントで返り、リンクが無ければ -1
    private external fun nativePosClassCount(): Int
    private external fun nativePosClass(lid: Int): Int
    private external fun nativePosGroup(lid: Int): Int

    private val tables: Array<String> by lazy { runCatching { Engine.posTables() }.getOrElse { arrayOf("", "") } }

    /** 文節の分類: 1 付属語・2 接頭・3 サ変接続の名詞・4 サ変・スルの動詞・0 自立語 */
    fun cls(lid: Int): Char {
        if (lid < 0) return '0'
        // Rust 版 (rext_pos_class) を先に。引けなければ従来の表に落ちる
        runCatching { nativePosClass(lid) }.getOrNull()?.let { if (it >= 0) return it.toChar() }
        return tables[0].getOrElse(lid) { '0' }
    }

    /** 学習を分ける粗い品詞 (N 名詞・V 動詞・P 助詞・X 助動詞 …。PC 版 group_of_lid)。分からなければ 0 */
    fun group(lid: Int): Char {
        if (lid < 0) return 0.toChar()
        // Rust 版 (rext_pos_group) を先に。引けなければ従来の表に落ちる
        runCatching { nativePosGroup(lid) }.getOrNull()?.let { if (it >= 0) return it.toChar() }
        return tables[1].getOrElse(lid) { 0.toChar() }
    }

    /** 文の語のうち、読みの位置 r0 から始まる文節の頭の語の品詞 (接頭詞は飛ばす。PC 版 head_lid)。無ければ -1 */
    fun headLid(ws: List<Engine.PosWord>, r0: Int): Int {
        var r = 0
        for (i in ws.indices) {
            if (r == r0) {
                var k = i
                while (k + 1 < ws.size && ws[k].lid >= 0 && cls(ws[k].lid) == '2') k++
                return ws[k].lid
            }
            if (r > r0) return -1
            r += ws[i].reading.length
        }
        return -1
    }
}
