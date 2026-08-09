# アイコンに lucide を採り、`assets/icons/*.svg` を `include_str!` で埋め込む

- **状態**: 採用
- **日付**: 2026-08-09
- **カテゴリ**: architecture
- **関連**: [グラフライブラリを使わず SVG を自前で描く](no-chart-library-hand-rolled-svg.md), [Service Worker はシェル全体を BUILD_ID で原子的に入れ替える](../pwa/sw-atomic-shell-swap.md), [ヘルプの図を `assets/` の SVG に置き `include_str!` + `inner_html` で挿す](help-figures-as-included-svg.md), [`color-scheme` を宣言し、クラスなしの `<button>` を作らない](../ux/declare-color-scheme-for-ua-widgets.md), [種目タブを部位の折りたたみ一覧にし、1 つだけ開く](../ux/menu-groups-as-single-open-accordion.md)

## 背景

これまでアイコンは**全部 Unicode のグリフをボタンのテキストとして置いていた**。`✕`（閉じる、7 箇所）、`↑` `↓`（並び替え、4 箇所）、`‹` `›`（カレンダーの月移動、2 箇所）である。共通のコンポーネントもヘルパも無く、`<button class="icon-btn" aria-label="…">"✕"</button>` という定型をコピーしていた。

[種目タブを部位の折りたたみ一覧にし、1 つだけ開く](../ux/menu-groups-as-single-open-accordion.md) で種目タブに**展開シェブロンと編集の鉛筆**が要ることになった。どちらも Unicode で代用できるグリフが無い（`✎` U+270E は端末によって絵文字に転ぶ）。ここでアイコンの調達方法を決める必要が出た。

一方、図の埋め込みは [ヘルプの図を `assets/` の SVG に置き `include_str!` + `inner_html` で挿す](help-figures-as-included-svg.md) が既に決めている。`assets/help/*.svg` を `include_str!` して `inner_html` で挿し、色は `public/styles.css` のクラスで与える形である。**この機構をアイコンにも広げられるかが論点**になった。

## 決定

**lucide（<https://lucide.dev>）のアイコンを使い、[ヘルプの図を `assets/` の SVG に置き `include_str!` + `inner_html` で挿す](help-figures-as-included-svg.md) と同じ機構で埋め込む。`src/views/icon.rs` に定数とラッパ関数を集める。**

```rust
pub const CHEVRON_RIGHT: &str = include_str!("../../assets/icons/chevron-right.svg");
// …

pub fn icon(svg: &'static str) -> impl IntoView {
    view! { <span class="icon" aria-hidden="true" inner_html=svg /> }
}
```

```css
.icon { display: inline-flex; flex: none; width: 20px; height: 20px }
.icon > svg {
  display: block; width: 100%; height: 100%;
  fill: none; stroke: currentColor; stroke-width: 2;
  stroke-linecap: round; stroke-linejoin: round;
}
```

[ヘルプの図を `assets/` の SVG に置き `include_str!` + `inner_html` で挿す](help-figures-as-included-svg.md) からの差分を 5 つ置いた。

- **lucide 既定の属性を全部剥がす。** 上流の SVG は `width="24" height="24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"` を持つ。ファイルに残すのは `viewBox` とパスの座標だけにして、残りは `.icon > svg` へ移す
- **`role="img"` / `aria-label` を付けず、`aria-hidden="true"` のラッパに入れる。** アイコンは必ず `aria-label` を持つボタンの中に置くので、名前はボタン側が持つ
- **`chevron-down` を持たず、`chevron-right` を CSS で 90 度回す**（[種目タブを部位の折りたたみ一覧にし、1 つだけ開く](../ux/menu-groups-as-single-open-accordion.md) の `aria-expanded` フック）
- **ラッパを関数にする。** `<span class="icon" aria-hidden="true" inner_html=X />` を 11 箇所に手書きしない
- **ライセンス表記を 2 か所に置く。** リポジトリには `assets/icons/LICENSE`、**配信物には `index.html` の HTML コメントに全文**。lucide は ISC、ただし `chevron-left` / `chevron-right` / `x` は Feather 由来で MIT も掛かるので両方を収録した

置き換えたのは**アイコンだけのボタン**（`.icon-btn` かつ `aria-label` を持つもの）に限る。収録は `chevron-left` / `chevron-right` / `pencil` / `x` の 4 枚。

## 理由

- **npm 依存を入れない前提を崩さずに済む。** `package.json` の依存は `@playwright/test` だけで、`index.html` に CDN 参照は 1 つも無い。lucide は SVG ファイルの集まりなので、必要な 4 枚をコピーすれば足りる。`lucide` パッケージや Web Components を入れると、この 2 つの前提が同時に壊れる。
- **`assets/` に置けば Service Worker のプリキャッシュ一覧が増えない。** `scripts/stamp-sw.sh` は `$TRUNK_STAGING_DIR` の中身からシェル一覧を作るので、`public/icons/` に置くと `cache.addAll` のエントリが 4 つ増える。これは**1 つでも失敗すると install 全体が落ちる**原子的操作（[Service Worker はシェル全体を BUILD_ID で原子的に入れ替える](../pwa/sw-atomic-shell-swap.md)）なので、エントリを増やすのは失敗確率を上げることを意味する。`assets/` は `index.html` の `data-trunk` 宣言に無いため `dist/` に入らず、SVG は wasm の中に文字列として乗るだけになる。
- **ただしその配置は、ライセンス表記を配信物から締め出す。** `assets/icons/LICENSE` は `dist/` に入らない一方、lucide のパスデータは `include_str!` で wasm に焼き込まれて配信される。`scripts/release.sh` は `dist/` をそのまま `docs/` へコピーするので、公開ページの利用者は **ISC / MIT の成果物を著作権表示なしで受け取る**ことになる。両ライセンスは「全ての複製に著作権表示と許諾文を添える」ことを求めているので、これは配置の副作用ではなく条件違反である。`index.html` の HTML コメントに全文を置いて解決した（trunk はコメントをそのまま `dist/index.html` へ通す）。**「SW のエントリを増やさない置き方」と「表記を配信する」は両立する**が、片方だけ考えると落ちる。
- **`stroke: currentColor` を CSS 側に置くと、既存のトークンだけでダークに追従する。** `.icon-btn { color: var(--muted) }` がそのまま線色になり、`.grp-toggle { color: var(--text) }` の中に置けば本文色になる。[ヘルプの図を `assets/` の SVG に置き `include_str!` + `inner_html` で挿す](help-figures-as-included-svg.md) が「`fill="var(--accent)"` を書くな」と決めた理由は presentation attribute で `var()` が解決されないことだったが、`currentColor` は解決される。それでも CSS へ寄せたのは、**線の太さと色を 2 箇所で管理しないため**である。上流の `stroke-width="2"` を残すと、20px に縮めた線が太すぎたときに直す場所が 4 ファイルに散る。
- **`aria-hidden` をラッパに固定したのは、付け忘れが名前を壊すから。** アイコンが `aria-hidden` でないと、ボタンのアクセシブル名の計算にパスの中身が混ざる余地が出る。11 箇所に手書きすれば 1 箇所は落ちる。関数にすれば落ちない。既に `menu.rs` の `opt_button` という「小さい部品を関数で出す」先例がある。
- **`chevron-down` を別ファイルにしない。** 回転で足りるものを 2 枚持つと、片方だけ差し替えたときに向きが揃わなくなる。
- **lucide を選んだ理由**は、線幅・端の丸め・グリッドが 1 セットとして揃っていて、`stroke` 系の属性を CSS へ寄せる形と相性が良いこと、ISC / MIT で表示義務が軽いこと、`chevron-right` を回して `chevron-down` にできる程度に幾何が素直なこと。
- **既存の `✕` `‹` `›` も置き換えたのは、同一画面で混在するから。** 種目タブのヘッダが lucide の鉛筆で、そこから開くシートの「閉じる」が Unicode の `✕` だと、線幅も字面も揃わないのが 1 画面の中で見える。`aria-label` と `data-testid` は変えていないので E2E は無傷だった。

## 結果（トレードオフ）

- **release の wasm は 882,138 → 890,130 bytes（+7,992、+0.91%）、gzip 後で 353,486 → 355,213（+1,727、+0.49%）。** SVG 4 枚のテキストは合計 530 bytes（93 / 94 / 230 / 113）なので、**増分の 93% は SVG ではなくコード**である。内訳は `icon.rs` 自体ではなく（定数 4 本と 1 行の関数しかない）、11 箇所に散った `icon(...)` 呼び出しの `view!` 展開が主である。[ヘルプの図を `assets/` の SVG に置き `include_str!` + `inner_html` で挿す](help-figures-as-included-svg.md) が図 4,305 bytes に対して +20,618 bytes を実測し「増加の 8 割は図ではなくコード」と結論したのと同じ傾向で、**wasm が膨らむかどうかは埋め込む SVG の量ではなく、一緒に入る leptos のコード量で決まる**。この 2 回の実測で言えるのはそこまでで、「SVG を埋め込むと重い」は 2 回とも当たっていない。

  ただしこの数字は [種目タブを部位の折りたたみ一覧にし、1 つだけ開く](../ux/menu-groups-as-single-open-accordion.md) の折りたたみ機構（`open_group` の配線と `GroupBlock` の再構成）を含み、逆に `move_exercise` / `move_group` / `swap_neighbor` と 4 つのボタンが消えた分を差し引いた**正味**である。lucide 単独の増分ではない。[ヘルプの図を `assets/` の SVG に置き `include_str!` + `inner_html` で挿す](help-figures-as-included-svg.md) では図 4,305 bytes に対して wasm が 20,618 bytes 増え、その 8 割がコード（`views::help` のコンポーネント 3 つ + serde derive）だった。今回は `icon.rs` が定数 4 本と 1 行の関数しか持たず、置き換え先は既存のボタンなので、コード側の増分がほぼ無い。**「SVG を埋め込むと膨らむ」は今回は当たらない**（膨らむかどうかは図の量ではなく一緒に入るコードの量で決まる、というのが 2 回の実測から言えること）。
- **SVG ファイル単体をブラウザで開いても何も見えない。** `fill` も `stroke` も CSS に寄せたので、単独で開くと塗りも線も無い。[ヘルプの図を `assets/` の SVG に置き `include_str!` + `inner_html` で挿す](help-figures-as-included-svg.md) の図が「色が付かない」だったのに対し、こちらは**形すら見えない**。確認はアプリを起動して行うしかない。
- **XML 宣言の罠を 2 度目に踏む位置に入った。** `<?xml version="1.0"?>` が混ざると HTML フラグメントパーサが bogus comment にして**エラーも出さずにアイコンが 1 つも出ない**。上流の lucide は付けていないが、手で編集するとエディタが足すことがある。E2E で `.icon > svg` の個数を固定した（`group-toggle` が 6、`group-edit` が 6、`menu-sheet-close` が 1、`cal-prev` / `cal-next` が各 1）。
- **`.icon-btn` に `display: inline-flex` が必要になった。** 中身がテキストのグリフだった頃は UA ボタンの `text-align: center` で中央に来ていたが、20px の `<span>` を入れると baseline 配置でずれる。既存の 13 箇所すべてに効く変更だが、グリフを入れていた頃と見え方は変わらない。
- **ライセンス表記が 2 か所になった。** `assets/icons/LICENSE`（リポジトリを読む人向け）と `index.html` のコメント（配信物）。アイコンを増やして出典が変わったら**両方**直す必要がある。1 か所にできなかったのは、リポジトリで読みたい場所と配信に載る場所が違うため。`index.html` のコメントは約 2.4KB で、SW のシェルに含まれるが `cache.addAll` のエントリは増えない（`index.html` は元から入っている）。
- **アイコンを足すたびにファイルが増える。** 上流からコピーして属性を剥がす手作業が要る（自動化していない）。4 枚のうちに留めているので許容したが、10 枚を超えたらスプライトか生成スクリプトを検討する。
- **上流の更新に追従しない。** lucide の `pencil` が描き直されてもこちらは古いままである。アイコンの見た目が勝手に変わらないという利点でもある。
- **Unicode グリフが一部残った。意図的である。**
  - `＋ 部位を追加` / `＋ 種目を追加`（`menu.rs`）、`+ セット`（`day.rs`）、`＋/－ コンディション`（`day.rs`）── アイコンボタンではなく**文字ラベルの飾り**。lucide にすると `<span class=icon>` + テキストの組み合わせになり `.link-btn` のレイアウト見直しが要る
  - `追加のしかた ›`（`help.rs`）── 同上。加えて [ホーム画面への追加の案内を記録タブ末尾のバナー + 手順シートにする](../ux/install-guide-banner-and-sheet.md) が `aria-hidden` を付けないことを含めて調整済みの箇所
  - `×`（`day.rs` の「60×10」）── これは乗算記号でアイコンではない
  
  やり残しではないので、次に触る人が無関係な差分を混ぜないようここに列挙しておく。
- **全角 `＋` と半角 `+` が混在したままである。** `menu.rs` が `＋`、`day.rs` が `+`。今回の範囲外だが、上の列挙を作る過程で見つかったので記録しておく。

## 検討した代替案

**Unicode グリフのままシェブロンと鉛筆を探す**: ファイルもコードも増えない。しかし鉛筆に使える文字は `✎` U+270E / `✏` U+270F しかなく、後者は多くの環境で絵文字（カラー）に転ぶ。前者も線幅がフォント任せで、隣の `✕` と揃わない。字形が端末のフォントに依存する時点で、44px ボタンの中身として制御できない。却下。

**`view!` マクロにインライン SVG を直書きする**: ファイルが増えず Rust の中で完結する。しかし leptos_macro の SVG 属性は無検査で `setAttribute` されるので、`view_box` / `stroke_width` のような綴り違いが**コンパイルエラーにならず実行時に黙って無視される**（`views::chart` の冒頭に警告として残っている罠）。[ヘルプの図を `assets/` の SVG に置き `include_str!` + `inner_html` で挿す](help-figures-as-included-svg.md) が同じ理由で却下している。却下。

**`public/icons/` に置いて `<img src>` で参照する**: trunk の `copy-dir` に 1 行足すだけで済む。しかし外部参照された SVG は別ドキュメントなので `currentColor` が届かず、ダークで線が見えなくなる。加えて SW のプリキャッシュ一覧が 4 エントリ増える。却下（[ヘルプの図を `assets/` の SVG に置き `include_str!` + `inner_html` で挿す](help-figures-as-included-svg.md) と同じ結論）。

**SVG スプライト（`<symbol>` + `<use>`）を 1 枚置く**: アイコンが増えても要素が 1 つで済み、同じアイコンを複数回使っても DOM が軽い。しかし 4 枚では利点が出ず、スプライトを `index.html` に埋めるか別ファイルにするかで [Service Worker はシェル全体を BUILD_ID で原子的に入れ替える](../pwa/sw-atomic-shell-swap.md) の話に戻る。10 枚を超えたら再検討する。却下（保留）。

**アイコンフォント（Material Icons など）を使う**: 実装は最小で、文字として置ける。しかし CDN か self-host のフォントファイルが要り、`index.html` の「外部参照ゼロ」と SW のプリキャッシュ一覧に同時に触る。FOIT / FOUT でアイコンが四角になる瞬間も出る。却下。

**`lucide` の npm パッケージを入れて build 時に切り出す**: 上流の更新に追従でき、アイコンを足すのもコマンド 1 つで済む。しかし依存が `@playwright/test` だけという前提が崩れ、ビルドに Node のステップが増える（現在 trunk の post_build hook は `stamp-sw.sh` だけ）。4 枚のためにビルドパイプラインを足すのは釣り合わない。却下。
