//! 純ロジック。**`leptos` / `web-sys` を一切 import しない。**
//!
//! `cargo test`（ホストターゲット）で検証する層。UI から呼ぶ計算はすべてここに置き、
//! 画面側は結果を並べるだけにする。

use std::collections::{BTreeMap, HashMap};

use chrono::{Datelike, NaiveDate, TimeDelta};

use crate::model::{
    Db, Exercise, ExerciseId, ExerciseLog, Group, GroupId, IdGen, Routine, RoutineId, SCHEMA,
    Session, SetEntry,
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
    pub const CHOICES: [(Metric, &'static str); 3] = [
        (Metric::Volume, "ボリューム"),
        (Metric::Sets, "セット数"),
        (Metric::Reps, "回数"),
    ];

    /// 表示に添える単位。ボリュームは重量と回数の合成量なので単位を持たない。
    pub fn unit(self) -> &'static str {
        match self {
            Metric::Volume => "",
            Metric::Sets => "セット",
            Metric::Reps => "回",
        }
    }
}

/// 1 セットのボリューム。**重量が入っていないセットは重量 1 として数える。**
///
/// これで自重種目は自然に「総レップ数」、時間種目は「総秒数」になり、
/// 種目ごとに式を分ける必要が無くなる。
///
/// ★ `max(1.0)` にするのは単調性のため。0.5kg を 0.5 倍で扱うと
/// 「重量を足したのに指標が下がる」が起きて、グラフの上下が負荷の増減を表さなくなる。
pub fn set_volume(s: &crate::model::SetEntry) -> f64 {
    f64::from(s.weight).max(1.0) * f64::from(s.reps)
}

/// 1 ログ（= その日のその種目）の指標。
pub fn log_value(m: Metric, l: &ExerciseLog) -> f64 {
    match m {
        Metric::Volume => l.sets.iter().map(set_volume).sum(),
        Metric::Sets => l.sets.len() as f64,
        Metric::Reps => l.sets.iter().map(|s| f64::from(s.reps)).sum(),
    }
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
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(x, y)| x.weight == y.weight && x.reps == y.reps)
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
            .find(|(i, m)| {
                !used[*i] && m.weight.to_bits() == t.weight.to_bits() && m.reps == t.reps
            })
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
fn append_note(dst: &mut String, src: &str) -> bool {
    let incoming = src.trim();
    if incoming.is_empty() || dst.contains(incoming) {
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

/// 空白だけのメモを空文字にする。**「空白 = 無い」を 1 箇所で決める。**
///
/// これが無いと `" "` が `skip_serializing_if = "String::is_empty"` をすり抜けて
/// 保存され続け、`ExerciseLog::is_empty()`（`trim` する）と JSON の見え方がずれる。
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

/// 指定日より**厳密に前**で最も新しい、その種目の記録。
///
/// 単一の `ExerciseLog` を返せるのは「1 日 1 種目 1 ログ」の不変条件に依存する。
pub fn last_log_before(
    db: &Db,
    ex: ExerciseId,
    before: NaiveDate,
) -> Option<(NaiveDate, &ExerciseLog)> {
    db.sessions
        .range(..date_key(before))
        .rev()
        .find_map(|(key, session)| {
            let log = session
                .logs
                .iter()
                .find(|l| l.exercise_id == ex && !l.sets.is_empty())?;
            Some((parse_date_key(key)?, log))
        })
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
/// 体重と体調メモは複製しない。どちらもその日の観測値であってメニュー構成ではない。
/// **種目メモとセットメモも同じ理由で複製しない**（adr/data-model/notes-on-logs-and-sets.md）。
pub fn copy_day(db: &mut Db, from: NaiveDate, to: NaiveDate, at: Option<i64>) -> Vec<ExerciseId> {
    // `&db` と `&mut db` の借用を分けるため先に取り出しておく
    let picked: Vec<(ExerciseId, Vec<SetEntry>)> = copyable(db, from)
        .map(|l| (l.exercise_id, sets_without_notes(&l.sets)))
        .collect();
    seed_day(db, to, picked, at)
}

/// セットの重量と回数だけを写す。**メモは持ち込まない。**
///
/// ★ `sets.clone()` は**メモまで運ぶ**。「3 セット目で肩に違和感」は前回の観測で、
/// 今日の観測ではない。複製すると起きていない観測が翌日に生える
/// （adr/data-model/notes-on-logs-and-sets.md）。この規則の実装はここ 1 箇所に畳む。
fn sets_without_notes(src: &[SetEntry]) -> Vec<SetEntry> {
    src.iter()
        .map(|s| SetEntry {
            weight: s.weight,
            reps: s.reps,
            note: String::new(),
        })
        .collect()
}

/// 空の日に種目とセットを流し込む。**書いた種目 ID** を返す。
///
/// [`copy_day`]（1 日を丸ごと写す）と [`apply_routine`]（種目ごとに直近から引く）の
/// 共通の書き込み口。**ガードを 2 箇所に書かない** — 片方だけ緩められると
/// 「1 日 1 種目 1 ログ」（adr/data-model/one-log-per-exercise-per-day.md）が破れる。
fn seed_day(
    db: &mut Db,
    to: NaiveDate,
    picked: Vec<(ExerciseId, Vec<SetEntry>)>,
    at: Option<i64>,
) -> Vec<ExerciseId> {
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
    for (exercise_id, sets) in picked {
        copied.push(exercise_id);
        // ★ ExerciseLog を clone してはいけない。clone すると元の日の `at` を
        //   引き継ぎ、`at = None` にしたい過去日バックフィルに古い epoch が入る。
        //   日数表示は日付キーから出るので日付が嘘になることはもう無いが（adr/data-model/elapsed-in-local-calendar-days.md）、
        //   「その日に実施した時刻」として存在しない値が残り、同じ暦日にコピーしたときの
        //   時刻粒度が捏造される。記録の正直さは表示の都合とは別に守る（adr/data-model/at-optional-same-day-only.md）
        session.logs.push(ExerciseLog {
            exercise_id,
            sets,
            at,
            note: String::new(),
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
/// 体重・体調メモ・種目メモ・セットメモは複製しない（[`copy_day`] と同じ規則）。
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
    let picked: Vec<(ExerciseId, Vec<SetEntry>)> = opened
        .iter()
        .filter_map(|ex| {
            let (_, log) = last_log_before(db, *ex, to)?;
            Some((*ex, sets_without_notes(&log.sets)))
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

/// 種目別の推移。
pub fn exercise_series(
    db: &Db,
    ex: ExerciseId,
    m: Metric,
    from: NaiveDate,
    to: NaiveDate,
) -> Vec<(NaiveDate, f64)> {
    sessions_in(db, from, to)
        .filter_map(|(date, session)| {
            let log = session
                .logs
                .iter()
                .find(|l| l.exercise_id == ex && !l.sets.is_empty())?;
            Some((date, log_value(m, log)))
        })
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
) -> Vec<(NaiveDate, f64)> {
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
                total += log_value(m, log);
            }
            hit.then_some((date, total))
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
pub fn humanize_days(days: i64) -> String {
    match days.max(0) {
        0 => "今日".to_string(),
        1 => "昨日".to_string(),
        n => format!("{n}日前"),
    }
}

/// 日を跨いだら日数、同じ暦日なら時刻粒度。
///
/// ★ 「2日5時間」形式は廃止した。日数部分が経過ミリ秒 / 24h のローリング日数だったため、
///   チップ（日粒度）とヒーロー（時刻粒度）が違う日を指すことがあった（adr/data-model/elapsed-in-local-calendar-days.md）。
pub fn humanize(e: Elapsed) -> String {
    if e.days > 0 {
        return humanize_days(e.days);
    }
    // 同じ暦日。`at` があるときだけ時刻粒度まで出せる
    let Some(ms) = e.ms else {
        return humanize_days(0);
    };
    let minutes = ms / 60_000;
    if minutes < 1 {
        return "たった今".to_string();
    }
    if minutes < 60 {
        return format!("{minutes}分");
    }
    let hours = minutes / 60;
    // ★ 同じ暦日なのに 24 時間以上 = `at` と日付キーが矛盾している（壊れたバックアップの
    //   取り込み、タイムゾーン移動、copy_day の `at` 漏れの退行）。日付キーを勝たせる。
    //   ここが無いと「336時間」のような表示が出る
    if hours < 24 {
        format!("{hours}時間")
    } else {
        humanize_days(0)
    }
}

/// 部位チップ用の短い表記。"3d" / "今日"
///
/// ★ `views` ではなくここに置く。元は `views/mod.rs` にあったが、そこは wasm32 の
///   cfg gate の内側で `cargo test` が一度も触れず、`ms / 86_400_000` というバグが
///   誰にも検出されないまま残っていた（adr/architecture/chart-layout-as-a-testable-module.md と同じ理由でロジックを core に置く）。
pub fn short_elapsed(e: Elapsed) -> String {
    match e.days() {
        0 => "今日".to_string(),
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
                                // schema ≤2 にメモは無い
                                note: String::new(),
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
            .filter(|g| g.name == preset.name)
            .map(|g| g.id)
            .collect();
        if let [only] = matched[..] {
            groups.insert(only, preset.id);
        }

        for (preset_id, preset_name) in preset.exercises {
            let matched: Vec<u32> = old
                .exercises
                .iter()
                .filter(|e| e.name == *preset_name)
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

/// 書き出す文字列。`storage::save` が `localStorage` に書くのと同じ compact JSON。
///
/// **エクスポート形式 = 保存形式**をこの 1 関数に固定する。別形式を作ると、
/// 保存側の変更に追随し忘れて「書き出したファイルが読み戻せない」が静かに起きる。
pub fn export_json(db: &Db) -> String {
    serde_json::to_string(db).unwrap_or_else(|_| "{}".to_string())
}

/// 書き出しのファイル名。**時刻まで入れる。**
///
/// 日付だけだと、同じ日に 2 回書き出したとき 2 回目が 1 回目を上書きする。
/// 「バックアップを取ったつもりが前のバックアップを潰した」はこの機能の存在意義を消す。
pub fn export_filename(now: chrono::NaiveDateTime) -> String {
    format!("fitness-memo-{}.json", now.format("%Y%m%d-%H%M"))
}

/// 読み込みの失敗。**利用者の次の行動が変わる粒度でだけ分ける。**
#[derive(Debug, PartialEq, Eq)]
pub enum ImportError {
    /// 空。貼り付け忘れ / 空ファイル
    Empty,
    /// JSON として壊れている。途中で切れた可能性が高い
    NotJson,
    /// JSON だがこのアプリのデータではない
    NotDb,
    /// 新しい版で作られている
    Unsupported(u32),
}

impl ImportError {
    pub fn message(&self) -> String {
        match self {
            Self::Empty => "中身がありません".to_string(),
            Self::NotJson => {
                "データが途中で切れているようです（全文がコピーできているか確認してください）"
                    .to_string()
            }
            Self::NotDb => "このアプリの記録ではないようです".to_string(),
            Self::Unsupported(v) => {
                format!("新しい版（形式 {v}）で作られた記録です。アプリを更新してください")
            }
        }
    }
}

/// 貼り付け / ファイルの中身 → `Db`。
///
/// 順序が大事:
/// 1. 空なら `Empty`
/// 2. **素の [`migrate`] が通ったらそのまま返す** — 正しい入力には絶対に手を触れない
/// 3. 通らなければ [`repair`] してもう一度だけ試す
/// 4. それでも駄目なら、JSON として読めるかどうかで理由を分ける
pub fn parse_import(raw: &str, ids: &mut IdGen) -> Result<Db, ImportError> {
    if raw.trim().is_empty() {
        return Err(ImportError::Empty);
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
    /// 追加したトレーニングメニューの本数。
    pub routines_added: usize,
    pub conflicts: Vec<Conflict>,
}

impl MergeReport {
    /// 何も足さなかったか。
    ///
    /// ★ **ここに数を足したら `views::backup::report_text` にも必ず足すこと。**
    /// 片方だけだと `is_noop` が偽なのに文言の部品が空になり、画面に
    /// 「 を追加しました」だけが出る。
    pub fn is_noop(&self) -> bool {
        self.groups_added == 0
            && self.exercises_added == 0
            && self.sessions_added == 0
            && self.logs_added == 0
            && self.notes_added == 0
            && self.routines_added == 0
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
    for e in theirs.exercises {
        if let Some(existing) = mine.exercise(e.id) {
            if existing.name != e.name {
                // 改名は正当な操作。取り込み先の名前を残し、あったことだけ伝える
                report.conflicts.push(Conflict::Renamed {
                    kept: existing.name.clone(),
                    incoming: e.name.clone(),
                });
            }
            exercise_alias.insert(e.id, e.id);
            continue;
        }
        if let Some(existing) = mine.exercises.iter().find(|x| x.name == e.name) {
            exercise_alias.insert(e.id, existing.id);
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
            .map(|l| ExerciseLog {
                exercise_id: exercise_alias
                    .get(&l.exercise_id)
                    .copied()
                    .unwrap_or(l.exercise_id),
                ..l
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
                *existing = ExerciseLog { note, ..log };
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
    use crate::model::{Exercise, Group, SetEntry};
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
                    note: String::new(),
                })
                .collect(),
            at,
            note: String::new(),
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
                })
                .collect(),
            at,
            note: note.to_string(),
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

    /// ★ 単調性: 重量を足して指標が下がってはいけない。
    ///
    /// `max(1.0)` を外して素の積にすると 0.5kg×10 = 5 になり、
    /// 「自重 10 回（= 10）→ 0.5kg を持って 10 回（= 5）」でグラフが下がる。
    /// 上下が負荷の増減を表さなくなるのでグラフの意味が壊れる。
    #[test]
    fn set_volume_is_monotonic_in_weight() {
        let bodyweight = set_volume(&set(0.0, 10));
        assert_eq!(bodyweight, 10.0);
        // 1kg 未満でも自重を下回らない
        assert_eq!(set_volume(&set(0.5, 10)), 10.0);
        assert!(set_volume(&set(0.5, 10)) >= bodyweight);
        assert!(set_volume(&set(2.0, 10)) > bodyweight);
    }

    /// schema 1 からの値の変化を固定する。ここが動いたら ADR とリリースノートも直す。
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
        assert_eq!(Metric::Volume.unit(), "");
        assert_eq!(Metric::Sets.unit(), "セット");
        assert_eq!(Metric::Reps.unit(), "回");
        assert_eq!(Metric::default(), Metric::Volume);
        assert_eq!(Metric::CHOICES.len(), 3);
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
                note: String::new(),
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

        reorder_logs(&mut db, day, &[e(11), e(20), e(10)]);
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

        reorder_logs(&mut db, day, &[e(20)]);
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

        reorder_logs(&mut db, day, &[e(10), e(10), e(11)]);
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
        let back = parse_import(&raw, &mut ids()).expect("読み戻せる");

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
        assert_eq!(humanize(elapsed), "2日前");
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
    fn copy_day_does_not_copy_the_exercise_note_or_the_set_notes() {
        // 「肩に違和感」は前回の観測。複製すると起きていない観測が翌日に生える
        // （adr/data-model/notes-on-logs-and-sets.md）
        let mut db = menu_db();
        put(
            &mut db,
            d(2026, 8, 5),
            vec![noted_log(
                10,
                "フォームが崩れた",
                &[(60.0, 10, "軽い"), (60.0, 8, "肩に違和感")],
                None,
            )],
        );

        assert_eq!(
            copy_day(&mut db, d(2026, 8, 5), d(2026, 8, 8), None),
            vec![e(10)]
        );
        let log = &db.sessions.get(&date_key(d(2026, 8, 8))).unwrap().logs[0];
        assert_eq!(log.note, "", "種目メモは運ばない");
        assert!(
            log.sets.iter().all(|s| s.note.is_empty()),
            "セットメモは運ばない: {:?}",
            log.sets
        );
        // 重量・回数はメニュー構成なので運ぶ
        assert_eq!(
            log.sets
                .iter()
                .map(|s| (s.weight, s.reps))
                .collect::<Vec<_>>(),
            vec![(60.0, 10), (60.0, 8)]
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
    fn apply_routine_does_not_copy_the_set_notes_or_the_exercise_note() {
        // 「3 セット目で肩に違和感」は前回の観測。複製すると起きていない観測が生える
        let mut db = routine_db();
        put(
            &mut db,
            d(2026, 8, 5),
            vec![noted_log(10, "調子が良い", &[(60.0, 10, "重い")], None)],
        );

        apply_routine(&mut db, r(1), d(2026, 8, 8), None);
        let log = &db.sessions[&date_key(d(2026, 8, 8))].logs[0];
        assert_eq!(log.note, "");
        assert_eq!(log.sets[0].note, "");
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
    fn apply_routine_ignores_history_older_than_the_menu_lookback() {
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

        let series = exercise_series(&db, e(10), Metric::Volume, d(2026, 8, 1), d(2026, 8, 8));
        assert_eq!(
            series,
            vec![(d(2026, 8, 1), 500.0), (d(2026, 8, 8), 1080.0)]
        );

        // 未知の種目は空
        assert!(
            exercise_series(&db, e(999), Metric::Volume, d(2026, 8, 1), d(2026, 8, 8)).is_empty()
        );
        // from > to でもパニックしない
        assert!(
            exercise_series(&db, e(10), Metric::Volume, d(2026, 8, 8), d(2026, 8, 1)).is_empty()
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
        let at = |m| exercise_series(&db, e(10), m, d(2026, 8, 1), d(2026, 8, 8));

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

        let chest = |m| group_series(&db, g(1), m, d(2026, 8, 1), d(2026, 8, 2));

        // ★ 旧実装は Kind ごとに単位が違って足せず「セット数」固定だった。
        //   式が 1 本になったので、重量を使う種目と使わない種目を混ぜて合算できる
        assert_eq!(chest(Metric::Volume), vec![(d(2026, 8, 1), 1092.0)]);
        assert_eq!(chest(Metric::Sets), vec![(d(2026, 8, 1), 3.0)]);
        assert_eq!(chest(Metric::Reps), vec![(d(2026, 8, 1), 30.0)]);

        // 体幹（重量を使わない種目だけ）でも 0 に潰れない
        assert_eq!(
            group_series(&db, g(2), Metric::Volume, d(2026, 8, 1), d(2026, 8, 2)),
            vec![(d(2026, 8, 2), 60.0)]
        );
        // 種目が 1 つも無い部位は空
        assert!(group_series(&db, g(3), Metric::Volume, d(2026, 8, 1), d(2026, 8, 2)).is_empty());
    }

    #[test]
    fn group_series_skips_days_without_that_group() {
        let mut db = test_db();
        put(&mut db, d(2026, 8, 1), vec![log(20, &[(0.0, 60)], None)]); // 体幹だけの日
        put(&mut db, d(2026, 8, 2), vec![log(10, &[], None)]); // 空セット = 未実施

        // 胸の点は 1 つも立たない（0 の点を置くと「やったが 0」と区別できない）
        assert!(group_series(&db, g(1), Metric::Volume, d(2026, 8, 1), d(2026, 8, 2)).is_empty());
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
        let metric = exercise_series(&db, e(10), Metric::Volume, d(2026, 8, 1), d(2026, 8, 3));
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

        let weekly = aggregate_weekly_avg(&[(d(2026, 8, 2), 70.0), (d(2026, 8, 8), 72.0)]);
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
        assert_eq!(humanize(e), "2日前");
    }

    #[test]
    fn elapsed_since_last_works_without_any_at() {
        let mut db = test_db();
        // 8/8 に 8/7 分をバックフィルした状態（at は入らない）
        put(&mut db, d(2026, 8, 7), vec![log(10, &[(60.0, 10)], None)]);

        let e = elapsed_since_last(&db, 1_800_000_000_000, d(2026, 8, 8)).expect("記録がある");
        assert_eq!(e, Elapsed::days_only(1));
        assert_eq!(humanize(e), "昨日");
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
        assert_eq!(humanize(e), "3時間");
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
        assert_eq!(humanize(e), "30分");
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
        assert_eq!(humanize(e), "昨日");
        assert_eq!(short_elapsed(e), "1d");
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
        assert_eq!(humanize(e), "昨日");
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
        assert_eq!(short_elapsed(by_group[&g(1)]), "1d");
    }

    #[test]
    fn hero_and_chip_agree_on_the_day_count() {
        // ヒーロー（humanize）とチップ（short_elapsed）が違う日を指してはいけない
        for e in [
            Elapsed::with_ms(1, 12 * HOUR_MS),
            Elapsed::with_ms(2, 36 * HOUR_MS),
            Elapsed::days_only(3),
        ] {
            let days = e.days();
            assert_eq!(humanize(e), humanize_days(days));
            assert_eq!(short_elapsed(e), format!("{days}d"));
        }
    }

    // ── humanize ────────────────────────────────────────────────────────────

    #[test]
    fn humanize_covers_every_granularity() {
        // 同じ暦日 = 時刻粒度
        assert_eq!(humanize(Elapsed::with_ms(0, 45 * 60_000)), "45分");
        assert_eq!(humanize(Elapsed::with_ms(0, 59 * 60_000 + 59_999)), "59分");
        assert_eq!(humanize(Elapsed::with_ms(0, HOUR_MS)), "1時間");
        assert_eq!(
            humanize(Elapsed::with_ms(0, 23 * HOUR_MS + 59 * 60_000)),
            "23時間"
        );
        // at を持たない当日の記録（取り込んだデータ）は日粒度に落ちる
        assert_eq!(humanize(Elapsed::days_only(0)), "今日");

        // ★ 日を跨いだら必ず日粒度。「2日5時間」形式は廃止した
        assert_eq!(humanize(Elapsed::with_ms(1, 12 * HOUR_MS)), "昨日");
        assert_eq!(
            humanize(Elapsed::with_ms(2, 2 * DAY_MS + 5 * HOUR_MS)),
            "2日前"
        );
        assert_eq!(humanize(Elapsed::with_ms(2, 2 * DAY_MS)), "2日前");
        assert_eq!(humanize(Elapsed::days_only(1)), "昨日");
        assert_eq!(humanize(Elapsed::days_only(5)), "5日前");
    }

    #[test]
    fn humanize_clamps_negatives_and_sub_minute() {
        assert_eq!(humanize(Elapsed::with_ms(0, 0)), "たった今");
        assert_eq!(humanize(Elapsed::with_ms(0, 30_000)), "たった今");
        // 端末時計のズレでも壊れた表示にしない
        assert_eq!(humanize(Elapsed::with_ms(0, -5000)), "たった今");
        assert_eq!(humanize(Elapsed::with_ms(-1, 5_000)), "たった今");
        assert_eq!(humanize(Elapsed::days_only(-1)), "今日");
    }

    #[test]
    fn humanize_falls_back_to_the_date_key_when_at_contradicts_it() {
        // 同じ暦日なのに 2 週間分の経過 = 壊れたデータ。「336時間」を出さず日付キーを勝たせる
        assert_eq!(humanize(Elapsed::with_ms(0, 14 * DAY_MS)), "今日");
    }

    #[test]
    fn short_elapsed_and_recency_use_calendar_days() {
        assert_eq!(short_elapsed(Elapsed::days_only(0)), "今日");
        assert_eq!(short_elapsed(Elapsed::with_ms(1, 12 * HOUR_MS)), "1d");
        assert_eq!(short_elapsed(Elapsed::days_only(10)), "10d");

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
                    note: String::new(),
                },
                SetEntry {
                    weight: 60.0,
                    reps: 8,
                    note: String::new(),
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
        let mut db = crate::presets::seeded_db();
        let bench = db.exercises[0].id;
        put(
            &mut db,
            d(2026, 8, 8),
            vec![ExerciseLog {
                exercise_id: bench,
                sets: vec![SetEntry {
                    weight: 60.0,
                    reps: 10,
                    note: String::new(),
                }],
                at: Some(1_800_000_000_000),
                note: String::new(),
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

    // ── 書き出し / 読み込み ──────────────────────────────────────────────────

    /// ★ この機能の生命線。書き出したものが、そのまま読み戻せる。
    #[test]
    fn export_round_trips_through_parse_import() {
        let mut db = crate::presets::seeded_db();
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        db.sessions.insert(
            date_key(d(2026, 8, 8)),
            Session {
                logs: vec![ExerciseLog {
                    exercise_id: bench,
                    sets: vec![SetEntry {
                        weight: 60.0,
                        reps: 10,
                        note: String::new(),
                    }],
                    at: Some(1_800_000_000_000),
                    note: String::new(),
                }],
                body_weight: Some(70.5),
                note: "調子よい".into(),
            },
        );

        let raw = export_json(&db);
        assert_eq!(parse_import(&raw, &mut ids()).expect("読み戻せる"), db);
    }

    #[test]
    fn export_round_trips_the_routines_too() {
        // 書き出し形式 = 保存形式なので `export_json` に手は要らないが、
        // 「メニューが往復で消えない」は明示的に固定しておく
        let mut db = crate::presets::seeded_db();
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        db.routines.push(Routine {
            id: r(1),
            name: "胸の日".into(),
            exercises: vec![bench],
        });

        let raw = export_json(&db);
        assert_eq!(parse_import(&raw, &mut ids()).expect("読み戻せる"), db);
    }

    #[test]
    fn import_errors_tell_the_user_what_to_do_next() {
        assert_eq!(parse_import("", &mut ids()), Err(ImportError::Empty));
        assert_eq!(parse_import("   \n ", &mut ids()), Err(ImportError::Empty));
        // 途中で切れた
        assert_eq!(
            parse_import(r#"{"schema":3,"groups":"#, &mut ids()),
            Err(ImportError::NotJson)
        );
        // JSON ではあるが別物
        assert_eq!(parse_import("[1,2,3]", &mut ids()), Err(ImportError::NotDb));
        assert_eq!(parse_import("null", &mut ids()), Err(ImportError::NotDb));
        assert_eq!(parse_import("{}", &mut ids()), Err(ImportError::NotDb));
        // 新しい版
        assert_eq!(
            parse_import(
                r#"{"schema":99,"groups":[],"exercises":[],"sessions":{}}"#,
                &mut ids()
            ),
            Err(ImportError::Unsupported(99))
        );
    }

    #[test]
    fn import_repairs_decorations_added_by_copy_paste() {
        let db = crate::presets::seeded_db();
        let raw = export_json(&db);

        for decorated in [
            format!("\u{feff}{raw}"),       // BOM
            format!("\n\n  {raw}  \n"),     // 前後の空白
            format!("```json\n{raw}\n```"), // コードフェンス
            format!("```\n{raw}\n```"),     // 言語指定なし
            raw.replace('"', "\u{201d}"),   // 全角引用符に化けた
        ] {
            assert_eq!(
                parse_import(&decorated, &mut ids()).expect("修復して読める"),
                db
            );
        }
    }

    /// ★ `repair` は素のパースが失敗したときしか動かない。だから種目名に全角引用符が
    /// 入っていても壊さない。ここが崩れると、正常なデータを黙って書き換える。
    #[test]
    fn import_does_not_touch_valid_data_containing_curly_quotes() {
        let mut db = crate::presets::seeded_db();
        db.exercises.push(Exercise {
            id: ExerciseId::from_bits(0xD00D),
            name: "\u{201c}特別\u{201d}なベンチ".into(),
            group_id: crate::presets::preset_group_id("胸").expect("プリセット"),
            order: 9,
            archived: false,
        });

        let raw = export_json(&db);
        let back = parse_import(&raw, &mut ids()).expect("読み戻せる");

        assert_eq!(back, db);
        assert!(back.exercises.iter().any(|e| e.name.contains('\u{201c}')));
    }

    #[test]
    fn export_filename_includes_the_time_so_a_second_export_does_not_clobber_the_first() {
        let at = |h, m| d(2026, 8, 8).and_hms_opt(h, m, 0).expect("有効な時刻");
        assert_eq!(export_filename(at(9, 5)), "fitness-memo-20260808-0905.json");
        assert_ne!(export_filename(at(9, 5)), export_filename(at(18, 30)));
    }

    #[test]
    fn summarize_counts_what_the_confirmation_screen_shows() {
        let mut db = crate::presets::seeded_db();
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
                            note: String::new(),
                        },
                        SetEntry {
                            weight: 60.0,
                            reps: 8,
                            note: String::new(),
                        },
                    ],
                    at: None,
                    note: String::new(),
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
        let mut db = crate::presets::seeded_db();
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

        let db = parse_import(raw, &mut ids()).expect("読める");

        let sets = &db.sessions["2026-08-08"].logs[0].sets;
        assert_eq!(sets.len(), 1, "表せない重量が残った: {sets:?}");
        assert_eq!(sets[0].weight, 60.0);
        assert_eq!(db.sessions["2026-08-08"].body_weight, None);

        // ★ 肝心なのは「書き戻せること」。ここが往復するなら次の起動で死なない
        let round = parse_import(&export_json(&db), &mut ids()).expect("読み戻せる");
        assert_eq!(round, db);
        // `body_weight` の null は Option の正常な表現。危険なのは重量側の null
        // （f32 に戻せず、次の起動で丸ごと Broken になる）
        assert!(
            !export_json(&db).contains("\"weight\":null"),
            "Infinity が weight の null として書き出された"
        );
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
        let mut db = crate::presets::seeded_db();
        let bench = crate::presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        let chest = crate::presets::preset_group_id("胸").expect("プリセット");
        db.exercises.push(Exercise {
            id: ExerciseId::from_bits(0xAAA1),
            name: "わたしの種目".into(),
            group_id: chest,
            order: 9,
            archived: false,
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
                            note: String::new(),
                        }],
                        at: None,
                        note: String::new(),
                    },
                    ExerciseLog {
                        exercise_id: ExerciseId::from_bits(0xAAA1),
                        sets: vec![SetEntry {
                            weight: 20.0,
                            reps: 15,
                            note: String::new(),
                        }],
                        at: None,
                        note: String::new(),
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
        let mut db = crate::presets::seeded_db();
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
                            note: String::new(),
                        }],
                        at: None,
                        note: String::new(),
                    },
                    ExerciseLog {
                        exercise_id: ExerciseId::from_bits(0xBBB1),
                        sets: vec![SetEntry {
                            weight: 45.0,
                            reps: 6,
                            note: String::new(),
                        }],
                        at: None,
                        note: String::new(),
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
            let mut db = crate::presets::seeded_db();
            db.sessions.insert(
                day.clone(),
                Session {
                    logs: vec![ExerciseLog {
                        exercise_id: bench,
                        sets,
                        at: None,
                        note: String::new(),
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
                    note: String::new(),
                },
                SetEntry {
                    weight: 60.0,
                    reps: 8,
                    note: String::new(),
                },
            ]
        };
        let heavy = || {
            vec![
                SetEntry {
                    weight: 62.0,
                    reps: 10,
                    note: String::new(),
                },
                SetEntry {
                    weight: 60.0,
                    reps: 8,
                    note: String::new(),
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
        let mut mine = crate::presets::seeded_db();
        mine.exercises
            .iter_mut()
            .find(|e| e.id == bench)
            .expect("プリセット")
            .name = "マイベンチ".into();

        // theirs: 同じ ID の「ベンチプレス」と、別 ID の「マイベンチ」を両方持つ
        let mut theirs = crate::presets::seeded_db();
        let other = ExerciseId::from_bits(0xE001);
        theirs.exercises.push(Exercise {
            id: other,
            name: "マイベンチ".into(),
            group_id: chest,
            order: 9,
            archived: false,
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
                            note: String::new(),
                        }],
                        at: None,
                        note: String::new(),
                    },
                    ExerciseLog {
                        exercise_id: other,
                        sets: vec![SetEntry {
                            weight: 40.0,
                            reps: 12,
                            note: String::new(),
                        }],
                        at: None,
                        note: String::new(),
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
        let reloaded = parse_import(&export_json(&mine), &mut ids()).expect("読み戻せる");
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
        let mut b = crate::presets::seeded_db();
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
        (crate::presets::seeded_db(), crate::presets::seeded_db())
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
            let mut db = crate::presets::seeded_db();
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
                            })
                            .collect(),
                        at: None,
                        note: note.to_string(),
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
        let mut a = crate::presets::seeded_db();
        // A には「じぶんの種目」を ID X で持たせる
        a.exercises.push(Exercise {
            id: ExerciseId::from_bits(0xB001),
            name: "じぶんの種目".into(),
            group_id: crate::presets::preset_group_id("胸").expect("プリセット"),
            order: 9,
            archived: false,
        });

        // B は同名の種目を**別の ID** で持ち、A に無い日に記録している
        let mut b = crate::presets::seeded_db();
        b.exercises.push(Exercise {
            id: ExerciseId::from_bits(0xC002),
            name: "じぶんの種目".into(),
            group_id: crate::presets::preset_group_id("胸").expect("プリセット"),
            order: 9,
            archived: false,
        });
        b.sessions.insert(
            date_key(d(2026, 9, 9)),
            Session {
                logs: vec![ExerciseLog {
                    exercise_id: ExerciseId::from_bits(0xC002),
                    sets: vec![SetEntry {
                        weight: 40.0,
                        reps: 12,
                        note: String::new(),
                    }],
                    at: None,
                    note: String::new(),
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
}
