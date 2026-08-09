# Service Worker はシェル全体を BUILD_ID で原子的に入れ替える

- **状態**: 採用
- **日付**: 2026-08-08
- **カテゴリ**: pwa
- **関連**: [fetch ハンドラで navigate を明示分岐する](sw-explicit-navigate-branch.md), [visible 復帰で `reg.update()` を呼ぶ](sw-update-on-visible.md)

## 背景

要件は「完全にオフラインでも動作する」。Service Worker でアプリシェル（index.html / js / wasm / css / manifest / icons）をキャッシュする必要がある。

trunk は既定で `filehash = true`（js/wasm の名前に内容ハッシュが入る）かつ SRI 有効（`integrity` 属性が付く）でビルドする。つまり **index.html が参照するファイル名はビルドごとに変わる**。

## 決定

1. `scripts/stamp-sw.sh`（trunk の `post_build` フック）が **staging の実ファイルから** precache 一覧と `BUILD_ID` を生成する
2. キャッシュ名は `fitness-memo-${BUILD_ID}`
3. `install` で `cache.addAll(SHELL)`（`{cache: 'reload'}` で HTTP キャッシュを迂回）→ `skipWaiting()`
4. `activate` で **`fitness-memo-` prefix を持つ古いキャッシュだけ**削除 → `clients.claim()`
5. `fetch` は**そのビルドのキャッシュからのみ**返し、**ミスした応答を `cache.put` しない**

## 理由

### なぜ一覧をスクリプトで生成するか

precache 一覧を手書きすると `filehash` によるファイル名変更に追従できず、デプロイのたびに壊れる。staging を走査すれば `filehash` の真偽にもファイル名規則にも依存しない。

`post_build` は「Step 5（HTML を staging に書く）後・Step 6（dist 置換）前」に走る（trunk の hooks ガイドとソースで確認）ので、触るべきは `TRUNK_STAGING_DIR` であって `TRUNK_DIST_DIR` ではない。`sw.js` は `copy-file` の不透明ブロブなので minify も SRI も対象外で、置換が壊れる順序問題はない。

`find` の列挙順はファイルシステム依存なので **`LC_ALL=C sort` を必ず挟む**。挟まないと同一内容でも `BUILD_ID` が変わり、全クライアントに無駄なフル再ダウンロードが起きる。ハッシュ入力には `public/sw.js`（テンプレート）と `scripts/stamp-sw.sh` 自身も含める。含めないと SW のロジックだけ変えたときにキャッシュ名が変わらず、新旧ロジックが同じキャッシュを共有してしまう。

### なぜ `cache.put` しないか

「ミス時はネットワーク → 成功なら cache に格納」は一見自然だが、GitHub Pages が全ファイルに `cache-control: max-age=600` を返す（実測）ため次の順で壊れる。

1. v2 をデプロイ。10 分以内にユーザーが起動する
2. `sw.js` の更新チェックは HTTP キャッシュをバイパスするので **v2 の SW が入り、activate が v1 キャッシュを削除する**
3. ナビゲーション `/fitness-memo/` は v2 キャッシュでミス → `fetch()` が **HTTP キャッシュから v1 の index.html** を返す
4. それが v2 の BUILD_ID キャッシュに `/fitness-memo/` キーで**保存される**
5. v1 の index.html は旧ハッシュ名の js/wasm を参照するが、v2 キャッシュにも Pages 上にも存在しない → **白画面**
6. しかもキャッシュ名は v2 なので activate でも消えず、**次のデプロイまで固定化する**

precache 済みのものだけを返し、ミスは素通しする（格納しない）ことで、「BUILD_ID でシェル全体を原子的に入れ替える」が初めて構造的な保証になる。同一ビルドのファイルしかキャッシュに入らないので、名前も integrity も常に整合する。

### なぜ prefix を絞って削除するか

Cache Storage API は**オリジン単位**で共有される。SW の scope が `/fitness-memo/` でも `caches.keys()` はオリジン全体のキャッシュを返す。`caches.keys()` を無差別に削除すると、同じオリジンに同居する他サイトのキャッシュを黙って破壊する。localStorage 側は `fitness-memo/v1` と prefix を切っているのに、キャッシュ側だけ無差別というのは非対称でもある。

## 結果（トレードオフ）

- 更新の反映は「次回起動」から（cache-first の宿命）。表示中のページは旧シェルのまま。個人用アプリとして許容する。更新の確認はアプリを完全終了してから再起動する必要がある。
- `filehash` を既定の `true` のままにできる。immutable な URL は HTTP キャッシュとの相性がよく、`false` にしたときの「旧キャッシュの本文と新 index.html の integrity が不一致」という別の壊れ方も避けられる。
- ミスを格納しないので、precache 一覧に漏れがあるとオフラインで取れない。一覧を staging 全走査で作っているので漏れは構造的に起きない。

## 検討した代替案

**stale-while-revalidate**: 更新が 1 起動早く届くが、js と wasm が別々のタイミングで更新されて「新しい js + 古い wasm」という不整合を作りうる。BUILD_ID 一括入れ替えのほうが安全。

**`filehash = false` にして固定名にする**: precache 一覧を手書きできるが、SRI の integrity は内容ハッシュなので「旧キャッシュの本文 vs 新 index.html の integrity」の不一致が起きうる。採らない。
