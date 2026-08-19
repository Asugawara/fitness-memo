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
    /// `views/backup.rs` — エクスポート / インポート
    pub backup: Backup,
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
    backup: JA_BACKUP,
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
    backup: EN_BACKUP,
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
}

const JA_SETTINGS: Settings = Settings {
    title: "設定",
    back: "設定へ戻る",
    row_backup: "エクスポート / インポート",
    row_routines: "トレーニングメニュー",
    row_exercises: "種目",
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
};

const EN_SETTINGS: Settings = Settings {
    title: "Settings",
    back: "Back to Settings",
    row_backup: "Export / Import",
    row_routines: "Routines",
    row_exercises: "Exercises",
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
    /// セレクタ 3 つの `aria-label`
    pub pick_target: &'static str,
    pub pick_metric: &'static str,
    pub pick_period: &'static str,
    /// `<optgroup>` の見出し
    pub optgroup_groups: &'static str,
    pub optgroup_exercises: &'static str,
    pub optgroup_archived: &'static str,
    /// 全期間だけ週単位に落ちることの断り。体重の線が出ているかで文が変わる
    pub weekly_note: &'static str,
    pub weekly_note_with_weight: &'static str,
    /// この期間・この対象に記録が無い
    pub empty_period_exercise: &'static str,
    pub empty_period: &'static str,
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
    pick_target: "対象",
    pick_metric: "指標",
    pick_period: "期間",
    optgroup_groups: "部位",
    optgroup_exercises: "種目",
    optgroup_archived: "アーカイブ済み",
    weekly_note: "全期間は週単位で集計しています",
    weekly_note_with_weight: "全期間は週単位で集計しています（体重は週平均）",
    empty_period_exercise: "この期間、この種目の記録はありません",
    empty_period: "この期間の記録はありません",
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
    pick_target: "Target",
    pick_metric: "Metric",
    pick_period: "Period",
    optgroup_groups: "Muscle groups",
    optgroup_exercises: "Exercises",
    optgroup_archived: "Archived",
    weekly_note: "Over all time, figures are grouped by week.",
    weekly_note_with_weight: "Over all time, figures are grouped by week (body weight is a weekly average).",
    empty_period_exercise: "No records for this exercise in this period.",
    empty_period: "No records in this period.",
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
    /// 前回の記録が 1 件も無い
    pub no_last_log: &'static str,
    pub copy_last: &'static str,
    /// セット行の入力欄
    pub weight: &'static str,
    pub reps: &'static str,
    pub delete_set: &'static str,
    /// 保存されない理由。**責めずに「あと何をすれば保存されるか」を書く**
    pub weight_missing: &'static str,
    pub reps_missing: &'static str,
    pub add_set: &'static str,
    pub exercise_note: &'static str,
    pub note_open: &'static str,
    pub note_close: &'static str,
    /// 種目をその日から外す
    pub remove_from_day: &'static str,
    pub remove_confirm: &'static str,
    pub remove_yes: &'static str,
    pub remove_no: &'static str,
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
    no_last_log: "前回 —",
    copy_last: "前回をコピー",
    weight: "重量",
    reps: "回数",
    delete_set: "このセットを削除",
    weight_missing: "重量未入力",
    reps_missing: "回数を入れると保存されます",
    add_set: "+ セット",
    exercise_note: "この種目のメモ",
    note_open: "＋ メモ",
    note_close: "－ メモ",
    remove_from_day: "この日から外す",
    remove_confirm: "この日の記録が消えます",
    remove_yes: "外す",
    remove_no: "やめる",
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
    no_last_log: "Last —",
    copy_last: "Copy last time",
    weight: "Weight",
    reps: "Reps",
    delete_set: "Delete this set",
    weight_missing: "No weight yet",
    reps_missing: "Enter reps and this set is saved",
    add_set: "+ Set",
    exercise_note: "Note for this exercise",
    note_open: "+ Note",
    note_close: "- Note",
    remove_from_day: "Remove from this day",
    remove_confirm: "This day's record will be deleted.",
    remove_yes: "Remove",
    remove_no: "Cancel",
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

    /// 前回いつやったか。「前回 3日前」/ "Last: 3 days ago"。
    pub fn last_log(self, when: &str) -> String {
        match self {
            Lang::Ja => format!("前回 {when}"),
            Lang::En => format!("Last: {when}"),
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
}
