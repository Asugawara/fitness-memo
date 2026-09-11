//! 純ロジック。**`leptos` / `web-sys` を一切 import しない。**
//!
//! `cargo test`（ホストターゲット）で検証する層。UI から呼ぶ計算はすべてここに置き、
//! 画面側は結果を並べるだけにする。

use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{Datelike, NaiveDate, TimeDelta};

use crate::i18n::{Lang, ReleaseNote};
use crate::model::{
    Db, DropStage, Exercise, ExerciseId, ExerciseLog, Group, GroupId, IdGen, Label, LabelId,
    MAX_DROPS, MAX_INTERVAL_SEC, MAX_LABEL_LEN, MAX_LABELS, MAX_PIN_LEN, MAX_PINS, Routine,
    RoutineId, SCHEMA, Session, SetEntry,
};

/// `Db::sessions` のキー書式。ゼロ埋め ISO なので辞書順 = 時系列順になる。
pub const DATE_FMT: &str = "%Y-%m-%d";

pub fn date_key(d: NaiveDate) -> String {
    d.format(DATE_FMT).to_string()
}

pub fn parse_date_key(k: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(k, DATE_FMT).ok()
}

// ── 指標 ────────────────────────────────────────────────────────────────────

/// グラフに出す値の種類。**種目の属性ではなく画面の表示設定。**
///
/// 旧 `Kind`（加重 / 自重 / 時間）を種目に持たせていたのは「自重種目に加重すると
/// 系列の意味が変わる」問題を防ぐためだったが、ユーザーに区別を選ばせる形は
/// 意味が伝わらなかった。**どの軸で見るかをその場で切り替えられる**ようにすることで
/// 同じ問題を解く。単位が `Metric` だけで決まるので、対象種目を切り替えても
/// 軸の意味は変わらない。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Metric {
    /// Σ(重量 × 回数)。重量が空 / 0 のセットは重量 1 として数える
    #[default]
    Volume,
    /// セットの本数
    Sets,
    /// Σ回数
    Reps,
}

impl Metric {
    /// 推移タブの指標セレクタに出す 3 択。
    ///
    /// ★ かつて `const CHOICES` だった。文言が言語で変わるので関数にしてある
    ///   （並びは不変 — 期間セレクタと同じく「軽い順」に並べる）。
    pub fn choices(lang: Lang) -> [(Metric, &'static str); 3] {
        let c = &lang.strings().core;
        [
            (Metric::Volume, c.metric_volume),
            (Metric::Sets, c.metric_sets),
            (Metric::Reps, c.metric_reps),
        ]
    }

    /// 表示に添える単位。ボリュームは重量と回数の合成量なので単位を持たない。
    pub fn unit(self, lang: Lang) -> &'static str {
        let c = &lang.strings().core;
        match self {
            Metric::Volume => "",
            Metric::Sets => c.unit_sets,
            Metric::Reps => c.unit_reps,
        }
    }
}

/// 推移タブの集計にドロップセットの段を入れるか。**既定は入れない。**
///
/// [`Metric`] と同じく**種目の属性ではなく画面の表示設定**で、保存先も `Db` ではなく
/// `fitness-memo/ui/v1`（adr/storage/ui-state-in-separate-key.md）。
///
/// ★ 効くのは**推移タブだけ**。記録タブの当日合計・カレンダーの月合計・`summarize` は
/// 常に入れる（[`log_value`] を通る）。設定は「推移の見せ方」であって記録ではないので、
/// その日やった仕事の合計から落としてはいけない。
///
/// ★ `bool` ではなく enum にするのは、[`exercise_series`] / [`group_series`] /
/// [`pick_series`] の 3 本に旗を通すことになるため。呼び出し側で
/// `pick_series(.., true)` と書かれると、真が「入れる」なのか「除く」なのか読めない。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Drops {
    #[default]
    Exclude,
    Include,
}

/// 1 セットのボリューム。**重量が入っていないセットは重量 1 として数える。**
///
/// これで自重種目は自然に「総レップ数」、時間種目は「総秒数」になり、
/// 種目ごとに式を分ける必要が無くなる。
///
/// ★ `max(1.0)` にするのは単調性のため。0.5kg を 0.5 倍で扱うと
/// 「重量を足したのに指標が下がる」が起きて、グラフの上下が負荷の増減を表さなくなる。
///
/// ★ **メインセットぶんだけを返す。** 落とした段は [`log_value_of`] が
/// [`counted_drops`] を通して別に数える。ここに段を足し込むと、推移タブから段を外す
/// 設定が効かなくなる（`Metric::Sets` が `sets` の本数を数えるので、値を 0 にする
/// やり方では成立しない）。
pub fn set_volume(s: &SetEntry) -> f64 {
    volume(s.weight, s.reps)
}

/// 重量 × 回数のボリューム。**メインセットも段もこの 1 本を通す。**
///
/// ★ 式を 2 つ書かない。`max(1.0)`（[`set_volume`] の doc にある単調性の理由）を
/// 片方だけ直すと、自重の段が古い規則のまま残る。
fn volume(weight: f32, reps: u32) -> f64 {
    f64::from(weight).max(1.0) * f64::from(reps)
}

/// 集計に数える段。`Drops::Exclude` なら空。
///
/// ★ **「段を数えるか」の分岐をここ 1 箇所に閉じる。** [`log_value_of`] の 3 つの腕が
/// それぞれ `if` を持つと、腕を 1 つ足したときに片方だけ段を数え忘れられる —
/// それがまさに doc で防ごうとしている食い違い。
fn counted_drops(s: &SetEntry, d: Drops) -> &[DropStage] {
    if d == Drops::Include { &s.drops } else { &[] }
}

/// 1 ログ（= その日のその種目）の指標。**段も数える。**
///
/// 記録タブ・カレンダー・`log_rank` の入口。推移タブは [`log_value_of`] を使う。
pub fn log_value(m: Metric, l: &ExerciseLog) -> f64 {
    log_value_of(m, l, Drops::Include)
}

/// 1 ログの指標。段を数えるかを選べる版。
///
/// ★ **3 指標すべてで段の扱いを揃える。** `Sets` だけ段を数えない、のような食い違いを
/// 作ると、同じ設定で「ボリュームは増えたのにセット数は変わらない」が起きて、
/// 利用者は設定が効いているのか壊れているのか区別できない。分岐を
/// [`counted_drops`] に畳んであるので、腕はどれも「メインセット + 段」の同じ形になる。
pub fn log_value_of(m: Metric, l: &ExerciseLog, d: Drops) -> f64 {
    l.sets
        .iter()
        .map(|s| match m {
            Metric::Volume => {
                volume(s.weight, s.reps)
                    + counted_drops(s, d)
                        .iter()
                        .map(|x| volume(x.weight, x.reps))
                        .sum::<f64>()
            }
            Metric::Sets => 1.0 + counted_drops(s, d).len() as f64,
            Metric::Reps => {
                f64::from(s.reps)
                    + counted_drops(s, d)
                        .iter()
                        .map(|x| f64::from(x.reps))
                        .sum::<f64>()
            }
        })
        .sum()
}

// ── 数値の整形 / パース ─────────────────────────────────────────────────────
//
// ★ もとは `views/mod.rs`（wasm32 専用）にあった。書き出しの TSV がここを通るので
//   ホストの `cargo test` から検証できる側へ移した。`views` は再輸出しているだけで、
//   呼び出し側（`day.rs` / `chart.rs`）は `use super::{..}` のまま変わっていない。

/// 60.0 → "60"、62.5 → "62.5"
pub fn fmt_weight(w: f32) -> String {
    if w == w.trunc() {
        format!("{}", w.trunc() as i64)
    } else {
        format!("{w}")
    }
}

/// 入力欄の生文字列 → 重量。
///
/// `"6."` は Rust の `f32` パーサが 6.0 として受けるので中間状態でも壊れない。
/// iOS のテンキーは小数点がロケール依存で `,` になることがあるので置換しておく。
pub fn parse_weight(s: &str) -> f32 {
    s.trim()
        .replace(',', ".")
        .parse::<f32>()
        .ok()
        .filter(|w| w.is_finite() && *w >= 0.0)
        .unwrap_or(0.0)
}

/// 入力欄の生文字列 → レップ数。**空欄と 0 は「行なし」として扱う。**
pub fn parse_reps(s: &str) -> Option<u32> {
    s.trim().parse::<u32>().ok().filter(|r| *r > 0)
}

/// 入力欄の生文字列 → インターバル（秒）。**空欄は「未設定」。**
///
/// ★ [`parse_reps`] と違って **0 を落とさない**。0 秒は「休まず次のセットへ」という
/// 正当な入力で、`0×0` のゴーストセットのような下流の汚染も無い
/// （adr/ux/interval-seconds-on-the-exercise.md）。
///
/// ★ 上限で丸めない。丸めるのは [`set_interval`] / [`normalize_exercises`] が共有する
/// [`clean_interval`] の仕事で、ここで別に丸めると規則が 2 本に割れる。
pub fn parse_interval(s: &str) -> Option<u32> {
    s.trim().parse::<u32>().ok()
}

// ── メモ（adr/data-model/notes-on-logs-and-sets.md）──────────────────────────

/// 重量と回数だけを見たセット列の一致。**メモを無視する。**
///
/// ★ [`SetEntry`] の `PartialEq` にメモが入ったので、`==` は「セットは同じでメモだけ
/// 違う」を**不一致**にする。それを「同じ記録か」の判定に使うと、[`merge_db`] では
/// [`log_rank`] が同点なので差し替えの分岐にも入らず、取り込む側のメモが
/// `Conflict` も出さずに黙って捨てられる。
///
/// 「同じセットか」を問うときは必ずここを通すこと。`==` を使ってよいのは
/// 「メモまで含めてまったく同じか」を問うときだけ。
fn same_sets(a: &[SetEntry], b: &[SetEntry]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| same_set(x, y))
}

/// セット 1 本の同一性。**重量と回数だけ**を見る。
///
/// ★ [`same_sets`] の doc が言う「必ずここを通す」の実体。要素単位で問う場所
/// （[`merge_set_notes_unordered`] / [`merge_set_drops_unordered`]）が式を書き直すと、
/// 「メモも段も同一性に入らない」という [`SetEntry::drops`] の不変条件が
/// 手書きの式の数だけ増える。
///
/// 重量は `to_bits` で比べる。`==` は NaN を不一致にするので、
/// [`drop_unrepresentable_weights`] が先に走る前提が崩れた瞬間に挙動が分かれる。
fn same_set(a: &SetEntry, b: &SetEntry) -> bool {
    a.weight.to_bits() == b.weight.to_bits() && a.reps == b.reps
}

/// 並びを無視したセット列の一致。**メモは見ない。**
///
/// ★ [`same_sets`] は `zip` で位置を比べるので、**セットを並べ替えただけの同じ記録**を
/// 食い違い扱いにする（adr/ux/drag-to-reorder-in-record-tab.md でセットの D&D を入れた）。
/// [`merge_db`] でそのまま落ちると [`log_rank`] の第 3 要素が位置依存の辞書順なので、
/// 勝ち負けが実質任意に決まり、負けた側のセットメモが `*existing = log` で消える。
/// **並びは端末ごとの好みであってデータではない**ので、ここで先に掬う。
///
/// 重量は非負なので `to_bits` の順序が値の順序と一致する（[`log_rank`] と同じ理由）。
fn same_sets_unordered(a: &[SetEntry], b: &[SetEntry]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let key = |s: &[SetEntry]| {
        let mut v: Vec<(u32, u32)> = s.iter().map(|x| (x.weight.to_bits(), x.reps)).collect();
        v.sort_unstable();
        v
    };
    key(a) == key(b)
}

/// 重量・回数が同じセット同士を突き合わせてメモを合流させる。足した数を返す。
///
/// [`same_sets`] のときの「位置で埋める」を、**並びが違うとき用に一般化したもの**。
/// 同じ重量・回数のセットが複数あるときは出現順に 1 対 1 で組む（どちらに付くかは
/// 決められないが、**捨てるよりは良い** — 同じ記録の同じ重量・回数の行なので、
/// メモが隣の行に付いても意味の壊れ方は位置ずれと同程度）。
///
/// ★ これが無いと、片方の端末でセットを並べ替えた瞬間に**もう片方のセットメモが
/// 二度と合流しなくなる**（並び替えが `same_sets` を false にするため）。
fn merge_set_notes_unordered(mine: &mut [SetEntry], theirs: &[SetEntry]) -> usize {
    let mut used = vec![false; mine.len()];
    let mut added = 0;
    for t in theirs.iter().filter(|t| !t.note.trim().is_empty()) {
        let found = mine
            .iter()
            .enumerate()
            .find(|(i, m)| !used[*i] && same_set(m, t))
            .map(|(i, _)| i);
        if let Some(i) = found {
            used[i] = true;
            if append_note(&mut mine[i].note, &t.note) {
                added += 1;
            }
        }
    }
    added
}

/// 重量・回数が同じセット同士を突き合わせて**ドロップの段**を合流させる。埋めた数を返す。
///
/// [`merge_set_notes_unordered`] とは**別のパスにする**。あちらは
/// `filter(|t| !t.note.trim().is_empty())` でメモを持つ相手だけを回すので `used` の
/// 取り合いが違い、共有すると `notes_added` が変わる（同じ重量・回数の行が複数ある
/// 記録で、メモが隣の行に二重に付く）。
///
/// **段が空のセットにだけ入れる。** 両方に段があって中身が違うときは取り込み先を残す
/// （`append_note` の「同じ文が既に入っていれば足さない」と同じ、足さない側に倒す判断）。
/// `Conflict` も出さない — 出すと同じファイルを 2 回入れるたびに同じ食い違いを報告する。
///
/// ★ **2 段で組む。** 素朴に「段付きの相手を、同じ重量・回数の未使用な先頭に入れる」と、
/// **同一のデータを初めて取り込んだだけで無関係なセットに段が付く**:
///
/// ```text
/// mine   = [(60,10,"A"), (60,10,段あり), (70,5)]
/// theirs = [(70,5),      (60,10,段あり), (60,10,"A")]   // 並べ替えただけの同じ記録
/// → theirs の段付きが mine[0]（段なし）に当たり、段が 2 セットに付く
/// ```
///
/// だから先に**同じ段を既に持っている相手**へ寄せ、余ったものだけが段の無い相手に入る。
/// これで自分のファイルを何度取り込んでも増えない（冪等）。
fn merge_set_drops_unordered(mine: &mut [SetEntry], theirs: &[SetEntry]) -> usize {
    let mut used = vec![false; mine.len()];
    let mut filled = 0;
    // 第 1 段: 同じ段を既に持っている相手に寄せる（何も変えないので数えない）
    let mut leftover = Vec::new();
    for t in theirs.iter().filter(|t| !t.drops.is_empty()) {
        let found = mine
            .iter()
            .enumerate()
            .find(|(i, m)| !used[*i] && m.drops == t.drops && same_set(m, t))
            .map(|(i, _)| i);
        match found {
            Some(i) => used[i] = true,
            None => leftover.push(t),
        }
    }
    // 第 2 段: 余ったものが、段の無い相手に入る
    for t in leftover {
        let found = mine
            .iter()
            .enumerate()
            .find(|(i, m)| !used[*i] && m.drops.is_empty() && same_set(m, t))
            .map(|(i, _)| i);
        if let Some(i) = found {
            used[i] = true;
            mine[i].drops = t.drops.clone();
            filled += 1;
        }
    }
    filled
}

/// メモの合流。**同じ文が既に入っていれば足さない。** 足したら `true`。
///
/// ★ 無条件に連結すると、同じファイルを 2 回取り込んでメモが 2 倍になる。
/// [`MergeReport`] の数が冪等でも**文字列は利用者に見える**ので、こちらも冪等でないと
/// 事故になる。`Session::note` が元から持っていたガードを 1 関数に切り出して、
/// [`merge_same_day`] / [`dedupe_logs`] / [`merge_db`] の 3 箇所で共有する。
/// 2 本目の規則を書いた瞬間にどれかが冪等でなくなる。
///
/// ★ 判定は部分一致。「痛」が「肩が痛い」の中にあると新しいメモでも足されない。
/// 誤って足さない側に倒すのは意図で、`Session::note` の既存挙動を変えないことと、
/// 冪等性のほうが短いメモの取りこぼしより重いことの両方から。
///
/// ★ **比較は空白を畳んでから行う（保存する文字列は畳まない）。** TSV は 1 セル 1 行
/// なので、書き出しでメモの改行を空白に潰す（[`flatten_cell`]）。畳まずに比べると
/// `"A\nB"` を持つ端末が自分の TSV を読み戻したときだけ `"A B"` が別物と判定され、
/// メモが 1 度だけ伸びる。空白の入り方が違うだけの同じ文は同じ文として扱う。
fn append_note(dst: &mut String, src: &str) -> bool {
    let incoming = src.trim();
    if incoming.is_empty() || fold_ws(dst).contains(&fold_ws(incoming)) {
        return false;
    }
    if dst.trim().is_empty() {
        dst.clear();
        dst.push_str(incoming);
    } else {
        dst.push('\n');
        dst.push_str(incoming);
    }
    true
}

/// 連続する空白（改行・タブを含む）を半角 1 つに畳む。**比較のためだけに使う。**
///
/// ★ これで正規化した文字列を保存してはいけない。利用者が入れた改行は表示に効くので、
/// 畳んだものを書き戻すとメモの見た目が勝手に変わる。畳むのは [`append_note`] の
/// 重複判定と、TSV の 1 セルに落とすとき（[`flatten_cell`]）だけ。
fn fold_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 空白だけのメモを空文字にする。**「空白 = 無い」を 1 箇所で決める。**
///
/// これが無いと `" "` が `skip_serializing_if = "String::is_empty"` をすり抜けて
/// 保存され続け、`ExerciseLog::is_empty()`（`trim` する）と JSON の見え方がずれる。
/// 回数が 0 の段と、上限を超えた段を落とす。**「段が在る」を 1 箇所で決める。**
///
/// ★ これが無いと、取り込みや手編集で入った `reps: 0` の段が `drops` を非空にして、
/// 推移タブの注記（「ドロップセットは含めていません」）が**何も外していないのに出る**。
/// 重量が `f32` で表せない段は `drop_unrepresentable_weights` が別に落とす。
fn prune_empty_drops(s: &mut Session) {
    for log in &mut s.logs {
        for set in &mut log.sets {
            set.drops = clean_drops(std::mem::take(&mut set.drops));
        }
    }
}

fn blank_notes_to_empty(s: &mut Session) {
    if s.note.trim().is_empty() {
        s.note.clear();
    }
    for log in &mut s.logs {
        if log.note.trim().is_empty() {
            log.note.clear();
        }
        for set in &mut log.sets {
            if set.note.trim().is_empty() {
                set.note.clear();
            }
        }
    }
}

// ── 参照 ────────────────────────────────────────────────────────────────────

/// 「前回」をどのラベルの中から引くか
/// （adr/data-model/labels-on-the-exercise-and-a-mark-on-the-log.md）。
///
/// ★ **`Option<LabelId>` を引数に足さない。** `None` が「絞らない」なのか
/// 「ラベルなしのログだけ」なのか、呼び出し側のコードから読めない。
///
/// ★ **`Unlabeled`（ラベルなしのログだけ）バリアントを作らない。** 6 か月ラベル
/// なしで記録 → 今日 H/P/S を定義 → 以後全部付ける、という利用で「指定なし」が
/// `Unlabeled` の意味だと**半年前の記録が出る**。`Any` なら昨日が出る。既存利用者の
/// 体験を変えないという要件はこちらでしか満たせない。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum LabelFilter {
    /// 絞らない。**従来の挙動そのもの。**
    #[default]
    Any,
    /// そのラベルが付いたログだけ。
    Only(LabelId),
}

impl LabelFilter {
    /// そのログを通すか。
    ///
    /// ★ `pub(crate)` にしてあるのは推移タブの**記録テーブル**が呼ぶため。あちらは
    /// [`pick_points`] を通らず `db.sessions` を直接走るので、グラフと同じ絞りを
    /// 掛けるには同じ述語が要る（規則を 2 本に割らない）。
    pub(crate) fn passes(self, log: &ExerciseLog) -> bool {
        match self {
            Self::Any => true,
            Self::Only(id) => log.label == Some(id),
        }
    }
}

/// 指定日より**厳密に前**の、その種目の記録を**新しい順**に走査する。
///
/// 1 日につき高々 1 件なのは「1 日 1 種目 1 ログ」の不変条件（`Vec` になるのは
/// **日をまたぐ方向**であって、同じ日に 2 本並ぶわけではない）。
///
/// ★ セットが空のログは飛ばす。メモだけ書いた日は実施日ではない
///   （[`crate::model::Session::is_trained`] と同じ式）。
///
/// ★ ラベルで絞っても**フォールバックしない**（0 件なら 0 件）。落とすと
///   コピーボタンが「表示と違うものを流し込む」ことになり、
///   adr/ux/copy-button-only-when-empty.md が消した 3 問題が別の入口から戻る。
fn logs_before(
    db: &Db,
    ex: ExerciseId,
    before: NaiveDate,
    filter: LabelFilter,
) -> impl Iterator<Item = (NaiveDate, &ExerciseLog)> {
    db.sessions
        .range(..date_key(before))
        .rev()
        .filter_map(move |(key, session)| {
            let log = session
                .logs
                .iter()
                .find(|l| l.exercise_id == ex && !l.sets.is_empty() && filter.passes(l))?;
            Some((parse_date_key(key)?, log))
        })
}

/// 指定日より**厳密に前**で最も新しい、その種目の記録。
///
/// 単一の `ExerciseLog` を返せるのは「1 日 1 種目 1 ログ」の不変条件に依存する。
///
/// ★ **ラベルで絞らない。** 「旧名は旧挙動、新名がパラメータ付き」なので、この名前で
/// 呼んだ側の挙動は今までと 1 バイトも変わらない（絞りたいときは
/// [`last_log_before_with`]）。
pub fn last_log_before(
    db: &Db,
    ex: ExerciseId,
    before: NaiveDate,
) -> Option<(NaiveDate, &ExerciseLog)> {
    last_log_before_with(db, ex, before, LabelFilter::Any)
}

/// [`last_log_before`] のラベル指定版。
pub fn last_log_before_with(
    db: &Db,
    ex: ExerciseId,
    before: NaiveDate,
    filter: LabelFilter,
) -> Option<(NaiveDate, &ExerciseLog)> {
    logs_before(db, ex, before, filter).next()
}

/// 指定日より**厳密に前**の、その種目の記録を**新しい順に最大 `limit` 件**。
///
/// 種目カードの履歴（`views::day`）が引く。件数は利用者の表示設定で、
/// [`history_count`] が `1..=MAX_HISTORY` に丸めたものが渡ってくる。
///
/// ★ **先頭が「前回」**（[`last_log_before`] と必ず一致する）。コピーと重量警告は
///   この先頭 1 件だけを見るので、降順であることが仕様の一部になっている。
pub fn last_logs_before(
    db: &Db,
    ex: ExerciseId,
    before: NaiveDate,
    limit: usize,
) -> Vec<(NaiveDate, &ExerciseLog)> {
    last_logs_before_with(db, ex, before, limit, LabelFilter::Any)
}

/// [`last_logs_before`] のラベル指定版。**種目カードの `history` Memo だけが呼ぶ。**
///
/// ★ 「重量を使う種目か」の判定（`views::day` の `uses_weight`）はここを通さない。
/// あれは種目の性質でモードの性質ではないので、無絞りの [`last_log_before`] を
/// 専用に引く（adr/ux/past-records-by-date-with-a-count-setting.md
/// 「表示設定は表示だけを変える」）。
pub fn last_logs_before_with(
    db: &Db,
    ex: ExerciseId,
    before: NaiveDate,
    limit: usize,
    filter: LabelFilter,
) -> Vec<(NaiveDate, &ExerciseLog)> {
    logs_before(db, ex, before, filter).take(limit).collect()
}

/// 種目カードに出す過去の記録の件数の既定値。**現状と同じ「前回 1 件だけ」。**
pub const DEFAULT_HISTORY: usize = 1;

/// 同上の上限。
///
/// ★ **3 で止める。** メモが並ぶと 1 件が複数行になるので、5 件だとカードの上半分が
///   10 行を超え、「重量を打つ → 回数を打つ → + セット」の動線が画面外へ出る。
///   それ以上の履歴は推移タブ（グラフ + 記録テーブル）の担当。
pub const MAX_HISTORY: usize = 3;

/// 保存値 → 実際に使う件数。**必ず `1..=MAX_HISTORY` を返す。**
///
/// ★ 範囲外は捨てずに `clamp` する（`clean_pins` の切り詰めと同じ規則）。
///   `DEFAULT_HISTORY` が下端と一致するので、未設定・0・負がすべて 1 に落ちる。
///
/// ★ **0 を返さないことが `views::day` の前提。** 0 になると履歴が消えるだけでなく、
///   「前回をコピー」と「重量未入力」の警告が黙って出なくなる（どちらも先頭 1 件を見る）。
///
/// 引数が `Option<i64>` なのは `storage::UiState` の受け口に合わせたもの。JSON に
/// どんな整数が入っていてもパースを落とさず、丸めをここ（ホストのテストが届く側）でやる。
pub fn history_count(saved: Option<i64>) -> usize {
    let Some(n) = saved else {
        return DEFAULT_HISTORY;
    };
    n.clamp(1, MAX_HISTORY as i64) as usize
}

/// ドロップセットの既定の落とし幅（%）。**メインセットから 20% 落とす。**
///
/// 20 なのは、実際のドロップセットで最も普通な刻み（60 → 50 / 100 → 80）に当たるため。
pub const DEFAULT_DROP_PCT: f32 = 20.0;

/// 落とし幅の上限（%）。
///
/// ★ 自由入力だが**上限は要る**。100 を入れると計算結果が 0kg になり、`set_volume` の
/// 「重量なし = 重量 1」に化けて自重種目の扱いになる。95 で止めれば、どんな入力でも
/// 「メインセットより軽い正の重量」に落ちる。
pub const MAX_DROP_PCT: f32 = 95.0;

/// 保存値 → 実際に使う落とし幅（%）。**小数点以下 1 桁に丸め、`0..=MAX_DROP_PCT` に収める。**
///
/// ★ 範囲外は捨てずに `clamp` する（`history_count` と同じ規則）。0 は「落とさない」で
/// 有効な選択なので弾かない — 段の重量がメインセットと同じになるだけ。
pub fn drop_pct(saved: Option<f64>) -> f32 {
    let Some(p) = saved else {
        return DEFAULT_DROP_PCT;
    };
    if !p.is_finite() {
        return DEFAULT_DROP_PCT;
    }
    round1((p as f32).clamp(0.0, MAX_DROP_PCT))
}

/// 小数点以下 1 桁に丸める。**落とし幅と、そこから出す重量の両方に使う。**
fn round1(v: f32) -> f32 {
    (v * 10.0).round() / 10.0
}

/// メインセットの重量から、落とし幅ぶん引いた段の重量。**小数点以下 1 桁。**
///
/// ★ 適用するのは**そのセットの 1 段目だけ**（呼び出し側の責任）。2 段目は落とす元が
/// 1 段目になるので基準が変わり、同じ式では出せない。
///
/// 重量の入っていないメインセット（自重種目）は `None`。掛ける相手が無いので、
/// 0 を入れるより空のままにして利用者に打たせるほうが正しい。
pub fn dropped_weight(main_weight: f32, pct: f32) -> Option<f32> {
    if main_weight <= 0.0 {
        return None;
    }
    Some(round1(main_weight * (1.0 - pct / 100.0)))
}

/// 保存値 → 推移タブがドロップセットを数えるか。**既定は数えない。**
///
/// 符号化は `0` = 除く / `1` = 入れる。引数が `Option<i64>` なのは
/// `storage::UiState` の受け口に合わせたもので、`Option<bool>` にしない理由は
/// そちらの doc にある（狭い型は `UiState` 全体のパースを落とす）。
///
/// ★ **`Some(1)` だけを `Include` にする。** 知らない値で集計を広げてはいけない —
/// 壊れた値が「入れる」に倒れると、既定が除外だと思っている利用者のグラフが黙って
/// 上がる。`history_count` が `clamp` なのは 1..=3 のどれもが妥当な選択肢だからで、
/// ここは 2 択なので「知らない値 = 既定」に落とすほうが正しい。
pub fn drops_setting(saved: Option<i64>) -> Drops {
    match saved {
        Some(1) => Drops::Include,
        _ => Drops::Exclude,
    }
}

// ── 並び替え ────────────────────────────────────────────────────────────────

/// `date` のログを `order` の並びに揃える。**ログの中身には一切触らない。**
///
/// 記録タブのドラッグで呼ぶ（adr/ux/drag-to-reorder-in-record-tab.md）。カード 1 枚が
/// `ExerciseLog` 1 本なので、並び替えは `logs` の順を入れ替えるだけで表現できる。
///
/// ★ **`sets` / `note` / `at` を 1 バイトも動かさない。** `at` は「その日に実施した時刻」で
/// （adr/data-model/at-optional-same-day-only.md）、並べ替えは実施ではない。ここで押すと
/// **触ってもいない他種目のログ**に「たった今トレした」証拠を捏造することになる。
/// セット並び替えのほうは `views::day` の `commit()` を通るので `at` が更新されるが、
/// あちらは自分のログ 1 本だけで、しかも同じ暦日を出ない。
///
/// 畳み方の規則（この 3 本が不変条件）:
/// - **`order` にあってログが無い ID は飛ばす。** 「種目を追加」で出しただけで 1 度も
///   commit されていないカードがここに来る（`views::day` の `pick` は画面の集合にしか
///   足さず、`write_log` はセットもメモも空なら書かない）
/// - **`order` に無いログは末尾へ、元の相対順のまま残す。落としてはいけない。**
///   [`merge_db`] は開いている日にもログを増やせるので、画面の集合を真実源にして
///   `logs` を作り直す実装にすると、取り込んだばかりのログが黙って消える
/// - `order` の重複は最初の 1 回だけ効かせる
///
/// 返り値は「並びが変わったか」。
pub fn reorder_logs(db: &mut Db, date: NaiveDate, order: &[ExerciseId]) -> bool {
    // ★ `entry().or_default()` を使わないこと。並べ替えの副作用で空のセッションが生まれると、
    //   何も記録していない日がカレンダーとバックアップに残る（`write_log` はわざわざ
    //   末尾で空セッションを掃除している）
    let Some(session) = db.sessions.get_mut(&date_key(date)) else {
        return false;
    };
    let before: Vec<ExerciseId> = session.logs.iter().map(|l| l.exercise_id).collect();
    let mut rest = std::mem::take(&mut session.logs);
    let mut out = Vec::with_capacity(rest.len());
    for id in order {
        // ★ `swap_remove` ではなく `remove`。残りの相対順を壊すと規則の 2 本目が破れる。
        //   1 日の種目は多くて 10 なので O(n²) で足りる
        if let Some(i) = rest.iter().position(|l| l.exercise_id == *id) {
            out.push(rest.remove(i));
        }
    }
    out.extend(rest);
    let changed = out.iter().map(|l| l.exercise_id).ne(before.iter().copied());
    session.logs = out;
    changed
}

// ── メニューのコピー ────────────────────────────────────────────────────────
//
// ★ このリポジトリで「メニュー」は 3 つの意味を持つ。取り違えると壊れるので明記する。
//   1. **設定タブ**（`views::settings`）— 種目マスタの管理画面。旧「種目タブ」
//   2. **過去の日の種目構成**（[`MenuCandidate`] / [`recent_menus`]）— この節の前半。
//      同一性は「日付」で、コピーするとその日の数値がそのまま入る
//   3. **保存済みのトレーニングメニュー**（[`crate::model::Routine`] /
//      [`RoutineCandidate`] / [`apply_routine`]）— この節の後半。同一性は「名前」で、
//      展開すると**種目ごとに別々の日**（各種目の直近）から数値が入る
//
//   2 と 3 は記録タブでは 1 本のリストに並ぶが、**同じ種目集合でも入る数値が違う**ので
//   型を統合してはいけない（またいで重複排除すると別物の選択肢を隠すことになる）。

/// メニュー候補として遡る上限。
///
/// 打ち切りが要るのは、毎回同じ種目構成しか無いユーザー（全身法）だと重複排除が
/// 効かず、`limit` 件に届かないまま全履歴を舐めてしまうため。半年より古いメニューを
/// 「前回」として出す意味も薄い。
const MENU_LOOKBACK_DAYS: i64 = 180;

/// コピー元のメニュー候補 1 件。
#[derive(Clone, Debug, PartialEq)]
pub struct MenuCandidate {
    pub date: NaiveDate,
    /// **実際にコピーされる種目 ID。** 元の日のログ順。
    pub exercises: Vec<ExerciseId>,
}

/// その日の「コピーできるログ」だけを返す。
///
/// ★ [`recent_menus`] と [`copy_day`] と [`day_exercises`] は必ず**これを通す**。
/// フィルタがずれると「5 種目」と表示された候補を押しても何も起きない死んだボタンができる。
///
/// アーカイブ済みを外すのは、「種目を追加」シートがアーカイブ済みを出さないため。
/// コピーで復活させると、カードを閉じたあとユーザーが自力で戻せない種目になる。
fn copyable(db: &Db, date: NaiveDate) -> impl Iterator<Item = &ExerciseLog> {
    db.sessions
        .get(&date_key(date))
        .into_iter()
        .flat_map(|s| s.logs.iter())
        .filter(|l| !l.sets.is_empty())
        .filter(|l| db.exercise(l.exercise_id).is_some_and(|e| !e.archived))
}

/// その日の記録から**メニューに写せる種目**（その日のログ順）。
///
/// 記録タブの「この日をメニューにする」が使う（adr/ux/save-a-day-as-a-routine.md）。
///
/// ★ [`copyable`] を通すので、「前回のメニューから始める」の候補・[`copy_day`] と
/// **同じ集合**になる。「この日をメニューにする」で保存したメニューを押した結果が、
/// その日をコピーした結果と種目単位で一致する（数値は種目ごとの直近から入るので別）。
///
/// ★ セットの数値は返さない。[`crate::model::Routine`] は数値を持たない
/// （adr/data-model/routines-as-named-exercise-lists.md）。
pub fn day_exercises(db: &Db, date: NaiveDate) -> Vec<ExerciseId> {
    copyable(db, date).map(|l| l.exercise_id).collect()
}

/// 指定日より**厳密に前**の、直近のメニュー候補（新しい順、最大 `limit` 件）。
///
/// 同じ種目構成の日は新しい方だけ残す。重複排除のキーを**種目集合**にしているのは、
/// 部位集合にすると A/B 法（同じ部位構成・違う種目）が 1 件に潰れ、利用者が却下した
/// 「直前の日をコピー」と同じものへ静かに退化するため。表示は部位名で出すが、
/// **キーとラベルは別物にする。**
pub fn recent_menus(db: &Db, before: NaiveDate, limit: usize) -> Vec<MenuCandidate> {
    let floor = before - TimeDelta::days(MENU_LOOKBACK_DAYS);
    let mut seen: Vec<Vec<ExerciseId>> = Vec::new();
    let mut out: Vec<MenuCandidate> = Vec::new();
    for key in db.sessions.range(..date_key(before)).rev().map(|(k, _)| k) {
        if out.len() >= limit {
            break;
        }
        let Some(date) = parse_date_key(key) else {
            continue;
        };
        // rev() なので日付は降順。1 つ下回ったら以降も全部下回る
        if date < floor {
            break;
        }
        let exercises: Vec<ExerciseId> = copyable(db, date).map(|l| l.exercise_id).collect();
        // 全種目がアーカイブ済み / 削除済み / 空セットの日は候補にしない（押せない行を作らない）
        if exercises.is_empty() {
            continue;
        }
        let mut dedupe_key = exercises.clone();
        dedupe_key.sort_unstable();
        if seen.contains(&dedupe_key) {
            continue;
        }
        seen.push(dedupe_key);
        out.push(MenuCandidate { date, exercises });
    }
    out
}

/// `from` の日のメニューを `to` の日へ複製し、**複製した種目 ID** を返す。
///
/// `at` は呼び出し側が渡す（core は時計を持たない）。当日なら `Some(now)`、
/// 過去日バックフィルなら `None`。
///
/// 体重と体調メモは複製しない。どちらも**その日**の観測値であってメニュー構成ではない。
/// **種目メモとセットメモは運ぶ** — あちらは種目に閉じたセッティング（「セーフティ 2 穴目」
/// 「グリップ広め」）が実質の中身で、毎回消えるほうが実害が大きい
/// （adr/ux/copy-carries-the-notes.md）。この非対称は意図。
pub fn copy_day(db: &mut Db, from: NaiveDate, to: NaiveDate, at: Option<i64>) -> Vec<ExerciseId> {
    // `&db` と `&mut db` の借用を分けるため先に取り出しておく
    let picked: Vec<Seed> = copyable(db, from).map(Seed::carry).collect();
    seed_day(db, to, picked, at)
}

/// コピーが**運ぶもの**だけを持つ。[`copy_day`] / [`apply_routine`] → [`seed_day`] の受け渡し。
///
/// ★ **`at` を持たない。これがこの型の存在理由。** [`ExerciseLog`] をそのまま渡すと元の日の
/// `at` が付いてきて、`at = None` にしたい過去日バックフィルに古い epoch が入る
/// （adr/data-model/at-optional-same-day-only.md）。「持ち込まない」を呼び出し側の
/// 注意力ではなく**フィールドの不在**で守る。
struct Seed {
    exercise_id: ExerciseId,
    sets: Vec<SetEntry>,
    note: String,
    label: Option<LabelId>,
}

impl Seed {
    /// コピー元のログから**運んでよいものだけ**を取り出す。**コピーの規則はここ 1 箇所。**
    ///
    /// 運ぶのはセット（重量・回数・セットメモ・ドロップの段）と種目メモ。`at` は運ばない
    /// （呼び出し側が渡す）。体重と体調メモは [`Session`] の側なのでここには届かない。
    fn carry(src: &ExerciseLog) -> Self {
        // ★ **分解して受ける。** フィールドで読むと、`ExerciseLog` に足した新しい
        //   フィールドがここを黙って素通りする — 「コピーが何を運ぶか」を決める場所が
        //   まさにここなのに、決め直す機会が失われる。分解しておけば追加した瞬間に
        //   この行がコンパイルエラーになり、運ぶ / 運ばないの判断を必ず通る
        //   （`merge_db` の `..log` は構造体更新なので黙って運ぶ側に倒れる。
        //   2 つの流儀が食い違ったまま出ていくのを止めるのはこのエラーだけ）
        let ExerciseLog {
            exercise_id,
            sets,
            at: _, // 運ばない。呼び出し側が渡す（この型に `at` が無いのがその保証）
            note,
            // ★ 運ぶ。`copy_day` は候補リストで日付を名指しして 1 日丸ごと写す操作で、
            //   ソースが可視なのでラベルはその日に実在した真実
            //   （adr/ux/label-chips-switch-the-history-and-the-copy.md）。
            //   `apply_routine` だけは [`Seed::without_label`] で落とす
            label,
        } = src;
        Self {
            exercise_id: *exercise_id,
            // ★ **セットも分解して組み直す。** `sets.clone()` だと `ExerciseLog` の
            //   ガードが 1 階層下で破れる — `SetEntry` に足したフィールドは
            //   コンパイルエラーにならずに黙って運ばれる。ドロップの段は運ぶと
            //   決めた（過去のログを再現する操作なので）が、次に足すものは
            //   ここで判断させる
            sets: sets
                .iter()
                .map(|s| {
                    let SetEntry {
                        weight,
                        reps,
                        note,
                        drops,
                    } = s;
                    SetEntry {
                        weight: *weight,
                        reps: *reps,
                        note: note.clone(),
                        // ★ 段も運ぶ。閉じていても行に見えるので、違えばその場で消せる
                        //   （adr/ux/copy-carries-the-notes.md と同じ規則）
                        drops: drops.clone(),
                    }
                })
                .collect(),
            note: note.clone(),
            label: *label,
        }
    }

    /// ラベルを落とす。**[`apply_routine`] 専用。**
    ///
    /// ★ メニューの展開は種目ごとに**別々の不可視の日**から引くので、たまたま最後が
    /// Power だった種目は今日が黙って Power になり、**来週の Power 履歴を汚染する** —
    /// この機能が直そうとしているバグそのものの再導入になる。[`copy_day`] は候補
    /// リストで日付を名指しする操作なのでソースが可視で、そちらは運ぶ側
    /// （adr/ux/label-chips-switch-the-history-and-the-copy.md）。
    ///
    /// ★ **名前で逸脱を可視化する。** `Seed::carry` の中で分岐すると、どちらの
    /// 呼び出しがどの規則で動いているのかが呼び出し側から読めない。
    fn without_label(mut self) -> Self {
        self.label = None;
        self
    }
}

/// 空の日に種目とセットを流し込む。**書いた種目 ID** を返す。
///
/// [`copy_day`]（1 日を丸ごと写す）と [`apply_routine`]（種目ごとに直近から引く）の
/// 共通の書き込み口。**ガードを 2 箇所に書かない** — 片方だけ緩められると
/// 「1 日 1 種目 1 ログ」（adr/data-model/one-log-per-exercise-per-day.md）が破れる。
fn seed_day(db: &mut Db, to: NaiveDate, picked: Vec<Seed>, at: Option<i64>) -> Vec<ExerciseId> {
    // ★ 書くものが無いならセッションを作らない。作ると「何も記録していない日」が
    //   カレンダーとバックアップに残る
    if picked.is_empty() {
        return Vec::new();
    }

    // ★ 既にログのある日には書かない。UI は「カードが 0 枚の日」にしか導線を出さないが、
    //   カードの再構築は Effect 経由なので「ログのある日 × 空のカード」の 1 tick が
    //   存在する。そこを踏むと exercise_id が重複して「1 日 1 種目 1 ログ」が壊れる
    if has_logs_on(db, to) {
        return Vec::new();
    }

    // ★ or_default で取る。insert で置き換えると、空の日に先に打ち込まれた
    //   体重・体調メモが消える（ConditionRow は 1 文字ごとに commit する）
    let session = db.sessions.entry(date_key(to)).or_default();
    let mut copied = Vec::with_capacity(picked.len());
    for Seed {
        exercise_id,
        sets,
        note,
        label,
    } in picked
    {
        copied.push(exercise_id);
        // ★ ExerciseLog を clone してはいけない。clone すると元の日の `at` を
        //   引き継ぎ、`at = None` にしたい過去日バックフィルに古い epoch が入る。
        //   日数表示は日付キーから出るので日付が嘘になることはもう無いが（adr/data-model/elapsed-in-local-calendar-days.md）、
        //   「その日に実施した時刻」として存在しない値が残り、同じ暦日にコピーしたときの
        //   時刻粒度が捏造される。記録の正直さは表示の都合とは別に守る（adr/data-model/at-optional-same-day-only.md）
        //
        //   ★ 引数が [`Seed`] なのはこのため。`at` を持たない型を通すことで、
        //     「持ち込まない」を呼び出し側の注意力ではなく型で守っている
        session.logs.push(ExerciseLog {
            exercise_id,
            sets,
            at,
            note,
            label,
        });
    }
    copied
}

// ── トレーニングメニュー（adr/data-model/routines-as-named-exercise-lists.md）──

/// そのメニューから**実際に展開される**種目 ID。
///
/// ★ [`usable_routines`] と [`apply_routine`] は必ず**これを通す**。2 つのフィルタが
/// ずれると「4 種目」と表示された候補を押しても何も起きない死んだボタンができる
/// （[`copyable`] とまったく同じ理由）。
///
/// - 存在しない種目・アーカイブ済みの種目は外す（`copyable` と同じ規則。展開で
///   アーカイブを復活させると、カードを閉じたあとユーザーが自力で戻せない種目になる）
/// - 重複は初出だけ残す。`normalize` が読み込み時に潰しているが、**編集中の `Db` も
///   ここを通る**ので受け口でも守る（「1 日 1 種目 1 ログ」は 3 層で守る）
fn expandable<'a>(db: &'a Db, r: &'a Routine) -> impl Iterator<Item = ExerciseId> + 'a {
    r.exercises
        .iter()
        .copied()
        .filter(|id| db.exercise(*id).is_some_and(|e| !e.archived))
        .filter(first_occurrence())
}

/// 「初出だけ残す」述語。`Iterator::filter` にも `Vec::retain` にもそのまま渡せる。
///
/// ★ **同じ種目を 2 回持たせない規則の唯一の実装。** 「1 日 1 種目 1 ログ」
/// （adr/data-model/one-log-per-exercise-per-day.md）はメニュー側で 3 箇所
/// （[`expandable`] / [`normalize_routines`] / [`merge_db`]）から守っているが、
/// **3 層で守るのと 3 回書き写すのは別のこと**で、後者はどれか 1 つが間違う口になる。
/// 順序を保つ（`sort` + `dedup` にしない）のは `dedupe_logs` と同じ理由 —
/// メニューの並びはそのまま記録タブのカードの並びになる。
///
/// 1 本のメニューは多くて 10 種目なので、`HashSet` ではなく `Vec::contains` で足りる。
fn first_occurrence() -> impl FnMut(&ExerciseId) -> bool {
    let mut seen: Vec<ExerciseId> = Vec::new();
    move |id| {
        let fresh = !seen.contains(id);
        if fresh {
            seen.push(*id);
        }
        fresh
    }
}

/// 記録タブの候補に出せるトレーニングメニュー 1 件。
#[derive(Clone, Debug, PartialEq)]
pub struct RoutineCandidate {
    pub id: RoutineId,
    /// 名前がメニューの同一性そのものなので、[`MenuCandidate`] と違ってここに持つ。
    pub name: String,
    /// **実際に展開される種目 ID。** メニューの並び順。
    pub exercises: Vec<ExerciseId>,
}

/// そのメニューを開いたときに**実際に出る種目の数**。0 なら候補にも出ない。
///
/// ★ 設定タブの「N 種目」はこれを出すこと。保存されている種目の数を出すと、
/// アーカイブ済みを 1 つ含むだけで「2 種目」と書いてあるのに 1 枚しか開かない、という
/// 食い違いになる。[`usable_routines`] と [`apply_routine`] が [`expandable`] を共有して
/// いるのと同じ理由で、**表示もここを通す**。
///
/// ★ 1 本ぶんの判定にわざわざ `usable_routines` を呼ばないこと。あちらは全メニューを
/// 走査して名前を clone した `Vec` を作るので、行ごとに呼ぶと N² 回の複製になる。
pub fn expandable_count(db: &Db, routine: RoutineId) -> usize {
    db.routine(routine).map_or(0, |r| expandable(db, r).count())
}

/// 候補に出せるメニュー（`Db::routines` の順）。
///
/// ★ **`limit` を取らない。** [`recent_menus`] の `limit` は「履歴を舐め続けないため」の
/// 走査打ち切りで、意味が違う。メニューはユーザーが自分で作った数しか無い。
///
/// ★ **`before` も取らない。** 「履歴のある種目が 1 つ以上あること」は条件にしない —
/// 履歴ゼロのメニューでも空のカードが並ぶので押した意味があり、**初めて組んだメニューが
/// 押せない**のは最悪の体験になる。
pub fn usable_routines(db: &Db) -> Vec<RoutineCandidate> {
    db.routines
        .iter()
        .filter_map(|r| {
            let exercises: Vec<ExerciseId> = expandable(db, r).collect();
            // 全種目が削除済み / アーカイブ済みのメニューは出さない（押せない行を作らない）
            (!exercises.is_empty()).then(|| RoutineCandidate {
                id: r.id,
                name: r.name.clone(),
                exercises,
            })
        })
        .collect()
}

/// メニューを `to` の日へ展開し、**その日に出すべき種目 ID** を並び順で返す。
///
/// セットは種目ごとに [`last_log_before`] から引く（[`copy_day`] が 1 日を丸ごと写すのに
/// 対し、こちらは**種目ごとに別々の日**から引く）。カード内の「前回をコピー」と同じ
/// 「前回」の定義になる。`at` は呼び出し側が渡す（core は時計を持たない）。
///
/// ★ **履歴が無い種目にはログを書かないが、返り値には含める。** 画面は
/// `views::day` の `pick()` と同じ「空のカード」として出す。セットが 0 本のログを
/// 書くと `dedupe_logs` が次回起動で落とし、**画面に出ているのに消える**という最悪の
/// 食い違いになる。`0×0` のダミーを入れる案も、指標・カレンダーのドット・`fmt_set`・
/// コピーを全部汚染するので採らない（adr/ux/start-from-a-saved-routine.md）。
///
/// ★ **`MENU_LOOKBACK_DAYS` は適用しない。** 種目カードの「前回」表示に上限が無いので、
/// ここだけ入れると「カードに『前回 730日前 60×10』と出ているのにメニューからは何も
/// 入らない」という食い違いが生まれる。
///
/// 体重と体調メモは複製しない。種目メモとセットメモは運ぶ（[`copy_day`] と同じ規則）。
///
/// ★ **メモも種目ごとに別々の日から来る。** セットの出所と必ず同じログなので、
/// 「ベンチのセッティング」と「スクワットのセッティング」が混ざることはない。
pub fn apply_routine(
    db: &mut Db,
    routine: RoutineId,
    to: NaiveDate,
    at: Option<i64>,
) -> Vec<ExerciseId> {
    let Some(r) = db.routine(routine) else {
        return Vec::new();
    };
    // ★ ログのある日には 1 枚も出さない。書き込みを止めるのは [`seed_day`] の仕事だが、
    //   ここでは**返り値（＝画面に出すカード）を決めるために答えが要る**ので先に問う。
    //   判定式そのものは `has_logs_on` の 1 箇所に畳んであるので、2 つがずれることはない
    if has_logs_on(db, to) {
        return Vec::new();
    }

    // ★ `last_log_before` が `&db` を借りるので、`&mut db` に入る前に集め切る
    //   （`copy_day` の `picked` と同じ形）
    let opened: Vec<ExerciseId> = expandable(db, r).collect();
    // ★ 履歴が無い種目はここで落とす。メモだけを引く形にすると `picked` が非空になって
    //   セッションが生まれ、`dedupe_logs` はメモのあるログを落とさないので**セット 0 本の
    //   ログがゴーストとして永続**する。そうなると `has_logs_on` が true になり、その日は
    //   候補リストから永久に外れる
    let picked: Vec<Seed> = opened
        .iter()
        // `last_log_before` はその種目のログしか返さないので `log.exercise_id == *ex`
        // ★ **読みは `Any`**（絞ると「カードに前回が出ているのにメニューからは何も
        //   入らない」食い違いが生まれる）。**書きは落とす**（[`Seed::without_label`]）
        .filter_map(|ex| {
            last_log_before(db, *ex, to).map(|(_, log)| Seed::carry(log).without_label())
        })
        .collect();

    // 履歴が 1 種目も無ければ `picked` は空。`seed_day` はセッションを作らずに返るので、
    // 空の日は空のまま残り、カードだけが画面に出る
    seed_day(db, to, picked, at);
    opened
}

/// その日にログがあるか。**「空の日か」の判定はこの 1 本を通す。**
///
/// ★ `is_trained()` ではなく `logs.is_empty()` で見る。空セットのログは `migrate` が
/// 読み込みのたびに落とすので通常は存在しないが、判定を緩めると「同じ種目のログが
/// 2 本ある」状態を作れてしまう側に倒れる。
fn has_logs_on(db: &Db, date: NaiveDate) -> bool {
    db.sessions
        .get(&date_key(date))
        .is_some_and(|s| !s.logs.is_empty())
}

/// `from`〜`to`（両端含む）のセッションを日付順で走査する。
fn sessions_in(
    db: &Db,
    from: NaiveDate,
    to: NaiveDate,
) -> impl Iterator<Item = (NaiveDate, &Session)> {
    db.sessions
        .iter()
        .filter_map(|(key, s)| Some((parse_date_key(key)?, s)))
        .filter(move |(date, _)| *date >= from && *date <= to)
}

/// 推移の 1 点。**ラベルを載せた `(date, value)`。**
/// adr/ux/label-colour-on-the-progress-dots.md
///
/// ★ `(NaiveDate, f64, Option<LabelId>)` の組にしない。推移タブは点の色と記録
/// テーブルの絞りで同じ列を 2 度使うので、添字ではなく名前で読めるほうが安全。
///
/// ★ **ラベル名も色も持たない。** 名前と色は種目の `labels` にあり、そこが
/// 唯一の真実源（[`label_name`] の「解決は必ずその種目の `labels` の中」）。
/// 点に写すと改名・色替えのたびに系列が古くなる。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SeriesPoint {
    pub date: NaiveDate,
    pub value: f64,
    /// その日のログに付いていたラベル。無ければ `None`。
    pub label: Option<LabelId>,
}

/// 種目別の推移（ラベル付き・絞り込みあり）。
///
/// ★ **絞りは `find` の述語に入れる**（[`logs_before`] と同じ形）。後ろで
/// `filter` すると「その日にその種目のログはあるがラベルが違う」場合に
/// `find` が先に当たって**日が丸ごと落ちる**のは同じだが、1 日 1 種目 1 ログの
/// 不変条件があるので結果は変わらない。述語に入れておくほうが規則が 1 本。
pub fn exercise_points(
    db: &Db,
    ex: ExerciseId,
    m: Metric,
    from: NaiveDate,
    to: NaiveDate,
    d: Drops,
    filter: LabelFilter,
) -> Vec<SeriesPoint> {
    sessions_in(db, from, to)
        .filter_map(|(date, session)| {
            let log = session
                .logs
                .iter()
                .find(|l| l.exercise_id == ex && !l.sets.is_empty() && filter.passes(l))?;
            Some(SeriesPoint {
                date,
                value: log_value_of(m, log, d),
                label: log.label,
            })
        })
        .collect()
}

/// 種目別の推移。
///
/// ★ **ラベルで絞らない。** 「旧名は旧挙動、新名がパラメータ付き」という
/// [`last_log_before`] / [`last_log_before_with`] と同じ作法で、この名前で呼んだ
/// 側の挙動は今までと 1 バイトも変わらない。
pub fn exercise_series(
    db: &Db,
    ex: ExerciseId,
    m: Metric,
    from: NaiveDate,
    to: NaiveDate,
    d: Drops,
) -> Vec<(NaiveDate, f64)> {
    exercise_points(db, ex, m, from, to, d, LabelFilter::Any)
        .into_iter()
        .map(|p| (p.date, p.value))
        .collect()
}

/// 部位別の推移。その部位の**全種目**の指標を日ごとに合算する。
///
/// 指標の式が 1 本になったので、自重種目と加重種目が混ざる部位（体幹など）でも
/// 合算が意味を持つ。旧実装がセット数固定だったのは `Kind` ごとに単位が違って
/// 足せなかったからで、その制約は無くなった。
pub fn group_series(
    db: &Db,
    g: GroupId,
    m: Metric,
    from: NaiveDate,
    to: NaiveDate,
    d: Drops,
) -> Vec<(NaiveDate, f64)> {
    group_points(db, g, m, from, to, d)
        .into_iter()
        .map(|p| (p.date, p.value))
        .collect()
}

/// [`group_series`] の [`SeriesPoint`] 版。
///
/// ★ **ラベルを見ない（`label` は常に `None`）。** ラベルは**種目ごとに独立**した
/// 分類体系なので、複数種目を合算した 1 点に載せられる `LabelId` が存在しない
/// （ベンチの `P` とスクワットの `P` は別 ID）。推移タブのチップ行も
/// 「種目を選んでいるときだけ」出す（`views::progress` の門番）ので、この枝に
/// 絞りが届くことはない。引数に `LabelFilter` を取らないのはその型での表明。
fn group_points(
    db: &Db,
    g: GroupId,
    m: Metric,
    from: NaiveDate,
    to: NaiveDate,
    d: Drops,
) -> Vec<SeriesPoint> {
    let ids = db.exercise_ids_of_group(g);
    if ids.is_empty() {
        return Vec::new();
    }
    sessions_in(db, from, to)
        .filter_map(|(date, session)| {
            let mut total = 0.0;
            let mut hit = false;
            for log in &session.logs {
                if log.sets.is_empty() || !ids.contains(&log.exercise_id) {
                    continue;
                }
                hit = true;
                total += log_value_of(m, log, d);
            }
            hit.then_some(SeriesPoint {
                date,
                value: total,
                label: None,
            })
        })
        .collect()
}

/// 実際に記録がある種目の ID。並びは `db.exercises` の順。
///
/// 推移タブの対象セレクタは**これで絞る**。プリセットの 28 種目を全部並べると、
/// 一度も使っていない種目を選んで空グラフを見る、という無意味な操作が普通に起きる。
pub fn used_exercise_ids(db: &Db) -> Vec<ExerciseId> {
    let used: std::collections::HashSet<ExerciseId> = db
        .sessions
        .values()
        .flat_map(|s| s.logs.iter())
        .filter(|l| !l.sets.is_empty())
        .map(|l| l.exercise_id)
        .collect();
    db.exercises
        .iter()
        .map(|e| e.id)
        .filter(|id| used.contains(id))
        .collect()
}

// ── 推移タブの対象（部位 + 種目） ───────────────────────────────────────────

/// 推移タブの対象。**部位と種目を必ず組で持つ。**
///
/// ★ 不変条件: `exercise` が `Some` なら `group` は**その種目の所属部位**。
///   ただし所属部位が `Db` から消えている種目（`migrate` / `merge_db` が
///   「宙に浮いた参照は宙に浮いたまま残す」ので実在しうる）では `None` になる。
///   この型のメソッド以外から状態を作らないことで成立させる。
///
/// ★ **画面のシグナルを部位・種目の 2 本に分けないための型**でもある。分けると
///   「部位を変えたら属さない種目を落とす」と「種目を選んだら部位を埋める」が
///   `Effect` で追随し合って循環する。加えて 2 回の `set` の間に render effect が
///   走るので、**新しい部位で絞った候補に古い部位の種目が選ばれている**中間状態が
///   DOM に出る（`<option selected>` がどれにも当たらず、ブラウザが先頭へ落とす）。
///   組で 1 つにすれば `set` が原子的になり、どちらも定義上ありえなくなる。
///
/// 両方 `None`（= どちらも「すべて」）は**グラフを描かない正当な状態**。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Pick {
    pub group: Option<GroupId>,
    pub exercise: Option<ExerciseId>,
}

impl Pick {
    /// 部位か種目のどちらかが選ばれているか。グラフを出すかどうかの判定に使う。
    pub fn is_set(self) -> bool {
        self.group.is_some() || self.exercise.is_some()
    }

    /// 部位セレクタを操作したときの新しい組。
    ///
    /// ★ **その部位に属さない種目は落とす。** `group` が `None`（すべて）なら種目も
    ///   必ず落ちる。「部位はすべて・種目は懸垂」のように 2 つのセレクタが食い違って
    ///   見える状態を作らないため（部位の「すべて」は絞り込みの解除ではなくリセット）。
    pub fn with_group(self, db: &Db, group: Option<GroupId>) -> Pick {
        let exercise = group.and_then(|g| {
            self.exercise
                .filter(|ex| db.exercise(*ex).is_some_and(|e| e.group_id == g))
        });
        Pick { group, exercise }
    }

    /// 種目セレクタを操作したときの新しい組。
    ///
    /// ★ **部位はその種目の所属で必ず上書きする**（種目から部位は一意に決まる）。
    ///   知らない種目 ID（消された種目 / 手で編集された保存値）は「すべて」に落とし、
    ///   部位だけ残す。`None`（すべて）も同じ腕に落ちる。
    ///
    /// ★ **所属部位が `Db` に無ければ `group` は `None`。** 宙に浮いた `group_id` を
    ///   そのまま入れると、どの `<option>` にも当たらず部位セレクタが黙って先頭
    ///   （「すべて」）に落ち、`pick` と表示が食い違う。`None` なら表示と一致し、
    ///   種目リストも絞られない（絞る部位が無いのだから正しい）。
    pub fn with_exercise(self, db: &Db, exercise: Option<ExerciseId>) -> Pick {
        match exercise.and_then(|ex| db.exercise(ex)) {
            Some(e) => Pick {
                group: db.group(e.group_id).map(|g| g.id),
                exercise: Some(e.id),
            },
            None => Pick {
                group: self.group,
                exercise: None,
            },
        }
    }

    /// 保存する文字列の組 `(部位, 種目)`。
    ///
    /// ★ `storage` が書く形そのもの。読み戻した生の値と突き合わせて
    ///   「候補から落ちた ID が保存値に残っていないか」を見るのにも使う。
    pub fn ids(self) -> (Option<String>, Option<String>) {
        (
            self.group.map(|id| id.to_string()),
            self.exercise.map(|id| id.to_string()),
        )
    }
}

/// 部位の並び順。`order` が同値なら宣言順。**知らない部位は末尾**。
fn group_rank(db: &Db, g: GroupId) -> (u32, usize) {
    db.groups
        .iter()
        .position(|x| x.id == g)
        .map_or((u32::MAX, usize::MAX), |i| (db.groups[i].order, i))
}

/// 推移タブのセレクタに出す種目。**記録がある種目だけ**を、画面と同じ並びで返す。
///
/// 並びは 部位の順 → 部位内の `order` → `id`。**アーカイブ済みは末尾**に回す
/// （記録があるものは消さない — 消すと過去データが参照不能になる）。
///
/// ★ 並べ替えを画面側に置かないのは、**既定値の規則（先頭）とセレクタの並びが
///   2 箇所に分かれる**のを避けるため。片方だけ直すと「既定は先頭の種目」が静かにずれる。
pub fn progress_candidates(db: &Db) -> Vec<ExerciseId> {
    let used = used_exercise_ids(db);
    if used.is_empty() {
        return Vec::new();
    }
    let mut sorted: Vec<&Exercise> = db
        .exercises
        .iter()
        .filter(|e| used.contains(&e.id))
        .collect();
    sorted.sort_by_key(|e| (e.archived, group_rank(db, e.group_id), e.order, e.id));
    sorted.into_iter().map(|e| e.id).collect()
}

/// 推移タブのセレクタに出す部位。**記録がある種目を 1 つ以上持つ部位だけ**を順に。
///
/// ★ アーカイブ済みの種目も数える。その部位を選べば合算に載るので、
///   セレクタから消すと「グラフには出るのに部位が選べない」食い違いになる。
pub fn progress_candidate_groups(db: &Db) -> Vec<GroupId> {
    let used = used_exercise_ids(db);
    if used.is_empty() {
        return Vec::new();
    }
    let mut groups: Vec<&Group> = db
        .groups
        .iter()
        .filter(|g| {
            db.exercises
                .iter()
                .any(|e| e.group_id == g.id && used.contains(&e.id))
        })
        .collect();
    // db.groups の順で集めたあと安定ソートするので、`order` が同値なら宣言順が残る
    groups.sort_by_key(|g| g.order);
    groups.into_iter().map(|g| g.id).collect()
}

/// 保存が無いときの既定。**記録がある先頭の種目**（部位はその所属で埋まる）。
///
/// ★ 部位ではなく種目を既定にするのは、この画面の主眼が種目別の仕事量だから
///   （改修前の `Options::first` と同じ規則）。記録が 1 件も無ければ両方「すべて」。
pub fn default_pick(db: &Db) -> Pick {
    Pick::default().with_exercise(db, progress_candidates(db).first().copied())
}

/// 保存値 → 実際に使う対象。**必ず候補の中にある組か [`default_pick`] を返す。**
///
/// 引数が `Option<&str>` なのは `storage::UiState` の受け口に合わせたもの
/// （[`history_count`] が `Option<i64>` を受けるのと同じ理由）。JSON にどんな文字列が
/// 入っていてもパースを落とさず、検証をここ（ホストのテストが届く側）でやる。
///
/// ★ **種目が生きていれば種目が勝つ**（部位はその所属で埋め直す）。保存された部位と
///   食い違っていても種目に寄せる — 部位は種目から一意に決まるので、食い違いは
///   保存後に種目が別の部位へ移されたことしか意味しない。
/// ★ **両方「すべて」は既定に倒す。** 案内文だけの画面を復元して見せる価値がない。
pub fn restore_pick(db: &Db, group: Option<&str>, exercise: Option<&str>) -> Pick {
    let ex = exercise
        .and_then(|s| s.parse::<ExerciseId>().ok())
        .filter(|id| progress_candidates(db).contains(id));
    if ex.is_some() {
        return Pick::default().with_exercise(db, ex);
    }
    let g = group
        .and_then(|s| s.parse::<GroupId>().ok())
        .filter(|id| progress_candidate_groups(db).contains(id));
    match g {
        Some(g) => Pick {
            group: Some(g),
            exercise: None,
        },
        None => default_pick(db),
    }
}

/// 選んだ組の推移。種目が入っていればその種目、部位だけならその部位の合算、
/// どちらも「すべて」なら空。
///
/// ★ グラフ側と記録テーブル側で同じ `match` を 2 度書かないための 1 本。
pub fn pick_series(
    db: &Db,
    p: Pick,
    m: Metric,
    from: NaiveDate,
    to: NaiveDate,
    d: Drops,
) -> Vec<(NaiveDate, f64)> {
    pick_points(db, p, m, from, to, d, LabelFilter::Any)
        .into_iter()
        .map(|p| (p.date, p.value))
        .collect()
}

/// [`pick_series`] の [`SeriesPoint`] 版（ラベル絞り込みつき）。
///
/// ★ **絞りが効くのは種目の枝だけ。** 部位の枝は [`group_points`] が
/// ラベルを見ない（合算にラベルは載らない）。呼び側の門番が「種目を選んで
/// いるときだけ `Only`」に倒しているので、ここで `Only` が部位の枝へ届くことは
/// 無いが、届いても**黙って無視する**（0 件にして空グラフを見せない）。
pub fn pick_points(
    db: &Db,
    p: Pick,
    m: Metric,
    from: NaiveDate,
    to: NaiveDate,
    d: Drops,
    filter: LabelFilter,
) -> Vec<SeriesPoint> {
    match (p.exercise, p.group) {
        (Some(ex), _) => exercise_points(db, ex, m, from, to, d, filter),
        (None, Some(g)) => group_points(db, g, m, from, to, d),
        (None, None) => Vec::new(),
    }
}

/// その対象・期間に**ドロップセットの段があるか**。
///
/// 推移タブの注記に使う。外したことを黙っていると、記録タブの合計と食い違う理由が
/// どこにも出ないまま「グラフが実際より低い」ように見える。
///
/// ★ **`Drops` を受け取らない。** 「段が在るか」はデータの事実で、それを注記に
/// するかどうかは画面の設定。`Period` を `(from, to)` に解いて渡すのと同じ線で、
/// 合成は `views::progress` 側でやる。
///
/// ★ 部位の ID 集合は `sessions_in` の**外**で 1 回だけ引く。`Db::exercise_ids_of_group`
/// は毎回 `Vec` を作るので、ログごとに呼ぶと 2 年ぶんで千回単位の確保になる
/// （[`group_series`] が同じ理由で外に出している）。
pub fn any_drops_in_scope(db: &Db, p: Pick, from: NaiveDate, to: NaiveDate) -> bool {
    if !p.is_set() {
        return false;
    }
    let ids = p
        .exercise
        .is_none()
        .then(|| p.group.map(|g| db.exercise_ids_of_group(g)));
    let in_scope = |log: &ExerciseLog| match (p.exercise, ids.as_ref()) {
        (Some(ex), _) => log.exercise_id == ex,
        (None, Some(Some(ids))) => ids.contains(&log.exercise_id),
        _ => false,
    };
    sessions_in(db, from, to).any(|(_, session)| {
        session
            .logs
            .iter()
            .any(|log| in_scope(log) && log.sets.iter().any(|s| !s.drops.is_empty()))
    })
}

/// 週の始まりは**日曜**（カレンダー画面の 日〜土 グリッドに合わせる）。
pub fn week_start(d: NaiveDate) -> NaiveDate {
    d - TimeDelta::days(i64::from(d.weekday().num_days_from_sunday()))
}

/// 「全期間」表示用の週単位集約。キーは週の開始日（日曜）。
pub fn aggregate_weekly(series: &[(NaiveDate, f64)]) -> Vec<(NaiveDate, f64)> {
    let mut weeks: BTreeMap<NaiveDate, f64> = BTreeMap::new();
    for (date, value) in series {
        *weeks.entry(week_start(*date)).or_insert(0.0) += *value;
    }
    weeks.into_iter().collect()
}

/// [`aggregate_weekly`] の [`SeriesPoint`] 版。値の集約は**まったく同じ（合計）**。
///
/// ★ **ラベルは「週内の全点が同じ 1 つのラベル」のときだけ残す。** それ以外は
/// `None` = 既定色。「すべて」表示で H / P / S が混ざった週に 1 色を選ぶと、
/// **その週の色が嘘になる**（3 分の 1 だけを指す色が週全体の点に付く）。
/// 逆に `Only` で絞っていれば週内は全部同じラベルなので、「全期間」でも
/// ちゃんとそのラベルの色が出る — チップで絞ったときに色が消えない。
///
/// ★ 加算の順序は [`aggregate_weekly`] と同じ（呼び側が日付昇順で渡す）。
/// f64 の加算は非結合なので、順序を変えると同じ入力で 1 ulp ずれる。
pub fn aggregate_weekly_points(series: &[SeriesPoint]) -> Vec<SeriesPoint> {
    // (合計, 最初に見たラベル, 週内で食い違ったか)
    let mut weeks: BTreeMap<NaiveDate, (f64, Option<LabelId>, bool)> = BTreeMap::new();
    for p in series {
        let slot = weeks
            .entry(week_start(p.date))
            .or_insert((0.0, p.label, false));
        slot.0 += p.value;
        if slot.1 != p.label {
            slot.2 = true;
        }
    }
    weeks
        .into_iter()
        .map(|(date, (value, label, mixed))| SeriesPoint {
            date,
            value,
            label: (!mixed).then_some(label).flatten(),
        })
        .collect()
}

// ── 体重（グラフの第2軸）──────────────────────────────────────────────────

/// グラフに載せられる体重の範囲。**これを外れた値は系列から落とす**（データは消さない）。
///
/// 上限が要るのは、入力側（`ConditionRow` の commit）も [`migrate`] も
/// `is_finite() && > 0.0` しか見ておらず、`f32` の上限まで素通りするため。
/// `3e38` のような値が 1 点でも混じると [`weight_band`] の帯が f64 の丸めで
/// 潰れ、`(v - lo) / (hi - lo)` が 0/0 = NaN になる。NaN 座標の `points` 属性は
/// SVG のパースエラーで**折れ線が丸ごと描かれなくなる**（例外も出ない）。
///
/// 999.5 に置いているのは軸ラベルの桁数を 5 文字（"999.5" / "1000"）で頭打ちにするため。
/// これで右軸ラベルが viewBox から溢れないことが桁数で保証できる。
pub const WEIGHT_MAX: f64 = 999.5;

/// 第2軸の目盛り刻み。ラベルを 0.5kg の倍数に乗せる。
pub const WEIGHT_TICK: f64 = 0.5;

/// 体重の推移。**トレーニングしていない日も点になる。**
///
/// `Session::is_empty()` が体重だけの日を「空ではない」と扱うので、休養日の計量も
/// セッションとして残っている。指標の系列（`exercise_series` / `group_series`）が
/// トレした日にしか点を持たないのと対照的で、体重の方が密になる。
pub fn body_weight_series(db: &Db, from: NaiveDate, to: NaiveDate) -> Vec<(NaiveDate, f64)> {
    sessions_in(db, from, to)
        .filter_map(|(date, session)| {
            let w = f64::from(session.body_weight?);
            (w.is_finite() && w > 0.0 && w <= WEIGHT_MAX).then_some((date, w))
        })
        .collect()
}

/// 週単位の**平均**。キーは [`aggregate_weekly`] と同じ週の開始日（日曜）。
///
/// ★ 体重を [`aggregate_weekly`]（合計）に通すと「全期間」で 400kg になる。
/// 指標と体重が同じグラフに乗る以上、**週キーが一致すること**が要件で、
/// 集約の仕方だけが違う（指標は合計、体重は平均）。
///
/// 既に週次の系列に対しては冪等（各週 1 点の平均はその点自身）。
pub fn aggregate_weekly_avg(series: &[(NaiveDate, f64)]) -> Vec<(NaiveDate, f64)> {
    let mut weeks: BTreeMap<NaiveDate, (f64, u32)> = BTreeMap::new();
    for (date, value) in series {
        let slot = weeks.entry(week_start(*date)).or_insert((0.0, 0));
        slot.0 += *value;
        slot.1 += 1;
    }
    weeks
        .into_iter()
        .map(|(k, (sum, n))| (k, sum / f64::from(n)))
        .collect()
}

/// 体重の第2軸の帯 `(lo, hi)`。
///
/// **契約: 返り値は必ず `[lo_v, hi_v]` を含み、`hi > lo` かつ両端が有限。**
/// 呼び出し側は `(v - lo) / (hi - lo)` をゼロ除算の心配なく使える。
///
/// ★ 0 起点にしない。指標の軸（0〜max×1.1）と同じ作りにすると 60〜65kg が
/// 画面上端に貼り付いた平線になり、体重の遷移が読めない。
///
/// 0.5kg 刻みに外側へ丸めたあと、**目盛り区間数が 2 以上かつ偶数**になるまで広げる。
/// 偶数にするのはグリッド 3 本の中央ラベルも 0.5 の倍数に乗せるため
/// （そうしないと `62.45` のような目盛りが出る）。広げる側は余白の少ない方を選び、
/// データを帯の中央へ寄せる（単一点が下端に貼り付いて 0 グリッド線と重なるのを防ぐ）。
pub fn weight_band(lo_v: f64, hi_v: f64) -> (f64, f64) {
    // ★ ここで弾かないと下のループが止まらない。NaN は `(hi-lo)/TICK` を NaN にし、
    //   `as i64` が 0 になって偶数条件を永久に満たさない
    if !lo_v.is_finite() || !hi_v.is_finite() {
        return (0.0, 2.0 * WEIGHT_TICK);
    }
    // `body_weight_series` が既に範囲外を落としているが、帯の計算単体でも契約を守る。
    // 上限を切ることで `(hi - lo) / TICK` が i64 に収まり、飽和して奇数のまま
    // 回り続ける経路も塞がる
    let lo_v = lo_v.clamp(0.0, WEIGHT_MAX);
    let hi_v = hi_v.clamp(lo_v, WEIGHT_MAX);

    let mut lo = (lo_v / WEIGHT_TICK).floor() * WEIGHT_TICK;
    let mut hi = (hi_v / WEIGHT_TICK).ceil() * WEIGHT_TICK;
    // 1 周ごとに区間数がちょうど 1 増えるので 3 周以内に必ず抜ける
    loop {
        let ticks = ((hi - lo) / WEIGHT_TICK).round() as i64;
        if ticks >= 2 && ticks % 2 == 0 {
            // ★ 体重の軸に負の目盛りを出さない。0.1kg のような打ち間違いが 1 点でも
            //   残っていると中央寄せで下端が -0.5 になる。幅（= 区間の偶数条件）を
            //   保ったまま帯ごと持ち上げる。上へずらすだけなので含有は壊れない
            if lo < 0.0 {
                return (0.0, hi - lo);
            }
            return (lo, hi);
        }
        if lo_v - lo <= hi - hi_v {
            lo -= WEIGHT_TICK;
        } else {
            hi += WEIGHT_TICK;
        }
    }
}

// ── 経過時間 ────────────────────────────────────────────────────────────────

/// 最後のトレーニングからの間隔。
///
/// ★ **日数はローカル暦の日差**であって「経過ミリ秒 / 24h」ではない。後者はトレした
///   時刻の 24 時間後に繰り上がるローリング日数で、8/8 20:00 の記録が 8/9 08:00 に
///   「今日」と出る（実際に出ていた。朝トレなら繰り上がりが UTC 深夜に来るので
///   「アプリが UTC で計っている」ように見える）。`ms` を private にしてあるのは
///   **この導出を型で書けなくする**ため。日数を読む経路は `days()` だけにする。
///   adr/data-model/elapsed-in-local-calendar-days.md 参照。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Elapsed {
    /// `today - 日付キー`。常に 0 以上
    days: i64,
    /// 当日入力の `at` があるときだけの経過ミリ秒。**日数の導出には使わない**
    /// （`days == 0` のときの時刻粒度表示のためだけに持つ）。常に 0 以上
    ms: Option<i64>,
}

impl Elapsed {
    /// **唯一の生成口。** 日数と時刻粒度を 1 箇所で同時に導出することで、
    /// 「片方だけ渡して食い違わせる」余地を残さない。
    pub fn since(today: NaiveDate, date: NaiveDate, last_at: Option<i64>, now_ms: i64) -> Self {
        Self {
            days: (today - date).num_days().max(0),
            ms: last_at.map(|at| (now_ms - at).max(0)),
        }
    }

    /// ローカル暦の日差。**画面が日数を出す唯一の経路。**
    pub fn days(self) -> i64 {
        self.days
    }

    #[cfg(test)]
    fn days_only(days: i64) -> Self {
        Self {
            days: days.max(0),
            ms: None,
        }
    }

    #[cfg(test)]
    fn with_ms(days: i64, ms: i64) -> Self {
        Self {
            days: days.max(0),
            ms: Some(ms.max(0)),
        }
    }
}

/// `keep` に合致するログを持つ最新セッション（`today` 以前）から間隔を出す。
///
/// 日数は必ず日付キーから出す。`now_ms` は**日数の計算には使われず**、そのセッションに
/// `Some(at)` があるときの時刻粒度（同じ暦日の中でだけ表示に出る）にしか効かない。
fn elapsed_matching(
    db: &Db,
    now_ms: i64,
    today: NaiveDate,
    keep: impl Fn(&ExerciseLog) -> bool,
) -> Option<Elapsed> {
    // 未来日のセッションは「最後のトレーニング」にしない（負の経過を出さない）
    db.sessions
        .range(..=date_key(today))
        .rev()
        .find_map(|(key, session)| {
            let date = parse_date_key(key)?;
            let mut trained = false;
            let mut last_at: Option<i64> = None;
            for log in &session.logs {
                if log.sets.is_empty() || !keep(log) {
                    continue;
                }
                trained = true;
                if let Some(at) = log.at {
                    last_at = Some(last_at.map_or(at, |cur: i64| cur.max(at)));
                }
            }
            if !trained {
                return None;
            }
            // ★ 日数は必ず日付キーから。`at` があっても暦の日差が真実源
            Some(Elapsed::since(today, date, last_at, now_ms))
        })
}

pub fn elapsed_since_last(db: &Db, now_ms: i64, today: NaiveDate) -> Option<Elapsed> {
    elapsed_matching(db, now_ms, today, |_| true)
}

/// 部位ごとの経過時間。記録が一度もない部位はキーごと出ない（画面は「—」を出す）。
pub fn elapsed_by_group(db: &Db, now_ms: i64, today: NaiveDate) -> HashMap<GroupId, Elapsed> {
    let mut out = HashMap::new();
    for group in &db.groups {
        let ids = db.exercise_ids_of_group(group.id);
        if ids.is_empty() {
            continue;
        }
        if let Some(e) = elapsed_matching(db, now_ms, today, |l| ids.contains(&l.exercise_id)) {
            out.insert(group.id, e);
        }
    }
    out
}

/// 日粒度の文言。「今日 / 昨日 / N日前」。
pub fn humanize_days(days: i64, lang: Lang) -> String {
    match days.max(0) {
        0 => lang.strings().core.today.to_string(),
        1 => lang.strings().core.yesterday.to_string(),
        n => lang.days_ago(n),
    }
}

/// 日を跨いだら日数、同じ暦日なら時刻粒度。
///
/// ★ 「2日5時間」形式は廃止した。日数部分が経過ミリ秒 / 24h のローリング日数だったため、
///   チップ（日粒度）とヒーロー（時刻粒度）が違う日を指すことがあった（adr/data-model/elapsed-in-local-calendar-days.md）。
pub fn humanize(e: Elapsed, lang: Lang) -> String {
    if e.days > 0 {
        return humanize_days(e.days, lang);
    }
    // 同じ暦日。`at` があるときだけ時刻粒度まで出せる
    let Some(ms) = e.ms else {
        return humanize_days(0, lang);
    };
    let minutes = ms / 60_000;
    if minutes < 1 {
        return lang.strings().core.just_now.to_string();
    }
    if minutes < 60 {
        return lang.minutes_ago(minutes);
    }
    let hours = minutes / 60;
    // ★ 同じ暦日なのに 24 時間以上 = `at` と日付キーが矛盾している（壊れたバックアップの
    //   取り込み、タイムゾーン移動、copy_day の `at` 漏れの退行）。日付キーを勝たせる。
    //   ここが無いと「336時間」のような表示が出る
    if hours < 24 {
        lang.hours_ago(hours)
    } else {
        humanize_days(0, lang)
    }
}

/// 部位チップ用の短い表記。"3d" / "今日"
///
/// ★ `views` ではなくここに置く。元は `views/mod.rs` にあったが、そこは wasm32 の
///   cfg gate の内側で `cargo test` が一度も触れず、`ms / 86_400_000` というバグが
///   誰にも検出されないまま残っていた（adr/architecture/chart-layout-as-a-testable-module.md と同じ理由でロジックを core に置く）。
pub fn short_elapsed(e: Elapsed, lang: Lang) -> String {
    match e.days() {
        0 => lang.strings().core.today.to_string(),
        // ★ "3d" は両言語で同じ。部位チップは幅が数十 px しかないので、
        //   英語でも "3 days" には広げない
        d => format!("{d}d"),
    }
}

/// チップの濃淡。**部位カラー × 経過濃淡の二重符号化を避けるため単色系に統一する。**
pub fn recency_class(e: Option<Elapsed>) -> &'static str {
    let Some(e) = e else { return "none" };
    match e.days() {
        0..=1 => "fresh",
        2..=3 => "recent",
        4..=6 => "stale",
        _ => "old",
    }
}

// ── お知らせ（新機能バナー） ────────────────────────────────────────────────

/// 未読のお知らせ。`notes` は新しい順、`last_seen` は `UiState.release_seen` の生値。
///
/// ★ `None`（未設定）は「全部既読」に倒す。新規利用者にも、この機能が乗る前からの
///   利用者にも過去分を出さないため（基準値を書くのは `whatsnew::bootstrap`）。
///
/// 引数を `&'static` にしていない。呼出側が `i18n::RELEASES` を渡せば省略記法で
/// `'static` が返るので、この形のほうがテストからローカルの配列を渡せて素直になる。
pub fn unseen_releases(notes: &[ReleaseNote], last_seen: Option<i64>) -> &[ReleaseNote] {
    let Some(last_seen) = last_seen else {
        return &[];
    };
    // notes は新しい順なので、未読は先頭からの連続 prefix
    let n = notes
        .iter()
        .take_while(|r| i64::from(r.id) > last_seen)
        .count();
    &notes[..n]
}

/// 最新のお知らせ番号。`notes` が空なら `None`。
pub fn latest_release_id(notes: &[ReleaseNote]) -> Option<u32> {
    notes.first().map(|r| r.id)
}

// ── 復元 ────────────────────────────────────────────────────────────────────

/// 復元の失敗。**「壊れている」と「新しすぎる」を分ける。**
///
/// 後者はデータが無傷なので、新しい版を入れ直せば救える。同じ文言で通知すると
/// 利用者が「全部消えた」と判断して諦めてしまう。
#[derive(Debug)]
pub enum RestoreError {
    /// JSON として読めない / このアプリのデータではない。
    Broken(serde_json::Error),
    /// 未知の（= 将来の）schema。**触っていないことを利用者に伝える。**
    Unsupported(u32),
}

impl std::fmt::Display for RestoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Broken(e) => write!(f, "読み込めない JSON: {e}"),
            Self::Unsupported(v) => write!(f, "未知の schema {v}"),
        }
    }
}

/// schema 差の吸収 + 「1 日 1 種目 1 ログ」への正規化。
///
/// `Err` なら呼び側（`storage.rs`）が raw を退避してからプリセット入りの `Db` に
/// フォールバックする。**破損データをプリセットで黙って上書きしない**ための境界。
///
/// ★ **schema を見て分岐する。** 見ずに `Db` として読むと、未来の版が書いた JSON を
/// 古い版が「知らないフィールドを落として」読み込み、そのまま書き戻してしまう。
///
/// 正規化の内容:
/// - 日付キーを `%Y-%m-%d` に再正規化し、パースできないキーのセッションは捨てる
///   （辞書順 = 時系列順の前提が壊れ、どの画面からも到達できないため）
/// - 同一 `exercise_id` の重複ログをマージ（セットを連結、`at` は `Some` の最大値、
///   メモは重複ガード付きで連結）
/// - 空白だけのメモを空にする
/// - **セットもメモも無い**ログを捨て、ログも体重もメモも無いセッションを捨てる
/// - トレーニングメニューの空白だけの名前を空にし、種目の重複を潰し、名前も種目も
///   無いものを捨て、**ID が重複しているものに採番し直す**
///   （adr/data-model/routines-as-named-exercise-lists.md）
pub fn migrate(raw: &str, ids: &mut IdGen) -> Result<Db, RestoreError> {
    // schema だけを先に取り出す。本体の形が世代ごとに違うので 2 段パースになる
    #[derive(serde::Deserialize)]
    struct Probe {
        schema: u32,
    }
    let probe: Probe = serde_json::from_str(raw).map_err(RestoreError::Broken)?;

    let mut db = match probe.schema {
        // 連番 u32 ID の世代。全参照を乱数 ID へ張り替える
        0..=2 => {
            let old: legacy::Db = serde_json::from_str(raw).map_err(RestoreError::Broken)?;
            upgrade_from_sequential(old, ids)
        }
        3 => serde_json::from_str(raw).map_err(RestoreError::Broken)?,
        other => return Err(RestoreError::Unsupported(other)),
    };

    normalize(&mut db, ids);
    db.schema = SCHEMA;
    Ok(db)
}

/// 世代に依らない正規化。
///
/// `ids` はトレーニングメニューの ID 重複を解くためだけに使う（[`normalize_routines`]）。
fn normalize(db: &mut Db, ids: &mut IdGen) {
    normalize_routines(db, ids);
    normalize_exercises(db, ids);

    let mut sessions: BTreeMap<String, Session> = BTreeMap::new();
    for (key, session) in std::mem::take(&mut db.sessions) {
        let Some(date) = parse_date_key(&key) else {
            continue;
        };
        merge_same_day(sessions.entry(date_key(date)).or_default(), session);
    }
    for session in sessions.values_mut() {
        drop_unrepresentable_weights(session);
        // ★ dedupe_logs より**先**。あちらは空白だけのメモを「ある」と見るので、
        //   先に潰さないと「保存する価値の無いログ」が残る
        blank_notes_to_empty(session);
        prune_empty_drops(session);
        dedupe_logs(session);
    }
    sessions.retain(|_, s| !s.is_empty());
    db.sessions = sessions;
}

/// トレーニングメニューの正規化（adr/data-model/routines-as-named-exercise-lists.md）。
///
/// ★ **存在しない種目・アーカイブ済みの種目への参照は消さない。** このアプリに種目の
/// 物理削除は無い（`views::settings` にあるのは `set_archived` だけ）ので、宙に浮いた
/// 参照は「他端末のデータ」しか作らない。つまり**後から相手のファイルを取り込めば
/// 生き返る**種類の参照であり、読み込みのたびに消すと生き返らせる機会を奪う。しかも
/// 消すのは不可逆で、残しても被害は「候補に出ない」だけ（可逆）。
/// `upgrade_from_sequential` の「宙に浮いた参照は宙に浮いたまま残す」と同じ立場。
///
/// 「押しても何も起きない死んだボタンを作らない」責任は、読み出し側の [`expandable`] が
/// 全部持つ（[`copyable`] が `recent_menus` と `copy_day` の両方を通っているのと同じ形）。
fn normalize_routines(db: &mut Db, ids: &mut IdGen) {
    // ★ 空白だけの名前を空にするのが**先**。後ろの `is_empty()` は trim して見るので、
    //   ここを飛ばすと `" "` が「名前がある」と「空」の判定でズレる。
    //   ★ **ここでは trim しない**（`blank_notes_to_empty` と同じ規則）。この関数が
    //   触るのは自分が作ったのではないデータ — 取り込んだファイルや旧版が書いた JSON —
    //   なので、両端の空白を削るのは書き換えになる。`views::settings` の編集シートが
    //   保存時に trim するのは矛盾しない。あちらは利用者が入力欄を見て「保存」を押した
    //   結果で、部位・種目の 4 つのエディタも同じく trim している
    for r in &mut db.routines {
        if r.name.trim().is_empty() {
            r.name.clear();
        }
        // ★ 重複は初出だけ残す。展開時に同一 `exercise_id` のログが 2 本でき、
        //   「1 日 1 種目 1 ログ」が破れる（adr/data-model/one-log-per-exercise-per-day.md）
        r.exercises.retain(first_occurrence());
    }
    db.routines.retain(|r| !r.is_empty());

    // ★ ID の重複だけは放置できない。画面は `<For key=id>` に使うので重複キーは
    //   keyed diff を壊し（wasm では panic = アプリが死ぬ）、削除は
    //   `retain(|r| r.id != id)` なので**片方消すと両方消える**。
    //   捨てずに採番し直すのは、名前を付けて組んだリストを黙って失わないため
    let mut seen: Vec<RoutineId> = Vec::with_capacity(db.routines.len());
    for r in &mut db.routines {
        if seen.contains(&r.id) {
            r.id = ids.alloc();
        }
        seen.push(r.id);
    }
}

/// 種目の正規化。[`Exercise::pins`]（マシンのピン）と
/// [`Exercise::interval_sec`]（インターバル）を見る。
///
/// ★ ここの `split_whitespace` は**書式の整形ではなく構造の適用**なので、
/// [`normalize_routines`] の「取り込んだデータを trim しない」規則とは矛盾しない。
/// あちらが守るのは利用者が書いた**自由記述の中身**で、こちらが守るのは
/// 「1 要素 = 空白を含まない 1 個の値」という [`Exercise::pins`] の不変条件。
/// TSV の `ピン` 列はセル内を空白で区切る（[`export_tsv`]）ので、ここが崩れると
/// 書き出して読み戻した値が一致しなくなる。
fn normalize_exercises(db: &mut Db, ids: &mut IdGen) {
    for e in &mut db.exercises {
        e.pins = clean_pins(std::mem::take(&mut e.pins));
        e.interval_sec = clean_interval(e.interval_sec);
        e.labels = clean_labels(std::mem::take(&mut e.labels), ids);
    }
}

/// [`normalize_exercises`] と [`set_pins`] が共有する 1 種目ぶんの規則。
///
/// ★ **2 経路に分けて書かない。** 食い違うと「画面で消えたはずの値が取り込みで
/// 生き返る」「取り込みで落ちた値が画面からは入る」が起きる。
///
/// ★ 重複は消さない。「シート高 3 / バー位置 3」は正当な入力で、
/// [`normalize_routines`] が `ExerciseId` の重複を消すのとは前提が違う（あちらは
/// 消さないと「1 日 1 種目 1 ログ」が破れる）。
///
/// ★ 並べ替えない。`Vec` の順が「上から下へ触る順」そのもの。
/// ドロップの段の正規化。**「段が在る」を決めるのはここ 1 箇所。**
///
/// ★ [`clean_pins`] / [`clean_interval`] と**同じ理由で共有する**。画面（`views::day` の
/// `commit`）・取り込み（`parse_drops_cell` → [`normalize`]）・読み込み
/// （[`prune_empty_drops`]）の 3 経路が別々に規則を持つと、「画面から打てるのに取り込みで
/// 丸められる」「取り込みで残るのに画面で消える」が起きる。
///
/// 落とすのは 3 つ:
///
/// - **回数 0**。挙げていないものは段ではない
/// - **`f32` で表せない重量**。残すと `"weight":null` が保存され、次回起動の `Db` の
///   パースが丸ごと落ちる（メインセットで潰した経路と同型）
/// - **[`MAX_DROPS`] を超えた分**
fn clean_drops(drops: Vec<DropStage>) -> Vec<DropStage> {
    drops
        .into_iter()
        .filter(|d| d.reps > 0 && d.weight.is_finite() && d.weight >= 0.0)
        .take(MAX_DROPS)
        .collect()
}

fn clean_pins(pins: Vec<String>) -> Vec<String> {
    pins.iter()
        .flat_map(|p| p.split_whitespace())
        // ★ char で数える。バイトで切ると UTF-8 の途中で割れて panic する
        .map(|p| p.chars().take(MAX_PIN_LEN).collect::<String>())
        // ★ 分割の**後**で本数を見る。`"1 2 3 4 5 6 7 8 9"` が 1 要素で入りうる
        .take(MAX_PINS)
        .collect()
}

/// 種目のマシンのピンを差し替える（記録タブの入力欄から呼ぶ）。
///
/// ★ 正規化を `views` 側に持たせない。`views/*` には unit test が無いので、
/// [`normalize_exercises`] と規則がずれても気づけない。
pub fn set_pins(db: &mut Db, id: ExerciseId, pins: Vec<String>) {
    if let Some(e) = db.exercises.iter_mut().find(|e| e.id == id) {
        e.pins = clean_pins(pins);
    }
}

/// [`normalize_exercises`] と [`set_interval`] が共有する 1 種目ぶんの規則。
///
/// ★ [`clean_pins`] と**同じ理由で共有する**。片方だけに上限を書くと「画面から
/// 打てるのに取り込みで丸められる」「取り込みで残るのに画面で消える」が起きる。
///
/// ★ **0 は落とさない。** 0 秒は「休まず次のセットへ」で、`parse_reps` が 0 を
/// 落とすのとは前提が違う（あちらは `0×0` のゴーストセットが指標を汚すため）。
fn clean_interval(sec: Option<u32>) -> Option<u32> {
    sec.map(|s| s.min(MAX_INTERVAL_SEC))
}

/// 種目のインターバル（秒）を差し替える（記録タブの入力欄から呼ぶ）。
///
/// ★ 正規化を `views` 側に持たせない理由は [`set_pins`] と同じ。
pub fn set_interval(db: &mut Db, id: ExerciseId, sec: Option<u32>) {
    if let Some(e) = db.exercises.iter_mut().find(|e| e.id == id) {
        e.interval_sec = clean_interval(sec);
    }
}

/// `#rrggbb` か。**色の妥当性判定はこの 1 本を通す。**
///
/// ★ `pub` にしてあるのは `views::progress` が CSS 変数へ載せる直前にもう一度
/// 通すため。`--dot:` に空文字が載ると `var(--dot, var(--accent))` の
/// フォールバックが**効かず点が黒くなる**（`var()` は「値が空」を「未定義」と
/// 見ない）ので、`Db` を信じきらずに描画の直前でも確かめる。
///
/// 3 桁（`#abc`）や名前付きの色（`red`）は通さない。`<input type="color">` が
/// 返すのは常に 6 桁の小文字なので、通す形を狭くしても画面から入る値は落ちない。
pub fn is_hex_color(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

/// まだ使われていないラベル色。**新しいラベルの既定色はここから採る。**
/// adr/ux/label-colour-on-the-progress-dots.md
///
/// ★ **位置ベース（`used.len() % 6`）にしない。** ✕ で 1 本消してから ＋ で足すと
/// 残った行と同じ色が生まれ、推移タブで 2 本のラベルが見分けられなくなる。
/// 「未使用の最初の色」なら、6 本までは必ず全部違う色になる。
///
/// ★ 大文字小文字を無視して比べる。手編集の JSON や取り込みで `#E0524A` が
/// 入っていても「使用済み」と数える（同じ色なのに違う扱いにしない）。
///
/// 6 色を使い切ったら重複を許して `n % 6` に落ちる。ただし
/// [`crate::model::MAX_LABELS`] が 6 でパレットも 6 色なので、**現状この枝には
/// 到達しない**（[`clean_labels`] も [`merge_labels`] も 6 本で切る）。上限を
/// 増やしたときに関数が答えを返せなくなるのを避けるための逃げ道として置いてある。
pub fn next_label_color<'a>(used: impl IntoIterator<Item = &'a str>) -> &'static str {
    let used: Vec<String> = used.into_iter().map(|c| c.to_ascii_lowercase()).collect();
    let palette = crate::presets::LABEL_COLOR_CHOICES;
    palette
        .iter()
        .find(|c| !used.iter().any(|u| u == *c))
        .copied()
        .unwrap_or(palette[used.len() % palette.len()])
}

/// [`normalize_exercises`] と [`set_labels`] が共有する 1 種目ぶんの規則。
///
/// ★ **2 経路に分けて書かない**（[`clean_pins`] と同じ理由）。食い違うと「画面で
/// 消えたはずの値が取り込みで生き返る」「取り込みで落ちた値が画面からは入る」が起きる。
///
/// - 名前を **trim しない**（[`normalize_routines`] の規則）が、空白だけの要素は落とす
/// - **`split_whitespace` しない。** ピンが分割するのは TSV の**セル内が空白区切り**
///   だからで、ラベルは 1 セル = 1 名前。「高重量 低レップ」を許したい
/// - [`crate::model::MAX_LABEL_LEN`] で **char 単位**に切り詰め、
///   [`crate::model::MAX_LABELS`] で本数を切る
/// - **重複 ID だけ**採番し直す。`<For key=id>` の重複キーは wasm で panic =
///   アプリが死ぬ。捨てずに採り直すのは名前を黙って失わないため。**渡された ID は
///   それ以外では保持する** — 設定タブの改名で ID が変わらないことの土台で、
///   ここを緩めると 1 打鍵で過去ログが全部宙に浮く
/// - **同名は潰さない**（`merge_db` が正当に生む）。**並べ替えない**（`Vec` 順 = 表示順）
/// - **色が `#rrggbb` でなければ [`next_label_color`] で埋める**（旧版の JSON・TSV 取込・
///   手編集の入口をここ 1 箇所で塞ぐ）。有効な色は同色でも書き換えない
///
/// ★ **色を埋めるのは 2 パス目。** 1 パス目で空名を落として ID を整え、
/// 2 パス目で埋める。1 パスで前方（`out`）の色だけを見ると、
/// `[{color: ""}, {color: "#e0524a"}]` のように**無効な色が先・有効な色が後ろ**という
/// 並びで 1 本目がパレットの先頭を取り、2 本目は有効なので据え置かれて**同色 2 本**が
/// できる。画面からは作れない（`add_label` が常に色を振る）が、手編集の JSON や
/// 色欄が一部欠けたファイルの取り込みでは起きうる。「利用者が選んでいないのに重複する」
/// のは説明できないので、**入力全体の有効な色を先に「使用済み」として集める**。
fn clean_labels(labels: Vec<Label>, ids: &mut IdGen) -> Vec<Label> {
    let mut out: Vec<Label> = Vec::with_capacity(labels.len().min(MAX_LABELS));
    for mut l in labels {
        if l.name.trim().is_empty() {
            continue;
        }
        if out.len() >= MAX_LABELS {
            break;
        }
        // ★ char で数える。バイトで切ると UTF-8 の途中で割れて panic する
        l.name = l.name.chars().take(MAX_LABEL_LEN).collect();
        if out.iter().any(|o| o.id == l.id) {
            l.id = ids.alloc();
        }
        out.push(l);
    }

    // ★ **有効な色は同色でも触らない。** 利用者が 2 本を同じ色にしたなら、それは
    //   選択であって壊れた値ではない。埋めるのは「持っていない」ときだけ。
    //   使用済みの集合は**残す行すべて**（後ろの行も含む）から始め、埋めた色も足していく
    let mut used: Vec<String> = out
        .iter()
        .filter(|l| is_hex_color(&l.color))
        .map(|l| l.color.clone())
        .collect();
    for l in &mut out {
        if !is_hex_color(&l.color) {
            l.color = next_label_color(used.iter().map(String::as_str)).into();
            used.push(l.color.clone());
        }
    }
    out
}

/// 種目のラベルの定義を差し替える（設定タブの種目編集シートから呼ぶ）。
///
/// ★ 正規化を `views` 側に持たせない理由は [`set_pins`] と同じ。
///
/// ★ **`Label.id` は呼び側が渡す。** 既存のラベルは既存の ID を、新規だけ
/// `storage::alloc_id()` を振る。ここで採番し直すと改名の 1 打鍵で
/// [`crate::model::ExerciseLog::label`] が全部宙に浮く。
pub fn set_labels(db: &mut Db, id: ExerciseId, labels: Vec<Label>, ids: &mut IdGen) {
    if let Some(e) = db.exercises.iter_mut().find(|e| e.id == id) {
        e.labels = clean_labels(labels, ids);
    }
}

/// その種目のラベルの中から ID で引く。**参照の解決は必ずこれを通す。**
///
/// ★ 「そのログの種目の `labels` の中」でしか解決しないので、種目をまたいだ宙に
/// 浮いた参照が構造的に起きない（共通プールを作らないという方針が、データの
/// 置き場所で強制される）。
pub fn label_name(db: &Db, ex: ExerciseId, id: LabelId) -> Option<&str> {
    db.exercise(ex)?
        .labels
        .iter()
        .find(|l| l.id == id)
        .map(|l| l.name.as_str())
}

/// 既存の種目に取り込み側のピンを**空のときだけ**入れる（[`merge_db`] から呼ぶ）。
///
/// ★ これが無いと、新品端末へ書き出しを戻したときに**ピンだけが黙って落ちる**。
/// プリセットは固定 ID を持つ（[`crate::presets::preset_exercise_id`]）ので取り込みは
/// 必ず「ID 一致」の枝を通り、その枝は取り込み側の `Exercise` を丸ごと捨てるため。
///
/// ★ 上書きはしない（adr/storage/import-is-merge-only.md の「足すだけ」）。マシンは
/// 端末ではなく**ジムに紐づく**ので、上書きすると引っ越し先で入れ直した値が
/// 古いファイル 1 枚で巻き戻る。体重の「最初の非空を採る」と同じ規則。
///
/// ★ `Conflict` は積まない。`order` / `archived` を黙って落としているのと同じ粒度で、
/// ここで報告を増やすと実質同じデータの取り込みでも確認画面が赤くなる。
fn fill_pins(existing: &mut Exercise, incoming: &Exercise) {
    if existing.pins.is_empty() {
        existing.pins = incoming.pins.clone();
    }
}

/// 既存の種目に取り込み側のインターバルを**未設定のときだけ**入れる
/// （[`merge_db`] から呼ぶ）。
///
/// ★ 落とし穴も規則も [`fill_pins`] とまったく同じ。プリセットは固定 ID を持つので
/// 新品端末への復元は必ず「ID 一致」の枝を通り、そこを直さないと**記録は全部戻るのに
/// インターバルだけ落ちる**。
///
/// ★ 「未設定のときだけ」は `Option::is_none` で見る。`Some(0)` は入っている値なので
/// 上書きしない（0 秒を明示的に選んだ利用者の入力を、古いファイル 1 枚で巻き戻さない）。
fn fill_interval(existing: &mut Exercise, incoming: &Exercise) {
    if existing.interval_sec.is_none() {
        existing.interval_sec = incoming.interval_sec;
    }
}

/// 既存の種目に取り込み側のラベル定義を足す。**足すだけ。既存は上書きしない。**
/// （[`merge_db`] から呼ぶ）
///
/// ★ 落とし穴は [`fill_pins`] とまったく同じで、**被害はこちらのほうが大きい。**
/// プリセットは固定 ID を持つので新品端末への復元は必ず「ID 一致」の枝を通り、その枝は
/// 取り込み側の `Exercise` を丸ごと捨てる。手当てしないと**記録は全部戻るのにラベル
/// 定義だけ落ち、ログには宙に浮いた `label` が残る**（履歴が全部「どのラベルにも
/// 属さない」になる最悪の形）。
///
/// 判定は**種目とまったく同じ梯子**: ID 一致（何もしない）→ 同名がちょうど 1 件
/// （`alias` に積む。これが「2 台で独立に定義した `P` を寄せたい」という名前側の
/// 利点の回収点。「ちょうど 1 件」なのは [`resolve_exercise`] と同じ理由 — 曖昧なら
/// 新規に倒す）→ 新規追加（[`crate::model::MAX_LABELS`] まで）。
///
/// ★ **写像のキーは `(写像後の ExerciseId, LabelId)`。** `LabelId` 単独にすると、
/// 同じ `LabelId` が取り込み側の 2 種目に居る場合（手編集 JSON、または
/// [`clean_labels`] が種目内の重複しか再採番しないので種目をまたぐ重複は生き残る）に
/// 後の `insert` が前を上書きし、**種目 A のログが種目 B のラベルへ張り替わって
/// 宙に浮く**。キーを組にすればこの経路が構造的に消える。
///
/// ★ `Conflict` は積まない（[`fill_pins`] と同じ粒度）。
///
/// ★ **色は新規追加の枝でだけ触る。** 同名寄せの枝は `alias` を張って取り込み側の
/// 定義を捨てるので、こちらの色が勝つ（自分が選んだ色が古いファイル 1 枚で
/// 巻き戻らない）。新規の枝は「こちらに既に居る色と衝突するなら塗り替える」。
///
/// 返すのは `(追加した数, MAX_LABELS で落とした数)`。落とした数を数えるのは、落ちると
/// そのログは取り込み直しても二度と生き返らない dangling になるのに、数も `Conflict` も
/// 出ないと気づけないため。
fn merge_labels(
    existing: &mut Exercise,
    incoming: &Exercise,
    alias: &mut HashMap<(ExerciseId, LabelId), LabelId>,
) -> (usize, usize) {
    let ex_id = existing.id;
    let mut added = 0;
    let mut dropped = 0;
    for l in &incoming.labels {
        // 1. ID 一致。何もしない（写像も要らない）
        if existing.labels.iter().any(|x| x.id == l.id) {
            continue;
        }
        // 2. 同名がちょうど 1 件なら寄せる。曖昧（0 件 / 2 件以上）なら新規に倒す
        if let Some(only) = exactly_one(existing.labels.iter().filter(|x| x.name == l.name)) {
            alias.insert((ex_id, l.id), only.id);
            continue;
        }
        // 3. 新規追加
        if existing.labels.len() >= MAX_LABELS {
            dropped += 1;
            continue;
        }
        // ★ **こちらに同じ色が既に居るなら塗り替える。** `merge_db` は `normalize` を
        //   呼ばないので [`clean_labels`] の門を通らず、2 台で独立に定義した
        //   1 本目どうし（どちらもパレットの先頭色）がそのまま並ぶと、推移タブで
        //   区別できない 2 本になる。不正な色も同じ枝で埋める。
        //   ★ 衝突していない有効な色は**そのまま採る** — あちらの端末で選んだ色を
        //     取り込みで黙って捨てない（同名寄せの枝で自分の色が勝つのと対称）
        let mut l = l.clone();
        let clash = existing
            .labels
            .iter()
            .any(|x| x.color.eq_ignore_ascii_case(&l.color));
        if !is_hex_color(&l.color) || clash {
            l.color = next_label_color(existing.labels.iter().map(|x| x.color.as_str())).into();
        }
        existing.labels.push(l);
        added += 1;
    }
    (added, dropped)
}

/// `f32` で表せない重量を捨てる。**取り込み境界で必ず通すこと。**
///
/// ★ ここが無いと、1 回の取り込みで**次回起動から永久に読めなくなる**:
///
/// 1. `3.5e38` は f64 では有限なので serde は受理し、f32 へ落として `inf` にする
/// 2. `serde_json::to_string` は `inf` / `NaN` を**エラーにせず `"weight":null`** と書く
///    （`save()` は成功するので容量超過の検知にも引っかからない）
/// 3. 次の起動で `null` は f32 にできず `Broken` → 退避 → 旧世代へ降格。退避した
///    データも同じ理由で読み戻せない
///
/// 負の重量も落とす。UI の `parse_weight` が弾くので入力からは入らないが、取り込みには
/// 入りうる。入ると [`log_rank`] の「重量は非負」という前提が崩れ、`to_bits` の順序が
/// 反転してマージが悪いほうを勝たせる。
/// ★ 捨てるセットのメモも一緒に消える。メモはセットの付属物なので正しい（「3 セット目が
/// キツかった」は 3 セット目が無ければ指すものが無い）が、無言の欠落なので明記しておく。
/// ここが発火するのは `3.5e38` のような壊れた取り込みだけで、UI からは入らない。
fn drop_unrepresentable_weights(s: &mut Session) {
    for log in &mut s.logs {
        // ★ 段の重量は [`clean_drops`] が同じ門を通す（[`prune_empty_drops`] 経由）。
        //   ここで二重に書くと「段が在る」の規則が 2 本に割れる
        log.sets
            .retain(|set| set.weight.is_finite() && set.weight >= 0.0);
    }
    if !s.body_weight.is_some_and(|w| w.is_finite() && w > 0.0) {
        s.body_weight = None;
    }
}

/// schema 2 までの形。**ここでしか使わない**ので private に閉じる。
mod legacy {
    use std::collections::BTreeMap;

    use serde::Deserialize;

    // ★ `SetEntry` は現行のものを再利用する。schema ≤2 の JSON にメモは存在しないが、
    //   `note` が `#[serde(default)]` なので欠けていても読める。これが成り立つのは
    //   **`SetEntry` に足すフィールドが常に default を持つ**あいだだけ。default の無い
    //   フィールドを足すと v1 / v2 の読み込みが `missing field` で落ちる
    //   （`migrates_from_schema_one` / `..._two` のテストが落ちるので気づける）。
    use crate::model::SetEntry;

    #[derive(Deserialize)]
    pub struct Db {
        pub groups: Vec<Group>,
        pub exercises: Vec<Exercise>,
        pub sessions: BTreeMap<String, Session>,
    }

    #[derive(Deserialize)]
    pub struct Group {
        pub id: u32,
        pub name: String,
        pub color: String,
        pub order: u32,
    }

    #[derive(Deserialize)]
    pub struct Exercise {
        pub id: u32,
        pub name: String,
        pub group_id: u32,
        pub order: u32,
        #[serde(default)]
        pub archived: bool,
    }

    #[derive(Deserialize)]
    pub struct ExerciseLog {
        pub exercise_id: u32,
        pub sets: Vec<SetEntry>,
        #[serde(default)]
        pub at: Option<i64>,
    }

    #[derive(Deserialize, Default)]
    pub struct Session {
        pub logs: Vec<ExerciseLog>,
        #[serde(default)]
        pub body_weight: Option<f32>,
        #[serde(default)]
        pub note: String,
    }
}

/// 連番 u32 → 乱数 [`crate::model::Id`]。**全参照を 1 つの写像で張り替える。**
fn upgrade_from_sequential(old: legacy::Db, ids: &mut IdGen) -> Db {
    // ① 写像を**先に作り切る**。参照だけに現れる ID（参照先が既に消えた
    //    group_id / exercise_id）も含める。ここで漏らすと後から別々の ID が
    //    振られて参照が壊れる。
    //
    //    ★ 型ごとに 2 つに分ける。健全なデータなら同じ u32 が部位と種目の両方に
    //      現れることはないが、`migrate` の入力に健全性の前提は置かない。
    let mut groups: HashMap<u32, GroupId> = HashMap::new();
    let mut exercises: HashMap<u32, ExerciseId> = HashMap::new();

    for g in &old.groups {
        groups.entry(g.id).or_insert_with(|| ids.alloc());
    }
    for e in &old.exercises {
        exercises.entry(e.id).or_insert_with(|| ids.alloc());
        groups.entry(e.group_id).or_insert_with(|| ids.alloc());
    }
    for s in old.sessions.values() {
        for l in &s.logs {
            exercises
                .entry(l.exercise_id)
                .or_insert_with(|| ids.alloc());
        }
    }

    // ② プリセット名に一致するものを固定 ID へ寄せる
    pin_presets(&old, &mut groups, &mut exercises);

    // ③ 以降は引くだけ。**`or_insert` は使わない**（引けたりする度に新しい ID を
    //    作ると、同じ旧 ID が別々の新 ID になって参照が割れる）。
    //
    //    ★ ただし添字アクセスで panic させてはいけない。`migrate` は `load()` から
    //      呼ばれるので、wasm では panic = abort = 白画面。次回起動も同じデータで
    //      同じ panic を踏み、`Err` 経路を通らないので**退避もフォールバックも
    //      発動しない**（起動不能ループ）。デバッグでは落として気づき、本番では
    //      番兵 ID に落として起動を守る。番兵はどの実体も指さないので合流はしない
    let to_group = |id: u32| -> GroupId {
        debug_assert!(groups.contains_key(&id), "① の列挙漏れ: group {id}");
        groups.get(&id).copied().unwrap_or_default()
    };
    let to_exercise = |id: u32| -> ExerciseId {
        debug_assert!(exercises.contains_key(&id), "① の列挙漏れ: exercise {id}");
        exercises.get(&id).copied().unwrap_or_default()
    };

    Db {
        schema: SCHEMA,
        groups: old
            .groups
            .into_iter()
            .map(|g| Group {
                id: to_group(g.id),
                name: g.name,
                color: g.color,
                order: g.order,
            })
            .collect(),
        exercises: old
            .exercises
            .into_iter()
            .map(|e| Exercise {
                id: to_exercise(e.id),
                name: e.name,
                // 宙に浮いた参照は宙に浮いたまま残す。偶然有効にするほうが危ない
                group_id: to_group(e.group_id),
                order: e.order,
                archived: e.archived,
                pins: Vec::new(),
                interval_sec: None,
                // schema ≤2 にラベルは無い
                labels: Vec::new(),
            })
            .collect(),
        // schema ≤2 にトレーニングメニューは存在しない（`legacy::Db` にフィールドが無い）
        routines: Vec::new(),
        sessions: old
            .sessions
            .into_iter()
            .map(|(key, s)| {
                (
                    key,
                    Session {
                        logs: s
                            .logs
                            .into_iter()
                            .map(|l| ExerciseLog {
                                exercise_id: to_exercise(l.exercise_id),
                                sets: l.sets,
                                at: l.at,
                                // schema ≤2 にメモもラベルも無い
                                note: String::new(),
                                label: None,
                            })
                            .collect(),
                        body_weight: s.body_weight,
                        note: s.note,
                    },
                )
            })
            .collect(),
    }
}

/// 名前がプリセットと一致する部位 / 種目を、全端末で共通の固定 ID に寄せる。
///
/// ★ **一致が「ちょうど 1 件」のときだけ寄せる。** 改名（`menu.rs` の
/// `rename_exercise` / `rename_group`）には重複チェックが無いので、「ダンベルプレス」を
/// 「ベンチプレス」に改名した DB では同名 2 種目が存在しうる。両方を同じ固定 ID に
/// 寄せると**別々の種目の履歴が無警告で 1 本に合流する** — この移行が潰そうとして
/// いるバグと同じ壊れ方になる。
fn pin_presets(
    old: &legacy::Db,
    groups: &mut HashMap<u32, GroupId>,
    exercises: &mut HashMap<u32, ExerciseId>,
) {
    for preset in crate::presets::PRESETS {
        let matched: Vec<u32> = old
            .groups
            .iter()
            // ★ 日英どちらの綴りでも寄せる。v1/v2 は日本語しか持たないが、規則を
            //   1 本にしておく（`presets::Names::matches` の doc を参照）
            .filter(|g| preset.name.matches(&g.name))
            .map(|g| g.id)
            .collect();
        if let [only] = matched[..] {
            groups.insert(only, preset.id);
        }

        for (preset_id, preset_name) in preset.exercises {
            let matched: Vec<u32> = old
                .exercises
                .iter()
                .filter(|e| preset_name.matches(&e.name))
                .map(|e| e.id)
                .collect();
            if let [only] = matched[..] {
                exercises.insert(only, *preset_id);
            }
        }
    }
}

/// 正規化で同じ日付キーに落ちた 2 つのセッションを 1 つにまとめる。
///
/// ★ **これはインポートのマージに流用してはいけない。** ログを無条件に連結するので、
/// 同じファイルを 2 回取り込むとセットが 2 倍になる（冪等でない）。メモの連結は
/// [`append_note`] の重複ガードで冪等になったが、**ログの連結は冪等でないまま**。
/// マージ側は `merge_db` が別に処理する。
fn merge_same_day(dst: &mut Session, src: Session) {
    dst.logs.extend(src.logs);
    if dst.body_weight.is_none() {
        dst.body_weight = src.body_weight;
    }
    append_note(&mut dst.note, &src.note);
}

/// 「1 日 1 種目 1 ログ」への正規化。初出の順序は保つ。
///
/// ★ **「初出の順序は保つ」は仕様である。** `logs` の並びは利用者がドラッグで決めた
/// その日の種目順そのもので（adr/ux/drag-to-reorder-in-record-tab.md）、ここは
/// 読み込みのたびに通る。並べ替える実装に変えると、**次回起動でユーザーの並びが
/// 黙って戻る**。
fn dedupe_logs(s: &mut Session) {
    let mut order: Vec<ExerciseId> = Vec::new();
    let mut merged: HashMap<ExerciseId, ExerciseLog> = HashMap::new();
    for log in std::mem::take(&mut s.logs) {
        match merged.get_mut(&log.exercise_id) {
            Some(existing) => {
                existing.sets.extend(log.sets);
                existing.at = match (existing.at, log.at) {
                    (Some(a), Some(b)) => Some(a.max(b)),
                    (Some(a), None) => Some(a),
                    (None, b) => b,
                };
                append_note(&mut existing.note, &log.note);
                // ★ ここが無いと後発の `label` が黙って落ちる。「空のときだけ埋める」
                //   なのは merge_db のセッション枝と同じ規則（先に来たものを優先）
                existing.label = existing.label.or(log.label);
            }
            None => {
                order.push(log.exercise_id);
                merged.insert(log.exercise_id, log);
            }
        }
    }
    s.logs = order
        .into_iter()
        .filter_map(|id| merged.remove(&id))
        // ★ **`!l.sets.is_empty()` にしてはいけない。** 種目メモだけのログ（「肩が痛いので
        //   今日はやめた」）はここを毎回の読み込みで通るので、セットで判定すると
        //   画面には出ているのに次回起動で消える — 保存と表示が食い違う最悪の形になる。
        //   「メモがある」と「トレした」は別で、後者は `Session::is_trained` が
        //   セットだけを見て判定し続ける（adr/data-model/notes-on-logs-and-sets.md）
        .filter(|l| !l.is_empty())
        .collect();
}

// ── 書き出し / 読み込み ─────────────────────────────────────────────────────

/// 保存形式の文字列。`storage::save` が `localStorage` に書くのと同じ compact JSON。
///
/// ★ **これはもう書き出し形式ではない**（adr/storage/tsv-export-for-spreadsheets.md）。
/// 端末の外に出すのは [`export_tsv`] の TSV で、こちらは `localStorage` 用に残る。
/// 分けた理由は「スプレッドシートで開けること」で、JSON はどの表計算でも開けない。
/// 代わりに **TSV は `Db` の全部を持てない**（ID・色・並び順・archived・`at`）ので、
/// 取り込み側が名前から組み立て直す。だから [`parse_import`] は JSON も受け続ける。
pub fn export_json(db: &Db) -> String {
    serde_json::to_string(db).unwrap_or_else(|_| "{}".to_string())
}

/// 書き出しのファイル名。**時刻まで入れる。**
///
/// 日付だけだと、同じ日に 2 回書き出したとき 2 回目が 1 回目を上書きする。
/// 「バックアップを取ったつもりが前のバックアップを潰した」はこの機能の存在意義を消す。
pub fn export_filename(now: chrono::NaiveDateTime) -> String {
    format!("fitness-memo-{}.tsv", now.format("%Y%m%d-%H%M"))
}

// ── TSV ─────────────────────────────────────────────────────────────────────
//
// 書き出しは「スプレッドシートで読む」ため。取り込みは**このアプリが書いた TSV を
// そのまま戻すこと**だけを一級で保証し、シート側で編集して戻す経路は best effort。
//
// ★ **版番号を持てない。** TSV には `schema` に当たる欄が無く、`Unsupported` を返す
// 手段が原理的に無い。代わりに進化規則を固定する:
//
//   1. **列は足すだけ。** 既存の列名と意味は永久に変えない
//   2. **取り込みは列を名前で引き、知らない列は無視する**（位置で読まない）
//   3. 足した列は「無くても読める」ものだけにする
//
// これを破ると、古いアプリが新しいファイルを黙って誤読する（新しすぎると言えない）。

/// 共有シート / ダウンロードに渡す MIME。**拡張子 `.tsv` と組で扱う。**
///
/// ★ iOS の UTType は `File.type` ではなく**ファイル名の拡張子**から決まるので、
/// 名前だけ、MIME だけを変えても意味が無い（adr/storage/share-sheet-over-download.md）。
pub const TSV_MIME: &str = "text/tab-separated-values";

/// 見出し行（日本語）。**この並びと綴りが外部仕様**なので、テストがバイト一致で
/// 固定している。**1 文字も変えてはいけない** — 過去に書き出したファイルが読めなくなる。
const TSV_HEADER_JA: [&str; 16] = [
    "日付",
    "部位",
    "種目",
    "セット",
    "重量kg",
    "回数",
    // ★ 同じく後から足した列。**セット単位**の列なので回数の隣に置く（種目単位の
    //   「ピン」「インターバル秒」と混ざると、シートで縦に読んだときに意味の階層が崩れる）。
    //   セルは `50×5 40×4` のように段を半角空白で並べる（「ピン」列と同じ流儀）。
    //   位置が変わっても取り込みは `tsv_header` が名前で引くので壊れない
    "ドロップ",
    "体重kg",
    "セットメモ",
    "種目メモ",
    "体調メモ",
    "時刻",
    // ★ 後から**足した**列（進化規則 1）。既存 11 列の意味は変えていないので、
    //   この列を知らない古いアプリでも記録は今までどおり読める
    "メニュー",
    // ★ 同じく後から足した列。ピンは種目に 1 つなので**その種目が最初に現れた行**に
    //   だけ書く（体重・種目メモと同じ「使ったら空にする」）。セル内は半角空白区切りで、
    //   1 要素が空白を含まないことは `normalize_exercises` が保証する
    "ピン",
    // ★ 同じく後から足した列。ピンと同じ「その種目が最初に現れた行にだけ書く」規則で、
    //   セルは裸の数字（単位は見出しに入れる — `重量kg` / `体重kg` と同じ流儀）
    "インターバル秒",
    // ★ 同じく後から足した列。**位置は末尾**（`e2e/backup.spec.mjs` が
    //   `split('\t')[2] === 'ベンチプレス'` と位置参照しているので既存 15 列の
    //   インデックスを動かさない）。
    //
    // ★ セルは ID ではなく**名前**（TSV の存在意義がスプレッドシートで読めること。
    //   既に全 ID を落として名前から作り直している）。
    //
    // ★ ピン / インターバルと違い**種目粒度ではなく日ごと**なので、
    //   `ex_meta_written` に相乗りさせない（させると 2 日目以降が全部落ちる）。
    //   書くのは「そのログの最初の行」だけ（`種目メモ` と同じ「使ったら空にする」）
    "ラベル",
];

/// 見出し行（英語）。位置と意味は [`TSV_HEADER_JA`] と 1:1。
///
/// ★ TSV は版番号を持てないので、**この綴りも足した時点で永久の外部仕様**になる
/// （adr/storage/tsv-header-follows-the-ui-language.md）。日本語版と同じ強度で
/// バイト一致テストが固定している。
const TSV_HEADER_EN: [&str; 16] = [
    "Date",
    "Muscle group",
    "Exercise",
    "Set",
    "Weight kg",
    "Reps",
    // ★ `Drop` 単独にしない。`is_known_header_cell` が `NotDb`（うちのファイルでは
    //   ない）と `NoHeader`（うちのファイルだが壊れている）を分けるのにこの配列を使う
    //   ので、ありふれた列名を入れると無関係な TSV が「壊れたうちのファイル」に化ける
    "Drop set",
    "Body weight kg",
    "Set note",
    "Exercise note",
    "Day note",
    "Time",
    "Routine",
    "Pins",
    "Interval sec",
    "Label",
];

/// 書き出しに使う見出し。**UI の言語に従う。**
///
/// ★ 書き出しの存在意義は「スプレッドシートで開いて読めること」
/// （adr/storage/tsv-export-for-spreadsheets.md）で、読めない言語の列名はその意義を
/// 失わせる。取り込み側は [`is_known_header_cell`] のとおり日英どちらも受けるので、
/// 言語を切り替えても過去のファイルは読める。
pub fn tsv_header_row(lang: Lang) -> [&'static str; 16] {
    match lang {
        Lang::Ja => TSV_HEADER_JA,
        Lang::En => TSV_HEADER_EN,
    }
}

/// 知っている見出しセルか。`NotDb`（うちのファイルではない）と `NoHeader`
/// （うちのファイルだが壊れている）の切り分けに使う。
fn is_known_header_cell(cell: &str) -> bool {
    TSV_HEADER_JA.contains(&cell) || TSV_HEADER_EN.contains(&cell)
}

/// 1 セルに落とす。**タブと改行を半角空白に潰す。**
///
/// ★ メモは `<input type="text">` から入るが、[`append_note`] が合流時に `\n` を挟むので
/// **改行入りのメモは実在する**。潰さないと行が崩壊して、その日の記録が丸ごと壊れる。
/// 潰したことで元の文字列と変わるが、[`append_note`] の重複判定が空白を畳んで比べる
/// ので、読み戻してもメモは伸びない。
fn flatten_cell(s: &str) -> String {
    if s.contains(['\t', '\n', '\r']) {
        fold_ws(s)
    } else {
        s.to_string()
    }
}

/// 1 行書く。★ 引数 11 個の関数を作らないための入れ物（`clippy::too_many_arguments`）。
fn push_row(out: &mut String, cells: [&str; 16]) {
    for (i, cell) in cells.iter().enumerate() {
        if i > 0 {
            out.push('\t');
        }
        out.push_str(&flatten_cell(cell));
    }
    out.push('\n');
}

/// `at`（epoch ms）を、その行の日付と**同じ暦日のときだけ** "HH:MM" にする。
///
/// ★ 食い違うときに書かないのは、壊れたデータ由来の `at`（`elapsed_since_last` が
/// 既に想定している状態）を前日の行に時刻として並べないため。「記録は起きたとおりに
/// 持つ」という `ExerciseLog::at` の契約を、表示にも通す。
fn tsv_time(at: Option<i64>, date: NaiveDate, tz: chrono::FixedOffset) -> String {
    let Some(ms) = at else {
        return String::new();
    };
    let Some(dt) = chrono::DateTime::from_timestamp_millis(ms) else {
        return String::new();
    };
    let local = dt.with_timezone(&tz).naive_local();
    if local.date() == date {
        local.format("%H:%M").to_string()
    } else {
        String::new()
    }
}

/// 書き出す TSV。**1 セット 1 行**の 1 枚の表。
///
/// `tz` は「時刻列をどのオフセットで出すか」。★ ここで `chrono::Local` を呼ばないのは、
/// `core` を実行環境非依存に保つため（呼ぶと `cargo test` が動かすマシンの TZ で
/// 結果が変わる）。このモジュールの他の関数が `now_ms` / `today` を引数で受けるのと同じ。
///
/// 表の作り:
///
/// - **体重・種目メモ・体調メモは、そのまとまりの先頭 1 行だけに書く。** 毎行書くと
///   シート側の `AVERAGE` が「セット数で重み付けした平均」になり、体重グラフが嘘をつく
/// - **重量 0 は空セル。** `set_volume` も `fmt_set` も 0 を「重量なし」として扱うので、
///   空セルが平均とグラフから外れるシートの挙動と一致する
/// - **末尾に「種目マスタ行」**（日付が空の行）。一度も記録していない自作種目と、
///   種目を持たない部位を黙って失わないため。手つかずのプリセットは書かない
///   （新規インストールに必ず同じ固定 ID で居るので、書かなくても失われない）
pub fn export_tsv(db: &Db, tz: chrono::FixedOffset, lang: Lang) -> String {
    let mut out = String::new();
    push_row(&mut out, tsv_header_row(lang));

    // 記録とメニューに出てきた種目。末尾のマスタ行から除くのに使う
    // （どちらの行も部位と種目名を書くので、マスタ行を重ねる必要が無い）
    let mut logged: HashSet<ExerciseId> = db
        .routines
        .iter()
        .flat_map(|r| r.exercises.iter().copied())
        .collect();
    // 種目に貼り付く設定（ピン・インターバル）を書き終えた種目。★ その種目が
    //   **ファイル中で最初に現れた行**にだけ書く（体重・種目メモの「使ったら空にする」と
    //   同じ手）。毎行書くとシートで同じ文字列が縦に伸びて読みづらく、行数ぶん容量も増える
    //
    // ★ ピンとインターバルで集合を分けない。条件が「その種目の初出行」で完全に同じなので、
    //   2 本持つと片方だけ `insert` を忘れる余地が生まれる（列が増えるたびに増える）
    let mut ex_meta_written: HashSet<ExerciseId> = HashSet::new();

    for (key, session) in &db.sessions {
        let Some(date) = parse_date_key(key) else {
            continue;
        };
        let weight = session.body_weight.map(fmt_weight).unwrap_or_default();
        // ★ 日の先頭行にだけ書く。使ったら空にする
        let mut day_weight = weight.as_str();
        let mut day_note = session.note.as_str();

        let mut wrote_any = false;
        for log in &session.logs {
            // ★ 種目が引けないログは書かない。ID をそのまま種目名として出すと、
            //   取り込み側に "00000000abc" という名前の種目が生える
            let Some(ex) = db.exercise(log.exercise_id) else {
                continue;
            };
            logged.insert(ex.id);
            let group = db
                .group(ex.group_id)
                .map(|g| crate::presets::group_name(g.id, &g.name, lang))
                .unwrap_or_default();
            // ★ 書き出すのは**表示名**（未改名のプリセットは UI の言語に追従する）。
            //   保存名をそのまま出すと、英語で使っている人のシートに日本語の種目名が
            //   並ぶ。取り込み側は `preset_exercise_id` が日英どちらの綴りでも
            //   固定 ID に寄せるので、往復は言語を跨いでも閉じる
            let ex_name = crate::presets::exercise_name(ex.id, &ex.name, lang);
            let time = tsv_time(log.at, date, tz);
            let mut log_note = log.note.as_str();
            // ★ **`ex_meta_written` に相乗りさせない。** あれは種目粒度で、ラベルは
            //   日ごとに変わる。相乗りさせると 2 日目以降が全部落ちる。
            //   規則は `log_note` と同じ「そのログの最初の行に書いて、使ったら空にする」
            let ex_label = log
                .label
                .and_then(|id| label_name(db, ex.id, id))
                .unwrap_or_default();
            let mut label_cell = ex_label;
            // ★ 1 回の `insert` でピンとインターバルの両方を決める（`ex_meta_written` の注記）
            let first_row_of_ex = ex_meta_written.insert(ex.id);
            let ex_pins = if first_row_of_ex {
                ex.pins.join(" ")
            } else {
                String::new()
            };
            let ex_interval = match (first_row_of_ex, ex.interval_sec) {
                (true, Some(s)) => s.to_string(),
                _ => String::new(),
            };
            let mut pins_cell = ex_pins.as_str();
            let mut interval_cell = ex_interval.as_str();

            if log.sets.is_empty() {
                // セットが 1 本も無いログ（「肩が痛いのでやめた」）。メモだけの行として残す。
                // ★ ここを落とすと `ExerciseLog::is_empty` が残すと決めた記録が消える
                push_row(
                    &mut out,
                    [
                        key,
                        group,
                        ex_name,
                        "",
                        "",
                        "",
                        "",
                        day_weight,
                        "",
                        log_note,
                        day_note,
                        &time,
                        "",
                        pins_cell,
                        interval_cell,
                        label_cell,
                    ],
                );
                day_weight = "";
                day_note = "";
                wrote_any = true;
                continue;
            }

            for (i, set) in log.sets.iter().enumerate() {
                let no = (i + 1).to_string();
                let w = if set.weight > 0.0 {
                    fmt_weight(set.weight)
                } else {
                    String::new()
                };
                let reps = set.reps.to_string();
                let drops = drops_cell(set);
                push_row(
                    &mut out,
                    [
                        key,
                        group,
                        ex_name,
                        &no,
                        &w,
                        &reps,
                        // ★ 段はセット単位なので、書くのはこの行だけ
                        &drops,
                        day_weight,
                        &set.note,
                        log_note,
                        day_note,
                        &time,
                        "",
                        pins_cell,
                        interval_cell,
                        label_cell,
                    ],
                );
                day_weight = "";
                day_note = "";
                log_note = "";
                pins_cell = "";
                interval_cell = "";
                label_cell = "";
                wrote_any = true;
            }
        }

        // 体重か体調メモだけの日。ログ行が 1 本も出ていないなら、日付だけの行を残す
        if !wrote_any && !(day_weight.is_empty() && day_note.trim().is_empty()) {
            push_row(
                &mut out,
                [
                    key, "", "", "", "", "", "", day_weight, "", "", day_note, "", "", "", "", "",
                ],
            );
        }
    }

    // ── 種目マスタ行（日付が空）──
    for ex in &db.exercises {
        if logged.contains(&ex.id) {
            continue;
        }
        // 手つかずのプリセット（名前も ID もそのまま）は書かない。新規インストールに
        // 必ず同じ固定 ID で居るので、書かなくても失われない。
        // ★ ただしピンやインターバルを入れた種目は「手つかず」ではない。ここで弾くと、
        //   記録もメニューも無いプリセットに付けた設定だけが黙って消える
        if ex.pins.is_empty()
            && ex.interval_sec.is_none()
            && crate::presets::preset_exercise_id(&ex.name) == Some(ex.id)
        {
            continue;
        }
        let pins = ex.pins.join(" ");
        let interval = ex.interval_sec.map(|s| s.to_string()).unwrap_or_default();
        let group = db
            .group(ex.group_id)
            .map(|g| crate::presets::group_name(g.id, &g.name, lang))
            .unwrap_or_default();
        push_row(
            &mut out,
            [
                "",
                group,
                crate::presets::exercise_name(ex.id, &ex.name, lang),
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                &pins,
                &interval,
                // ★ **ラベル定義はマスタ行に載らない**（`labels.is_empty()` を上の
                //   プリセット除外判定に足さないのと同じ理由）。ピン / インターバルは
                //   マスタ行自身のセルが運ぶが、ラベルは**日ごと**の列なので
                //   マスタ行が運べない。詳細は
                //   adr/data-model/labels-on-the-exercise-and-a-mark-on-the-log.md
                "",
            ],
        );
    }
    // 種目を 1 つも持たない部位。画面から作れるので、書かないと静かに消える
    for g in &db.groups {
        if !db.exercise_ids_of_group(g.id).is_empty() {
            continue;
        }
        if crate::presets::preset_group_id(&g.name) == Some(g.id) {
            continue;
        }
        push_row(
            &mut out,
            [
                "",
                crate::presets::group_name(g.id, &g.name, lang),
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
            ],
        );
    }

    // ── トレーニングメニュー行（日付が空・メニュー列が埋まる）──
    //
    // ★ 書かないと**機種変更でメニューだけ消える**。記録は行に出るので残るが、
    //   メニューは `Db` にしか無い（adr/data-model/routines-as-named-exercise-lists.md）。
    //
    // ★ **並び順は `セット` 列に書く**（1 始まり）。行の順序だけに頼ってはいけない —
    //   **メニュー名は一意ではない**（画面から同名のものを作れるし、`merge_db` は
    //   名前だけでは寄せない方針なので実際に並ぶ）。名前でひとまとまりにすると、
    //   「胸の日」が 2 本あるファイルを読み戻したときに**中身が融合した別物**になる。
    //   番号が 1 に戻るところが区切りになるので、同名が続いても分けられる。
    //   記録行の `セット` と意味が並行（どちらも「何番目か」）なので列は増やさない
    for r in &db.routines {
        if r.exercises.is_empty() {
            // 名前だけのメニュー（種目を選ぶ前に閉じた状態）。`migrate` が残すと決めた形
            push_row(
                &mut out,
                [
                    "", "", "", "", "", "", "", "", "", "", "", "", &r.name, "", "", "",
                ],
            );
            continue;
        }
        let mut no = 0usize;
        for id in &r.exercises {
            // 引けない種目は書かない（種目マスタ行と同じ規則。ID が名前として出るのを防ぐ）
            let Some(ex) = db.exercise(*id) else {
                continue;
            };
            no += 1;
            let pos = no.to_string();
            let first_row_of_ex = ex_meta_written.insert(ex.id);
            let pins = if first_row_of_ex {
                ex.pins.join(" ")
            } else {
                String::new()
            };
            let interval = match (first_row_of_ex, ex.interval_sec) {
                (true, Some(s)) => s.to_string(),
                _ => String::new(),
            };
            let group = db
                .group(ex.group_id)
                .map(|g| crate::presets::group_name(g.id, &g.name, lang))
                .unwrap_or_default();
            push_row(
                &mut out,
                [
                    "",
                    group,
                    crate::presets::exercise_name(ex.id, &ex.name, lang),
                    &pos,
                    "",
                    "",
                    "",
                    "",
                    "",
                    "",
                    "",
                    "",
                    &r.name,
                    &pins,
                    &interval,
                    "",
                ],
            );
        }
    }

    out
}

/// 読み込みの失敗。**利用者の次の行動が変わる粒度でだけ分ける。**
#[derive(Debug, PartialEq, Eq)]
pub enum ImportError {
    /// 空。空ファイル
    Empty,
    /// JSON として壊れている。途中で切れた可能性が高い。
    ///
    /// ★ **JSON の枝でしか返らない。** TSV は途中で切れても「そこまでの行」が読めるので、
    /// 「切れている」と言える瞬間が無い。名前を形式非依存にしたくなるが、変えると
    /// 文言まで曖昧になるのでこのまま。
    NotJson,
    /// このアプリの記録ではない
    NotDb,
    /// TSV の見出し行が読めない（列名は知っているものが混じっている）
    NoHeader,
    /// 見出しはあるが、取り込める行が 1 つも無い
    NoRecords,
    /// 日付や回数が読めず、記録が 1 日も残らなかった。
    ///
    /// ★ `NoRecords` と分けるのは、**利用者の次の行動が違う**から。あちらは「中身が
    /// 入っていない」で、こちらは「入っているが書式が変わってしまっている」。後者は
    /// 表計算アプリが日付や数値を書き戻したときに起きるので、言うことが違う。
    Unreadable,
    /// 新しい版で作られている（JSON のみ。TSV は版番号を持たない）
    Unsupported(u32),
}

impl ImportError {
    pub fn message(&self, lang: Lang) -> String {
        let c = &lang.strings().core;
        match self {
            Self::Empty => c.err_empty.to_string(),
            Self::NotJson => c.err_not_json.to_string(),
            Self::NotDb => c.err_not_db.to_string(),
            Self::NoHeader => c.err_no_header.to_string(),
            Self::NoRecords => c.err_no_records.to_string(),
            Self::Unreadable => c.err_unreadable.to_string(),
            Self::Unsupported(v) => lang.err_unsupported(*v),
        }
    }
}

/// ファイルの中身 → `Db`。**TSV と JSON の両方を受ける。**
///
/// `mine` は取り込み**先**の `Db`。TSV は ID を持たないので、種目と部位を名前から
/// 引き当てるのに要る（[`parse_tsv`] の解決の梯子を参照）。JSON の枝は見ない。
///
/// 順序が大事:
/// 1. 空なら `Empty`
/// 2. **JSON に見えるなら JSON として読む。** 素の [`migrate`] が通ればそのまま返し
///    （正しい入力には絶対に手を触れない）、駄目なら [`repair`] してもう一度だけ試す
/// 3. そうでなければ TSV として読む
///
/// ★ **TSV を [`repair`] に通してはいけない。** あれは全角引用符を一括置換するので、
/// メモを黙って書き換える。「素のパースが失敗したときしか呼ばれないから安全」という
/// `repair` の前提は、区切り文字だけで構造が決まる TSV では成立しない。
pub fn parse_import(raw: &str, ids: &mut IdGen, mine: &Db) -> Result<Db, ImportError> {
    if raw.trim().is_empty() {
        return Err(ImportError::Empty);
    }
    if !looks_like_json(raw) {
        return parse_tsv(raw, ids, mine);
    }
    match migrate(raw, ids) {
        Ok(db) => return Ok(db),
        Err(RestoreError::Unsupported(v)) => return Err(ImportError::Unsupported(v)),
        Err(RestoreError::Broken(_)) => {}
    }
    match migrate(&repair(raw), ids) {
        Ok(db) => Ok(db),
        Err(RestoreError::Unsupported(v)) => Err(ImportError::Unsupported(v)),
        Err(RestoreError::Broken(_)) => {
            if serde_json::from_str::<serde_json::Value>(raw).is_ok() {
                Err(ImportError::NotDb)
            } else {
                Err(ImportError::NotJson)
            }
        }
    }
}

/// JSON の枝へ送るか。**中身では判断しない**（TSV のセルに `{` が入っていても困らない）。
///
/// ★ `[` を含めないのは、`[1,2,3]` を「JSON だがうちのデータではない」ではなく TSV 側の
/// `NotDb` に落としたいから — どちらでも同じ文言に着地するので、分岐を 1 つ減らす。
/// 旧版が書いた `.json` は必ずオブジェクトなので、`{` とコードフェンスで足りる。
fn looks_like_json(raw: &str) -> bool {
    let s = raw.trim_start_matches('\u{feff}').trim_start();
    s.starts_with('{') || s.starts_with("```")
}

/// 貼り付け経路で混入する装飾だけを剥がす。
///
/// ★ **素のパースが失敗したときしか呼ばれない。** だから正常なデータを壊す心配がない
/// （種目名に全角引用符が入っていても、素のパースが通るのでここには来ない）。
fn repair(raw: &str) -> String {
    let mut s = raw.trim().trim_start_matches('\u{feff}').trim();
    // チャットやメモ経由で付くコードフェンス
    if let Some(rest) = s.strip_prefix("```") {
        let rest = rest.strip_prefix("json").unwrap_or(rest);
        s = rest.trim_start().trim_end_matches('`').trim();
    }
    // リッチテキスト経由で化ける引用符
    s.replace(['\u{201c}', '\u{201d}'], "\"")
        .replace(['\u{2018}', '\u{2019}'], "'")
}

// ── TSV の読み込み ──────────────────────────────────────────────────────────

/// 見出し行で分かった列の位置。**位置ではなく名前で引く**ための対応表。
///
/// ★ 位置で読むフォールバックを作らない。列順を推測すると「回数を重量として
/// 取り込む」事故が黙って通り、しかも数字なので画面を見ても気づけない。
#[derive(Default)]
struct TsvCols {
    date: Option<usize>,
    group: Option<usize>,
    exercise: Option<usize>,
    set_no: Option<usize>,
    weight: Option<usize>,
    reps: Option<usize>,
    body_weight: Option<usize>,
    drop: Option<usize>,
    set_note: Option<usize>,
    log_note: Option<usize>,
    day_note: Option<usize>,
    routine: Option<usize>,
    pins: Option<usize>,
    interval: Option<usize>,
    label: Option<usize>,
}

/// 見出し行 → 列の対応。知らない列は無視する（形式の進化規則 2）。
///
/// 別名を少しだけ許すのは、シートで見出しを打ち直した人を落とさないため。
/// **必須は「日付」と「種目」**で、これが無いと行の意味が決まらない。
fn tsv_header(line: &str) -> Option<TsvCols> {
    let mut cols = TsvCols::default();
    for (i, cell) in line.split('\t').enumerate() {
        // ★ **日英どちらの見出しも受ける**（adr/storage/tsv-header-follows-the-ui-language.md）。
        //   英語で初期化した端末が書き出したファイルを日本語の端末で取り込む経路が
        //   あるので、片方だけでは往復が閉じない。
        // ★ 大文字小文字は現行どおり厳密一致。既存の別名（「重量」「体重」）を手で
        //   列挙している方針をそのまま踏襲する
        let slot = match cell.trim() {
            "日付" | "Date" => &mut cols.date,
            "部位" | "Muscle group" | "Group" => &mut cols.group,
            "種目" | "Exercise" => &mut cols.exercise,
            "セット" | "セット番号" | "Set" | "Set no" => &mut cols.set_no,
            "重量kg" | "重量" | "Weight kg" | "Weight" => &mut cols.weight,
            "回数" | "Reps" => &mut cols.reps,
            // ★ 書き出す綴りは「ドロップ」/「Drop set」。`Drop` と「ドロップセット」は
            //   **読み取り専用の別名**（「重量」と同じ手）で、見出し配列には入れない
            //   ので `is_known_header_cell` の `NotDb` 判定は広がらない
            "ドロップ" | "ドロップセット" | "Drop set" | "Drop" => &mut cols.drop,
            "体重kg" | "体重" | "Body weight kg" | "Body weight" => &mut cols.body_weight,
            "セットメモ" | "Set note" => &mut cols.set_note,
            "種目メモ" | "Exercise note" => &mut cols.log_note,
            "体調メモ" | "Day note" => &mut cols.day_note,
            "メニュー" | "Routine" => &mut cols.routine,
            "ピン" | "Pins" => &mut cols.pins,
            "ラベル" | "Label" => &mut cols.label,
            "インターバル秒" | "インターバル" | "Interval sec" | "Interval" => {
                &mut cols.interval
            }
            // 「時刻」/ "Time" は書き出し専用（[`export_tsv`] の doc 参照）。読まない
            _ => continue,
        };
        // 同名が複数あれば最初を採る
        if slot.is_none() {
            *slot = Some(i);
        }
    }
    // 「日付」と「種目」が無いと行の意味が決まらない。呼び側が NoHeader（うちのファイル
    // だが壊れている）と NotDb（うちのファイルではない）を分ける
    (cols.date.is_some() && cols.exercise.is_some()).then_some(cols)
}

/// `2026-08-01` / `2026-8-1` / `2026/8/1` を受ける。**年は 4 桁必須。**
///
/// スラッシュを受けるのは、スプレッドシートが日付セルをロケール既定で書き戻すため。
///
/// ★ **年を 4 桁に縛るのが要。** 縛らないと `8/1/26`（M/D/Y ロケール）が「西暦 8 年
/// 1 月 26 日」として通る。`parse_date_key` は往復するので保存まで通り、カレンダーにも
/// グラフにも 2000 年前の記録として残って**二度と見つけられない**。読めない日付は
/// 黙って別の日にせず、行ごと弾いて呼び側に数えさせる。
fn parse_tsv_date(cell: &str) -> Option<NaiveDate> {
    let s = cell.trim();
    if s.is_empty() {
        return None;
    }
    let norm = s.replace('/', "-");
    let mut parts = norm.split('-');
    let year = parts.next()?.trim();
    // 4 桁でない先頭フィールドは年ではない（2 桁年 / M/D/Y の月）
    if year.len() != 4 || !year.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let y = year.parse::<i32>().ok()?;
    let m = parts.next()?.trim().parse::<u32>().ok()?;
    let d = parts.next()?.trim().parse::<u32>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    NaiveDate::from_ymd_opt(y, m, d)
}

/// セルの回数 / セット番号。**小数付きを受ける。**
///
/// ★ `parse_reps` を使わないのは、あちらが画面の入力欄用で `"10.0"` を弾くため。
/// スプレッドシートは数値列を `10.0` と書き戻すことがあり、弾くと**セットが丸ごと
/// 消える**（しかもシート上は正しく見えているので気づけない）。`parse_weight` が
/// 小数を受けるのと同じ寛容さをここにも置く。
fn parse_cell_count(s: &str) -> Option<u32> {
    let v = s.trim().replace(',', ".").parse::<f64>().ok()?;
    if !v.is_finite() || v < 0.5 {
        return None;
    }
    Some(v.round() as u32)
}

/// 「重量×回数」の 1 セット分の表記。**重量が入っていなければ回数だけ。**
///
/// ★ 表示（`views::fmt_set`）と書き出し（[`drops_cell`]）で同じ 1 本を通す。
/// `×` の綴りと「重量 0 = 重量なし」の約束は [`set_volume`] と共有する規範なので、
/// 書き分けると片方だけ直る。
pub fn fmt_wr(weight: f32, reps: u32) -> String {
    if weight > 0.0 {
        format!("{}×{}", fmt_weight(weight), reps)
    } else {
        reps.to_string()
    }
}

/// 段の並びを 1 セルに落とす。`50×5 40×4` のように**半角空白区切り**。
///
/// ★ `ピン` 列と同じ流儀（1 セル 1 要素にせず空白で並べる）。段は 1 セットに
/// 最大 [`MAX_DROPS`] 個なので、列を段数ぶん増やすより 1 セルに畳むほうが
/// 「1 セット 1 行」を壊さない（adr/storage/tsv-export-for-spreadsheets.md）。
///
/// 重量が 0 の段は `×5` のように重量を省く（`重量kg` 列で 0 を空セルにするのと同じ）。
fn drops_cell(s: &SetEntry) -> String {
    s.drops
        .iter()
        .map(|d| {
            if d.weight > 0.0 {
                fmt_wr(d.weight, d.reps)
            } else {
                // ★ 自重の段は `×5`。回数だけにすると `parse_drops_cell` が
                //   区切りと区別できず往復が閉じない（表示側の `fmt_wr` と違う点）
                format!("×{}", d.reps)
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// [`drops_cell`] の逆。読めない要素は**黙って捨てる**（行ごと落とすほどではない）。
///
/// ★ 区切りは空白 / 読点 / カンマを受け、掛け算記号は `×` `x` `X` `*` を受ける。
/// スプレッドシートで手で書き足す人が居るので、綴りは書き出しより広く取る。
/// 回数は [`parse_cell_count`] と同じ寛容さ（`5.0` を受ける）。
///
/// ★ **ここで [`MAX_DROPS`] を切らない。** 上限は [`clean_drops`] の仕事で、
/// [`parse_tsv`] は最後に必ず [`normalize`] を通る。ここで別に切ると規則が 2 本に割れる
/// （`parse_interval` が上限で丸めないのと同じ理由）。
fn parse_drops_cell(cell: &str) -> Vec<DropStage> {
    cell.split([' ', '\t', '、', ',', ';'])
        .filter(|part| !part.trim().is_empty())
        .filter_map(|part| {
            let (w, r) = part
                .trim()
                .split_once(['×', 'x', 'X', '*'])
                .map(|(a, b)| (a.trim(), b.trim()))?;
            // 重量は空を許す（自重の段）。回数は必須
            Some(DropStage {
                weight: parse_weight(w),
                reps: parse_cell_count(r)?,
            })
        })
        .collect()
}

/// 並べ直す前のセット。`(セット番号, 行番号, セット)`。
///
/// ★ 行番号まで持つのは、セット番号が欠けた行（手で足した行）の順序を安定させるため。
/// 番号が無いものは `u32::MAX` に落ちるので、番号付きの後ろに出現順で並ぶ。
type StagedSet = (u32, usize, SetEntry);

/// 名前から種目・部位を引き当てながら `Db` を組み立てる。
///
/// **解決の梯子（種目）**:
/// 1. `mine` に (部位名, 種目名) が**ちょうど 1 件** → その `Exercise` をそのまま複製
/// 2. `mine` に 種目名 が**ちょうど 1 件** → 同上（部位が変わっていても既存を優先）
/// 3. プリセット名に一致 → `presets::preset_exercise_id` の**固定 ID**
/// 4. それ以外 → 新規採番
///
/// ★ **1・2 が要る理由**: `merge_db` は同名一致のたびに `Conflict::NameMatched` を積む。
/// 新しい ID で渡すと、自分のファイルを戻すだけで確認画面が「同じ種目とみなしました」で
/// 埋まる。既存の `Exercise` を複製すれば `merge_db` は ID 一致の枝を素通りする。
///
/// ★ **3 が要る理由**: プリセットを改名した端末に別端末の TSV を入れると、名前照合が
/// 当たらず**同じ種目が 2 つになって履歴が割れる**。これは
/// adr/data-model/random-ids-for-safe-merge.md が固定 ID で潰した壊れ方そのもの。
///
/// ★ **1・2 が「ちょうど 1 件」なのは [`pin_presets`] と同じ理由。** 同名の種目が 2 つ
/// あるとき片方に寄せると、別種目の履歴が無警告で合流する。曖昧なら新規に倒す。
fn parse_tsv(raw: &str, ids: &mut IdGen, mine: &Db) -> Result<Db, ImportError> {
    let body = raw.trim_start_matches('\u{feff}');
    let mut lines = body
        .split('\n')
        .map(|l| l.trim_end_matches('\r'))
        .filter(|l| !l.trim().is_empty());

    let header = lines.next().ok_or(ImportError::Empty)?;
    let cols = match tsv_header(header) {
        Some(c) => c,
        None => {
            // 見出しに知っている列名が 1 つも無ければ「うちのファイルではない」
            return Err(
                if header.split('\t').any(|c| is_known_header_cell(c.trim())) {
                    ImportError::NoHeader
                } else {
                    ImportError::NotDb
                },
            );
        }
    };

    let mut out = Db::default();
    // ★ キャッシュは必須。無いと同じ部位・種目に行ごとに採番して `Db` が行数分ふくらむ
    let mut group_ids: HashMap<String, GroupId> = HashMap::new();
    let mut ex_ids: ExerciseCache = HashMap::new();
    // ★ キャッシュは必須（`LabelCache` の doc）。無いと行ごとに採番して
    //   1 種目に数百ラベルが生える
    let mut label_ids: LabelCache = HashMap::new();
    // ログごとのセット。(日付, 種目) → [(セット番号, 行番号, セット)]
    let mut staged: HashMap<(String, ExerciseId), Vec<StagedSet>> = HashMap::new();
    // メニュー名 → 種目の並び（**行の順序**がそのまま並び順）。
    // ★ `Vec` で持つのは順序が意味を持つから。`HashMap` の値に `HashSet` を使うと
    //   「胸の日」を開いたときのカードの並びが毎回変わる
    // (メニュー名, 種目の並び, 直前の行の番号)
    let mut staged_routines: Vec<(String, Vec<ExerciseId>, u32)> = Vec::new();
    // ★ 読めなかったセル。**黙って捨てない**ための計数（下の `NoRecords` 判定で使う）
    let mut unread = 0usize;

    for (row, line) in lines.enumerate() {
        let cells: Vec<&str> = line.split('\t').collect();
        let at = |slot: Option<usize>| -> &str {
            slot.and_then(|i| cells.get(i))
                .map(|s| s.trim())
                .unwrap_or("")
        };
        let date_cell = at(cols.date);
        let date = parse_tsv_date(date_cell);
        // ★ 日付欄が埋まっているのに読めない = 行ごと落ちる。これを数えないと、
        //   ロケール書き戻しで全行が落ちたファイルが「増えるものはありません」で通る
        if date.is_none() && !date_cell.is_empty() {
            unread += 1;
            continue;
        }
        let group_name = at(cols.group);
        let ex_name = at(cols.exercise);
        let routine_name = at(cols.routine);

        // トレーニングメニューの行（日付が空・メニュー列が埋まる）。
        // ★ 記録行より先に判定する。日付が入っている行にメニュー名が混ざっていても、
        //   それは記録として扱う（メニューは日付を持たないものと決めてある）
        if date.is_none() && !routine_name.is_empty() {
            let pos = parse_cell_count(at(cols.set_no)).unwrap_or(0);
            // ★ **名前でまとめてはいけない。** メニュー名は一意ではない（画面から同名を
            //   作れるし、`merge_db` は名前だけでは寄せない方針なので実際に並ぶ）。
            //   名前でまとめると「胸の日」が 2 本あるファイルを読み戻したときに
            //   **中身が融合した別物**になる。名前が変わるか、番号が増えなくなったら
            //   次のメニューとして開く
            let start_new = match staged_routines.last() {
                Some((name, _, last_pos)) => name != routine_name || pos <= *last_pos,
                None => true,
            };
            if start_new {
                staged_routines.push((routine_name.to_string(), Vec::new(), pos));
            } else if let Some(last) = staged_routines.last_mut() {
                last.2 = pos;
            }
            let members = &mut staged_routines.last_mut().expect("直前に確保した").1;
            if !ex_name.is_empty()
                && let Some(id) = resolve_exercise(
                    &mut out,
                    mine,
                    &mut group_ids,
                    &mut ex_ids,
                    ids,
                    group_name,
                    ex_name,
                )
            {
                take_pins(&mut out, id, at(cols.pins));
                take_interval(&mut out, id, at(cols.interval));
                // ★ 重複は初出だけ残す。同じ種目が 2 回入ると展開時にログが 2 本でき、
                //   「1 日 1 種目 1 ログ」が破れる（`normalize_routines` も同じことをする）
                if !members.contains(&id) {
                    members.push(id);
                }
            }
            continue;
        }

        // 部位だけの行（種目を持たない部位のマスタ行）
        if date.is_none() && ex_name.is_empty() {
            if !group_name.is_empty() {
                resolve_group(&mut out, mine, &mut group_ids, ids, group_name);
            }
            continue;
        }

        // 種目が要る行は、まず種目を引き当てる
        let ex_id = if ex_name.is_empty() {
            None
        } else {
            resolve_exercise(
                &mut out,
                mine,
                &mut group_ids,
                &mut ex_ids,
                ids,
                group_name,
                ex_name,
            )
        };

        if let Some(id) = ex_id {
            take_pins(&mut out, id, at(cols.pins));
            take_interval(&mut out, id, at(cols.interval));
        }

        let Some(date) = date else {
            continue; // 種目マスタ行。ここまでで `out.exercises` に載っている
        };
        let key = date_key(date);
        let session = out.sessions.entry(key.clone()).or_default();

        // 体重・体調メモは「その日の最初の非空」を採る（書き出しは先頭行にだけ書くが、
        // シートで並べ替えられても拾えるように、どの行から来ても受ける）
        let bw = parse_weight(at(cols.body_weight));
        if bw > 0.0 && session.body_weight.is_none() {
            session.body_weight = Some(bw);
        }
        append_note(&mut session.note, at(cols.day_note));

        let Some(ex_id) = ex_id else {
            continue; // 体重・体調メモだけの日
        };
        // ★ ログは**解決後の ID** でキーにする。同名が別部位で 2 行に分かれていても 1 本に
        let log = match session.logs.iter_mut().find(|l| l.exercise_id == ex_id) {
            Some(l) => l,
            None => {
                session.logs.push(ExerciseLog {
                    exercise_id: ex_id,
                    sets: Vec::new(),
                    // ★ 外から来たセットは過去日のバックフィルなので時刻を持たない
                    //   （adr/data-model/at-optional-same-day-only.md）。TSV の「時刻」列を
                    //   読まないのはこの規範に乗るためでもある
                    at: None,
                    note: String::new(),
                    label: None,
                });
                session.logs.last_mut().expect("今 push した")
            }
        };
        append_note(&mut log.note, at(cols.log_note));
        // ★ ラベルは「その日の最初の非空」を採る（体重・体調メモと同じ規則。書き出しは
        //   ログの先頭行にだけ書くが、シートで並べ替えられても拾えるように、どの行から
        //   来ても受ける）。`log` の借用を切ってから引き当てる
        let label_cell = at(cols.label);
        let need_label = log.label.is_none() && !label_cell.is_empty();
        if need_label
            && let Some(id) = resolve_label(&mut out, mine, &mut label_ids, ids, ex_id, label_cell)
            && let Some(log) = out
                .sessions
                .get_mut(&key)
                .and_then(|s| s.logs.iter_mut().find(|l| l.exercise_id == ex_id))
        {
            log.label = Some(id);
        }

        // 回数が空の行はセットを作らない（メモだけの行）。空でないのに読めないなら数える
        let reps_cell = at(cols.reps);
        if let Some(reps) = parse_cell_count(reps_cell) {
            let no = parse_cell_count(at(cols.set_no)).unwrap_or(u32::MAX);
            staged.entry((key, ex_id)).or_default().push((
                no,
                row,
                SetEntry {
                    weight: parse_weight(at(cols.weight)),
                    reps,
                    note: at(cols.set_note).to_string(),
                    drops: parse_drops_cell(at(cols.drop)),
                },
            ));
        } else if !reps_cell.is_empty() {
            unread += 1;
        }
    }

    // セット番号で並べ直す。★ シートで行を並べ替えられても順序が戻る。番号が無い行は
    // 出現順（`u32::MAX` と行番号の組で安定ソート）
    for ((key, ex_id), mut sets) in staged {
        sets.sort_by_key(|(no, row, _)| (*no, *row));
        if let Some(log) = out
            .sessions
            .get_mut(&key)
            .and_then(|s| s.logs.iter_mut().find(|l| l.exercise_id == ex_id))
        {
            log.sets = sets.into_iter().map(|(_, _, s)| s).collect();
        }
    }

    // ── メニューを組み立てる ──
    //
    // ★ **ID は毎回新しく採る。** 名前で `mine` に寄せない。`merge_db` は
    //   「ID 一致 → 名前と種目が**両方**一致 → 追加」で突き合わせる規則で、名前だけで
    //   寄せないことを意図して選んでいる（adr/data-model/routines-as-named-exercise-lists.md）。
    //   ここで名前に寄せると、その判断を取り込み経路だけ裏口から覆すことになる。
    //   自分のファイルを戻すぶんには名前も種目も一致するので重複しない。
    for (name, exercises, _) in staged_routines {
        out.routines.push(Routine {
            id: ids.alloc(),
            name,
            exercises,
        });
    }

    // ★ JSON の枝が `migrate` の中で通しているのと同じ関数を共有する。日付キーの
    //   再正規化・重複ログの畳み込み・空白メモの正規化・空セッション削除・`f32` で
    //   表せない重量の除去が、TSV 経由でも自動で効く。ここを自前で書き直すと必ずズレ、
    //   `"weight":null` を保存して次回起動が死ぬ経路（`drop_unrepresentable_weights`）に繋がる
    normalize(&mut out, ids);

    // ★ 判定は `normalize` の**後**。先に見ると、行は読めたが中身が全部落ちた形
    //   （部位が引けない種目だけのファイル）が「空の Db を取り込んだ」で通ってしまう。
    //
    //   ★ **記録が 1 日も残らなかったら、種目だけ拾えていても失敗にする。** ここを
    //   「groups と exercises と sessions が全部空」にしていると、日付や回数が
    //   ロケール書き戻しで全滅したファイルが `Ok(sessions: {})` で通り、画面は
    //   「新しく取り込むものはありません」と出す。**全部落ちたことに気づけない**のが
    //   このアプリでいちばん避けたい終わり方なので、読めなかったセルがあるなら言う
    //   ★ **「セッションが残ったか」ではなく「セットが 1 本でも読めたか」で見る枝を先に置く。**
    //   種目メモの列があると、回数が全滅してもログが**メモで生き残る**（この関数は回数を
    //   読む前にログを push して `append_note` する。`dedupe_logs` の刈り取りは
    //   「セットもメモも無い」なので落ちない）。すると `sessions` が非空になって下の
    //   判定を素通りし、`summarize` は「0 日・0 セット」と出るのに `added_text` は
    //   「N 日分・M 件の記録を追加します」と言う、という食い違いになる。
    //   コピーがメモを全記録日に載せるようになった（adr/ux/copy-carries-the-notes.md）
    //   ので、この形が例外ではなく既定になった。
    //
    //   ★ **下の式を書き換えて済ませてはいけない。** あちらは `(unread > 0 ||
    //   exercises.is_empty())` を含むので、条件を「セットが無い」に緩めると
    //   **体重・体調メモだけの正当なファイル**（`unread == 0`）が `NoRecords` になる。
    //   `unread > 0` を前置した別の枝にすれば、読めなかったセルが 1 つも無いファイルは
    //   1 本も触らない
    if unread > 0
        // ★ **ログが 1 本でも残っていること**が前提。ここを落とすと `all` が空集合に
        //   対して真になり、**体重だけの日しか無い正当なファイル**（ログが 0 本）が
        //   壊れた行 1 つで丸ごと拒否される。下の枝に任せるべき形をここで奪わない
        && out.sessions.values().any(|s| !s.logs.is_empty())
        // ★ `!is_trained()` は「そのセッションにセットが 1 本も無い」。**述語を手で
        //   書き直さない** — カレンダーのドット・月フッタ・グラフが見ているものと
        //   同じ関数を通すことで、「トレした」の定義が変わったときに取り込みの判定
        //   だけが取り残されるのを防ぐ
        && out.sessions.values().all(|s| !s.is_trained())
    {
        return Err(ImportError::Unreadable);
    }

    if out.sessions.is_empty()
        && out.routines.is_empty()
        && (unread > 0 || out.exercises.is_empty())
    {
        return Err(if unread > 0 {
            ImportError::Unreadable
        } else {
            ImportError::NoRecords
        });
    }

    out.schema = SCHEMA;
    Ok(out)
}

/// 種目の引き当てキャッシュ。**キーは (部位名, 種目名)。**
///
/// ★ 種目名だけにしてはいけない — 部位違いの同名種目が 1 つに潰れる。
type ExerciseCache = HashMap<(String, String), ExerciseId>;

/// ラベルの引き当てキャッシュ。**キーは (種目 ID, ラベル名)。**
///
/// ★ **必須。** 無いと行ごとに採番して 1 種目に数百ラベルが生える
/// （`ExerciseCache` と同じ理由）。
///
/// ★ 種目 ID を含めるのは、ラベルが**種目ごとに独立**しているから
/// （ラベル名だけだと種目 A の "P" と種目 B の "P" が 1 つに潰れる）。
type LabelCache = HashMap<(ExerciseId, String), LabelId>;

/// ちょうど 1 件のときだけ返す。**曖昧なら `None`。**
///
/// ★ 2 件目を見た時点で打ち切る。「ちょうど 1 件」は [`pin_presets`] と共通の規則で、
/// 同名が複数あるときに片方へ寄せると**別種目の履歴が無警告で合流する**。
fn exactly_one<T>(mut it: impl Iterator<Item = T>) -> Option<T> {
    let first = it.next()?;
    it.next().is_none().then_some(first)
}

/// 取り込んだ行の `ラベル` 列を `out` の種目のラベル定義へ引き当てる。
///
/// **解決の梯子**（[`resolve_exercise`] の 1 階層下。同じ形にしてある）:
/// 1. `mine` の**その種目**に同名がちょうど 1 件 → その `LabelId`
/// 2. `out` に既に作った同名 → キャッシュから
/// 3. それ以外 → 新規採番して `out` の種目に足す
///
/// ★ **1 が要る理由**: 自分のファイルを戻すだけで新しい ID が生えると、`merge_db` は
/// 「同名がちょうど 1 件」で寄せてくれるものの、ログの `label` が写像を通るぶん
/// 余計な往復が増える。手元の ID をそのまま使えば ID 一致の枝を素通りする
/// （`Conflict` も出ない）。
///
/// ★ 「ちょうど 1 件」なのは [`resolve_exercise`] / [`pin_presets`] と同じ理由 —
/// 同名が複数あるときに片方へ寄せると別の狙いの履歴が無警告で合流する。曖昧なら
/// 新規に倒す。
///
/// ★ [`crate::model::MAX_LABELS`] はここでは見ない。`parse_import` 末尾の
/// [`normalize`] が [`clean_labels`] を通すので、TSV 経路にも自動で効く。
fn resolve_label(
    out: &mut Db,
    mine: &Db,
    cache: &mut LabelCache,
    ids: &mut IdGen,
    ex_id: ExerciseId,
    name: &str,
) -> Option<LabelId> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let key = (ex_id, name.to_string());
    if let Some(id) = cache.get(&key) {
        return Some(*id);
    }
    // 1. `mine` の同じ種目に同名がちょうど 1 件
    let id = mine
        .exercise(ex_id)
        .and_then(|e| exactly_one(e.labels.iter().filter(|l| l.name == name)))
        .map(|l| l.id)
        // 3. 新規採番
        .unwrap_or_else(|| ids.alloc());
    // ★ `out` の側にも定義を足す。足さないとログだけがラベルを指して宙に浮く
    if let Some(e) = out.exercises.iter_mut().find(|e| e.id == ex_id)
        && !e.labels.iter().any(|l| l.id == id)
    {
        // ★ 色は TSV に無いので**ここで振る**（部位の `resolve_group` と同じ作法）。
        //   その種目に既に居るラベルの色を避けるので、1 ファイルで H / P / S が
        //   入ってきても 3 色に散る。`normalize` の [`clean_labels`] でも埋まるが、
        //   あちらは「出力位置基準」なので取り込み側の既存色を見ない
        let color = next_label_color(e.labels.iter().map(|l| l.color.as_str())).to_string();
        e.labels.push(Label {
            id,
            name: name.to_string(),
            color,
        });
    }
    cache.insert(key, id);
    Some(id)
}

/// 取り込んだ行の `ピン` 列を `out` の種目へ入れる。**空のときだけ**入れる。
///
/// ★ 「最初の非空を採る」のは体重・体調メモと同じ規則で、書き出しが種目の初出行に
/// しか書かない一方、シートで並べ替えられていても拾えるようにするため。
///
/// ★ 上書きしないのは `fill_pins`（`merge_db`）と同じ理由。梯子 1・2 で手元の種目を
/// 複製した枝では、ここに来た時点で既にローカルのピンが入っている。
fn take_pins(out: &mut Db, id: ExerciseId, cell: &str) {
    if cell.trim().is_empty() {
        return;
    }
    if let Some(e) = out.exercises.iter_mut().find(|e| e.id == id)
        && e.pins.is_empty()
    {
        e.pins = clean_pins(vec![cell.to_string()]);
    }
}

/// 取り込んだ行の `インターバル秒` 列を `out` の種目へ入れる。**未設定のときだけ**入れる。
///
/// ★ 規則は [`take_pins`] とまったく同じ（最初の非空を採る / 上書きしない）。
///
/// ★ **読めないセルは黙って無視する。** シートで `1:30` や `90秒` と打ち直された値を
/// 0 として取り込むと「休まない種目」に化ける。落とすほうが安全で、元の値は
/// ファイルに残っている。
fn take_interval(out: &mut Db, id: ExerciseId, cell: &str) {
    let Some(sec) = parse_interval(cell) else {
        return;
    };
    if let Some(e) = out.exercises.iter_mut().find(|e| e.id == id)
        && e.interval_sec.is_none()
    {
        e.interval_sec = clean_interval(Some(sec));
    }
}

/// 部位名 → `GroupId`。引き当てた部位は `out.groups` にも載せる。
fn resolve_group(
    out: &mut Db,
    mine: &Db,
    cache: &mut HashMap<String, GroupId>,
    ids: &mut IdGen,
    name: &str,
) -> GroupId {
    if let Some(id) = cache.get(name) {
        return *id;
    }
    let group = if let Some(only) = exactly_one(mine.groups.iter().filter(|g| g.name == name)) {
        // ★ 既存をそのまま複製する。`merge_db` が ID 一致の枝を素通りし、副作用が出ない
        only.clone()
    } else {
        let id = crate::presets::preset_group_id(name).unwrap_or_else(|| ids.alloc());
        Group {
            id,
            name: name.to_string(),
            // 色と並び順は TSV に無いので作り直す。`order` は `merge_db` が振り直す
            color: crate::presets::COLOR_CHOICES
                [(mine.groups.len() + cache.len()) % crate::presets::COLOR_CHOICES.len()]
            .to_string(),
            order: 0,
        }
    };
    let id = group.id;
    if !out.groups.iter().any(|g| g.id == id) {
        out.groups.push(group);
    }
    cache.insert(name.to_string(), id);
    id
}

/// 種目名（+ 部位名）→ `ExerciseId`。引き当てた種目は `out.exercises` にも載せる。
///
/// 部位が空で `mine` にも無い新しい種目は取り込まない（`None` を返す）。「その他」部位を
/// 自動生成しないのは、**部位がグラフの集計単位**だから — 増やすと過去の集計が割れる。
#[allow(clippy::too_many_arguments)]
fn resolve_exercise(
    out: &mut Db,
    mine: &Db,
    groups: &mut HashMap<String, GroupId>,
    cache: &mut ExerciseCache,
    ids: &mut IdGen,
    group_name: &str,
    name: &str,
) -> Option<ExerciseId> {
    // ★ キャッシュは **(部位名, 種目名)** で引く。種目名だけで引くと、部位違いの同名種目
    //   （画面に重複名チェックが無いので作れてしまう）が 1 つに潰れ、下の梯子 1 が
    //   意味を失う。**自分の書き出しを読み戻すだけで別種目にセットが移る。**
    let key = (group_name.to_string(), name.to_string());
    if let Some(id) = cache.get(&key) {
        return Some(*id);
    }

    // 1. (部位名, 種目名) がちょうど 1 件
    let in_group = exactly_one(mine.exercises.iter().filter(|e| {
        e.name == name && mine.group(e.group_id).is_some_and(|g| g.name == group_name)
    }));
    // 2. 種目名がちょうど 1 件
    let by_name = || exactly_one(mine.exercises.iter().filter(|e| e.name == name));

    let exercise = if let Some(only) = in_group {
        only.clone()
    } else if let Some(only) = by_name() {
        only.clone()
    } else {
        // 3/4. 新規。部位が引けないなら取り込まない
        if group_name.is_empty() {
            return None;
        }
        let group_id = resolve_group(out, mine, groups, ids, group_name);
        let id = crate::presets::preset_exercise_id(name).unwrap_or_else(|| ids.alloc());
        Exercise {
            id,
            name: name.to_string(),
            group_id,
            order: 0,
            archived: false,
            pins: Vec::new(),
            interval_sec: None,
            labels: Vec::new(),
        }
    };

    // 既存を複製した枝では、その種目の部位も `out` に要る（`merge_db` が group_alias を引く）
    let gid = exercise.group_id;
    if !out.groups.iter().any(|g| g.id == gid)
        && let Some(g) = mine.group(gid)
    {
        out.groups.push(g.clone());
        groups.insert(g.name.clone(), gid);
    }

    let id = exercise.id;
    if !out.exercises.iter().any(|e| e.id == id) {
        out.exercises.push(exercise);
    }
    cache.insert(key, id);
    Some(id)
}

/// 取り込み前後を数字で並べるための要約。**インポート事故を止める唯一の道具。**
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DbSummary {
    pub exercises: usize,
    pub days: usize,
    pub sets: usize,
    pub first: Option<NaiveDate>,
    pub last: Option<NaiveDate>,
}

pub fn summarize(db: &Db) -> DbSummary {
    let trained: Vec<NaiveDate> = db
        .sessions
        .iter()
        .filter(|(_, s)| s.is_trained())
        .filter_map(|(k, _)| parse_date_key(k))
        .collect();
    DbSummary {
        exercises: db.exercises.iter().filter(|e| !e.archived).count(),
        days: trained.len(),
        sets: db
            .sessions
            .values()
            .flat_map(|s| s.logs.iter())
            .map(|l| l.sets.len())
            .sum(),
        first: trained.iter().min().copied(),
        last: trained.iter().max().copied(),
    }
}

// ── マージ ──────────────────────────────────────────────────────────────────

/// マージで判断が必要だった箇所。**黙って混ぜず、数えて画面に出す。**
#[derive(Debug, PartialEq, Eq)]
pub enum Conflict {
    /// 同じ ID なのに名前が違った（改名）。取り込み先の名前を残した
    Renamed { kept: String, incoming: String },
    /// ID は違うが同名だった。同じものとみなした
    NameMatched { name: String },
    /// 同じ日・同じ種目でセットが食い違い、取り込む側を採った
    SetsDiverged { date: String, name: String },
    /// 同じ日で体重が食い違った。取り込み先を残した
    BodyWeight { date: String },
    /// 同じ ID のトレーニングメニューで中身が違った。取り込み先を残した
    ///
    /// ★ 「名前が違う」と「種目が違う」を分けない。利用者の次の行動（そのメニューを
    /// 開いて確かめる）がどちらでも同じなので、粒度を細かくしても選択肢が増えない。
    RoutineDiverged { name: String },
}

/// [`merge_db`] の結果。
///
/// ★ **数のカウンタは冪等**（同じファイルを 2 回入れると 2 回目は全部 0）。
/// `conflicts` は「2 つを突き合わせた結果」の記述なので、1 回目に取り込む側が勝った
/// 項目は 2 回目には食い違いが解消していて出てこない。**冪等性は数で見ること。**
#[derive(Debug, Default, PartialEq, Eq)]
pub struct MergeReport {
    pub groups_added: usize,
    pub exercises_added: usize,
    pub sessions_added: usize,
    pub logs_added: usize,
    /// 追記したメモの本数（体調メモ・種目メモ・セットメモの合計）。
    ///
    /// ★ これが無いと、メモだけが増えたマージで [`MergeReport::is_noop`] が真になり、
    /// 画面が「新しく取り込むものはありませんでした」と嘘をつく。メモの冪等性を
    /// **数で**見る口でもある（追記は `conflicts` に出ないので、他に見る手段が無い）。
    pub notes_added: usize,
    /// 新しく段が入ったセットの数。
    ///
    /// ★ [`notes_added`](Self::notes_added) と同じ理由で必要。段だけが増えたマージは
    /// `conflicts` に出ない（`same_sets` が段を見ないと決めたので）のに**推移タブの
    /// 数字が動く**ので、数えないと [`MergeReport::is_noop`] が真になって画面が
    /// 「新しく取り込むものはありませんでした」と嘘をつく。
    pub drops_added: usize,
    /// 追加したトレーニングメニューの本数。
    pub routines_added: usize,
    /// 増えたラベル（**定義の追加とログへの付与の合算**。`notes_added` が既に
    /// 異種混合の先例）。
    ///
    /// ★ これが無いと、ラベルだけが増えたマージで [`MergeReport::is_noop`] が真になり、
    /// 画面が「新しく取り込むものはありませんでした」と嘘をつく。ラベルは `conflicts` に
    /// 出ないのに**チップが増えて履歴の見え方が変わる**ので、数える以外に見る手段が無い。
    pub labels_added: usize,
    /// [`crate::model::MAX_LABELS`] を超えて取り込めなかったラベル定義の数。
    ///
    /// ★ **[`MergeReport::is_noop`] には入れない**（何も増えていないので）。代わりに
    /// `views::backup` の警告文に出す — 落ちたラベルを指すログは取り込み直しても
    /// 二度と生き返らない dangling になるのに、数も `Conflict` も出ないと気づけない。
    ///
    /// ★ `conflicts` に積まないのは、確認画面が `conflicts.is_empty()` で
    /// 「入れ替わる記録があります」に分岐するため。記録は 1 件も入れ替わらないので、
    /// 積むと確認画面が嘘をつく。
    pub labels_dropped: usize,
    pub conflicts: Vec<Conflict>,
}

impl MergeReport {
    /// 何も足さなかったか。
    ///
    /// ★ **ここに数を足したら `views::backup::added_text` にも必ず足すこと。**
    /// 片方だけだと `is_noop` が偽なのに文言の部品が空になり、画面に
    /// 「 を追加しました」だけが出る。
    pub fn is_noop(&self) -> bool {
        self.groups_added == 0
            && self.exercises_added == 0
            && self.sessions_added == 0
            && self.logs_added == 0
            && self.notes_added == 0
            && self.drops_added == 0
            && self.routines_added == 0
            && self.labels_added == 0
    }
}

/// ログの「強さ」。**順序に依存しない決定的なキー**。
///
/// セット数だけで比べると `[(60,10),(60,8)]` と `[(62,10),(60,8)]` の勝者が決まらず、
/// `merge(a, b)` と `merge(b, a)` で結果が変わる（可換でなくなる）。総ボリュームと
/// セット列の辞書順まで見て、必ず一方に決まるようにする。
type LogRank = (usize, u64, Vec<(u32, u32)>);

/// 別名写像で同じ種目に落ちたログを 1 本にまとめる。**強いほうを残す。**
///
/// 連結してはいけない（同じファイルを 2 回入れるとセットが倍になる）。初出の順序は保つ。
fn dedupe_by_exercise(logs: Vec<ExerciseLog>) -> Vec<ExerciseLog> {
    let mut order: Vec<ExerciseId> = Vec::new();
    let mut best: HashMap<ExerciseId, ExerciseLog> = HashMap::new();
    for log in logs {
        match best.get_mut(&log.exercise_id) {
            Some(existing) => {
                if log_rank(&log) > log_rank(existing) {
                    *existing = log;
                }
            }
            None => {
                order.push(log.exercise_id);
                best.insert(log.exercise_id, log);
            }
        }
    }
    order
        .into_iter()
        .filter_map(|id| best.remove(&id))
        .collect()
}

/// ★ **メモを見ない。** メモの有無で「どちらのセットを採るか」が変わってはいけない
/// （2 セットのログがメモ 1 個で 5 セットのログに勝つ形は論外）。同点の tie-break に
/// メモを足す案も、`*existing = log` で自分側のセットメモを失うので採らない —
/// [`merge_db`] は一致時に位置でメモを埋めるほうで情報を守る。
fn log_rank(l: &ExerciseLog) -> LogRank {
    let volume: f64 = l.sets.iter().map(set_volume).sum();
    (
        l.sets.len(),
        // f64 は Ord を持たないので整数へ。1000 倍は 0.5kg 刻みを潰さないため
        (volume * 1000.0) as u64,
        // 重量は非負なので `to_bits` の順序が値の順序と一致する
        l.sets
            .iter()
            .map(|s| (s.weight.to_bits(), s.reps))
            .collect(),
    )
}

/// 2 つの `Db` を混ぜる。**追加のみ。`mine` の既存の値は上書きしない。**
///
/// 守る不変条件はただ 1 つ:
/// **マージ前に「ベンチプレス」を指していたログは、マージ後も「ベンチプレス」を指す。**
///
/// これが成り立つのは ID が乱数で、プリセットが全端末共通の固定 ID を持つから。
/// 連番 ID のままだと、同じ種目を登録順だけ変えて登録した 2 台で `id = 2` が別々の
/// 種目を指し、突き合わせた瞬間に履歴が入れ替わる。
///
/// 規則:
/// - 部位 / 種目は **ID 一致 → 同名 → 新規追加** の順に判定する
/// - **取り込む側のログは、採用の仕方に関わらず必ず別名写像を通す。**
///   「`mine` に無い日をまるごと採用する」枝で写像を忘れると、`mine` に存在しない
///   種目を指す宙に浮いたログが生まれ、上の不変条件がその枝だけで破れる
/// - 同じ日の同じ種目でセットが違うときは [`log_rank`] の大きいほうを採る。
///   **連結してはいけない** — 同じファイルを 2 回入れるとセットが倍になる
pub fn merge_db(mine: &mut Db, theirs: Db) -> MergeReport {
    let mut report = MergeReport::default();

    // ── 部位 ──
    let mut group_alias: HashMap<GroupId, GroupId> = HashMap::new();
    for g in theirs.groups {
        if mine.group(g.id).is_some() {
            group_alias.insert(g.id, g.id);
            continue;
        }
        if let Some(existing) = mine.groups.iter().find(|x| x.name == g.name) {
            group_alias.insert(g.id, existing.id);
            continue;
        }
        let id = g.id;
        let order = mine.groups.len() as u32;
        mine.groups.push(Group { order, ..g });
        group_alias.insert(id, id);
        report.groups_added += 1;
    }

    // ── 種目 ──
    let mut exercise_alias: HashMap<ExerciseId, ExerciseId> = HashMap::new();
    // ★ キーは `(写像後の ExerciseId, LabelId)`（[`merge_labels`] の ★）
    let mut label_alias: HashMap<(ExerciseId, LabelId), LabelId> = HashMap::new();
    for e in theirs.exercises {
        if let Some(existing) = mine.exercises.iter_mut().find(|x| x.id == e.id) {
            if existing.name != e.name {
                // 改名は正当な操作。取り込み先の名前を残し、あったことだけ伝える
                report.conflicts.push(Conflict::Renamed {
                    kept: existing.name.clone(),
                    incoming: e.name.clone(),
                });
            }
            fill_pins(existing, &e);
            fill_interval(existing, &e);
            let (added, dropped) = merge_labels(existing, &e, &mut label_alias);
            report.labels_added += added;
            report.labels_dropped += dropped;
            exercise_alias.insert(e.id, e.id);
            continue;
        }
        if let Some(existing) = mine.exercises.iter_mut().find(|x| x.name == e.name) {
            exercise_alias.insert(e.id, existing.id);
            fill_pins(existing, &e);
            fill_interval(existing, &e);
            // ★ **同名寄せの枝でも呼ぶ。** 忘れると名前で寄せた種目のラベルだけが
            //   落ちる（一番踏みやすいミス）。3 つ目の新規追加枝は `..e` なので
            //   `labels` が自動で乗る
            let (added, dropped) = merge_labels(existing, &e, &mut label_alias);
            report.labels_added += added;
            report.labels_dropped += dropped;
            report.conflicts.push(Conflict::NameMatched {
                name: e.name.clone(),
            });
            continue;
        }
        let id = e.id;
        let group_id = group_alias.get(&e.group_id).copied().unwrap_or(e.group_id);
        let order = mine
            .exercises
            .iter()
            .filter(|x| x.group_id == group_id)
            .count() as u32;
        mine.exercises.push(Exercise {
            group_id,
            order,
            ..e
        });
        exercise_alias.insert(id, id);
        report.exercises_added += 1;
    }

    // ── トレーニングメニュー ──
    //
    // ★ **種目ループの後に置く。** `exercise_alias` が完成していないと写像を張れない。
    //
    // 判定は `ID 一致 → (同名 かつ 種目列が完全一致) → 新規追加`。
    // ★ **同名でも中身が違えば寄せない**（種目とはここだけ規則を変える）。種目で
    //   同名を寄せるのは履歴がぶら下がっているからで、寄せないとグラフの系列が 2 本に
    //   割れる。メニューには**何もぶら下がっていない**ので、天秤が逆になる:
    //     - 寄せて外す → 取り込む側のリストがどこにも残らず消える（不可逆・不可視）
    //     - 寄せずに残す → 「胸の日」が 2 行並ぶ（可視・1 タップで消せる）
    //   ID が違って名前だけ同じなのは「2 台で独立に作った」ときで、それは本当に別物。
    //   中身が完全一致するときだけ寄せれば、純粋な重複は潰しつつ 1 本も失わない。
    // ★ 却下: 種目列を union する。「この端末では意図的にチェストフライを外した」が
    //   毎回の取り込みで無言に戻る。`merge_db` がセットを連結しないのと同型
    for r in theirs.routines {
        // ★ どの枝を通るかに関わらず、先に写像を適用しておく（セッションと同じ規則）。
        //   同名判定も**写像適用後**の列で行う。前で比べると、同じ種目を指しているのに
        //   ID が違うだけで「別物」と判定され重複が残る
        let exercises: Vec<ExerciseId> = r
            .exercises
            .iter()
            .map(|id| exercise_alias.get(id).copied().unwrap_or(*id))
            // ★ 写像は単射とは限らない。取り込み先で改名済みの種目と、取り込む側の
            //   同名の別種目が同じ ID に落ちると、1 本のメニューに同じ exercise_id が
            //   2 回入る。展開すると「1 日 1 種目 1 ログ」が破れる
            //   （セッション側の `dedupe_by_exercise` とまったく同じ危険）
            .filter(first_occurrence())
            .collect();

        if let Some(existing) = mine.routine(r.id) {
            if existing.name != r.name || existing.exercises != exercises {
                report.conflicts.push(Conflict::RoutineDiverged {
                    name: existing.name.clone(),
                });
            }
            continue;
        }
        if mine
            .routines
            .iter()
            .any(|x| x.name == r.name && x.exercises == exercises)
        {
            continue;
        }
        mine.routines.push(Routine {
            id: r.id,
            name: r.name,
            exercises,
        });
        report.routines_added += 1;
    }

    // ── セッション ──
    for (date, session) in theirs.sessions {
        // ★ どの枝を通るかに関わらず、先に写像を適用しておく
        let mapped: Vec<ExerciseLog> = session
            .logs
            .into_iter()
            .map(|l| {
                let ex_id = exercise_alias
                    .get(&l.exercise_id)
                    .copied()
                    .unwrap_or(l.exercise_id);
                ExerciseLog {
                    exercise_id: ex_id,
                    // ★ **写像後の種目 ID で引く。** 種目が寄ったのにラベルが元の
                    //   種目のキーで引かれると張り替えが起きない
                    label: l
                        .label
                        .map(|id| label_alias.get(&(ex_id, id)).copied().unwrap_or(id)),
                    ..l
                }
            })
            .collect();
        // ★ 写像は単射とは限らない。取り込み先で改名済みの種目と、取り込む側の
        //   同名の別種目が同じ ID に落ちると、同じ日に同一 exercise_id のログが
        //   2 本できる（adr/data-model/one-log-per-exercise-per-day.md「1 日 1 種目 1 ログ」違反）。そのまま入れると
        //   画面は 1 本目しか見ず、**次回起動の dedupe_logs が別種目のセットを
        //   連結する** — このリリースが潰そうとしている壊れ方そのものになる
        let logs = dedupe_by_exercise(mapped);

        let Some(dst) = mine.sessions.get_mut(&date) else {
            report.sessions_added += 1;
            report.logs_added += logs.len();
            mine.sessions.insert(
                date,
                Session {
                    logs,
                    body_weight: session.body_weight,
                    note: session.note,
                },
            );
            continue;
        };

        for log in logs {
            let Some(existing) = dst
                .logs
                .iter_mut()
                .find(|x| x.exercise_id == log.exercise_id)
            else {
                dst.logs.push(log);
                report.logs_added += 1;
                continue;
            };
            // ★ セットの採否より**先に**種目メモを合わせる。あとに回すと下の
            //   `*existing = log` が取り込み先のメモを取り込む側のもので上書きして消す
            if append_note(&mut existing.note, &log.note) {
                report.notes_added += 1;
            }
            // ★ ラベルは**空のときだけ埋める**（`mine` 優先。
            //   adr/storage/import-is-merge-only.md の「足すだけ」）。**あとに回しては
            //   いけない** — 下の `*existing = log` が上書きして消す
            if existing.label.is_none() && log.label.is_some() {
                existing.label = log.label;
                report.labels_added += 1;
            }
            // ★ `==` ではなく `same_sets`。メモだけの違いを食い違い扱いにすると、
            //   rank が同点なので下の分岐にも入れず、取り込む側のセットメモが
            //   `Conflict` も出さずに黙って捨てられる
            if same_sets(&existing.sets, &log.sets) {
                // 重量・回数が一致する組だけ、位置でセットメモを埋める。並びが同じなので
                // 対応がつく（食い違うときは埋めない — 別のセットにメモが付くほうが害が大きい）
                for (mine, theirs) in existing.sets.iter_mut().zip(&log.sets) {
                    if append_note(&mut mine.note, &theirs.note) {
                        report.notes_added += 1;
                    }
                    // ★ ドロップの段も同じ理由で合流させる。`same_sets` が段を見ない
                    //   （= 段の差だけでは食い違いにしない）ので、ここで運ばないと
                    //   他端末で入れた段が `Conflict` も出さずに黙って消える。
                    //   **空のときだけ入れる**（両方に段があれば取り込み先を残す）
                    if mine.drops.is_empty() && !theirs.drops.is_empty() {
                        mine.drops = theirs.drops.clone();
                        report.drops_added += 1;
                    }
                }
                continue;
            }
            // ★ 中身は同じで**並びだけ**違う ＝ 食い違いではない。取り込み先の並びを残す。
            //   ここで掬わないと下の rank 比較に落ち、第 3 要素が位置依存なので勝ち負けが
            //   実質任意に決まって、並びが黙って戻るうえ負けた側のセットメモが消える
            //   （adr/ux/drag-to-reorder-in-record-tab.md）。`Conflict` も出さない —
            //   利用者から見て食い違っていないものを食い違いとして報告するのは嘘になる。
            //   ★ **メモは位置ではなく「重量・回数が同じセット同士」で合流させる。**
            //     ここで捨てると、片方の端末で 1 度並べ替えただけで**もう片方の
            //     セットメモが二度と合流しなくなる**（上の `same_sets` の枝に
            //     二度と入らないため）。並びが違うだけで記録は同じなのだから、
            //     メモを落とす理由が無い
            if same_sets_unordered(&existing.sets, &log.sets) {
                report.notes_added += merge_set_notes_unordered(&mut existing.sets, &log.sets);
                report.drops_added += merge_set_drops_unordered(&mut existing.sets, &log.sets);
                continue;
            }
            // 取り込む側が強いときだけ差し替える。逆向きは黙って捨てる
            // （記録すると、同じファイルを 2 回入れたとき同じ食い違いを毎回報告する）
            if log_rank(&log) > log_rank(existing) {
                report.conflicts.push(Conflict::SetsDiverged {
                    date: date.clone(),
                    name: mine
                        .exercises
                        .iter()
                        .find(|x| x.id == log.exercise_id)
                        .map_or_else(|| log.exercise_id.to_string(), |x| x.name.clone()),
                });
                // ★ 上で合流させた種目メモを持ち越す。`*existing = log` だけだと、
                //   セットが負けたせいで**取り込み先のメモまで消える**
                let note = std::mem::take(&mut existing.note);
                // ★ 時刻も同じ理由で持ち越す。TSV は `at` を持たない
                //   （adr/storage/tsv-export-for-spreadsheets.md）ので、当日の記録に
                //   1 セット足したファイルを取り込むだけで**実施時刻が消える**。
                //   取り込む側が時刻を持っているならそちらを優先する
                let at = log.at.or(existing.at);
                // ★ ラベルも持ち越す。`..log` に任せると**勝った側で上書きされる** —
                //   上で `mine` 優先で埋めたはずの値が、セットが負けただけで
                //   取り込む側のものに入れ替わる（`note` / `at` と同じ理由）
                let label = existing.label.or(log.label);
                *existing = ExerciseLog {
                    note,
                    at,
                    label,
                    ..log
                };
            }
        }

        match (dst.body_weight, session.body_weight) {
            (None, Some(w)) => dst.body_weight = Some(w),
            (Some(a), Some(b)) if a != b => report
                .conflicts
                .push(Conflict::BodyWeight { date: date.clone() }),
            _ => {}
        }

        if append_note(&mut dst.note, &session.note) {
            report.notes_added += 1;
        }
    }

    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DropStage, Exercise, Group, SetEntry};
    use chrono::Weekday;

    const HOUR_MS: i64 = 3_600_000;
    const DAY_MS: i64 = 24 * HOUR_MS;

    /// テスト用の ID。小さい数字をそのまま書けるようにする糖衣。
    ///
    /// ★ 予約領域（`RESERVED_MAX` = 1024）の外に置く。中に置くとプリセットの固定 ID と
    /// たまたま衝突し、移行テストの意味が変わってしまう。
    fn g(n: u64) -> GroupId {
        GroupId::from_bits(0x1_0000 + n)
    }

    fn e(n: u64) -> ExerciseId {
        ExerciseId::from_bits(0x1_0000 + n)
    }

    /// ★ 種目 ID と**別の帯**（`0x2_0000`）に置く。同じ帯だと
    /// 「`(ExerciseId, LabelId)` のキーが要る」ことを見るテストで、
    /// たまたま一致して通ってしまう
    fn lb(n: u64) -> LabelId {
        LabelId::from_bits(0x2_0000 + n)
    }

    /// 決定的な採番器。`migrate` に渡す。
    fn ids() -> IdGen {
        IdGen::from_seed(1)
    }

    // 胸(1): ベンチプレス(10) / プッシュアップ(11)
    // 体幹(2): プランク(20)
    // 脚(3): 種目なし
    //
    // 旧 Kind でいう Weighted / Bodyweight / Duration が 1 つずつ混ざる構成のまま
    // （指標の式が 1 本になっても、混在部位の合算が壊れないことを見たいので）
    fn test_db() -> Db {
        let mut db = Db::default();
        db.groups.push(Group {
            id: g(1),
            name: "胸".into(),
            color: "#e0524a".into(),
            order: 0,
        });
        db.groups.push(Group {
            id: g(2),
            name: "体幹".into(),
            color: "#6b7280".into(),
            order: 1,
        });
        db.groups.push(Group {
            id: g(3),
            name: "脚".into(),
            color: "#2fa06a".into(),
            order: 2,
        });
        db.exercises.push(ex(10, "ベンチプレス", 1));
        db.exercises.push(ex(11, "プッシュアップ", 1));
        db.exercises.push(ex(20, "プランク", 2));
        db
    }

    fn ex(id: u64, name: &str, group_id: u64) -> Exercise {
        Exercise {
            id: e(id),
            name: name.into(),
            group_id: g(group_id),
            order: 0,
            archived: false,
            pins: Vec::new(),
            interval_sec: None,
            labels: Vec::new(),
        }
    }

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).expect("有効な日付")
    }

    fn log(exercise_id: u64, sets: &[(f32, u32)], at: Option<i64>) -> ExerciseLog {
        ExerciseLog {
            exercise_id: e(exercise_id),
            sets: sets
                .iter()
                .map(|(weight, reps)| SetEntry {
                    weight: *weight,
                    reps: *reps,
                    ..Default::default()
                })
                .collect(),
            at,
            note: String::new(),
            label: None,
        }
    }

    /// メモ入りのログ。種目メモとセットメモを 1 本で組み立てる。
    fn noted_log(
        exercise_id: u64,
        note: &str,
        sets: &[(f32, u32, &str)],
        at: Option<i64>,
    ) -> ExerciseLog {
        ExerciseLog {
            exercise_id: e(exercise_id),
            sets: sets
                .iter()
                .map(|(weight, reps, set_note)| SetEntry {
                    weight: *weight,
                    reps: *reps,
                    note: set_note.to_string(),
                    ..Default::default()
                })
                .collect(),
            at,
            note: note.to_string(),
            label: None,
        }
    }

    fn put(db: &mut Db, date: NaiveDate, logs: Vec<ExerciseLog>) {
        db.sessions.insert(
            date_key(date),
            Session {
                logs,
                ..Session::default()
            },
        );
    }

    fn r(n: u64) -> RoutineId {
        RoutineId::from_bits(0x2_0000 + n)
    }

    /// テスト用のトレーニングメニュー。
    fn routine(id: u64, name: &str, exercises: &[u64]) -> Routine {
        Routine {
            id: r(id),
            name: name.into(),
            exercises: exercises.iter().map(|n| e(*n)).collect(),
        }
    }

    /// `db` を JSON にして `migrate` で読み戻す。正規化の観測に使う。
    fn round_trip(db: &Db) -> Db {
        let raw = serde_json::to_string(db).expect("直列化できる");
        migrate(&raw, &mut ids()).expect("自分が書いた JSON は読める")
    }

    // ── 指標 ────────────────────────────────────────────────────────────────

    fn set(weight: f32, reps: u32) -> SetEntry {
        SetEntry {
            weight,
            reps,
            ..Default::default()
        }
    }

    /// ドロップの段が付いたメインセット。段は `(重量, 回数)` の並びで渡す。
    fn drop_set(weight: f32, reps: u32, stages: &[(f32, u32)]) -> SetEntry {
        SetEntry {
            weight,
            reps,
            drops: stages
                .iter()
                .map(|(w, r)| DropStage {
                    weight: *w,
                    reps: *r,
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn set_volume_treats_missing_weight_as_one() {
        // 重量あり = 素直な積
        assert_eq!(set_volume(&set(60.0, 10)), 600.0);
        // 重量なし = 実質レップ数（自重種目）
        assert_eq!(set_volume(&set(0.0, 12)), 12.0);
        // 重量なし = 実質秒数（時間種目）。式は上と同じ
        assert_eq!(set_volume(&set(0.0, 60)), 60.0);
        // 空のログは 0
        assert_eq!(log_value(Metric::Volume, &log(10, &[], None)), 0.0);
    }

    /// ★ **3 指標すべてで段の扱いを揃える。** `Sets` だけ段を数えないような食い違いを
    ///   作ると、同じ設定で「ボリュームは増えたのにセット数は変わらない」が起きて、
    ///   利用者は設定が効いているのか壊れているのか区別できない。
    #[test]
    fn log_value_of_counts_the_drop_stages_only_when_asked() {
        // 60×10 + 60×6 で、2 セット目に 50×5 / 40×4 の段が付いている
        let l = ExerciseLog {
            exercise_id: e(10),
            sets: vec![set(60.0, 10), drop_set(60.0, 6, &[(50.0, 5), (40.0, 4)])],
            label: None,
            at: None,
            note: String::new(),
        };

        for (m, all, main_only) in [
            (Metric::Volume, 1370.0, 960.0),
            (Metric::Sets, 4.0, 2.0),
            (Metric::Reps, 25.0, 16.0),
        ] {
            assert_eq!(log_value_of(m, &l, Drops::Include), all, "{m:?} を含める側");
            assert_eq!(
                log_value_of(m, &l, Drops::Exclude),
                main_only,
                "{m:?} を除く側"
            );
            // 記録タブ・カレンダーの入口は常に段も数える
            assert_eq!(log_value(m, &l), all, "{m:?} の既定が変わっている");
        }
    }

    /// ★ 段はメインセットにぶら下がるので、`set_volume` は**メインセットぶんだけ**を
    ///   返さなければならない。ここに足し込むと `Metric::Sets` に効かせられない。
    #[test]
    fn set_volume_excludes_the_stages() {
        let s = drop_set(60.0, 6, &[(50.0, 5), (40.0, 4)]);
        assert_eq!(set_volume(&s), 360.0);

        let l = ExerciseLog {
            exercise_id: e(10),
            sets: vec![s],
            label: None,
            at: None,
            note: String::new(),
        };
        // 段のぶんは 50×5 + 40×4 = 410
        assert_eq!(log_value_of(Metric::Volume, &l, Drops::Include), 770.0);
        // 段のほうも「重量なし = 重量 1」が効く（自重の段）
        let bodyweight = ExerciseLog {
            exercise_id: e(10),
            sets: vec![drop_set(60.0, 6, &[(0.0, 8)])],
            label: None,
            at: None,
            note: String::new(),
        };
        assert_eq!(
            log_value_of(Metric::Volume, &bodyweight, Drops::Include),
            368.0
        );
    }

    /// ★ 既定は「入れない」。ここが逆に倒れると、設定を触っていない利用者の
    ///   グラフが黙って上がる。
    #[test]
    fn drops_setting_only_includes_on_an_exact_one() {
        assert_eq!(drops_setting(None), Drops::Exclude, "未設定は除く");
        assert_eq!(drops_setting(Some(0)), Drops::Exclude);
        assert_eq!(drops_setting(Some(1)), Drops::Include);
        // ★ 知らない値で集計を広げてはいけない（2 択なので clamp ではなく既定へ）
        for weird in [2, 7, -1, i64::MAX, i64::MIN] {
            assert_eq!(
                drops_setting(Some(weird)),
                Drops::Exclude,
                "知らない値 {weird} を含める側に倒している"
            );
        }
        assert_eq!(Drops::default(), Drops::Exclude);
    }

    /// ★ 落とし幅は自由入力なので、**丸めと clamp をここで閉じる**。100 を通すと
    ///   計算結果が 0kg になり `set_volume` の「重量なし = 重量 1」に化ける。
    #[test]
    fn drop_pct_rounds_to_one_decimal_and_clamps() {
        assert_eq!(drop_pct(None), DEFAULT_DROP_PCT);
        assert_eq!(drop_pct(Some(20.0)), 20.0);
        assert_eq!(drop_pct(Some(12.5)), 12.5);
        // 小数点以下 2 桁目は丸める
        assert_eq!(drop_pct(Some(12.46)), 12.5);
        assert_eq!(drop_pct(Some(12.44)), 12.4);
        // 0 は「落とさない」で有効
        assert_eq!(drop_pct(Some(0.0)), 0.0);
        // 範囲外は clamp（捨てない）
        assert_eq!(drop_pct(Some(-5.0)), 0.0);
        assert_eq!(drop_pct(Some(100.0)), MAX_DROP_PCT);
        assert_eq!(drop_pct(Some(1e9)), MAX_DROP_PCT);
        // 壊れた値は既定へ
        assert_eq!(drop_pct(Some(f64::NAN)), DEFAULT_DROP_PCT);
        assert_eq!(drop_pct(Some(f64::INFINITY)), DEFAULT_DROP_PCT);
    }

    /// ★ プリフィルは**そのセットの 1 段目だけ**（呼び出し側の責任）。ここは式だけを見る。
    #[test]
    fn dropped_weight_takes_the_percentage_off_the_main_set() {
        assert_eq!(dropped_weight(60.0, 20.0), Some(48.0));
        assert_eq!(dropped_weight(100.0, 12.5), Some(87.5));
        // 小数点以下 1 桁に丸める（62.5 の 20% 引きは 50.0）
        assert_eq!(dropped_weight(62.5, 20.0), Some(50.0));
        // 0% は落とさない
        assert_eq!(dropped_weight(60.0, 0.0), Some(60.0));
        // ★ 重量の無いメインセット（自重種目）は `None`。0 を入れるより空のまま
        //   利用者に打たせるほうが正しい
        assert_eq!(dropped_weight(0.0, 20.0), None);
    }

    /// ★ 単調性: 重量を足して指標が下がってはいけない。
    ///
    /// `max(1.0)` を外して素の積にすると 0.5kg×10 = 5 になり、
    /// 「自重 10 回（= 10）→ 0.5kg を持って 10 回（= 5）」でグラフが下がる。
    /// 上下が負荷の増減を表さなくなるのでグラフの意味が壊れる。
    #[test]
    fn set_volume_is_monotonic_in_weight() {
        // `max(1.0)` の境界 1.0 を跨いでも非減少であること。1.0 未満は全て自重の 10 に
        // 潰れることまでリテラルで固定する（`max(0.5)` 等に緩めても非減少という
        // 性質だけなら壊れないため、比較ではなく値そのものを見る）
        let weights = [0.0, 0.5, 0.999, 1.0, 1.001, 2.0];
        let got: Vec<f64> = weights.iter().map(|w| set_volume(&set(*w, 10))).collect();
        assert_eq!(got, vec![10.0, 10.0, 10.0, 10.0, 10.010000467300415, 20.0]);
    }

    /// schema 1 からの値の変化を固定する。ここが動いたら ADR とリリースノートも直す。
    /// `set_volume_treats_missing_weight_as_one`（4462）/ `set_volume_is_monotonic_in_weight`
    /// （4592）と重なるが、ADR の 3 ケースを名指しで固定する。
    #[test]
    fn set_volume_changes_these_three_cases_from_schema_1() {
        // 1. 自重 + 追加重量（旧 Bodyweight は weight を指標に載せなかった）
        //    ディップス +10kg × 8: 旧 8 → 新 80
        assert_eq!(set_volume(&set(10.0, 8)), 80.0);
        // 2. 加重種目で重量を空のまま保存した記録: 旧 0 → 新 reps
        assert_eq!(set_volume(&set(0.0, 10)), 10.0);
        // 3. 0 < 重量 < 1（0.5kg プレート）: 旧 5 → 新 10
        assert_eq!(set_volume(&set(0.5, 10)), 10.0);
    }

    #[test]
    fn log_value_covers_every_metric() {
        // 60×10 + 60×8
        let l = log(10, &[(60.0, 10), (60.0, 8)], None);
        assert_eq!(log_value(Metric::Volume, &l), 1080.0);
        assert_eq!(log_value(Metric::Sets, &l), 2.0);
        assert_eq!(log_value(Metric::Reps, &l), 18.0);

        // 重量なしでも 3 指標とも意味を持つ
        let bw = log(11, &[(0.0, 12), (0.0, 10)], None);
        assert_eq!(log_value(Metric::Volume, &bw), 22.0);
        assert_eq!(log_value(Metric::Sets, &bw), 2.0);
        assert_eq!(log_value(Metric::Reps, &bw), 22.0);
    }

    #[test]
    fn metric_units_come_from_the_metric_not_the_exercise() {
        // ボリュームは重量と回数の合成量なので単位を持たない
        assert_eq!(Metric::Volume.unit(crate::i18n::Lang::Ja), "");
        assert_eq!(Metric::Sets.unit(crate::i18n::Lang::Ja), "セット");
        assert_eq!(Metric::Reps.unit(crate::i18n::Lang::Ja), "回");
        assert_eq!(Metric::default(), Metric::Volume);
        assert_eq!(Metric::choices(crate::i18n::Lang::Ja).len(), 3);
    }

    // ── last_log_before ─────────────────────────────────────────────────────

    #[test]
    fn last_log_before_returns_the_latest_strictly_earlier_record() {
        let mut db = test_db();
        put(&mut db, d(2026, 8, 1), vec![log(10, &[(50.0, 10)], None)]);
        put(&mut db, d(2026, 8, 4), vec![log(10, &[(55.0, 10)], None)]);
        put(&mut db, d(2026, 8, 8), vec![log(10, &[(60.0, 10)], None)]);

        // 8/8 自身は含まない（厳密に前）
        let (date, l) = last_log_before(&db, e(10), d(2026, 8, 8)).expect("8/4 がある");
        assert_eq!(date, d(2026, 8, 4));
        assert_eq!(
            l.sets,
            vec![SetEntry {
                weight: 55.0,
                reps: 10,
                ..Default::default()
            }]
        );

        let (date, _) = last_log_before(&db, e(10), d(2026, 8, 4)).expect("8/1 がある");
        assert_eq!(date, d(2026, 8, 1));

        // 最古の記録日より前には何もない
        assert_eq!(last_log_before(&db, e(10), d(2026, 8, 1)), None);
        // 別種目の記録は拾わない
        assert_eq!(last_log_before(&db, e(11), d(2026, 8, 8)), None);
    }

    #[test]
    fn last_log_before_skips_sessions_without_that_exercise() {
        let mut db = test_db();
        put(&mut db, d(2026, 8, 1), vec![log(10, &[(50.0, 10)], None)]);
        put(&mut db, d(2026, 8, 5), vec![log(20, &[(0.0, 60)], None)]);
        put(&mut db, d(2026, 8, 7), vec![log(10, &[], None)]); // 空セットは記録ではない

        let (date, _) = last_log_before(&db, e(10), d(2026, 8, 8)).expect("8/1 まで遡る");
        assert_eq!(date, d(2026, 8, 1));
    }

    // ── last_logs_before / history_count ────────────────────────────────────

    #[test]
    fn last_logs_before_returns_the_newest_days_first() {
        let mut db = test_db();
        put(&mut db, d(2026, 8, 1), vec![log(10, &[(50.0, 10)], None)]);
        put(&mut db, d(2026, 8, 4), vec![log(10, &[(55.0, 10)], None)]);
        put(&mut db, d(2026, 8, 8), vec![log(10, &[(60.0, 10)], None)]);

        let got = last_logs_before(&db, e(10), d(2026, 8, 8), 3);
        assert_eq!(
            got.iter().map(|(date, _)| *date).collect::<Vec<_>>(),
            vec![d(2026, 8, 4), d(2026, 8, 1)],
            "8/8 自身は含まず、新しい順に並ぶ"
        );

        // 足りなくても panic しない。あるぶんだけ返す
        assert_eq!(last_logs_before(&db, e(10), d(2026, 8, 2), 3).len(), 1);
        assert!(last_logs_before(&db, e(10), d(2026, 8, 1), 3).is_empty());
        assert!(
            last_logs_before(&db, e(11), d(2026, 8, 8), 3).is_empty(),
            "別種目の記録は拾わない"
        );
    }

    #[test]
    fn last_logs_before_stops_at_the_limit() {
        let mut db = test_db();
        put(&mut db, d(2026, 8, 1), vec![log(10, &[(50.0, 10)], None)]);
        put(&mut db, d(2026, 8, 4), vec![log(10, &[(55.0, 10)], None)]);
        put(&mut db, d(2026, 8, 6), vec![log(10, &[(57.5, 10)], None)]);

        let got = last_logs_before(&db, e(10), d(2026, 8, 8), 2);
        assert_eq!(
            got.iter().map(|(date, _)| *date).collect::<Vec<_>>(),
            vec![d(2026, 8, 6), d(2026, 8, 4)],
            "新しいほうから 2 件で打ち切る"
        );
    }

    #[test]
    fn last_logs_before_skips_days_without_that_exercise_or_with_no_sets() {
        let mut db = test_db();
        put(&mut db, d(2026, 8, 1), vec![log(10, &[(50.0, 10)], None)]);
        put(&mut db, d(2026, 8, 5), vec![log(20, &[(0.0, 60)], None)]);
        // ★ メモだけの日は履歴に出ない。「実施日ではない」の定義が「前回」と揃っている
        put(
            &mut db,
            d(2026, 8, 7),
            vec![noted_log(10, "肩が痛いので飛ばす", &[], None)],
        );

        let got = last_logs_before(&db, e(10), d(2026, 8, 8), 3);
        assert_eq!(
            got.iter().map(|(date, _)| *date).collect::<Vec<_>>(),
            vec![d(2026, 8, 1)],
            "他種目の日とメモだけの日は飛ばす"
        );
    }

    #[test]
    fn last_logs_before_and_last_log_before_agree_on_the_first_entry() {
        // ★ 「前回をコピー」と「重量未入力」の警告はどちらも先頭 1 件しか見ない。
        //   ここがずれると、表示件数という UI 設定がその 2 つの挙動を変えてしまう
        let mut db = test_db();
        put(&mut db, d(2026, 8, 1), vec![log(10, &[(50.0, 10)], None)]);
        put(&mut db, d(2026, 8, 4), vec![log(10, &[(55.0, 10)], None)]);
        put(&mut db, d(2026, 8, 6), vec![log(10, &[(57.5, 10)], None)]);

        let want = last_log_before(&db, e(10), d(2026, 8, 8));
        for limit in 1..=MAX_HISTORY {
            let got = last_logs_before(&db, e(10), d(2026, 8, 8), limit);
            assert_eq!(
                got.first().copied(),
                want,
                "件数 {limit} でも先頭は last_log_before と同じ"
            );
        }
    }

    #[test]
    fn last_logs_before_carries_both_note_kinds() {
        let mut db = test_db();
        put(
            &mut db,
            d(2026, 8, 4),
            vec![noted_log(
                10,
                "フォーム意識",
                &[(60.0, 10, ""), (60.0, 8, "最後潰れた")],
                None,
            )],
        );

        let got = last_logs_before(&db, e(10), d(2026, 8, 8), 3);
        let (_, l) = got.first().expect("8/4 がある");
        assert_eq!(l.note, "フォーム意識", "種目メモが届く");
        assert_eq!(l.sets[1].note, "最後潰れた", "セットメモも届く");
    }

    #[test]
    fn last_logs_before_with_a_zero_limit_yields_nothing() {
        // `limit` は型で 0 を防げないので挙動を固定しておく。
        // 実際に 0 が渡らないことは `history_count_never_returns_zero` が担保する
        let mut db = test_db();
        put(&mut db, d(2026, 8, 4), vec![log(10, &[(55.0, 10)], None)]);
        assert!(last_logs_before(&db, e(10), d(2026, 8, 8), 0).is_empty());
    }

    #[test]
    fn history_count_clamps_out_of_range_values_into_the_allowed_span() {
        assert_eq!(history_count(None), DEFAULT_HISTORY, "未設定は既定");
        assert_eq!(history_count(Some(0)), 1, "0 は下端へ");
        assert_eq!(history_count(Some(-3)), 1, "負も下端へ");
        for n in 1..=MAX_HISTORY {
            assert_eq!(history_count(Some(n as i64)), n, "範囲内はそのまま");
        }
        assert_eq!(history_count(Some(4)), MAX_HISTORY, "上限を超えたら上端へ");
        assert_eq!(history_count(Some(9999)), MAX_HISTORY);
        assert_eq!(history_count(Some(i64::MAX)), MAX_HISTORY, "桁溢れしない");
        assert_eq!(history_count(Some(i64::MIN)), 1);
    }

    #[test]
    fn history_count_never_returns_zero() {
        // ★ 0 が返ると履歴が消えるだけでなく、「前回をコピー」と「重量未入力」の
        //   警告が黙って出なくなる（どちらも先頭 1 件を見る）
        for saved in [
            None,
            Some(i64::MIN),
            Some(-1),
            Some(0),
            Some(1),
            Some(i64::MAX),
        ] {
            let n = history_count(saved);
            assert!(
                (1..=MAX_HISTORY).contains(&n),
                "{saved:?} → {n} が 1..={MAX_HISTORY} の外に出た"
            );
        }
    }

    #[test]
    fn the_default_history_is_the_low_end_of_the_allowed_span() {
        // 既定が下端 = 今までどおり「前回 1 件」で、増やすのは明示的な操作だけ
        assert_eq!(DEFAULT_HISTORY, 1);
        assert_eq!(MAX_HISTORY, 3, "5 件はカードが破裂するので 3 で止める");
    }

    // ── 並び替え ────────────────────────────────────────────────────────────

    /// その日のログの種目 ID を並び順のまま取り出す。
    fn log_order(db: &Db, date: NaiveDate) -> Vec<ExerciseId> {
        db.sessions
            .get(&date_key(date))
            .map(|s| s.logs.iter().map(|l| l.exercise_id).collect())
            .unwrap_or_default()
    }

    #[test]
    fn reorder_logs_follows_the_given_order() {
        let mut db = test_db();
        let day = d(2026, 8, 10);
        put(
            &mut db,
            day,
            vec![
                log(10, &[(60.0, 10)], None),
                log(11, &[(0.0, 20)], None),
                log(20, &[(0.0, 60)], None),
            ],
        );

        assert!(reorder_logs(&mut db, day, &[e(20), e(10), e(11)]));
        assert_eq!(log_order(&db, day), vec![e(20), e(10), e(11)]);
    }

    #[test]
    fn reorder_logs_skips_ids_that_have_no_log_yet() {
        // 「種目を追加」で出しただけで 1 度も commit されていないカード。画面の集合には
        // 居るが `logs` には居ないので、位置を表現しようがない
        let mut db = test_db();
        let day = d(2026, 8, 10);
        put(
            &mut db,
            day,
            vec![log(10, &[(60.0, 10)], None), log(11, &[(0.0, 20)], None)],
        );

        assert!(reorder_logs(&mut db, day, &[e(11), e(20), e(10)]));
        assert_eq!(log_order(&db, day), vec![e(11), e(10)]);
    }

    #[test]
    fn reorder_logs_keeps_logs_missing_from_the_order_at_the_end() {
        // ★ 取り込み（merge_db）は開いている日にもログを増やせる。画面の集合を真実源にして
        //   作り直す実装だと、そのログが黙って消える
        let mut db = test_db();
        let day = d(2026, 8, 10);
        put(
            &mut db,
            day,
            vec![
                log(10, &[(60.0, 10)], None),
                log(11, &[(0.0, 20)], None),
                log(20, &[(0.0, 60)], None),
            ],
        );

        assert!(reorder_logs(&mut db, day, &[e(20)]));
        assert_eq!(
            log_order(&db, day),
            vec![e(20), e(10), e(11)],
            "order に無い 2 本は元の相対順のまま末尾へ"
        );
    }

    #[test]
    fn reorder_logs_never_drops_or_duplicates_a_log() {
        let day = d(2026, 8, 10);
        let seed = vec![
            log(10, &[(60.0, 10)], None),
            log(11, &[(0.0, 20)], None),
            log(20, &[(0.0, 60)], None),
        ];
        for order in [vec![], vec![e(20), e(11), e(10)], vec![e(11)], vec![e(99)]] {
            let mut db = test_db();
            put(&mut db, day, seed.clone());
            reorder_logs(&mut db, day, &order);

            let mut got = log_order(&db, day);
            got.sort_unstable();
            assert_eq!(got, vec![e(10), e(11), e(20)], "order = {order:?}");
        }
    }

    #[test]
    fn reorder_logs_does_not_touch_at_sets_or_notes() {
        // ★ `at` 決定の回帰テスト。カードの並び替えは触っていない他種目を巻き込むので、
        //   ここで now を押すと「並べ替えただけの種目を今やった」という捏造になる
        let mut db = test_db();
        let day = d(2026, 8, 10);
        let before = vec![
            noted_log(
                10,
                "肩に違和感",
                &[(60.0, 10, "1本目キツい"), (60.0, 8, "")],
                Some(1_000_000),
            ),
            noted_log(11, "", &[(0.0, 20, "フォーム意識")], None),
            noted_log(20, "サボり気味", &[], Some(2_000_000)),
        ];
        put(&mut db, day, before.clone());

        reorder_logs(&mut db, day, &[e(20), e(11), e(10)]);

        let after = &db.sessions[&date_key(day)].logs;
        for want in &before {
            let got = after
                .iter()
                .find(|l| l.exercise_id == want.exercise_id)
                .expect("ログは消えない");
            assert_eq!(got, want, "{:?} の中身が動いた", want.exercise_id);
        }
    }

    #[test]
    fn reorder_logs_ignores_duplicate_ids_in_the_order() {
        let mut db = test_db();
        let day = d(2026, 8, 10);
        put(
            &mut db,
            day,
            vec![log(10, &[(60.0, 10)], None), log(11, &[(0.0, 20)], None)],
        );

        assert!(
            !reorder_logs(&mut db, day, &[e(10), e(10), e(11)]),
            "並びは既に order どおりなので変化なし"
        );
        assert_eq!(log_order(&db, day), vec![e(10), e(11)]);
    }

    #[test]
    fn reorder_logs_on_a_missing_date_does_nothing_and_creates_no_session() {
        // ★ entry().or_default() を使うと、記録の無い日が「実施した日」として
        //   カレンダーとバックアップに残る
        let mut db = test_db();
        assert!(!reorder_logs(&mut db, d(2026, 8, 10), &[e(10)]));
        assert!(db.sessions.is_empty(), "空のセッションを作らない");
    }

    #[test]
    fn reorder_logs_reports_whether_it_changed_and_is_idempotent() {
        let mut db = test_db();
        let day = d(2026, 8, 10);
        put(
            &mut db,
            day,
            vec![log(10, &[(60.0, 10)], None), log(11, &[(0.0, 20)], None)],
        );

        let order = [e(11), e(10)];
        assert!(reorder_logs(&mut db, day, &order), "1 回目は変わる");
        assert!(!reorder_logs(&mut db, day, &order), "2 回目は変わらない");
        assert_eq!(log_order(&db, day), vec![e(11), e(10)]);
    }

    #[test]
    fn a_reordered_day_survives_an_export_import_round_trip() {
        // ★ dedupe_logs の「初出順を保つ」が本機能の前提であることを釘付けにする。
        //   ここを並べ替える実装に変えると、次回起動で利用者の並びが黙って戻る
        let mut db = test_db();
        let day = d(2026, 8, 10);
        put(
            &mut db,
            day,
            vec![
                log(10, &[(60.0, 10), (60.0, 8), (60.0, 6)], None),
                log(11, &[(0.0, 20)], None),
                log(20, &[(0.0, 60)], None),
            ],
        );
        reorder_logs(&mut db, day, &[e(20), e(11), e(10)]);
        // セット側も入れ替えておく（views::day の commit が書く形と同じ結果）
        db.sessions
            .get_mut(&date_key(day))
            .expect("その日")
            .logs
            .iter_mut()
            .find(|l| l.exercise_id == e(10))
            .expect("ベンチプレス")
            .sets
            .swap(0, 2);

        let raw = export_json(&db);
        let back = parse_import(&raw, &mut ids(), &Db::default()).expect("読み戻せる");

        assert_eq!(log_order(&back, day), vec![e(20), e(11), e(10)]);
        let sets = &back.sessions[&date_key(day)]
            .logs
            .iter()
            .find(|l| l.exercise_id == e(10))
            .expect("ベンチプレス")
            .sets;
        assert_eq!(
            sets.iter().map(|s| s.reps).collect::<Vec<_>>(),
            vec![6, 8, 10],
            "セットの並びも保たれる"
        );
    }

    #[test]
    fn copy_day_copies_the_reordered_order() {
        let mut db = test_db();
        let from = d(2026, 8, 10);
        put(
            &mut db,
            from,
            vec![
                log(10, &[(60.0, 10)], None),
                log(11, &[(0.0, 20)], None),
                log(20, &[(0.0, 60)], None),
            ],
        );
        reorder_logs(&mut db, from, &[e(20), e(10), e(11)]);

        let copied = copy_day(&mut db, from, d(2026, 8, 11), None);
        assert_eq!(copied, vec![e(20), e(10), e(11)]);
        assert_eq!(log_order(&db, d(2026, 8, 11)), vec![e(20), e(10), e(11)]);
    }

    // ── メニューのコピー ────────────────────────────────────────────────────

    /// 脚に 2 種目足した Db。
    ///
    /// `test_db()` 自体は変えない（脚に種目が無いことに依存しているテストがある）。
    /// 「同じ部位構成・違う種目」を作れないと種目集合キーの検証ができないので、
    /// ここでだけ足す。
    fn menu_db() -> Db {
        let mut db = test_db();
        db.exercises.push(ex(30, "スクワット", 3));
        db.exercises.push(ex(31, "レッグプレス", 3));
        db
    }

    #[test]
    fn recent_menus_returns_newest_first_and_excludes_the_day_itself() {
        let mut db = menu_db();
        put(&mut db, d(2026, 8, 1), vec![log(10, &[(50.0, 10)], None)]);
        put(&mut db, d(2026, 8, 4), vec![log(20, &[(0.0, 60)], None)]);
        put(&mut db, d(2026, 8, 8), vec![log(30, &[(80.0, 5)], None)]);

        let got = recent_menus(&db, d(2026, 8, 8), 4);
        // 8/8 自身は候補にならない（厳密に前）
        assert_eq!(
            got.iter().map(|c| c.date).collect::<Vec<_>>(),
            vec![d(2026, 8, 4), d(2026, 8, 1)]
        );
        assert_eq!(got[0].exercises, vec![e(20)]);

        // limit を超えない
        assert_eq!(recent_menus(&db, d(2026, 8, 9), 2).len(), 2);
        assert!(recent_menus(&db, d(2026, 8, 1), 4).is_empty());
    }

    #[test]
    fn recent_menus_keeps_two_days_with_the_same_groups_but_different_exercises() {
        // ★ 重複排除キーを部位集合にすると落ちるテスト。
        //   A/B 法（同じ脚の日でも種目が違う）が 1 件に潰れると、利用者が却下した
        //   「直前の日をコピー」と同じものに退化する
        let mut db = menu_db();
        put(&mut db, d(2026, 8, 2), vec![log(31, &[(100.0, 10)], None)]);
        put(&mut db, d(2026, 8, 5), vec![log(30, &[(80.0, 5)], None)]);

        let got = recent_menus(&db, d(2026, 8, 8), 4);
        assert_eq!(got.len(), 2, "部位が同じでも種目が違えば別の候補");
        assert_eq!(got[0].exercises, vec![e(30)]);
        assert_eq!(got[1].exercises, vec![e(31)]);
    }

    #[test]
    fn recent_menus_dedupes_days_with_the_same_exercise_set() {
        let mut db = menu_db();
        // 並び順が違うだけの同じ構成。キーはソートするので同一視される
        put(
            &mut db,
            d(2026, 8, 2),
            vec![log(10, &[(50.0, 10)], None), log(11, &[(0.0, 20)], None)],
        );
        put(
            &mut db,
            d(2026, 8, 5),
            vec![log(11, &[(0.0, 15)], None), log(10, &[(60.0, 8)], None)],
        );
        put(&mut db, d(2026, 8, 6), vec![log(30, &[(80.0, 5)], None)]);

        let got = recent_menus(&db, d(2026, 8, 8), 4);
        assert_eq!(
            got.iter().map(|c| c.date).collect::<Vec<_>>(),
            vec![d(2026, 8, 6), d(2026, 8, 5)],
            "同じ種目集合は新しい方だけ残る"
        );
        // 残るのは新しい方の並び順
        assert_eq!(got[1].exercises, vec![e(11), e(10)]);
    }

    #[test]
    fn recent_menus_skips_days_with_nothing_copyable() {
        let mut db = menu_db();
        db.exercises.push(Exercise {
            archived: true,
            ..ex(40, "封印した種目", 1)
        });
        // 空セットだけの日。
        // ★ 8/5 が拾う種目（10）とは**別の種目**にする。同じ 10 にすると、この日は
        //   空セットフィルタではなく 8/5 との重複排除で落ちるので、フィルタを丸ごと
        //   消してもテストが通ってしまう（実際に一度そうなっていた）
        put(&mut db, d(2026, 8, 2), vec![log(11, &[], None)]);
        // 削除済み種目（db.exercises に無い ID）だけの日
        put(&mut db, d(2026, 8, 3), vec![log(99, &[(10.0, 10)], None)]);
        // アーカイブ済み種目だけの日
        put(&mut db, d(2026, 8, 4), vec![log(40, &[(10.0, 10)], None)]);
        // アーカイブ済みが混ざった日は、残りだけが候補になる
        put(
            &mut db,
            d(2026, 8, 5),
            vec![log(40, &[(10.0, 10)], None), log(10, &[(60.0, 8)], None)],
        );

        let got = recent_menus(&db, d(2026, 8, 8), 4);
        assert_eq!(
            got.iter().map(|c| c.date).collect::<Vec<_>>(),
            vec![d(2026, 8, 5)],
            "押しても何も起きない候補を作らない"
        );
        assert_eq!(
            got[0].exercises,
            vec![e(10)],
            "アーカイブ済みは数にも入れない"
        );
    }

    /// ★ 名前どおり「除外する」ことだけを見るテスト。**走査の打ち切り
    /// （`break`）そのものは観測できない** — `break` を `continue` に変えても
    /// 出力は同一になる。打ち切りは速度の話で、外から見える振る舞いではない。
    #[test]
    fn recent_menus_excludes_days_older_than_the_lookback() {
        let mut db = menu_db();
        let before = d(2026, 8, 8);
        let at_day = |n: i64| before - TimeDelta::days(n);
        put(
            &mut db,
            at_day(MENU_LOOKBACK_DAYS - 1),
            vec![log(10, &[(50.0, 10)], None)],
        );
        // ちょうど 180 日前は含む（境界を固定する。`<` を `<=` にすると落ちる）
        put(
            &mut db,
            at_day(MENU_LOOKBACK_DAYS),
            vec![log(20, &[(0.0, 60)], None)],
        );
        put(
            &mut db,
            at_day(MENU_LOOKBACK_DAYS + 1),
            vec![log(30, &[(80.0, 5)], None)],
        );

        let got = recent_menus(&db, before, 4);
        assert_eq!(
            got.iter().map(|c| c.date).collect::<Vec<_>>(),
            vec![at_day(MENU_LOOKBACK_DAYS - 1), at_day(MENU_LOOKBACK_DAYS)],
            "181 日前は落とし、ちょうど 180 日前は残す"
        );
    }

    #[test]
    fn copy_day_duplicates_every_set_in_order() {
        let mut db = menu_db();
        put(
            &mut db,
            d(2026, 8, 5),
            vec![
                log(10, &[(60.0, 10), (60.0, 8)], None),
                log(11, &[(0.0, 20)], None),
            ],
        );

        let copied = copy_day(&mut db, d(2026, 8, 5), d(2026, 8, 8), None);
        assert_eq!(copied, vec![e(10), e(11)]);

        let session = db
            .sessions
            .get(&date_key(d(2026, 8, 8)))
            .expect("できている");
        assert_eq!(
            session
                .logs
                .iter()
                .map(|l| l.exercise_id)
                .collect::<Vec<_>>(),
            vec![e(10), e(11)],
            "元の日のログ順を保つ"
        );
        assert_eq!(session.logs[0].sets, vec![set(60.0, 10), set(60.0, 8)]);
        assert_eq!(session.logs[1].sets, vec![set(0.0, 20)]);
        // 元の日は変わらない
        assert_eq!(
            db.sessions
                .get(&date_key(d(2026, 8, 5)))
                .unwrap()
                .logs
                .len(),
            2
        );
    }

    #[test]
    fn copy_day_always_uses_the_given_at_never_the_source_one() {
        // ★ ExerciseLog を clone すると元の `at` が付いてくる。adr/data-model/at-optional-same-day-only.md の回帰テスト
        let mut db = menu_db();
        put(
            &mut db,
            d(2026, 8, 5),
            vec![log(10, &[(60.0, 10)], Some(1_000_000))],
        );

        copy_day(&mut db, d(2026, 8, 5), d(2026, 8, 8), None);
        let session = db.sessions.get(&date_key(d(2026, 8, 8))).unwrap();
        assert_eq!(
            session.logs[0].at, None,
            "過去日バックフィルは at を持たない"
        );

        copy_day(&mut db, d(2026, 8, 5), d(2026, 8, 9), Some(42));
        let session = db.sessions.get(&date_key(d(2026, 8, 9))).unwrap();
        assert_eq!(session.logs[0].at, Some(42), "当日入力は渡した値をそのまま");
    }

    #[test]
    fn copy_day_with_no_at_keeps_elapsed_in_day_granularity() {
        // 上の観測形。`at` が漏れると「たった今」寄りの Exact になり要件の出力が嘘になる
        let mut db = menu_db();
        let now = 1_800_000_000_000;
        put(
            &mut db,
            d(2026, 8, 1),
            vec![log(10, &[(60.0, 10)], Some(now))],
        );
        copy_day(&mut db, d(2026, 8, 1), d(2026, 8, 6), None);

        let elapsed = elapsed_since_last(&db, now, d(2026, 8, 8)).expect("記録がある");
        assert_eq!(elapsed, Elapsed::days_only(2), "日付キーだけで測る");
        assert_eq!(humanize(elapsed, crate::i18n::Lang::Ja), "2日前");
    }

    #[test]
    fn copy_day_does_nothing_when_the_target_already_has_logs() {
        let mut db = menu_db();
        put(&mut db, d(2026, 8, 5), vec![log(10, &[(60.0, 10)], None)]);
        put(&mut db, d(2026, 8, 8), vec![log(20, &[(0.0, 60)], None)]);
        let before = db.sessions.get(&date_key(d(2026, 8, 8))).cloned();

        assert!(copy_day(&mut db, d(2026, 8, 5), d(2026, 8, 8), None).is_empty());
        assert_eq!(db.sessions.get(&date_key(d(2026, 8, 8))).cloned(), before);
    }

    #[test]
    fn copy_day_refuses_a_target_holding_an_empty_set_log() {
        // 空セットのログが残る旧データに書き足すと exercise_id が重複し、
        // 「1 日 1 種目 1 ログ」（adr/data-model/one-log-per-exercise-per-day.md）が壊れる
        let mut db = menu_db();
        put(&mut db, d(2026, 8, 5), vec![log(10, &[(60.0, 10)], None)]);
        put(&mut db, d(2026, 8, 8), vec![log(10, &[], None)]);

        assert!(copy_day(&mut db, d(2026, 8, 5), d(2026, 8, 8), None).is_empty());
        let logs = &db.sessions.get(&date_key(d(2026, 8, 8))).unwrap().logs;
        assert_eq!(logs.len(), 1, "exercise_id が重複しない");
    }

    #[test]
    fn copy_day_keeps_the_body_weight_and_note_already_on_the_target() {
        let mut db = menu_db();
        put(&mut db, d(2026, 8, 5), vec![log(10, &[(60.0, 10)], None)]);
        db.sessions.insert(
            date_key(d(2026, 8, 8)),
            Session {
                logs: Vec::new(),
                body_weight: Some(70.5),
                note: "よく寝た".into(),
            },
        );

        assert_eq!(
            copy_day(&mut db, d(2026, 8, 5), d(2026, 8, 8), None),
            vec![e(10)]
        );
        let session = db.sessions.get(&date_key(d(2026, 8, 8))).unwrap();
        assert_eq!(session.body_weight, Some(70.5), "体重はコピーで消えない");
        assert_eq!(session.note, "よく寝た");
        assert_eq!(session.logs.len(), 1);
    }

    #[test]
    fn copy_day_does_not_copy_the_source_body_weight_and_note() {
        let mut db = menu_db();
        db.sessions.insert(
            date_key(d(2026, 8, 5)),
            Session {
                logs: vec![log(10, &[(60.0, 10)], None)],
                body_weight: Some(70.5),
                note: "絶好調".into(),
            },
        );

        copy_day(&mut db, d(2026, 8, 5), d(2026, 8, 8), None);
        let session = db.sessions.get(&date_key(d(2026, 8, 8))).unwrap();
        assert_eq!(session.body_weight, None, "その日の観測値は運ばない");
        assert_eq!(session.note, "");
    }

    #[test]
    fn copy_day_copies_the_exercise_note_and_the_set_notes() {
        // ★ **非対称そのものが決定**（adr/ux/copy-carries-the-notes.md）。種目メモと
        //   セットメモは種目に閉じたセッティングなので運び、体重と体調メモは**その日**に
        //   閉じた観測値なので運ばない。1 本のテストで両側を固定する
        let mut db = menu_db();
        db.sessions.insert(
            date_key(d(2026, 8, 5)),
            Session {
                logs: vec![noted_log(
                    10,
                    "セーフティ 2 穴目",
                    &[(60.0, 10, "軽い"), (60.0, 8, "限界")],
                    None,
                )],
                body_weight: Some(70.5),
                note: "よく寝た".into(),
            },
        );

        assert_eq!(
            copy_day(&mut db, d(2026, 8, 5), d(2026, 8, 8), None),
            vec![e(10)]
        );
        let session = db.sessions.get(&date_key(d(2026, 8, 8))).unwrap();
        assert_eq!(session.body_weight, None, "体重は運ばない");
        assert_eq!(session.note, "", "体調メモは運ばない");

        let log = &session.logs[0];
        assert_eq!(log.note, "セーフティ 2 穴目", "種目メモは運ぶ");
        assert_eq!(
            log.sets
                .iter()
                .map(|s| (s.weight, s.reps, s.note.as_str()))
                .collect::<Vec<_>>(),
            vec![(60.0, 10, "軽い"), (60.0, 8, "限界")],
            "セットメモは重量・回数と同じ行に付いたまま運ぶ"
        );
    }

    /// ★ `Seed::carry` は `SetEntry` まで分解して組み直すので、印を運ぶ / 運ばないは
    ///   必ず判断を通る。運ぶと決めたのは、コピーが「過去のログを再現する」操作で、
    ///   しかも印は**メモを閉じていても行に見える**ので違えばその場で外せるから。
    ///   `apply_routine`（メニューから始める）も同じ経路を通るので一緒に固定する。
    #[test]
    fn copy_day_and_apply_routine_carry_the_drop_stages() {
        let mut db = routine_db();
        db.sessions.insert(
            date_key(d(2026, 8, 5)),
            Session {
                logs: vec![ExerciseLog {
                    exercise_id: e(10),
                    sets: vec![set(60.0, 10), drop_set(60.0, 6, &[(50.0, 5)])],
                    label: None,
                    at: None,
                    note: String::new(),
                }],
                ..Session::default()
            },
        );
        let stages = |db: &Db, day: NaiveDate| {
            db.sessions[&date_key(day)].logs[0]
                .sets
                .iter()
                .map(|s| s.drops.len())
                .collect::<Vec<_>>()
        };

        copy_day(&mut db, d(2026, 8, 5), d(2026, 8, 8), None);
        assert_eq!(
            stages(&db, d(2026, 8, 8)),
            vec![0, 1],
            "コピーで段が落ちている"
        );

        // メニューは前回のログ（8/5）を種にするので、段もそこから来る
        apply_routine(&mut db, r(1), d(2026, 8, 9), None);
        assert_eq!(
            stages(&db, d(2026, 8, 9)),
            vec![0, 1],
            "メニューから始めたときに段が落ちている"
        );
    }

    #[test]
    fn copy_day_refuses_a_target_that_only_holds_a_note_only_log() {
        // メモだけのログがある日に書き足すと exercise_id が重複しうる。
        // UI 側はカードが出るので導線が出ないが、判定は logs.is_empty() に寄せている
        let mut db = menu_db();
        put(&mut db, d(2026, 8, 5), vec![log(10, &[(60.0, 10)], None)]);
        put(
            &mut db,
            d(2026, 8, 8),
            vec![noted_log(10, "肩が痛いのでやめた", &[], None)],
        );

        assert!(copy_day(&mut db, d(2026, 8, 5), d(2026, 8, 8), None).is_empty());
        let logs = &db.sessions.get(&date_key(d(2026, 8, 8))).unwrap().logs;
        assert_eq!(logs.len(), 1, "exercise_id が重複しない");
        assert_eq!(logs[0].note, "肩が痛いのでやめた", "メモを消さない");
    }

    #[test]
    fn copy_day_leaves_no_empty_session_when_there_is_nothing_to_copy() {
        let mut db = menu_db();
        db.exercises.push(Exercise {
            archived: true,
            ..ex(40, "封印した種目", 1)
        });
        put(&mut db, d(2026, 8, 5), vec![log(40, &[(10.0, 10)], None)]);

        // コピー元が存在しない
        assert!(copy_day(&mut db, d(2026, 7, 1), d(2026, 8, 8), None).is_empty());
        // コピー元はあるが全種目アーカイブ済み
        assert!(copy_day(&mut db, d(2026, 8, 5), d(2026, 8, 8), None).is_empty());
        assert!(
            !db.sessions.contains_key(&date_key(d(2026, 8, 8))),
            "空の Session を置き去りにしない"
        );
    }

    // ── トレーニングメニューの展開 ──────────────────────────────────────────
    // adr/data-model/routines-as-named-exercise-lists.md
    // adr/ux/start-from-a-saved-routine.md

    /// メニュー入りの Db。胸の日 = ベンチプレス(10) / プランク(20) / スクワット(30)。
    ///
    /// **3 種目とも別の部位**にしてある。「種目ごとに別々の日から引く」を見るテストで
    /// 日付をばらけさせたいので、同じ日にまとまりがちな同一部位を避ける。
    fn routine_db() -> Db {
        let mut db = menu_db();
        db.routines.push(routine(1, "胸の日", &[10, 20, 30]));
        db
    }

    #[test]
    fn usable_routines_drops_exercises_that_are_archived_or_missing() {
        let mut db = routine_db();
        db.exercises
            .iter_mut()
            .find(|x| x.id == e(20))
            .expect("プランクがある")
            .archived = true;
        db.routines[0].exercises.push(e(99)); // 存在しない種目

        let got = usable_routines(&db);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].exercises, vec![e(10), e(30)]);
        assert_eq!(got[0].name, "胸の日");
    }

    #[test]
    fn usable_routines_skips_a_routine_with_nothing_left_to_expand() {
        // 押せない行を作らない（recent_menus が空の日を候補にしないのと同じ）
        let mut db = routine_db();
        db.routines.push(routine(2, "幽霊の日", &[98, 99]));

        let got = usable_routines(&db);
        assert_eq!(
            got.len(),
            1,
            "展開できるものが 1 つも無いメニューは出さない"
        );
        assert_eq!(got[0].id, r(1));
    }

    #[test]
    fn usable_routines_lists_a_routine_even_when_no_exercise_has_history() {
        // ★ 初めて組んだメニューが押せないのは最悪。履歴ゼロでも候補に出す
        let db = routine_db();
        assert_eq!(usable_routines(&db).len(), 1);
    }

    #[test]
    fn usable_routines_dedupes_the_exercises_of_an_unnormalized_routine() {
        // 編集中の Db もここを通る（normalize は読み込みのときしか走らない）
        let mut db = routine_db();
        db.routines[0].exercises = vec![e(10), e(20), e(10)];

        assert_eq!(usable_routines(&db)[0].exercises, vec![e(10), e(20)]);
    }

    #[test]
    fn usable_routines_keeps_the_stored_order() {
        let mut db = routine_db();
        db.routines[0].exercises = vec![e(30), e(10), e(20)];
        db.routines.push(routine(2, "背中の日", &[11]));

        let got = usable_routines(&db);
        assert_eq!(got[0].exercises, vec![e(30), e(10), e(20)]);
        assert_eq!(
            got.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            vec!["胸の日", "背中の日"]
        );
    }

    /// ★ **死んだボタン対策の本丸。** 表示に使う集合と、実際に展開される集合が
    /// 同じであることを直接主張する（`copyable` が 2 つのフィルタを兼ねているのと同じ契約）。
    #[test]
    fn usable_routines_and_apply_routine_agree_on_the_exercises() {
        let mut db = routine_db();
        db.exercises
            .iter_mut()
            .find(|x| x.id == e(20))
            .expect("プランクがある")
            .archived = true;
        db.routines[0].exercises.push(e(99));
        // 一部にだけ履歴を入れる（履歴の有無で集合が変わらないことも同時に見る）
        put(&mut db, d(2026, 8, 5), vec![log(10, &[(60.0, 10)], None)]);

        let listed = usable_routines(&db);
        let opened = apply_routine(&mut db, r(1), d(2026, 8, 8), None);
        assert_eq!(listed[0].exercises, opened);
    }

    #[test]
    fn apply_routine_fills_each_exercise_from_its_own_last_record() {
        // ★ この機能の核。ベンチは 3 日前、スクワットは 10 日前から引く
        let mut db = routine_db();
        put(&mut db, d(2026, 7, 29), vec![log(30, &[(80.0, 5)], None)]);
        put(
            &mut db,
            d(2026, 8, 5),
            vec![log(10, &[(60.0, 10), (60.0, 8)], None)],
        );

        let opened = apply_routine(&mut db, r(1), d(2026, 8, 8), None);
        assert_eq!(opened, vec![e(10), e(20), e(30)], "メニューの並び順で返す");

        let logs = &db.sessions[&date_key(d(2026, 8, 8))].logs;
        assert_eq!(
            logs.iter().map(|l| l.exercise_id).collect::<Vec<_>>(),
            vec![e(10), e(30)],
            "履歴のあるものだけログになる。並びはメニュー順"
        );
        assert_eq!(logs[0].sets, vec![set(60.0, 10), set(60.0, 8)]);
        assert_eq!(logs[1].sets, vec![set(80.0, 5)]);
    }

    #[test]
    fn apply_routine_reports_exercises_without_history_but_writes_no_log() {
        // ★ 0 セットのログを書くと dedupe_logs が次回起動で落とし、
        //   「画面に出ているのに消える」になる。カードは返り値で出す
        let mut db = routine_db();
        put(&mut db, d(2026, 8, 5), vec![log(10, &[(60.0, 10)], None)]);

        let opened = apply_routine(&mut db, r(1), d(2026, 8, 8), None);
        assert_eq!(opened, vec![e(10), e(20), e(30)]);
        assert_eq!(db.sessions[&date_key(d(2026, 8, 8))].logs.len(), 1);
    }

    #[test]
    fn apply_routine_leaves_no_session_when_no_exercise_has_history() {
        let mut db = routine_db();

        let opened = apply_routine(&mut db, r(1), d(2026, 8, 8), None);
        assert_eq!(opened, vec![e(10), e(20), e(30)], "カードは全部出す");
        assert!(
            !db.sessions.contains_key(&date_key(d(2026, 8, 8))),
            "何も記録していない日をカレンダーに残してはいけない"
        );
    }

    #[test]
    fn apply_routine_ignores_records_on_or_after_the_target_day() {
        // 「前回」は指定日より厳密に前（last_log_before と同じ定義）
        let mut db = routine_db();
        put(&mut db, d(2026, 8, 8), vec![log(10, &[(70.0, 5)], None)]);
        put(&mut db, d(2026, 8, 9), vec![log(30, &[(90.0, 3)], None)]);
        put(&mut db, d(2026, 8, 5), vec![log(10, &[(60.0, 10)], None)]);

        let opened = apply_routine(&mut db, r(1), d(2026, 8, 7), None);
        assert_eq!(opened, vec![e(10), e(20), e(30)]);
        let logs = &db.sessions[&date_key(d(2026, 8, 7))].logs;
        assert_eq!(logs.len(), 1, "8/8 と 8/9 は「前回」ではない");
        assert_eq!(logs[0].sets, vec![set(60.0, 10)]);
    }

    #[test]
    fn apply_routine_does_not_use_a_note_only_log_as_a_source() {
        // last_log_before はセットが空のログを飛ばす。メモはトレーニングではない
        let mut db = routine_db();
        put(&mut db, d(2026, 8, 1), vec![log(10, &[(60.0, 10)], None)]);
        put(
            &mut db,
            d(2026, 8, 5),
            vec![noted_log(10, "肩が痛いのでやめた", &[], None)],
        );

        apply_routine(&mut db, r(1), d(2026, 8, 8), None);
        let logs = &db.sessions[&date_key(d(2026, 8, 8))].logs;
        assert_eq!(logs[0].sets, vec![set(60.0, 10)], "8/1 まで遡る");
    }

    #[test]
    fn apply_routine_copies_the_set_notes_and_the_exercise_note() {
        // メニュー展開も copy_day と同じ規則でメモを運ぶ（adr/ux/copy-carries-the-notes.md）
        let mut db = routine_db();
        put(
            &mut db,
            d(2026, 8, 5),
            vec![noted_log(
                10,
                "セーフティ 2 穴目",
                &[(60.0, 10, "重い")],
                None,
            )],
        );

        apply_routine(&mut db, r(1), d(2026, 8, 8), None);
        let log = &db.sessions[&date_key(d(2026, 8, 8))].logs[0];
        assert_eq!(log.note, "セーフティ 2 穴目");
        assert_eq!(log.sets[0].note, "重い");
    }

    #[test]
    fn apply_routine_takes_each_note_from_the_day_that_exercise_came_from() {
        // ★ メニュー展開は種目ごとに**別々の日**から引く。メモも必ずセットと同じログから
        //   来ること — ベンチのセッティングがスクワットのカードに出てはいけない
        let mut db = routine_db();
        put(
            &mut db,
            d(2026, 8, 1),
            vec![noted_log(30, "ベルトあり", &[(80.0, 5, "深く")], None)],
        );
        put(
            &mut db,
            d(2026, 8, 5),
            vec![noted_log(
                10,
                "セーフティ 2 穴目",
                &[(60.0, 10, "軽い")],
                None,
            )],
        );

        apply_routine(&mut db, r(1), d(2026, 8, 8), None);
        let logs = &db.sessions[&date_key(d(2026, 8, 8))].logs;
        // routine(1) は [10, 20, 30]。20 は履歴が無いのでログにならない（空カードで出る）
        assert_eq!(
            logs.iter()
                .map(|l| (l.exercise_id, l.note.as_str(), l.sets[0].note.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (e(10), "セーフティ 2 穴目", "軽い"),
                (e(30), "ベルトあり", "深く"),
            ]
        );
    }

    #[test]
    fn apply_routine_always_uses_the_given_at_never_the_source_one() {
        let mut db = routine_db();
        put(
            &mut db,
            d(2026, 8, 5),
            vec![log(10, &[(60.0, 10)], Some(1))],
        );

        apply_routine(&mut db, r(1), d(2026, 8, 8), None);
        assert_eq!(
            db.sessions[&date_key(d(2026, 8, 8))].logs[0].at,
            None,
            "過去日バックフィルに元の epoch を持ち込まない"
        );

        apply_routine(&mut db, r(1), d(2026, 8, 9), Some(42));
        assert_eq!(db.sessions[&date_key(d(2026, 8, 9))].logs[0].at, Some(42));
    }

    #[test]
    fn apply_routine_does_nothing_when_the_target_already_has_logs() {
        let mut db = routine_db();
        put(&mut db, d(2026, 8, 5), vec![log(10, &[(60.0, 10)], None)]);
        put(&mut db, d(2026, 8, 8), vec![log(11, &[(0.0, 20)], None)]);

        assert!(
            apply_routine(&mut db, r(1), d(2026, 8, 8), None).is_empty(),
            "カードを 1 枚も出さない（出すと画面と Db がずれる）"
        );
        assert_eq!(db.sessions[&date_key(d(2026, 8, 8))].logs.len(), 1);
    }

    #[test]
    fn apply_routine_refuses_a_target_holding_an_empty_set_log() {
        // 判定は is_trained() ではなく logs.is_empty()。緩めると同じ種目のログが 2 本できる
        let mut db = routine_db();
        put(&mut db, d(2026, 8, 5), vec![log(10, &[(60.0, 10)], None)]);
        put(&mut db, d(2026, 8, 8), vec![log(10, &[], None)]);

        assert!(apply_routine(&mut db, r(1), d(2026, 8, 8), None).is_empty());
    }

    #[test]
    fn apply_routine_keeps_the_body_weight_and_note_already_on_the_target() {
        // ConditionRow は 1 文字ごとに commit するので、先に体重が入っていることがある
        let mut db = routine_db();
        put(&mut db, d(2026, 8, 5), vec![log(10, &[(60.0, 10)], None)]);
        let session = db.sessions.entry(date_key(d(2026, 8, 8))).or_default();
        session.body_weight = Some(62.5);
        session.note = "よく寝た".into();

        apply_routine(&mut db, r(1), d(2026, 8, 8), None);
        let session = &db.sessions[&date_key(d(2026, 8, 8))];
        assert_eq!(session.body_weight, Some(62.5));
        assert_eq!(session.note, "よく寝た");
        assert_eq!(session.logs.len(), 1);
    }

    #[test]
    fn apply_routine_never_writes_two_logs_for_the_same_exercise() {
        // 正規化前の Db（編集中）でも「1 日 1 種目 1 ログ」を守る
        let mut db = routine_db();
        db.routines[0].exercises = vec![e(10), e(10), e(10)];
        put(&mut db, d(2026, 8, 5), vec![log(10, &[(60.0, 10)], None)]);

        let opened = apply_routine(&mut db, r(1), d(2026, 8, 8), None);
        assert_eq!(opened, vec![e(10)]);
        assert_eq!(db.sessions[&date_key(d(2026, 8, 8))].logs.len(), 1);
    }

    #[test]
    fn apply_routine_uses_history_older_than_the_menu_lookback() {
        // ★ MENU_LOOKBACK_DAYS は**適用しない**。カードの「前回」表示に上限が無いので、
        //   ここだけ打ち切ると「前回 730日前 60×10 と出ているのに何も入らない」になる
        let mut db = routine_db();
        let old = d(2026, 8, 8) - TimeDelta::days(MENU_LOOKBACK_DAYS + 400);
        put(&mut db, old, vec![log(10, &[(60.0, 10)], None)]);

        apply_routine(&mut db, r(1), d(2026, 8, 8), None);
        assert_eq!(
            db.sessions[&date_key(d(2026, 8, 8))].logs[0].sets,
            vec![set(60.0, 10)],
            "何年前でも「前回」は「前回」"
        );
    }

    #[test]
    fn apply_routine_on_an_unknown_routine_does_nothing() {
        let mut db = routine_db();
        put(&mut db, d(2026, 8, 5), vec![log(10, &[(60.0, 10)], None)]);

        assert!(apply_routine(&mut db, r(99), d(2026, 8, 8), None).is_empty());
        assert!(!db.sessions.contains_key(&date_key(d(2026, 8, 8))));
    }

    // ── 系列 ────────────────────────────────────────────────────────────────

    #[test]
    fn exercise_series_is_within_the_inclusive_range() {
        let mut db = test_db();
        put(&mut db, d(2026, 7, 31), vec![log(10, &[(40.0, 10)], None)]);
        put(&mut db, d(2026, 8, 1), vec![log(10, &[(50.0, 10)], None)]);
        put(
            &mut db,
            d(2026, 8, 8),
            vec![log(10, &[(60.0, 10), (60.0, 8)], None)],
        );
        put(&mut db, d(2026, 8, 9), vec![log(10, &[(70.0, 10)], None)]);

        let series = exercise_series(
            &db,
            e(10),
            Metric::Volume,
            d(2026, 8, 1),
            d(2026, 8, 8),
            Drops::Include,
        );
        assert_eq!(
            series,
            vec![(d(2026, 8, 1), 500.0), (d(2026, 8, 8), 1080.0)]
        );

        // 未知の種目は空
        assert!(
            exercise_series(
                &db,
                e(999),
                Metric::Volume,
                d(2026, 8, 1),
                d(2026, 8, 8),
                Drops::Include
            )
            .is_empty()
        );
        // from > to でもパニックしない
        assert!(
            exercise_series(
                &db,
                e(10),
                Metric::Volume,
                d(2026, 8, 8),
                d(2026, 8, 1),
                Drops::Include
            )
            .is_empty()
        );
    }

    #[test]
    fn exercise_series_switches_axis_with_the_metric() {
        let mut db = test_db();
        put(
            &mut db,
            d(2026, 8, 8),
            vec![log(10, &[(60.0, 10), (60.0, 8)], None)],
        );
        let at = |m| exercise_series(&db, e(10), m, d(2026, 8, 1), d(2026, 8, 8), Drops::Include);

        assert_eq!(at(Metric::Volume), vec![(d(2026, 8, 8), 1080.0)]);
        assert_eq!(at(Metric::Sets), vec![(d(2026, 8, 8), 2.0)]);
        assert_eq!(at(Metric::Reps), vec![(d(2026, 8, 8), 18.0)]);
    }

    #[test]
    fn group_series_sums_every_exercise_in_the_group() {
        let mut db = test_db();
        put(
            &mut db,
            d(2026, 8, 1),
            vec![
                log(10, &[(60.0, 10), (60.0, 8)], None), // ベンチ: 1,080 / 2 セット / 18 回
                log(11, &[(0.0, 12)], None),             // プッシュアップ: 12 / 1 セット / 12 回
            ],
        );
        put(&mut db, d(2026, 8, 2), vec![log(20, &[(0.0, 60)], None)]);

        let chest = |m| group_series(&db, g(1), m, d(2026, 8, 1), d(2026, 8, 2), Drops::Include);

        // ★ 旧実装は Kind ごとに単位が違って足せず「セット数」固定だった。
        //   式が 1 本になったので、重量を使う種目と使わない種目を混ぜて合算できる
        assert_eq!(chest(Metric::Volume), vec![(d(2026, 8, 1), 1092.0)]);
        assert_eq!(chest(Metric::Sets), vec![(d(2026, 8, 1), 3.0)]);
        assert_eq!(chest(Metric::Reps), vec![(d(2026, 8, 1), 30.0)]);

        // 体幹（重量を使わない種目だけ）でも 0 に潰れない
        assert_eq!(
            group_series(
                &db,
                g(2),
                Metric::Volume,
                d(2026, 8, 1),
                d(2026, 8, 2),
                Drops::Include
            ),
            vec![(d(2026, 8, 2), 60.0)]
        );
        // 種目が 1 つも無い部位は空
        assert!(
            group_series(
                &db,
                g(3),
                Metric::Volume,
                d(2026, 8, 1),
                d(2026, 8, 2),
                Drops::Include
            )
            .is_empty()
        );
    }

    #[test]
    fn group_series_skips_days_without_that_group() {
        let mut db = test_db();
        put(&mut db, d(2026, 8, 1), vec![log(20, &[(0.0, 60)], None)]); // 体幹だけの日
        put(&mut db, d(2026, 8, 2), vec![log(10, &[], None)]); // 空セット = 未実施

        // 胸の点は 1 つも立たない（0 の点を置くと「やったが 0」と区別できない）
        assert!(
            group_series(
                &db,
                g(1),
                Metric::Volume,
                d(2026, 8, 1),
                d(2026, 8, 2),
                Drops::Include
            )
            .is_empty()
        );
    }

    // ── used_exercise_ids ───────────────────────────────────────────────────

    #[test]
    fn used_exercise_ids_lists_only_exercises_with_records() {
        let mut db = test_db();
        assert!(used_exercise_ids(&db).is_empty(), "記録が無ければ候補も空");

        put(&mut db, d(2026, 8, 1), vec![log(20, &[(0.0, 60)], None)]);
        put(
            &mut db,
            d(2026, 8, 2),
            vec![
                log(10, &[(60.0, 10)], None),
                log(11, &[], None), // 空セットのログは「使った」に数えない
            ],
        );

        // 並びは db.exercises の順（記録された順ではない）
        assert_eq!(used_exercise_ids(&db), vec![e(10), e(20)]);
    }

    #[test]
    fn used_exercise_ids_ignores_logs_of_deleted_exercises() {
        let mut db = test_db();
        // db.exercises に存在しない ID のログ（通常経路では起きないが、壊れた JSON では有りうる）
        put(&mut db, d(2026, 8, 1), vec![log(999, &[(60.0, 10)], None)]);
        assert!(used_exercise_ids(&db).is_empty());
    }

    // ── 推移タブの対象（Pick / 候補 / 復元） ────────────────────────────────

    /// 3 部位 3 種目に記録を入れた `test_db`。胸(ベンチ/プッシュアップ) と 体幹(プランク)
    /// に記録があり、脚は種目が無いので候補に出ない。
    fn picks_db() -> Db {
        let mut db = test_db();
        put(&mut db, d(2026, 8, 1), vec![log(20, &[(0.0, 60)], None)]);
        put(
            &mut db,
            d(2026, 8, 2),
            vec![log(10, &[(60.0, 10)], None), log(11, &[(0.0, 20)], None)],
        );
        db
    }

    #[test]
    fn progress_candidates_lists_only_exercises_that_have_records() {
        let db = test_db();
        assert!(
            progress_candidates(&db).is_empty(),
            "記録が無ければ候補も空"
        );

        let mut db = db;
        put(&mut db, d(2026, 8, 2), vec![log(10, &[(60.0, 10)], None)]);
        // 空セットのログは「使った」に数えない（used_exercise_ids と同じ規則）
        put(&mut db, d(2026, 8, 3), vec![log(11, &[], None)]);
        assert_eq!(progress_candidates(&db), vec![e(10)]);
    }

    #[test]
    fn progress_candidates_are_ordered_by_group_then_exercise_order() {
        let mut db = picks_db();
        // 胸(order 0) の中では order の小さいプッシュアップが先、体幹(order 1) は後ろ
        db.exercises[0].order = 1; // ベンチプレス
        db.exercises[1].order = 0; // プッシュアップ
        assert_eq!(progress_candidates(&db), vec![e(11), e(10), e(20)]);
    }

    #[test]
    fn progress_candidates_put_archived_exercises_last() {
        let mut db = picks_db();
        // 胸のベンチプレスをアーカイブ。部位の順（胸 → 体幹）より後ろへ回る
        db.exercises[0].archived = true;
        assert_eq!(progress_candidates(&db), vec![e(11), e(20), e(10)]);
    }

    #[test]
    fn progress_candidate_groups_skip_groups_without_records() {
        let db = picks_db();
        // 脚(g3) は種目そのものが無いので出ない
        assert_eq!(progress_candidate_groups(&db), vec![g(1), g(2)]);
        assert!(
            progress_candidate_groups(&test_db()).is_empty(),
            "記録が無ければ部位も空"
        );
    }

    /// アーカイブ済みでも記録があれば部位は選べる。ここが漏れると
    /// 「グラフには合算で出るのに部位がセレクタに無い」食い違いになる。
    #[test]
    fn progress_candidate_groups_count_archived_exercises() {
        let mut db = picks_db();
        db.exercises[2].archived = true; // プランク（体幹の唯一の種目）
        assert!(progress_candidate_groups(&db).contains(&g(2)));
    }

    #[test]
    fn default_pick_is_the_first_candidate_exercise_with_its_group() {
        let db = picks_db();
        assert_eq!(
            default_pick(&db),
            Pick {
                group: Some(g(1)),
                exercise: Some(e(10)),
            }
        );
    }

    #[test]
    fn default_pick_is_empty_when_nothing_is_recorded() {
        let p = default_pick(&test_db());
        assert_eq!(p, Pick::default());
        assert!(!p.is_set(), "両方すべては「選ばれていない」");
    }

    /// 既定値の規則とセレクタの並びが 2 箇所に分かれていないことを固定する。
    #[test]
    fn default_pick_agrees_with_the_head_of_progress_candidates() {
        let mut db = picks_db();
        db.exercises[0].archived = true; // 先頭が入れ替わる状況を作る
        assert_eq!(
            default_pick(&db).exercise,
            progress_candidates(&db).first().copied()
        );
    }

    #[test]
    fn picking_an_exercise_fills_in_the_group_it_belongs_to() {
        let db = picks_db();
        let p = Pick::default().with_exercise(&db, Some(e(20)));
        assert_eq!(p.group, Some(g(2)), "プランクの所属は体幹");
        assert_eq!(p.exercise, Some(e(20)));
    }

    #[test]
    fn picking_an_exercise_from_another_group_moves_the_group() {
        let db = picks_db();
        let p = default_pick(&db).with_exercise(&db, Some(e(20)));
        assert_eq!(
            p,
            Pick {
                group: Some(g(2)),
                exercise: Some(e(20)),
            }
        );
    }

    /// 所属部位が `Db` から消えた種目（`migrate` / `merge_db` が宙に浮いた参照を
    /// 残すので実在しうる）。部位に入れると、どの `<option>` にも当たらず
    /// セレクタが黙って「すべて」へ落ちて `Pick` と表示が食い違う。
    #[test]
    fn picking_an_exercise_whose_group_is_gone_leaves_the_group_unset() {
        let mut db = picks_db();
        db.groups.retain(|x| x.id != g(2)); // 体幹を消す。プランクの group_id が宙に浮く
        let p = Pick::default().with_exercise(&db, Some(e(20)));
        assert_eq!(p.group, None, "実在しない部位は入れない");
        assert_eq!(
            p.exercise,
            Some(e(20)),
            "種目自体は選べる（記録は参照できる）"
        );
    }

    /// 宙に浮いた種目も候補には残す。消すと過去データが推移タブから参照不能になる。
    #[test]
    fn progress_candidates_keep_an_exercise_whose_group_is_gone() {
        let mut db = picks_db();
        db.groups.retain(|x| x.id != g(2));
        assert!(progress_candidates(&db).contains(&e(20)));
        assert!(
            !progress_candidate_groups(&db).contains(&g(2)),
            "部位のほうは実在しないので出さない"
        );
    }

    #[test]
    fn restore_pick_leaves_the_group_unset_for_an_orphaned_exercise() {
        let mut db = picks_db();
        db.groups.retain(|x| x.id != g(2));
        let p = restore_pick(&db, Some(&g(2).to_string()), Some(&e(20).to_string()));
        assert_eq!(
            p,
            Pick {
                group: None,
                exercise: Some(e(20)),
            }
        );
    }

    #[test]
    fn picking_no_exercise_keeps_the_group() {
        let db = picks_db();
        let p = default_pick(&db).with_exercise(&db, None);
        assert_eq!(p.group, Some(g(1)), "部位は残る（その部位の合算になる）");
        assert_eq!(p.exercise, None);
    }

    #[test]
    fn picking_an_unknown_exercise_falls_back_to_no_exercise() {
        let db = picks_db();
        let p = default_pick(&db).with_exercise(&db, Some(e(999)));
        assert_eq!(p.group, Some(g(1)));
        assert_eq!(p.exercise, None);
    }

    #[test]
    fn switching_the_group_keeps_an_exercise_that_belongs_to_it() {
        let db = picks_db();
        let p = default_pick(&db).with_group(&db, Some(g(1)));
        assert_eq!(p.exercise, Some(e(10)), "同じ部位なら種目は残る");
    }

    #[test]
    fn switching_the_group_drops_an_exercise_from_another_group() {
        let db = picks_db();
        let p = default_pick(&db).with_group(&db, Some(g(2)));
        assert_eq!(
            p,
            Pick {
                group: Some(g(2)),
                exercise: None,
            },
            "属さない種目は落ちてその部位の合算になる"
        );
    }

    /// 部位の「すべて」は絞り込みの解除ではなくリセット。2 つのセレクタが
    /// 食い違って見える状態（部位=すべて / 種目=懸垂）を作らない。
    #[test]
    fn switching_the_group_to_all_clears_the_exercise() {
        let db = picks_db();
        let p = default_pick(&db).with_group(&db, None);
        assert_eq!(p, Pick::default());
    }

    #[test]
    fn restore_pick_returns_the_saved_exercise_and_its_group() {
        let db = picks_db();
        let p = restore_pick(&db, Some(&g(2).to_string()), Some(&e(20).to_string()));
        assert_eq!(
            p,
            Pick {
                group: Some(g(2)),
                exercise: Some(e(20)),
            }
        );
    }

    #[test]
    fn restore_pick_keeps_a_group_only_selection() {
        let db = picks_db();
        let p = restore_pick(&db, Some(&g(2).to_string()), None);
        assert_eq!(
            p,
            Pick {
                group: Some(g(2)),
                exercise: None,
            }
        );
    }

    /// 保存後に種目が別の部位へ移されたときは種目に寄せる（部位は種目から一意）。
    #[test]
    fn restore_pick_prefers_the_saved_exercise_over_a_disagreeing_saved_group() {
        let db = picks_db();
        let p = restore_pick(&db, Some(&g(2).to_string()), Some(&e(10).to_string()));
        assert_eq!(p.group, Some(g(1)), "ベンチプレスの所属である胸に寄る");
        assert_eq!(p.exercise, Some(e(10)));
    }

    #[test]
    fn restore_pick_ignores_an_exercise_that_no_longer_has_records() {
        let db = picks_db();
        // e(999) は候補に無い。部位だけが残る
        let p = restore_pick(&db, Some(&g(2).to_string()), Some(&e(999).to_string()));
        assert_eq!(
            p,
            Pick {
                group: Some(g(2)),
                exercise: None,
            }
        );
    }

    #[test]
    fn restore_pick_ignores_a_group_that_no_longer_has_records() {
        let db = picks_db();
        // 脚(g3) は候補に無いので既定へ倒れる
        assert_eq!(
            restore_pick(&db, Some(&g(3).to_string()), None),
            default_pick(&db)
        );
    }

    /// 手で編集された値・schema 2 以前の数値 ID の残骸が来ても既定に落ちるだけ。
    /// ここが `Option<String>` で受ける理由（`Id` の deserialize は落ちる）。
    #[test]
    fn restore_pick_survives_garbage_in_the_saved_values() {
        let db = picks_db();
        for raw in ["", "not-an-id", "7", "zzzzzzzzzzzzz", "0000000000000"] {
            assert_eq!(
                restore_pick(&db, Some(raw), Some(raw)),
                default_pick(&db),
                "壊れた保存値 {raw:?} は既定へ落ちる"
            );
        }
    }

    /// `ids` で書いた値は `restore_pick` でそのまま戻る。保存と復元が
    /// 同じ表現を使っていることを 1 本で固定する。
    #[test]
    fn a_pick_survives_a_round_trip_through_its_saved_ids() {
        let db = picks_db();
        for p in [
            default_pick(&db),
            Pick {
                group: Some(g(2)),
                exercise: None,
            },
            Pick::default().with_exercise(&db, Some(e(20))),
        ] {
            let (g_raw, e_raw) = p.ids();
            assert_eq!(restore_pick(&db, g_raw.as_deref(), e_raw.as_deref()), p);
        }
    }

    #[test]
    fn restore_pick_falls_back_to_the_default_when_nothing_is_saved() {
        let db = picks_db();
        assert_eq!(restore_pick(&db, None, None), default_pick(&db));
    }

    #[test]
    fn pick_series_uses_the_exercise_when_one_is_chosen() {
        let db = picks_db();
        let p = Pick::default().with_exercise(&db, Some(e(10)));
        assert_eq!(
            pick_series(
                &db,
                p,
                Metric::Volume,
                d(2026, 8, 1),
                d(2026, 8, 2),
                Drops::Include
            ),
            // 8/1 はベンチプレスの記録が無い日なので出ない。8/2 は 60kg×10 = 600
            vec![(d(2026, 8, 2), 600.0)],
        );
    }

    /// ★ 段はメインセットにぶら下がるので、「段だけの日」は構造上作れない。
    ///   系列が見るのは日ごとの値が設定に従うことだけ。
    #[test]
    fn series_follow_the_drops_setting() {
        let mut db = test_db();
        put(&mut db, d(2026, 8, 1), vec![log(10, &[(60.0, 10)], None)]);
        db.sessions.insert(
            date_key(d(2026, 8, 2)),
            Session {
                logs: vec![ExerciseLog {
                    exercise_id: e(10),
                    sets: vec![drop_set(60.0, 6, &[(50.0, 5)])],
                    label: None,
                    at: None,
                    note: String::new(),
                }],
                ..Session::default()
            },
        );

        let range = (d(2026, 8, 1), d(2026, 8, 2));
        assert_eq!(
            exercise_series(&db, e(10), Metric::Volume, range.0, range.1, Drops::Exclude),
            vec![(d(2026, 8, 1), 600.0), (d(2026, 8, 2), 360.0)],
            "除く側で段が混ざっている"
        );
        assert_eq!(
            exercise_series(&db, e(10), Metric::Volume, range.0, range.1, Drops::Include),
            vec![(d(2026, 8, 1), 600.0), (d(2026, 8, 2), 610.0)]
        );

        // 部位の枝も同じ
        assert_eq!(
            group_series(&db, g(1), Metric::Sets, range.0, range.1, Drops::Exclude),
            vec![(d(2026, 8, 1), 1.0), (d(2026, 8, 2), 1.0)]
        );
        assert_eq!(
            group_series(&db, g(1), Metric::Sets, range.0, range.1, Drops::Include),
            vec![(d(2026, 8, 1), 1.0), (d(2026, 8, 2), 2.0)]
        );
    }

    /// ★ 外したことを黙ると、記録タブの合計と食い違う理由が画面のどこにも出ない。
    ///   「段が在るか」はデータの事実なので、設定との合成は画面側が持つ。
    #[test]
    fn any_drops_in_scope_answers_only_when_a_stage_is_there() {
        let mut db = test_db();
        put(&mut db, d(2026, 8, 1), vec![log(10, &[(60.0, 10)], None)]);
        let range = (d(2026, 8, 1), d(2026, 8, 2));
        let p = Pick::default().with_exercise(&db, Some(e(10)));

        assert!(
            !any_drops_in_scope(&db, p, range.0, range.1),
            "段が 1 つも無いのに注記を出している"
        );

        db.sessions.insert(
            date_key(d(2026, 8, 2)),
            Session {
                logs: vec![ExerciseLog {
                    exercise_id: e(10),
                    sets: vec![set(60.0, 10), drop_set(60.0, 6, &[(50.0, 5)])],
                    label: None,
                    at: None,
                    note: String::new(),
                }],
                ..Session::default()
            },
        );
        assert!(any_drops_in_scope(&db, p, range.0, range.1));

        // 対象外の種目の段は数えない
        let other = Pick::default().with_exercise(&db, Some(e(20)));
        assert!(!any_drops_in_scope(&db, other, range.0, range.1));
        // 期間外の段も数えない
        assert!(!any_drops_in_scope(&db, p, d(2026, 8, 1), d(2026, 8, 1)));
        // 部位で選んでいても効く
        let grp = Pick {
            group: Some(g(1)),
            exercise: None,
        };
        assert!(any_drops_in_scope(&db, grp, range.0, range.1));
        // 何も選んでいなければ偽
        assert!(!any_drops_in_scope(&db, Pick::default(), range.0, range.1));
    }

    #[test]
    fn pick_series_sums_the_group_when_only_a_group_is_chosen() {
        let db = picks_db();
        let p = Pick {
            group: Some(g(1)),
            exercise: None,
        };
        // 胸は 8/2 にベンチ 60×10 と プッシュアップ 20（重量なし）で 620
        assert_eq!(
            pick_series(
                &db,
                p,
                Metric::Volume,
                d(2026, 8, 1),
                d(2026, 8, 2),
                Drops::Include
            ),
            vec![(d(2026, 8, 2), 620.0)],
        );
    }

    #[test]
    fn pick_series_is_empty_when_nothing_is_chosen() {
        let db = picks_db();
        assert!(
            pick_series(
                &db,
                Pick::default(),
                Metric::Volume,
                d(2026, 8, 1),
                d(2026, 8, 2),
                Drops::Include
            )
            .is_empty()
        );
    }

    // ── aggregate_weekly ────────────────────────────────────────────────────

    #[test]
    fn aggregate_weekly_splits_on_the_sunday_boundary() {
        // 前提: 週の始まりは日曜
        assert_eq!(d(2026, 8, 2).weekday(), Weekday::Sun);
        assert_eq!(d(2026, 8, 8).weekday(), Weekday::Sat);
        assert_eq!(d(2026, 8, 9).weekday(), Weekday::Sun);

        let series = vec![
            (d(2026, 8, 2), 1.0),  // 週 8/2 の初日
            (d(2026, 8, 8), 2.0),  // 週 8/2 の最終日
            (d(2026, 8, 9), 4.0),  // ← ここが週境界。次の週へ
            (d(2026, 8, 15), 8.0), // 週 8/9 の最終日
        ];

        assert_eq!(
            aggregate_weekly(&series),
            vec![(d(2026, 8, 2), 3.0), (d(2026, 8, 9), 12.0)]
        );
    }

    #[test]
    fn aggregate_weekly_sorts_and_handles_empty_input() {
        assert!(aggregate_weekly(&[]).is_empty());

        let unsorted = vec![(d(2026, 8, 15), 8.0), (d(2026, 8, 2), 1.0)];
        assert_eq!(
            aggregate_weekly(&unsorted),
            vec![(d(2026, 8, 2), 1.0), (d(2026, 8, 9), 8.0)]
        );
    }

    #[test]
    fn week_start_is_idempotent_on_sunday() {
        assert_eq!(week_start(d(2026, 8, 9)), d(2026, 8, 9));
        assert_eq!(week_start(d(2026, 8, 15)), d(2026, 8, 9));
        // 月をまたぐ週
        assert_eq!(week_start(d(2026, 8, 1)), d(2026, 7, 26));
    }

    // ── 体重（第2軸）────────────────────────────────────────────────────────

    /// その日に体重（と任意でログ）を置く。
    fn put_weight(db: &mut Db, date: NaiveDate, kg: Option<f32>, logs: Vec<ExerciseLog>) {
        db.sessions.insert(
            date_key(date),
            Session {
                logs,
                body_weight: kg,
                ..Session::default()
            },
        );
    }

    #[test]
    fn body_weight_series_skips_days_without_a_weight() {
        let mut db = test_db();
        put_weight(&mut db, d(2026, 8, 1), Some(70.0), vec![]);
        // ログだけの日は体重の系列に乗らない
        put_weight(
            &mut db,
            d(2026, 8, 2),
            None,
            vec![log(10, &[(60.0, 10)], None)],
        );
        put_weight(&mut db, d(2026, 8, 3), Some(70.5), vec![]);

        assert_eq!(
            body_weight_series(&db, d(2026, 8, 1), d(2026, 8, 3)),
            vec![(d(2026, 8, 1), 70.0), (d(2026, 8, 3), 70.5)]
        );
    }

    /// ★ 指標の系列と違い、**トレーニングしていない日にも点が立つ**。
    ///
    /// これがグラフの X ドメインを両系列の合併にする理由でもある（最後にトレした
    /// 日より後の計量が、そのままでは軸の外に落ちる）。
    #[test]
    fn body_weight_series_includes_rest_days() {
        let mut db = test_db();
        put_weight(
            &mut db,
            d(2026, 8, 1),
            Some(70.0),
            vec![log(10, &[(60.0, 10)], None)],
        );
        put_weight(&mut db, d(2026, 8, 2), Some(70.2), vec![]); // 休養日
        put_weight(&mut db, d(2026, 8, 3), Some(70.4), vec![]); // 休養日

        let weight = body_weight_series(&db, d(2026, 8, 1), d(2026, 8, 3));
        let metric = exercise_series(
            &db,
            e(10),
            Metric::Volume,
            d(2026, 8, 1),
            d(2026, 8, 3),
            Drops::Include,
        );
        assert_eq!(weight.len(), 3);
        assert_eq!(metric.len(), 1);
        // 体重の方が後ろまで伸びる
        assert!(weight.last().expect("空でない").0 > metric.last().expect("空でない").0);
    }

    #[test]
    fn body_weight_series_includes_both_ends_and_is_sorted_by_date() {
        let mut db = test_db();
        for (day, kg) in [(1, 70.0), (5, 71.0), (9, 72.0)] {
            put_weight(&mut db, d(2026, 8, day), Some(kg), vec![]);
        }
        let got = body_weight_series(&db, d(2026, 8, 1), d(2026, 8, 9));
        assert_eq!(
            got,
            vec![
                (d(2026, 8, 1), 70.0),
                (d(2026, 8, 5), 71.0),
                (d(2026, 8, 9), 72.0)
            ]
        );
        // 範囲外は落ちる
        assert_eq!(
            body_weight_series(&db, d(2026, 8, 2), d(2026, 8, 8)),
            vec![(d(2026, 8, 5), 71.0)]
        );
    }

    /// ★ 表示できない値を系列から外す。
    ///
    /// 入力側（`ConditionRow`）も `migrate` も `is_finite() && > 0.0` しか見ていないので、
    /// `3e38` は `f32` として有限であり素通りする。1 点でも混じると `weight_band` の帯が
    /// f64 の丸めで潰れ、NaN 座標で折れ線が丸ごと消える。
    #[test]
    fn body_weight_series_drops_values_the_chart_cannot_represent() {
        let mut db = test_db();
        put_weight(&mut db, d(2026, 8, 1), Some(70.0), vec![]);
        put_weight(&mut db, d(2026, 8, 2), Some(3e38), vec![]); // f32 として有限
        put_weight(&mut db, d(2026, 8, 3), Some(1500.0), vec![]); // WEIGHT_MAX 超
        put_weight(&mut db, d(2026, 8, 4), Some(71.0), vec![]);

        assert_eq!(
            body_weight_series(&db, d(2026, 8, 1), d(2026, 8, 4)),
            vec![(d(2026, 8, 1), 70.0), (d(2026, 8, 4), 71.0)]
        );
    }

    #[test]
    fn aggregate_weekly_avg_averages_instead_of_summing() {
        let series = vec![(d(2026, 8, 2), 70.0), (d(2026, 8, 8), 72.0)];
        // 合計版だと 142（体重として無意味）
        assert_eq!(aggregate_weekly(&series), vec![(d(2026, 8, 2), 142.0)]);
        assert_eq!(aggregate_weekly_avg(&series), vec![(d(2026, 8, 2), 71.0)]);
    }

    /// ★ 指標（合計）と体重（平均）が同じグラフに乗るので、**週キーが一致すること**が要件。
    /// ここがズレると「全期間」で 2 本の線が別の日付軸に並ぶ。
    #[test]
    fn aggregate_weekly_avg_uses_the_same_week_keys_as_aggregate_weekly() {
        let series = vec![
            (d(2026, 8, 2), 70.0),
            (d(2026, 8, 8), 72.0),
            (d(2026, 8, 9), 71.0),
            (d(2026, 8, 15), 73.0),
        ];
        let sum_keys: Vec<_> = aggregate_weekly(&series)
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        let avg_keys: Vec<_> = aggregate_weekly_avg(&series)
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        assert_eq!(sum_keys, avg_keys);
        assert_eq!(sum_keys, vec![d(2026, 8, 2), d(2026, 8, 9)]);
    }

    /// 「全期間」は `progress.rs` が既に週平均を渡すので、描画側の再集約が二重適用にならないこと。
    #[test]
    fn aggregate_weekly_avg_is_idempotent_and_handles_empty_input() {
        assert!(aggregate_weekly_avg(&[]).is_empty());

        // 2 週にまたがる 4 点（1 週 1 点に潰れて自明にならないよう複数週を使う）
        let series = vec![
            (d(2026, 8, 2), 70.0),
            (d(2026, 8, 8), 72.0),
            (d(2026, 8, 9), 71.0),
            (d(2026, 8, 15), 73.0),
        ];
        let weekly = aggregate_weekly_avg(&series);
        assert_eq!(aggregate_weekly_avg(&weekly), weekly);
    }

    // ── weight_band ─────────────────────────────────────────────────────────

    /// 帯が守るべき契約をまとめて確認する。ここを破ると `(v - lo) / (hi - lo)` が
    /// NaN や範囲外になり、SVG の折れ線が黙って消える。
    fn assert_band_contract(lo_v: f64, hi_v: f64) -> (f64, f64) {
        let (lo, hi) = weight_band(lo_v, hi_v);
        assert!(lo.is_finite() && hi.is_finite(), "有限: {lo_v}..{hi_v}");
        assert!(hi > lo, "幅がある: {lo_v}..{hi_v} -> {lo}..{hi}");
        // 目盛り区間が偶数 = 中央ラベルも 0.5 の倍数に乗る
        let ticks = ((hi - lo) / WEIGHT_TICK).round() as i64;
        assert!(ticks >= 2 && ticks % 2 == 0, "区間 {ticks}: {lo}..{hi}");
        (lo, hi)
    }

    #[test]
    fn weight_band_always_contains_the_data() {
        for (lo_v, hi_v) in [
            (70.0, 70.0),
            (60.0, 65.0),
            (62.4, 63.1),
            (0.5, 0.5),
            (61.8, 63.1),
            (65.0, 80.3),
            (999.0, WEIGHT_MAX),
        ] {
            let (lo, hi) = assert_band_contract(lo_v, hi_v);
            assert!(lo <= lo_v, "下端 {lo} <= {lo_v}");
            assert!(hi >= hi_v, "上端 {hi} >= {hi_v}");
        }
    }

    /// 平坦な系列（毎日同じ体重）を下端に貼り付かせない。
    /// 下端は主軸の 0 グリッド線と重なるので、線が軸に化けて見える。
    #[test]
    fn weight_band_centers_a_flat_series() {
        assert_eq!(weight_band(62.0, 62.0), (61.5, 62.5));
        assert_eq!(weight_band(70.0, 70.0), (69.5, 70.5));
    }

    #[test]
    fn weight_band_rounds_to_half_kilograms() {
        // 61.8〜63.1 → 61.5 / 62.5 / 63.5 の 3 ラベル
        assert_eq!(weight_band(61.8, 63.1), (61.5, 63.5));
        // 既に 0.5 の倍数で区間が偶数ならそのまま
        assert_eq!(weight_band(60.0, 65.0), (60.0, 65.0));
    }

    /// ★ 無限ループの退行テスト。
    ///
    /// NaN は `(hi - lo) / TICK` を NaN にし `as i64` が 0 になるので、
    /// ガードが無いと「区間 2 以上かつ偶数」を永久に満たさない。
    /// 巨大値は `as i64` が飽和して `i64::MAX`（奇数）で回り続ける。
    #[test]
    fn weight_band_terminates_on_non_finite_and_absurd_input() {
        for (lo_v, hi_v) in [
            (f64::NAN, f64::NAN),
            (f64::NEG_INFINITY, f64::INFINITY),
            (70.0, f64::NAN),
            (3e38, 3e38),
            (0.0, 3e38),
            (-100.0, -100.0),
        ] {
            assert_band_contract(lo_v, hi_v);
        }
    }

    // ── 経過時間 ────────────────────────────────────────────────────────────

    #[test]
    fn elapsed_since_last_keeps_the_at_but_reports_calendar_days() {
        let mut db = test_db();
        let at = 1_800_000_000_000;
        put(
            &mut db,
            d(2026, 8, 6),
            vec![log(10, &[(60.0, 10)], Some(at))],
        );

        let now = at + 2 * DAY_MS + 5 * HOUR_MS;
        let e = elapsed_since_last(&db, now, d(2026, 8, 8)).expect("記録がある");
        assert_eq!(e, Elapsed::with_ms(2, 2 * DAY_MS + 5 * HOUR_MS));
        assert_eq!(e.days(), 2, "日数は日付キーから出す");
        assert_eq!(humanize(e, crate::i18n::Lang::Ja), "2日前");
    }

    #[test]
    fn elapsed_since_last_works_without_any_at() {
        let mut db = test_db();
        // 8/8 に 8/7 分をバックフィルした状態（at は入らない）
        put(&mut db, d(2026, 8, 7), vec![log(10, &[(60.0, 10)], None)]);

        let e = elapsed_since_last(&db, 1_800_000_000_000, d(2026, 8, 8)).expect("記録がある");
        assert_eq!(e, Elapsed::days_only(1));
        assert_eq!(humanize(e, crate::i18n::Lang::Ja), "昨日");
    }

    #[test]
    fn elapsed_since_last_prefers_at_when_the_session_mixes_some_and_none() {
        let mut db = test_db();
        let at = 1_800_000_000_000;
        put(
            &mut db,
            d(2026, 8, 8),
            vec![
                log(10, &[(60.0, 10)], None),
                log(11, &[(0.0, 12)], Some(at)),
            ],
        );

        let e = elapsed_since_last(&db, at + 3 * HOUR_MS, d(2026, 8, 8)).expect("記録がある");
        assert_eq!(e, Elapsed::with_ms(0, 3 * HOUR_MS));
        // 同じ暦日なので時刻粒度まで出る
        assert_eq!(humanize(e, crate::i18n::Lang::Ja), "3時間");
    }

    #[test]
    fn elapsed_since_last_uses_the_largest_at_in_the_session() {
        let mut db = test_db();
        let first = 1_800_000_000_000;
        let last = first + HOUR_MS;
        put(
            &mut db,
            d(2026, 8, 8),
            vec![
                log(10, &[(60.0, 10)], Some(first)),
                log(11, &[(0.0, 12)], Some(last)),
            ],
        );

        let e = elapsed_since_last(&db, last + 30 * 60_000, d(2026, 8, 8)).expect("記録がある");
        assert_eq!(e, Elapsed::with_ms(0, 30 * 60_000));
        assert_eq!(humanize(e, crate::i18n::Lang::Ja), "30分");
    }

    #[test]
    fn elapsed_since_last_skips_sessions_without_training_and_ignores_the_future() {
        let mut db = test_db();
        put(&mut db, d(2026, 8, 3), vec![log(10, &[(60.0, 10)], None)]);
        put(&mut db, d(2026, 8, 5), vec![log(10, &[], None)]); // 空セット = 未実施
        db.sessions.insert(
            date_key(d(2026, 8, 6)), // 体重とメモだけの日も未実施
            Session {
                logs: vec![],
                body_weight: Some(70.0),
                note: "疲労".into(),
            },
        );
        put(&mut db, d(2026, 8, 20), vec![log(10, &[(60.0, 10)], None)]); // 未来日

        let e = elapsed_since_last(&db, 1_800_000_000_000, d(2026, 8, 8)).expect("8/3 がある");
        assert_eq!(e, Elapsed::days_only(5));

        assert_eq!(
            elapsed_since_last(&test_db(), 1_800_000_000_000, d(2026, 8, 8)),
            None
        );
    }

    #[test]
    fn elapsed_by_group_is_per_group_and_omits_untrained_groups() {
        let mut db = test_db();
        let at = 1_800_000_000_000;
        put(&mut db, d(2026, 8, 1), vec![log(20, &[(0.0, 60)], None)]);
        put(
            &mut db,
            d(2026, 8, 6),
            vec![log(10, &[(60.0, 10)], Some(at))],
        );

        let by_group = elapsed_by_group(&db, at + 2 * DAY_MS + 5 * HOUR_MS, d(2026, 8, 8));

        assert_eq!(
            by_group.get(&g(1)),
            Some(&Elapsed::with_ms(2, 2 * DAY_MS + 5 * HOUR_MS))
        );
        assert_eq!(by_group.get(&g(2)), Some(&Elapsed::days_only(7)));
        // 種目が無い部位・未実施の部位はキーごと出ない（画面が「—」を出す）
        assert_eq!(by_group.get(&g(3)), None);
        assert_eq!(by_group.len(), 2);
    }

    // ★ 以下 4 本は「経過日数はローカル暦の日差であって経過ミリ秒 / 24h ではない」ことを
    //   固定する。旧実装は `Exact(ms)` しか持たず、日数が必要な views 側が
    //   `ms / 86_400_000` を書いていたため、繰り上がりが暦の 0 時ではなくトレーニング
    //   時刻の 24 時間後に起きていた（adr/data-model/elapsed-in-local-calendar-days.md）。
    //
    //   旧テストがこれを捕まえられなかったのは、8/6 → 8/8 という「暦の日差 2」と
    //   「ms / 86_400_000 = 2」が偶然一致する組み合わせしか使っていなかったから。
    //   日を跨ぐのに 24 時間未満、というケースを必ず含めること。

    #[test]
    fn elapsed_reports_calendar_days_even_when_at_is_within_24_hours() {
        let mut db = test_db();
        let at = 1_800_000_000_000; // 8/8 20:00 のつもり
        put(
            &mut db,
            d(2026, 8, 8),
            vec![log(10, &[(60.0, 10)], Some(at))],
        );

        // 翌朝に見る。経過は 12 時間だが暦では 1 日
        let e = elapsed_since_last(&db, at + 12 * HOUR_MS, d(2026, 8, 9)).expect("記録がある");
        assert_eq!(e.days(), 1, "24 時間で割ったローリング日数にしない");
        assert_eq!(humanize(e, crate::i18n::Lang::Ja), "昨日");
        assert_eq!(short_elapsed(e, crate::i18n::Lang::Ja), "1d");
        assert_eq!(recency_class(Some(e)), "fresh");
    }

    #[test]
    fn elapsed_reports_yesterday_right_after_midnight() {
        let mut db = test_db();
        let at = 1_800_000_000_000; // 8/8 23:50 のつもり
        put(
            &mut db,
            d(2026, 8, 8),
            vec![log(10, &[(60.0, 10)], Some(at))],
        );

        // 経過 30 分でも日を跨いでいれば「昨日」。暦日セマンティクスの対称コスト
        let e = elapsed_since_last(&db, at + 30 * 60_000, d(2026, 8, 9)).expect("記録がある");
        assert_eq!(humanize(e, crate::i18n::Lang::Ja), "昨日");
    }

    #[test]
    fn elapsed_by_group_reports_calendar_days_for_at_bearing_logs() {
        let mut db = test_db();
        let at = 1_800_000_000_000;
        put(
            &mut db,
            d(2026, 8, 8),
            vec![log(10, &[(60.0, 10)], Some(at))],
        ); // 胸 = g(1)
        put(&mut db, d(2026, 8, 6), vec![log(20, &[(0.0, 60)], None)]); // 体幹 = g(2)

        let by_group = elapsed_by_group(&db, at + 12 * HOUR_MS, d(2026, 8, 9));
        assert_eq!(by_group.get(&g(1)).map(|e| e.days()), Some(1));
        assert_eq!(by_group.get(&g(2)).map(|e| e.days()), Some(3));
        assert_eq!(short_elapsed(by_group[&g(1)], crate::i18n::Lang::Ja), "1d");
    }

    #[test]
    fn hero_and_chip_agree_on_the_day_count() {
        // ヒーロー（humanize）とチップ（short_elapsed）が違う日を指してはいけない
        for (e, want_humanize, want_short) in [
            (Elapsed::with_ms(1, 12 * HOUR_MS), "昨日", "1d"),
            (Elapsed::with_ms(2, 36 * HOUR_MS), "2日前", "2d"),
            (Elapsed::days_only(3), "3日前", "3d"),
        ] {
            assert_eq!(humanize(e, crate::i18n::Lang::Ja), want_humanize);
            assert_eq!(short_elapsed(e, crate::i18n::Lang::Ja), want_short);
        }
    }

    // ── humanize ────────────────────────────────────────────────────────────

    #[test]
    fn humanize_covers_every_granularity() {
        // 同じ暦日 = 時刻粒度
        assert_eq!(
            humanize(Elapsed::with_ms(0, 45 * 60_000), crate::i18n::Lang::Ja),
            "45分"
        );
        assert_eq!(
            humanize(
                Elapsed::with_ms(0, 59 * 60_000 + 59_999),
                crate::i18n::Lang::Ja
            ),
            "59分"
        );
        assert_eq!(
            humanize(Elapsed::with_ms(0, HOUR_MS), crate::i18n::Lang::Ja),
            "1時間"
        );
        assert_eq!(
            humanize(
                Elapsed::with_ms(0, 23 * HOUR_MS + 59 * 60_000),
                crate::i18n::Lang::Ja
            ),
            "23時間"
        );
        // at を持たない当日の記録（取り込んだデータ）は日粒度に落ちる
        assert_eq!(
            humanize(Elapsed::days_only(0), crate::i18n::Lang::Ja),
            "今日"
        );

        // ★ 日を跨いだら必ず日粒度。「2日5時間」形式は廃止した
        assert_eq!(
            humanize(Elapsed::with_ms(1, 12 * HOUR_MS), crate::i18n::Lang::Ja),
            "昨日"
        );
        assert_eq!(
            humanize(
                Elapsed::with_ms(2, 2 * DAY_MS + 5 * HOUR_MS),
                crate::i18n::Lang::Ja
            ),
            "2日前"
        );
        assert_eq!(
            humanize(Elapsed::with_ms(2, 2 * DAY_MS), crate::i18n::Lang::Ja),
            "2日前"
        );
        assert_eq!(
            humanize(Elapsed::days_only(1), crate::i18n::Lang::Ja),
            "昨日"
        );
        assert_eq!(
            humanize(Elapsed::days_only(5), crate::i18n::Lang::Ja),
            "5日前"
        );
    }

    #[test]
    fn humanize_rounds_sub_minute_down_to_just_now() {
        assert_eq!(
            humanize(Elapsed::with_ms(0, 0), crate::i18n::Lang::Ja),
            "たった今"
        );
        assert_eq!(
            humanize(Elapsed::with_ms(0, 30_000), crate::i18n::Lang::Ja),
            "たった今"
        );
    }

    /// 端末時計が記録時刻より巻き戻っていても、`Elapsed` は負の ms/days を持たない
    /// （`#[cfg(test)]` コンストラクタの `.max(0)` ではなく `Elapsed::since` 本体のクランプを見る）。
    #[test]
    fn elapsed_since_last_clamps_when_the_clock_runs_behind_the_record() {
        let mut db = test_db();
        let at = 1_800_000_000_000;
        put(
            &mut db,
            d(2026, 8, 8),
            vec![log(10, &[(60.0, 10)], Some(at))],
        );

        let e = elapsed_since_last(&db, at - 5 * HOUR_MS, d(2026, 8, 8));
        assert_eq!(e, Some(Elapsed::with_ms(0, 0)));
    }

    #[test]
    fn humanize_falls_back_to_the_date_key_when_at_contradicts_it() {
        // 同じ暦日なのに 2 週間分の経過 = 壊れたデータ。「336時間」を出さず日付キーを勝たせる
        assert_eq!(
            humanize(Elapsed::with_ms(0, 14 * DAY_MS), crate::i18n::Lang::Ja),
            "今日"
        );
    }

    #[test]
    fn short_elapsed_and_recency_use_calendar_days() {
        assert_eq!(
            short_elapsed(Elapsed::days_only(0), crate::i18n::Lang::Ja),
            "今日"
        );
        assert_eq!(
            short_elapsed(Elapsed::with_ms(1, 12 * HOUR_MS), crate::i18n::Lang::Ja),
            "1d"
        );
        assert_eq!(
            short_elapsed(Elapsed::days_only(10), crate::i18n::Lang::Ja),
            "10d"
        );

        assert_eq!(recency_class(None), "none");
        assert_eq!(recency_class(Some(Elapsed::days_only(0))), "fresh");
        assert_eq!(
            recency_class(Some(Elapsed::with_ms(1, 12 * HOUR_MS))),
            "fresh"
        );
        assert_eq!(
            recency_class(Some(Elapsed::with_ms(2, 36 * HOUR_MS))),
            "recent"
        );
        assert_eq!(recency_class(Some(Elapsed::days_only(3))), "recent");
        assert_eq!(recency_class(Some(Elapsed::days_only(4))), "stale");
        assert_eq!(recency_class(Some(Elapsed::days_only(6))), "stale");
        assert_eq!(recency_class(Some(Elapsed::days_only(7))), "old");
    }

    // ── unseen_releases / latest_release_id ─────────────────────────────────

    /// 生値の異常系をここで全部潰す。`UiState.release_seen` は `Option<i64>` で
    /// 範囲を絞らずに受けているので、ここが唯一の検証地点になる。
    #[test]
    fn unseen_releases_handles_every_raw_last_seen_value() {
        // 新しい順（id は 5..=1）。本文はテストに関係ないのでダミーで揃える
        let notes: Vec<ReleaseNote> = (1..=5)
            .rev()
            .map(|id| ReleaseNote {
                id,
                date: "2026-01-01",
                ja: &["x"],
                en: &["x"],
            })
            .collect();

        assert_eq!(unseen_releases(&notes, None).len(), 0, "未設定は空");
        assert_eq!(unseen_releases(&notes, Some(5)).len(), 0, "最新と同値は空");
        assert_eq!(
            unseen_releases(&notes, Some(4)).len(),
            1,
            "1 つ古いなら 1 件"
        );
        assert_eq!(
            unseen_releases(&notes, Some(2)).len(),
            3,
            "3 つ古いなら 3 件"
        );
        assert_eq!(unseen_releases(&notes, Some(-1)).len(), 5, "負値は全件");
        assert_eq!(
            unseen_releases(&notes, Some(i64::MAX)).len(),
            0,
            "桁溢れしない"
        );
        assert_eq!(
            unseen_releases(&notes, Some(100)).len(),
            0,
            "未来の番号（最新より大きい）は空"
        );
        assert_eq!(unseen_releases(&[], Some(0)).len(), 0, "notes が空なら空");
    }

    #[test]
    fn latest_release_id_is_the_first_entry_or_none() {
        let notes = [ReleaseNote {
            id: 7,
            date: "2026-01-01",
            ja: &["x"],
            en: &["x"],
        }];
        assert_eq!(latest_release_id(&notes), Some(7));
        assert_eq!(latest_release_id(&[]), None);
    }

    // ── migrate ─────────────────────────────────────────────────────────────

    #[test]
    fn migrate_returns_err_for_broken_json() {
        // 呼び側はこの Err を見て raw を .bak-<epoch> に退避する
        assert!(migrate("", &mut ids()).is_err());
        assert!(migrate("{壊れている", &mut ids()).is_err());
        assert!(migrate("null", &mut ids()).is_err());
        assert!(migrate("[1,2,3]", &mut ids()).is_err());
        // 形は JSON でも必須フィールドが欠けていれば Err（プリセットで黙って上書きしない）
        assert!(migrate(r#"{"schema":1}"#, &mut ids()).is_err());
        assert!(
            migrate(
                r#"{"schema":1,"next_id":1,"groups":[],"exercises":[]}"#,
                &mut ids()
            )
            .is_err()
        );
    }

    #[test]
    fn migrate_merges_duplicate_logs_of_the_same_exercise() {
        let raw = r#"{
          "schema": 1, "next_id": 100, "groups": [], "exercises": [],
          "sessions": {
            "2026-08-08": { "logs": [
              {"exercise_id": 10, "sets": [{"weight": 60.0, "reps": 10}], "at": 111},
              {"exercise_id": 20, "sets": [{"weight": 0.0, "reps": 60}]},
              {"exercise_id": 10, "sets": [{"weight": 60.0, "reps": 8}], "at": 222}
            ]}
          }
        }"#;

        let db = migrate(raw, &mut ids()).expect("正当な JSON");
        let session = &db.sessions["2026-08-08"];

        assert_eq!(
            session.logs.len(),
            2,
            "同一 exercise_id は 1 ログに畳まれる"
        );
        // 初出の順序が保たれる（ID は乱数に張り替わるので、別物であることだけ見る。
        // どちらがどちらかはこの下のセット内容で確定する）
        assert_ne!(session.logs[0].exercise_id, session.logs[1].exercise_id);
        // セットは出現順に連結
        assert_eq!(
            session.logs[0].sets,
            vec![
                SetEntry {
                    weight: 60.0,
                    reps: 10,
                    ..Default::default()
                },
                SetEntry {
                    weight: 60.0,
                    reps: 8,
                    ..Default::default()
                }
            ]
        );
        // at は Some の最大値
        assert_eq!(session.logs[0].at, Some(222));
        assert_eq!(session.logs[1].at, None);
        // 正規化後は last_log_before が単一ログを返せる
        assert_eq!(log_value(Metric::Volume, &session.logs[0]), 1080.0);
    }

    #[test]
    fn migrate_keeps_at_when_only_one_duplicate_has_it() {
        let raw = r#"{
          "schema": 1, "next_id": 100, "groups": [], "exercises": [],
          "sessions": {
            "2026-08-08": { "logs": [
              {"exercise_id": 10, "sets": [{"weight": 60.0, "reps": 10}], "at": 999},
              {"exercise_id": 10, "sets": [{"weight": 60.0, "reps": 8}]}
            ]}
          }
        }"#;

        let db = migrate(raw, &mut ids()).expect("正当な JSON");
        assert_eq!(db.sessions["2026-08-08"].logs[0].at, Some(999));
    }

    #[test]
    fn migrate_normalizes_date_keys_and_merges_the_collisions() {
        // ゼロ埋めされていないキーを残すと「辞書順 = 時系列順」が壊れる
        let raw = r#"{
          "schema": 1, "next_id": 100, "groups": [], "exercises": [],
          "sessions": {
            "2026-8-3":    { "logs": [{"exercise_id": 10, "sets": [{"weight": 60.0, "reps": 10}]}] },
            "2026-08-03":  { "logs": [{"exercise_id": 10, "sets": [{"weight": 60.0, "reps": 8}]}] },
            "こわれた":     { "logs": [{"exercise_id": 10, "sets": [{"weight": 60.0, "reps": 5}]}] }
          }
        }"#;

        let db = migrate(raw, &mut ids()).expect("正当な JSON");

        assert_eq!(db.sessions.keys().collect::<Vec<_>>(), vec!["2026-08-03"]);
        assert_eq!(db.sessions["2026-08-03"].logs.len(), 1);
        assert_eq!(db.sessions["2026-08-03"].logs[0].sets.len(), 2);
    }

    #[test]
    fn migrate_drops_only_completely_empty_sessions() {
        let raw = r#"{
          "schema": 1, "next_id": 100, "groups": [], "exercises": [],
          "sessions": {
            "2026-08-01": { "logs": [] },
            "2026-08-02": { "logs": [{"exercise_id": 10, "sets": []}] },
            "2026-08-03": { "logs": [], "body_weight": 70.5 },
            "2026-08-04": { "logs": [], "note": "睡眠不足" }
          }
        }"#;

        let db = migrate(raw, &mut ids()).expect("正当な JSON");

        // 閲覧しただけの空セッションは消える
        assert_eq!(
            db.sessions.keys().collect::<Vec<_>>(),
            vec!["2026-08-03", "2026-08-04"]
        );
        // 体重・メモだけの日は残る（実施日ではない）
        assert_eq!(db.sessions["2026-08-03"].body_weight, Some(70.5));
        assert!(!db.sessions["2026-08-03"].is_trained());
        assert_eq!(db.sessions["2026-08-04"].note, "睡眠不足");
    }

    #[test]
    fn migrate_keeps_a_log_that_only_has_a_note() {
        // ★ ここが `!l.sets.is_empty()` に戻ると、画面に出ている種目メモが
        //   次回起動で消える（adr/data-model/notes-on-logs-and-sets.md）
        let raw = r#"{
          "schema": 3, "groups": [], "exercises": [],
          "sessions": {
            "2026-08-08": { "logs": [
              {"exercise_id": "00000000000a", "sets": [], "note": "肩が痛いのでやめた"}
            ]}
          }
        }"#;

        let db = migrate(raw, &mut ids()).expect("正当な JSON");

        let session = &db.sessions["2026-08-08"];
        assert_eq!(session.logs.len(), 1, "メモだけのログを捨ててはいけない");
        assert_eq!(session.logs[0].note, "肩が痛いのでやめた");
        assert!(
            !session.is_trained(),
            "メモだけの日を実施日にしてはいけない"
        );
    }

    #[test]
    fn migrate_drops_a_log_with_neither_sets_nor_a_note() {
        let raw = r#"{
          "schema": 3, "groups": [], "exercises": [],
          "sessions": {
            "2026-08-08": { "logs": [
              {"exercise_id": "00000000000a", "sets": []},
              {"exercise_id": "00000000000b", "sets": [], "note": "  "}
            ], "body_weight": 70.0 }
          }
        }"#;

        let db = migrate(raw, &mut ids()).expect("正当な JSON");

        assert!(
            db.sessions["2026-08-08"].logs.is_empty(),
            "空白だけのメモは「無い」"
        );
    }

    #[test]
    fn migrate_clears_a_whitespace_only_note_at_every_level() {
        // 空白が残ると skip_serializing_if をすり抜けて保存され、
        // is_empty()（trim する）と JSON の見え方がずれる
        let raw = r#"{
          "schema": 3, "groups": [], "exercises": [],
          "sessions": {
            "2026-08-08": {
              "logs": [{
                "exercise_id": "00000000000a",
                "sets": [{"weight": 60.0, "reps": 10, "note": "\n"}],
                "note": "　"
              }],
              "note": " "
            }
          }
        }"#;

        let db = migrate(raw, &mut ids()).expect("正当な JSON");

        let session = &db.sessions["2026-08-08"];
        assert_eq!(session.note, "");
        assert_eq!(session.logs[0].note, "");
        assert_eq!(session.logs[0].sets[0].note, "");
        // ★ 新しく足した 2 つ（ログ・セット）の空メモは JSON に出ない。`Session.note` は
        //   `skip_serializing_if` を持たず昔から `"note":""` を書いているので、そこは
        //   数えない（既存の保存形式を変えないため意図的にそのまま）
        let json = export_json(&db);
        assert_eq!(
            json.matches("\"note\"").count(),
            1,
            "ログ / セットの空メモが書き出されている: {json}"
        );
    }

    #[test]
    fn migrate_merges_the_notes_of_duplicate_logs() {
        let raw = r#"{
          "schema": 3, "groups": [], "exercises": [],
          "sessions": {
            "2026-08-08": { "logs": [
              {"exercise_id": "00000000000a", "sets": [{"weight": 60.0, "reps": 10}], "note": "A"},
              {"exercise_id": "00000000000a", "sets": [{"weight": 60.0, "reps": 8}], "note": "B"}
            ]}
          }
        }"#;

        let db = migrate(raw, &mut ids()).expect("正当な JSON");

        let logs = &db.sessions["2026-08-08"].logs;
        assert_eq!(logs.len(), 1, "1 日 1 種目 1 ログ");
        assert_eq!(logs[0].note, "A\nB", "どちらのメモも失わない");
        assert_eq!(logs[0].sets.len(), 2, "セットは連結される");
    }

    #[test]
    fn migrate_does_not_duplicate_an_identical_note_of_duplicate_logs() {
        let raw = r#"{
          "schema": 3, "groups": [], "exercises": [],
          "sessions": {
            "2026-08-08": { "logs": [
              {"exercise_id": "00000000000a", "sets": [{"weight": 60.0, "reps": 10}], "note": "重い"},
              {"exercise_id": "00000000000a", "sets": [{"weight": 60.0, "reps": 8}], "note": "重い"}
            ]}
          }
        }"#;

        let db = migrate(raw, &mut ids()).expect("正当な JSON");

        assert_eq!(db.sessions["2026-08-08"].logs[0].note, "重い");
    }

    #[test]
    fn migrate_from_schema_two_leaves_every_note_blank() {
        // legacy が現行の SetEntry を再利用しているので、note が default で読めることを固定する
        let raw = r##"{
          "schema": 2, "next_id": 1,
          "groups": [{"id": 3, "name": "胸", "color": "#e0524a", "order": 0}],
          "exercises": [{"id": 42, "name": "わたしの種目", "group_id": 3, "order": 0}],
          "sessions": {
            "2026-08-08": { "logs": [
              {"exercise_id": 42, "sets": [{"weight": 60.0, "reps": 10}]}
            ]}
          }
        }"##;

        let db = migrate(raw, &mut ids()).expect("正当な JSON");

        let log = &db.sessions["2026-08-08"].logs[0];
        assert_eq!(log.note, "");
        assert_eq!(log.sets[0].note, "");
    }

    #[test]
    fn dropping_an_unrepresentable_weight_drops_that_sets_note_too() {
        // メモはセットの付属物なので一緒に消えるのが正しい。無言の欠落なので固定しておく
        let raw = r#"{
          "schema": 3, "groups": [], "exercises": [],
          "sessions": {
            "2026-08-08": { "logs": [{
              "exercise_id": "00000000000a",
              "sets": [
                {"weight": 3.5e38, "reps": 10, "note": "壊れた重量"},
                {"weight": 60.0, "reps": 8, "note": "残る"}
              ]
            }]}
          }
        }"#;

        let db = migrate(raw, &mut ids()).expect("正当な JSON");

        let sets = &db.sessions["2026-08-08"].logs[0].sets;
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].note, "残る");
    }

    #[test]
    fn migrate_round_trips_notes_at_every_level() {
        let mut db = menu_db();
        db.sessions.insert(
            date_key(d(2026, 8, 8)),
            Session {
                logs: vec![noted_log(
                    10,
                    "フォームが崩れた",
                    &[(60.0, 10, "軽い"), (60.0, 8, "肩に違和感")],
                    Some(1_800_000_000_000),
                )],
                body_weight: Some(70.5),
                note: "睡眠不足".into(),
            },
        );

        let again = migrate(&export_json(&db), &mut ids()).expect("自分が書いた JSON");
        assert_eq!(again, db);
    }

    #[test]
    fn same_sets_ignores_the_notes() {
        let a = vec![
            SetEntry {
                weight: 60.0,
                reps: 10,
                note: "きつい".into(),
                ..Default::default()
            },
            set(60.0, 8),
        ];
        let b = vec![set(60.0, 10), set(60.0, 8)];
        assert!(same_sets(&a, &b), "メモの違いで不一致にしてはいけない");
        assert_ne!(a, b, "== はメモを見る（だから same_sets が要る）");

        assert!(!same_sets(&a, &[set(60.0, 10)]), "長さが違う");
        assert!(!same_sets(&a, &[set(62.0, 10), set(60.0, 8)]), "重量が違う");
        assert!(!same_sets(&a, &[set(60.0, 10), set(60.0, 6)]), "回数が違う");
    }

    /// ★ 印を識別に入れると、印の差だけで `merge_db` が食い違い扱いになり、
    ///   rank が同点なので差し替えの分岐にも入れず、負けた側のセットメモが消える。
    #[test]
    fn same_sets_and_log_rank_ignore_the_drop_stages() {
        let plain = vec![set(60.0, 10), set(50.0, 5)];
        let marked = vec![set(60.0, 10), drop_set(50.0, 5, &[(50.0, 5)])];
        assert!(
            same_sets(&plain, &marked),
            "印の違いで不一致にしてはいけない"
        );
        assert!(same_sets_unordered(&plain, &marked));
        assert_ne!(plain, marked, "== は印を見る（だから same_sets が要る）");

        let a = ExerciseLog {
            exercise_id: e(10),
            sets: plain,
            label: None,
            at: None,
            note: String::new(),
        };
        let b = ExerciseLog {
            exercise_id: e(10),
            sets: marked,
            label: None,
            at: None,
            note: String::new(),
        };
        assert_eq!(
            log_rank(&a),
            log_rank(&b),
            "印の有無で勝ち負けが決まってはいけない"
        );
    }

    #[test]
    fn log_rank_ignores_the_notes() {
        // メモの有無でどちらのセットが勝つかが変わってはいけない
        let plain = log(10, &[(60.0, 10)], None);
        let noted = noted_log(10, "メモつき", &[(60.0, 10, "きつい")], None);
        assert_eq!(log_rank(&plain), log_rank(&noted));

        // 2 セットのログがメモ 1 個で 5 セットのログに勝たない
        let five = log(10, &[(60.0, 10); 5], None);
        assert!(log_rank(&five) > log_rank(&noted));
    }

    /// ★ 旧 `migrate_repairs_next_id_so_new_ids_cannot_collide` の後継。
    ///
    /// 連番 ID を乱数へ張り替えるとき、**全参照が一貫して動く**ことを見る。
    /// ID ではなく**名前**で検証するので、ログが別の種目を指すようになったら落ちる。
    #[test]
    fn migrate_rewrites_every_reference_consistently() {
        // 色に # が入るので r##"…"## にする（r#"…"# だと `"#e0524a` が終端になる）
        // 名前はプリセットに無いものにする（固定 ID へ寄せる経路と分けて見たい）
        let raw = r##"{
          "schema": 2, "next_id": 1,
          "groups": [{"id": 3, "name": "わたしの部位", "color": "#e0524a", "order": 0}],
          "exercises": [
            {"id": 42, "name": "わたしの種目", "group_id": 3, "order": 0},
            {"id": 43, "name": "べつの種目", "group_id": 3, "order": 1}
          ],
          "sessions": {
            "2026-08-08": { "logs": [
              {"exercise_id": 42, "sets": [{"weight": 60.0, "reps": 10}]},
              {"exercise_id": 43, "sets": [{"weight": 30.0, "reps": 12}]}
            ]}
          }
        }"##;

        let db = migrate(raw, &mut ids()).expect("正当な JSON");

        assert_eq!(db.schema, SCHEMA);

        let logs = &db.sessions["2026-08-08"].logs;
        let name_of = |ex: ExerciseId| db.exercise(ex).map(|e| e.name.as_str());
        assert_eq!(name_of(logs[0].exercise_id), Some("わたしの種目"));
        assert_eq!(name_of(logs[1].exercise_id), Some("べつの種目"));

        // 種目 → 部位の参照も一貫して張り替わる
        for ex in &db.exercises {
            assert_eq!(
                db.group(ex.group_id).map(|g| g.name.as_str()),
                Some("わたしの部位"),
                "{} の所属が宙に浮いた",
                ex.name
            );
        }

        // プリセット名ではないので予約領域には入らない
        assert!(db.groups.iter().all(|g| !g.id.is_reserved()));
        assert!(db.exercises.iter().all(|e| !e.id.is_reserved()));

        // archived は serde default で補われる
        assert!(!db.exercises[0].archived);
    }

    /// プリセットと同じ名前なら、全端末で共通の固定 ID に寄せる。
    /// これが無いと、別々に初期化された 2 台のマージが名前突合に落ちる。
    #[test]
    fn migrate_pins_preset_names_to_their_shared_fixed_ids() {
        let raw = r##"{
          "schema": 2, "next_id": 1,
          "groups": [{"id": 1, "name": "胸", "color": "#e0524a", "order": 0}],
          "exercises": [{"id": 2, "name": "ベンチプレス", "group_id": 1, "order": 0}],
          "sessions": {}
        }"##;

        let db = migrate(raw, &mut ids()).expect("正当な JSON");

        assert_eq!(
            db.groups[0].id,
            crate::presets::preset_group_id("胸").expect("プリセットにある")
        );
        assert_eq!(
            db.exercises[0].id,
            crate::presets::preset_exercise_id("ベンチプレス").expect("プリセットにある")
        );
        // 参照も固定 ID に追随している
        assert_eq!(db.exercises[0].group_id, db.groups[0].id);
    }

    /// ★ 改名には重複チェックが無いので、同じ名前の種目が 2 つある DB が実在しうる。
    /// 両方を同じ固定 ID に寄せると**別々の種目の履歴が無警告で 1 本に合流する** —
    /// この移行が潰そうとしているバグと同じ壊れ方になる。寄せてはいけない。
    #[test]
    fn migrate_does_not_pin_when_two_exercises_share_a_preset_name() {
        let raw = r##"{
          "schema": 2, "next_id": 1,
          "groups": [{"id": 1, "name": "胸", "color": "#e0524a", "order": 0}],
          "exercises": [
            {"id": 2, "name": "ベンチプレス", "group_id": 1, "order": 0},
            {"id": 3, "name": "ベンチプレス", "group_id": 1, "order": 1}
          ],
          "sessions": {
            "2026-08-08": { "logs": [
              {"exercise_id": 2, "sets": [{"weight": 60.0, "reps": 10}]},
              {"exercise_id": 3, "sets": [{"weight": 30.0, "reps": 12}]}
            ]}
          }
        }"##;

        let db = migrate(raw, &mut ids()).expect("正当な JSON");

        assert_ne!(
            db.exercises[0].id, db.exercises[1].id,
            "2 つが 1 つに潰れた"
        );
        assert!(
            db.exercises.iter().all(|e| !e.id.is_reserved()),
            "曖昧なときは固定 ID に寄せない"
        );
        // 2 本のログが別々の種目を指したまま残る
        let logs = &db.sessions["2026-08-08"].logs;
        assert_eq!(logs.len(), 2);
        assert_ne!(logs[0].exercise_id, logs[1].exercise_id);
    }

    #[test]
    fn migrate_round_trips_a_seeded_db() {
        let mut db = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let bench = db.exercises[0].id;
        put(
            &mut db,
            d(2026, 8, 8),
            vec![ExerciseLog {
                exercise_id: bench,
                sets: vec![SetEntry {
                    weight: 60.0,
                    reps: 10,
                    ..Default::default()
                }],
                at: Some(1_800_000_000_000),
                note: String::new(),
                label: None,
            }],
        );

        // schema 3 なので ID はそのまま。往復して同じものが返る
        let raw = serde_json::to_string(&db).expect("直列化できる");
        assert_eq!(migrate(&raw, &mut ids()).expect("復元できる"), db);
    }

    /// 未知の（= 将来の）schema は**触らない**。黙って読むと、知らないフィールドを
    /// 落としたまま書き戻して未来のデータを壊す。
    #[test]
    fn migrate_refuses_a_newer_schema_instead_of_dropping_fields() {
        let raw = r#"{"schema": 99, "groups": [], "exercises": [], "sessions": {}}"#;

        match migrate(raw, &mut ids()) {
            Err(RestoreError::Unsupported(99)) => {}
            other => panic!("Unsupported(99) を期待したが {other:?}"),
        }
    }

    // ── トレーニングメニューの正規化 ────────────────────────────────────────
    // adr/data-model/routines-as-named-exercise-lists.md

    #[test]
    fn migrate_dedupes_repeated_exercises_inside_a_routine() {
        // 残ると展開時に同じ種目のログが 2 本でき「1 日 1 種目 1 ログ」が破れる
        let mut db = test_db();
        db.routines
            .push(routine(1, "胸の日", &[10, 11, 10, 11, 10]));

        let out = round_trip(&db);
        assert_eq!(
            out.routines[0].exercises,
            vec![e(10), e(11)],
            "初出の順で残す"
        );
    }

    // ── TSV ─────────────────────────────────────────────────────────────────

    /// テスト用の固定オフセット（JST）。★ `Local` を使わないので実行環境の TZ に依らない。
    fn jst() -> chrono::FixedOffset {
        chrono::FixedOffset::east_opt(9 * 3600).expect("有効なオフセット")
    }

    /// ベンチプレス 2 セット + 体重 + 体調メモの 1 日。
    fn tsv_sample() -> Db {
        let mut db = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        db.sessions.insert(
            date_key(d(2026, 8, 1)),
            Session {
                logs: vec![ExerciseLog {
                    exercise_id: bench,
                    sets: vec![
                        SetEntry {
                            weight: 60.0,
                            reps: 10,
                            ..Default::default()
                        },
                        SetEntry {
                            weight: 62.5,
                            reps: 8,
                            note: "きつい".into(),
                            ..Default::default()
                        },
                    ],
                    at: None,
                    note: "肩が良い".into(),
                    label: None,
                }],
                body_weight: Some(72.5),
                note: "よく寝た".into(),
            },
        );
        db
    }

    /// TSV の行を `\t` 区切りのセル配列にする。
    fn rows(tsv: &str) -> Vec<Vec<&str>> {
        tsv.lines().map(|l| l.split('\t').collect()).collect()
    }

    /// 見出し名から列の位置を引く。**本文のセルを位置で読まないための入口。**
    ///
    /// ★ 位置で読むと、列を 1 本足すたびに関係の無いテストが軒並み落ちる。しかも
    /// ずれた添字が隣の列を指すので、**落ちずに通ってしまう**組み合わせがある
    /// （「体重kg」と「セットメモ」はどちらも空セルになる行が多い）。[`TsvCols`] が
    /// 取り込み側で「位置ではなく名前で引く」と決めているのと同じ規範を、テストにも通す。
    fn col(r: &[Vec<&str>], name: &str) -> usize {
        r[0].iter()
            .position(|c| *c == name)
            .unwrap_or_else(|| panic!("{name} 列がある"))
    }

    /// ★ 見出しは外部仕様。並びが変わると既存のシートの数式が全部ずれるので、
    ///   バイト一致で固定する（形式の進化規則 1「列は足すだけ」の実行部）。
    #[test]
    fn export_tsv_starts_with_the_documented_header() {
        let tsv = export_tsv(
            &crate::presets::seeded_db(crate::i18n::Lang::Ja),
            jst(),
            crate::i18n::Lang::Ja,
        );
        assert_eq!(
            tsv.lines().next().expect("見出し行"),
            "日付\t部位\t種目\tセット\t重量kg\t回数\tドロップ\t体重kg\tセットメモ\t種目メモ\t体調メモ\t時刻\tメニュー\tピン\tインターバル秒\tラベル"
        );
    }

    /// 英語の見出しも**同じ強度で固定する**。TSV は版番号を持てないので、
    /// 一度出した綴りは永久の外部仕様（adr/storage/tsv-header-follows-the-ui-language.md）。
    #[test]
    fn export_tsv_in_english_starts_with_the_documented_header() {
        let tsv = export_tsv(
            &crate::presets::seeded_db(crate::i18n::Lang::En),
            jst(),
            crate::i18n::Lang::En,
        );
        assert_eq!(
            tsv.lines().next().expect("見出し行"),
            "Date\tMuscle group\tExercise\tSet\tWeight kg\tReps\tDrop set\tBody weight kg\tSet note\tExercise note\tDay note\tTime\tRoutine\tPins\tInterval sec\tLabel"
        );
    }

    /// 日英の見出しが**1 語も衝突しない**。衝突すると `tsv_cols` の `match` で
    /// 片方の腕が到達不能になり、列が黙って別の意味に読まれる。
    #[test]
    fn tsv_header_names_do_not_collide_across_languages() {
        for ja in TSV_HEADER_JA {
            assert!(
                !TSV_HEADER_EN.contains(&ja),
                "{ja} が日英の見出しで衝突している"
            );
        }
        assert_eq!(TSV_HEADER_JA.len(), TSV_HEADER_EN.len());
    }

    /// **過去に書き出した日本語 TSV が、英語で使っている端末でもそのまま取り込める。**
    /// ここが落ちると、言語を切り替えた人の手元のファイルが読めなくなる。
    #[test]
    fn a_japanese_tsv_still_imports_on_an_english_device() {
        let mine = crate::presets::seeded_db(crate::i18n::Lang::En);
        let tsv = "日付\t部位\t種目\tセット\t重量kg\t回数\n                   2026-08-01\t胸\tベンチプレス\t1\t60\t10\n";

        let db = parse_import(tsv, &mut ids(), &mine).expect("日本語 TSV が読めること");

        // 固定 ID に寄っている＝種目が 2 本に割れていない
        let log = &db.sessions["2026-08-01"].logs[0];
        assert_eq!(log.exercise_id, ExerciseId::from_bits(0x11));
        assert_eq!(log.sets[0].reps, 10);
    }

    /// 逆向き。英語の見出しと英語のプリセット名で書かれた TSV を、日本語で使っている
    /// 端末が取り込んでも同じ固定 ID に寄る。
    #[test]
    fn an_english_tsv_imports_on_a_japanese_device() {
        let mut mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let tsv = "Date\tMuscle group\tExercise\tSet\tWeight kg\tReps\n                   2026-08-01\tChest\tBench Press\t1\t60\t10\n";

        let incoming = parse_import(tsv, &mut ids(), &mine).expect("英語 TSV が読めること");
        assert_eq!(
            incoming.sessions["2026-08-01"].logs[0].exercise_id,
            ExerciseId::from_bits(0x11),
            "英語の綴りが固定 ID に寄っていない"
        );

        // 取り込みはマージのみ（adr/storage/import-is-merge-only.md）。実際の経路で見る
        let before = mine.exercises.len();
        merge_db(&mut mine, incoming);

        assert_eq!(
            mine.exercises.len(),
            before,
            "英語 TSV の取り込みで種目が増えている（同じ種目が 2 本に割れた）"
        );
        // ★ 既に持っている名前は**上書きされない**。取り込みで自分の付けた名前が
        //   相手の言語に書き換わったら、それは記録ではなく破壊
        let ex = mine
            .exercises
            .iter()
            .find(|e| e.id == ExerciseId::from_bits(0x11))
            .expect("ベンチプレス");
        assert_eq!(ex.name, "ベンチプレス");
        assert_eq!(mine.sessions["2026-08-01"].logs[0].sets[0].reps, 10);
    }

    /// 英語で書き出したものを英語で取り込む往復。**種目が増えない**ことを見る。
    #[test]
    fn export_import_round_trips_in_english() {
        let en = crate::i18n::Lang::En;
        let mut mine = crate::presets::seeded_db(en);
        mine.sessions.insert(
            "2026-08-01".to_string(),
            Session {
                logs: vec![ExerciseLog {
                    exercise_id: ExerciseId::from_bits(0x11),
                    sets: vec![SetEntry {
                        weight: 60.0,
                        reps: 10,
                        ..Default::default()
                    }],
                    at: None,
                    note: String::new(),
                    label: None,
                }],
                ..Default::default()
            },
        );

        let tsv = export_tsv(&mine, jst(), en);
        let before = mine.exercises.len();
        let parsed = parse_import(&tsv, &mut ids(), &mine).expect("往復できること");

        merge_db(&mut mine, parsed);
        assert_eq!(
            mine.exercises.len(),
            before,
            "往復で種目が増えている（固定 ID に寄っていない）"
        );
    }

    /// ★ **日本語で初期化した端末が英語で書き出すと、種目名も英語で出る。**
    /// 保存名は日本語のままなので、ここが表示名を通っていないと英語のシートに
    /// 日本語の種目名が並ぶ。
    #[test]
    fn export_in_english_writes_english_names_for_untouched_presets() {
        let ja = crate::i18n::Lang::Ja;
        let en = crate::i18n::Lang::En;
        let mut db = crate::presets::seeded_db(ja); // 保存名は日本語
        db.sessions.insert(
            "2026-08-01".to_string(),
            Session {
                logs: vec![ExerciseLog {
                    exercise_id: ExerciseId::from_bits(0x11),
                    sets: vec![SetEntry {
                        weight: 60.0,
                        reps: 10,
                        ..Default::default()
                    }],
                    at: None,
                    note: String::new(),
                    label: None,
                }],
                ..Default::default()
            },
        );

        let tsv = export_tsv(&db, jst(), en);
        assert!(
            tsv.contains("Bench Press"),
            "英語の種目名が出ていない:\n{tsv}"
        );
        assert!(
            tsv.contains("\tChest\t"),
            "英語の部位名が出ていない:\n{tsv}"
        );
        assert!(
            !tsv.contains("ベンチプレス"),
            "保存名がそのまま出ている:\n{tsv}"
        );

        // 日本語で書き出せば日本語のまま（保存名を書き換えていない証拠）
        let tsv_ja = export_tsv(&db, jst(), ja);
        assert!(tsv_ja.contains("ベンチプレス"));
    }

    /// 改名した種目は**どちらの言語で書き出しても利用者が付けた名前**。
    #[test]
    fn export_keeps_a_renamed_preset_as_the_user_named_it() {
        let en = crate::i18n::Lang::En;
        let mut db = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        db.exercises
            .iter_mut()
            .find(|e| e.id == ExerciseId::from_bits(0x11))
            .expect("ベンチプレス")
            .name = "マイベンチ".to_string();
        db.sessions.insert(
            "2026-08-01".to_string(),
            Session {
                logs: vec![ExerciseLog {
                    exercise_id: ExerciseId::from_bits(0x11),
                    sets: vec![SetEntry {
                        weight: 60.0,
                        reps: 10,
                        ..Default::default()
                    }],
                    at: None,
                    note: String::new(),
                    label: None,
                }],
                ..Default::default()
            },
        );

        let tsv = export_tsv(&db, jst(), en);
        assert!(tsv.contains("マイベンチ"));
        assert!(!tsv.contains("Bench Press"));
    }

    /// ★ **表示名で書き出しても往復が閉じる。** 日本語の端末が英語で書き出した
    /// ファイルを自分で取り込み直しても、種目が 2 本に割れない。
    #[test]
    fn an_english_export_reimports_into_the_japanese_device_it_came_from() {
        let en = crate::i18n::Lang::En;
        let mut mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        mine.sessions.insert(
            "2026-08-01".to_string(),
            Session {
                logs: vec![ExerciseLog {
                    exercise_id: ExerciseId::from_bits(0x11),
                    sets: vec![SetEntry {
                        weight: 60.0,
                        reps: 10,
                        ..Default::default()
                    }],
                    at: None,
                    note: String::new(),
                    label: None,
                }],
                ..Default::default()
            },
        );
        let before = mine.exercises.len();

        let tsv = export_tsv(&mine, jst(), en);
        let incoming = parse_import(&tsv, &mut ids(), &mine).expect("読み戻せる");
        merge_db(&mut mine, incoming);

        assert_eq!(
            mine.exercises.len(),
            before,
            "英語で書き出したファイルを戻すと種目が増える"
        );
        // 保存名は日本語のまま（取り込みが名前を書き換えていない）
        assert_eq!(
            mine.exercises
                .iter()
                .find(|e| e.id == ExerciseId::from_bits(0x11))
                .map(|e| e.name.as_str()),
            Some("ベンチプレス")
        );
    }

    #[test]
    fn migrate_keeps_a_routine_that_points_at_a_missing_exercise() {
        // ★ 消してはいけない。宙に浮いた参照は「相手の端末のデータ」しか作らないので、
        //   後からそのファイルを取り込めば生き返る。読み込みのたびに消すと救済できない
        let mut db = test_db();
        db.routines.push(routine(1, "胸の日", &[10, 99]));

        let out = round_trip(&db);
        assert_eq!(out.routines[0].exercises, vec![e(10), e(99)]);
    }

    #[test]
    fn migrate_keeps_a_routine_that_points_at_an_archived_exercise() {
        // アーカイブは可逆な操作。参照を消すと戻したときに復元できない
        let mut db = test_db();
        db.exercises[1].archived = true;
        db.routines.push(routine(1, "胸の日", &[10, 11]));

        let out = round_trip(&db);
        assert_eq!(out.routines[0].exercises, vec![e(10), e(11)]);
    }

    #[test]
    fn migrate_keeps_a_named_routine_that_has_no_exercises() {
        // 名前を打って種目を選ぶ前に閉じた状態を消してはいけない
        let mut db = test_db();
        db.routines.push(routine(1, "胸の日", &[]));

        assert_eq!(round_trip(&db).routines.len(), 1);
    }

    #[test]
    fn migrate_keeps_an_unnamed_routine_that_has_exercises() {
        let mut db = test_db();
        db.routines.push(routine(1, "", &[10]));

        assert_eq!(round_trip(&db).routines.len(), 1);
    }

    #[test]
    fn migrate_drops_a_routine_with_neither_a_name_nor_exercises() {
        let mut db = test_db();
        db.routines.push(routine(1, "胸の日", &[10]));
        db.routines.push(routine(2, "", &[]));

        let out = round_trip(&db);
        assert_eq!(out.routines.len(), 1);
        assert_eq!(out.routines[0].id, r(1));
    }

    #[test]
    fn migrate_clears_a_whitespace_only_routine_name() {
        // ★ 空白だけの名前を空にするのが is_empty より先。順が逆だと " " が
        //   「名前がある」と「空」の判定でズレる
        let mut db = test_db();
        db.routines.push(routine(1, "　", &[10]));
        db.routines.push(routine(2, " \n ", &[]));

        let out = round_trip(&db);
        assert_eq!(out.routines.len(), 1, "空白だけ × 種目なし は落とす");
        assert_eq!(out.routines[0].name, "");
    }

    #[test]
    fn migrate_does_not_trim_a_routine_name_that_has_other_characters() {
        // 両端の空白を削るのはユーザーが打った文字の書き換えになる
        let mut db = test_db();
        db.routines.push(routine(1, " 胸の日 ", &[10]));

        assert_eq!(round_trip(&db).routines[0].name, " 胸の日 ");
    }

    #[test]
    fn migrate_gives_a_new_id_to_a_routine_that_repeats_an_existing_one() {
        // 重複キーは <For> の keyed diff を壊し、削除は「片方消すと両方消える」になる。
        // 捨てずに振り直すのは、組んだリストを黙って失わないため
        let mut db = test_db();
        db.routines.push(routine(1, "胸の日", &[10]));
        db.routines.push(routine(1, "背中の日", &[11]));

        let out = round_trip(&db);
        assert_eq!(out.routines.len(), 2, "捨ててはいけない");
        assert_eq!(out.routines[0].id, r(1), "先に出たほうは動かさない");
        assert_ne!(out.routines[1].id, r(1));
        assert_eq!(out.routines[1].name, "背中の日");
    }

    #[test]
    fn migrate_keeps_the_order_of_the_routines_and_of_their_exercises() {
        let mut db = test_db();
        db.routines.push(routine(1, "胸の日", &[11, 10]));
        db.routines.push(routine(2, "体幹の日", &[20]));

        let out = round_trip(&db);
        let names: Vec<&str> = out.routines.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, vec!["胸の日", "体幹の日"]);
        assert_eq!(out.routines[0].exercises, vec![e(11), e(10)]);
    }

    #[test]
    fn migrate_from_schema_two_leaves_the_routines_empty() {
        let raw = r#"{"schema":2,"groups":[],"exercises":[],"sessions":{}}"#;
        let db = migrate(raw, &mut ids()).expect("旧世代が読める");
        assert!(db.routines.is_empty());
    }

    #[test]
    fn migrate_reads_schema_three_json_written_before_routines_existed() {
        let raw = r#"{"schema":3,"groups":[],"exercises":[],"sessions":{}}"#;
        let db = migrate(raw, &mut ids()).expect("メニュー以前の schema 3 が読める");
        assert!(db.routines.is_empty());
    }

    #[test]
    fn export_tsv_writes_one_row_per_set_with_a_one_based_index() {
        let tsv = export_tsv(&tsv_sample(), jst(), crate::i18n::Lang::Ja);
        let r = rows(&tsv);
        assert_eq!(r.len(), 3, "見出し + 2 セット: {tsv}");
        assert_eq!(
            r[1][0..6],
            ["2026-08-01", "胸", "ベンチプレス", "1", "60", "10"]
        );
        assert_eq!(
            r[2][0..6],
            ["2026-08-01", "胸", "ベンチプレス", "2", "62.5", "8"]
        );
        assert_eq!(
            r[2][col(&r, "セットメモ")],
            "きつい",
            "セットメモが行に付いていない"
        );
    }

    /// ★ 毎行書くとシート側の `AVERAGE` が「セット数で重み付けした平均」になり、
    ///   体重グラフが嘘の値を出す。日・ログの先頭 1 行にだけ書く。
    #[test]
    fn export_tsv_writes_the_day_scoped_cells_only_once() {
        let tsv = export_tsv(&tsv_sample(), jst(), crate::i18n::Lang::Ja);
        let r = rows(&tsv);
        assert_eq!(
            (
                r[1][col(&r, "体重kg")],
                r[1][col(&r, "種目メモ")],
                r[1][col(&r, "体調メモ")],
            ),
            ("72.5", "肩が良い", "よく寝た")
        );
        assert_eq!(
            (
                r[2][col(&r, "体重kg")],
                r[2][col(&r, "種目メモ")],
                r[2][col(&r, "体調メモ")],
            ),
            ("", "", "")
        );
    }

    /// 自重種目。`set_volume` も `fmt_set` も 0 を「重量なし」として扱うので、
    /// シートで平均・グラフから外れる空セルと意味が一致する。
    #[test]
    fn export_tsv_leaves_the_weight_cell_empty_for_bodyweight_sets() {
        let mut db = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let pullup = crate::presets::preset_exercise_id("懸垂").expect("プリセット");
        db.sessions.insert(
            date_key(d(2026, 8, 2)),
            Session {
                logs: vec![ExerciseLog {
                    exercise_id: pullup,
                    sets: vec![SetEntry {
                        weight: 0.0,
                        reps: 12,
                        ..Default::default()
                    }],
                    at: None,
                    note: String::new(),
                    label: None,
                }],
                body_weight: None,
                note: String::new(),
            },
        );
        let tsv = export_tsv(&db, jst(), crate::i18n::Lang::Ja);
        let r = rows(&tsv);
        assert_eq!(r[1][4], "", "重量 0 が 0 として書かれている");
        assert_eq!(r[1][5], "12");
    }

    /// `ExerciseLog::is_empty` が「残す」と決めた形（セット 0・メモあり）を落とさない。
    #[test]
    fn export_tsv_keeps_a_note_only_log_as_a_row_without_sets() {
        let mut db = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        db.sessions.insert(
            date_key(d(2026, 8, 3)),
            Session {
                logs: vec![ExerciseLog {
                    exercise_id: bench,
                    sets: Vec::new(),
                    at: None,
                    note: "肩が痛いのでやめた".into(),
                    label: None,
                }],
                body_weight: None,
                note: String::new(),
            },
        );
        let tsv = export_tsv(&db, jst(), crate::i18n::Lang::Ja);
        let r = rows(&tsv);
        assert_eq!(r[1][2], "ベンチプレス");
        assert_eq!((r[1][3], r[1][5]), ("", ""), "セットが立っている");
        assert_eq!(r[1][col(&r, "種目メモ")], "肩が痛いのでやめた");
    }

    /// 体重・体調メモだけの日も 1 行残す（`Session::is_empty` が残すと決めた形）。
    #[test]
    fn export_tsv_keeps_a_day_that_only_has_a_body_weight() {
        let mut db = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        db.sessions.insert(
            date_key(d(2026, 8, 4)),
            Session {
                logs: Vec::new(),
                body_weight: Some(71.0),
                note: String::new(),
            },
        );
        let tsv = export_tsv(&db, jst(), crate::i18n::Lang::Ja);
        let r = rows(&tsv);
        assert_eq!(r[1][0], "2026-08-04");
        assert_eq!((r[1][2], r[1][col(&r, "体重kg")]), ("", "71"));
    }

    /// ★ 未使用の自作種目を黙って失わない。手つかずのプリセットは書かない
    ///   （新規インストールに必ず同じ固定 ID で居るので、書かなくても失われない）。
    #[test]
    fn export_tsv_writes_a_master_row_only_for_exercises_that_could_be_lost() {
        let mut db = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let mut g = ids();
        db.exercises.push(Exercise {
            id: g.alloc(),
            name: "自作マシン".into(),
            group_id: crate::presets::preset_group_id("胸").expect("プリセット"),
            order: 99,
            archived: false,
            pins: Vec::new(),
            interval_sec: None,
            labels: Vec::new(),
        });
        let tsv = export_tsv(&db, jst(), crate::i18n::Lang::Ja);
        let r = rows(&tsv);
        assert_eq!(r.len(), 2, "プリセット 28 種目まで書かれている: {tsv}");
        assert_eq!(r[1][0..3], ["", "胸", "自作マシン"]);
    }

    /// ★ `append_note` が合流時に `\n` を挟むので、改行入りのメモは実在する。
    ///   潰さないと行が崩壊して、その日の記録が丸ごと壊れる。
    #[test]
    fn export_tsv_flattens_tabs_and_newlines_in_notes() {
        let mut db = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        db.sessions.insert(
            date_key(d(2026, 8, 5)),
            Session {
                logs: vec![ExerciseLog {
                    exercise_id: bench,
                    sets: vec![SetEntry {
                        weight: 60.0,
                        reps: 10,
                        note: "前半\tきつい".into(),
                        ..Default::default()
                    }],
                    at: None,
                    note: "1 本目\n2 本目".into(),
                    label: None,
                }],
                body_weight: None,
                note: String::new(),
            },
        );
        let tsv = export_tsv(&db, jst(), crate::i18n::Lang::Ja);
        assert_eq!(tsv.lines().count(), 2, "改行でレコードが割れている: {tsv}");
        let r = rows(&tsv);
        assert_eq!(r[1].len(), 16, "タブで列がずれている");
        assert_eq!(r[1][col(&r, "セットメモ")], "前半 きつい");
        assert_eq!(r[1][col(&r, "種目メモ")], "1 本目 2 本目");
    }

    /// `at` はその行の日付と同じ暦日のときだけ出す。壊れたデータ由来の `at` を
    /// 前日の行に時刻として並べない（`ExerciseLog::at` の契約を表示にも通す）。
    #[test]
    fn export_tsv_writes_the_time_only_when_it_agrees_with_the_date() {
        let mut db = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        // 2026-08-01 18:32 JST
        let at = d(2026, 8, 1)
            .and_hms_opt(18, 32, 0)
            .expect("有効な時刻")
            .and_local_timezone(jst())
            .single()
            .expect("一意")
            .timestamp_millis();
        let log = |exercise_id, at| ExerciseLog {
            exercise_id,
            sets: vec![SetEntry {
                weight: 60.0,
                reps: 10,
                ..Default::default()
            }],
            at,
            note: String::new(),
            label: None,
        };
        db.sessions.insert(
            date_key(d(2026, 8, 1)),
            Session {
                logs: vec![log(bench, Some(at))],
                body_weight: None,
                note: String::new(),
            },
        );
        // 日付キーと食い違う `at`
        db.sessions.insert(
            date_key(d(2026, 8, 9)),
            Session {
                logs: vec![log(bench, Some(at))],
                body_weight: None,
                note: String::new(),
            },
        );
        let tsv = export_tsv(&db, jst(), crate::i18n::Lang::Ja);
        let r = rows(&tsv);
        assert_eq!(r[1][col(&r, "時刻")], "18:32");
        assert_eq!(
            r[2][col(&r, "時刻")],
            "",
            "日付と食い違う at を時刻として出している"
        );
    }

    /// ★ この機能の生命線その 2。書き出した TSV が、そのまま記録に戻る。
    #[test]
    fn tsv_round_trips_every_record_except_the_clock() {
        let db = tsv_sample();
        let tsv = export_tsv(&db, jst(), crate::i18n::Lang::Ja);

        let mut mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let incoming = parse_import(&tsv, &mut ids(), &mine).expect("読み戻せる");
        merge_db(&mut mine, incoming);

        assert_eq!(summarize(&mine), summarize(&db));
        let s = mine.sessions.get("2026-08-01").expect("その日がある");
        assert_eq!(s.body_weight, Some(72.5));
        assert_eq!(s.note, "よく寝た");
        let log = &s.logs[0];
        assert_eq!(log.note, "肩が良い");
        assert_eq!(
            log.sets,
            vec![
                SetEntry {
                    weight: 60.0,
                    reps: 10,
                    ..Default::default()
                },
                SetEntry {
                    weight: 62.5,
                    reps: 8,
                    note: "きつい".into(),
                    ..Default::default()
                },
            ]
        );
        // ★ 時刻列は書き出し専用。取り込みでは必ず落ちる
        assert_eq!(log.at, None, "TSV から at を復元してしまっている");
    }

    /// メインセット 2 本で、2 本目に段が 2 つぶら下がった記録。
    fn drop_sample() -> Db {
        let mut db = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        db.sessions.insert(
            date_key(d(2026, 8, 1)),
            Session {
                logs: vec![ExerciseLog {
                    exercise_id: bench,
                    sets: vec![set(60.0, 10), drop_set(60.0, 6, &[(50.0, 5), (40.0, 4)])],
                    label: None,
                    at: None,
                    note: String::new(),
                }],
                body_weight: None,
                note: String::new(),
            },
        );
        db
    }

    /// ★ 段はセット単位。ピン / インターバルの「その種目の初出行にだけ書く」を
    ///   真似て 1 行にまとめると、どのセットから落としたのかがシートから読めなくなる。
    #[test]
    fn export_tsv_writes_the_stages_on_the_row_they_hang_from() {
        let tsv = export_tsv(&drop_sample(), jst(), crate::i18n::Lang::Ja);
        let r = rows(&tsv);
        let c = col(&r, "ドロップ");
        assert_eq!(
            (r[1][c], r[2][c]),
            ("", "50×5 40×4"),
            "段が別の行に書かれている: {tsv}"
        );
    }

    /// ★ 書き出しで落ちると、機種変更でドロップの段が静かに消える。
    ///   消えたぶんは推移に混ざり直すので、グラフが後から勝手に増える。
    #[test]
    fn tsv_round_trips_the_drop_stages() {
        let db = drop_sample();
        let tsv = export_tsv(&db, jst(), crate::i18n::Lang::Ja);

        let mut mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let incoming = parse_import(&tsv, &mut ids(), &mine).expect("読み戻せる");
        merge_db(&mut mine, incoming);

        let s = mine.sessions.get("2026-08-01").expect("その日がある");
        assert_eq!(s.logs[0].sets, db.sessions["2026-08-01"].logs[0].sets);
    }

    /// ★ 列を足す前に書き出したファイルは今も読める（形式の進化規則 2）。
    ///   `tsv_header` が名前で引くので、無い列は `None` のまま空に落ちる。
    #[test]
    fn tsv_import_reads_a_file_written_before_the_drop_column_existed() {
        let old = "日付\t部位\t種目\tセット\t重量kg\t回数\t体重kg\tセットメモ\t種目メモ\t体調メモ\t時刻\tメニュー\tピン\tインターバル秒\n\
                   2026-08-01\t胸\tベンチプレス\t1\t60\t10\t\t\t\t\t\t\t\t\n";
        let mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let db = parse_import(old, &mut ids(), &mine).expect("旧 14 列も読める");
        let s = db.sessions.get("2026-08-01").expect("その日がある");
        assert!(
            s.logs[0].sets[0].drops.is_empty(),
            "無い列から段が生えている"
        );
    }

    /// ★ セルの綴りは書き出しより広く受ける。シートで手で書き足す人が居るので、
    ///   区切りと掛け算記号を取り違えると打った段が黙って落ちる。
    #[test]
    fn parse_drops_cell_reads_the_spellings_a_sheet_can_produce() {
        let want = vec![
            DropStage {
                weight: 50.0,
                reps: 5,
            },
            DropStage {
                weight: 40.0,
                reps: 4,
            },
        ];
        for cell in [
            "50×5 40×4",
            "50x5 40x4",
            "50X5 40X4",
            "50*5 40*4",
            "50×5,40×4",
            "50×5、40×4",
            " 50×5   40×4 ",
            "50×5.0 40×4.0",
        ] {
            assert_eq!(parse_drops_cell(cell), want, "読めていない: {cell:?}");
        }

        // 空セルは段なし
        assert!(parse_drops_cell("").is_empty());
        assert!(parse_drops_cell("  ").is_empty());
        // ★ 読めない要素は黙って捨てる（行ごと落とすほどではない）
        assert_eq!(parse_drops_cell("50×5 ゴミ 40×4"), want);
        // 回数が無い要素は捨てる（重量だけでは段にならない）
        assert!(parse_drops_cell("50").is_empty());
        assert!(parse_drops_cell("50×").is_empty());
        // 重量なしの段（自重）は受ける
        assert_eq!(
            parse_drops_cell("×8"),
            vec![DropStage {
                weight: 0.0,
                reps: 8
            }]
        );
        // ★ 上限はここで切らない（`clean_drops` の仕事）。`parse_tsv` は最後に
        //   `normalize` を通るので、取り込み結果としては 4 段に収まる
        assert_eq!(parse_drops_cell("1×1 2×1 3×1 4×1 5×1").len(), 5);
    }

    /// ★ 回数 0 の段が `drops` を非空にすると、何も外していないのに推移タブの注記が出る。
    ///   上限も同じ 1 箇所（`clean_drops`）が切ることを一緒に固定する。
    #[test]
    fn normalize_prunes_stages_that_cannot_be_a_set() {
        let mut db = test_db();
        db.sessions.insert(
            date_key(d(2026, 8, 1)),
            Session {
                logs: vec![ExerciseLog {
                    exercise_id: e(10),
                    sets: vec![drop_set(60.0, 6, &[(50.0, 0), (40.0, 4)])],
                    label: None,
                    at: None,
                    note: String::new(),
                }],
                ..Session::default()
            },
        );

        normalize(&mut db, &mut ids());

        assert_eq!(
            db.sessions["2026-08-01"].logs[0].sets[0].drops,
            vec![DropStage {
                weight: 40.0,
                reps: 4
            }],
            "回数 0 の段が残っている"
        );
    }

    /// ★ 上限を切るのも `clean_drops` の 1 箇所。取り込みのパーサ側で切らないので、
    ///   ここが唯一の門になる。
    #[test]
    fn normalize_caps_the_stages_at_the_limit() {
        let many: Vec<(f32, u32)> = (1..=MAX_DROPS as u32 + 2).map(|n| (10.0, n)).collect();
        let mut db = test_db();
        db.sessions.insert(
            date_key(d(2026, 8, 1)),
            Session {
                logs: vec![ExerciseLog {
                    exercise_id: e(10),
                    sets: vec![drop_set(60.0, 6, &many)],
                    label: None,
                    at: None,
                    note: String::new(),
                }],
                ..Session::default()
            },
        );

        normalize(&mut db, &mut ids());

        assert_eq!(
            db.sessions["2026-08-01"].logs[0].sets[0].drops.len(),
            MAX_DROPS
        );
    }

    /// ★ メニューを書かないと**機種変更でメニューだけ消える**。記録は行に出るので
    ///   残るが、メニューは `Db` にしか無い。
    #[test]
    fn tsv_round_trips_routines() {
        let mut db = tsv_sample();
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        let fly = crate::presets::preset_exercise_id("チェストフライ").expect("プリセット");
        let mut g = ids();
        db.routines.push(Routine {
            id: g.alloc(),
            name: "胸の日".into(),
            exercises: vec![bench, fly],
        });
        // 名前だけのメニュー（種目を選ぶ前に閉じた状態）も落とさない
        db.routines.push(Routine {
            id: g.alloc(),
            name: "これから作る".into(),
            exercises: Vec::new(),
        });

        let tsv = export_tsv(&db, jst(), crate::i18n::Lang::Ja);
        let mut mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let incoming = parse_import(&tsv, &mut ids(), &mine).expect("読み戻せる");
        merge_db(&mut mine, incoming);

        assert_eq!(
            mine.routines
                .iter()
                .map(|x| (x.name.as_str(), x.exercises.clone()))
                .collect::<Vec<_>>(),
            vec![("胸の日", vec![bench, fly]), ("これから作る", Vec::new()),],
            "メニューが往復していない: {tsv}"
        );
    }

    /// ★ **メニュー名は一意ではない**（画面から同名を作れるし、`merge_db` は名前だけでは
    ///   寄せない）。名前でまとめると、同名 2 本のファイルを読み戻したときに
    ///   **中身が融合した別物**になる。`セット` 列の番号が 1 に戻るところで区切る。
    #[test]
    fn tsv_round_trips_two_routines_that_share_a_name() {
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        let fly = crate::presets::preset_exercise_id("チェストフライ").expect("プリセット");
        let push = crate::presets::preset_exercise_id("プッシュアップ").expect("プリセット");
        let mut g = ids();
        let mut db = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        db.routines.push(Routine {
            id: g.alloc(),
            name: "胸の日".into(),
            exercises: vec![bench, fly],
        });
        db.routines.push(Routine {
            id: g.alloc(),
            name: "胸の日".into(),
            exercises: vec![bench, push],
        });

        let tsv = export_tsv(&db, jst(), crate::i18n::Lang::Ja);
        let mut mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let incoming = parse_import(&tsv, &mut ids(), &mine).expect("読み戻せる");
        merge_db(&mut mine, incoming);

        assert_eq!(
            mine.routines
                .iter()
                .map(|x| (x.name.as_str(), x.exercises.clone()))
                .collect::<Vec<_>>(),
            vec![("胸の日", vec![bench, fly]), ("胸の日", vec![bench, push])],
            "同名のメニューが融合した: {tsv}"
        );

        // 2 回目で増えない（融合していると毎回 1 本ずつ増える）
        let again = parse_import(&tsv, &mut ids(), &mine).expect("2 回目");
        assert!(merge_db(&mut mine, again).is_noop(), "2 回目で増えている");
    }

    /// ★ メニューは名前だけでは寄せない規則なので、2 回入れて増えないことを別に見る
    ///   （`merge_db` は「名前と種目が両方一致」で初めて重複と判断する）。
    ///   routines 無しの TSV も同じ経路（`importing_the_same_tsv_twice_adds_nothing` は
    ///   このテストの部分集合だったので削除した）。
    #[test]
    fn importing_a_tsv_with_routines_twice_adds_nothing() {
        let mut db = tsv_sample();
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        db.routines.push(Routine {
            id: ids().alloc(),
            name: "胸の日".into(),
            exercises: vec![bench],
        });
        let tsv = export_tsv(&db, jst(), crate::i18n::Lang::Ja);

        let mut mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let first = parse_import(&tsv, &mut ids(), &mine).expect("1 回目");
        merge_db(&mut mine, first);
        let second = parse_import(&tsv, &mut ids(), &mine).expect("2 回目");
        let report = merge_db(&mut mine, second);

        assert!(report.is_noop(), "2 回目で増えている: {report:?}");
        assert_eq!(mine.routines.len(), 1);
    }

    /// ★ 自分のファイルを戻すだけで確認画面が「同じ種目とみなしました」で埋まらない。
    ///   これが `parse_import` に `mine` を渡している理由。
    #[test]
    fn importing_my_own_tsv_reports_no_conflict() {
        let mut mine = tsv_sample();
        let mut g = ids();
        let custom = g.alloc();
        mine.exercises.push(Exercise {
            id: custom,
            name: "自作マシン".into(),
            group_id: crate::presets::preset_group_id("胸").expect("プリセット"),
            order: 99,
            archived: false,
            pins: Vec::new(),
            interval_sec: None,
            labels: Vec::new(),
        });

        let tsv = export_tsv(&mine, jst(), crate::i18n::Lang::Ja);
        let incoming = parse_import(&tsv, &mut ids(), &mine).expect("読める");
        let report = merge_db(&mut mine, incoming);

        assert!(report.conflicts.is_empty(), "{:?}", report.conflicts);
        assert!(report.is_noop(), "{report:?}");
    }

    /// ★ プリセットを改名した端末に別端末の TSV を入れても、履歴が 2 本に割れない。
    ///   名前照合だけに頼ると壊れる形（adr/data-model/random-ids-for-safe-merge.md）。
    #[test]
    fn tsv_import_joins_history_with_a_renamed_preset() {
        let tsv = export_tsv(&tsv_sample(), jst(), crate::i18n::Lang::Ja);

        let mut theirs = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        let before = theirs.exercises.len();
        theirs
            .exercises
            .iter_mut()
            .find(|e| e.id == bench)
            .expect("居る")
            .name = "ベンチプレス（スミス）".into();

        let incoming = parse_import(&tsv, &mut ids(), &theirs).expect("読める");
        merge_db(&mut theirs, incoming);

        assert_eq!(theirs.exercises.len(), before, "同じ種目が 2 つになった");
        let s = theirs.sessions.get("2026-08-01").expect("その日がある");
        assert_eq!(s.logs.len(), 1);
        assert_eq!(s.logs[0].exercise_id, bench);
    }

    /// ★ 部位違いの同名があっても (部位, 名前) が一意なら部位で解決する
    ///   （`resolve_exercise` 梯子 1）。
    #[test]
    fn tsv_import_resolves_a_preset_name_by_its_group() {
        let mut mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let mut g = ids();
        // プリセットと同名だが部位違いの自作種目（画面から作れてしまう）
        mine.exercises.push(Exercise {
            id: g.alloc(),
            name: "ベンチプレス".into(),
            group_id: crate::presets::preset_group_id("肩").expect("プリセット"),
            order: 9,
            archived: false,
            pins: Vec::new(),
            interval_sec: None,
            labels: Vec::new(),
        });

        let tsv =
            "日付\t部位\t種目\tセット\t重量kg\t回数\n2026-08-01\t胸\tベンチプレス\t1\t60\t10\n";
        let incoming = parse_import(tsv, &mut ids(), &mine).expect("読める");

        // (部位, 名前) が一意に当たるので、胸のプリセットに解決される
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        assert_eq!(incoming.exercises.len(), 1);
        assert_eq!(incoming.exercises[0].id, bench);
    }

    /// ★ **現状固定**。`pin_presets`（2454, 2465 の `if let [only]`）と違い TSV 経路には
    ///   ガードが無く、同じ部位に同名 2 件でもプリセット ID に寄る。仕様として
    ///   「曖昧なら新規に倒す」にするなら `resolve_exercise`（3798）と `merge_db` の
    ///   同名寄せ（4051）の両方に `exactly_one` が要る（別 PR）。そのときこのテストは
    ///   意図的に赤くなる。
    #[test]
    fn tsv_import_pins_the_preset_id_even_when_the_name_is_ambiguous_in_one_group() {
        let mut mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let mut g = ids();
        // プリセットと同名・同部位の自作種目（画面に重複名チェックが無いので作れる）
        mine.exercises.push(Exercise {
            id: g.alloc(),
            name: "ベンチプレス".into(),
            group_id: crate::presets::preset_group_id("胸").expect("プリセット"),
            order: 9,
            archived: false,
            pins: Vec::new(),
            interval_sec: None,
            labels: Vec::new(),
        });

        let tsv =
            "日付\t部位\t種目\tセット\t重量kg\t回数\n2026-08-01\t胸\tベンチプレス\t1\t60\t10\n";
        let incoming = parse_import(tsv, &mut ids(), &mine).expect("読める");

        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        assert_eq!(incoming.exercises.len(), 1);
        assert_eq!(
            incoming.exercises[0].id, bench,
            "同部位に同名2件でもプリセットIDに寄る"
        );

        merge_db(&mut mine, incoming);
        let s = mine.sessions.get("2026-08-01").expect("その日がある");
        assert_eq!(s.logs.len(), 1);
        assert_eq!(
            s.logs[0].exercise_id, bench,
            "ログはプリセットのベンチプレスに付く"
        );
    }

    /// ★ 部位違いの同名種目（画面に重複名チェックが無いので作れる）が 1 本に潰れない。
    ///   キャッシュを種目名だけで引くと、**自分の書き出しを読み戻すだけで**肩のセットが
    ///   胸のログに移る。
    #[test]
    fn tsv_round_trips_two_exercises_that_share_a_name_across_groups() {
        let mut mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let mut g = ids();
        let shoulder_bench = g.alloc();
        mine.exercises.push(Exercise {
            id: shoulder_bench,
            name: "ベンチプレス".into(),
            group_id: crate::presets::preset_group_id("肩").expect("プリセット"),
            order: 9,
            archived: false,
            pins: Vec::new(),
            interval_sec: None,
            labels: Vec::new(),
        });
        let chest_bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        let set = |weight, reps| SetEntry {
            weight,
            reps,
            ..Default::default()
        };
        mine.sessions.insert(
            date_key(d(2026, 8, 1)),
            Session {
                logs: vec![
                    ExerciseLog {
                        exercise_id: chest_bench,
                        sets: vec![set(60.0, 10)],
                        at: None,
                        note: String::new(),
                        label: None,
                    },
                    ExerciseLog {
                        exercise_id: shoulder_bench,
                        sets: vec![set(20.0, 12)],
                        at: None,
                        note: String::new(),
                        label: None,
                    },
                ],
                body_weight: None,
                note: String::new(),
            },
        );

        let tsv = export_tsv(&mine, jst(), crate::i18n::Lang::Ja);
        let incoming = parse_import(&tsv, &mut ids(), &mine).expect("読める");
        let report = merge_db(&mut mine, incoming);

        assert!(report.is_noop(), "自分の書き出しで記録が動いた: {report:?}");
        assert!(report.conflicts.is_empty(), "{:?}", report.conflicts);
        let s = mine.sessions.get("2026-08-01").expect("その日がある");
        assert_eq!(s.logs.len(), 2, "同名の 2 種目が 1 本に潰れた");
        assert_eq!(s.log_of(chest_bench).expect("胸").sets, vec![set(60.0, 10)]);
        assert_eq!(
            s.log_of(shoulder_bench).expect("肩").sets,
            vec![set(20.0, 12)]
        );
    }

    /// ★ `8/1/26` を「西暦 8 年」として通さない。通すと `parse_date_key` が往復して
    ///   保存まで抜け、2000 年前の記録としてカレンダーに残り二度と見つけられない。
    #[test]
    fn tsv_import_rejects_a_two_digit_year_instead_of_inventing_one() {
        let mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let tsv = "日付\t部位\t種目\tセット\t重量kg\t回数\n8/1/26\t胸\tベンチプレス\t1\t60\t10\n";
        assert_eq!(
            parse_import(tsv, &mut ids(), &mine),
            Err(ImportError::Unreadable)
        );
    }

    /// ★ 行は読めたのにセッションが 1 日も残らなかったら**失敗にする**。
    ///   種目だけ拾えていると `Ok` になり、画面が「増えるものはありません」と出して、
    ///   全部落ちたことに気づけないまま終わる。
    #[test]
    fn tsv_import_says_so_when_every_row_failed_to_parse() {
        let mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        // 日付がロケール書き戻しで全滅
        let bad_dates =
            "日付\t部位\t種目\tセット\t重量kg\t回数\nAugust 1, 2026\t胸\tベンチプレス\t1\t60\t10\n";
        assert_eq!(
            parse_import(bad_dates, &mut ids(), &mine),
            Err(ImportError::Unreadable)
        );
    }

    /// ★ **種目メモがあると上のテストをすり抜ける経路**を塞いだことの網。
    ///   `parse_tsv` は回数を読む**前に**ログを push して `append_note` するので、
    ///   種目メモの列に値があると回数が全滅してもログがメモで生き残る。
    ///   `dedupe_logs` は「セットもメモも無い」ログしか落とさないので `sessions` が
    ///   非空になり、`sessions.is_empty()` を見る判定を素通りしていた。
    ///   コピーがメモを全記録日に載せるようになった（adr/ux/copy-carries-the-notes.md）
    ///   ので、これが例外ではなく既定の形になる。
    #[test]
    fn tsv_import_says_so_when_every_count_failed_even_with_a_note() {
        let mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        // 回数が「10回」で全滅。種目メモだけは読める
        let tsv = "日付\t部位\t種目\tセット\t重量kg\t回数\tセットメモ\t種目メモ\n                   2026-08-01\t胸\tベンチプレス\t1\t60\t10回\t軽い\tセーフティ 2 穴目\n";
        assert_eq!(
            parse_import(tsv, &mut ids(), &mine),
            Err(ImportError::Unreadable),
            "セットが 1 本も読めていないなら、メモが残っていても失敗と言う"
        );
    }

    /// ★ 上の枝が**空集合に対して真になっていない**ことの網。`all` だけで書くと
    ///   ログが 0 本のファイル（体重だけの日）で真になり、壊れた行 1 つで**正当な
    ///   記録ごと拒否される**。ここが落ちたら `any(|s| !s.logs.is_empty())` が消えている。
    #[test]
    fn tsv_import_keeps_a_body_weight_file_even_when_one_row_is_unreadable() {
        let mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        // 体重だけの日は正当。もう 1 行は日付がロケール書き戻しで壊れている
        let tsv = "日付\t部位\t種目\tセット\t重量kg\t回数\t体重kg\n\
                   2026-08-01\t\t\t\t\t\t70.5\n\
                   August 2, 2026\t胸\tベンチプレス\t1\t60\t10\t\n";
        let db = parse_import(tsv, &mut ids(), &mine).expect("体重の行は読めているので通す");
        assert_eq!(
            db.sessions.get("2026-08-01").expect("その日").body_weight,
            Some(70.5)
        );
    }

    /// ★ 上の枝が**正当なファイルを巻き込んでいない**ことの網。メモだけの記録
    ///   （「肩が痛いのでやめた」）は `export_tsv` がセット無しの行として書き出す形で、
    ///   読めなかったセルは 1 つも無い。ここが落ちるなら枝の条件が広すぎる。
    #[test]
    fn tsv_import_keeps_a_file_that_only_holds_notes_and_reads_every_cell() {
        let mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let tsv = "日付\t部位\t種目\tセット\t重量kg\t回数\tセットメモ\t種目メモ\n                   2026-08-01\t胸\tベンチプレス\t\t\t\t\t肩が痛いのでやめた\n";
        let db = parse_import(tsv, &mut ids(), &mine).expect("読める");
        let log = &db.sessions.get("2026-08-01").expect("その日").logs[0];
        assert_eq!(log.note, "肩が痛いのでやめた");
        assert!(log.sets.is_empty());
    }

    /// ★ シートが数値列を `10.0` と書き戻してもセットが消えない。ここが落ちると
    ///   「シートで編集して戻す」という機能の主目的が成立しない。
    #[test]
    fn tsv_import_accepts_decimal_reps_and_set_numbers() {
        let mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let tsv = "日付\t部位\t種目\tセット\t重量kg\t回数\n\
                   2026-08-01\t胸\tベンチプレス\t2.0\t62.5\t8.0\n\
                   2026-08-01\t胸\tベンチプレス\t1.0\t60.0\t10.0\n";
        let db = parse_import(tsv, &mut ids(), &mine).expect("読める");
        let sets = &db.sessions.get("2026-08-01").expect("その日").logs[0].sets;
        assert_eq!(
            sets.iter().map(|s| (s.weight, s.reps)).collect::<Vec<_>>(),
            vec![(60.0, 10), (62.5, 8)]
        );
    }

    /// ★ TSV は `at` を持たないので、当日の記録に 1 セット足したファイルを取り込むと
    ///   `merge_db` の差し替えで実施時刻が消えていた。取り込む側が持たないなら残す。
    #[test]
    fn merging_a_stronger_log_without_a_clock_keeps_the_existing_one() {
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        let set = |reps| SetEntry {
            weight: 60.0,
            reps,
            ..Default::default()
        };
        let day = |sets: Vec<SetEntry>, at| Session {
            logs: vec![ExerciseLog {
                exercise_id: bench,
                sets,
                at,
                note: String::new(),
                label: None,
            }],
            body_weight: None,
            note: String::new(),
        };
        let mut mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        mine.sessions.insert(
            date_key(d(2026, 8, 1)),
            day(vec![set(10)], Some(1_800_000_000_000)),
        );
        let mut theirs = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        theirs
            .sessions
            .insert(date_key(d(2026, 8, 1)), day(vec![set(10), set(8)], None));

        merge_db(&mut mine, theirs);

        let log = &mine.sessions.get("2026-08-01").expect("その日").logs[0];
        assert_eq!(log.sets.len(), 2, "強いほうが採られていない");
        assert_eq!(log.at, Some(1_800_000_000_000), "実施時刻が消えた");
    }

    #[test]
    fn tsv_import_accepts_bom_crlf_and_a_trailing_newline() {
        let plain =
            "日付\t部位\t種目\tセット\t重量kg\t回数\n2026-08-01\t胸\tベンチプレス\t1\t60\t10\n";
        let crlf = plain.replace('\n', "\r\n");
        let bom = format!("\u{feff}{plain}");
        let no_eol = plain.trim_end_matches('\n').to_string();

        let mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let read = |raw: &str| parse_import(raw, &mut ids(), &mine).expect("読める");
        let base = read(plain);
        for variant in [crlf.as_str(), bom.as_str(), no_eol.as_str()] {
            assert_eq!(read(variant), base, "変種で結果が変わった: {variant:?}");
        }
    }

    /// ★ 列は名前で引く。位置で読むと「回数を重量として取り込む」が黙って通る。
    #[test]
    fn tsv_import_reads_columns_by_name_and_ignores_unknown_ones() {
        let shuffled =
            "メモ2\t回数\t種目\t重量kg\t部位\t日付\n無視\t10\tベンチプレス\t60\t胸\t2026-08-01\n";
        let mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let db = parse_import(shuffled, &mut ids(), &mine).expect("読める");
        let s = db.sessions.get("2026-08-01").expect("その日がある");
        assert_eq!(
            s.logs[0].sets,
            vec![SetEntry {
                weight: 60.0,
                reps: 10,
                ..Default::default()
            }]
        );
    }

    /// シートで行を並べ替えられてもセット順が戻る。
    #[test]
    fn tsv_import_restores_set_order_from_the_set_number() {
        let tsv = "日付\t部位\t種目\tセット\t重量kg\t回数\n\
                   2026-08-01\t胸\tベンチプレス\t3\t65\t6\n\
                   2026-08-01\t胸\tベンチプレス\t1\t60\t10\n\
                   2026-08-01\t胸\tベンチプレス\t2\t62.5\t8\n";
        let mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let db = parse_import(tsv, &mut ids(), &mine).expect("読める");
        let sets = &db.sessions.get("2026-08-01").expect("その日").logs[0].sets;
        assert_eq!(
            sets.iter().map(|s| s.reps).collect::<Vec<_>>(),
            vec![10, 8, 6]
        );
    }

    /// スプレッドシートが日付とロケール小数点を書き戻す形。
    #[test]
    fn tsv_import_accepts_slash_dates_and_comma_decimals() {
        let tsv =
            "日付\t部位\t種目\tセット\t重量kg\t回数\n2026/8/1\t胸\tベンチプレス\t1\t62,5\t8\n";
        let mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let db = parse_import(tsv, &mut ids(), &mine).expect("読める");
        let sets = &db.sessions.get("2026-08-01").expect("その日").logs[0].sets;
        assert_eq!(sets[0].weight, 62.5);
    }

    #[test]
    fn tsv_import_errors_tell_the_user_what_to_do_next() {
        let mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        // 見出しが無い（知っている列名も無い）→ うちのファイルではない
        assert_eq!(
            parse_import("あ\tい\nう\tえ\n", &mut ids(), &mine),
            Err(ImportError::NotDb)
        );
        // 知っている列名はあるが「種目」が無い → うちのファイルだが壊れている
        assert_eq!(
            parse_import("日付\t部位\t重量kg\n", &mut ids(), &mine),
            Err(ImportError::NoHeader)
        );
        // 見出しだけ
        assert_eq!(
            parse_import(
                "日付\t部位\t種目\tセット\t重量kg\t回数\n",
                &mut ids(),
                &mine
            ),
            Err(ImportError::NoRecords)
        );
    }

    /// ★ TSV を `repair` に通してはいけない（全角引用符の一括置換がメモを書き換える）。
    ///   分岐の順序をここで固定する。
    #[test]
    fn tsv_import_does_not_run_the_json_repair_on_notes() {
        let tsv = "日付\t部位\t種目\tセット\t重量kg\t回数\tセットメモ\n\
                   2026-08-01\t胸\tベンチプレス\t1\t60\t10\t\u{201c}軽い\u{201d}\n";
        let mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let db = parse_import(tsv, &mut ids(), &mine).expect("読める");
        let sets = &db.sessions.get("2026-08-01").expect("その日").logs[0].sets;
        assert_eq!(
            sets[0].note, "\u{201c}軽い\u{201d}",
            "引用符が書き換えられた"
        );
    }

    /// 部位が引けない新しい種目は取り込まない（「その他」部位を自動生成しない）。
    #[test]
    fn tsv_import_drops_a_new_exercise_without_a_group() {
        let tsv = "日付\t部位\t種目\tセット\t重量kg\t回数\n2026-08-01\t\t謎の種目\t1\t60\t10\n";
        let mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        assert_eq!(
            parse_import(tsv, &mut ids(), &mine),
            Err(ImportError::NoRecords)
        );
    }

    /// メモだけの行は「トレした日」にしない（`Session::is_trained` はセットだけを見る）。
    #[test]
    fn tsv_import_keeps_a_note_only_row_out_of_the_trained_days() {
        let tsv = "日付\t部位\t種目\tセット\t重量kg\t回数\t種目メモ\n\
                   2026-08-01\t胸\tベンチプレス\t\t\t\t肩が痛いのでやめた\n";
        let mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let db = parse_import(tsv, &mut ids(), &mine).expect("読める");
        let s = db.sessions.get("2026-08-01").expect("その日がある");
        assert!(!s.is_trained(), "セットが無いのに実施日になっている");
        assert_eq!(s.logs[0].note, "肩が痛いのでやめた");
    }

    /// 見た目が JSON でないものは TSV 側へ、JSON はこれまでどおり JSON 側へ。
    #[test]
    fn json_still_imports_after_the_format_dispatch() {
        let db = tsv_sample();
        let mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        assert_eq!(
            parse_import(&export_json(&db), &mut ids(), &mine).expect("JSON が読める"),
            db
        );
        // コードフェンス付き（貼り付け経路）も JSON 側に落ちる
        let fenced = format!("```json\n{}\n```", export_json(&db));
        assert_eq!(
            parse_import(&fenced, &mut ids(), &mine).expect("装飾を剥がして読める"),
            db
        );
    }

    // ── 数値の整形 / パース（`views` から移設）────────────────────────────────

    #[test]
    fn fmt_weight_drops_the_decimal_only_when_it_is_whole() {
        assert_eq!(fmt_weight(60.0), "60");
        assert_eq!(fmt_weight(62.5), "62.5");
        assert_eq!(fmt_weight(0.0), "0");
    }

    #[test]
    fn parse_weight_accepts_partial_input_and_refuses_nonsense() {
        assert_eq!(parse_weight("6."), 6.0);
        assert_eq!(parse_weight("62,5"), 62.5, "iOS のロケール小数点");
        assert_eq!(parse_weight(" 60 "), 60.0);
        assert_eq!(parse_weight("-1"), 0.0);
        assert_eq!(parse_weight("inf"), 0.0);
        assert_eq!(parse_weight(""), 0.0);
    }

    /// ★ `parse_reps` と非対称なのが要点。0 秒は「休まず次のセットへ」で、
    ///   落とすとスーパーセットの記録が黙って消える。
    #[test]
    fn parse_interval_treats_blank_as_unset_and_keeps_zero() {
        assert_eq!(parse_interval("90"), Some(90));
        assert_eq!(parse_interval(" 180 "), Some(180));
        assert_eq!(parse_interval("0"), Some(0), "0 秒は正当な入力");
        assert_eq!(parse_interval(""), None);
        assert_eq!(parse_interval("  "), None);
        assert_eq!(parse_interval("-1"), None);
        assert_eq!(parse_interval("1.5"), None, "秒は整数");
        assert_eq!(parse_interval("あ"), None);
        assert_eq!(parse_interval("1:30"), None, "分:秒 は受けない");
    }

    /// ★ 丸めるのは `clean_interval` の仕事。ここで丸めると規則が 2 本に割れる。
    #[test]
    fn parse_interval_does_not_clamp() {
        assert_eq!(parse_interval("5000"), Some(5000));
    }

    #[test]
    fn parse_reps_treats_zero_and_blank_as_no_row() {
        assert_eq!(parse_reps("10"), Some(10));
        assert_eq!(parse_reps("0"), None);
        assert_eq!(parse_reps(""), None);
        assert_eq!(parse_reps("あ"), None);
    }

    // ── 書き出し / 読み込み ──────────────────────────────────────────────────

    /// ★ この機能の生命線。書き出したものが、そのまま読み戻せる。
    #[test]
    fn export_round_trips_through_parse_import() {
        let mut db = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        db.sessions.insert(
            date_key(d(2026, 8, 8)),
            Session {
                logs: vec![ExerciseLog {
                    exercise_id: bench,
                    sets: vec![SetEntry {
                        weight: 60.0,
                        reps: 10,
                        ..Default::default()
                    }],
                    at: Some(1_800_000_000_000),
                    note: String::new(),
                    label: None,
                }],
                body_weight: Some(70.5),
                note: "調子よい".into(),
            },
        );

        let raw = export_json(&db);
        assert_eq!(
            parse_import(&raw, &mut ids(), &Db::default()).expect("読み戻せる"),
            db
        );
    }

    #[test]
    fn export_round_trips_the_routines_too() {
        // 書き出し形式 = 保存形式なので `export_json` に手は要らないが、
        // 「メニューが往復で消えない」は明示的に固定しておく
        let mut db = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        db.routines.push(Routine {
            id: r(1),
            name: "胸の日".into(),
            exercises: vec![bench],
        });

        let raw = export_json(&db);
        assert_eq!(
            parse_import(&raw, &mut ids(), &Db::default()).expect("読み戻せる"),
            db
        );
    }

    #[test]
    fn import_errors_tell_the_user_what_to_do_next() {
        assert_eq!(
            parse_import("", &mut ids(), &Db::default()),
            Err(ImportError::Empty)
        );
        assert_eq!(
            parse_import("   \n ", &mut ids(), &Db::default()),
            Err(ImportError::Empty)
        );
        // 途中で切れた
        assert_eq!(
            parse_import(r#"{"schema":3,"groups":"#, &mut ids(), &Db::default()),
            Err(ImportError::NotJson)
        );
        // JSON ではあるが別物
        assert_eq!(
            parse_import("[1,2,3]", &mut ids(), &Db::default()),
            Err(ImportError::NotDb)
        );
        assert_eq!(
            parse_import("null", &mut ids(), &Db::default()),
            Err(ImportError::NotDb)
        );
        assert_eq!(
            parse_import("{}", &mut ids(), &Db::default()),
            Err(ImportError::NotDb)
        );
        // 新しい版
        assert_eq!(
            parse_import(
                r#"{"schema":99,"groups":[],"exercises":[],"sessions":{}}"#,
                &mut ids(),
                &Db::default()
            ),
            Err(ImportError::Unsupported(99))
        );
    }

    #[test]
    fn import_repairs_decorations_added_by_copy_paste() {
        let db = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let raw = export_json(&db);

        for decorated in [
            format!("\u{feff}{raw}"),       // BOM
            format!("\n\n  {raw}  \n"),     // 前後の空白
            format!("```json\n{raw}\n```"), // コードフェンス
            format!("```\n{raw}\n```"),     // 言語指定なし
            raw.replace('"', "\u{201d}"),   // 全角引用符に化けた
        ] {
            assert_eq!(
                parse_import(&decorated, &mut ids(), &Db::default()).expect("修復して読める"),
                db
            );
        }
    }

    /// ★ `repair` は素のパースが失敗したときしか動かない。だから種目名に全角引用符が
    /// 入っていても壊さない。ここが崩れると、正常なデータを黙って書き換える。
    #[test]
    fn import_does_not_touch_valid_data_containing_curly_quotes() {
        let mut db = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        db.exercises.push(Exercise {
            id: ExerciseId::from_bits(0xD00D),
            name: "\u{201c}特別\u{201d}なベンチ".into(),
            group_id: crate::presets::preset_group_id("胸").expect("プリセット"),
            order: 9,
            archived: false,
            pins: Vec::new(),
            interval_sec: None,
            labels: Vec::new(),
        });

        let raw = export_json(&db);
        let back = parse_import(&raw, &mut ids(), &Db::default()).expect("読み戻せる");

        assert_eq!(back, db);
        assert!(back.exercises.iter().any(|e| e.name.contains('\u{201c}')));
    }

    #[test]
    fn export_filename_includes_the_time_so_a_second_export_does_not_clobber_the_first() {
        let at = |h, m| d(2026, 8, 8).and_hms_opt(h, m, 0).expect("有効な時刻");
        assert_eq!(export_filename(at(9, 5)), "fitness-memo-20260808-0905.tsv");
        assert_ne!(export_filename(at(9, 5)), export_filename(at(18, 30)));
    }

    #[test]
    fn summarize_counts_what_the_confirmation_screen_shows() {
        let mut db = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        db.sessions.insert(
            date_key(d(2026, 8, 1)),
            Session {
                logs: vec![ExerciseLog {
                    exercise_id: bench,
                    sets: vec![
                        SetEntry {
                            weight: 60.0,
                            reps: 10,
                            ..Default::default()
                        },
                        SetEntry {
                            weight: 60.0,
                            reps: 8,
                            ..Default::default()
                        },
                    ],
                    at: None,
                    note: String::new(),
                    label: None,
                }],
                ..Session::default()
            },
        );
        // 体重だけの日は「実施日」に数えない
        db.sessions.insert(
            date_key(d(2026, 8, 5)),
            Session {
                body_weight: Some(70.0),
                ..Session::default()
            },
        );

        let s = summarize(&db);
        assert_eq!(s.exercises, 28);
        assert_eq!(s.days, 1);
        assert_eq!(s.sets, 2);
        assert_eq!(s.first, Some(d(2026, 8, 1)));
        assert_eq!(s.last, Some(d(2026, 8, 1)));

        assert_eq!(summarize(&Db::default()), DbSummary::default());
    }

    #[test]
    fn summarize_does_not_count_a_note_only_day_as_trained() {
        // ★ `DbSummary` にメモの件数は足さない。取り込み事故を止めている 3 つの数
        //   （種目 / 実施日 / セット）が 1 行の中で薄まる。メモだけの DB は
        //   「0 日・0 セット」と出るので、置き換えようとした利用者には異常が見える
        let mut db = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        db.sessions.insert(
            date_key(d(2026, 8, 1)),
            Session {
                logs: vec![noted_log(
                    crate::presets::preset_exercise_id("ベンチプレス")
                        .expect("プリセット")
                        .bits(),
                    "肩が痛いのでやめた",
                    &[],
                    None,
                )],
                ..Session::default()
            },
        );

        let s = summarize(&db);
        assert_eq!(s.days, 0, "メモだけの日を実施日に数えない");
        assert_eq!(s.sets, 0);
        assert_eq!(s.first, None);
    }

    /// ★ 敵対的レビューで実証された全損経路の回帰テスト。
    ///
    /// `3.5e38` は f64 では有限なので serde が受理し、f32 に落として `inf` にする。
    /// `serde_json` はそれを**エラーではなく `"weight":null`** と書くので、保存は成功し、
    /// **次の起動で自分が書いた JSON を読めなくなる**（`Broken` → 退避 → 旧世代へ降格）。
    #[test]
    fn import_drops_weights_that_f32_cannot_represent() {
        let raw = r#"{
          "schema": 3, "groups": [], "exercises": [],
          "sessions": {
            "2026-08-08": { "logs": [
              {"exercise_id": "00000000000h", "sets": [
                {"weight": 3.5e38, "reps": 10},
                {"weight": 60.0, "reps": 8},
                {"weight": -5.0, "reps": 5}
              ]}
            ], "body_weight": 3.5e38 }
          }
        }"#;

        let db = parse_import(raw, &mut ids(), &Db::default()).expect("読める");

        let sets = &db.sessions["2026-08-08"].logs[0].sets;
        assert_eq!(sets.len(), 1, "表せない重量が残った: {sets:?}");
        assert_eq!(sets[0].weight, 60.0);
        assert_eq!(db.sessions["2026-08-08"].body_weight, None);

        // ★ 肝心なのは「書き戻せること」。ここが往復するなら次の起動で死なない
        let round =
            parse_import(&export_json(&db), &mut ids(), &Db::default()).expect("読み戻せる");
        assert_eq!(round, db);
    }

    // ── merge_db ────────────────────────────────────────────────────────────

    /// 「ログが指す種目名」の一覧。ID ではなく名前で見るので、参照が別物に
    /// 張り替わったら落ちる。
    fn log_names(db: &Db, date: NaiveDate) -> Vec<&str> {
        db.sessions
            .get(&date_key(date))
            .map(|s| {
                s.logs
                    .iter()
                    .map(|l| {
                        db.exercise(l.exercise_id)
                            .map_or("<宙に浮いたログ>", |e| e.name.as_str())
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 端末 A: プリセットに「わたしの種目」を足し、8/1 に記録。
    fn device_a() -> Db {
        let mut db = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        let chest = crate::presets::preset_group_id("胸").expect("プリセット");
        db.exercises.push(Exercise {
            id: ExerciseId::from_bits(0xAAA1),
            name: "わたしの種目".into(),
            group_id: chest,
            order: 9,
            archived: false,
            pins: Vec::new(),
            interval_sec: None,
            labels: Vec::new(),
        });
        db.sessions.insert(
            date_key(d(2026, 8, 1)),
            Session {
                logs: vec![
                    ExerciseLog {
                        exercise_id: bench,
                        sets: vec![SetEntry {
                            weight: 60.0,
                            reps: 10,
                            ..Default::default()
                        }],
                        at: None,
                        note: String::new(),
                        label: None,
                    },
                    ExerciseLog {
                        exercise_id: ExerciseId::from_bits(0xAAA1),
                        sets: vec![SetEntry {
                            weight: 20.0,
                            reps: 15,
                            ..Default::default()
                        }],
                        at: None,
                        note: String::new(),
                        label: None,
                    },
                ],
                body_weight: Some(70.0),
                note: "Aのメモ".into(),
            },
        );
        db
    }

    /// 端末 B: 同じプリセットから始め、**ベンチプレスを改名**し、A とは別の
    /// ユーザー種目を足して 8/2 に記録。
    ///
    /// ★ ここが回帰の要。A の「わたしの種目」と B の「べつの種目」は、連番採番なら
    /// **どちらも同じ番号**（プリセット 34 件の次）になる。ID で突き合わせると
    /// 8/1 の「わたしの種目」のログが「べつの種目」を指すようになる — この移行が
    /// 潰そうとしている壊れ方そのもの。
    fn device_b() -> Db {
        let mut db = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        db.exercises
            .iter_mut()
            .find(|e| e.id == bench)
            .expect("プリセットにある")
            .name = "ベンチプレス（スミス）".into();
        db.exercises.push(Exercise {
            id: ExerciseId::from_bits(0xBBB1),
            name: "べつの種目".into(),
            group_id: crate::presets::preset_group_id("背中").expect("プリセット"),
            order: 9,
            archived: false,
            pins: Vec::new(),
            interval_sec: None,
            labels: Vec::new(),
        });
        db.sessions.insert(
            date_key(d(2026, 8, 2)),
            Session {
                logs: vec![
                    ExerciseLog {
                        exercise_id: bench,
                        sets: vec![SetEntry {
                            weight: 65.0,
                            reps: 8,
                            ..Default::default()
                        }],
                        at: None,
                        note: String::new(),
                        label: None,
                    },
                    ExerciseLog {
                        exercise_id: ExerciseId::from_bits(0xBBB1),
                        sets: vec![SetEntry {
                            weight: 45.0,
                            reps: 6,
                            ..Default::default()
                        }],
                        at: None,
                        note: String::new(),
                        label: None,
                    },
                ],
                body_weight: None,
                note: String::new(),
            },
        );
        db
    }

    /// ★ この機能の本体。**旧 u32 連番方式ならこのテストは落ちる。**
    #[test]
    fn merge_preserves_the_exercise_behind_every_log() {
        let mut a = device_a();
        let before = log_names(&a, d(2026, 8, 1)).join(",");

        merge_db(&mut a, device_b());

        assert_eq!(
            log_names(&a, d(2026, 8, 1)).join(","),
            before,
            "元からあったログの指す種目が変わった"
        );
        // B 側のログも正しい種目を指す（ベンチは A 側の名前が残る）
        assert_eq!(
            log_names(&a, d(2026, 8, 2)),
            vec!["ベンチプレス", "べつの種目"]
        );
        // A と B のユーザー種目は別物として両方残る（連番なら 1 つに潰れる）
        assert_eq!(
            a.exercises
                .iter()
                .filter(|e| e.name == "わたしの種目" || e.name == "べつの種目")
                .count(),
            2
        );
        // 宙に浮いたログが 1 本も無い
        for session in a.sessions.values() {
            for l in &session.logs {
                assert!(a.exercise(l.exercise_id).is_some(), "宙に浮いたログ");
            }
        }
    }

    #[test]
    fn merge_does_not_duplicate_presets_across_independently_seeded_devices() {
        let mut a = device_a();
        let exercises_before = a.exercises.len();
        let groups_before = a.groups.len();

        let report = merge_db(&mut a, device_b());

        assert_eq!(report.groups_added, 0, "プリセットの部位が二重に入った");
        // 増えてよいのは B 側のユーザー種目 1 件だけ。プリセット 28 件は増えない
        assert_eq!(report.exercises_added, 1, "プリセットの種目が二重に入った");
        assert_eq!(a.groups.len(), groups_before);
        assert_eq!(a.exercises.len(), exercises_before + 1);
        assert_eq!(
            a.exercises.iter().filter(|e| e.id.is_reserved()).count(),
            28,
            "プリセット由来の種目が増減した"
        );
        // 改名は報告されるが、取り込み先の名前を残す
        assert!(report.conflicts.contains(&Conflict::Renamed {
            kept: "ベンチプレス".into(),
            incoming: "ベンチプレス（スミス）".into(),
        }));
    }

    #[test]
    fn merge_is_idempotent() {
        let mut a = device_a();
        merge_db(&mut a, device_b());
        let once = a.clone();

        let report = merge_db(&mut a, device_b());

        assert_eq!(a, once, "2 回目のマージが DB を変えた");
        assert!(report.is_noop(), "2 回目に何かを足した: {report:?}");
    }

    /// 集合としては可換。スカラーの衝突（体重・メモ・改名）は `mine` 優先なので
    /// 非対称だが、それは仕様（画面が `conflicts` を出す）。
    #[test]
    fn merge_is_commutative_as_a_set_of_records() {
        let mut ab = device_a();
        merge_db(&mut ab, device_b());
        let mut ba = device_b();
        merge_db(&mut ba, device_a());

        let shape = |db: &Db| {
            let mut days: Vec<String> = db.sessions.keys().cloned().collect();
            days.sort();
            let mut ids: Vec<u64> = db.exercises.iter().map(|e| e.id.bits()).collect();
            ids.sort_unstable();
            (days, ids, db.groups.len())
        };
        assert_eq!(shape(&ab), shape(&ba));
    }

    /// ★ セット数が同じで内容が違う場合。「多いほうを採る」だけでは勝者が決まらず、
    /// マージの向きで結果が変わってしまう。
    #[test]
    fn merge_breaks_set_ties_deterministically() {
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        let day = date_key(d(2026, 8, 1));
        let with = |sets: Vec<SetEntry>| {
            let mut db = crate::presets::seeded_db(crate::i18n::Lang::Ja);
            db.sessions.insert(
                day.clone(),
                Session {
                    logs: vec![ExerciseLog {
                        exercise_id: bench,
                        sets,
                        at: None,
                        note: String::new(),
                        label: None,
                    }],
                    ..Session::default()
                },
            );
            db
        };
        let light = || {
            vec![
                SetEntry {
                    weight: 60.0,
                    reps: 10,
                    ..Default::default()
                },
                SetEntry {
                    weight: 60.0,
                    reps: 8,
                    ..Default::default()
                },
            ]
        };
        let heavy = || {
            vec![
                SetEntry {
                    weight: 62.0,
                    reps: 10,
                    ..Default::default()
                },
                SetEntry {
                    weight: 60.0,
                    reps: 8,
                    ..Default::default()
                },
            ]
        };

        // どちらの向きから混ぜても、勝者は同じ（総ボリュームの大きいほう）
        let mut forward = with(light());
        merge_db(&mut forward, with(heavy()));
        let mut backward = with(heavy());
        merge_db(&mut backward, with(light()));

        assert_eq!(forward.sessions[&day].logs[0].sets, heavy());
        assert_eq!(backward.sessions[&day].logs[0].sets, heavy());
    }

    /// ★ 敵対的レビューで実証された経路の回帰テスト。
    ///
    /// 取り込み先で種目を改名しており、取り込む側に「元の名前の同じ種目」と
    /// 「新しい名前の別種目」の両方がある場合、写像は 2 つを同じ ID に落とす。
    /// そのまま同じ日に入れると同一 `exercise_id` のログが 2 本でき、
    /// **次回起動の `dedupe_logs` が別種目のセットを連結する**。
    #[test]
    fn merge_does_not_let_two_incoming_exercises_collapse_into_one_log() {
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        let chest = crate::presets::preset_group_id("胸").expect("プリセット");

        // mine: ベンチプレスを「マイベンチ」に改名済み
        let mut mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        mine.exercises
            .iter_mut()
            .find(|e| e.id == bench)
            .expect("プリセット")
            .name = "マイベンチ".into();

        // theirs: 同じ ID の「ベンチプレス」と、別 ID の「マイベンチ」を両方持つ
        let mut theirs = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let other = ExerciseId::from_bits(0xE001);
        theirs.exercises.push(Exercise {
            id: other,
            name: "マイベンチ".into(),
            group_id: chest,
            order: 9,
            archived: false,
            pins: Vec::new(),
            interval_sec: None,
            labels: Vec::new(),
        });
        theirs.sessions.insert(
            date_key(d(2026, 9, 9)),
            Session {
                logs: vec![
                    ExerciseLog {
                        exercise_id: bench,
                        sets: vec![SetEntry {
                            weight: 60.0,
                            reps: 10,
                            ..Default::default()
                        }],
                        at: None,
                        note: String::new(),
                        label: None,
                    },
                    ExerciseLog {
                        exercise_id: other,
                        sets: vec![SetEntry {
                            weight: 40.0,
                            reps: 12,
                            ..Default::default()
                        }],
                        at: None,
                        note: String::new(),
                        label: None,
                    },
                ],
                ..Session::default()
            },
        );

        merge_db(&mut mine, theirs);

        let logs = &mine.sessions[&date_key(d(2026, 9, 9))].logs;
        let mut seen: Vec<ExerciseId> = logs.iter().map(|l| l.exercise_id).collect();
        let total = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(
            seen.len(),
            total,
            "同じ日に同一 exercise_id のログが 2 本ある"
        );

        // 次回起動（= normalize）を通しても、別種目のセットが連結されない
        let reloaded =
            parse_import(&export_json(&mine), &mut ids(), &Db::default()).expect("読み戻せる");
        for log in &reloaded.sessions[&date_key(d(2026, 9, 9))].logs {
            assert!(
                log.sets.len() <= 1,
                "別種目のセットが連結された: {:?}",
                log.sets
            );
        }
    }

    /// メモを無条件に連結すると、同じファイルを 2 回入れて 2 倍になる。
    #[test]
    fn merge_does_not_append_the_same_note_twice() {
        let mut a = device_a();
        let mut b = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        b.sessions.insert(
            date_key(d(2026, 8, 1)),
            Session {
                logs: Vec::new(),
                body_weight: None,
                note: "Bのメモ".into(),
            },
        );

        merge_db(&mut a, b.clone());
        let after_first = a.sessions[&date_key(d(2026, 8, 1))].note.clone();
        merge_db(&mut a, b);

        assert_eq!(a.sessions[&date_key(d(2026, 8, 1))].note, after_first);
        assert_eq!(after_first, "Aのメモ\nBのメモ");
    }

    // ── トレーニングメニューのマージ ────────────────────────────────────────
    // adr/data-model/routines-as-named-exercise-lists.md
    //
    // ★ `device_a()` / `device_b()` は触らない。共有フィクスチャに手を入れると
    //   他の assertion（冪等性・可換性）に波及するので、専用の Db を組む。

    /// メニューだけを見るための最小の 2 台。プリセットの固定 ID を使うので
    /// 「独立に seed された 2 台」の前提がそのまま成り立つ。
    fn routine_devices() -> (Db, Db) {
        (
            crate::presets::seeded_db(crate::i18n::Lang::Ja),
            crate::presets::seeded_db(crate::i18n::Lang::Ja),
        )
    }

    fn preset(name: &str) -> ExerciseId {
        crate::presets::preset_exercise_id(name).expect("プリセット")
    }

    fn routine_names(db: &Db) -> Vec<&str> {
        db.routines.iter().map(|x| x.name.as_str()).collect()
    }

    #[test]
    fn merge_adds_a_routine_from_the_other_device() {
        let (mut mine, mut theirs) = routine_devices();
        theirs.routines.push(Routine {
            id: r(1),
            name: "胸の日".into(),
            exercises: vec![preset("ベンチプレス")],
        });

        let report = merge_db(&mut mine, theirs);
        assert_eq!(report.routines_added, 1);
        assert_eq!(routine_names(&mine), vec!["胸の日"]);
        assert!(!report.is_noop(), "メニューだけ増えたときも noop ではない");
    }

    #[test]
    fn merge_maps_the_exercise_ids_inside_an_incoming_routine() {
        // 取り込む側の「わたしの種目」は取り込み先の同名種目へ寄る。
        // メニューの中の参照もその写像を通らなければ、宙に浮いた種目を指すことになる
        let (mut mine, mut theirs) = routine_devices();
        let chest = crate::presets::preset_group_id("胸").expect("プリセット");
        let mine_id = ExerciseId::from_bits(0xAAA1);
        let theirs_id = ExerciseId::from_bits(0xBBB1);
        for (db, id) in [(&mut mine, mine_id), (&mut theirs, theirs_id)] {
            db.exercises.push(Exercise {
                id,
                name: "わたしの種目".into(),
                group_id: chest,
                order: 9,
                archived: false,
                pins: Vec::new(),
                interval_sec: None,
                labels: Vec::new(),
            });
        }
        theirs.routines.push(Routine {
            id: r(1),
            name: "胸の日".into(),
            exercises: vec![theirs_id],
        });

        merge_db(&mut mine, theirs);
        assert_eq!(
            mine.routines[0].exercises,
            vec![mine_id],
            "取り込み先の ID へ張り替わっていなければ宙に浮く"
        );
    }

    #[test]
    fn merge_does_not_let_two_incoming_exercises_collapse_into_one_routine_entry() {
        // 写像は単射とは限らない。潰れた結果が同じメニューに 2 回入ると、
        // 展開時に「1 日 1 種目 1 ログ」が破れる
        let (mut mine, mut theirs) = routine_devices();
        let chest = crate::presets::preset_group_id("胸").expect("プリセット");
        let bench = preset("ベンチプレス");
        // 取り込む側だけが持つ別 ID の「ベンチプレス」→ 同名で bench に寄る
        let dup = ExerciseId::from_bits(0xBBB2);
        theirs.exercises.push(Exercise {
            id: dup,
            name: "ベンチプレス".into(),
            group_id: chest,
            order: 9,
            archived: false,
            pins: Vec::new(),
            interval_sec: None,
            labels: Vec::new(),
        });
        theirs.routines.push(Routine {
            id: r(1),
            name: "胸の日".into(),
            exercises: vec![bench, dup],
        });

        merge_db(&mut mine, theirs);
        assert_eq!(mine.routines[0].exercises, vec![bench]);
    }

    #[test]
    fn merge_keeps_my_routine_when_the_same_id_holds_different_exercises() {
        let (mut mine, mut theirs) = routine_devices();
        mine.routines.push(Routine {
            id: r(1),
            name: "胸の日".into(),
            exercises: vec![preset("ベンチプレス")],
        });
        theirs.routines.push(Routine {
            id: r(1),
            name: "胸の日".into(),
            exercises: vec![preset("ベンチプレス"), preset("チェストフライ")],
        });

        let report = merge_db(&mut mine, theirs);
        assert_eq!(report.routines_added, 0);
        assert_eq!(
            mine.routines[0].exercises,
            vec![preset("ベンチプレス")],
            "取り込み先を残す（union にすると外した種目が毎回戻る）"
        );
        assert_eq!(
            report.conflicts,
            vec![Conflict::RoutineDiverged {
                name: "胸の日".into()
            }]
        );
    }

    #[test]
    fn merge_reports_a_divergence_when_a_shared_routine_id_was_renamed() {
        let (mut mine, mut theirs) = routine_devices();
        mine.routines.push(Routine {
            id: r(1),
            name: "胸の日".into(),
            exercises: vec![preset("ベンチプレス")],
        });
        theirs.routines.push(Routine {
            id: r(1),
            name: "プッシュの日".into(),
            exercises: vec![preset("ベンチプレス")],
        });

        let report = merge_db(&mut mine, theirs);
        assert_eq!(mine.routines[0].name, "胸の日");
        assert_eq!(
            report.conflicts,
            vec![Conflict::RoutineDiverged {
                name: "胸の日".into()
            }]
        );
    }

    #[test]
    fn merge_keeps_both_routines_that_only_share_a_name() {
        // ★ 種目とはここだけ規則が違う。メニューには履歴がぶら下がっていないので、
        //   寄せて外すと不可逆に消える。並べておけば 1 タップで消せる
        let (mut mine, mut theirs) = routine_devices();
        mine.routines.push(Routine {
            id: r(1),
            name: "胸の日".into(),
            exercises: vec![preset("ベンチプレス")],
        });
        theirs.routines.push(Routine {
            id: r(2),
            name: "胸の日".into(),
            exercises: vec![preset("チェストフライ")],
        });

        let report = merge_db(&mut mine, theirs);
        assert_eq!(report.routines_added, 1);
        assert_eq!(routine_names(&mine), vec!["胸の日", "胸の日"]);
        assert!(report.conflicts.is_empty(), "食い違いではなく別物");
    }

    #[test]
    fn merge_does_not_duplicate_a_routine_that_both_devices_built_identically() {
        // 2 台で同じものを作った / 同じファイルを別経路で 2 度入れた場合
        let (mut mine, mut theirs) = routine_devices();
        mine.routines.push(Routine {
            id: r(1),
            name: "胸の日".into(),
            exercises: vec![preset("ベンチプレス")],
        });
        theirs.routines.push(Routine {
            id: r(2),
            name: "胸の日".into(),
            exercises: vec![preset("ベンチプレス")],
        });

        let report = merge_db(&mut mine, theirs);
        assert_eq!(report.routines_added, 0);
        assert_eq!(mine.routines.len(), 1);
    }

    #[test]
    fn merge_of_routines_is_idempotent() {
        let (mut mine, mut theirs) = routine_devices();
        theirs.routines.push(Routine {
            id: r(1),
            name: "胸の日".into(),
            exercises: vec![preset("ベンチプレス")],
        });

        merge_db(&mut mine, theirs.clone());
        let again = merge_db(&mut mine, theirs);
        assert_eq!(again.routines_added, 0);
        assert_eq!(mine.routines.len(), 1);
        assert!(again.conflicts.is_empty());
    }

    // ── メモのマージ（adr/data-model/notes-on-logs-and-sets.md）─────────────────

    /// 種目メモ・セットメモを載せた 2 台を作る。セットの内容は引数で変える。
    fn noted_pair(mine_sets: &[(f32, u32, &str)], theirs_sets: &[(f32, u32, &str)]) -> (Db, Db) {
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        let day = date_key(d(2026, 8, 1));
        let build = |note: &str, sets: &[(f32, u32, &str)]| {
            let mut db = crate::presets::seeded_db(crate::i18n::Lang::Ja);
            db.sessions.insert(
                day.clone(),
                Session {
                    logs: vec![ExerciseLog {
                        exercise_id: bench,
                        sets: sets
                            .iter()
                            .map(|(w, r, n)| SetEntry {
                                weight: *w,
                                reps: *r,
                                note: n.to_string(),
                                ..Default::default()
                            })
                            .collect(),
                        at: None,
                        note: note.to_string(),
                        label: None,
                    }],
                    ..Session::default()
                },
            );
            db
        };
        (
            build("わたしのメモ", mine_sets),
            build("あちらのメモ", theirs_sets),
        )
    }

    /// `(重量, 回数, メモ, 段)` でセットを組む取り込みペア。種目メモは付けない
    /// （段だけが増えたマージを見たいので、メモの数を混ぜたくない）。
    #[expect(clippy::type_complexity, reason = "テストのフィクスチャ")]
    fn dropped_pair(
        mine_sets: &[(f32, u32, &str, &[(f32, u32)])],
        theirs_sets: &[(f32, u32, &str, &[(f32, u32)])],
    ) -> (Db, Db) {
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        let day = date_key(d(2026, 8, 1));
        let build = |sets: &[(f32, u32, &str, &[(f32, u32)])]| {
            let mut db = crate::presets::seeded_db(crate::i18n::Lang::Ja);
            db.sessions.insert(
                day.clone(),
                Session {
                    logs: vec![ExerciseLog {
                        exercise_id: bench,
                        sets: sets
                            .iter()
                            .map(|(w, r, n, stages)| SetEntry {
                                weight: *w,
                                reps: *r,
                                note: n.to_string(),
                                drops: stages
                                    .iter()
                                    .map(|(dw, dr)| DropStage {
                                        weight: *dw,
                                        reps: *dr,
                                    })
                                    .collect(),
                            })
                            .collect(),
                        label: None,
                        at: None,
                        note: String::new(),
                    }],
                    ..Session::default()
                },
            );
            db
        };
        (build(mine_sets), build(theirs_sets))
    }

    fn merged_log(db: &Db) -> &ExerciseLog {
        &db.sessions[&date_key(d(2026, 8, 1))].logs[0]
    }

    #[test]
    fn merge_appends_the_exercise_note_even_when_the_sets_are_identical() {
        // ★ セット一致で早期 continue する枝が、取り込む側のメモを見ていなかった
        let (mut mine, theirs) = noted_pair(&[(60.0, 10, "")], &[(60.0, 10, "")]);

        let report = merge_db(&mut mine, theirs);

        assert_eq!(merged_log(&mine).note, "わたしのメモ\nあちらのメモ");
        assert_eq!(report.notes_added, 1);
    }

    #[test]
    fn merge_fills_set_notes_positionally_when_the_sets_match() {
        let (mut mine, theirs) = noted_pair(
            &[(60.0, 10, ""), (60.0, 8, "自分の 2 本目")],
            &[(60.0, 10, "あちらの 1 本目"), (60.0, 8, "")],
        );

        merge_db(&mut mine, theirs);

        let sets = &merged_log(&mine).sets;
        assert_eq!(sets[0].note, "あちらの 1 本目", "空いていた側は埋まる");
        assert_eq!(sets[1].note, "自分の 2 本目", "自分のメモは残る");
    }

    #[test]
    fn merge_does_not_touch_set_notes_when_the_sets_diverge() {
        // 並びが食い違うときに位置で埋めると、別のセットにメモが付く
        let (mut mine, theirs) = noted_pair(
            &[(60.0, 10, "自分のだけ")],
            &[(62.0, 10, "あちらの 1"), (62.0, 8, "あちらの 2")],
        );

        merge_db(&mut mine, theirs);

        let sets = &merged_log(&mine).sets;
        // セットは取り込む側が強いので差し替わる。**そのメモも一緒に来る**
        assert_eq!(sets.len(), 2);
        assert_eq!(sets[0].note, "あちらの 1");
        assert_eq!(sets[1].note, "あちらの 2");
    }

    #[test]
    fn merge_keeps_my_set_order_when_the_other_side_is_the_same_sets_reordered() {
        // ★ セットの D&D（adr/ux/drag-to-reorder-in-record-tab.md）が開けた穴の回帰。
        //   same_sets は位置で比べるので、並べ替えただけの同じ記録が食い違い扱いになり、
        //   log_rank の位置依存な辞書順で勝ち負けが決まって**こちらのセットメモが消える**
        let (mut mine, theirs) = noted_pair(
            &[(60.0, 10, "1本目"), (62.5, 8, "2本目"), (65.0, 6, "3本目")],
            &[(65.0, 6, ""), (60.0, 10, ""), (62.5, 8, "")],
        );

        let report = merge_db(&mut mine, theirs);

        let sets = &merged_log(&mine).sets;
        assert_eq!(
            sets.iter().map(|s| s.reps).collect::<Vec<_>>(),
            vec![10, 8, 6],
            "並びは端末ごとの好みなので、取り込み先のものを残す"
        );
        assert_eq!(
            sets.iter().map(|s| s.note.as_str()).collect::<Vec<_>>(),
            vec!["1本目", "2本目", "3本目"],
            "セットメモが行から剥がれていない"
        );
        assert!(
            report.conflicts.is_empty(),
            "食い違っていないものを食い違いとして報告した: {report:?}"
        );
    }

    #[test]
    fn merge_still_takes_set_notes_from_a_reordered_other_device() {
        // ★ ここを捨てると、片方の端末で 1 度並べ替えただけで**もう片方のセットメモが
        //   二度と合流しなくなる**（same_sets の枝に二度と入らないため）
        let (mut mine, theirs) = noted_pair(
            &[(60.0, 10, ""), (62.5, 8, "自分の 2 本目"), (65.0, 6, "")],
            &[
                (65.0, 6, "あちらの 3 本目"),
                (60.0, 10, "あちらの 1 本目"),
                (62.5, 8, "あちらの 2 本目"),
            ],
        );

        let report = merge_db(&mut mine, theirs);

        let sets = &merged_log(&mine).sets;
        assert_eq!(
            sets.iter().map(|s| s.reps).collect::<Vec<_>>(),
            vec![10, 8, 6],
            "並びは取り込み先のまま"
        );
        assert_eq!(
            sets.iter().map(|s| s.note.as_str()).collect::<Vec<_>>(),
            vec![
                "あちらの 1 本目",
                "自分の 2 本目\nあちらの 2 本目",
                "あちらの 3 本目",
            ],
            "重量・回数が同じセット同士でメモが合流する"
        );
        assert_eq!(report.notes_added, 4, "セットメモ 3 本 + 種目メモ 1 本");
        assert!(
            report.conflicts.is_empty(),
            "食い違っていないものを食い違いとして報告した: {report:?}"
        );
        assert_eq!(
            merged_log(&mine).note,
            "わたしのメモ\nあちらのメモ",
            "種目メモの合流は今までどおり効く"
        );
    }

    /// ★ 段を識別から外した以上、合流の一手が無いと他端末で入れた段が
    ///   `Conflict` も出さずに黙って消える。並びが同じ枝（`same_sets`）。
    #[test]
    fn merge_carries_the_drop_stages_when_the_order_matches() {
        let (mut mine, theirs) = dropped_pair(
            &[(60.0, 10, "", &[]), (60.0, 6, "", &[])],
            &[(60.0, 10, "", &[]), (60.0, 6, "", &[(50.0, 5)])],
        );

        let report = merge_db(&mut mine, theirs);

        assert_eq!(
            merged_log(&mine)
                .sets
                .iter()
                .map(|s| s.drops.len())
                .collect::<Vec<_>>(),
            vec![0, 1]
        );
        assert_eq!(report.drops_added, 1);
        assert!(
            !report.is_noop(),
            "段だけが増えたマージを no-op と言っている"
        );
        assert!(report.conflicts.is_empty(), "{report:?}");
    }

    /// ★ 並べ替えただけの枝（`same_sets_unordered`）でも運ぶ。ここで落とすと、
    ///   片方の端末で 1 度並べ替えるだけで段が二度と合流しなくなる。
    #[test]
    fn merge_carries_the_drop_stages_when_only_the_order_differs() {
        let (mut mine, theirs) = dropped_pair(
            &[(60.0, 10, "", &[]), (60.0, 6, "", &[])],
            &[(60.0, 6, "", &[(50.0, 5)]), (60.0, 10, "", &[])],
        );

        let report = merge_db(&mut mine, theirs);

        assert_eq!(
            merged_log(&mine)
                .sets
                .iter()
                .map(|s| (s.reps, s.drops.len()))
                .collect::<Vec<_>>(),
            vec![(10, 0), (6, 1)],
            "取り込み先の並びのまま段だけが乗る"
        );
        assert_eq!(report.drops_added, 1);
    }

    /// ★ **両方に段があって中身が違うときは取り込み先を残す。** `append_note` の
    ///   「同じ文が既に入っていれば足さない」と同じ、足さない側に倒す判断。
    ///   `Conflict` も出さない（出すと同じファイルを 2 回入れるたびに報告する）。
    #[test]
    fn merge_keeps_my_stages_when_both_sides_have_some() {
        let (mut mine, theirs) = dropped_pair(
            &[(60.0, 6, "", &[(50.0, 5)])],
            &[(60.0, 6, "", &[(45.0, 8), (35.0, 6)])],
        );

        let report = merge_db(&mut mine, theirs);

        assert_eq!(
            merged_log(&mine).sets[0].drops,
            vec![DropStage {
                weight: 50.0,
                reps: 5
            }],
            "取り込む側の段で上書きしている"
        );
        assert_eq!(report.drops_added, 0);
        assert!(report.conflicts.is_empty(), "{report:?}");
    }

    /// ★ **合流を素朴に書くと、同じデータを初めて取り込んだだけで段が増える。**
    ///
    /// 「段付きの相手を、同じ重量・回数の未使用な先頭に入れる」1 段だけの実装だと、
    /// あちらの段付き `(60,10)` が**段の無い** `mine[0]` に当たって段が 2 セットに付く。
    /// `merge_set_drops_unordered` が先に「同じ段を既に持っている相手」へ寄せることで
    /// 起きなくなる。同じ重量・回数の行が並んでいる記録は珍しくない。
    #[test]
    fn merging_the_same_data_does_not_invent_a_drop_stage() {
        let stages: &[(f32, u32)] = &[(50.0, 5)];
        let (mut mine, theirs) = dropped_pair(
            &[
                (60.0, 10, "A", &[]),
                (60.0, 10, "", stages),
                (70.0, 5, "", &[]),
            ],
            &[
                (70.0, 5, "", &[]),
                (60.0, 10, "", stages),
                (60.0, 10, "A", &[]),
            ],
        );

        let report = merge_db(&mut mine, theirs);

        assert_eq!(
            merged_log(&mine)
                .sets
                .iter()
                .map(|s| s.drops.len())
                .collect::<Vec<_>>(),
            vec![0, 1, 0],
            "無関係なセットに段を付けている"
        );
        assert_eq!(report.drops_added, 0);
        assert_eq!(report.notes_added, 0, "メモも二重に付いてはいけない");
        assert!(report.is_noop(), "同じデータなのに何かを取り込んでいる");
    }

    /// ★ 自分のファイルを何度取り込んでも段が増えない（冪等）。
    #[test]
    fn importing_the_same_drop_stages_twice_adds_nothing() {
        let db = drop_sample();
        let tsv = export_tsv(&db, jst(), crate::i18n::Lang::Ja);
        let mut mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);

        for round in 1..=2 {
            let incoming = parse_import(&tsv, &mut ids(), &mine).expect("読み戻せる");
            let report = merge_db(&mut mine, incoming);
            if round == 2 {
                assert_eq!(report.drops_added, 0, "2 回目で段が増えている");
                assert!(report.is_noop(), "2 回目が no-op でない: {report:?}");
            }
        }
        assert_eq!(
            mine.sessions["2026-08-01"].logs[0]
                .sets
                .iter()
                .filter(|s| !s.drops.is_empty())
                .count(),
            1
        );
    }

    #[test]
    fn merge_still_reports_a_divergence_when_the_sets_really_differ() {
        // 本数が同じで中身が違うときまで「並べ替えただけ」に見えては困る
        let (mut mine, theirs) = noted_pair(
            &[(60.0, 10, ""), (60.0, 8, "")],
            &[(62.0, 10, ""), (62.0, 8, "")],
        );

        let report = merge_db(&mut mine, theirs);

        assert_eq!(merged_log(&mine).sets[0].weight, 62.0, "強いほうを採る");
        assert!(
            report
                .conflicts
                .iter()
                .any(|c| matches!(c, Conflict::SetsDiverged { .. }))
        );
    }

    #[test]
    fn merge_keeps_my_exercise_note_when_the_incoming_sets_win() {
        // ★ `*existing = log` だけだと、セットが負けたせいで取り込み先のメモまで消える
        let (mut mine, theirs) = noted_pair(&[(60.0, 10, "")], &[(62.0, 10, ""), (62.0, 8, "")]);

        let report = merge_db(&mut mine, theirs);

        let log = merged_log(&mine);
        assert_eq!(log.sets.len(), 2, "強いほうのセットを採る");
        assert_eq!(
            log.note, "わたしのメモ\nあちらのメモ",
            "セットが負けても種目メモは失わない"
        );
        assert!(
            report
                .conflicts
                .iter()
                .any(|c| matches!(c, Conflict::SetsDiverged { .. }))
        );
    }

    #[test]
    fn merge_does_not_silently_drop_a_note_when_only_the_notes_differ() {
        // ★ `==` のままだと rank が同点で下の分岐にも入らず、メモが黙って消えていた
        let (mut mine, theirs) = noted_pair(&[(60.0, 10, "")], &[(60.0, 10, "あちらのセットメモ")]);

        merge_db(&mut mine, theirs);

        assert_eq!(merged_log(&mine).sets[0].note, "あちらのセットメモ");
    }

    #[test]
    fn merge_reports_notes_added_so_the_screen_does_not_claim_nothing_happened() {
        // メモだけが増えたマージで is_noop が真になると、画面が
        // 「新しく取り込むものはありませんでした」と嘘をつく
        let (mut mine, theirs) = noted_pair(&[(60.0, 10, "")], &[(60.0, 10, "あちらのセットメモ")]);

        let report = merge_db(&mut mine, theirs);

        assert!(!report.is_noop(), "メモが増えたのに noop 扱い: {report:?}");
        assert_eq!(report.logs_added, 0, "ログは増えていない");
        assert_eq!(report.notes_added, 2, "種目メモとセットメモの 2 本");
    }

    #[test]
    fn merging_notes_twice_adds_nothing_the_second_time() {
        let (mut mine, theirs) = noted_pair(
            &[(60.0, 10, "自分の")],
            &[(60.0, 10, "あちらの"), (60.0, 8, "あちらの 2")],
        );

        merge_db(&mut mine, theirs.clone());
        let once = mine.clone();
        let report = merge_db(&mut mine, theirs);

        assert_eq!(mine, once, "2 回目のマージが DB を変えた");
        assert_eq!(report.notes_added, 0, "同じメモを 2 回足した");
        assert!(report.is_noop(), "2 回目に何かを足した: {report:?}");
    }

    /// 取り込む側にしか無い日は、ログごと採用する。**そのログも写像を通っている。**
    #[test]
    fn merge_maps_ids_even_for_days_taken_wholesale() {
        let mut a = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        // A には「じぶんの種目」を ID X で持たせる
        a.exercises.push(Exercise {
            id: ExerciseId::from_bits(0xB001),
            name: "じぶんの種目".into(),
            group_id: crate::presets::preset_group_id("胸").expect("プリセット"),
            order: 9,
            archived: false,
            pins: Vec::new(),
            interval_sec: None,
            labels: Vec::new(),
        });

        // B は同名の種目を**別の ID** で持ち、A に無い日に記録している
        let mut b = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        b.exercises.push(Exercise {
            id: ExerciseId::from_bits(0xC002),
            name: "じぶんの種目".into(),
            group_id: crate::presets::preset_group_id("胸").expect("プリセット"),
            order: 9,
            archived: false,
            pins: Vec::new(),
            interval_sec: None,
            labels: Vec::new(),
        });
        b.sessions.insert(
            date_key(d(2026, 9, 9)),
            Session {
                logs: vec![ExerciseLog {
                    exercise_id: ExerciseId::from_bits(0xC002),
                    sets: vec![SetEntry {
                        weight: 40.0,
                        reps: 12,
                        ..Default::default()
                    }],
                    at: None,
                    note: String::new(),
                    label: None,
                }],
                ..Session::default()
            },
        );

        merge_db(&mut a, b);

        // 名前で同一視され、A 側の ID に張り替わっている
        let logs = &a.sessions[&date_key(d(2026, 9, 9))].logs;
        assert_eq!(logs[0].exercise_id, ExerciseId::from_bits(0xB001));
        assert_eq!(log_names(&a, d(2026, 9, 9)), vec!["じぶんの種目"]);
        assert_eq!(
            a.exercises
                .iter()
                .filter(|e| e.name == "じぶんの種目")
                .count(),
            1,
            "同名の種目が 2 つに増えた"
        );
    }

    // ── マシンのピン（adr/ux/machine-pins-on-the-exercise.md）──────────────────

    /// 取り込んだ JSON がどんな形でも、`Vec` の 1 要素は「空白を含まない 1 個の値」に
    /// なる。TSV の `ピン` 列がセル内を空白で区切るので、ここが崩れると書き出して
    /// 読み戻した値が一致しなくなる。
    #[test]
    fn normalize_splits_a_pin_that_contains_whitespace() {
        assert_eq!(clean_pins(vec!["3 5".into()]), ["3", "5"]);
        assert_eq!(clean_pins(vec!["  7  ".into()]), ["7"]);
        assert_eq!(clean_pins(vec!["3\t5\n2".into()]), ["3", "5", "2"]);
    }

    #[test]
    fn normalize_drops_blank_pins() {
        assert_eq!(clean_pins(vec!["".into(), " ".into(), "3".into()]), ["3"]);
        assert!(clean_pins(vec!["".into(), "　".into()]).is_empty());
    }

    /// 上限は UI の制限ではなく**取り込みのための門番**（`drop_unrepresentable_weights`
    /// と同じ立場）。★ 数えるのは分割の**後** — `"1 2 3 …"` が 1 要素で入りうる。
    #[test]
    fn normalize_caps_the_number_of_pins() {
        let many: Vec<String> = (0..MAX_PINS + 5).map(|i| i.to_string()).collect();
        assert_eq!(clean_pins(many).len(), MAX_PINS);
        assert_eq!(
            clean_pins(vec!["1 2 3 4 5 6 7 8 9 10 11".into()]).len(),
            MAX_PINS
        );
    }

    /// ★ char で切る。バイトで切ると UTF-8 の途中で割れて panic する。
    #[test]
    fn normalize_truncates_a_long_pin_by_chars_not_bytes() {
        assert_eq!(
            clean_pins(vec!["あいうえおかきくけこ".into()]),
            ["あいうえおか"]
        );
    }

    /// 「シート高 3 / バー位置 3」は正当な入力。`normalize_routines` が `ExerciseId` の
    /// 重複を消すのとは前提が違う（あちらは消さないと「1 日 1 種目 1 ログ」が破れる）。
    #[test]
    fn normalize_keeps_duplicate_pins() {
        assert_eq!(clean_pins(vec!["3".into(), "3".into()]), ["3", "3"]);
    }

    /// `Vec` の順が「上から下へ触る順」そのもの。並べ替えたら別のマシン設定になる。
    #[test]
    fn normalize_keeps_the_pin_order() {
        assert_eq!(
            clean_pins(vec!["9".into(), "1".into(), "5".into()]),
            ["9", "1", "5"]
        );
    }

    /// 画面からの書き込みと取り込みで規則が食い違うと、**画面で消えたはずの値が
    /// 取り込みで生き返る**（またはその逆）。
    #[test]
    fn set_pins_normalizes_the_same_way_as_normalize() {
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        let raw: Vec<String> = vec!["  3 5 ".into(), "".into(), "あいうえおかきく".into()];

        let mut through_ui = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        set_pins(&mut through_ui, bench, raw.clone());

        let mut through_import = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        if let Some(e) = through_import.exercises.iter_mut().find(|e| e.id == bench) {
            e.pins = raw;
        }
        normalize_exercises(&mut through_import, &mut IdGen::from_seed(1));

        assert_eq!(
            through_ui.exercise(bench).expect("種目").pins,
            ["3", "5", "あいうえおか"]
        );
        assert_eq!(
            through_ui.exercise(bench).expect("種目").pins,
            through_import.exercise(bench).expect("種目").pins,
            "画面からの書き込みと取り込みで規則がずれている"
        );
    }

    /// ★ ピンを足しても schema は上げない（`model::SCHEMA` の doc）。上げると旧版が
    /// `RestoreError::Unsupported` で退避し、前方互換を積極的に壊す側になる。
    #[test]
    fn migrate_does_not_bump_the_schema_for_pins() {
        let db = migrate(
            r#"{"schema":3,"groups":[],"exercises":[{"id":"000000000001","name":"自作マシン","group_id":"000000000002","order":0,"archived":false,"pins":["3 5"]}],"sessions":{}}"#,
            &mut ids(),
        )
        .expect("読める");

        assert_eq!(db.schema, SCHEMA);
        assert_eq!(SCHEMA, 3, "ピンの追加で schema を上げてはいけない");
        assert_eq!(
            db.exercises[0].pins,
            ["3", "5"],
            "migrate が normalize_exercises を通っていない"
        );
    }

    /// ★ **これが落ちると機能そのものが成立しない。**
    ///
    /// 新品端末へ書き出しを戻す経路は「プリセットは固定 ID を持つので必ず ID 一致
    /// する」（`presets::preset_exercise_id`、`resolve_exercise` の梯子 3）を通る。
    /// ID 一致の枝が取り込み側のピンを見ないと **機種変更でピンだけ黙って消える** —
    /// しかも記録は全部戻ってくるので気づきにくい。
    #[test]
    fn merge_fills_empty_pins_from_the_incoming_db() {
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        let mut mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let mut theirs = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        set_pins(&mut theirs, bench, vec!["3".into(), "5".into()]);

        merge_db(&mut mine, theirs);

        assert_eq!(
            mine.exercise(bench).expect("種目").pins,
            ["3", "5"],
            "ID 一致の枝でピンが落ちた"
        );
    }

    /// 名前一致の枝（2 台で別々に作った同名の自作種目）も同じ扱い。
    #[test]
    fn merge_fills_empty_pins_when_the_exercise_matched_by_name() {
        let chest = crate::presets::preset_group_id("胸").expect("プリセット");
        let mut mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        mine.exercises.push(Exercise {
            id: ExerciseId::from_bits(0xB001),
            name: "ペックフライ".into(),
            group_id: chest,
            order: 9,
            archived: false,
            pins: Vec::new(),
            interval_sec: None,
            labels: Vec::new(),
        });
        let mut theirs = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        theirs.exercises.push(Exercise {
            id: ExerciseId::from_bits(0xC001),
            name: "ペックフライ".into(),
            group_id: chest,
            order: 9,
            archived: false,
            pins: vec!["7".into()],
            interval_sec: None,
            labels: Vec::new(),
        });

        merge_db(&mut mine, theirs);

        assert_eq!(
            mine.exercise(ExerciseId::from_bits(0xB001))
                .expect("種目")
                .pins,
            ["7"],
            "名前一致の枝でピンが落ちた"
        );
    }

    /// 取り込みは足すだけ（adr/storage/import-is-merge-only.md）。ピンも同じで、
    /// **手元に値があるなら触らない**。マシンは端末ではなくジムに紐づくので、
    /// 上書きすると「引っ越し先のジムの値」が古いファイルで巻き戻る。
    #[test]
    fn merge_never_overwrites_pins_that_are_already_set() {
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        let mut mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        set_pins(&mut mine, bench, vec!["1".into()]);
        let mut theirs = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        set_pins(&mut theirs, bench, vec!["9".into()]);

        merge_db(&mut mine, theirs);

        assert_eq!(
            mine.exercise(bench).expect("種目").pins,
            ["1"],
            "取り込みが手元のピンを書き換えた"
        );
    }

    /// 毎行書くとシートで同じ文字列が縦に伸び、行数ぶん容量も増える。
    /// 体重・種目メモと同じ「まとまりの先頭 1 行だけ」。
    #[test]
    fn export_tsv_writes_the_pins_only_on_the_first_row_of_an_exercise() {
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        let mut db = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        set_pins(&mut db, bench, vec!["3".into(), "5".into()]);
        db.sessions.insert(
            date_key(d(2026, 8, 1)),
            Session {
                logs: vec![ExerciseLog {
                    exercise_id: bench,
                    sets: vec![
                        SetEntry {
                            weight: 60.0,
                            reps: 10,
                            ..Default::default()
                        },
                        SetEntry {
                            weight: 60.0,
                            reps: 8,
                            ..Default::default()
                        },
                    ],
                    at: None,
                    note: String::new(),
                    label: None,
                }],
                ..Session::default()
            },
        );

        let tsv = export_tsv(&db, jst(), crate::i18n::Lang::Ja);
        let r = rows(&tsv);
        let col = col(&r, "ピン");
        let cells: Vec<&str> = r[1..]
            .iter()
            .filter(|row| row[2] == "ベンチプレス")
            .map(|row| row[col])
            .collect();

        assert_eq!(
            cells,
            ["3 5", ""],
            "毎行書いている（または 1 行も書いていない）"
        );
    }

    /// ★ 手つかずのプリセットを書き出さない規則の穴。記録にもメニューにも出てこない
    /// 種目にピンだけ付けると、種目マスタ行の「プリセットは書かない」で弾かれて
    /// **ピンだけが黙って消える**。
    #[test]
    fn export_tsv_keeps_a_preset_that_only_has_pins() {
        let squat = crate::presets::preset_exercise_id("スクワット").expect("プリセット");
        let mut db = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        set_pins(&mut db, squat, vec!["7".into()]);

        let tsv = export_tsv(&db, jst(), crate::i18n::Lang::Ja);
        let r = rows(&tsv);
        let col = col(&r, "ピン");
        let row = r[1..]
            .iter()
            .find(|row| row[2] == "スクワット")
            .expect("ピンを持つプリセットは種目マスタ行に出る");

        assert_eq!(row[col], "7");
        // 手つかずのプリセットは今までどおり書かない（この規則自体は壊していない）
        assert!(
            !r[1..].iter().any(|row| row[2] == "デッドリフト"),
            "ピンの無いプリセットまで書き出している"
        );
    }

    /// 書き出し → 新品端末へ戻す。**この経路が通らないと機種変更でピンが消える。**
    #[test]
    fn tsv_round_trips_pins() {
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        let mut db = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        set_pins(&mut db, bench, vec!["3".into(), "5".into(), "2".into()]);
        let tsv = export_tsv(&db, jst(), crate::i18n::Lang::Ja);

        let mut fresh = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let incoming = parse_import(&tsv, &mut ids(), &fresh).expect("読み戻せる");
        merge_db(&mut fresh, incoming);

        assert_eq!(
            fresh.exercise(bench).expect("種目").pins,
            ["3", "5", "2"],
            "TSV の往復でピンが落ちた"
        );
    }

    /// 自作種目（プリセットに無い名前）でも往復する。ID は取り込み側で採番されるので、
    /// 名前で引き直して見る。
    #[test]
    fn tsv_round_trips_pins_for_a_custom_exercise() {
        let chest = crate::presets::preset_group_id("胸").expect("プリセット");
        let mut db = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        db.exercises.push(Exercise {
            id: ExerciseId::from_bits(0xD001),
            name: "ペックフライ".into(),
            group_id: chest,
            order: 9,
            archived: false,
            pins: vec!["4".into(), "12".into()],
            interval_sec: None,
            labels: Vec::new(),
        });
        let tsv = export_tsv(&db, jst(), crate::i18n::Lang::Ja);

        let mut fresh = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let incoming = parse_import(&tsv, &mut ids(), &fresh).expect("読み戻せる");
        merge_db(&mut fresh, incoming);

        let got = fresh
            .exercises
            .iter()
            .find(|e| e.name == "ペックフライ")
            .expect("自作種目が復活する");
        assert_eq!(got.pins, ["4", "12"]);
    }

    /// 取り込みは足すだけ。`merge_db` の `fill_pins` と `parse_tsv` の `take_pins` の
    /// **両方**が上書きしないことを、経路の端から端まで通して見る。
    #[test]
    fn tsv_import_does_not_overwrite_local_pins() {
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        let mut theirs = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        set_pins(&mut theirs, bench, vec!["9".into()]);
        let tsv = export_tsv(&theirs, jst(), crate::i18n::Lang::Ja);

        let mut mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        set_pins(&mut mine, bench, vec!["1".into()]);
        let incoming = parse_import(&tsv, &mut ids(), &mine).expect("読み戻せる");
        merge_db(&mut mine, incoming);

        assert_eq!(
            mine.exercise(bench).expect("種目").pins,
            ["1"],
            "取り込みが手元のピンを書き換えた"
        );
    }

    #[test]
    fn importing_the_same_tsv_twice_keeps_the_pins_unchanged() {
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        let mut db = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        set_pins(&mut db, bench, vec!["3".into(), "5".into()]);
        let tsv = export_tsv(&db, jst(), crate::i18n::Lang::Ja);

        let mut mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        for _ in 0..2 {
            let incoming = parse_import(&tsv, &mut ids(), &mine).expect("読み戻せる");
            merge_db(&mut mine, incoming);
        }

        assert_eq!(mine.exercise(bench).expect("種目").pins, ["3", "5"]);
    }

    // ── インターバル（adr/ux/interval-seconds-on-the-exercise.md）────────────────

    /// 上限は UI の制限ではなく**取り込みの門番**（`clean_pins` の `MAX_PINS` と同じ
    /// 立場）。取り込んだ JSON には `u32::MAX` が入りうる。
    #[test]
    fn normalize_caps_the_interval() {
        assert_eq!(clean_interval(Some(u32::MAX)), Some(MAX_INTERVAL_SEC));
        assert_eq!(clean_interval(Some(1000)), Some(MAX_INTERVAL_SEC));
        assert_eq!(clean_interval(Some(999)), Some(999));
        assert_eq!(clean_interval(Some(90)), Some(90));
        assert_eq!(clean_interval(None), None);
    }

    /// ★ 0 は落とさない。`parse_reps` が 0 を落とすのとは前提が違う（あちらは
    /// `0×0` のゴーストセットが指標を汚すため）。ここで落とすと「休まず次のセットへ」
    /// を選んだ利用者の入力が黙って未設定に戻る。
    #[test]
    fn normalize_keeps_a_zero_interval() {
        assert_eq!(clean_interval(Some(0)), Some(0));
    }

    /// 画面からの書き込み（`set_interval`）と取り込み（`normalize_exercises`）で
    /// 規則が割れていないこと。`set_pins` と同じ検証。
    #[test]
    fn set_interval_normalizes_the_same_way_as_normalize() {
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");

        let mut through_ui = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        set_interval(&mut through_ui, bench, Some(5000));

        let mut through_import = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        if let Some(e) = through_import.exercises.iter_mut().find(|e| e.id == bench) {
            e.interval_sec = Some(5000);
        }
        normalize_exercises(&mut through_import, &mut IdGen::from_seed(1));

        assert_eq!(
            through_ui.exercise(bench).expect("種目").interval_sec,
            through_import.exercise(bench).expect("種目").interval_sec,
            "画面からの書き込みと取り込みで規則がずれている"
        );
        assert_eq!(
            through_ui.exercise(bench).expect("種目").interval_sec,
            Some(MAX_INTERVAL_SEC)
        );
    }

    /// 空欄まで戻せる（`None` を渡すと未設定に戻る）。戻せないと、間違って打った
    /// 秒数がカードの薄字に永久に残る。
    #[test]
    fn set_interval_can_clear_the_value() {
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        let mut db = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        set_interval(&mut db, bench, Some(90));
        set_interval(&mut db, bench, None);
        assert_eq!(db.exercise(bench).expect("種目").interval_sec, None);
    }

    /// ★ インターバルを足しても schema は上げない（`model::SCHEMA` の doc）。
    /// 上げると旧版が `RestoreError::Unsupported` で退避し、前方互換を積極的に壊す
    /// 側になる（ピンを足したときとまったく同じ理由）。
    #[test]
    fn migrate_does_not_bump_the_schema_for_the_interval() {
        let db = migrate(
            r#"{"schema":3,"groups":[],"exercises":[{"id":"000000000001","name":"自作マシン","group_id":"000000000002","order":0,"archived":false,"interval_sec":5000}],"sessions":{}}"#,
            &mut ids(),
        )
        .expect("読める");

        assert_eq!(db.schema, SCHEMA);
        assert_eq!(SCHEMA, 3, "インターバルの追加で schema を上げてはいけない");
        assert_eq!(
            db.exercises[0].interval_sec,
            Some(MAX_INTERVAL_SEC),
            "migrate が normalize_exercises を通っていない"
        );
    }

    /// schema ≤2 のデータからの昇格でも壊れない（フィールドが無いので未設定）。
    #[test]
    fn migrate_from_schema_two_leaves_the_interval_unset() {
        let db = migrate(
            r##"{"schema":2,"next_id":3,"groups":[{"id":1,"name":"胸","color":"#e0524a","order":0}],"exercises":[{"id":2,"name":"自作マシン","group_id":1,"order":0,"archived":false}],"sessions":{}}"##,
            &mut ids(),
        )
        .expect("読める");
        assert!(db.exercises.iter().all(|e| e.interval_sec.is_none()));
    }

    /// ★ **これが落ちると機能そのものが成立しない。** 新品端末への復元は
    /// 「プリセットは固定 ID を持つので必ず ID 一致する」枝を通るので、そこが
    /// 取り込み側を見ないと**記録は全部戻るのにインターバルだけ消える**。
    #[test]
    fn merge_fills_an_empty_interval_from_the_incoming_db() {
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        let mut mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let mut theirs = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        set_interval(&mut theirs, bench, Some(90));

        merge_db(&mut mine, theirs);

        assert_eq!(
            mine.exercise(bench).expect("種目").interval_sec,
            Some(90),
            "ID 一致の枝でインターバルが落ちた"
        );
    }

    /// 名前一致の枝（2 台で別々に作った同名の自作種目）も同じ扱い。
    #[test]
    fn merge_fills_an_empty_interval_when_the_exercise_matched_by_name() {
        let chest = crate::presets::preset_group_id("胸").expect("プリセット");
        let mut mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        mine.exercises.push(Exercise {
            id: ExerciseId::from_bits(0xB002),
            name: "ペックフライ".into(),
            group_id: chest,
            order: 9,
            archived: false,
            pins: Vec::new(),
            interval_sec: None,
            labels: Vec::new(),
        });
        let mut theirs = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        theirs.exercises.push(Exercise {
            id: ExerciseId::from_bits(0xC002),
            name: "ペックフライ".into(),
            group_id: chest,
            order: 9,
            archived: false,
            pins: Vec::new(),
            interval_sec: Some(120),
            labels: Vec::new(),
        });

        merge_db(&mut mine, theirs);

        assert_eq!(
            mine.exercise(ExerciseId::from_bits(0xB002))
                .expect("種目")
                .interval_sec,
            Some(120),
            "名前一致の枝でインターバルが落ちた"
        );
    }

    /// 取り込みは足すだけ（adr/storage/import-is-merge-only.md）。**手元に値があるなら
    /// 触らない** — 上書きすると引っ越し先で入れ直した値が古いファイル 1 枚で巻き戻る。
    #[test]
    fn merge_never_overwrites_an_interval_that_is_already_set() {
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        let mut mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        set_interval(&mut mine, bench, Some(60));
        let mut theirs = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        set_interval(&mut theirs, bench, Some(180));

        merge_db(&mut mine, theirs);

        assert_eq!(
            mine.exercise(bench).expect("種目").interval_sec,
            Some(60),
            "取り込みが手元のインターバルを書き換えた"
        );
    }

    /// ★ `Some(0)` は「入っている値」。`is_none()` で見ているので上書きされない。
    /// `unwrap_or(0) == 0` のような判定に変えるとここが落ちる。
    #[test]
    fn merge_never_overwrites_a_zero_interval() {
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        let mut mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        set_interval(&mut mine, bench, Some(0));
        let mut theirs = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        set_interval(&mut theirs, bench, Some(180));

        merge_db(&mut mine, theirs);

        assert_eq!(
            mine.exercise(bench).expect("種目").interval_sec,
            Some(0),
            "0 秒を未設定として扱っている"
        );
    }

    /// ピンと同じ「その種目が最初に現れた行にだけ書く」。毎行書くとシートで同じ
    /// 数字が縦に伸び、行数ぶん容量も増える。
    #[test]
    fn export_tsv_writes_the_interval_only_on_the_first_row_of_an_exercise() {
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        let mut db = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        set_interval(&mut db, bench, Some(90));
        db.sessions.insert(
            date_key(d(2026, 8, 1)),
            Session {
                logs: vec![ExerciseLog {
                    exercise_id: bench,
                    sets: vec![
                        SetEntry {
                            weight: 60.0,
                            reps: 10,
                            ..Default::default()
                        },
                        SetEntry {
                            weight: 60.0,
                            reps: 8,
                            ..Default::default()
                        },
                    ],
                    at: None,
                    note: String::new(),
                    label: None,
                }],
                ..Session::default()
            },
        );

        let tsv = export_tsv(&db, jst(), crate::i18n::Lang::Ja);
        let r = rows(&tsv);
        let col = col(&r, "インターバル秒");
        let cells: Vec<&str> = r[1..]
            .iter()
            .filter(|row| row[2] == "ベンチプレス")
            .map(|row| row[col])
            .collect();

        assert_eq!(
            cells,
            ["90", ""],
            "毎行書いている（または 1 行も書いていない）"
        );
    }

    /// ★ ピンと 1 つの `insert` を共有しているので、**両方が同じ行に出る**。
    /// 集合を 2 本に割ると片方だけ `insert` を忘れる余地が生まれる。
    #[test]
    fn export_tsv_writes_the_pins_and_the_interval_on_the_same_row() {
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        let mut db = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        set_pins(&mut db, bench, vec!["3".into(), "5".into()]);
        set_interval(&mut db, bench, Some(90));

        let tsv = export_tsv(&db, jst(), crate::i18n::Lang::Ja);
        let r = rows(&tsv);
        let row = r[1..]
            .iter()
            .find(|row| row[2] == "ベンチプレス")
            .expect("種目マスタ行に出る");

        assert_eq!(row[col(&r, "ピン")], "3 5");
        assert_eq!(row[col(&r, "インターバル秒")], "90");
    }

    /// ★ 手つかずのプリセットを書き出さない規則の穴。記録にもメニューにも出てこない
    /// 種目にインターバルだけ付けると、種目マスタ行の「プリセットは書かない」で
    /// 弾かれて**インターバルだけが黙って消える**（ピンとまったく同じ経路）。
    #[test]
    fn export_tsv_keeps_a_preset_that_only_has_an_interval() {
        let squat = crate::presets::preset_exercise_id("スクワット").expect("プリセット");
        let mut db = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        set_interval(&mut db, squat, Some(180));

        let tsv = export_tsv(&db, jst(), crate::i18n::Lang::Ja);
        let r = rows(&tsv);
        let col = col(&r, "インターバル秒");
        let row = r[1..]
            .iter()
            .find(|row| row[2] == "スクワット")
            .expect("インターバルを持つプリセットは種目マスタ行に出る");

        assert_eq!(row[col], "180");
        assert!(
            !r[1..].iter().any(|row| row[2] == "デッドリフト"),
            "インターバルの無いプリセットまで書き出している"
        );
    }

    /// ★ 0 秒も書き出す。`String::new()` と `"0"` を混同すると、スーパーセットの
    /// 設定が機種変更で消える。
    #[test]
    fn export_tsv_writes_a_zero_interval() {
        let squat = crate::presets::preset_exercise_id("スクワット").expect("プリセット");
        let mut db = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        set_interval(&mut db, squat, Some(0));

        let tsv = export_tsv(&db, jst(), crate::i18n::Lang::Ja);
        let r = rows(&tsv);
        let row = r[1..]
            .iter()
            .find(|row| row[2] == "スクワット")
            .expect("種目マスタ行に出る");
        assert_eq!(row[col(&r, "インターバル秒")], "0");
    }

    /// ★ メニューにだけ入っている種目（記録が 1 日も無い）の経路。`logged` が
    /// メニューの種目を先に集めるので**種目マスタ行は書かれず**、メニュー行が
    /// 唯一の運び手になる。ここで `insert` を忘れると、記録の無い種目の設定だけが
    /// 機種変更で黙って消える。
    #[test]
    fn tsv_round_trips_the_interval_for_an_exercise_only_in_a_routine() {
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        let mut db = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        set_pins(&mut db, bench, vec!["3".into()]);
        set_interval(&mut db, bench, Some(90));
        db.routines.push(Routine {
            id: RoutineId::from_bits(0xE001),
            name: "胸の日".into(),
            exercises: vec![bench],
        });

        let tsv = export_tsv(&db, jst(), crate::i18n::Lang::Ja);
        let r = rows(&tsv);
        // 種目マスタ行は出ず、メニュー行（メニュー列が埋まる行）だけがある
        let row = exactly_one(r[1..].iter().filter(|row| row[2] == "ベンチプレス"))
            .expect("ベンチプレスの行はちょうど 1 本");
        assert_eq!(row[col(&r, "メニュー")], "胸の日", "メニュー行ではない");
        assert_eq!(row[col(&r, "インターバル秒")], "90");
        assert_eq!(row[col(&r, "ピン")], "3");

        let mut fresh = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let incoming = parse_import(&tsv, &mut ids(), &fresh).expect("読み戻せる");
        merge_db(&mut fresh, incoming);

        assert_eq!(fresh.exercise(bench).expect("種目").interval_sec, Some(90));
        assert_eq!(fresh.exercise(bench).expect("種目").pins, ["3"]);
    }

    /// 書き出し → 新品端末へ戻す。**この経路が通らないと機種変更で消える。**
    #[test]
    fn tsv_round_trips_the_interval() {
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        let mut db = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        set_interval(&mut db, bench, Some(90));
        let tsv = export_tsv(&db, jst(), crate::i18n::Lang::Ja);

        let mut fresh = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let incoming = parse_import(&tsv, &mut ids(), &fresh).expect("読み戻せる");
        merge_db(&mut fresh, incoming);

        assert_eq!(
            fresh.exercise(bench).expect("種目").interval_sec,
            Some(90),
            "TSV の往復でインターバルが落ちた"
        );
    }

    /// 自作種目でも往復する。ID は取り込み側で採番されるので名前で引き直して見る。
    #[test]
    fn tsv_round_trips_the_interval_for_a_custom_exercise() {
        let chest = crate::presets::preset_group_id("胸").expect("プリセット");
        let mut db = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        db.exercises.push(Exercise {
            id: ExerciseId::from_bits(0xD002),
            name: "ペックフライ".into(),
            group_id: chest,
            order: 9,
            archived: false,
            pins: Vec::new(),
            interval_sec: Some(75),
            labels: Vec::new(),
        });
        let tsv = export_tsv(&db, jst(), crate::i18n::Lang::Ja);

        let mut fresh = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let incoming = parse_import(&tsv, &mut ids(), &fresh).expect("読み戻せる");
        merge_db(&mut fresh, incoming);

        let got = fresh
            .exercises
            .iter()
            .find(|e| e.name == "ペックフライ")
            .expect("自作種目が復活する");
        assert_eq!(got.interval_sec, Some(75));
    }

    /// 英語で書き出したファイルを日本語の端末で取り込める（見出しは日英どちらも受ける）。
    #[test]
    fn tsv_round_trips_the_interval_across_languages() {
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        let mut db = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        set_interval(&mut db, bench, Some(90));
        let tsv = export_tsv(&db, jst(), crate::i18n::Lang::En);

        let mut fresh = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let incoming = parse_import(&tsv, &mut ids(), &fresh).expect("読み戻せる");
        merge_db(&mut fresh, incoming);

        assert_eq!(fresh.exercise(bench).expect("種目").interval_sec, Some(90));
    }

    /// 取り込みは足すだけ。`merge_db` の `fill_interval` と `parse_tsv` の
    /// `take_interval` の**両方**が上書きしないことを、経路の端から端まで通して見る。
    #[test]
    fn tsv_import_does_not_overwrite_a_local_interval() {
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        let mut theirs = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        set_interval(&mut theirs, bench, Some(180));
        let tsv = export_tsv(&theirs, jst(), crate::i18n::Lang::Ja);

        let mut mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        set_interval(&mut mine, bench, Some(60));
        let incoming = parse_import(&tsv, &mut ids(), &mine).expect("読み戻せる");
        merge_db(&mut mine, incoming);

        assert_eq!(
            mine.exercise(bench).expect("種目").interval_sec,
            Some(60),
            "取り込みが手元のインターバルを書き換えた"
        );
    }

    /// ★ シートで `1:30` や `90秒` と打ち直された値を 0 として取り込まない
    /// （「休まない種目」に化ける）。読めないセルは黙って無視する。
    #[test]
    fn tsv_import_ignores_an_unreadable_interval_cell() {
        let mut mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let incoming = parse_import(
            "日付\t部位\t種目\tセット\t重量kg\t回数\tインターバル秒\n2026-08-01\t胸\tベンチプレス\t1\t60\t10\t1:30\n",
            &mut ids(),
            &mine,
        )
        .expect("読み戻せる");
        merge_db(&mut mine, incoming);

        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        assert_eq!(
            mine.exercise(bench).expect("種目").interval_sec,
            None,
            "読めない値を 0 秒として取り込んでいる"
        );
    }

    /// インターバル列を持たない古いファイルも今までどおり読める（進化規則 3）。
    #[test]
    fn tsv_import_reads_a_file_written_before_the_interval_column() {
        let mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let incoming = parse_import(
            "日付\t部位\t種目\tセット\t重量kg\t回数\n2026-08-01\t胸\tベンチプレス\t1\t60\t10\n",
            &mut ids(),
            &mine,
        )
        .expect("インターバル列より前しか無い TSV も読める");
        assert!(incoming.exercises.iter().all(|e| e.interval_sec.is_none()));
    }

    #[test]
    fn importing_the_same_tsv_twice_keeps_the_interval_unchanged() {
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        let mut db = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        set_interval(&mut db, bench, Some(90));
        let tsv = export_tsv(&db, jst(), crate::i18n::Lang::Ja);

        let mut mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        for _ in 0..2 {
            let incoming = parse_import(&tsv, &mut ids(), &mine).expect("読み戻せる");
            merge_db(&mut mine, incoming);
        }

        assert_eq!(mine.exercise(bench).expect("種目").interval_sec, Some(90));
    }

    // ── ラベル ──────────────────────────────────────────────────────────────
    // adr/data-model/labels-on-the-exercise-and-a-mark-on-the-log.md
    // adr/ux/label-chips-switch-the-history-and-the-copy.md

    /// ラベル定義入りの Db。ベンチプレス(10) に H(1) / P(2) / S(3)。
    fn label_db() -> Db {
        let mut db = menu_db();
        if let Some(x) = db.exercises.iter_mut().find(|x| x.id == e(10)) {
            x.labels = vec![label(1, "H"), label(2, "P"), label(3, "S")];
        }
        db
    }

    /// 色は空。**`clean_labels` を通す経路のテストでは埋まる**ので、色を見ないテストは
    /// このまま使い、色そのものを見るテストだけ [`colored`] で明示する。
    fn label(n: u64, name: &str) -> Label {
        Label {
            id: lb(n),
            name: name.into(),
            color: String::new(),
        }
    }

    /// 色を明示したラベル。
    fn colored(n: u64, name: &str, color: &str) -> Label {
        Label {
            color: color.into(),
            ..label(n, name)
        }
    }

    /// ラベル付きのログ。
    fn tagged(exercise_id: u64, label: u64, sets: &[(f32, u32)]) -> ExerciseLog {
        ExerciseLog {
            label: Some(lb(label)),
            ..log(exercise_id, sets, None)
        }
    }

    /// H(1) が 8/1、P(2) が 8/2 と 8/8、ラベルなしが 8/5。
    fn hps_db() -> Db {
        let mut db = label_db();
        put(&mut db, d(2026, 8, 1), vec![tagged(10, 1, &[(70.0, 10)])]);
        put(&mut db, d(2026, 8, 2), vec![tagged(10, 2, &[(100.0, 3)])]);
        put(&mut db, d(2026, 8, 5), vec![log(10, &[(80.0, 8)], None)]);
        put(&mut db, d(2026, 8, 8), vec![tagged(10, 2, &[(105.0, 3)])]);
        db
    }

    // ── フィルタ ────────────────────────────────────────────────────────────

    #[test]
    fn only_returns_just_that_labels_days_newest_first() {
        let db = hps_db();
        let got = last_logs_before_with(&db, e(10), d(2026, 8, 9), 3, LabelFilter::Only(lb(2)));
        assert_eq!(
            got.iter().map(|(date, _)| *date).collect::<Vec<_>>(),
            vec![d(2026, 8, 8), d(2026, 8, 2)],
            "P の日だけを新しい順に返す"
        );
        // 先頭が「前回」であることは無絞りと同じ仕様
        let (date, _) = last_log_before_with(&db, e(10), d(2026, 8, 9), LabelFilter::Only(lb(2)))
            .expect("P の記録がある");
        assert_eq!(date, d(2026, 8, 8));
    }

    /// 委譲の固定。絶対値は `last_logs_before_returns_the_newest_days_first` 等が見る。
    #[test]
    fn last_logs_before_delegates_to_the_any_filter() {
        let db = hps_db();
        for limit in 1..=MAX_HISTORY {
            let plain: Vec<NaiveDate> = last_logs_before(&db, e(10), d(2026, 8, 9), limit)
                .into_iter()
                .map(|(date, _)| date)
                .collect();
            let any: Vec<NaiveDate> =
                last_logs_before_with(&db, e(10), d(2026, 8, 9), limit, LabelFilter::Any)
                    .into_iter()
                    .map(|(date, _)| date)
                    .collect();
            assert_eq!(plain, any, "limit={limit}");
        }
        assert_eq!(
            last_log_before(&db, e(10), d(2026, 8, 9)).map(|(date, _)| date),
            last_log_before_with(&db, e(10), d(2026, 8, 9), LabelFilter::Any).map(|(date, _)| date),
        );
    }

    /// ★ **フォールバックしない**ことの唯一の型上の主張。落とすとコピーボタンが
    /// 「表示と違うものを流し込む」ことになり、
    /// adr/ux/copy-button-only-when-empty.md が消した 3 問題が別の入口から戻る。
    #[test]
    fn a_label_with_no_history_returns_nothing_instead_of_falling_back() {
        let db = hps_db();
        assert!(
            last_logs_before_with(&db, e(10), d(2026, 8, 9), 3, LabelFilter::Only(lb(3)))
                .is_empty(),
            "S は 1 日も使っていないのでラベルなしの前回に落ちてはいけない"
        );
        assert_eq!(
            last_log_before_with(&db, e(10), d(2026, 8, 9), LabelFilter::Only(lb(3))),
            None
        );
        // 定義されていないラベル ID でも同じ（回復手段は「指定なし」チップ）
        assert_eq!(
            last_log_before_with(&db, e(10), d(2026, 8, 9), LabelFilter::Only(lb(99))),
            None
        );
    }

    #[test]
    fn only_still_skips_days_without_sets() {
        // メモだけ書いた日は実施日ではない。ラベルが付いていても変わらない
        let mut db = label_db();
        put(&mut db, d(2026, 8, 1), vec![tagged(10, 2, &[(100.0, 3)])]);
        let mut memo_only = tagged(10, 2, &[]);
        memo_only.note = "肩が痛いのでやめた".into();
        put(&mut db, d(2026, 8, 7), vec![memo_only]);

        let (date, _) = last_log_before_with(&db, e(10), d(2026, 8, 8), LabelFilter::Only(lb(2)))
            .expect("8/1 まで遡る");
        assert_eq!(date, d(2026, 8, 1));
    }

    /// ★ 既存利用者の体験不変。半年ラベルなしで記録してきた人が「指定なし」を
    /// 押したとき、半年前ではなく**昨日**が出る（`Unlabeled` を作らない理由）。
    #[test]
    fn logs_written_before_labels_existed_all_show_up_under_any() {
        let mut db = label_db();
        put(&mut db, d(2026, 8, 1), vec![log(10, &[(50.0, 10)], None)]);
        put(&mut db, d(2026, 8, 4), vec![log(10, &[(55.0, 10)], None)]);
        put(&mut db, d(2026, 8, 8), vec![tagged(10, 2, &[(100.0, 3)])]);

        let got = last_logs_before_with(&db, e(10), d(2026, 8, 9), 3, LabelFilter::Any);
        assert_eq!(
            got.iter().map(|(date, _)| *date).collect::<Vec<_>>(),
            vec![d(2026, 8, 8), d(2026, 8, 4), d(2026, 8, 1)]
        );
    }

    // ── コピー ──────────────────────────────────────────────────────────────

    /// 候補リストで日付を名指しして 1 日丸ごと写す操作。ソースが可視なので、
    /// ラベルはその日に実在した真実。**両方向で固定する**（片方だけだと将来
    /// 「揃えよう」で崩される）。
    #[test]
    fn copy_day_carries_the_label() {
        let mut db = label_db();
        put(&mut db, d(2026, 8, 8), vec![tagged(10, 2, &[(100.0, 3)])]);

        copy_day(&mut db, d(2026, 8, 8), d(2026, 8, 9), None);

        let copied = &db.sessions[&date_key(d(2026, 8, 9))].logs[0];
        assert_eq!(copied.label, Some(lb(2)), "名指しした日のラベルは運ぶ");
    }

    /// ★ メニューの展開は種目ごとに**別々の不可視の日**から引くので、たまたま最後が
    /// Power だった種目は今日が黙って Power になり、来週の Power 履歴を汚染する。
    #[test]
    fn apply_routine_does_not_carry_the_label() {
        let mut db = label_db();
        db.routines.push(routine(1, "胸の日", &[10, 30]));
        put(&mut db, d(2026, 8, 8), vec![tagged(10, 2, &[(100.0, 3)])]);

        apply_routine(&mut db, r(1), d(2026, 8, 9), None);

        let opened = &db.sessions[&date_key(d(2026, 8, 9))].logs;
        let bench = opened
            .iter()
            .find(|l| l.exercise_id == e(10))
            .expect("ベンチプレスが展開されている");
        assert_eq!(
            bench.label, None,
            "メニューは狙いを持たないのでラベルを押し付けてはいけない"
        );
        // セットは運ぶ（落とすのはラベルだけ）
        assert_eq!(bench.sets.len(), 1);
    }

    /// **読みは `Any`。** 絞ると「カードに前回が出ているのにメニューからは何も
    /// 入らない」食い違いが生まれる。
    #[test]
    fn apply_routine_reads_without_filtering_by_label() {
        let mut db = label_db();
        db.routines.push(routine(1, "胸の日", &[10]));
        // 直近はラベル付き。読みが `Only(なにか)` に寄っていたら 0 件になる
        put(&mut db, d(2026, 8, 8), vec![tagged(10, 2, &[(100.0, 3)])]);

        let opened = apply_routine(&mut db, r(1), d(2026, 8, 9), None);

        assert_eq!(opened, vec![e(10)]);
        assert_eq!(
            db.sessions[&date_key(d(2026, 8, 9))].logs[0].sets.len(),
            1,
            "ラベル付きの直近が読めていない"
        );
    }

    // ── ID の安定 ───────────────────────────────────────────────────────────

    /// ★ この機能の目的が「数か月にわたる Power の履歴」なので、`P` → `Power` の
    /// 改名で過去ログが外れる形は採れない。設定タブの実装（`LabelRow` が `LabelId`
    /// を持つ）が崩れるとここが落ちる。
    #[test]
    fn renaming_a_label_keeps_its_id() {
        let mut db = hps_db();
        let before = last_logs_before_with(&db, e(10), d(2026, 8, 9), 3, LabelFilter::Only(lb(2)))
            .into_iter()
            .map(|(date, _)| date)
            .collect::<Vec<_>>();

        // 「P」→「Power」。**ID は据え置き**で名前だけ差し替える
        set_labels(
            &mut db,
            e(10),
            vec![label(1, "H"), label(2, "Power"), label(3, "S")],
            &mut ids(),
        );

        assert_eq!(label_name(&db, e(10), lb(2)), Some("Power"));
        assert_eq!(
            last_logs_before_with(&db, e(10), d(2026, 8, 9), 3, LabelFilter::Only(lb(2)))
                .into_iter()
                .map(|(date, _)| date)
                .collect::<Vec<_>>(),
            before,
            "改名で過去ログが外れてはいけない"
        );
    }

    // ── 正規化 ──────────────────────────────────────────────────────────────

    #[test]
    fn clean_labels_drops_blank_names_and_truncates_by_char() {
        let got = clean_labels(
            vec![
                label(1, ""),
                label(2, "  "),
                label(3, "　"),
                // ★ char で切る。バイトで切ると UTF-8 の途中で割れて panic する
                label(4, "あいうえおかきくけこさしすせそ"),
                label(5, "Hypertrophy"),
            ],
            &mut ids(),
        );
        assert_eq!(
            got.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(),
            ["あいうえおかきくけこさし", "Hypertrophy"]
        );
    }

    /// ★ **`split_whitespace` しない。** ピンが分割するのは TSV のセル内が空白
    /// 区切りだからで、ラベルは 1 セル = 1 名前。「高重量 低レップ」を許したい。
    #[test]
    fn clean_labels_does_not_split_a_name_that_contains_a_space() {
        let got = clean_labels(vec![label(1, "高重量 低レップ")], &mut ids());
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, "高重量 低レップ");
        // trim もしない（取り込んだデータを書き換えない。`normalize_routines` の規則）
        let got = clean_labels(vec![label(1, " P ")], &mut ids());
        assert_eq!(got[0].name, " P ");
    }

    #[test]
    fn clean_labels_keeps_duplicate_names_and_their_order() {
        // 同名は merge が正当に生む。並べ替えない（`Vec` 順 = 表示順）
        let got = clean_labels(
            vec![label(3, "S"), label(1, "P"), label(2, "P")],
            &mut ids(),
        );
        assert_eq!(
            got.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(),
            ["S", "P", "P"]
        );
        assert_eq!(
            got.iter().map(|l| l.id).collect::<Vec<_>>(),
            [lb(3), lb(1), lb(2)]
        );
    }

    /// ★ 重複 ID は `<For key=id>` の keyed diff を壊す（wasm では panic =
    /// アプリが死ぬ）ので採り直す。**それ以外の ID は保持する** — ここを緩めると
    /// 改名の 1 打鍵で `ExerciseLog.label` が全部宙に浮く。
    #[test]
    fn clean_labels_only_reallocates_duplicate_ids() {
        let got = clean_labels(
            vec![label(1, "H"), label(1, "P"), label(2, "S")],
            &mut ids(),
        );
        assert_eq!(got.len(), 3);
        assert_eq!(got[0].id, lb(1), "初出の ID は据え置く");
        assert_ne!(got[1].id, lb(1), "重複した ID は採り直す");
        assert_eq!(got[2].id, lb(2), "重複していない ID は据え置く");
        assert_eq!(got[1].name, "P", "名前は黙って失わない");
    }

    #[test]
    fn clean_labels_caps_the_number_of_labels() {
        let many: Vec<Label> = (0..MAX_LABELS as u64 + 5)
            .map(|i| label(i, &format!("L{i}")))
            .collect();
        assert_eq!(clean_labels(many, &mut ids()).len(), MAX_LABELS);
    }

    /// ★ 画面からの書き込みと取り込みで規則が食い違うと、**画面で消えたはずの値が
    /// 取り込みで生き返る**（またはその逆）。
    /// `set_pins_normalizes_the_same_way_as_normalize` の鏡像。
    #[test]
    fn set_labels_normalizes_the_same_way_as_normalize() {
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        let raw = vec![
            label(1, "H"),
            label(1, "P"),
            label(2, ""),
            label(3, "あいうえおかきくけこさしすせそ"),
        ];

        let mut through_ui = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        set_labels(&mut through_ui, bench, raw.clone(), &mut ids());

        let mut through_import = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        if let Some(x) = through_import.exercises.iter_mut().find(|x| x.id == bench) {
            x.labels = raw;
        }
        normalize_exercises(&mut through_import, &mut ids());

        assert_eq!(
            through_ui.exercise(bench).expect("種目").labels,
            through_import.exercise(bench).expect("種目").labels,
            "画面からの書き込みと取り込みで規則がずれている"
        );
        assert_eq!(
            through_ui
                .exercise(bench)
                .expect("種目")
                .labels
                .iter()
                .map(|l| l.name.as_str())
                .collect::<Vec<_>>(),
            ["H", "P", "あいうえおかきくけこさし"]
        );
    }

    /// ★ 宙に浮いた `label` は**消さない**（`normalize_routines` の「宙に浮いた参照は
    /// 宙に浮いたまま残す」）。後から相手のファイルを取り込めば生き返る。
    #[test]
    fn normalize_leaves_a_dangling_label_on_the_log() {
        let mut db = label_db();
        put(&mut db, d(2026, 8, 8), vec![tagged(10, 99, &[(100.0, 3)])]);
        normalize(&mut db, &mut ids());
        assert_eq!(
            db.sessions[&date_key(d(2026, 8, 8))].logs[0].label,
            Some(lb(99))
        );
    }

    /// ★ ここが無いと重複ログを畳むときに後発の `label` が黙って落ちる。
    #[test]
    fn dedupe_logs_does_not_drop_a_later_label() {
        let mut s = Session {
            logs: vec![log(10, &[(100.0, 3)], None), tagged(10, 2, &[(100.0, 2)])],
            ..Session::default()
        };
        dedupe_logs(&mut s);
        assert_eq!(s.logs.len(), 1);
        assert_eq!(s.logs[0].label, Some(lb(2)));
        assert_eq!(s.logs[0].sets.len(), 2, "セットは連結される");
    }

    /// 「空のときだけ埋める」なので、先に来たものが勝つ。
    #[test]
    fn dedupe_logs_keeps_the_first_label_it_saw() {
        let mut s = Session {
            logs: vec![tagged(10, 1, &[(70.0, 10)]), tagged(10, 2, &[(100.0, 3)])],
            ..Session::default()
        };
        dedupe_logs(&mut s);
        assert_eq!(s.logs[0].label, Some(lb(1)));
    }

    // ── 色 ──────────────────────────────────────────────────────────────────
    // adr/ux/label-colour-on-the-progress-dots.md

    /// パレット。テストが `LABEL_COLOR_CHOICES` の綴りを写さないように 1 本で引く。
    fn palette(i: usize) -> &'static str {
        crate::presets::LABEL_COLOR_CHOICES[i]
    }

    /// ★ **色の門番は `clean_labels` の 1 箇所だけ**という主張。旧版の JSON（空）も、
    /// 手編集の壊れた値も、ここを通れば必ず `#rrggbb` になる。
    #[test]
    fn clean_labels_fills_a_missing_colour_from_the_palette() {
        let got = clean_labels(
            vec![
                colored(1, "H", ""),
                colored(2, "P", "red"),
                colored(3, "S", "#xyzxyz"),
                colored(4, "T", "#e0524a12"),
            ],
            &mut ids(),
        );
        assert_eq!(
            got.iter().map(|l| l.color.as_str()).collect::<Vec<_>>(),
            [palette(0), palette(1), palette(2), palette(3)],
            "名前付きの色・16 進でない値・8 桁は全部「持っていない」扱い"
        );
    }

    /// ★ **利用者が設定した有効な色は同色でも触らない。** 2 本を同じ色にしたのは
    /// 選択であって壊れた値ではない。ここで「重複だから」と塗り替えると、設定
    /// シートで選んだ色が次の正規化で黙って変わる。
    #[test]
    fn clean_labels_keeps_a_valid_colour_even_when_it_repeats() {
        let got = clean_labels(
            vec![
                colored(1, "H", "#123456"),
                colored(2, "P", "#123456"),
                // 大文字も有効な `#rrggbb`。小文字へ正規化もしない
                colored(3, "S", "#ABCDEF"),
            ],
            &mut ids(),
        );
        assert_eq!(
            got.iter().map(|l| l.color.as_str()).collect::<Vec<_>>(),
            ["#123456", "#123456", "#ABCDEF"]
        );
    }

    /// ★ **色を埋めるのは 2 パス目**であることの主張。1 パスで前方（`out`）の色だけを
    /// 見ると、**無効な色が先・有効な色が後ろ**という並びで 1 本目がパレットの先頭を
    /// 取り、2 本目は有効なので据え置かれて同色 2 本ができる。画面からは作れないが、
    /// 手編集の JSON や色欄が一部欠けたファイルの取り込みでは起きうる経路。
    #[test]
    fn clean_labels_does_not_collide_with_a_valid_colour_that_comes_later() {
        let got = clean_labels(
            vec![
                colored(1, "H", ""),
                // 2 本目はパレットの先頭色を**有効な値として**持っている
                colored(2, "P", palette(0)),
                colored(3, "S", ""),
                // 大文字違いも「使用済み」として数える
                colored(4, "T", palette(1).to_uppercase().as_str()),
            ],
            &mut ids(),
        );
        let colors: Vec<&str> = got.iter().map(|l| l.color.as_str()).collect();
        assert_eq!(
            colors,
            [
                palette(2),
                palette(0),
                palette(3),
                &palette(1).to_uppercase()
            ],
            "後ろの行の有効な色を「使用済み」に数えていない"
        );
        // 大文字小文字を無視して見ても 4 色が全部違う
        let lower: std::collections::HashSet<String> =
            colors.iter().map(|c| c.to_ascii_lowercase()).collect();
        assert_eq!(lower.len(), 4, "同じ色のラベルが 2 本並んだ: {colors:?}");
    }

    #[test]
    fn next_label_color_skips_colours_already_in_use() {
        // 先頭が空いていれば先頭
        assert_eq!(next_label_color(std::iter::empty()), palette(0));
        // 使用済みを飛ばす（並び順ではなく「未使用の最初」）
        assert_eq!(next_label_color([palette(0), palette(2)]), palette(1));
        // 大文字小文字を無視して「使用済み」と数える
        assert_eq!(
            next_label_color([palette(0).to_uppercase().as_str()]),
            palette(1)
        );
        // 6 色を使い切ったら重複を許して `n % 6` に落ちる
        let all: Vec<&str> = crate::presets::LABEL_COLOR_CHOICES.to_vec();
        assert_eq!(next_label_color(all.clone()), palette(0));
        let mut seven = all;
        seven.push(palette(0));
        assert_eq!(next_label_color(seven), palette(1));
    }

    /// ★ **位置ベース（`out.len() % 6`）では駄目**という主張。✕ で 1 本消してから
    /// ＋ で足したときに残った行と同じ色が生まれる形を、ここで固定する。
    /// 併せて、基準が**入力ではなく出力の並び**であること（空名の行は数に入らない）。
    #[test]
    fn a_new_label_never_repeats_a_colour_already_on_the_exercise() {
        // 空名の行を先頭に混ぜても、残る 2 本の色はパレットの先頭 2 色
        let got = clean_labels(
            vec![colored(1, "", ""), colored(2, "H", ""), colored(3, "P", "")],
            &mut ids(),
        );
        assert_eq!(
            got.iter().map(|l| l.color.as_str()).collect::<Vec<_>>(),
            [palette(0), palette(1)],
            "落とした行を数に入れている（出力位置が基準）"
        );

        // 2 色目を消してから足す → 空いた色が戻ってきて、残った行とは重ならない
        let got = clean_labels(
            vec![colored(1, "H", palette(0)), colored(3, "S", palette(2))],
            &mut ids(),
        );
        let mut kept = got;
        kept.push(colored(4, "新", ""));
        let got = clean_labels(kept, &mut ids());
        let colors: Vec<&str> = got.iter().map(|l| l.color.as_str()).collect();
        assert_eq!(colors, [palette(0), palette(2), palette(1)]);
        assert_eq!(
            colors
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            3,
            "同じ色のラベルが 2 本並んだ"
        );
    }

    /// 同名寄せの枝は `alias` を張って取り込み側の定義を捨てる = **自分の色が勝つ**。
    /// 古いファイル 1 枚で自分が選んだ色が巻き戻らない（ログの `label` と同じ規則）。
    #[test]
    fn merge_keeps_my_colour_when_a_label_matches_by_name() {
        let mut mine = preset_db_with_labels(vec![colored(1, "P", "#123456")]);
        merge_db(
            &mut mine,
            preset_db_with_labels(vec![colored(7, "P", "#abcdef")]),
        );

        assert_eq!(bench_colors(&mine), ["#123456"], "自分の色が上書きされた");
    }

    /// 新規追加の枝は**衝突しない有効な色をそのまま採る**。あちらの端末で選んだ色を
    /// 取り込みで黙って捨てない。
    #[test]
    fn merge_carries_the_colour_of_a_newly_added_label() {
        let mut mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        merge_db(
            &mut mine,
            preset_db_with_labels(vec![colored(1, "H", "#123456"), colored(2, "P", "#abcdef")]),
        );

        assert_eq!(bench_colors(&mine), ["#123456", "#abcdef"]);
    }

    /// ★ **`merge_db` は `normalize` を呼ばない**ので `clean_labels` の門を通らない。
    /// 2 台で独立に定義した 1 本目どうしはどちらもパレットの先頭色なので、そのまま
    /// 足すと推移タブで区別できない 2 本になる。新規追加の枝で塗り替える。
    #[test]
    fn merge_recolours_an_incoming_label_that_clashes_with_mine() {
        let mut mine = preset_db_with_labels(vec![colored(1, "H", palette(0))]);
        merge_db(
            &mut mine,
            // 別名・同色（大文字違いも衝突と見る）+ 壊れた色
            preset_db_with_labels(vec![
                colored(2, "P", palette(0).to_uppercase().as_str()),
                colored(3, "S", "red"),
            ]),
        );

        assert_eq!(bench_colors(&mine), [palette(0), palette(1), palette(2)]);
    }

    /// TSV に色列は無い（部位と同じ）。取り込みで生まれたラベルにも色が付き、
    /// **同じ種目の中で重ならない**。
    #[test]
    fn tsv_import_gives_a_new_label_a_colour() {
        let mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let incoming = parse_import(
            "日付\t部位\t種目\tセット\t重量kg\t回数\tラベル\n\
             2026-08-01\t胸\tベンチプレス\t1\t70\t10\tH\n\
             2026-08-08\t胸\tベンチプレス\t1\t100\t3\tP\n",
            &mut ids(),
            &mine,
        )
        .expect("読める");

        assert_eq!(bench_colors(&incoming), [palette(0), palette(1)]);
    }

    /// 画面からの書き込みと取り込みで色の規則が食い違わないこと
    /// （`set_labels_normalizes_the_same_way_as_normalize` の色版）。
    #[test]
    fn set_labels_fills_the_colour_like_normalize_does() {
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        let raw = vec![
            colored(1, "H", ""),
            colored(2, "P", "#123456"),
            colored(3, "S", "壊れた"),
        ];

        let mut through_ui = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        set_labels(&mut through_ui, bench, raw.clone(), &mut ids());

        let mut through_import = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        if let Some(x) = through_import.exercises.iter_mut().find(|x| x.id == bench) {
            x.labels = raw;
        }
        normalize_exercises(&mut through_import, &mut ids());

        assert_eq!(
            through_ui.exercise(bench).expect("種目").labels,
            through_import.exercise(bench).expect("種目").labels,
            "画面からの書き込みと取り込みで色の規則がずれている"
        );
        assert_eq!(
            bench_colors(&through_ui),
            [palette(0), "#123456", palette(1)],
            "有効な色は据え置き、無い色だけ未使用のパレット色で埋める"
        );
    }

    // ── 推移タブの絞り込み ──────────────────────────────────────────────────
    // adr/ux/label-colour-on-the-progress-dots.md

    /// hps_db の日ごとの値（`Metric::Volume`）。
    const HPS_VALUES: [(u32, f64); 4] = [(1, 700.0), (2, 300.0), (5, 640.0), (8, 315.0)];

    fn hps_points(p: Pick, filter: LabelFilter) -> Vec<SeriesPoint> {
        pick_points(
            &hps_db(),
            p,
            Metric::Volume,
            d(2026, 8, 1),
            d(2026, 8, 31),
            Drops::Include,
            filter,
        )
    }

    fn bench_pick() -> Pick {
        Pick::default().with_exercise(&hps_db(), Some(e(10)))
    }

    #[test]
    fn pick_points_filters_the_exercise_branch_by_label() {
        let got = hps_points(bench_pick(), LabelFilter::Only(lb(2)));
        assert_eq!(
            got.iter().map(|p| (p.date, p.value)).collect::<Vec<_>>(),
            vec![(d(2026, 8, 2), 300.0), (d(2026, 8, 8), 315.0)],
            "P の日だけが残る"
        );
        assert!(
            got.iter().all(|p| p.label == Some(lb(2))),
            "残った点に P 以外のラベルが載っている"
        );
        // ラベルなしの日（8/5）は `Only` では出ない
        assert!(!got.iter().any(|p| p.date == d(2026, 8, 5)));
    }

    /// ★ **旧名は旧挙動。** `pick_series` を委譲に変えたことで既存利用者の画面が
    /// 1 バイトも変わらないことの主張。
    #[test]
    fn pick_points_is_identical_to_pick_series_when_unfiltered() {
        let db = hps_db();
        for p in [
            bench_pick(),
            Pick {
                group: Some(g(1)),
                exercise: None,
            },
        ] {
            for m in [Metric::Volume, Metric::Sets, Metric::Reps] {
                let points = pick_points(
                    &db,
                    p,
                    m,
                    d(2026, 8, 1),
                    d(2026, 8, 31),
                    Drops::Include,
                    LabelFilter::Any,
                );
                assert_eq!(
                    points.iter().map(|p| (p.date, p.value)).collect::<Vec<_>>(),
                    pick_series(&db, p, m, d(2026, 8, 1), d(2026, 8, 31), Drops::Include),
                );
            }
        }
        // ラベルは無絞りでも載る（点の色に使う）
        let got = hps_points(bench_pick(), LabelFilter::Any);
        assert_eq!(
            got.iter().map(|p| p.label).collect::<Vec<_>>(),
            vec![Some(lb(1)), Some(lb(2)), None, Some(lb(2))]
        );
        assert_eq!(
            got.iter()
                .map(|p| (p.date.day(), p.value))
                .collect::<Vec<_>>(),
            HPS_VALUES.to_vec()
        );
    }

    /// ★ **部位の枝はラベルを見ない。** ラベルは種目ごとに独立した体系なので、
    /// 複数種目の合算に載せられる `LabelId` が無い。`Only` が届いても黙って無視して
    /// 従来どおりの合算を返す（0 件にして空グラフを見せない）。
    #[test]
    fn pick_points_ignores_labels_on_the_group_branch() {
        let chest = Pick {
            group: Some(g(1)),
            exercise: None,
        };
        let any = hps_points(chest, LabelFilter::Any);
        let only = hps_points(chest, LabelFilter::Only(lb(2)));

        assert_eq!(any, only, "部位の枝で絞りが効いてしまっている");
        assert_eq!(
            any.iter()
                .map(|p| (p.date.day(), p.value))
                .collect::<Vec<_>>(),
            HPS_VALUES.to_vec()
        );
        assert!(
            any.iter().all(|p| p.label.is_none()),
            "合算の点にラベルが載っている"
        );
    }

    /// ★ **週内の全点が同じラベルのときだけ残す。** 混ざった週に 1 色を選ぶと、
    /// 3 分の 1 だけを指す色がその週全体の点に付いて**色が嘘になる**。
    #[test]
    fn aggregate_weekly_points_keeps_a_label_only_when_the_whole_week_agrees() {
        let sp = |day: u32, label: Option<u64>| SeriesPoint {
            date: d(2026, 8, day),
            value: 1.0,
            label: label.map(lb),
        };

        // 同一ラベルの週 → 残る
        let got = aggregate_weekly_points(&[sp(2, Some(2)), sp(5, Some(2))]);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].label, Some(lb(2)));

        // 別 ID が混ざる → 既定色へ落とす
        let got = aggregate_weekly_points(&[sp(2, Some(1)), sp(5, Some(2))]);
        assert_eq!(got[0].label, None);

        // `None` が混ざる（どちら向きでも）→ 既定色へ落とす
        assert_eq!(
            aggregate_weekly_points(&[sp(2, Some(2)), sp(5, None)])[0].label,
            None
        );
        assert_eq!(
            aggregate_weekly_points(&[sp(2, None), sp(5, Some(2))])[0].label,
            None
        );

        // 全部 `None` の週は `None`（「混ざった」ではないが結果は同じ）
        assert_eq!(
            aggregate_weekly_points(&[sp(2, None), sp(5, None)])[0].label,
            None
        );

        // ★ `Only` で絞れば週内は必ず同じラベル = 「全期間」でも色が消えない
        let weekly = aggregate_weekly_points(&hps_points(bench_pick(), LabelFilter::Only(lb(2))));
        assert_eq!(
            weekly
                .iter()
                .map(|p| (p.date, p.value, p.label))
                .collect::<Vec<_>>(),
            vec![(d(2026, 8, 2), 615.0, Some(lb(2)))]
        );
    }

    /// 値の集約は [`aggregate_weekly`] と**まったく同じ**（週キーも合計も）。
    /// ずれると指標と体重（`aggregate_weekly_avg`）の週キーが噛み合わなくなる。
    #[test]
    fn aggregate_weekly_points_sums_like_aggregate_weekly() {
        let points = hps_points(bench_pick(), LabelFilter::Any);
        let plain: Vec<(NaiveDate, f64)> = points.iter().map(|p| (p.date, p.value)).collect();

        assert_eq!(
            aggregate_weekly_points(&points)
                .iter()
                .map(|p| (p.date, p.value))
                .collect::<Vec<_>>(),
            aggregate_weekly(&plain)
        );
        // 混ざった週は既定色（8/2 の週に P と ラベルなしが同居する）
        assert_eq!(
            aggregate_weekly_points(&points)
                .iter()
                .map(|p| p.label)
                .collect::<Vec<_>>(),
            vec![Some(lb(1)), None]
        );
        assert!(aggregate_weekly_points(&[]).is_empty());
    }

    // ── ラベルのマージ ──────────────────────────────────────────────────────

    /// ラベル付きのプリセット Db。ベンチプレスに指定の定義を入れる。
    fn preset_db_with_labels(labels: Vec<Label>) -> Db {
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        let mut db = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        if let Some(x) = db.exercises.iter_mut().find(|x| x.id == bench) {
            x.labels = labels;
        }
        db
    }

    /// ベンチプレスのラベルの色（並び順そのまま）。
    fn bench_colors(db: &Db) -> Vec<String> {
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        db.exercise(bench)
            .expect("種目")
            .labels
            .iter()
            .map(|l| l.color.clone())
            .collect()
    }

    fn bench_labels(db: &Db) -> Vec<(LabelId, String)> {
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        db.exercise(bench)
            .expect("種目")
            .labels
            .iter()
            .map(|l| (l.id, l.name.clone()))
            .collect()
    }

    /// ★ **プリセットは固定 ID を持つので新品端末への復元は必ず「ID 一致」の枝を
    /// 通り、その枝は取り込み側の `Exercise` を丸ごと捨てる。** 手当てしないと
    /// 記録は全部戻るのにラベル定義だけ落ち、ログには宙に浮いた `label` が残る。
    /// `fill_pins` の ★ と同型で、被害はこちらのほうが大きい。
    #[test]
    fn merging_into_a_fresh_device_keeps_the_labels() {
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        let mut theirs = preset_db_with_labels(vec![label(1, "H"), label(2, "P")]);
        theirs.sessions.insert(
            date_key(d(2026, 8, 8)),
            Session {
                logs: vec![ExerciseLog {
                    exercise_id: bench,
                    label: Some(lb(2)),
                    ..log(0, &[(100.0, 3)], None)
                }],
                ..Session::default()
            },
        );

        let mut fresh = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let report = merge_db(&mut fresh, theirs);

        assert_eq!(
            bench_labels(&fresh),
            vec![(lb(1), "H".into()), (lb(2), "P".into())],
            "ID 一致の枝でラベル定義が落ちた"
        );
        assert_eq!(report.labels_added, 2);
        // 履歴が「どのラベルにも属さない」にならない
        assert_eq!(
            last_logs_before_with(&fresh, bench, d(2026, 8, 9), 3, LabelFilter::Only(lb(2))).len(),
            1
        );
    }

    /// ★ **同名寄せの枝でも `merge_labels` を呼んでいる。** 忘れると名前で寄せた
    /// 種目のラベルだけが落ちる（一番踏みやすいミス）。
    #[test]
    fn merge_fills_the_labels_on_the_name_matched_branch_too() {
        let chest = crate::presets::preset_group_id("胸").expect("プリセット");
        let make = |id: u64, labels: Vec<Label>| {
            let mut db = crate::presets::seeded_db(crate::i18n::Lang::Ja);
            db.exercises.push(Exercise {
                id: ExerciseId::from_bits(id),
                name: "ペックフライ".into(),
                group_id: chest,
                order: 9,
                archived: false,
                pins: Vec::new(),
                interval_sec: None,
                labels,
            });
            db
        };
        // ID は違うが同名 → 「同名寄せ」の枝を通る
        let mut mine = make(0xAAA1, Vec::new());
        let theirs = make(0xBBB1, vec![label(1, "H")]);

        let report = merge_db(&mut mine, theirs);

        let got = mine
            .exercises
            .iter()
            .find(|e| e.name == "ペックフライ")
            .expect("同名に寄っている");
        assert_eq!(
            got.labels
                .iter()
                .map(|l| l.name.as_str())
                .collect::<Vec<_>>(),
            ["H"],
            "同名寄せの枝でラベルが落ちた"
        );
        assert_eq!(report.labels_added, 1);
    }

    /// 同名がちょうど 1 件なら寄せ、**ログの `label` が写像で張り替わる**。
    /// これが「2 台で独立に定義した `P` を寄せたい」という名前側の利点の回収点。
    #[test]
    fn merge_maps_a_log_label_onto_the_locally_named_label() {
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        let mut mine = preset_db_with_labels(vec![label(1, "P")]);
        // 別 ID・同名。ログはあちらの ID を指している
        let mut theirs = preset_db_with_labels(vec![label(7, "P")]);
        theirs.sessions.insert(
            date_key(d(2026, 8, 8)),
            Session {
                logs: vec![ExerciseLog {
                    exercise_id: bench,
                    label: Some(lb(7)),
                    ..log(0, &[(100.0, 3)], None)
                }],
                ..Session::default()
            },
        );

        merge_db(&mut mine, theirs);

        assert_eq!(bench_labels(&mine).len(), 1, "同名は 2 本に増やさない");
        assert_eq!(
            mine.sessions[&date_key(d(2026, 8, 8))].logs[0].label,
            Some(lb(1)),
            "ログのラベルが写像で張り替わっていない"
        );
    }

    /// ★ **写像のキーが `(ExerciseId, LabelId)` の組であることの主張。**
    /// `LabelId` 単独だと後の `insert` が前を上書きし、**種目 A のログが種目 B の
    /// ラベルへ張り替わって宙に浮く**。
    #[test]
    fn merge_does_not_confuse_the_same_label_id_used_by_two_exercises() {
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        let squat = crate::presets::preset_exercise_id("スクワット").expect("プリセット");
        // 取り込み先: 2 種目それぞれに別 ID で同名の "P"
        let mut mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        for (ex, id) in [(bench, 1), (squat, 2)] {
            if let Some(x) = mine.exercises.iter_mut().find(|x| x.id == ex) {
                x.labels = vec![label(id, "P")];
            }
        }
        // 取り込む側: **同じ `LabelId`(7)** を 2 種目で使い回している
        let mut theirs = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        for ex in [bench, squat] {
            if let Some(x) = theirs.exercises.iter_mut().find(|x| x.id == ex) {
                x.labels = vec![label(7, "P")];
            }
        }
        theirs.sessions.insert(
            date_key(d(2026, 8, 8)),
            Session {
                logs: vec![
                    ExerciseLog {
                        exercise_id: bench,
                        label: Some(lb(7)),
                        ..log(0, &[(100.0, 3)], None)
                    },
                    ExerciseLog {
                        exercise_id: squat,
                        label: Some(lb(7)),
                        ..log(0, &[(120.0, 5)], None)
                    },
                ],
                ..Session::default()
            },
        );

        merge_db(&mut mine, theirs);

        let logs = &mine.sessions[&date_key(d(2026, 8, 8))].logs;
        let of = |ex| {
            logs.iter()
                .find(|l| l.exercise_id == ex)
                .expect("ログがある")
                .label
        };
        assert_eq!(
            of(bench),
            Some(lb(1)),
            "ベンチのログが別種目のラベルを指した"
        );
        assert_eq!(
            of(squat),
            Some(lb(2)),
            "スクワットのログが別種目のラベルを指した"
        );
    }

    /// ログの `label` は**空のときだけ**埋める（`mine` 優先）。
    #[test]
    fn merge_fills_a_log_label_only_when_it_is_empty() {
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        let build = |label_id: Option<u64>| {
            let mut db = preset_db_with_labels(vec![label(1, "H"), label(2, "P")]);
            db.sessions.insert(
                date_key(d(2026, 8, 8)),
                Session {
                    logs: vec![ExerciseLog {
                        exercise_id: bench,
                        label: label_id.map(lb),
                        ..log(0, &[(100.0, 3)], None)
                    }],
                    ..Session::default()
                },
            );
            db
        };

        // 空いている側は埋まる
        let mut mine = build(None);
        let report = merge_db(&mut mine, build(Some(2)));
        assert_eq!(
            mine.sessions[&date_key(d(2026, 8, 8))].logs[0].label,
            Some(lb(2))
        );
        assert_eq!(report.labels_added, 1);

        // 入っている側は上書きしない
        let mut mine = build(Some(1));
        let report = merge_db(&mut mine, build(Some(2)));
        assert_eq!(
            mine.sessions[&date_key(d(2026, 8, 8))].logs[0].label,
            Some(lb(1)),
            "古いファイル 1 枚で自分の選択を巻き戻してはいけない"
        );
        assert_eq!(report.labels_added, 0);
    }

    /// ★ **ラベルの差だけで「食い違い」の枝に落ちてはいけない。** 落ちると
    /// `log_rank` が同点なので差し替えの分岐にも入らず、**取り込む側のセットメモが
    /// `Conflict` も出さずに黙って捨てられる**
    /// （adr/data-model/notes-on-logs-and-sets.md 決定 8 の ★ そのもの）。
    #[test]
    fn a_label_difference_alone_is_not_a_set_conflict() {
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        let build = |label_id: u64, set_note: &str| {
            let mut db = preset_db_with_labels(vec![label(1, "H"), label(2, "P")]);
            db.sessions.insert(
                date_key(d(2026, 8, 8)),
                Session {
                    logs: vec![ExerciseLog {
                        exercise_id: bench,
                        label: Some(lb(label_id)),
                        sets: vec![SetEntry {
                            weight: 100.0,
                            reps: 3,
                            note: set_note.to_string(),
                            drops: Vec::new(),
                        }],
                        at: None,
                        note: String::new(),
                    }],
                    ..Session::default()
                },
            );
            db
        };

        let mut mine = build(1, "");
        let report = merge_db(&mut mine, build(2, "あちらのメモ"));

        assert!(
            report.conflicts.is_empty(),
            "ラベルの差を食い違いにしてはいけない: {:?}",
            report.conflicts
        );
        assert_eq!(
            mine.sessions[&date_key(d(2026, 8, 8))].logs[0].sets[0].note,
            "あちらのメモ",
            "セットメモが黙って捨てられた"
        );
    }

    /// ★ セットが負けた枝でもラベルを持ち越す。`..log` に任せると勝った側で
    /// 上書きされ、`mine` 優先で埋めたはずの値が入れ替わる。
    #[test]
    fn merge_keeps_my_label_even_when_my_sets_lose() {
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        let build = |label_id: u64, sets: &[(f32, u32)]| {
            let mut db = preset_db_with_labels(vec![label(1, "H"), label(2, "P")]);
            db.sessions.insert(
                date_key(d(2026, 8, 8)),
                Session {
                    logs: vec![ExerciseLog {
                        exercise_id: bench,
                        label: Some(lb(label_id)),
                        ..log(0, sets, None)
                    }],
                    ..Session::default()
                },
            );
            db
        };

        // 取り込む側のほうがセットが多い → `log_rank` の勝ち枝を通る
        let mut mine = build(1, &[(100.0, 3)]);
        merge_db(&mut mine, build(2, &[(100.0, 3), (100.0, 3), (100.0, 3)]));

        let got = &mine.sessions[&date_key(d(2026, 8, 8))].logs[0];
        assert_eq!(got.sets.len(), 3, "強いほうのセットを採る");
        assert_eq!(got.label, Some(lb(1)), "自分のラベルが上書きされた");
    }

    #[test]
    fn merging_the_same_labels_twice_adds_nothing_the_second_time() {
        let theirs = || preset_db_with_labels(vec![label(1, "H"), label(2, "P")]);
        let mut mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);

        let first = merge_db(&mut mine, theirs());
        let second = merge_db(&mut mine, theirs());

        assert_eq!(first.labels_added, 2);
        assert_eq!(second.labels_added, 0, "カウンタが冪等でない");
        assert_eq!(bench_labels(&mine).len(), 2, "同じラベルが 2 本に増えた");
    }

    /// ★ ラベルだけが増えたマージは `conflicts` に出ないのに**チップが増えて履歴の
    /// 見え方が変わる**。数えないと画面が「新しく取り込むものはありませんでした」と
    /// 嘘をつく。
    #[test]
    fn a_merge_that_only_adds_labels_is_not_a_noop() {
        let mut mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let report = merge_db(&mut mine, preset_db_with_labels(vec![label(1, "H")]));

        assert_eq!(report.labels_added, 1);
        assert!(!report.is_noop(), "ラベルだけ増えたときも noop ではない");
    }

    /// ラベルの列位置を見出しから引く（テストが列順に依存しないように）。
    fn label_col(r: &[Vec<&str>]) -> usize {
        r[0].iter()
            .position(|c| *c == "ラベル")
            .expect("ラベル列がある")
    }

    /// ★ **`ex_meta_written` に相乗りしていないことの証明。** あれは種目粒度で、
    /// ラベルは日ごとに変わる。相乗りさせると 2 日目以降が全部落ちる。
    #[test]
    fn export_tsv_writes_the_label_once_per_log_not_once_per_exercise() {
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        let mut db = preset_db_with_labels(vec![label(1, "H"), label(2, "P")]);
        for (date, label_id, sets) in [
            (d(2026, 8, 1), 1, &[(70.0, 10), (70.0, 10)][..]),
            (d(2026, 8, 8), 2, &[(100.0, 3), (100.0, 3)][..]),
        ] {
            db.sessions.insert(
                date_key(date),
                Session {
                    logs: vec![ExerciseLog {
                        exercise_id: bench,
                        label: Some(lb(label_id)),
                        ..log(0, sets, None)
                    }],
                    ..Session::default()
                },
            );
        }

        let tsv = export_tsv(&db, jst(), crate::i18n::Lang::Ja);
        let r = rows(&tsv);
        let col = label_col(&r);
        let cells: Vec<(&str, &str)> = r[1..]
            .iter()
            .filter(|row| row[2] == "ベンチプレス")
            .map(|row| (row[0], row[col]))
            .collect();

        assert_eq!(
            cells,
            vec![
                ("2026-08-01", "H"),
                ("2026-08-01", ""),
                ("2026-08-08", "P"),
                ("2026-08-08", ""),
            ],
            "日ごとに 1 回だけ書く（2 日目が落ちていない）"
        );
    }

    /// セットが 1 本も無いログ（「肩が痛いのでやめた」）の行にも書く。
    #[test]
    fn export_tsv_writes_the_label_on_a_log_without_sets() {
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        let mut db = preset_db_with_labels(vec![label(2, "P")]);
        db.sessions.insert(
            date_key(d(2026, 8, 8)),
            Session {
                logs: vec![ExerciseLog {
                    exercise_id: bench,
                    sets: Vec::new(),
                    at: None,
                    note: "肩が痛いのでやめた".into(),
                    label: Some(lb(2)),
                }],
                ..Session::default()
            },
        );

        let tsv = export_tsv(&db, jst(), crate::i18n::Lang::Ja);
        let r = rows(&tsv);
        let col = label_col(&r);
        assert_eq!(r[1][col], "P");
    }

    /// 体重だけの行 / 種目マスタ行 / メニュー行は空。
    ///
    /// ★ **未使用のラベル定義は TSV に載らない**（明示的トレードオフ）。
    /// adr/storage/tsv-export-for-spreadsheets.md の基準「落ちてよいのは名前から
    /// 作り直せるもの」を満たす — 未使用ラベルは打ち直せば完全に戻り、**ぶら下がる
    /// 記録が 0 件**。使ったラベルは各ログ行が名前を運ぶので必ず復元される。
    #[test]
    fn export_tsv_leaves_the_label_empty_where_it_has_no_meaning() {
        let squat = crate::presets::preset_exercise_id("スクワット").expect("プリセット");
        let mut db = preset_db_with_labels(vec![label(1, "H")]);
        // 使っていないラベルを持つ種目 + 体重だけの日 + メニュー
        set_pins(&mut db, squat, vec!["7".into()]);
        db.sessions.insert(
            date_key(d(2026, 8, 2)),
            Session {
                logs: Vec::new(),
                body_weight: Some(70.0),
                note: String::new(),
            },
        );
        db.routines.push(Routine {
            id: r(1),
            name: "胸の日".into(),
            exercises: vec![squat],
        });

        let tsv = export_tsv(&db, jst(), crate::i18n::Lang::Ja);
        let rs = rows(&tsv);
        let col = label_col(&rs);
        for row in &rs[1..] {
            assert_eq!(row[col], "", "記録行以外にラベルが出ている: {row:?}");
        }
        // ★ ラベル定義しか持たない種目のためにマスタ行を増やさない（非対称は意図）
        assert!(
            !rs[1..].iter().any(|row| row[2] == "ベンチプレス"),
            "未使用のラベルのためにマスタ行を増やしている"
        );
    }

    /// 書き出し → 新品端末へ戻す。**この経路が通らないと機種変更でラベルが消える。**
    #[test]
    fn tsv_round_trips_the_label_so_only_still_finds_the_same_day() {
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        let mut db = preset_db_with_labels(vec![label(1, "H"), label(2, "P")]);
        for (date, label_id, sets) in [
            (d(2026, 8, 1), 1, &[(70.0, 10)][..]),
            (d(2026, 8, 8), 2, &[(100.0, 3)][..]),
        ] {
            db.sessions.insert(
                date_key(date),
                Session {
                    logs: vec![ExerciseLog {
                        exercise_id: bench,
                        label: Some(lb(label_id)),
                        ..log(0, sets, None)
                    }],
                    ..Session::default()
                },
            );
        }
        let tsv = export_tsv(&db, jst(), crate::i18n::Lang::Ja);

        let mut fresh = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let incoming = parse_import(&tsv, &mut ids(), &fresh).expect("読み戻せる");
        merge_db(&mut fresh, incoming);

        let labels = fresh.exercise(bench).expect("種目").labels.clone();
        assert_eq!(
            labels.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(),
            ["H", "P"],
            "TSV の往復でラベル定義が落ちた"
        );
        // 「P の前回」が 8/8 に戻る
        let p = labels.iter().find(|l| l.name == "P").expect("P がある").id;
        let got = last_logs_before_with(&fresh, bench, d(2026, 8, 9), 3, LabelFilter::Only(p));
        assert_eq!(
            got.iter().map(|(date, _)| *date).collect::<Vec<_>>(),
            vec![d(2026, 8, 8)]
        );
    }

    /// 自分のファイルを戻すだけなら**手元の ID を再利用**して `Conflict` を出さない
    /// （`resolve_label` の梯子 1）。
    #[test]
    fn tsv_import_reuses_my_own_label_id_and_reports_no_conflict() {
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        let mut mine = preset_db_with_labels(vec![label(1, "H"), label(2, "P")]);
        mine.sessions.insert(
            date_key(d(2026, 8, 8)),
            Session {
                logs: vec![ExerciseLog {
                    exercise_id: bench,
                    label: Some(lb(2)),
                    ..log(0, &[(100.0, 3)], None)
                }],
                ..Session::default()
            },
        );
        let tsv = export_tsv(&mine, jst(), crate::i18n::Lang::Ja);

        let incoming = parse_import(&tsv, &mut ids(), &mine).expect("読み戻せる");
        // ★ ログが指す ID が**手元のもの**であること。新しく採番されると `merge_db` の
        //   同名寄せ枝を通り、写像でログを張り替える余計な往復が増える。
        //   （定義そのものは `resolve_exercise` の梯子 1 が手元の `Exercise` を丸ごと
        //   複製するので未使用のものまで乗る。ファイルには載っていないので、
        //   新品端末では往復テストのとおり使ったラベルだけが復元される）
        assert_eq!(
            incoming.sessions[&date_key(d(2026, 8, 8))].logs[0].label,
            Some(lb(2)),
            "手元の ID を再利用していない"
        );

        let report = merge_db(&mut mine, incoming);
        assert_eq!(report.labels_added, 0, "自分のファイルでラベルが増えた");
        assert_eq!(
            mine.exercise(bench).expect("種目").labels.len(),
            2,
            "同名のラベルが増えた"
        );
    }

    /// ★ **キャッシュの存在証明。** 無いと行ごとに採番して 1 種目に数百ラベルが生える。
    #[test]
    fn tsv_import_allocates_one_label_id_for_thirty_rows() {
        let mut tsv = String::from("日付\t部位\t種目\tセット\t重量kg\t回数\tラベル\n");
        for i in 1..=30 {
            tsv.push_str(&format!("2026-08-{i:02}\t胸\tベンチプレス\t1\t100\t3\tP\n"));
        }

        let mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let incoming = parse_import(&tsv, &mut ids(), &mine).expect("読める");

        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        assert_eq!(
            incoming.exercise(bench).expect("種目").labels.len(),
            1,
            "行ごとに採番している"
        );
        // 全 30 日が同じラベルを指す
        let id = incoming.exercise(bench).expect("種目").labels[0].id;
        assert!(
            incoming
                .sessions
                .values()
                .all(|s| s.logs[0].label == Some(id))
        );
    }

    /// ラベル列を持たない古いファイルも今までどおり読める（進化規則 3）。
    #[test]
    fn tsv_import_reads_a_file_written_before_the_label_column() {
        let mine = crate::presets::seeded_db(crate::i18n::Lang::Ja);
        let incoming = parse_import(
            "日付\t部位\t種目\tセット\t重量kg\t回数\n2026-08-01\t胸\tベンチプレス\t1\t60\t10\n",
            &mut ids(),
            &mine,
        )
        .expect("ラベル列より前しか無い TSV も読める");
        assert!(incoming.exercises.iter().all(|e| e.labels.is_empty()));
        assert!(
            incoming
                .sessions
                .values()
                .all(|s| s.logs.iter().all(|l| l.label.is_none()))
        );
    }

    /// ★ 落ちた定義を指すログは取り込み直しても二度と生き返らない dangling になる。
    /// **`is_noop()` には入れない**（何も増えていない）が、数は出す。
    #[test]
    fn labels_over_the_cap_are_counted_as_dropped() {
        let full: Vec<Label> = (0..MAX_LABELS as u64)
            .map(|i| label(i, &format!("L{i}")))
            .collect();
        let mut mine = preset_db_with_labels(full);
        let report = merge_db(
            &mut mine,
            preset_db_with_labels(vec![label(99, "はみ出し")]),
        );

        assert_eq!(report.labels_dropped, 1);
        assert_eq!(report.labels_added, 0);
        assert_eq!(bench_labels(&mine).len(), MAX_LABELS, "上限を超えて入った");
        assert!(
            report.is_noop(),
            "何も増えていないので is_noop は真。落ちたことは labels_dropped が言う"
        );
    }
}
