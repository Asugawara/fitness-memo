//! 純ロジック。**`leptos` / `web-sys` を一切 import しない。**
//!
//! `cargo test`（ホストターゲット）で検証する層。UI から呼ぶ計算はすべてここに置き、
//! 画面側は結果を並べるだけにする。

use std::collections::{BTreeMap, HashMap};

use chrono::{Datelike, NaiveDate, TimeDelta};

use crate::model::{
    Db, Exercise, ExerciseId, ExerciseLog, Group, GroupId, IdGen, SCHEMA, Session, SetEntry,
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

    // ★ 既にログのある日には書かない。UI は「カードが 0 枚の日」にしか導線を出さないが、
    //   カードの再構築は Effect 経由なので「ログのある日 × 空のカード」の 1 tick が
    //   存在する。そこを踏むと exercise_id が重複して「1 日 1 種目 1 ログ」が壊れる。
    //
    //   判定は `is_trained()` ではなく `logs.is_empty()` にする。空セットのログは
    //   `migrate` が読み込みのたびに落とすので通常は存在しないが、判定を緩めると
    //   「同じ種目のログが 2 本ある」状態を作れてしまう側に倒れる
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
        //   日数表示は日付キーから出るので日付が嘘になることはもう無いが（ADR-0054）、
        //   「その日に実施した時刻」として存在しない値が残り、同じ暦日にコピーしたときの
        //   時刻粒度が捏造される。記録の正直さは表示の都合とは別に守る（ADR-0006）
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
///   ADR-0054 参照。
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
///   チップ（日粒度）とヒーロー（時刻粒度）が違う日を指すことがあった（ADR-0054）。
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
///   誰にも検出されないまま残っていた（ADR-0045 と同じ理由でロジックを core に置く）。
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
/// - 同一 `exercise_id` の重複ログをマージ（セットを連結、`at` は `Some` の最大値）
/// - セットが空のログを捨て、ログも体重もメモも無いセッションを捨てる
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

    normalize(&mut db);
    db.schema = SCHEMA;
    Ok(db)
}

/// 世代に依らない正規化。
fn normalize(db: &mut Db) {
    let mut sessions: BTreeMap<String, Session> = BTreeMap::new();
    for (key, session) in std::mem::take(&mut db.sessions) {
        let Some(date) = parse_date_key(&key) else {
            continue;
        };
        merge_same_day(sessions.entry(date_key(date)).or_default(), session);
    }
    for session in sessions.values_mut() {
        drop_unrepresentable_weights(session);
        dedupe_logs(session);
    }
    sessions.retain(|_, s| !s.is_empty());
    db.sessions = sessions;
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
/// ★ **これはインポートのマージに流用してはいけない。** メモを無条件に連結するので、
/// 同じファイルを 2 回取り込むとメモが 2 回増える（冪等でない）。マージ側は
/// `merge_db` が重複ガード付きで別に処理する。
fn merge_same_day(dst: &mut Session, src: Session) {
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
    pub conflicts: Vec<Conflict>,
}

impl MergeReport {
    /// 何も足さなかったか。
    pub fn is_noop(&self) -> bool {
        self.groups_added == 0
            && self.exercises_added == 0
            && self.sessions_added == 0
            && self.logs_added == 0
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
        //   2 本できる（ADR-0008「1 日 1 種目 1 ログ」違反）。そのまま入れると
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
            if existing.sets == log.sets {
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
                *existing = log;
            }
        }

        match (dst.body_weight, session.body_weight) {
            (None, Some(w)) => dst.body_weight = Some(w),
            (Some(a), Some(b)) if a != b => report
                .conflicts
                .push(Conflict::BodyWeight { date: date.clone() }),
            _ => {}
        }

        // ★ 重複ガード。無条件に連結すると同じファイルを 2 回入れてメモが 2 倍になる
        let incoming = session.note.trim();
        if !incoming.is_empty() && !dst.note.contains(incoming) {
            if dst.note.trim().is_empty() {
                dst.note = session.note;
            } else {
                dst.note.push('\n');
                dst.note.push_str(&session.note);
            }
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
        let (date, l) = last_log_before(&db, e(10), d(2026, 8, 8)).expect("8/4 がある");
        assert_eq!(date, d(2026, 8, 4));
        assert_eq!(
            l.sets,
            vec![SetEntry {
                weight: 55.0,
                reps: 10
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
    //   時刻の 24 時間後に起きていた（ADR-0054）。
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
                }],
                at: Some(1_800_000_000_000),
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
                    }],
                    at: Some(1_800_000_000_000),
                }],
                body_weight: Some(70.5),
                note: "調子よい".into(),
            },
        );

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
                        },
                        SetEntry {
                            weight: 60.0,
                            reps: 8,
                        },
                    ],
                    at: None,
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
                        }],
                        at: None,
                    },
                    ExerciseLog {
                        exercise_id: ExerciseId::from_bits(0xAAA1),
                        sets: vec![SetEntry {
                            weight: 20.0,
                            reps: 15,
                        }],
                        at: None,
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
                        }],
                        at: None,
                    },
                    ExerciseLog {
                        exercise_id: ExerciseId::from_bits(0xBBB1),
                        sets: vec![SetEntry {
                            weight: 45.0,
                            reps: 6,
                        }],
                        at: None,
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
                },
                SetEntry {
                    weight: 60.0,
                    reps: 8,
                },
            ]
        };
        let heavy = || {
            vec![
                SetEntry {
                    weight: 62.0,
                    reps: 10,
                },
                SetEntry {
                    weight: 60.0,
                    reps: 8,
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
                        }],
                        at: None,
                    },
                    ExerciseLog {
                        exercise_id: other,
                        sets: vec![SetEntry {
                            weight: 40.0,
                            reps: 12,
                        }],
                        at: None,
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
                    }],
                    at: None,
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
