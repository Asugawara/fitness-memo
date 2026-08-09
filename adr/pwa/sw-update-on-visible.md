# visible 復帰で `reg.update()` を呼ぶ

- **状態**: 採用
- **日付**: 2026-08-08
- **カテゴリ**: pwa
- **関連**: [Service Worker はシェル全体を BUILD_ID で原子的に入れ替える](sw-atomic-shell-swap.md), [`visibilitychange` の hidden で debounce を flush する](../storage/flush-on-visibilitychange.md)

## 背景

Service Worker の更新チェックが起きる契機は仕様で決まっている。

- スコープ内ページへの**ナビゲーション**
- push / sync イベント
- `register()` に渡す URL が変わったとき
- 明示的な `registration.update()`

ここで iOS のホーム画面 PWA の挙動が問題になる。**アプリは再起動されずレジュームされ続けるので、ナビゲーションが発生しない。** アイコンをタップして開いても、前回の状態が復帰するだけでページ遷移は起きない。

このアプリは push も sync も使わない（通信しないので）。`register()` の URL は `./sw.js` で固定である。つまり**更新チェックの契機が 1 つも発生しない**。スワイプでアプリを完全終了しない限り、**何週間も旧版に固定され得る**。

## 決定

**SW 登録後に `visibilitychange` を購読し、visible 復帰のたびに `reg.update()` を呼ぶ。**

```js
navigator.serviceWorker.register('./sw.js').then(reg => {
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'visible') reg.update();
  });
});
```

## 理由

- **visible 復帰が、レジュームされ続ける PWA で唯一定期的に発生する「アプリを開いた」相当のイベントである。** ナビゲーションが起きない環境で更新チェックの契機を作るには、これを使うしかない。
- **`update()` は SW スクリプトの再取得だけを行い、ページには影響しない。** 新しい `sw.js` が見つかれば install → `skipWaiting()` → activate まで進む（[Service Worker はシェル全体を BUILD_ID で原子的に入れ替える](sw-atomic-shell-swap.md)）。新しいシェルは次の起動から使われるので、表示中の画面が突然差し替わることはない。
- **この処理は `index.html` の inline script に置いた。** Rust 側から `ServiceWorkerContainer` を触ると `web-sys` の `Navigator` / `ServiceWorkerContainer` / `ServiceWorkerRegistration` feature が必要になる。[UI 依存を wasm32 の target 別 dependencies に置く](../architecture/wasm-target-scoped-dependencies.md) で feature を手で管理する方針にしたので、JS 3 行で済むものを Rust に持ち込まない。
- **古典的なデッドロックは起きない**ことを確認済み。「`sw.js` 自身が cache-first に捕まって永久に更新されない」という有名な事故は、SW スクリプトの更新フェッチがブラウザの Update ジョブによる内部リクエストであり、Fetch 仕様の service-workers mode `"none"` として **SW の fetch ハンドラを経由しない**ため発生しない。加えて `updateViaCache` の既定 `'imports'` によりメインスクリプトは HTTP キャッシュもバイパスする。GitHub Pages の実測ヘッダは `cache-control: max-age=600` なので、仮にキャッシュを尊重する実装でも遅延は最大 10 分、仕様の 24 時間上限で強制的にネットワークへ行く。

## 結果（トレードオフ）

- **更新の反映は「次回起動」から。** cache-first + `skipWaiting` の帰結として 1 起動遅れる。visible 復帰で `update()` が走り、新しいシェルが install/activate されるが、表示中のページは旧シェルのままである。個人用アプリとして許容する。更新の確認はアプリを完全終了してから再起動する必要がある。
- **アプリを開くたびにネットワークリクエストが 1 本出る。** オフライン前提のアプリで「完全にオフラインで動く」という要件に対して、通信が発生する箇所が生まれる。ただし `update()` の失敗は無視されるので、オフラインでも動作に影響しない（`.then()` にエラーハンドラを付けていないため未処理の rejection がコンソールに出る可能性はある）。
- **`visibilitychange` のリスナーが 2 つになる。** Rust 側（`views/mod.rs`）が同じイベントで flush と日付再評価を行っている（[`visibilitychange` の hidden で debounce を flush する](../storage/flush-on-visibilitychange.md)）。責務が違う（片方は保存とアプリ状態、片方は SW 更新）ので分けたままにしたが、同じイベントに対する処理が 2 ファイルに散っている状態ではある。
- **`reg.update()` の呼び出し頻度に上限を付けていない。** アプリを頻繁に前面/背面に切り替えるとその都度リクエストが出る。ブラウザ側にレート制限があるので実害はないが、明示的な間隔制御はしていない。
- **開発サーバ（ポート 8080）ではこの経路に入らない。** SW を登録しない分岐が先にあるため（[開発サーバ（ポート 8080）では SW を登録しない](no-sw-in-dev.md)）。

## 検討した代替案

**何もしない（仕様の契機に任せる）**: 実装ゼロ。しかしレジュームされ続ける PWA では契機が発生せず、旧版に固定される。更新できないアプリは不具合を修正しても届かないので却下。

**`setInterval` で定期的に `update()` を呼ぶ**: 一定間隔で確実にチェックできる。しかしバックグラウンドではタイマーが凍結されるので、結局「前面に戻ったとき」しか動かない。それなら `visibilitychange` を直接使うほうが正確。却下。

**`updateViaCache: 'none'` を明示する**: 既定の `'imports'` でもメインスクリプトは HTTP キャッシュをバイパスするので効果が重複する。オプションを増やす意味が薄い。採らない。

**画面に「更新があります」バナーを出して即リロードさせる**: 更新が 1 起動遅れる問題が解ける。しかし入力中にリロードを促されるとデータを失う懸念があり（[`visibilitychange` の hidden で debounce を flush する](../storage/flush-on-visibilitychange.md) の debounce と競合する）、UI も増える。「非常にシンプル」の要件から外れるので v1 では入れない。

**`?sw=off` の脱出口だけで運用する**: SW が壊れたときの手動リセットは用意してある（[開発サーバ（ポート 8080）では SW を登録しない](no-sw-in-dev.md)）が、standalone PWA には**アドレスバーが無いのでクエリ付き URL を入力できない**。日常的な更新手段にはなり得ない。
