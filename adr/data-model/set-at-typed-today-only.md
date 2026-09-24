# セットごとの `at` を当日の手入力時だけ埋め、コピーした行には持ち込まない

- **状態**: 採用
- **日付**: 2026-09-22
- **カテゴリ**: data-model
- **関連**: [`at` を `Option<i64>` にし当日入力時のみ埋める](at-optional-same-day-only.md)（ログの `at` はそのまま。セット単位を足す）, [経過日数をローカル暦の日差にし、時刻粒度を同じ日の中だけに閉じる](elapsed-in-local-calendar-days.md), [コピーは種目メモとセットメモを持ち込む（体調メモと体重は持ち込まない）](../ux/copy-carries-the-notes.md)（決定 6「運ばない」の適用先が 1 階層下がる）, [種目メモとセットメモを `ExerciseLog` / `SetEntry` に持たせ、空のメモは書き出さない](notes-on-logs-and-sets.md), [ドロップセットをメインセットにぶら下がる段（`SetEntry.drops`）として持つ](drop-sets-as-stages-under-the-main-set.md)（`skip_serializing_if` の同型）

## 背景

種目を記録したときの時刻を、端末のタイムゾーンで自動で残したいという要望がある。「前回をコピー」して同じ重量・回数しかやらなかったセットは入力イベントが起きないので、時刻が取れない場合がある。

`ExerciseLog.at: Option<i64>`（epoch ms）は既にある（[`at` を `Option<i64>` にし当日入力時のみ埋める](at-optional-same-day-only.md)）。`write_log` が当日ならどの commit でも `now_ms()` で上書きし、コピー 3 経路（`copy_day` / `core::apply_routine` / `views::day` の `copy_last`）は `at = コピー時刻` を持つ。**このログ単位の意味は変えない。**

だが、これは「その日にそのログを最後に触った時刻」であって「そのセットをやった時刻」ではない。朝にメニューを適用して夜にトレーニングすると、ログの `at` は夜の時刻（最後に触った時刻）になる一方、8:00 に適用しただけのセットが実際にやられたのは夜である。セット単位の「やった時刻」を持つ手段が無かった。

セット単位の時刻を画面に出すアプリは、競合 14 本（Strong / Hevy / FitNotes / Jefit / Fitbod / Liftosaur / RepCount ほか）を調査した限り見当たらない。共通形は「ワークアウトの開始–終了をヘッダ / サマリーに薄く出す」+「セット完了はチェックの 1 タップ」で、セット単位の時刻は hidden-by-default にする形が業界の共通形と整合する。

## 決定

### 1. `SetEntry.at: Option<i64>` を `drops` の後に追加する

`#[serde(default, skip_serializing_if = "Option::is_none")]`。**当日に重量か回数を手で打った行だけ** `Some`（epoch ms）。コピー・`+ セット`・過去日・TSV 取り込みは `None`。一度入ったら打ち直しで動かない。

`ExerciseLog.at`（その日にそのログを最後に触った時刻。コピーでも入る）とは別物で、日ヘッダの開始–終了は**こちらだけ**から出す。`same_set` の判定には入れない。`skip_serializing_if` を外さない（e2e が JSON のキー集合を固定している）。`SCHEMA` は上げない。

### 2. 打つ規則は `core` の純関数（`stamp_set_at`）にし、分岐の順序を固定する

```rust
/// セット行の重量か回数を手で打った直後に呼ぶ。
/// - 過去日なら触らない（打ち直しで消さない・捏造しない）
/// - 一度入った時刻は打ち直し・回数の消去でも動かさない（動かすには行を ✕ で消して足し直す）
/// - まだ無い行は、回数が読めた最初の打鍵で now。読めないうちは None（"0" と空欄は `parse_reps` が None）
pub fn stamp_set_at(prev: Option<i64>, reps_readable: bool, is_today: bool, now_ms: i64) -> Option<i64> {
    if !is_today || prev.is_some() { prev } else if reps_readable { Some(now_ms) } else { None }
}
```

呼ぶのは `src/views/day.rs` のセット行の重量 / 回数の `on:input` だけ。メモ欄・段の入力・`copy_last` / `add_row` / `move_row` / `remove_row` / `commit` からは呼ばない（`copy_last` → `commit` の経路で全行にコピー時刻が入るのを防ぐ）。

### 3. `Row.at: Option<i64>`。`initial` は `s.at` を写す。`copy_last` / `add_row` / `Row::blank` は `None`

過去日に打った当日の `at` を往復させるため、`commit` は `at: r.at` を素通しする（過去日の新規行は規則 2 で `None` のまま）。

### 4. `is_today` はカード内に 1 本だけ持つ

`commit` と on:input の両方が同じ判定を見る。判定がずれると「表示は今日なのに保存は過去扱い」のような食い違いが生まれる。

### 5. `Seed::carry` は `at` を運ばない。`parse_tsv` は `at: None` で組む

`Seed`（`copy_day` / `apply_routine` の受け渡し型）は分解代入で `at: _` を明示し、次に `ExerciseLog` へフィールドが増えてもここがコンパイルエラーになるようにしてある。TSV 取り込みは過去日のバックフィルなので時刻を持たない。

### 6. `core::day_span(session: &Session) -> Option<(i64, i64)>`: 全セットの `at` だけの min / max

**`ExerciseLog.at` は使わない** — コピー時刻が入るので、朝にメニューを適用して夜にトレすると「08:00–20:10」という嘘の開始時刻になる。コピーだけの日はヘッダに時刻が出ない（「触らなかったセットに時刻なし」と整合する）。`day_span` は時計を持たない。

### 7. `merge_db` は取り込み先の時刻を消さない最小限の合流を行う

- `same_sets` の枝: 位置で `mine.at.is_none() && theirs.at.is_some()` なら埋め、`MergeReport.times_added += 1`（`drops` の「空のときだけ入れる」と同じ）。
- `log_rank` の差し替え枝: `*existing = ExerciseLog { .., ..log }` の**前**に `carry_set_ats(&existing.sets, &mut log.sets)` を呼び、勝った側のセットへ取り込み先の時刻を運ぶ。`carry_set_ats` は `merge_set_drops_unordered` と同じ 2 段構成（同じ時刻を既に持っている相手へ先に寄せ、余ったものだけを時刻の無い相手に入れる）。**数えない**（追加ではなく保持）。これが無いと、当日の記録に 1 セット足した TSV を取り込むだけでその日のセット時刻が全部消える。
- `same_sets_unordered` の枝（並べ替えただけ）は触らない。JSON 復元 × 他端末で並び替え済みのときしか踏まない現状固定。
- `MergeReport.times_added: usize` を足し、`is_noop` に数え、`views::backup::added_text` にも出す。`Lang::added_times(n)`（`added_drops` と同形）: JA `"{n} 件の時刻"` / EN `"{n} set time(s)"`。

### 8. `ExerciseLog.at` / `dedupe_logs` / `merge_db` のログ `at` / TSV は変えない

セット時刻は JSON バックアップには乗るが TSV には出ない（出すなら別列。この ADR のスコープ外）。

## 理由

入力イベント以外に「そのセットをやった」という信号が無い。捏造しない — 起きていない時刻を作らない。既存のスキーマは不変（`#[serde(default)]` + `skip_serializing_if`）。`Seed::carry` の分解代入で「運ばない」を型で強制できる。ログの `at` はコピー時刻を含むので開始–終了には使わない（朝にメニューを適用して夜にトレすると 08:00 が開始になる、という嘘を避ける）。一度入った時刻を打ち直しで動かさないのは「そのセットをやった時刻」を守るため。

## 結果（トレードオフ）

- 触らなかったセットは時刻なし。コピーだけの日はヘッダに時刻が出ない。
- 最初の行を ✕ で消すと開始が後ろへ動く（消した記録は無かったものとして扱う）。
- 日跨ぎ: `dates.today` は `visibilitychange` / タブ切替でしか再評価されないので、00:05 に打つと前日キーに翌日の epoch が入り、ヘッダは「23:10–00:05」になる（既存の `log.at` と同じ挙動。TSV の `tsv_time` は同日ガードで空欄になる）。
- 当日手入力のセットは 1 本 +約 19 B（`,"at":1758500000000`）。週 4 × 5 種目 × 3.5 セットで年 +約 70 KB、10 年で見積り +約 1.24 MB → 合計で約 1.9 MB になる（5 MB 上限内。[localStorage の単一キーに JSON 全体を持つ](../storage/localstorage-single-key-json.md) の見積りに追記する）。
- TSV には出ない（JSON バックアップには乗る）。出すなら「時刻」列に混ぜず別列にする。
- 取り込みは `same_sets` で空を埋め、差し替えで持ち越し、並びだけ違う合流では運ばない。

## 検討した代替案

- **ログの `at` を「最初のセットの時刻」に改めて開始に使う** — コピー時刻が開始になる。却下。
- **種目ごとに開始・終了の 2 値を持つ** — 終了はセットから導けて冗長で、セット単位の分析ができない。
- **コピー時刻を全セットに入れる** — 同じ時刻が並び、やっていないことをやったことにする嘘になる。却下。
- **セット番号のタップで「今」を打つ** — 閉じた状態で手応えが無く、任意操作が増える。使ってみて足りなければ次版で検討する。
- **回数を消したら時刻も消す** — "10" → "" → "8" の打ち直しで時刻が動いてしまう。却下。
- **秒・分単位で持つ** — `log.at` と単位が割れる。
- **保存せず表示だけにする** — 要望は記録として残すことなので満たさない。
