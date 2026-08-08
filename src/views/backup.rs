//! データの書き出し / 読み込みシート。
//!
//! 設計上の要点:
//!
//! - **取り込みの確認は「現在」と「読込後」を両方数字で並べる。** 片方だけ出す UI では
//!   「記録 0 日のファイルで全部消す」事故が止まらない。0 日が見えれば誰も押さない
//! - **取り込みの直前に必ず自動退避する**（`storage::snapshot_current`）。実行するのは
//!   定義上「データを失って動転している人」なので、戻せることが要る
//! - **iOS では `<a download>` を使わない。** `transfer::pick_route` が構造的に選ばない。
//!   standalone で踏むと WebView が blob URL へ遷移し、戻る UI が無いのでアプリごと
//!   固まる（`transfer.rs` のモジュールコメント参照）
//! - **textarea は折りたたみの中**。UX が悪いので既定では見せないが、共有シートも
//!   クリップボードも駄目だった端末に残る最後の逃げ道なので消さない
//! - textarea の `font-size` は 16px 以上（`styles.css` の `input` 指定は textarea に
//!   効かない）。16px 未満だと iOS がフォーカス時にページごとズームする
//! - `autocorrect="off"` は leptos の view! が受け付けないので付けていない。iOS が
//!   引用符を全角に変えることがあるが、`core::repair` が読み込み時に戻すので実害はない

use leptos::prelude::*;

use crate::core::{self, Conflict, DbSummary, MergeReport};
use crate::model::Db;
use crate::{storage, transfer};

use super::{kb_blur, kb_focus, use_db, use_kb};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pane {
    Export,
    Import,
}

/// 取り込み方。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// 丸ごと置き換える。移行（Safari → PWA）と復旧はこれ
    Replace,
    /// 足すだけ。既存は 1 つも書き換えない。2 台の統合はこれ
    Merge,
}

/// 確認待ちの取り込み。
#[derive(Clone)]
struct Pending {
    db: Db,
    summary: DbSummary,
}

fn summary_text(s: &DbSummary) -> String {
    let range = match (s.first, s.last) {
        (Some(a), Some(b)) if a == b => format!(" ・ {a}"),
        (Some(a), Some(b)) => format!(" ・ {a} 〜 {b}"),
        _ => String::new(),
    };
    format!(
        "種目 {} ・ 記録 {} 日 ・ {} セット{range}",
        s.exercises, s.days, s.sets
    )
}

fn conflict_text(c: &Conflict) -> String {
    match c {
        Conflict::Renamed { kept, incoming } => {
            format!("「{incoming}」は「{kept}」として扱いました")
        }
        Conflict::NameMatched { name } => format!("「{name}」は同じ種目とみなしました"),
        Conflict::SetsDiverged { date, name } => {
            format!("{date} の「{name}」は取り込んだ側のセットを採りました")
        }
        Conflict::BodyWeight { date } => format!("{date} の体重は元の値を残しました"),
    }
}

fn report_text(r: &MergeReport) -> String {
    if r.is_noop() {
        return "新しく取り込むものはありませんでした".to_string();
    }
    let mut parts = Vec::new();
    if r.sessions_added > 0 {
        parts.push(format!("{} 日分", r.sessions_added));
    }
    if r.logs_added > 0 {
        parts.push(format!("{} 件の記録", r.logs_added));
    }
    if r.exercises_added > 0 {
        parts.push(format!("{} 種目", r.exercises_added));
    }
    if r.groups_added > 0 {
        parts.push(format!("{} 部位", r.groups_added));
    }
    format!("{} を追加しました", parts.join(" ・ "))
}

/// 退避キーの末尾 epoch を読める日時にする。
fn backup_label(key: &str) -> String {
    let kind = if key.contains(".newer-") {
        "新しい版のデータ"
    } else if key.contains(".pre-") {
        "取り込み前の控え"
    } else {
        "読み込み失敗の退避"
    };
    let stamp = key
        .rsplit('-')
        .next()
        .and_then(|s| s.parse::<i64>().ok())
        .and_then(chrono::DateTime::from_timestamp_millis)
        .map(|dt| {
            dt.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "日時不明".to_string());
    format!("{stamp} ・ {kind}")
}

#[component]
pub fn BackupSheet(open: RwSignal<bool>) -> impl IntoView {
    let db = use_db();
    let kb = use_kb();

    let pane = RwSignal::new(Pane::Export);
    let pasted = RwSignal::new(String::new());
    let pending = RwSignal::new(None::<Pending>);
    let mode = RwSignal::new(Mode::Replace);
    let note = RwSignal::new(None::<String>);
    // 取り込み直前の退避キー。「元に戻す」で使う
    let undo = RwSignal::new(None::<String>);
    let backups = RwSignal::new(Vec::<String>::new());

    let refresh_backups = move || backups.set(storage::backup_keys());

    // 現在の DB の JSON。折りたたみの textarea と各経路が同じものを使う
    let payload = Memo::new(move |_| db.with(core::export_json));
    let current = Memo::new(move |_| db.with(core::summarize));

    let close = move || {
        open.set(false);
        pending.set(None);
        note.set(None);
        pasted.set(String::new());
    };

    // ── 書き出し ──
    let do_export = move |_| {
        // ★ ここは同期。await を挟むと iOS の transient activation（5 秒）を失う
        let json = payload.get_untracked();
        let name = core::export_filename(chrono::Local::now().naive_local());
        match transfer::pick_route() {
            transfer::Route::Share => {
                transfer::share_file(&name, &json, move |outcome| {
                    match outcome {
                    transfer::ShareOutcome::Shared => note.set(Some(
                        "書き出しました。「ファイルに保存」→ iCloud Drive を選ぶと、機種を替えても残ります".into(),
                    )),
                    // ★ キャンセルを成功にしない。「保存した」と思わせるのが一番危ない
                    transfer::ShareOutcome::Cancelled => {
                        note.set(Some("保存を中止しました（データは変わっていません）".into()))
                    }
                    transfer::ShareOutcome::Failed => note.set(Some(
                        "共有できませんでした。下の「うまくいかないとき」からコピーしてください".into(),
                    )),
                }
                });
            }
            transfer::Route::Download => {
                transfer::download_file(&name, &json);
                note.set(Some(format!("{name} を書き出しました")));
            }
            transfer::Route::Clipboard => {
                transfer::copy_text(&json);
                note.set(Some(
                    "コピーしました。メモや自分宛メールに貼り付けて保存してください".into(),
                ));
            }
        }
    };

    let do_copy = move |_| {
        transfer::copy_text(&payload.get_untracked());
        note.set(Some(
            "コピーしました。うまくいかないときは下のテキストを長押しして選択してください".into(),
        ));
    };

    // ── 読み込み ──
    let stage = move |raw: String| {
        let parsed = storage::with_ids(|ids| core::parse_import(&raw, ids));
        match parsed {
            Ok(next) => {
                let summary = core::summarize(&next);
                pending.set(Some(Pending { db: next, summary }));
                note.set(None);
            }
            Err(e) => {
                pending.set(None);
                note.set(Some(e.message()));
            }
        }
    };

    let on_file = move |ev: leptos::ev::Event| {
        use wasm_bindgen::JsCast;
        let Some(input) = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
        else {
            return;
        };
        transfer::read_file_text(&input, move |text| match text {
            Some(raw) => stage(raw),
            None => note.set(Some("ファイルを読めませんでした".into())),
        });
    };

    let apply = move |_| {
        let Some(Pending { db: next, .. }) = pending.get_untracked() else {
            return;
        };
        // ★ 何よりも先に控えを取る。失敗したらその事実を文言に出す
        let snapshot = storage::snapshot_current();
        let picked = mode.get_untracked();

        let message = match picked {
            Mode::Replace => {
                db.set(next);
                "取り込みました（置き換え）".to_string()
            }
            Mode::Merge => {
                let mut report = MergeReport::default();
                db.update(|cur| report = core::merge_db(cur, next));
                let mut text = report_text(&report);
                if !report.conflicts.is_empty() {
                    text.push('\n');
                    text.push_str(
                        &report
                            .conflicts
                            .iter()
                            .map(conflict_text)
                            .collect::<Vec<_>>()
                            .join("\n"),
                    );
                }
                text
            }
        };
        db.with_untracked(storage::replace_now);

        let message = match snapshot {
            Some(_) => message,
            None => format!("{message}\n（控えを保存できなかったので、元に戻せません）"),
        };
        undo.set(snapshot);
        pending.set(None);
        pasted.set(String::new());
        note.set(Some(message));
        refresh_backups();
    };

    let do_undo = move |_| {
        let Some(key) = undo.get_untracked() else {
            return;
        };
        let Some(raw) = storage::read_backup(&key) else {
            note.set(Some("控えを読み出せませんでした".into()));
            return;
        };
        match storage::with_ids(|ids| core::parse_import(&raw, ids)) {
            Ok(prev) => {
                db.set(prev);
                db.with_untracked(storage::replace_now);
                undo.set(None);
                note.set(Some("元に戻しました".into()));
            }
            Err(e) => note.set(Some(e.message())),
        }
    };

    Effect::new(move |_| {
        if open.get() {
            refresh_backups();
        }
    });

    view! {
        <Show when=move || open.get()>
            <div
                class="sheet-backdrop"
                style="z-index:19"
                data-testid="backup-sheet-backdrop"
                on:click=move |_| close()
            ></div>
            <div
                class="sheet"
                style="z-index:20"
                role="dialog"
                aria-label="データの書き出し / 読み込み"
                data-testid="backup-sheet"
            >
                <header class="sheet-head">
                    <strong>"データの書き出し / 読み込み"</strong>
                    <button
                        class="icon-btn"
                        aria-label="閉じる"
                        data-testid="backup-sheet-close"
                        on:click=move |_| close()
                    >
                        "✕"
                    </button>
                </header>
                <div class="sheet-body">
                    <div class="opts">
                        <button
                            class:opt=true
                            class:on=move || pane.get() == Pane::Export
                            data-testid="backup-pane-export"
                            on:click=move |_| pane.set(Pane::Export)
                        >
                            "書き出し"
                        </button>
                        <button
                            class:opt=true
                            class:on=move || pane.get() == Pane::Import
                            data-testid="backup-pane-import"
                            on:click=move |_| pane.set(Pane::Import)
                        >
                            "読み込み"
                        </button>
                    </div>

                    <Show when=move || note.get().is_some()>
                        <p class="menu-note" role="status" data-testid="backup-note">
                            {move || note.get().unwrap_or_default()}
                        </p>
                    </Show>

                    <Show when=move || pane.get() == Pane::Export>
                        <p class="muted">{move || summary_text(&current.get())}</p>
                        <div class="sheet-actions">
                            <button
                                class="primary"
                                data-testid="backup-export"
                                on:click=do_export
                            >
                                "ファイルとして書き出す"
                            </button>
                        </div>
                        <p class="muted">
                            "メモ・自分宛メール・ファイルアプリ のどれかに保存してください。"
                            "端末の中だけに置くと機種変更で消えます"
                        </p>
                        <details>
                            <summary>"うまくいかないとき"</summary>
                            <div class="sheet-actions">
                                <button data-testid="backup-copy" on:click=do_copy>
                                    "コピー"
                                </button>
                            </div>
                            <textarea
                                class="json-box"
                                readonly
                                autocapitalize="off"
                                spellcheck="false"
                                data-testid="backup-json"
                                prop:value=move || payload.get()
                            ></textarea>
                        </details>
                    </Show>

                    <Show when=move || pane.get() == Pane::Import>
                        <Show
                            when=move || pending.get().is_some()
                            fallback=move || {
                                view! {
                                    <p class="muted">
                                        "書き出したファイルを選ぶか、下の欄に貼り付けてください"
                                    </p>
                                    // ★ accept は付けない（iOS の accept は壊れていて、
                                    //   Files ピッカーで .json が灰色になることがある）
                                    <input
                                        type="file"
                                        data-testid="backup-file"
                                        on:change=on_file
                                    />
                                    <details>
                                        <summary>"貼り付けで読み込む"</summary>
                                        <textarea
                                            class="json-box"
                                            autocapitalize="off"
                                                        spellcheck="false"
                                            data-testid="backup-paste"
                                            on:focusin=move |_| kb_focus(kb)
                                            on:focusout=move |_| kb_blur(kb)
                                            on:input=move |ev| pasted.set(event_target_value(&ev))
                                            prop:value=move || pasted.get()
                                        ></textarea>
                                        <div class="sheet-actions">
                                            <button
                                                data-testid="backup-paste-load"
                                                on:click=move |_| stage(pasted.get_untracked())
                                            >
                                                "読み込む"
                                            </button>
                                        </div>
                                    </details>
                                }
                            }
                        >
                            // ★ 現在と読込後を両方出す。片方だけでは事故が止まらない
                            <div class="warn-box" data-testid="backup-confirm">
                                <p>
                                    <strong>"現在"</strong>
                                    " "
                                    {move || summary_text(&current.get())}
                                </p>
                                <p>
                                    <strong>"読込後"</strong>
                                    " "
                                    {move || {
                                        pending
                                            .get()
                                            .map(|p| summary_text(&p.summary))
                                            .unwrap_or_default()
                                    }}
                                </p>
                            </div>
                            <div class="opts">
                                <button
                                    class:opt=true
                                    class:on=move || mode.get() == Mode::Replace
                                    data-testid="backup-mode-replace"
                                    on:click=move |_| mode.set(Mode::Replace)
                                >
                                    "置き換える"
                                </button>
                                <button
                                    class:opt=true
                                    class:on=move || mode.get() == Mode::Merge
                                    data-testid="backup-mode-merge"
                                    on:click=move |_| mode.set(Mode::Merge)
                                >
                                    "足すだけ"
                                </button>
                            </div>
                            <p class="muted">
                                {move || match mode.get() {
                                    Mode::Replace => "今の記録は控えを取ってから丸ごと入れ替えます",
                                    Mode::Merge => "今の記録は 1 つも書き換えず、無い分だけ足します",
                                }}
                            </p>
                            <div class="sheet-actions">
                                <button
                                    class="primary"
                                    data-testid="backup-apply"
                                    on:click=apply
                                >
                                    "取り込む"
                                </button>
                                <button
                                    class="link-btn"
                                    data-testid="backup-cancel"
                                    on:click=move |_| pending.set(None)
                                >
                                    "やめる"
                                </button>
                            </div>
                        </Show>

                        <Show when=move || undo.get().is_some()>
                            <div class="sheet-actions">
                                <button
                                    class="link-btn"
                                    data-testid="backup-undo"
                                    on:click=do_undo
                                >
                                    "元に戻す"
                                </button>
                            </div>
                        </Show>
                    </Show>

                    // ── 退避データ ──
                    // ADR-0012 が「退避データを UI から読む手段がない」と自認していた穴。
                    // ここが無いと、破損時に退避したデータは iPhone 単体では救出できない
                    <Show when=move || !backups.get().is_empty()>
                        <details data-testid="backup-quarantine">
                            <summary>
                                {move || format!("保管中のデータ（{} 件）", backups.get().len())}
                            </summary>
                            <ul class="backup-list">
                                <For
                                    each=move || backups.get()
                                    key=|key| key.clone()
                                    let:key
                                >
                                    {
                                        let for_show = key.clone();
                                        let for_remove = key.clone();
                                        view! {
                                            <li>
                                                <span class="muted">{backup_label(&key)}</span>
                                                <div class="sheet-actions">
                                                    <button
                                                        class="link-btn"
                                                        data-testid="backup-restore"
                                                        on:click=move |_| {
                                                            match storage::read_backup(&for_show) {
                                                                Some(raw) => {
                                                                    pane.set(Pane::Import);
                                                                    stage(raw);
                                                                }
                                                                None => {
                                                                    note.set(Some("読み出せませんでした".into()))
                                                                }
                                                            }
                                                        }
                                                    >
                                                        "中身を見る"
                                                    </button>
                                                    <button
                                                        class="link-btn danger"
                                                        data-testid="backup-delete"
                                                        on:click=move |_| {
                                                            storage::remove_key(&for_remove);
                                                            refresh_backups();
                                                        }
                                                    >
                                                        "削除"
                                                    </button>
                                                </div>
                                            </li>
                                        }
                                    }
                                </For>
                            </ul>
                        </details>
                    </Show>
                </div>
            </div>
        </Show>
    }
}
