//! 記録タブの下半分。**カレンダーで選んだ 1 日**の入力欄。
//!
//! 単独のタブではなく [`super::calendar`] が自分の下に置くコンポーネント。
//! 書き込み対象の日付は `DateCtx::selected` だけが決める（カレンダーの選択日と同一）。
//!
//! 経過時間と部位チップは「今日どこを鍛えるか」を決めるための情報なので、
//! カレンダーと入力欄に挟まれた**常時 1 行**に圧縮してある。以前は種目を 1 つでも
//! 追加したら畳む排他表示にしていたが、上にカレンダーが載って画面が縦に伸びた今は
//! 大きいヒーローを置く余地が無く、出し分ける意味も無くなった。

use chrono::NaiveDate;
use leptos::prelude::*;
use web_sys::PointerEvent;

use crate::core;
use crate::core::Metric;
use crate::model::{Db, ExerciseId, ExerciseLog, GroupId, RoutineId, SetEntry};
use crate::reorder;

use super::drag::{
    self, Drag, EDGE_SCROLLING, PRESS, PRESS_DELAY_CARD, PRESS_SLOP_PX, Press, Scroller, capture,
    end_press, holds, measure_slots, release, start_edge_scroll,
};
use super::icon::{self, icon};
use super::{
    Sheet, fmt_date, fmt_metric, fmt_set, fmt_weight, kb_blur, kb_focus, now_ms, parse_reps,
    parse_weight, scroll_to_id, use_dates, use_db, use_kb,
};

/// 選択日に並べているカード 1 枚。
///
/// 日付キーを持つのは `<For>` のキーに混ぜるため。過去日へ切り替えたときにカードの
/// DOM ごと作り直さないと、カード内の編集中文字列が前の日のまま残る。
#[derive(Clone, PartialEq, Eq)]
struct CardRef {
    date: String,
    ex: ExerciseId,
}

/// 編集中のセット 1 行。
///
/// **保存モデル（`SetEntry`）ではなく文字列で持つ。** `f32` / `u32` では空欄が表現できず
/// 0 に落ちるうえ、`"6."` のような中間状態を保持できない。
#[derive(Clone, Debug, PartialEq)]
struct Row {
    key: u32,
    weight: String,
    reps: String,
    /// このセットのメモ。**保存はセットに従属する**（回数の無い行のメモは保存されない）。
    /// adr/data-model/notes-on-logs-and-sets.md
    note: String,
}

impl Row {
    fn blank(key: u32) -> Self {
        Self {
            key,
            weight: String::new(),
            reps: String::new(),
            note: String::new(),
        }
    }
}

fn card_dom_id(ex: ExerciseId) -> String {
    format!("card-{ex}")
}

/// セット行の DOM id。掴んだときに並び全体の箱を測るのに使う。
///
/// `ex` は「1 日 1 種目 1 ログ」（adr/data-model/one-log-per-exercise-per-day.md）なので
/// カードを跨いで衝突せず、`key` はカードの中で単調増加（`next_key`）なので行を跨いでも
/// 衝突しない。
fn set_dom_id(ex: ExerciseId, key: u32) -> String {
    format!("set-{ex}-{key}")
}

// ── ドラッグで並び替える（adr/ux/drag-to-reorder-in-record-tab.md）────────────
//
// 掴む / 測る / 端でスクロールする / 畳む は `views::drag` に出してある
// （メニュー編集シートの「選択中」が 3 つ目のハンドルになった時点で共有した）。
// ここに残すのは記録タブ固有のもの — DOM id の作り方と、カードの並びの読み出し。

/// 画面に並んでいるカードの順（その日のぶんだけ）。`core::reorder_logs` に渡す。
///
/// ★ `CardRef.date` で絞るのは、日付を切り替えた直後の 1 tick で `cards` が前日のままでも
/// 別の日の ID が混ざらないようにするため。**ドロップ側と `commit` の 2 か所から呼ぶので
/// 関数にしてある** — 片方だけ絞り方を変えると「`logs` の順 == `cards` の順」という
/// 不変条件が片方の経路でだけ破れる（adr/ux/drag-to-reorder-in-record-tab.md）。
fn card_order(cards: RwSignal<Vec<CardRef>>, date: NaiveDate) -> Vec<ExerciseId> {
    let key = core::date_key(date);
    cards.with_untracked(|cs| cs.iter().filter(|c| c.date == key).map(|c| c.ex).collect())
}

/// 候補リストに並べる上限。
///
/// ★ 増やさない。上にカレンダーが載っているぶん候補は最初から画面外に始まるので、
/// 行が増えるほどスクロール量が伸びる。実測（393×760・4 件）では 1 回スクロールすれば
/// 4 件と「種目を追加」が同時に収まる。4 日サイクルの分割法まではこれで足りる。
const MENU_CANDIDATES: usize = 4;
/// 1 行に出す部位名の数。溢れたら「他」を付ける。
const MENU_GROUP_CAP: usize = 3;
/// 1 行に出す種目名の数。溢れたら「他N種目」を付ける。
const MENU_NAME_CAP: usize = 2;

/// 候補リストの 1 行。**保存したメニューと過去の日で同じ形を使う。**
///
/// **種目名まで出すのが肝。** 「8/5 胸 5種目」だけでは胸の日が 2 つ並んだときに
/// 区別できず、候補をリストにした意味が消える。誤タップは事後の取り消しではなく
/// 選び間違いを起こさせないことで防ぐ。保存したメニューでも同じで、「胸の日」と
/// 「胸の日（軽め）」を名前だけで見分けさせない。
#[derive(Clone, PartialEq)]
struct MenuRow {
    /// 左上。日なら "8/5 (火)"、保存したメニューなら名前。
    title: String,
    /// "胸・腕"
    groups: String,
    /// "ベンチプレス, インクラインダンベルプレス 他3種目"
    names: String,
}

impl MenuRow {
    fn build(db: &Db, title: String, exercises: &[ExerciseId]) -> Self {
        let mut seen: Vec<GroupId> = Vec::new();
        let mut ordered: Vec<(u32, String)> = Vec::new();
        let mut names: Vec<String> = Vec::new();
        for id in exercises {
            // core 側で存在を確認済みだが、ここでも落とさず素通しできるようにしておく
            let Some(e) = db.exercise(*id) else { continue };
            names.push(e.name.clone());
            if seen.contains(&e.group_id) {
                continue;
            }
            seen.push(e.group_id);
            if let Some(g) = db.group(e.group_id) {
                ordered.push((g.order, g.name.clone()));
            }
        }
        // 部位は Group::order 順に直す。種目の入力順で並べると、同じ部位構成の日でも
        // 行ごとに並びが変わって読み比べられない
        ordered.sort_by_key(|(order, _)| *order);

        let groups = join_capped(
            ordered.iter().map(|(_, n)| n.as_str()),
            ordered.len(),
            MENU_GROUP_CAP,
            "・",
            |_| " 他".to_string(),
        );
        let names = join_capped(
            names.iter().map(String::as_str),
            names.len(),
            MENU_NAME_CAP,
            ", ",
            |rest| format!(" 他{rest}種目"),
        );
        Self {
            title,
            groups,
            names,
        }
    }
}

/// 空の日に出す候補一式。**2 つの出所を 1 つの `Memo` にまとめる。**
///
/// ★ 別々の `Memo` に割らないこと。「カードが 0 枚」「未来日ではない」の 2 つのガードは
/// どちらの出所にも等しく効かなければならず、分けると片方だけ抜けたときに
/// **未来日に実施済みのデータが生える**（adr/ux/copy-whole-day-menu.md）。
#[derive(Clone, Default, PartialEq)]
struct Candidates {
    /// 保存したトレーニングメニュー。**全件出す**（ユーザーが作ったものを隠さない）。
    routines: Vec<(RoutineId, MenuRow)>,
    /// 直近の記録。最大 [`MENU_CANDIDATES`] 件。
    days: Vec<(NaiveDate, MenuRow)>,
}

/// 先頭 `cap` 件だけ連結し、溢れた分は `overflow` の文言で畳む。
fn join_capped<'a>(
    items: impl Iterator<Item = &'a str>,
    total: usize,
    cap: usize,
    sep: &str,
    overflow: impl Fn(usize) -> String,
) -> String {
    let mut out = items.take(cap).collect::<Vec<_>>().join(sep);
    if total > cap {
        out.push_str(&overflow(total - cap));
    }
    out
}

fn confirm_dom_id(ex: ExerciseId) -> String {
    format!("confirm-{ex}")
}

/// その日・その種目のセットと種目メモを丸ごと差し替える。
///
/// ★ **刈り取りの規則は「セットもメモも無ければ落とす」の 1 本。** セットだけで
/// 判定していた形は使えない — 種目メモを打った直後のセット commit（行の ✕、`+ セット`、
/// 他の行の打鍵）でログごと消え、**打った文字が黙って消える**
/// （adr/data-model/notes-on-logs-and-sets.md）。カードの集合は `load_cards` が
/// `session.logs` から引くので、落とすとカードごとこの日から消える。
fn write_log(
    db: &mut Db,
    date: NaiveDate,
    ex: ExerciseId,
    sets: Vec<SetEntry>,
    note: String,
    is_today: bool,
) {
    let key = core::date_key(date);
    if sets.is_empty() && note.trim().is_empty() {
        if let Some(session) = db.sessions.get_mut(&key) {
            session.logs.retain(|l| l.exercise_id != ex);
        }
    } else {
        let session = db.sessions.entry(key.clone()).or_default();
        match session.logs.iter_mut().find(|l| l.exercise_id == ex) {
            Some(log) => {
                log.sets = sets;
                log.note = note;
                // ★ at は当日入力時のみ埋める。過去日バックフィルは None のまま。
                //   ここで now を入れると「最後のトレーニングから」が「たった今」になり
                //   明示要件の出力が嘘になる
                //
                //   ★ **セットが 1 本以上あるときだけ**。メモを打つのはトレーニングでは
                //     ないので、メモだけのログに時刻が入ると「その日に実施した」証拠を
                //     持ってしまう（adr/data-model/at-optional-same-day-only.md）。
                //     旧実装はセットが空なら削除枝に入っていたのでこの判定が要らなかった
                if is_today && !log.sets.is_empty() {
                    log.at = Some(now_ms());
                }
            }
            None => {
                let at = (is_today && !sets.is_empty()).then(now_ms);
                session.logs.push(ExerciseLog {
                    exercise_id: ex,
                    sets,
                    at,
                    note,
                });
            }
        }
    }
    // ログも体重もメモも無くなった日は残さない（過去日を閲覧しただけで実施日にしない）
    if db.sessions.get(&key).is_some_and(|s| s.is_empty()) {
        db.sessions.remove(&key);
    }
}

#[component]
pub fn DayEditor() -> impl IntoView {
    let db = use_db();
    let dates = use_dates();

    let cards: RwSignal<Vec<CardRef>> = RwSignal::new(Vec::new());
    // カードのドラッグ。全カードが押しのけ量を読むので `DayEditor` が持つ。
    // 長押し待ちは所有者を持たない `PRESS`（module 冒頭）に置く。
    let card_drag: RwSignal<Option<Drag>> = RwSignal::new(None);

    // ★ この画面は `mod.rs` の `match tab.get()` の枝なので、**タブを切り替えると
    //   ここごと破棄される**。ドラッグや長押しの途中で切り替えると、生き残った
    //   タイマーと rAF ループが破棄済みの `card_drag` を触って panic する
    //   （wasm では unreachable = アプリが死ぬ）。畳んでから消える。
    //   `EDGE_SCROLLING` を下ろし忘れると再入防止のフラグが立ちっぱなしになり、
    //   戻ってきても自動スクロールが二度と動かない
    on_cleanup(|| {
        end_press();
        EDGE_SCROLLING.set(false);
    });
    let sheet = RwSignal::new(false);

    // カードを Db から引き直す。
    // db は untracked で読む（1 文字打つたびにカードが作り直されるのを防ぐ）。
    // メニューを丸ごとコピーした直後にも呼ぶので、Effect の外から使える形にしてある
    let load_cards = move |date: NaiveDate| {
        let key = core::date_key(date);
        let ids = db.with_untracked(|d| {
            d.sessions
                .get(&key)
                .map(|s| s.logs.iter().map(|l| l.exercise_id).collect::<Vec<_>>())
                .unwrap_or_default()
        });
        cards.set(
            ids.into_iter()
                .map(|ex| CardRef {
                    date: key.clone(),
                    ex,
                })
                .collect(),
        );
    };

    // 日付が変わったら引き直す。追跡するのは selected だけ
    Effect::new(move |_| load_cards(dates.selected.get()));

    // ★ ヒーローと部位チップは「今日を始める前」の間隔を出す。
    //   今日のセッションを外した snapshot に core の関数をそのまま当てることで、
    //   1 セット入れた瞬間に「たった今」へ化けて意味を失うのを防ぐ。
    //   Memo なので今日のキー入力では下流が再評価されない。
    //
    //   この snapshot は今日のキーを持たないので、見つかるセッションは必ず昨日以前になる。
    //   つまり `Elapsed::days()` は常に 1 以上で、ヒーローもチップもローカル暦の日数で
    //   表示される（`humanize` の時刻粒度の分岐はここからは踏まない）。adr/data-model/elapsed-in-local-calendar-days.md 参照。
    let before_today = Memo::new(move |_| {
        let today = dates.today.get();
        db.with(|d| {
            let mut snapshot = d.clone();
            snapshot.sessions.remove(&core::date_key(today));
            snapshot
        })
    });

    let elapsed = Memo::new(move |_| {
        let today = dates.today.get();
        before_today.with(|snapshot| core::elapsed_since_last(snapshot, now_ms(), today))
    });

    // 部位チップ 1 個ぶんのデータ。
    type Chip = (String, Option<crate::core::Elapsed>);

    let all_chips = Memo::new(move |_| {
        let today = dates.today.get();
        before_today.with(|snapshot| {
            let by_group = core::elapsed_by_group(snapshot, now_ms(), today);
            let mut groups = snapshot.groups.clone();
            groups.sort_by_key(|g| g.order);
            groups
                .into_iter()
                .map(|g| {
                    let e = by_group.get(&g.id).copied();
                    (g.name, e) as Chip
                })
                .collect::<Vec<_>>()
        })
    });

    let pick = move |ex: ExerciseId| {
        sheet.set(false);
        let date = core::date_key(dates.selected.get_untracked());
        let exists = cards.with_untracked(|cs| cs.iter().any(|c| c.ex == ex));
        if !exists {
            cards.update(|cs| cs.push(CardRef { date, ex }));
        }
        // 既にあるなら新規カードを作らず既存カードへスクロールする
        scroll_to_id(card_dom_id(ex));
    };

    // 「メニューから始める」の候補。保存したメニューと直近の記録の 2 つの出所を持つ。
    //
    // カードがある日は候補を出さないので、履歴を走査せず即座に抜ける。セットの数値は
    // 1 文字ごとに commit するため、ここを通さないと打鍵のたびに `recent_menus` が走る。
    //
    // ★ 逆に**空の日では短絡しない**（`cards` が空なので下へ素通りする）。空の日の
    //   体重・メモ入力では毎文字 db を読み直すままで、そこは守っていない。候補は数件・
    //   走査も 180 日で打ち切られるので実測では問題にならないが、取り違えないこと。
    let candidates = Memo::new(move |_| {
        if !cards.get().is_empty() {
            return Candidates::default();
        }
        let before = dates.selected.get();
        // ★ 未来日には出さない。まだやっていないトレーニングが「実施済み」として
        //   カレンダーのドット・月フッタ・グラフに乗ってしまう。
        //   **保存したメニューにも同じく効く**（同じ Memo に入れてあるのはこのため）
        if before > dates.today.get() {
            return Candidates::default();
        }
        db.with(|d| Candidates {
            routines: core::usable_routines(d)
                .into_iter()
                .map(|c| {
                    let title = if c.name.trim().is_empty() {
                        "（名前なし）".to_string()
                    } else {
                        c.name
                    };
                    (c.id, MenuRow::build(d, title, &c.exercises))
                })
                .collect(),
            days: core::recent_menus(d, before, MENU_CANDIDATES)
                .into_iter()
                .map(|c| (c.date, MenuRow::build(d, fmt_date(c.date), &c.exercises)))
                .collect(),
        })
    });

    let copy_menu = move |from: NaiveDate| {
        let to = dates.selected.get_untracked();
        // ★ commit() と同じ式にする。is_past_edit() は tracked な signal を読むので
        //   クリックハンドラの中では使わない
        let is_today = to == dates.today.get_untracked();
        let at = is_today.then(now_ms);
        let mut copied = Vec::new();
        db.update(|d| copied = core::copy_day(d, from, to, at));
        // 二重タップや別候補の連打で何も起きなかったときは画面も動かさない
        if copied.is_empty() {
            return;
        }
        load_cards(to);
        // pick() と同じ。トレ中の視点を先頭種目に合わせる
        scroll_to_id(card_dom_id(copied[0]));
    };

    // 保存したメニューを展開する。セットは**種目ごとの直近の記録**から入る。
    let start_routine = move |id: RoutineId| {
        let to = dates.selected.get_untracked();
        let is_today = to == dates.today.get_untracked();
        let at = is_today.then(now_ms);
        let mut opened = Vec::new();
        db.update(|d| opened = core::apply_routine(d, id, to, at));
        if opened.is_empty() {
            return;
        }
        // ★ **`load_cards(to)` を呼んではいけない。** あちらは `session.logs` から
        //   カードを組み直すので、**履歴がまだ無い種目の空カードが全部消える**
        //   （セットが空のログは書かない仕様なので `logs` に現れない）。
        //   `apply_routine` の返り値がそのまま「その日に出すべきカード」。
        let date = core::date_key(to);
        cards.set(
            opened
                .iter()
                .map(|ex| CardRef {
                    date: date.clone(),
                    ex: *ex,
                })
                .collect(),
        );
        scroll_to_id(card_dom_id(opened[0]));
    };

    view! {
            <section class="day" data-testid="screen-day">
                // ★ 経過と部位チップは常時 1 行。カレンダーのドットは「いつやったか」しか
                //   示さないので、「どの部位が空いているか」はここでしか読めない
                <div class="hero" data-testid="hero">
                    // ラベルと値を分けて持つ（値だけを読めるようにしておく）
                    <span class="hero-elapsed">
                        "最後から "
                        <b data-testid="elapsed">
                            {move || {
                                elapsed.get().map_or_else(|| "—".to_string(), core::humanize)
                            }}
                        </b>
                    </span>
                    <div class="chips" data-testid="group-chips">
                        {move || {
                            all_chips
                                .get()
                                .into_iter()
                                .map(|(name, e)| {
                                    let label = e
                                        .map_or_else(|| "—".to_string(), core::short_elapsed);
                                    view! {
                                        <span
                                            class="chip"
                                            data-recency=core::recency_class(e)
                                            data-testid="group-chip"
                                        >
                                            <b>{name}</b>
                                            <i>{label}</i>
                                        </span>
                                    }
                                })
                                .collect::<Vec<_>>()
                        }}
                    </div>
                </div>

                <header class="day-head" class:past=move || dates.is_past_edit()>
    // ★ h1 ではなく h2。記録タブは 1 画面にカレンダーと選択日の入力欄が縦に並ぶので
    // （adr/ux/record-tab-calendar-with-day-editor.md）、h1 は上のカレンダーの月見出しが持つ。両方を h1 にすると
    // 見出しの階層が 1 画面に 2 本立ち、支援技術のアウトラインで前後関係が読めなくなる
    <h2 data-testid="today-date">{move || fmt_date(dates.selected.get())}</h2>
                    {move || {
                        if dates.is_past_edit() {
                            view! {
                                <button
                                    class="link-btn"
                                    data-testid="back-to-today"
                                    on:click=move |_| dates.back_to_today()
                                >
                                    "今日へ戻る"
                                </button>
                            }
                                .into_any()
                        } else {
                            view! { <span class="badge">"今日"</span> }.into_any()
                        }
                    }}
                </header>

                {move || {
                    dates
                        .is_past_edit()
                        .then(|| {
                            view! {
                                <p class="past-banner" data-testid="past-banner">
                                    {move || format!("{} を編集中", fmt_date(dates.selected.get()))}
                                </p>
                            }
                        })
                }}

                // 体重・体調メモ。日付が変わったら初期値ごと作り直す
                {move || {
                    let _ = dates.selected.get();
                    view! { <ConditionRow /> }
                }}

                <div
                    class="cards"
                    data-dragging=move || card_drag.with(|d| d.is_some().then_some("true"))
                >
                    <For
                        each=move || cards.get()
                        key=|c| (c.date.clone(), c.ex)
                        children=move |c| {
                            view! {
                                <ExerciseCard ex=c.ex cards=cards card_drag=card_drag />
                            }
                        }
                    />
                </div>

                // ★ 空の日にしか出さない。カードが 1 枚でもある状態でコピーすると
                //   <For> がカードを使い回して入力欄が古いまま残り、次の 1 打鍵の
                //   commit() がコピー結果を上書きして消す。表示条件は見た目の話ではない
                {move || {
                    let c = candidates.get();
                    if c.routines.is_empty() && c.days.is_empty() {
                        return None;
                    }
                    // ★ メニューが 1 本も無い人の画面は**今までと 1 文字も変えない**。
                    //   見出しを 2 つに割るのは、並ぶ出所が実際に 2 つあるときだけ
                    let split = !c.routines.is_empty();
                    Some(
                        view! {
                            <section class="menu-copy" data-testid="menu-copy">
                                {split
                                    .then(|| {
                                        view! {
                                            <h3 class="menu-copy-label">"トレーニングメニュー"</h3>
                                        }
                                    })}
                                {c
                                    .routines
                                    .into_iter()
                                    .map(|(id, r)| {
                                        view! {
                                            <button
                                                class="menu-cand"
                                                data-testid="routine-candidate"
                                                on:click=move |_| start_routine(id)
                                            >
                                                <span class="cand-head">
                                                    <b>{r.title}</b>
                                                    <i>{r.groups}</i>
                                                </span>
                                                <span class="cand-names">{r.names}</span>
                                            </button>
                                        }
                                    })
                                    .collect::<Vec<_>>()}
                                {(!c.days.is_empty())
                                    .then(|| {
                                        view! {
                                            <h3 class="menu-copy-label">
                                                {if split { "最近の記録から" } else { "前回のメニューから始める" }}
                                            </h3>
                                        }
                                    })}
                                {c
                                    .days
                                    .into_iter()
                                    .map(|(from, r)| {
                                        view! {
                                            <button
                                                class="menu-cand"
                                                data-testid="menu-candidate"
                                                on:click=move |_| copy_menu(from)
                                            >
                                                <span class="cand-head">
                                                    <b>{r.title}</b>
                                                    <i>{r.groups}</i>
                                                </span>
                                                <span class="cand-names">{r.names}</span>
                                            </button>
                                        }
                                    })
                                    .collect::<Vec<_>>()}
                            </section>
                        },
                    )
                }}

                <div class="add-wrap">
                    <button
                        class="primary"
                        data-testid="add-exercise"
                        on:click=move |_| sheet.set(true)
                    >
                        "種目を追加"
                    </button>
                </div>

                <Sheet
                    open=sheet
                    on_close=Callback::new(move |_| sheet.set(false))
                    title="種目を追加".to_string()
                    testid="add-sheet"
                    close_testid="add-sheet-close"
                >
                    {move || {
                        db.with(|d| {
                            let mut groups = d.groups.clone();
                            groups.sort_by_key(|g| g.order);
                            groups
                                .into_iter()
                                .map(|g| {
                                    let mut exercises: Vec<_> = d
                                        .exercises
                                        .iter()
                                        .filter(|e| e.group_id == g.id && !e.archived)
                                        .cloned()
                                        .collect();
                                    exercises.sort_by_key(|e| e.order);
                                    view! {
                                        <section class="sheet-group">
                                            <h3 style=format!("--dot:{}", g.color)>{g.name}</h3>
                                            <div class="pick-list">
                                                {exercises
                                                    .into_iter()
                                                    .map(|e| {
                                                        let id = e.id;
                                                        view! {
                                                            <button
                                                                class="pick"
                                                                // ★ 追跡する（`with_untracked` にしない）。
                                                                //   シートは常時マウントなので、開いた瞬間に
                                                                //   作り直されることを当てにできない
                                                                class:added=move || {
                                                                    cards.with(|cs| cs.iter().any(|c| c.ex == id))
                                                                }
                                                                data-testid="pick-exercise"
                                                                on:click=move |_| pick(id)
                                                            >
                                                                {e.name}
                                                            </button>
                                                        }
                                                    })
                                                    .collect::<Vec<_>>()}
                                            </div>
                                        </section>
                                    }
                                })
                                .collect::<Vec<_>>()
                        })
                    }}
                </Sheet>
            </section>
        }
}

/// 体重・体調メモの折りたたみ 1 行。
#[component]
fn ConditionRow() -> impl IntoView {
    let db = use_db();
    let dates = use_dates();
    let kb = use_kb();

    let date_key = core::date_key(dates.selected.get_untracked());
    let (weight0, note0) = db.with_untracked(|d| {
        d.sessions
            .get(&date_key)
            .map(|s| {
                (
                    s.body_weight.map(fmt_weight).unwrap_or_default(),
                    s.note.clone(),
                )
            })
            .unwrap_or_default()
    });

    let open = RwSignal::new(!weight0.is_empty() || !note0.is_empty());
    let weight = RwSignal::new(weight0.clone());
    let note = RwSignal::new(note0.clone());

    // このコンポーネントは日付が変わるたび作り直されるので、キーはその都度引き直せばよい
    // （`String` を capture すると commit が Copy でなくなり view! の FnMut 要件に落ちる）
    let commit = move || {
        let body_weight = weight.with_untracked(|w| {
            w.trim()
                .replace(',', ".")
                .parse::<f32>()
                .ok()
                .filter(|v| v.is_finite() && *v > 0.0)
        });
        let text = note.get_untracked();
        let key = core::date_key(dates.selected.get_untracked());
        db.update(move |d| {
            {
                let session = d.sessions.entry(key.clone()).or_default();
                session.body_weight = body_weight;
                session.note = text;
            }
            if d.sessions.get(&key).is_some_and(|s| s.is_empty()) {
                d.sessions.remove(&key);
            }
        });
    };

    view! {
        <div class="condition" data-testid="condition">
            <button
                class="link-btn cond-toggle"
                data-testid="condition-toggle"
                on:click=move |_| open.update(|o| *o = !*o)
            >
                {move || if open.get() { "－ コンディション" } else { "＋ コンディション" }}
            </button>
            {move || {
                open
                    .get()
                    .then(|| {
                        view! {
                            <div class="cond-fields">
                                <label>
                                    "体重"
                                    <input
                                        type="text"
                                        inputmode="decimal"
                                        pattern="[0-9]*([.,][0-9]*)?"
                                        value=weight0.clone()
                                        data-testid="body-weight"
                                        on:focusin=move |_| kb_focus(kb)
                                        on:focusout=move |_| kb_blur(kb)
                                        on:input=move |ev| {
                                            weight.set(event_target_value(&ev));
                                            commit();
                                        }
                                    />
                                    <span class="unit">"kg"</span>
                                </label>
                                <label class="note">
                                    "メモ"
                                    <input
                                        type="text"
                                        value=note0.clone()
                                        data-testid="condition-note"
                                        on:focusin=move |_| kb_focus(kb)
                                        on:focusout=move |_| kb_blur(kb)
                                        on:input=move |ev| {
                                            note.set(event_target_value(&ev));
                                            commit();
                                        }
                                    />
                                </label>
                            </div>
                        }
                    })
            }}
        </div>
    }
}

#[component]
fn ExerciseCard(
    ex: ExerciseId,
    cards: RwSignal<Vec<CardRef>>,
    card_drag: RwSignal<Option<Drag>>,
) -> impl IntoView {
    let db = use_db();
    let dates = use_dates();
    let kb = use_kb();

    // Memo にするのは「値が変わったときだけ」下流を再描画させるため。
    // 素の closure だと db が動くたびに構造ごと作り直され、入力中の文字列が消える
    let name = Memo::new(move |_| {
        db.with(|d| d.exercise(ex).map(|e| e.name.clone()))
            .unwrap_or_else(|| "(削除された種目)".to_string())
    });
    let group_name = Memo::new(move |_| {
        db.with(|d| {
            d.exercise(ex)
                .and_then(|e| d.group(e.group_id))
                .map(|g| g.name.clone())
        })
        .unwrap_or_default()
    });

    let last = Memo::new(move |_| {
        let before = dates.selected.get();
        db.with(|d| core::last_log_before(d, ex, before).map(|(date, l)| (date, l.clone())))
    });

    // ★「前回をコピー」はその種目の今日のセットが空のときだけ出す。
    //   これで「置換か追記か」の曖昧さ・誤タップで入力済みセットを消す事故・undo の
    //   必要性がまとめて消える
    let show_copy = Memo::new(move |_| {
        let key = core::date_key(dates.selected.get());
        last.with(|l| l.is_some())
            && db.with(|d| {
                d.sessions
                    .get(&key)
                    .and_then(|s| s.log_of(ex))
                    .is_none_or(|l| l.sets.is_empty())
            })
    });

    let (initial, note0): (Vec<Row>, String) = db.with_untracked(|d| {
        d.sessions
            .get(&core::date_key(dates.selected.get_untracked()))
            .and_then(|s| s.log_of(ex))
            .map(|l| {
                let rows: Vec<Row> = l
                    .sets
                    .iter()
                    .enumerate()
                    .map(|(i, s)| Row {
                        key: i as u32,
                        weight: fmt_weight(s.weight),
                        reps: s.reps.to_string(),
                        note: s.note.clone(),
                    })
                    .collect();
                (rows, l.note.clone())
            })
            .unwrap_or_default()
    });
    let next_key = RwSignal::new(initial.len() as u32 + 1);
    let rows = RwSignal::new(if initial.is_empty() {
        vec![Row::blank(0)]
    } else {
        initial
    });
    // その日のその種目のメモ。
    let ex_note = RwSignal::new(note0);

    // 「+ セット」で足した行。この行の回数欄へフォーカスを移したら None に戻す。
    let focus_key: RwSignal<Option<u32>> = RwSignal::new(None);
    // セット行のドラッグ。カードの中で閉じるので `ExerciseCard` が持つ。
    let row_drag: RwSignal<Option<Drag>> = RwSignal::new(None);
    // この種目をこの日から外す確認を出しているか。
    let confirm_close = RwSignal::new(false);
    // メモ欄（種目メモ + 全セット行のメモ）を開いているか。
    //
    // ★ **既定は常に閉。`ConditionRow` の「値があれば最初から開く」は継がない。**
    //   あちらが自動で開くのは、閉じているとき値がまったく見えないから。こちらは
    //   閉じたまま薄字で読めるので、その必要が消えている。継ぐと、一度メモを書いた種目が
    //   永久に入力欄 N+1 本ぶん高いカードになり（3 セットで約 200px）、それが日をまたいで
    //   復活する。コンディションは 1 日 1 個、種目カードは 1 日 5〜8 個なので効き方が違う。
    //
    // ★ `storage::UiState` に入れない。あのキーの前提は「`Db` を参照しないフラグ」で、
    //   種目ごとの開閉は `ExerciseId` の集合 = `Db` 参照になり、種目を消すと dangling する。
    //   リロードで閉じるのは仕様（薄字が残るので情報は失われない）。
    //   adr/ux/exercise-and-set-notes-behind-one-toggle.md
    let note_open = RwSignal::new(false);

    // ★ この種目が普段「重量を使う」種目かを**実データから**判定する。
    //
    // 旧実装は `Kind::Weighted` で判定していたが `Kind` は無くなった。単に
    // 判定を消すわけにいかないのは、新しい指標が「重量が空 = 重量 1」だから。
    // ベンチプレスで重量を入れ忘れると 600 が黙って 10 になり、グラフが崩れても
    // 気づく手掛かりが無い。逆に全種目で警告を出すと懸垂 12 回の毎行に
    // 「重量未入力」が出て邪魔になる。
    //
    // 前回ログか、この日の他の行に重量が入っていれば「重量を使う種目」とみなす。
    let uses_weight = Memo::new(move |_| {
        last.with(|l| {
            l.as_ref()
                .is_some_and(|(_, log)| log.sets.iter().any(|s| s.weight > 0.0))
        }) || rows.with(|rs| rs.iter().any(|r| parse_weight(&r.weight) > 0.0))
    });

    let commit = move || {
        let sets: Vec<SetEntry> = rows.with_untracked(|rs| {
            rs.iter()
                .filter_map(|r| {
                    // 空行が 0×0 として保存されコピーで複製され続けるのを防ぐ。
                    // ★ メモだけの行もここで落ちる。通すと 0×0 のゴーストセットが
                    //   生まれて指標・カレンダーのドット・コピーが全部汚染される
                    //   （adr/ux/text-input-not-number.md が潰した経路の復活）。
                    //   セットに紐付かないメモの置き場所は種目メモとして用意してあり、
                    //   落ちることは行内の `.warn` で見せる（note_orphan）
                    let reps = parse_reps(&r.reps)?;
                    Some(SetEntry {
                        weight: parse_weight(&r.weight),
                        reps,
                        note: r.note.clone(),
                    })
                })
                .collect()
        });
        let note = ex_note.get_untracked();
        let date = dates.selected.get_untracked();
        let is_today = date == dates.today.get_untracked();
        // ★ 画面の並びを唯一の真実にする。`write_log` の新規枝は `logs.push` なので、
        //   **並び替えたあと「まだログの無いカード」に 1 文字目を打つと、そのログだけ
        //   末尾に生える**。書き込みのたびに揃え直すことで「`logs` の順 == `cards` の順」が
        //   不変条件になる（adr/ux/drag-to-reorder-in-record-tab.md）。
        //   `CardRef.date` で絞るのは、日付を切り替えた直後の 1 tick で `cards` が
        //   前日のままでも別の日の ID が混ざらないようにするため
        let order = card_order(cards, date);
        db.update(|d| {
            write_log(d, date, ex, sets, note, is_today);
            core::reorder_logs(d, date, &order);
        });
    };

    let fresh_key = move || {
        let key = next_key.get_untracked();
        next_key.set(key + 1);
        key
    };

    // ★ 直前行の重量をコピーして足し、回数欄へフォーカスする。
    //
    // セットは 60×10 / 60×8 / 60×6 のように重量を据え置いて回数だけ変えるのが普通なので、
    // 「+ セット」→ 重量欄をタップ →打つ→ 回数欄をタップ →打つ、の 4 手のうち
    // 前半 2 手が毎セット無駄になっていた。プリフィル + フォーカスで回数を打つだけになる。
    let add_row = move |_| {
        let key = fresh_key();
        let weight =
            rows.with_untracked(|rs| rs.last().map(|r| r.weight.clone()).unwrap_or_default());
        rows.update(|rs| {
            rs.push(Row {
                key,
                weight,
                reps: String::new(),
                // ★ メモはプリフィルしない。重量を引き継ぐのはそれが**次のセットの計画値**
                //   だからで、メモは**そのセットで起きたことの観測値**。コピーすると
                //   起きていない観測を捏造する（copy_day が `at` を持ち込まないのと同型）
                note: String::new(),
            })
        });
        focus_key.set(Some(key));
        // 重量だけの行は parse_reps が None を返して保存されないので commit は要らない
    };

    // ★ 確認を挟まない（adr/ux/set-delete-without-confirmation.md）。消えるのは 1 行で打ち直しは数秒、対してトレ中は
    //   1 種目に 3〜5 行あり打ち間違いの消し直しも含めれば何度も踏む。確認のコストのほうが
    //   失うものより高い。**カード削除（confirm_close）の確認は残す** — スコープが違う。
    let remove_row = move |key: u32| {
        rows.update(|rs| rs.retain(|r| r.key != key));
        // 行が 0 本になると入力欄ごと消えるので、必ず空行を 1 本残す
        if rows.with_untracked(Vec::is_empty) {
            let key = fresh_key();
            rows.update(|rs| rs.push(Row::blank(key)));
        }
        commit();
    };

    // ★ 行を動かすのは `rows` の入れ替え + いつもの `commit()` だけ。
    //   `log.sets` を添字で並べ替える経路は**作らない** — `commit` は
    //   `parse_reps` が None の行（空行・メモだけの行）を落とすので、`rows` と
    //   `log.sets` は添字が対応しない（adr/ux/drag-to-reorder-in-record-tab.md）。
    //
    // ★ `Row.key` を振り直さないこと。キーが変わらなければ tachys の keyed diff が
    //   既存 DOM を move するので、入力欄の値・IME の変換中の文字・`focus_key` の
    //   Effect が全部生きたまま行だけが動く。
    let move_row = move |from: usize, to: usize| {
        if from == to {
            return;
        }
        rows.update(|rs| reorder::move_item(rs, from, to));
        commit();
    };

    let copy_last = move |_| {
        let Some((_, log)) = last.get_untracked() else {
            return;
        };
        // ★ 新しいキーを振る。既存キーを再利用すると <For> が DOM を作り直さないため
        //   入力欄の value が古いまま残る
        let base = next_key.get_untracked();
        let filled: Vec<Row> = log
            .sets
            .iter()
            .enumerate()
            .map(|(i, s)| Row {
                key: base + i as u32,
                weight: fmt_weight(s.weight),
                reps: s.reps.to_string(),
                // ★ 前回のメモは持ち込まない。「肩に違和感」は前回の観測で今日の観測では
                //   ない。core::copy_day（メニューの丸ごとコピー）と同じ規則
                note: String::new(),
            })
            .collect();
        next_key.set(base + filled.len() as u32 + 1);
        rows.set(filled);
        commit();
    };

    // ★ 保存済みセットが 1 つも無いカードは、消えるものが無いので確認を挟まない。
    //   行削除の「空行は確認しない」（adr/ux/set-entry-prefill-and-focus.md）をカードへ広げたもの。空カードに
    //   「この日の記録が消えます」と出すのは嘘だし、シートで種目を押し間違えた直後の
    //   取り消しが最も多い用途なので、そこに確認を挟むと邪魔でしかない。
    //   重量だけ打って回数が空の行は保存されない（parse_reps が None）ので「空」に入る。
    //
    //   ★ **種目メモも数える。** セットだけで判定すると、メモだけのカードが「空」扱いで
    //     確認なしに消え、書いたメモが黙って捨てられる。セットメモは保存済みセットにしか
    //     存在しないので `!l.sets.is_empty()` 側で覆える。
    let has_content = move || {
        let key = core::date_key(dates.selected.get_untracked());
        db.with_untracked(|d| {
            d.sessions
                .get(&key)
                .and_then(|s| s.log_of(ex))
                .is_some_and(|l| !l.is_empty())
        })
    };

    let close_card = move || {
        let date = dates.selected.get_untracked();
        // ★ メモも空で渡す。セットだけ空にしても、メモが残っているとログが残って
        //   「この日から外す」が効かない（write_log の刈り取りは両方を見る）
        db.update(|d| write_log(d, date, ex, Vec::new(), String::new(), false));
        cards.update(|cs| cs.retain(|c| c.ex != ex));
    };

    let request_close = move |_| {
        if !has_content() {
            close_card();
            return;
        }
        confirm_close.set(true);
        // ★ 確認はカード末尾に出るので、sticky の「種目を追加」の背後に
        //   入って見えないことがある。開いたら必ず視界へ送る
        scroll_to_id(confirm_dom_id(ex));
    };

    let today_metric = move || {
        let key = core::date_key(dates.selected.get());
        db.with(|d| {
            d.sessions
                .get(&key)
                .and_then(|s| s.log_of(ex))
                .map_or(0.0, |l| core::log_value(Metric::Volume, l))
        })
    };

    // ── カードのドラッグ ──────────────────────────────────────────────────
    //
    // 掴む場所は見出し行（`.card-head`）。ここにはボタンが 1 つも無く
    // （adr/ux/destructive-affordance-quiet-at-rest.md）、カード幅いっぱいの帯なので
    // 新しい要素を足さずに掴める。

    // このカードの、`cards` の中での位置。
    let card_slot = move || cards.with(|cs| cs.iter().position(|c| c.ex == ex));

    // ★ 画面を動かしてから `Db` を揃える。`cards` が真実源で、`reorder_logs` は
    //   そこに追いつくだけ（`commit()` からも同じことを呼ぶので冪等）。
    //   ★ ドロップ側と `commit` の**両方**が要る — ドロップだけだと未 commit の
    //     カードが後で末尾に生え、`commit` だけだと 1 文字も打たない並び替えが
    //     保存されない
    let move_card = move |from: usize, to: usize| {
        if from == to {
            return;
        }
        cards.update(|cs| reorder::move_item(cs, from, to));
        let date = dates.selected.get_untracked();
        let order = card_order(cards, date);
        db.update(|d| {
            core::reorder_logs(d, date, &order);
        });
    };

    // 長押しが満了したら、**そのときの指の位置**を基準に掴む。
    //
    // ★ 押した瞬間の位置を基準にしないこと。待っている間の 0〜10px ぶん、掴んだ
    //   瞬間にカードが跳ねる。測るのもここ（押した瞬間ではない）。
    let arm_card = move || {
        let Some(p) = PRESS.get() else {
            return;
        };
        // ★ `try_` で読む。長押しの途中でタブを切り替えると `DayEditor` ごと破棄され、
        //   このタイマーだけが生き残る。`on_cleanup` でタイマーを止めてはいるが、
        //   既にキューへ入った 1 発は止められないので、ここでも受け止める
        let Some((ids, here)) = cards.try_with_untracked(|cs| {
            (
                cs.iter().map(|c| card_dom_id(c.ex)).collect::<Vec<_>>(),
                cs.iter().position(|c| c.ex == ex),
            )
        }) else {
            return;
        };
        // ★ 記録タブはページ全体がスクロール容器。シートの中で掴む
        //   `views::routine` だけが `Scroller::of` で `.sheet-body` を引く
        let (Some(from), Some(slots)) = (here, measure_slots(&ids, &Scroller::Window)) else {
            return;
        };
        if card_drag
            .try_set(Some(Drag::start(
                p.pointer_id,
                from,
                p.last_y,
                slots,
                Scroller::Window,
            )))
            .is_none()
        {
            start_edge_scroll(card_drag);
        }
    };

    let grab_card = move |ev: PointerEvent| {
        if card_drag.with_untracked(Option::is_some) || !capture(&ev) {
            return;
        }
        // ★ **前の待ちが残っていても弾かずに畳んで立て直す。** `capture` が非 primary と
        //   左ボタン以外を落としているので、ここまで来たのは必ず新しいジェスチャである。
        //   「残っていたら何もしない」形にすると、`pointerup` を 1 度でも取りこぼしたとき
        //   （タブ切替で画面ごと消える等）**以後カードが二度と掴めなくなる**
        end_press();
        // ★ capture は**待つ前に**取る。取らないと、待っている 250ms の間に指が
        //   ハンドルの外へ出たとき `pointermove` が届かず slop を判定できない
        let y = f64::from(ev.client_y());
        let timer = set_timeout_with_handle(arm_card, PRESS_DELAY_CARD).ok();
        PRESS.set(Some(Press {
            pointer_id: ev.pointer_id(),
            down_y: y,
            last_y: y,
            timer,
        }));
    };

    let track_card = move |ev: PointerEvent| {
        if holds(card_drag, &ev) {
            card_drag.update(|d| {
                if let Some(d) = d {
                    d.advance(f64::from(ev.client_y()));
                }
            });
            return;
        }
        // まだ長押し待ち。動きすぎたらこのジェスチャは捨てる
        let Some(mut p) = PRESS.get().filter(|p| p.pointer_id == ev.pointer_id()) else {
            return;
        };
        p.last_y = f64::from(ev.client_y());
        if (p.last_y - p.down_y).abs() > PRESS_SLOP_PX
            && let Some(timer) = p.timer.take()
        {
            timer.clear();
        }
        PRESS.set(Some(p));
    };

    let drop_card = move |ev: PointerEvent| {
        end_press();
        let Some(d) = card_drag.get_untracked() else {
            return;
        };
        if d.pointer_id != ev.pointer_id() {
            return;
        }
        card_drag.set(None);
        move_card(d.from, d.to);
    };

    let cancel_card = move |_: PointerEvent| {
        end_press();
        release(card_drag);
    };

    // ドラッグの代わり。カードの中のどこにフォーカスがあっても効く（行の入力欄は
    // 自分で `stop_propagation` するので、ここへは届かない）
    let nudge_card = move |ev: web_sys::KeyboardEvent| {
        let Some(up) = drag::alt_arrow(&ev) else {
            return;
        };
        ev.prevent_default();
        let Some(from) = card_slot() else { return };
        let len = cards.with_untracked(Vec::len);
        move_card(from, reorder::neighbor(from, up, len));
    };

    view! {
        <article
            class="card"
            id=card_dom_id(ex)
            data-testid="exercise-card"
            data-drag=move || {
                card_drag.with(|d| (d.as_ref()?.from == card_slot()?).then_some("lift"))
            }
            style:transform=move || card_drag.with(|d| d.as_ref()?.transform(card_slot()?))
            on:keydown=nudge_card
        >
            // ★ 見出しに削除ボタンを置かない。カードの一番上・右端は
            //   「種目を追加」を探して下スクロールする指が最初に触る位置で、
            //   追加しようとして種目ごと消す事故が起きていた。導線はフッタの左端へ
            //   （カードの右端は「行の ✕ → + セット → sticky の種目を追加」が並ぶ列なので、
            //   そこには置かない。詳細は adr/ux/destructive-affordance-quiet-at-rest.md）
            // ★ 掴む場所。`<button>` にはできない — `<button>` の content model は
            //   phrasing content なので `<h3>` を入れると不正な HTML になり、`role="button"`
            //   にすると children presentational で見出しが a11y ツリーから消えて
            //   adr/ux/focus-ring-and-heading-order.md の見出し階層が壊れる。
            //   キーボードからの並び替えは下の `on:keydown`（Alt + ↑↓）が受ける。
            <header
                class="card-head"
                data-testid="card-handle"
                on:pointerdown=grab_card
                on:pointermove=track_card
                on:pointerup=drop_card
                on:pointercancel=cancel_card
                on:lostpointercapture=cancel_card
            >
                // 選択日（h2）の下にぶら下がる種目なので h3
                <h3 data-testid="card-name">{move || name.get()}</h3>
                <span class="group-name">{move || group_name.get()}</span>
            </header>

            <div class="last-row">
                {move || match last.get() {
                    None => view! { <span class="muted" data-testid="last-log">"前回 —"</span> }.into_any(),
                    Some((date, log)) => {
                        let days = (dates.selected.get() - date).num_days();
                        let when = core::humanize_days(days);
                        let sets = log.sets.iter().map(fmt_set).collect::<Vec<_>>().join("  ");
                        let metric = fmt_metric(core::log_value(Metric::Volume, &log));
                        view! {
                            <span class="when" data-testid="last-log">{format!("前回 {when}")}</span>
                            <span class="sets">{sets}</span>
                            <span class="metric">{metric}</span>
                        }
                            .into_any()
                    }
                }}
            </div>

            {move || {
                show_copy
                    .get()
                    .then(|| {
                        view! {
                            <button
                                class="secondary copy"
                                data-testid="copy-last"
                                on:click=copy_last
                            >
                                "前回をコピー"
                            </button>
                        }
                    })
            }}

            // ★ transition は「ドラッグ中の親」にだけ生やす（styles.css の該当節）。
            //   ここが常に付いていると、指を離した瞬間に「<For> の DOM move」と
            //   「transform の除去」が同時に起きて、行が 1 スロット飛んでから滑って戻る
            <div
                class="sets-editor"
                data-dragging=move || row_drag.with(|d| d.is_some().then_some("true"))
            >
                <For
                    each=move || rows.get()
                    key=|r| r.key
                    children=move |row| {
                        let key = row.key;
                        // 並びの中での位置。`slot` は 0 起点で模型上の位置、`index` は
                        // 1 起点の表示用。
                        //
                        // ★ 表示は**画面で見えている位置**を出す。ドラッグ中は `Vec` を
                        //   入れ替えないので模型の添字とずれ、そのまま描くと「2 番目に
                        //   見えている行に 1 と書いてある」状態になる。番号をハンドルに
                        //   した理由（番号 ＝ 順番）が指を離すまで嘘になるので通す。
                        //   落ちる先が中点を越えた瞬間に番号が変わり、先読みもできる
                        let slot = move || rows.with(|rs| rs.iter().position(|r| r.key == key));
                        let index = move || {
                            let Some(i) = slot() else { return 0 };
                            row_drag.with(|d| d.as_ref().map_or(i, |d| d.seen_at(i))) + 1
                        };
                        // 重量を使う種目で reps だけ入っている行は「入力忘れ」の可能性が高い。
                        // 黙って指標を変えず、行にヒントを出して保持する
                        let weight_missing = move || {
                            uses_weight.get()
                                && rows.with(|rs| {
                                    rs.iter()
                                        .find(|r| r.key == key)
                                        .is_some_and(|r| {
                                            parse_reps(&r.reps).is_some()
                                                && parse_weight(&r.weight) <= 0.0
                                        })
                                })
                        };
                        let note_of = move || {
                            rows.with(|rs| {
                                rs.iter()
                                    .find(|r| r.key == key)
                                    .map(|r| r.note.clone())
                                    .unwrap_or_default()
                            })
                        };
                        // メモは入っているが回数が空 = commit で落ちる行。
                        // weight_missing の完全な対称（あちらは reps あり、こちらは reps なし）
                        let note_orphan = move || {
                            rows.with(|rs| {
                                rs.iter()
                                    .find(|r| r.key == key)
                                    .is_some_and(|r| {
                                        !r.note.trim().is_empty()
                                            && parse_reps(&r.reps).is_none()
                                    })
                            })
                        };
                        // ★ 「+ セット」で足した行の回数欄へフォーカスを移す。
                        //   iOS はユーザー操作起点のタスク内でしか focus() でキーボードを
                        //   開かないので、set_timeout を挟まず Effect（マイクロタスク）で
                        //   完結させる。仮に開かなくても重量はプリフィル済みなので
                        //   タップ 1 回で回数を打てる
                        let reps_ref = NodeRef::<leptos::html::Input>::new();
                        Effect::new(move |_| {
                            if focus_key.get() == Some(key)
                                && let Some(el) = reps_ref.get()
                            {
                                let _ = el.focus();
                                focus_key.set(None);
                            }
                        });
                        // ★ 掴む場所は左端のセット番号。新しいボタンを足さないための
                        //   選択で、「番号 ＝ 順番」なので意味も一致する
                        //   （adr/ux/drag-to-reorder-in-record-tab.md）
                        let grab = move |ev: PointerEvent| {
                            if row_drag.with_untracked(Option::is_some) || !capture(&ev) {
                                return;
                            }
                            // ★ id の並びと `from` は同じ 1 回の読み出しから作る。
                            //   別々に読むと、その間に行が増減したとき添字がずれる
                            let (ids, here) = rows
                                .with_untracked(|rs| {
                                    (
                                        rs.iter().map(|r| set_dom_id(ex, r.key)).collect::<Vec<_>>(),
                                        rs.iter().position(|r| r.key == key),
                                    )
                                });
                            let (Some(from), Some(slots)) = (
                                here,
                                measure_slots(&ids, &Scroller::Window),
                            ) else {
                                // 測れないまま始めると、押しのけ量が 1 つずれた並びが
                                // それらしく動いてしまう。掴まないほうがまし
                                return;
                            };
                            row_drag
                                .set(
                                    Some(
                                        Drag::start(
                                            ev.pointer_id(),
                                            from,
                                            f64::from(ev.client_y()),
                                            slots,
                                            Scroller::Window,
                                        ),
                                    ),
                                );
                            start_edge_scroll(row_drag);
                        };
                        let track = move |ev: PointerEvent| {
                            if !holds(row_drag, &ev) {
                                return;
                            }
                            row_drag
                                .update(|d| {
                                    if let Some(d) = d {
                                        d.advance(f64::from(ev.client_y()));
                                    }
                                });
                        };
                        // ドラッグの代わり。入力欄にフォーカスがあるまま行を動かせる
                        let nudge_row = move |ev: web_sys::KeyboardEvent| {
                            let Some(up) = drag::alt_arrow(&ev) else { return };
                            ev.prevent_default();
                            // ★ カード側の同じハンドラに届かせない。届くと 1 打鍵で
                            //   行とカードが同時に動く
                            ev.stop_propagation();
                            let Some(from) = rows
                                .with_untracked(|rs| rs.iter().position(|r| r.key == key))
                            else {
                                return;
                            };
                            let len = rows.with_untracked(Vec::len);
                            move_row(from, reorder::neighbor(from, up, len));
                        };
                        let drop_row = move |ev: PointerEvent| {
                            let Some(d) = row_drag.get_untracked() else { return };
                            if d.pointer_id != ev.pointer_id() {
                                return;
                            }
                            row_drag.set(None);
                            // ★ 落ちた先が元の位置なら signal にも db にも触らない。
                            //   触ると打鍵していないのに保存の debounce が再武装される。
                            //   タップで並びが変わらないのは閾値ではなくこの分岐が保証する
                            move_row(d.from, d.to);
                        };
                        view! {
                            <div
                                class="set-row"
                                id=set_dom_id(ex, key)
                                data-testid="set-row"
                                data-drag=move || {
                                    row_drag
                                        .with(|d| {
                                            (d.as_ref()?.from == slot()?).then_some("lift")
                                        })
                                }
                                style:transform=move || {
                                    row_drag.with(|d| d.as_ref()?.transform(slot()?))
                                }
                            >
                                <span
                                    class="set-no"
                                    data-testid="set-handle"
                                    on:pointerdown=grab
                                    on:pointermove=track
                                    on:pointerup=drop_row
                                    on:pointercancel=move |_| release(row_drag)
                                    on:lostpointercapture=move |_| release(row_drag)
                                >
                                    {index}
                                </span>
                                // 重量欄は常に出す。空のままなら重量 1 として数えられるので、
                                // 自重種目でも時間種目でも「入れなければよい」で成立する
                                <input
                                    class="num"
                                    type="text"
                                    inputmode="decimal"
                                    pattern="[0-9]*([.,][0-9]*)?"
                                    value=row.weight.clone()
                                    aria-label="重量"
                                    data-testid="set-weight"
                                    on:keydown=nudge_row
                                    on:focusin=move |_| kb_focus(kb)
                                    on:focusout=move |_| kb_blur(kb)
                                    on:input=move |ev| {
                                        let v = event_target_value(&ev);
                                        rows.update(|rs| {
                                            if let Some(r) = rs.iter_mut().find(|r| r.key == key) {
                                                r.weight = v;
                                            }
                                        });
                                        commit();
                                    }
                                />
                                <span class="unit">"kg"</span>
                                <span class="times">"×"</span>
                                <input
                                    class="num"
                                    type="text"
                                    inputmode="numeric"
                                    value=row.reps.clone()
                                    aria-label="回数"
                                    data-testid="set-reps"
                                    node_ref=reps_ref
                                    on:keydown=nudge_row
                                    on:focusin=move |_| kb_focus(kb)
                                    on:focusout=move |_| kb_blur(kb)
                                    on:input=move |ev| {
                                        let v = event_target_value(&ev);
                                        rows.update(|rs| {
                                            if let Some(r) = rs.iter_mut().find(|r| r.key == key) {
                                                r.reps = v;
                                            }
                                        });
                                        commit();
                                    }
                                />
                                // 回数欄に単位は添えない。プランクの 60 に「回」と付くほうが
                                // 嘘になるし、それが秒だと分かるのは種目名からで表記からではない
                                //
                                // ★ 確認を挟まないので、離す設計が唯一の事故対策になった。
                                //   削除は入力欄と地続きにしない。margin-left:auto で右端へ寄せた上に
                                //   区切り線と内側余白で離す（auto を外すと回数欄の直後に来て
                                //   今より押しやすくなる）
                                <button
                                    class="icon-btn"
                                    aria-label="このセットを削除"
                                    data-testid="remove-set"
                                    on:click=move |_| remove_row(key)
                                >
                                    {icon(icon::X)}
                                </button>
                                {move || {
                                    weight_missing()
                                        .then(|| {
                                            view! {
                                                <span class="warn" data-testid="weight-missing">
                                                    "重量未入力"
                                                </span>
                                            }
                                        })
                                }}
                                // ★ メモだけ打って回数が空の行は保存されない。黙って捨てず、
                                //   行に理由を出す。weight_missing（回数あり × 重量なし）とは
                                //   条件が排他なので 2 つ同時には出ない
                                {move || {
                                    note_orphan()
                                        .then(|| {
                                            view! {
                                                <span class="warn" data-testid="note-orphan">
                                                    "回数を入れると保存されます"
                                                </span>
                                            }
                                        })
                                }}
                                // メモ欄は ✕ の**後**に置く。`.set-row .icon-btn` の
                                // margin-left:auto が ✕ を 1 行目の右端に留める前提なので、
                                // flex-basis:100% の項目を前に置くと ✕ が 2 行目へ落ちる
                                {move || {
                                    if note_open.get() {
                                        view! {
                                            <input
                                                class="set-note"
                                                type="text"
                                                value=note_of()
                                                aria-label=move || {
                                                    format!("{} セット目のメモ", index())
                                                }
                                                data-testid="set-note"
                                                // ★ 行の入力欄には**全部**付ける。
                                                //   1 つでも漏らすと、そこからは
                                                //   `nudge_card` へ bubble して
                                                //   「行を動かしたつもりがカードが動く」
                                                on:keydown=nudge_row
                                                on:focusin=move |_| kb_focus(kb)
                                                on:focusout=move |_| kb_blur(kb)
                                                on:input=move |ev| {
                                                    let v = event_target_value(&ev);
                                                    rows.update(|rs| {
                                                        if let Some(r) = rs
                                                            .iter_mut()
                                                            .find(|r| r.key == key)
                                                        {
                                                            r.note = v;
                                                        }
                                                    });
                                                    commit();
                                                }
                                            />
                                        }
                                            .into_any()
                                    } else {
                                        // 閉じていても入力済みメモは読める。省略はしない
                                        // （静止時に利用者のメモを隠すのは要件の否定）
                                        (!note_of().trim().is_empty())
                                            .then(|| {
                                                view! {
                                                    <span
                                                        class="set-note-read"
                                                        data-testid="set-note-read"
                                                    >
                                                        {note_of()}
                                                    </span>
                                                }
                                            })
                                            .into_any()
                                    }
                                }}
                            </div>
                        }
                    }
                />
                <div class="add-set-wrap">
                    <button class="secondary" data-testid="add-set" on:click=add_row>
                        "+ セット"
                    </button>
                </div>
            </div>

            // ── 種目メモ ──────────────────────────────────────────────────────
            //
            // ★ セット行群の**下**に置く。`.last-row`（前回の記録）の直下に置くと
            //   12px --muted の行が 2 本続いて「前回の記録の続き」に読める（--muted は
            //   1 つしか無いので色で分けられない）。ここなら**字下げの有無**だけで
            //   「行のメモ（23px 字下げ）/ 種目のメモ（字下げなし）」が読め、ラベル文字が
            //   要らない。開いたときトグルの真上に現れるので、押した場所と近い
            {move || {
                if note_open.get() {
                    view! {
                        <label class="ex-note">
                            "メモ"
                            <input
                                type="text"
                                value=ex_note.get_untracked()
                                aria-label="この種目のメモ"
                                data-testid="exercise-note"
                                on:focusin=move |_| kb_focus(kb)
                                on:focusout=move |_| kb_blur(kb)
                                on:input=move |ev| {
                                    ex_note.set(event_target_value(&ev));
                                    commit();
                                }
                            />
                        </label>
                    }
                        .into_any()
                } else {
                    (!ex_note.get().trim().is_empty())
                        .then(|| {
                            view! {
                                <p class="ex-note-read" data-testid="exercise-note-read">
                                    {move || ex_note.get()}
                                </p>
                            }
                        })
                        .into_any()
                }
            }}

            // 前回比はセッション中に出さない。途中の不完全な合計を前回の完了セッションと
            // 比べても意味がないため（比較は推移タブで行う）
            //
            // ★ 外す導線はこのフッタの左端に畳む。専用の行を持たせない（カード 1 枚が
            //   その分だけ縦に縮む）ことと、右端の列から抜けることが目的。
            //   ラベルが「この種目」ではなく「この日」なのは、曖昧なのは主語ではなく
            //   スコープ（種目マスタから消えるのか、この日から外れるだけか）だから
            //
            // ★ メモの入口をここに畳むのは、カード内で 44px のタップ標的が**既に
            //   予算されている行がフッタだけ**だから。他のどこ（ヘッダ直下・`.last-row` の
            //   下・`+ セット` の隣に新しい行）に置いても 44px × カード枚数だけ縦に伸びる
            //   （5 種目で 220px）。フッタは既に 44px の .link-btn を抱えているので
            //   高さが 1px も増えない。横も 393px 幅で外す導線から 58px 空き、
            //   destructive-affordance-quiet-at-rest.md が事故として挙げた
            //   「+ セットと 41px・右端完全一致」より離れ、3 者が別の列（左端/中央/右端）に居る。
            //   adr/ux/exercise-and-set-notes-behind-one-toggle.md
            <footer class="card-foot">
                <button
                    class="link-btn card-remove"
                    data-testid="close-card"
                    on:click=request_close
                >
                    "この日から外す"
                </button>
                <button
                    class="link-btn note-toggle"
                    aria-label="メモ"
                    // ★ bool を渡さない。aria-expanded は列挙属性なので、真偽属性として
                    //   扱われると false のとき属性ごと消える（＝折りたためることが消える）
                    aria-expanded=move || if note_open.get() { "true" } else { "false" }
                    data-testid="note-toggle"
                    on:click=move |_| note_open.update(|o| *o = !*o)
                >
                    {move || if note_open.get() { "－ メモ" } else { "＋ メモ" }}
                </button>
                <span class="foot-total">
                    <span>"今日"</span>
                    <strong data-testid="today-metric">{move || fmt_metric(today_metric())}</strong>
                </span>
            </footer>

            {move || {
                confirm_close
                    .get()
                    .then(|| {
                        view! {
                            <div class="warn-box" id=confirm_dom_id(ex)>
                                <p data-testid="close-card-warning">
                                    "この日の記録が消えます"
                                </p>
                                <div class="sheet-actions">
                                    <button
                                        class="primary"
                                        data-testid="close-card-yes"
                                        on:click=move |_| close_card()
                                    >
                                        "外す"
                                    </button>
                                    <button
                                        class="link-btn"
                                        data-testid="close-card-no"
                                        on:click=move |_| confirm_close.set(false)
                                    >
                                        "やめる"
                                    </button>
                                </div>
                            </div>
                        }
                    })
            }}
        </article>
    }
}
