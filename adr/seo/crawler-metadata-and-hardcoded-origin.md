# クローラ向けメタデータを本番 URL のハードコードで持ち、オフラインシェルから外す

- **状態**: 採用
- **日付**: 2026-08-09
- **カテゴリ**: seo
- **関連**: [Rust + Leptos (CSR) + trunk を採用する](../architecture/rust-leptos-csr-trunk.md), [Service Worker はシェル全体を BUILD_ID で原子的に入れ替える](../pwa/sw-atomic-shell-swap.md), [manifest の URL を全て相対にする](../pwa/manifest-relative-urls.md), [GitHub Pages の branch deploy（`release` / `docs`）を使う](../deploy/github-pages-branch-deploy.md)

## 背景

公開先は <https://asugawara.github.io/fitness-memo/> だが、**SEO 資産がリポジトリの全履歴を通じて一度も存在しなかった**。`git log --all -S'name="description"'` は空を返し、`origin/release` の `docs/` に配信されていたのは 11 ファイル（HTML / js / wasm / css / manifest / sw / アイコン 4 枚）だけだった。

結果として次の 2 つが起きていた。

- **SNS / Slack / LINE にリンクを貼ってもカードにならない。** `<title>fitness-memo</title>` の 1 行しか読むものがない
- **検索結果にスニペットが出ない。** CSR（[Rust + Leptos (CSR) + trunk を採用する](../architecture/rust-leptos-csr-trunk.md)）なので配信 HTML の `<body>` にはテキストが 1 文字も無く、`description` も無いので拾える文が存在しない

一方で、この構成には SEO のメタデータを足すときに素直にはいかない事情が 3 つある。

1. **絶対 URL が必要なタグがある。** `canonical` / `og:url` / `og:image` は相対 URL ではスクレイパが解決できない。だがこのアプリはローカル（`localhost:4173/`）と本番（`/fitness-memo/`）の 2 つのベースパスで動き、[manifest の URL を全て相対にする](../pwa/manifest-relative-urls.md) は「相対 URL に統一してベースパスの差を吸収する」と決めている。**trunk の `--public-url` が書き換えるのは trunk 自身が生成した行だけ**なので、手書きの meta は書き換え対象外になる
2. **`public/` に足したものは自動でオフラインシェルに載る。** `scripts/stamp-sw.sh` は `$TRUNK_STAGING_DIR` 配下を `find` で全列挙して Service Worker の precache SHELL と BUILD_ID を作る（[Service Worker はシェル全体を BUILD_ID で原子的に入れ替える](../pwa/sw-atomic-shell-swap.md)）。現状のシェル総量は約 1.0MB（1,022,587 B。wasm 885KB + js 42KB + css 39KB + アイコン 4 枚 47KB + HTML と manifest）
3. **アプリ名が 4 箇所で固定されている。** コミット 4907db3 が `<title>` / `apple-mobile-web-app-title` / manifest の `name` / `short_name` を `fitness-memo` に揃えており、`e2e/pwa.spec.mjs` がそれを回帰テストとして握っている

## 決定

### 1. `canonical` / `og:url` / `og:image` は本番 URL をハードコードする

```html
<link rel="canonical" href="https://asugawara.github.io/fitness-memo/">
<meta property="og:url" content="https://asugawara.github.io/fitness-memo/">
<meta property="og:image" content="https://asugawara.github.io/fitness-memo/og.png">
```

### 2. OGP 画像は SW の SHELL と BUILD_ID の**両方**から外す

`scripts/stamp-sw.sh` の `find` 2 箇所に `! -name og.png` を足す。

### 3. `<title>` は `fitness-memo` のまま据え置き、日本語のキーワードは `description` / `og:title` / JSON-LD が担う

アプリ名 4 点セットには一切触らない。

## 理由

- **canonical / og を読むのは検索エンジンと SNS のスクレイパだけで、どちらも本番しか見に来ない。** ローカルビルドに本番の絶対 URL が焼き込まれていても実害がゼロである。[manifest の URL を全て相対にする](../pwa/manifest-relative-urls.md) が相対を選んだのは **manifest をブラウザがローカルでも本番でも読むから**であり、対象が違う。同じ「絶対 URL 禁止」を機械的に当てはめる場面ではない。
- **置換機構を 2 本目に増やさない。** [manifest の URL を全て相対にする](../pwa/manifest-relative-urls.md) は「置換対象が増えるほど置換漏れで壊れる経路が増える」を理由に manifest の後処理置換を却下している。`stamp-sw.sh` という置換機構がすでに 1 本走っている状況で、SEO のために 2 本目を足すのは同じ理由で割に合わない。
- **ハードコードの副作用として E2E の期待値が環境非依存の定数になる。** `e2e/seo.spec.mjs` は `dist`（`E2E_BASE=/`）でも `dist-release`（`E2E_BASE=/fitness-memo/`）でも同じ文字列を assert でき、重い側 E2E がサブパス配信での正しさを自動的に確認する。
- **OGP 画像はアプリが一度も参照しない。** 取りに来るのはクローラと SNS のスクレイパのサーバだけで、オフラインのユーザーが要求することは原理的にない。参照しないものを precache するのはカテゴリエラーであり、約 1.0MB のシェルに 21KB を恒久的に上乗せするのは、オフライン起動の速さがこの PWA の存在理由であることと矛盾する。
- **BUILD_ID からも外すのは「シェルに入らないものはシェルの同一性にも影響させない」ため。** SHELL だけ外して BUILD_ID に残すと、`og.png` を差し替えたときに**中身が同一のシェル**に対して新しいキャッシュ世代が切られ、全クライアントが約 1.0MB を無駄に再ダウンロードする。除外を片方だけにすると、除外した意味が逆転する。
- **`<title>` を変える必要がない。** 4907db3 が揃えた 4 箇所のうち、**実機のホーム画面に出るアプリ名を決めるのは `short_name` と `apple-mobile-web-app-title` の 2 つだけ**で、`<title>` は検索結果とタブの見出しにしか出ない。つまり日本語化しても実機の見え方は変わらないが、**変えたことで得られるものも `description` / `og:title` / JSON-LD の `alternateName` が既に持っている**。据え置きが最も安く、効果を実測してから再検討できる。
- **`og:title` にだけ日本語の説明を足す。** OGP 画像をアイコンのみにした結果、カードで「何のアプリか」を伝えるのは見出し文だけになった。`og:title` は 4 点セットに含まれないタグなので、`fitness-memo — オフラインで使える筋トレ記録アプリ` としてブランド表記を先頭に残したまま説明を足せる。Google がタイトルリンクを書き換えるときに `og:title` を採ることがあり、その経路でも効く。

## 結果（トレードオフ）

- **ローカルで開いた HTML にも本番の URL が出る。** DevTools で見ると `canonical` が `localhost` を指していないので一瞬混乱する。`index.html` のコメントと `e2e/seo.spec.mjs` にこれが意図であることを書いてある。
- **公開先の URL を変えると 3 箇所を手で直すことになる。** GitHub Pages のサブパスは `deploy` の決定（[GitHub Pages の branch deploy（`release` / `docs`）を使う](../deploy/github-pages-branch-deploy.md)）に紐づいており、変わるとしても稀。`e2e/seo.spec.mjs` の `SITE` 定数が落ちるので、直し漏れは検知できる。
- **`stamp-sw.sh` の除外は `find` の 1 語で、消えても挙動は正常なまま**（クローラ専用の画像が全ユーザーのシェルに載って太るだけ）。目視では絶対に気づけないので、`e2e/seo.spec.mjs` の「og.png は SW のオフラインシェルに入っていない」で機械的に固定した。
- **[Service Worker はシェル全体を BUILD_ID で原子的に入れ替える](../pwa/sw-atomic-shell-swap.md) の「precache 一覧を staging 全走査で作っているので漏れは構造的に起きない」に、初めて例外を作った。** 保証は「全走査」から「全走査 − 明示除外リスト」になる。以後この `find` の除外に載せてよいのは**アプリが一度も参照しないファイルだけ**で、それ以外を足すとオフライン起動が壊れる。[Service Worker はシェル全体を BUILD_ID で原子的に入れ替える](../pwa/sw-atomic-shell-swap.md) の決定そのものは生きているので置換扱いにはしない。
- **Google の Software App リッチリザルトには乗らない。** 必須プロパティが `name` / `offers` / **`aggregateRating` または `review`** で、自作アプリに自前の星を書くのはポリシー違反なので埋められない。Search Console の Software App レポートにエラーが 1 件常駐するのは織り込み済みである。この JSON-LD の実益は、検索エンジンのエンティティ理解と、**JS を実行しないクローラ（LLM 系を含む）が読める唯一の構造化された説明**であることに置いている。
- **`<title>` が英語のままなので、日本語クエリでタイトル側にヒットする語が無い。** スニペットの関連性は `description` に依存する。実測して足りなければ `<title>` の日本語化を再検討する余地は残る（そのときは 4 点セットの扱いを含めて改めて決める）。
- **CSR なので、検索エンジンが本文を読むには JS の実行が要る。** Googlebot は実行するので通常は成立するが、**JS を実行しないクローラ（LLM 系の多くを含む）にとっては `<noscript>` の中身がこのサイトの本文そのものになる**。Googlebot のレンダリングが失敗・タイムアウトしたときの保険でもある。この 2 つの役割を持つ以上、`<noscript>` は 1 行の断り書きではなく機能の要約まで書く。scripting 有効な文書では要素に parse されず `display:none` になるので、`mount_to_body`（append）とも既存のレイアウト検証（`e2e/pwa.spec.mjs` のボトムタブ `boundingBox`）とも競合しない。

## 検討した代替案

**`stamp-sw.sh` と同じ post_build フックで URL を置換する**: 絶対 URL の利点を保ちつつ環境差を吸収できる。しかし [manifest の URL を全て相対にする](../pwa/manifest-relative-urls.md) が同じ形の案を「置換漏れで壊れる経路が増える」で却下しており、置換しなくても実害がゼロな以上、機構を増やす理由がない。却下。

**`og.png` を SHELL に載せたまま運用する**: 除外を書かなくて済む。しかし約 1.0MB のオフラインシェルに、アプリが一度も参照しない 21KB を恒久的に足すことになる。却下。

**`<title>` を日本語化する（例: `筋トレメモ | オフラインで使える筋トレ記録アプリ`）**: 検索結果に出る唯一の見出しで、on-page で最も強いシグナル。実機のアイコン名にも書き出しファイル名にも影響しないことは確認済み。しかしアプリ名 4 点セットは 4907db3 で明示的に揃えた決定であり、`description` / `og:title` / JSON-LD で日本語キーワードは持てる。今回は崩さない。却下（ただし将来の再検討は残す）。

**静的コンテンツを `<body>` に置き、wasm 起動後に JS で消す**: CSR の空 `<body>` を根本から埋められる。しかし `mount_to_body` は append なので静的ブロックはアプリの上に残り続け、wasm 885KB のロード完了まで実在する。視覚的な入れ替わり・レイアウトシフト・既存の `boundingBox` 検証との競合を新たに背負う一方、Googlebot は JS を実行するので得られる差分はほぼ無い。却下。

**manifest に `screenshots` を足す**: Chrome のリッチなインストールダイアログに効く。しかし `assets/` のスクショ 3 枚で 430KB を dist に配ることになり、`copy-file` 宣言が 3 本と SW の除外がさらに増える。**主対象の iPhone では完全に無視される**。却下。

**sitemap.xml を置く**: ルーターを持たず（[ルーターを使わずタブを enum signal で切り替える](../architecture/no-router-tab-enum-signal.md)）インデックス可能な URL が 1 本しか無いので、sitemap の役割（クロール対象の発見）が成立しない。しかも robots.txt から宣言する経路も無い（下記）。必要になれば Search Console に URL を直接送信すれば足りる。却下。

**robots.txt を置く**: サブパス配信（[GitHub Pages の branch deploy（`release` / `docs`）を使う](../deploy/github-pages-branch-deploy.md)）なので `/fitness-memo/robots.txt` はどのクローラも読まない。オリジンルートの `asugawara.github.io/robots.txt` は別リポジトリの管轄であり、ここからは配置できない。実測すると 404 が返る＝既定で全許可なので、そもそも置く必要がない。却下。

**`Trunk.toml` の `minify` を有効にする**: Core Web Vitals は SEO のランキング要因なので HTML / CSS / JS の圧縮は理屈として効く。しかし SRI ハッシュと `stamp-sw.sh` の BUILD_ID に触るリスクを負うことになり、そもそも転送量は wasm 885KB が支配している。SEO を理由にこのリスクは取らない。却下（速度を詰めるなら wasm 側の別タスク）。

**`twitter:title` / `twitter:description` / `twitter:image` を個別指定する**: X の表示を独立して制御できる。しかし未指定なら `og:*` にフォールバックするので、二重管理が増えるだけになる。`twitter:card` のみ指定する。却下。
