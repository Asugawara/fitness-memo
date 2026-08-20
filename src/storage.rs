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

use std::cell::{Cell, RefCell};
use std::time::Duration;

use chrono::Local;
use leptos::prelude::*;

use crate::i18n::{self, Lang};
use crate::model::{Db, Id, IdGen};
use crate::{core, presets};

/// 現行の保存キー。書き込みは常にここだけ。
const KEY: &str = "fitness-memo/v3";

/// 旧世代のキー。**読むだけで、書き戻さない。**
///
/// 新しいほうから順に並べる。世代が増えたらここに足す。
///
/// v2 → v3 は ID を連番から乱数に変えた世代。旧キーを残すので、移行直前の状態が
/// `fitness-memo/v2` に**正常なまま凍結**される（`.bak-` と違って壊れて退避された
/// ものではない）。移行にバグがあっても手で戻せる。
const LEGACY_KEYS: &[&str] = &["fitness-memo/v2", "fitness-memo/v1"];

/// 入力のたびに書かないための debounce 幅。
const DEBOUNCE: Duration = Duration::from_millis(400);

thread_local! {
    /// まだ書き込んでいない最新の `Db`。
    static PENDING: RefCell<Option<Db>> = const { RefCell::new(None) };
    /// 進行中の debounce タイマー。
    static HANDLE: RefCell<Option<TimeoutHandle>> = const { RefCell::new(None) };
    /// ID 採番器。**起動ごとに `crypto` から 1 回だけシードを引く。**
    ///
    /// `Db` に持たせてはいけない — エクスポートで PRNG の状態ごと複製され、
    /// 2 台が同じ ID 列を生成するようになる。
    static IDGEN: RefCell<IdGen> = RefCell::new(IdGen::from_seed(crypto_seed()));
    /// 直近の `set_item` が失敗したか（容量超過など）。
    static SAVE_FAILED: Cell<bool> = const { Cell::new(false) };
}

/// [`IdGen`] の種。`getRandomValues` は iOS 6.1+ なので実質どこでも通る。
///
/// ★ フォールバックに時刻だけを使ってはいけない。2 台が同じミリ秒に初回起動すると
/// シードが一致し、以後まったく同じ ID 列を生成する（連番より悪い）。アドレスと
/// 高精度時刻を混ぜて、同一ミリ秒でも分岐させる。
fn crypto_seed() -> u64 {
    let mut buf = [0u8; 8];
    let ok = window()
        .crypto()
        .ok()
        .filter(|c| c.get_random_values_with_u8_array(&mut buf).is_ok())
        .is_some();
    if ok {
        return u64::from_le_bytes(buf);
    }

    let time = Local::now().timestamp_millis() as u64;
    let perf = window().performance().map_or(0, |p| p.now().to_bits());
    let addr = std::ptr::addr_of!(buf) as u64;
    let mut z =
        time.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ perf.rotate_left(17) ^ addr.rotate_left(31);
    // SplitMix64 の finalizer で混ぜてから返す
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// UI から呼ぶ唯一の採番口。
pub fn alloc_id<T>() -> Id<T> {
    IDGEN.with_borrow_mut(IdGen::alloc)
}

/// 採番器を借りる。`core::migrate` / `core::parse_import` に渡すための口。
///
/// 採番器を `core` 側に置かないのは、シードを引くのに `web-sys` が要るから。
/// `core` は `IdGen` を引数で受け取るだけなのでホストの `cargo test` がそのまま動く。
pub fn with_ids<R>(f: impl FnOnce(&mut IdGen) -> R) -> R {
    IDGEN.with_borrow_mut(f)
}

/// Safari のプライベートモードなどでは `local_storage()` が例外を投げるので
/// `Result` と `Option` の両方を畳む。
fn store() -> Option<web_sys::Storage> {
    window().local_storage().ok().flatten()
}

/// 起動時の読み込み。戻り値の 2 番目は「一度だけ出す通知メッセージ」。
///
/// ★ `lang` は**通知の言語**と**初回投入するプリセットの言語**の両方に効く。
///   `App` が `LangCtx` を provide する前に決めた値をそのまま渡す
///   （adr/storage/preset-names-are-user-data-seeded-once.md）。
pub fn load(lang: Lang) -> (Db, Option<String>) {
    let Some(store) = store() else {
        return (
            presets::seeded_db(lang),
            Some(lang.strings().boot.cannot_save.to_string()),
        );
    };

    // ★ 現行キー → 旧世代キーの順に、**`migrate` が通るまで**降りていく。
    //
    //   「最初に中身があったキー」で打ち切ると、v3 だけが壊れたときに健全な v2 が
    //   残っていてもプリセットに落ちる。それでは旧キーを残している意味が無い
    //   （旧キーは全損に対する唯一の退路。adr/storage/storage-key-per-schema-generation.md）。
    let mut quarantined = false;
    let mut newer: Option<u32> = None;
    // ★ 退避に失敗したまま先へ進むと、下の `save()` が原本を上書きして永久に失う。
    //   退避が「呼ばれた」ことと「成立した」ことは違う
    let mut rescue_failed = false;

    for key in std::iter::once(KEY).chain(LEGACY_KEYS.iter().copied()) {
        let Some(raw) = store
            .get_item(key)
            .ok()
            .flatten()
            .filter(|raw| !raw.trim().is_empty())
        else {
            continue;
        };

        match with_ids(|ids| core::migrate(&raw, ids)) {
            Ok(db) => {
                let note = if key == KEY {
                    // 採用したのは現行キー。旧世代のほうが新しければ知らせる（下記）
                    newer_legacy_note(&store, &db, lang)
                } else if rescue_failed {
                    // ★ 退避できていない原本が現行キーに残っている。ここで `save` すると
                    //   それを上書きして永久に失う。**書かずに表示だけする。**
                    //   次回起動もこの世代から読み直せばよく、原本は残る
                    Some(lang.strings().boot.rescue_failed.to_string())
                } else {
                    // 旧世代から読んだので現行キーへ写す。App 側の Effect が 400ms 後に
                    // 保存するが、その前にプロセスを kill されると次回も旧キーから
                    // 読み直すことになるため、ここで確定させる。**旧キーは消さない**
                    save(&db);
                    match newer {
                        // 未来のデータを踏み越えて古い世代を採用した。保管先を伝える
                        Some(v) => Some(lang.boot_restored_over_newer(v)),
                        None => quarantined
                            .then(|| lang.strings().boot.restored_from_backup.to_string()),
                    }
                };
                return (db, note);
            }
            // ★ 破損データをプリセットで黙って上書きするのは全損を確定させる動作。
            //   必ず退避してから次の世代へ進む。退避先は読んだキー側に付ける
            Err(core::RestoreError::Broken(_)) => {
                let backup_key = format!("{key}.bak-{}", Local::now().timestamp_millis());
                if store.set_item(&backup_key, &raw).is_err() {
                    rescue_failed = true;
                }
                quarantined = true;
            }
            // 新しい版が書いたデータ。**壊れてはいない**ので退避先も文言も分ける。
            // 「復元できませんでした」と出すと、新しい版に戻せば救えることが伝わらない
            Err(core::RestoreError::Unsupported(v)) => {
                let backup_key = format!("{key}.newer-{v}-{}", Local::now().timestamp_millis());
                if store.set_item(&backup_key, &raw).is_err() {
                    rescue_failed = true;
                }
                newer = Some(v);
            }
        }
    }

    if let Some(v) = newer {
        return (presets::seeded_db(lang), Some(lang.boot_found_newer(v)));
    }
    if quarantined {
        // どの世代も読めなかった。退避は済んでいる
        return (
            presets::seeded_db(lang),
            Some(lang.strings().boot.restore_failed.to_string()),
        );
    }

    // 初回起動。プリセットを投入する
    (presets::seeded_db(lang), None)
}

/// 現行キーを採用したとき、旧世代のほうに**新しい記録**が残っていれば知らせる。
///
/// ★ 旧版へロールバックしている間の記録は旧キーに書かれる。新版へ戻ると現行キーが
/// 非空なのでそのまま採用され、ロールバック期間の記録が黙って画面から消える。
/// 自動マージはしない（同じ日を両方で編集していると、どちらを正とするか決められない）。
/// **消えていないことだけは伝える。**
fn newer_legacy_note(store: &web_sys::Storage, current: &Db, lang: Lang) -> Option<String> {
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
        let Ok(old) = with_ids(|ids| core::migrate(&raw, ids)) else {
            continue;
        };
        if let Some(theirs) = newest(&old)
            && mine.as_deref().is_none_or(|m| theirs.as_str() > m)
        {
            return Some(lang.boot_newer_legacy(&theirs));
        }
    }
    None
}

/// 即時保存。
///
/// ★ **失敗を握りつぶさない。** 容量超過（`QuotaExceededError`）で書けなくなっても
/// アプリは何事もなく動き続けるので、利用者は数週間分の入力を失ってから気づくことになる。
/// 起きたことを [`save_failed`] に残し、画面が警告を出せるようにする。
pub fn save(db: &Db) {
    let Some(store) = store() else {
        // 保存できない環境（プライベートブラウズ等）は load() の通知が担当済み
        return;
    };
    let Ok(json) = serde_json::to_string(db) else {
        SAVE_FAILED.set(true);
        return;
    };
    SAVE_FAILED.set(store.set_item(KEY, &json).is_err());
}

/// 直近の保存が失敗したか。`visibilitychange` の visible で拾って警告に出す。
pub fn save_failed() -> bool {
    SAVE_FAILED.get()
}

// ── 退避データ ──────────────────────────────────────────────────────────────

/// 取り込み前の自動退避に使う前置。**古い順に剪定する。**
///
/// 破損退避（`.bak-`）と分けるのは、あちらが「二度と手に入らないかもしれないデータ」
/// なので機械的に消したくないため。こちらは直前の状態なので世代を絞ってよい。
const PRE_PREFIX_SUFFIX: &str = ".pre-";
/// `.pre-` を残す数。
const PRE_KEEP: usize = 3;

/// 退避データのキー一覧（新しい順）。`.bak-` / `.pre-` / `.newer-` を全部拾う。
///
/// ★ 画面からは使わない（adr/ux/one-screen-export-import.md で一覧 UI を畳んだ）。
/// [`snapshot_current`] の剪定だけが呼ぶので private に閉じてある。
fn backup_keys() -> Vec<String> {
    let Some(store) = store() else {
        return Vec::new();
    };
    let mut keys: Vec<String> = (0..store.length().unwrap_or(0))
        .filter_map(|i| store.key(i).ok().flatten())
        .filter(|k| k.contains(".bak-") || k.contains(PRE_PREFIX_SUFFIX) || k.contains(".newer-"))
        .collect();
    // キー末尾の epoch が単調増加するので、文字列の降順で新しい順になる
    keys.sort_unstable_by(|a, b| b.cmp(a));
    keys
}

pub fn read_backup(key: &str) -> Option<String> {
    store()?.get_item(key).ok().flatten()
}

/// ★ 画面からは使わない。[`snapshot_current`] の剪定専用。
fn remove_key(key: &str) {
    if let Some(store) = store() {
        let _ = store.remove_item(key);
    }
}

/// 現在の保存内容を `.pre-<epoch>` へ複製する。**取り込みの直前に必ず呼ぶ。**
///
/// 戻り値は退避先のキー。`None` は控えが取れていないので、呼び側は追加の確認を出す。
pub fn snapshot_current() -> Option<String> {
    let store = store()?;
    let raw = store
        .get_item(KEY)
        .ok()
        .flatten()
        .filter(|raw| !raw.trim().is_empty())?;

    // 書く前に古い世代を落とす（新しい世代のぶんを空けてから書く）
    let mut existing: Vec<String> = backup_keys()
        .into_iter()
        .filter(|k| k.contains(PRE_PREFIX_SUFFIX))
        .collect();
    while existing.len() >= PRE_KEEP {
        if let Some(oldest) = existing.pop() {
            remove_key(&oldest);
        }
    }

    let key = format!(
        "{KEY}{PRE_PREFIX_SUFFIX}{}",
        Local::now().timestamp_millis()
    );
    store.set_item(&key, &raw).ok().map(|()| key)
}

/// `Db` を丸ごと差し替えて即座に書く。**呼ぶ前に [`snapshot_current`] すること。**
pub fn replace_now(db: &Db) {
    cancel_pending_timer();
    PENDING.with_borrow_mut(|pending| *pending = None);
    save(db);
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

// ── UI の状態 ───────────────────────────────────────────────────────────────
//
// ★ `Db` に混ぜない。`Db` の JSON はそのままエクスポート形式になる前提（adr/storage/defer-export-import.md）で、
//   UI 都合のフィールドを入れるとその前提が崩れる。schema 世代を切る必要も出る。
//
// ★ adr/storage/localstorage-single-key-json.md の単一キー方針にも反しない。あの方針は「`Db` の参照整合性を部分書き込みで
//   壊さない」ためのもので、`Db` を一切参照しないフラグには及ばない。
//
// ★ このキーは失われても害がない。読めなければ案内がもう一度出るだけなので、
//   移行（`LEGACY_KEYS`）も退避（`.bak-`）も持たせない。

/// UI の状態を持つキー。`Db` とは分ける。
const UI_KEY: &str = "fitness-memo/ui/v1";

/// `UI_KEY` に入れる内容。フィールドは全て `#[serde(default)]` にして、
/// 増減しても既存の JSON がそのまま読めるようにする（読めなくても害はないが、
/// 無意味に案内が復活するのは避ける）。
#[derive(serde::Serialize, serde::Deserialize, Default)]
struct UiState {
    #[serde(default)]
    install_hint_dismissed: bool,
    /// 設定画面で**明示的に選ばれた**言語。`None` は「まだ選んでいない」で、
    /// このときだけブラウザの言語に従う。
    ///
    /// ★ `Option<Lang>` ではなく `Option<String>` で持つ。知らない綴り（手で編集された /
    ///   将来の言語が書いた）が入っていると `Lang` の deserialize が失敗し、
    ///   **`UiState` 全体のパースが落ちて `install_hint_dismissed` まで巻き添えで消える**。
    ///   文字列で受けて `i18n::parse_saved` で寛容に解けば、知らない値は「未設定」に
    ///   落ちるだけで済む
    #[serde(default)]
    lang: Option<String>,
}

fn ui_state() -> UiState {
    store()
        .and_then(|s| s.get_item(UI_KEY).ok().flatten())
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

/// ホーム画面追加の案内を利用者が閉じたか。
pub fn install_hint_dismissed() -> bool {
    ui_state().install_hint_dismissed
}

/// 設定画面で選ばれた言語。**未設定なら `None`**（呼び側がブラウザの言語に倒す）。
pub fn saved_lang() -> Option<Lang> {
    ui_state().lang.as_deref().and_then(i18n::parse_saved)
}

/// 言語の選択を保存する。
///
/// クリックのたびに 1 回きりなので debounce しない（`dismiss_install_hint` と同じ）。
pub fn save_lang(lang: Lang) {
    let Some(store) = store() else {
        return;
    };
    // ★ 読んでから 1 フィールドだけ差し替える（`dismiss_install_hint` と同じ理由）
    let mut next = ui_state();
    next.lang = Some(lang.tag().to_string());
    if let Ok(json) = serde_json::to_string(&next) {
        let _ = store.set_item(UI_KEY, &json);
    }
}

/// ホーム画面追加の案内を今後出さない。
///
/// クリック 1 回きりなので debounce しない（`save_debounced` と違って連打されない）。
pub fn dismiss_install_hint() {
    let Some(store) = store() else {
        return;
    };
    // ★ 読んでから 1 フィールドだけ差し替える。`UiState { install_hint_dismissed: true }`
    //   と書くとフィールドが増えたときに既存値を黙って消す。`..ui_state()` を足す形は
    //   フィールドが 1 つの間 clippy::needless_update に当たるので、この形にしておく
    let mut next = ui_state();
    next.install_hint_dismissed = true;
    if let Ok(json) = serde_json::to_string(&next) {
        let _ = store.set_item(UI_KEY, &json);
    }
}
