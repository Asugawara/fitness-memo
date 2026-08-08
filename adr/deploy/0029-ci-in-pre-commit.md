# ADR-0029: CI を `.githooks/pre-commit` で回す

- **状態**: 採用
- **日付**: 2026-08-08
- **カテゴリ**: deploy
- **関連**: [ADR-0026](0026-no-workflow-files.md), [ADR-0003](../architecture/0003-wasm-target-scoped-dependencies.md)

## 背景

利用者の指示は「GitHub Actions を使わないようにして。**CI はプレコミットに含めるようにする**」だった（[ADR-0026](0026-no-workflow-files.md)）。ワークフローファイルを書かないので、自動検証はローカルの git hook に載せるしかない。

hook の有効化には仕組みが必要である。`.git/hooks/` はリポジトリに含まれないので、clone しただけでは効かない。

## 決定

**`.githooks/pre-commit` に検証を置き、`git config core.hooksPath .githooks` で有効化する。** 有効化は `scripts/setup.sh` が行う。

```sh
#!/bin/sh
set -eu
[ "${SKIP_HOOKS:-}" = "1" ] && exit 0

# main に docs/ を入れない（release 専用。混入するとマージが停止する）
if git diff --cached --name-only | grep -q '^docs/'; then
  echo "docs/ は release 専用。main にコミット禁止"; exit 1
fi

cargo fmt --all -- --check
cargo clippy --target wasm32-unknown-unknown --all-features -- -D warnings
cargo test                                    # core.rs の純ロジック（ホスト）
trunk build                                   # debug / 差分ビルド
npx playwright test --project=chromium --project=harness
```

重い検証（WebKit / iPhone エミュレーション / SW / オフライン）は `scripts/release.sh` が `main` → `release` の PR 作成前に実行する。

## 理由

- **`core.hooksPath` を使うのは、フック自体をリポジトリで管理するため。** husky のような追加ツールが不要で、`git config` 1 行で済む。フックの内容が git 管理下にあるので、変更が履歴に残る。
- **`docs/` ガードを最初に置いた。** `main` に `docs/` が混入すると 2 回目以降のマージが modify/delete コンフリクトで停止する（[ADR-0025](0025-github-pages-branch-deploy.md)）。他の検証より先に、速く落ちる。
- **`--target wasm32-unknown-unknown` を clippy に必ず付ける。** UI 層は wasm32 専用に cfg gate してあるので（[ADR-0003](../architecture/0003-wasm-target-scoped-dependencies.md)）、付け忘れると `views` / `storage` を一切見ないまま「clippy が通った」状態になる。
- **`cargo test` はホストターゲットで走る。** target 別 dependencies により leptos の巨大な依存グラフをビルドしないので数秒で終わる。ここが遅いと pre-commit 全体が使われなくなる。
- **`trunk build` は debug の差分ビルドにする。** `--release` は `wasm-opt` が走るので数十秒かかる。debug でもコンパイルエラーと `view!` マクロの展開失敗は検出できるので、pre-commit の目的（壊れたコードをコミットさせない）には足りる。
- **Playwright は Chromium 1 ブラウザに絞る。** 想定所要時間は差分ビルド時で 25〜50 秒。ここに WebKit と iPhone エミュを足すと数分になり、コミットのたびに待てない。役割を分けて重い側を release 時に寄せた。
- **`SKIP_HOOKS=1` の脱出口を用意した。** WIP コミットやリベース中に検証を待てない場面がある。脱出口がないと `--no-verify` が習慣化し、フック全体が無効化されるほうが危険である。
- **`release.sh` 内の `git commit` は `--no-verify` を付ける。** 付けないとリリースのたびに fmt/clippy/test/trunk build/playwright が二重で走る。

## 結果（トレードオフ）

- **ローカルの pre-commit が唯一の自動検証になる。** ここをすり抜けたバグは誰も気づかない。この帰結として E2E を厚めに敷き（12 ケース）、**v1 で追加すると決めた機能とデータモデルの危険なエッジケースを必ず含める**ことにした。具体的には `at: Option` の検証（過去日バックフィルで「たった今」にならないこと）、flush の検証（`visibilitychange` 発火後にリロードして残ること）、`Kind` ごとの単位、1 日 1 種目 1 ログ、`type="text"` の中間状態。
- **`SKIP_HOOKS=1` や `--no-verify` で簡単に飛ばせる。** 強制力がない。個人プロジェクトなので許容するが、「CI が通っていないコードが `main` に入りうる」ことは事実である。**push 時に再検証する仕組みがない**ので、飛ばしたまま push すると誰も気づかない。
- **他のマシンで clone しても `scripts/setup.sh` を実行するまでフックが効かない。** `core.hooksPath` はローカル設定なので clone では引き継がれない。単一開発者・単一マシンの前提に依存している。
- **コミットのたびに 25〜50 秒待つ。** 細かくコミットする習慣とは相性が悪い。実測が 60 秒を超えたら Playwright を release 側へ移す判断になる（検証手順に「`sh .githooks/pre-commit` を直接実行して所要時間を計測（60 秒以内か）」を入れた）。
- **`npx playwright test` が毎回静的サーバを起動する。** `playwright.config.mjs` の `webServer` が `reuseExistingServer: false` なので、古いビルドを掴んだまま使い回されない代わりに毎回起動コストがかかる。
- **`harness` project を別に走らせている。** `e2e/harness.spec.mjs` は `dist/` に依存せず自前で固定ポートのサーバを起動するので、他 project と並列させるとポートが衝突する。`--project=chromium --project=harness` と明示指定し、config 側で `testIgnore` / `testMatch` を切り分けた。テスト基盤自体の健全性を検証する層があるのは、pre-commit が唯一の防波堤である構成では妥当と判断した。

## 検討した代替案

**GitHub Actions で CI を回す**: 強制力があり、環境も再現される。push すれば必ず走るので飛ばせない。**利用者が明確に拒否したため採らない**（[ADR-0026](0026-no-workflow-files.md)）。

**pre-push フックにする**: コミットは速いまま、push 前に検証できる。飛ばしにくさも同等。しかし「壊れたコミットが履歴に残る」ことを許すので、`git bisect` が効きにくくなる。利用者の指示が「プレコミットに含める」だったので、そのまま従った。

**husky などのツールでフックを管理する**: `npm install` で自動的に有効化されるので `setup.sh` の実行忘れが起きない。しかし Node の依存が 1 つ増え、`core.hooksPath` で同じことができる。却下。

**pre-commit に全ブラウザの E2E を入れる**: 検出力は最大。しかし数分かかるのでコミットのたびに待てず、結果として `--no-verify` が習慣化する。検証の総量ではなく「実際に走る検証の量」を最大化するため、軽い側と重い側に分けた。

**フックを使わず手動で `sh .githooks/pre-commit` を実行する運用にする**: 柔軟だが、忘れる。自動化する価値が明確なので却下。
