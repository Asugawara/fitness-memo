//! 他アプリの画面から起こしたテキストを読み取る。**ターゲット非依存の純関数。**
//!
//! 移行元にエクスポートが無いアプリからデータを移す唯一の汎用手段が、iOS の
//! テキスト認識表示（Live Text）でスクリーンショットから起こした文字列を貼り付けて
//! もらうこと。iOS のサンドボックスは他アプリのコンテナを一切見せないので、
//! こちらから自動で読む道は無い（adr/ux/migrate-by-ocr-paste.md）。
//!
//! `web-sys` に触れないので、ホストの `cargo test` がそのまま検証できる
//! （`chart_layout` と同じ立て付け / adr/architecture/chart-layout-as-a-testable-module.md）。OCR の出力は端末とアプリで
//! いくらでも揺れる。**書式の判定はテストで固めて、実機で外した分だけ足す。**
//!
//! ## 設計上の要点
//!
//! - **読み取れなかった行は必ず [`Draft::ignored`] に積む。** 黙って捨てると、
//!   移行できたつもりで前のアプリを消される。何を捨てたかは画面に全文で出す
//! - **未来の日付を作らない。** 年の無い `8/7` は「今日以前で最も近い年」に寄せる。
//!   ここを間違えると、カレンダーの先の方に幽霊の記録が湧く
//! - **`ExerciseLog.at` は常に `None`。** 過去日のバックフィル扱い（adr/data-model/at-optional-same-day-only.md）。
//!   `now` を書くと「最後のトレーニングから」が嘘になる
//! - **新しい部位は作らない。** 新規種目の置き場所は利用者が既存の部位から選ぶ

use std::collections::BTreeMap;

use chrono::{Datelike, NaiveDate};

use crate::core::date_key;
use crate::model::{Db, Exercise, ExerciseId, ExerciseLog, GroupId, IdGen, Session, SetEntry};

/// lb → kg。国際ポンドの定義値。
const LB_TO_KG: f64 = 0.453_592_37;

// ── 読み取り結果 ────────────────────────────────────────────────────────────

/// 1 種目分の読み取り結果。
#[derive(Debug, Clone, PartialEq)]
pub struct DraftLog {
    /// 画面に出す元の表記。新規種目の名前にもこれを使う
    pub raw_name: String,
    /// 照合キー。新規種目の割り当ては**これ**をキーにする（表記揺れを 1 つにまとめるため）
    pub key: String,
    /// 既存の種目に当たったか。`None` なら新規
    pub matched: Option<ExerciseId>,
    pub sets: Vec<SetEntry>,
}

/// 1 日分の読み取り結果。
#[derive(Debug, Clone, PartialEq)]
pub struct DraftDay {
    /// `None` = 日付が読めなかった塊。取り込み時に利用者が日付を 1 つ選ぶ
    pub date: Option<NaiveDate>,
    pub logs: Vec<DraftLog>,
    pub body_weight: Option<f32>,
}

/// 新しく作ることになる種目。画面で部位を選ばせるために並べる。
#[derive(Debug, Clone, PartialEq)]
pub struct NewName {
    pub key: String,
    pub display: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Draft {
    pub days: Vec<DraftDay>,
    pub new_names: Vec<NewName>,
    /// 解釈できなかった行。**画面に全文を出すこと**
    pub ignored: Vec<String>,
    /// lb を kg に換算したか。換算した事実は画面に出す
    pub converted_lb: bool,
}

impl Draft {
    pub fn is_empty(&self) -> bool {
        self.days.iter().all(|d| d.logs.is_empty())
    }

    /// 日付が読めなかった塊があるか。あれば画面で日付を選ばせる。
    pub fn has_undated(&self) -> bool {
        self.days
            .iter()
            .any(|d| d.date.is_none() && !d.logs.is_empty())
    }

    /// `(種目数, 日数, セット数)`。読み取り直後の要約に使う。
    pub fn counts(&self) -> (usize, usize, usize) {
        let mut keys: Vec<&str> = self
            .days
            .iter()
            .flat_map(|d| d.logs.iter().map(|l| l.key.as_str()))
            .collect();
        keys.sort_unstable();
        keys.dedup();
        let days = self.days.iter().filter(|d| !d.logs.is_empty()).count();
        let sets = self
            .days
            .iter()
            .flat_map(|d| d.logs.iter())
            .map(|l| l.sets.len())
            .sum();
        (keys.len(), days, sets)
    }
}

/// 新規種目の行き先。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Assign {
    Into(GroupId),
    /// 取り込まない
    Skip,
}

// ── 正規化 ──────────────────────────────────────────────────────────────────

/// 全角 → 半角、乗算記号の統一、空白の圧縮。
///
/// ★ **`ー`（長音符）には触らない。** ハイフンに寄せると `ショルダープレス` が
/// `ショルダ-プレス` になって、種目名の照合が全滅する。
fn normalize(line: &str) -> String {
    let mapped: String = line
        .chars()
        .map(|c| match c {
            '\u{3000}' | '\u{00a0}' => ' ',
            // 全角英数記号 → ASCII
            '\u{ff01}'..='\u{ff5e}' => char::from_u32(c as u32 - 0xfee0).unwrap_or(c),
            '×' | '✕' | '╳' | '✖' | '⨯' => 'x',
            '−' | '–' | '—' => '-',
            '、' => ',',
            '\u{338f}' => 'k', // ㎏ は "kg" に開けないのでここでは k だけ。下で潰す
            _ => c,
        })
        .collect();
    let mut out = String::with_capacity(mapped.len());
    let mut prev_space = false;
    for c in mapped.chars() {
        if c.is_whitespace() {
            if !prev_space {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.push(c);
            prev_space = false;
        }
    }
    out.trim().to_string()
}

/// 種目名の照合キー。表記揺れ（空白 / 中黒 / 括弧 / 大文字小文字）を潰す。
pub fn name_key(s: &str) -> String {
    normalize(s)
        .to_lowercase()
        .chars()
        .filter(|c| {
            !c.is_whitespace() && !matches!(c, '・' | '-' | '_' | '(' | ')' | '.' | ',' | '/')
        })
        .collect()
}

// ── 行の分類 ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Unit {
    Kg,
    Lb,
    Reps,
    Sec,
    Sets,
    None,
}

#[derive(Debug, Clone, PartialEq)]
enum Item {
    Num(f64, Unit),
    Mul,
    Sep,
    Word,
}

/// 数値の直後に置ける単位。**長いものから並べる**（`kgs` が `kg` に食われないように）。
///
/// ★ 1 文字の単位（`r` / `s`）は入れない。`60 squat` の `s` を秒として食う。
const UNITS: &[(&str, Unit)] = &[
    ("セット目", Unit::Sets),
    ("キロ", Unit::Kg),
    ("ポンド", Unit::Lb),
    ("セット", Unit::Sets),
    ("reps", Unit::Reps),
    ("sets", Unit::Sets),
    ("rep", Unit::Reps),
    ("set", Unit::Sets),
    ("sec", Unit::Sec),
    ("kgs", Unit::Kg),
    ("lbs", Unit::Lb),
    ("kg", Unit::Kg),
    ("lb", Unit::Lb),
    ("回", Unit::Reps),
    ("秒", Unit::Sec),
];

fn read_unit(cs: &[char]) -> (Unit, usize) {
    for (text, unit) in UNITS {
        let want: Vec<char> = text.chars().collect();
        if cs.len() >= want.len() && cs[..want.len()] == want[..] {
            return (*unit, want.len());
        }
    }
    (Unit::None, 0)
}

/// 行を「数・乗算記号・区切り・語」に割る。**小文字化した行**を渡すこと。
fn tokenize(cs: &[char]) -> Vec<Item> {
    let mut items = Vec::new();
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        if c.is_ascii_digit() {
            let start = i;
            while i < cs.len()
                && (cs[i].is_ascii_digit()
                    || (cs[i] == '.' && cs.get(i + 1).is_some_and(char::is_ascii_digit)))
            {
                i += 1;
            }
            let text: String = cs[start..i].iter().collect();
            let value: f64 = text.parse().unwrap_or(0.0);
            // 単位は空白 1 つまで挟んでよい（`60 kg`）
            let probe = if cs.get(i) == Some(&' ') { i + 1 } else { i };
            let (unit, taken) = read_unit(&cs[probe..]);
            if unit != Unit::None {
                i = probe + taken;
            }
            items.push(Item::Num(value, unit));
            continue;
        }
        match c {
            // `@` は「重量 @ 回数」の書き方があるので乗算記号として扱う
            'x' | '*' | '@' => {
                items.push(Item::Mul);
                i += 1;
            }
            ',' | '/' | '・' | '|' => {
                items.push(Item::Sep);
                i += 1;
            }
            ' ' => i += 1,
            _ => {
                let start = i;
                while i < cs.len()
                    && !cs[i].is_ascii_digit()
                    && !matches!(cs[i], 'x' | '*' | '@' | ',' | '/' | '・' | '|' | ' ')
                {
                    i += 1;
                }
                if i == start {
                    i += 1;
                }
                items.push(Item::Word);
            }
        }
    }
    items
}

/// 読み取ったセットと、lb 換算をしたか。
fn collect_sets(items: &[Item]) -> (Vec<(f64, f64)>, bool) {
    let mut out: Vec<(f64, f64)> = Vec::new();
    let mut used_lb = false;
    // 行内で引き継ぐ重量。`60kg 10,10,8` の 2 セット目以降がこれを使う
    let mut weight: Option<f64> = None;
    let mut i = 0;
    while i < items.len() {
        let Item::Num(value, unit) = items[i] else {
            i += 1;
            continue;
        };
        match unit {
            Unit::Kg | Unit::Lb => {
                let w = if unit == Unit::Lb {
                    used_lb = true;
                    value * LB_TO_KG
                } else {
                    value
                };
                weight = Some(w);
                let mut j = i + 1;
                while matches!(items.get(j), Some(Item::Mul) | Some(Item::Sep)) {
                    j += 1;
                }
                match items.get(j) {
                    Some(Item::Num(r, Unit::Reps | Unit::Sec | Unit::None)) => {
                        out.push((w, *r));
                        i = j + 1;
                    }
                    _ => i += 1,
                }
            }
            Unit::Reps | Unit::Sec => {
                out.push((weight.unwrap_or(0.0), value));
                i += 1;
            }
            // セット番号。`3セット目 60kg x 10` の 3
            Unit::Sets => i += 1,
            Unit::None => {
                let had_mul = matches!(items.get(i + 1), Some(Item::Mul));
                let mut j = i + 1;
                while matches!(items.get(j), Some(Item::Mul) | Some(Item::Sep)) {
                    j += 1;
                }
                let adjacent = j == i + 1;
                match items.get(j) {
                    Some(Item::Num(r, Unit::Reps | Unit::None)) if had_mul || adjacent => {
                        out.push((value, *r));
                        weight = Some(value);
                        i = j + 1;
                    }
                    _ => {
                        // 重量が確定済みなら回数として拾う（`60kg 10 8 8`）
                        if let Some(w) = weight {
                            out.push((w, value));
                        } else if i > 0 && matches!(items.get(i - 1), Some(Item::Mul)) {
                            // `自重 x 12` のように重量が書かれていない形
                            out.push((0.0, value));
                        }
                        i += 1;
                    }
                }
            }
        }
    }

    // ★ 回数が先に来る書き方（`10 reps 60 kg`）は、回数を読んだ時点で重量が未確定に
    //   なる。行の中に重量が 1 つでもあるなら、重量 0 のまま残ったセットに埋め戻す。
    //   自重種目の行には重量そのものが無いので、ここでは触られない
    let stated = items.iter().find_map(|it| match it {
        Item::Num(v, Unit::Kg) => Some(*v),
        Item::Num(v, Unit::Lb) => Some(*v * LB_TO_KG),
        _ => None,
    });
    if let Some(w) = stated {
        for pair in &mut out {
            if pair.0 == 0.0 {
                pair.0 = w;
            }
        }
    }
    (out, used_lb)
}

/// 1 行からセットを読む。読めなければ `None`。
///
/// ★ **数が 1 つだけの行はセットにしない。** 単独の `5` を「5 回」と読むと、
/// ページ番号や順位の行が全部セットになる。
fn parse_sets(items: &[Item]) -> Option<(Vec<SetEntry>, bool)> {
    let numbers = items
        .iter()
        .filter(|it| matches!(it, Item::Num(..)))
        .count();
    let has_unit = items.iter().any(|it| {
        matches!(
            it,
            Item::Num(_, Unit::Kg | Unit::Lb | Unit::Reps | Unit::Sec)
        )
    });
    let has_mul = items.iter().any(|it| matches!(it, Item::Mul));
    if numbers == 0 || (!has_unit && !has_mul && numbers < 2) {
        return None;
    }

    // 先頭の小さな裸数字はセット番号のことが多い（`1 60 x 10`）。落として読めるなら落とす
    let mut best = collect_sets(items);
    if items.len() >= 3
        && let Item::Num(n, Unit::None | Unit::Sets) = items[0]
        && n.fract() == 0.0
        && (1.0..=20.0).contains(&n)
    {
        let trimmed = collect_sets(&items[1..]);
        if !trimmed.0.is_empty() {
            best = trimmed;
        }
    }

    let (pairs, used_lb) = best;
    let sets: Vec<SetEntry> = pairs
        .into_iter()
        .filter(|(w, r)| (0.0..=1000.0).contains(w) && (1.0..=999.0).contains(r))
        .map(|(w, r)| SetEntry {
            // 0.1kg 刻みに丸める。lb 換算の端数がそのまま出ると読めない
            weight: ((w * 10.0).round() / 10.0) as f32,
            reps: r.round() as u32,
        })
        .collect();
    if sets.is_empty() {
        return None;
    }
    Some((sets, used_lb))
}

/// 集計行 / 見出し行。**セットとして読むと嘘の記録が増える**ので先に落とす。
const IGNORE_JP: &[&str] = &[
    "合計",
    "総重量",
    "ボリューム",
    "平均",
    "休憩",
    "インターバル",
    "トレーニング時間",
    "所要時間",
    "消費カロリー",
    "前回",
];

/// 英語は**行頭一致**で見る。部分一致にすると種目名を巻き込む。
const IGNORE_EN: &[&str] = &[
    "total", "volume", "average", "avg", "rest", "duration", "calories", "1rm", "est.", "weight",
    "reps", "sets", "set ", "previous", "notes",
];

fn is_noise(low: &str) -> bool {
    IGNORE_JP.iter().any(|k| low.contains(k)) || IGNORE_EN.iter().any(|k| low.starts_with(k))
}

// ── 日付 ────────────────────────────────────────────────────────────────────

const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];

fn digits(cs: &[char], i: usize) -> Option<(u32, usize)> {
    let mut j = i;
    while j < cs.len() && cs[j].is_ascii_digit() {
        j += 1;
    }
    if j == i || j - i > 4 {
        return None;
    }
    let text: String = cs[i..j].iter().collect();
    text.parse().ok().map(|v| (v, j))
}

/// 日付の直後は英数字であってはいけない。`8.7kg` を 8 月 7 日と読まないための境界。
fn boundary(cs: &[char], i: usize) -> bool {
    cs.get(i).is_none_or(|c| !c.is_alphanumeric())
}

/// 年の無い日付に年を当てる。**未来にしない。**
fn resolve_year(month: u32, day: u32, today: NaiveDate) -> Option<NaiveDate> {
    for year in [today.year(), today.year() - 1, today.year() - 2] {
        if let Some(d) = NaiveDate::from_ymd_opt(year, month, day)
            && d <= today
        {
            return Some(d);
        }
    }
    None
}

fn month_name(cs: &[char], i: usize) -> Option<(u32, usize)> {
    if cs.len() < i + 3 {
        return None;
    }
    let head: String = cs[i..i + 3].iter().collect();
    let index = MONTHS.iter().position(|m| *m == head)?;
    let mut j = i + 3;
    while j < cs.len() && cs[j].is_alphabetic() {
        j += 1;
    }
    Some((index as u32 + 1, j))
}

/// 曜日表記を読み飛ばす。`(木)` `土曜日` `thu` など。
fn skip_weekday(cs: &[char], mut i: usize) -> usize {
    while cs.get(i) == Some(&' ') {
        i += 1;
    }
    if cs.get(i) == Some(&'(')
        && let Some(close) = cs[i..].iter().position(|c| *c == ')')
        && close <= 5
    {
        return i + close + 1;
    }
    if matches!(
        cs.get(i),
        Some('月' | '火' | '水' | '木' | '金' | '土' | '日')
    ) {
        let mut j = i + 1;
        if cs.get(j) == Some(&'曜') {
            j += 1;
            if cs.get(j) == Some(&'日') {
                j += 1;
            }
            return j;
        }
        // 単独の漢字 1 文字は曜日か「日」の残り。行末なら曜日とみなす
        if boundary(cs, j) && cs.get(j).is_none() {
            return j;
        }
    }
    i
}

/// 行頭の日付を読む。返すのは `(日付, 消費した char 数)`。
fn parse_leading_date(cs: &[char], today: NaiveDate) -> Option<(NaiveDate, usize)> {
    // 今日 / 昨日
    for (word, back) in [
        ("今日", 0u32),
        ("today", 0),
        ("昨日", 1),
        ("yesterday", 1),
        ("一昨日", 2),
    ] {
        let want: Vec<char> = word.chars().collect();
        if cs.len() >= want.len() && cs[..want.len()] == want[..] && boundary(cs, want.len()) {
            let date = today - chrono::Duration::days(i64::from(back));
            return Some((date, want.len()));
        }
    }

    // 英語（Aug 7, 2026 / 7 Aug 2026 / Aug 7）
    if let Some((m, i)) = month_name(cs, 0) {
        let mut j = i;
        while cs.get(j) == Some(&' ') {
            j += 1;
        }
        if let Some((d, j2)) = digits(cs, j)
            && (1..=31).contains(&d)
        {
            let mut j3 = j2;
            if cs.get(j3) == Some(&',') {
                j3 += 1;
            }
            while cs.get(j3) == Some(&' ') {
                j3 += 1;
            }
            if let Some((y, j4)) = digits(cs, j3)
                && y >= 1000
                && boundary(cs, j4)
                && let Some(date) = NaiveDate::from_ymd_opt(y as i32, m, d)
            {
                return Some((date, j4));
            }
            if boundary(cs, j2) {
                return resolve_year(m, d, today).map(|date| (date, j2));
            }
        }
    }
    if let Some((d, i)) = digits(cs, 0)
        && (1..=31).contains(&d)
    {
        let mut j = i;
        while cs.get(j) == Some(&' ') {
            j += 1;
        }
        if let Some((m, j2)) = month_name(cs, j) {
            let mut j3 = j2;
            while cs.get(j3) == Some(&' ') {
                j3 += 1;
            }
            if let Some((y, j4)) = digits(cs, j3)
                && y >= 1000
                && boundary(cs, j4)
                && let Some(date) = NaiveDate::from_ymd_opt(y as i32, m, d)
            {
                return Some((date, j4));
            }
            if boundary(cs, j2) {
                return resolve_year(m, d, today).map(|date| (date, j2));
            }
        }
    }

    // 数字だけの日付
    let (head, i1) = digits(cs, 0)?;
    if head >= 1000 {
        // YYYY[/-.年]M[/-.月]D[日]
        let i2 = matches!(cs.get(i1), Some('/' | '-' | '.' | '年')).then_some(i1 + 1)?;
        let (m, i3) = digits(cs, i2)?;
        let i4 = matches!(cs.get(i3), Some('/' | '-' | '.' | '月')).then_some(i3 + 1)?;
        let (d, i5) = digits(cs, i4)?;
        let i6 = if cs.get(i5) == Some(&'日') {
            i5 + 1
        } else {
            i5
        };
        if !boundary(cs, i6) {
            return None;
        }
        return NaiveDate::from_ymd_opt(head as i32, m, d).map(|date| (date, i6));
    }
    if (1..=12).contains(&head) {
        // M[/月]D[日]。`-` は範囲表記と紛らわしいので受けない
        let i2 = matches!(cs.get(i1), Some('/' | '月')).then_some(i1 + 1)?;
        let (d, i3) = digits(cs, i2)?;
        let i4 = if cs.get(i3) == Some(&'日') {
            i3 + 1
        } else {
            i3
        };
        if (1..=31).contains(&d) && boundary(cs, i4) {
            return resolve_year(head, d, today).map(|date| (date, i4));
        }
    }
    None
}

// ── 体重 ────────────────────────────────────────────────────────────────────

fn parse_body_weight(low: &str, items: &[Item]) -> Option<f32> {
    let looks_like = low.contains("体重")
        || low.starts_with("body weight")
        || low.starts_with("bodyweight")
        || low.starts_with("weight");
    if !looks_like {
        return None;
    }
    items.iter().find_map(|it| match it {
        Item::Num(v, Unit::Kg | Unit::None) if (20.0..=300.0).contains(v) => {
            Some(((v * 10.0).round() / 10.0) as f32)
        }
        Item::Num(v, Unit::Lb) if (44.0..=660.0).contains(v) => {
            Some(((v * LB_TO_KG * 10.0).round() / 10.0) as f32)
        }
        _ => None,
    })
}

// ── 種目名 ──────────────────────────────────────────────────────────────────

/// 英語 / 別表記 → プリセットの日本語名。
///
/// ★ **ID ではなく名前に紐づける。** `presets.rs` の固定 ID をここにも書くと、
/// 片方だけ直したときに静かに別種目へ繋がる。改名済みの端末では当たらなくなるが、
/// そのときは画面で選び直せる。
const ALIASES: &[(&str, &str)] = &[
    ("benchpress", "ベンチプレス"),
    ("barbellbenchpress", "ベンチプレス"),
    ("flatbenchpress", "ベンチプレス"),
    ("ベンチ", "ベンチプレス"),
    ("dumbbellbenchpress", "ダンベルプレス"),
    ("dumbbellpress", "ダンベルプレス"),
    ("inclinebenchpress", "インクラインベンチプレス"),
    ("inclinebarbellbenchpress", "インクラインベンチプレス"),
    ("inclinepress", "インクラインベンチプレス"),
    ("chestfly", "チェストフライ"),
    ("dumbbellfly", "チェストフライ"),
    ("cablefly", "チェストフライ"),
    ("pecdeck", "チェストフライ"),
    ("ペックフライ", "チェストフライ"),
    ("pushup", "プッシュアップ"),
    ("pushups", "プッシュアップ"),
    ("腕立て伏せ", "プッシュアップ"),
    ("腕立て", "プッシュアップ"),
    ("pullup", "懸垂"),
    ("pullups", "懸垂"),
    ("chinup", "懸垂"),
    ("chinups", "懸垂"),
    ("チンニング", "懸垂"),
    ("latpulldown", "ラットプルダウン"),
    ("latpull", "ラットプルダウン"),
    ("barbellrow", "ベントオーバーロウ"),
    ("bentoverrow", "ベントオーバーロウ"),
    ("ベントオーバーロー", "ベントオーバーロウ"),
    ("seatedrow", "シーテッドロウ"),
    ("cablerow", "シーテッドロウ"),
    ("シーテッドロー", "シーテッドロウ"),
    ("deadlift", "デッドリフト"),
    ("shoulderpress", "ショルダープレス"),
    ("overheadpress", "ショルダープレス"),
    ("militarypress", "ショルダープレス"),
    ("ohp", "ショルダープレス"),
    ("lateralraise", "サイドレイズ"),
    ("sideraise", "サイドレイズ"),
    ("frontraise", "フロントレイズ"),
    ("rearraise", "リアレイズ"),
    ("reardeltfly", "リアレイズ"),
    ("リアデルトフライ", "リアレイズ"),
    ("barbellcurl", "バーベルカール"),
    ("ezbarcurl", "バーベルカール"),
    ("dumbbellcurl", "ダンベルカール"),
    ("bicepcurl", "ダンベルカール"),
    ("bicepscurl", "ダンベルカール"),
    ("tricepextension", "トライセプスエクステンション"),
    ("tricepsextension", "トライセプスエクステンション"),
    ("skullcrusher", "トライセプスエクステンション"),
    ("フレンチプレス", "トライセプスエクステンション"),
    ("triceppushdown", "ケーブルプレスダウン"),
    ("tricepspushdown", "ケーブルプレスダウン"),
    ("cablepushdown", "ケーブルプレスダウン"),
    ("pushdown", "ケーブルプレスダウン"),
    ("プレスダウン", "ケーブルプレスダウン"),
    ("dip", "ディップス"),
    ("dips", "ディップス"),
    ("squat", "スクワット"),
    ("barbellsquat", "スクワット"),
    ("backsquat", "スクワット"),
    ("legpress", "レッグプレス"),
    ("legextension", "レッグエクステンション"),
    ("legcurl", "レッグカール"),
    ("lyinglegcurl", "レッグカール"),
    ("calfraise", "カーフレイズ"),
    ("standingcalfraise", "カーフレイズ"),
    ("plank", "プランク"),
    ("sideplank", "サイドプランク"),
    ("crunch", "クランチ"),
    ("crunches", "クランチ"),
    ("腹筋", "クランチ"),
    ("legraise", "レッグレイズ"),
    ("legraises", "レッグレイズ"),
    ("hanginglegraise", "レッグレイズ"),
];

/// 照合キー → 既存の種目。アーカイブ済みも見る（過去の記録が繋がるので）。
fn match_exercise(key: &str, db: &Db) -> Option<ExerciseId> {
    if let Some(found) = db.exercises.iter().find(|e| name_key(&e.name) == key) {
        return Some(found.id);
    }
    let (_, jp) = ALIASES.iter().find(|(alias, _)| *alias == key)?;
    let jp_key = name_key(jp);
    db.exercises
        .iter()
        .find(|e| name_key(&e.name) == jp_key)
        .map(|e| e.id)
}

/// 種目名として通す条件。
///
/// 数字が半分以上を占める行は弾く。`60 kg` のような読み損ねたセット行が
/// 種目として登録されるのを止める。
fn looks_like_name(s: &str) -> bool {
    let chars: Vec<char> = s.chars().filter(|c| !c.is_whitespace()).collect();
    if chars.is_empty() || chars.len() > 40 {
        return false;
    }
    if !chars.iter().any(|c| c.is_alphabetic()) {
        return false;
    }
    let digits = chars.iter().filter(|c| c.is_ascii_digit()).count();
    digits * 2 < chars.len()
}

// ── 読み取り ────────────────────────────────────────────────────────────────

enum Line {
    Date(NaiveDate),
    BodyWeight(f32),
    Sets(Vec<SetEntry>, bool),
    Name(String),
    /// 読み取れなかった文。**そのまま画面に出す**ので、日付だけ消費した行では
    /// 残り（`8/7 胸の日` の「胸の日」）だけを持つ
    Ignored(String),
}

fn classify(norm: &str, today: NaiveDate) -> Vec<Line> {
    if norm.is_empty() {
        return vec![Line::Ignored(String::new())];
    }
    let low = norm.to_lowercase();
    let low_chars: Vec<char> = low.chars().collect();
    let norm_chars: Vec<char> = norm.chars().collect();

    if let Some((date, used)) = parse_leading_date(&low_chars, today) {
        let after = skip_weekday(&low_chars, used);
        let rest: String = norm_chars
            .get(after..)
            .map(|cs| cs.iter().collect::<String>())
            .unwrap_or_default();
        let rest = rest.trim();
        if rest.is_empty() {
            return vec![Line::Date(date)];
        }
        // 日付の後ろに続くのはセットか体重のときだけ拾う。種目名として拾うと
        // 「8/7 胸の日」の「胸の日」が種目になる
        let mut out = vec![Line::Date(date)];
        let rest_low = rest.to_lowercase();
        let items = tokenize(&rest_low.chars().collect::<Vec<_>>());
        if let Some(w) = parse_body_weight(&rest_low, &items) {
            out.push(Line::BodyWeight(w));
        } else if !is_noise(&rest_low) {
            match parse_sets(&items) {
                Some((sets, lb)) => out.push(Line::Sets(sets, lb)),
                None => out.push(Line::Ignored(rest.to_string())),
            }
        }
        return out;
    }

    let items = tokenize(&low_chars);
    if let Some(w) = parse_body_weight(&low, &items) {
        return vec![Line::BodyWeight(w)];
    }
    if is_noise(&low) {
        return vec![Line::Ignored(norm.to_string())];
    }
    // 時刻（`10:30`）をセットとして読まない
    if low.contains(':') && !low.contains("kg") && !low.contains('回') {
        return vec![Line::Ignored(norm.to_string())];
    }
    if let Some((sets, lb)) = parse_sets(&items) {
        return vec![Line::Sets(sets, lb)];
    }
    if looks_like_name(norm) {
        return vec![Line::Name(norm.to_string())];
    }
    vec![Line::Ignored(norm.to_string())]
}

/// 溜めている「その種目のひとかたまり」。
///
/// 行ごとに直接 [`DraftDay`] へ足すと、同じスクショを 2 回貼ったときに気づけない
/// （足す側は毎回 1 セットしか持たないので、既にあるセット列と比べようがない）。
struct Block {
    day: usize,
    name: String,
    sets: Vec<SetEntry>,
}

/// `haystack` の中に `needle` がそのまま並んで入っているか。
fn contains_run(haystack: &[SetEntry], needle: &[SetEntry]) -> bool {
    needle.is_empty()
        || haystack.len() >= needle.len() && haystack.windows(needle.len()).any(|w| w == needle)
}

/// 貼り付けられたテキスト → [`Draft`]。
pub fn parse(raw: &str, db: &Db, today: NaiveDate) -> Draft {
    let mut draft = Draft::default();
    let mut current_date: Option<NaiveDate> = None;
    // 直近の種目名と、その名前がセットを 1 つでも受け取ったか
    let mut current_name: Option<(String, bool)> = None;
    let mut block: Option<Block> = None;

    for original in raw.lines() {
        let norm = normalize(original);
        if norm.is_empty() {
            continue;
        }
        for line in classify(&norm, today) {
            match line {
                Line::Date(date) => {
                    flush(&mut draft, &mut block, db);
                    drop_unused_name(&mut draft, &mut current_name);
                    current_date = Some(date);
                    ensure_day(&mut draft, Some(date));
                }
                Line::BodyWeight(w) => {
                    let day = ensure_day(&mut draft, current_date);
                    // 先に出たほうを残す（サマリと詳細で 2 回出るアプリがある）
                    day.body_weight.get_or_insert(w);
                }
                Line::Sets(sets, lb) => {
                    draft.converted_lb |= lb;
                    let Some((name, used)) = current_name.as_mut() else {
                        draft.ignored.push(norm.clone());
                        continue;
                    };
                    *used = true;
                    let name = name.clone();
                    let day = ensure_day_index(&mut draft, current_date);
                    match block.as_mut() {
                        Some(b) if b.day == day && b.name == name => b.sets.extend(sets),
                        _ => {
                            flush(&mut draft, &mut block, db);
                            block = Some(Block { day, name, sets });
                        }
                    }
                }
                Line::Name(name) => {
                    flush(&mut draft, &mut block, db);
                    drop_unused_name(&mut draft, &mut current_name);
                    current_name = Some((name, false));
                }
                // 空文字は「空行」なので画面に出さない
                Line::Ignored(text) if text.is_empty() => {}
                Line::Ignored(text) => draft.ignored.push(text),
            }
        }
    }
    flush(&mut draft, &mut block, db);
    drop_unused_name(&mut draft, &mut current_name);

    // 何も入らなかった日は捨てる（日付見出しだけの行が並ぶスクショで空の日が量産される）
    draft
        .days
        .retain(|d| !d.logs.is_empty() || d.body_weight.is_some());

    let mut seen: Vec<String> = Vec::new();
    for day in &draft.days {
        for log in &day.logs {
            if log.matched.is_none() && !seen.contains(&log.key) {
                seen.push(log.key.clone());
                draft.new_names.push(NewName {
                    key: log.key.clone(),
                    display: log.raw_name.clone(),
                });
            }
        }
    }
    draft
}

fn ensure_day_index(draft: &mut Draft, date: Option<NaiveDate>) -> usize {
    if let Some(index) = draft.days.iter().position(|d| d.date == date) {
        return index;
    }
    draft.days.push(DraftDay {
        date,
        logs: Vec::new(),
        body_weight: None,
    });
    draft.days.len() - 1
}

fn ensure_day(draft: &mut Draft, date: Option<NaiveDate>) -> &mut DraftDay {
    let index = ensure_day_index(draft, date);
    &mut draft.days[index]
}

/// 溜めていたかたまりを日に落とす。
fn flush(draft: &mut Draft, block: &mut Option<Block>, db: &Db) {
    let Some(Block { day, name, sets }) = block.take() else {
        return;
    };
    let key = name_key(&name);
    let matched = match_exercise(&key, db);
    match draft.days[day].logs.iter_mut().find(|l| l.key == key) {
        // ★ 同じスクショを 2 回貼ってもセットを倍にしない。かたまりごと既に
        //   入っているなら捨てる。別のセット列なら（分割スクショなので）足す
        Some(log) if contains_run(&log.sets, &sets) => {}
        Some(log) => log.sets.extend(sets),
        None => draft.days[day].logs.push(DraftLog {
            raw_name: name,
            key,
            matched,
            sets,
        }),
    }
}

/// セットが 1 つも続かなかった種目名は、読み取れなかった行として画面に出す。
///
/// 挨拶文や見出しがここに落ちる。**黙って捨てない**ための出口。
fn drop_unused_name(draft: &mut Draft, current: &mut Option<(String, bool)>) {
    if let Some((name, false)) = current.take() {
        draft.ignored.push(name);
    }
}

// ── Db への変換 ─────────────────────────────────────────────────────────────

/// [`Draft`] → 取り込む `Db`。**`core::merge_db` に渡す前提**（追加のみ）。
///
/// `assign` は新規種目の行き先（[`Draft::new_names`] の `key` で引く）。
/// `fallback` は日付が読めなかった塊を載せる日。
pub fn to_db(
    draft: &Draft,
    assign: &BTreeMap<String, Assign>,
    fallback: NaiveDate,
    db: &Db,
    ids: &mut IdGen,
) -> Db {
    let mut out = Db::default();
    // key → 使う ExerciseId
    let mut resolved: BTreeMap<String, ExerciseId> = BTreeMap::new();

    for day in &draft.days {
        for log in &day.logs {
            if resolved.contains_key(&log.key) {
                continue;
            }
            if let Some(id) = log.matched {
                resolved.insert(log.key.clone(), id);
                // 既存の種目は**取り込み先の名前のまま**入れる。読み取った表記
                // （`Bench Press`）を入れると merge_db が改名として報告する
                if let Some(existing) = db.exercise(id)
                    && !out.exercises.iter().any(|e| e.id == id)
                {
                    out.exercises.push(existing.clone());
                }
                continue;
            }
            match assign.get(&log.key) {
                Some(Assign::Into(group_id)) => {
                    let id: ExerciseId = ids.alloc();
                    resolved.insert(log.key.clone(), id);
                    out.exercises.push(Exercise {
                        id,
                        name: log.raw_name.clone(),
                        group_id: *group_id,
                        // merge_db が取り込み先の並びで振り直すので 0 でよい
                        order: 0,
                        archived: false,
                    });
                }
                // 未割り当ては取り込まない（画面で部位を選ばなかった種目）
                Some(Assign::Skip) | None => {}
            }
        }
    }

    // 参照している部位を連れていく。merge_db は ID 一致で素通りするので副作用は無い
    for exercise in &out.exercises {
        if out.groups.iter().any(|g| g.id == exercise.group_id) {
            continue;
        }
        if let Some(group) = db.group(exercise.group_id) {
            out.groups.push(group.clone());
        }
    }

    for day in &draft.days {
        let date = day.date.unwrap_or(fallback);
        let key = date_key(date);
        let session = out.sessions.entry(key).or_default();
        if let Some(w) = day.body_weight {
            session.body_weight.get_or_insert(w);
        }
        for log in &day.logs {
            let Some(id) = resolved.get(&log.key).copied() else {
                continue;
            };
            match session.logs.iter_mut().find(|l| l.exercise_id == id) {
                Some(existing) if existing.sets == log.sets => {}
                Some(existing) => existing.sets.extend(log.sets.iter().copied()),
                None => session.logs.push(ExerciseLog {
                    exercise_id: id,
                    sets: log.sets.clone(),
                    // ★ 過去日のバックフィル扱い（adr/data-model/at-optional-same-day-only.md）
                    at: None,
                }),
            }
        }
    }
    out.sessions.retain(|_, s: &mut Session| !s.is_empty());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presets;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 8, 9).expect("固定日")
    }

    fn db() -> Db {
        presets::seeded_db()
    }

    fn ids() -> IdGen {
        IdGen::from_seed(42)
    }

    fn sets(pairs: &[(f32, u32)]) -> Vec<SetEntry> {
        pairs
            .iter()
            .map(|(weight, reps)| SetEntry {
                weight: *weight,
                reps: *reps,
            })
            .collect()
    }

    /// その日のその種目のセット。
    fn sets_on(draft: &Draft, date: Option<NaiveDate>, name: &str) -> Vec<SetEntry> {
        draft
            .days
            .iter()
            .find(|d| d.date == date)
            .and_then(|d| d.logs.iter().find(|l| l.key == name_key(name)))
            .map(|l| l.sets.clone())
            .unwrap_or_default()
    }

    #[test]
    fn reads_a_japanese_app_screen() {
        let text = "\
2026年8月7日(木)
ベンチプレス
1 60kg × 10
2 60kg × 8
3 55kg × 8
ラットプルダウン
1 50kg × 12";
        let draft = parse(text, &db(), today());
        let date = NaiveDate::from_ymd_opt(2026, 8, 7);

        assert_eq!(
            sets_on(&draft, date, "ベンチプレス"),
            sets(&[(60.0, 10), (60.0, 8), (55.0, 8)])
        );
        assert_eq!(
            sets_on(&draft, date, "ラットプルダウン"),
            sets(&[(50.0, 12)])
        );
        assert!(draft.new_names.is_empty(), "全部プリセットに当たるはず");
        assert!(!draft.has_undated());
    }

    #[test]
    fn reads_an_english_app_screen() {
        let text = "\
Aug 7, 2026
Bench Press
1  60 kg x 10
2  60 kg x 8
Lat Pulldown
1  50 kg x 12";
        let draft = parse(text, &db(), today());
        let date = NaiveDate::from_ymd_opt(2026, 8, 7);

        // 英語名がプリセットの日本語名に寄る
        assert_eq!(
            sets_on(&draft, date, "benchpress"),
            sets(&[(60.0, 10), (60.0, 8)])
        );
        assert!(
            draft.new_names.is_empty(),
            "別名表で当たるはず: {:?}",
            draft.new_names
        );
    }

    #[test]
    fn reads_several_sets_from_one_line() {
        let text = "8/7\nスクワット\n80kg 10, 10, 8";
        let draft = parse(text, &db(), today());
        assert_eq!(
            sets_on(&draft, NaiveDate::from_ymd_opt(2026, 8, 7), "スクワット"),
            sets(&[(80.0, 10), (80.0, 10), (80.0, 8)])
        );
    }

    #[test]
    fn full_width_text_reads_the_same_as_half_width() {
        let wide = parse("８月７日\nベンチプレス\n１ ６０ｋｇ×１０", &db(), today());
        let narrow = parse("8月7日\nベンチプレス\n1 60kg x 10", &db(), today());
        assert_eq!(wide, narrow);
    }

    #[test]
    fn a_year_less_date_never_lands_in_the_future() {
        // 1 月に 12 月のテキストを読む
        let jan = NaiveDate::from_ymd_opt(2026, 1, 5).expect("固定日");
        let draft = parse("12/28\nベンチプレス\n60kg x 10", &db(), jan);
        assert_eq!(
            draft.days[0].date,
            NaiveDate::from_ymd_opt(2025, 12, 28),
            "未来の日付を作ってはいけない"
        );
    }

    #[test]
    fn summary_lines_do_not_become_sets() {
        let text = "\
8/7
ベンチプレス
60kg x 10
合計 3,600kg
トレーニング時間 58分
Total Volume 12,000 kg";
        let draft = parse(text, &db(), today());
        assert_eq!(
            sets_on(&draft, NaiveDate::from_ymd_opt(2026, 8, 7), "ベンチプレス"),
            sets(&[(60.0, 10)])
        );
        assert_eq!(draft.counts().2, 1, "集計行がセットとして混ざっている");
    }

    #[test]
    fn body_weight_is_not_read_as_a_set() {
        let draft = parse("8/7\n体重 72.5kg\nベンチプレス\n60kg x 10", &db(), today());
        let day = &draft.days[0];
        assert_eq!(day.body_weight, Some(72.5));
        assert_eq!(day.logs.len(), 1, "体重が種目になっている");
        assert_eq!(day.logs[0].sets, sets(&[(60.0, 10)]));
    }

    #[test]
    fn bodyweight_only_sets_get_weight_zero() {
        let draft = parse("8/7\n懸垂\n12回\n10回", &db(), today());
        assert_eq!(
            sets_on(&draft, NaiveDate::from_ymd_opt(2026, 8, 7), "懸垂"),
            sets(&[(0.0, 12), (0.0, 10)])
        );
    }

    #[test]
    fn seconds_become_reps_so_volume_is_total_seconds() {
        let draft = parse("8/7\nプランク\n60秒\n45秒", &db(), today());
        assert_eq!(
            sets_on(&draft, NaiveDate::from_ymd_opt(2026, 8, 7), "プランク"),
            sets(&[(0.0, 60), (0.0, 45)])
        );
    }

    #[test]
    fn pounds_are_converted_and_reported() {
        let draft = parse("8/7\nBench Press\n135 lb x 10", &db(), today());
        assert!(draft.converted_lb, "換算した事実を伝えないと重量が嘘になる");
        let got = sets_on(&draft, NaiveDate::from_ymd_opt(2026, 8, 7), "benchpress");
        assert_eq!(got.len(), 1);
        assert!((got[0].weight - 61.2).abs() < 0.05, "{got:?}");
    }

    #[test]
    fn unreadable_lines_are_kept_for_the_screen() {
        let draft = parse(
            "8/7\nベンチプレス\n60kg x 10\n★★★ おつかれさま！",
            &db(),
            today(),
        );
        assert!(
            draft.ignored.iter().any(|l| l.contains("おつかれさま")),
            "捨てた行を黙らせてはいけない: {:?}",
            draft.ignored
        );
    }

    #[test]
    fn sets_without_an_exercise_go_to_ignored() {
        let draft = parse("8/7\n60kg x 10", &db(), today());
        assert!(draft.is_empty());
        assert_eq!(draft.ignored, vec!["60kg x 10".to_string()]);
    }

    #[test]
    fn a_new_exercise_is_listed_for_assignment() {
        let draft = parse("8/7\nヒップスラスト\n100kg x 10", &db(), today());
        assert_eq!(draft.new_names.len(), 1);
        assert_eq!(draft.new_names[0].display, "ヒップスラスト");

        // 割り当てないと取り込まれない
        let empty = to_db(&draft, &BTreeMap::new(), today(), &db(), &mut ids());
        assert!(empty.sessions.is_empty());

        let mut assign = BTreeMap::new();
        let hip = presets::preset_group_id("脚").expect("プリセットの部位");
        assign.insert(draft.new_names[0].key.clone(), Assign::Into(hip));
        let built = to_db(&draft, &assign, today(), &db(), &mut ids());
        assert_eq!(built.exercises.len(), 1);
        assert_eq!(built.exercises[0].name, "ヒップスラスト");
        assert_eq!(built.exercises[0].group_id, hip);
        assert_eq!(built.sessions.len(), 1);
    }

    #[test]
    fn undated_records_land_on_the_fallback_date() {
        let draft = parse("ベンチプレス\n60kg x 10", &db(), today());
        assert!(draft.has_undated());
        let fallback = NaiveDate::from_ymd_opt(2026, 8, 5).expect("固定日");
        let built = to_db(&draft, &BTreeMap::new(), fallback, &db(), &mut ids());
        assert_eq!(built.sessions.keys().collect::<Vec<_>>(), ["2026-08-05"]);
    }

    #[test]
    fn imported_logs_have_no_timestamp() {
        // `at` に now を書くと「最後のトレーニングから」が嘘になる（adr/data-model/at-optional-same-day-only.md）
        let draft = parse("8/7\nベンチプレス\n60kg x 10", &db(), today());
        let built = to_db(&draft, &BTreeMap::new(), today(), &db(), &mut ids());
        for session in built.sessions.values() {
            for log in &session.logs {
                assert_eq!(log.at, None);
            }
        }
    }

    #[test]
    fn merging_the_same_text_twice_does_not_double_the_sets() {
        let text = "8/7\nベンチプレス\n60kg x 10\n60kg x 8";
        let draft = parse(text, &db(), today());

        let mut mine = db();
        let first = crate::core::merge_db(
            &mut mine,
            to_db(&draft, &BTreeMap::new(), today(), &db(), &mut ids()),
        );
        assert_eq!(first.logs_added, 1);

        let second = to_db(&draft, &BTreeMap::new(), today(), &mine, &mut ids());
        let again = crate::core::merge_db(&mut mine, second);
        assert!(again.is_noop(), "2 回目で増えている: {again:?}");
        let session = mine.sessions.get("2026-08-07").expect("取り込んだ日");
        assert_eq!(session.logs[0].sets, sets(&[(60.0, 10), (60.0, 8)]));
    }

    #[test]
    fn the_same_screenshot_pasted_twice_is_not_doubled() {
        let once = "8/7\nベンチプレス\n60kg x 10\n60kg x 8";
        let twice = format!("{once}\n{once}");
        assert_eq!(
            parse(&twice, &db(), today()).counts().2,
            parse(once, &db(), today()).counts().2
        );
    }

    #[test]
    fn a_renamed_exercise_still_matches_by_its_current_name() {
        let mut mine = db();
        let id = presets::preset_exercise_id("ベンチプレス").expect("プリセット");
        mine.exercises
            .iter_mut()
            .find(|e| e.id == id)
            .expect("居る")
            .name = "BP".to_string();

        let draft = parse("8/7\nBP\n60kg x 10", &mine, today());
        assert_eq!(draft.days[0].logs[0].matched, Some(id));
    }

    #[test]
    fn date_like_weights_are_not_read_as_dates() {
        // `8.7kg` を 8 月 7 日と読むと、その行のセットが日付見出しに化ける
        let draft = parse("8/7\nサイドレイズ\n8.7kg x 12", &db(), today());
        assert_eq!(
            sets_on(&draft, NaiveDate::from_ymd_opt(2026, 8, 7), "サイドレイズ"),
            sets(&[(8.7, 12)])
        );
        assert_eq!(draft.days.len(), 1);
    }

    #[test]
    fn a_clock_time_is_not_read_as_a_set() {
        let draft = parse("8/7\nベンチプレス\n10:30\n60kg x 10", &db(), today());
        assert_eq!(
            sets_on(&draft, NaiveDate::from_ymd_opt(2026, 8, 7), "ベンチプレス"),
            sets(&[(60.0, 10)])
        );
    }

    #[test]
    fn a_date_line_with_a_title_keeps_only_the_date() {
        let draft = parse("8/7 胸の日\nベンチプレス\n60kg x 10", &db(), today());
        assert_eq!(draft.days.len(), 1);
        assert_eq!(draft.days[0].date, NaiveDate::from_ymd_opt(2026, 8, 7));
        assert_eq!(draft.days[0].logs.len(), 1, "「胸の日」が種目になっている");
    }

    #[test]
    fn several_days_are_kept_apart() {
        let text = "\
8/5
ベンチプレス
60kg x 10
8/7
ベンチプレス
62.5kg x 8";
        let draft = parse(text, &db(), today());
        assert_eq!(
            sets_on(&draft, NaiveDate::from_ymd_opt(2026, 8, 5), "ベンチプレス"),
            sets(&[(60.0, 10)])
        );
        assert_eq!(
            sets_on(&draft, NaiveDate::from_ymd_opt(2026, 8, 7), "ベンチプレス"),
            sets(&[(62.5, 8)])
        );
    }

    #[test]
    fn reps_first_layout_is_understood() {
        let draft = parse("8/7\nベンチプレス\n10 reps 60 kg", &db(), today());
        assert_eq!(
            sets_on(&draft, NaiveDate::from_ymd_opt(2026, 8, 7), "ベンチプレス"),
            sets(&[(60.0, 10)])
        );
    }

    #[test]
    fn a_bare_pair_without_units_is_understood() {
        let draft = parse("8/7\nベンチプレス\n60 10", &db(), today());
        assert_eq!(
            sets_on(&draft, NaiveDate::from_ymd_opt(2026, 8, 7), "ベンチプレス"),
            sets(&[(60.0, 10)])
        );
    }

    #[test]
    fn empty_input_reads_as_nothing() {
        let draft = parse("   \n\n ", &db(), today());
        assert!(draft.is_empty());
        assert!(draft.ignored.is_empty());
        assert_eq!(draft.counts(), (0, 0, 0));
    }
}
