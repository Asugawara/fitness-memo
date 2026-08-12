//! 筋トレメモ — iPhone のホーム画面から完全オフラインで動く記録アプリ。
//!
//! ロジック層（`model` / `core` / `presets` / `chart_layout` / `reorder`）はターゲット
//! 非依存で、`cargo test` がホストで検証する。UI 層（`storage` / `views`）は wasm32 専用に
//! cfg gate してあり、ホストビルドが leptos の巨大な依存グラフを引かないようにしている。

pub mod chart_layout;
pub mod core;
pub mod model;
pub mod presets;
pub mod reorder;

#[cfg(target_arch = "wasm32")]
pub mod storage;
#[cfg(target_arch = "wasm32")]
pub mod transfer;
#[cfg(target_arch = "wasm32")]
pub mod views;
