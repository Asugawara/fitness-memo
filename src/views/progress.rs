//! 推移タブ。対象セレクタ（部位 + 種目）+ 期間 + グラフ + 統計 + 記録テーブル。

use chrono::{Months, NaiveDate};
use leptos::prelude::*;

use crate::core;
use crate::core::{Drops, Metric, Pick};
use crate::model::{Db, ExerciseId, GroupId};
use crate::storage;

use super::chart::Chart;
use super::{
    cur_lang, ex_name, fmt_date, fmt_metric, fmt_set, grp_name, t, use_dates, use_db, use_drops,
};
use crate::i18n::Lang;

/// 記録テーブルの表示上限。超えた分は件数を明示して省く（黙って切らない）。
const MAX_ROWS: usize = 100;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Period {
    M1,
    M3,
    M6,
    Y1,
    All,
}

impl Period {
    /// 期間セレクタの 5 択。
    ///
    /// ★ 1M / 3M / 6M / 1Y は**両言語で同じ**。記号に近い短縮なので、
    ///   日本語だけ「1ヶ月」に広げるとセグメントの幅が揃わなくなる。
    ///   最後の 1 つだけが語なので表から引く
    fn choices(lang: Lang) -> [(Period, &'static str); 5] {
        [
            (Period::M1, "1M"),
            (Period::M3, "3M"),
            (Period::M6, "6M"),
            (Period::Y1, "1Y"),
            (Period::All, lang.strings().progress.period_all),
        ]
    }

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

/// 画面の下半分に出すもの。
///
/// ★ `Memo` にして**この 3 値が変わったときだけ**下を作り直す。`pick` を直に
///   購読すると種目を切り替えるたびに [`Chart`] が破棄・再生成され、読み取り点
///   （`chart-readout`）の選択が毎回リセットされる
#[derive(Clone, Copy, PartialEq, Eq)]
enum Body {
    /// 記録が 1 件も無い（セレクタの上の空状態が既に説明している）
    NoRecords,
    /// 部位も種目も「すべて」
    NoPick,
    Chart,
}

/// 種目セレクタの 1 項目。
///
/// ★ 部位ごとの入れ子（`Vec<(GroupId, Vec<..>)>`）にしない。部位で絞る操作は
///   フラットな並びを `filter` するだけで済み、「すべての部位」のときは今までどおり
///   種目 / アーカイブ済み の 2 セクションがそのまま使える。入れ子にすると
///   アーカイブ済み種目を「所属部位の節」と「アーカイブ済みの節」のどちらに置くかで
///   表が二重化する。
#[derive(Clone, PartialEq)]
struct ExOption {
    id: ExerciseId,
    /// 所属部位。**`Db` から消えていれば `None`**（`core::Pick::with_exercise` と同じ規則）。
    /// `None` の種目は「すべての部位」のときだけ並ぶ — 絞る部位が無いので当然。
    group: Option<GroupId>,
    name: String,
    archived: bool,
}

/// セレクタに出す選択肢。
///
/// **記録がある種目だけ**を出す。プリセットは 28 種目あるので、全部並べると
/// 一度も使っていない種目を選んで空グラフを眺める操作が普通に起きる。
///
/// **アーカイブ済みでも記録があれば末尾セクションに出す**（過去データが
/// 参照不能になるのを防ぐ）。部位は「記録のある種目を持つ部位」だけ。
///
/// 絞り込みと並べ替えの規則は [`core::progress_candidates`] にある。ここは
/// ID を表示名に写すだけ。
#[derive(Clone, PartialEq, Default)]
struct Options {
    groups: Vec<(GroupId, String)>,
    exercises: Vec<ExOption>,
}

impl Options {
    fn is_empty(&self) -> bool {
        self.groups.is_empty() && self.exercises.is_empty()
    }

    /// 種目セレクタに出す並び。`group` が `Some` ならその部位だけに絞る。
    fn exercises_of(
        &self,
        group: Option<GroupId>,
        archived: bool,
    ) -> impl Iterator<Item = &ExOption> {
        self.exercises
            .iter()
            .filter(move |e| e.archived == archived)
            .filter(move |e| group.is_none_or(|g| e.group == Some(g)))
    }

    /// その組が今も候補として成立するか。**部位の一致まで見る**
    /// （種目だけ生きていて部位が食い違う組を通すと、2 つの select が食い違う）。
    fn is_live(&self, p: Pick) -> bool {
        match (p.group, p.exercise) {
            (None, None) => false, // 既定へ寄せ直す（案内文だけの画面に留めない）
            (Some(g), None) => self.groups.iter().any(|(id, _)| *id == g),
            (_, Some(ex)) => self
                .exercises
                .iter()
                .any(|e| e.id == ex && e.group == p.group),
        }
    }

    /// 既定の組。[`core::default_pick`] と同じ規則（`exercises` が
    /// `core::progress_candidates` をそのまま写しているので必ず一致する）。
    fn default_pick(&self) -> Pick {
        match self.exercises.first() {
            Some(e) => Pick {
                group: e.group,
                exercise: Some(e.id),
            },
            None => Pick::default(),
        }
    }
}

fn options(d: &Db) -> Options {
    Options {
        groups: core::progress_candidate_groups(d)
            .into_iter()
            .filter_map(|id| d.group(id).map(|g| (id, grp_name(g).to_string())))
            .collect(),
        exercises: core::progress_candidates(d)
            .into_iter()
            .filter_map(|id| d.exercise(id))
            .map(|e| ExOption {
                id: e.id,
                // 宙に浮いた `group_id` は `None`（`Pick::with_exercise` と同じ規則）
                group: d.group(e.group_id).map(|g| g.id),
                name: ex_name(e).to_string(),
                archived: e.archived,
            })
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
    // 文言は本体の先頭で 1 回だけ引く（`views::t` の doc を参照）
    let t = t();
    let db = use_db();
    let dates = use_dates();
    let drops = use_drops();

    let opts = Memo::new(move |_| db.with(options));

    // ★ **タブ切替のたびに `Progress` は作り直される**（`App` の `match tab.get()`）。
    //   `OpenGroupCtx` のようにコンテキストへ載せるのではなく、保存値から読み直すことで
    //   記録⇄推移の往復に耐える。真実源が `localStorage` 1 箇所で済み、`App` に
    //   「`storage::load` の後で `Db` 依存の検証をやる」順序制約も作らない。
    //   読むのはマウント 1 回きりなので、打鍵ごとに走る `HistoryCtx` とは事情が違う
    let pick = RwSignal::new(db.with_untracked(|d| {
        let saved = storage::saved_progress_pick();
        let restored = core::restore_pick(d, saved.0.as_deref(), saved.1.as_deref());
        // ★ **候補から落ちた ID を保存値に残さない。** 残すと、その種目の記録を
        //   もう一度作った瞬間に選択が黙って戻る（消えたはずの対象が復活する）。
        //   一致していれば書かないので、ふつうのマウントで localStorage は触らない
        if restored.ids() != saved {
            storage::save_progress_pick(restored);
        }
        restored
    }));
    let period = RwSignal::new(Period::M3);
    let metric = RwSignal::new(Metric::default());

    // ★ **部位と種目を組で 1 回だけ set する唯一の経路。** 不変条件は `core::Pick` が
    //   持ち、ここは保存とシグナル更新に徹する。シグナルを 2 本に分けて `Effect` で
    //   追随させると循環し、2 回の set の間に食い違った中間状態が DOM に出る
    let commit = move |p: Pick| {
        if pick.get_untracked() == p {
            return; // 同値なら localStorage を無駄に叩かない（settings.rs の pick と同じ形）
        }
        storage::save_progress_pick(p);
        pick.set(p);
    };

    // ★ 表示中の種目の記録を全部消すと、その種目は候補から外れるのに pick は残る。
    //   select は先頭項目を表示しているのにグラフは消えた種目の空系列、という
    //   食い違いが起きるので、候補の変化を購読して寄せ直す。
    //
    //   ★ **保存値に `Db` の ID を置ける根拠がこの受け皿**
    //     （adr/storage/db-ids-in-ui-state-behind-a-fallback.md）。宙に浮いた ID は
    //     ここで必ず既定へ倒れ、保存も同時に揃う
    Effect::new(move |_| {
        let opts = opts.get(); // 購読はここだけ（`db` を直に見ると打鍵ごとに走る）
        if opts.is_live(pick.get_untracked()) {
            return;
        }
        commit(opts.default_pick());
    });

    // 画面の下半分に何を出すか。`pick` を直に購読しない理由は `Body` の doc を参照
    let body = Memo::new(move |_| {
        if pick.get().is_set() {
            Body::Chart
        } else if opts.get().is_empty() {
            Body::NoRecords
        } else {
            Body::NoPick
        }
    });

    // ★ 単位は選んだ指標だけで決まる（対象種目では決まらない）。
    //   対象を切り替えても軸の意味が変わらないのが、旧 Kind 方式との違い
    let unit = Memo::new(move |_| metric.get().unit(cur_lang()).to_string());

    let series = Memo::new(move |_| {
        let p = pick.get();
        let today = dates.today.get();
        let period = period.get();
        let m = metric.get();
        let drops = drops.get();
        db.with(|d| {
            let (from, to) = bounds(period, today, earliest_session(d));
            // 種目 / 部位 / どちらも「すべて」の分岐は `core::pick_series` に 1 本化してある
            let raw = core::pick_series(d, p, m, from, to, drops);
            // ★「全期間」は週単位集約（1 年分 100 点超をそのまま描くと潰れる）
            if period == Period::All {
                core::aggregate_weekly(&raw)
            } else {
                raw
            }
        })
    });

    // ★ 体重は「対象」でも「指標」でもなく常に重なる第2軸なので、pick / metric を
    //   購読しない。種目を切り替えても再計算されない
    let weight = Memo::new(move |_| {
        let today = dates.today.get();
        let period = period.get();
        db.with(|d| {
            let (from, to) = bounds(period, today, earliest_session(d));
            let raw = core::body_weight_series(d, from, to);
            // ★ 合計ではなく平均。`aggregate_weekly` に通すと「全期間」で 400kg になる。
            //   週キーは指標側と一致するので、2 本の線は同じ日付軸に並ぶ
            if period == Period::All {
                core::aggregate_weekly_avg(&raw)
            } else {
                raw
            }
        })
    });

    // 集計から外したドロップセットがあるか。**外したことを黙らない。**
    //
    // ★ 記録タブは常に全部を数える（設定は「推移の見せ方」なので）。黙って外すと、
    //   同じ日なのに記録タブとセット数・合計が違う理由が画面のどこにも出ない。
    let hidden = Memo::new(move |_| {
        let p = pick.get();
        if !p.is_set() {
            return false;
        }
        let today = dates.today.get();
        let period = period.get();
        let drops = drops.get();
        db.with(|d| {
            let (from, to) = bounds(period, today, earliest_session(d));
            // ★ 「段が在るか」はデータの事実、それを注記にするかは画面の設定。
            //   合成をここでやる（`Period` を `(from, to)` に解くのと同じ線）
            drops == Drops::Exclude && core::any_drops_in_scope(d, p, from, to)
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
        let p = pick.get();
        if !p.is_set() {
            return Vec::new();
        }
        let today = dates.today.get();
        let period = period.get();
        let m = metric.get();
        let drops = drops.get();
        db.with(|d| {
            let (from, to) = bounds(period, today, earliest_session(d));
            let unit = m.unit(cur_lang());
            let show = |v: f64| {
                let n = fmt_metric(v);
                if unit.is_empty() {
                    n
                } else {
                    format!("{n} {unit}")
                }
            };
            let mut rows: Vec<(NaiveDate, String, String)> = Vec::new();
            for (key, session) in &d.sessions {
                let Some(date) = core::parse_date_key(key) else {
                    continue;
                };
                if date < from || date > to {
                    continue;
                }
                // ★ 種目が入っていれば種目が勝つ。分岐は `core::pick_series` と同じ形に
                //   揃える（グラフとテーブルで違う対象を出さないため）
                match (p.exercise, p.group) {
                    (Some(ex), _) => {
                        let Some(log) = session.log_of(ex).filter(|l| !l.sets.is_empty()) else {
                            continue;
                        };
                        // ★ 詳細列も設定に従う。数字だけ外して段を並べると、表の中で
                        //   「見えているセットの合計」と値が食い違う
                        let detail = log
                            .sets
                            .iter()
                            .map(|s| fmt_set(s, drops))
                            .collect::<Vec<_>>()
                            .join("  ");
                        rows.push((date, detail, show(core::log_value_of(m, log, drops))));
                    }
                    (None, Some(g)) => {
                        let ids = d.exercise_ids_of_group(g);
                        let mut names = Vec::new();
                        let mut total = 0.0;
                        let mut hit = false;
                        for log in &session.logs {
                            if ids.contains(&log.exercise_id) && !log.sets.is_empty() {
                                hit = true;
                                total += core::log_value_of(m, log, drops);
                                if let Some(e) = d.exercise(log.exercise_id) {
                                    names.push(ex_name(e).to_string());
                                }
                            }
                        }
                        if !hit {
                            continue;
                        }
                        rows.push((date, names.join("・"), show(total)));
                    }
                    (None, None) => continue, // 上で早期 return 済み
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

    let metric_button = move |m: Metric, label: &'static str| {
        view! {
            <button
                class="seg-btn"
                class:active=move || metric.get() == m
                data-testid="metric-btn"
                on:click=move |_| metric.set(m)
            >
                {label}
            </button>
        }
    };

    // 対象が選ばれているときだけ出す一式（グラフ / 週次注記 / 統計 / 記録テーブル）。
    //
    // ★ `body` の 3 値でだけ作り直す。`pick` を直に購読すると種目を切り替えるたびに
    //   `Chart` が破棄・再生成され、読み取り点の選択が毎回リセットされる
    let chart_body = move || {
        view! {
                <Chart series=series unit=unit weight=weight />

                {move || {
                    (period.get() == Period::All)
                        .then(|| {
                            // 体重が 1 件も無い人に「体重は週平均」と言わない
                            let note = if weight.get().is_empty() {
                                {t.progress.weekly_note}
                            } else {
                                {t.progress.weekly_note_with_weight}
                            };
                            view! {
                                <p class="muted note" data-testid="weekly-note">
                                    {note}
                                </p>
                            }
                        })
                }}

                // ★ **`weekly_note` の隣に無条件で置く。** 下の `chart-metric-empty` は
                //   「体重が 1 件でもある」で括られているので、その中に入れると体重を
                //   量っていない人には出ない。ドロップだけの種目でグラフが空になる
                //   行き止まりを説明するのもこの 1 行
                {move || {
                    hidden
                        .get()
                        .then(|| {
                            view! {
                                <p class="muted note" data-testid="drops-hidden">
                                    {t.progress.drops_hidden_note}
                                </p>
                            }
                        })
                }}

                // ★ その期間にその種目の記録が無くても体重の点線は出る。左軸が消えているので
                //   何のグラフか分からなくなるのを 1 行で補う
                //
                // ★ ドロップを外して空になったときは上の注記のほうが具体的なので出さない
                //   （「この種目の記録はありません」だけだと理由が嘘になる）
                {move || {
                    (series.get().is_empty() && !weight.get().is_empty() && !hidden.get())
                        .then(|| {
                            view! {
                                <p class="muted note" data-testid="chart-metric-empty">
                                    {t.progress.empty_period_exercise}
                                </p>
                            }
                        })
                }}

                <dl class="stats" data-testid="stats">
                    <div>
                        <dt>{t.progress.stat_delta}</dt>
                        <dd data-testid="stat-delta">
                            {move || {
                                let (last, prev, _, _) = stats.get();
                                fmt_delta(last, prev)
                            }}
                        </dd>
                    </div>
                    <div>
                        <dt>{t.progress.stat_best}</dt>
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
                        <dt>{t.progress.stat_average}</dt>
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
                            <p class="muted" data-testid="records-empty">{t.progress.empty_period}</p>
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
                                    <th>{t.progress.col_date}</th>
                                    <th>{t.progress.col_content}</th>
                                    <th>{t.progress.col_metric}</th>
                                </tr>
                            </thead>
                            <tbody>
                                {shown
                                    .into_iter()
                                    .map(|(date, detail, metric)| {
                                        view! {
                                            <tr data-testid="record-row">
                                                <td class="rec-date">{fmt_date(date, cur_lang())}</td>
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
                                                    {cur_lang().n_more_hidden(hidden)}
                                                </td>
                                            </tr>
                                        </tfoot>
                                    }
                                })}
                        </table>
                    }
                        .into_any()
                }}
        }
    };

    view! {
        <section class="progress" data-testid="screen-progress">
            <h1 class="screen-title">{t.progress.title}</h1>

            // 記録が 1 件も無いうちはセレクタも空になる。無言の空画面にしない
            {move || {
                opts.get()
                    .is_empty()
                    .then(|| {
                        view! {
                            <p class="muted" data-testid="progress-empty">
                                {t.progress.empty_all}
                            </p>
                        }
                    })
            }}

            <div class="selectors">
            // ★ 部位と種目を**横に並べる**。縦に積むと 44px + gap ぶんグラフが下がり、
            //   この改修の起点（下の項目が押しにくい）をセレクタ 1 本増やして作り直す
            <div class="target-row">
                <select
                    class="target-select"
                    data-testid="group-select"
                    aria-label=t.progress.pick_group
                    // ★「すべて」は value=""。`Id` の FromStr は 12 文字ちょうどを
                    //   要求するので空文字は必ず None に落ち、番兵値を決めなくて済む
                    on:change=move |ev| {
                        let g = event_target_value(&ev).parse::<GroupId>().ok();
                        db.with_untracked(|d| commit(pick.get_untracked().with_group(d, g)));
                    }
                >
                    {move || {
                        let opts = opts.get();
                        let current = pick.get().group;
                        view! {
                            // ★ `selected=` ではなく `prop:selected=`。**content attribute の
                            //   `selected` は、利用者が一度でも触った `<option>`（HTML の
                            //   dirtiness が立った要素）では selectedness を動かさない。**
                            //   leptos は要素を作り直さず属性を差し替えるので、背中を選ぶ →
                            //   すべての部位に戻す、で部位セレクタが「背中」から動かなくなり、
                            //   `pick` と表示が食い違う（実測済み）。IDL property なら常に勝つ
                            <option value="" prop:selected=current.is_none()>
                                {t.progress.all_groups}
                            </option>
                            {opts
                                .groups
                                .iter()
                                .map(|(id, name)| {
                                    view! {
                                        <option
                                            value=id.to_string()
                                            prop:selected=current == Some(*id)
                                        >
                                            {name.clone()}
                                        </option>
                                    }
                                })
                                .collect::<Vec<_>>()}
                        }
                    }}
                </select>

                <select
                    class="target-select"
                    data-testid="exercise-select"
                    aria-label=t.progress.pick_exercise
                    on:change=move |ev| {
                        let ex = event_target_value(&ev).parse::<ExerciseId>().ok();
                        db.with_untracked(|d| commit(pick.get_untracked().with_exercise(d, ex)));
                    }
                >
                    {move || {
                        let opts = opts.get();
                        let p = pick.get();
                        // 部位が選ばれていればその部位の種目だけ。「すべて」なら全部位ぶん
                        // `prop:selected` を使う理由は部位セレクタ側のコメントを参照
                        let option = move |e: &ExOption| {
                            let selected = p.exercise == Some(e.id);
                            view! {
                                <option value=e.id.to_string() prop:selected=selected>
                                    {e.name.clone()}
                                </option>
                            }
                        };
                        let archived: Vec<_> = opts
                            .exercises_of(p.group, true)
                            .map(&option)
                            .collect();
                        let active: Vec<_> = opts
                            .exercises_of(p.group, false)
                            .map(&option)
                            .collect();
                        view! {
                            <option value="" prop:selected=p.exercise.is_none()>
                                {t.progress.all_exercises}
                            </option>
                            // 中身ゼロの見出しをネイティブのピッカーに出さない
                            // （記録が無い / その部位が全部アーカイブ済みのとき）
                            {(!active.is_empty())
                                .then(|| {
                                    view! {
                                        <optgroup label=t.progress.optgroup_exercises>
                                            {active}
                                        </optgroup>
                                    }
                                })}
                            // アーカイブ済みも出さないと過去データが参照不能になる
                            {(!archived.is_empty())
                                .then(|| {
                                    view! {
                                        <optgroup label=t.progress.optgroup_archived>
                                            {archived}
                                        </optgroup>
                                    }
                                })}
                        }
                    }}
                </select>
            </div>

                // ★ 指標は種目の属性ではなく画面の表示設定なので、対象と並べてここに置く。
                //   単位もこの選択だけで決まる（種目を切り替えても軸の意味が変わらない）
                <div class="segmented" role="group" aria-label=t.progress.pick_metric data-testid="metric-select">
                    {Metric::choices(cur_lang())
                        .into_iter()
                        .map(|(m, label)| metric_button(m, label))
                        .collect::<Vec<_>>()}
                </div>

                <div class="segmented" role="group" aria-label=t.progress.pick_period data-testid="period-select">
                    {Period::choices(cur_lang())
                        .into_iter()
                        .map(|(p, label)| period_button(p, label))
                        .collect::<Vec<_>>()}
                </div>
            </div>

            // 部位も種目も「すべて」ならグラフを出さず、何を選べばよいかだけ言う
            {move || match body.get() {
                Body::Chart => chart_body().into_any(),
                Body::NoPick => {
                    view! {
                        <p class="muted" data-testid="progress-pick-hint">
                            {t.progress.pick_hint}
                        </p>
                    }
                        .into_any()
                }
                // 記録が 1 件も無いときは、セレクタの上の空状態が既に説明している
                Body::NoRecords => ().into_any(),
            }}
        </section>
    }
}
