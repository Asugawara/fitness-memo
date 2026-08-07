//! 純ロジック。**`leptos` / `web-sys` を一切 import しない。**
//!
//! `cargo test`（ホストターゲット）で検証する層。UI から呼ぶ計算はすべてここに置き、
//! 画面側は結果を並べるだけにする。

use std::collections::{BTreeMap, HashMap};

use chrono::{Datelike, NaiveDate, TimeDelta};

use crate::model::{Db, ExerciseId, ExerciseLog, GroupId, Kind, SCHEMA, Session};

/// `Db::sessions` のキー書式。ゼロ埋め ISO なので辞書順 = 時系列順になる。
pub const DATE_FMT: &str = "%Y-%m-%d";

pub fn date_key(d: NaiveDate) -> String {
    d.format(DATE_FMT).to_string()
}

pub fn parse_date_key(k: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(k, DATE_FMT).ok()
}

// ── 指標 ────────────────────────────────────────────────────────────────────

pub fn set_metric(kind: Kind, s: &crate::model::SetEntry) -> f64 {
    match kind {
        Kind::Weighted => f64::from(s.weight) * f64::from(s.reps),
        // Bodyweight の weight は「追加重量」。表示はするが指標には folding しない
        // （系列の一貫性を優先。加重の進行はセット表示 `+10kg × 8` で読む）
        Kind::Bodyweight | Kind::Duration => f64::from(s.reps),
    }
}

pub fn log_metric(kind: Kind, l: &ExerciseLog) -> f64 {
    l.sets.iter().map(|s| set_metric(kind, s)).sum()
}

pub fn unit_of(kind: Kind) -> &'static str {
    match kind {
        Kind::Weighted => "kg·回",
        Kind::Bodyweight => "回",
        Kind::Duration => "秒",
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

/// 種目別の推移。値は `Kind` に応じた指標（volume / 総レップ / 総秒）。
pub fn exercise_series(
    db: &Db,
    ex: ExerciseId,
    from: NaiveDate,
    to: NaiveDate,
) -> Vec<(NaiveDate, f64)> {
    let Some(kind) = db.exercise(ex).map(|e| e.kind) else {
        return Vec::new();
    };
    sessions_in(db, from, to)
        .filter_map(|(date, session)| {
            let log = session
                .logs
                .iter()
                .find(|l| l.exercise_id == ex && !l.sets.is_empty())?;
            Some((date, log_metric(kind, log)))
        })
        .collect()
}

/// 部位別の推移は **セット数**。
///
/// `Kind` が混在する部位（体幹は Bodyweight/Duration 中心）で volume を合算すると
/// 意味を失い恒常的にほぼ 0 になるため。週あたりセット数は部位のトレーニング量として
/// 標準的な指標でもある。
pub fn group_set_series(
    db: &Db,
    g: GroupId,
    from: NaiveDate,
    to: NaiveDate,
) -> Vec<(NaiveDate, f64)> {
    let ids = db.exercise_ids_of_group(g);
    if ids.is_empty() {
        return Vec::new();
    }
    sessions_in(db, from, to)
        .filter_map(|(date, session)| {
            let sets: usize = session
                .logs
                .iter()
                .filter(|l| ids.contains(&l.exercise_id))
                .map(|l| l.sets.len())
                .sum();
            (sets > 0).then_some((date, sets as f64))
        })
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

    // 胸(1): ベンチプレス(10, Weighted) / プッシュアップ(11, Bodyweight)
    // 体幹(2): プランク(20, Duration)
    // 脚(3): 種目なし
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
        db.exercises.push(ex(10, "ベンチプレス", 1, Kind::Weighted));
        db.exercises
            .push(ex(11, "プッシュアップ", 1, Kind::Bodyweight));
        db.exercises.push(ex(20, "プランク", 2, Kind::Duration));
        db
    }

    fn ex(id: ExerciseId, name: &str, group_id: GroupId, kind: Kind) -> Exercise {
        Exercise {
            id,
            name: name.into(),
            group_id,
            kind,
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

    #[test]
    fn set_metric_per_kind() {
        let s = SetEntry {
            weight: 60.0,
            reps: 10,
        };
        assert_eq!(set_metric(Kind::Weighted, &s), 600.0);
        // Bodyweight の weight（= 追加重量）は指標に folding しない
        assert_eq!(set_metric(Kind::Bodyweight, &s), 10.0);
        // Duration は reps を秒として扱う
        assert_eq!(
            set_metric(
                Kind::Duration,
                &SetEntry {
                    weight: 0.0,
                    reps: 60
                }
            ),
            60.0
        );
    }

    #[test]
    fn log_metric_per_kind() {
        // 計画の実例: 60×10 + 60×8 = 1,080
        let weighted = log(10, &[(60.0, 10), (60.0, 8)], None);
        assert_eq!(log_metric(Kind::Weighted, &weighted), 1080.0);

        let bodyweight = log(11, &[(0.0, 12), (10.0, 8)], None);
        assert_eq!(log_metric(Kind::Bodyweight, &bodyweight), 20.0);

        let duration = log(20, &[(0.0, 60), (0.0, 45)], None);
        assert_eq!(log_metric(Kind::Duration, &duration), 105.0);

        assert_eq!(log_metric(Kind::Weighted, &log(10, &[], None)), 0.0);
    }

    #[test]
    fn units_per_kind() {
        assert_eq!(unit_of(Kind::Weighted), "kg·回");
        assert_eq!(unit_of(Kind::Bodyweight), "回");
        assert_eq!(unit_of(Kind::Duration), "秒");
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

        let series = exercise_series(&db, 10, d(2026, 8, 1), d(2026, 8, 8));
        assert_eq!(
            series,
            vec![(d(2026, 8, 1), 500.0), (d(2026, 8, 8), 1080.0)]
        );

        // 未知の種目は空
        assert!(exercise_series(&db, 999, d(2026, 8, 1), d(2026, 8, 8)).is_empty());
        // from > to でもパニックしない
        assert!(exercise_series(&db, 10, d(2026, 8, 8), d(2026, 8, 1)).is_empty());
    }

    #[test]
    fn group_set_series_counts_sets_across_mixed_kinds() {
        let mut db = test_db();
        put(
            &mut db,
            d(2026, 8, 1),
            vec![
                log(10, &[(60.0, 10), (60.0, 8)], None),
                log(11, &[(0.0, 12)], None),
            ],
        );
        put(&mut db, d(2026, 8, 2), vec![log(20, &[(0.0, 60)], None)]);

        // 胸 = ベンチ 2 セット + プッシュアップ 1 セット
        assert_eq!(
            group_set_series(&db, 1, d(2026, 8, 1), d(2026, 8, 2)),
            vec![(d(2026, 8, 1), 3.0)]
        );
        // 体幹（Duration）も同じ「セット数」で数えられる
        assert_eq!(
            group_set_series(&db, 2, d(2026, 8, 1), d(2026, 8, 2)),
            vec![(d(2026, 8, 2), 1.0)]
        );
        // 種目が 1 つも無い部位は空
        assert!(group_set_series(&db, 3, d(2026, 8, 1), d(2026, 8, 2)).is_empty());
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
        assert_eq!(log_metric(Kind::Weighted, &session.logs[0]), 1080.0);
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
