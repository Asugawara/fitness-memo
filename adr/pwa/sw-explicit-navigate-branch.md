# fetch ハンドラで navigate を明示分岐する

- **状態**: 採用
- **日付**: 2026-08-08
- **カテゴリ**: pwa
- **関連**: [Service Worker はシェル全体を BUILD_ID で原子的に入れ替える](sw-atomic-shell-swap.md), [manifest の URL を全て相対にする](manifest-relative-urls.md)

## 背景

Service Worker の precache 一覧は `scripts/stamp-sw.sh` が staging のファイル走査から生成する（[Service Worker はシェル全体を BUILD_ID で原子的に入れ替える](sw-atomic-shell-swap.md)）。ファイル走査なので、index.html のキーは **`./index.html`** になる。

一方、実際の起動ナビゲーションの URL は **`https://asugawara.github.io/fitness-memo/`**（トレイリングスラッシュのディレクトリ URL）である。manifest の `start_url` も `./` なので同じ。

つまり素朴な `caches.match(event.request)` は**起動時に必ずミスする**。キャッシュには `/fitness-memo/index.html` があり、リクエストは `/fitness-memo/` だからである。

## 決定

**`fetch` ハンドラでナビゲーションを明示分岐し、`./index.html` を固定キーとして引く。**

```js
const fromCache = key => caches.open(CACHE).then(c => c.match(key)).then(r => r || fetch(e.request));

// "/fitness-memo/"（ディレクトリURL）も "?sw=off" もここで 1 つの key に収束する
e.respondWith(e.request.mode === 'navigate' ? fromCache('./index.html') : fromCache(e.request));
```

**非ナビゲーションの GET には index.html をフォールバックしない。** ミスはそのままネットワークへ素通しする。

## 理由

- **オンラインではミスがネットワークに救われるので、この不具合は気づかれない。** 起動ナビゲーションが常にキャッシュミスしても `fetch()` が成功するため、開発中もオンラインの実機でも正常に見える。**壊れるのはオフライン起動だけ**で、それは要件の中心（「完全オフラインで動く」）である。しかも DevTools の Offline チェックだけでは、Chrome が別の経路で救ってしまうことがある。気づきにくさの度合いが高いので明示分岐で構造的に塞ぐ。
- **SW 内の相対 URL は `self.location` 基準で解決される。** `sw.js` は `…/fitness-memo/sw.js` に配置されるので、`./index.html` は `/fitness-memo/index.html` に解決される。これは precache のキーと一致する。ローカル（`/`）でもサブパス（`/fitness-memo/`）でも同じ 1 行が正しく動くので、[manifest の URL を全て相対にする](manifest-relative-urls.md) の「URL を全て相対にする」と同じ性質に乗っている。
- **`?sw=off` のような検索文字列付きのナビゲーションも同じキーに収束する。** `caches.match` は既定でクエリ文字列を比較するため、素朴な実装だと `?sw=off` で必ずミスする。ナビゲーションを固定キーに畳むことで、`ignoreSearch` オプションを使わずに解決している。
- **非ナビゲーションにフォールバックしてはいけない理由は SRI である。** trunk は既定で SRI（`integrity` 属性）を有効にしてビルドする。js / wasm のリクエストに `text/html`（index.html の中身）を返すと、**integrity 不一致と MIME エラーでモジュール読み込みが即死する**。よくある「全部 index.html を返す SPA フォールバック」をそのまま書くと、キャッシュに漏れがあったときにエラーが「integrity 不一致」という無関係な形で現れ、原因追跡が難しくなる。エラーはそのまま伝播させるほうが診断しやすい。

## 結果（トレードオフ）

- **`fetch` ハンドラに分岐が 1 つ増える。** 1 行だが、意図が分からないと「なぜ navigate だけ特別扱いなのか」が読めない。`sw.js` にコメントを残した。
- **precache 一覧の生成方式に依存している。** `stamp-sw.sh` がファイル走査で `./index.html` を出すという前提の上に成り立つ。もし将来ディレクトリ URL 側をキーにする形に変えたら、この分岐も直す必要がある。
- **ナビゲーションは常に同じ index.html を返す。** [ルーターを使わずタブを enum signal で切り替える](../architecture/no-router-tab-enum-signal.md) でルーターを使わないと決めているので URL は 1 つしかなく、これは正しい。ルーターを導入したら「どのパスでも index.html」というフォールバックの意味に変わり、SRI の問題と改めて向き合うことになる。
- **キャッシュに無いサブリソースはオフラインで単純に失敗する。** ミスを格納しない設計（[Service Worker はシェル全体を BUILD_ID で原子的に入れ替える](sw-atomic-shell-swap.md)）と合わせて、「precache 一覧に漏れがあればオフラインで動かない」が唯一の失敗モードになる。一覧を staging 全走査で作っているので漏れは構造的に起きない。
- 検証は Chromium 系のみで行う。Playwright の WebKit は Service Worker を扱えないため、`pwa.spec.mjs` の SW 系検証は `test.skip(({browserName}) => browserName === 'webkit')` で除外する。iOS の実挙動は実機の機内モード起動でしか確認できない。

## 検討した代替案

**`caches.match(request, { ignoreSearch: true })` だけで済ませる**: クエリ文字列の問題は解けるが、**`/fitness-memo/` と `/fitness-memo/index.html` の不一致は解けない**（パスが違うので）。本質的な問題に効かない。却下。

**precache 一覧にディレクトリ URL（`./`）も足す**: `stamp-sw.sh` が `./` を追加すれば素朴な `match` で当たる。しかし `cache.addAll` が `./` と `./index.html` の**同じ本文を 2 回ダウンロード・2 回格納する**ことになり、`BUILD_ID` の計算とも噛み合わない（ファイル走査の結果に人工的なエントリが混ざる）。却下。

**全ての GET にキャッシュ → index.html フォールバックを付ける（定番の SPA 構成）**: 実装が最短。しかし上記のとおり SRI と MIME で即死する経路を作る。却下。

**`navigationPreload` を使う**: ナビゲーションのネットワーク取得を並行させて高速化できる。しかし iOS Safari の対応が不明で、オフライン前提のアプリでは得るものがない。却下。
