//! 種目タブ。**Wave 4（Phase 3c）で実装する。**
//!
//! 今は `views/mod.rs` のモジュール宣言を先に確定させるための stub。
//!
//! 実装時の要件（計画「4. 種目」より）:
//! - 部位グループ: 追加 / 改名 / 並び替え / 色変更 / 削除
//!   **削除ガードはアーカイブ済み種目も所属種目に数える**（`Db::exercise_ids_of_group`
//!   がアーカイブ込みで返すのでそれを使う）
//! - 種目: 追加 / 改名 / 部位変更 / **`Kind` 変更（単位が変わる警告付き）** /
//!   並び替え / アーカイブ
//! - 「プリセットを追加」は `presets::seed(&mut db)` を呼ぶだけでよい
//!   （同名スキップ・ID は `Db::alloc_id` 経由が実装済み）

use leptos::prelude::*;

#[component]
pub fn Menu() -> impl IntoView {
    view! { <p class="stub">"種目（Wave 4 で実装）"</p> }
}
