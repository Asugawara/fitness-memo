//! 初期投入するプリセット（部位 6 種 + 種目 28 種）。
//!
//! **固定 ID は持たない。** 全ての ID は [`Db::alloc_id`] 経由で採番するので、
//! 「プリセットを追加」で再投入しても既存 ID と衝突しない。

use crate::model::{Db, Exercise, Group};

pub struct PresetGroup {
    pub name: &'static str,
    pub color: &'static str,
    pub exercises: &'static [&'static str],
}

/// 部位 6 種。`order` は宣言順。
pub const PRESETS: &[PresetGroup] = &[
    PresetGroup {
        name: "胸",
        color: "#e0524a",
        exercises: &[
            "ベンチプレス",
            "ダンベルプレス",
            "インクラインベンチプレス",
            "チェストフライ",
            "プッシュアップ",
        ],
    },
    PresetGroup {
        name: "背中",
        color: "#2f7fd1",
        exercises: &[
            "懸垂",
            "ラットプルダウン",
            "ベントオーバーロウ",
            "シーテッドロウ",
            "デッドリフト",
        ],
    },
    PresetGroup {
        name: "肩",
        color: "#e0912a",
        exercises: &[
            "ショルダープレス",
            "サイドレイズ",
            "フロントレイズ",
            "リアレイズ",
        ],
    },
    PresetGroup {
        name: "腕",
        color: "#7a56c9",
        exercises: &[
            "バーベルカール",
            "ダンベルカール",
            "トライセプスエクステンション",
            "ケーブルプレスダウン",
            "ディップス",
        ],
    },
    PresetGroup {
        name: "脚",
        color: "#2fa06a",
        exercises: &[
            "スクワット",
            "レッグプレス",
            "レッグエクステンション",
            "レッグカール",
            "カーフレイズ",
        ],
    },
    PresetGroup {
        name: "体幹",
        color: "#6b7280",
        exercises: &["プランク", "サイドプランク", "クランチ", "レッグレイズ"],
    },
];

/// プリセットを流し込む。**同名が既にあればスキップする**ので、初回投入にも
/// 種目タブの「プリセットを追加」にもそのまま使える（何度呼んでも増殖しない）。
///
/// - 部位は名前一致でスキップし、既存部位に不足している種目だけを足す
/// - 種目はアーカイブ済みも含めて**全体で**名前一致を見る（部位を移した種目が
///   元の部位に複製されるのを防ぐ）
/// - 改名済みプリセットが別種目として復活する限界は既知の挙動として受け入れる
pub fn seed(db: &mut Db) {
    for preset in PRESETS {
        let group_id = match db.groups.iter().find(|g| g.name == preset.name) {
            Some(g) => g.id,
            None => {
                let id = db.alloc_id();
                let order = db.groups.len() as u32;
                db.groups.push(Group {
                    id,
                    name: preset.name.to_string(),
                    color: preset.color.to_string(),
                    order,
                });
                id
            }
        };

        for name in preset.exercises {
            if db.exercises.iter().any(|e| e.name == *name) {
                continue;
            }
            let id = db.alloc_id();
            let order = db
                .exercises
                .iter()
                .filter(|e| e.group_id == group_id)
                .count() as u32;
            db.exercises.push(Exercise {
                id,
                name: (*name).to_string(),
                group_id,
                order,
                archived: false,
            });
        }
    }
}

/// 初回起動 / 復元失敗時に渡す、プリセット入りの `Db`。
pub fn seeded_db() -> Db {
    let mut db = Db::default();
    seed(&mut db);
    db
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeded_db_has_six_groups_and_all_ids_are_unique() {
        let db = seeded_db();

        assert_eq!(db.groups.len(), 6);
        let names: Vec<&str> = db.groups.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, ["胸", "背中", "肩", "腕", "脚", "体幹"]);

        // 各部位 4〜5 種目
        for group in &db.groups {
            let n = db
                .exercises
                .iter()
                .filter(|e| e.group_id == group.id)
                .count();
            assert!(
                (4..=5).contains(&n),
                "{} の種目数が {} 件（4〜5 件であること）",
                group.name,
                n
            );
        }

        // ID は Db::alloc_id 経由の連番で、部位と種目をまたいで一意
        let mut ids: Vec<u32> = db
            .groups
            .iter()
            .map(|g| g.id)
            .chain(db.exercises.iter().map(|e| e.id))
            .collect();
        let total = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), total, "ID が重複している");
        assert!(ids.iter().all(|id| *id < db.next_id));
        assert!(!ids.contains(&0), "0 は未採番の番兵として空けておく");
    }

    #[test]
    fn preset_names_are_unique_across_groups() {
        // seed の同名スキップは**部位をまたいで全体で**名前を見るので、
        // プリセット定義側に同名があると片方が投入されない
        let db = seeded_db();
        let mut names: Vec<&str> = db.exercises.iter().map(|e| e.name.as_str()).collect();
        let total = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), total, "プリセットに同名の種目がある");
    }

    #[test]
    fn seed_is_idempotent() {
        let mut db = seeded_db();
        let before = db.clone();
        let next_id_before = db.next_id;

        seed(&mut db);

        assert_eq!(db.groups.len(), before.groups.len());
        assert_eq!(db.exercises.len(), before.exercises.len());
        assert_eq!(db.next_id, next_id_before, "スキップ時は採番しない");
    }

    #[test]
    fn seed_refills_only_the_missing_exercises() {
        let mut db = seeded_db();
        let removed = db
            .exercises
            .iter()
            .position(|e| e.name == "サイドレイズ")
            .expect("プリセットに含まれる");
        db.exercises.remove(removed);
        let total = db.exercises.len();

        seed(&mut db);

        assert_eq!(db.exercises.len(), total + 1);
        let refilled = db
            .exercises
            .iter()
            .find(|e| e.name == "サイドレイズ")
            .expect("再投入される");
        // 既存の「肩」に入り、新しい部位は増えない
        assert_eq!(db.groups.len(), 6);
        assert_eq!(
            db.group(refilled.group_id).map(|g| g.name.as_str()),
            Some("肩")
        );
    }

    #[test]
    fn seed_does_not_duplicate_an_exercise_moved_to_another_group() {
        let mut db = seeded_db();
        let shoulder = db
            .groups
            .iter()
            .find(|g| g.name == "肩")
            .expect("プリセットに含まれる")
            .id;
        let bench = db
            .exercises
            .iter_mut()
            .find(|e| e.name == "ベンチプレス")
            .expect("プリセットに含まれる");
        bench.group_id = shoulder;
        let total = db.exercises.len();

        seed(&mut db);

        assert_eq!(
            db.exercises.len(),
            total,
            "部位をまたいだ同名の複製が起きない"
        );
    }
}
