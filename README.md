# 筋トレメモ

iPhone のホーム画面から起動して完全オフラインで動く、個人用の筋トレ記録 PWA。
サーバ通信もアカウント登録も持たず、記録は端末の `localStorage` にのみ保存する。
「前回何をどれだけやったかを見て、同じかそれ以上をやる」というループだけを最短手数で回すことに絞っている。

公開先: <https://asugawara.github.io/fitness-memo/>

## 画面

タブは **記録 / 推移 / 設定** の 3 つ。記録タブはカレンダーと選択日の入力欄が縦に並んだ 1 画面で、日セルをタップするとその下の入力欄がその日のものになる（[記録タブをカレンダー + 選択日エディタの単一画面にする](adr/ux/record-tab-calendar-with-day-editor.md)）。設定タブは **データの書き出し / 読み込み・トレーニングメニュー・種目・ホーム画面への追加のしかた** の 4 行だけを並べ、押すとその節に入る（[設定タブの入口を節の一覧にし、中身は 1 階層下ろす](adr/ux/settings-as-a-list-of-sections.md)）。

| 記録 | 推移 | 設定 |
|---|---|---|
| ![記録タブ](assets/1-record.png) | ![推移タブ](assets/2-progress.png) | ![設定タブ](assets/3-menu.png) |

スクリーンショットは `trunk build && node scripts/shots.mjs` で撮り直す。端末（iPhone 15 Pro 相当）・standalone 起動・投入する記録を固定してあるので、UI を変えたら 3 枚まとめて同じ条件で更新できる。

アイコン（`assets/icon-master.png`）を差し替えたときは `sh scripts/gen-icons.sh` と `sh scripts/gen-og.sh` を両方回す。後者は SNS のカードに出る OGP 画像（`public/og.png`）で、アイコンを 1200x630 のキャンバスに置いただけのもの。**クローラしか取りに来ないので Service Worker のオフラインシェルからは外してある**（[クローラ向けメタデータを本番 URL のハードコードで持ち、オフラインシェルから外す](adr/seo/crawler-metadata-and-hardcoded-origin.md)）。URL がハッシュ無しの固定名なので、差し替えてリリースしたあとは Facebook の Sharing Debugger と X の Card Validator で再スクレイプさせないと各 SNS には古い絵が出続ける。

## 主な機能

- **前回の参照とコピー** — 種目ごとに前回のセット数・レップ数・重量を表示する。その日のセットがまだ空のときだけ「前回をコピー」が出て、ワンタップで今日の入力欄にプリフィルできる。
- **トレーニングメニュー** — よくやる種目の組み合わせに名前を付けて保存できる。空の日にはその一覧が候補として並び、1 タップで種目のカードが揃う。入る数値は**種目ごとに別々の日**から引く — その種目の直近の記録なので、「胸の日」の外でベンチプレスをやった分もちゃんと反映される。まだ一度もやったことのない種目は空のカードとして出る（[保存したメニューから始める（種目タブを設定タブに改める）](adr/ux/start-from-a-saved-routine.md) / [トレーニングメニューを「名前 + 種目 ID の並び」だけのデータにする](adr/data-model/routines-as-named-exercise-lists.md)）。
- **その日の記録をそのままメニューにする** — 作り方は 2 通り。設定タブで種目を選んで組むか、記録タブで**その日の下に出る「＋ この日をメニューにする」**を押す。後者はカレンダーで選んだ任意の日が対象で、その日の種目が初期選択で入るので名前を付けるだけで終わる。開くシートは設定タブと同じものなので、その場で種目を足し引きしてから保存できる（[その日の記録から直接メニューを作れるようにする](adr/ux/save-a-day-as-a-routine.md)）。
- **カレンダーからの記録追加** — 記録タブは月グリッドと選択日の入力欄が縦に並んだ 1 画面。実施日は部位カラーのドットで示す。記録が無い日でも、**日セルをタップした時点で下の入力欄がその日のものになる**ので、前日の記録し忘れをタブ往復なしでそのまま入れられる。
- **部位ごとのグループ分け** — 胸 / 背中 / 肩 / 腕 / 脚 / 体幹 の 6 部位に 28 種目をプリセットとして初期投入する。設定タブの「種目」節は**部位だけを並べ**、押すとその部位の種目が開く（同時に開くのは 1 つ）。名前と色の変更は右端の鉛筆から。部位も種目も追加・改名・削除ができる（[種目タブを部位の折りたたみ一覧にし、1 つだけ開く](adr/ux/menu-groups-as-single-open-accordion.md)）。
- **仕事量のグラフ** — 種目別・部位別を折れ線で表示する。指標は ボリューム = Σ(重量 × 回数) / セット数 / 回数 をその場で切り替えられる。期間は 1M / 3M / 6M / 1Y / 全期間。
- **体重を第2軸に重ねたグラフ** — 記録した体重を、指標の折れ線と同じグラフに右軸の破線として**常に**重ねる（表示の切り替えは無い）。「重量は伸びたが体重も増えていたのか」を 1 画面で読めるようにするため。右軸は 0 起点ではなくデータに合わせるので、数百グラム単位の遷移も見える（[体重を推移グラフの第2軸に常時重ねる](adr/ux/body-weight-second-axis-always-on.md)）。
- **最後のトレーニングからの経過** — 全体と部位ごとに表示する。**日数はローカル暦の日差**で、日を跨いだら「昨日 / 3日前」、同じ日のうちだけ「45分 / 12時間」の時刻粒度になる。経過時間を 24 時間で割ると繰り上がりがトレーニング時刻の 24 時間後に来てしまい、昨夜の記録が翌朝に「今日」と出る（[経過日数をローカル暦の日差にし、時刻粒度を同じ日の中だけに閉じる](adr/data-model/elapsed-in-local-calendar-days.md)）。
- **体重と体調メモ** — 日ごとに 1 行で記録できる。トレーニングしていない日でも記録でき、その日も上のグラフに乗る。

**指標は種目の属性ではなくグラフの表示設定**にしている。種目ごとに単位が違うと同じ軸で比べられず、後から種目の性質が変わると過去のグラフが遡って壊れるため（[指標を種目の属性ではなくグラフの表示設定にする](adr/data-model/metric-is-a-view-setting.md)）。重量欄は全種目に出し、**空欄は重量 1 として数える**ので、自重種目も時間種目も「入れなければよい」で成立する。

## 技術構成

| 項目 | 選択 |
|---|---|
| 言語 / フレームワーク | Rust + [leptos](https://leptos.dev/) 0.8（CSR）。ビルドは [trunk](https://trunkrs.dev/) |
| ルーティング | なし。タブは enum の signal で切り替える |
| グラフ | ライブラリを使わず SVG を自前で描画 |
| アイコン | [lucide](https://lucide.dev/)（ISC / 一部 MIT）の SVG を `assets/icons/` に置き `include_str!` で埋め込む。npm 依存も CDN 参照も増やさない |
| 永続化 | `localStorage` の単一キー `fitness-memo/v3` に JSON 全体（旧 `v2` / `v1` は読み取り専用で引き継ぐ） |
| CSS | 素の CSS 1 ファイル + CSS 変数 |
| デプロイ | GitHub Pages の branch deploy（`release` ブランチの `/docs`） |
| CI | **GitHub Actions のワークフローファイルを書かない。** `.githooks/pre-commit` でローカル実行する |

UI 層（`leptos` / `web-sys` に依存する部分）は `[target.'cfg(target_arch = "wasm32")'.dependencies]` に置いてあるので、`cargo test` はホスト向けに leptos の依存グラフをビルドしない。純ロジックは `src/core.rs`（`Db` を読む計算）と `src/chart_layout.rs`（グラフの座標計算）に集約してあり、ここが単体テストの主対象になる。

## 開発

### 前提ツール

- [rustup](https://rustup.rs/)（`rust-toolchain.toml` で stable と `wasm32-unknown-unknown` を宣言しているので、リポジトリ内での初回実行時にターゲットは自動で入る）
- trunk — `brew install trunk`
- Node.js（Playwright 用）

### セットアップ

```sh
sh scripts/setup.sh
```

`git config core.hooksPath .githooks` でフックを有効化し、`npm install` と Playwright のブラウザ（Chromium / WebKit）を入れる。**このスクリプトを実行しないと `pre-commit` は一度も発火しない。** GitHub Actions のワークフローを持たない構成なので、`pre-commit` が唯一の防波堤になる。

### 開発サーバ

```sh
trunk serve
```

<http://localhost:8080> で起動する。**この 8080 番では Service Worker を登録しない**（`index.html` が `location.port` で判定している）。開発中に cache-first の Service Worker に捕まって古い成果物を見続ける事故を避けるため。すでに登録済みの Service Worker を外したいときは `?sw=off` を付けて 2 回リロードする。

### テスト

```sh
cargo test                              # src/core.rs の純ロジック（ホスト）
trunk build                             # dist/ を作る（E2E はこれを配信する）
npx playwright test --project=chromium  # 軽い E2E
npx playwright test                     # 全 project（Chromium / iPhone 15 Pro (WebKit) / Pixel 7）
```

`.githooks/pre-commit` は `main` への `docs/` 混入をガードしたうえで、`cargo fmt --all -- --check` → `cargo clippy --target wasm32-unknown-unknown --all-features -- -D warnings` → `cargo test` → `trunk build` → `npx playwright test --project=chromium --project=harness` を順に実行する。緊急時は `SKIP_HOOKS=1 git commit` で飛ばせる。

複数人（または複数エージェント）で並行作業する場合、`dist/` と Playwright のポート 4173 は共有資源になる。出力先を分けたいときは `trunk build --dist <ディレクトリ>` と `DIST_DIR=<同じディレクトリ> npx playwright test` を組み合わせる。

## デプロイ

`main` はソースのみを持ち、`release` ブランチの `docs/` がビルド成果物であり **GitHub Pages の配信元**になる。`main` には `docs/` を絶対にコミットしない（混入すると以後のマージが modify/delete コンフリクトで停止する。`pre-commit` がガードしている）。

```sh
sh scripts/bootstrap-release.sh   # 初回 1 回だけ
sh scripts/release.sh             # 2 回目以降
```

`release.sh` は本番と同じパス構成（`--public-url /fitness-memo/`）でビルドし、WebKit と iPhone エミュレータを含む重い E2E を通してから `release` ブランチへの PR を作る。PR を **merge コミットで**マージすると Pages が自動デプロイされる（squash / rebase はリポジトリ設定で無効化してある。これらを使うと `main` のコミットが `release` の祖先に入らず、以後コンフリクトが多発する）。

> [!IMPORTANT]
> **リポジトリ設定の Actions を無効化してはいけない。**
> branch deploy を選んでいても、GitHub Pages は内部で必ず `pages build and deployment` というワークフローを実行する。Actions を無効にすると `Error: Actor is not allowed to trigger Actions workflows` でデプロイが止まる。「Actions を使わない」という方針は **`.github/workflows/` を書かない**という意味に限定される。

## iPhone へのインストール

> [!WARNING]
> **記録を付ける前に、先にホーム画面へ追加すること。**
> iOS では Safari のタブと、ホーム画面に追加した standalone PWA とで `localStorage` が共有されない。先に Safari のタブで記録を付けてからホーム画面に追加すると、PWA 側は空のデータベースで起動し、それまでの記録は見えなくなる。
> この状態になっても、Safari のタブ側で「エクスポート」→ PWA 側で「インポート」を通せば移行できる（設定タブの「エクスポート / インポート」）。
> アプリ側でも、standalone で起動していないときは記録タブの末尾（「種目を追加」より下）に注意書きを出す。これは押せるボタンで、図解つきの手順シートが開く。**記録の邪魔をしないよう折り返しの下に置いてあるので、スクロールしないと見えない。**

1. iPhone の **Safari** で <https://asugawara.github.io/fitness-memo/> を開く（Chrome など他のブラウザではこの手順は使えない）
2. 画面の下のまん中にある共有ボタンを押す
3. リストを**下にスクロール**して「ホーム画面に追加」を選ぶ
4. 右上の「追加」を押す
5. **ホーム画面のアイコンから起動して**、そこで記録を始める

記録タブの注意書きが出なくなれば standalone で起動できている。出たままならまだブラウザのタブである。

一度追加すれば機内モードでも起動し、記録の追加・編集ができる。手順はアプリ内からも読める（記録タブの注意書き、または設定タブの「ホーム画面への追加のしかた」）。

注意書きは ✕ で今後表示しないようにできる（`localStorage` の `fitness-memo/ui/v1` に記録され、`Db` には入らない）。消しても設定タブの導線は残るので、手順自体は読める。

## データを守る

記録は端末の `localStorage` にしかない。**同じ端末の中に何重にコピーを置いても、「履歴とWebサイトデータを消去」や機種変更では全部同時に消える**（iOS では localStorage と IndexedDB が同じ削除単位にある）。守れるのは端末の外に出したファイルだけなので、そこに絞ってある。

設定タブの「エクスポート / インポート」から:

- **エクスポート** — 1 タップで **TSV**（`fitness-memo-YYYYMMDD-HHMM.tsv`）を出す。iPhone では共有シートが開くので、**「ファイルに保存」→ iCloud Drive / Google Drive** を選ぶと機種を替えても残る。**Google スプレッドシートでそのまま開ける**（1 セット 1 行の表）
- **インポート** — エクスポートしたファイルを選ぶ。実行前に「現在」と「取り込み後」の件数、そして何が増えるかを出し、**取り込む直前に今のデータを自動で退避**してから適用する（直後なら「元に戻す」で戻せる）
  - 取り込みは**足すだけ**に固定してある。今の記録は 1 つも消えず、無い日と無い種目だけが増える（[`adr/storage/import-is-merge-only.md`](adr/storage/import-is-merge-only.md)）
  - 旧版が書いた `.json` もそのまま読める

## 現時点の制限

- 保存済み JSON のパースに失敗した場合は、上書きせず `fitness-memo/v3.bak-<epoch>` に退避してから初期状態で起動し、起動時に一度だけ通知を出す（破損データをプリセットで黙って上書きしないため）。**退避したデータを画面から取り出す導線は今は無い**（[`adr/storage/quarantine-on-parse-failure.md`](adr/storage/quarantine-on-parse-failure.md) の追記）。
- 書き出しの TSV は `Db` の全部を持たない。**ID・部位の色・並び順・アーカイブ状態・記録時刻**は落ち、取り込み時に名前とプリセットの固定 ID から作り直す（色と並び順は既定に戻る）。
- スプレッドシートで編集して戻す経路は best effort。改行・CRLF・`YYYY/M/D`・`62,5`・列の増減・行の並べ替えは吸収するが、**`=` `+` `-` `@` で始まるメモはシート上で数式になる**ので直せない。
- 1 日 = 1 セッション、1 日 1 種目 1 ログ。日付はローカル日付なので、深夜 0 時をまたいだトレーニングは 2 日に分かれる。
- 自動バックアップは無い（iOS では共有シートもダウンロードもユーザー操作なしには起動できないため）。書き出しは手動で行う。

## 設計判断

この構成に至った経緯と、検討した代替案は [`adr/README.md`](adr/README.md) にまとめてある。
