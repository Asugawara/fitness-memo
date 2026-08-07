//! 推移タブ。**Wave 3b（Phase 3b）で実装する。**
//!
//! 今は `views/mod.rs` のモジュール宣言を先に確定させるための stub。
//!
//! 実装時の要件（計画「3. 推移」より）:
//! - 対象セレクタ（部位 or 種目）+ 期間（1M / 3M / 6M / 1Y / 全期間）
//! - グラフ下に「前回比 / 期間内ベスト / 期間内平均」、さらに下に記録テーブル（新しい順）
//! - 種目セレクタには**アーカイブ済み種目も末尾セクションに表示**する
//! - 「全期間」は `core::aggregate_weekly` で週単位集約
//! - 種目別は `core::exercise_series`、部位別は `core::group_set_series`（セット数）

use leptos::prelude::*;

#[component]
pub fn Progress() -> impl IntoView {
    view! { <p class="stub">"推移（Wave 3b で実装）"</p> }
}
