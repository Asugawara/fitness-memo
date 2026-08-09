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

**Service Worker が入っているので、2 回目以降の起動はそもそもネットワークを踏まない。** ホーム画面から起動する常用形態では、この 886KB はキャッシュから出る。初回だけの話である。

**計測を CI に常設していない。** 数字は上の表に固定してあるだけで、退行したら自動で落ちる仕組みは無い。E2E のパフォーマンス測定は環境差で揺れやすく、`.githooks/pre-commit` が唯一の防波堤である以上、そこに不安定なテストを足すのは割に合わない。**気になったら再計測する**方に倒す。

## 検討した代替案

**とりあえず `fetchpriority="high"` だけ付けておく。** 1 属性なので害が無いように見えるが、Trunk の生成物への後処理（`scripts/stamp-sw.sh` と同じ post_build フック）を 1 つ増やすことになる。効果が測れないものにビルド手順を足さない。

**Lighthouse を入れて総合スコアを見る。** スコアは分かるが、この 3 つの判断に必要な数字（リソースの開始時刻、打鍵からペイントまで）は Playwright で直接取れる。npm のランタイム依存 0 を保つためにも、既にあるツールで足りるなら足すべきでない。
