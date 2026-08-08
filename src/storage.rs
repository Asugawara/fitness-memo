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

    // ★ 現行キー → 旧世代キーの順に、**`migrate` が通るまで**降りていく。
    //
    //   「最初に中身があったキー」で打ち切ると、v2 だけが壊れたときに健全な v1 が
    //   残っていてもプリセットに落ちる。それでは旧キーを残している意味が無い
    //   （旧キーは全損に対する唯一の退路。ADR-0034）。
    let mut quarantined = false;
    for key in std::iter::once(KEY).chain(LEGACY_KEYS.iter().copied()) {
        let Some(raw) = store
            .get_item(key)
            .ok()
            .flatten()
            .filter(|raw| !raw.trim().is_empty())
        else {
            continue;
        };

        match core::migrate(&raw) {
            Ok(db) => {
                let note = if key == KEY {
                    // 採用したのは現行キー。旧世代のほうが新しければ知らせる（下記）
                    newer_legacy_note(&store, &db)
                } else {
                    // 旧世代から読んだので現行キーへ写す。App 側の Effect が 400ms 後に
                    // 保存するが、その前にプロセスを kill されると次回も旧キーから
                    // 読み直すことになるため、ここで確定させる。**旧キーは消さない**
                    save(&db);
                    quarantined.then(|| {
                        "最新のデータを復元できなかったため、以前のバックアップから復元しました"
                            .to_string()
                    })
                };
                return (db, note);
            }
            Err(_) => {
                // ★ 破損データをプリセットで黙って上書きするのは全損を確定させる動作。
                //   必ず退避してから次の世代へ進む。退避先は読んだキー側に付ける
                let backup_key = format!("{key}.bak-{}", Local::now().timestamp_millis());
                let _ = store.set_item(&backup_key, &raw);
                quarantined = true;
            }
        }
    }

    if quarantined {
        // どの世代も読めなかった。退避は済んでいる
        return (
            presets::seeded_db(),
            Some("以前のデータを復元できませんでした（退避済み）".to_string()),
        );
    }

    // 初回起動。プリセットを投入する
    (presets::seeded_db(), None)
}

/// 現行キーを採用したとき、旧世代のほうに**新しい記録**が残っていれば知らせる。
///
/// ★ 旧版へロールバックしている間の記録は旧キーに書かれる。新版へ戻ると現行キーが
/// 非空なのでそのまま採用され、ロールバック期間の記録が黙って画面から消える。
/// 自動マージはしない（同じ日を両方で編集していると、どちらを正とするか決められない）。
/// **消えていないことだけは伝える。**
fn newer_legacy_note(store: &web_sys::Storage, current: &Db) -> Option<String> {
    // 日付キーはゼロ埋め ISO なので辞書順比較でよい
    let newest = |db: &Db| db.sessions.keys().next_back().cloned();
    let mine = newest(current);

    for key in LEGACY_KEYS {
        let Some(raw) = store
            .get_item(key)
            .ok()
            .flatten()
            .filter(|raw| !raw.trim().is_empty())
        else {
            continue;
        };
        let Ok(old) = core::migrate(&raw) else {
            continue;
        };
        if let Some(theirs) = newest(&old)
            && mine.as_deref().is_none_or(|m| theirs.as_str() > m)
        {
            return Some(format!(
                "以前のバージョンで付けた記録が {theirs} まで残っています（今の表示には含まれていません）"
            ));
        }
    }
    None
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
