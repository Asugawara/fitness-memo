//! 自前 SVG 折れ線グラフ。ライブラリなし。
//!
//! **座標計算は [`crate::chart_layout`] にある。**ここは描画と書式だけを持つ。
//! 軸ラベルは数値・日付のまま受け取り、`fmt_axis_label` / `fmt_md` をここで当てる
//! （書式ヘルパが `views` にあるので、計算側に持たせると wasm32 gate から出られない）。
//!
//! ⚠ `view!` の名前空間の罠: `svg` / `g` / `line` / `polyline` / `circle` / `rect` /
//! `text` / `tspan` は leptos_macro の SVG リストに入っているので正しく解決される。
//! **`<title>` / `<a>` / `<script>` は曖昧要素**で、親コンテキストが不明だと HTML 要素に
//! なる。ここではアクセシブルネームに `role="img"` + `aria-label` を使い、`<title>` を
//! 使わないことで罠自体を回避している。
//!
//! ⚠ SVG 属性は無検査で `setAttribute` される。`view_box` や `stroke_width` のような
//! snake_case のタイポはコンパイルエラーにならず**実行時に黙って無視される**。
//! 綴りは `viewBox` / `stroke-width` / `text-anchor`。
//! 見た目は極力 CSS クラスに寄せて（`--accent` などのトークンでダークモードに追随させ）、
//! 属性には座標だけを置く。

use chrono::{Datelike, NaiveDate};
use leptos::prelude::*;

use crate::chart_layout::{GRID_Y, X0, Y0, Y1, layout, n};

use super::{fmt_date, fmt_metric, fmt_weight};

/// "8/8"
fn fmt_md(d: NaiveDate) -> String {
    format!("{}/{}", d.month(), d.day())
}

/// Y 軸ラベル専用の表記。**6 桁（100,000）以上は千/百万単位の短縮表記に切り替える。**
///
/// `fmt_metric` の桁区切り表記のまま置くと、ラベルは `text-anchor="end"` で
/// `x = X0 - 5.0`（プロット領域の左マージン内）に収まる前提で描かれているため、
/// 文字数が伸びると viewBox の左端（x=0）から溢れて先頭の桁が欠ける
/// （実測: "2,954,576" が x=-11.6 で描画され "954,576" に見える）。単なる見切れではなく
/// 数値の誤読なので、桁区切りを増やす代わりに単位を切り替えて文字数を頭打ちにする。
/// 6桁到達時点で既に "999,999"（7文字）が幅を超えるため、閾値は6桁（10^5）に置く。
fn fmt_axis_label(v: f64) -> String {
    const TIERS: [(f64, f64, &str); 3] = [
        (1e9, 1e9, "B"),
        (1e6, 1e6, "M"),
        // "k" だけ判定閾値(1e5 = 6桁)と割る単位(1e3)が異なる。5桁以下は桁区切りのままで
        // 幅に収まるため短縮しない
        (1e5, 1e3, "k"),
    ];
    let abs = v.abs();
    for (threshold, scale, suffix) in TIERS {
        if abs >= threshold {
            let scaled = v / scale;
            // 3桁(100 以上)は小数を出さなくても "990k" のように十分な精度が残る。
            // 2桁以下は小数1桁を残して丸めの粗さを抑える（"2.9M" 等）
            return if scaled.abs() >= 100.0 {
                format!("{scaled:.0}{suffix}")
            } else {
                format!("{scaled:.1}{suffix}")
            };
        }
    }
    fmt_metric(v)
}

/// 体重の表記。`60.0 → "60"`、`62.5 → "62.5"`。
///
/// ★ 小数 1 桁に丸めてから渡す。右軸ラベル（帯の端）は必ず 0.5 の倍数なので丸めても
/// 変わらないが、**読み取り欄と `aria-label` には週平均が来る**（「全期間」は
/// `aggregate_weekly_avg` を通った値）。素通しすると `71.88571 kg` と表示され、
/// 桁数も 5 文字を超えて右軸ラベルが viewBox から溢れる。
fn fmt_weight_label(v: f64) -> String {
    fmt_weight((v * 10.0).round() as f32 / 10.0)
}

#[component]
pub fn Chart(
    /// (日付, 値) の系列。日付昇順であること
    #[prop(into)]
    series: Signal<Vec<(NaiveDate, f64)>>,
    /// 値の単位（"kg·回" / "回" / "秒" / "セット"）
    #[prop(into)]
    unit: Signal<String>,
    /// (日付, 体重kg) の系列。日付昇順であること。**第2軸として常に重ねる**
    #[prop(into)]
    weight: Signal<Vec<(NaiveDate, f64)>>,
) -> impl IntoView {
    let plot = Memo::new(move |_| layout(&series.get(), &weight.get()));
    let selected = RwSignal::new(None::<usize>);

    // 系列が変わったら選択を最新点に寄せる（読み取り欄が常に何かを示す）
    Effect::new(move |_| {
        let count = plot.with(|l| l.bands.len());
        selected.set(count.checked_sub(1));
    });

    view! {
        <div class="chart-wrap">
            {move || {
                let l = plot.get();
                if l.is_empty() {
                    return view! {
                        <p class="chart-empty" data-testid="chart-empty">"記録がありません"</p>
                    }
                        .into_any();
                }
                let x1 = l.x1;
                let span = l
                    .x_labels
                    .first()
                    .zip(l.x_labels.last())
                    .map(|((_, from, _), (_, to, _))| {
                        format!("{} から {} まで", fmt_date(*from), fmt_date(*to))
                    })
                    .unwrap_or_default();
                let metric_part = l
                    .y_values
                    .map(|_| format!("の推移。最大 {} {}", fmt_metric(l.max), unit.get()))
                    .unwrap_or_else(|| "の体重の推移".to_string());
                let weight_part = l
                    .weight
                    .as_ref()
                    .filter(|_| l.y_values.is_some())
                    .map(|w| {
                        format!(
                            "。体重 {}〜{} kg",
                            fmt_weight_label(w.min),
                            fmt_weight_label(w.max),
                        )
                    })
                    .unwrap_or_default();
                let label = format!("{span}{metric_part}{weight_part}");
                let show_dots = !l.dense;
                let last_idx = l.pts.len().saturating_sub(1);
                let weight_points = l.weight.as_ref().map_or(0, |w| w.points);
                view! {
                    <svg
                        class="chart"
                        viewBox="0 0 320 160"
                        role="img"
                        aria-label=label
                        data-testid="chart"
                        data-points=l.pts.len().to_string()
                        data-weight-points=weight_points.to_string()
                        data-dense=l.dense.to_string()
                    >
                        // グリッドは 3 本のまま両軸で共用する。左ラベルがメイン指標、
                        // 右ラベルが体重を指すだけで、線は増やさない
                        {GRID_Y
                            .iter()
                            .map(|y| {
                                view! {
                                    <line
                                        class="chart-grid"
                                        x1=n(X0)
                                        y1=n(*y)
                                        x2=n(x1)
                                        y2=n(*y)
                                    />
                                }
                            })
                            .collect::<Vec<_>>()}

                        // ★ メイン系列が空のときは左ラベルを出さない。y_max が 1.0 に
                        //   化けるので "1 / 0.5 / 0" が並び、体重の目盛りだと誤読される
                        {l
                            .y_values
                            .map(|values| {
                                GRID_Y
                                    .iter()
                                    .zip(values)
                                    .map(|(y, value)| {
                                        view! {
                                            <text
                                                class="chart-label"
                                                x=n(X0 - 5.0)
                                                y=n(*y + 3.5)
                                                text-anchor="end"
                                            >
                                                {fmt_axis_label(value)}
                                            </text>
                                        }
                                    })
                                    .collect::<Vec<_>>()
                            })}

                        // 右軸（体重）。★ chart-label も付けるのは、既存の
                        // 「ラベルが viewBox から溢れていないか」を見る E2E ヘルパが
                        // text.chart-label を走査しているため（無改修で検査対象に入る）
                        {l
                            .weight
                            .as_ref()
                            .map(|w| {
                                GRID_Y
                                    .iter()
                                    .zip(w.values)
                                    .map(|(y, value)| {
                                        view! {
                                            <text
                                                class="chart-label chart-label-weight"
                                                x=n(x1 + 5.0)
                                                y=n(*y + 3.5)
                                                text-anchor="start"
                                            >
                                                {fmt_weight_label(value)}
                                            </text>
                                        }
                                    })
                                    .collect::<Vec<_>>()
                            })}

                        {l
                            .x_labels
                            .iter()
                            .map(|(x, date, anchor)| {
                                view! {
                                    <text
                                        class="chart-label"
                                        x=n(*x)
                                        y=n(Y1 + 14.0)
                                        text-anchor=*anchor
                                    >
                                        {fmt_md(*date)}
                                    </text>
                                }
                            })
                            .collect::<Vec<_>>()}

                        // ★ 体重はメインの線より先に描いて背面に置く。控えめさは色ではなく
                        //   （--muted と --accent のコントラスト比はほぼ同じ）、線の細さ・
                        //   破線・ドットを描かないこと・背面配置で作る
                        {l
                            .weight
                            .as_ref()
                            .map(|w| {
                                let dot = w
                                    .dot
                                    .map(|(x, y)| {
                                        view! {
                                            <circle class="chart-dot-weight" cx=n(x) cy=n(y) r="2.5" />
                                        }
                                    });
                                view! {
                                    <g data-testid="chart-weight">
                                        <polyline
                                            class="chart-line-weight"
                                            points=w.polyline.clone()
                                        />
                                        {dot}
                                    </g>
                                }
                            })}

                        {(!l.polyline.is_empty())
                            .then(|| {
                                view! { <polyline class="chart-line" points=l.polyline.clone() /> }
                            })}

                        // ★ 点が多いと circle が重なって潰れるので、密なときは最新点だけ描く
                        {l
                            .pts
                            .iter()
                            .filter(|p| show_dots || p.idx == last_idx)
                            .map(|p| {
                                view! {
                                    <circle class="chart-dot" cx=n(p.x) cy=n(p.y) r="3" />
                                }
                            })
                            .collect::<Vec<_>>()}

                        // 選択中の点を強調（縦ガイド + 大きめの丸）
                        {move || {
                            let l = plot.get();
                            selected
                                .get()
                                .and_then(|i| l.bands.get(i).cloned())
                                .map(|b| {
                                    let metric_dot = b
                                        .y
                                        .map(|y| {
                                            view! {
                                                <circle
                                                    class="chart-dot selected"
                                                    cx=n(b.x)
                                                    cy=n(y)
                                                    r="5"
                                                />
                                            }
                                        });
                                    // ★ 線の上に実際に点がある日だけ打つ。週平均に落として
                                    //   いる間は打たない（存在しない観測値を捏造しない）
                                    let weight_dot = b
                                        .w_y
                                        .map(|y| {
                                            view! {
                                                <circle
                                                    class="chart-dot-weight selected"
                                                    cx=n(b.x)
                                                    cy=n(y)
                                                    r="3"
                                                />
                                            }
                                        });
                                    view! {
                                        <g data-testid="chart-cursor">
                                            <line
                                                class="chart-cursor"
                                                x1=n(b.x)
                                                y1=n(Y0)
                                                x2=n(b.x)
                                                y2=n(Y1)
                                            />
                                            {weight_dot}
                                            {metric_dot}
                                        </g>
                                    }
                                })
                        }}

                        // ★ タップは点ではなくプロット領域の全高を覆う透明な rect で受ける。
                        //   r=3 は実機で直径約 7px しかなく min-height:44px 規約に反する。
                        //   1 点 = 隣接点との中点までの帯にしてあるので、タッチ X 座標の
                        //   最近傍点へのスナップが座標計算なしで成立する
                        {l
                            .bands
                            .iter()
                            .map(|b| {
                                let idx = b.idx;
                                view! {
                                    <rect
                                        class="chart-hit"
                                        x=n(b.band_x)
                                        y=n(Y0)
                                        width=n(b.band_w)
                                        height=n(Y1 - Y0)
                                        data-testid="chart-hit"
                                        on:click=move |_| selected.set(Some(idx))
                                    />
                                }
                            })
                            .collect::<Vec<_>>()}
                    </svg>
                }
                    .into_any()
            }}

            {move || {
                let l = plot.get();
                let unit = unit.get();
                selected
                    .get()
                    .and_then(|i| l.bands.get(i).cloned())
                    .map(|b| {
                        let metric = b
                            .value
                            .map(|v| view! { <strong>{format!("{} {}", fmt_metric(v), unit)}</strong> });
                        let weight = b
                            .weight
                            .map(|w| {
                                view! {
                                    <span class="rd-weight" data-testid="readout-weight">
                                        {format!("{} kg", fmt_weight_label(w))}
                                    </span>
                                }
                            });
                        view! {
                            <p class="chart-readout" data-testid="chart-readout">
                                <span class="muted">{fmt_date(b.date)}</span>
                                <span class="rd-values">{metric} {weight}</span>
                            </p>
                        }
                    })
            }}
        </div>
    }
}
