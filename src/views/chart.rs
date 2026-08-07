//! 自前 SVG グラフ。**Wave 3b（Phase 3b）で実装する。**
//!
//! 今は `views/mod.rs` のモジュール宣言を先に確定させるための stub。
//! props は Wave 3b の裁量に任せるため、あえて何も生やしていない。
//!
//! 実装時の要件（計画「グラフ」より）:
//! - `viewBox="0 0 320 160"` + `width:100%; height:auto`。Y 軸は 0〜max×1.1、グリッド 3 本
//! - **X は時間軸に比例配置**（等間隔にしない。休んだ週が空白として見える）
//! - 点数が 40 を超えたら `<circle>` を省略し `polyline` + 最新点のみ
//! - タップは点ではなく**タッチ X 座標の最近傍点へスナップ**（プロット領域の全高を
//!   透明な `<rect>` のヒット領域にする。`r=3` は実機で直径約 7px しかなく
//!   `min-height: 44px` 規約に反する）
//! - 0 件は「記録がありません」、1 件は点のみ
//!
//! ⚠ `view!` の名前空間の罠: `svg` / `polyline` / `circle` / `rect` / `text` / `tspan` は
//! SVG 解決されるが、**`<title>` / `<a>` / `<script>` は曖昧要素**で親コンテキスト不明なら
//! HTML 要素になる。SVG のアクセシブルネームに `<title>` を使うなら**同一 `view!` の
//! `<svg>` 直下に書く**こと。軸ラベルを別関数の `view!` に切り出すと無言に壊れる。

use leptos::prelude::*;

#[component]
pub fn Chart() -> impl IntoView {
    view! { <p class="stub">"グラフ（Wave 3b で実装）"</p> }
}
