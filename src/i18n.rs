//! 画面に出る文言の日英 2 か国語分。
//!
//! **ターゲット非依存**（`leptos` も `web_sys` も import しない）。`core` と `presets` が
//! 引くので wasm32 に閉じられないし、閉じないおかげで `cargo test` がホストで
//! 「TSV の見出しが日英で衝突しない」「プリセット名が両言語で一意」といった
//! 不変条件を検証できる（adr/architecture/i18n-hand-rolled-string-table.md）。
//!
//! # 表の持ち方
//!
//! [`S`] の各フィールドはソースファイルと 1:1 のサブ struct で、
//! `struct 定義 → const JA_X → const EN_X` を隣接させてある。**構造体リテラルなので
//! フィールドを 1 つでも書き忘れるとコンパイルが通らない** — 264 個の文言を人力で
//! 突き合わせずに済むのがこの形を選んだ理由。
//!
//! # 規約
//!
//! ★ **表示用の日本語リテラルはこのファイルの外に置かない。** 例外は開発者向けの
//!   `expect` / `debug_assert!` のメッセージ（利用者に出ないので翻訳しない）と、
//!   `presets.rs` の [`crate::presets::Names`]（あちらは文言ではなくデータ）。
//!
//!   検証: `rg '"[^"]*[ぁ-んァ-ヶ一-龠]' src/ -g '!src/i18n.rs'` の残りが
//!   `//` / `///` / `expect(` / `debug_assert!` / `Names` だけになること。
//!
//! ★ **引数が要る文言は表に置かず [`Lang`] のメソッドにする。** `format!` は
//!   フォーマット文字列がリテラルでなければならず、表から引いた `&'static str` は
//!   渡せない。メソッドなら `match self` の腕落としがコンパイルエラーになるので、
//!   表の struct リテラルと同じ強度が保てる。

/// UI の言語。**2 つだけ。**
///
/// ★ 3 つ目を足すときは、この enum に腕を足せばコンパイラが未対応箇所を全部挙げる
///   （[`S`] の `const` が 1 枚足りない、`impl Lang` の `match` の腕が足りない、…）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Ja,
    En,
}

impl Lang {
    /// 言語切り替えの選択肢。`(言語, その言語自身での綴り)`。
    ///
    /// ★ 表記は必ず **endonym**（その言語自身での呼び名）。「Japanese」と英語で書くと、
    ///   英語を読めない人が自分の言語を選べない。
    pub const CHOICES: [(Lang, &'static str); 2] = [(Lang::Ja, "日本語"), (Lang::En, "English")];

    /// BCP-47 の言語タグ。`<html lang>` と `localStorage` の保存値に使う。
    pub const fn tag(self) -> &'static str {
        match self {
            Lang::Ja => "ja",
            Lang::En => "en",
        }
    }

    /// その言語自身での呼び名。設定画面の行に出す現在値。
    pub const fn endonym(self) -> &'static str {
        match self {
            Lang::Ja => "日本語",
            Lang::En => "English",
        }
    }

    /// この言語の文言表。
    pub const fn strings(self) -> &'static S {
        match self {
            Lang::Ja => &JA,
            Lang::En => &EN,
        }
    }
}

/// BCP-47 のタグ → 言語。**primary subtag だけを見る。**
///
/// ★ `starts_with("ja")` にしてはいけない — `jam`（ジャマイカ・クレオール）が日本語として
///   通る。区切り（`-` / `_`）で切ってから比較する。
///
/// ★ 知らない言語は**英語に倒す**。ここはブラウザの申告を解釈する入口で、
///   「日本語だと分かったときだけ日本語」が正しい既定
///   （adr/ux/language-follows-the-browser-then-the-setting.md）。
pub fn from_bcp47(tag: &str) -> Lang {
    let primary = tag.split(['-', '_']).next().unwrap_or("");
    if primary.eq_ignore_ascii_case("ja") {
        Lang::Ja
    } else {
        Lang::En
    }
}

/// `localStorage` の保存値 → 言語。**知らない綴りは `None`。**
///
/// ★ [`from_bcp47`] と分けるのが要点。あちらは「英語に倒す」が、こちらは
///   「未設定に倒す」必要がある — 保存値が壊れているときに英語で固定してしまうと、
///   ブラウザ言語に戻る道が塞がる。
pub fn parse_saved(s: &str) -> Option<Lang> {
    match s {
        "ja" => Some(Lang::Ja),
        "en" => Some(Lang::En),
        _ => None,
    }
}

/// 英語の単複を選ぶ。**辞書は持たない** — 呼び側が 2 語を書く。
///
/// ★ 不規則変化（`foot`/`feet`）まで面倒を見る仕組みは要らない。使うのは
///   `day`/`days` のような規則形だけで、呼び側が両方書けば規則も不規則も同じ扱いで済む。
pub const fn plural(n: usize, one: &'static str, many: &'static str) -> &'static str {
    if n == 1 { one } else { many }
}

// ── 表 ──────────────────────────────────────────────────────────────────────

/// 文言表。フィールドはソースファイルと 1:1。
pub struct S {
    /// `views/mod.rs` — タブ名・シートの閉じる・起動時の通知
    pub common: Common,
    /// カレンダーと日付の整形（`views/calendar.rs` / `views/mod.rs::fmt_date`）
    pub cal: Cal,
    /// `storage.rs` — 起動時に一度だけ出す通知
    pub boot: Boot,
    /// `views/settings.rs` — 設定タブ
    pub settings: Settings,
    /// `core.rs` — 指標のラベルと単位、経過時間の文言
    pub core: Core,
    /// `views/progress.rs` + `views/chart.rs` — 推移タブ
    pub progress: Progress,
    /// `views/routine.rs` — トレーニングメニューの編集
    pub routine: Routine,
    /// `views/day.rs` — 記録タブの入力欄
    pub day: Day,
    /// `views/help.rs` — ホーム画面への追加の案内
    pub help: Help,
    /// `views/manual.rs` — 設定タブの使い方マニュアル
    pub manual: Manual,
    /// `views/backup.rs` — エクスポート / インポート
    pub backup: Backup,
    /// `views/whatsnew.rs` — 新機能のお知らせバナーとシート。お知らせ本文自体は
    /// [`RELEASES`] にある（あちらは文言ではなくデータなので表に持たない）
    pub releases: Releases,
}

const JA: S = S {
    common: JA_COMMON,
    cal: JA_CAL,
    boot: JA_BOOT,
    settings: JA_SETTINGS,
    core: JA_CORE,
    progress: JA_PROGRESS,
    routine: JA_ROUTINE,
    day: JA_DAY,
    help: JA_HELP,
    manual: JA_MANUAL,
    backup: JA_BACKUP,
    releases: JA_RELEASES,
};

const EN: S = S {
    common: EN_COMMON,
    cal: EN_CAL,
    boot: EN_BOOT,
    settings: EN_SETTINGS,
    core: EN_CORE,
    progress: EN_PROGRESS,
    routine: EN_ROUTINE,
    day: EN_DAY,
    help: EN_HELP,
    manual: EN_MANUAL,
    backup: EN_BACKUP,
    releases: EN_RELEASES,
};

// ── views/mod.rs ────────────────────────────────────────────────────────────

pub struct Common {
    pub tab_record: &'static str,
    pub tab_progress: &'static str,
    pub tab_settings: &'static str,
    /// シート右上の × の `aria-label`
    pub close: &'static str,
    /// 通知バーの × の `aria-label`
    pub close_notice: &'static str,
    /// 保存に失敗していることが分かったときに出す。**控えを取る導線まで書く** —
    /// 「保存できていません」だけでは、この後どうすれば記録が守れるのか分からない
    pub save_failed: &'static str,
}

const JA_COMMON: Common = Common {
    tab_record: "記録",
    tab_progress: "推移",
    tab_settings: "設定",
    close: "閉じる",
    close_notice: "通知を閉じる",
    save_failed: "記録を保存できていません。設定タブの「データの書き出し / 読み込み」から今すぐ控えを取ってください",
};

const EN_COMMON: Common = Common {
    tab_record: "Record",
    tab_progress: "Progress",
    tab_settings: "Settings",
    close: "Close",
    close_notice: "Dismiss",
    save_failed: "Your log is not being saved. Open Settings › Export / Import and back it up now.",
};

// ── カレンダーと日付 ────────────────────────────────────────────────────────

pub struct Cal {
    /// 曜日（短縮）。**日曜始まり** — `Weekday::num_days_from_sunday()` の 0..=6 と
    /// インデックスが一致する。
    ///
    /// ★ かつて `views/mod.rs::weekday_ja` と `views/calendar.rs::WEEKDAYS` に
    ///   同じ表が二重にあった。ここに一本化してある。
    pub weekdays: [&'static str; 7],
    /// 月名（短縮）。日付 1 個の整形に使う
    pub months_short: [&'static str; 12],
    /// 月名（フル）。カレンダーの見出しに使う
    pub months_long: [&'static str; 12],
    /// 前後の月へ動かすボタンの `aria-label`
    pub prev_month: &'static str,
    pub next_month: &'static str,
    /// 月フッタの 3 つ。**「実施」は日数、「合計」はボリューム、「セット」はセット数**
    pub stat_trained: &'static str,
    pub stat_volume: &'static str,
    pub stat_sets: &'static str,
}

const JA_CAL: Cal = Cal {
    weekdays: ["日", "月", "火", "水", "木", "金", "土"],
    // 日本語の日付は "8/8" 形式なので短縮月名は使わないが、両言語で同じ形の表を
    // 持たせておく（片方だけ欠けた配列を作らない）
    months_short: [
        "1月", "2月", "3月", "4月", "5月", "6月", "7月", "8月", "9月", "10月", "11月", "12月",
    ],
    months_long: [
        "1月", "2月", "3月", "4月", "5月", "6月", "7月", "8月", "9月", "10月", "11月", "12月",
    ],
    prev_month: "前の月",
    next_month: "次の月",
    stat_trained: "実施",
    stat_volume: "合計",
    stat_sets: "セット",
};

const EN_CAL: Cal = Cal {
    weekdays: ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"],
    months_short: [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ],
    months_long: [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ],
    prev_month: "Previous month",
    next_month: "Next month",
    stat_trained: "Days",
    stat_volume: "Volume",
    stat_sets: "Sets",
};

// ── storage.rs（起動時の通知） ──────────────────────────────────────────────

/// 起動時に一度だけ出す通知。**どれも「この後どうすればよいか」まで書く** —
/// 「復元できませんでした」だけでは、記録が消えたのか救えるのかが読み取れない。
pub struct Boot {
    /// `localStorage` 自体が使えない（プライベートブラウズ等）
    pub cannot_save: &'static str,
    /// 読めなかったデータの退避に失敗した。**この状態では保存しない**ので、そう伝える
    pub rescue_failed: &'static str,
    /// 旧世代から復元した（退避済みの壊れたデータがある）
    pub restored_from_backup: &'static str,
    /// どの世代も読めなかった
    pub restore_failed: &'static str,
}

const JA_BOOT: Boot = Boot {
    cannot_save: "この端末では記録を保存できません（プライベートブラウズ中かもしれません）",
    rescue_failed: "空き容量が足りず、読めなかったデータを保管できませんでした。以前のバックアップの内容を表示しています（この画面での変更はまだ保存されません。空き容量を空けてください）",
    restored_from_backup: "最新のデータを復元できなかったため、以前のバックアップから復元しました",
    restore_failed: "以前のデータを復元できませんでした（退避済み）",
};

const EN_BOOT: Boot = Boot {
    cannot_save: "This device cannot save your log (you may be browsing privately).",
    rescue_failed: "There was not enough space to set aside the data that could not be read, so an earlier backup is shown instead. Changes made here are not being saved yet — please free up some space.",
    restored_from_backup: "The most recent data could not be restored, so an earlier backup was loaded.",
    restore_failed: "Your earlier data could not be restored. It has been set aside.",
};

// ── views/settings.rs ───────────────────────────────────────────────────────

pub struct Settings {
    /// 設定トップの h1
    pub title: &'static str,
    /// 節ヘッダの「‹ 設定」ボタンの `aria-label`
    pub back: &'static str,
    pub row_backup: &'static str,
    pub row_routines: &'static str,
    pub row_exercises: &'static str,
    /// 過去の記録の件数の行 / そのサブページの h1。
    ///
    /// ★ **何の数かはラベルに書かない。** 右端に現在値（`1 回分`）が出ていて、
    ///   入れば注記が説明するので、行のラベルは他の 5 行と同じ短さで揃える
    pub row_history: &'static str,
    /// 件数サブページの注記。**ラベルが短いぶん、ここが説明を持つ。**
    /// 「表示数」だけでは何の数か読めないので、ここで種目カードの話だと言う
    pub history_note: &'static str,
    /// 推移タブにドロップセットを含めるかの行
    pub row_drop_sets: &'static str,
    pub drop_sets_note: &'static str,
    /// その 2 択のラベル。**行の右端の現在値にも同じものを使う**（言語行の endonym /
    /// 表示数行の `n_past_sessions` と同じ流儀）
    pub drop_sets_exclude: &'static str,
    pub drop_sets_include: &'static str,
    /// 落とし幅（%）の見出しと注記。段を足すときの重量をここから計算する
    pub drop_pct_label: &'static str,
    pub drop_pct_note: &'static str,
    /// 言語の行 / 言語サブページの h1
    pub row_language: &'static str,
    /// 言語サブページの注記。**種目名が変わらないことを先に言う** —
    /// 切り替えてから「英語にしたのに種目名が日本語のまま」と迷わせない
    pub language_note: &'static str,

    /// 編集シートの見出し
    pub edit_group: &'static str,
    pub add_group: &'static str,
    pub edit_exercise: &'static str,
    pub add_exercise: &'static str,
    pub edit_routine: &'static str,
    pub add_routine: &'static str,
    /// 節が空のときの案内。**何のためのものかを書く**（「0 件です」で終わらせない）
    pub routines_empty: &'static str,
    pub archive_note: &'static str,
    /// 追加ボタン
    pub add_routine_cta: &'static str,
    pub add_group_cta: &'static str,
    pub add_exercise_cta: &'static str,
    /// 名前が空のまま保存された部位・種目
    pub unnamed: &'static str,
    /// 部位が記録タブに出ない理由
    pub no_usable_exercise: &'static str,
    pub archived_header: &'static str,
    pub no_group: &'static str,
    pub restore: &'static str,
    /// 編集フォームの項目名
    pub field_name: &'static str,
    pub field_color: &'static str,
    pub field_group: &'static str,
    /// 部位の削除
    pub delete_group: &'static str,
    pub move_exercises_first: &'static str,
    pub delete_group_confirm: &'static str,
    pub delete_yes: &'static str,
    pub delete_no: &'static str,
    pub close: &'static str,
    /// 名前の重複
    pub duplicate_group: &'static str,
    pub duplicate_exercise: &'static str,
    pub add: &'static str,
    /// 種目のアーカイブ
    pub archive_exercise: &'static str,
    pub archive_explain: &'static str,
    /// 種目ごとのラベル。**定義（作成・改名・削除）はこのシートだけに置く**
    /// （選択は記録タブのチップ行で 1 タップ）。
    /// adr/ux/label-chips-switch-the-history-and-the-copy.md
    pub field_labels: &'static str,
    /// ラベル欄の注記。**何のためのものかを書く**（`routines_empty` と同じ規則）
    pub labels_note: &'static str,
    pub label_value: &'static str,
    pub label_add: &'static str,
    pub duplicate_label: &'static str,
    /// 削除の確認。**「同じ名前で作り直しても戻らない」を必ず含める** —
    /// 参照は ID なので、消すと過去の記録はそのラベルから永久に外れる（本物の罠）
    pub delete_label_confirm: &'static str,
}

const JA_SETTINGS: Settings = Settings {
    title: "設定",
    back: "設定へ戻る",
    row_backup: "エクスポート / インポート",
    row_routines: "トレーニングメニュー",
    row_exercises: "種目",
    row_history: "表示数",
    history_note: "種目カードに、その種目をやった直近の記録を何回分出すか。日付の新しい順に並びます",
    row_drop_sets: "ドロップセット",
    drop_sets_note: "推移タブのグラフ・統計・記録に、ドロップセットの段を入れるか。含めないと、落とした段を除いたメインセットだけで推移が出ます。記録タブの合計は設定にかかわらず全部を数えます",
    drop_sets_exclude: "含めない",
    drop_sets_include: "含める",
    drop_pct_label: "落とし幅",
    drop_pct_note: "段を 1 つ足すとき、メインセットの重量から何 % 落とすか。計算して入れるのは 1 段目だけで、2 段目からは前の段の重量をそのままコピーします。小数点以下 1 桁まで",
    row_language: "言語",
    language_note: "種目名と部位名は変わりません（自分で付けた名前として扱うため）。変えたいときは「種目」から 1 つずつ編集してください",
    edit_group: "部位を編集",
    add_group: "部位を追加",
    edit_exercise: "種目を編集",
    add_exercise: "種目を追加",
    edit_routine: "メニューを編集",
    add_routine: "メニューを追加",
    routines_empty: "よくやる種目の組み合わせに名前を付けておくと、記録タブで 1 タップで呼び出せます",
    archive_note: "アーカイブした種目は「種目を追加」に出なくなりますが、過去の記録は残り推移タブから参照できます",
    add_routine_cta: "＋ メニューを追加",
    add_group_cta: "＋ 部位を追加",
    add_exercise_cta: "＋ 種目を追加",
    unnamed: "（名前なし）",
    no_usable_exercise: "使える種目がないため記録タブに出ません",
    archived_header: "アーカイブ済み ",
    no_group: "(部位なし)",
    restore: "戻す",
    field_name: "名前",
    field_color: "色",
    field_group: "部位",
    delete_group: "この部位を削除",
    move_exercises_first: "先に種目を別の部位へ移してください",
    delete_group_confirm: "この部位を削除します",
    delete_yes: "削除する",
    delete_no: "やめる",
    close: "閉じる",
    duplicate_group: "同じ名前の部位があります",
    duplicate_exercise: "同じ名前の種目があります",
    add: "追加",
    archive_exercise: "この種目をアーカイブ",
    archive_explain: "アーカイブは記録を消しません。過去のログは残り、「種目を追加」に出なくなります",
    field_labels: "ラベル",
    labels_note: "同じ種目で狙いを変えるとき（高重量・高回数など）に付けます。記録タブでラベルを選ぶと、そのラベルの前回だけが出ます",
    label_value: "ラベルの名前",
    label_add: "ラベルを追加",
    duplicate_label: "同じ名前のラベルがあります",
    delete_label_confirm: "このラベルを削除します。過去の記録は消えませんが、同じ名前で作り直しても過去の記録には戻りません",
};

const EN_SETTINGS: Settings = Settings {
    title: "Settings",
    back: "Back to Settings",
    row_backup: "Export / Import",
    row_routines: "Routines",
    row_exercises: "Exercises",
    row_history: "Sessions shown",
    history_note: "How many of an exercise's most recent sessions its card shows, newest first.",
    row_drop_sets: "Drop sets",
    drop_sets_note: "Whether the drop-set stages count towards the Progress tab's chart, stats and records. Excluded, progress is based on your main sets alone. The Record tab's totals always count everything.",
    drop_sets_exclude: "Excluded",
    drop_sets_include: "Included",
    drop_pct_label: "Drop by",
    drop_pct_note: "When you add a stage, how much to take off the main set's weight. Only the first stage is worked out from it; from the second on, the previous stage's weight is copied. One decimal place.",
    row_language: "Language",
    language_note: "Exercise and muscle-group names do not change — they are treated as names you gave them. Edit them one by one under Exercises if you want them in another language.",
    edit_group: "Edit muscle group",
    add_group: "Add muscle group",
    edit_exercise: "Edit exercise",
    add_exercise: "Add exercise",
    edit_routine: "Edit routine",
    add_routine: "Add routine",
    routines_empty: "Name a set of exercises you often do together, and you can pull it up with one tap on the Record tab.",
    archive_note: "Archived exercises stop appearing under Add exercise, but their past records stay and can be seen on the Progress tab.",
    add_routine_cta: "+ Add routine",
    add_group_cta: "+ Add muscle group",
    add_exercise_cta: "+ Add exercise",
    unnamed: "(unnamed)",
    no_usable_exercise: "Not shown on the Record tab — it has no usable exercises",
    archived_header: "Archived ",
    no_group: "(no muscle group)",
    restore: "Restore",
    field_name: "Name",
    field_color: "Colour",
    field_group: "Muscle group",
    delete_group: "Delete this muscle group",
    move_exercises_first: "Move the exercises to another muscle group first.",
    delete_group_confirm: "Delete this muscle group?",
    delete_yes: "Delete",
    delete_no: "Cancel",
    close: "Close",
    duplicate_group: "A muscle group with that name already exists.",
    duplicate_exercise: "An exercise with that name already exists.",
    add: "Add",
    archive_exercise: "Archive this exercise",
    archive_explain: "Archiving deletes nothing. Past records stay, and the exercise stops appearing under Add exercise.",
    field_labels: "Labels",
    labels_note: "Use these when you vary your aim on the same exercise — heavy singles, high reps, and so on. Pick a label on the Record tab and you see only that label's last session.",
    label_value: "Label name",
    label_add: "Add a label",
    duplicate_label: "A label with that name already exists.",
    delete_label_confirm: "Delete this label? Your records stay, but making a label with the same name again will not bring them back to it.",
};

// ── core.rs ─────────────────────────────────────────────────────────────────

pub struct Core {
    pub metric_volume: &'static str,
    pub metric_sets: &'static str,
    pub metric_reps: &'static str,
    /// 指標に添える単位。**ボリュームは重量と回数の合成量なので単位を持たない**
    pub unit_sets: &'static str,
    pub unit_reps: &'static str,
    /// 経過表示（日粒度）
    pub today: &'static str,
    pub yesterday: &'static str,
    /// 経過表示（時刻粒度、1 分未満）
    pub just_now: &'static str,
    /// 取り込みが失敗したときの文言。**どれも「次に何をすればよいか」まで書く**
    pub err_empty: &'static str,
    pub err_not_json: &'static str,
    pub err_not_db: &'static str,
    pub err_no_header: &'static str,
    pub err_no_records: &'static str,
    pub err_unreadable: &'static str,
}

const JA_CORE: Core = Core {
    metric_volume: "ボリューム",
    metric_sets: "セット数",
    metric_reps: "回数",
    unit_sets: "セット",
    unit_reps: "回",
    today: "今日",
    yesterday: "昨日",
    just_now: "たった今",
    err_empty: "中身がありません",
    err_not_json: "データが途中で切れているようです（全文がコピーできているか確認してください）",
    err_not_db: "このアプリの記録ではないようです",
    err_no_header: "1 行目の見出しが読めません（書き出したファイルをそのまま選んでください）",
    err_no_records: "取り込める記録が入っていません",
    err_unreadable: "日付や回数の書き方が変わっていて読めませんでした（日付は 2026-08-01、回数は 10 の形で書いてください）",
};

const EN_CORE: Core = Core {
    metric_volume: "Volume",
    metric_sets: "Sets",
    metric_reps: "Reps",
    unit_sets: "sets",
    unit_reps: "reps",
    today: "Today",
    yesterday: "Yesterday",
    just_now: "Just now",
    err_empty: "There is nothing in it.",
    err_not_json: "The data looks cut off — check that you copied all of it.",
    err_not_db: "This does not look like a log from this app.",
    err_no_header: "The header row cannot be read. Please pick the file exactly as it was exported.",
    err_no_records: "There are no records to import.",
    err_unreadable: "The dates or reps are written in a way that cannot be read. Use 2026-08-01 for dates and 10 for reps.",
};

// ── views/progress.rs + views/chart.rs ──────────────────────────────────────

pub struct Progress {
    pub title: &'static str,
    /// 期間セレクタの最後の 1 つ。**他の 4 つ（1M/3M/6M/1Y）は両言語で同じ**なので
    /// 表に持たない（記号に近い短縮で、訳すと逆に読みにくい）
    pub period_all: &'static str,
    /// 記録がまだ 1 件も無い
    pub empty_all: &'static str,
    /// 指標 / 期間セグメントの `aria-label`
    pub pick_metric: &'static str,
    pub pick_period: &'static str,
    /// 対象セレクタ 2 つの `aria-label`。見えるラベルは置かないのでここだけが頼り
    pub pick_group: &'static str,
    pub pick_exercise: &'static str,
    /// 各セレクタの先頭（絞り込みなし）。
    ///
    /// ★ 両方を「すべて」の 1 語にしない。2 つの select が横に並ぶので、
    ///   同じ語だとどちらの「すべて」か取り違える
    pub all_groups: &'static str,
    pub all_exercises: &'static str,
    /// 部位も種目も選ばれていないときにグラフの代わりに出す案内
    pub pick_hint: &'static str,
    /// 種目セレクタの `<optgroup>` の見出し
    pub optgroup_exercises: &'static str,
    pub optgroup_archived: &'static str,
    /// 全期間だけ週単位に落ちることの断り。体重の線が出ているかで文が変わる
    pub weekly_note: &'static str,
    pub weekly_note_with_weight: &'static str,
    /// ドロップセットを集計から外していることの断り。**外したことを黙らない**
    /// （記録タブは常に全部数えるので、黙ると同じ日の合計が食い違う理由が出ない）
    pub drops_hidden_note: &'static str,
    /// ラベルのチップ行の `aria-label`。見えるラベルは置かないのでここだけが頼り
    /// （対象セレクタ 2 つと同じ作法）
    pub pick_label: &'static str,
    /// ラベルのチップ行の先頭（絞り込みなし）。
    ///
    /// ★ **`period_all` / `all_groups` / `all_exercises` と同語にしない。**
    ///   期間の「全期間」/"All" と 2 つのセレクタの「すべての〜」がすぐ隣に並ぶので、
    ///   同じ語だとどれの「すべて」か取り違える（`all_groups` の ★ と同じ理由）。
    ///
    /// ★ 記録タブのチップ行の「指定なし」/"Any" とも別語。あちらのチップは
    ///   **今日のログの宛先**も兼ねるので「絞らない」ではなく「付けない」で、
    ///   こちらは表示の絞り込みしか意味しない
    ///   （adr/ux/label-colour-on-the-progress-dots.md）
    pub all_labels: &'static str,
    /// この期間・この対象に記録が無い
    pub empty_period_exercise: &'static str,
    pub empty_period: &'static str,
    /// ラベルで絞った結果が 0 件。**`empty_period_exercise` と分ける** —
    /// 種目には記録があるのにチップで絞って消えた、が読み取れないと
    /// 「すべて」に戻せば見えることに気づけない
    pub empty_period_label: &'static str,
    /// サマリの 3 つ
    pub stat_delta: &'static str,
    pub stat_best: &'static str,
    pub stat_average: &'static str,
    /// 記録テーブルの見出し
    pub col_date: &'static str,
    pub col_content: &'static str,
    pub col_metric: &'static str,
    /// グラフに 1 点も無い
    pub chart_empty: &'static str,
}

const JA_PROGRESS: Progress = Progress {
    title: "推移",
    period_all: "全期間",
    empty_all: "まだ記録がありません。記録タブで種目を追加すると、ここに推移が出ます",
    pick_metric: "指標",
    pick_period: "期間",
    pick_group: "部位",
    pick_exercise: "種目",
    all_groups: "すべての部位",
    all_exercises: "すべての種目",
    pick_hint: "部位か種目を選んでください",
    optgroup_exercises: "種目",
    optgroup_archived: "アーカイブ済み",
    weekly_note: "全期間は週単位で集計しています",
    weekly_note_with_weight: "全期間は週単位で集計しています（体重は週平均）",
    drops_hidden_note: "ドロップセットの段は含めていません（設定で変えられます）",
    pick_label: "ラベル",
    all_labels: "すべて",
    empty_period_exercise: "この期間、この種目の記録はありません",
    empty_period: "この期間の記録はありません",
    empty_period_label: "この期間、このラベルの記録はありません",
    stat_delta: "前回比",
    stat_best: "期間内ベスト",
    stat_average: "期間内平均",
    col_date: "日付",
    col_content: "内容",
    col_metric: "指標",
    chart_empty: "記録がありません",
};

const EN_PROGRESS: Progress = Progress {
    title: "Progress",
    period_all: "All",
    empty_all: "Nothing recorded yet. Add an exercise on the Record tab and your progress will show up here.",
    pick_metric: "Metric",
    pick_period: "Period",
    pick_group: "Muscle group",
    pick_exercise: "Exercise",
    all_groups: "All groups",
    all_exercises: "All exercises",
    pick_hint: "Pick a muscle group or an exercise.",
    optgroup_exercises: "Exercises",
    optgroup_archived: "Archived",
    weekly_note: "Over all time, figures are grouped by week.",
    weekly_note_with_weight: "Over all time, figures are grouped by week (body weight is a weekly average).",
    drops_hidden_note: "Drop sets are not included. You can change this in Settings.",
    pick_label: "Label",
    all_labels: "All labels",
    empty_period_exercise: "No records for this exercise in this period.",
    empty_period: "No records in this period.",
    empty_period_label: "No records for this label in this period.",
    stat_delta: "vs. last",
    stat_best: "Best in period",
    stat_average: "Average in period",
    col_date: "Date",
    col_content: "Sets",
    col_metric: "Metric",
    chart_empty: "No records",
};

// ── views/routine.rs ────────────────────────────────────────────────────────

pub struct Routine {
    /// 記録タブの「この日をメニューにする」ボタンとそのシート見出し
    pub save_day_button: &'static str,
    pub save_day_title: &'static str,
    /// 入力の検証。**何を直せばよいかだけを書く**（責める文にしない）
    pub need_name: &'static str,
    pub need_exercise: &'static str,
    pub name_label: &'static str,
    /// メニューが参照している種目が消えている
    pub deleted_exercise: &'static str,
    /// 削除の確認
    pub delete: &'static str,
    pub delete_confirm: &'static str,
    /// **記録が消えないことを先に言う。** ここが一番不安な点
    pub delete_note: &'static str,
    pub delete_yes: &'static str,
    pub delete_no: &'static str,
    pub save: &'static str,
}

const JA_ROUTINE: Routine = Routine {
    save_day_button: "＋ この日をメニューにする",
    save_day_title: "この日をメニューにする",
    need_name: "メニュー名を入れてください",
    need_exercise: "種目を 1 つ以上選んでください",
    name_label: "メニュー名（必須）",
    deleted_exercise: "（削除された種目）",
    delete: "このメニューを削除",
    delete_confirm: "このメニューを削除します",
    delete_note: "記録は 1 件も消えません（メニューは種目の組み合わせを覚えているだけです）",
    delete_yes: "削除する",
    delete_no: "やめる",
    save: "保存",
};

const EN_ROUTINE: Routine = Routine {
    save_day_button: "+ Save this day as a routine",
    save_day_title: "Save this day as a routine",
    need_name: "Please enter a name for the routine.",
    need_exercise: "Please pick at least one exercise.",
    name_label: "Routine name (required)",
    deleted_exercise: "(deleted exercise)",
    delete: "Delete this routine",
    delete_confirm: "Delete this routine?",
    delete_note: "No records are deleted — a routine only remembers which exercises go together.",
    delete_yes: "Delete",
    delete_no: "Cancel",
    save: "Save",
};

// ── views/day.rs ────────────────────────────────────────────────────────────

pub struct Day {
    /// 名前を付けずに保存されたメニュー
    pub unnamed: &'static str,
    /// ヒーローの「最後から N」。**ラベルと値を分けて持つ**（値だけ読めるように）
    pub since_last: &'static str,
    pub back_to_today: &'static str,
    pub today_badge: &'static str,
    /// コピー元の見出し。メニューと日付の両方が出るときだけ文が変わる
    pub menu_heading: &'static str,
    pub from_recent: &'static str,
    pub from_last_menu: &'static str,
    pub add_exercise: &'static str,
    /// コンディション欄の開閉（体重とその日のメモ）
    pub condition_open: &'static str,
    pub condition_close: &'static str,
    pub body_weight: &'static str,
    pub note: &'static str,
    pub deleted_exercise: &'static str,
    /// 前回までの記録が 1 件も無い
    pub no_last_log: &'static str,
    pub copy_last: &'static str,
    /// 履歴ブロックの `aria-label`。**表記が日付になると
    /// 「これは過去の記録だ」という語が画面から消える**ので、ここで補う
    pub past_records: &'static str,
    /// セット行の入力欄
    pub weight: &'static str,
    pub reps: &'static str,
    pub delete_set: &'static str,
    /// 段を 1 つ足すボタンの `aria-label`（表示は lucide の
    /// `arrow-down-wide-narrow` のみ）/ 段の入力欄 / 段を消すボタン
    pub drop_add: &'static str,
    pub drop_weight: &'static str,
    pub drop_reps: &'static str,
    pub drop_delete: &'static str,
    /// 保存されない理由。**責めずに「あと何をすれば保存されるか」を書く**
    pub weight_missing: &'static str,
    pub reps_missing: &'static str,
    pub add_set: &'static str,
    /// マシンのピン。**ラベル 1 語で種目メモと読み分ける**
    /// （adr/ux/machine-pins-on-the-exercise.md 決定 3）
    pub pins: &'static str,
    pub pin_value: &'static str,
    pub pin_delete: &'static str,
    pub pin_add: &'static str,
    /// セット間のインターバル。ラベルと単位を分けて持つ（値だけ読めるように）
    /// adr/ux/interval-seconds-on-the-exercise.md
    pub interval: &'static str,
    pub interval_unit: &'static str,
    pub exercise_note: &'static str,
    pub note_open: &'static str,
    pub note_close: &'static str,
    /// 種目をその日から外す
    pub remove_from_day: &'static str,
    pub remove_confirm: &'static str,
    pub remove_yes: &'static str,
    pub remove_no: &'static str,
    /// ラベルのチップ行の `aria-label`。**定義がある種目にしか出ない**
    /// （adr/ux/label-chips-switch-the-history-and-the-copy.md）
    pub labels: &'static str,
    /// 先頭のチップ。**「ラベルなし」にしない** — これは `LabelFilter::Any`
    /// （絞らない）であって「ラベルが無いログだけ」ではない。半年ラベルなしで
    /// 記録してきた利用者が押したときに昨日の記録が出る側の意味にする
    pub label_any: &'static str,
}

const JA_DAY: Day = Day {
    unnamed: "（名前なし）",
    since_last: "最後から ",
    back_to_today: "今日へ戻る",
    today_badge: "今日",
    menu_heading: "トレーニングメニュー",
    from_recent: "最近の記録から",
    from_last_menu: "前回のメニューから始める",
    add_exercise: "種目を追加",
    condition_open: "＋ コンディション",
    condition_close: "－ コンディション",
    body_weight: "体重",
    note: "メモ",
    deleted_exercise: "(削除された種目)",
    no_last_log: "記録なし",
    copy_last: "前回をコピー",
    past_records: "前回までの記録",
    weight: "重量",
    reps: "回数",
    delete_set: "このセットを削除",
    drop_add: "ドロップを足す",
    drop_weight: "ドロップの重量",
    drop_reps: "ドロップの回数",
    drop_delete: "この段を削除",
    weight_missing: "重量未入力",
    reps_missing: "回数を入れると保存されます",
    add_set: "+ セット",
    pins: "ピン",
    pin_value: "ピンの番号",
    pin_delete: "このピンを削除",
    pin_add: "ピンを追加",
    interval: "インターバル",
    interval_unit: "秒",
    exercise_note: "この種目のメモ",
    note_open: "＋ メモ",
    note_close: "－ メモ",
    remove_from_day: "この日から外す",
    remove_confirm: "この日の記録が消えます",
    remove_yes: "外す",
    remove_no: "やめる",
    labels: "ラベル",
    label_any: "指定なし",
};

const EN_DAY: Day = Day {
    unnamed: "(unnamed)",
    since_last: "Last trained ",
    back_to_today: "Back to today",
    today_badge: "Today",
    menu_heading: "Routines",
    from_recent: "From recent records",
    from_last_menu: "Start from your last routine",
    add_exercise: "Add exercise",
    condition_open: "+ Condition",
    condition_close: "- Condition",
    body_weight: "Body weight",
    note: "Note",
    deleted_exercise: "(deleted exercise)",
    no_last_log: "No records",
    copy_last: "Copy last time",
    past_records: "Past records",
    weight: "Weight",
    reps: "Reps",
    delete_set: "Delete this set",
    drop_add: "Add a drop stage",
    drop_weight: "Drop weight",
    drop_reps: "Drop reps",
    drop_delete: "Delete this stage",
    weight_missing: "No weight yet",
    reps_missing: "Enter reps and this set is saved",
    add_set: "+ Set",
    pins: "Pins",
    pin_value: "Pin number",
    pin_delete: "Delete this pin",
    pin_add: "Add a pin",
    interval: "Interval",
    interval_unit: "s",
    exercise_note: "Note for this exercise",
    note_open: "+ Note",
    note_close: "- Note",
    remove_from_day: "Remove from this day",
    remove_confirm: "This day's record will be deleted.",
    remove_yes: "Remove",
    remove_no: "Cancel",
    labels: "Labels",
    label_any: "Any",
};

// ── views/help.rs ───────────────────────────────────────────────────────────

/// ホーム画面への追加の案内。
///
/// ★ **iOS では Safari のタブと standalone PWA で `localStorage` が共有されない。**
/// 先に追加しないと記録が引き継がれないので、この文言群は「損失を防ぐ案内」であって
/// 単なる使い方の説明ではない（adr/ux/install-guide-banner-and-sheet.md）。
pub struct Help {
    pub banner_title: &'static str,
    pub banner_body: &'static str,
    pub banner_cta: &'static str,
    pub banner_dismiss: &'static str,
    pub row_label: &'static str,
    pub sheet_title: &'static str,
    pub why_split: &'static str,
    pub why_invisible: &'static str,
    pub why_offline: &'static str,
    pub step1: &'static str,
    pub step1_safari: &'static str,
    pub step1_other: &'static str,
    pub step2: &'static str,
    pub step2_note: &'static str,
    pub step3: &'static str,
    pub ipad_note: &'static str,
    pub verify_title: &'static str,
    pub verify_body: &'static str,
    pub already_title: &'static str,
    pub already_lead: &'static str,
    pub already_where: &'static str,
    pub already_body: &'static str,
    pub already_order: &'static str,
}

const JA_HELP: Help = Help {
    banner_title: "記録を付ける前にホーム画面に追加してください",
    banner_body: "Safari のタブで付けた記録は引き継がれません",
    banner_cta: "追加のしかた ›",
    banner_dismiss: "この案内を今後表示しない",
    row_label: "ホーム画面への追加のしかた",
    sheet_title: "ホーム画面に追加",
    why_split: "iPhone では、Safari のタブとホーム画面のアプリで記録の保存場所が分かれています。",
    why_invisible: "Safari のタブで付けた記録は、ホーム画面に追加したあとでは見えません。まだ記録していないなら、先に追加してください。",
    why_offline: "追加すると、電波の届かないジムでも開けて、ホーム画面のアイコンから 1 タップで起動します。",
    step1: "1. 画面の下のまん中にある共有ボタンを押す",
    step1_safari: "Safari で開いてください。",
    step1_other: "他のブラウザだとこの手順は使えません。",
    step2: "2. 「ホーム画面に追加」を選ぶ",
    step2_note: "リストを下にスクロールすると出てきます。",
    step3: "3. 右上の「追加」を押す",
    ipad_note: "図は iPhone を縦向きで使っているときの画面です。iPad では共有ボタンは画面の上のほうにあります。",
    verify_title: "追加できたかの確かめ方",
    verify_body: "ホーム画面のアイコンから開くと、この注意書きが出なくなります。まだ出ているならブラウザのタブのままです。",
    already_title: "すでに Safari で記録してしまった場合",
    already_lead: "移せます。",
    already_where: "Safari のタブのまま",
    already_body: "設定タブを開いて「エクスポート」でファイルに保存し、ホーム画面のアプリ側の「インポート」で取り込んでください。",
    already_order: "ホーム画面に追加してから書き出そうとしても、そちらは空なので意味がありません。順番に注意してください。",
};

const EN_HELP: Help = Help {
    banner_title: "Add this to your home screen before you start logging",
    banner_body: "Records made in a Safari tab are not carried over.",
    banner_cta: "How to add it ›",
    banner_dismiss: "Do not show this again",
    row_label: "How to add it to your home screen",
    sheet_title: "Add to home screen",
    why_split: "On iPhone, a Safari tab and a home-screen app store your records in separate places.",
    why_invisible: "Records made in a Safari tab are not visible once you add the app to your home screen. If you have not logged anything yet, add it first.",
    why_offline: "Once added, it opens in a gym with no signal, and launches in one tap from the home-screen icon.",
    step1: "1. Tap the share button at the bottom centre of the screen",
    step1_safari: "Open this in Safari.",
    step1_other: "These steps do not work in other browsers.",
    step2: "2. Choose \"Add to Home Screen\"",
    step2_note: "Scroll down the list to find it.",
    step3: "3. Tap \"Add\" at the top right",
    ipad_note: "The figures show an iPhone held upright. On iPad the share button is near the top of the screen.",
    verify_title: "How to check it worked",
    verify_body: "Open it from the home-screen icon and this notice stops appearing. If it is still there, you are in a browser tab.",
    already_title: "If you already logged records in Safari",
    already_lead: "You can move them.",
    already_where: "While still in the Safari tab,",
    already_body: "open the Settings tab, save a file with Export, then load it with Import in the home-screen app.",
    already_order: "Exporting after you add it to the home screen achieves nothing — that side is empty. The order matters.",
};

// ── views/manual.rs ─────────────────────────────────────────────────────────

/// マニュアルの 1 章の文言。**章の骨格（順序・図の有無）は `manual.rs` の
/// [`crate::manual::Chapter`] / [`crate::manual::MANUAL_CHAPTERS`] が持つ。**
///
/// ★ **`fig` は言語別。** クリップは要素の外接矩形なので内容依存で、同じ章でも
///   ja / en で寸法が違いうる（`adr/architecture/manual-figures-as-served-screenshots.md`）。
pub struct ChapterText {
    pub title: &'static str,
    /// 段落 3〜5。**図が無くても本文だけで手順が完結すること**
    /// （図はオフラインでは出ないので、あくまで補助）。
    pub body: &'static [&'static str],
    /// 図の alt。**1 行に収まる短いラベル**（iOS Safari は折り返さないので
    ///   長いと描かれない）。このコミットでは全章空。
    pub fig_alt: &'static str,
    /// 図の宣言寸法。このコミットでは全章 `None`（撮影はコミット #4）。
    pub fig: Option<(u32, u32)>,
}

/// 使い方マニュアル全体の文言。
///
/// ★ **設定タブの節として持つ**（シートにしない）理由と、記録タブに出す
///   `hint_*` の排他条件・配置は `adr/ux/manual-as-a-settings-section-with-one-open-chapter.md`
///   に書く。
pub struct Manual {
    /// 設定タブの行ラベル。マニュアル節の `<h1>` にも使う。
    pub row_label: &'static str,
    /// 節の先頭に置く導入文。上から順に読む必要が無いことを断る。
    pub intro: &'static str,
    /// 圏外では図が出ないことを先に言う。「黙って欠ける」を作らない
    pub offline_note: &'static str,
    /// 図はライトテーマの画面であることを断る（ダークテーマ利用者への注記）
    pub light_note: &'static str,
    /// ホーム画面追加は `help.rs` の手順シートが説明済みなので、重複させず 1 行で誘導する
    pub see_install_help: &'static str,
    /// 記録タブに初回だけ出す手掛かり（`install_hint` と同じ 3 点セット）の本文
    pub hint_body: &'static str,
    pub hint_cta: &'static str,
    /// ✕ の `aria-label`（見た目は ✕ でも支援技術には言葉で届く）
    pub hint_dismiss: &'static str,
    /// [`crate::manual::MANUAL_CHAPTERS`] と同じ順・同じ長さ
    pub chapters: &'static [ChapterText],
}

const JA_MANUAL: Manual = Manual {
    row_label: "使い方",
    intro: "気づきにくい操作をまとめました。上から順に読む必要はありません。気になる章だけ開いてください。",
    offline_note: "圏外では図が表示されません。文章だけで手順が分かるようにしてあります。",
    light_note: "図はライトテーマの画面です。ダークテーマで使っていても配置は同じです。",
    see_install_help: "ホーム画面への追加は「ホーム画面への追加のしかた」を見てください。",
    hint_body: "このアプリには気づきにくい機能がいくつかあります。設定タブの「使い方」でまとめて確認できます。",
    hint_cta: "使い方を見る ›",
    hint_dismiss: "この案内を今後表示しない",
    chapters: &[
        ChapterText {
            title: "推移タブの絞り込み",
            body: &[
                "推移タブには「部位」と「種目」の2つのセレクタがあります。部位を選ぶと、その部位に属する種目だけが種目セレクタの候補になります。",
                "種目のほうを直接選ぶと、部位セレクタはその種目が属する部位に自動で切り替わります。逆に部位を「すべて」に戻すと、種目の選択も一緒に外れます — 「部位はすべて、種目はベンチプレス」のような食い違った組み合わせにはなりません。",
                "部位・種目のどちらも「すべて」のままだと、絞り込みが足りないためグラフは出ません。どちらか一方を選ぶとグラフが表示されます。最後に選んだ組み合わせは端末に残り、次にタブを開いたときも引き継がれます。",
            ],
            fig_alt: "",
            fig: None,
        },
        ChapterText {
            title: "グラフの読み取り欄",
            body: &[
                "グラフの下の読み取り欄は常に表示されていて、既定では一番新しい点の日付と数値を示します。グラフ上のどこかをタップすると、読み取り欄はタップした点へ動きます — タップは値を変えるのではなく、読む点を移動する操作です。",
                "指標が「ボリューム」のとき、重量を入力しなかったセットは重量1kgとして計算されます。自重種目では実質「総レップ数」に、時間で数える種目では「総秒数」になります。",
                "期間を「全期間」にすると、日ごとの点が週単位にまとめられます。指標（ボリューム・セット数・回数）は週の合計、体重だけは週の平均です。同じグラフに乗せる以上どちらも週単位で揃えていますが、まとめ方が違います。",
                "体重の第2軸は、指標のグラフが実際に表示されているときだけ重ねて出ます。指標側に記録が無い期間では、体重の記録があっても第2軸は出ません。",
            ],
            fig_alt: "",
            fig: None,
        },
        ChapterText {
            title: "前回をコピー",
            body: &[
                "「前回をコピー」ボタンは、その種目のその日のセットがまだ空のときだけ出ます。押すと直近1回分の記録が読み込まれます。表示件数の設定（1〜3件）を増やしても、コピーされるのは常に一番新しい1回分だけです。",
                "種目メモは今日の欄が空のときだけコピーで埋まります。すでに何か書いてあれば上書きしません。セットごとのメモは、今日その行にすでに書いた内容があればそちらを優先し、空の行にだけ前回のメモが入ります。",
                "体重やその日の体調メモはコピーの対象に含まれません。持ち込まれるのはセットの数値とメモだけです。",
            ],
            fig_alt: "",
            fig: None,
        },
        ChapterText {
            title: "「＋ メモ」で開く4つ",
            body: &[
                "種目カードの「＋ メモ」を押すと、ピン・インターバル（秒）・種目メモ・セット行ごとのメモ欄の4つが一度に開きます。閉じるときも「－ メモ」で4つまとめて畳まれます。",
                "ピンとインターバルは種目そのものに貼り付く設定で、その日限りではなく日をまたいで残ります。マシンの設定値を毎回打ち直さずに済むための項目です。",
                "閉じているあいだも、何か入力済みなら薄い字で読めます。ピン・インターバル・種目メモ・セットメモのいずれも、確認するためだけに毎回開き直す必要はありません。インターバルはあくまで参考の秒数の表示で、カウントダウンはありません。",
            ],
            fig_alt: "",
            fig: None,
        },
        ChapterText {
            title: "空の日から始める",
            body: &[
                "候補リストは、その日にまだ1枚もカードが無いときだけ出ます。1種目でもカードを追加すると候補は消え、未来の日には最初から出ません。",
                "「最近の記録から」の候補は、直近180日以内・最大4件まで遡って出ます。種目名まで表示されるので、同じ部位の日が複数あっても見分けられます。",
                "候補を選ぶとその日は「実施済み」として扱われ、カレンダーのドット・月末の集計・グラフに反映されます。「最近の記録から」はセット付きの記録をそのままコピーするので必ず実施済みになりますが、保存したメニューを展開する場合は少し違います — メニューの種目のうち履歴がまだ1つも無いものは空のカードだけが出て記録は入らず、全種目に履歴が無ければその日は実施済みになりません。",
                "選んだあとにやらなかった種目があれば、そのカードの「この日から外す」でその種目だけを取り消せます。",
            ],
            fig_alt: "",
            fig: None,
        },
        ChapterText {
            title: "部位の折りたたみ",
            body: &[
                "部位ごとに折りたためる一覧は3か所にありますが、同時に開いておける数がそれぞれ違います。設定タブの「種目」節と、記録タブの「種目を追加」シートは、どちらも一度に1つの部位しか開けません。",
                "一方、トレーニングメニューを編集するシートの「選択中」だけは複数の部位を同時に開けます。1本のメニューを組む間に胸と脚を行き来するような使い方を想定しているためです。",
                "メニューを作る入口は2つあります。記録タブの「＋ この日をメニューにする」は、その日に実際にセット付きの記録があるときだけ表示されます。空のカードを追加しただけでは出ません。",
            ],
            fig_alt: "",
            fig: None,
        },
        ChapterText {
            title: "並び替え",
            body: &[
                "並び替えは3か所で掴む場所と待ち時間が違います。種目カードの見出し行と、メニュー編集シートの行は、掴んでから250ミリ秒待つと並び替えが始まります。",
                "セット行だけは違い、左端のセット番号を掴んだ瞬間に動き始めます。待ち時間はありません。指を置いてすぐ動くので、番号のところだけをつまむようにしてください。",
                "見出し行やメニューの行が待つのは、指を置いた瞬間に動き出すと縦のフリックスクロールと区別できず、スクロールしたつもりで順番が変わってしまうためです。キーボードでの並び替えはAlt+↑/↓で行えます（矢印キーだけだと入力欄のカーソル移動と衝突します）。",
            ],
            fig_alt: "",
            fig: None,
        },
        ChapterText {
            title: "書き出しと読み込み",
            body: &[
                "設定タブの「エクスポート / インポート」でTSV形式のファイルを書き出せます。表計算ソフトでそのまま開けます。iOSでは共有シートから「ファイルに保存」を選んでください。",
                "読み込みは基本的に追加だけです。同じ日・同じ種目でセットの内容が食い違っている場合は、「セット数 → ボリューム → 各セットの中身」の順で比べて、多いほうが残ります。ファイル側が多ければ今ある記録が入れ替わり、確認画面に「入れ替わる記録があります」と出ます。逆に今ある記録のほうが多ければ何も起きず、「新しく取り込むものはありません」と出ます — この場合ファイルの内容は捨てられます。",
                "取り込んだ直後なら「元に戻す」で戻せます。誤操作を防ぐため、押すと1度目は確認の表示に変わり、もう一度押すと実際に戻ります。シートを閉じるとその「元に戻す」の機会も失われます。",
                "TSVには種目のID・色・並び順・アーカイブ状態は含まれません。記録された時刻の列は書き出されますが、読み込み時には使われません。",
            ],
            fig_alt: "",
            fig: None,
        },
    ],
};

const EN_MANUAL: Manual = Manual {
    row_label: "How to use",
    intro: "This collects the parts of the app that are easy to miss. You do not need to read it in order — open only the chapter you need.",
    offline_note: "Offline, the figures do not load. The text alone is enough to follow each step.",
    light_note: "The figures show the light theme. The layout is the same if you use the dark theme.",
    see_install_help: "See \"How to add it to your home screen\" for adding this app to your home screen.",
    hint_body: "This app has a few features that are easy to miss. See \"How to use\" in the Settings tab for a rundown.",
    hint_cta: "See how to use it ›",
    hint_dismiss: "Do not show this again",
    chapters: &[
        ChapterText {
            title: "Filtering on the Progress tab",
            body: &[
                "The Progress tab has two selectors: muscle group and exercise. Choosing a group narrows the exercise selector to only the exercises in that group.",
                "Choosing an exercise directly switches the group selector to that exercise's group automatically. Conversely, setting the group back to \"All\" also clears the exercise — you never end up with a mismatched pair like \"group: All, exercise: Bench Press.\"",
                "If both selectors are left on \"All,\" the chart does not appear because nothing has been narrowed down; picking either one shows it. The last combination you picked stays on the device and is still selected the next time you open the tab.",
            ],
            fig_alt: "",
            fig: None,
        },
        ChapterText {
            title: "Reading the chart",
            body: &[
                "The readout line under the chart is always visible. By default it shows the date and value of the most recent point. Tapping anywhere on the chart moves the readout to the point you tapped — tapping does not change a value, it only moves which point you are reading.",
                "When the metric is Volume, a set with no weight entered counts as 1kg. For bodyweight exercises this effectively becomes \"total reps\"; for exercises measured in time it becomes \"total seconds.\"",
                "Setting the period to \"All\" groups the daily points by week. The metric (volume, sets, or reps) becomes a weekly sum, while body weight becomes a weekly average. Both are aggregated by the same week so they can share one chart, but the way they are aggregated differs.",
                "The body-weight second axis only overlays when the metric chart actually has something to show. In a period with no metric records, the second axis does not appear even if body weight was logged.",
            ],
            fig_alt: "",
            fig: None,
        },
        ChapterText {
            title: "Copy last time",
            body: &[
                "The \"Copy last time\" button only appears while today's sets for that exercise are still empty. Tapping it loads the single most recent record. Even if you raise the history display count to 2 or 3 in settings, copying always brings in just the newest one.",
                "The exercise note is filled in only when today's note is still empty; it never overwrites something you already typed. For each set's note, whatever you already typed in that row today takes priority — the previous note only fills rows that are still empty.",
                "Body weight and the day's condition note are not part of what gets copied. Only the set values and their notes are carried over.",
            ],
            fig_alt: "",
            fig: None,
        },
        ChapterText {
            title: "The four things \"+ Note\" opens",
            body: &[
                "Tapping \"+ Note\" on an exercise card opens four things at once: pins, interval (in seconds), the exercise note, and a note field for each set row. \"- Note\" folds all four back up together.",
                "Pins and interval are attached to the exercise itself, not just to today — they carry over from day to day. They exist so you don't have to retype a machine's settings every time.",
                "Even while closed, anything already filled in is still visible in dim text. You never need to reopen pins, interval, the exercise note, or a set's note just to check them. The interval is only a reference number of seconds — there is no countdown.",
            ],
            fig_alt: "",
            fig: None,
        },
        ChapterText {
            title: "Starting from an empty day",
            body: &[
                "The candidate list only appears on a day that has no cards yet. As soon as one card is added, the candidates disappear, and they never appear on a future date to begin with.",
                "\"From recent records\" candidates look back up to 180 days and show at most 4 of them. Exercise names are shown too, so two days for the same muscle group can still be told apart.",
                "Picking a candidate marks that day as trained, which shows up on the calendar dot, the month's footer totals, and the chart. \"From recent records\" always carries actual sets, so it always counts as trained. Expanding a saved routine is a little different: any of its exercises with no history anywhere get only an empty card and no numbers, and if none of the routine's exercises have history, the day is not marked as trained at all.",
                "If an exercise you did not actually do ends up on the day, use \"Remove from this day\" on that card to take just that one back out.",
            ],
            fig_alt: "",
            fig: None,
        },
        ChapterText {
            title: "Collapsing by muscle group",
            body: &[
                "There are three places with a fold-by-muscle-group list, and each allows a different number to stay open at once. Both the \"Exercises\" section in Settings and the \"Add exercise\" sheet on the Record tab allow only one group open at a time.",
                "The routine-editing sheet's \"Selected\" list is the exception — it allows several groups open at once, since building one routine often means jumping back and forth between, say, chest and legs.",
                "There are two entry points for creating a routine. \"+ Save this day as a routine\" on the Record tab only appears once the day actually has logged sets — adding an empty card alone is not enough to show it.",
            ],
            fig_alt: "",
            fig: None,
        },
        ChapterText {
            title: "Reordering",
            body: &[
                "The three places you can drag to reorder differ in where you grab and how long you wait. Both an exercise card's header row and a row in the routine-editing sheet start reordering only after you hold for 250 milliseconds.",
                "Set rows are the exception — grabbing the set number at the left edge starts moving it the instant you touch it, with no wait at all. Since it reacts immediately, be sure to grab the number itself.",
                "Header rows and routine rows wait because starting to move the instant you touch them would be indistinguishable from a vertical scroll flick, reordering things when you only meant to scroll. From a keyboard, reorder with Alt+↑/↓ — plain arrow keys are reserved for moving the cursor inside the input.",
            ],
            fig_alt: "",
            fig: None,
        },
        ChapterText {
            title: "Export and import",
            body: &[
                "\"Export / Import\" in the Settings tab writes out a TSV file, which opens directly in a spreadsheet app. On iOS, choose \"Save to Files\" from the share sheet.",
                "Importing only ever adds. When the same day and exercise have sets that disagree, they are compared in order — set count, then volume, then the contents of each set — and the larger one wins. If the file has more, it replaces what you already have, and the confirmation screen says so. If what you already have is larger, nothing happens and the screen says there is nothing new to import — in that case the file's version is discarded.",
                "Right after an import, \"Undo\" can reverse it. To guard against a stray tap, the first press only arms a confirmation, and a second press actually undoes it. Closing the sheet gives up that chance to undo.",
                "A TSV file does not include an exercise's ID, color, sort order, or archived state. The time-of-day column is written on export but is not read back on import.",
            ],
            fig_alt: "",
            fig: None,
        },
    ],
};

// ── views/backup.rs ─────────────────────────────────────────────────────────

pub struct Backup {
    pub sheet_title: &'static str,
    /// 取り込む前と後の見出し
    pub before: &'static str,
    pub after: &'static str,
    /// **今ある記録が消えないことを先に言う。** ここが一番不安な点
    pub merge_only: &'static str,
    pub apply: &'static str,
    pub cancel: &'static str,
    pub undo: &'static str,
    pub export: &'static str,
    pub copy_text: &'static str,
    pub import: &'static str,
    /// 書き出しの結果
    pub exported_share: &'static str,
    pub export_cancelled: &'static str,
    pub share_failed: &'static str,
    pub copied: &'static str,
    pub copy_failed: &'static str,
    /// 取り込みの結果
    pub file_unreadable: &'static str,
    pub imported: &'static str,
    pub imported_nothing_new: &'static str,
    /// 控えが取れなかったので元に戻せない、の追記。**先頭の改行込みで持つ**
    pub no_undo_available: &'static str,
    pub undo_unreadable: &'static str,
    pub undone: &'static str,
    pub undone_no_redo: &'static str,
    /// 何も増えないが差し替えは起きる / 本当に何も起きない
    pub replaces_records: &'static str,
    pub nothing_new: &'static str,
    /// 控えの日時が読めない
    pub unknown_time: &'static str,
    /// 無名のメニュー（他画面と同じ表記に寄せる）
    pub unnamed_routine: &'static str,
    /// `added_text` の区切り
    pub join: &'static str,
}

const JA_BACKUP: Backup = Backup {
    sheet_title: "エクスポート / インポート",
    before: "現在",
    after: "取り込み後",
    merge_only: "今ある記録は消えません。無い日と無い種目だけを足します",
    apply: "取り込む",
    cancel: "やめる",
    undo: "元に戻す",
    export: "エクスポート",
    copy_text: "文字でコピー",
    import: "インポート",
    exported_share: "エクスポートしました。「ファイルに保存」を選ぶと、機種を替えても残ります",
    export_cancelled: "保存を中止しました（データは変わっていません）",
    share_failed: "共有できませんでした。「文字でコピー」でメモや自分宛メールに貼り付けてください",
    copied: "コピーしました。メモや自分宛メールに貼り付けて保存してください",
    copy_failed: "コピーできませんでした（この端末ではエクスポートする手段がありません）",
    file_unreadable: "ファイルを読めませんでした",
    imported: "取り込みました",
    imported_nothing_new: "取り込みましたが、新しく増えたものはありませんでした",
    no_undo_available: "\n（控えを保存できなかったので、元に戻せません）",
    undo_unreadable: "控えを読み出せませんでした",
    undone: "元に戻しました",
    undone_no_redo: "元に戻しました（戻す前の状態は保管できませんでした）",
    replaces_records: "入れ替わる記録があります",
    nothing_new: "新しく取り込むものはありません",
    unknown_time: "日時不明",
    unnamed_routine: "無名のメニューは元の内容を残しました",
    join: " ・ ",
};

const EN_BACKUP: Backup = Backup {
    sheet_title: "Export / Import",
    before: "Now",
    after: "After import",
    merge_only: "Nothing you already have is deleted. Only missing days and missing exercises are added.",
    apply: "Import",
    cancel: "Cancel",
    undo: "Undo",
    export: "Export",
    copy_text: "Copy as text",
    import: "Import",
    exported_share: "Exported. Choose \"Save to Files\" and it survives changing phones.",
    export_cancelled: "Saving was cancelled. Nothing changed.",
    share_failed: "Could not share. Use \"Copy as text\" and paste it into a note or an email to yourself.",
    copied: "Copied. Paste it into a note or an email to yourself to keep it.",
    copy_failed: "Could not copy — this device offers no way to export.",
    file_unreadable: "The file could not be read.",
    imported: "Imported.",
    imported_nothing_new: "Imported, but there was nothing new to add.",
    no_undo_available: "\n(A backup could not be saved, so this cannot be undone.)",
    undo_unreadable: "The backup could not be read.",
    undone: "Undone.",
    undone_no_redo: "Undone. The state before undoing could not be kept.",
    replaces_records: "Some records will be replaced.",
    nothing_new: "There is nothing new to import.",
    unknown_time: "time unknown",
    unnamed_routine: "The unnamed routine kept its original contents.",
    join: ", ",
};

// ── views/whatsnew.rs ───────────────────────────────────────────────────────

/// バナーとシートの固定文言。**お知らせ本文自体はここに置かない**（[`RELEASES`] へ）。
pub struct Releases {
    /// バナーの CTA。「見る ›」のように短く保つ（本文は [`Lang::whatsnew_banner`] が持つ）
    pub banner_cta: &'static str,
    /// バナーの ✕ の `aria-label`
    pub banner_dismiss: &'static str,
    pub sheet_title: &'static str,
}

const JA_RELEASES: Releases = Releases {
    banner_cta: "見る ›",
    banner_dismiss: "このお知らせを閉じる",
    sheet_title: "新機能のお知らせ",
};

const EN_RELEASES: Releases = Releases {
    banner_cta: "See what's new ›",
    banner_dismiss: "Dismiss this notice",
    sheet_title: "What's new",
};

/// 1 リリースぶんのお知らせ。
///
/// ★ 言語ごとにリストを分けず、エントリの中で分岐する（[`crate::presets::Names`] と
///   同じ形）。リストを 2 本にすると片方への足し忘れがコンパイルを通ってしまう。
pub struct ReleaseNote {
    /// お知らせ番号。**単調増加。二度と振り直さない**（`storage::release_seen` の基準値）
    pub id: u32,
    /// ISO 8601。表示はこのまま出す
    pub date: &'static str,
    pub ja: &'static [&'static str],
    pub en: &'static [&'static str],
}

impl ReleaseNote {
    pub const fn items(&self, lang: Lang) -> &'static [&'static str] {
        match lang {
            Lang::Ja => self.ja,
            Lang::En => self.en,
        }
    }
}

/// **新しい順。先頭が最新。** `id` は厳密減少（`core::unseen_releases` が prefix 切り出しの
/// 前提にしている）。
///
/// リリースのたびに機能 PR が自分のエントリを先頭に足す。`date` はマージ日、`id` は
/// 直前の最新から 1 つ進める。`&'static [ReleaseNote]` ではなく `&[ReleaseNote]` と書く
/// （clippy::redundant_static_lifetimes。`presets::PRESETS` と同じ書き方）。
pub const RELEASES: &[ReleaseNote] = &[
    ReleaseNote {
        id: 3,
        date: "2026-09-10",
        ja: &[
            "ラベルごとに色を選べるようにしました。設定タブ → 種目 のラベルの行に色見本が出ます。新しいラベルには互いに重ならない色が自動で付きます。",
            "推移タブでラベルを絞り込めるようにしました。種目を選ぶとラベルのチップが並び、押すとグラフの点も記録の表もそのラベルだけになります。「すべて」ではラベルの付いた日がその色の点で出ます。",
        ],
        en: &[
            "Labels can have a colour. A swatch now sits on each label row under Settings › Exercises, and new labels get colours that never repeat each other.",
            "The Progress tab can be filtered by label. Pick an exercise and its labels appear as chips; tapping one narrows both the graph and the records table to that label. Under \"All labels\", labelled days are drawn in their label's colour.",
        ],
    },
    ReleaseNote {
        id: 2,
        date: "2026-09-09",
        ja: &[
            "記録タブの「種目を追加」を部位ごとのアコーディオンにしました。部位を開くとその種目だけが並び、同時に開くのは 1 つです。",
            "新機能をまとめて知らせるこのバナーを追加しました。閉じると次のお知らせまで出ません。",
        ],
        en: &[
            "The Record tab's \"Add exercise\" sheet is now a muscle-group accordion. Opening a group shows only its exercises, and only one opens at a time.",
            "Added this banner to announce new features together. Once you close it, it stays away until the next release.",
        ],
    },
    ReleaseNote {
        id: 1,
        date: "2026-08-25",
        ja: &[
            "推移タブの対象を「部位」と「種目」の 2 段セレクタにしました。部位を選ぶと種目の候補がその部位だけに絞られます。",
        ],
        en: &[
            "The Progress tab's target is now two selects, muscle group and exercise. Picking a group narrows the exercise list to it.",
        ],
    },
];

// ── 引数が要る文言 ──────────────────────────────────────────────────────────
//
// ★ `format!` はフォーマット文字列がリテラルでなければならず、表から引いた
//   `&'static str` は渡せない。ここだけメソッドにして両言語を `match` の 2 腕に書く
//   （腕を落とせばコンパイルが通らないので、表の struct リテラルと同じ強度になる）。

impl Lang {
    /// 未来の版が書いたデータを踏み越えて旧世代から復元した。
    pub fn boot_restored_over_newer(self, version: u32) -> String {
        match self {
            Lang::Ja => format!(
                "新しい版（形式 {version}）で作られた記録は開けないので、そのまま保管しています。以前のバックアップから復元しました"
            ),
            Lang::En => format!(
                "Records created by a newer version (format {version}) cannot be opened, so they have been set aside. An earlier backup was loaded instead."
            ),
        }
    }

    /// 未来の版が書いたデータしか無かった。**新しい版に戻せば読めることを伝える。**
    pub fn boot_found_newer(self, version: u32) -> String {
        match self {
            Lang::Ja => format!(
                "新しい版（形式 {version}）で作られた記録が見つかりました。このままでは開けないので、そのまま保管しています（新しい版に戻すと読めます）"
            ),
            Lang::En => format!(
                "Records created by a newer version (format {version}) were found. They cannot be opened here, so they have been set aside — go back to the newer version to read them."
            ),
        }
    }

    /// 新しい版で作られた記録を取り込もうとした。
    pub fn err_unsupported(self, version: u32) -> String {
        match self {
            Lang::Ja => {
                format!("新しい版（形式 {version}）で作られた記録です。アプリを更新してください")
            }
            Lang::En => format!(
                "This log was created by a newer version (format {version}). Please update the app."
            ),
        }
    }

    /// 取り込み前後のサマリ。「種目 28 ・ 記録 12 日 ・ 90 セット ・ 2026-08-01 〜 2026-08-19」
    ///
    /// ★ 助詞と中黒で繋いでいた組み立てをここに畳んである（`chart_summary` と同じ理由）。
    pub fn db_summary(
        self,
        exercises: usize,
        days: usize,
        sets: usize,
        range: Option<(&str, &str)>,
    ) -> String {
        let sep = self.strings().backup.join;
        let span = match range {
            Some((a, b)) if a == b => format!("{sep}{a}"),
            Some((a, b)) => match self {
                Lang::Ja => format!("{sep}{a} 〜 {b}"),
                Lang::En => format!("{sep}{a} – {b}"),
            },
            None => String::new(),
        };
        match self {
            Lang::Ja => format!("種目 {exercises} ・ 記録 {days} 日 ・ {sets} セット{span}"),
            Lang::En => format!(
                "{exercises} {} {sep}{days} {} logged{sep}{sets} {}{span}",
                plural(exercises, "exercise", "exercises"),
                plural(days, "day", "days"),
                plural(sets, "set", "sets"),
            ),
        }
    }

    /// マージで起きた食い違い 1 件の説明。
    pub fn conflict_renamed(self, incoming: &str, kept: &str) -> String {
        match self {
            Lang::Ja => format!("「{incoming}」は「{kept}」として扱いました"),
            Lang::En => format!("\"{incoming}\" was treated as \"{kept}\""),
        }
    }

    pub fn conflict_name_matched(self, name: &str) -> String {
        match self {
            Lang::Ja => format!("「{name}」は同じ種目とみなしました"),
            Lang::En => format!("\"{name}\" was taken to be the same exercise"),
        }
    }

    pub fn conflict_sets_diverged(self, date: &str, name: &str) -> String {
        match self {
            Lang::Ja => format!("{date} の「{name}」は取り込んだ側のセットを採りました"),
            Lang::En => format!("For \"{name}\" on {date}, the imported sets were used"),
        }
    }

    pub fn conflict_body_weight(self, date: &str) -> String {
        match self {
            Lang::Ja => format!("{date} の体重は元の値を残しました"),
            Lang::En => format!("The body weight on {date} kept its original value"),
        }
    }

    pub fn conflict_routine_diverged(self, name: &str) -> String {
        match self {
            Lang::Ja => format!("メニュー「{name}」は元の内容を残しました"),
            Lang::En => format!("The routine \"{name}\" kept its original contents"),
        }
    }

    /// ラベルで絞っているときの履歴ブロックの `aria-label`。
    ///
    /// ★ `Day::past_records` と別に持つのは、`format!` がリテラルな書式文字列しか
    /// 取れないので `&'static str` では組み立てられないため。
    pub fn past_records_of(self, label: &str) -> String {
        match self {
            Lang::Ja => format!("{label} の前回までの記録"),
            Lang::En => format!("Past {label} sessions"),
        }
    }

    /// ラベルの ✕ ボタンの `aria-label`。
    ///
    /// ★ 名前を `delete_label` にする（`remove_label` にすると
    /// `core::remove_label` と読み分けられない）。
    pub fn delete_label(self, name: &str) -> String {
        match self {
            Lang::Ja => format!("{name} を削除"),
            Lang::En => format!("Delete {name}"),
        }
    }

    /// ラベルの色ピッカーの `aria-label`。
    ///
    /// ★ **空名なら `settings.field_color`（「色」/ "Colour"）へ落とす。** `＋` で
    /// 足した直後の行は名前がまだ無く、そのまま埋め込むと「 の色」/"Colour of "
    /// という尻切れの読み上げになる。部位編集の色ピッカーと同じ語に落ちるだけなので
    /// 意味も失われない。
    pub fn color_of(self, name: &str) -> String {
        if name.trim().is_empty() {
            return self.strings().settings.field_color.to_string();
        }
        match self {
            Lang::Ja => format!("{name} の色"),
            Lang::En => format!("Colour of {name}"),
        }
    }

    /// 増えるものの名詞句の部品。**語尾を付けない** — 確認では「を追加します」、
    /// 実行後は「を追加」と付け替えるので、ここで文にすると両方に使えない。
    pub fn added_days(self, n: usize) -> String {
        match self {
            Lang::Ja => format!("{n} 日分"),
            Lang::En => format!("{n} {}", plural(n, "day", "days")),
        }
    }

    pub fn added_logs(self, n: usize) -> String {
        match self {
            Lang::Ja => format!("{n} 件の記録"),
            Lang::En => format!("{n} {}", plural(n, "record", "records")),
        }
    }

    pub fn added_notes(self, n: usize) -> String {
        match self {
            Lang::Ja => format!("{n} 件のメモ"),
            Lang::En => format!("{n} {}", plural(n, "note", "notes")),
        }
    }

    /// 取り込みで新しく段が入ったセットの数。
    pub fn added_drops(self, n: usize) -> String {
        match self {
            Lang::Ja => format!("{n} 件のドロップ"),
            Lang::En => format!("{n} {}", plural(n, "drop set", "drop sets")),
        }
    }

    pub fn added_groups(self, n: usize) -> String {
        match self {
            Lang::Ja => format!("{n} 部位"),
            Lang::En => format!("{n} muscle {}", plural(n, "group", "groups")),
        }
    }

    pub fn added_routines(self, n: usize) -> String {
        match self {
            Lang::Ja => format!("{n} 件のメニュー"),
            Lang::En => format!("{n} {}", plural(n, "routine", "routines")),
        }
    }

    /// ラベルの定義の追加とログへの付与を**合算した** 1 本のカウンタ
    /// （`added_notes` が既に異種混合の先例）。
    pub fn added_labels(self, n: usize) -> String {
        match self {
            Lang::Ja => format!("{n} 件のラベル"),
            Lang::En => format!("{n} {}", plural(n, "label", "labels")),
        }
    }

    /// 上限に達していて取り込めなかったラベル定義の警告。
    ///
    /// ★ **`Conflict` としては出さない。** あちらは「記録の食い違い」の型で、
    /// 確認画面が `conflicts.is_empty()` で「入れ替わる記録があります」に分岐するので、
    /// 記録が 1 件も入れ替わらないこの事象を積むと確認画面が嘘をつく。
    ///
    /// ★ **責めずに「何が起きたか」と「何をすれば直るか」を書く**
    /// （`Day::weight_missing` の「回数を入れると保存されます」と同じ作法）。
    /// 落ちた定義を指す記録は消えていない — 「指定なし」で見られる。
    pub fn dropped_labels(self, n: usize) -> String {
        match self {
            Lang::Ja => format!(
                "{n} 件のラベルは、その種目のラベルが上限に達していたため取り込みませんでした。記録は消えていないので「指定なし」で見られます。使わないラベルを設定タブで消してから取り込み直すと入ります"
            ),
            Lang::En => format!(
                "{n} label {} {} not imported — that exercise is already at its label limit. No records were lost; they are still visible under Any. Delete a label you no longer use under Settings, then import again.",
                plural(n, "definition", "definitions"),
                plural(n, "was", "were"),
            ),
        }
    }

    /// 確認画面の 1 行目（増えるものがあるとき）。
    pub fn will_add(self, added: &str) -> String {
        match self {
            Lang::Ja => format!("{added} を追加します"),
            Lang::En => format!("Will add {added}."),
        }
    }

    /// 実行後の報告（増えたものがあるとき）。
    pub fn imported_with(self, added: &str) -> String {
        match self {
            Lang::Ja => format!("取り込みました（{added} を追加）"),
            Lang::En => format!("Imported. Added {added}."),
        }
    }

    /// 書き出し先の名前。
    pub fn exported_to(self, name: &str) -> String {
        match self {
            Lang::Ja => format!("{name} にエクスポートしました"),
            Lang::En => format!("Exported to {name}."),
        }
    }

    /// 「元に戻す」の武装。**押すと何が消えるかを書く。**
    pub fn undo_arm(self, when: &str) -> String {
        match self {
            Lang::Ja => format!(
                "{when} の状態に戻します。それ以降に付けた記録は消えます。もう一度押すと実行します"
            ),
            Lang::En => format!(
                "This restores the state from {when}. Anything recorded after that is deleted. Press again to go ahead."
            ),
        }
    }

    /// アーカイブ済み種目があるため記録タブに出ない、の説明。
    pub fn archived_only(self, n: usize) -> String {
        match self {
            Lang::Ja => format!("アーカイブ済みの {n} 種目は記録タブに出ません"),
            Lang::En => format!(
                "{n} archived {} do not appear on the Record tab",
                plural(n, "exercise", "exercises")
            ),
        }
    }

    /// 部位の編集ボタンの `aria-label`。
    pub fn edit_group_label(self, name: &str) -> String {
        match self {
            Lang::Ja => format!("{name} の名前と色を編集"),
            Lang::En => format!("Edit the name and colour of {name}"),
        }
    }

    /// アーカイブ済みの件数。「{n} 件」/ "{n}"。
    pub fn n_items(self, n: usize) -> String {
        match self {
            Lang::Ja => format!("{n} 件"),
            Lang::En => format!("{n}"),
        }
    }

    /// 部位を消せない理由。`archived` はそのうちアーカイブ済みの数。
    pub fn cannot_delete_group(self, total: usize, archived_only: bool) -> String {
        match (self, archived_only) {
            (Lang::Ja, true) => format!("アーカイブ済み種目が {total} 件あるため削除できません"),
            (Lang::Ja, false) => format!("種目が {total} 件あるため削除できません"),
            (Lang::En, true) => format!(
                "Cannot delete — it still has {total} archived {}",
                plural(total, "exercise", "exercises")
            ),
            (Lang::En, false) => format!(
                "Cannot delete — it still has {total} {}",
                plural(total, "exercise", "exercises")
            ),
        }
    }

    /// うち何件がアーカイブ済みか。
    pub fn of_which_archived(self, archived: usize) -> String {
        match self {
            Lang::Ja => format!("うち {archived} 件はアーカイブ済みです"),
            Lang::En => format!("{archived} of them are archived"),
        }
    }

    /// 種目の追加先。
    pub fn adding_to_group(self, group_name: &str) -> String {
        match self {
            Lang::Ja => format!("{group_name} に追加します"),
            Lang::En => format!("Adding to {group_name}"),
        }
    }

    /// メニューのコピー候補で、収まらなかった部位名を畳む語。「 他」/ " +more"。
    pub fn and_more(self) -> String {
        match self {
            Lang::Ja => " 他".to_string(),
            Lang::En => " +more".to_string(),
        }
    }

    /// 同上、種目名を畳むとき。「 他{n}種目」/ " +{n} more"。
    pub fn and_n_more_exercises(self, n: usize) -> String {
        match self {
            Lang::Ja => format!(" 他{n}種目"),
            Lang::En => format!(" +{n} more"),
        }
    }

    /// 選択日を編集していることの断り。
    pub fn editing_day(self, date: &str) -> String {
        match self {
            Lang::Ja => format!("{date} を編集中"),
            Lang::En => format!("Editing {date}"),
        }
    }

    /// セットメモ欄の `aria-label`。「{n} セット目のメモ」。
    pub fn set_note_label(self, index: usize) -> String {
        match self {
            Lang::Ja => format!("{index} セット目のメモ"),
            Lang::En => format!("Note for set {index}"),
        }
    }

    /// メニュー編集で「選択中」から 1 件外すボタンの `aria-label`。
    pub fn remove_from_routine(self, name: &str) -> String {
        match self {
            Lang::Ja => format!("{name} を外す"),
            Lang::En => format!("Remove {name}"),
        }
    }

    /// 「{n} 種目」/ "{n} exercises"。メニューの行に出す件数。
    pub fn n_exercises(self, n: usize) -> String {
        match self {
            Lang::Ja => format!("{n} 種目"),
            Lang::En => format!("{n} {}", plural(n, "exercise", "exercises")),
        }
    }

    /// 種目カードに出す過去の記録の件数。「3 回分」/ "3 sessions"。
    ///
    /// ★ **「日」ではなく「回」。** 出すのは直近 N 日ではなく**その種目をやった直近 N 回**で、
    ///   3 回分が 3 週間にまたがることがある。
    /// ★ 設定行の右端とセグメントのラベルで**同じこれを使う**（`endonym()` と同じ作法）。
    ///   素の数字だと「種目 28」「メニュー 0」と同じ件数に見えて意味が読めない
    pub fn n_past_sessions(self, n: usize) -> String {
        match self {
            Lang::Ja => format!("{n} 回分"),
            Lang::En => format!("{n} {}", plural(n, "session", "sessions")),
        }
    }

    /// グラフの `aria-label`。**1 本のメソッドに畳んである。**
    ///
    /// ★ 日本語版は「{期間}の推移。最大 {n} {単位}。体重 {min}〜{max} kg。体重の線は週平均」と
    ///   助詞で部品を繋いでいた。助詞の連結は英語に移せない（語順も接続詞も違う）ので、
    ///   組み立てごと言語ごとに書く。日本語の腕は元の文を 1 文字も変えずに再現してある。
    ///
    /// - `span`: `(始まりの日, 終わりの日)`。点が無ければ `None`
    /// - `metric`: `(最大値, 単位)`。体重だけのグラフなら `None`
    /// - `weight`: `(最小, 最大)` kg。体重の線が出ていなければ `None`
    /// - `smoothed`: 体重の線が週平均に落ちているか
    pub fn chart_summary(
        self,
        span: Option<(&str, &str)>,
        metric: Option<(&str, &str)>,
        weight: Option<(&str, &str)>,
        smoothed: bool,
    ) -> String {
        let mut out = String::new();
        match self {
            Lang::Ja => {
                if let Some((from, to)) = span {
                    out.push_str(&format!("{from} から {to} まで"));
                }
                match metric {
                    Some((max, unit)) => out.push_str(&format!("の推移。最大 {max} {unit}")),
                    None => out.push_str("の体重の推移"),
                }
                if let Some((min, max)) = weight {
                    out.push_str(&format!("。体重 {min}〜{max} kg"));
                }
                if smoothed {
                    out.push_str("。体重の線は週平均");
                }
            }
            Lang::En => {
                if let Some((from, to)) = span {
                    out.push_str(&format!("{from} to {to}. "));
                }
                match metric {
                    // ★ ボリュームは単位を持たないので、空文字を挟んで
                    //   "Peak 1,080 ." にしない
                    Some((max, "")) => out.push_str(&format!("Peak {max}.")),
                    Some((max, unit)) => out.push_str(&format!("Peak {max} {unit}.")),
                    None => out.push_str("Body weight over time."),
                }
                if let Some((min, max)) = weight {
                    out.push_str(&format!(" Body weight {min}–{max} kg."));
                }
                if smoothed {
                    out.push_str(" The body-weight line is a weekly average.");
                }
            }
        }
        out
    }

    /// 記録テーブルの省略行。「他 {n} 件は表示していません」。
    pub fn n_more_hidden(self, n: usize) -> String {
        match self {
            Lang::Ja => format!("他 {n} 件は表示していません"),
            Lang::En => format!("{n} more {} not shown", plural(n, "record", "records")),
        }
    }

    /// 「{n}日前」/ "{n} days ago"。
    ///
    /// ★ 英語の単複はここでは出ない — 呼び側の `humanize_days` が 0 と 1 を
    ///   `today` / `yesterday` で先に返すので、ここへ来る `n` は必ず 2 以上。
    ///   それでも `plural` を通しておく（後から 1 を渡す人が罠を踏まない）。
    pub fn days_ago(self, n: i64) -> String {
        match self {
            Lang::Ja => format!("{n}日前"),
            Lang::En => format!(
                "{n} {} ago",
                plural(n.unsigned_abs() as usize, "day", "days")
            ),
        }
    }

    /// 「{n}分」/ "{n} min"。同じ暦日の時刻粒度。
    pub fn minutes_ago(self, n: i64) -> String {
        match self {
            Lang::Ja => format!("{n}分"),
            Lang::En => format!("{n} min"),
        }
    }

    /// 「{n}時間」/ "{n} hr"。
    pub fn hours_ago(self, n: i64) -> String {
        match self {
            Lang::Ja => format!("{n}時間"),
            Lang::En => format!("{n} hr"),
        }
    }

    /// カレンダー月フッタの「実施」の値。「{n} 日」/ "{n} days"。
    pub fn n_days(self, n: usize) -> String {
        match self {
            Lang::Ja => format!("{n} 日"),
            Lang::En => format!("{n} {}", plural(n, "day", "days")),
        }
    }

    /// カレンダーの見出し。"2026年8月" / "August 2026"。
    pub fn month_heading(self, year: i32, month0: usize) -> String {
        let m = self.strings().cal.months_long[month0];
        match self {
            Lang::Ja => format!("{year}年{m}"),
            Lang::En => format!("{m} {year}"),
        }
    }

    /// 旧世代キーのほうに新しい記録が残っている。**消えていないことだけを伝える。**
    pub fn boot_newer_legacy(self, date: &str) -> String {
        match self {
            Lang::Ja => format!(
                "以前のバージョンで付けた記録が {date} まで残っています（今の表示には含まれていません）"
            ),
            Lang::En => format!(
                "Records made in an earlier version go up to {date}. They are kept, but are not included in what you see now."
            ),
        }
    }

    /// 一番上のバナーの本文。`n` は未読リリースの**項目の総数**（リリース数ではない）。
    ///
    /// ★ 件数を含む文言は表に置けない（`format!` はフォーマット文字列がリテラルを
    ///   要求するので、表から引いた `&'static str` は渡せない）。ここに置いて
    ///   `match` の腕を 1 つでも落とせばコンパイルが通らないようにする。
    pub fn whatsnew_banner(self, n: usize) -> String {
        match self {
            Lang::Ja => format!("新しい機能が {n} 件あります"),
            Lang::En => format!("{n} new {} to see", plural(n, "feature", "features")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_tags_resolve_to_japanese_only_for_the_ja_primary_subtag() {
        assert_eq!(from_bcp47("ja"), Lang::Ja);
        assert_eq!(from_bcp47("ja-JP"), Lang::Ja);
        assert_eq!(from_bcp47("ja_JP"), Lang::Ja);
        assert_eq!(from_bcp47("JA-jp"), Lang::Ja);

        assert_eq!(from_bcp47("en"), Lang::En);
        assert_eq!(from_bcp47("en-US"), Lang::En);
        assert_eq!(from_bcp47("fr"), Lang::En);
        assert_eq!(from_bcp47(""), Lang::En);
    }

    /// ★ `starts_with("ja")` で書くと通ってしまう綴り。回帰で落とすために単独で持つ。
    #[test]
    fn a_language_whose_tag_merely_starts_with_ja_is_not_japanese() {
        assert_eq!(from_bcp47("jam"), Lang::En); // ジャマイカ・クレオール
        assert_eq!(from_bcp47("jav"), Lang::En); // ジャワ語
        assert_eq!(from_bcp47("jbo"), Lang::En); // ロジバン
    }

    #[test]
    fn an_unknown_saved_value_falls_back_to_unset_rather_than_a_language() {
        assert_eq!(parse_saved("ja"), Some(Lang::Ja));
        assert_eq!(parse_saved("en"), Some(Lang::En));

        // 手で編集された / 将来の言語が書いた値。英語で固定せず「未設定」に落ちる
        assert_eq!(parse_saved("fr"), None);
        assert_eq!(parse_saved("ja-JP"), None);
        assert_eq!(parse_saved(""), None);
    }

    /// 保存 → 復元の往復。`tag()` が書いた値は `parse_saved` が必ず読める。
    #[test]
    fn every_language_round_trips_through_its_tag() {
        for (lang, _) in Lang::CHOICES {
            assert_eq!(parse_saved(lang.tag()), Some(lang));
        }
    }

    #[test]
    fn choices_cover_every_language_and_label_each_in_its_own_script() {
        assert_eq!(Lang::CHOICES.len(), 2);
        for (lang, label) in Lang::CHOICES {
            assert_eq!(label, lang.endonym());
        }
    }

    #[test]
    fn plural_switches_only_at_one() {
        assert_eq!(plural(0, "day", "days"), "days");
        assert_eq!(plural(1, "day", "days"), "day");
        assert_eq!(plural(2, "day", "days"), "days");
    }

    /// 2 つの select は横に並ぶ。先頭の「すべて」が同じ語だと、どちらを
    /// 触っているのか分からなくなる。ラベルも同様。
    #[test]
    fn the_two_progress_selectors_read_apart_in_every_language() {
        for (lang, _) in Lang::CHOICES {
            let p = &lang.strings().progress;
            assert_ne!(p.all_groups, p.all_exercises, "{lang:?} の「すべて」が同語");
            assert_ne!(p.pick_group, p.pick_exercise, "{lang:?} のラベルが同語");
        }
    }

    /// 対象セレクタには見えるラベルが無く、`aria-label` だけが読み上げの頼り。
    #[test]
    fn the_progress_selector_labels_are_never_empty() {
        for (lang, _) in Lang::CHOICES {
            let p = &lang.strings().progress;
            for s in [
                p.pick_group,
                p.pick_exercise,
                p.all_groups,
                p.all_exercises,
                p.pick_hint,
            ] {
                assert!(!s.is_empty(), "{lang:?} に空の文言がある");
            }
        }
    }

    /// 先頭のチップは `LabelFilter::Any`（絞らない）で、利用者が定義した
    /// どのラベルとも別の状態。名詞句が「ラベル」自体と同語だと、押したときに
    /// 何が起きるのか読めない。
    #[test]
    fn the_any_label_chip_reads_apart_from_the_label_row_itself() {
        for (lang, _) in Lang::CHOICES {
            let d = &lang.strings().day;
            assert_ne!(d.label_any, d.labels, "{lang:?} のチップと行が同語");
            assert!(!d.label_any.is_empty(), "{lang:?} に空の文言がある");
            assert!(!d.labels.is_empty(), "{lang:?} に空の文言がある");
        }
    }

    /// ラベル名を埋め込む文言は `&'static str` では作れないので `impl Lang` 側に
    /// ある。**名前が本文に必ず出ること**を見る（消えると「何を削除するのか」が
    /// 読めないボタンになる）。削除の確認は「作り直しても戻らない」まで言う。
    #[test]
    fn the_label_texts_carry_the_name_and_warn_that_deleting_is_final() {
        for (lang, _) in Lang::CHOICES {
            assert!(lang.past_records_of("Power").contains("Power"), "{lang:?}");
            assert!(lang.delete_label("Power").contains("Power"), "{lang:?}");
            assert!(lang.added_labels(2).contains('2'), "{lang:?}");
            assert!(lang.color_of("Power").contains("Power"), "{lang:?}");
            // ★ 空名は「 の色」/"Colour of " と尻切れにせず、部位の色と同じ語へ落とす
            assert_eq!(
                lang.color_of("  "),
                lang.strings().settings.field_color,
                "{lang:?}: 名前の無い行の色ピッカーが尻切れの読み上げになる"
            );

            let confirm = lang.strings().settings.delete_label_confirm;
            assert!(!confirm.is_empty(), "{lang:?} に空の文言がある");
            // 同じ名前で作り直しても ID が変わるので過去の記録は戻らない
            let again = match lang {
                Lang::Ja => "同じ名前",
                Lang::En => "same name",
            };
            assert!(confirm.contains(again), "{lang:?}: {confirm}");
        }
    }

    /// ★ **推移タブのラベルのチップは、隣の 2 つのセレクタとも期間とも別語。**
    /// `.selectors` の中に「すべての部位」「すべての種目」「全期間」「すべて」が
    /// 同時に並ぶので、どれか 2 つが同語だとどれの「すべて」か取り違える
    /// （`all_groups` の ★ と同じ理由を、面が 1 つ増えたぶんまで広げたもの）。
    #[test]
    fn the_progress_label_chip_reads_apart_from_the_selectors_and_the_period() {
        for (lang, _) in Lang::CHOICES {
            let p = &lang.strings().progress;
            let words = [
                p.all_labels,
                p.all_groups,
                p.all_exercises,
                p.period_all,
                p.pick_label,
                p.empty_period_label,
            ];
            for w in words {
                assert!(!w.is_empty(), "{lang:?} に空の文言がある");
            }
            let uniq: std::collections::HashSet<&str> = words.into_iter().collect();
            assert_eq!(uniq.len(), words.len(), "{lang:?}: {words:?} に同語がある");
            // 絞って 0 件の文言は、種目の 0 件とも別（「すべて」に戻せることが読めない）
            assert_ne!(p.empty_period_label, p.empty_period_exercise, "{lang:?}");
            assert_ne!(p.empty_period_label, p.empty_period, "{lang:?}");
        }
    }

    // ── お知らせ（新機能バナー） ────────────────────────────────────────────

    /// 各エントリで `ja.len() == en.len()`、空文字を含まない。
    ///
    /// ★ リストを 1 本にした狙いそのものの検証。片方の言語だけ項目を書き忘れても
    ///   `RELEASES` の宣言はコンパイルを通るので、ここで人力の突き合わせを肩代わりする。
    #[test]
    fn release_notes_line_up_across_languages() {
        for r in RELEASES {
            assert_eq!(
                r.ja.len(),
                r.en.len(),
                "id={} で日英の項目数が食い違っている",
                r.id
            );
            for s in r.ja.iter().chain(r.en.iter()) {
                assert!(!s.is_empty(), "id={} に空の項目がある", r.id);
            }
        }
    }

    /// マニュアルの章数・見出し・本文が日英とも `MANUAL_CHAPTERS` と揃っていること、
    /// および `has_fig` / `fig` / `fig_alt` の 3 者が食い違っていないことを見る。
    #[test]
    fn every_language_has_the_same_manual_chapters() {
        let want = crate::manual::MANUAL_CHAPTERS.len();
        assert_eq!(
            JA_MANUAL.chapters.len(),
            want,
            "日本語の章数が MANUAL_CHAPTERS と食い違う"
        );
        assert_eq!(
            EN_MANUAL.chapters.len(),
            want,
            "英語の章数が MANUAL_CHAPTERS と食い違う"
        );

        for (lang, _) in Lang::CHOICES {
            let m = &lang.strings().manual;
            for (skeleton, text) in crate::manual::MANUAL_CHAPTERS.iter().zip(m.chapters) {
                assert!(
                    !text.title.is_empty(),
                    "{lang:?} の {} に空の見出し",
                    skeleton.id
                );
                assert!(
                    !text.body.is_empty(),
                    "{lang:?} の {} に本文が無い",
                    skeleton.id
                );
                for p in text.body {
                    assert!(
                        !p.is_empty(),
                        "{lang:?} の {} に空の段落がある",
                        skeleton.id
                    );
                }
                assert_eq!(
                    skeleton.has_fig,
                    text.fig.is_some(),
                    "{lang:?} の {} で has_fig と fig の有無が食い違う",
                    skeleton.id
                );
                assert_eq!(
                    text.fig.is_some(),
                    !text.fig_alt.is_empty(),
                    "{lang:?} の {} で fig と fig_alt の有無が食い違う",
                    skeleton.id
                );
                // ★ このコミットでは図がまだ無い（撮影はコミット #4）ので、px 幅ベースの
                //   言語別上限（ja ≈ 29 字 / en ≈ 55〜70 字、実測して #4 で決める）ではなく
                //   「空であること」だけを見ている。上の 2 つの assert_eq! が
                //   `has_fig == fig.is_some() == !fig_alt.is_empty()` を already 保証するので、
                //   これは意図の確認（今はまだ 1 枚も無いはず）を兼ねる。
                assert!(
                    text.fig_alt.is_empty(),
                    "{lang:?} の {} の fig_alt はこのコミットでは空のはず",
                    skeleton.id
                );
            }
        }
    }

    /// 新しい順で id が厳密減少する（`core::unseen_releases` が prefix 切り出しの前提にしている）。
    #[test]
    fn release_ids_run_strictly_downward() {
        for pair in RELEASES.windows(2) {
            assert!(
                pair[0].id > pair[1].id,
                "id={} の次に id={} が来ている（新しい順・厳密減少ではない）",
                pair[0].id,
                pair[1].id
            );
        }
    }

    /// `RELEASES` が非空。**空だと `None` が永久に残る**（`whatsnew::bootstrap` が
    /// `latest_release_id` を引けず基準値を書かないため、次のリリースも出なくなる）。
    #[test]
    fn there_is_always_at_least_one_release() {
        assert!(!RELEASES.is_empty(), "RELEASES を空のまま出荷しない");
    }

    /// ★ `assert_ne!(banner(1), banner(2))` では**何も検証できない**。`n` が本文に
    ///   埋まるので数字だけで必ず異なり、`plural` を外しても通ってしまう。語形そのものを見る。
    #[test]
    fn the_whatsnew_banner_switches_at_one() {
        // 前後の空白まで含めて見る。`" feature "` は "features to see" には一致しない
        assert!(
            Lang::En.whatsnew_banner(1).contains(" feature "),
            "英語の 1 件が単数形になっていない"
        );
        assert!(
            Lang::En.whatsnew_banner(2).contains(" features "),
            "英語の 2 件が複数形になっていない"
        );
        // 日本語は単複で語形が変わらない。数字以外が同形であることを確かめる
        assert_eq!(
            Lang::Ja.whatsnew_banner(1).replace('1', "N"),
            Lang::Ja.whatsnew_banner(2).replace('2', "N"),
            "日本語で数字以外が変わっている"
        );
    }
    /// 段落数の日英ずれを検出する（片方だけ 1 段落増えるのがドリフトの典型）。
    #[test]
    fn the_manual_paragraph_counts_match_across_languages() {
        for ((skeleton, ja), en) in crate::manual::MANUAL_CHAPTERS
            .iter()
            .zip(JA_MANUAL.chapters)
            .zip(EN_MANUAL.chapters)
        {
            assert_eq!(
                ja.body.len(),
                en.body.len(),
                "{} の段落数が日英で食い違う",
                skeleton.id
            );
        }
    }
}
