# グラフの座標計算を `chart_layout` に切り出してテスト可能にする

- **状態**: 採用
- **日付**: 2026-08-09
- **カテゴリ**: architecture
- **関連**: [グラフライブラリを使わず SVG を自前で描く](no-chart-library-hand-rolled-svg.md)（弱点の解消）, [UI 依存を wasm32 の target 別 dependencies に置く](wasm-target-scoped-dependencies.md), [体重を推移グラフの第2軸に常時重ねる](../ux/body-weight-second-axis-always-on.md)

## 背景

[グラフライブラリを使わず SVG を自前で描く](no-chart-library-hand-rolled-svg.md) は自前 SVG を選んだうえで、
結果の節に**自分で弱点を書いている**。

> グラフの見た目は E2E で検証しにくい。`chart.rs` のロジックは `layout()` に集めてあるが、
> wasm32 専用モジュールなので `cargo test` の対象外である。**`layout()` のテストが無いのは
> 弱点で**、`core.rs` に移せばホストでテストできた（座標計算は純粋な関数である）。

[UI 依存を wasm32 の target 別 dependencies に置く](wasm-target-scoped-dependencies.md) により `views` は `#[cfg(target_arch = "wasm32")]`
で gate されているので、`views/chart.rs` にある限りホストのテストからは触れない。

[体重を推移グラフの第2軸に常時重ねる](../ux/body-weight-second-axis-always-on.md) で第2軸を足すにあたり、
座標計算に**失敗すると黙って壊れる**種類の複雑さが入ることになった。

- 体重の帯（`weight_band`）が幅 0 を返すと `(v-lo)/(hi-lo)` が NaN になる
- NaN 座標を含む `points` 属性は SVG のパースエラーで**折れ線が丸ごと描かれない**（例外も出ない）
- X ドメインの合併、密なときの週平均、描画点 1 個のフォールバック、ヒット帯の敷き詰め

どれも「画面に何も出ない / 線が消える」形で失敗するので、E2E では気づきにくい。

## 決定

**`src/chart_layout.rs` を新設し、`layout()` と座標定数をそこへ移す。**
`lib.rs` に **cfg gate 無し**で宣言し、`cargo test` の対象にする。

移設の要点は **`Layout` から文字列整形を剥がすこと**。

```rust
// 移設前（views/chart.rs）— fmt_axis_label / fmt_md を内部で当てていた
grid: Vec<(f64, String)>
x_labels: Vec<(f64, String, &'static str)>

// 移設後（chart_layout.rs）— 数値・日付のまま返す
y_values: Option<[f64; 3]>
x_labels: Vec<(f64, NaiveDate, &'static str)>
```

書式ヘルパ（`fmt_metric` / `fmt_weight`）は `views/mod.rs` にあるので、
`Layout` が文字列を持つ限り `views` から出られない。整形は view 側で当てる。

**データの関数（`body_weight_series` / `aggregate_weekly_avg` / `weight_band`）は `core.rs` に置く。**
`sessions_in` が private なこともあるが、区別は「`Db` を読むのが `core`、画面座標を出すのが `chart_layout`」。

移設は**振る舞いを変えない単独のステップ**として行い、その時点の全テスト
（`cargo test` 88 本 / Playwright 61 本）が通ることを確認してから第2軸を足した。
本数は移設時点の値で、第2軸とその後の修正でどちらも増えている。

## 理由

- **失敗モードがテストの種類を決めた。** 「線が消える」は Playwright でも検出できるが、
  検出できるのは書いたケースだけである。座標が有限であることを極端入力で総当りする
  （`3e38` / `f64::MAX` / 全部 0 / 単一点）のは単体テストの仕事で、実際に
  `extreme_input_never_produces_non_finite_coordinates` として置いた。
- **`core.rs` ではなく新しいファイルにしたのは責務のため。** `core.rs` は 2774 行あり、
  `VIEW_W` / `X0` のような SVG の viewport 定数を「純ロジック」に混ぜるのは筋が悪い。
  `core` は `Db` の知識を持ち、`chart_layout` は画面の寸法の知識を持つ、で分かれる。
- **文字列を剥がしたのは移設のためだが、責務としても改善している。** 「1,080」や「8/8」は
  表示の都合で、座標計算が知る必要はない。`fmt_axis_label` の桁数短縮ロジックが view 側に
  残ったのも、あれが viewBox からの溢れ（描画の問題）への対処だから一貫している。
- **移設と機能追加を分けたのは切り分けのため。** まとめてやると、回帰が出たときに
  「移設で壊れたのか、第2軸で壊れたのか」が分からない。

## 結果（トレードオフ）

**`views/chart.rs` と `chart_layout.rs` の間に往復が生まれた。** 座標は `chart_layout` が出し、
ラベルの文字列は `chart.rs` が当てるので、1 つの軸ラベルを描くのに 2 ファイルを見る必要がある。
`Layout` が完成した文字列を持っていた頃より、読むときの視線移動は増えた。

**`Pt` / `Layout` / `Band` / `WeightLayer` が全部 `pub` になった。** テストと view の両方から
触るため。モジュール内に閉じていた頃より、フィールドを変えたときの影響範囲が見えにくい。

**ホストビルドのコンパイル対象が増えた。** `chart_layout` は依存が `chrono` と `core` だけなので
実測できる差は無いが、「ロジック層は薄く」という前提には 1 モジュール分近づいた。

**E2E が不要になったわけではない。** SVG 属性の綴り（`viewBox` を `view_box` と書くと
黙って無視される）や CSS の詳細度は単体テストでは捕まらない。座標の正しさは `cargo test`、
描画されているかは Playwright、という役割分担になった。

## 検討した代替案

**`core.rs` に移す（[グラフライブラリを使わず SVG を自前で描く](no-chart-library-hand-rolled-svg.md) の記述どおり）。** ファイルが増えない。しかし 2774 行にさらに
足すことになり、`Db` を知らない画面寸法の定数が「純ロジック」の中に混ざる。却下。

**`views/chart.rs` に置いたまま、`#[cfg(test)]` だけホスト向けに通す。** 移設不要。
しかし `views` モジュール全体が wasm32 gate されているので、`chart.rs` だけ通すには
gate を分解することになり、[UI 依存を wasm32 の target 別 dependencies に置く](wasm-target-scoped-dependencies.md) の
「ホストビルドが leptos の依存グラフを引かない」が崩れる。却下。

**`wasm-bindgen-test` を導入してブラウザ上で単体テストする。** 移設せずに `layout()` を
テストできる。しかしヘッドレスブラウザを回す実行基盤が増え、
[CI を `.githooks/pre-commit` で回す](../deploy/ci-in-pre-commit.md) の pre-commit で回すには重い。
座標計算は純粋な関数なのでブラウザは要らない。却下。

**移設せず、第2軸を E2E だけで検証する。** 差分が最小。しかし NaN 座標のような
「黙って線が消える」失敗を、書いたケースの外で捕まえられない。
[グラフライブラリを使わず SVG を自前で描く](no-chart-library-hand-rolled-svg.md) が弱点だと認めた状態を、
より複雑になった状態で維持することになる。却下。
