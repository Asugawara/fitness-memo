# Architecture Decision Records

このプロジェクトで下した意思決定の記録。カテゴリごとにディレクトリを分け、番号は全カテゴリ通し番号にしている。

## なぜ `docs/` ではなく `adr/` なのか

GitHub Pages の branch deploy は公開ディレクトリが `/` か `/docs` の2択しかなく、本プロジェクトは `release` ブランチの `/docs` をビルド成果物の配信元にしている。`scripts/release.sh` は毎回 `rm -rf docs` で作り直し、`.githooks/pre-commit` は `main` への `docs/` コミットを拒否する（これが無いと2回目以降のマージが modify/delete で停止する）。したがって `docs/` に文書を置くとデプロイ機構と衝突する。詳細は [ADR-0025](deploy/0025-github-pages-branch-deploy.md) と [ADR-0032](process/0032-adr-in-adr-directory.md)。

## フォーマット

各 ADR は「背景 / 決定 / 理由 / 結果（トレードオフ）/ 検討した代替案」で構成する。状態は `採用` / `置換済み` / `破棄` のいずれか。

## 索引

### architecture — 技術選定と全体構造

| # | タイトル | 状態 |
|---|---|---|
| [0001](architecture/0001-rust-leptos-csr-trunk.md) | Rust + Leptos (CSR) + trunk を採用する | 採用 |
| [0002](architecture/0002-no-router-tab-enum-signal.md) | ルーターを使わずタブを enum signal で切り替える | 採用 |
| [0003](architecture/0003-wasm-target-scoped-dependencies.md) | UI 依存を wasm32 の target 別 dependencies に置く | 採用 |
| [0004](architecture/0004-no-chart-library-hand-rolled-svg.md) | グラフライブラリを使わず SVG を自前で描く | 採用（`layout()` の置き場所は [0045](architecture/0045-chart-layout-as-a-testable-module.md) で修正） |
| [0041](architecture/0041-help-figures-as-included-svg.md) | ヘルプの図を `assets/` の SVG に置き `include_str!` + `inner_html` で挿す | 採用 |
| [0045](architecture/0045-chart-layout-as-a-testable-module.md) | グラフの座標計算を `chart_layout` に切り出してテスト可能にする | 採用 |

### data-model — データ構造と不変条件

| # | タイトル | 状態 |
|---|---|---|
| [0005](data-model/0005-session-keyed-by-local-date.md) | セッションをローカル日付文字列で BTreeMap に持つ | 採用 |
| [0006](data-model/0006-at-optional-same-day-only.md) | `at` を `Option<i64>` にし当日入力時のみ埋める | 採用 |
| [0007](data-model/0007-exercise-kind-explicit.md) | 指標の種類を種目の明示属性にする（推論しない） | 置換済み → [0033](data-model/0033-metric-is-a-view-setting.md) |
| [0008](data-model/0008-one-log-per-exercise-per-day.md) | 「1日1種目1ログ」を不変条件にする | 採用 |
| [0009](data-model/0009-group-metric-is-set-count.md) | 部位別の指標を volume ではなくセット数にする | 置換済み → [0033](data-model/0033-metric-is-a-view-setting.md) |
| [0010](data-model/0010-sequential-ids-no-uuid.md) | ID を `next_id` の連番にし uuid を使わない | 置換済み → [0037](data-model/0037-random-ids-for-safe-merge.md) |
| [0037](data-model/0037-random-ids-for-safe-merge.md) | ID を 60 bit 乱数にし、プリセットには固定 ID を与える | 採用 |
| [0033](data-model/0033-metric-is-a-view-setting.md) | 指標を種目の属性ではなくグラフの表示設定にする | 採用 |

### storage — 永続化

| # | タイトル | 状態 |
|---|---|---|
| [0011](storage/0011-localstorage-single-key-json.md) | localStorage の単一キーに JSON 全体を持つ | 採用（容量超過の握りつぶしは撤回） |
| [0012](storage/0012-quarantine-on-parse-failure.md) | パース失敗時は上書きせず退避する | 採用 |
| [0013](storage/0013-flush-on-visibilitychange.md) | `visibilitychange` の hidden で debounce を flush する | 採用 |
| [0014](storage/0014-defer-export-import.md) | JSON エクスポート/インポートを v1 に入れない | 履行済み → [0038](storage/0038-share-sheet-over-download.md) |
| [0034](storage/0034-storage-key-per-schema-generation.md) | 保存キーを schema 世代ごとに切り、旧キーを読み取り専用で残す | 採用 |
| [0038](storage/0038-share-sheet-over-download.md) | 書き出しは共有シートを主経路にし、iOS では `<a download>` を使わない | 採用 |
| [0039](storage/0039-no-same-origin-redundancy.md) | 同一オリジン内の多層バックアップを採用しない | 採用 |
| [0042](storage/0042-ui-state-in-separate-key.md) | UI の状態を `Db` に入れず別キーに置く | 採用 |

### pwa — オフライン動作と iOS 実機

| # | タイトル | 状態 |
|---|---|---|
| [0015](pwa/0015-sw-atomic-shell-swap.md) | Service Worker はシェル全体を BUILD_ID で原子的に入れ替える | 採用 |
| [0016](pwa/0016-sw-explicit-navigate-branch.md) | fetch ハンドラで navigate を明示分岐する | 採用 |
| [0017](pwa/0017-sw-update-on-visible.md) | visible 復帰で `reg.update()` を呼ぶ | 採用 |
| [0018](pwa/0018-no-sw-in-dev.md) | 開発サーバ（ポート 8080）では SW を登録しない | 採用 |
| [0019](pwa/0019-hide-tabs-when-keyboard-open.md) | キーボード表示中はボトムタブを隠す | 採用 |
| [0020](pwa/0020-manifest-relative-urls.md) | manifest の URL を全て相対にする | 採用 |

### ux — 画面と操作

| # | タイトル | 状態 |
|---|---|---|
| [0021](ux/0021-copy-button-only-when-empty.md) | 「前回をコピー」はセットが空のときだけ出す | 採用 |
| [0022](ux/0022-pre-workout-and-in-workout-exclusive.md) | トレ前情報とトレ中情報を排他表示にする | 置換済み → [0035](ux/0035-record-tab-calendar-with-day-editor.md) |
| [0023](ux/0023-text-input-not-number.md) | 数値入力に `type="number"` を使わない | 採用 |
| [0024](ux/0024-calendar-add-from-empty-day.md) | カレンダーの空日からも記録を追加できるようにする | 採用（導線は [0035](ux/0035-record-tab-calendar-with-day-editor.md) で改訂） |
| [0035](ux/0035-record-tab-calendar-with-day-editor.md) | 記録タブをカレンダー + 選択日エディタの単一画面にする | 採用 |
| [0036](ux/0036-set-entry-prefill-and-focus.md) | セット追加は直前行の重量をコピーして回数欄へフォーカスする | 採用 |
| [0037](ux/0037-copy-whole-day-menu.md) | 1 日分のメニューは候補リストから 1 タップで丸ごとコピーする | 採用 |
| [0040](ux/0040-install-guide-banner-and-sheet.md) | ホーム画面への追加の案内を記録タブ末尾のバナー + 手順シートにする | 採用 |
| [0043](ux/0043-destructive-affordance-quiet-at-rest.md) | 破壊的操作は静止時に警告色を持たない（カード削除をフッタへ畳む） | 採用 |
| [0044](ux/0044-body-weight-second-axis-always-on.md) | 体重を推移グラフの第2軸に常時重ねる | 採用 |

### deploy — 配信とブランチ運用

| # | タイトル | 状態 |
|---|---|---|
| [0025](deploy/0025-github-pages-branch-deploy.md) | GitHub Pages の branch deploy（`release` / `docs`）を使う | 採用 |
| [0026](deploy/0026-no-workflow-files.md) | ワークフローファイルを書かない（Actions 機能は無効化しない） | 採用 |
| [0027](deploy/0027-release-branch-from-main.md) | `release` を `main` から派生させ orphan 運用にしない | 採用 |
| [0028](deploy/0028-force-merge-commit-only.md) | マージ方式を merge コミットのみに固定する | 採用 |
| [0029](deploy/0029-ci-in-pre-commit.md) | CI を `.githooks/pre-commit` で回す | 採用 |

### process — 進め方

| # | タイトル | 状態 |
|---|---|---|
| [0030](process/0030-adversarial-review-before-implementation.md) | 実装前に敵対的レビューを回す | 採用 |
| [0031](process/0031-herdr-wave-parallelism.md) | Herdr の波状並列でファイル所有権を分けて実装する | 採用 |
| [0032](process/0032-adr-in-adr-directory.md) | ADR を `adr/` にカテゴリ別で置く | 採用 |
