//! 初期投入するプリセット（部位 6 種 + 種目 28 種）。
//!
//! **ID は全端末で同じ固定値**（[`crate::model::RESERVED_MAX`] 未満の予約領域）。
//! これがあるので、別々に初期化された 2 台のデータをマージしても「ベンチプレス」は
//! 1 本にまとまる。改名されていても ID で追跡できる。
//!
//! 割り当ては部位ごとに 16 個ずつのブロック: 部位 N が `0xN0`、その種目が `0xN1..`。
//! 上限は `0x64` なので予約領域（1024）に余裕で収まる。

use crate::i18n::Lang;
use crate::model::{Db, Exercise, ExerciseId, Group, GroupId};

/// 部位名 / 種目名の日英。**初回投入でどちらか一方だけが `Db` に入る。**
///
/// ★ これは文言ではなく**データ**なので `i18n.rs` ではなくここに置く。投入されたら
///   ユーザーが自由に改名できるただの名前になり、言語を切り替えても書き換わらない
///   （adr/storage/preset-names-are-user-data-seeded-once.md）。
#[derive(Clone, Copy)]
pub struct Names {
    pub ja: &'static str,
    pub en: &'static str,
}

impl Names {
    pub const fn get(self, lang: Lang) -> &'static str {
        match lang {
            Lang::Ja => self.ja,
            Lang::En => self.en,
        }
    }

    /// どちらかの言語の綴りと一致するか。**両方見るのが要点** — 片方しか見ないと、
    /// 英語で初期化した端末が書き出した TSV を日本語の端末で取り込んだときに
    /// 固定 ID へ寄らず、同じ種目が 2 本に割れる。
    pub fn matches(self, name: &str) -> bool {
        self.ja == name || self.en == name
    }
}

const fn names(ja: &'static str, en: &'static str) -> Names {
    Names { ja, en }
}

pub struct PresetGroup {
    pub id: GroupId,
    pub name: Names,
    pub color: &'static str,
    /// `(固定 ID, 名前)`。ID は**二度と変えてはいけない** — 変えると既存の
    /// ユーザーの端末で同じ種目が 2 つになる。
    ///
    /// ★ **名前は言語で変わるが ID は変わらない。** だから日本語で初期化した端末と
    ///   英語で初期化した端末のデータを混ぜても「ベンチプレス」と "Bench Press" は
    ///   1 本にまとまる（adr/data-model/random-ids-for-safe-merge.md の狙いそのもの）。
    pub exercises: &'static [(ExerciseId, Names)],
}

/// 部位 6 種。`order` は宣言順。
pub const PRESETS: &[PresetGroup] = &[
    PresetGroup {
        id: GroupId::from_bits(0x10),
        name: names("胸", "Chest"),
        color: "#e0524a",
        exercises: &[
            (
                ExerciseId::from_bits(0x11),
                names("ベンチプレス", "Bench Press"),
            ),
            (
                ExerciseId::from_bits(0x12),
                names("ダンベルプレス", "Dumbbell Press"),
            ),
            (
                ExerciseId::from_bits(0x13),
                names("インクラインベンチプレス", "Incline Bench Press"),
            ),
            (
                ExerciseId::from_bits(0x14),
                names("チェストフライ", "Chest Fly"),
            ),
            (
                ExerciseId::from_bits(0x15),
                names("プッシュアップ", "Push-Up"),
            ),
        ],
    },
    PresetGroup {
        id: GroupId::from_bits(0x20),
        name: names("背中", "Back"),
        color: "#2f7fd1",
        exercises: &[
            (ExerciseId::from_bits(0x21), names("懸垂", "Pull-Up")),
            (
                ExerciseId::from_bits(0x22),
                names("ラットプルダウン", "Lat Pulldown"),
            ),
            (
                ExerciseId::from_bits(0x23),
                names("ベントオーバーロウ", "Bent-Over Row"),
            ),
            (
                ExerciseId::from_bits(0x24),
                names("シーテッドロウ", "Seated Row"),
            ),
            (
                ExerciseId::from_bits(0x25),
                names("デッドリフト", "Deadlift"),
            ),
        ],
    },
    PresetGroup {
        id: GroupId::from_bits(0x30),
        name: names("肩", "Shoulders"),
        color: "#e0912a",
        exercises: &[
            (
                ExerciseId::from_bits(0x31),
                names("ショルダープレス", "Shoulder Press"),
            ),
            (
                ExerciseId::from_bits(0x32),
                names("サイドレイズ", "Lateral Raise"),
            ),
            (
                ExerciseId::from_bits(0x33),
                names("フロントレイズ", "Front Raise"),
            ),
            (
                ExerciseId::from_bits(0x34),
                names("リアレイズ", "Rear Delt Raise"),
            ),
        ],
    },
    PresetGroup {
        id: GroupId::from_bits(0x40),
        name: names("腕", "Arms"),
        color: "#7a56c9",
        exercises: &[
            (
                ExerciseId::from_bits(0x41),
                names("バーベルカール", "Barbell Curl"),
            ),
            (
                ExerciseId::from_bits(0x42),
                names("ダンベルカール", "Dumbbell Curl"),
            ),
            (
                ExerciseId::from_bits(0x43),
                names("トライセプスエクステンション", "Triceps Extension"),
            ),
            (
                ExerciseId::from_bits(0x44),
                names("ケーブルプレスダウン", "Cable Pushdown"),
            ),
            (ExerciseId::from_bits(0x45), names("ディップス", "Dips")),
        ],
    },
    PresetGroup {
        id: GroupId::from_bits(0x50),
        name: names("脚", "Legs"),
        color: "#2fa06a",
        exercises: &[
            (ExerciseId::from_bits(0x51), names("スクワット", "Squat")),
            (
                ExerciseId::from_bits(0x52),
                names("レッグプレス", "Leg Press"),
            ),
            (
                ExerciseId::from_bits(0x53),
                names("レッグエクステンション", "Leg Extension"),
            ),
            (
                ExerciseId::from_bits(0x54),
                names("レッグカール", "Leg Curl"),
            ),
            (
                ExerciseId::from_bits(0x55),
                names("カーフレイズ", "Calf Raise"),
            ),
        ],
    },
    PresetGroup {
        id: GroupId::from_bits(0x60),
        name: names("体幹", "Core"),
        color: "#6b7280",
        exercises: &[
            (ExerciseId::from_bits(0x61), names("プランク", "Plank")),
            (
                ExerciseId::from_bits(0x62),
                names("サイドプランク", "Side Plank"),
            ),
            (ExerciseId::from_bits(0x63), names("クランチ", "Crunch")),
            (
                ExerciseId::from_bits(0x64),
                names("レッグレイズ", "Leg Raise"),
            ),
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
/// ★ `lang` は**初回投入で使う綴りを選ぶだけ**。以後この関数が既存の名前を書き換える
/// ことはない（判定が固定 ID なので、言語を変えて呼び直しても何も起きない）。
///
/// 採番は一切しない（プリセットの ID は定数）。
pub fn seed(db: &mut Db, lang: Lang) {
    for preset in PRESETS {
        if db.group(preset.id).is_none() {
            let order = db.groups.len() as u32;
            db.groups.push(Group {
                id: preset.id,
                name: preset.name.get(lang).to_string(),
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
                name: name.get(lang).to_string(),
                group_id: preset.id,
                order,
                archived: false,
                pins: Vec::new(),
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

/// 表示に使う種目名。**未改名のプリセットだけが言語に追従する。**
///
/// ★ 判定は「保存されている名前が、その固定 ID のプリセット名の**どちらかの言語の綴り**と
/// 一致するか」。一致すれば未改名なので今の言語の綴りを返し、違えば利用者が付けた名前
/// なのでそのまま返す（adr/ux/preset-names-follow-the-ui-language.md）。
///
/// ★ **両言語を見るのが要点。** 片方だけだと、日本語で初期化した端末で英語に切り替えて
/// 戻したときに「一度英語で表示された = 改名された」と誤判定しうる。保存名は一切
/// 書き換えないので、どちらの綴りで入っていても未改名と分かる。
///
/// ★ 自作の種目は固定 ID を持たないので、この関数は素通りして `stored` を返す。
///
/// ★ 利用者が偶然もう一方の言語のプリセット名そのものに改名した場合
/// （「ベンチプレス」→「Bench Press」）は未改名と見なされ、日本語に戻すと
/// 「ベンチプレス」に戻る。同じ種目を指す綴りなので害が無く、判定を単純に保つほうを採った。
pub fn exercise_name(id: ExerciseId, stored: &str, lang: Lang) -> &str {
    PRESETS
        .iter()
        .flat_map(|p| p.exercises)
        .find(|(preset_id, _)| *preset_id == id)
        .filter(|(_, names)| names.matches(stored))
        .map_or(stored, |(_, names)| names.get(lang))
}

/// 表示に使う部位名。規則は [`exercise_name`] と同じ。
pub fn group_name(id: GroupId, stored: &str, lang: Lang) -> &str {
    PRESETS
        .iter()
        .find(|p| p.id == id)
        .filter(|p| p.name.matches(stored))
        .map_or(stored, |p| p.name.get(lang))
}

/// プリセット名 → 固定 ID。移行で「名前が一致する種目を固定 ID に寄せる」ときに引く。
///
/// ★ **日英どちらの綴りでも引ける。** 英語で初期化した端末が書き出した TSV には
/// "Bench Press" が入る。片方しか見ないと、日本語の端末で取り込んだときに固定 ID へ
/// 寄らず**同じ種目が 2 本に割れる**。
pub fn preset_exercise_id(name: &str) -> Option<ExerciseId> {
    PRESETS
        .iter()
        .flat_map(|p| p.exercises)
        .find(|(_, preset_name)| preset_name.matches(name))
        .map(|(id, _)| *id)
}

/// プリセット名 → 固定 ID（部位）。[`preset_exercise_id`] と同じく日英どちらでも引ける。
pub fn preset_group_id(name: &str) -> Option<GroupId> {
    PRESETS.iter().find(|p| p.name.matches(name)).map(|p| p.id)
}

/// 新しい部位に振る色の候補。**プリセット 6 部位の色そのもの。**
///
/// ★ `views/menu.rs` から移した。TSV の取り込みが作る新規部位にも同じ色を振るので、
/// wasm32 専用のモジュールに置いておくと `core` から引けない。名前は「部位を作る」
/// 全経路（画面 / 取り込み）で 1 本にしておく。
pub const COLOR_CHOICES: [&str; 6] = [
    "#e0524a", "#2f7fd1", "#e0912a", "#7a56c9", "#2fa06a", "#6b7280",
];

/// 初回起動 / 復元失敗時に渡す、プリセット入りの `Db`。
pub fn seeded_db(lang: Lang) -> Db {
    let mut db = Db::default();
    seed(&mut db, lang);
    db
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeded_db_has_six_groups_and_all_ids_are_unique() {
        let db = seeded_db(Lang::Ja);

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
        let db = seeded_db(Lang::Ja);
        let mut names: Vec<&str> = db.exercises.iter().map(|e| e.name.as_str()).collect();
        let total = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), total, "プリセットに同名の種目がある");
    }

    #[test]
    fn seed_is_idempotent() {
        let mut db = seeded_db(Lang::Ja);
        let before = db.clone();

        seed(&mut db, Lang::Ja);

        assert_eq!(db, before, "2 回目の seed は何も変えない");
    }

    /// ★ 固定 ID 化の要。名前でスキップ判定していた頃の規則をそのまま残すと、
    /// 改名済みプリセットに seed したとき**同じ ID の種目が 2 つできる**。
    #[test]
    fn seed_does_not_resurrect_a_renamed_preset() {
        let mut db = seeded_db(Lang::Ja);
        let bench = db
            .exercises
            .iter_mut()
            .find(|e| e.name == "ベンチプレス")
            .expect("プリセットに含まれる");
        let bench_id = bench.id;
        bench.name = "ベンチプレス（スミス）".to_string();
        let total = db.exercises.len();

        seed(&mut db, Lang::Ja);

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
        let a = seeded_db(Lang::Ja);
        let b = seeded_db(Lang::Ja);

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
        let mut db = seeded_db(Lang::Ja);
        let removed = db
            .exercises
            .iter()
            .position(|e| e.name == "サイドレイズ")
            .expect("プリセットに含まれる");
        db.exercises.remove(removed);
        let total = db.exercises.len();

        seed(&mut db, Lang::Ja);

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
        let mut db = seeded_db(Lang::Ja);
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

        seed(&mut db, Lang::Ja);

        assert_eq!(
            db.exercises.len(),
            total,
            "部位をまたいだ同名の複製が起きない"
        );
    }

    /// 34 個 × 2 言語 = 68 個の名前が全部一意。**`preset_*_id` が曖昧にならない保証。**
    ///
    /// ★ 片方の言語で衝突していなくても、日英を跨いで同じ綴りがあれば
    /// `Names::matches` が 2 つのプリセットに当たる。両方まとめて見る必要がある。
    #[test]
    fn preset_names_are_unique_across_both_languages() {
        let mut all: Vec<&str> = Vec::new();
        for p in PRESETS {
            all.push(p.name.ja);
            all.push(p.name.en);
            for (_, n) in p.exercises {
                all.push(n.ja);
                all.push(n.en);
            }
        }
        assert_eq!(all.len(), (6 + 28) * 2);

        let mut sorted = all.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            all.len(),
            "プリセット名が日英を跨いで重複している"
        );
    }

    /// 言語を変えても **ID 集合は完全に同じ**。ここが崩れると、日本語で初期化した端末と
    /// 英語で初期化した端末のデータをマージしたときに種目が二重化する。
    #[test]
    fn preset_ids_are_identical_across_languages() {
        let ja = seeded_db(Lang::Ja);
        let en = seeded_db(Lang::En);

        let ids = |db: &Db| {
            let mut g: Vec<_> = db.groups.iter().map(|g| g.id).collect();
            let mut e: Vec<_> = db.exercises.iter().map(|e| e.id).collect();
            g.sort_unstable();
            e.sort_unstable();
            (g, e)
        };
        assert_eq!(ids(&ja), ids(&en));

        // 名前のほうは全部違う（同じ表を 2 回投入しただけ、ではないことの確認）
        let names = |db: &Db| db.groups.iter().map(|g| g.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(&ja), ["胸", "背中", "肩", "腕", "脚", "体幹"]);
        assert_eq!(
            names(&en),
            ["Chest", "Back", "Shoulders", "Arms", "Legs", "Core"]
        );
    }

    /// 英語の綴りでも日本語の綴りでも**同じ固定 ID**に寄る。
    #[test]
    fn either_language_spelling_resolves_to_the_same_fixed_id() {
        assert_eq!(
            preset_exercise_id("Bench Press"),
            preset_exercise_id("ベンチプレス")
        );
        assert_eq!(
            preset_exercise_id("ベンチプレス"),
            Some(ExerciseId::from_bits(0x11))
        );
        assert_eq!(preset_group_id("Chest"), preset_group_id("胸"));
        assert_eq!(preset_group_id("胸"), Some(GroupId::from_bits(0x10)));

        // 知らない名前はどちらの言語でも None
        assert_eq!(preset_exercise_id("Bench press"), None); // 大小は厳密一致
        assert_eq!(preset_exercise_id("マイ種目"), None);
    }

    /// 言語を変えて呼び直しても**何も起きない**（判定が固定 ID なので）。
    ///
    /// ★ ここが崩れると、言語を切り替えるたびに 28 種目が英語名で復活して
    /// 一覧が倍になる。
    #[test]
    fn seeding_again_in_another_language_changes_nothing() {
        let mut db = seeded_db(Lang::Ja);
        let before = db.exercises.len();

        seed(&mut db, Lang::En);

        assert_eq!(db.exercises.len(), before);
        assert_eq!(db.groups.len(), 6);
        // 名前は最初に入れた言語のまま
        assert_eq!(db.groups[0].name, "胸");
    }

    /// **未改名のプリセットは言語に追従する。** どちらの言語で初期化されていても、
    /// 両方の綴りから今の言語の綴りへ引ける。
    #[test]
    fn an_untouched_preset_follows_the_ui_language() {
        let bench = ExerciseId::from_bits(0x11);

        // 日本語で初期化された端末
        assert_eq!(
            exercise_name(bench, "ベンチプレス", Lang::Ja),
            "ベンチプレス"
        );
        assert_eq!(
            exercise_name(bench, "ベンチプレス", Lang::En),
            "Bench Press"
        );
        // 英語で初期化された端末
        assert_eq!(
            exercise_name(bench, "Bench Press", Lang::Ja),
            "ベンチプレス"
        );
        assert_eq!(exercise_name(bench, "Bench Press", Lang::En), "Bench Press");

        let chest = GroupId::from_bits(0x10);
        assert_eq!(group_name(chest, "胸", Lang::En), "Chest");
        assert_eq!(group_name(chest, "Chest", Lang::Ja), "胸");
    }

    /// ★ **改名したら二度と書き換えない。** 言語を切り替えても利用者が付けた名前が出る。
    #[test]
    fn a_renamed_preset_keeps_the_name_the_user_gave_it() {
        let bench = ExerciseId::from_bits(0x11);

        assert_eq!(exercise_name(bench, "マイベンチ", Lang::Ja), "マイベンチ");
        assert_eq!(exercise_name(bench, "マイベンチ", Lang::En), "マイベンチ");
        assert_eq!(exercise_name(bench, "My Bench", Lang::Ja), "My Bench");

        let chest = GroupId::from_bits(0x10);
        assert_eq!(group_name(chest, "胸の日", Lang::En), "胸の日");
    }

    /// 自作の種目・部位は固定 ID を持たないので素通りする。
    #[test]
    fn a_user_created_exercise_is_never_localized() {
        // 予約領域の外＝自作
        let mine = ExerciseId::from_bits(5000);
        assert!(!mine.is_reserved());
        assert_eq!(
            exercise_name(mine, "アームカール改", Lang::En),
            "アームカール改"
        );
        // **プリセットと同じ綴りでも、ID が違えば触らない**
        assert_eq!(
            exercise_name(mine, "ベンチプレス", Lang::En),
            "ベンチプレス"
        );

        let mine_g = GroupId::from_bits(5001);
        assert_eq!(group_name(mine_g, "胸", Lang::En), "胸");
    }

    /// ★ **別のプリセットの綴りに改名しても、その種目のものとしては扱わない。**
    /// 「ダンベルプレス」を「ベンチプレス」に改名した DB で、英語にしたときに
    /// 2 つとも "Bench Press" になってはいけない。
    #[test]
    fn renaming_one_preset_to_another_presets_name_does_not_localize_it() {
        let dumbbell = ExerciseId::from_bits(0x12);
        assert_eq!(
            exercise_name(dumbbell, "ベンチプレス", Lang::En),
            "ベンチプレス"
        );
        assert_eq!(
            exercise_name(dumbbell, "ダンベルプレス", Lang::En),
            "Dumbbell Press"
        );
    }

    /// 全プリセットが、どちらの言語の綴りからでも往復できる。
    #[test]
    fn every_preset_round_trips_between_languages() {
        for p in PRESETS {
            assert_eq!(group_name(p.id, p.name.ja, Lang::En), p.name.en);
            assert_eq!(group_name(p.id, p.name.en, Lang::Ja), p.name.ja);
            for (id, n) in p.exercises {
                assert_eq!(exercise_name(*id, n.ja, Lang::En), n.en);
                assert_eq!(exercise_name(*id, n.en, Lang::Ja), n.ja);
            }
        }
    }
}
