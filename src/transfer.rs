//! ファイルの受け渡し（共有シート / ダウンロード / クリップボード）。**wasm32 専用。**
//!
//! JS interop をここに閉じ込め、`storage.rs` を「`localStorage` を読み書きするだけ」に
//! 保つ。あちらの薄さは「hidden ハンドラで何が起きるかが 4 関数を読めば分かる」という
//! 検証可能性を支えているので、Blob や Promise を混ぜてはいけない。
//!
//! ## iOS で守るべきこと
//!
//! - **`<a download>` を iOS で使わない。** standalone では `download` 属性が無視され、
//!   WebView が href 自体へ遷移する。戻る UI が無いのでアプリを強制終了するまで
//!   復帰できない。しかも `click()` は成否を返さないので検知もできない
//! - **`ShareData` には `files` だけを入れる。** `title` / `text` / `url` を混ぜると
//!   共有アイテム配列に別要素として積まれ、「ファイルに保存」が候補から消える
//! - **`share()` はクリックハンドラから同期的に呼ぶ。** WebKit の transient activation は
//!   5 秒。このアプリは書き出しが同期処理（localStorage 読み → String → File）なので
//!   2 段階フローは要らない
//! - **`<input type="file">` に `accept` を付けない。** iOS の `accept` は壊れていて
//!   （rdar://36726477）、最初の型しか効かず残りがピッカーで灰色になる。iCloud Drive
//!   経由だとさらに悪化する。種別の検証は `core::parse_import` がやる
//! - **ファイル名と MIME は必ず組で渡す。** iOS の UTType は `File.type` ではなく
//!   **拡張子**から決まるので、片方だけ合わせても意味が無い。`can_share_file` の
//!   プローブも本番と同じ拡張子・同じ MIME にしておく（違う型で可否を判定しない）

use leptos::prelude::*;
use wasm_bindgen::{JsCast, JsValue, prelude::Closure};

/// 書き出しの経路。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// 共有シート（iOS の主経路）。「ファイルに保存」で iCloud Drive に置ける
    Share,
    /// `<a download>`。**iOS では絶対に選ばない**
    Download,
    /// クリップボードへコピー
    Clipboard,
}

/// どの経路で書き出すか。
///
/// ★ **iOS なら `Download` を構造的に返さない。** standalone かどうかで分けないのは、
/// Safari タブでも共有シートは使えるうえ、タブ側で `<a download>` に落とすと
/// 「非 standalone なら安全」という検証していない前提に寄りかかることになるため。
pub fn pick_route() -> Route {
    if is_ios() {
        if can_share_file() {
            Route::Share
        } else {
            Route::Clipboard
        }
    } else {
        Route::Download
    }
}

/// iPadOS は macOS の UA を返すので、タッチ点数も見る。
fn is_ios() -> bool {
    let nav = window().navigator();
    let ua = nav.user_agent().unwrap_or_default();
    let ipad_as_mac = ua.contains("Macintosh") && nav.max_touch_points() > 1;
    ua.contains("iPhone") || ua.contains("iPad") || ua.contains("iPod") || ipad_as_mac
}

/// ファイル共有が使えるか。
///
/// ★ 1 バイトのプローブで**クリック直後に同期的に**判定する。`await` の後に
/// `canShare` を呼ぶと、その時点でジェスチャの窓が閉じていて手遅れになる。
pub fn can_share_file() -> bool {
    let nav = window().navigator();
    let has = |name: &str| js_sys::Reflect::has(&nav, &JsValue::from_str(name)).unwrap_or(false);
    if !has("share") || !has("canShare") {
        return false;
    }
    // ★ 本番と同じ拡張子・同じ MIME で試す。違う型で可否を判定すると、
    //   「共有できると判定して実際は出せない」が起きる
    let Some(probe) = make_file("probe.tsv", "0", crate::core::TSV_MIME) else {
        return false;
    };
    let data = web_sys::ShareData::new();
    data.set_files(&js_sys::Array::of1(&probe));
    nav.can_share_with_data(&data)
}

fn make_file(name: &str, body: &str, mime: &str) -> Option<JsValue> {
    let parts = js_sys::Array::of1(&JsValue::from_str(body));
    let opts = web_sys::FilePropertyBag::new();
    opts.set_type(mime);
    web_sys::File::new_with_str_sequence_and_options(&parts, name, &opts)
        .ok()
        .map(Into::into)
}

/// 共有シートを出す。**クリックハンドラから同期的に呼ぶこと。**
///
/// `done` は成否で呼ばれる。`AbortError`（利用者がキャンセルした）は**失敗ではない** —
/// ここで「保存した」扱いにすると、実際には保存していないのに催促が止まる。
pub fn share_file(name: &str, body: &str, mime: &str, done: impl FnOnce(ShareOutcome) + 'static) {
    let Some(file) = make_file(name, body, mime) else {
        done(ShareOutcome::Failed);
        return;
    };
    let data = web_sys::ShareData::new();
    // ★ files だけ。title / text / url を足すと「ファイルに保存」が消える
    data.set_files(&js_sys::Array::of1(&file));

    let promise = window().navigator().share_with_data(&data);

    // 成功と失敗のどちらか一方だけが呼ばれる。共有スロットに入れて先に来たほうが取る
    let slot = std::rc::Rc::new(std::cell::RefCell::new(Some(
        Box::new(done) as Box<dyn FnOnce(ShareOutcome)>
    )));

    let ok_slot = std::rc::Rc::clone(&slot);
    let on_ok = Closure::wrap(Box::new(move |_: JsValue| {
        if let Some(f) = ok_slot.borrow_mut().take() {
            f(ShareOutcome::Shared);
        }
    }) as Box<dyn FnMut(JsValue)>);
    let on_err = Closure::wrap(Box::new(move |err: JsValue| {
        // DOMException.name が "AbortError" なら利用者のキャンセル
        let name = js_sys::Reflect::get(&err, &JsValue::from_str("name"))
            .ok()
            .and_then(|v| v.as_string())
            .unwrap_or_default();
        let outcome = if name == "AbortError" {
            ShareOutcome::Cancelled
        } else {
            ShareOutcome::Failed
        };
        if let Some(f) = slot.borrow_mut().take() {
            f(outcome);
        }
    }) as Box<dyn FnMut(JsValue)>);

    let _ = promise.then2(&on_ok, &on_err);
    // ★ `then2` は Rust 側の Closure を借用するだけなので、drop すると JS から
    //   呼べなくなる。書き出しは年に数回の操作なので、2 つ分のリークは受け入れる
    on_ok.forget();
    on_err.forget();
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShareOutcome {
    Shared,
    /// 利用者がキャンセルした。**保存済みにしてはいけない**
    Cancelled,
    Failed,
}

/// `<a download>` でファイルを落とす。**iOS では呼ばない**（[`pick_route`] 参照）。
pub fn download_file(name: &str, body: &str, mime: &str) {
    let parts = js_sys::Array::of1(&JsValue::from_str(body));
    let opts = web_sys::BlobPropertyBag::new();
    opts.set_type(mime);
    let Ok(blob) = web_sys::Blob::new_with_str_sequence_and_options(&parts, &opts) else {
        return;
    };
    let Ok(url) = web_sys::Url::create_object_url_with_blob(&blob) else {
        return;
    };

    let document = document();
    let Ok(anchor) = document.create_element("a") else {
        return;
    };
    let Ok(anchor) = anchor.dyn_into::<web_sys::HtmlAnchorElement>() else {
        return;
    };
    anchor.set_href(&url);
    anchor.set_download(name);
    anchor.click();

    // ★ click 直後に revoke すると、まだ読み出していないブラウザで空ファイルになる
    let revoke = url.clone();
    set_timeout(
        move || {
            let _ = web_sys::Url::revoke_object_url(&revoke);
        },
        std::time::Duration::from_secs(60),
    );
}

/// クリップボードへ。**クリックハンドラから同期的に呼ぶこと。**
///
/// ★ 成否を捨ててはいけない。共有シートが使えない端末ではここが最後のバックアップ
/// 経路で、失敗を「コピーしました」と報告すると、書けたつもりで端末を初期化される。
/// 表示の更新にジェスチャの窓は要らないので、非同期に受けて構わない。
pub fn copy_text(text: &str, done: impl FnOnce(bool) + 'static) {
    let promise = window().navigator().clipboard().write_text(text);

    let slot = std::rc::Rc::new(std::cell::RefCell::new(Some(
        Box::new(done) as Box<dyn FnOnce(bool)>
    )));
    let ok_slot = std::rc::Rc::clone(&slot);
    let on_ok = Closure::wrap(Box::new(move |_: JsValue| {
        if let Some(f) = ok_slot.borrow_mut().take() {
            f(true);
        }
    }) as Box<dyn FnMut(JsValue)>);
    let on_err = Closure::wrap(Box::new(move |_: JsValue| {
        if let Some(f) = slot.borrow_mut().take() {
            f(false);
        }
    }) as Box<dyn FnMut(JsValue)>);

    let _ = promise.then2(&on_ok, &on_err);
    on_ok.forget();
    on_err.forget();
}

/// 選択されたファイルを読む。読み込みにジェスチャは要らないので非同期でよい。
///
/// ★ **読み終えたら `value` を空にする。** 空にしないと、同じファイルをもう一度選んでも
/// `change` が飛ばない（値が変わっていないため）。「確認画面でやめる → もう一度同じ
/// ファイル」は普通に踏む操作で、そのとき**何も起きないのに理由が画面に出ない**。
/// `files()` は同期で掴んでから空にするので、読み出し自体には影響しない。
pub fn read_file_text(
    input: &web_sys::HtmlInputElement,
    done: impl FnOnce(Option<String>) + 'static,
) {
    let picked = input.files().and_then(|list| list.get(0));
    input.set_value("");
    let Some(file) = picked else {
        done(None);
        return;
    };
    let Ok(reader) = web_sys::FileReader::new() else {
        done(None);
        return;
    };

    let reader_for_load = reader.clone();
    let mut done = Some(done);
    let on_load = Closure::once_into_js(move |_: web_sys::Event| {
        let text = reader_for_load.result().ok().and_then(|v| v.as_string());
        if let Some(f) = done.take() {
            f(text);
        }
    });
    reader.set_onloadend(Some(on_load.unchecked_ref::<js_sys::Function>()));
    let _ = reader.read_as_text(&file);
}
