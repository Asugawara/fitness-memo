# 許諾文をコメントの外へ出して trunk の minify を解禁する

- **状態**: 採用
- **日付**: 2026-08-20
- **カテゴリ**: architecture
- **関連**: [Rust + Leptos (CSR) + trunk を採用する](rust-leptos-csr-trunk.md), [読み込みと操作は実測して、何も入れないと決めた](measure-before-optimizing-and-do-nothing.md), [アイコンに lucide を採り、`assets/icons/*.svg` を `include_str!` で埋め込む](lucide-icons-as-included-svg.md), [クローラ向けメタデータを本番 URL のハードコードで持ち、オフラインシェルから外す](../seo/crawler-metadata-and-hardcoded-origin.md)

## 背景

Lighthouse 12.6.1（mobile / `--throttling-method=simulate`）で本番を 5 回測ってパフォーマンスが **92**（中央値）だった。
落としているのは **LCP だけ**（3306ms / 0.69）で、FCP / TBT / CLS / SI は満点。
`unminified-css` が転送 24,318B の CSS に削減余地を指摘していた。`public/styles.css` は 78,998B / 2,244 行で、
バイト数の大半は日本語の設計コメントである。

**それでも `minify` は trunk の既定（`never`）に固定してあった。** 理由は `index.html` の `<head>` に
lucide (ISC) と Feather (MIT) の許諾文全文を **HTML コメント**として置いていたこと。
両ライセンスは「全ての複製に著作権表示と許諾文を添える」ことを要求するので、配信物に必ず入る場所として
`<head>` のコメントを選んでいた（[アイコンに lucide を採り〜](lucide-icons-as-included-svg.md)）。
trunk の HTML minify は `minify_html::Cfg::spec_compliant()` を使い `keep_comments` を立てないため、
minify を有効にすると**許諾文が配信物から消える** = ライセンス違反になる。
`data-no-minify` はアセット単位のオプトアウトで、HTML 本体には効かない。

つまり「コメントに置く」という 1 つの判断が、CSS の minify ごと止めていた。

## 決定

**許諾文を `<script type="text/plain" id="third-party-licenses">` の中身へ移し、`Trunk.toml` に `minify = "on_release"` を入れる。**

## 理由

### 許諾文の置き場所

- `<script>` の中身は HTML としてパースされないので、**原文が字句どおり残る**（`<` や `&` のエスケープも要らない）。
- `type` が JS ではないので実行されない。レイアウト・アクセシビリティツリー・LCP 候補のどこにも出ない。
- 実測: minify 込みのビルドで `dist/index.html` に ISC と MIT の本文が両方残る（`grep -c 'THE SOFTWARE IS PROVIDED'` が 2、`grep -c 'Permission to use, copy, modify'` が 1）。同じビルドで `grep -c '<!--'` は 0。

`<noscript>` の中には入れなかった。[クローラ向けメタデータ〜](../seo/crawler-metadata-and-hardcoded-origin.md) が
「JS を実行しないクローラにとって noscript の中身がこのサイトの本文そのもの」と位置づけている資産を、2.5KB の法文で薄めない。

### minify を release ビルドだけにする

`on_release` なら `.githooks/pre-commit` の `trunk build`（debug）と `trunk serve` の出力は読めるまま残り、
効くのは `scripts/release.sh` の `trunk build --release` だけ。
**その release.sh は全 project の E2E を `dist-release` に対して回すので、minify 後の成果物が必ずテストされる。**

### 外部ツールを足さない

trunk 0.21.14 の内蔵 CSS minifier は **lightningcss（targets 指定なし）そのもの**（`src/processing/minify.rs`）。
lightningcss を devDependency に足しても出力は同じで、`trunk build` が node を要求するようになるぶんだけ悪くなる。

## 結果（トレードオフ）

release ビルドの実測（転送相当は `gzip -9`）:

| ファイル | raw before | raw after | 転送 before | 転送 after |
|---|---|---|---|---|
| `index.html` | 14,432 | **7,407** | 6,560 | **3,532** |
| `styles-*.css` | 78,998 | **20,095** | 24,318 | **4,543** |
| `fitness-memo-*.js` | 50,318 | 50,318 | 8,876 | 8,876 |
| `*_bg.wasm` | 1,124,677 | 1,124,677 | 447,526 | 447,526 |

**転送で 22,803B（約 22KB）減る。** 追記の式（`LCP_sim ≒ TTFB + グラフ内バイト × 8 ÷ 1,638,400`）に入れると
LCP の見積りは約 111ms 縮む。オフラインシェルもそのぶん軽くなる。

★ **上の表は手元の `gzip -9` を物差しにした A/B で、本番の配信量ではない。** GitHub Pages の圧縮は
`gzip -9` より約 1.8% 緩く、実配信は wasm 455,534B / CSS 24,905B / index.html 6,312B（`Content-Length`）。
本番基準に置き直すと **−23,187B ≒ −113ms**（LCP 3,304ms → 約 3,191ms）で、**カテゴリ 92 → 93 の見込み**。
`--public-url /fitness-memo/` を付けると URL が伸びて `index.html` が 65B ほど増えるので、
上の raw も本番ビルドでは 14,497 / 7,472 になる。

**93 は未検証の予測である。** 手元に gzip するサーバを立てて minify の有無だけを変えた A/B では
490,484B / LCP 3,601ms / **90** → 467,392B / LCP 3,451ms / **91** と 1 点動いた（3 回ずつ / ぶれ 1〜2ms）。
ただし localhost は HTTP/1.1 なので接続確立の RTT が余分に積まれ、**本番より 1〜2 点低く出る**
（[追記](measure-before-optimizing-and-do-nothing.md)の表を参照）。**本番の数字はマージ後に測って書き足すこと。**

- **dist から設計コメントが全部消える。** ソースは無変更で、消えるのは配信物だけ。
  `<noscript>` の本文・meta・OGP・JSON-LD は要素なので残る（`e2e/seo.spec.mjs` が固定している）。
- **JS の minify は失敗する。** trunk が `WARN Failed to minify JS: RequiredTokenNotFound(Identifier) [token=Some(KeywordDefault)]`
  を 1 行出す。wasm-bindgen の glue の `export default` を内蔵の minify-js がパースできない。
  **失敗時は元のバイト列がそのまま使われる**（minify なしビルドの glue と `cmp` でバイト一致を確認済み）ので害は無い。
  `data-no-minify` を `rel="rust"` に付けても警告は消えない（実測）。JS の 8,876B は縮まないままである。
- **SRI は minify の後に計算される。** 実測で `dist/index.html` の `integrity` 3 件が、minify 後のファイルの sha384 と一致した。
  ここが逆だとブラウザが CSS と module を拒否して真っ白になるので、
  **リリース前に `dist-release` に対して E2E を回す手順は外せない**（release.sh が既にそうなっている）。
- debug と release で配信物の形が違う。debug で読めた CSS が release では 1 行になる。

**CSS の意味が変わっていないことは A/B のピクセル比較で確認した。** 同じ wasm で CSS だけ差し替えた 2 つの release ビルド
（`--minify false` / minify あり）を静的サーバで並べ、412×823 の Chromium で記録 / 推移 / 設定の 3 タブ ×
ライト / ダークの 6 枚をフルページで撮って sha256 を比べ、**6 枚すべてバイト一致**した。
`prefers-color-scheme` / `color-scheme: light dark` / `env(safe-area-inset-*)` / `font-variant-numeric` /
`overscroll-behavior` の出現数も minify 前後で同じ（1 / 2 / 4 / 12 / 2）。
内蔵 minifier が lightningcss で、**安全と証明できない最適化を行わない**という同ツールの方針どおりの結果である。

## 検討した代替案

**lightningcss-cli を devDependency に入れ、pre_build フックで CSS だけ minify する。**
許諾文をコメントのまま残せるが、`trunk build` が全経路（pre-commit の debug ビルドを含む）で node を要求するようになり、
生成物の置き場所と `.gitignore` も要る。`[watch] ignore` は起動時に `canonicalize()` されるのでパスが無いと
`trunk serve` がその場で落ちるうえ、trunk は cargo の target ディレクトリを実行時に自動で無視するので不要だった。
**内蔵 minifier が同じ lightningcss である以上、複雑さのぶんだけ損である。**

**許諾文を `THIRD-PARTY-LICENSES.txt` として `copy-file` で配信する。**
オフラインシェル（約 1.0MB）にファイルが 1 つ増える。`og.png` をシェルから外した
[クローラ向けメタデータ〜](../seo/crawler-metadata-and-hardcoded-origin.md) の判断と逆を向く。

**CSS を `rel="inline"` で HTML に取り込む。** `render-blocking-resources`（150ms）は消えるが、
Lantern の見積りでは**総バイトが変わらない**（CSS のバイトが document ノードへ移るだけ）。
効くとすれば「CSS のリクエストが消えて接続が 1 本空き、wasm の開始が前倒しになる」経路だけで、
本番の devtoolslog では同一 origin への接続が h2 で 2〜3 本しかない（`ConnectionPool` は h2 なら `minConnections = 1`）。
**この本数では効果の符号が反転する**（1〜2 本なら −145〜−245ms、3 本以上なら ±0〜+22ms 悪化）。
そして**その差は本番にデプロイしないと測れない**。CSS の SRI が消え、オフラインシェルの構成も変わる。
[読み込みと操作は実測して、何も入れないと決めた](measure-before-optimizing-and-do-nothing.md) の作法に従い、
**符号が分からないものは入れない。**
なお `data-inline` を `rel="css"` に付ける形は trunk 0.21.14 に存在しない（`src/pipelines/css.rs` に inline 分岐が無く、
属性は出力から黙って除去される＝外部 CSS のまま成功する）。使えるのは `rel="inline"` だけである。

**trunk の `minify` を `always` にする。** debug の出力が読めなくなり `trunk serve` での確認が辛くなる。release に無い利得。
