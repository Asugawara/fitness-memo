# 読み込みと操作は実測して、何も入れないと決めた

- **状態**: 採用
- **日付**: 2026-08-09
- **カテゴリ**: architecture
- **関連**: [Rust + Leptos (CSR) + trunk を採用する](rust-leptos-csr-trunk.md), [GitHub Pages の branch deploy（`release` / `docs`）を使う](../deploy/github-pages-branch-deploy.md), [ブラウザサポートは Safari を基準にし、polyfill を入れない](browser-support-policy.md)

## 背景

Web パフォーマンスの一般的なガイドから、このアプリに当てられる候補が 3 つ挙がった。

1. `optimize-script-priority` — wasm の preload に `fetchpriority="high"` を付ける
2. `interactions-in-complex-layouts` — 種目カード列に `content-visibility: auto` を当ててリフローを封じ込める
3. `identify-inp-causes` — `web-vitals` で INP の内訳を計測して分析基盤へ送る

**入れる前に測る**ことにした。CSR の wasm アプリで「重そうだから」を根拠に手を入れると、効かない変更が恒久的な複雑さとして残る。

## 決定

**3 つとも入れない。**

## 理由

### 計測（Playwright / Chromium / release ビルド / localhost）

読み込み:

| 項目 | 値 |
|---|---|
| LCP | **56ms**（= FCP） |
| DOMContentLoaded / load | 11ms |
| `styles.css` | start 6ms / 4ms / 45KB |
| `fitness-memo-*.js` | start 6ms / 4ms / 46KB |
| `fitness-memo-*_bg.wasm` | start **6ms** / 6ms / **886KB** |

操作（種目カード 8 枚を並べた状態。1 日ぶんとしては多め）:

| 項目 | 値 |
|---|---|
| 重量欄への打鍵 → 次のペイント | median **16.6ms** / p95 18.4ms / max 19ms |
| タブ切替 → 次のペイント | **17.7ms** |

### `fetchpriority="high"` は並べ替える相手がいない

**3 つのリソースが全部 `start: 6ms` で同時に始まっている。** Trunk が出す `<link rel="modulepreload">` と `<link rel="preload" as="fetch" type="application/wasm">` により、wasm は JS のパースを待たずに最初の HTML パースで発見されている。

`fetchpriority` が効くのは**優先度の競合があるとき**に順番を入れ替える場面で、ここには競合が無い（クリティカルなリソースが 3 つあるだけで、後回しにしてよいものが 1 つも無い）。付けても計測上の差が出る余地がない。

### `content-visibility` は封じ込める相手がいない

打鍵もタブ切替も **1 フレーム（16.7ms）以内**に収まっている。この計測は `requestAnimationFrame` 2 回待ちを下限に持つので、実際の作業時間はこれより短い。INP の「良好」の閾値 200ms に対して 1 桁以上の余裕がある。

`content-visibility: auto` はリフローの影響範囲を切るための道具だが、切るべき大きなリフローが観測できない。実運用の 1 日あたりのカード枚数は数枚で、計測はその倍以上を積んでいる。

### `identify-inp-causes` はこのアプリの設計と両立しない

このガイドは `web-vitals` ライブラリを入れ、`navigator.sendBeacon` で分析基盤へ送ることを前提にしている。**このアプリはサーバを持たず、外部へ 1 バイトも送らない**（README の「サーバ通信もアカウント登録も持たない」）。送り先が無い。

加えて `package.json` のランタイム依存は 0 で、配信物に JS ライブラリは 1 つも載っていない（[ブラウザサポートは Safari を基準にし、polyfill を入れない](browser-support-policy.md)）。個人用の記録アプリに RUM の常設計装を入れるのは、得られる情報に対して代償が大きすぎる。**必要になったらローカルで測る**（今回まさにそうした）。

## 結果（トレードオフ）

**localhost の計測なので、ネットワーク遅延は入っていない。** 実回線では 886KB の wasm 転送が支配的になり、LCP は 56ms より大きくなる。ただし**それは優先度の問題ではなくサイズの問題**で、`fetchpriority` では動かない。効くとすれば wasm 自体を小さくする話になり、それは既に `opt-level = "z"` / `lto` / `codegen-units = 1` / `wasm-opt=z` でやっている（[Rust + Leptos (CSR) + trunk を採用する](rust-leptos-csr-trunk.md)）。

> **追記（2026-08-20）**: **この段落は半分しか当たっていなかった。** Lighthouse 12.6.1（mobile / `simulate`）で本番を
> 5 回測ると **92**（中央値 / FCP 1057ms・1.00 / **LCP 3306ms・0.69** / TBT・CLS・SI 満点）で、落ちているのは LCP だけ。
> Lantern の実装（`@paulirish/trace_engine` の
> `models/trace/lantern/metrics/{FirstContentfulPaint,LargestContentfulPaint}.ts`）を読むと理由が構造にある。
>
> 1. 依存グラフのネットワークノードは**終了時刻**で切られる — `node.endTime > cutoffTimestamp` なら除外。
>    LCP の `cutoffTimestamp` は**観測**した LCP の時刻。
> 2. LCP の optimistic / pessimistic は**どちらも wasm を含む**（optimistic が除くのは Low/VeryLow priority の画像だけ）。
> 3. LCP の見積りは**グラフ内ノード終了時刻の最大値**（`getEstimateFromSimulation` の `Math.max(...)`）。
>
> body が空で wasm が mount するまで 1px も描かないので、**観測 LCP は必ず wasm の受信完了より後になり、
> 1MB 超の wasm がまるごと LCP の見積りに載る**。実測で
> `LCP_sim ≒ TTFB(783ms) + グラフ内バイト(499,571B) × 8 ÷ 1,638,400bps` が 84ms の誤差で成立した。
> **「サイズの問題」ではあるが、それ以上に「wasm を待たないと何も描かない」構造の問題である。**
> localhost の上の表（LCP = FCP = 56ms）は、wasm が数 ms で届くためにこの構造をまるごと隠していた。
>
> **wasm を小さくする道は既に閉じている。** 4 つ測って全部落とした
> （release / 物差しは手元の `gzip -9` / 現状は raw 1,124,677・`gzip -9` で 447,526）:
>
> ★ **`gzip -9` は本番の配信量ではない。** GitHub Pages が実際に返す wasm は
>   **455,534B**（`Content-Length`。Lighthouse の `transferSize` はヘッダを含むので 456,159B）で、
>   `gzip -9` より 8,008B（1.8%）緩い。**候補どうしを比べる物差しとしては `gzip -9` で一貫していれば足りるが、
>   絶対値の予測には本番の数字を使う。**
>
> | 候補 | raw | 転送 | 判定 |
> |---|---|---|---|
> | `data-wasm-opt-params="--converge"` | 1,123,551 (−1,126) | 447,714 (**+188**) | 却下。raw は減るが圧縮後に増える |
> | `data-reference-types` | 1,124,677 (±0) | 447,526 (±0) | 却下。wasm は 1 バイトも動かない |
> | `opt-level = "s"` | 1,354,575 (+229,898) | 509,278 (**+61,752**) | 却下 |
> | `data-no-demangle` | — | — | 却下。配信 wasm に `_ZN` は 0 件で name section も無い（削る対象が無い） |
> | `console_error_panic_hook` の cfg gate | 未実施 | | 却下。`src/views/day.rs` の keyed diff が壊れる縁があり、サーバもテレメトリも無いこのアプリでは console のスタックトレースが唯一の手掛かり |
>
> **ビルド設定の次に「コードそのもの」も 3 つ測った。全部落とした。** stable のまま、
> `.into_any()` による view の型消去 / ソートの単型化の畳み込み / `<For>` の見直し
> （baseline は raw 1,124,677・転送 447,526）:
>
> | 候補 | raw | 転送 | 判定 |
> |---|---|---|---|
> | `App` の言語切替境界の `view!` に `.into_any()` | 1,123,974 (−703) | 447,456 (−70) | 却下。呼び手が 1 箇所なので箱に入れても実体は残る |
> | `sort_by_key(\|g\| g.order)` 6 箇所を共有 `fn` 2 本へ畳む | 1,114,420 (**−10,257**) | 446,991 (−535) | 却下。**raw:転送 = 19:1** |
> | `<For>` の見直し | 未実施 | | 却下。上と同じ「重複コード」機構なので転送はやはり動かず、`u32` をキーにしている理由（keyed diff の panic）を崩すリスクだけが残る |
>
> **ここで分かったことが本質。** 単型化の重複を削っても**転送バイトは動かない**。
> 重複コードはほぼ同一バイトの並びなので、**gzip が既にそれを畳んでいる**。
> ソートの畳み込みは raw を 10KB 削ったのに転送は 535B（LCP 換算 −2.6ms）しか動かず、
> 削れたバイトの圧縮率は 94.8% だった。転送を動かすのは**固有の**コードやデータを消すことだけである。
>
> **必要な量が桁で違う。** minify 後の LCP は約 3,191ms（下の表）で、カテゴリ 95 の境界は 3,032ms。
> 159ms 詰めるには転送を **−32,563B** 動かす必要があり、wasm 全体の圧縮率 2.51:1 で
> 固有コードに換算すると約 **−82KB raw** — release wasm の code セクション
> 996,642B（全体の 89.4%。data は 101,003B / 関数 4,762 本）の 8.2% にあたる。
> 依存を 1 つ落としても届かない（chrono + serde_json を合わせて raw 44KB しか無い）。
> **`raw` の表と `転送` の表を混ぜて読まないこと。** Lantern が見るのは転送だけである。
>
> **本番の実配信量（2026-08-21 / `Content-Length`。GitHub Pages / HTTP/2 / gzip）:**
>
> | | minify なし（現状の本番） | minify あり（PR #50 後） |
> |---|---|---|
> | index.html | 6,312 | 約 3,620 |
> | styles-*.css | **24,905** | **約 4,600** |
> | glue JS | 9,170 | 約 8,980 |
> | wasm | 455,534 | 455,534（変わらない） |
> | **LCP グラフ合計** | **495,921** | **約 472,700** |
>
> minify 後の欄は手元の `gzip -9` に本番の 1.8% の緩さを掛けた見積り。**−23,187B ≒ −113ms** で、
> LCP は 3,304ms → 約 3,191ms、カテゴリは 92 → 93 の見込み（**未検証。マージして本番で測ること**）。
>
> **localhost の数字を本番と並べてはいけない。** 手元の `scripts/static-server.mjs` は gzip しないので、
> Lighthouse は wasm を 1,124,677B のまま受け取り、LCP が **+3,602ms** 化けてカテゴリが **76** に落ちる。
> gzip するサーバを立てても本番とは一致しない:
>
> | 台 | LCP グラフ転送 | LCP | カテゴリ |
> |---|---|---|---|
> | 本番（minify なし / HTTP/2 / TLS） | 495,921 | 3,304ms | **92** |
> | localhost + gzip（minify あり / HTTP/1.1 / 平文） | 467,392 | 3,451ms | 91 |
>
> localhost の方が **28,529B 少ない**（バイトだけなら −139ms 有利）のに **+147ms 遅い**。
> 差し引き**約 286ms は台のペナルティ**で、ビルドとは無関係。HTTP/1.1 だとリクエストごとに接続確立の
> RTT が積まれるのを Lantern がそのまま計算するため。**localhost は本番より 1〜2 点低く出る（逆方向にはならない）。**
>
> **だから localhost は A/B にだけ使う。** 同じ台で minify の有無だけを変えると
> 490,484B / LCP 3,601ms / 90 → 467,392B / LCP 3,451ms / **91** で、3 回ずつ回して LCP のぶれは 1〜2ms しかない。
> Lantern はバイト数から決定的に計算するので、**「効いたか」の判定は本番より手元の方が精度が高い**。
> ただし手元の −23,092B は −150ms 動いていて `バイト × 8 ÷ 1,638,400`（−113ms）と合わない。
> **絶対値も換算式も本番で確かめる。**
>
> **代わりに配信バイトを約 23,187B 削った** — [許諾文をコメントの外へ出して trunk の minify を解禁する](minify-with-licenses-outside-comments.md)。
> LCP の見積りは約 113ms 縮む見込みで、カテゴリは 92 → 93 になる**はず**（手元の A/B では
> 90 → 91 と 1 点動いた。本番では未検証）。**それ以上は構造を変えないと動かない。**
>
> **構造を変える道は分かっていて、今回は採らなかった。** 「観測 LCP を wasm の受信完了より前に起こす」＝
> **wasm を待たずに何かを描く**ことができれば、wasm がグラフから外れて LCP は FCP 相当（約 1,050ms）まで落ち、
> カテゴリは 100 になる。具体案は `position: fixed; inset: 0` の out-of-flow なブートスケルトンを `index.html` に置き、
> mount 直後に Rust から外す形。out-of-flow なので追加も削除もレイアウトシフトを生まず CLS は 0 のまま保てる
> （`index.html` が却下しているのは**フロー内**に置く形なので、その判断自体は今も有効）。
> 成立には (a) スケルトン側にアプリの最大テキスト候補（`h1.screen-title` ≒ 3,060px²）より大きい**単一 block の**テキストを置く、
> (b) `<main>` / `<h1>` / `data-testid` / focusable 要素 / `opacity` を置かない、(c) mount と同じタスクで外す、が必要。
> `page.route` で wasm を遅らせ「**最後の** LCP エントリの `startTime` < wasm の `responseEnd`」を固定する E2E で守れる
> （ミリ秒の閾値を 1 つも見ないので、この ADR の「環境差で揺れる計測を常設しない」とも衝突しない）。
> **スコアのために描画の構造を変えるかどうかは、スコアとは別の判断として残してある。**
>
> なお同 ADR が却下した「Lighthouse を入れる」は **npm 依存を増やすこと**への却下なので、
> `npx lighthouse@12.6.1` を手元で 1 回回すのは「必要になったらローカルで測る」の範囲にある（`package.json` は触っていない）。

**Service Worker が入っているので、2 回目以降の起動はそもそもネットワークを踏まない。** ホーム画面から起動する常用形態では、この 886KB はキャッシュから出る。初回だけの話である。

**計測を CI に常設していない。** 数字は上の表に固定してあるだけで、退行したら自動で落ちる仕組みは無い。E2E のパフォーマンス測定は環境差で揺れやすく、`.githooks/pre-commit` が唯一の防波堤である以上、そこに不安定なテストを足すのは割に合わない。**気になったら再計測する**方に倒す。

## 検討した代替案

**とりあえず `fetchpriority="high"` だけ付けておく。** 1 属性なので害が無いように見えるが、Trunk の生成物への後処理（`scripts/stamp-sw.sh` と同じ post_build フック）を 1 つ増やすことになる。効果が測れないものにビルド手順を足さない。

**Lighthouse を入れて総合スコアを見る。** スコアは分かるが、この 3 つの判断に必要な数字（リソースの開始時刻、打鍵からペイントまで）は Playwright で直接取れる。npm のランタイム依存 0 を保つためにも、既にあるツールで足りるなら足すべきでない。
