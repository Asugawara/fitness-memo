//! 記録タブ。**月カレンダー + その下に選択日の入力欄**の 1 画面。
//!
//! 月グリッド（日〜土）+ 前月・翌月ナビ。実施日に部位カラーのドット（最大 3 色）。
//! 月フッタに「実施 N 日 / 合計 / セット」。その下に [`DayEditor`]。
//!
//! **日セルをタップすると、そのまま下の入力欄がその日のものになる。**
//! 以前は読み取り専用のサマリと「この日を編集」ボタンを挟んで別タブへ飛ばしていたが、
//! 「昨日記録し忘れた分を入れる」たびにタブを往復することになっていた。
//!
//! 選択日は `DateCtx::selected` **だけ**が持つ。ここにローカルの `picked` を併置すると
//! 「グリッドで選んだ日」と「入力欄が書き込む日」がずれる余地ができる。
//!
//! 実施日の判定は [`Session::is_trained`]（セット付きのログが 1 つでもあるか）で行う。
//! 過去日を閲覧しただけの空セッションを実施日にしないための境界。

use chrono::{Datelike, NaiveDate, TimeDelta};
use leptos::prelude::*;

use crate::core;
use crate::core::Metric;
use crate::model::{Db, Group, GroupId, Session};

use super::day::DayEditor;
use super::help::InstallBanner;
use super::icon::{self, icon};
use super::{fmt_metric, use_dates, use_db};

/// 日曜始まり。`Weekday::num_days_from_sunday()` の 0..=6 とインデックスが一致する。
const WEEKDAYS: [&str; 7] = ["日", "月", "火", "水", "木", "金", "土"];

/// ドットの最大色数。これ以上は部位の並び順で切り捨てる。
const MAX_DOTS: usize = 3;

// 装飾は `public/styles.css` の `.cal-*` に一本化してある。
// 唯一の例外がドットの色で、これは `Group.color` 由来のデータなので CSS では表現できず
// インラインの `style` で渡す（状態は `class:is-today` / `class:is-picked` で伝える）。

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
    /// 全種目の合計ボリューム。指標の式が 1 本になったので絞り込みは要らない。
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
                out.volume += core::log_value(Metric::Volume, log);
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

// ── 画面 ────────────────────────────────────────────────────────────────────

#[component]
pub fn Calendar() -> impl IntoView {
    let db = use_db();
    let dates = use_dates();

    // タブ切替のたびにこのコンポーネントは作り直されるので、開くたび「見ている日付」の月
    // から始まる（過去日を編集中なら、その月がそのまま出る）
    let month = RwSignal::new(first_of_month(dates.selected.get_untracked()));

    // 選択日が月をまたいだらグリッドを追従させる（「今日へ戻る」や日跨ぎの resync 経由）。
    // 逆向き（月ナビ）は選択日を動かさない — 隣の月を眺めるだけで編集対象が変わるのは困る
    Effect::new(move |_| {
        let first = first_of_month(dates.selected.get());
        if month.get_untracked() != first {
            month.set(first);
        }
    });

    let data = Memo::new(move |_| db.with(|d| month_data(d, month.get())));

    view! {
        <section class="calendar" data-testid="screen-record">
            <header class="cal-nav">
                <button
                    class="icon-btn"
                    aria-label="前の月"
                    data-testid="cal-prev"
                    on:click=move |_| month.update(|m| *m = shift_month(*m, -1))
                >
                    {icon(icon::CHEVRON_LEFT)}
                </button>
                <h1 class="screen-title" data-testid="cal-title">
                    {move || fmt_month(month.get())}
                </h1>
                <button
                    class="icon-btn"
                    aria-label="次の月"
                    data-testid="cal-next"
                    on:click=move |_| month.update(|m| *m = shift_month(*m, 1))
                >
                    {icon(icon::CHEVRON_RIGHT)}
                </button>
            </header>

            <div class="cal-week">
                {WEEKDAYS
                    .iter()
                    .map(|w| view! { <span class="muted cal-wd">{*w}</span> })
                    .collect::<Vec<_>>()}
            </div>

            <div class="cal-grid" data-testid="cal-grid">
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
                                            class:is-picked=move || date == dates.selected.get()
                                            data-testid="cal-day"
                                            data-date=core::date_key(date)
                                            data-trained=if c.trained { "true" } else { "false" }
                                            // ★ set ではなく open。RwSignal::set は同値でも通知するので、
                                            //   選択中の日をもう一度タップすると下の ConditionRow が
                                            //   作り直され、体重欄の「62.」が確定値へ巻き戻る
                                            on:click=move |_| dates.open(date)
                                        >
                                            <span class="cal-num">{c.day}</span>
                                            <span class="cal-dots">
                                                {colors
                                                    .into_iter()
                                                    .map(|color| {
                                                        view! {
                                                            <i
                                                                class="cal-dot"
                                                                data-testid="cal-dot"
                                                                style=format!("background:{color}")
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

            // 月フッタ。合計は全種目のボリューム（指標の式が 1 本なので絞り込み不要）。
            // 重量の大小に引きずられないセット数も並べる
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
                        {move || fmt_metric(data.with(|m| m.volume))}
                    </dd>
                </div>
                <div>
                    <dt>"セット"</dt>
                    <dd data-testid="cal-sets">{move || data.with(|m| m.sets).to_string()}</dd>
                </div>
            </dl>

            // ★ 選択日の入力欄。読み取り専用のサマリと「この日を編集」ボタンは持たない。
            //   日をタップした時点でここがその日のものになるので、記録がある日も無い日も
            //   同じ 1 手で書ける（要件「カレンダーに対して追加できる」の成立点）
            <DayEditor />

            // ★ iOS では Safari のタブと standalone PWA で localStorage が共有されない。
            //   先に Safari で記録すると、ホーム画面に追加した後で全部見えなくなる。
            //
            // ★ 置く場所は `DayEditor` の**外**の末尾。中に入れると `.add-wrap`
            //   （「種目を追加」）の sticky な帯に覆われて読めない。sticky は
            //   `<section class="day">` で完結するので、その後ろなら干渉しない。
            //   記録の導線（カレンダー → 入力欄 → 種目を追加）より上に置くと
            //   毎回目に入って邪魔なので、下に流す。出す条件と手順シートは help 側が持つ
            <InstallBanner />
        </section>
    }
}
