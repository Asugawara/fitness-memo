//! エクスポート / インポートのシート。
//!
//! 設計上の要点:
//!
//! - **1 画面に「エクスポート」と「インポート」を並べる。** ペイン切替も折りたたみも置かない
//!   （adr/ux/one-screen-export-import.md）。ここは年に数回しか通らない導線なので、
//!   稀な失敗のために常設 UI を積むと、毎回の 1 タップを全員が払うことになる
//! - **取り込みは「足すだけ」に固定**（adr/storage/import-is-merge-only.md）。
//!   `core::merge_db` は種目も記録日もセットも減らさないので、「記録 0 日のファイルで
//!   全部消す」が構造的に起きない。だからモードを選ばせる必要がなくなった
//! - **確認画面の「取り込み後」は、マージ**済み**の結果**。取り込むファイル自身の要約を
//!   出してはいけない — 足すだけになった今、それは「取り込み後の姿」ではない。
//!   事故を止める唯一の道具が嘘をつくのが最悪なので、表示した数字と適用する `Db` は
//!   同じ計算（`stage` で 1 回だけ走る `merge_db`）の産物にしてある
//! - **取り込みの直前に必ず自動退避する**（`storage::snapshot_current`）。ただし退避に
//!   失敗しても**止めない**。実行前に「もう一度押すと実行します」と出す形は、容量が
//!   逼迫した端末で毎回 2 度押しを強いるだけで、結果は同じ。戻せないことは結果の文言で伝える
//! - **`.pre-` 退避は保存形式（JSON）なので `core::migrate` で読む。** `parse_import` は
//!   「外から来たファイル」用で、控えは外から来ていない。層を混ぜると、書き出し形式を
//!   変えた瞬間に「元に戻す」が静かに壊れる
//! - **iOS では `<a download>` を使わない。** `transfer::pick_route` が構造的に選ばない。
//!   standalone で踏むと WebView が blob URL へ遷移し、戻る UI が無いのでアプリごと
//!   固まる（`transfer.rs` のモジュールコメント参照）
//! - **`<input type="file">` は視覚的に隠してボタンから開く**
//!   （adr/ux/hidden-file-input-behind-a-button.md）。`display:none` にはしない。
//!   `<Show>` の外に常時マウントするのは、中に置くと状態遷移のたびに `NodeRef` が
//!   無効化されて `click()` が空振りするため

use leptos::prelude::*;

use crate::core::{self, Conflict, DbSummary, MergeReport};
use crate::model::Db;
use crate::{storage, transfer};

use super::icon::{self, icon};
use super::{Sheet, cur_lang, t, use_db};

/// 確認待ちの取り込み。**マージ済みの結果**を持つ。
#[derive(Clone)]
struct Pending {
    /// 「取り込む」で `db` にそのまま入れる最終形
    merged: Db,
    /// `summarize(&merged)`。確認画面の「取り込み後」
    after: DbSummary,
    /// "7 日分 ・ 67 件の記録" のような名詞句。何も増えないなら `None`
    added: Option<String>,
    /// 判断が要った箇所（`conflict_text` 済み）
    conflicts: Vec<String>,
}

fn summary_text(s: &DbSummary) -> String {
    // ★ 中黒と助詞で繋いでいた組み立ては `Lang::db_summary` に畳んである
    //   （語順も区切りも言語で変わるので、部品を渡して向こうで組む）
    // ★ 日付は ISO のまま（`NaiveDate` の Display）。ここは控えの範囲を示す数値で、
    //   読み上げる文ではないので `fmt_date` のロケール整形には通さない
    let range = s
        .first
        .zip(s.last)
        .map(|(a, b)| (a.to_string(), b.to_string()));
    cur_lang().db_summary(
        s.exercises,
        s.days,
        s.sets,
        range.as_ref().map(|(a, b)| (a.as_str(), b.as_str())),
    )
}

fn conflict_text(c: &Conflict) -> String {
    match c {
        Conflict::Renamed { kept, incoming } => cur_lang().conflict_renamed(incoming, kept),
        Conflict::NameMatched { name } => cur_lang().conflict_name_matched(name),
        Conflict::SetsDiverged { date, name } => cur_lang().conflict_sets_diverged(date, name),
        Conflict::BodyWeight { date } => cur_lang().conflict_body_weight(date),
        // ★ 無名のメニューは他の画面と同じ「（名前なし）」にする。生のまま入れると
        //   「メニュー「」は…」になり、何を指しているのか読めない文が出る。
        //   無名は UI からは作れないが、旧版や他端末のファイルには入りうる
        Conflict::RoutineDiverged { name } if name.trim().is_empty() => {
            t().backup.unnamed_routine.to_string()
        }
        Conflict::RoutineDiverged { name } => cur_lang().conflict_routine_diverged(name),
    }
}

/// 増えるものの名詞句。**語尾を付けない** — 確認では「を追加します」、実行後は
/// 「を追加」と付け替えるので、ここで文にすると両方に使えない。
fn added_text(r: &MergeReport) -> Option<String> {
    if r.is_noop() {
        return None;
    }
    let mut parts = Vec::new();
    if r.sessions_added > 0 {
        parts.push(cur_lang().added_days(r.sessions_added));
    }
    if r.logs_added > 0 {
        parts.push(cur_lang().added_logs(r.logs_added));
    }
    // ★ メモだけが増えることがある（セットが同じでメモだけ違う日）。ここを出さないと
    //   `is_noop` が偽なのに parts が空になり「 を追加します」だけが出る
    if r.notes_added > 0 {
        parts.push(cur_lang().added_notes(r.notes_added));
    }
    if r.exercises_added > 0 {
        parts.push(cur_lang().n_exercises(r.exercises_added));
    }
    if r.groups_added > 0 {
        parts.push(cur_lang().added_groups(r.groups_added));
    }
    // ★ メニューだけが増えることもある（`is_noop` に数えたものは必ずここにも出す）
    if r.routines_added > 0 {
        parts.push(cur_lang().added_routines(r.routines_added));
    }
    Some(parts.join(t().backup.join))
}

/// 確認画面の 1 行目。**「何も起きません」と言ってよい条件を 1 箇所に閉じる。**
///
/// ★ `MergeReport::is_noop` を単独で信じてはいけない。`Conflict::SetsDiverged` は
/// 既存のログを丸ごと差し替える（`merge_db` の `*existing = ExerciseLog { .. }`）のに、
/// カウンタがどれも増えないので `is_noop` は真になる。そのまま
/// 「新しく取り込むものはありません」と出すと、**セットが入れ替わる取り込みを
/// 「何も起きない」と言って実行させる**ことになる。確認画面が嘘をつくのが最悪。
fn change_text(p: &Pending) -> String {
    match (&p.added, p.conflicts.is_empty()) {
        (Some(added), _) => cur_lang().will_add(added),
        (None, false) => t().backup.replaces_records.to_string(),
        (None, true) => t().backup.nothing_new.to_string(),
    }
}

/// 控えのキー末尾 epoch を読める日時にする。`.pre-` しか渡らない。
fn snapshot_time(key: &str) -> String {
    key.rsplit('-')
        .next()
        .and_then(|s| s.parse::<i64>().ok())
        .and_then(chrono::DateTime::from_timestamp_millis)
        .map(|dt| {
            dt.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| t().backup.unknown_time.to_string())
}

#[component]
pub fn BackupSheet(open: RwSignal<bool>) -> impl IntoView {
    let db = use_db();

    let pending = RwSignal::new(None::<Pending>);
    let note = RwSignal::new(None::<String>);
    // 取り込み直前の退避キー。「元に戻す」で使う
    let undo = RwSignal::new(None::<String>);
    // 「元に戻す」の確認待ち。巻き戻しは取り込みと同じくらい破壊的なので確認を挟む
    let confirm_undo = RwSignal::new(false);
    // 共有に失敗したときだけ出す救済ボタン。静止時の要素数はゼロ
    let copy_rescue = RwSignal::new(false);
    let file_ref: NodeRef<leptos::html::Input> = NodeRef::new();

    let current = Memo::new(move |_| db.with(core::summarize));

    let close = move || {
        open.set(false);
        pending.set(None);
        note.set(None);
        copy_rescue.set(false);
        // ★ 「元に戻す」を持ち越さない。iOS の PWA は何日もレジュームされるので、
        //   残しておくと数日後に誤タップされ、その間の記録が消える
        undo.set(None);
        confirm_undo.set(false);
    };

    // ── エクスポート ──
    let do_export = move |_| {
        // ★ ここは同期。await を挟むと iOS の transient activation（5 秒）を失う。
        //   `chrono::Local` を触るのはこの層の仕事で、`core::export_tsv` は
        //   オフセットを引数で受けて実行環境非依存を保っている
        let now = chrono::Local::now();
        let tsv = db.with_untracked(|d| core::export_tsv(d, *now.offset(), cur_lang()));
        let name = core::export_filename(now.naive_local());
        copy_rescue.set(false);
        // ★ 「もう一度押すと実行します」の警告文をこの後 note が上書きするので、
        //   武装も一緒に解く。文字が消えたのに 1 タップで発火する状態を残さない
        confirm_undo.set(false);
        match transfer::pick_route() {
            transfer::Route::Share => {
                transfer::share_file(&name, &tsv, core::TSV_MIME, move |outcome| {
                    match outcome {
                        transfer::ShareOutcome::Shared => {
                            note.set(Some({ t().backup.exported_share }.into()))
                        }
                        // ★ キャンセルを成功にしない。「保存した」と思わせるのが一番危ない
                        transfer::ShareOutcome::Cancelled => {
                            note.set(Some(t().backup.export_cancelled.into()))
                        }
                        transfer::ShareOutcome::Failed => {
                            // 失敗した人にだけ最後の逃げ道を出す
                            copy_rescue.set(true);
                            note.set(Some(t().backup.share_failed.into()));
                        }
                    }
                });
            }
            transfer::Route::Download => {
                transfer::download_file(&name, &tsv, core::TSV_MIME);
                note.set(Some(cur_lang().exported_to(&name)));
            }
            transfer::Route::Clipboard => {
                // ★ 成否を待ってから文言を決める。失敗を「コピーしました」と出すと、
                //   書けたつもりで端末を初期化されかねない
                transfer::copy_text(&tsv, move |ok| {
                    note.set(Some(if ok {
                        t().backup.copied.into()
                    } else {
                        t().backup.copy_failed.to_string()
                    }));
                });
            }
        }
    };

    let do_copy = move |_| {
        confirm_undo.set(false);
        let now = chrono::Local::now();
        let tsv = db.with_untracked(|d| core::export_tsv(d, *now.offset(), cur_lang()));
        transfer::copy_text(&tsv, move |ok| {
            note.set(Some(if ok {
                t().backup.copied.into()
            } else {
                t().backup.copy_failed.to_string()
            }));
        });
    };

    // ── インポート ──
    let open_picker = move |_| {
        // ★ クリックハンドラから同期的に。iOS はここでジェスチャの活性を見る
        if let Some(input) = file_ref.get_untracked() {
            input.click();
        }
    };

    // 読み込んだ文字列 → マージ結果を確認画面に載せる
    let stage = move |raw: String| {
        let parsed =
            db.with_untracked(|mine| storage::with_ids(|ids| core::parse_import(&raw, ids, mine)));
        match parsed {
            Ok(incoming) => {
                // ★ ここで一度だけマージし、その結果を確認画面と適用の**両方**に使う。
                //   2 回計算すると、表示した数字と実際に入るものが食い違う経路ができる
                let mut merged = db.get_untracked();
                let report = core::merge_db(&mut merged, incoming);
                let after = core::summarize(&merged);
                pending.set(Some(Pending {
                    merged,
                    after,
                    added: added_text(&report),
                    conflicts: report.conflicts.iter().map(conflict_text).collect(),
                }));
                note.set(None);
                copy_rescue.set(false);
                confirm_undo.set(false);
            }
            Err(e) => {
                pending.set(None);
                note.set(Some(e.message(cur_lang())));
                confirm_undo.set(false);
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
            None => note.set(Some(t().backup.file_unreadable.into())),
        });
    };

    let apply = move |_| {
        let Some(p) = pending.get_untracked() else {
            return;
        };
        // ★ 何よりも先に控えを取る。取れなくても**止めない** — 実行前に 2 度押しを
        //   強いても結果は同じで、容量が逼迫した端末で毎回 2 タップ増えるだけ。
        //   戻せないことは下の文言で伝える
        let snapshot = storage::snapshot_current();

        db.set(p.merged);
        db.with_untracked(storage::replace_now);

        // ★ 確認画面と同じ規則で言う。片方だけ `is_noop` を信じると、確認では
        //   「入れ替わる記録があります」と言ったのに結果は「増えたものはありません」
        //   になって、どちらが本当か分からなくなる
        let mut message = match (&p.added, p.conflicts.is_empty()) {
            (Some(added), _) => cur_lang().imported_with(added),
            (None, false) => t().backup.imported.to_string(),
            (None, true) => t().backup.imported_nothing_new.to_string(),
        };
        if !p.conflicts.is_empty() {
            message.push('\n');
            message.push_str(&p.conflicts.join("\n"));
        }
        if snapshot.is_none() {
            message.push_str(t().backup.no_undo_available);
        }

        undo.set(snapshot);
        confirm_undo.set(false);
        pending.set(None);
        note.set(Some(message));
    };

    let do_undo = move |_| {
        let Some(key) = undo.get_untracked() else {
            return;
        };
        // ★ 巻き戻しは取り込みと同じだけ破壊的（戻す先より後に付けた記録が消える）。
        //   一度確認を挟む
        if !confirm_undo.get_untracked() {
            confirm_undo.set(true);
            note.set(Some(cur_lang().undo_arm(&snapshot_time(&key))));
            return;
        }
        let Some(raw) = storage::read_backup(&key) else {
            note.set(Some(t().backup.undo_unreadable.into()));
            return;
        };
        // ★ 控えは**保存形式**（JSON）なので `migrate` で読む。`parse_import` は
        //   「外から来たファイル」用で、そちらに通すと書き出し形式を変えるたびに壊れる
        match storage::with_ids(|ids| core::migrate(&raw, ids)) {
            Ok(prev) => {
                // ★ 巻き戻す前の状態も退避する。これが無いと「戻す」で消えた分を
                //   取り戻す手段が無くなる（復旧操作そのものが全損経路になる）
                let saved = storage::snapshot_current().is_some();
                db.set(prev);
                db.with_untracked(storage::replace_now);
                undo.set(None);
                confirm_undo.set(false);
                // ★ 確認待ちを捨てる。`Pending::merged` は**巻き戻す前**の `db` から
                //   作ったので、残したまま「取り込む」を押されると、今戻したものが
                //   そのまま戻ってくる（巻き戻しが 1 タップで打ち消される）
                pending.set(None);
                note.set(Some(if saved {
                    // 退避の一覧 UI は無いので「保管しています」とだけ言い、
                    // 取り出せるかのように書かない（adr/ux/one-screen-export-import.md）
                    t().backup.undone.to_string()
                } else {
                    t().backup.undone_no_redo.to_string()
                }));
            }
            Err(_) => note.set(Some(t().backup.undo_unreadable.into())),
        }
    };

    view! {
        <Sheet
            open=open
            on_close=Callback::new(move |_| close())
            title=t().backup.sheet_title.to_string()
            testid="backup-sheet"
            close_testid="backup-sheet-close"
        >
            <Show when=move || note.get().is_some()>
                <p class="settings-note backup-note" role="status" data-testid="backup-note">
                    {move || note.get().unwrap_or_default()}
                </p>
            </Show>

            // ★ 通知の直下に置く。どのメッセージに紐づく操作なのかが読み取れる位置。
            //   **確認待ちのときは出さない** — 1 画面 1 判断にするだけでなく、確認中に
            //   巻き戻されると `Pending::merged`（巻き戻す前の `db` から作った）が
            //   古くなり、「取り込む」で巻き戻しを打ち消してしまう
            <Show when=move || undo.get().is_some() && pending.with(Option::is_none)>
                <div class="sheet-actions">
                    <button class="link-btn" data-testid="backup-undo" on:click=do_undo>
                        {t().backup.undo}
                    </button>
                </div>
            </Show>

            <Show
                when=move || pending.get().is_some()
                fallback=move || {
                    view! {
                        <p class="muted">{move || summary_text(&current.get())}</p>
                        <div class="sheet-actions">
                            // ★ アイコンは装飾（`icon()` が `aria-hidden` を付ける）。
                            //   名前はボタンの文字が持つので、読み上げは「エクスポート」だけになる
                            <button class="primary wide" data-testid="backup-export" on:click=do_export>
                                {icon(icon::UPLOAD)}
                                {t().backup.export}
                            </button>
                        </div>
                        // ★ 共有に失敗した人にだけ出す。静止時は 1 要素も増やさない
                        <Show when=move || copy_rescue.get()>
                            <div class="sheet-actions">
                                <button class="secondary" data-testid="backup-copy" on:click=do_copy>
                                    {t().backup.copy_text}
                                </button>
                            </div>
                        </Show>

                        // ★ ボタンの下に説明文を置かない。「「ファイルに保存」を選ぶと〜」は
                        //   共有が成功した**そのとき**の通知に出るし、「今ある記録は消えず〜」は
                        //   取り込みを決める確認画面に出る。**必要な瞬間に出るものを、
                        //   常時見える位置に前置きしても読まれずに嵩むだけ**
                        <div class="backup-split">
                            <div class="sheet-actions">
                                <button
                                    class="secondary wide"
                                    data-testid="backup-import"
                                    on:click=open_picker
                                >
                                    {icon(icon::DOWNLOAD)}
                                    {t().backup.import}
                                </button>
                            </div>
                        </div>
                    }
                }
            >
                // ★ 現在と取り込み後を両方出す。何が増えるかを押す前に見せる。
                //   読み出しは全て `with`（`get` は `Pending` ごと clone するので、
                //   `merged: Db` が再描画のたびに丸ごと複製される）
                <div class="warn-box" data-testid="backup-confirm">
                    <p>
                        <strong>{t().backup.before}</strong>
                        " "
                        {move || summary_text(&current.get())}
                    </p>
                    <p>
                        <strong>{t().backup.after}</strong>
                        " "
                        {move || {
                            pending.with(|p| p.as_ref().map(|p| summary_text(&p.after)))
                                .unwrap_or_default()
                        }}
                    </p>
                    <p>
                        {move || {
                            pending
                                .with(|p| p.as_ref().map(change_text))
                                .unwrap_or_default()
                        }}
                    </p>
                    // ★ 判断が要った箇所は**押す前**に出す。実行後にだけ見せていると、
                    //   「取り込んだ側のセットを採りました」を読むのが手遅れになる
                    <Show when=move || pending.with(|p| p.as_ref().is_some_and(|p| !p.conflicts.is_empty()))>
                        <p class="backup-note">
                            {move || {
                                pending
                                    .with(|p| p.as_ref().map(|p| p.conflicts.join("\n")))
                                    .unwrap_or_default()
                            }}
                        </p>
                    </Show>
                </div>
                <p class="muted">{t().backup.merge_only}</p>
                <div class="sheet-actions">
                    <button class="primary wide" data-testid="backup-apply" on:click=apply>
                        {t().backup.apply}
                    </button>
                </div>
                <div class="sheet-actions">
                    <button
                        class="link-btn"
                        data-testid="backup-cancel"
                        on:click=move |_| pending.set(None)
                    >
                        {t().backup.cancel}
                    </button>
                </div>
            </Show>

            // ★ `<Show>` の**外**に常時マウントする。中に置くと状態遷移のたびに
            //   NodeRef が無効化され、`open_picker` の `click()` が空振りする。
            //   `accept` は付けない（iOS の accept は壊れていて、Files ピッカーで
            //   目当てのファイルが灰色になる）
            <input
                type="file"
                class="file-input"
                tabindex="-1"
                aria-hidden="true"
                data-testid="backup-file"
                node_ref=file_ref
                on:change=on_file
            />
        </Sheet>
    }
}
