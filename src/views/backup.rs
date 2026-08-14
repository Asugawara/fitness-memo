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
use crate::{sheet, storage, transfer};

use super::{Sheet, kb_blur, kb_focus, use_db, use_kb};

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

/// 何から取り込むか。**形式ごとに許す操作が違う。**
#[derive(Clone, Copy, PartialEq, Eq)]
enum Source {
    /// JSON。保存形式そのものなのでロスレス。置き換えも足すだけも選べる
    Json,
    /// 表計算（CSV / TSV / Google スプレッドシート）。ID・色・並び順・archived を
    /// 持たないので、置き換えると部位の色や並び順が黙って消える。**足すだけに固定する**
    Csv,
}

/// 確認待ちの取り込み。
#[derive(Clone)]
struct Pending {
    db: Db,
    summary: DbSummary,
    source: Source,
    /// 表から読んだときの内訳。**取り込めなかった行を黙って捨てないための口**
    report: Option<sheet::SheetReport>,
}

/// 表から読んだときに、確認画面へ足す行。
///
/// ★ **黙って 0 件取り込むことはしない。** 落とした行があるなら数と理由を出す。
/// 落とした行を見せずに「取り込みました」と言うのが、この機能で一番危ない出方になる。
fn sheet_note(r: &sheet::SheetReport) -> String {
    let mut lines = Vec::new();
    if !r.exercises_created.is_empty() {
        lines.push(format!(
            "新しく作る種目: {}",
            r.exercises_created.join("・")
        ));
    }
    if !r.skipped.is_empty() {
        lines.push(format!(
            "取り込めない行: {} / {} 件",
            r.skipped.len(),
            r.rows
        ));
        for s in r.skipped.iter().take(3) {
            lines.push(format!("{} 行目: {}", s.line, s.reason.message()));
        }
        if r.skipped.len() > 3 {
            lines.push(format!("ほか {} 件", r.skipped.len() - 3));
        }
    }
    lines.join("\n")
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
    // ★ メモだけが増えることがある（セットが同じでメモだけ違う日）。ここを出さないと
    //   `is_noop` が偽なのに parts が空になり「 を追加しました」だけが出る
    if r.notes_added > 0 {
        parts.push(format!("{} 件のメモ", r.notes_added));
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
    // 「元に戻す」の確認待ち。巻き戻しは取り込みと同じくらい破壊的なので確認を挟む
    let confirm_undo = RwSignal::new(false);
    // 控えが取れなかったときの「それでも実行する」確認待ち
    let force = RwSignal::new(false);
    let backups = RwSignal::new(Vec::<String>::new());
    // Google スプレッドシートの URL と、取得中かどうか
    let url = RwSignal::new(String::new());
    let loading = RwSignal::new(false);
    // ★ 取得の世代。シートを閉じたら進めて、飛んでいる応答を無効にする。
    //   `BackupSheet` は常時マウント（`<dialog>` を open で倒すだけ）なので、
    //   閉じたあとに応答が着くと、次に開いたとき**頼んでいない確認画面**が
    //   「取り込む」ボタン付きで出ている
    let fetch_gen = RwSignal::new(0u32);

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
        url.set(String::new());
        loading.set(false);
        fetch_gen.update(|g| *g = g.wrapping_add(1));
    };

    // ── 書き出し ──
    let do_export = move |_| {
        // ★ ここは同期。await を挟むと iOS の transient activation（5 秒）を失う
        let json = payload.get_untracked();
        let name = core::export_filename(chrono::Local::now().naive_local());
        match transfer::pick_route() {
            transfer::Route::Share => {
                transfer::share_file(&name, &json, transfer::JSON, move |outcome| {
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
                transfer::download_file(&name, &json, transfer::JSON);
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

    // ── 表計算への書き出し ──
    //
    // JSON とは用途が違う。あちらは復元のための控えで、こちらは Excel / Numbers /
    // Google スプレッドシートで開いて眺めるためのもの。だから `export_json` は
    // 分岐させず、別関数として並べる（core.rs「エクスポート形式 = 保存形式」）。
    let do_export_csv = move |_| {
        // ★ ここも同期。await を挟むと iOS の transient activation（5 秒）を失う
        let csv = db.with_untracked(sheet::export_csv);
        let name = sheet::export_csv_filename(chrono::Local::now().naive_local());
        match transfer::pick_route() {
            transfer::Route::Share => {
                transfer::share_file(&name, &csv, transfer::CSV, move |outcome| match outcome {
                    transfer::ShareOutcome::Shared => note.set(Some(
                        "書き出しました。Google ドライブに保存してスプレッドシートで開けます"
                            .into(),
                    )),
                    transfer::ShareOutcome::Cancelled => note.set(Some(
                        "保存を中止しました（データは変わっていません）".into(),
                    )),
                    transfer::ShareOutcome::Failed => note.set(Some(
                        "共有できませんでした。上の「ファイルとして書き出す」で保存してください"
                            .into(),
                    )),
                });
            }
            transfer::Route::Download => {
                transfer::download_file(&name, &csv, transfer::CSV);
                note.set(Some(format!("{name} を書き出しました")));
            }
            // ★ **クリップボードに落とさない。** 共有シートが使えない端末では
            //   ファイルを取り出せないが、CSV の文字列をコピーしても貼り付け先で
            //   1 列に潰れて表にならない。10 年分なら 1.28 MB を大域状態に置いた挙句、
            //   役にも立たないという出方になる。控えは JSON 側に任せて、できないと言う
            transfer::Route::Clipboard => {
                note.set(Some(
                    "この端末ではファイルを共有できません。上の「ファイルとして書き出す」を使ってください"
                        .into(),
                ));
            }
        }
    };

    // ── 読み込み ──
    //
    // ★ **入口を分けない。** ファイル選択・貼り付け・スプレッドシートの URL の
    // どれもここに来て、JSON か表かはこちらが見分ける。利用者に種別を選ばせると、
    // 選び間違いという新しい失敗を 1 つ増やすだけで、得るものが無い。
    let stage = move |raw: String| {
        match storage::with_ids(|ids| core::parse_import(&raw, ids)) {
            Ok(next) => {
                let summary = core::summarize(&next);
                pending.set(Some(Pending {
                    db: next,
                    summary,
                    source: Source::Json,
                    report: None,
                }));
                note.set(None);
            }
            Err(json_err) => {
                // JSON として読めなかった。表として読み直す
                let read =
                    db.with_untracked(|cur| storage::with_ids(|ids| sheet::parse(&raw, cur, ids)));
                match read {
                    Ok((next, report)) => {
                        let summary = core::summarize(&next);
                        // ★ `mode` は触らない。表では「置き換える」を描画せず `apply` が
                        //   構造的に Merge へ落とすので、ここで倒す必要が無い。倒すと
                        //   **次に JSON を読むとき既定が「足すだけ」になり**、復元の
                        //   つもりの人が気づかず統合してしまう
                        pending.set(Some(Pending {
                            db: next,
                            summary,
                            source: Source::Csv,
                            report: Some(report),
                        }));
                        note.set(None);
                    }
                    Err(sheet_err) => {
                        pending.set(None);
                        // ★ どちらの理由を出すかで、次の一手が決まる。JSON のつもりの
                        //   入力に「見出しがありません」と言っても直しようがないので、
                        //   中身の見た目で言い分けを変える
                        let looks_json = raw.trim_start().starts_with(['{', '[']);
                        let msg = match json_err {
                            core::ImportError::Empty | core::ImportError::Unsupported(_) => {
                                json_err.message()
                            }
                            _ if looks_json => json_err.message(),
                            _ => sheet_err.message(),
                        };
                        note.set(Some(msg));
                    }
                }
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

    // Google スプレッドシートの URL から読む。**このアプリで唯一の外部通信。**
    //
    // 送るのは URL だけで記録は外に出ない。取得できないときの理由は
    // `transfer::FetchError` が「共有設定」「オフライン」まで絞り込んでいる
    // （Google は非公開シートに CORS ヘッダ付きの 404 + text/html を返すので見分けられる）。
    let load_url = move |_| {
        if loading.get_untracked() {
            return;
        }
        match sheet::csv_url(&url.get_untracked()) {
            Err(e) => {
                pending.set(None);
                note.set(Some(e.message()));
            }
            Ok(target) => {
                loading.set(true);
                note.set(Some("スプレッドシートを読み込んでいます…".into()));
                let mine = fetch_gen.get_untracked();
                transfer::fetch_text(&target, move |res| {
                    // ★ 途中で閉じられていたら何もしない。`close()` が世代を進めている
                    if fetch_gen.get_untracked() != mine {
                        return;
                    }
                    loading.set(false);
                    match res {
                        Ok(text) => stage(text),
                        Err(e) => {
                            pending.set(None);
                            note.set(Some(e.message()));
                        }
                    }
                });
            }
        }
    };

    let apply = move |_| {
        let Some(Pending {
            db: next, source, ..
        }) = pending.get_untracked()
        else {
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
        // ★ 表からの取り込みは、画面の状態に関わらず「足すだけ」に落とす。
        //   `Mode::Replace` のボタンを描画していないことに寄りかかると、将来
        //   別の経路がステージしたときに部位の色と並び順が黙って消える
        let picked = match source {
            Source::Csv => Mode::Merge,
            Source::Json => mode.get_untracked(),
        };

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

                        // ── 表計算へ ──
                        // 控え（上の JSON）とは用途が違うので、間に区切りを入れて分ける。
                        // 主ボタンは 1 つのままにする（バックアップを取る導線を薄めない）
                        <hr class="sheet-rule" />
                        <div class="sheet-actions">
                            <button
                                class="secondary"
                                data-testid="backup-export-csv"
                                on:click=do_export_csv
                            >
                                "表計算用に書き出す（CSV）"
                            </button>
                        </div>

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

                                    // ── Google スプレッドシートから ──
                                    // iOS では、シートアプリから CSV を書き出して
                                    // ファイルアプリ経由で選ぶより、URL を貼るほうが速い。
                                    //
                                    // ★ 共有設定の事前案内は置かない。読み飛ばされるうえ、
                                    //   失敗したときは `transfer::FetchError::NotShared` が
                                    //   「共有 →「リンクを知っている全員」→「閲覧者」に」と
                                    //   直し方まで出す。**要るときに出るほうが読まれる**
                                    <hr class="sheet-rule" />
                                    // ★ type="url" にしない。UA の検証 UI（:user-invalid の赤枠）が
                                    //   入力途中に出るし、このアプリは他の入力も全部 text にしている
                                    //   （adr/ux/text-input-not-number.md）
                                    <input
                                        type="text"
                                        class="text-input"
                                        inputmode="url"
                                        autocapitalize="off"
                                        autocomplete="off"
                                        spellcheck="false"
                                        placeholder="https://docs.google.com/spreadsheets/..."
                                        data-testid="backup-sheet-url"
                                        on:focusin=move |_| kb_focus(kb)
                                        on:focusout=move |_| kb_blur(kb)
                                        on:input=move |ev| url.set(event_target_value(&ev))
                                        prop:value=move || url.get()
                                    />
                                    <div class="sheet-actions">
                                        <button
                                            class="secondary"
                                            data-testid="backup-sheet-load"
                                            prop:disabled=move || loading.get()
                                            on:click=load_url
                                        >
                                            "スプレッドシートから読み込む"
                                        </button>
                                    </div>

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

                            // ★ 表から読んだときは内訳を出す。落とした行を見せずに
                            //   「取り込みました」と言うのが、この機能で一番危ない出方になる
                            <Show when=move || {
                                pending.get().and_then(|p| p.report).is_some_and(|r| {
                                    !r.skipped.is_empty() || !r.exercises_created.is_empty()
                                })
                            }>
                                <p class="menu-note sheet-report" data-testid="backup-sheet-report">
                                    {move || {
                                        pending
                                            .get()
                                            .and_then(|p| p.report)
                                            .map(|r| sheet_note(&r))
                                            .unwrap_or_default()
                                    }}
                                </p>
                            </Show>

                            // ★ 表は ID・色・並び順を持たないので「置き換える」を出さない。
                            //   出すと、取り込んだ瞬間に部位の色と並び順が黙って消える
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
                            // ★ 表のときは選択肢が無いので、何が起きるかだけを言う。
                            //   「置き換えには使えません」と理由を足さない —— 選べない
                            //   ものの理由は、選べると思っている人にしか要らない
                            <p class="muted">
                                {move || {
                                    let csv = pending
                                        .get()
                                        .is_some_and(|p| p.source == Source::Csv);
                                    match (csv, mode.get()) {
                                        (false, Mode::Replace) => {
                                            "今の記録は控えを取ってから丸ごと入れ替えます"
                                        }
                                        _ => "今の記録は 1 つも書き換えず、無い分だけ足します",
                                    }
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
