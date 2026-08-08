//! 自前 SVG 折れ線グラフ。ライブラリなし。
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

use chrono::{Datelike, NaiveDate, TimeDelta};
use leptos::prelude::*;

use super::{fmt_date, fmt_metric};

const VIEW_W: f64 = 320.0;
const VIEW_H: f64 = 160.0;
/// プロット領域（左は Y ラベル、下は X ラベルのぶん空ける）
const X0: f64 = 40.0;
const X1: f64 = VIEW_W - 10.0;
const Y0: f64 = 12.0;
const Y1: f64 = VIEW_H - 22.0;

/// ★ これを超えたら `<circle>` を省略する。
/// 1 年分 100 点超を幅 320 に r=3 で置くと全て重なって判読不能になる。
const DENSE_POINTS: usize = 40;

#[derive(Clone, Debug, PartialEq)]
struct Pt {
    idx: usize,
    x: f64,
    y: f64,
    /// 最近傍スナップのヒット領域（左端 / 幅）。隣接点との中点まで受け持つ
    band_x: f64,
    band_w: f64,
    date: NaiveDate,
    value: f64,
}

#[derive(Clone, Debug, PartialEq, Default)]
struct Layout {
    pts: Vec<Pt>,
    polyline: String,
    /// (y 座標, ラベル)
    grid: Vec<(f64, String)>,
    /// (x 座標, ラベル, text-anchor)
    x_labels: Vec<(f64, String, &'static str)>,
    dense: bool,
    max: f64,
}

fn n(v: f64) -> String {
    format!("{v:.1}")
}

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

fn layout(series: &[(NaiveDate, f64)]) -> Layout {
    if series.is_empty() {
        return Layout::default();
    }
    let count = series.len();
    let max = series.iter().map(|(_, v)| *v).fold(0.0_f64, f64::max);
    // Y 軸は 0〜max×1.1。全部 0 のときでもゼロ除算しないよう下駄を履かせる
    let y_max = if max > 0.0 { max * 1.1 } else { 1.0 };

    let first = series[0].0;
    let last = series[count - 1].0;
    let span_days = (last - first).num_days();

    // ★ X は時間軸に比例配置（等間隔ではない）。休んだ週が空白として見えることが
    //   「これまでの比較」に合う
    let x_of = |d: NaiveDate| -> f64 {
        if span_days > 0 {
            X0 + ((d - first).num_days() as f64 / span_days as f64) * (X1 - X0)
        } else {
            (X0 + X1) / 2.0 // 1 点だけ / 全部同じ日は中央に置く
        }
    };
    let y_of = |v: f64| -> f64 { Y1 - (v / y_max).clamp(0.0, 1.0) * (Y1 - Y0) };

    let xs: Vec<f64> = series.iter().map(|(d, _)| x_of(*d)).collect();
    let pts: Vec<Pt> = series
        .iter()
        .enumerate()
        .map(|(i, (date, value))| {
            let left = if i == 0 {
                X0
            } else {
                xs[i - 1].midpoint(xs[i])
            };
            let right = if i + 1 == count {
                X1
            } else {
                xs[i].midpoint(xs[i + 1])
            };
            Pt {
                idx: i,
                x: xs[i],
                y: y_of(*value),
                band_x: left,
                band_w: (right - left).max(0.0),
                date: *date,
                value: *value,
            }
        })
        .collect();

    let polyline = pts
        .iter()
        .map(|p| format!("{:.1},{:.1}", p.x, p.y))
        .collect::<Vec<_>>()
        .join(" ");

    // グリッド 3 本（上端 / 中間 / 0）。fmt_metric ではなく fmt_axis_label を使う
    // 理由は同関数のコメントを参照（viewBox からの溢れ = 数値の誤読を防ぐため）
    let grid = [y_max, y_max / 2.0, 0.0]
        .into_iter()
        .map(|v| (y_of(v), fmt_axis_label(v)))
        .collect();

    // X 軸ラベルは最初・中間・最後の 3 個。軸が時間に線形なので中間ラベルは
    // 中央の x にそのまま「期間の中日」を置けばよい
    let x_labels = if span_days > 0 {
        let mid = first + TimeDelta::days(span_days / 2);
        vec![
            (X0, fmt_md(first), "start"),
            ((X0 + X1) / 2.0, fmt_md(mid), "middle"),
            (X1, fmt_md(last), "end"),
        ]
    } else {
        vec![((X0 + X1) / 2.0, fmt_md(first), "middle")]
    };

    Layout {
        pts,
        polyline,
        grid,
        x_labels,
        dense: count > DENSE_POINTS,
        max,
    }
}

#[component]
pub fn Chart(
    /// (日付, 値) の系列。日付昇順であること
    #[prop(into)]
    series: Signal<Vec<(NaiveDate, f64)>>,
    /// 値の単位（"kg·回" / "回" / "秒" / "セット"）
    #[prop(into)]
    unit: Signal<String>,
) -> impl IntoView {
    let plot = Memo::new(move |_| layout(&series.get()));
    let selected = RwSignal::new(None::<usize>);

    // 系列が変わったら選択を最新点に寄せる（読み取り欄が常に何かを示す）
    Effect::new(move |_| {
        let count = plot.with(|l| l.pts.len());
        selected.set(count.checked_sub(1));
    });

    view! {
        <div class="chart-wrap">
            {move || {
                let l = plot.get();
                if l.pts.is_empty() {
                    return view! {
                        <p class="chart-empty" data-testid="chart-empty">"記録がありません"</p>
                    }
                        .into_any();
                }
                let label = format!(
                    "{} から {} までの推移。最大 {} {}",
                    fmt_date(l.pts[0].date),
                    fmt_date(l.pts[l.pts.len() - 1].date),
                    fmt_metric(l.max),
                    unit.get(),
                );
                let show_dots = !l.dense;
                let last_idx = l.pts.len() - 1;
                view! {
                    <svg
                        class="chart"
                        viewBox="0 0 320 160"
                        role="img"
                        aria-label=label
                        data-testid="chart"
                        data-points=l.pts.len().to_string()
                        data-dense=l.dense.to_string()
                    >
                        {l
                            .grid
                            .iter()
                            .map(|(y, text)| {
                                view! {
                                    <g>
                                        <line
                                            class="chart-grid"
                                            x1=n(X0)
                                            y1=n(*y)
                                            x2=n(X1)
                                            y2=n(*y)
                                        />
                                        <text
                                            class="chart-label"
                                            x=n(X0 - 5.0)
                                            y=n(*y + 3.5)
                                            text-anchor="end"
                                        >
                                            {text.clone()}
                                        </text>
                                    </g>
                                }
                            })
                            .collect::<Vec<_>>()}

                        {l
                            .x_labels
                            .iter()
                            .map(|(x, text, anchor)| {
                                view! {
                                    <text
                                        class="chart-label"
                                        x=n(*x)
                                        y=n(Y1 + 14.0)
                                        text-anchor=*anchor
                                    >
                                        {text.clone()}
                                    </text>
                                }
                            })
                            .collect::<Vec<_>>()}

                        <polyline class="chart-line" points=l.polyline.clone() />

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
                                .and_then(|i| l.pts.get(i).cloned())
                                .map(|p| {
                                    view! {
                                        <g data-testid="chart-cursor">
                                            <line
                                                class="chart-cursor"
                                                x1=n(p.x)
                                                y1=n(Y0)
                                                x2=n(p.x)
                                                y2=n(Y1)
                                            />
                                            <circle
                                                class="chart-dot selected"
                                                cx=n(p.x)
                                                cy=n(p.y)
                                                r="5"
                                            />
                                        </g>
                                    }
                                })
                        }}

                        // ★ タップは点ではなくプロット領域の全高を覆う透明な rect で受ける。
                        //   r=3 は実機で直径約 7px しかなく min-height:44px 規約に反する。
                        //   1 点 = 隣接点との中点までの帯にしてあるので、タッチ X 座標の
                        //   最近傍点へのスナップが座標計算なしで成立する
                        {l
                            .pts
                            .iter()
                            .map(|p| {
                                let idx = p.idx;
                                view! {
                                    <rect
                                        class="chart-hit"
                                        x=n(p.band_x)
                                        y=n(Y0)
                                        width=n(p.band_w)
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
                    .and_then(|i| l.pts.get(i).cloned())
                    .map(|p| {
                        view! {
                            <p class="chart-readout" data-testid="chart-readout">
                                <span class="muted">{fmt_date(p.date)}</span>
                                <strong>{format!("{} {}", fmt_metric(p.value), unit)}</strong>
                            </p>
                        }
                    })
            }}
        </div>
    }
}
