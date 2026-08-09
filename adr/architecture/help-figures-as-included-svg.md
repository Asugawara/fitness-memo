# ヘルプの図を `assets/` の SVG に置き `include_str!` + `inner_html` で挿す

- **状態**: 採用
- **日付**: 2026-08-08
- **カテゴリ**: architecture
- **関連**: [グラフライブラリを使わず SVG を自前で描く](no-chart-library-hand-rolled-svg.md), [Service Worker はシェル全体を BUILD_ID で原子的に入れ替える](../pwa/sw-atomic-shell-swap.md), [ホーム画面への追加の案内を記録タブ末尾のバナー + 手順シートにする](../ux/install-guide-banner-and-sheet.md)

## 背景

[ホーム画面への追加の案内を記録タブ末尾のバナー + 手順シートにする](../ux/install-guide-banner-and-sheet.md) の手順シートに、iOS の「ホーム画面に追加」を 3 ステップの図で見せる。「共有ボタンを押す」「ホーム画面に追加を選ぶ」「右上の追加を押す」の 3 枚である。

実機スクリーンショットは使えない。iOS の共有シートは最上段に AirDrop の連絡先候補を出すので、そのまま公開サイトに載せると連絡先が公開される。ライト／ダーク両対応（`prefers-color-scheme`）なのでスクショの明暗もどちらかとずれる。

したがって図は SVG の模式図になる。問題はその SVG をどうやってアプリに載せるかである。

## 決定

**`assets/help/*.svg` に置き、`include_str!` で埋め込んで `inner_html` で挿す。色は `public/styles.css` の `.hlp-*` クラスで与える。**

```rust
const STEP1_SVG: &str = include_str!("../../assets/help/step1-share.svg");
// …
<div class="hlp-fig" inner_html=STEP1_SVG />
```

SVG ファイルには**座標と class 名だけ**を置く。

```xml
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 128 160" role="img" aria-label="…">
  <rect class="hlp-panel" x="8" y="6" width="112" height="148" rx="14" />
  <circle class="hlp-mark" cx="64" cy="136" r="14" />
</svg>
```

規則を 4 つ置いた。

- **`viewBox` だけを持ち `width` / `height` は書かない。** レスポンシブは `.hlp-fig > svg { display: block; width: 100%; height: auto }` で `<svg>` 自身に効かせる
- **`<?xml … ?>` と DOCTYPE を書かない**
- **`fill="var(--accent)"` を書かない。** presentation attribute では `var()` が解決されない
- **`role="img"` + `aria-label` を使い `<title>` は使わない**（[グラフライブラリを使わず SVG を自前で描く](no-chart-library-hand-rolled-svg.md) と同じ）

## 理由

- **`<img src="…svg">` はダークモードで破綻する。** 外部参照された SVG は別ドキュメントなので、`public/styles.css` の CSS 変数も `currentColor` も届かない。ライト用の色を焼き込むとダークで見えなくなる。図の中身は「端末の面 = `--surface`」「枠 = `--line`」「ハイライト = `--accent`」というトークンそのものなので、テーマ追随は必須である。
- **`innerHTML` に入れた `<svg>` は同一文書に入るので CSS がそのまま効く。** Shadow DOM ではないため `.hlp-panel { fill: var(--surface) }` のようなクラスセレクタが素直に当たる。先例は `views::chart` の `.chart-*`（`public/styles.css:687-727`）で、[グラフライブラリを使わず SVG を自前で描く](no-chart-library-hand-rolled-svg.md) が「見た目は極力 CSS クラスに寄せ、SVG 属性には座標だけを置いた」と決めているのと同じ形である。
- **Service Worker のプリキャッシュ一覧を増やさない。** `scripts/stamp-sw.sh` は `$TRUNK_STAGING_DIR` の中身からシェル一覧を作る。`public/` に置いた SVG は配信物になり、`sw.js` の `cache.addAll` に載る。`cache.addAll` は**1 つでも失敗すると install 全体が失敗する原子的操作**（[Service Worker はシェル全体を BUILD_ID で原子的に入れ替える](../pwa/sw-atomic-shell-swap.md)）なので、エントリを増やすことは失敗確率を上げることを意味する。`assets/` は `index.html` の `data-trunk` 宣言に無いので `dist/` に入らず、SVG は wasm の中に文字列として乗るだけになる。
- **`view!` に SVG を直書きしない理由がある。** leptos_macro の SVG 属性は無検査で `setAttribute` されるので、`view_box` / `stroke_width` のような綴り違いが**コンパイルエラーにならず実行時に黙って無視される**。`<title>` は曖昧要素で、親コンテキストが不明だと HTML 要素に解決される。どちらも `views::chart` の冒頭に警告として残っている罠である。`.svg` ファイルなら HTML パーサ経路になり、この 2 つを踏まない。
- **差し替え点が 1 行になる。** 実機スクショを撮る気になったら `inner_html=STEP1_SVG` を `<img>` / `<picture>` に置き換えるだけで済む。図の内容と表示手段が分離している。
- **SVG のテキスト自体は小さい。** 3 枚で 4,305 bytes（1,344 / 1,740 / 1,221）。ただし wasm 全体の増加はこれより大きい（下記の実測を参照）。

## 結果（トレードオフ）

- **wasm の増加は SVG のバイト数より一桁大きい。** 図を入れる前と実測比較すると release の wasm は **667,343 → 687,961 bytes（+20,618、+3.1%）**、gzip 後で **272,705 → 279,729（+7,024、+2.6%）**。SVG のテキストは 4,305 bytes なので、**増加の 8 割は図ではなく `views::help` のコード**（コンポーネント 3 つ + `UiState` の serde derive）である。「図を埋め込んだから増えた」と読み違えないこと。

  debug ビルドでは **31.4 MB → 33.1 MB（+5.4%）**。E2E は debug の wasm を配信するので気にはなるが、`--repeat-each=3` で回しても全件通るので実害は確認できていない。
- **iOS の UI 変更で腐る。** Safari のクロームを描いた図なので、Apple がツールバーや共有シートを動かせば嘘になる（実際 iOS 15 でツールバーが下へ移動している）。緩和として (1) 忠実な再現ではなく**模式図**にして枠・帯・行・ハイライトだけを描く、(2) 図の下に「iPhone を縦向きで使っているときの画面です。iPad では共有ボタンは画面の上のほうにあります」というキャプションを必ず出す、の 2 つを入れた。**腐ったときに黙って間違っている状態を作らない**ことが目的である。
- **SVG ファイル単体をブラウザで開いても色が付かない。** 色を `styles.css` に寄せた当然の帰結で、単独で開くと `var(--*)` が全部未解決になる。確認できるのは座標だけで、見た目はアプリを起動して見るしかない。
- **XML 宣言の罠がある。** Illustrator / Inkscape は既定で `<?xml version="1.0"?>` を吐く。それが混ざると HTML フラグメントパーサが bogus comment にして**図が 1 枚も出ない**（エラーも出ない）。`views::help` の冒頭にコメントで残し、さらに E2E で `.hlp-fig > svg` が 3 個あることを固定した。
- **`inner_html` は子を持たない要素にしか付かない。** `impl` の境界が `HtmlElement<E, At, ()>` なので `<div … inner_html=SVG />` と自己閉じで書く必要がある。子を足そうとするとコンパイルエラーになる（静かに壊れないので害は小さい）。
- **図のクラスが `styles.css` に 9 個増えた。** `.hlp-panel` / `.hlp-line` / `.hlp-ghost` / `.hlp-glyph` / `.hlp-mark` / `.hlp-mark-fill` / `.hlp-label` / `.hlp-label-on` / `.hlp-fig`。SVG 側だけを見ても色が分からず、CSS 側だけを見ても形が分からないので、2 ファイルを往復して読むことになる。
- **`assets/` の役割が 2 つになった。** これまでは README 用スクショとアイコンのマスター（どちらも配信されない素材）だったが、`assets/help/*.svg` は**配信される**（wasm に埋め込まれる形で）。「`assets/` は配信されない」という理解のまま画像を足すと意図せず wasm が膨らむ。

## 検討した代替案

**実機スクリーンショット（PNG / WebP）**: 忠実度は最高で、ユーザーが見る画面とそのまま一致する。しかし (1) 共有シート最上段の AirDrop 連絡先候補をトリミングし忘れると公開サイトに連絡先が出る、(2) ライト／ダークのどちらかと必ず明暗がずれる、(3) 1 枚数十〜数百 KB を 3 枚。却下。ただし差し替え点を 1 行に閉じてあるので、後から採る余地は残した。

**`public/help/*.svg` に置いて `<img src>` で参照する**: 最も素直で、trunk の `copy-dir` に 1 行足すだけ。しかし外部参照された SVG には CSS 変数が届かずダークモードで破綻する。加えて Service Worker のプリキャッシュ一覧が 3 エントリ増え、`cache.addAll` の失敗確率が上がる。却下。

**`view!` マクロにインライン SVG を直書きする**: ファイルが増えず、Rust の中で完結する。しかし属性の綴り違いが実行時に黙って無視される罠と `<title>` の曖昧要素問題を踏む位置に自分から入ることになる。座標が数十個ある静的な図で、その罠を引く確率は `chart.rs` より高い。却下。

**絵文字と番号付きテキストだけの手順（図なし）**: 実装が最小で、腐らず、翻訳もしやすい。しかし「画面の下のまん中」がどこか、共有アイコンがどんな形かは、文字では正確に伝わらない。この案内が必要な人は iOS の共有シートに馴染みがない人なので、そこを言葉に任せると詰む。却下。

**Rust 側で SVG 文字列を組み立てて `inner_html` に渡す**: 図をパラメータ化できる（サイズやハイライト位置を変える等）。しかし今回の 3 枚は完全に静的で、パラメータ化する軸が無い。文字列連結で SVG を組むと diff も読めなくなる。却下。
