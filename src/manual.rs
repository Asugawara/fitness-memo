//! 使い方マニュアルの章立てと図の検証。**ターゲット非依存**（`leptos` も `web_sys` も
//! import しない。`chart_layout.rs` / `reorder.rs` と同じ切り方）。
//!
//! 文言（`Manual` / `ChapterText` / `JA_MANUAL` / `EN_MANUAL`）は `src/i18n.rs` にある。
//! ここに置くのは言語に依存しない章の骨格 [`Chapter`] / [`MANUAL_CHAPTERS`] と、
//! 図の WebP ファイルを検証する [`webp_size`] だけ — どちらも文言ではないので
//! `adr/architecture/i18n-hand-rolled-string-table.md` の役割分担から見て `i18n.rs` の
//! 外に置くのが正しい。

/// マニュアルの 1 章。**言語非依存の骨格だけ**（見出し・本文は `i18n.rs` の `ChapterText`）。
pub struct Chapter {
    /// DOM id / testid / 図のファイル名の基底（`"man-"` を前置して使う）。ASCII のスラッグ。
    pub id: &'static str,
    /// この章が図を持つか。
    ///
    /// ★ **このコミットでは全章 `false`。** 撮影がまだ無いので図を持たせられない
    ///   （`.githooks/pre-commit` の `cargo test` が毎コミット走るため、`fig: None` の
    ///   まま `has_fig: true` にすると `ChapterText` 側との不一致で即座にテストが落ちる）。
    ///   図とその実測寸法はコミット #4 で足す。そのとき `chart-readout` / `copy-last` /
    ///   `exercise-memo` / `empty-day` / `accordions` の 5 章が `true` に変わり、
    ///   `progress-target` / `reorder` / `backup` の 3 章は `false` のまま残る。
    pub has_fig: bool,
}

/// マニュアルの章立て。8 章、`i18n.rs` の `Manual::chapters` と同じ順・同じ長さ
/// （`every_language_has_the_same_manual_chapters` が突き合わせる）。
pub const MANUAL_CHAPTERS: &[Chapter] = &[
    Chapter {
        id: "progress-target",
        has_fig: false,
    },
    Chapter {
        id: "chart-readout",
        has_fig: false,
    },
    Chapter {
        id: "copy-last",
        has_fig: false,
    },
    Chapter {
        id: "exercise-memo",
        has_fig: false,
    },
    Chapter {
        id: "empty-day",
        has_fig: false,
    },
    Chapter {
        id: "accordions",
        has_fig: false,
    },
    Chapter {
        id: "reorder",
        has_fig: false,
    },
    Chapter {
        id: "backup",
        has_fig: false,
    },
];

/// 3 バイト LE を `u32` にする（`VP8X` の canvas 寸法・`VP8L` の内包値の下地）。
fn u24_le(b0: u8, b1: u8, b2: u8) -> u32 {
    u32::from(b0) | u32::from(b1) << 8 | u32::from(b2) << 16
}

/// WebP 1 枚の宣言寸法を読む。**チャンク走査で書く。**
///
/// Playwright が吐く WebP は常に拡張形式 `VP8X(10) + ICCP(456) + VP8L`/`"VP8 "` になる
/// （`--force-color-profile=srgb` で sRGB プロファイルが挟まるため）。byte 20 は `VP8X`
/// の flags であって `VP8L` の signature（`0x2f`）ではないので、offset 固定では読めない。
/// `VP8X` チャンクがあればその canvas 寸法（値 - 1 が 3 バイト LE で 2 つ）を採り、
/// 無ければ `VP8L`（lossless）/ `"VP8 "`（lossy）のビットストリームヘッダを読む。
/// `quality` の指定でどちらを吐くか変わるので両方を扱う。
///
/// **コミット #4 のテスト（`every_manual_figure_exists_with_the_declared_size`）が使う。**
/// `include_bytes!` は使わない — 非 cfg モジュールなので使うと図が wasm に埋まる。
/// 呼び側は `std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/public/manual/…"))` で読む。
pub fn webp_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WEBP" {
        return None;
    }

    let mut pos = 12;
    while pos + 8 <= bytes.len() {
        let fourcc = &bytes[pos..pos + 4];
        let size = u32::from(bytes[pos + 4])
            | u32::from(bytes[pos + 5]) << 8
            | u32::from(bytes[pos + 6]) << 16
            | u32::from(bytes[pos + 7]) << 24;
        let size = size as usize;
        let data_start = pos + 8;
        let data_end = data_start.checked_add(size)?;
        if data_end > bytes.len() {
            return None;
        }
        let data = &bytes[data_start..data_end];

        match fourcc {
            b"VP8X" if data.len() >= 10 => {
                let w = u24_le(data[4], data[5], data[6]) + 1;
                let h = u24_le(data[7], data[8], data[9]) + 1;
                return Some((w, h));
            }
            b"VP8L" if data.len() >= 5 && data[0] == 0x2f => {
                let bits = u32::from(data[1])
                    | u32::from(data[2]) << 8
                    | u32::from(data[3]) << 16
                    | u32::from(data[4]) << 24;
                let w = (bits & 0x3FFF) + 1;
                let h = (bits >> 14 & 0x3FFF) + 1;
                return Some((w, h));
            }
            // キーフレームヘッダ: 3 バイトのフレームタグの後、start code
            // 0x9d 0x01 0x2a、続けて 14bit 幅・14bit 高さが LE で並ぶ
            b"VP8 "
                if data.len() >= 10 && data[3] == 0x9d && data[4] == 0x01 && data[5] == 0x2a =>
            {
                let w = u32::from(data[6]) | u32::from(data[7]) << 8;
                let h = u32::from(data[8]) | u32::from(data[9]) << 8;
                return Some((w & 0x3FFF, h & 0x3FFF));
            }
            _ => {}
        }

        // チャンクは偶数境界にパディングされる
        pos = data_end + (size & 1);
    }
    None
}
