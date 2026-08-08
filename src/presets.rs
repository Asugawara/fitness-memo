//! 初期投入するプリセット（部位 6 種 + 種目 28 種）。
//!
//! **ID は全端末で同じ固定値**（[`crate::model::RESERVED_MAX`] 未満の予約領域）。
//! これがあるので、別々に初期化された 2 台のデータをマージしても「ベンチプレス」は
//! 1 本にまとまる。改名されていても ID で追跡できる。
//!
//! 割り当ては部位ごとに 16 個ずつのブロック: 部位 N が `0xN0`、その種目が `0xN1..`。
//! 上限は `0x64` なので予約領域（1024）に余裕で収まる。

use crate::model::{Db, Exercise, ExerciseId, Group, GroupId};

pub struct PresetGroup {
    pub id: GroupId,
    pub name: &'static str,
    pub color: &'static str,
    /// `(固定 ID, 名前)`。ID は**二度と変えてはいけない** — 変えると既存の
    /// ユーザーの端末で同じ種目が 2 つになる。
    pub exercises: &'static [(ExerciseId, &'static str)],
}

/// 部位 6 種。`order` は宣言順。
pub const PRESETS: &[PresetGroup] = &[
    PresetGroup {
        id: GroupId::from_bits(0x10),
        name: "胸",
        color: "#e0524a",
        exercises: &[
            (ExerciseId::from_bits(0x11), "ベンチプレス"),
            (ExerciseId::from_bits(0x12), "ダンベルプレス"),
            (ExerciseId::from_bits(0x13), "インクラインベンチプレス"),
            (ExerciseId::from_bits(0x14), "チェストフライ"),
            (ExerciseId::from_bits(0x15), "プッシュアップ"),
        ],
    },
    PresetGroup {
        id: GroupId::from_bits(0x20),
        name: "背中",
        color: "#2f7fd1",
        exercises: &[
            (ExerciseId::from_bits(0x21), "懸垂"),
            (ExerciseId::from_bits(0x22), "ラットプルダウン"),
            (ExerciseId::from_bits(0x23), "ベントオーバーロウ"),
            (ExerciseId::from_bits(0x24), "シーテッドロウ"),
            (ExerciseId::from_bits(0x25), "デッドリフト"),
        ],
    },
    PresetGroup {
        id: GroupId::from_bits(0x30),
        name: "肩",
        color: "#e0912a",
        exercises: &[
            (ExerciseId::from_bits(0x31), "ショルダープレス"),
            (ExerciseId::from_bits(0x32), "サイドレイズ"),
            (ExerciseId::from_bits(0x33), "フロントレイズ"),
            (ExerciseId::from_bits(0x34), "リアレイズ"),
        ],
    },
    PresetGroup {
        id: GroupId::from_bits(0x40),
        name: "腕",
        color: "#7a56c9",
        exercises: &[
            (ExerciseId::from_bits(0x41), "バーベルカール"),
            (ExerciseId::from_bits(0x42), "ダンベルカール"),
            (ExerciseId::from_bits(0x43), "トライセプスエクステンション"),
            (ExerciseId::from_bits(0x44), "ケーブルプレスダウン"),
            (ExerciseId::from_bits(0x45), "ディップス"),
        ],
    },
    PresetGroup {
        id: GroupId::from_bits(0x50),
        name: "脚",
        color: "#2fa06a",
        exercises: &[
            (ExerciseId::from_bits(0x51), "スクワット"),
            (ExerciseId::from_bits(0x52), "レッグプレス"),
            (ExerciseId::from_bits(0x53), "レッグエクステンション"),
            (ExerciseId::from_bits(0x54), "レッグカール"),
            (ExerciseId::from_bits(0x55), "カーフレイズ"),
        ],
    },
    PresetGroup {
        id: GroupId::from_bits(0x60),
        name: "体幹",
        color: "#6b7280",
        exercises: &[
            (ExerciseId::from_bits(0x61), "プランク"),
            (ExerciseId::from_bits(0x62), "サイドプランク"),
            (ExerciseId::from_bits(0x63), "クランチ"),
            (ExerciseId::from_bits(0x64), "レッグレイズ"),
        ],
    },
];

/// プリセットを流し込む。**既に同じ固定 ID があればスキップする**ので、初回投入にも
/// 再投入にもそのまま使える（何度呼んでも増殖しない）。
///
/// ★ **判定は名前ではなく固定 ID で行う。** 名前で見ていた頃は「改名済みプリセットが
/// 別種目として復活する」だけの軽微な挙動だったが、固定 ID の下では同じことをすると
/// **同一 ID の種目が 2 つできて不変条件が壊れる**。改名は正当な操作なので、
/// 名前が変わっていても ID が居れば触らない。
///
/// 採番は一切しない（プリセットの ID は定数）。
pub fn seed(db: &mut Db) {
    for preset in PRESETS {
        if db.group(preset.id).is_none() {
            let order = db.groups.len() as u32;
            db.groups.push(Group {
                id: preset.id,
                name: preset.name.to_string(),
                color: preset.color.to_string(),
                order,
            });
        }

        for (id, name) in preset.exercises {
            if db.exercise(*id).is_some() {
                continue;
            }
            let order = db
                .exercises
                .iter()
                .filter(|e| e.group_id == preset.id)
                .count() as u32;
            db.exercises.push(Exercise {
                id: *id,
                name: (*name).to_string(),
                group_id: preset.id,
                order,
                archived: false,
            });
        }
    }
}

/// その ID がプリセットのものか。移行時に「プリセット由来」を見分けるのに使う。
pub fn is_preset_exercise(id: ExerciseId) -> bool {
    PRESETS
        .iter()
        .flat_map(|p| p.exercises)
        .any(|(preset_id, _)| *preset_id == id)
}

/// プリセット名 → 固定 ID。移行で「名前が一致する種目を固定 ID に寄せる」ときに引く。
pub fn preset_exercise_id(name: &str) -> Option<ExerciseId> {
    PRESETS
        .iter()
        .flat_map(|p| p.exercises)
        .find(|(_, preset_name)| *preset_name == name)
        .map(|(id, _)| *id)
}

/// プリセット名 → 固定 ID（部位）。
pub fn preset_group_id(name: &str) -> Option<GroupId> {
    PRESETS.iter().find(|p| p.name == name).map(|p| p.id)
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

        // 固定 ID は部位と種目をまたいで一意（同じ ID 空間を共有している）
        let mut ids: Vec<u64> = db
            .groups
            .iter()
            .map(|g| g.id.bits())
            .chain(db.exercises.iter().map(|e| e.id.bits()))
            .collect();
        let total = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), total, "ID が重複している");
        assert!(
            db.groups.iter().all(|g| g.id.is_reserved()),
            "プリセットの部位は予約領域にいること"
        );
        assert!(
            db.exercises.iter().all(|e| e.id.is_reserved()),
            "プリセットの種目は予約領域にいること"
        );
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

        seed(&mut db);

        assert_eq!(db, before, "2 回目の seed は何も変えない");
    }

    /// ★ 固定 ID 化の要。名前でスキップ判定していた頃の規則をそのまま残すと、
    /// 改名済みプリセットに seed したとき**同じ ID の種目が 2 つできる**。
    #[test]
    fn seed_does_not_resurrect_a_renamed_preset() {
        let mut db = seeded_db();
        let bench = db
            .exercises
            .iter_mut()
            .find(|e| e.name == "ベンチプレス")
            .expect("プリセットに含まれる");
        let bench_id = bench.id;
        bench.name = "ベンチプレス（スミス）".to_string();
        let total = db.exercises.len();

        seed(&mut db);

        assert_eq!(db.exercises.len(), total, "改名しても復活させない");
        assert_eq!(
            db.exercises.iter().filter(|e| e.id == bench_id).count(),
            1,
            "同じ ID の種目が 2 つできてはいけない"
        );
        assert_eq!(
            db.exercise(bench_id).map(|e| e.name.as_str()),
            Some("ベンチプレス（スミス）"),
            "ユーザーが付けた名前を勝手に戻さない"
        );
    }

    /// 別々の端末で初期化しても、プリセットの ID は全部一致する。
    /// これが無いとマージが名前突合に落ちる（= 改名で履歴が 2 本に割れる）。
    #[test]
    fn independently_seeded_devices_agree_on_every_preset_id() {
        let a = seeded_db();
        let b = seeded_db();

        let ids = |db: &Db| -> Vec<u64> {
            let mut v: Vec<u64> = db
                .groups
                .iter()
                .map(|g| g.id.bits())
                .chain(db.exercises.iter().map(|e| e.id.bits()))
                .collect();
            v.sort_unstable();
            v
        };
        assert_eq!(ids(&a), ids(&b));
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
