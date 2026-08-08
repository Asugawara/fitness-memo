//! 永続化されるデータモデル。
//!
//! ID は `u32` 連番で、**必ず [`Db::alloc_id`] 経由で採番する**（プリセット再投入も含む）。
//! `uuid` を使わないのは wasm32 の getrandom バックエンド設定を避けるため。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// `Db::schema` の現在値。
///
/// 2 で `Exercise.kind`（加重 / 自重 / 時間）を廃止した。指標は「重量 × 回数、
/// 重量が空なら重量 1」の単一式になり、どの軸で見るかは画面側の設定
/// （[`crate::core::Metric`]）が持つ。
///
/// ★ フィールドを消す変更は前方互換を壊す（旧版の serde が `missing field` で
/// 拒否する）。schema を上げるときは `storage::KEY` も必ず切ること。
pub const SCHEMA: u32 = 2;

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

/// 種目。
///
/// ★ 指標の種類（旧 `Kind`: 加重 / 自重 / 時間）は**持たない**。種目名を見れば
/// 懸垂が自重でプランクが時間だと分かるので、ユーザーに選ばせる意味が無かった。
/// 指標は [`crate::core::set_volume`] の単一式に統一され、どの軸で見るかは
/// [`crate::core::Metric`]（画面の表示設定）が決める。
///
/// schema 1 の JSON に残っている `"kind"` は serde が未知フィールドとして無視する
/// （`deny_unknown_fields` を付けていないのはこのため）。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Exercise {
    pub id: ExerciseId,
    pub name: String,
    pub group_id: GroupId,
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
