# 記録タブの日付の横に開始–終了を薄字で出し、セットの時刻はメモを開いたときだけ出す

- **状態**: 採用
- **日付**: 2026-09-22
- **カテゴリ**: ux
- **関連**: [メモは種目カードのトグル 1 つで開き、閉じても薄字で残す](exercise-and-set-notes-behind-one-toggle.md), [破壊的操作は静止時に警告色を持たない（カード削除をフッタへ畳む）](destructive-affordance-quiet-at-rest.md), [記録タブをカレンダー + 選択日エディタの単一画面にする](record-tab-calendar-with-day-editor.md), [セットごとの `at` を当日の手入力時だけ埋め、コピーした行には持ち込まない](../data-model/set-at-typed-today-only.md)

## 背景

セットごとの時刻を持てるようになった（[セットごとの `at` を当日の手入力時だけ埋め、コピーした行には持ち込まない](../data-model/set-at-typed-today-only.md)）が、見せ方は別に決める必要があった。

14 アプリ（Strong / Hevy / FitNotes / Jefit / Fitbod / Liftosaur / RepCount ほか。出典: help.strongapp.io / hevyapp.com / fitnotesapp.com / support.jefit.com / help.fitbod.me / liftosaur.com/doc/api / getrepcount.app）を調査した結果、セット単位の時刻を画面に出すアプリは無かった。共通形は「ワークアウトの開始–終了（所要時間）をヘッダ / サマリーに薄く出す」+「セット完了はチェックの 1 タップ」で、前回値はプリフィル / プレースホルダとして出し、確認はそのチェックに統合されている。この調査から、ヘッダの開始–終了は業界の共通形、セット時刻は hidden-by-default が妥当と判断した。

利用者の選択（AskUserQuestion 済み）: 見せ方は「日ヘッダに開始–終了 + 『＋ メモ』を開いたときだけセット行に時刻」。コピー後に触らなかったセットは「時刻なしのまま」（捏造しない）。

## 決定

### 1. 日ヘッダに `day-span` を出す

`h2`（`today-date`）の直後に `<span class="day-span" data-testid="day-span">` を置く。`Memo`（`db` から選択日のセッションを引いて `core::day_span`）が打鍵ごとに 1 セッション分を走査する（`history` / `show_copy` と同級のコスト）。`span_text`: `fmt_clock(start)` と `fmt_clock(end)` が同じ文字列なら 1 つ、違えば `"{start}–{end}"`（U+2013、空白なし）。時刻の無い日は要素ごと出さない。過去日でも同じ位置（`back-to-today` の左）に出す。所要時間は出さない。

### 2. `views::mod::fmt_clock(ms: i64) -> String`

`chrono::DateTime::from_timestamp_millis` → `with_timezone(&Local)` → `%H:%M`（`views::backup::snapshot_time` と同じ手）。chrono 0.4 は default features（wasmbind）が効いているので `Local` はブラウザの実 TZ を使う。24 時間表記で両言語共通。`core` に `Local` を持ち込まない境界をここに置く。

### 3. CSS: `.day-span`

```css
.day-span { flex: 0 0 auto; margin-right: auto; font-size: 12px; color: var(--muted); font-variant-numeric: tabular-nums; white-space: nowrap; }
```

`.day-head` は `space-between` / `align-items: baseline` / nowrap のまま。`margin-right: auto` が余白を先に吸うので `h2` → 8px → `span` → 余白 → バッジ / 今日へ戻る の順で並ぶ。`flex-shrink: 0` が無いと nowrap で縮まずはみ出す。

### 4. セット行: メモを開いたときだけ `set-at` を出す

`note_open` が真の枝を `<input class="set-note">` と `<span class="set-at" data-testid="set-at">` の 2 ルートにする。`set-at` の中身は打った行なら `fmt_clock(at)`、コピーのまま触っていない行は空文字。**閉じているとき（`set-note-read` 側）は出さない**（DOM に無い）。開いているときは `None` でも空の `span` を出す（行ごとにメモ欄の幅を揃えるため）。

### 5. CSS: `.set-note` / `.set-at` の分離

共有していた `.set-note-read, .set-row .set-note { flex-basis: calc(100% - 23px); margin-left: 23px }` を分け、`.set-note-read` は据え置く（閉じた薄字の左端 23px を e2e が固定している）。

```css
.set-row .set-note { flex: 1 1 calc(100% - 23px - 60px); min-width: 0; margin-left: 23px; }
.set-at { flex: 0 0 auto; min-width: 3em; text-align: right; font-size: 12px; color: var(--muted); font-variant-numeric: tabular-nums; }
```

hypothetical outer（`.set-note` の basis `100% - 60px` + gap 6px + `.set-at` の概算 36px）= `100% - 18px` で、幅に依らず 2 行目に同居する（375 / 393 / 412px で確認済み）。✕ は DOM 順で前・1 行目・`margin-left: auto` のまま。`.warn`（basis 100%）と `.drop-row`（outer 100%）は独立行のまま。

### 6. 新しいボタン・設定項目・アイコンは足さない

`day-span` / `set-at` はどちらも読み取り専用の表示要素で、タップ標的を増やさない。i18n の新規文字列は `added_times` だけ（時刻そのものは "HH:MM" で言語非依存）。

## 理由

1 日 1 行なのでカード枚数に比例して縦が伸びない。メモのトグルへの相乗り 6 つ目（ピン・インターバル・種目メモ・セットメモ・段の口に続く）。利用者が書いた文字ではないので静止時に隠してよい（メモの薄字とは前提が違う — メモは「利用者が入力したので消してはいけない」観測、`set-at` は「システムが記録した」メタデータ）。

## 結果（トレードオフ）

- メモを開いたときメモ欄が約 42px 狭くなる（`.set-at` の幅ぶん）。
- トレ中は終了側が動く。
- 開かないと「何時に何を」は見えない。所要時間の暗算が要る。

## 検討した代替案

- **カードフッタに種目ごとの開始–終了** — 393px で 3 者（✕・フッタ・確認）の間隔 58px が約 13px に潰れ、[破壊的操作は静止時に警告色を持たない（カード削除をフッタへ畳む）](destructive-affordance-quiet-at-rest.md) の事故距離を割る。却下。
- **セット行 1 行目の ✕ の左** — 375px で折り返して ✕ の列が崩れる。却下。
- **過去日だけ出す** — 当日と過去日で見え方が変わり、記録の連続性が読みにくい。
- **設定でオンオフ** — 1 スイッチの価値に対して設定項目を増やすコストが見合わない。
- **常時セット行に出す** — 業界の共通形（hidden-by-default）から外れ、静止画面が常に賑やかになる。
- **所要時間を併記** — ヘッダがさらに長くなり、375px での折り返しリスクが増える。
- **TSV だけに出す** — 記録タブで見えないと日々の確認に使えない。
