# `release` を `main` から派生させ orphan 運用にしない

- **状態**: 採用
- **日付**: 2026-08-08
- **カテゴリ**: deploy
- **関連**: [GitHub Pages の branch deploy（`release` / `docs`）を使う](github-pages-branch-deploy.md), [マージ方式を merge コミットのみに固定する](force-merge-commit-only.md)

## 背景

配信の要件は「特定ブランチへの push がデプロイになる」「開発ブランチからそのブランチへの PR で、時間のかかる E2E テストを走らせる」の 2 つである。

これを満たす構成は次の 2 ブランチになる。

| ブランチ | 内容 |
|---|---|
| `main` | ソースのみ。`docs/` を持たない |
| `release` | `main` の内容 + `docs/`（ビルド成果物）。Pages の配信元 |

`release` をどう作るかで、当初は `git switch --orphan release` を考えた。ビルド成果物だけの独立した履歴にするのが素直に思えたためである。

## 決定

**`release` は `main` の初回コミット後に `main` から派生させる**。orphan にはしない。

```sh
git switch --orphan release   # ← これはやらない
```

既に orphan で作ってしまった場合に限り、初回だけ `git merge --allow-unrelated-histories --no-ff main` で救済する。

`scripts/release.sh` は毎回 `origin/release` を起点にする（ローカル `release` の鮮度に依存しない）。

```sh
git fetch origin
git switch -c "release-$(date +%Y%m%d-%H%M)" origin/release
git merge --no-ff main
```

## 理由

orphan には 3 つの独立した欠陥が重なっていた。

1. **`git switch --orphan` は unborn ブランチを作る。** コミットを 1 つ作るまで ref が生まれないので、そのままでは push できない（`src refspec release does not match any`）。「`release` を作ってから起点にする」という手順自体が成立しない。
2. **orphan は `main` と共通祖先を持たないので `git merge` が必ず失敗する。** Git 2.9 以降 `git merge` は共通祖先のない履歴のマージを既定で拒否する（`fatal: refusing to merge unrelated histories`。`--no-ff` を付けても同じ）。しかも**初回だけ失敗し 2 回目以降は通る**という非対称な挙動になるので、テストで見つけにくい。
3. **`git switch --orphan` は tracked ファイルを作業ツリーから全て消す。** `.gitignore` も消えるため、この状態で `git add -A` すると ignore されなくなった `target/`（数 GB）・`node_modules/`・`dist/` が stage される。public リポジトリへの巨大なゴミ混入か push 失敗で終わる。

`main` から派生させれば 3 つとも起きない。そして orphan にする**利点は元々なかった**。「`main` は `docs/` を持たないのでマージで `docs/` が削除扱いにならない」という本構成の要は、共通祖先があっても全く同様に成立する。3-way マージは「merge base に無く、theirs（main）にも無く、ours（release）にだけあるパス」を ours 側の追加として保持するからである。

## 結果（トレードオフ）

- `release` の履歴に `main` の全コミットが載る。ビルド成果物専用ブランチとしては冗長だが、**PR の diff が「ソース変更 + `docs/` の差分」になりレビューできる**ので、「PR 作成時に重い E2E」という要件にはむしろ合う。
- `docs/` 保持の保証を壊す経路が 2 つ残るので、両方を塞いだ。
  - squash / rebase マージ → [マージ方式を merge コミットのみに固定する](force-merge-commit-only.md) でリポジトリ設定から禁止
  - `main` への `docs/` 誤コミット → `.githooks/pre-commit` のガードで拒否
- `release.sh` 内の `git commit` は `--no-verify` を付ける。付けないと pre-commit が再発火して fmt/clippy/test/trunk build/playwright がリリースのたびに二重で走る。

## 検討した代替案

**orphan `release`（成果物のみ）**: 履歴が綺麗になるが、上記 3 欠陥に加えて `main` → `release` の PR が「全く別のツリー同士の diff」になり、要件である PR レビューが成立しない。却下。

**`main` の `/docs` を Pages 配信元にする**: ブランチが 1 本で済むが、PR フローが作れない。却下。
