//! 永続化されるデータモデル。
//!
//! ID は `u32` 連番で、**必ず [`Db::alloc_id`] 経由で採番する**（プリセット再投入も含む）。
//! `uuid` を使わないのは wasm32 の getrandom バックエンド設定を避けるため。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// `Db::schema` の現在値。
pub const SCHEMA: u32 = 1;

pub type GroupId = u32;
pub type ExerciseId = u32;

/// 部位（胸 / 背中 / …）。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Group {
    pub id: GroupId,
    pub name: String,
    pub color: String,
    pub order: u32,
}

/// 指標の種類。**種目作成時にユーザーが選ぶ**（データからの推論はしない）。
///
/// 推論が破綻する例: 自重ディップスを 12 週記録した後にベルトで +10kg 付けると
/// 「全て weight == 0」が偽になり、系列全体が volume 指標へ切り替わって過去 12 週が
/// 0 の直線に潰れる。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// 指標 = Σ(weight × reps)、単位 "kg·回"
    Weighted,
    /// 指標 = Σ(reps)、単位 "回"。`weight` は「追加重量」として表示のみ
    Bodyweight,
    /// `reps` を秒として扱う。指標 = Σ(reps)、単位 "秒"
    Duration,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Exercise {
    pub id: ExerciseId,
    pub name: String,
    pub group_id: GroupId,
    pub kind: Kind,
    pub order: u32,
    #[serde(default)]
    pub archived: bool,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct SetEntry {
    pub weight: f32,
    pub reps: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ExerciseLog {
    pub exercise_id: ExerciseId,
    pub sets: Vec<SetEntry>,
    /// ★ 当日入力時のみ `Some(epoch ms)`。過去日バックフィルは `None`。
    ///
    /// 日付キーと `at` が両方「いつトレーニングしたか」の真実源になると、過去日
    /// バックフィルで必ず矛盾する（`at = now` を書くと「最後のトレーニングから」が
    /// 「たった今」になり要件の出力が嘘になる）。
    #[serde(default)]
    pub at: Option<i64>,
}

/// 1 日分の記録。**1 日 = 1 セッション、1 日 1 種目 1 ログ。**
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Session {
    /// 不変条件: `exercise_id` は重複しない（[`crate::core::migrate`] が正規化する）
    pub logs: Vec<ExerciseLog>,
    #[serde(default)]
    pub body_weight: Option<f32>,
    #[serde(default)]
    pub note: String,
}

impl Session {
    /// 何も入っていない = 保存する価値がない（過去日を閲覧しただけの空セッション）。
    ///
    /// 体重やメモだけが入っているセッションは「空」ではない（破棄すると
    /// コンディションのみの記録が消える）。
    pub fn is_empty(&self) -> bool {
        self.logs.is_empty() && self.body_weight.is_none() && self.note.trim().is_empty()
    }

    /// カレンダーの「実施日」判定。セット付きのログが 1 つでもあるか。
    pub fn is_trained(&self) -> bool {
        self.logs.iter().any(|l| !l.sets.is_empty())
    }

    /// その種目の当日のログ。
    pub fn log_of(&self, ex: ExerciseId) -> Option<&ExerciseLog> {
        self.logs.iter().find(|l| l.exercise_id == ex)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Db {
    pub schema: u32,
    pub next_id: u32,
    pub groups: Vec<Group>,
    pub exercises: Vec<Exercise>,
    /// "YYYY-MM-DD" → 日付順が自動で保たれる（ゼロ埋め ISO なので辞書順 = 時系列順）
    pub sessions: BTreeMap<String, Session>,
}

impl Default for Db {
    fn default() -> Self {
        Self {
            schema: SCHEMA,
            next_id: 1,
            groups: Vec::new(),
            exercises: Vec::new(),
            sessions: BTreeMap::new(),
        }
    }
}

impl Db {
    /// 唯一の ID 採番口。プリセット投入もユーザー追加もここを通す。
    pub fn alloc_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    pub fn group(&self, id: GroupId) -> Option<&Group> {
        self.groups.iter().find(|g| g.id == id)
    }

    pub fn exercise(&self, id: ExerciseId) -> Option<&Exercise> {
        self.exercises.iter().find(|e| e.id == id)
    }

    /// アーカイブ済みも含む、その部位の全種目 ID。
    ///
    /// アーカイブ済みを外すと部位グラフ・ドット色・チップが過去分だけ欠ける。
    pub fn exercise_ids_of_group(&self, g: GroupId) -> Vec<ExerciseId> {
        self.exercises
            .iter()
            .filter(|e| e.group_id == g)
            .map(|e| e.id)
            .collect()
    }
}
