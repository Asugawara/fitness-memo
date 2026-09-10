# UI 関連パスを触ったコミットでだけスクリーンショットを撮り直す

- **状態**: 採用
- **日付**: 2026-09-09
- **カテゴリ**: deploy
- **関連**: [CI を `.githooks/pre-commit` で回す](ci-in-pre-commit.md), [ワークフローファイルを書かない（ただし Actions 機能は無効化しない）](no-workflow-files.md), [マージ方式を merge コミットのみに固定する](force-merge-commit-only.md), [マニュアルの図は `public/` に配信するスクリーンショットにし、`<img>` で参照する](../architecture/manual-figures-as-served-screenshots.md), [使い方マニュアルを設定タブの節にし、章は 1 つだけ開く](../ux/manual-as-a-settings-section-with-one-open-chapter.md)

## 背景

[マニュアルの図は `public/` に配信するスクリーンショットにし、`<img>` で参照する](../architecture/manual-figures-as-served-screenshots.md) で、マニュアルの図をアプリ自身のスクリーンショットにした。スクショは**実装が変わると嘘になる**。しかも嘘になったことは誰も気づかない — グラフの読み取り欄を 1 段下げても、図は古い位置のまま何事もなく表示され続ける。

`scripts/shots.mjs` は既にあり、README の 3 枚を撮っていた。これは**手で叩く前提**で、README のスクショが実装から遅れていても実害が小さかったから成立していた。マニュアルの図は「どこに何があるか」を説明するものなので、ずれた瞬間に説明として間違いになる。

自動化の置き場は 1 つしかない。[ワークフローファイルを書かない（ただし Actions 機能は無効化しない）](no-workflow-files.md) が `.github/workflows/` を一切書かないと決めているので、**`.githooks/pre-commit` が唯一の CI である**（[CI を `.githooks/pre-commit` で回す](ci-in-pre-commit.md)）。

問題は毎コミット撮ると git が膨らむことである。ADR や `Cargo.toml` だけを触ったコミットでも撮り直すなら、12 枚 約 214KiB のバイナリが履歴に積まれ続ける。[マージ方式を merge コミットのみに固定する](force-merge-commit-only.md) で squash しないと決めているので、**PR の中間コミットの blob も永久に残る**。

## 決定

**`.githooks/pre-commit` の `trunk build` の後、`npx playwright test` の前に撮影ブロックを挟む。stage された変更が `UI_PATHS` に当たるときだけ撮り、撮った図を `git add` する。**

```sh
UI_PATHS='^src/|^index\.html$|^public/styles\.css$|^public/manual/|^assets/icons/|^scripts/shots\.mjs$|^package(-lock)?\.json$'

if [ "${SHOTS:-1}" != "0" ] && git diff --cached --name-only -z | tr '\0' '\n' | grep -Eq "$UI_PATHS"; then
  if git diff --name-only -z | tr '\0' '\n' | grep -Eq "$UI_PATHS"; then
    echo "警告: stage していない UI 変更があります。図はワークツリーの状態で撮られます" >&2
  fi
  node scripts/shots.mjs --only=manual
  git add -- public/manual
  git diff --cached --name-only -- public/manual | sed 's/^/  撮り直した: /'
  if ! git diff --quiet --cached HEAD -- public/manual; then
    rm -rf dist/manual
    cp -R public/manual dist/manual
  fi
fi
```

付随して 5 つを決める。

1. **撮影の時計を固定する。** `scripts/shots.mjs` が `context.clock.setFixedTime(new Date('2026-03-12T10:30:00+09:00'))` と `timezoneId: 'Asia/Tokyo'` を全コンテキストに掛ける
2. **`@playwright/test` を exact pin にする**（`^1.62.1` → `1.62.1`）
3. **撮るのはマニュアルの図だけ**（`--only=manual`）。README の 3 枚は手動更新
4. **`SHOTS=0` を `SKIP_HOOKS=1` とは別の脱出口にする**
5. **`scripts/shots.mjs --check` を撮り直しの網にする。** 一時ディレクトリへ撮ってバイト比較し、差があれば exit 1。**README の 3 枚も対象に含める**

## 理由

- **時計の固定がこの機能を成立させている。** `shots.mjs` のシードは `daysAgo` 基準なので、固定しないと **UI を 1 文字も変えていない「日を跨いだコミット」でも**カレンダーの当日位置・経過表示・X 軸の右端が動く。自動 `git add` する設計でそれをやると、毎日ノイズ diff が出て履歴に blob が積まれ続ける。**オフセット（`+09:00`）を必ず書く**のも同じ話で、付けないと Node のローカル TZ で解釈され、マシン TZ が UTC のとき JST 19:30 になる。
- **実測で差分ゼロを確認した。** 図が入った後の最初の実走（コミット #4）で、撮り直しても **1 バイトも変わらず `git add` が no-op** になった。`public/manual/` は `git status` に出ない。これが設計どおり動いている証拠で、**ここが壊れると UI を触るコミットごとに約 214KiB の blob が積まれ始める**。
- **`UI_PATHS` の基準は「利用者が見るものが変わりうるか」。** `src/` を丸ごと入れているのは、`views` だけでなく `chart_layout`（折れ線の座標）・`core`（図に写る指標値）・`presets`（種目名）も図に出るからである。`assets/icons/` は lucide のグリフがシェブロンや鉛筆として写る。`package*.json` は Playwright を上げると Chromium が変わって全枚数が churn する。逆に `adr/` `Cargo.*` `Trunk.toml` `e2e/` は入れない（利用者の体験が変わらないので、撮っても差分が出ないぶんの時間が無駄になる）。
- **`-z | tr '\0' '\n'` を通すのは `core.quotePath` のため。** 既定 true なので `git diff --name-only` は非 ASCII を含むパスを `"src/…"` と引用符で囲んで出す。そのままだと `^src/` が当たらず、日本語ファイル名を含む UI 変更で撮影が黙って走らない。
- **README の 3 枚を pre-commit で撮らない。** README の PNG は 1 回の撮り直しで **383KB**（143,101 + 158,114 + 82,268）で、`assets/1-record.png` は既に 6 世代ある。UI を触りながら 5 コミット積む PR ではその 5 世代が履歴に残り、[マージ方式を merge コミットのみに固定する](force-merge-commit-only.md) により squash で消えない。マニュアルの図 12 枚（計約 214KiB、うち差分が出るのは変わった枚数だけ）とは桁が違う。**代わりに「UI を触った PR は最後のコミットで `--only=readme` を手で撮る」を規則にし、ドリフトは `--check` が捕まえる。**
- **`SHOTS=0` が要るのは `SKIP_HOOKS=1` が粗すぎるから。** `SKIP_HOOKS=1` は fmt / clippy / test / playwright まで全部飛ばすので、「今は撮り直したくないが検証は通したい」（撮影が壊れている、図を別コミットに分けたい）に使えない。脱出口が 1 つしかないと、撮影を飛ばしたい人が検証ごと飛ばすようになる。
- **判定を `= "1"` ではなく `!= "0"` にする。** `= "1"` だと `SHOTS=yes` / `SHOTS=2` / `SHOTS=true` と書いた人の撮影が**黙って**飛ぶ。止めたい人だけが `"0"` を書く形にして、綴りを間違えたら撮影が走る側へ倒す。
- **2 回目の `trunk build` ではなく `cp -R` で足りる。** `index.html` の `copy-dir public/manual` はファイル名にハッシュを付けない素のコピーで、`manual` は SW のシェルから外してあるので `sw.js` の再スタンプも要らない。`trunk build` は cargo が no-op でも wasm-bindgen を 62MB の debug wasm に走らせる。
- **Actions を採らない根拠は既に決まっている。** [ワークフローファイルを書かない（ただし Actions 機能は無効化しない）](no-workflow-files.md)。強制力のある CI で撮ってコミットし直す形（Actions が push する）は、そもそも書けるワークフローが無い。
- **`--check` が README も見るのは、含めないと誰も見ないから。** README を網から外すと「PR ごとに最新のスクショ」という要求が代替なしに消える。含めたうえで手動更新を規則化すれば、blob の増加は PR あたり 1 セットに収まる。実測で `--check` は 13 枚（README 3 + マニュアル 10）を見て 4.1 秒。

## 結果（トレードオフ）

- **撮影の増分と hook 全体は分けて読むこと。** 実測は次のとおり。

  | コミット | 触ったパス | 撮影 | Playwright | hook 全体 |
  |---|---|---|---|---|
  | #1 章の骨格と文言 | `src/` | なし（旧 hook） | 247 件 | 1.6 分 |
  | #2 `shots.mjs` | `scripts/` | なし（旧 hook） | 247 件 | 1.8 分 |
  | #6 マニュアル UI | `src/views/` 他 | なし（旧 hook） | 247 件 | 1.1 分 |
  | #3 hook 自身 | `.githooks/` | なし（`UI_PATHS` に当たらない） | 248 件 | 59.6 秒 |
  | **#4 図 + 寸法** | `public/manual/` `src/` | **あり（10 枚）** | 248 件 | **57.6 秒**（うち Playwright 52.2 秒） |
  | 3 章 + 図 12 枚（rebase 後） | `src/` `public/manual/` | あり（12 枚） | 305 件 | 67 秒 |

  **[CI を `.githooks/pre-commit` で回す](ci-in-pre-commit.md) の「実測が 60 秒を超えたら Playwright を release 側へ移す判断になる」という閾値は、この PR より前から超えていた。** 撮影なしのコミットが 59.6 秒〜1.8 分かかっている。主因は E2E が 262 件（本 PR で +15）に増えたことで、**撮影の責任ではない。** 撮影の増分は、差分ゼロのとき数秒に留まる（`--only=manual` 単体で 3.4 秒、`cp -R` が走らないため）。閾値を評価するときはこの 2 つを混ぜないこと。

  **README の 3 枚だけ並列に混ぜてはいけないことが実測で分かった。** 3 コンテキスト（README / マニュアル ja / マニュアル en）を `Promise.all` で同時に走らせると `assets/1-record.png` が 2 種類のバイト列（142,692B / 142,688B）のあいだで揺れる。マニュアルの図は 1 枚も揺れない。差は待ち方にある — マニュアルの図は `scrollUnion` がスクロール量を整数へ丸め、待ちが全部条件待ちなのに対し、README の 3 枚は `scrollIntoView` が端数のスクロール位置を作り、グラフの落ち着きを `waitForTimeout(300)` の一律待ちで見ている。他の 2 コンテキストが CPU を奪うとどちらも足りなくなる。**`shootReadme` を `Promise.all` の外へ出して直列にした。** 負荷（`yes` 4 本）をかけた実測で、直列化前は 6 run 中 1 回ずれ、直列化後は 12 run（負荷あり 6 + なし 6）とも一致した。**pre-commit はこの直列化を 1 秒も払わない** — `--only=manual` では README を撮らないので、効くのは全枚数撮影と `--check` のときだけである。`--check` は `scripts/release.sh` の hard gate（この ADR の「`SKIP_HOOKS=1` の穴」の受け皿）なので、ここでの速さより決定性を優先した。

**rebase 後の実測でも同じ構図が繰り返された。** `main` から 3 機能（ドロップセット・種目ごとのラベル・新機能のお知らせ）が入り、マニュアルの章が 3 本・図が 2 枚増えた状態で計測すると、`node scripts/shots.mjs --only=manual` は **10 枚 3.51s → 12 枚 3.65s（+0.14s）**、hook 全体は **67 秒（E2E 305 件・図 12 枚）**。**60 秒を超えたが、増分の内訳は撮影 +0.14 秒・テスト件数 248 → 305 件で、支配項は今回もテストの件数である。** [CI を `.githooks/pre-commit` で回す](ci-in-pre-commit.md) の「60 秒を超えたら Playwright を release 側へ移す判断になる」という条件には到達したが、**判断そのものはこの PR では下さず、到達した事実と帰属を記録するに留める**（E2E の実行場所を動かすのは範囲外）。
- **rebase とマージでは走らない。** `git help rebase` は pre-rebase 以外の hook を約束しておらず、`git rebase --continue` の競合解消後は sequencer が `-n` を付けるので **pre-commit は走らない**（`cherry-pick --continue` は走る）。マージのコミットに掛かるのは `pre-merge-commit` で、それは `.githooks/` に置いていない。**GitHub 上のマージはそもそもローカル hook を通らない。** したがって次の穴が開く。

  | 経路 | pre-commit | 図の撮り直し |
  |---|---|---|
  | `git add` → `git commit` / `-a` | 走る | される |
  | `git commit <paths>`（partial） | 走る（false index 上） | **コミットには入るが実インデックスには入らない** |
  | `git commit --amend` | 走る | 元コミットに含まれていた UI 変更では再撮影されない |
  | `git rebase --continue` | **走らない** | されない |
  | ローカルの `git merge` の競合解消後の `git commit` | 走る | される |
  | GitHub 上のマージ | **走らない** | されない |

  **`scripts/shots.mjs --check` が唯一の受け皿である。** `scripts/release.sh` のブランチ作成前に置く（そこなら `die` しても後始末が要らず、クリーン要求とも整合する）。
- **partial commit では `git status` が二重に出る。** `git commit <paths>` のとき hook は一時的な false index の上で走るので、ここでの `git add` は実インデックスへ反映されない。図はコミットには入るが、直後の `git status` で `public/manual/*` が staged と unstaged の両方に出る。通常の `git add` → `git commit` と `-a` では起きない。
- **UI が変わって図の寸法が動くと 2 パスになる。** hook の順序は `cargo test` → `trunk build` → **撮影 + `git add`** → `playwright` である。`cargo test`（`src/manual.rs` の「宣言寸法 = 実ファイル」）は**撮影前の古い図**に対して通り、そのあと撮り直された新しい図で **E2E（配信物の `<img width height>` と実ファイルの比較）が落ちる**。直し方は `src/i18n.rs` の `fig` を新しい寸法に書き換えて再コミットすること。**これは寸法が動くたびに要る。** 最初に踏んだ人が `SKIP_HOOKS=1` に逃げないよう、hook のコメントと `src/manual.rs` の doc の両方に書いた。

  自動化する案は 2 つとも却下した。`shots.mjs` が寸法を生成ファイルに吐いて `include!` する案はビルド成果物が `src/` に入り、`width`/`height` を捨てて CSS の `aspect-ratio` にする案はレイアウトシフト防止と寸法ドリフト検出の両方を失う。
- **stage していない UI 変更があると、図はワークツリーの状態で撮られる。** `trunk build` も `playwright` も同じ性質なので新しい問題ではないが、**生成物をコミットに焼く**ぶん残る。だから警告を出す。止めないのは、誤検知で止まるガードが `SKIP_HOOKS=1` の常用を招くからである。
- **決定性を壊す要因が 4 つある。** どれも「壊れても挙動は正常なまま、git だけが太る」という静かな失敗をする。
  - **Playwright を上げると Chromium が変わる。** WebP のエンコーダは Chromium 内蔵の libwebp で、CDP の `Page.captureScreenshot` に投げているだけなので、revision が上がれば出力バイト列が変わりうる。だから `package.json` を exact pin にし、`package*.json` を `UI_PATHS` に入れた（上げた日に必ず撮り直る）
  - **macOS のフォント更新は exact pin では防げない。** システムフォントが変われば文字のラスタライズが変わる。単一マシン前提（[CI を `.githooks/pre-commit` で回す](ci-in-pre-commit.md) が既に依存している前提）にそのまま乗っている
  - **darwin と Linux ではフォントが違う。** エンコーダ自体は Chromium 内蔵なので platform 分岐は無い（`recodePngToWebp` の darwin 分岐は WebKit 側の実装で、Chromium は通らない）が、描画結果が同じである保証は無い。別 OS で撮ると全枚数が churn する可能性がある
  - **`SKIP_HOOKS=1`。** 撮影も飛ぶので図が UI から遅れる。強制はできない（[CI を `.githooks/pre-commit` で回す](ci-in-pre-commit.md) がそう決めている）
- **`.gitignore` に入れない。** `public/manual/*.webp` は `assets/*.png` や `public/og.png` と同じ**コミットする生成物**である。ignore すると配信物から消える。
- **debug と release、サブパス配信でもピクセルは一致した。** `DIST_DIR=dist-release E2E_BASE=/fitness-memo/` で `--check` を通したところ 13 枚中 12 枚がバイト一致した（残る 1 枚は撮影時点の実装差によるもので、debug/release の差ではない）。`release.sh` の hard gate として成立する。

## 検討した代替案

**毎コミット無条件に撮る**: パス判定が要らず、`UI_PATHS` の網羅漏れという失敗クラスが消える。しかし ADR だけ・`Cargo.toml` だけのコミットでも Playwright が 3 秒余分に走る。時計を固定してあるので blob は増えないが、増えないことに賭けた設計をわざわざ全コミットで試す理由がない。却下。

**撮らずに「図が古いかもしれない」を受容する**: 実装が最小。しかし [マニュアルの図は `public/` に配信するスクリーンショットにし、`<img>` で参照する](../architecture/manual-figures-as-served-screenshots.md) が「模式図だと図と実物の一致を人間が保証し続けることになる」を理由に実スクショを選んでいる。撮り直さないならその理由が消え、SVG の模式図に戻したほうがよい。却下。

**pre-push で撮る**: コミットが速いまま、push 前に 1 回だけ撮ればよい。しかし図の変更がコミットに含まれず、「push が勝手に作業ツリーを書き換える」形になる。どのコミットの図なのかも決まらない。却下。

**GitHub Actions で撮って push する**: 強制力があり、環境も固定されるので darwin/Linux の差も消える。**ワークフローファイルを書かない方針のため採らない**（[ワークフローファイルを書かない（ただし Actions 機能は無効化しない）](no-workflow-files.md)）。

**`--check` だけにして自動 `git add` をやめる**: hook がインデックスを触らなくなるので partial commit の二重表示も消え、挙動が読みやすい。しかし「落ちてから手で撮り直して再コミット」が UI を触るたびに必ず 1 往復増える。寸法が動くときは今でも 2 パスになるが、それは寸法が動くときだけで、通常は 0 往復で済んでいる。却下（ただし `--check` は `release.sh` に別途置いた）。

**README の 3 枚も pre-commit で撮る**: ドリフトの網が hook 側で完結し、`--check` の対象を絞れる。しかし 1 回 383KB × PR 内のコミット数が履歴に残る。マニュアルの図と桁が違うので、同じ扱いにはできない。却下。

**図を Git LFS に置く**: blob の肥大が本体リポジトリから消える。しかし GitHub Pages の branch deploy が LFS ポインタをそのまま配信するので、**図が壊れる**。clone に LFS のセットアップも要る。却下。
