//! 記録タブの下半分。**カレンダーで選んだ 1 日**の入力欄。
//!
//! 単独のタブではなく [`super::calendar`] が自分の下に置くコンポーネント。
//! 書き込み対象の日付は `DateCtx::selected` だけが決める（カレンダーの選択日と同一）。
//!
//! 経過時間と部位チップは「今日どこを鍛えるか」を決めるための情報なので、
//! カレンダーと入力欄に挟まれた**常時 1 行**に圧縮してある。以前は種目を 1 つでも
//! 追加したら畳む排他表示にしていたが、上にカレンダーが載って画面が縦に伸びた今は
//! 大きいヒーローを置く余地が無く、出し分ける意味も無くなった。

use chrono::NaiveDate;
use leptos::prelude::*;

use crate::core;
use crate::core::Metric;
use crate::model::{Db, ExerciseId, ExerciseLog, SetEntry};

use super::{
    fmt_date, fmt_metric, fmt_set, fmt_weight, kb_blur, kb_focus, now_ms, parse_reps, parse_weight,
    recency_class, scroll_to_id, short_elapsed, use_dates, use_db, use_kb,
};

/// 今日タブが表示しているカード 1 枚。
///
/// 日付キーを持つのは `<For>` のキーに混ぜるため。過去日へ切り替えたときにカードの
/// DOM ごと作り直さないと、カード内の編集中文字列が前の日のまま残る。
#[derive(Clone, PartialEq, Eq)]
struct CardRef {
    date: String,
    ex: ExerciseId,
}

/// 編集中のセット 1 行。
///
/// **保存モデル（`SetEntry`）ではなく文字列で持つ。** `f32` / `u32` では空欄が表現できず
/// 0 に落ちるうえ、`"6."` のような中間状態を保持できない。
#[derive(Clone, Debug, PartialEq)]
struct Row {
    key: u32,
    weight: String,
    reps: String,
}

impl Row {
    fn blank(key: u32) -> Self {
        Self {
            key,
            weight: String::new(),
            reps: String::new(),
        }
    }
}

fn card_dom_id(ex: ExerciseId) -> String {
    format!("card-{ex}")
}

fn confirm_dom_id(ex: ExerciseId) -> String {
    format!("confirm-{ex}")
}

/// その日・その種目のセットを丸ごと差し替える。
fn write_log(db: &mut Db, date: NaiveDate, ex: ExerciseId, sets: Vec<SetEntry>, is_today: bool) {
    let key = core::date_key(date);
    if sets.is_empty() {
        if let Some(session) = db.sessions.get_mut(&key) {
            session.logs.retain(|l| l.exercise_id != ex);
        }
    } else {
        let session = db.sessions.entry(key.clone()).or_default();
        match session.logs.iter_mut().find(|l| l.exercise_id == ex) {
            Some(log) => {
                log.sets = sets;
                // ★ at は当日入力時のみ埋める。過去日バックフィルは None のまま。
                //   ここで now を入れると「最後のトレーニングから」が「たった今」になり
                //   明示要件の出力が嘘になる
                if is_today {
                    log.at = Some(now_ms());
                }
            }
            None => session.logs.push(ExerciseLog {
                exercise_id: ex,
                sets,
                at: is_today.then(now_ms),
            }),
        }
    }
    // ログも体重もメモも無くなった日は残さない（過去日を閲覧しただけで実施日にしない）
    if db.sessions.get(&key).is_some_and(|s| s.is_empty()) {
        db.sessions.remove(&key);
    }
}

#[component]
pub fn DayEditor() -> impl IntoView {
    let db = use_db();
    let dates = use_dates();

    let cards: RwSignal<Vec<CardRef>> = RwSignal::new(Vec::new());
    let sheet = RwSignal::new(false);

    // 日付が変わったらカードを Db から引き直す。
    // db は untracked で読む（1 文字打つたびにカードが作り直されるのを防ぐ）
    Effect::new(move |_| {
        let key = core::date_key(dates.selected.get());
        let ids = db.with_untracked(|d| {
            d.sessions
                .get(&key)
                .map(|s| s.logs.iter().map(|l| l.exercise_id).collect::<Vec<_>>())
                .unwrap_or_default()
        });
        cards.set(
            ids.into_iter()
                .map(|ex| CardRef {
                    date: key.clone(),
                    ex,
                })
                .collect(),
        );
    });

    // ★ ヒーローと部位チップは「今日を始める前」の間隔を出す。
    //   今日のセッションを外した snapshot に core の関数をそのまま当てることで、
    //   1 セット入れた瞬間に「たった今」へ化けて意味を失うのを防ぐ。
    //   Memo なので今日のキー入力では下流が再評価されない。
    let before_today = Memo::new(move |_| {
        let today = dates.today.get();
        db.with(|d| {
            let mut snapshot = d.clone();
            snapshot.sessions.remove(&core::date_key(today));
            snapshot
        })
    });

    let elapsed = Memo::new(move |_| {
        let today = dates.today.get();
        before_today.with(|snapshot| core::elapsed_since_last(snapshot, now_ms(), today))
    });

    // 部位チップ 1 個ぶんのデータ。
    type Chip = (String, Option<crate::core::Elapsed>);

    let all_chips = Memo::new(move |_| {
        let today = dates.today.get();
        before_today.with(|snapshot| {
            let by_group = core::elapsed_by_group(snapshot, now_ms(), today);
            let mut groups = snapshot.groups.clone();
            groups.sort_by_key(|g| g.order);
            groups
                .into_iter()
                .map(|g| {
                    let e = by_group.get(&g.id).copied();
                    (g.name, e) as Chip
                })
                .collect::<Vec<_>>()
        })
    });

    let pick = move |ex: ExerciseId| {
        sheet.set(false);
        let date = core::date_key(dates.selected.get_untracked());
        let exists = cards.with_untracked(|cs| cs.iter().any(|c| c.ex == ex));
        if !exists {
            cards.update(|cs| cs.push(CardRef { date, ex }));
        }
        // 既にあるなら新規カードを作らず既存カードへスクロールする
        scroll_to_id(card_dom_id(ex));
    };

    view! {
        <section class="day" data-testid="screen-day">
            // ★ 経過と部位チップは常時 1 行。カレンダーのドットは「いつやったか」しか
            //   示さないので、「どの部位が空いているか」はここでしか読めない
            <div class="hero" data-testid="hero">
                // ラベルと値を分けて持つ（値だけを読めるようにしておく）
                <span class="hero-elapsed">
                    "最後から "
                    <b data-testid="elapsed">
                        {move || {
                            elapsed.get().map_or_else(|| "—".to_string(), core::humanize)
                        }}
                    </b>
                </span>
                <div class="chips" data-testid="group-chips">
                    {move || {
                        all_chips
                            .get()
                            .into_iter()
                            .map(|(name, e)| {
                                let label = e.map_or_else(|| "—".to_string(), short_elapsed);
                                view! {
                                    <span
                                        class="chip"
                                        data-recency=recency_class(e)
                                        data-testid="group-chip"
                                    >
                                        <b>{name}</b>
                                        <i>{label}</i>
                                    </span>
                                }
                            })
                            .collect::<Vec<_>>()
                    }}
                </div>
            </div>

            <header class="day-head" class:past=move || dates.is_past_edit()>
                <h1 data-testid="today-date">{move || fmt_date(dates.selected.get())}</h1>
                {move || {
                    if dates.is_past_edit() {
                        view! {
                            <button
                                class="link-btn"
                                data-testid="back-to-today"
                                on:click=move |_| dates.back_to_today()
                            >
                                "今日へ戻る"
                            </button>
                        }
                            .into_any()
                    } else {
                        view! { <span class="badge">"今日"</span> }.into_any()
                    }
                }}
            </header>

            {move || {
                dates
                    .is_past_edit()
                    .then(|| {
                        view! {
                            <p class="past-banner" data-testid="past-banner">
                                {move || format!("{} を編集中", fmt_date(dates.selected.get()))}
                            </p>
                        }
                    })
            }}

            // 体重・体調メモ。日付が変わったら初期値ごと作り直す
            {move || {
                let _ = dates.selected.get();
                view! { <ConditionRow /> }
            }}

            <div class="cards">
                <For
                    each=move || cards.get()
                    key=|c| (c.date.clone(), c.ex)
                    children=move |c| view! { <ExerciseCard ex=c.ex cards=cards /> }
                />
            </div>

            <div class="add-wrap">
                <button
                    class="primary"
                    data-testid="add-exercise"
                    on:click=move |_| sheet.set(true)
                >
                    "種目を追加"
                </button>
            </div>

            {move || {
                sheet
                    .get()
                    .then(|| {
                        view! {
                            <div
                                class="sheet-backdrop"
                                on:click=move |_| sheet.set(false)
                                data-testid="sheet-backdrop"
                            ></div>
                            <div
                                class="sheet"
                                role="dialog"
                                aria-label="種目を追加"
                                data-testid="add-sheet"
                            >
                                <header class="sheet-head">
                                    <strong>"種目を追加"</strong>
                                    <button class="link-btn" on:click=move |_| sheet.set(false)>
                                        "閉じる"
                                    </button>
                                </header>
                                <div class="sheet-body">
                                    {move || {
                                        db.with(|d| {
                                            let mut groups = d.groups.clone();
                                            groups.sort_by_key(|g| g.order);
                                            groups
                                                .into_iter()
                                                .map(|g| {
                                                    let mut exercises: Vec<_> = d
                                                        .exercises
                                                        .iter()
                                                        .filter(|e| e.group_id == g.id && !e.archived)
                                                        .cloned()
                                                        .collect();
                                                    exercises.sort_by_key(|e| e.order);
                                                    view! {
                                                        <section class="sheet-group">
                                                            <h3 style=format!("--dot:{}", g.color)>{g.name}</h3>
                                                            <div class="pick-list">
                                                                {exercises
                                                                    .into_iter()
                                                                    .map(|e| {
                                                                        let id = e.id;
                                                                        let added = cards
                                                                            .with_untracked(|cs| cs.iter().any(|c| c.ex == id));
                                                                        view! {
                                                                            <button
                                                                                class="pick"
                                                                                class:added=added
                                                                                data-testid="pick-exercise"
                                                                                on:click=move |_| pick(id)
                                                                            >
                                                                                {e.name}
                                                                            </button>
                                                                        }
                                                                    })
                                                                    .collect::<Vec<_>>()}
                                                            </div>
                                                        </section>
                                                    }
                                                })
                                                .collect::<Vec<_>>()
                                        })
                                    }}
                                </div>
                            </div>
                        }
                    })
            }}
        </section>
    }
}

/// 体重・体調メモの折りたたみ 1 行。
#[component]
fn ConditionRow() -> impl IntoView {
    let db = use_db();
    let dates = use_dates();
    let kb = use_kb();

    let date_key = core::date_key(dates.selected.get_untracked());
    let (weight0, note0) = db.with_untracked(|d| {
        d.sessions
            .get(&date_key)
            .map(|s| {
                (
                    s.body_weight.map(fmt_weight).unwrap_or_default(),
                    s.note.clone(),
                )
            })
            .unwrap_or_default()
    });

    let open = RwSignal::new(!weight0.is_empty() || !note0.is_empty());
    let weight = RwSignal::new(weight0.clone());
    let note = RwSignal::new(note0.clone());

    // このコンポーネントは日付が変わるたび作り直されるので、キーはその都度引き直せばよい
    // （`String` を capture すると commit が Copy でなくなり view! の FnMut 要件に落ちる）
    let commit = move || {
        let body_weight = weight.with_untracked(|w| {
            w.trim()
                .replace(',', ".")
                .parse::<f32>()
                .ok()
                .filter(|v| v.is_finite() && *v > 0.0)
        });
        let text = note.get_untracked();
        let key = core::date_key(dates.selected.get_untracked());
        db.update(move |d| {
            {
                let session = d.sessions.entry(key.clone()).or_default();
                session.body_weight = body_weight;
                session.note = text;
            }
            if d.sessions.get(&key).is_some_and(|s| s.is_empty()) {
                d.sessions.remove(&key);
            }
        });
    };

    view! {
        <div class="condition" data-testid="condition">
            <button
                class="link-btn cond-toggle"
                data-testid="condition-toggle"
                on:click=move |_| open.update(|o| *o = !*o)
            >
                {move || if open.get() { "－ コンディション" } else { "＋ コンディション" }}
            </button>
            {move || {
                open
                    .get()
                    .then(|| {
                        view! {
                            <div class="cond-fields">
                                <label>
                                    "体重"
                                    <input
                                        type="text"
                                        inputmode="decimal"
                                        pattern="[0-9]*([.,][0-9]*)?"
                                        value=weight0.clone()
                                        data-testid="body-weight"
                                        on:focusin=move |_| kb_focus(kb)
                                        on:focusout=move |_| kb_blur(kb)
                                        on:input=move |ev| {
                                            weight.set(event_target_value(&ev));
                                            commit();
                                        }
                                    />
                                    <span class="unit">"kg"</span>
                                </label>
                                <label class="note">
                                    "メモ"
                                    <input
                                        type="text"
                                        value=note0.clone()
                                        data-testid="condition-note"
                                        on:focusin=move |_| kb_focus(kb)
                                        on:focusout=move |_| kb_blur(kb)
                                        on:input=move |ev| {
                                            note.set(event_target_value(&ev));
                                            commit();
                                        }
                                    />
                                </label>
                            </div>
                        }
                    })
            }}
        </div>
    }
}

#[component]
fn ExerciseCard(ex: ExerciseId, cards: RwSignal<Vec<CardRef>>) -> impl IntoView {
    let db = use_db();
    let dates = use_dates();
    let kb = use_kb();

    // Memo にするのは「値が変わったときだけ」下流を再描画させるため。
    // 素の closure だと db が動くたびに構造ごと作り直され、入力中の文字列が消える
    let name = Memo::new(move |_| {
        db.with(|d| d.exercise(ex).map(|e| e.name.clone()))
            .unwrap_or_else(|| "(削除された種目)".to_string())
    });
    let group_name = Memo::new(move |_| {
        db.with(|d| {
            d.exercise(ex)
                .and_then(|e| d.group(e.group_id))
                .map(|g| g.name.clone())
        })
        .unwrap_or_default()
    });

    let last = Memo::new(move |_| {
        let before = dates.selected.get();
        db.with(|d| core::last_log_before(d, ex, before).map(|(date, l)| (date, l.clone())))
    });

    // ★「前回をコピー」はその種目の今日のセットが空のときだけ出す。
    //   これで「置換か追記か」の曖昧さ・誤タップで入力済みセットを消す事故・undo の
    //   必要性がまとめて消える
    let show_copy = Memo::new(move |_| {
        let key = core::date_key(dates.selected.get());
        last.with(|l| l.is_some())
            && db.with(|d| {
                d.sessions
                    .get(&key)
                    .and_then(|s| s.log_of(ex))
                    .is_none_or(|l| l.sets.is_empty())
            })
    });

    let initial: Vec<Row> = db.with_untracked(|d| {
        d.sessions
            .get(&core::date_key(dates.selected.get_untracked()))
            .and_then(|s| s.log_of(ex))
            .map(|l| {
                l.sets
                    .iter()
                    .enumerate()
                    .map(|(i, s)| Row {
                        key: i as u32,
                        weight: fmt_weight(s.weight),
                        reps: s.reps.to_string(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    });
    let next_key = RwSignal::new(initial.len() as u32 + 1);
    let rows = RwSignal::new(if initial.is_empty() {
        vec![Row::blank(0)]
    } else {
        initial
    });

    // 「+ セット」で足した行。この行の回数欄へフォーカスを移したら None に戻す。
    let focus_key: RwSignal<Option<u32>> = RwSignal::new(None);
    // 削除確認を出している行。
    let pending_remove: RwSignal<Option<u32>> = RwSignal::new(None);
    // この種目をこの日から外す確認を出しているか。
    let confirm_close = RwSignal::new(false);

    // ★ この種目が普段「重量を使う」種目かを**実データから**判定する。
    //
    // 旧実装は `Kind::Weighted` で判定していたが `Kind` は無くなった。単に
    // 判定を消すわけにいかないのは、新しい指標が「重量が空 = 重量 1」だから。
    // ベンチプレスで重量を入れ忘れると 600 が黙って 10 になり、グラフが崩れても
    // 気づく手掛かりが無い。逆に全種目で警告を出すと懸垂 12 回の毎行に
    // 「重量未入力」が出て邪魔になる。
    //
    // 前回ログか、この日の他の行に重量が入っていれば「重量を使う種目」とみなす。
    let uses_weight = Memo::new(move |_| {
        last.with(|l| {
            l.as_ref()
                .is_some_and(|(_, log)| log.sets.iter().any(|s| s.weight > 0.0))
        }) || rows.with(|rs| rs.iter().any(|r| parse_weight(&r.weight) > 0.0))
    });

    let commit = move || {
        let sets: Vec<SetEntry> = rows.with_untracked(|rs| {
            rs.iter()
                .filter_map(|r| {
                    // 空行が 0×0 として保存されコピーで複製され続けるのを防ぐ
                    let reps = parse_reps(&r.reps)?;
                    Some(SetEntry {
                        weight: parse_weight(&r.weight),
                        reps,
                    })
                })
                .collect()
        });
        let date = dates.selected.get_untracked();
        let is_today = date == dates.today.get_untracked();
        db.update(|d| write_log(d, date, ex, sets, is_today));
    };

    let fresh_key = move || {
        let key = next_key.get_untracked();
        next_key.set(key + 1);
        key
    };

    // ★ 直前行の重量をコピーして足し、回数欄へフォーカスする。
    //
    // セットは 60×10 / 60×8 / 60×6 のように重量を据え置いて回数だけ変えるのが普通なので、
    // 「+ セット」→ 重量欄をタップ →打つ→ 回数欄をタップ →打つ、の 4 手のうち
    // 前半 2 手が毎セット無駄になっていた。プリフィル + フォーカスで回数を打つだけになる。
    let add_row = move |_| {
        let key = fresh_key();
        let weight =
            rows.with_untracked(|rs| rs.last().map(|r| r.weight.clone()).unwrap_or_default());
        rows.update(|rs| {
            rs.push(Row {
                key,
                weight,
                reps: String::new(),
            })
        });
        focus_key.set(Some(key));
        // 重量だけの行は parse_reps が None を返して保存されないので commit は要らない
    };

    let remove_row = move |key: u32| {
        rows.update(|rs| rs.retain(|r| r.key != key));
        if rows.with_untracked(Vec::is_empty) {
            let key = fresh_key();
            rows.update(|rs| rs.push(Row::blank(key)));
        }
        pending_remove.set(None);
        commit();
    };

    // 中身が空の行は消えるものが無いので確認を挟まない。
    let request_remove = move |key: u32| {
        let empty = rows.with_untracked(|rs| {
            rs.iter()
                .find(|r| r.key == key)
                .is_some_and(|r| r.weight.trim().is_empty() && r.reps.trim().is_empty())
        });
        if empty {
            remove_row(key);
        } else {
            pending_remove.set(Some(key));
        }
    };

    let copy_last = move |_| {
        let Some((_, log)) = last.get_untracked() else {
            return;
        };
        // ★ 新しいキーを振る。既存キーを再利用すると <For> が DOM を作り直さないため
        //   入力欄の value が古いまま残る
        let base = next_key.get_untracked();
        let filled: Vec<Row> = log
            .sets
            .iter()
            .enumerate()
            .map(|(i, s)| Row {
                key: base + i as u32,
                weight: fmt_weight(s.weight),
                reps: s.reps.to_string(),
            })
            .collect();
        next_key.set(base + filled.len() as u32 + 1);
        rows.set(filled);
        commit();
    };

    let close_card = move |_| {
        let date = dates.selected.get_untracked();
        db.update(|d| write_log(d, date, ex, Vec::new(), false));
        cards.update(|cs| cs.retain(|c| c.ex != ex));
    };

    let today_metric = move || {
        let key = core::date_key(dates.selected.get());
        db.with(|d| {
            d.sessions
                .get(&key)
                .and_then(|s| s.log_of(ex))
                .map_or(0.0, |l| core::log_value(Metric::Volume, l))
        })
    };

    view! {
        <article class="card" id=card_dom_id(ex) data-testid="exercise-card">
            // ★ 見出しに削除ボタンを置かない。カードの一番上・右端は
            //   「種目を追加」を探して下スクロールする指が最初に触る位置で、
            //   追加しようとして種目ごと消す事故が起きていた。導線はカード末尾へ
            <header class="card-head">
                <h2 data-testid="card-name">{move || name.get()}</h2>
                <span class="group-name">{move || group_name.get()}</span>
            </header>

            <div class="last-row">
                {move || match last.get() {
                    None => view! { <span class="muted" data-testid="last-log">"前回 —"</span> }.into_any(),
                    Some((date, log)) => {
                        let days = (dates.selected.get() - date).num_days();
                        let when = core::humanize(core::Elapsed::Days(days));
                        let sets = log.sets.iter().map(fmt_set).collect::<Vec<_>>().join("  ");
                        let metric = fmt_metric(core::log_value(Metric::Volume, &log));
                        view! {
                            <span class="when" data-testid="last-log">{format!("前回 {when}")}</span>
                            <span class="sets">{sets}</span>
                            <span class="metric">{metric}</span>
                        }
                            .into_any()
                    }
                }}
            </div>

            {move || {
                show_copy
                    .get()
                    .then(|| {
                        view! {
                            <button
                                class="secondary copy"
                                data-testid="copy-last"
                                on:click=copy_last
                            >
                                "前回をコピー"
                            </button>
                        }
                    })
            }}

            <div class="sets-editor">
                <For
                    each=move || rows.get()
                    key=|r| r.key
                    children=move |row| {
                        let key = row.key;
                        let index = move || {
                            rows.with(|rs| rs.iter().position(|r| r.key == key).map_or(0, |i| i + 1))
                        };
                        // 重量を使う種目で reps だけ入っている行は「入力忘れ」の可能性が高い。
                        // 黙って指標を変えず、行にヒントを出して保持する
                        let weight_missing = move || {
                            uses_weight.get()
                                && rows.with(|rs| {
                                    rs.iter()
                                        .find(|r| r.key == key)
                                        .is_some_and(|r| {
                                            parse_reps(&r.reps).is_some()
                                                && parse_weight(&r.weight) <= 0.0
                                        })
                                })
                        };
                        // ★ 「+ セット」で足した行の回数欄へフォーカスを移す。
                        //   iOS はユーザー操作起点のタスク内でしか focus() でキーボードを
                        //   開かないので、set_timeout を挟まず Effect（マイクロタスク）で
                        //   完結させる。仮に開かなくても重量はプリフィル済みなので
                        //   タップ 1 回で回数を打てる
                        let reps_ref = NodeRef::<leptos::html::Input>::new();
                        Effect::new(move |_| {
                            if focus_key.get() == Some(key)
                                && let Some(el) = reps_ref.get()
                            {
                                let _ = el.focus();
                                focus_key.set(None);
                            }
                        });
                        view! {
                            <div class="set-row" data-testid="set-row">
                                <span class="set-no">{index}</span>
                                // 重量欄は常に出す。空のままなら重量 1 として数えられるので、
                                // 自重種目でも時間種目でも「入れなければよい」で成立する
                                <input
                                    class="num"
                                    type="text"
                                    inputmode="decimal"
                                    pattern="[0-9]*([.,][0-9]*)?"
                                    value=row.weight.clone()
                                    aria-label="重量"
                                    data-testid="set-weight"
                                    on:focusin=move |_| kb_focus(kb)
                                    on:focusout=move |_| kb_blur(kb)
                                    on:input=move |ev| {
                                        let v = event_target_value(&ev);
                                        rows.update(|rs| {
                                            if let Some(r) = rs.iter_mut().find(|r| r.key == key) {
                                                r.weight = v;
                                            }
                                        });
                                        commit();
                                    }
                                />
                                <span class="unit">"kg"</span>
                                <span class="times">"×"</span>
                                <input
                                    class="num"
                                    type="text"
                                    inputmode="numeric"
                                    value=row.reps.clone()
                                    aria-label="回数"
                                    data-testid="set-reps"
                                    node_ref=reps_ref
                                    on:focusin=move |_| kb_focus(kb)
                                    on:focusout=move |_| kb_blur(kb)
                                    on:input=move |ev| {
                                        let v = event_target_value(&ev);
                                        rows.update(|rs| {
                                            if let Some(r) = rs.iter_mut().find(|r| r.key == key) {
                                                r.reps = v;
                                            }
                                        });
                                        commit();
                                    }
                                />
                                // 回数欄に単位は添えない。プランクの 60 に「回」と付くほうが
                                // 嘘になるし、それが秒だと分かるのは種目名からで表記からではない
                                //
                                // ★ 削除は入力欄と地続きにしない。margin-left:auto で右端へ寄せた上に
                                //   区切り線と内側余白で離す（auto を外すと回数欄の直後に来て
                                //   今より押しやすくなる）。中身のある行は確認を挟む
                                <button
                                    class="icon-btn danger-btn"
                                    aria-label="このセットを削除"
                                    data-testid="remove-set"
                                    on:click=move |_| request_remove(key)
                                >
                                    "✕"
                                </button>
                                {move || {
                                    weight_missing()
                                        .then(|| {
                                            view! {
                                                <span class="warn" data-testid="weight-missing">
                                                    "重量未入力"
                                                </span>
                                            }
                                        })
                                }}
                                {move || {
                                    (pending_remove.get() == Some(key))
                                        .then(|| {
                                            view! {
                                                <span class="row-confirm" data-testid="remove-set-confirm">
                                                    <span>"このセットを削除しますか？"</span>
                                                    <button
                                                        class="link-btn danger"
                                                        data-testid="remove-set-yes"
                                                        on:click=move |_| remove_row(key)
                                                    >
                                                        "削除"
                                                    </button>
                                                    <button
                                                        class="link-btn"
                                                        data-testid="remove-set-no"
                                                        on:click=move |_| pending_remove.set(None)
                                                    >
                                                        "やめる"
                                                    </button>
                                                </span>
                                            }
                                        })
                                }}
                            </div>
                        }
                    }
                />
                <div class="add-set-wrap">
                    <button class="secondary" data-testid="add-set" on:click=add_row>
                        "+ セット"
                    </button>
                </div>
            </div>

            // 前回比はセッション中に出さない。途中の不完全な合計を前回の完了セッションと
            // 比べても意味がないため（比較は推移タブで行う）
            <footer class="card-foot">
                <span>"今日"</span>
                <strong data-testid="today-metric">{move || fmt_metric(today_metric())}</strong>
            </footer>

            <div class="card-remove">
                <button
                    class="link-btn danger"
                    data-testid="close-card"
                    on:click=move |_| {
                        confirm_close.set(true);
                        // ★ 確認はカード末尾に出るので、sticky の「種目を追加」の背後に
                        //   入って見えないことがある。開いたら必ず視界へ送る
                        scroll_to_id(confirm_dom_id(ex));
                    }
                >
                    "この種目を外す"
                </button>
            </div>

            {move || {
                confirm_close
                    .get()
                    .then(|| {
                        view! {
                            <div class="warn-box" id=confirm_dom_id(ex)>
                                <p data-testid="close-card-warning">
                                    "この日の記録が消えます"
                                </p>
                                <div class="sheet-actions">
                                    <button
                                        class="primary"
                                        data-testid="close-card-yes"
                                        on:click=close_card
                                    >
                                        "外す"
                                    </button>
                                    <button
                                        class="link-btn"
                                        data-testid="close-card-no"
                                        on:click=move |_| confirm_close.set(false)
                                    >
                                        "やめる"
                                    </button>
                                </div>
                            </div>
                        }
                    })
            }}
        </article>
    }
}
