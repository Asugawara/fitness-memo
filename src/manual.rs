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
    /// ★ **`true` にするなら `public/manual/{ja,en}/{id}.webp` が実在し、`ChapterText`
    ///   の `fig` / `fig_alt` が両言語とも埋まっていること。** 3 つは
    ///   `every_language_has_the_same_manual_chapters`（`i18n.rs`）と
    ///   [`tests::every_manual_figure_exists_with_the_declared_size`] が揃って固定する。
    ///
    /// ★ **寸法（`fig`）は言語非依存にできない**ので、ここには持たせずに `ChapterText`
    ///   側へ置いてある。クリップが要素の外接矩形なので内容依存で、実際に `accordions`
    ///   は ja 786×1030 / en 786×994 と高さが違う（部位名の折り返し行数の差）。
    ///
    /// 図を持たない 6 章（`progress-target` / `labels` / `drop-sets` / `reorder` /
    /// `backup` / `whats-new`）が `false` なのは、静止画で伝わらない（`reorder`）か、
    /// 閉じた `<select>` しか写らない（`progress-target`）か、文章で足りる
    /// （`backup` / `whats-new`）か、この波ではまだ図を持たせない（`labels` /
    /// `drop-sets`）ため。
    pub has_fig: bool,
}

/// マニュアルの章立て。11 章、`i18n.rs` の `Manual::chapters` と同じ順・同じ長さ
/// （`every_language_has_the_same_manual_chapters` が突き合わせる）。
pub const MANUAL_CHAPTERS: &[Chapter] = &[
    Chapter {
        id: "progress-target",
        has_fig: false,
    },
    Chapter {
        id: "chart-readout",
        has_fig: true,
    },
    Chapter {
        id: "copy-last",
        has_fig: true,
    },
    Chapter {
        id: "labels",
        has_fig: false,
    },
    Chapter {
        id: "exercise-memo",
        has_fig: true,
    },
    Chapter {
        id: "drop-sets",
        has_fig: false,
    },
    Chapter {
        id: "empty-day",
        has_fig: true,
    },
    Chapter {
        id: "accordions",
        has_fig: true,
    },
    Chapter {
        id: "reorder",
        has_fig: false,
    },
    Chapter {
        id: "backup",
        has_fig: false,
    },
    Chapter {
        id: "whats-new",
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
/// 下の [`tests::every_manual_figure_exists_with_the_declared_size`] と
/// `e2e/manual.spec.mjs`（配信物側の担当）が同じ読み方をする。
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Lang;
    use std::collections::HashSet;
    use std::path::{Path, PathBuf};

    /// `public/manual` の絶対パス。
    ///
    /// ★ `include_bytes!` を使わない理由は [`webp_size`] の doc のとおり（このモジュールは
    ///   非 cfg なので、埋め込むと**図がそのまま wasm に入る**）。テストはホストでしか
    ///   走らないので、コンパイル時に決まるのはパスだけにして中身は実行時に読む。
    fn manual_root() -> PathBuf {
        PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/public/manual"))
    }

    /// 宣言した図が実在し、**宣言寸法が実ファイルと言語ごとに一致**していること。
    /// 逆向き（どの章からも参照されていない図が残っていないこと）も見る。
    ///
    /// ★ **これが `src/i18n.rs` の `fig` を実物に縛る唯一の仕組み。** `fig` は
    ///   `<img width height>` に出てレイアウトシフトを防ぐ値なので、実ファイルとずれると
    ///   図だけが縦に伸び縮みして表示される（見た目では気づけない）。
    ///
    /// ★ **逆向きを見るのは、章を消して図を消し忘れても誰も気づかないから。** 参照されない
    ///   `.webp` は `copy-dir` でそのまま配信物に載り続け、`stamp-sw.sh` の除外にも
    ///   引っかからないまま Pages に残る。
    ///
    /// ★ クリップ寸法が変わる UI 変更をすると**このテストは撮影前の古い図に対して通り**、
    ///   そのあと `.githooks/pre-commit` が撮り直した図で E2E 側が落ちる。直し方は
    ///   `src/i18n.rs` の `fig` を新しい寸法へ書き換えること（hook のコメントに同じ説明がある）。
    #[test]
    fn every_manual_figure_exists_with_the_declared_size() {
        let root = manual_root();
        let mut referenced: HashSet<PathBuf> = HashSet::new();

        for (lang, _) in Lang::CHOICES {
            let dir = root.join(lang.tag());
            let texts = lang.strings().manual.chapters;
            assert_eq!(
                texts.len(),
                MANUAL_CHAPTERS.len(),
                "{lang:?} の章数が MANUAL_CHAPTERS と食い違う"
            );

            for (chapter, text) in MANUAL_CHAPTERS.iter().zip(texts) {
                let path = dir.join(format!("{}.webp", chapter.id));
                let Some(declared) = text.fig else {
                    assert!(
                        !chapter.has_fig,
                        "{lang:?} の {} は has_fig なのに fig が無い",
                        chapter.id
                    );
                    assert!(
                        !path.exists(),
                        "図を持たない章の画像が残っている: {}",
                        path.display()
                    );
                    continue;
                };
                assert!(
                    chapter.has_fig,
                    "{lang:?} の {} は fig を持つのに has_fig が false",
                    chapter.id
                );

                let bytes = std::fs::read(&path)
                    .unwrap_or_else(|e| panic!("{} が読めない: {e}", path.display()));
                let actual = webp_size(&bytes)
                    .unwrap_or_else(|| panic!("{} の WebP ヘッダが読めない", path.display()));
                assert_eq!(
                    actual,
                    declared,
                    "{} の実寸法 {actual:?} が宣言 {declared:?} と食い違う。\
                     `node scripts/shots.mjs --only=manual` で撮り直したなら \
                     src/i18n.rs の fig を実測値へ直すこと",
                    path.display()
                );
                referenced.insert(path);
            }
        }

        // ── 逆向き: 孤児が残っていないこと ──────────────────────────────────
        let langs: HashSet<&str> = Lang::CHOICES.iter().map(|(l, _)| l.tag()).collect();
        for lang_dir in read_dir_sorted(&root) {
            let name = lang_dir
                .file_name()
                .expect("public/manual の直下")
                .to_string_lossy()
                .into_owned();
            // ★ ドットファイルは飛ばす。`.DS_Store` で `cargo test` が落ちると
            //   pre-commit ごと止まる。配信物から落とす担当は `stamp-sw.sh` の
            //   `shell_files()`（`! -name '.*'`）で、こちらの責務ではない。
            if name.starts_with('.') {
                continue;
            }
            assert!(
                langs.contains(name.as_str()),
                "public/manual/{name} はどの言語にも対応しない"
            );
            for fig in read_dir_sorted(&lang_dir) {
                if fig
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with('.'))
                {
                    continue;
                }
                assert!(
                    referenced.contains(&fig),
                    "{} はどの章からも参照されていない（章を消したら図も消すこと）",
                    fig.display()
                );
            }
        }
    }

    /// `read_dir` は順序を約束しないので、失敗メッセージが run ごとに変わらないよう並べる。
    fn read_dir_sorted(dir: &Path) -> Vec<PathBuf> {
        let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
            .unwrap_or_else(|e| panic!("{} が読めない: {e}", dir.display()))
            .map(|e| e.expect("ディレクトリの走査").path())
            .collect();
        paths.sort();
        paths
    }
}
