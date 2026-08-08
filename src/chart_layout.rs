//! 推移グラフの座標計算。**ターゲット非依存なので `cargo test` で検証できる。**
//!
//! [ADR-0004](../adr/architecture/0004-no-chart-library-hand-rolled-svg.md) は
//! 「`layout()` のテストが無いのは弱点で、`core.rs` に移せばホストでテストできた」と
//! 自分で書いている。描画（`views::chart`）から座標計算だけを切り出したのがこのモジュール。
//!
//! **文字列整形はここに置かない。** 軸ラベルは数値・日付のまま返し、書式は view 側が当てる。
//! こうしないと `views::mod` の `fmt_metric` に依存して wasm32 gate から出られない。
//!
//! ## 2 つの軸
//!
//! 左（メイン指標）は 0 起点で `0〜max×1.1`。右（体重）は [`core::weight_band`] が作る
//! min/max ベースの帯。**横のグリッド線は 3 本のまま共用**で、左右のラベルが別の値を指すだけ。
//!
//! 体重は毎日、トレーニングは週数回なので、**X のドメインは両系列の合併**にする。
//! そうしないと最後にトレした日より後の計量が軸の外に落ちて見えなくなる。

use chrono::{NaiveDate, TimeDelta};

use crate::core;

pub const VIEW_W: f64 = 320.0;
pub const VIEW_H: f64 = 160.0;
/// プロット領域（左は Y ラベル、下は X ラベルのぶん空ける）
pub const X0: f64 = 40.0;
/// 体重が無いときの右端。**既存の見た目を 1px も動かさないための値。**
pub const X1: f64 = VIEW_W - 10.0;
/// 体重があるときの右端。右軸ラベルのぶんを空ける。
///
/// 予算は `VIEW_W - (X1_DUAL + 5) = 29px`。実測でこの端末のフォント（`-apple-system` 系）は
/// 数字 ≈ 0.55em / `.` ≈ 0.26em なので、`font-size: 9px` なら 5 文字（"137.5" ≈ 23px）まで入る。
/// ラベルが 5 文字を超えないことは [`core::WEIGHT_MAX`] が保証している。
pub const X1_DUAL: f64 = VIEW_W - 34.0;
pub const Y0: f64 = 12.0;
pub const Y1: f64 = VIEW_H - 22.0;

/// ★ これを超えたら `<circle>` を省略する。
/// 1 年分 100 点超を幅 320 に r=3 で置くと全て重なって判読不能になる。
pub const DENSE_POINTS: usize = 40;

/// ★ 体重をこれ以上の点数で描くと破線が潰れる。超えたら**描画だけ**週平均に落とす。
///
/// 1Y × 毎日計量 = 365 点をプロット幅 ~250px に置くと 0.68px/点。日々の体重は ±0.8kg
/// 揺れるうえ第2軸は min/max にぴったり合わせてあるので、破線（周期 10px）が完全に潰れて
/// **灰色の帯**になる。「そんなに目立たないように」の真逆なので、線だけ滑らかにする。
/// 読み取り欄（[`Band::weight`]）は集約前の実測値のままにする。
pub const WEIGHT_DENSE_POINTS: usize = 45;

/// 横のグリッド線の y 座標（上端 / 中間 / 0）。**左右の軸で共用する。**
///
/// 主軸の `y_of` に `[y_max, y_max/2, 0]` を通した結果と厳密に一致する定数。
pub const GRID_Y: [f64; 3] = [Y0, (Y0 + Y1) / 2.0, Y1];

#[derive(Clone, Debug, PartialEq)]
pub struct Pt {
    pub idx: usize,
    pub x: f64,
    pub y: f64,
    pub date: NaiveDate,
    pub value: f64,
}

/// タップの受け皿。**1 点 = 隣接点との中点までの帯**なので、タッチ X 座標の
/// 最近傍点へのスナップが座標計算なしで成立する。
///
/// 読み取り欄もここから作る。メイン指標と体重は日付が揃わない（体重の方が密）ので、
/// どちらも `Option` で持つ。
#[derive(Clone, Debug, PartialEq)]
pub struct Band {
    pub idx: usize,
    pub x: f64,
    pub band_x: f64,
    pub band_w: f64,
    pub date: NaiveDate,
    /// その日のメイン指標
    pub value: Option<f64>,
    pub y: Option<f64>,
    /// その日の体重。**集約前の実測値**（読み取り欄に出すのは記録した数字そのもの）
    pub weight: Option<f64>,
    /// 描画中の体重の線の上に同じ日付の点があるときだけ。
    /// 週平均に落としている間は `None`（存在しない観測値を点として捏造しない）
    pub w_y: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WeightLayer {
    pub polyline: String,
    /// 描画点が 1 個のときだけ。`polyline` は頂点 1 個だと何も描かれない
    pub dot: Option<(f64, f64)>,
    /// 右軸ラベルの値。[`GRID_Y`] と同じ順（上 / 中央 / 下）
    pub values: [f64; 3],
    /// 描画用に週平均へ落としたか
    pub aggregated: bool,
    /// 描画した点の数
    pub points: usize,
    /// aria-label 用。**実データ**の範囲（帯ではない）
    pub min: f64,
    pub max: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub pts: Vec<Pt>,
    pub polyline: String,
    pub bands: Vec<Band>,
    /// 左軸ラベルの値（上 / 中央 / 0）。**メイン系列が空なら `None`。**
    /// 空のまま出すと `1 / 0.5 / 0` という無意味な目盛りが残り、体重の線が
    /// メイン指標だと誤読される
    pub y_values: Option<[f64; 3]>,
    /// (x 座標, 日付, text-anchor)
    pub x_labels: Vec<(f64, NaiveDate, &'static str)>,
    pub dense: bool,
    pub max: f64,
    /// プロット領域の右端。体重の有無で変わるので、グリッド線と X ラベルもこれを見る
    pub x1: f64,
    pub weight: Option<WeightLayer>,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            pts: Vec::new(),
            polyline: String::new(),
            bands: Vec::new(),
            y_values: None,
            x_labels: Vec::new(),
            dense: false,
            max: 0.0,
            x1: X1,
            weight: None,
        }
    }
}

impl Layout {
    /// 描くものが何も無い（両系列とも空）。
    pub fn is_empty(&self) -> bool {
        self.pts.is_empty() && self.weight.is_none()
    }
}

/// 座標を SVG 属性に載せる形にする。
pub fn n(v: f64) -> String {
    format!("{v:.1}")
}

/// `series` はメイン指標、`weight` は体重。どちらも日付昇順であること。
pub fn layout(series: &[(NaiveDate, f64)], weight: &[(NaiveDate, f64)]) -> Layout {
    if series.is_empty() && weight.is_empty() {
        return Layout::default();
    }

    let x1 = if weight.is_empty() { X1 } else { X1_DUAL };

    // ── X は両系列の合併ドメイン ──
    // 体重は毎日あるので、実質「最初の記録 〜 今日」になる。最後にトレした日から
    // 今日までが右の空白として見えるのは、休んだ期間を空白で示す既存の方針と同じ
    let first = [series.first(), weight.first()]
        .into_iter()
        .flatten()
        .map(|(d, _)| *d)
        .min()
        .expect("どちらかは空でない");
    let last = [series.last(), weight.last()]
        .into_iter()
        .flatten()
        .map(|(d, _)| *d)
        .max()
        .expect("どちらかは空でない");
    let span_days = (last - first).num_days();

    // ★ X は時間軸に比例配置（等間隔ではない）。休んだ週が空白として見えることが
    //   「これまでの比較」に合う
    let x_of = |d: NaiveDate| -> f64 {
        if span_days > 0 {
            X0 + ((d - first).num_days() as f64 / span_days as f64) * (x1 - X0)
        } else {
            (X0 + x1) / 2.0 // 1 点だけ / 全部同じ日は中央に置く
        }
    };

    // ── 左軸（メイン指標）。0 起点 ──
    let max = series.iter().map(|(_, v)| *v).fold(0.0_f64, f64::max);
    // 全部 0 のときでもゼロ除算しないよう下駄を履かせる
    let y_max = if max > 0.0 { max * 1.1 } else { 1.0 };
    let y_of = |v: f64| -> f64 { Y1 - (v / y_max).clamp(0.0, 1.0) * (Y1 - Y0) };

    let pts: Vec<Pt> = series
        .iter()
        .enumerate()
        .map(|(i, (date, value))| Pt {
            idx: i,
            x: x_of(*date),
            y: y_of(*value),
            date: *date,
            value: *value,
        })
        .collect();
    let polyline = polyline_of(pts.iter().map(|p| (p.x, p.y)));

    // ── 右軸（体重）。min/max ベースの帯 ──
    // 描画用は密なら週平均に落とす。帯も描画用系列から作るので、描いた線は必ず帯に収まる
    let drawn: Vec<(NaiveDate, f64)> = if weight.len() > WEIGHT_DENSE_POINTS {
        core::aggregate_weekly_avg(weight)
    } else {
        weight.to_vec()
    };
    // 契約により hi > lo かつ有限。ここでゼロ除算 / NaN 座標が出ないことが担保される
    let band = (!drawn.is_empty()).then(|| {
        let lo_v = drawn.iter().map(|(_, v)| *v).fold(f64::INFINITY, f64::min);
        let hi_v = drawn
            .iter()
            .map(|(_, v)| *v)
            .fold(f64::NEG_INFINITY, f64::max);
        core::weight_band(lo_v, hi_v)
    });
    // 選択カーソルが体重の線の上に丸を打つための座標。日付で引けるように持つ
    let drawn_y: Vec<(NaiveDate, f64)> = match band {
        Some((lo, hi)) => drawn
            .iter()
            .map(|(d, v)| (*d, Y1 - ((v - lo) / (hi - lo)).clamp(0.0, 1.0) * (Y1 - Y0)))
            .collect(),
        None => Vec::new(),
    };
    let weight_layer = band.map(|(lo, hi)| {
        let xy: Vec<(f64, f64)> = drawn_y.iter().map(|(d, y)| (x_of(*d), *y)).collect();
        WeightLayer {
            polyline: polyline_of(xy.iter().copied()),
            dot: (xy.len() == 1).then(|| xy[0]),
            values: [hi, (lo + hi) / 2.0, lo],
            aggregated: drawn.len() != weight.len(),
            points: xy.len(),
            min: weight.iter().map(|(_, v)| *v).fold(f64::INFINITY, f64::min),
            max: weight
                .iter()
                .map(|(_, v)| *v)
                .fold(f64::NEG_INFINITY, f64::max),
        }
    });

    // ── タップ帯 ──
    // ★ メイン系列の日付から作る。体重は毎日あるので、そこから帯を作ると 3M で 1 本 3px に
    //   なりタップターゲット規約（44px）に反する。トレした日に吸着させれば帯は十数個で済み、
    //   その日の体重はほぼ必ず存在するので読み取り欄で読める。
    //   メインが空のときだけ体重（描画用）を軸にする
    let metric_anchored = !pts.is_empty();
    let seeds: Vec<(NaiveDate, f64)> = if metric_anchored {
        pts.iter().map(|p| (p.date, p.x)).collect()
    } else {
        drawn_y.iter().map(|(d, _)| (*d, x_of(*d))).collect()
    };
    let bands = seeds
        .iter()
        .enumerate()
        .map(|(i, (date, x))| {
            let left = if i == 0 {
                X0
            } else {
                seeds[i - 1].1.midpoint(*x)
            };
            let right = if i + 1 == seeds.len() {
                x1
            } else {
                x.midpoint(seeds[i + 1].1)
            };
            let pt = pts.iter().find(|p| p.date == *date);
            Band {
                idx: i,
                x: *x,
                band_x: left,
                band_w: (right - left).max(0.0),
                date: *date,
                value: pt.map(|p| p.value),
                y: pt.map(|p| p.y),
                // 読み取り欄は集約前の実測値。メインが空で体重を軸にしている間は
                // 軸そのものが描画用系列なので、そちらの値を出す（線と数字を一致させる）
                weight: if metric_anchored {
                    value_at(weight, *date)
                } else {
                    value_at(&drawn, *date)
                },
                w_y: value_at(&drawn_y, *date),
            }
        })
        .collect();

    // X 軸ラベルは最初・中間・最後の 3 個。軸が時間に線形なので中間ラベルは
    // 中央の x にそのまま「期間の中日」を置けばよい
    let x_labels = if span_days > 0 {
        let mid = first + TimeDelta::days(span_days / 2);
        vec![
            (X0, first, "start"),
            ((X0 + x1) / 2.0, mid, "middle"),
            (x1, last, "end"),
        ]
    } else {
        vec![((X0 + x1) / 2.0, first, "middle")]
    };

    Layout {
        pts,
        polyline,
        bands,
        y_values: (!series.is_empty()).then_some([y_max, y_max / 2.0, 0.0]),
        x_labels,
        dense: series.len() > DENSE_POINTS,
        max,
        x1,
        weight: weight_layer,
    }
}

fn polyline_of(points: impl Iterator<Item = (f64, f64)>) -> String {
    points
        .map(|(x, y)| format!("{x:.1},{y:.1}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// その日付の値。**線形探索**にしてあるのは、未ソートの入力でも黙って外さないため
/// （二分探索だと「読み取り欄の体重が常に出ない」という静かな壊れ方をする）。
fn value_at(series: &[(NaiveDate, f64)], date: NaiveDate) -> Option<f64> {
    series.iter().find(|(d, _)| *d == date).map(|(_, v)| *v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).expect("有効な日付")
    }

    /// `from` から 1 日刻みで `values` を並べた系列。
    fn daily(from: NaiveDate, values: &[f64]) -> Vec<(NaiveDate, f64)> {
        values
            .iter()
            .enumerate()
            .map(|(i, v)| (from + TimeDelta::days(i as i64), *v))
            .collect()
    }

    fn metric() -> Vec<(NaiveDate, f64)> {
        vec![
            (d(2026, 8, 1), 600.0),
            (d(2026, 8, 4), 700.0),
            (d(2026, 8, 6), 650.0),
        ]
    }

    /// SVG の `points` に混ざった `NaN` はパースエラーになり、**折れ線が丸ごと
    /// 描かれなくなる**（例外も出ない）。座標が全て有限であることを機械的に確かめる。
    fn assert_all_coords_finite(l: &Layout) {
        for p in &l.pts {
            assert!(p.x.is_finite() && p.y.is_finite(), "pt {p:?}");
        }
        for b in &l.bands {
            assert!(b.x.is_finite() && b.band_x.is_finite() && b.band_w.is_finite());
            assert!(b.y.is_none_or(f64::is_finite));
            assert!(b.w_y.is_none_or(f64::is_finite));
        }
        for (x, _, _) in &l.x_labels {
            assert!(x.is_finite());
        }
        if let Some(w) = &l.weight {
            assert!(
                w.values.iter().all(|v| v.is_finite()),
                "右軸 {:?}",
                w.values
            );
            assert!(!w.polyline.contains("NaN"), "polyline: {}", w.polyline);
            assert!(!w.polyline.contains("inf"), "polyline: {}", w.polyline);
        }
        assert!(!l.polyline.contains("NaN"), "polyline: {}", l.polyline);
    }

    #[test]
    fn empty_input_draws_nothing() {
        let l = layout(&[], &[]);
        assert!(l.is_empty());
        assert!(l.weight.is_none());
        assert_eq!(l.x1, X1);
    }

    /// ★ 体重が無いときの見た目が 1px も動かないこと。
    /// 右軸のぶん右端を縮めるのは体重があるときだけ。
    #[test]
    fn geometry_is_unchanged_when_there_is_no_weight() {
        let l = layout(&metric(), &[]);
        assert_eq!(l.x1, X1);
        assert!(l.weight.is_none());
        // 最初の点は X0、最後の点は X1 に載る
        assert_eq!(l.pts[0].x, X0);
        assert_eq!(l.pts[2].x, X1);
        assert_eq!(l.x_labels.last().expect("3個ある").0, X1);
    }

    #[test]
    fn geometry_reserves_the_right_margin_when_weight_is_present() {
        let l = layout(&metric(), &daily(d(2026, 8, 1), &[70.0, 70.2, 70.1]));
        assert_eq!(l.x1, X1_DUAL);
        assert_eq!(l.x_labels.last().expect("3個ある").0, X1_DUAL);
        let w = l.weight.expect("体重レイヤがある");
        assert_eq!(w.points, 3);
        assert!(!w.aggregated);
    }

    /// ★ 最後にトレした日より後の計量が軸の外に落ちないこと。
    #[test]
    fn x_domain_is_the_union_of_both_series() {
        // メインは 8/1〜8/6、体重は 7/30〜8/9
        let weight = daily(d(2026, 7, 30), &[70.0; 11]);
        let l = layout(&metric(), &weight);

        assert_eq!(l.x_labels[0].1, d(2026, 7, 30), "左端は体重の初日");
        assert_eq!(l.x_labels[2].1, d(2026, 8, 9), "右端は体重の最終日");
        // メインの線は左端にも右端にも届かない（休んでいた期間が空白として見える）
        assert!(l.pts[0].x > X0);
        assert!(l.pts.last().expect("3点ある").x < l.x1);
        assert_all_coords_finite(&l);
    }

    /// メイン 1 点 × 体重多数。合併ドメインなので中央寄せ（`span_days == 0`）が解除される。
    #[test]
    fn a_single_metric_point_is_not_centered_when_weight_spans_days() {
        // 8/3 は 8/1〜8/9 の中央（8/5）ではないので、中央寄せなら座標が合わない
        let one = vec![(d(2026, 8, 3), 600.0)];
        let l = layout(&one, &daily(d(2026, 8, 1), &[70.0; 9]));
        assert_ne!(l.pts[0].x, (X0 + l.x1) / 2.0);
        assert_eq!(l.pts[0].x, X0 + (2.0 / 8.0) * (l.x1 - X0));
        assert_eq!(l.x_labels.len(), 3);
    }

    /// メイン多数 × 体重 1 点。頂点 1 個の `polyline` は何も描かれないので丸を出す。
    #[test]
    fn a_single_weight_point_falls_back_to_a_dot() {
        let l = layout(&metric(), &[(d(2026, 8, 4), 70.0)]);
        let w = l.weight.expect("体重レイヤがある");
        assert!(w.dot.is_some(), "1 点なら丸を描く");

        let many = layout(&metric(), &daily(d(2026, 8, 1), &[70.0, 70.2, 70.1]));
        assert!(
            many.weight.expect("ある").dot.is_none(),
            "2 点以上なら丸は不要"
        );
    }

    /// `dense` はメイン系列だけで決まる。体重が何点あっても主のドット表示に影響しない。
    #[test]
    fn dense_depends_only_on_the_metric_series() {
        let l = layout(&metric(), &daily(d(2026, 1, 1), &[70.0; 300]));
        assert!(!l.dense, "メインは 3 点なので密ではない");

        let many_metric = daily(d(2026, 1, 1), &[600.0; DENSE_POINTS + 1]);
        assert!(layout(&many_metric, &[]).dense);
    }

    /// ★ 密なときは**描画だけ**週平均に落とし、読み取り欄は実測のまま。
    #[test]
    fn a_dense_weight_series_is_smoothed_for_drawing_but_not_for_the_readout() {
        // 8/2(日) から 70 日ぶん。1 日ごとに 0.1kg ずつ増える
        let start = d(2026, 8, 2);
        let values: Vec<f64> = (0..70).map(|i| 70.0 + f64::from(i) * 0.1).collect();
        let weight = daily(start, &values);
        assert!(weight.len() > WEIGHT_DENSE_POINTS);

        // メインの点は体重の 3 日目に 1 つだけ置く
        let m = vec![(start + TimeDelta::days(2), 600.0)];
        let l = layout(&m, &weight);
        let w = l.weight.as_ref().expect("体重レイヤがある");
        assert!(w.aggregated);
        assert_eq!(w.points, 10, "70 日 = 10 週");

        // 読み取り欄はその日の実測値（70.2）。週平均（70.3）ではない
        let b = &l.bands[0];
        assert_eq!(b.weight, Some(70.2));
        // 週キーと一致しない日なので、線の上には丸を打たない
        assert_eq!(b.w_y, None);
        assert_all_coords_finite(&l);
    }

    /// 集約していないときは、その日の体重が線の上にあるので丸を打てる。
    #[test]
    fn the_cursor_marks_the_weight_line_when_the_day_is_actually_drawn() {
        let weight = daily(d(2026, 8, 1), &[70.0, 70.2, 70.1, 70.4, 70.3, 70.5]);
        let l = layout(&metric(), &weight);
        let b = l
            .bands
            .iter()
            .find(|b| b.date == d(2026, 8, 4))
            .expect("8/4 の帯");
        assert_eq!(b.weight, Some(70.4));
        assert!(b.w_y.is_some());
        assert!(b.value.is_some());
    }

    /// ヒット帯が `[X0, x1]` を隙間なく敷き詰めること（タップの取りこぼしを作らない）。
    #[test]
    fn hit_bands_tile_the_whole_plot_area() {
        for (m, w) in [
            (metric(), daily(d(2026, 7, 30), &[70.0; 11])),
            (metric(), Vec::new()),
            (Vec::new(), daily(d(2026, 8, 1), &[70.0, 70.2, 70.1])),
        ] {
            let l = layout(&m, &w);
            assert!(!l.bands.is_empty());
            assert_eq!(l.bands[0].band_x, X0);
            let mut cursor = X0;
            for b in &l.bands {
                assert!((b.band_x - cursor).abs() < 1e-9, "隙間: {b:?}");
                cursor = b.band_x + b.band_w;
            }
            assert!(
                (cursor - l.x1).abs() < 1e-9,
                "右端まで届く: {cursor} != {}",
                l.x1
            );
        }
    }

    /// ★ メイン指標が空でも体重だけで描く（「常に一緒に見られる」が要件）。
    /// ただし左軸ラベルは出さない（`1 / 0.5 / 0` は体重の目盛りだと誤読される）。
    #[test]
    fn weight_alone_still_draws_but_without_the_left_axis() {
        let weight = daily(d(2026, 8, 1), &[70.0, 70.2, 70.1]);
        let l = layout(&[], &weight);

        assert!(!l.is_empty());
        assert!(l.pts.is_empty());
        assert_eq!(l.y_values, None, "左軸ラベルは出さない");
        assert!(l.weight.is_some());
        // 帯は体重から作られ、読み取り欄はその点の値になる
        assert_eq!(l.bands.len(), 3);
        assert_eq!(l.bands[1].weight, Some(70.2));
        assert_eq!(l.bands[1].value, None);
        assert!(l.bands[1].w_y.is_some());
        assert_all_coords_finite(&l);
    }

    #[test]
    fn the_left_axis_is_present_whenever_the_metric_has_points() {
        let l = layout(&metric(), &daily(d(2026, 8, 1), &[70.0, 70.2]));
        let y = l.y_values.expect("左軸ラベルがある");
        assert_eq!(y[2], 0.0, "下端は 0 起点");
        assert!(y[0] > l.max, "上端は max より上（1.1 倍）");
    }

    /// 右軸ラベルは上から下へ降順で、グリッド線と同じ 3 段に載る。
    #[test]
    fn the_right_axis_labels_run_from_high_to_low() {
        let l = layout(&metric(), &daily(d(2026, 8, 1), &[61.8, 63.1, 62.4]));
        let w = l.weight.expect("体重レイヤがある");
        assert_eq!(w.values, [63.5, 62.5, 61.5]);
        assert_eq!(GRID_Y, [Y0, 75.0, Y1]);
        assert_eq!(w.min, 61.8);
        assert_eq!(w.max, 63.1);
    }

    /// 極端な入力でも座標が壊れないこと。`core::body_weight_series` が弾く値でも
    /// `Chart` の prop 経由なら到達しうるので、レイアウト単体で塞ぐ。
    #[test]
    fn extreme_input_never_produces_non_finite_coordinates() {
        let cases: Vec<(Vec<(NaiveDate, f64)>, Vec<(NaiveDate, f64)>)> = vec![
            (metric(), vec![(d(2026, 8, 1), 3e38), (d(2026, 8, 2), 70.0)]),
            (metric(), daily(d(2026, 8, 1), &[0.0, 0.0, 0.0])),
            (vec![(d(2026, 8, 1), 0.0)], vec![(d(2026, 8, 1), 70.0)]),
            (
                vec![(d(2026, 8, 1), f64::MAX)],
                vec![(d(2026, 8, 1), f64::MAX)],
            ),
            (Vec::new(), vec![(d(2026, 8, 1), 70.0)]),
        ];
        for (m, w) in cases {
            assert_all_coords_finite(&layout(&m, &w));
        }
    }
}
