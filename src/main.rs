//! `fn main` を cfg で丸ごと分岐する。
//!
//! **オプションなしの `cargo test` は lib に加えて bin もテストハーネスとしてビルドする。**
//! `lib.rs` 側で `views` を cfg gate しても、ここが `views::App` を無条件参照していれば
//! ホストビルドが E0433 で落ち、pre-commit が常に失敗する。

#[cfg(target_arch = "wasm32")]
fn main() {
    console_error_panic_hook::set_once();
    // mount_to_body は prelude には無く leptos::mount:: が必要
    leptos::mount::mount_to_body(fitness_memo::views::App);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {}
