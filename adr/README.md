# Architecture Decision Records

このプロジェクトで下した意思決定の記録。カテゴリごとにディレクトリを分けている。

## なぜ `docs/` ではなく `adr/` なのか

GitHub Pages の branch deploy は公開ディレクトリが `/` か `/docs` の2択しかなく、本プロジェクトは `release` ブランチの `/docs` をビルド成果物の配信元にしている。`scripts/release.sh` は毎回 `rm -rf docs` で作り直し、`.githooks/pre-commit` は `main` への `docs/` コミットを拒否する（これが無いと2回目以降のマージが modify/delete で停止する）。したがって `docs/` に文書を置くとデプロイ機構と衝突する。詳細は [GitHub Pages の branch deploy（`release` / `docs`）を使う](deploy/github-pages-branch-deploy.md) と [ADR を `adr/` にカテゴリ別で置く](process/adr-in-adr-directory.md)。

## フォーマット

各 ADR は「背景 / 決定 / 理由 / 結果（トレードオフ）/ 検討した代替案」で構成する。状態は `採用` / `置換済み` / `破棄` のいずれか。

**ファイル名は内容を表すスラッグだけにし、番号は振らない。** 番号は「次の空き番号」を取り合うので、ブランチが分かれると必ず衝突する（[ADR を `adr/` にカテゴリ別で置く](process/adr-in-adr-directory.md)）。ADR を指すときは `adr/<カテゴリ>/<スラッグ>.md` のパスを使う。

## 索引

### architecture — 技術選定と全体構造

| タイトル | 状態 |
|---|---|
| [Rust + Leptos (CSR) + trunk を採用する](architecture/rust-leptos-csr-trunk.md) | 採用 |
| [ルーターを使わずタブを enum signal で切り替える](architecture/no-router-tab-enum-signal.md) | 採用 |
| [UI 依存を wasm32 の target 別 dependencies に置く](architecture/wasm-target-scoped-dependencies.md) | 採用 |
| [グラフライブラリを使わず SVG を自前で描く](architecture/no-chart-library-hand-rolled-svg.md) | 採用（`layout()` の置き場所は [グラフの座標計算を `chart_layout` に切り出してテスト可能にする](architecture/chart-layout-as-a-testable-module.md) で修正） |
| [ヘルプの図を `assets/` の SVG に置き `include_str!` + `inner_html` で挿す](architecture/help-figures-as-included-svg.md) | 採用 |
| [グラフの座標計算を `chart_layout` に切り出してテスト可能にする](architecture/chart-layout-as-a-testable-module.md) | 採用 |
| [ブラウザサポートは Safari を基準にし、polyfill を入れない](architecture/browser-support-policy.md) | 採用 |
| [読み込みと操作は実測して、何も入れないと決めた](architecture/measure-before-optimizing-and-do-nothing.md) | 採用 |
| [アイコンに lucide を採り、`assets/icons/*.svg` を `include_str!` で埋め込む](architecture/lucide-icons-as-included-svg.md) | 採用 |

### data-model — データ構造と不変条件

| タイトル | 状態 |
|---|---|
| [セッションをローカル日付文字列で BTreeMap に持つ](data-model/session-keyed-by-local-date.md) | 採用 |
| [`at` を `Option<i64>` にし当日入力時のみ埋める](data-model/at-optional-same-day-only.md) | 採用（表示規則は [経過日数をローカル暦の日差にし、時刻粒度を同じ日の中だけに閉じる](data-model/elapsed-in-local-calendar-days.md) で改訂） |
| [指標の種類を種目の明示属性にする（推論しない）](data-model/exercise-kind-explicit.md) | 置換済み → [指標を種目の属性ではなくグラフの表示設定にする](data-model/metric-is-a-view-setting.md) |
| [「1日1種目1ログ」を不変条件にする](data-model/one-log-per-exercise-per-day.md) | 採用 |
| [部位別の指標を volume ではなくセット数にする](data-model/group-metric-is-set-count.md) | 置換済み → [指標を種目の属性ではなくグラフの表示設定にする](data-model/metric-is-a-view-setting.md) |
| [ID を `next_id` の連番にし uuid を使わない](data-model/sequential-ids-no-uuid.md) | 置換済み → [ID を 60 bit 乱数にし、プリセットには固定 ID を与える](data-model/random-ids-for-safe-merge.md) |
| [ID を 60 bit 乱数にし、プリセットには固定 ID を与える](data-model/random-ids-for-safe-merge.md) | 採用 |
| [指標を種目の属性ではなくグラフの表示設定にする](data-model/metric-is-a-view-setting.md) | 採用 |
| [経過日数をローカル暦の日差にし、時刻粒度を同じ日の中だけに閉じる](data-model/elapsed-in-local-calendar-days.md) | 採用 |
| [テキスト取り込みは「足すだけ」に固定し、部位を増やさず `at` を書かない](data-model/text-import-is-merge-only.md) | 破棄（[取り込みごと撤去](ux/migrate-by-ocr-paste.md)） |
| [種目メモとセットメモを `ExerciseLog` / `SetEntry` に持たせ、空のメモは書き出さない](data-model/notes-on-logs-and-sets.md) | 採用 |
| [トレーニングメニューを「名前 + 種目 ID の並び」だけのデータにする](data-model/routines-as-named-exercise-lists.md) | 採用 |

### storage — 永続化

| タイトル | 状態 |
|---|---|
| [localStorage の単一キーに JSON 全体を持つ](storage/localstorage-single-key-json.md) | 採用（容量超過の握りつぶしと「保存形式 = 書き出し形式」は撤回） |
| [パース失敗時は上書きせず退避する](storage/quarantine-on-parse-failure.md) | 採用（UI からの救出導線は撤回） |
| [`visibilitychange` の hidden で debounce を flush する](storage/flush-on-visibilitychange.md) | 採用 |
| [JSON エクスポート/インポートを v1 に入れない](storage/defer-export-import.md) | 履行済み → [書き出しは共有シートを主経路にし、iOS では `<a download>` を使わない](storage/share-sheet-over-download.md) |
| [保存キーを schema 世代ごとに切り、旧キーを読み取り専用で残す](storage/storage-key-per-schema-generation.md) | 採用 |
| [書き出しは共有シートを主経路にし、iOS では `<a download>` を使わない](storage/share-sheet-over-download.md) | 採用（textarea の常設は撤回） |
| [書き出しを TSV にし、保存形式（JSON）と分ける](storage/tsv-export-for-spreadsheets.md) | 採用 |
| [取り込みは「足すだけ」に固定する](storage/import-is-merge-only.md) | 採用 |
| [同一オリジン内の多層バックアップを採用しない](storage/no-same-origin-redundancy.md) | 採用 |
| [UI の状態を `Db` に入れず別キーに置く](storage/ui-state-in-separate-key.md) | 採用 |

### pwa — オフライン動作と iOS 実機

| タイトル | 状態 |
|---|---|
| [Service Worker はシェル全体を BUILD_ID で原子的に入れ替える](pwa/sw-atomic-shell-swap.md) | 採用 |
| [fetch ハンドラで navigate を明示分岐する](pwa/sw-explicit-navigate-branch.md) | 採用 |
| [visible 復帰で `reg.update()` を呼ぶ](pwa/sw-update-on-visible.md) | 採用 |
| [開発サーバ（ポート 8080）では SW を登録しない](pwa/no-sw-in-dev.md) | 採用 |
| [キーボード表示中はボトムタブを隠す](pwa/hide-tabs-when-keyboard-open.md) | 採用 |
| [manifest の URL を全て相対にする](pwa/manifest-relative-urls.md) | 採用 |

### ux — 画面と操作

| タイトル | 状態 |
|---|---|
| [「前回をコピー」はセットが空のときだけ出す](ux/copy-button-only-when-empty.md) | 採用 |
| [トレ前情報とトレ中情報を排他表示にする](ux/pre-workout-and-in-workout-exclusive.md) | 置換済み → [記録タブをカレンダー + 選択日エディタの単一画面にする](ux/record-tab-calendar-with-day-editor.md) |
| [数値入力に `type="number"` を使わない](ux/text-input-not-number.md) | 採用 |
| [カレンダーの空日からも記録を追加できるようにする](ux/calendar-add-from-empty-day.md) | 採用（導線は [記録タブをカレンダー + 選択日エディタの単一画面にする](ux/record-tab-calendar-with-day-editor.md) で改訂） |
| [記録タブをカレンダー + 選択日エディタの単一画面にする](ux/record-tab-calendar-with-day-editor.md) | 採用 |
| [セット追加は直前行の重量をコピーして回数欄へフォーカスする](ux/set-entry-prefill-and-focus.md) | 採用（削除確認の判定は [セット削除は確認を挟まない（カード削除の確認は残す）](ux/set-delete-without-confirmation.md) で改訂） |
| [1 日分のメニューは候補リストから 1 タップで丸ごとコピーする](ux/copy-whole-day-menu.md) | 採用（候補の出所は [保存したメニューから始める（種目タブを設定タブに改める）](ux/start-from-a-saved-routine.md) で拡張） |
| [ホーム画面への追加の案内を記録タブ末尾のバナー + 手順シートにする](ux/install-guide-banner-and-sheet.md) | 採用 |
| [破壊的操作は静止時に警告色を持たない（カード削除をフッタへ畳む）](ux/destructive-affordance-quiet-at-rest.md) | 採用 |
| [体重を推移グラフの第2軸に常時重ねる](ux/body-weight-second-axis-always-on.md) | 採用 |
| [セット削除は確認を挟まない（カード削除の確認は残す）](ux/set-delete-without-confirmation.md) | 採用 |
| [`color-scheme` を宣言し、クラスなしの `<button>` を作らない](ux/declare-color-scheme-for-ua-widgets.md) | 採用（`input[type=file]` は受益者から外れた） |
| [シートをネイティブ `<dialog>` にし、手動の重なり順から降りる](ux/native-dialog-for-sheets.md) | 採用 |
| [タブ切替に方向つき View Transition を掛ける](ux/directional-tab-transitions.md) | 破棄（0.2s の演出が視線を取るので当日撤去。切替は即時） |
| [フォーカスリングを明示し、記録タブの見出しを 1 本の階層にする](ux/focus-ring-and-heading-order.md) | 採用 |
| [他アプリからの移行はスクショの文字起こしを貼り付けて受ける](ux/migrate-by-ocr-paste.md) | 破棄（読み取りが成立せず操作量も見合わないため撤去） |
| [種目タブを部位の折りたたみ一覧にし、1 つだけ開く](ux/menu-groups-as-single-open-accordion.md) | 採用（タブ名は [保存したメニューから始める（種目タブを設定タブに改める）](ux/start-from-a-saved-routine.md) で「設定」に改称、置き場所は [設定タブの入口を節の一覧にし、中身は 1 階層下ろす](ux/settings-as-a-list-of-sections.md) で「種目」節の中へ） |
| [メモは種目カードのトグル 1 つで開き、閉じても薄字で残す](ux/exercise-and-set-notes-behind-one-toggle.md) | 採用 |
| [記録タブのカードとセットをドラッグで並び替え、`Vec` の並びをそのまま保存する](ux/drag-to-reorder-in-record-tab.md) | 採用 |
| [保存したメニューから始める（種目タブを設定タブに改める）](ux/start-from-a-saved-routine.md) | 採用（メニューを作る導線は [その日の記録から直接メニューを作れるようにする](ux/save-a-day-as-a-routine.md)、画面構成は [設定タブの入口を節の一覧にし、中身は 1 階層下ろす](ux/settings-as-a-list-of-sections.md)、編集シートの操作は [メニュー編集シートの「選択中」をドラッグで並べ替え、種目ピッカーを複数開けるアコーディオンにする](ux/routine-editor-drag-and-accordion.md) で拡張） |
| [その日の記録から直接メニューを作れるようにする](ux/save-a-day-as-a-routine.md) | 採用 |
| [設定タブの入口を節の一覧にし、中身は 1 階層下ろす](ux/settings-as-a-list-of-sections.md) | 採用 |
| [書き出し / 読み込みを 1 画面に畳み、逃げ道 UI を常設しない](ux/one-screen-export-import.md) | 採用 |
| [`<input type="file">` を視覚的に隠し、ボタンから `click()` する](ux/hidden-file-input-behind-a-button.md) | 採用 |
| [メニュー編集シートの「選択中」をドラッグで並べ替え、種目ピッカーを複数開けるアコーディオンにする](ux/routine-editor-drag-and-accordion.md) | 採用 |
| [マシンのピンは種目に持たせ、メモのトグルに相乗りさせる](ux/machine-pins-on-the-exercise.md) | 採用 |

### deploy — 配信とブランチ運用

| タイトル | 状態 |
|---|---|
| [GitHub Pages の branch deploy（`release` / `docs`）を使う](deploy/github-pages-branch-deploy.md) | 採用 |
| [ワークフローファイルを書かない（Actions 機能は無効化しない）](deploy/no-workflow-files.md) | 採用 |
| [`release` を `main` から派生させ orphan 運用にしない](deploy/release-branch-from-main.md) | 採用 |
| [マージ方式を merge コミットのみに固定する](deploy/force-merge-commit-only.md) | 採用 |
| [CI を `.githooks/pre-commit` で回す](deploy/ci-in-pre-commit.md) | 採用 |

### seo — 検索エンジンと SNS への見え方

| タイトル | 状態 |
|---|---|
| [クローラ向けメタデータを本番 URL のハードコードで持ち、オフラインシェルから外す](seo/crawler-metadata-and-hardcoded-origin.md) | 採用 |

### process — 進め方

| タイトル | 状態 |
|---|---|
| [実装前に敵対的レビューを回す](process/adversarial-review-before-implementation.md) | 採用 |
| [Herdr の波状並列でファイル所有権を分けて実装する](process/herdr-wave-parallelism.md) | 採用 |
| [ADR を `adr/` にカテゴリ別で置く](process/adr-in-adr-directory.md) | 採用 |
