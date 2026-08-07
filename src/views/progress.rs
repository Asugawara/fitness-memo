//! 推移タブ。対象セレクタ（部位 or 種目）+ 期間 + グラフ + 統計 + 記録テーブル。

use chrono::{Months, NaiveDate};
use leptos::prelude::*;

use crate::core;
use crate::model::{Db, ExerciseId, GroupId, Kind};

use super::chart::Chart;
use super::{fmt_date, fmt_metric, fmt_set, use_dates, use_db};

/// 記録テーブルの表示上限。超えた分は件数を明示して省く（黙って切らない）。
const MAX_ROWS: usize = 100;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Target {
    Group(GroupId),
    Exercise(ExerciseId),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Period {
    M1,
    M3,
    M6,
    Y1,
    All,
}

impl Period {
    const CHOICES: [(Period, &'static str); 5] = [
        (Period::M1, "1M"),
        (Period::M3, "3M"),
        (Period::M6, "6M"),
        (Period::Y1, "1Y"),
        (Period::All, "全期間"),
    ];

    fn months(self) -> Option<u32> {
        match self {
            Period::M1 => Some(1),
            Period::M3 => Some(3),
            Period::M6 => Some(6),
            Period::Y1 => Some(12),
            Period::All => None,
        }
    }
}

fn target_value(t: Target) -> String {
    match t {
        Target::Group(id) => format!("g:{id}"),
        Target::Exercise(id) => format!("e:{id}"),
    }
}

fn parse_target(raw: &str) -> Option<Target> {
    let (kind, id) = raw.split_once(':')?;
    let id = id.parse().ok()?;
    match kind {
        "g" => Some(Target::Group(id)),
        "e" => Some(Target::Exercise(id)),
        _ => None,
    }
}

/// セレクタに出す選択肢。**アーカイブ済み種目も末尾セクションに出す**
/// （過去データが参照不能になるのを防ぐ）。
#[derive(Clone, PartialEq, Default)]
struct Options {
    groups: Vec<(GroupId, String)>,
    active: Vec<(ExerciseId, String)>,
    archived: Vec<(ExerciseId, String)>,
}

fn options(d: &Db) -> Options {
    let mut groups = d.groups.clone();
    groups.sort_by_key(|g| g.order);
    let group_rank = |gid: GroupId| {
        groups
            .iter()
            .position(|g| g.id == gid)
            .unwrap_or(usize::MAX)
    };

    let mut sorted = d.exercises.clone();
    sorted.sort_by_key(|e| (group_rank(e.group_id), e.order, e.id));

    Options {
        groups: groups.iter().map(|g| (g.id, g.name.clone())).collect(),
        active: sorted
            .iter()
            .filter(|e| !e.archived)
            .map(|e| (e.id, e.name.clone()))
            .collect(),
        archived: sorted
            .iter()
            .filter(|e| e.archived)
            .map(|e| (e.id, e.name.clone()))
            .collect(),
    }
}

fn bounds(period: Period, today: NaiveDate, earliest: Option<NaiveDate>) -> (NaiveDate, NaiveDate) {
    let from = match period.months() {
        Some(m) => today.checked_sub_months(Months::new(m)).unwrap_or(today),
        None => earliest.unwrap_or(today),
    };
    (from.min(today), today)
}

fn earliest_session(d: &Db) -> Option<NaiveDate> {
    d.sessions.keys().find_map(|k| core::parse_date_key(k))
}

/// "+120 (+12%)" / "-80 (-8%)" / "±0" / "—"
fn fmt_delta(last: Option<f64>, prev: Option<f64>) -> String {
    let (Some(last), Some(prev)) = (last, prev) else {
        return "—".to_string();
    };
    let diff = last - prev;
    if diff.abs() < 0.5 {
        return "±0".to_string();
    }
    let sign = if diff > 0.0 { "+" } else { "-" };
    let pct = if prev > 0.0 {
        format!(" ({sign}{:.0}%)", diff.abs() / prev * 100.0)
    } else {
        String::new()
    };
    format!("{sign}{}{pct}", fmt_metric(diff.abs()))
}

#[component]
pub fn Progress() -> impl IntoView {
    let db = use_db();
    let dates = use_dates();

    // 既定は最初の種目（無ければ最初の部位）。要件の主眼が種目別の仕事量なので種目を優先する
    let initial = db.with_untracked(|d| {
        let opts = options(d);
        opts.active
            .first()
            .map(|(id, _)| Target::Exercise(*id))
            .or_else(|| opts.groups.first().map(|(id, _)| Target::Group(*id)))
    });
    let target = RwSignal::new(initial);
    let period = RwSignal::new(Period::M3);

    let unit = Memo::new(move |_| match target.get() {
        // 部位別は Kind 非依存の「セット数」（混在部位で volume を合算すると意味を失う）
        Some(Target::Group(_)) => "セット".to_string(),
        Some(Target::Exercise(ex)) => db
            .with(|d| d.exercise(ex).map(|e| core::unit_of(e.kind).to_string()))
            .unwrap_or_default(),
        None => String::new(),
    });

    let series = Memo::new(move |_| {
        let Some(t) = target.get() else {
            return Vec::new();
        };
        let today = dates.today.get();
        let period = period.get();
        db.with(|d| {
            let (from, to) = bounds(period, today, earliest_session(d));
            let raw = match t {
                Target::Group(g) => core::group_set_series(d, g, from, to),
                Target::Exercise(ex) => core::exercise_series(d, ex, from, to),
            };
            // ★「全期間」は週単位集約（1 年分 100 点超をそのまま描くと潰れる）
            if period == Period::All {
                core::aggregate_weekly(&raw)
            } else {
                raw
            }
        })
    });

    let stats = Memo::new(move |_| {
        let s = series.get();
        let last = s.last().map(|(_, v)| *v);
        let prev = s
            .len()
            .checked_sub(2)
            .and_then(|i| s.get(i))
            .map(|(_, v)| *v);
        let best = s.iter().map(|(_, v)| *v).reduce(f64::max);
        let avg = if s.is_empty() {
            None
        } else {
            Some(s.iter().map(|(_, v)| *v).sum::<f64>() / s.len() as f64)
        };
        (last, prev, best, avg)
    });

    // 記録テーブルは集約せず生の記録を新しい順に出す
    let records = Memo::new(move |_| {
        let Some(t) = target.get() else {
            return Vec::new();
        };
        let today = dates.today.get();
        let period = period.get();
        db.with(|d| {
            let (from, to) = bounds(period, today, earliest_session(d));
            let mut rows: Vec<(NaiveDate, String, String)> = Vec::new();
            for (key, session) in &d.sessions {
                let Some(date) = core::parse_date_key(key) else {
                    continue;
                };
                if date < from || date > to {
                    continue;
                }
                match t {
                    Target::Exercise(ex) => {
                        let Some(log) = session.log_of(ex).filter(|l| !l.sets.is_empty()) else {
                            continue;
                        };
                        let kind = d.exercise(ex).map_or(Kind::Weighted, |e| e.kind);
                        let detail = log
                            .sets
                            .iter()
                            .map(|s| fmt_set(kind, s))
                            .collect::<Vec<_>>()
                            .join("  ");
                        let metric = format!(
                            "{} {}",
                            fmt_metric(core::log_metric(kind, log)),
                            core::unit_of(kind)
                        );
                        rows.push((date, detail, metric));
                    }
                    Target::Group(g) => {
                        let ids = d.exercise_ids_of_group(g);
                        let mut names = Vec::new();
                        let mut sets = 0usize;
                        for log in &session.logs {
                            if ids.contains(&log.exercise_id) && !log.sets.is_empty() {
                                sets += log.sets.len();
                                if let Some(e) = d.exercise(log.exercise_id) {
                                    names.push(e.name.clone());
                                }
                            }
                        }
                        if sets == 0 {
                            continue;
                        }
                        rows.push((date, names.join("・"), format!("{sets} セット")));
                    }
                }
            }
            rows.reverse(); // sessions は日付昇順なので反転して新しい順にする
            rows
        })
    });

    let period_button = move |p: Period, label: &'static str| {
        view! {
            <button
                class="seg-btn"
                class:active=move || period.get() == p
                data-testid="period-btn"
                on:click=move |_| period.set(p)
            >
                {label}
            </button>
        }
    };

    view! {
        <section class="progress" data-testid="screen-progress">
            <h1 class="screen-title">"推移"</h1>

            <div class="selectors">
                <select
                    class="target-select"
                    data-testid="target-select"
                    aria-label="対象"
                    on:change=move |ev| target.set(parse_target(&event_target_value(&ev)))
                >
                    {move || {
                        let opts = db.with(options);
                        let current = target.get();
                        let option = move |value: String, name: String, t: Target| {
                            let is_current = Some(t) == current;
                            view! {
                                <option value=value selected=is_current>
                                    {name}
                                </option>
                            }
                        };
                        view! {
                            <optgroup label="部位">
                                {opts
                                    .groups
                                    .iter()
                                    .map(|(id, name)| {
                                        let t = Target::Group(*id);
                                        option(target_value(t), name.clone(), t)
                                    })
                                    .collect::<Vec<_>>()}
                            </optgroup>
                            <optgroup label="種目">
                                {opts
                                    .active
                                    .iter()
                                    .map(|(id, name)| {
                                        let t = Target::Exercise(*id);
                                        option(target_value(t), name.clone(), t)
                                    })
                                    .collect::<Vec<_>>()}
                            </optgroup>
                            // アーカイブ済みも出さないと過去データが参照不能になる
                            {(!opts.archived.is_empty())
                                .then(|| {
                                    view! {
                                        <optgroup label="アーカイブ済み">
                                            {opts
                                                .archived
                                                .iter()
                                                .map(|(id, name)| {
                                                    let t = Target::Exercise(*id);
                                                    option(target_value(t), name.clone(), t)
                                                })
                                                .collect::<Vec<_>>()}
                                        </optgroup>
                                    }
                                })}
                        }
                    }}
                </select>

                <div class="segmented" role="group" aria-label="期間" data-testid="period-select">
                    {Period::CHOICES
                        .into_iter()
                        .map(|(p, label)| period_button(p, label))
                        .collect::<Vec<_>>()}
                </div>
            </div>

            <Chart series=series unit=unit />

            {move || {
                (period.get() == Period::All)
                    .then(|| {
                        view! {
                            <p class="muted note" data-testid="weekly-note">
                                "全期間は週単位で集計しています"
                            </p>
                        }
                    })
            }}

            <dl class="stats" data-testid="stats">
                <div>
                    <dt>"前回比"</dt>
                    <dd data-testid="stat-delta">
                        {move || {
                            let (last, prev, _, _) = stats.get();
                            fmt_delta(last, prev)
                        }}
                    </dd>
                </div>
                <div>
                    <dt>"期間内ベスト"</dt>
                    <dd data-testid="stat-best">
                        {move || {
                            let (_, _, best, _) = stats.get();
                            best.map_or_else(
                                || "—".to_string(),
                                |v| format!("{} {}", fmt_metric(v), unit.get()),
                            )
                        }}
                    </dd>
                </div>
                <div>
                    <dt>"期間内平均"</dt>
                    <dd data-testid="stat-avg">
                        {move || {
                            let (_, _, _, avg) = stats.get();
                            avg.map_or_else(
                                || "—".to_string(),
                                |v| format!("{} {}", fmt_metric(v), unit.get()),
                            )
                        }}
                    </dd>
                </div>
            </dl>

            {move || {
                let rows = records.get();
                if rows.is_empty() {
                    return view! {
                        <p class="muted" data-testid="records-empty">"この期間の記録はありません"</p>
                    }
                        .into_any();
                }
                let total = rows.len();
                let shown: Vec<_> = rows.into_iter().take(MAX_ROWS).collect();
                let hidden = total.saturating_sub(shown.len());
                view! {
                    <table class="records" data-testid="records">
                        <thead>
                            <tr>
                                <th>"日付"</th>
                                <th>"内容"</th>
                                <th>"指標"</th>
                            </tr>
                        </thead>
                        <tbody>
                            {shown
                                .into_iter()
                                .map(|(date, detail, metric)| {
                                    view! {
                                        <tr data-testid="record-row">
                                            <td class="rec-date">{fmt_date(date)}</td>
                                            <td class="rec-detail">{detail}</td>
                                            <td class="rec-metric">{metric}</td>
                                        </tr>
                                    }
                                })
                                .collect::<Vec<_>>()}
                        </tbody>
                        {(hidden > 0)
                            .then(|| {
                                view! {
                                    <tfoot>
                                        <tr>
                                            <td colspan="3" class="muted">
                                                {format!("他 {hidden} 件は表示していません")}
                                            </td>
                                        </tr>
                                    </tfoot>
                                }
                            })}
                    </table>
                }
                    .into_any()
            }}
        </section>
    }
}
