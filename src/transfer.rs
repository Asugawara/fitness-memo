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
//!
//! ## 唯一の外部通信
//!
//! [`fetch_text`] だけがこのアプリの外へ出る。**利用者が URL を貼って押したときにしか
//! 走らず、送るのは URL だけで記録は一切外に出ない**（adr/storage/import-from-published-sheet-url.md）。
//! `public/sw.js` は `url.origin !== self.location.origin` で早期 return するので、
//! Service Worker はこの経路に一切関与しない。

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
    let Some(probe) = make_file("probe.json", "0", JSON) else {
        return false;
    };
    let data = web_sys::ShareData::new();
    data.set_files(&js_sys::Array::of1(&probe));
    nav.can_share_with_data(&data)
}

/// 書き出す MIME。**iOS の UTType は `File.type` ではなくファイル名の拡張子から決まる**
/// （`WKShareSheet.mm`）ので、`name` の拡張子と組で渡すこと。
pub const JSON: &str = "application/json";
pub const CSV: &str = "text/csv";

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

/// 取得の失敗。**利用者の次の行動が変わる粒度でだけ分ける。**
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchError {
    /// ネットワークに繋がっていない
    Offline,
    /// 繋がったが、そのシートを読ませてもらえなかった（共有設定）
    NotShared,
    /// それ以外（遮断・障害）
    Network,
}

impl FetchError {
    pub fn message(&self) -> String {
        match self {
            Self::Offline => {
                "オフラインです。ネットワークにつないでからもう一度試してください".to_string()
            }
            Self::NotShared => "シートを読み取れませんでした。共有 →「リンクを知っている全員」→「閲覧者」にしてから、もう一度試してください".to_string(),
            Self::Network => "取得できませんでした。時間をおいてもう一度試してください".to_string(),
        }
    }
}

type Sink = std::rc::Rc<std::cell::RefCell<Option<Box<dyn FnOnce(Result<String, FetchError>)>>>>;

/// 先に来たほうが取る。成功と失敗の両方が呼ばれても 2 回渡さない。
fn finish(sink: &Sink, v: Result<String, FetchError>) {
    if let Some(f) = sink.borrow_mut().take() {
        f(v);
    }
}

/// URL の中身を取ってくる。**このアプリで唯一、外へ出る通信。**
///
/// 記録は送らない（GET だけで本文を持たない）。取ってきた文字列を `sheet::parse` に
/// 渡すのは呼び出し側の仕事で、ここは「文字列が取れたか」までしか判断しない。
///
/// ★ **失敗の見分け方が UX の本体。** Google は非公開のシートに対して、CORS ヘッダ付きの
/// **404 + `text/html`**（ログイン / エラーページ）を返す。つまり fetch 自体は成功するので、
/// 「なぜか失敗しました」ではなく「共有設定を直してください」と言い切れる。
/// `content-type` を先に見るのは、公開シートでも 200 のままログインページに化けることが
/// あるため（`ok()` だけ見ていると HTML を CSV として読ませてしまう）。
///
/// ジェスチャの窓は要らない（共有シートと違い `share()` を呼ばない）ので非同期でよい。
pub fn fetch_text(url: &str, done: impl FnOnce(Result<String, FetchError>) + 'static) {
    let sink: Sink = std::rc::Rc::new(std::cell::RefCell::new(Some(Box::new(done))));
    let promise = window().fetch_with_str(url);

    let ok_sink = std::rc::Rc::clone(&sink);
    let on_ok = Closure::wrap(Box::new(move |v: JsValue| {
        let Ok(resp) = v.dyn_into::<web_sys::Response>() else {
            finish(&ok_sink, Err(FetchError::Network));
            return;
        };
        let ctype = resp
            .headers()
            .get("content-type")
            .ok()
            .flatten()
            .unwrap_or_default();
        if ctype.contains("text/html") {
            finish(&ok_sink, Err(FetchError::NotShared));
            return;
        }
        if !resp.ok() {
            let err = match resp.status() {
                401 | 403 | 404 => FetchError::NotShared,
                _ => FetchError::Network,
            };
            finish(&ok_sink, Err(err));
            return;
        }
        let Ok(text) = resp.text() else {
            finish(&ok_sink, Err(FetchError::Network));
            return;
        };

        let text_ok = std::rc::Rc::clone(&ok_sink);
        let text_err = std::rc::Rc::clone(&ok_sink);
        let on_text = Closure::wrap(Box::new(move |t: JsValue| match t.as_string() {
            Some(s) => finish(&text_ok, Ok(s)),
            None => finish(&text_ok, Err(FetchError::Network)),
        }) as Box<dyn FnMut(JsValue)>);
        let on_text_err = Closure::wrap(Box::new(move |_: JsValue| {
            finish(&text_err, Err(FetchError::Network));
        }) as Box<dyn FnMut(JsValue)>);
        let _ = text.then2(&on_text, &on_text_err);
        on_text.forget();
        on_text_err.forget();
    }) as Box<dyn FnMut(JsValue)>);

    let err_sink = std::rc::Rc::clone(&sink);
    let on_err = Closure::wrap(Box::new(move |_: JsValue| {
        // reject は TypeError しか来ない（通信断 / DNS / CORS 拒否）。中身では
        // 区別できないので、繋がっているかどうかだけで言い分けを変える
        let err = if window().navigator().on_line() {
            FetchError::Network
        } else {
            FetchError::Offline
        };
        finish(&err_sink, Err(err));
    }) as Box<dyn FnMut(JsValue)>);

    let _ = promise.then2(&on_ok, &on_err);
    // ★ `share_file` と同じ理由で forget する。取り込みは年に数回の操作
    on_ok.forget();
    on_err.forget();
}

/// 選択されたファイルを読む。読み込みにジェスチャは要らないので非同期でよい。
pub fn read_file_text(
    input: &web_sys::HtmlInputElement,
    done: impl FnOnce(Option<String>) + 'static,
) {
    let Some(file) = input.files().and_then(|list| list.get(0)) else {
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
