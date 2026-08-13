//! 表計算（CSV / TSV）との受け渡し。**`leptos` / `web-sys` を一切 import しない。**
//!
//! `cargo test`（ホストターゲット）で検証する層。ネットワークは触らない —
//! Google スプレッドシートから本文を取ってくるのは `transfer::fetch_text` の仕事で、
//! ここは「貼られた URL を CSV の URL に直す」純関数（[`csv_url`]）までを持つ。
//!
//! 設計上の要点:
//!
//! - **CSV は二次形式で、正は JSON。** ID / 色 / 並び順 / `archived` を持たないので
//!   往復でそれらは復元されない。だから取り込みは「足すだけ」に固定する
//!   （adr/storage/csv-as-a-secondary-lossy-format.md）
//! - **列はヘッダ名で解決する。位置に依存しない。** 書き出したファイルは
//!   利用者の手元に残り続けるので、列を足す / 並べ替える自由を最初から確保しておく
//! - **人に何も聞かない。** 撤去された `import_text.rs` は「未知の種目ごとに部位を
//!   選ばせる」操作量で死んだ（adr/ux/migrate-by-ocr-paste.md）。ここでは未知の種目は
//!   部位列を見て自動で作り、部位が分からない行は**件数を報告して**落とす
//! - **`at` は書かない。** 取り込みは過去日のバックフィルなので時刻を持たない
//!   （adr/data-model/at-optional-same-day-only.md）

use std::collections::{BTreeMap, HashMap};

use chrono::NaiveDate;

use crate::core::{self, date_key};
use crate::model::{
    Db, Exercise, ExerciseId, ExerciseLog, GroupId, IdGen, SCHEMA, Session, SetEntry,
};
use crate::presets;

// ── 列 ──────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
enum Col {
    Date,
    Group,
    Exercise,
    SetNo,
    Weight,
    Reps,
    SetNote,
    ExerciseNote,
    BodyWeight,
    DayNote,
}

/// 書き出す列と、その並び。**この順序と名前は書き出したファイルの一部として残り続ける。**
///
/// 足すのは末尾だけにする。取り込み側は名前で引くので並べ替えても読めるが、
/// 既存の名前を変えると、利用者の手元にある古いファイルが読めなくなる。
const COLUMNS: [(Col, &str); 10] = [
    (Col::Date, "日付"),
    (Col::Group, "部位"),
    (Col::Exercise, "種目"),
    (Col::SetNo, "セット"),
    (Col::Weight, "重量kg"),
    (Col::Reps, "回数"),
    (Col::SetNote, "セットメモ"),
    (Col::ExerciseNote, "種目メモ"),
    (Col::BodyWeight, "体重kg"),
    (Col::DayNote, "当日メモ"),
];

/// これが揃わない表は読まない。`部位` は**未知の種目が来たときだけ**必要なので入れない。
const REQUIRED: [(Col, &str); 4] = [
    (Col::Date, "日付"),
    (Col::Exercise, "種目"),
    (Col::Weight, "重量kg"),
    (Col::Reps, "回数"),
];

/// ヘッダ名 → 列。**別名は「他所で作った表を読む」ためのもの**で、自前の書き出しは
/// 常に [`COLUMNS`] の名前を出す。
fn col_of(header: &str) -> Option<Col> {
    match normalize_header(header).as_str() {
        "日付" | "日づけ" | "年月日" | "date" | "day" => Some(Col::Date),
        "部位" | "カテゴリ" | "group" | "category" | "bodypart" => Some(Col::Group),
        "種目" | "種目名" | "メニュー" | "exercise" | "menu" => Some(Col::Exercise),
        "セット" | "セット番号" | "セットno" | "set" | "setno" => Some(Col::SetNo),
        "重量kg" | "重量" | "kg" | "weight" | "weightkg" => Some(Col::Weight),
        "回数" | "レップ" | "回" | "reps" | "rep" => Some(Col::Reps),
        // ★ 素の「メモ」は**セット**側に寄せる。日単位に寄せると「その日で最初に
        //   現れた非空の値」の規則で 2 行目以降が黙って捨てられる —— 手で作った表の
        //   メモ列は 1 行 1 メモなので、120 行なら 119 個が消える
        "セットメモ" | "メモ" | "setnote" | "setmemo" | "note" | "memo" => {
            Some(Col::SetNote)
        }
        "種目メモ" | "exercisenote" | "exercisememo" => Some(Col::ExerciseNote),
        "体重kg" | "体重" | "bodyweight" | "bodyweightkg" => Some(Col::BodyWeight),
        "当日メモ" | "日メモ" | "daynote" => Some(Col::DayNote),
        _ => None,
    }
}

// ── 表記揺れの正規化 ─────────────────────────────────────────────────────────

/// 全角 ASCII を半角に落とす。`１` → `1`、`．` → `.`、`ｋ` → `k`。
fn to_halfwidth(c: char) -> char {
    match c as u32 {
        0xff01..=0xff5e => char::from_u32(c as u32 - 0xfee0).unwrap_or(c),
        _ => c,
    }
}

/// 名前の突き合わせキー。**潰すのは空白・中黒・全角半角・大文字小文字だけ。**
///
/// ★ これ以上寄せてはいけない。「レッグカール」と「レッグエクステンション」のような
/// 近い名前を誤って同一視すると、**間違って繋がった履歴は間違いに気づけない**
/// （グラフが滑らかに繋がってしまう）。編集距離で寄せる案が却下されたのと同じ理由。
fn normalize_key(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace() && *c != '・')
        .map(to_halfwidth)
        .flat_map(char::to_lowercase)
        .collect()
}

/// ヘッダ専用。名前より強く潰す（列名の取り違えは履歴を壊さないので安全側に倒せる）。
fn normalize_header(s: &str) -> String {
    normalize_key(s)
        .chars()
        .filter(|c| !matches!(c, '(' | ')' | '[' | ']' | '/' | '_' | '-' | '.'))
        .collect()
}

/// 手書きの表に付く単位を落とす。`60kg` → `60`、`10回` → `10`。
fn strip_unit(s: &str) -> &str {
    for u in ["kg", "KG", "Kg", "kG", "キロ", "reps", "rep", "回目", "回"] {
        if let Some(rest) = s.strip_suffix(u) {
            return rest.trim();
        }
    }
    s
}

/// 数値セル。**カンマは落とさない。** `60,5`（小数点にカンマを使う書式）を 605 に
/// 化けさせるより、読めないと言って行を報告するほうが良い。
fn parse_num(s: &str) -> Option<f64> {
    let t: String = s.trim().chars().map(to_halfwidth).collect();
    let t = strip_unit(t.trim());
    if t.is_empty() {
        return None;
    }
    let v = t.parse::<f64>().ok()?;
    v.is_finite().then_some(v)
}

/// 日付セル。**年が先頭の 4 桁である形しか受けない。**
///
/// Google スプレッドシートの `/export` は**表示値**を返すので、`2026-08-13` と書いても
/// 日本語ロケールでは `2026/08/13` で出てくる。だから区切りは複数受ける。
///
/// ★ 一方で `08/13/2026` と `13/08/2026` は**どちらの解釈も成り立つ**。推測して当たれば
/// 何も起きず、外すと記録が半年ずれた日に静かに入る。取り込めない行として報告する。
fn parse_date(s: &str) -> Option<NaiveDate> {
    // ★ 日付**時刻**の書式（`2026/08/01 0:00:00`、`2026-08-01T12:00`）を落とす。
    //   別のアプリから貼った列や `NOW()` の入った列は日時になっていることがあり、
    //   ここで弾くと表が丸ごと 1 行も読めなくなる。
    //   `:` か `T` があるときだけ切るので、`2026 / 08 / 01` のような空白入りは壊さない
    let s = s.trim();
    let s = if s.contains(':') || s.contains('T') {
        s.split(['T', ' ', '\u{3000}']).next().unwrap_or(s)
    } else {
        s
    };
    let t: String = s
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(to_halfwidth)
        .collect();
    let t = t.replace(['年', '月'], "-");
    let t = t.trim_end_matches('日').trim_end_matches('-');
    let t = t.replace(['/', '.'], "-");

    let mut parts = t.split('-');
    let y = parts.next()?;
    let m = parts.next()?;
    let d = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    // ★ 4 桁でない先頭は年と決めつけない
    if y.len() != 4 {
        return None;
    }
    NaiveDate::from_ymd_opt(y.parse().ok()?, m.parse().ok()?, d.parse().ok()?)
}

// ── CSV / TSV の読み書き ─────────────────────────────────────────────────────

/// 1 レコード。`line` は**ファイル先頭からの物理行番号**（1 始まり）。
///
/// ★ レコードの通し番号ではない。メモに改行が入ると 2 つはずれていき、
/// 「N 行目を直してください」がテキストエディタで開いた表と合わなくなる。
struct Record {
    line: usize,
    cells: Vec<String>,
}

/// RFC4180 の CSV / TSV を行 × セルに開く。
///
/// メモが自由記述なので、引用符の中のカンマ・改行・二重引用符を必ず通す必要がある。
/// CRLF と LF、末尾改行の有無をどれも受ける。
///
/// 2 つ目の返り値は**引用符が閉じないまま終わったか**。true のときセルの中身は
/// 信用できない（残り全部が 1 セルに入っている）ので、呼び出し側は行の解釈へ進まない。
fn read_delimited(raw: &str, delim: char) -> (Vec<Record>, bool) {
    let mut rows: Vec<Record> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = raw.chars().peekable();
    let mut line = 1usize;
    let mut start = 1usize;

    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    quoted = false;
                }
            } else {
                if c == '\n' {
                    line += 1;
                }
                field.push(c);
            }
            continue;
        }
        if c == delim {
            row.push(std::mem::take(&mut field));
        } else if c == '"' && field.trim().is_empty() {
            // ★ 引用符の前の空白は捨てる。`, "軽い, 余裕"` のように区切りのあとへ
            //   空白を入れた表は珍しくなく、literal 扱いにすると中のカンマで
            //   セルが割れ、以降の列が 1 つずつずれる
            field.clear();
            quoted = true;
        } else if c == '\n' || c == '\r' {
            if c == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            row.push(std::mem::take(&mut field));
            rows.push(Record {
                line: start,
                cells: std::mem::take(&mut row),
            });
            line += 1;
            start = line;
        } else {
            field.push(c);
        }
    }
    // 末尾に改行が無い場合だけ最後の行が残る
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(Record {
            line: start,
            cells: row,
        });
    }
    (rows, quoted)
}

/// RFC4180 の引用。メモが自由記述なので、カンマ・改行・二重引用符を必ず逃がす。
fn write_field(out: &mut String, s: &str) {
    if s.contains([',', '"', '\n', '\r']) {
        out.push('"');
        for c in s.chars() {
            if c == '"' {
                out.push('"');
            }
            out.push(c);
        }
        out.push('"');
    } else {
        out.push_str(s);
    }
}

/// 数を人が読む形で出す。`60.0` は `60`、`62.5` は `62.5`。
fn num(v: f32) -> String {
    format!("{v}")
}

// ── 書き出し ────────────────────────────────────────────────────────────────

/// Excel が UTF-8 と判るための BOM。付けないと日本語環境の Excel が文字化けする。
const BOM: &str = "\u{feff}";

/// 表計算で開くための CSV。**BOM 付き。**
///
/// ★ 書き出しは CSV だけ。TSV をクリップボードへ入れる経路も一度作ったが、
/// 10 年分で 900k 文字になるうえ、iOS のユニバーサルクリップボードが
/// 記録を他端末へ同期してしまうので撤去した
/// （adr/storage/csv-as-a-secondary-lossy-format.md）。
pub fn export_csv(db: &Db) -> String {
    format!("{BOM}{}", render(db))
}

/// 書き出しのファイル名。[`core::export_filename`] と同じく**時刻まで入れる**
/// （同じ日に 2 回書き出したとき 1 回目を潰さないため）。
pub fn export_csv_filename(now: chrono::NaiveDateTime) -> String {
    format!("fitness-memo-{}.csv", now.format("%Y%m%d-%H%M"))
}

fn render(db: &Db) -> String {
    let mut out = String::new();
    for (i, (_, name)) in COLUMNS.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_field(&mut out, name);
    }
    out.push('\n');

    for (date, session) in &db.sessions {
        let body = session.body_weight.map(num).unwrap_or_default();
        let day_note = session.note.clone();

        let mut wrote = false;

        for log in &session.logs {
            let ex = db.exercise(log.exercise_id);
            let ex_name = ex.map_or_else(|| log.exercise_id.to_string(), |e| e.name.clone());
            let group = ex
                .and_then(|e| db.group(e.group_id))
                .map(|g| g.name.clone())
                .unwrap_or_default();
            let ex_note = log.note.clone();

            // ★ セットが無くてもメモだけの種目は残る（`core::normalize` が捨てるのは
            //   「セットもメモも無い」ログだけ）。行を出さないと往復でメモが消える。
            //   取り込み側は重量と回数がどちらも空の行を「メモだけの行」として読む
            if log.sets.is_empty() {
                if ex_note.is_empty() {
                    continue;
                }
                let cells = vec![
                    date.clone(),
                    group.clone(),
                    ex_name.clone(),
                    String::new(),
                    String::new(),
                    String::new(),
                    String::new(),
                    ex_note.clone(),
                    body.clone(),
                    day_note.clone(),
                ];
                push_row(&mut out, &cells);
                wrote = true;
                continue;
            }

            for (i, set) in log.sets.iter().enumerate() {
                let cells = vec![
                    date.clone(),
                    group.clone(),
                    ex_name.clone(),
                    (i + 1).to_string(),
                    num(set.weight),
                    set.reps.to_string(),
                    set.note.clone(),
                    ex_note.clone(),
                    body.clone(),
                    day_note.clone(),
                ];
                push_row(&mut out, &cells);
                wrote = true;
            }
        }

        // ★ 1 行も出なかった日（体重・メモだけの休養日）も残す。落とすと往復で
        //   休養日の体重が消える。取り込み側は種目が空の行を「日だけの行」として読む
        if !wrote && (!body.is_empty() || !day_note.is_empty()) {
            let mut cells = vec![String::new(); COLUMNS.len()];
            cells[0] = date.clone();
            cells[8] = body.clone();
            cells[9] = day_note.clone();
            push_row(&mut out, &cells);
        }
    }
    out
}

fn push_row(out: &mut String, cells: &[String]) {
    for (i, cell) in cells.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_field(out, cell);
    }
    out.push('\n');
}

// ── 取り込みの結果 ───────────────────────────────────────────────────────────

/// 取り込めなかった行の理由。**黙って捨てず、必ず数えて画面に出す。**
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// 種目が未知で、部位も分からないので置き場所が決まらない
    UnknownGroup { exercise: String, group: String },
    /// 同じ名前の部位 / 種目が 2 つ以上あり、どちらを指しているか決まらない
    Ambiguous { name: String },
    /// 日付が読めない（`08/13/2026` のように月日が曖昧な形を含む）
    BadDate(String),
    /// 重量か回数が空か、数として読めない
    BadNumber,
}

/// 画面に出す文字列へ埋める前に丸める。
///
/// ★ セルの中身をそのまま `<p>` に流してはいけない。表の作り方次第で 1 セルが
/// 数千文字になることがあり（引用符の閉じ忘れなど）、確認画面が読めなくなる。
fn clip(s: &str) -> String {
    const MAX: usize = 24;
    if s.chars().count() <= MAX {
        return s.to_string();
    }
    format!("{}…", s.chars().take(MAX).collect::<String>())
}

impl SkipReason {
    pub fn message(&self) -> String {
        match self {
            Self::UnknownGroup { exercise, group } if group.is_empty() => {
                format!(
                    "「{}」は未登録の種目です（部位の列が空です）",
                    clip(exercise)
                )
            }
            Self::UnknownGroup { exercise, group } => {
                format!(
                    "「{}」の部位「{}」がありません",
                    clip(exercise),
                    clip(group)
                )
            }
            Self::Ambiguous { name } => {
                format!(
                    "「{}」が 2 つ以上あります（種目タブで名前を分けてください）",
                    clip(name)
                )
            }
            Self::BadDate(s) => {
                format!("日付「{}」が読めません（年から書いてください）", clip(s))
            }
            Self::BadNumber => "重量か回数が空か、数として読めません".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    /// ファイル先頭からの行番号（1 始まり）。表の中で見つけられるように出す
    pub line: usize,
    pub reason: SkipReason,
}

/// 取り込みの内訳。**確認画面に数字で出すためのもの。**
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SheetReport {
    /// ヘッダを除いたデータ行の数（空行は数えない）
    pub rows: usize,
    /// 取り込んだ行の数
    pub taken: usize,
    /// 新しく作った種目の名前
    pub exercises_created: Vec<String>,
    pub skipped: Vec<Skipped>,
}

/// 表として読めなかったとき。**利用者の次の行動が変わる粒度でだけ分ける。**
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SheetError {
    /// 空
    Empty,
    /// ヘッダ行が見つからない（列名が違う / 表になっていない）
    MissingColumns(Vec<&'static str>),
    /// 引用符が閉じていない。**セルの区切りが全部ずれるので、行の解釈へ進まない**
    UnclosedQuote,
    /// ヘッダはあるがデータ行が 1 つも取り込めなかった
    AllRowsSkipped(Vec<Skipped>),
}

impl SheetError {
    pub fn message(&self) -> String {
        match self {
            Self::Empty => "中身がありません".to_string(),
            Self::MissingColumns(missing) => format!(
                "表の見出しに「{}」が見つかりません（1 行目を見出しにしてください）",
                missing.join("」「")
            ),
            Self::UnclosedQuote => {
                "引用符（\"）が閉じていません。メモの中の \" を確かめてください".to_string()
            }
            Self::AllRowsSkipped(skipped) => {
                let head = skipped
                    .first()
                    .map(|s| s.reason.message())
                    .unwrap_or_default();
                format!("取り込める行がありませんでした。{head}")
            }
        }
    }
}

// ── 取り込み ────────────────────────────────────────────────────────────────

/// 表 → `Db`。**`db` は突き合わせに使うだけで、書き換えない。**
///
/// 返す `Db` は [`core::merge_db`] に渡す前提で、参照した既存の部位 / 種目は
/// **そのまま複製して入れる**。読み取った表記で入れると `merge_db` が
/// `Conflict::NameMatched` を並べるが、**同じものを同じと言っただけで改名は
/// 起きていない**（adr/data-model/text-import-is-merge-only.md 決定 4 と同じ理由）。
///
/// 区切りは `,` で試し、見出しが揃わなければ `\t` で読み直す。これで
/// 「スプレッドシートのセルを選択してコピーし、貼り付け欄に貼る」経路が動く。
pub fn parse(raw: &str, db: &Db, ids: &mut IdGen) -> Result<(Db, SheetReport), SheetError> {
    let raw = raw.trim_start_matches('\u{feff}');
    if raw.trim().is_empty() {
        return Err(SheetError::Empty);
    }

    // ★ どちらの区切りでも見出しが揃わなかったときは、**不足が少ないほう**を報告する。
    //   後から試したほうで上書きすると、カンマ区切りの表なのにタブで読んだ結果
    //   （行全体が 1 セルなので全列が不足）を見せることになり、直し方が伝わらない
    let mut best: Option<Vec<&'static str>> = None;
    let mut unclosed = false;
    for delim in [',', '\t'] {
        match parse_with(raw, delim, db, ids) {
            Ok(ok) => return Ok(ok),
            Err(SheetError::MissingColumns(m)) => {
                if best.as_ref().is_none_or(|b| m.len() < b.len()) {
                    best = Some(m);
                }
            }
            // 区切りを取り違えると引用符の見え方も変わるので、もう一方も試す
            Err(SheetError::UnclosedQuote) => unclosed = true,
            // 見出しは見つかったので区切りの推定は当たっている。もう一方で読み直さない
            Err(other) => return Err(other),
        }
    }
    if unclosed {
        // 「見出しが無い」より直し方が具体的なので、こちらを優先して出す
        return Err(SheetError::UnclosedQuote);
    }
    Err(best.map_or(SheetError::Empty, SheetError::MissingColumns))
}

fn parse_with(
    raw: &str,
    delim: char,
    db: &Db,
    ids: &mut IdGen,
) -> Result<(Db, SheetReport), SheetError> {
    let (rows, unclosed) = read_delimited(raw, delim);
    if unclosed {
        // ★ ここで止める。残り全部が 1 セルに入っているので、進めても列は
        //   全部ずれるうえ、そのセルが「未登録の種目」として画面に流れ出る
        return Err(SheetError::UnclosedQuote);
    }

    // 見出しは 1 行目とは限らない（表題やコメント行が上にあることがある）
    let mut head: Option<(usize, HashMap<Col, usize>)> = None;
    for (i, row) in rows.iter().enumerate().take(20) {
        let map = header_map(&row.cells);
        if REQUIRED.iter().all(|(c, _)| map.contains_key(c)) {
            head = Some((i, map));
            break;
        }
    }
    let Some((header_row, cols)) = head else {
        // 一番惜しかった行ではなく、常に「全部足りない」と言うと不親切なので
        // 先頭 20 行で最もよく当たった行の不足を返す
        let missing = rows
            .iter()
            .take(20)
            .map(|row| {
                let map = header_map(&row.cells);
                REQUIRED
                    .iter()
                    .filter(|(c, _)| !map.contains_key(c))
                    .map(|(_, n)| *n)
                    .collect::<Vec<_>>()
            })
            .min_by_key(Vec::len)
            .unwrap_or_else(|| REQUIRED.iter().map(|(_, n)| *n).collect());
        return Err(SheetError::MissingColumns(missing));
    };

    let mut resolver = Resolver::new(db);
    let mut report = SheetReport::default();
    let mut sessions: BTreeMap<String, Session> = BTreeMap::new();

    for row in rows.iter().skip(header_row + 1) {
        let line = row.line;
        let cell = |c: Col| -> &str {
            cols.get(&c)
                .and_then(|&ix| row.cells.get(ix))
                .map(|s| s.trim())
                .unwrap_or("")
        };
        if row.cells.iter().all(|s| s.trim().is_empty()) {
            continue;
        }
        report.rows += 1;

        let Some(date) = parse_date(cell(Col::Date)) else {
            report.skipped.push(Skipped {
                line,
                reason: SkipReason::BadDate(cell(Col::Date).to_string()),
            });
            continue;
        };

        // ★ **セッションにはまだ触らない。** 先に触ると、この後で落とす行の
        //   体重・当日メモだけが「記録のない日」として入り、利用者が取り込まないと
        //   言われた日がカレンダーに現れる
        let name = cell(Col::Exercise);
        let entry = if name.is_empty() {
            // 体重・メモだけの行。休養日の往復がここで成り立つ
            None
        } else {
            // ★ 種目の解決を数の解釈より先にやる。置き場所が決まらない行に
            //   「重量が読めません」と言っても直しようがない
            let id = match resolver.resolve(name, cell(Col::Group), ids, &mut report) {
                Ok(id) => id,
                Err(reason) => {
                    report.skipped.push(Skipped { line, reason });
                    continue;
                }
            };
            let note = cell(Col::ExerciseNote);
            // 重量と回数がどちらも空なら「メモだけの種目」の行。ただし種目メモも
            // 無ければ、この行は何も持っていない —— `core::normalize` が黙って
            // 捨てるので、取り込んだことにせず報告する
            let blank = cell(Col::Weight).is_empty() && cell(Col::Reps).is_empty();
            let set = match (blank, note.is_empty()) {
                (true, false) => None,
                (true, true) => {
                    report.skipped.push(Skipped {
                        line,
                        reason: SkipReason::BadNumber,
                    });
                    continue;
                }
                _ => match (parse_num(cell(Col::Weight)), parse_num(cell(Col::Reps))) {
                    // ★ f32 に落として有限かつ非負であることまで見る。`3.5e38` は
                    //   f64 では有限だが f32 では inf になり、`core::normalize` が
                    //   黙って捨てる —— 捨てられる値をここで弾けば必ず報告できる
                    (Some(w), Some(r))
                        if r >= 0.0
                            && r.fract() == 0.0
                            && r <= u32::MAX as f64
                            && (w as f32).is_finite()
                            && w >= 0.0 =>
                    {
                        Some((w as f32, r as u32))
                    }
                    _ => {
                        report.skipped.push(Skipped {
                            line,
                            reason: SkipReason::BadNumber,
                        });
                        continue;
                    }
                },
            };
            Some((id, set))
        };

        // ここまで来た行だけがセッションを作る
        let session = sessions.entry(date_key(date)).or_default();

        // 日単位の値は行に繰り返して書き出しているので、**最初に現れた非空の値**を採る
        if session.body_weight.is_none()
            && let Some(w) = parse_num(cell(Col::BodyWeight))
            && (w as f32).is_finite()
            && w > 0.0
        {
            session.body_weight = Some(w as f32);
        }
        if session.note.is_empty() {
            session.note = cell(Col::DayNote).to_string();
        }

        if let Some((id, set)) = entry {
            let log = match session.logs.iter_mut().find(|l| l.exercise_id == id) {
                Some(l) => l,
                None => {
                    session.logs.push(ExerciseLog {
                        exercise_id: id,
                        sets: Vec::new(),
                        // ★ 過去日のバックフィルなので時刻を持たない
                        at: None,
                        note: String::new(),
                    });
                    session.logs.last_mut().expect("直前に push した")
                }
            };
            if log.note.is_empty() {
                log.note = cell(Col::ExerciseNote).to_string();
            }
            // ★ セット番号の列は読まない。表に見えている**行の並び**を採る
            if let Some((weight, reps)) = set {
                log.sets.push(SetEntry {
                    weight,
                    reps,
                    note: cell(Col::SetNote).to_string(),
                });
            }
        }
        report.taken += 1;
    }

    if report.taken == 0 {
        return Err(SheetError::AllRowsSkipped(report.skipped));
    }

    let (groups, exercises) = resolver.into_parts();
    let mut out = Db {
        schema: SCHEMA,
        groups,
        exercises,
        sessions,
    };
    // ★ 取り込み境界では必ず通す。`drop_unrepresentable_weights` を経由しないと
    //   `3.5e38` のような値が `inf` → `null` で保存され、次回起動から永久に読めなくなる
    core::normalize(&mut out);
    Ok((out, report))
}

fn header_map(row: &[String]) -> HashMap<Col, usize> {
    let mut map = HashMap::new();
    for (ix, name) in row.iter().enumerate() {
        if let Some(col) = col_of(name) {
            // 同じ列が 2 回出たら左を採る
            map.entry(col).or_insert(ix);
        }
    }
    map
}

/// 名前 → 既存の ID。**新しい部位は作らない。**
///
/// 部位はグラフの集計単位なので、増やすと過去の集計まで割れる
/// （adr/data-model/group-metric-is-set-count.md）。移行元の画面に写っている
/// 「Push Day」のようなカテゴリ名をそのまま部位にすると、プリセットの 6 部位と
/// 意味が重なった部位が並び、以後ずっと種目が二重に分かれる。
struct Resolver<'a> {
    db: &'a Db,
    /// 正規化名 → ID。**`None` は「同じ名前が 2 つ以上あって決まらない」。**
    groups: HashMap<String, Option<GroupId>>,
    exercises: HashMap<String, Option<ExerciseId>>,
    /// プリセットの正規化名 → 固定 ID
    preset_ids: HashMap<String, ExerciseId>,
    used_groups: Vec<GroupId>,
    used_exercises: Vec<ExerciseId>,
    created: Vec<Exercise>,
}

impl<'a> Resolver<'a> {
    fn new(db: &'a Db) -> Self {
        // ★ 同名の種目は実在しうる。`rename_exercise` に重複チェックが無いので
        //   「ベンチプレス」を 2 つ作れる。片方を勝手に選ぶと**別々の種目の履歴が
        //   無警告で 1 本に合流し、グラフが滑らかに繋がるので間違いに気づけない**。
        //   adr/data-model/random-ids-for-safe-merge.md が v2 → v3 の移行で
        //   「一致がちょうど 1 件のときだけ寄せる」と決めたのと同じ穴。
        let mut exercises: HashMap<String, Option<ExerciseId>> = HashMap::new();
        for e in &db.exercises {
            exercises
                .entry(normalize_key(&e.name))
                .and_modify(|slot| *slot = None)
                .or_insert(Some(e.id));
        }
        // ★ 部位も同じ。`rename_group`（src/views/menu.rs）にも重複チェックが無く、
        //   `core::pin_presets` は部位と種目の**両方**に「ちょうど 1 件」の規則を
        //   掛けている。片方だけ守っても、種目が別の「胸」に入って集計が割れる
        let mut groups: HashMap<String, Option<GroupId>> = HashMap::new();
        for g in &db.groups {
            groups
                .entry(normalize_key(&g.name))
                .and_modify(|slot| *slot = None)
                .or_insert(Some(g.id));
        }
        Self {
            db,
            groups,
            exercises,
            // ★ プリセットも正規化名で引く。生の名前で引くと「デッド リフト」が
            //   固定 ID に当たらず乱数を引き、2 台で ID が割れる
            preset_ids: presets::seeded_db()
                .exercises
                .into_iter()
                .map(|e| (normalize_key(&e.name), e.id))
                .collect(),
            used_groups: Vec::new(),
            used_exercises: Vec::new(),
            created: Vec::new(),
        }
    }

    fn resolve(
        &mut self,
        name: &str,
        group: &str,
        ids: &mut IdGen,
        report: &mut SheetReport,
    ) -> Result<ExerciseId, SkipReason> {
        let key = normalize_key(name);
        match self.exercises.get(&key) {
            Some(Some(id)) => {
                let id = *id;
                if !self.used_exercises.contains(&id) {
                    self.used_exercises.push(id);
                    if let Some(g) = self.db.exercise(id).map(|e| e.group_id) {
                        self.use_group(g);
                    }
                }
                return Ok(id);
            }
            // 決まらないものは推測しない。名前を分ければ直せると画面で言う
            Some(None) => {
                return Err(SkipReason::Ambiguous {
                    name: name.to_string(),
                });
            }
            None => {}
        }

        // 未知の種目。部位が既存のものに当たるときだけ作る
        let gid = match self.groups.get(&normalize_key(group)) {
            Some(Some(id)) => *id,
            Some(None) => {
                return Err(SkipReason::Ambiguous {
                    name: group.to_string(),
                });
            }
            None => {
                return Err(SkipReason::UnknownGroup {
                    exercise: name.to_string(),
                    group: group.to_string(),
                });
            }
        };
        self.use_group(gid);

        // ★ プリセット名なら固定 ID を使う。乱数を引くと、同じ種目が端末ごとに
        //   別 ID になって履歴が 2 本に割れる（adr/data-model/random-ids-for-safe-merge.md）
        let id = self
            .preset_ids
            .get(&key)
            .copied()
            .unwrap_or_else(|| ids.alloc());
        let order = (self
            .db
            .exercises
            .iter()
            .filter(|e| e.group_id == gid)
            .count()
            + self.created.iter().filter(|e| e.group_id == gid).count()) as u32;
        self.created.push(Exercise {
            id,
            name: name.to_string(),
            group_id: gid,
            order,
            archived: false,
        });
        self.exercises.insert(key, Some(id));
        report.exercises_created.push(name.to_string());
        Ok(id)
    }

    fn use_group(&mut self, id: GroupId) {
        if !self.used_groups.contains(&id) {
            self.used_groups.push(id);
        }
    }

    /// 参照した既存の部位 / 種目を**そのまま複製して**返す。`merge_db` は ID 一致の枝で
    /// 素通りするので副作用が無く、`Db` 単体で見ても整合する。
    fn into_parts(self) -> (Vec<crate::model::Group>, Vec<Exercise>) {
        let groups = self
            .used_groups
            .iter()
            .filter_map(|&id| self.db.group(id).cloned())
            .collect();
        let mut exercises: Vec<Exercise> = self
            .used_exercises
            .iter()
            .filter_map(|&id| self.db.exercise(id).cloned())
            .collect();
        exercises.extend(self.created);
        (groups, exercises)
    }
}

// ── Google スプレッドシートの URL ────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SheetUrlError {
    Empty,
    /// Google スプレッドシートの URL ではない
    NotSheets,
    /// スプレッドシートの ID が取り出せない
    NoId,
}

impl SheetUrlError {
    pub fn message(&self) -> String {
        match self {
            Self::Empty => "URL が空です".to_string(),
            Self::NotSheets => {
                "Google スプレッドシートの URL を貼ってください（docs.google.com で始まります）"
                    .to_string()
            }
            Self::NoId => "URL からスプレッドシートを特定できません".to_string(),
        }
    }
}

/// 貼られた URL → CSV を返す URL。
///
/// 利用者がブラウザのアドレス欄からコピーする形（`/edit?gid=..#gid=..`）、共有ダイアログの
/// 形（`/edit?usp=sharing`）、「ウェブに公開」の形（`/d/e/2PACX-.../pub`）をどれも受ける。
///
/// **`/d/e/` は別物**で、`export?format=csv` を受け付けない（公開専用の ID なので
/// `pub?output=csv` を使う）。混ぜると「共有されていません」と誤診する。
pub fn csv_url(pasted: &str) -> Result<String, SheetUrlError> {
    let s = pasted.trim();
    if s.is_empty() {
        return Err(SheetUrlError::Empty);
    }
    if !s.contains("docs.google.com/spreadsheets/") {
        return Err(SheetUrlError::NotSheets);
    }
    let gid = find_gid(s);

    let after = s
        .split("/spreadsheets/")
        .nth(1)
        .ok_or(SheetUrlError::NotSheets)?;
    // ★ 複数の Google アカウントにログインしていると、アドレス欄の URL は
    //   `/spreadsheets/u/1/d/{ID}/edit` になる。**これを読めないと、2 つ以上
    //   アカウントを持っている人が全員この機能を使えない**
    let after = strip_account(after);

    if let Some(rest) = after.strip_prefix("d/e/") {
        let id = take_id(rest).ok_or(SheetUrlError::NoId)?;
        let mut url = format!("https://docs.google.com/spreadsheets/d/e/{id}/pub?output=csv");
        if let Some(g) = gid {
            // ★ 公開 URL は `single=true` を添えないと gid を無視して既定のシートを
            //   返す（`/export` 側とは指定の仕方が違う）
            url.push_str(&format!("&gid={g}&single=true"));
        }
        return Ok(url);
    }
    let rest = after.strip_prefix("d/").ok_or(SheetUrlError::NoId)?;
    let id = take_id(rest).ok_or(SheetUrlError::NoId)?;
    let mut url = format!("https://docs.google.com/spreadsheets/d/{id}/export?format=csv");
    if let Some(g) = gid {
        url.push_str(&format!("&gid={g}"));
    }
    Ok(url)
}

/// `u/1/d/{ID}/...` の先頭のアカウント指定を読み飛ばす。無ければそのまま返す。
fn strip_account(s: &str) -> &str {
    let Some(rest) = s.strip_prefix("u/") else {
        return s;
    };
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    match rest[digits..].strip_prefix('/') {
        Some(r) if digits > 0 => r,
        _ => s,
    }
}

/// URL の一部から ID を取り出す。Google の ID は `[A-Za-z0-9_-]` だけで出来ている。
fn take_id(rest: &str) -> Option<String> {
    let id: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    // 短すぎるものは ID ではない（`/d/edit` のような取り違えを弾く）
    (id.len() >= 20).then_some(id)
}

/// `?gid=0` も `#gid=0` も拾う。**最後に現れたものを採る** — アドレス欄からコピーすると
/// `/edit?gid=123#gid=123` の形になり、利用者が見ているのはフラグメントのほうだから。
fn find_gid(s: &str) -> Option<String> {
    let mut found = None;
    for part in s.split("gid=").skip(1) {
        let g: String = part.chars().take_while(char::is_ascii_digit).collect();
        if !g.is_empty() {
            found = Some(g);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Group;

    fn ids() -> IdGen {
        IdGen::from_seed(1)
    }

    fn g(n: u64) -> GroupId {
        GroupId::from_bits(0x1_0000 + n)
    }

    fn e(n: u64) -> ExerciseId {
        ExerciseId::from_bits(0x1_0000 + n)
    }

    /// 胸(1): ベンチプレス(10) / プッシュアップ(11)、脚(2): スクワット(20)
    fn test_db() -> Db {
        let mut db = Db {
            schema: SCHEMA,
            ..Db::default()
        };
        db.groups.push(Group {
            id: g(1),
            name: "胸".into(),
            color: "#e0524a".into(),
            order: 0,
        });
        db.groups.push(Group {
            id: g(2),
            name: "脚".into(),
            color: "#2fa06a".into(),
            order: 1,
        });
        db.exercises.push(Exercise {
            id: e(10),
            name: "ベンチプレス".into(),
            group_id: g(1),
            order: 0,
            archived: false,
        });
        db.exercises.push(Exercise {
            id: e(11),
            name: "プッシュアップ".into(),
            group_id: g(1),
            order: 1,
            archived: false,
        });
        db.exercises.push(Exercise {
            id: e(20),
            name: "スクワット".into(),
            group_id: g(2),
            order: 0,
            archived: false,
        });
        db
    }

    fn set(weight: f32, reps: u32, note: &str) -> SetEntry {
        SetEntry {
            weight,
            reps,
            note: note.into(),
        }
    }

    fn log(id: ExerciseId, sets: Vec<SetEntry>, note: &str) -> ExerciseLog {
        ExerciseLog {
            exercise_id: id,
            sets,
            at: None,
            note: note.into(),
        }
    }

    /// 記録入りの `Db`。メモにカンマ・引用符・改行を混ぜてある（引用の往復を見るため）
    fn db_with_records() -> Db {
        let mut db = test_db();
        db.sessions.insert(
            "2026-08-01".into(),
            Session {
                logs: vec![
                    log(
                        e(10),
                        vec![
                            set(60.0, 10, "軽い, 余裕"),
                            set(62.5, 8, "肩が\"詰まる\""),
                            set(62.5, 6, ""),
                        ],
                        "フォーム確認",
                    ),
                    log(e(20), vec![set(80.0, 5, "")], ""),
                ],
                body_weight: Some(70.5),
                note: "調子よい\n睡眠 7 時間".into(),
            },
        );
        db.sessions.insert(
            "2026-08-03".into(),
            Session {
                logs: vec![log(e(11), vec![set(0.0, 20, "")], "")],
                body_weight: None,
                note: String::new(),
            },
        );
        db
    }

    // ── 書き出し ────────────────────────────────────────────────────────────

    #[test]
    fn csv_export_starts_with_a_utf8_bom_so_excel_opens_it_as_utf8() {
        let csv = export_csv(&db_with_records());
        assert!(
            csv.starts_with('\u{feff}'),
            "BOM が無いと Excel が文字化けする"
        );
        // BOM の直後は見出し行
        assert!(csv[3..].starts_with("日付,部位,種目,"), "{}", &csv[..40]);
    }

    #[test]
    fn csv_quotes_fields_containing_commas_quotes_and_newlines() {
        let csv = export_csv(&db_with_records());
        assert!(
            csv.contains(r#""軽い, 余裕""#),
            "カンマを含むメモが引用されていない"
        );
        assert!(
            csv.contains(r#""肩が""詰まる""""#),
            "二重引用符が 2 個に増えていない"
        );
        assert!(
            csv.contains("\"調子よい\n睡眠 7 時間\""),
            "改行を含むメモが引用されていない"
        );
    }

    #[test]
    fn csv_export_writes_one_row_per_set_with_a_running_set_number() {
        let csv = export_csv(&db_with_records());
        let (rows, _) = read_delimited(csv.trim_start_matches('\u{feff}'), ',');
        // 見出し + 3 セット + 1 セット + 1 セット
        assert_eq!(rows.len(), 6);
        assert_eq!(rows[1].cells[0], "2026-08-01");
        assert_eq!(rows[1].cells[2], "ベンチプレス");
        assert_eq!(rows[1].cells[3], "1");
        assert_eq!(rows[2].cells[3], "2");
        assert_eq!(rows[3].cells[3], "3");
        // 種目が変わるとセット番号は 1 に戻る
        assert_eq!(rows[4].cells[2], "スクワット");
        assert_eq!(rows[4].cells[3], "1");
    }

    #[test]
    fn csv_export_repeats_day_level_columns_on_every_row() {
        let csv = export_csv(&db_with_records());
        let (rows, _) = read_delimited(csv.trim_start_matches('\u{feff}'), ',');
        for row in rows.iter().skip(1).take(4) {
            assert_eq!(row.cells[8], "70.5", "体重が全行に出ていない");
        }
    }

    #[test]
    fn csv_export_keeps_a_rest_day_that_only_has_body_weight() {
        let mut db = test_db();
        db.sessions.insert(
            "2026-08-05".into(),
            Session {
                logs: Vec::new(),
                body_weight: Some(69.8),
                note: "休養".into(),
            },
        );
        let csv = export_csv(&db);
        let (rows, _) = read_delimited(csv.trim_start_matches('\u{feff}'), ',');
        assert_eq!(rows.len(), 2, "休養日の行が落ちている");
        assert_eq!(rows[1].cells[0], "2026-08-05");
        assert_eq!(rows[1].cells[2], "", "種目は空のまま出す");
        assert_eq!(rows[1].cells[8], "69.8");
    }

    #[test]
    fn csv_round_trips_an_exercise_that_only_has_a_note() {
        // ★ `core::normalize` が捨てるのは「セットもメモも無い」ログだけなので、
        //   メモだけの種目は残る（adr/data-model/notes-on-logs-and-sets.md）。
        //   行を出さないと往復でメモが消える
        let mut db = test_db();
        db.sessions.insert(
            "2026-08-01".into(),
            Session {
                logs: vec![
                    log(e(10), vec![set(60.0, 10, "")], ""),
                    log(e(20), Vec::new(), "今日は張りが出たのでやめた"),
                ],
                body_weight: None,
                note: String::new(),
            },
        );
        let csv = export_csv(&db);
        let (rows, _) = read_delimited(csv.trim_start_matches('\u{feff}'), ',');
        assert_eq!(rows.len(), 3, "メモだけの種目の行が落ちている");
        assert_eq!(rows[2].cells[2], "スクワット");
        assert_eq!(rows[2].cells[4], "", "セットが無いので重量は空");
        assert_eq!(rows[2].cells[7], "今日は張りが出たのでやめた");

        let (parsed, report) = parse(&csv, &db, &mut ids()).unwrap();
        assert_eq!(
            report.skipped,
            Vec::new(),
            "自前で書いた行を読めないと言っている"
        );
        assert_eq!(parsed.sessions, db.sessions);
    }

    #[test]
    fn csv_and_json_filenames_both_carry_the_time_so_a_second_export_does_not_clobber_the_first() {
        let at = NaiveDate::from_ymd_opt(2026, 8, 13)
            .unwrap()
            .and_hms_opt(9, 5, 0)
            .unwrap();
        assert_eq!(export_csv_filename(at), "fitness-memo-20260813-0905.csv");
    }

    // ── 読み込み ────────────────────────────────────────────────────────────

    #[test]
    fn csv_export_round_trips_through_csv_import() {
        // この機能の生命線。書き出したものが同じ記録として戻らないなら、
        // 表計算で編集して戻す経路が丸ごと成立しない
        let db = db_with_records();
        let csv = export_csv(&db);
        let (parsed, report) = parse(&csv, &db, &mut ids()).expect("往復できる");
        assert_eq!(parsed.sessions, db.sessions);
        assert_eq!(report.skipped, Vec::new());
        assert!(
            report.exercises_created.is_empty(),
            "既存の種目を作り直している"
        );
    }

    #[test]
    fn csv_reader_accepts_crlf_and_a_trailing_newline() {
        let with = "日付,種目,重量kg,回数\r\n2026-08-01,ベンチプレス,60,10\r\n";
        let without = "日付,種目,重量kg,回数\n2026-08-01,ベンチプレス,60,10";
        let db = test_db();
        let (a, _) = parse(with, &db, &mut ids()).unwrap();
        let (b, _) = parse(without, &db, &mut ids()).unwrap();
        assert_eq!(a.sessions, b.sessions);
        assert_eq!(
            a.sessions["2026-08-01"].logs[0].sets,
            vec![set(60.0, 10, "")]
        );
    }

    #[test]
    fn csv_columns_are_resolved_by_header_name_not_position() {
        // 書き出したファイルは利用者の手元に残り続けるので、列を並べ替えたり
        // 自分用の列を足したりしても読めなければならない
        let csv = "\
メモ,回数,重量(kg),種目,自分用のメモ,日付
最高,10,60,ベンチプレス,無視される,2026-08-01
";
        let db = test_db();
        let (parsed, _) = parse(csv, &db, &mut ids()).unwrap();
        let day = &parsed.sessions["2026-08-01"];
        // ★ 素の「メモ」はセット側。日単位に寄せると 2 行目以降が黙って消える
        assert_eq!(day.logs[0].sets, vec![set(60.0, 10, "最高")]);
        assert_eq!(day.note, "");
    }

    #[test]
    fn a_single_memo_column_keeps_every_row_not_just_the_first() {
        // 手で作った移行用の表はメモ列が 1 本しかないことが多い。日単位に寄せると
        // 「その日で最初に現れた非空の値」の規則で 2 行目以降が全部消える
        let db = test_db();
        let csv = "\
日付,種目,重量kg,回数,メモ
2026-08-01,ベンチプレス,60,10,軽い
2026-08-01,ベンチプレス,60,8,きつい
2026-08-01,ベンチプレス,60,6,限界
";
        let (parsed, _) = parse(csv, &db, &mut ids()).unwrap();
        assert_eq!(
            parsed.sessions["2026-08-01"].logs[0].sets,
            vec![
                set(60.0, 10, "軽い"),
                set(60.0, 8, "きつい"),
                set(60.0, 6, "限界")
            ]
        );
    }

    #[test]
    fn csv_import_finds_the_header_below_a_title_row() {
        let csv = "\
2026 年 8 月のトレーニング,,,
,,,
日付,種目,重量kg,回数
2026-08-01,ベンチプレス,60,10
";
        let db = test_db();
        let (parsed, _) = parse(csv, &db, &mut ids()).unwrap();
        assert_eq!(parsed.sessions.len(), 1);
    }

    #[test]
    fn csv_import_reads_tab_separated_cells_pasted_from_a_spreadsheet() {
        // スプレッドシートでセルを選択してコピーすると TSV になる。共有設定を
        // 変えずに済む最短経路なので、貼り付け欄でも受ける
        let tsv = "日付\t種目\t重量kg\t回数\n2026-08-01\tベンチプレス\t60\t10\n";
        let db = test_db();
        let (parsed, _) = parse(tsv, &db, &mut ids()).unwrap();
        assert_eq!(
            parsed.sessions["2026-08-01"].logs[0].sets,
            vec![set(60.0, 10, "")]
        );
    }

    #[test]
    fn csv_import_accepts_the_dates_google_sheets_reformats() {
        // `/export` は表示値を返すので、`2026-08-13` と書いても日本語ロケールでは
        // `2026/08/13` で出てくる。ここを落とすと自前の往復が動かない
        let db = test_db();
        for form in [
            "2026-08-01",
            "2026/08/01",
            "2026/8/1",
            "2026年8月1日",
            "2026.08.01",
        ] {
            let csv = format!("日付,種目,重量kg,回数\n{form},ベンチプレス,60,10\n");
            let (parsed, _) = parse(&csv, &db, &mut ids()).unwrap_or_else(|e| {
                panic!("{form} が読めない: {}", e.message());
            });
            assert!(parsed.sessions.contains_key("2026-08-01"), "{form}");
        }
    }

    #[test]
    fn csv_import_refuses_to_guess_ambiguous_dates() {
        // `08/13/2026` と `13/08/2026` はどちらの解釈も成り立つ。推測して外すと
        // 記録が半年ずれた日に静かに入り、誰も気づけない
        let db = test_db();
        let csv = "\
日付,種目,重量kg,回数
08/13/2026,ベンチプレス,60,10
2026-08-01,ベンチプレス,60,10
";
        let (parsed, report) = parse(csv, &db, &mut ids()).unwrap();
        assert_eq!(parsed.sessions.len(), 1, "曖昧な日付を取り込んでいる");
        assert_eq!(
            report.skipped,
            vec![Skipped {
                line: 2,
                reason: SkipReason::BadDate("08/13/2026".into()),
            }]
        );
    }

    #[test]
    fn csv_import_matches_names_that_differ_only_in_width_or_spacing() {
        let db = test_db();
        let csv = "日付,種目,重量kg,回数\n2026-08-01, ベンチ プレス ,６０,１０\n";
        let (parsed, report) = parse(csv, &db, &mut ids()).unwrap();
        assert!(
            report.exercises_created.is_empty(),
            "同じ種目を作り直している"
        );
        assert_eq!(parsed.sessions["2026-08-01"].logs[0].exercise_id, e(10));
        assert_eq!(
            parsed.sessions["2026-08-01"].logs[0].sets,
            vec![set(60.0, 10, "")]
        );
    }

    #[test]
    fn csv_import_does_not_merge_names_that_are_merely_similar() {
        // 間違って繋がった履歴は、グラフが滑らかに繋がるので間違いに気づけない
        let mut db = test_db();
        db.exercises.push(Exercise {
            id: e(30),
            name: "レッグカール".into(),
            group_id: g(2),
            order: 1,
            archived: false,
        });
        let csv = "日付,部位,種目,重量kg,回数\n2026-08-01,脚,レッグエクステンション,40,12\n";
        let (parsed, report) = parse(csv, &db, &mut ids()).unwrap();
        assert_eq!(report.exercises_created, vec!["レッグエクステンション"]);
        assert_ne!(parsed.sessions["2026-08-01"].logs[0].exercise_id, e(30));
    }

    #[test]
    fn csv_import_refuses_to_pick_between_two_exercises_with_the_same_name() {
        // ★ `rename_exercise` に重複チェックが無いので同名の種目は実在しうる。
        //   片方を勝手に選ぶと、別々の種目の履歴が無警告で 1 本に合流する
        let mut db = test_db();
        db.exercises.push(Exercise {
            id: e(12),
            name: "ベンチプレス".into(),
            group_id: g(1),
            order: 2,
            archived: false,
        });
        let csv = "\
日付,部位,種目,重量kg,回数
2026-08-01,胸,ベンチプレス,60,10
2026-08-01,胸,プッシュアップ,0,20
";
        let (parsed, report) = parse(csv, &db, &mut ids()).unwrap();
        assert_eq!(
            report.skipped,
            vec![Skipped {
                line: 2,
                reason: SkipReason::Ambiguous {
                    name: "ベンチプレス".into(),
                },
            }]
        );
        // 新しく作り直すこともしない（3 つ目のベンチプレスを生やさない）
        assert!(report.exercises_created.is_empty());
        assert_eq!(parsed.sessions["2026-08-01"].logs.len(), 1);
        assert_eq!(parsed.sessions["2026-08-01"].logs[0].exercise_id, e(11));
    }

    #[test]
    fn csv_import_does_not_create_new_groups() {
        // 部位はグラフの集計単位なので、増やすと過去の集計まで割れる
        let db = test_db();
        let csv = "\
日付,部位,種目,重量kg,回数
2026-08-01,Push Day,Bench Press,60,10
2026-08-01,胸,ベンチプレス,60,10
";
        let (parsed, report) = parse(csv, &db, &mut ids()).unwrap();
        assert_eq!(parsed.groups.len(), 1);
        assert_eq!(parsed.groups[0].name, "胸");
        assert_eq!(
            report.skipped,
            vec![Skipped {
                line: 2,
                reason: SkipReason::UnknownGroup {
                    exercise: "Bench Press".into(),
                    group: "Push Day".into(),
                },
            }]
        );
    }

    #[test]
    fn csv_import_creates_an_unknown_exercise_inside_the_group_the_sheet_names() {
        // ここが撤去された import_text.rs との分かれ目。未知の種目ごとに部位を
        // 選ばせる確認画面を出さず、表に書いてある部位をそのまま使う
        let db = test_db();
        let csv = "日付,部位,種目,重量kg,回数\n2026-08-01,脚,レッグプレス,120,10\n";
        let (parsed, report) = parse(csv, &db, &mut ids()).unwrap();
        assert_eq!(report.exercises_created, vec!["レッグプレス"]);
        let made = parsed
            .exercises
            .iter()
            .find(|x| x.name == "レッグプレス")
            .expect("作られている");
        assert_eq!(made.group_id, g(2), "表に書いてある部位に入る");
        assert_eq!(made.order, 1, "既存のスクワットの次に並ぶ");
    }

    #[test]
    fn csv_import_gives_preset_names_their_fixed_id_so_two_devices_agree() {
        // 乱数を引くと、同じ種目が端末ごとに別 ID になって履歴が 2 本に割れる
        let mut db = test_db();
        db.groups.push(Group {
            id: presets::preset_group_id("背中").expect("プリセットの部位"),
            name: "背中".into(),
            color: "#3b82f6".into(),
            order: 2,
        });
        let csv = "日付,部位,種目,重量kg,回数\n2026-08-01,背中,デッドリフト,100,5\n";
        let (parsed, _) = parse(csv, &db, &mut ids()).unwrap();
        let made = parsed
            .exercises
            .iter()
            .find(|x| x.name == "デッドリフト")
            .unwrap();
        assert_eq!(Some(made.id), presets::preset_exercise_id("デッドリフト"));
    }

    #[test]
    fn csv_import_reports_rows_it_could_not_place_instead_of_dropping_them_silently() {
        let db = test_db();
        let csv = "\
日付,部位,種目,重量kg,回数
2026-08-01,胸,ベンチプレス,60,10
2026-08-01,胸,ベンチプレス,おもい,10
なんとか,胸,ベンチプレス,60,10
2026-08-01,不明な部位,知らない種目,60,10
";
        let (_, report) = parse(csv, &db, &mut ids()).unwrap();
        assert_eq!(report.rows, 4);
        assert_eq!(report.taken, 1);
        assert_eq!(report.skipped.len(), 3);
        assert_eq!(report.skipped[0].reason, SkipReason::BadNumber);
        assert!(matches!(report.skipped[1].reason, SkipReason::BadDate(_)));
        assert!(matches!(
            report.skipped[2].reason,
            SkipReason::UnknownGroup { .. }
        ));
        // 行番号はファイル先頭からの 1 始まり。表の中で探せる形で出す
        assert_eq!(
            report.skipped.iter().map(|s| s.line).collect::<Vec<_>>(),
            vec![3, 4, 5]
        );
    }

    #[test]
    fn csv_import_fails_loudly_when_no_row_could_be_taken() {
        let db = test_db();
        let csv = "日付,部位,種目,重量kg,回数\n2026-08-01,不明,知らない種目,60,10\n";
        let err = parse(csv, &db, &mut ids()).unwrap_err();
        assert!(matches!(err, SheetError::AllRowsSkipped(_)));
        assert!(err.message().contains("知らない種目"), "{}", err.message());
    }

    #[test]
    fn csv_import_reports_missing_columns_by_name() {
        let db = test_db();
        let err = parse("日付,種目\n2026-08-01,ベンチプレス\n", &db, &mut ids()).unwrap_err();
        assert_eq!(
            err,
            SheetError::MissingColumns(vec!["重量kg", "回数"]),
            "足りない列を名指しできていない"
        );
    }

    #[test]
    fn csv_import_rejects_an_empty_body() {
        let db = test_db();
        assert_eq!(
            parse("   \n  ", &db, &mut ids()).unwrap_err(),
            SheetError::Empty
        );
    }

    #[test]
    fn csv_import_never_writes_at() {
        // 取り込みは過去日のバックフィル。now を書くと「最後のトレーニングから 0 分」に
        // なり、core::elapsed_since_last の出力が嘘になる
        let db = test_db();
        let csv = "日付,種目,重量kg,回数\n2026-08-01,ベンチプレス,60,10\n";
        let (parsed, _) = parse(csv, &db, &mut ids()).unwrap();
        assert!(
            parsed
                .sessions
                .values()
                .all(|s| s.logs.iter().all(|l| l.at.is_none()))
        );
    }

    #[test]
    fn csv_import_takes_the_first_non_empty_value_for_day_level_columns() {
        // 日単位の値は全行に繰り返して書き出すので、利用者が 1 行だけ書き換えて
        // 矛盾させうる。どの行を採るかは決めておく
        let db = test_db();
        let csv = "\
日付,種目,重量kg,回数,体重kg,当日メモ
2026-08-01,ベンチプレス,60,10,,
2026-08-01,ベンチプレス,60,8,70.5,よい
2026-08-01,ベンチプレス,60,6,99.9,わるい
";
        let (parsed, _) = parse(csv, &db, &mut ids()).unwrap();
        let day = &parsed.sessions["2026-08-01"];
        assert_eq!(day.body_weight, Some(70.5));
        assert_eq!(day.note, "よい");
    }

    #[test]
    fn csv_import_reads_a_rest_day_row_that_has_no_exercise() {
        let db = test_db();
        let csv = "日付,種目,重量kg,回数,体重kg,当日メモ\n2026-08-05,,,,69.8,休養\n";
        let (parsed, report) = parse(csv, &db, &mut ids()).unwrap();
        assert_eq!(report.taken, 1);
        assert_eq!(report.skipped, Vec::new());
        let day = &parsed.sessions["2026-08-05"];
        assert_eq!(day.body_weight, Some(69.8));
        assert_eq!(day.note, "休養");
        assert!(day.logs.is_empty());
    }

    #[test]
    fn csv_import_ignores_the_set_number_column_and_uses_row_order() {
        // セット番号は書き出し時の目印。並べ替えた表をそのまま読めるほうが素直で、
        // 番号の重複や抜けをどう扱うかという別の曖昧さも持ち込まない
        let db = test_db();
        let csv = "\
日付,種目,セット,重量kg,回数
2026-08-01,ベンチプレス,7,60,10
2026-08-01,ベンチプレス,3,62.5,8
";
        let (parsed, _) = parse(csv, &db, &mut ids()).unwrap();
        assert_eq!(
            parsed.sessions["2026-08-01"].logs[0].sets,
            vec![set(60.0, 10, ""), set(62.5, 8, "")]
        );
    }

    #[test]
    fn csv_import_drops_weights_that_f32_cannot_represent() {
        // 3.5e38 は f64 では有限なので serde を通ってしまい、f32 で inf になる。
        // inf は `"weight":null` として保存され、次回起動から永久に読めなくなる
        let db = test_db();
        let csv = "\
日付,種目,重量kg,回数
2026-08-01,ベンチプレス,3.5e38,10
2026-08-02,ベンチプレス,60,10
";
        let (parsed, _) = parse(csv, &db, &mut ids()).unwrap();
        assert!(
            !parsed.sessions.contains_key("2026-08-01"),
            "inf の重量が残っている"
        );
        assert_eq!(
            parsed.sessions["2026-08-02"].logs[0].sets,
            vec![set(60.0, 10, "")]
        );
    }

    #[test]
    fn csv_import_reads_units_people_type_by_hand() {
        let db = test_db();
        let csv = "日付,種目,重量kg,回数\n2026-08-01,ベンチプレス,60kg,10回\n";
        let (parsed, _) = parse(csv, &db, &mut ids()).unwrap();
        assert_eq!(
            parsed.sessions["2026-08-01"].logs[0].sets,
            vec![set(60.0, 10, "")]
        );
    }

    #[test]
    fn csv_import_does_not_read_a_comma_as_a_decimal_point() {
        // `60,5` を 605 に化けさせるより、読めないと言って行を報告するほうが良い
        let db = test_db();
        let csv = "\
日付,種目,重量kg,回数
2026-08-01,ベンチプレス,\"60,5\",10
2026-08-02,ベンチプレス,60,10
";
        let (_, report) = parse(csv, &db, &mut ids()).unwrap();
        assert_eq!(report.skipped[0].reason, SkipReason::BadNumber);
    }

    // ── 落とした行は必ず報告する ─────────────────────────────────────────────
    //
    // ★ ここの 4 本が守る不変条件は 1 つ: **どの行も「取り込まれて Db に残る」か
    //   「skipped に出る」かのどちらかで、その間に落ちる隙間を作らない。**
    //   `report.taken` は「読んだ行」であって「入った行」ではないので、
    //   後段の `core::normalize` が捨てる値は行の時点で弾いておく必要がある。

    #[test]
    fn a_row_that_holds_nothing_is_reported_rather_than_counted_as_taken() {
        // 重量も回数も種目メモも無い行は `normalize` が黙って捨てる。
        // taken に数えると「取り込みました」と言われて何も入らない
        let db = test_db();
        let csv = "\
日付,種目,重量kg,回数,セットメモ
2026-08-01,ベンチプレス,,,きつかった
2026-08-02,ベンチプレス,60,10,
";
        let (parsed, report) = parse(csv, &db, &mut ids()).unwrap();
        assert_eq!(report.taken, 1);
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(report.skipped[0].reason, SkipReason::BadNumber);
        assert!(!parsed.sessions.contains_key("2026-08-01"));
    }

    #[test]
    fn a_row_that_is_reported_does_not_leave_its_body_weight_behind() {
        // ★ 「取り込めません」と言った行の体重・当日メモだけが残ると、
        //   利用者が知らない休養日がカレンダーに生える
        let db = test_db();
        let csv = "\
日付,部位,種目,重量kg,回数,体重kg,当日メモ
2026-08-01,不明,知らない種目,60,10,70.5,絶好調
2026-08-02,胸,ベンチプレス,60,10,,
";
        let (parsed, report) = parse(csv, &db, &mut ids()).unwrap();
        assert_eq!(report.skipped.len(), 1);
        assert!(
            !parsed.sessions.contains_key("2026-08-01"),
            "落とした行の日が残っている: {:?}",
            parsed.sessions.get("2026-08-01")
        );
    }

    #[test]
    fn weights_that_normalize_would_throw_away_are_reported_not_dropped() {
        // 負の重量と、f64 では有限だが f32 で inf になる値。どちらも
        // `drop_unrepresentable_weights` が捨てるので、行の時点で弾いて報告する
        let db = test_db();
        let csv = "\
日付,種目,重量kg,回数
2026-08-01,ベンチプレス,-60,10
2026-08-02,ベンチプレス,3.5e38,10
2026-08-03,ベンチプレス,60,10
";
        let (parsed, report) = parse(csv, &db, &mut ids()).unwrap();
        assert_eq!(report.taken, 1);
        assert_eq!(report.skipped.len(), 2, "捨てた 2 行が報告されていない");
        assert_eq!(parsed.sessions.len(), 1);
        assert!(parsed.sessions.contains_key("2026-08-03"));
    }

    #[test]
    fn the_reported_line_number_is_the_physical_line_in_the_file() {
        // メモに改行が入るとレコードの通し番号と物理行がずれる。ずれたまま出すと、
        // テキストエディタで開いた利用者が別の行を直しにいく
        let db = test_db();
        let csv = "\
日付,種目,重量kg,回数,当日メモ
2026-08-01,ベンチプレス,60,10,\"1 行目
2 行目
3 行目\"
2026-08-02,ベンチプレス,おもい,10,
";
        let (_, report) = parse(csv, &db, &mut ids()).unwrap();
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(
            report.skipped[0].line, 5,
            "物理行ではなくレコード番号を出している"
        );
    }

    // ── 壊れた表を壊れたまま流さない ─────────────────────────────────────────

    #[test]
    fn an_unclosed_quote_is_named_instead_of_swallowing_the_rest_of_the_file() {
        // ★ 閉じ忘れると残り全部が 1 セルに入る。それを「未登録の種目」として
        //   画面に流すと、確認画面が数千文字になるうえ診断も嘘になる
        let db = test_db();
        let csv = "\
日付,種目,重量kg,回数
2026-08-01,\"ベンチプレス,60,10
2026-08-02,ベンチプレス,60,10
";
        let err = parse(csv, &db, &mut ids()).unwrap_err();
        assert_eq!(err, SheetError::UnclosedQuote);
        assert!(err.message().contains("引用符"), "{}", err.message());
    }

    #[test]
    fn a_quote_after_a_space_still_quotes_the_field() {
        // 手で書いた CSV は区切りのあとに空白を入れがち。literal 扱いにすると
        // 中のカンマでセルが割れ、以降の列が 1 つずつずれる
        let db = test_db();
        let csv = "\
日付,種目,重量kg,回数,セットメモ,体重kg
2026-08-01,ベンチプレス,60,10, \"軽い, 余裕\",70.5
";
        let (parsed, report) = parse(csv, &db, &mut ids()).unwrap();
        assert_eq!(report.skipped, Vec::new());
        let day = &parsed.sessions["2026-08-01"];
        assert_eq!(day.logs[0].sets, vec![set(60.0, 10, "軽い, 余裕")]);
        assert_eq!(day.body_weight, Some(70.5), "列が 1 つずれている");
    }

    #[test]
    fn long_cells_are_clipped_before_they_reach_the_screen() {
        let msg = SkipReason::UnknownGroup {
            exercise: "あ".repeat(500),
            group: "不明".into(),
        }
        .message();
        assert!(
            msg.chars().count() < 60,
            "画面に流す前に丸めていない: {msg}"
        );
    }

    #[test]
    fn csv_import_accepts_a_date_column_formatted_as_a_date_time() {
        // Sheets の `/export` は表示値を返すので、日時書式の列は時刻ごと出てくる。
        // 弾くと表が丸ごと 1 行も読めない
        let db = test_db();
        for form in [
            "2026/08/01 0:00:00",
            "2026-08-01T12:00",
            "2026年8月1日 9:30",
        ] {
            let csv = format!("日付,種目,重量kg,回数\n{form},ベンチプレス,60,10\n");
            let (parsed, _) = parse(&csv, &db, &mut ids())
                .unwrap_or_else(|e| panic!("{form} が読めない: {}", e.message()));
            assert!(parsed.sessions.contains_key("2026-08-01"), "{form}");
        }
    }

    #[test]
    fn csv_import_refuses_to_pick_between_two_groups_with_the_same_name() {
        // `rename_group` にも重複チェックが無い。種目が別の「胸」に入ると
        // 部位別の集計が 2 つに割れる
        let mut db = test_db();
        db.groups.push(Group {
            id: g(9),
            name: "胸".into(),
            color: "#ffffff".into(),
            order: 2,
        });
        let csv = "\
日付,部位,種目,重量kg,回数
2026-08-01,胸,謎の種目,60,10
2026-08-01,脚,スクワット,80,5
";
        let (_, report) = parse(csv, &db, &mut ids()).unwrap();
        assert_eq!(
            report.skipped,
            vec![Skipped {
                line: 2,
                reason: SkipReason::Ambiguous { name: "胸".into() },
            }]
        );
        assert!(report.exercises_created.is_empty());
    }

    #[test]
    fn preset_ids_are_matched_through_the_same_normalization_as_names() {
        // 生の名前で引くと「デッド リフト」が固定 ID に当たらず乱数になり、
        // 同じ表を読んだ 2 台で ID が割れてグラフが 2 本になる
        let mut db = test_db();
        db.groups.push(Group {
            id: presets::preset_group_id("背中").expect("プリセットの部位"),
            name: "背中".into(),
            color: "#3b82f6".into(),
            order: 2,
        });
        let csv = "日付,部位,種目,重量kg,回数\n2026-08-01,背中,デッド リフト,100,5\n";
        let (parsed, _) = parse(csv, &db, &mut ids()).unwrap();
        let made = parsed
            .exercises
            .iter()
            .find(|x| x.name == "デッド リフト")
            .unwrap();
        assert_eq!(Some(made.id), presets::preset_exercise_id("デッドリフト"));
    }

    // ── マージとの噛み合わせ ─────────────────────────────────────────────────

    #[test]
    fn importing_the_same_sheet_twice_does_not_duplicate_sets() {
        // merge_db はセットを連結せず「強いほう」で置き換えるので冪等になる。
        // ここが崩れると、取り込むたびにセットが倍になる
        let db = db_with_records();
        let csv = export_csv(&db);

        let mut mine = db.clone();
        let (theirs, _) = parse(&csv, &mine, &mut ids()).unwrap();
        core::merge_db(&mut mine, theirs);
        let after_once = mine.clone();

        let (theirs, _) = parse(&csv, &mine, &mut ids()).unwrap();
        let report = core::merge_db(&mut mine, theirs);

        assert_eq!(mine.sessions, after_once.sessions);
        assert!(report.is_noop(), "2 回目で何かが増えている: {report:?}");
    }

    #[test]
    fn merging_a_parsed_sheet_does_not_report_renames() {
        // 読み取った表記ではなく取り込み先の Exercise をそのまま入れているので、
        // merge_db は ID 一致の枝を通る。同じものを同じと言っただけで改名は起きていない
        let db = db_with_records();
        let csv = export_csv(&db);
        let mut mine = db.clone();
        let (theirs, _) = parse(&csv, &mine, &mut ids()).unwrap();
        let report = core::merge_db(&mut mine, theirs);
        assert_eq!(report.conflicts, Vec::new(), "{:?}", report.conflicts);
    }

    #[test]
    fn a_sheet_edited_in_a_spreadsheet_adds_the_new_day_without_touching_the_old_ones() {
        let db = db_with_records();
        let mut csv = export_csv(&db);
        csv.push_str("2026-08-10,胸,ベンチプレス,1,65,10,,,71,\n");

        let mut mine = db.clone();
        let (theirs, _) = parse(&csv, &mine, &mut ids()).unwrap();
        let report = core::merge_db(&mut mine, theirs);

        assert_eq!(report.sessions_added, 1);
        assert_eq!(mine.sessions["2026-08-10"].body_weight, Some(71.0));
        assert_eq!(mine.sessions["2026-08-01"], db.sessions["2026-08-01"]);
    }

    // ── URL ─────────────────────────────────────────────────────────────────

    const ID: &str = "1BxiMVs0XRA5nFMdKvBdBZjgmUUqptlbs74OgvE2upms";

    #[test]
    fn sheet_url_extracts_id_and_gid_from_every_form_a_user_can_paste() {
        let export = format!("https://docs.google.com/spreadsheets/d/{ID}/export?format=csv");

        // アドレス欄からのコピー。gid は最後（フラグメント側）を採る
        assert_eq!(
            csv_url(&format!(
                "https://docs.google.com/spreadsheets/d/{ID}/edit?gid=123#gid=456"
            )),
            Ok(format!("{export}&gid=456"))
        );
        // 共有ダイアログの形
        assert_eq!(
            csv_url(&format!(
                "https://docs.google.com/spreadsheets/d/{ID}/edit?usp=sharing"
            )),
            Ok(export.clone())
        );
        // 末尾スラッシュだけ / 前後の空白
        assert_eq!(
            csv_url(&format!("  https://docs.google.com/spreadsheets/d/{ID}/  ")),
            Ok(export.clone())
        );
        // 「ウェブに公開」は別系統。export?format=csv を受け付けないので pub に振る
        let pub_id = "2PACX-1vQabcdefghijklmnopqrstuvwxyz0123456789";
        assert_eq!(
            csv_url(&format!(
                "https://docs.google.com/spreadsheets/d/e/{pub_id}/pubhtml?gid=7"
            )),
            // ★ 公開 URL は single=true が無いと gid を無視する
            Ok(format!(
                "https://docs.google.com/spreadsheets/d/e/{pub_id}/pub?output=csv&gid=7&single=true"
            ))
        );
    }

    #[test]
    fn sheet_url_reads_the_multi_account_form_people_actually_copy() {
        // ★ 2 つ以上の Google アカウントにログインしていると、アドレス欄は必ず
        //   この形になる。落とすと、その人たちは URL 取り込みを一切使えない
        let export = format!("https://docs.google.com/spreadsheets/d/{ID}/export?format=csv");
        assert_eq!(
            csv_url(&format!(
                "https://docs.google.com/spreadsheets/u/1/d/{ID}/edit?gid=0#gid=0"
            )),
            Ok(format!("{export}&gid=0"))
        );
        assert_eq!(
            csv_url(&format!(
                "https://docs.google.com/spreadsheets/u/0/d/{ID}/edit"
            )),
            Ok(export)
        );
        let pub_id = "2PACX-1vQabcdefghijklmnopqrstuvwxyz0123456789";
        assert_eq!(
            csv_url(&format!(
                "https://docs.google.com/spreadsheets/u/2/d/e/{pub_id}/pubhtml"
            )),
            Ok(format!(
                "https://docs.google.com/spreadsheets/d/e/{pub_id}/pub?output=csv"
            ))
        );
    }

    #[test]
    fn sheet_url_rejects_urls_that_are_not_google_sheets() {
        assert_eq!(csv_url(""), Err(SheetUrlError::Empty));
        assert_eq!(csv_url("   "), Err(SheetUrlError::Empty));
        assert_eq!(
            csv_url("https://example.com/a.csv"),
            Err(SheetUrlError::NotSheets)
        );
        // Google ドライブのフォルダ URL（スプレッドシートではない）
        assert_eq!(
            csv_url("https://drive.google.com/drive/folders/abc"),
            Err(SheetUrlError::NotSheets)
        );
        // ドキュメントの URL
        assert_eq!(
            csv_url(&format!("https://docs.google.com/document/d/{ID}/edit")),
            Err(SheetUrlError::NotSheets)
        );
        // ID が短すぎる（取り違え）
        assert_eq!(
            csv_url("https://docs.google.com/spreadsheets/d/edit"),
            Err(SheetUrlError::NoId)
        );
    }
}
