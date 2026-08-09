# 開発サーバ（ポート 8080）では SW を登録しない

- **状態**: 採用
- **日付**: 2026-08-08
- **カテゴリ**: pwa
- **関連**: [Service Worker はシェル全体を BUILD_ID で原子的に入れ替える](sw-atomic-shell-swap.md), [visible 復帰で `reg.update()` を呼ぶ](sw-update-on-visible.md)

## 背景

Service Worker は cache-first でシェル全体を返す（[Service Worker はシェル全体を BUILD_ID で原子的に入れ替える](sw-atomic-shell-swap.md)）。開発中に `trunk serve` でこれが動くと、ソースを変えてリロードしても**キャッシュされた旧シェルが返る**。しかも trunk のライブリロードが挿入するスクリプトも SW 経由になるため、「変更が反映されない」「ライブリロードが効かない」が同時に起きる。

原因が SW だと気づくまでに時間を溶かしやすく、しかも一度登録された SW は同じオリジン（`localhost`）に残り続けるので、後で別のプロジェクトを `localhost` で開いたときにも影響しうる。

## 決定

**開発サーバのポート（8080）では SW を登録せず、既存の登録を解除する。** さらに `?sw=off` の脱出口を用意する。

```js
if ('serviceWorker' in navigator) {
  if (location.port === '8080') {                     // trunk serve では SW を使わない
    navigator.serviceWorker.getRegistrations().then(rs => rs.forEach(r => r.unregister()));
  } else if (location.search.includes('sw=off')) {    // 脱出口
    Promise.all([
      navigator.serviceWorker.getRegistrations().then(rs => Promise.all(rs.map(r => r.unregister()))),
      caches.keys().then(ks => Promise.all(
        ks.filter(k => k.startsWith('fitness-memo-')).map(k => caches.delete(k)))),
    ]).then(() => location.replace('./'));
  } else {
    navigator.serviceWorker.register('./sw.js').then(/* … */);
  }
}
```

この判定が成立するよう **`Trunk.toml` に `[serve] port = 8080` を明示コミットして固定する。**

## 理由

- **開発中の cache-first 事故を構造的に避ける。** 「開発では SW を使わない」は PWA の定番だが、判定方法を決めておく必要がある。`location.port` は追加の設定なしに読めて、trunk serve と本番（GitHub Pages / ポート 443）を確実に区別できる。
- **`unregister()` も一緒に呼ぶのが重要である。** 判定を入れる前に一度でも SW を登録してしまうと、以後 `localhost:8080` は登録済みの SW に制御され続ける。登録しないだけでは既存の登録が残るので、明示的に解除する。
- **`?sw=off` は standalone PWA が壊れたときの最後の手段である。** SW が壊れた状態で cache-first だと、修正版をデプロイしても届かない可能性がある。クエリを付けて開けば SW とキャッシュを消して素の状態に戻せる。
- **`?sw=off` の後始末が終わってから自力でリロードする。** `unregister()` は**現在ページの controller を外さない**ので、そのままでは 1 回目のアクセスで解除されても表示は SW 経由のままになる。`Promise.all` を待って `location.replace('./')` するので、1 回のアクセスで完了する。standalone にはアドレスバーもリロード UI も無いため、手動リロードを前提にできない。
- **キャッシュ削除は `fitness-memo-` prefix に絞る。** `caches.keys()` はオリジン全体（`asugawara.github.io`）を返すので、無差別に消すと同じアカウントの他プロジェクトの Pages サイトを壊す（[Service Worker はシェル全体を BUILD_ID で原子的に入れ替える](sw-atomic-shell-swap.md) の activate と同じ理由）。
- **Phase 4（PWA 化）を画面完成後に置いた**のも同じ動機である。開発期間の大半を SW 無しで進めることで、この事故の窓自体を小さくした。

## 結果（トレードオフ）

- **`Trunk.toml` の `port = 8080` が SW の挙動と結合している。** ポートを変えると本番判定に落ちて SW が登録され、開発中に cache-first 事故が起きる。`Trunk.toml` にコメントで警告を残したが、**設定ファイルとアプリコードにまたがる暗黙の契約**であることは弱点である。
- **`trunk serve` では SW / オフライン動作を一切確認できない。** 確認するには `trunk build` してから別の静的サーバ（`scripts/static-server.mjs`、ポート 4173）で配る必要がある。手数が 1 つ増える。
- **軽い側 E2E（ポート 4173）では SW が登録される。** つまり `smoke.spec.mjs` のリロード検証はキャッシュ越しになる。[`visibilitychange` の hidden で debounce を flush する](../storage/flush-on-visibilitychange.md) の flush 検証は `localStorage` の話なので影響しないが、「リロードして新しい HTML が来ること」を前提にしたテストを書くと SW に阻まれる。認識したうえで、SW 自体の検証は重い側（`pwa.spec.mjs`）に寄せた。
- **`?sw=off` を standalone から実行する方法が実質ない。** アドレスバーが無いので、Safari で同じ URL を開いて実行することになる。ところが iOS では Safari と standalone で SW / キャッシュが別物なので（[JSON エクスポート/インポートを v1 に入れない](../storage/defer-export-import.md)）、**Safari 側で `?sw=off` しても standalone 側は直らない**。standalone の復旧は「ホーム画面から削除して再追加」が現実的な手段になる。脱出口は主にデスクトップと E2E 用である。
- ポート判定なので、`trunk serve --port` を変えたり別のポートで静的配信すると意図しない側に落ちる。

## 検討した代替案

**`cfg!(debug_assertions)` で Rust 側から切り替える**: ビルドプロファイルで判定できるので設定ファイルとの結合が消える。しかし SW 登録は `index.html` の inline script にあり（`web-sys` の feature を増やさないため。[UI 依存を wasm32 の target 別 dependencies に置く](../architecture/wasm-target-scoped-dependencies.md)）、Rust から JS に値を渡す仕組みが必要になる。得るものに対して機構が増える。却下。

**`location.hostname === 'localhost'` で判定する**: ポート設定に依存しなくなる。しかし軽い側 E2E（`localhost:4173`）でも SW が無効になり、**SW の検証が localhost で一切できなくなる**。オフライン起動の検証は localhost で行いたいので却下。

**`trunk serve` でも SW を登録し、開発時は DevTools の「Bypass for network」を使う**: 設定が要らない。しかし毎回の手動操作に依存し、忘れた瞬間に事故が起きる。しかも DevTools を開いていない状態では効かない。却下。

**`stamp-sw.sh` を debug ビルドでは走らせない**: `sw.js` が置換されないので SHELL が空になり、install が成功して**何もキャッシュしない SW** ができる。動くが「SW はあるが空」という中間状態は理解しにくい。登録しないほうが明快。却下。
