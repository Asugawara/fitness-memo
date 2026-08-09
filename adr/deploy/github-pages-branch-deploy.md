# GitHub Pages の branch deploy（`release` / `docs`）を使う

- **状態**: 採用
- **日付**: 2026-08-08
- **カテゴリ**: deploy
- **関連**: [ワークフローファイルを書かない（ただし Actions 機能は無効化しない）](no-workflow-files.md), [`release` を `main` から派生させ orphan 運用にしない](release-branch-from-main.md), [マージ方式を merge コミットのみに固定する](force-merge-commit-only.md)

## 背景

利用者の指示は 2 つあった。

- 「GitHub Actions を使わないようにして」（[ワークフローファイルを書かない（ただし Actions 機能は無効化しない）](no-workflow-files.md)）
- 「特定ブランチに push すればデプロイになる方式」「開発ブランチからそのブランチへの PR を作成する際に、時間のかかる E2E テストを行う」

ワークフローを書かずに GitHub Pages へ配信する手段は branch deploy（`build_type: "legacy"`）だけである。公開ディレクトリは **`/` か `/docs` の 2 択**しかない。

リポジトリの実測状態は次のとおりだった。

| 項目 | 現状 |
|---|---|
| `github.com/Asugawara/fitness-memo` | **既に作成済み**（`gh repo create` は実行しない） |
| 可視性 | `PUBLIC`（Pages が Pro なしで使える） |
| 中身 | `isEmpty: true`。ブランチ 0、default branch 未設定 |
| ローカルの remote | 未設定 |
| Pages | 未設定（404） |

## 決定

**`release` ブランチの `/docs` を Pages の配信元にする。**

| ブランチ | 内容 |
|---|---|
| `main` | ソースのみ。`dist/` は `.gitignore`。**`docs/` を絶対に持たない** |
| `release` | `main` の内容 + `docs/`（ビルド成果物）。**Pages の配信元** |

公開 URL は `https://asugawara.github.io/fitness-memo/`。

**Phase 5 の手順は順序を守る。**

```sh
git remote add origin https://github.com/Asugawara/fitness-memo.git
git push -u origin main
gh api -X PATCH repos/Asugawara/fitness-memo \
  -F allow_squash_merge=false -F allow_rebase_merge=false -F allow_merge_commit=true
sh scripts/bootstrap-release.sh        # docs/ 入りの release を作る
echo '{"build_type":"legacy","source":{"branch":"release","path":"/docs"}}' \
  | gh api -X POST repos/Asugawara/fitness-memo/pages --input -
```

## 理由

- **`/docs` を選んだのは `/`（ルート）だとソースと成果物が混ざるため。** ルート配信にすると `Cargo.toml` や `src/` が公開ディレクトリに並び、`index.html` の隣にソースが置かれる。`/docs` なら成果物が 1 ディレクトリに閉じる。
- **`docs/` 入りの `release` を push した後に Pages を有効化する。** GitHub Docs は branch deploy の前提として「the branch you want to use as your publishing source already exists in your repository」と明記し、「`/docs` を選んで後から消すと missing /docs folder のビルドエラーになる」とも書いている。順序を逆にすると詰まる。
- **`build_type: "legacy"` は 2026 年 8 月時点で廃止も非推奨宣言もされていない。** 2024 年 6 月に廃止されたのは Jekyll ビルドの旧 legacy worker 基盤であって、branch deploy 機能そのものではない。混同しやすいので記録しておく。
- **`.nojekyll` を置く。** trunk の出力には `_` 始まりのファイルが生まれうるが、Jekyll は既定でそれを無視する。`touch docs/.nojekyll` で Jekyll 処理そのものを止める。
- **public リポジトリのままにする。** Pages を無料アカウントで使うには public である必要がある。個人の筋トレ記録アプリのソースを公開することになるが、**データはリポジトリに一切入らない**（`localStorage` のみ。[localStorage の単一キーに JSON 全体を持つ](../storage/localstorage-single-key-json.md)）ので、公開されるのはコードだけである。
- **`main` に `docs/` を持たせないことが `docs/` 保持の保証の前提になる。** 2 回目以降の merge-base は「前回マージで取り込んだ `main` のコミット」で、そこに `docs/` は無い。base に無く ours（`release` 側）にのみ存在するパスは 3-way マージで「ours 側の追加」として保持される。`main` に `docs/` が誤って入ると modify/delete コンフリクトでマージが停止する。`.githooks/pre-commit` の先頭でガードしている。

## 結果（トレードオフ）

- **ビルド成果物をリポジトリにコミットする。** wasm + js + css + icons が毎リリース分だけ履歴に積まれる。opt-level="z" + wasm-opt で圧縮しているとはいえ数百 KB / リリースなので、リリース回数が増えると履歴が膨らむ。個人用アプリのリリース頻度なら問題にならない。
- **リリース手順が `scripts/release.sh` に依存する。** 手作業で `docs/` を作ると内容がずれる。スクリプトが `trunk build --release --public-url /fitness-memo/` から `git push` と PR 作成まで一貫して行う。
- **公開ディレクトリが `/docs` なので、リポジトリ内の `docs/` を文書置き場として使えない。** ADR を `adr/` に置いた理由がこれである（[ADR を `adr/` にカテゴリ別で置く](../process/adr-in-adr-directory.md)）。一般的な慣習（`docs/adr/`）から外れるが、デプロイ機構との衝突を避けるほうが優先度が高い。
- **サブパス配信（`/fitness-memo/`）になるので、パス関連のバグがローカルで再現しない。** `trunk build --public-url` で js/wasm は解決されるが、manifest と SW は別の対策が必要になった（[manifest の URL を全て相対にする](../pwa/manifest-relative-urls.md), [fetch ハンドラで navigate を明示分岐する](../pwa/sw-explicit-navigate-branch.md)）。重い側 E2E を `E2E_BASE=/fitness-memo/` で走らせて本番と同じパス構成を再現する。
- **マージからサイト反映まで最大 10 分ずれる。** Pages のビルドと CDN（実測 `cache-control: max-age=600`）による。「マージしたのに古い」を不具合と誤認しないよう検証手順に明記した。
- **`release` ブランチ不在で `POST /pages` を叩いた際の正確なエラーコード（409 か 422 か）は未実測である。** 公式 docs は「ブランチが事前に存在すること」を要求すると明記するのみ。上記の順序を守れば踏まない。

## 検討した代替案

**GitHub Actions で build → deploy（`build_type: "workflow"`）**: 一般的な構成で、成果物をコミットしなくて済み、履歴も汚れない。**利用者が明確に拒否したため採らない**（[ワークフローファイルを書かない（ただし Actions 機能は無効化しない）](no-workflow-files.md)）。

**`main` の `/docs` を配信元にする**: ブランチが 1 本で済み、bootstrap も不要。しかし「開発ブランチから配信ブランチへの PR で重い E2E を走らせる」という要件のフローが作れない。却下。

**`gh-pages` ブランチ（`/` 配信）**: Pages の伝統的な構成で、成果物だけを持つブランチになる。しかし orphan 相当の運用になり [`release` を `main` から派生させ orphan 運用にしない](release-branch-from-main.md) の 3 つの落とし穴を踏む。加えて PR の diff が別ツリー同士になりレビューできない。却下。

**Netlify / Cloudflare Pages / Vercel**: 設定が楽でサブパス問題も起きない。しかし外部サービスのアカウントが増え、「GitHub だけで完結する」という暗黙の前提から外れる。利用者から要望がなかったので検討のみ。

**リポジトリを private にする**: ソースが公開されない。しかし無料アカウントでは Pages が使えない。データはリポジトリに入らないので public のリスクは小さいと判断した。
