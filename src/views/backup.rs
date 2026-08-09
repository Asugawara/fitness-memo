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
//! - **他アプリからの移行もここに置く**（`Pane::Text`）。専用のシートを立てると、
//!   自動退避 → 適用 → 元に戻す の分岐が 2 本になる。データを失う経路は 1 本に保つ

use std::collections::BTreeMap;

use leptos::prelude::*;

use crate::core::{self, Conflict, DbSummary, MergeReport};
use crate::import_text::{self, Assign, Draft};
use crate::model::{Db, GroupId};
use crate::{storage, transfer};

use super::{Sheet, kb_blur, kb_focus, use_dates, use_db, use_kb};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pane {
    Export,
    Import,
    /// 他アプリの画面から起こしたテキストを読む
    Text,
}

/// 確認待ちのデータがどこから来たか。
///
/// テキスト由来のものは**置き換えを選ばせない**。読み取り違いが混じり得る入力で
/// 既存を全部消す理由が無い。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Source {
    Json,
    Text,
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
    source: Source,
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
    // ★ ペインを跨いでも消さない。読み取り直しのたびに貼り直させるのは無しにする
    let text_raw = RwSignal::new(String::new());
    let pending = RwSignal::new(None::<Pending>);
    let mode = RwSignal::new(Mode::Replace);
    let note = RwSignal::new(None::<String>);
    // 取り込み直前の退避キー。「元に戻す」で使う
    let undo = RwSignal::new(None::<String>);
    // 「元に戻す」の確認待ち。巻き戻しは取り込みと同じくらい破壊的なので確認を挟む
    let confirm_undo = RwSignal::new(false);
    // 控えが取れなかったときの「それでも実行する」確認待ち
    let force = RwSignal::new(false);
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
        // ★ 「元に戻す」を持ち越さない。iOS の PWA は何日もレジュームされるので、
        //   残しておくと数日後に誤タップされ、その間の記録が消える
        undo.set(None);
        confirm_undo.set(false);
        force.set(false);
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
                // ★ 成否を待ってから文言を決める。失敗を「コピーしました」と出すと、
                //   書けたつもりで端末を初期化されかねない
                transfer::copy_text(&json, move |ok| {
                    note.set(Some(if ok {
                        "コピーしました。メモや自分宛メールに貼り付けて保存してください".into()
                    } else {
                        "コピーできませんでした。下の「うまくいかないとき」のテキストを長押しして選択してください".to_string()
                    }));
                });
            }
        }
    };

    let do_copy = move |_| {
        transfer::copy_text(&payload.get_untracked(), move |ok| {
            note.set(Some(if ok {
                "コピーしました。うまくいかないときは下のテキストを長押しして選択してください"
                    .into()
            } else {
                "コピーできませんでした。下のテキストを長押しして選択してください".to_string()
            }));
        });
    };

    // ── 読み込み ──
    let stage = move |raw: String| {
        let parsed = storage::with_ids(|ids| core::parse_import(&raw, ids));
        match parsed {
            Ok(next) => {
                let summary = core::summarize(&next);
                pending.set(Some(Pending {
                    db: next,
                    summary,
                    source: Source::Json,
                }));
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
        // ★ 何よりも先に控えを取る
        let snapshot = storage::snapshot_current();
        // ★ 控えが取れないまま進めてはいけない（`storage::snapshot_current` の契約）。
        //   容量が逼迫しているときに起きるが、そういう端末ほど失うものが大きい。
        //   実行**後**に「戻せません」と言われても手遅れなので、先に止める
        if snapshot.is_none() && !force.get_untracked() {
            force.set(true);
            note.set(Some(
                "控えを保存できません（空き容量が足りない可能性があります）。このまま取り込むと元に戻せません。もう一度押すと実行します"
                    .into(),
            ));
            return;
        }
        force.set(false);
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

        let message = match &snapshot {
            Some(_) => message,
            None => format!("{message}\n（控えを保存できなかったので、元に戻せません）"),
        };
        undo.set(snapshot);
        confirm_undo.set(false);
        pending.set(None);
        pasted.set(String::new());
        note.set(Some(message));
        refresh_backups();
    };

    let do_undo = move |_| {
        let Some(key) = undo.get_untracked() else {
            return;
        };
        // ★ 巻き戻しは取り込みと同じだけ破壊的（戻す先より後に付けた記録が消える）。
        //   一度確認を挟む
        if !confirm_undo.get_untracked() {
            confirm_undo.set(true);
            note.set(Some(format!(
                "{} の状態に戻します。それ以降に付けた記録は消えます。もう一度押すと実行します",
                backup_label(&key)
            )));
            return;
        }
        let Some(raw) = storage::read_backup(&key) else {
            note.set(Some("控えを読み出せませんでした".into()));
            return;
        };
        match storage::with_ids(|ids| core::parse_import(&raw, ids)) {
            Ok(prev) => {
                // ★ 巻き戻す前の状態も退避する。これが無いと「戻す」で消えた分を
                //   取り戻す手段が無くなる（復旧操作そのものが全損経路になる）
                let saved = storage::snapshot_current().is_some();
                db.set(prev);
                db.with_untracked(storage::replace_now);
                undo.set(None);
                confirm_undo.set(false);
                note.set(Some(if saved {
                    "元に戻しました（戻す前の状態も保管しています）".to_string()
                } else {
                    "元に戻しました（戻す前の状態は保管できませんでした）".to_string()
                }));
                refresh_backups();
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
        <Sheet
            open=open
            on_close=Callback::new(move |_| close())
            title="データの書き出し / 読み込み".to_string()
            testid="backup-sheet"
            close_testid="backup-sheet-close"
        >
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
                        <button
                            class:opt=true
                            class:on=move || pane.get() == Pane::Text
                            data-testid="backup-pane-text"
                            on:click=move |_| {
                                pending.set(None);
                                note.set(None);
                                pane.set(Pane::Text);
                            }
                        >
                            "他アプリから"
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
                                // ★ クラスなしの <button> を作らない（adr/ux/declare-color-scheme-for-ua-widgets.md）。
                                //   UA 既定の chrome に任せるとダークで文字が消え、
                                //   タップ標的も 44px に届かない
                                <button
                                    class="secondary"
                                    data-testid="backup-copy"
                                    on:click=do_copy
                                >
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
                                                class="secondary"
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
                            // ★ テキスト由来のときは置き換えを出さない。読み取り違いが
                            //   混じり得る入力で既存を全部消す道を残す理由が無い
                            <Show when=move || {
                                pending.get().is_some_and(|p| p.source == Source::Json)
                            }>
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
                            </Show>
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

                    <Show when=move || pane.get() == Pane::Text>
                        <TextPane
                            raw=text_raw
                            pane=pane
                            mode=mode
                            pending=pending
                            note=note
                        />
                    </Show>

                    // ── 退避データ ──
                    // adr/storage/quarantine-on-parse-failure.md が「退避データを UI から読む手段がない」と自認していた穴。
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
        </Sheet>
    }
}

// ── 他アプリから ────────────────────────────────────────────────────────────

/// 種目 1 つ分の割り当て行。
#[derive(Clone, PartialEq)]
struct Row {
    /// `import_text` の照合キー。割り当てはこれで引く
    key: String,
    /// 読み取った表記
    display: String,
    /// 既存の種目に当たったならその名前
    target: Option<String>,
    sets: usize,
}

/// 他アプリの画面から起こしたテキストを読むペイン。
///
/// ここは**確認画面まで**を作る。取り込みそのものは読み込みペインの `apply` に
/// 渡す（自動退避 → 適用 → 元に戻す の分岐を 2 本にしないため）。
#[component]
fn TextPane(
    /// 貼り付けた元テキスト。ペインを跨いでも消えないよう親が持つ
    raw: RwSignal<String>,
    pane: RwSignal<Pane>,
    mode: RwSignal<Mode>,
    pending: RwSignal<Option<Pending>>,
    note: RwSignal<Option<String>>,
) -> impl IntoView {
    let db = use_db();
    let dates = use_dates();
    let kb = use_kb();

    let draft = RwSignal::new(None::<Draft>);
    // 新規種目の行き先。**選ばれなかった種目は取り込まない**
    let assign = RwSignal::new(BTreeMap::<String, Assign>::new());
    // 日付が読めなかった塊を載せる日
    let fallback = RwSignal::new(String::new());

    let read = move |_| {
        let text = raw.get_untracked();
        if text.trim().is_empty() {
            note.set(Some("中身がありません".into()));
            return;
        }
        let today = dates.today.get_untracked();
        let parsed = db.with_untracked(|cur| import_text::parse(&text, cur, today));
        note.set(if parsed.is_empty() {
            Some("記録を読み取れませんでした。種目名の行と「60kg × 10」のような行が要ります".into())
        } else {
            None
        });
        assign.set(BTreeMap::new());
        fallback.set(core::date_key(today));
        draft.set(Some(parsed));
    };

    let rows = Memo::new(move |_| {
        let mut rows: Vec<Row> = Vec::new();
        draft.with(|d| {
            let Some(d) = d else { return };
            db.with(|cur| {
                for day in &d.days {
                    for log in &day.logs {
                        match rows.iter_mut().find(|r| r.key == log.key) {
                            Some(row) => row.sets += log.sets.len(),
                            None => rows.push(Row {
                                key: log.key.clone(),
                                display: log.raw_name.clone(),
                                target: log
                                    .matched
                                    .and_then(|id| cur.exercise(id))
                                    .map(|e| e.name.clone()),
                                sets: log.sets.len(),
                            }),
                        }
                    }
                }
            });
        });
        rows
    });

    // 部位は既存のものだけ。移行で新しい部位を勝手に増やさない
    let groups = Memo::new(move |_| {
        db.with(|cur| {
            let mut list: Vec<(u32, GroupId, String)> = cur
                .groups
                .iter()
                .map(|g| (g.order, g.id, g.name.clone()))
                .collect();
            list.sort_by_key(|(order, _, _)| *order);
            list.into_iter()
                .map(|(_, id, name)| (id, name))
                .collect::<Vec<_>>()
        })
    });

    let summary_line = move || {
        draft.with(|d| {
            d.as_ref().map_or(String::new(), |d| {
                let (exercises, days, sets) = d.counts();
                let weighed = d
                    .days
                    .iter()
                    .filter(|day| day.body_weight.is_some())
                    .count();
                let weight = if weighed > 0 {
                    format!(" ・ 体重 {weighed} 日分")
                } else {
                    String::new()
                };
                format!("種目 {exercises} ・ {days} 日分 ・ {sets} セット{weight}を読み取りました")
            })
        })
    };

    let confirm = move |_| {
        let Some(parsed) = draft.get_untracked() else {
            return;
        };
        let today = dates.today.get_untracked();
        let landing = core::parse_date_key(&fallback.get_untracked()).unwrap_or(today);
        let picked = assign.get_untracked();
        let built = db.with_untracked(|cur| {
            storage::with_ids(|ids| import_text::to_db(&parsed, &picked, landing, cur, ids))
        });
        let summary = core::summarize(&built);
        if summary.sets == 0 {
            note.set(Some(
                "取り込むものがありません（新しい種目は部位を選ぶと取り込まれます）".into(),
            ));
            return;
        }
        note.set(None);
        // ★ テキスト由来は必ず「足すだけ」
        mode.set(Mode::Merge);
        pending.set(Some(Pending {
            db: built,
            summary,
            source: Source::Text,
        }));
        pane.set(Pane::Import);
    };

    view! {
        <details open data-testid="text-import-howto">
            <summary>"前のアプリから移す手順"</summary>
            <ol class="howto">
                <li>"前のアプリで記録の画面を出してスクリーンショットを撮る"</li>
                <li>"写真アプリでそのスクショを開き、右下の「テキスト認識表示」を押す"</li>
                <li>"文字を長押しして「すべてを選択」→「コピー」"</li>
                <li>"下の欄に貼り付けて「読み取る」"</li>
            </ol>
            <p class="muted">
                "前のアプリのデータをこのアプリから直接読むことはできません（iPhone が"
                "アプリどうしを隔てているため）。画面の文字を写すのが唯一の道です"
            </p>
        </details>

        // ★ 読み取った後は縮める。160px のまま居座ると、割り当ての行を見るのに
        //   毎回スクロールすることになる。消さないのは、読み違いを直して読み直す
        //   のがこの機能で一番よくある操作だから
        <textarea
            class="json-box"
            class:compact=move || draft.with(Option::is_some)
            autocapitalize="off"
            spellcheck="false"
            data-testid="text-import-input"
            on:focusin=move |_| kb_focus(kb)
            on:focusout=move |_| kb_blur(kb)
            on:input=move |ev| raw.set(event_target_value(&ev))
            prop:value=move || raw.get()
        ></textarea>
        <div class="sheet-actions">
            <button class="primary" data-testid="text-import-read" on:click=read>
                "読み取る"
            </button>
        </div>

        <Show when=move || draft.with(|d| d.as_ref().is_some_and(|d| !d.is_empty()))>
            <p class="muted" data-testid="text-import-summary">
                {summary_line}
            </p>

            <Show when=move || draft.with(|d| d.as_ref().is_some_and(|d| d.converted_lb))>
                <p class="menu-note" data-testid="text-import-lb">
                    "lb で書かれていた重量は kg に換算しました"
                </p>
            </Show>

            <ul class="backup-list" data-testid="text-import-rows">
                <For each=move || rows.get() key=|row| row.key.clone() let:row>
                    {
                        let key = row.key.clone();
                        view! {
                            <li>
                                <span>{row.display.clone()}</span>
                                " "
                                <span class="muted">{format!("（{} セット）", row.sets)}</span>
                                {match row.target.clone() {
                                    Some(name) => {
                                        view! {
                                            <p class="muted">{format!("→ {name}")}</p>
                                        }
                                            .into_any()
                                    }
                                    None => {
                                        view! {
                                            <select
                                                class="target-select"
                                                data-testid="text-import-assign"
                                                on:change=move |ev| {
                                                    let value = event_target_value(&ev);
                                                    let key = key.clone();
                                                    assign
                                                        .update(|picked| match value.parse::<GroupId>() {
                                                            Ok(group) => {
                                                                picked.insert(key, Assign::Into(group));
                                                            }
                                                            Err(_) => {
                                                                picked.remove(&key);
                                                            }
                                                        });
                                                }
                                            >
                                                <option value="">"取り込まない"</option>
                                                <For
                                                    each=move || groups.get()
                                                    key=|(id, _)| *id
                                                    let:group
                                                >
                                                    <option value=group
                                                        .0
                                                        .to_string()>{group.1.clone()}</option>
                                                </For>
                                            </select>
                                        }
                                            .into_any()
                                    }
                                }}
                            </li>
                        }
                    }
                </For>
            </ul>

            <Show when=move || draft.with(|d| d.as_ref().is_some_and(Draft::has_undated))>
                <p class="muted">"日付が書かれていない分があります。実際にトレーニングした日を選んでください"</p>
                <input
                    type="date"
                    class="date-input"
                    data-testid="text-import-date"
                    on:change=move |ev| fallback.set(event_target_value(&ev))
                    prop:value=move || fallback.get()
                />
            </Show>

        </Show>

        // ★ 読み取れなかった行を黙って捨てない。移行できたつもりで前のアプリを
        //   消されるのが、この機能で一番あり得ない結末。**「確認へ」より上に置く** —
        //   進んだ後に出しても、取りこぼしに気づく前に押されている
        <Show when=move || draft.with(|d| d.as_ref().is_some_and(|d| !d.ignored.is_empty()))>
            <details data-testid="text-import-ignored">
                <summary>
                    {move || {
                        let n = draft.with(|d| d.as_ref().map_or(0, |d| d.ignored.len()));
                        format!("読み取れなかった行（{n} 行）")
                    }}
                </summary>
                <ul class="backup-list">
                    <For
                        each=move || {
                            draft.with(|d| d.as_ref().map_or(Vec::new(), |d| d.ignored.clone()))
                        }
                        key=|line| line.clone()
                        let:line
                    >
                        <li class="muted">{line}</li>
                    </For>
                </ul>
            </details>
        </Show>

        <Show when=move || draft.with(|d| d.as_ref().is_some_and(|d| !d.is_empty()))>
            <div class="sheet-actions">
                <button class="primary" data-testid="text-import-confirm" on:click=confirm>
                    "確認へ"
                </button>
            </div>
        </Show>
    }
}
