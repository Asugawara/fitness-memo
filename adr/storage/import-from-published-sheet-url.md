# 公開リンクの Google スプレッドシートを URL で取り込む

- **状態**: 採用
- **日付**: 2026-08-13
- **カテゴリ**: storage
- **関連**: [CSV を二次形式として足し、正は JSON のままにする](csv-as-a-secondary-lossy-format.md)（読み取る形式）, [表からの取り込みは利用者に何も聞かない](../ux/spreadsheet-import-asks-nothing.md)（取り込みの UI）, [書き出しは共有シートを主経路にし、iOS では `<a download>` を使わない](share-sheet-over-download.md), [localStorage の単一キーに JSON 全体を持つ](localstorage-single-key-json.md)（サーバ同期を却下した ADR）, [ブラウザサポートは Safari を基準にし、polyfill を入れない](../architecture/browser-support-policy.md)

## 背景

[CSV を二次形式として足し、正は JSON のままにする](csv-as-a-secondary-lossy-format.md) で表を読めるようにしたが、**iPhone で表をアプリに渡す手段が重い**。

Google スプレッドシートのアプリから CSV を書き出し、ファイルアプリに保存し、`<input type="file">` で選ぶ。3 アプリをまたぐうえ、iOS の Files ピッカーは [書き出しは共有シートを主経路にし、iOS では `<a download>` を使わない](share-sheet-over-download.md) が書いたとおり `accept` が壊れているので候補も絞れない。

**URL を貼るなら 2 タップで済む。** ただしこのアプリは「サーバ通信もアカウント登録も持たない」ことを README の 1 行目に書いており、[localStorage の単一キーに JSON 全体を持つ](localstorage-single-key-json.md) は**サーバ同期を「要件の中心に反する。検討対象外」として却下している**。外へ出る通信を足すなら、線をどこに引くかを決める必要がある。

## 決定

**`transfer::fetch_text` を 1 つだけ足す。線は次のように引く。**

1. **利用者が URL を貼って押したときにしか走らない。** 起動時・保存時・定期実行のいずれからも呼ばない
2. **記録は外に出ない。** GET だけで本文を持たない。送るのは利用者が貼った URL だけ
3. **オフラインの起動と記録は今までどおり。** この経路が失敗してもアプリの他の機能は 1 つも変わらない
4. **アカウントも OAuth も持たない。** シート側の「リンクを知っている全員が閲覧可」に寄りかかる

URL は `sheet::csv_url`（純関数）で CSV の URL に直す。

```
https://docs.google.com/spreadsheets/d/{ID}/edit?gid=123#gid=456
  → https://docs.google.com/spreadsheets/d/{ID}/export?format=csv&gid=456

https://docs.google.com/spreadsheets/u/1/d/{ID}/edit                 （複数アカウント）
  → https://docs.google.com/spreadsheets/d/{ID}/export?format=csv

https://docs.google.com/spreadsheets/d/e/{PUB_ID}/pubhtml?gid=7      （ウェブに公開）
  → https://docs.google.com/spreadsheets/d/e/{PUB_ID}/pub?output=csv&gid=7&single=true
```

**`u/N/` を必ず読み飛ばす。** 2 つ以上の Google アカウントにログインしていると、アドレス欄の URL は常にこの形になる。落とすと、その人たちは URL 取り込みを一切使えない（`sheet_url_reads_the_multi_account_form_people_actually_copy`）。

## 理由

### 実測（推測ではない）

curl に `Origin` を付けて確かめた。

| 対象 | 認証 | 結果 |
|---|---|---|
| `/export?format=csv`（リンク共有のシート） | 不要 | **200 + `text/csv`**。`access-control-allow-origin` に Origin を反射し、リダイレクト先の `googleusercontent.com` も `access-control-allow-origin: *` |
| `/export?format=csv`（非公開 / 存在しない ID） | — | **404 + `text/html`（CORS ヘッダ付き）** |
| Sheets API v4 読み | 必要 | 403 `Method doesn't allow unregistered callers` |
| Sheets API v4 書き | 必要 | 401 `Request is missing required authentication credential` |
| Sheets API v4 書き + API キー | — | 401 **`API keys are not supported by this API. Expected OAuth2 access token`** |

読み取りだけが非対称に開いているのは、`/export` が Sheets API ではなく **docs.google.com のファイル書き出しエンドポイント**で、公開設定なら Cookie 無しで通るから。

### 失敗の見分け方が UX の本体

**非公開シートは CORS エラーにならない。** 404 + `text/html` が CORS ヘッダ付きで返るので `fetch` は成功する。つまり「なぜか失敗しました」ではなく、

> シートを読み取れませんでした。共有 →「リンクを知っている全員」→「閲覧者」にしてから、もう一度試してください

と**言い切れる**。これが無ければこの機能は入れる価値がなかった。原因が分からないまま何度も押させる導線は、[他アプリからの移行はスクショの文字起こしを貼り付けて受ける](../ux/migrate-by-ocr-paste.md) が撤去された理由（操作量が移行 1 回に見合わない）と同じ穴に落ちる。

`content-type` を `ok()` より**先**に見るのは、公開シートでも 200 のままログインページに化けることがあるため。`ok()` だけ見ていると HTML を CSV として読ませて「見出しがありません」と誤診する。

reject（`TypeError`）は通信断・DNS・CORS 拒否のどれでも中身が同じで区別できないので、`navigator.onLine` だけで「オフライン」と「取得できません」を言い分ける。

### Service Worker と CSP は触らなくてよい

`public/sw.js` の `fetch` ハンドラは 2 行目で `url.origin !== self.location.origin` なら早期 return する（[fetch ハンドラで navigate を明示分岐する](../pwa/sw-explicit-navigate-branch.md)）。**この経路に SW は一切関与しない。** `index.html` に CSP の meta は無く、GitHub Pages も CSP ヘッダを付けない。

### `RequestInit` を宣言しない

Google が `cache-control: no-cache, no-store, max-age=0, must-revalidate` を返すので、`RequestCache::NoStore` を指定する必要がない。`Window::fetch_with_str` は既定で `mode: cors` / `redirect: follow` なので `Request` / `RequestInit` も要らない。**増えた web-sys feature は `Response` と `Headers` の 2 つだけ。**

### なぜ書き戻さないのか

上の表のとおり、**公開設定に関係なく匿名の書き込み経路は存在しない**。「リンクを知っている全員が**編集**可」にしても Sheets API v4 は OAuth を要求する（API キーは明示的に拒否される）。

OAuth を入れれば書けるが、Google Cloud プロジェクト・OAuth 同意画面・外部 JS の読み込みが必要になる。最後のものは [ブラウザサポートは Safari を基準にし、polyfill を入れない](../architecture/browser-support-policy.md) の「CDN からの動的 import はオフライン起動で落ちる」に正面から抵触する。

書き出し側は**ファイル（共有シート / ダウンロード）と TSV クリップボード**で足りている。貼れば列に分かれるので、スプレッドシートに載せるまでが 3 タップで済む。

## 結果（トレードオフ）

- **このアプリ初の外部通信になった。** README の「サーバ通信を持たない」は「**自前のサーバを持たない / 記録を送らない**」の意味に精密化する必要がある。取り込みの GET 1 本だけが外に出る
- **「リンクを知っている全員が閲覧可」は、記録が推測困難な URL で公開される状態である。** 短時間で戻せるが、戻し忘れると公開されたままになる。UI に「取り込みが終わったら共有は元に戻せます」と 1 行書いた。**これは注意書きであって解決策ではない**（OAuth が唯一の解決策で、上の理由で採らなかった）
- **Google の URL 形式と `/export` の挙動に依存する。** 壊れたら `sheet::csv_url` と `transfer::fetch_text` の 2 箇所を直すことになる。壊れても他の取り込み経路（ファイル選択・貼り付け）は動く
- **E2E は `page.route` のモックでしか書けない。** 実物の Google に当てるテストは、共有設定の変更が要るうえ外部サービスの死活で赤くなる。**モックが緑でも実物で動く保証はない**ので、実シートでの往復は手で確認する
- **`/d/e/`（ウェブに公開）と `/d/`（リンク共有）を混ぜてはいけない。** 前者は `export?format=csv` を受け付けないので、混ぜると「共有されていません」と誤診する。`sheet_url_extracts_id_and_gid_from_every_form_a_user_can_paste` が両方を固定している
- **`/pub` 側の `single=true` は実測していない。** 上の表は `/export` を curl で確かめたものだが、`/d/e/` は公開済みのシートが手元に無いと試せない。Google の公開 CSV は `gid` 単独では既定のシートを返すとされているので `single=true` を添えてあるが、**この 1 点だけは推測**。主経路は `/d/`（リンク共有）で、そちらは実測済み

## 検討した代替案

**OAuth（Google Identity Services + Sheets API v4）**: 非公開のまま読め、書き戻しもできる。**一度は却下理由を間違えた**ので、調べ直した結果を残す。

当初は [ブラウザサポートは Safari を基準にし、polyfill を入れない](../architecture/browser-support-policy.md) の「CDN からの動的 import はオフライン起動で落ちる」を根拠にしたが、**あれはオフラインシェルの話**で、タップしたときだけ読むオンライン専用機能には当たらない。適用が誤りだった。

調べ直して分かったこと:

- `drive.file`（アプリが作ったファイルだけ）は**非センシティブ扱いで Google の審査が不要**
- ブラウザアプリは OAuth の public client なのでクライアントシークレットを持たない。**クライアント ID は公開で問題なく**、守っているのは「承認済み JavaScript 生成元」の allowlist
- トークンはメモリだけに置けば永続化されず、サーバも無いので記録が第三者のバックエンドを通ることもない

**つまりセキュリティ上の理由では却下できない。** それでも採らないのは、**依存が 1 つ増えるから**である。

- `accounts.google.com/gsi/client` を**自オリジンで実行する**ことになる。このアプリは現在ランタイムの外部 JS が**ゼロ**で、Google 製とはいえ DOM と `localStorage` に触れる第三者コードを常設することになる
- `asugawara.github.io` は同じアカウントの全 GitHub Pages プロジェクトで**共有のオリジン**なので、生成元 allowlist はパスを見ない（[Service Worker はシェル全体を BUILD_ID で原子的に入れ替える](../pwa/sw-atomic-shell-swap.md) が `caches.keys()` について書いたのと同じ性質）
- 得られるのは**年に数回しか通らない導線**の利便性

**恒久的な依存に見合わない。** 共有シート経由の CSV と、この ADR の URL 取り込みで用は足りている。もう一度検討するときは、上の 3 点のどれかが変わったときにする。

**CORS プロキシを挟む**: 非公開シートは読めないままなので、得るものが無い。しかも記録が第三者のサーバを通る。却下。

**利用者に Google Apps Script を配置してもらう**: 非公開のまま読め、書き戻しもできる。ただし利用者がスクリプトを貼ってデプロイする手順が要る。**生涯数回の導線に、他のどの画面より多い操作を置く**という、[他アプリからの移行はスクショの文字起こしを貼り付けて受ける](../ux/migrate-by-ocr-paste.md) を撤去した理由そのものになる。却下。

**URL 取り込みを入れず、CSV ファイル選択だけにする**: 外部通信ゼロを維持できる。ただし iPhone では 3 アプリをまたぐ操作になる。なお**スプレッドシートのセルを選択してコピーし、貼り付け欄に貼る経路**（TSV）は共有設定を変えずに済むので残してある — こちらのほうがプライバシー上は望ましく、URL は「量が多いとき」「繰り返すとき」の経路という位置づけ。

**取得した CSV をキャッシュして再利用する**: 編集して取り込み直すのが主な使い方なので、古い内容を返すのは害しかない。Google が `no-store` を返すのでそもそも起きない。
