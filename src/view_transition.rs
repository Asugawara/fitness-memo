//! `document.startViewTransition()` の薄いラッパ。タブ切替の方向を絵で伝えるために使う。
//!
//! ★ **web-sys の `Document::start_view_transition` は使わない。**
//!   あれは `#[cfg(web_sys_unstable_apis)]` の下にあり、使うには `.cargo/config.toml` の
//!   `rustflags` に cfg を足すことになる。それは web-sys の unstable 面を丸ごと開ける操作で、
//!   `Cargo.toml` が宣言している「使う API は全て自前で宣言する」方針と噛み合わない
//!   （偶然有効になった API に寄りかかると、パッチ更新で無言に壊れる）。
//!   ここで extern を 1 本立てるほうが、依存も影響範囲も小さい。

use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::future_to_promise;

#[wasm_bindgen]
unsafe extern "C" {
    #[wasm_bindgen(js_namespace = document, js_name = startViewTransition)]
    fn start_view_transition(options: &JsValue);
}

/// 遷移の向き。文字列は CSS の `:active-view-transition-type()` と綴りを合わせる。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Direction {
    Forward,
    Backward,
}

impl Direction {
    fn as_str(self) -> &'static str {
        match self {
            Direction::Forward => "forward",
            Direction::Backward => "backward",
        }
    }
}

/// 方向つきの View Transition を使えるか。
///
/// ★ `startViewTransition` の**有無では判定しない。** 引数にオブジェクトを渡す形
///   （`{update, types}`）は関数 1 個を渡す旧形より後から入った。旧形しか無いブラウザに
///   オブジェクトを渡すと `update` が呼ばれず、**タブが切り替わらなくなる**。
///   機能の劣化ではなく機能の停止なので、ここだけは厳しく判定する。
///   `:active-view-transition-type()` が使えることは `types` が通ることと同時期なので、
///   セレクタの対応で代表させる。
fn types_supported() -> bool {
    web_sys::css::supports("selector(:active-view-transition-type(forward))").unwrap_or(false)
}

/// `update` で DOM を書き換えつつ、方向つきの遷移を走らせる。
///
/// 非対応環境では `update` をその場で呼ぶだけ（progressive enhancement）。
pub fn run(direction: Direction, update: impl FnOnce() + 'static) {
    if !types_supported() {
        update();
        return;
    }

    // ★ `update` は「呼ばれた時点で DOM を同期更新する」ことを期待されるが、leptos の
    //   signal → DOM 反映は `RenderEffect` が `Executor::spawn_local` に載せるので
    //   microtask 送りになる。同期クロージャを渡すと、まだ古い DOM のままスナップショットを
    //   撮り直すことになり、遷移が「何も変わらないアニメーション」になる。
    //   `update` は Promise を返してよいので、1 tick 待ってから解決する。
    let callback = Closure::once_into_js(move || -> js_sys::Promise {
        future_to_promise(async move {
            update();
            leptos::task::tick().await;
            Ok(JsValue::UNDEFINED)
        })
    });

    let options = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&options, &JsValue::from_str("update"), &callback);
    let _ = js_sys::Reflect::set(
        &options,
        &JsValue::from_str("types"),
        &js_sys::Array::of1(&JsValue::from_str(direction.as_str())),
    );
    start_view_transition(&options);
}
