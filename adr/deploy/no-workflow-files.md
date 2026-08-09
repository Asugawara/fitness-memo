# ワークフローファイルを書かない（ただし Actions 機能は無効化しない）

- **状態**: 採用
- **日付**: 2026-08-08
- **カテゴリ**: deploy
- **関連**: [GitHub Pages の branch deploy（`release` / `docs`）を使う](github-pages-branch-deploy.md), [CI を `.githooks/pre-commit` で回す](ci-in-pre-commit.md)

## 背景

本プロジェクトは **GitHub Actions のワークフローを自分で書かない**という方針で始まっている。CI はプレコミットで回す（[CI を `.githooks/pre-commit` で回す](ci-in-pre-commit.md)）。

問題は、この方針をどこまで進めてよいかである。素直に読むと「リポジトリ設定の Actions 機能そのものを off にする」まで含みそうに見える。

## 決定

**`.github/workflows/` を一切書かない。** CI は `.githooks/pre-commit` でローカル実行する（[CI を `.githooks/pre-commit` で回す](ci-in-pre-commit.md)）。

ただし **リポジトリ設定の Actions 機能は有効のままにする。**

## 理由

GitHub Pages の branch deploy（`build_type: "legacy"`）を選んでも、**Pages は内部で必ず `pages build and deployment` という Actions ワークフローを実行する**。GitHub の公式ドキュメントに明記されている。

> Your GitHub Pages site will always be deployed with a GitHub Actions workflow run, even if you've configured your GitHub Pages site to be built using a different CI tool.

したがってリポジトリ設定で Actions を無効化すると `Error: Actor is not allowed to trigger Actions workflows` が出て **Pages のデプロイ自体が止まる**（GitHub community #201551 に実例）。

つまり「Actions を使わない」は、**「自分でワークフローを書かない・管理しない」としてのみ実現可能**で、「Actions 機能を off にする」まで進めると要件（デプロイできること）と衝突する。

この注記が無いと、後からこの設定を触る人が「方針どおり Actions を切っておこう」と気を利かせた瞬間にデプロイ不能になる。原因も分かりにくい（Pages の設定は正しいのにサイトが更新されない）。そのため**明示的な禁止事項として記録する**。

## 結果（トレードオフ）

- Actions タブに `pages build and deployment` の実行履歴が並ぶ。これは正常な状態であり、異常ではない。
- マージからサイト反映まで、ビルド + CDN（`cache-control: max-age=600` を実測）で**最大 10 分程度ずれる**。「マージしたのにまだ古い」を不具合と誤認しないこと。判定は `curl -sI` の etag 変化で見る。
- ローカルの pre-commit が唯一の自動検証になるため、そこをすり抜けたバグは誰も気づかない。E2E を厚めに敷き、データモデルの危険なエッジケースを必ず含めることにした。

## 検討した代替案

**GitHub Actions で build → deploy する（`build_type: "workflow"`）**: 一般的な構成で、ビルド成果物をリポジトリにコミットしなくて済む。ワークフローファイルを書かない方針のため採らない。

**Actions 機能ごと無効化する**: 上記のとおりデプロイが止まる。実現不能。
