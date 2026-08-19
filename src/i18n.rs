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
}

const JA: S = S {
    common: JA_COMMON,
    cal: JA_CAL,
    boot: JA_BOOT,
    settings: JA_SETTINGS,
    core: JA_CORE,
};

const EN: S = S {
    common: EN_COMMON,
    cal: EN_CAL,
    boot: EN_BOOT,
    settings: EN_SETTINGS,
    core: EN_CORE,
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
}

const JA_SETTINGS: Settings = Settings {
    title: "設定",
    back: "設定へ戻る",
    row_backup: "エクスポート / インポート",
    row_routines: "トレーニングメニュー",
    row_exercises: "種目",
    row_language: "言語",
    language_note: "種目名と部位名は変わりません（自分で付けた名前として扱うため）。変えたいときは「種目」から 1 つずつ編集してください",
};

const EN_SETTINGS: Settings = Settings {
    title: "Settings",
    back: "Back to Settings",
    row_backup: "Export / Import",
    row_routines: "Routines",
    row_exercises: "Exercises",
    row_language: "Language",
    language_note: "Exercise and muscle-group names do not change — they are treated as names you gave them. Edit them one by one under Exercises if you want them in another language.",
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
