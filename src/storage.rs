//! `localStorage` への永続化。**wasm32 専用。**
//!
//! エクスポート手段がまだ無いので、**パース失敗時は絶対に上書きしない**。
//! `core::migrate` が `Err` なら raw を `<キー>.bak-<epoch>` へ退避してから
//! プリセット入りの `Db` を返し、起動時に一度だけ通知を出す。
//!
//! ## キーは schema 非互換の変更のたびに切る
//!
//! `Db` のフィールドを**消す**変更は前方互換を壊す。`Exercise.kind` を消したときの例:
//! 新版が書いた JSON に `kind` が無いので、旧版の serde は `missing field` で `Err` を
//! 返し、上の退避パスに落ちて**プリセット入りの空 Db が表示される**。退避キーから
//! 読み戻す UI は無いので、ユーザーには全記録が消えたようにしか見えない。
//!
//! GitHub Pages のロールバック・SW の更新失敗・`?sw=off` 後に古いキャッシュを掴む、の
//! いずれでも旧版は動きうる。だから**キーを共有せず世代ごとに分ける**。
//! 旧キーは消さずに読み取り専用で残すので、旧版へ戻ってもその世代の時点まで
//! 巻き戻るだけで済む（新版で追記した分は旧版から見えないが、全損よりはるかに軽い）。

use std::cell::RefCell;
use std::time::Duration;

use chrono::Local;
use leptos::prelude::*;

use crate::model::Db;
use crate::{core, presets};

/// 現行の保存キー。書き込みは常にここだけ。
const KEY: &str = "fitness-memo/v2";

/// 旧世代のキー。**読むだけで、書き戻さない。**
///
/// 新しいほうから順に並べる。世代が増えたらここに足す。
const LEGACY_KEYS: &[&str] = &["fitness-memo/v1"];

/// 入力のたびに書かないための debounce 幅。
const DEBOUNCE: Duration = Duration::from_millis(400);

thread_local! {
    /// まだ書き込んでいない最新の `Db`。
    static PENDING: RefCell<Option<Db>> = const { RefCell::new(None) };
    /// 進行中の debounce タイマー。
    static HANDLE: RefCell<Option<TimeoutHandle>> = const { RefCell::new(None) };
}

/// Safari のプライベートモードなどでは `local_storage()` が例外を投げるので
/// `Result` と `Option` の両方を畳む。
fn store() -> Option<web_sys::Storage> {
    window().local_storage().ok().flatten()
}

/// 起動時の読み込み。戻り値の 2 番目は「一度だけ出す通知メッセージ」。
pub fn load() -> (Db, Option<String>) {
    let Some(store) = store() else {
        return (
            presets::seeded_db(),
            Some(
                "この端末では記録を保存できません（プライベートブラウズ中かもしれません）"
                    .to_string(),
            ),
        );
    };

    // 現行キー → 旧世代キーの順に探す。最初に中身が入っていたものを採用する
    let found = std::iter::once(KEY)
        .chain(LEGACY_KEYS.iter().copied())
        .find_map(|key| {
            let raw = store
                .get_item(key)
                .ok()
                .flatten()
                .filter(|raw| !raw.trim().is_empty())?;
            Some((key, raw))
        });

    let Some((key, raw)) = found else {
        // 初回起動。プリセットを投入する
        return (presets::seeded_db(), None);
    };

    match core::migrate(&raw) {
        Ok(db) => {
            // 旧世代から読んだときは現行キーへ写しておく。App 側の Effect が 400ms 後に
            // 保存するので必須ではないが、その前にプロセスを kill されると次回も
            // 旧キーから読み直すことになるため、ここで確定させる。
            // **旧キーは消さない**（旧版へ戻ったときの退路）
            if key != KEY {
                save(&db);
            }
            (db, None)
        }
        Err(_) => {
            // ★ 破損データをプリセットで黙って上書きするのは全損を確定させる動作。
            //   必ず退避してから差し替える。退避先は読んだキー側に付ける
            let backup_key = format!("{key}.bak-{}", Local::now().timestamp_millis());
            let saved = store.set_item(&backup_key, &raw).is_ok();
            let msg = if saved {
                "以前のデータを復元できませんでした（退避済み）"
            } else {
                "以前のデータを復元できませんでした（退避にも失敗しました）"
            };
            (presets::seeded_db(), Some(msg.to_string()))
        }
    }
}

/// 即時保存。
pub fn save(db: &Db) {
    let Some(store) = store() else {
        return;
    };
    if let Ok(json) = serde_json::to_string(db) {
        // 容量超過（QuotaExceededError）は握りつぶす。1 セット ≈ 30 bytes、
        // 10 年で約 1.1 MB なので Safari の約 5 MB 上限には当たらない見積り
        let _ = store.set_item(KEY, &json);
    }
}

/// 400ms の debounce 付き保存。入力 1 文字ごとに JSON 直列化するのを避ける。
pub fn save_debounced(db: Db) {
    PENDING.with_borrow_mut(|pending| *pending = Some(db));
    cancel_pending_timer();
    if let Ok(handle) = set_timeout_with_handle(flush, DEBOUNCE) {
        HANDLE.with_borrow_mut(|slot| *slot = Some(handle));
    } else {
        // タイマーが張れない環境では即時保存にフォールバックする
        flush();
    }
}

/// pending の保存を即時実行する。
///
/// **`visibilitychange` の hidden で必ず呼ぶこと。** 最終セットを打ち込んだ直後に
/// スワイプでホームへ戻るのはジムで最も普通の操作だが、バックグラウンドで JS タイマーは
/// 凍結され iOS は PWA プロセスを頻繁に kill するため、pending の debounce が発火せず
/// 最後の入力が消える。`pagehide` は iOS で信頼できない。
pub fn flush() {
    cancel_pending_timer();
    if let Some(db) = PENDING.with_borrow_mut(Option::take) {
        save(&db);
    }
}

fn cancel_pending_timer() {
    if let Some(handle) = HANDLE.with_borrow_mut(Option::take) {
        handle.clear();
    }
}
