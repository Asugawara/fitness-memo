//! 筋トレメモ — iPhone のホーム画面から完全オフラインで動く記録アプリ。
//!
//! ロジック層（`model` / `core` / `presets` / `chart_layout` / `reorder` / `i18n`）はターゲット
//! 非依存で、`cargo test` がホストで検証する。UI 層（`storage` / `views`）は wasm32 専用に
//! cfg gate してあり、ホストビルドが leptos の巨大な依存グラフを引かないようにしている。

pub mod chart_layout;
pub mod core;
// ★ cfg を付けない。`core` と `presets` が文言を引くので wasm32 に閉じられない
// （閉じないおかげで `cargo test` が日英の不変条件を検証できる）
pub mod i18n;
pub mod model;
pub mod presets;
pub mod reorder;

#[cfg(target_arch = "wasm32")]
pub mod storage;
#[cfg(target_arch = "wasm32")]
pub mod transfer;
#[cfg(target_arch = "wasm32")]
pub mod views;
