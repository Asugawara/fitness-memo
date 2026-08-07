//! カレンダータブ。
//!
//! 月グリッド（日〜土）+ 前月・翌月ナビ。実施日に部位カラーのドット（最大 3 色）。
//! 月フッタに「実施 N 日 / 合計 X kg·回」。
//!
//! **この画面の核心は「どの日付にどの筋トレ項目をしたかを追加できる」こと。**
//! 記録がある日もない日も、タップすればその日付で今日タブを開ける導線を必ず出す。
//! 最も典型的な「昨日記録し忘れた分をカレンダーから入れる」が成立しなければ要件を
//! 満たさないので、記録が無い日は「記録なし」＋「この日に記録する」を必ず表示する。
//!
//! 実施日の判定は [`Session::is_trained`]（セット付きのログが 1 つでもあるか）で行う。
//! 過去日を閲覧しただけの空セッションを実施日にしないための境界。

use chrono::{Datelike, NaiveDate, TimeDelta};
use leptos::prelude::*;

use crate::core;
use crate::model::{Db, Group, GroupId, Kind, Session};

use super::{fmt_date, fmt_metric, fmt_set, fmt_weight, use_dates, use_db};

/// 日曜始まり。`Weekday::num_days_from_sunday()` の 0..=6 とインデックスが一致する。
const WEEKDAYS: [&str; 7] = ["日", "月", "火", "水", "木", "金", "土"];

/// ドットの最大色数。これ以上は部位の並び順で切り捨てる。
const MAX_DOTS: usize = 3;

// ── 暫定インラインスタイル ──────────────────────────────────────────────────
//
// ★ `public/styles.css` は 3 ワーカーが同時に追記すると壊れるので Wave 3a では書かない。
//   ただしカレンダーは 7 列グリッドとドットが CSS 無しでは画面として成立しないため、
//   **完了報告で要求する CSS と同じ宣言**をインラインで暫定的に置いている。
//   `styles.css` に `.cal-*` が入ったら、この節の定数・関数と対応する `style=` 属性を
//   消すだけで移行できる（クラス名は既に最終形を付けてある）。

const S_NAV: &str = "display:flex;align-items:center;justify-content:space-between;gap:8px";
const S_WEEK: &str = "display:grid;grid-template-columns:repeat(7,1fr);text-align:center";
const S_GRID: &str = "display:grid;grid-template-columns:repeat(7,1fr);gap:2px";
const S_DOTS: &str = "display:flex;align-items:center;justify-content:center;gap:2px;height:6px";
const S_LOGS: &str = "list-style:none;margin:8px 0 0;padding:0";
const S_ACTIONS: &str =
    "display:flex;align-items:center;justify-content:space-between;gap:8px;margin-top:10px";
const S_LOG_NAME: &str = "color:var(--text);font-weight:600";

/// 日セル。`min-height:44px` はタップターゲット規約（計画のデザイン節）。
fn s_day(is_picked: bool, is_today: bool) -> String {
    let mut s = String::from(
        "min-height:44px;display:flex;flex-direction:column;align-items:center;\
         justify-content:center;gap:3px;padding:2px 0;border:1px solid transparent;\
         border-radius:8px;background:none;font-variant-numeric:tabular-nums",
    );
    if is_today {
        s.push_str(";font-weight:600;color:var(--accent)");
    }
    if is_picked {
        s.push_str(";border-color:var(--accent);background:var(--chip-0)");
    }
    s
}

fn s_dot(color: &str) -> String {
    format!("width:6px;height:6px;border-radius:50%;background:{color}")
}

// ── 月の算術 ────────────────────────────────────────────────────────────────

fn first_of_month(d: NaiveDate) -> NaiveDate {
    NaiveDate::from_ymd_opt(d.year(), d.month(), 1).unwrap_or(d)
}

/// 月初日を `delta` か月ずらす。
///
/// 月番号の算術で閉じているので `Months` の「1/31 → 2/28 に丸める」挙動を考えなくてよい
/// （入力が常に 1 日だから成り立つ）。
fn shift_month(first: NaiveDate, delta: i32) -> NaiveDate {
    let total = first.year() * 12 + first.month0() as i32 + delta;
    NaiveDate::from_ymd_opt(total.div_euclid(12), total.rem_euclid(12) as u32 + 1, 1)
        .unwrap_or(first)
}

fn fmt_month(first: NaiveDate) -> String {
    format!("{}年{}月", first.year(), first.month())
}

// ── 月データ ────────────────────────────────────────────────────────────────

#[derive(Clone, PartialEq)]
struct DayCell {
    date: NaiveDate,
    day: u32,
    trained: bool,
    /// 実施した部位の色。部位の並び順で最大 [`MAX_DOTS`] 色。
    colors: Vec<String>,
}

#[derive(Clone, PartialEq, Default)]
struct MonthData {
    /// 日曜始まりのグリッド。`None` は前月・翌月にはみ出す空きマス。
    cells: Vec<Option<DayCell>>,
    trained_days: usize,
    sets: usize,
    /// `Kind::Weighted` の種目だけの合計。単位の違う指標を足しても意味を持たないため。
    volume: f64,
}

/// 実施した部位の色を部位の並び順で最大 [`MAX_DOTS`] 色。
fn dot_colors(db: &Db, session: &Session) -> Vec<String> {
    let mut ids: Vec<GroupId> = Vec::new();
    for log in &session.logs {
        if log.sets.is_empty() {
            continue;
        }
        if let Some(e) = db.exercise(log.exercise_id)
            && !ids.contains(&e.group_id)
        {
            ids.push(e.group_id);
        }
    }
    let mut groups: Vec<&Group> = ids.iter().filter_map(|id| db.group(*id)).collect();
    groups.sort_by_key(|g| g.order);
    groups
        .into_iter()
        .take(MAX_DOTS)
        .map(|g| g.color.clone())
        .collect()
}

fn month_data(db: &Db, first: NaiveDate) -> MonthData {
    let next = shift_month(first, 1);
    let days = (next - first).num_days();
    let lead = i64::from(first.weekday().num_days_from_sunday());

    let mut out = MonthData {
        cells: vec![None; lead as usize],
        ..MonthData::default()
    };

    for i in 0..days {
        let date = first + TimeDelta::days(i);
        let session = db.sessions.get(&core::date_key(date));
        let trained = session.is_some_and(Session::is_trained);
        if trained {
            out.trained_days += 1;
        }
        if let Some(s) = session {
            for log in &s.logs {
                out.sets += log.sets.len();
                // 単位が違う指標を足すと意味を失うので、合計は Weighted のみ
                if db
                    .exercise(log.exercise_id)
                    .is_some_and(|e| e.kind == Kind::Weighted)
                {
                    out.volume += core::log_metric(Kind::Weighted, log);
                }
            }
        }
        out.cells.push(Some(DayCell {
            date,
            day: date.day(),
            trained,
            colors: session.map(|s| dot_colors(db, s)).unwrap_or_default(),
        }));
    }

    // 最終週を 7 マスに揃える（グリッドの列がずれないように）
    while !out.cells.len().is_multiple_of(7) {
        out.cells.push(None);
    }
    out
}

// ── 選択日のサマリ ──────────────────────────────────────────────────────────

#[derive(Clone, PartialEq)]
struct LogLine {
    name: String,
    group: String,
    sets: String,
    metric: String,
}

#[derive(Clone, PartialEq, Default)]
struct DayDetail {
    logs: Vec<LogLine>,
    body_weight: Option<String>,
    note: String,
}

fn day_detail(db: &Db, date: NaiveDate) -> DayDetail {
    let Some(session) = db.sessions.get(&core::date_key(date)) else {
        return DayDetail::default();
    };
    let logs = session
        .logs
        .iter()
        .filter(|l| !l.sets.is_empty())
        .map(|l| {
            let exercise = db.exercise(l.exercise_id);
            let kind = exercise.map_or(Kind::Weighted, |e| e.kind);
            LogLine {
                name: exercise.map_or_else(|| "(削除された種目)".to_string(), |e| e.name.clone()),
                group: exercise
                    .and_then(|e| db.group(e.group_id))
                    .map(|g| g.name.clone())
                    .unwrap_or_default(),
                sets: l
                    .sets
                    .iter()
                    .map(|s| fmt_set(kind, s))
                    .collect::<Vec<_>>()
                    .join("  "),
                metric: format!(
                    "{} {}",
                    fmt_metric(core::log_metric(kind, l)),
                    core::unit_of(kind)
                ),
            }
        })
        .collect();
    DayDetail {
        logs,
        body_weight: session.body_weight.map(fmt_weight),
        note: session.note.clone(),
    }
}

// ── タブ切替 ────────────────────────────────────────────────────────────────

/// ボトムタブの「今日」へ合成クリックを投げてタブを切り替える。
///
/// ★ `views/mod.rs` の `tab` は `App` のローカル signal で `provide_context` されて
///   いないため、カレンダーからは直接触れない。**本筋は `mod.rs` が
///   `TabCtx(pub RwSignal<Tab>)` を provide すること**で、そうなればこの関数は消して
///   `use_context::<TabCtx>().0.set(Tab::Today)` に置き換えられる（完了報告に記載）。
///   ここを諦めると「カレンダーから記録を追加する」という要件そのものが成立しないので、
///   `mod.rs` を触れない Wave 3a では DOM 経由で同じ導線（`switch()`）を通す。
///
/// 実装上の注意 2 点:
/// - `createEvent("Event")` ではなく **`"MouseEvent"`**。leptos の `on:click` は
///   `MouseEvent` を受け取る型付きハンドラなので、素の `Event` を投げると境界で型が
///   食い違う。`initEvent` は `Event.prototype` のメソッドなので MouseEvent 実体にも効く
/// - `bubbles = true` にするのは leptos がイベント委譲でハンドラを document 側に張るため。
///   既定の `createEvent` は bubbles が false で、委譲ハンドラに届かない
///
/// 使う web-sys API は `Document` / `Element` / `Event` / `EventTarget` のみで、いずれも
/// `Cargo.toml` が明示宣言している（tachys 経由で偶然有効な feature には寄りかからない）。
fn focus_today_tab() {
    let doc = document();
    let Some(button) = doc
        .query_selector("[data-testid='tab-today']")
        .ok()
        .flatten()
    else {
        return;
    };
    let Ok(event) = doc.create_event("MouseEvent") else {
        return;
    };
    event.init_event_with_bubbles_and_cancelable("click", true, true);
    let _ = button.dispatch_event(&event);
}

// ── 画面 ────────────────────────────────────────────────────────────────────

#[component]
pub fn Calendar() -> impl IntoView {
    let db = use_db();
    let dates = use_dates();

    // タブ切替のたびにこのコンポーネントは作り直されるので、開くたび「見ている日付」の月
    // から始まる（過去日を編集中なら、その月がそのまま出る）
    let month = RwSignal::new(first_of_month(dates.selected.get_untracked()));
    let picked = RwSignal::new(dates.selected.get_untracked());

    let data = Memo::new(move |_| db.with(|d| month_data(d, month.get())));
    let detail = Memo::new(move |_| db.with(|d| day_detail(d, picked.get())));
    let trained = Memo::new(move |_| detail.with(|d| !d.logs.is_empty()));

    // ★ 日付を先に確定させてからタブを切り替える。
    //   `mod.rs` の `switch()` は `dates.resync(false)` を通るが、`selected` が `today` と
    //   食い違っていれば当日へ戻さない実装なので、この順序なら選んだ過去日が残り、
    //   今日タブ側が「編集中」バナーを出す。
    let open_day = move |date: NaiveDate| {
        dates.open(date);
        focus_today_tab();
    };

    view! {
        <section class="calendar" data-testid="screen-calendar">
            <header class="cal-nav" style=S_NAV>
                <button
                    class="icon-btn"
                    aria-label="前の月"
                    data-testid="cal-prev"
                    on:click=move |_| month.update(|m| *m = shift_month(*m, -1))
                >
                    "‹"
                </button>
                <h1 class="screen-title" style="margin:0" data-testid="cal-title">
                    {move || fmt_month(month.get())}
                </h1>
                <button
                    class="icon-btn"
                    aria-label="次の月"
                    data-testid="cal-next"
                    on:click=move |_| month.update(|m| *m = shift_month(*m, 1))
                >
                    "›"
                </button>
            </header>

            <div class="cal-week" style=S_WEEK>
                {WEEKDAYS
                    .iter()
                    .map(|w| view! { <span class="muted cal-wd">{*w}</span> })
                    .collect::<Vec<_>>()}
            </div>

            <div class="cal-grid" style=S_GRID data-testid="cal-grid">
                {move || {
                    data.with(|m| {
                        m.cells
                            .iter()
                            .map(|cell| match cell {
                                // 前月・翌月にはみ出すマス。タップ対象を当月だけに限ることで
                                // 「どの月の何日を選んだか」が曖昧にならない
                                None => view! { <span class="cal-blank"></span> }.into_any(),
                                Some(c) => {
                                    let date = c.date;
                                    let colors = c.colors.clone();
                                    view! {
                                        <button
                                            class="cal-day"
                                            class:is-trained=c.trained
                                            class:is-today=move || date == dates.today.get()
                                            class:is-picked=move || date == picked.get()
                                            style=move || {
                                                s_day(date == picked.get(), date == dates.today.get())
                                            }
                                            data-testid="cal-day"
                                            data-date=core::date_key(date)
                                            data-trained=if c.trained { "true" } else { "false" }
                                            on:click=move |_| picked.set(date)
                                        >
                                            <span class="cal-num">{c.day}</span>
                                            <span class="cal-dots" style=S_DOTS>
                                                {colors
                                                    .into_iter()
                                                    .map(|color| {
                                                        view! {
                                                            <i
                                                                class="cal-dot"
                                                                data-testid="cal-dot"
                                                                style=s_dot(&color)
                                                            ></i>
                                                        }
                                                    })
                                                    .collect::<Vec<_>>()}
                                            </span>
                                        </button>
                                    }
                                        .into_any()
                                }
                            })
                            .collect::<Vec<_>>()
                    })
                }}
            </div>

            // 月フッタ。合計は Kind::Weighted のみ（単位の違う指標を足さない）。
            // 部位が混ざっても数えられるセット数を並べて、落ちた分を見えなくしない
            <dl class="stats" data-testid="cal-stats">
                <div>
                    <dt>"実施"</dt>
                    <dd data-testid="cal-trained-days">
                        {move || format!("{} 日", data.with(|m| m.trained_days))}
                    </dd>
                </div>
                <div>
                    <dt>"合計"</dt>
                    <dd data-testid="cal-volume">
                        {move || {
                            format!(
                                "{} {}",
                                fmt_metric(data.with(|m| m.volume)),
                                core::unit_of(Kind::Weighted),
                            )
                        }}
                    </dd>
                </div>
                <div>
                    <dt>"セット"</dt>
                    <dd data-testid="cal-sets">{move || data.with(|m| m.sets).to_string()}</dd>
                </div>
            </dl>

            <article class="card cal-detail" data-testid="cal-detail">
                <header class="card-head">
                    <h2 data-testid="cal-detail-date">{move || fmt_date(picked.get())}</h2>
                </header>

                // 体重・メモは実施の有無に関わらず、入っていれば出す
                {move || {
                    let (weight, note) = detail
                        .with(|d| (d.body_weight.clone(), d.note.trim().to_string()));
                    let has_note = !note.is_empty();
                    (weight.is_some() || has_note)
                        .then(move || {
                            view! {
                                <p class="last-row" data-testid="cal-condition">
                                    {weight
                                        .map(|w| {
                                            view! {
                                                <span data-testid="cal-body-weight">
                                                    {format!("体重 {w} kg")}
                                                </span>
                                            }
                                        })}
                                    {has_note
                                        .then(move || {
                                            view! { <span data-testid="cal-note">{note}</span> }
                                        })}
                                </p>
                            }
                        })
                }}

                {move || {
                    let logs = detail.with(|d| d.logs.clone());
                    (!logs.is_empty())
                        .then(|| {
                            view! {
                                <ul class="cal-logs" style=S_LOGS data-testid="cal-logs">
                                    {logs
                                        .into_iter()
                                        .map(|l| {
                                            view! {
                                                <li class="last-row cal-log" data-testid="cal-log">
                                                    <span class="cal-log-name" style=S_LOG_NAME>
                                                        {l.name}
                                                    </span>
                                                    <span class="group-name">{l.group}</span>
                                                    <span class="sets">{l.sets}</span>
                                                    <span class="metric">{l.metric}</span>
                                                </li>
                                            }
                                        })
                                        .collect::<Vec<_>>()}
                                </ul>
                            }
                        })
                }}

                // ★ 記録が無い日でも必ず「記録なし」と「この日に記録する」を出す。
                //   要件の動詞は「カレンダーに対して追加できる」で、最も典型的な
                //   「昨日記録し忘れた分をカレンダーから入れる」がここで成立する
                <div class="cal-actions" style=S_ACTIONS>
                    {move || {
                        (!trained.get())
                            .then(|| {
                                view! {
                                    <span class="muted" data-testid="cal-empty">
                                        "記録なし"
                                    </span>
                                }
                            })
                    }}
                    <button
                        class="primary cal-open"
                        style="margin-left:auto"
                        data-testid="cal-open-day"
                        data-mode=move || if trained.get() { "edit" } else { "new" }
                        on:click=move |_| open_day(picked.get_untracked())
                    >
                        {move || if trained.get() { "この日を編集" } else { "この日に記録する" }}
                    </button>
                </div>
            </article>
        </section>
    }
}
