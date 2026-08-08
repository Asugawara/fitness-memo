//! 純ロジック。**`leptos` / `web-sys` を一切 import しない。**
//!
//! `cargo test`（ホストターゲット）で検証する層。UI から呼ぶ計算はすべてここに置き、
//! 画面側は結果を並べるだけにする。

use std::collections::{BTreeMap, HashMap};

use chrono::{Datelike, NaiveDate, TimeDelta};

use crate::model::{Db, ExerciseId, ExerciseLog, GroupId, SCHEMA, Session, SetEntry};

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

// ── メニューのコピー ────────────────────────────────────────────────────────

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
/// ★ [`recent_menus`] と [`copy_day`] は必ず**これを通す**。2 つのフィルタがずれると
/// 「5 種目」と表示された候補を押しても何も起きない死んだボタンができる。
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
pub fn copy_day(db: &mut Db, from: NaiveDate, to: NaiveDate, at: Option<i64>) -> Vec<ExerciseId> {
    // `&db` と `&mut db` の借用を分けるため先に取り出しておく
    let picked: Vec<(ExerciseId, Vec<SetEntry>)> = copyable(db, from)
        .map(|l| (l.exercise_id, l.sets.clone()))
        .collect();
    if picked.is_empty() {
        return Vec::new();
    }

    // ★ 既にログのある日には書かない。UI は空の日にしか導線を出さないが、カードの
    //   再構築は Effect 経由なので「ログのある日 × 空のカード」の 1 tick が存在する。
    //   加えて、空セットのログが残る旧データを弾かないと exercise_id が重複して
    //   「1 日 1 種目 1 ログ」が壊れる
    let to_key = date_key(to);
    if db.sessions.get(&to_key).is_some_and(|s| !s.logs.is_empty()) {
        return Vec::new();
    }

    // ★ or_default で取る。insert で置き換えると、空の日に先に打ち込まれた
    //   体重・体調メモが消える（ConditionRow は 1 文字ごとに commit する）
    let session = db.sessions.entry(to_key).or_default();
    let mut copied = Vec::with_capacity(picked.len());
    for (exercise_id, sets) in picked {
        copied.push(exercise_id);
        // ★ ExerciseLog を clone してはいけない。clone すると元の日の `at` を
        //   引き継ぎ、`at = None` にしたい過去日バックフィルに古い epoch が入る。
        //   すると elapsed_matching が Exact を返し、「14日3時間」のような
        //   捏造された精度が表示される（ADR-0006 が防いでいるのはこれ）
        session.logs.push(ExerciseLog {
            exercise_id,
            sets,
            at,
        });
    }
    copied
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

// ── 経過時間 ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Elapsed {
    /// 当日入力された `at` がある場合の経過ミリ秒
    Exact(i64),
    /// 過去日バックフィル（`at` が全て `None`）の場合の経過日数
    Days(i64),
}

/// `keep` に合致するログを持つ最新セッション（`today` 以前）から経過時間を出す。
///
/// そのセッションのログに `Some(at)` があれば `Exact(now_ms - max(at))`、
/// なければ `Days(today - 日付キー)`。
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
            Some(match last_at {
                Some(at) => Elapsed::Exact((now_ms - at).max(0)),
                None => Elapsed::Days((today - date).num_days().max(0)),
            })
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

/// `Exact` → 「45分」「5時間」「2日5時間」、`Days` → 「今日 / 昨日 / N日前」。
pub fn humanize(e: Elapsed) -> String {
    match e {
        Elapsed::Exact(ms) => {
            let minutes = ms.max(0) / 60_000;
            if minutes < 1 {
                return "たった今".to_string();
            }
            let hours = minutes / 60;
            if hours < 1 {
                return format!("{minutes}分");
            }
            let days = hours / 24;
            if days < 1 {
                return format!("{hours}時間");
            }
            match hours % 24 {
                0 => format!("{days}日"),
                rest => format!("{days}日{rest}時間"),
            }
        }
        Elapsed::Days(d) => match d.max(0) {
            0 => "今日".to_string(),
            1 => "昨日".to_string(),
            n => format!("{n}日前"),
        },
    }
}

// ── 復元 ────────────────────────────────────────────────────────────────────

/// schema 差の吸収 + 「1 日 1 種目 1 ログ」への正規化。
///
/// `Err` なら呼び側（`storage.rs`）が raw を退避してからプリセット入りの `Db` に
/// フォールバックする。**破損データをプリセットで黙って上書きしない**ための境界。
///
/// 正規化の内容:
/// - 日付キーを `%Y-%m-%d` に再正規化し、パースできないキーのセッションは捨てる
///   （辞書順 = 時系列順の前提が壊れ、どの画面からも到達できないため）
/// - 同一 `exercise_id` の重複ログをマージ（セットを連結、`at` は `Some` の最大値）
/// - セットが空のログを捨て、ログも体重もメモも無いセッションを捨てる
/// - `next_id` が既存 ID 以下なら繰り上げる（ID 衝突を作らない）
pub fn migrate(raw: &str) -> Result<Db, serde_json::Error> {
    let mut db: Db = serde_json::from_str(raw)?;

    let mut sessions: BTreeMap<String, Session> = BTreeMap::new();
    for (key, session) in std::mem::take(&mut db.sessions) {
        let Some(date) = parse_date_key(&key) else {
            continue;
        };
        merge_into(sessions.entry(date_key(date)).or_default(), session);
    }
    for session in sessions.values_mut() {
        dedupe_logs(session);
    }
    sessions.retain(|_, s| !s.is_empty());
    db.sessions = sessions;

    let max_id = db
        .groups
        .iter()
        .map(|g| g.id)
        .chain(db.exercises.iter().map(|e| e.id))
        .chain(
            db.sessions
                .values()
                .flat_map(|s| s.logs.iter().map(|l| l.exercise_id)),
        )
        .max()
        .unwrap_or(0);
    db.next_id = db.next_id.max(max_id.saturating_add(1));
    db.schema = SCHEMA;

    Ok(db)
}

/// 正規化で同じ日付キーに落ちた 2 つのセッションを 1 つにまとめる。
fn merge_into(dst: &mut Session, src: Session) {
    dst.logs.extend(src.logs);
    if dst.body_weight.is_none() {
        dst.body_weight = src.body_weight;
    }
    if dst.note.trim().is_empty() {
        dst.note = src.note;
    } else if !src.note.trim().is_empty() {
        dst.note.push('\n');
        dst.note.push_str(&src.note);
    }
}

/// 「1 日 1 種目 1 ログ」への正規化。初出の順序は保つ。
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
        .filter(|l| !l.sets.is_empty())
        .collect();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Exercise, Group, SetEntry};
    use chrono::Weekday;

    const HOUR_MS: i64 = 3_600_000;
    const DAY_MS: i64 = 24 * HOUR_MS;

    // 胸(1): ベンチプレス(10) / プッシュアップ(11)
    // 体幹(2): プランク(20)
    // 脚(3): 種目なし
    //
    // 旧 Kind でいう Weighted / Bodyweight / Duration が 1 つずつ混ざる構成のまま
    // （指標の式が 1 本になっても、混在部位の合算が壊れないことを見たいので）
    fn test_db() -> Db {
        let mut db = Db {
            next_id: 100,
            ..Db::default()
        };
        db.groups.push(Group {
            id: 1,
            name: "胸".into(),
            color: "#e0524a".into(),
            order: 0,
        });
        db.groups.push(Group {
            id: 2,
            name: "体幹".into(),
            color: "#6b7280".into(),
            order: 1,
        });
        db.groups.push(Group {
            id: 3,
            name: "脚".into(),
            color: "#2fa06a".into(),
            order: 2,
        });
        db.exercises.push(ex(10, "ベンチプレス", 1));
        db.exercises.push(ex(11, "プッシュアップ", 1));
        db.exercises.push(ex(20, "プランク", 2));
        db
    }

    fn ex(id: ExerciseId, name: &str, group_id: GroupId) -> Exercise {
        Exercise {
            id,
            name: name.into(),
            group_id,
            order: 0,
            archived: false,
        }
    }

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).expect("有効な日付")
    }

    fn log(exercise_id: ExerciseId, sets: &[(f32, u32)], at: Option<i64>) -> ExerciseLog {
        ExerciseLog {
            exercise_id,
            sets: sets
                .iter()
                .map(|(weight, reps)| SetEntry {
                    weight: *weight,
                    reps: *reps,
                })
                .collect(),
            at,
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

    // ── 指標 ────────────────────────────────────────────────────────────────

    fn set(weight: f32, reps: u32) -> SetEntry {
        SetEntry { weight, reps }
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
        let (date, l) = last_log_before(&db, 10, d(2026, 8, 8)).expect("8/4 がある");
        assert_eq!(date, d(2026, 8, 4));
        assert_eq!(
            l.sets,
            vec![SetEntry {
                weight: 55.0,
                reps: 10
            }]
        );

        let (date, _) = last_log_before(&db, 10, d(2026, 8, 4)).expect("8/1 がある");
        assert_eq!(date, d(2026, 8, 1));

        // 最古の記録日より前には何もない
        assert_eq!(last_log_before(&db, 10, d(2026, 8, 1)), None);
        // 別種目の記録は拾わない
        assert_eq!(last_log_before(&db, 11, d(2026, 8, 8)), None);
    }

    #[test]
    fn last_log_before_skips_sessions_without_that_exercise() {
        let mut db = test_db();
        put(&mut db, d(2026, 8, 1), vec![log(10, &[(50.0, 10)], None)]);
        put(&mut db, d(2026, 8, 5), vec![log(20, &[(0.0, 60)], None)]);
        put(&mut db, d(2026, 8, 7), vec![log(10, &[], None)]); // 空セットは記録ではない

        let (date, _) = last_log_before(&db, 10, d(2026, 8, 8)).expect("8/1 まで遡る");
        assert_eq!(date, d(2026, 8, 1));
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
        assert_eq!(got[0].exercises, vec![20]);

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
        assert_eq!(got[0].exercises, vec![30]);
        assert_eq!(got[1].exercises, vec![31]);
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
        assert_eq!(got[1].exercises, vec![11, 10]);
    }

    #[test]
    fn recent_menus_skips_days_with_nothing_copyable() {
        let mut db = menu_db();
        db.exercises.push(Exercise {
            archived: true,
            ..ex(40, "封印した種目", 1)
        });
        // 空セットだけの日
        put(&mut db, d(2026, 8, 2), vec![log(10, &[], None)]);
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
        assert_eq!(got[0].exercises, vec![10], "アーカイブ済みは数にも入れない");
    }

    #[test]
    fn recent_menus_stops_at_the_lookback_limit() {
        let mut db = menu_db();
        let before = d(2026, 8, 8);
        put(
            &mut db,
            before - TimeDelta::days(MENU_LOOKBACK_DAYS - 1),
            vec![log(10, &[(50.0, 10)], None)],
        );
        put(
            &mut db,
            before - TimeDelta::days(MENU_LOOKBACK_DAYS + 1),
            vec![log(20, &[(0.0, 60)], None)],
        );

        let got = recent_menus(&db, before, 4);
        assert_eq!(got.len(), 1, "打ち切りより古い日は候補にしない");
        assert_eq!(got[0].exercises, vec![10]);
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
        assert_eq!(copied, vec![10, 11]);

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
            vec![10, 11],
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
        // ★ ExerciseLog を clone すると元の `at` が付いてくる。ADR-0006 の回帰テスト
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
        assert_eq!(elapsed, Elapsed::Days(2), "日付キーだけで測る");
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
        // 「1 日 1 種目 1 ログ」（ADR-0008）が壊れる
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
            vec![10]
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

        let series = exercise_series(&db, 10, Metric::Volume, d(2026, 8, 1), d(2026, 8, 8));
        assert_eq!(
            series,
            vec![(d(2026, 8, 1), 500.0), (d(2026, 8, 8), 1080.0)]
        );

        // 未知の種目は空
        assert!(exercise_series(&db, 999, Metric::Volume, d(2026, 8, 1), d(2026, 8, 8)).is_empty());
        // from > to でもパニックしない
        assert!(exercise_series(&db, 10, Metric::Volume, d(2026, 8, 8), d(2026, 8, 1)).is_empty());
    }

    #[test]
    fn exercise_series_switches_axis_with_the_metric() {
        let mut db = test_db();
        put(
            &mut db,
            d(2026, 8, 8),
            vec![log(10, &[(60.0, 10), (60.0, 8)], None)],
        );
        let at = |m| exercise_series(&db, 10, m, d(2026, 8, 1), d(2026, 8, 8));

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

        let chest = |m| group_series(&db, 1, m, d(2026, 8, 1), d(2026, 8, 2));

        // ★ 旧実装は Kind ごとに単位が違って足せず「セット数」固定だった。
        //   式が 1 本になったので、重量を使う種目と使わない種目を混ぜて合算できる
        assert_eq!(chest(Metric::Volume), vec![(d(2026, 8, 1), 1092.0)]);
        assert_eq!(chest(Metric::Sets), vec![(d(2026, 8, 1), 3.0)]);
        assert_eq!(chest(Metric::Reps), vec![(d(2026, 8, 1), 30.0)]);

        // 体幹（重量を使わない種目だけ）でも 0 に潰れない
        assert_eq!(
            group_series(&db, 2, Metric::Volume, d(2026, 8, 1), d(2026, 8, 2)),
            vec![(d(2026, 8, 2), 60.0)]
        );
        // 種目が 1 つも無い部位は空
        assert!(group_series(&db, 3, Metric::Volume, d(2026, 8, 1), d(2026, 8, 2)).is_empty());
    }

    #[test]
    fn group_series_skips_days_without_that_group() {
        let mut db = test_db();
        put(&mut db, d(2026, 8, 1), vec![log(20, &[(0.0, 60)], None)]); // 体幹だけの日
        put(&mut db, d(2026, 8, 2), vec![log(10, &[], None)]); // 空セット = 未実施

        // 胸の点は 1 つも立たない（0 の点を置くと「やったが 0」と区別できない）
        assert!(group_series(&db, 1, Metric::Volume, d(2026, 8, 1), d(2026, 8, 2)).is_empty());
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
        assert_eq!(used_exercise_ids(&db), vec![10, 20]);
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

    // ── 経過時間 ────────────────────────────────────────────────────────────

    #[test]
    fn elapsed_since_last_takes_the_exact_branch_when_at_is_some() {
        let mut db = test_db();
        let at = 1_800_000_000_000;
        put(
            &mut db,
            d(2026, 8, 6),
            vec![log(10, &[(60.0, 10)], Some(at))],
        );

        let now = at + 2 * DAY_MS + 5 * HOUR_MS;
        let e = elapsed_since_last(&db, now, d(2026, 8, 8)).expect("記録がある");
        assert_eq!(e, Elapsed::Exact(2 * DAY_MS + 5 * HOUR_MS));
        assert_eq!(humanize(e), "2日5時間");
    }

    #[test]
    fn elapsed_since_last_takes_the_days_branch_when_every_at_is_none() {
        let mut db = test_db();
        // 8/8 に 8/7 分をバックフィルした状態（at は入らない）
        put(&mut db, d(2026, 8, 7), vec![log(10, &[(60.0, 10)], None)]);

        let e = elapsed_since_last(&db, 1_800_000_000_000, d(2026, 8, 8)).expect("記録がある");
        assert_eq!(e, Elapsed::Days(1));
        // ★ ここが at: Option の要。now を入れていたら「たった今」になってしまう
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
        assert_eq!(e, Elapsed::Exact(3 * HOUR_MS));
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
        assert_eq!(e, Elapsed::Exact(30 * 60_000));
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
        assert_eq!(e, Elapsed::Days(5));

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
            by_group.get(&1),
            Some(&Elapsed::Exact(2 * DAY_MS + 5 * HOUR_MS))
        );
        assert_eq!(by_group.get(&2), Some(&Elapsed::Days(7)));
        // 種目が無い部位・未実施の部位はキーごと出ない（画面が「—」を出す）
        assert_eq!(by_group.get(&3), None);
        assert_eq!(by_group.len(), 2);
    }

    // ── humanize ────────────────────────────────────────────────────────────

    #[test]
    fn humanize_covers_every_granularity() {
        // 分
        assert_eq!(humanize(Elapsed::Exact(45 * 60_000)), "45分");
        assert_eq!(humanize(Elapsed::Exact(59 * 60_000 + 59_999)), "59分");
        // 時間
        assert_eq!(humanize(Elapsed::Exact(HOUR_MS)), "1時間");
        assert_eq!(
            humanize(Elapsed::Exact(23 * HOUR_MS + 59 * 60_000)),
            "23時間"
        );
        // 日 + 時間
        assert_eq!(
            humanize(Elapsed::Exact(2 * DAY_MS + 5 * HOUR_MS)),
            "2日5時間"
        );
        assert_eq!(humanize(Elapsed::Exact(2 * DAY_MS)), "2日");
        // 日粒度
        assert_eq!(humanize(Elapsed::Days(0)), "今日");
        assert_eq!(humanize(Elapsed::Days(1)), "昨日");
        assert_eq!(humanize(Elapsed::Days(5)), "5日前");
    }

    #[test]
    fn humanize_clamps_negatives_and_sub_minute() {
        assert_eq!(humanize(Elapsed::Exact(0)), "たった今");
        assert_eq!(humanize(Elapsed::Exact(30_000)), "たった今");
        // 端末時計のズレでも壊れた表示にしない
        assert_eq!(humanize(Elapsed::Exact(-5000)), "たった今");
        assert_eq!(humanize(Elapsed::Days(-1)), "今日");
    }

    // ── migrate ─────────────────────────────────────────────────────────────

    #[test]
    fn migrate_returns_err_for_broken_json() {
        // 呼び側はこの Err を見て raw を .bak-<epoch> に退避する
        assert!(migrate("").is_err());
        assert!(migrate("{壊れている").is_err());
        assert!(migrate("null").is_err());
        assert!(migrate("[1,2,3]").is_err());
        // 形は JSON でも必須フィールドが欠けていれば Err（プリセットで黙って上書きしない）
        assert!(migrate(r#"{"schema":1}"#).is_err());
        assert!(migrate(r#"{"schema":1,"next_id":1,"groups":[],"exercises":[]}"#).is_err());
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

        let db = migrate(raw).expect("正当な JSON");
        let session = &db.sessions["2026-08-08"];

        assert_eq!(
            session.logs.len(),
            2,
            "同一 exercise_id は 1 ログに畳まれる"
        );
        // 初出の順序が保たれる
        assert_eq!(session.logs[0].exercise_id, 10);
        assert_eq!(session.logs[1].exercise_id, 20);
        // セットは出現順に連結
        assert_eq!(
            session.logs[0].sets,
            vec![
                SetEntry {
                    weight: 60.0,
                    reps: 10
                },
                SetEntry {
                    weight: 60.0,
                    reps: 8
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

        let db = migrate(raw).expect("正当な JSON");
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

        let db = migrate(raw).expect("正当な JSON");

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

        let db = migrate(raw).expect("正当な JSON");

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
    fn migrate_repairs_next_id_so_new_ids_cannot_collide() {
        // 色に # が入るので r##"…"## にする（r#"…"# だと `"#e0524a` が終端になる）
        let raw = r##"{
          "schema": 0, "next_id": 1,
          "groups": [{"id": 3, "name": "胸", "color": "#e0524a", "order": 0}],
          "exercises": [{"id": 42, "name": "ベンチプレス", "group_id": 3, "kind": "Weighted", "order": 0}],
          "sessions": {}
        }"##;

        let mut db = migrate(raw).expect("正当な JSON");

        assert_eq!(db.schema, SCHEMA);
        assert_eq!(db.next_id, 43);
        assert_eq!(db.alloc_id(), 43);
        // archived は serde default で補われる
        assert!(!db.exercises[0].archived);
    }

    #[test]
    fn migrate_round_trips_a_seeded_db() {
        let mut db = crate::presets::seeded_db();
        let bench = db.exercises[0].id;
        put(
            &mut db,
            d(2026, 8, 8),
            vec![log(bench, &[(60.0, 10)], Some(1_800_000_000_000))],
        );

        let raw = serde_json::to_string(&db).expect("直列化できる");
        assert_eq!(migrate(&raw).expect("復元できる"), db);
    }
}
