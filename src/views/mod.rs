//! 画面の共通土台。ボトムタブ 4 つ + 全画面が依存する日付コンテキスト。

pub mod calendar;
pub mod chart;
pub mod menu;
pub mod progress;
pub mod today;

use std::cell::Cell;
use std::time::Duration;

use chrono::{Datelike, Local, NaiveDate, Weekday};
use leptos::prelude::*;

use crate::core::Elapsed;
use crate::model::{Db, Kind, SetEntry};
use crate::storage;

use calendar::Calendar;
use menu::Menu;
use progress::Progress;
use today::Today;

// ── コンテキスト ────────────────────────────────────────────────────────────

/// アプリ唯一の `Db`。`Effect` で購読して debounce 保存する。
#[derive(Clone, Copy)]
pub struct DbCtx(pub RwSignal<Db>);

/// 日付コンテキスト。**全画面が依存する。**
///
/// iOS のホーム画面 PWA は再起動されず何日もレジュームされるため、mount 時に決めた
/// 「今日」を持ち続けると月曜に開いたアプリで水曜のトレーニングを月曜に記録してしまう。
#[derive(Clone, Copy)]
pub struct DateCtx {
    /// 実時刻から求めた「今日」。レジューム / タブ切替で再評価する
    pub today: RwSignal<NaiveDate>,
    /// 今日タブが表示している日付。過去日編集中だけ `today` と食い違う
    pub selected: RwSignal<NaiveDate>,
}

impl DateCtx {
    /// 過去日（または未来日）を編集中か。ヘッダの見た目とバナーの出し分けに使う。
    pub fn is_past_edit(&self) -> bool {
        self.selected.get() != self.today.get()
    }

    pub fn back_to_today(&self) {
        let today = self.today.get_untracked();
        if self.selected.get_untracked() != today {
            self.selected.set(today);
        }
    }

    /// 他タブから「この日を編集」で今日タブを開くときに使う。
    pub fn open(&self, date: NaiveDate) {
        if self.selected.get_untracked() != date {
            self.selected.set(date);
        }
    }

    /// 現在日付を引き直す。
    ///
    /// `force` は「visible 復帰」で真。ユーザーが明示的に過去日を選んでいても当日へ戻す。
    /// タブ切替（`force = false`）では、当日を見ていた場合だけ新しい当日へ追従する。
    fn resync(&self, force: bool) {
        let now = today_local();
        let prev = self.today.get_untracked();
        if prev != now {
            self.today.set(now);
        }
        if (force || self.selected.get_untracked() == prev) && self.selected.get_untracked() != now
        {
            self.selected.set(now);
        }
    }
}

/// ソフトキーボードが出ているか。`true` の間はボトムタブを隠す。
#[derive(Clone, Copy)]
pub struct KbCtx(pub RwSignal<bool>);

pub fn use_db() -> RwSignal<Db> {
    use_context::<DbCtx>()
        .expect("DbCtx が provide されていない")
        .0
}

pub fn use_dates() -> DateCtx {
    use_context::<DateCtx>().expect("DateCtx が provide されていない")
}

pub fn use_kb() -> KbCtx {
    use_context::<KbCtx>().expect("KbCtx が provide されていない")
}

// ── キーボード対策 ──────────────────────────────────────────────────────────

thread_local! {
    /// focusout → focusin の間でタブバーが一瞬ちらつくのを防ぐための遅延解除タイマー。
    static KB_TIMER: Cell<Option<TimeoutHandle>> = const { Cell::new(None) };
}

/// 入力欄の `on:focusin` から呼ぶ。
///
/// ★ iOS Safari / standalone PWA はキーボード表示時に layout viewport が縮まず
/// visual viewport だけ変化するため、`position: fixed` 要素はキーボードの背後に隠れる。
/// 回避策の `interactive-widget=resizes-visual` は **standalone モードでは無視される**。
/// 本アプリの中核操作は「テンキーでセットを打ち込む」ことなので、対策しないと毎セット
/// タブバーが入力域に被る。**DevTools のレスポンシブモードでは再現しない。**
pub fn kb_focus(kb: KbCtx) {
    if let Some(handle) = KB_TIMER.take() {
        handle.clear();
    }
    if !kb.0.get_untracked() {
        kb.0.set(true);
    }
}

/// 入力欄の `on:focusout` から呼ぶ。
pub fn kb_blur(kb: KbCtx) {
    if let Some(handle) = KB_TIMER.take() {
        handle.clear();
    }
    let open = kb.0;
    match set_timeout_with_handle(move || open.set(false), Duration::from_millis(150)) {
        Ok(handle) => KB_TIMER.set(Some(handle)),
        Err(_) => open.set(false),
    }
}

// ── 日付・数値のフォーマット ────────────────────────────────────────────────

pub fn today_local() -> NaiveDate {
    // ★ chrono の default features（wasmbind）が効いているのでブラウザの実 TZ を引く。
    //   default-features = false にすると黙って UTC になり日付キーが 9 時間ズレる
    Local::now().date_naive()
}

pub fn now_ms() -> i64 {
    Local::now().timestamp_millis()
}

pub fn weekday_ja(d: NaiveDate) -> &'static str {
    match d.weekday() {
        Weekday::Mon => "月",
        Weekday::Tue => "火",
        Weekday::Wed => "水",
        Weekday::Thu => "木",
        Weekday::Fri => "金",
        Weekday::Sat => "土",
        Weekday::Sun => "日",
    }
}

/// "8/8 (金)"
pub fn fmt_date(d: NaiveDate) -> String {
    format!("{}/{} ({})", d.month(), d.day(), weekday_ja(d))
}

/// 指標を "1,080" の形にする（小数は落とす）。
pub fn fmt_metric(v: f64) -> String {
    let n = v.round() as i64;
    let digits = n.abs().to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

/// 60.0 → "60"、62.5 → "62.5"
pub fn fmt_weight(w: f32) -> String {
    if w == w.trunc() {
        format!("{}", w.trunc() as i64)
    } else {
        format!("{w}")
    }
}

/// 入力欄の生文字列 → 重量。
///
/// `"6."` は Rust の `f32` パーサが 6.0 として受けるので中間状態でも壊れない。
/// iOS のテンキーは小数点がロケール依存で `,` になることがあるので置換しておく。
pub fn parse_weight(s: &str) -> f32 {
    s.trim()
        .replace(',', ".")
        .parse::<f32>()
        .ok()
        .filter(|w| w.is_finite() && *w >= 0.0)
        .unwrap_or(0.0)
}

/// 入力欄の生文字列 → レップ数。**空欄と 0 は「行なし」として扱う。**
pub fn parse_reps(s: &str) -> Option<u32> {
    s.trim().parse::<u32>().ok().filter(|r| *r > 0)
}

/// 1 セットの表示。"60×10" / "+10×8" / "60秒"
pub fn fmt_set(kind: Kind, s: &SetEntry) -> String {
    match kind {
        Kind::Weighted => format!("{}×{}", fmt_weight(s.weight), s.reps),
        // Bodyweight の weight は「追加重量」。指標には入らないが表示はする
        Kind::Bodyweight if s.weight > 0.0 => format!("+{}×{}", fmt_weight(s.weight), s.reps),
        Kind::Bodyweight => format!("{}", s.reps),
        Kind::Duration => format!("{}秒", s.reps),
    }
}

/// レップ欄の単位ラベル。
pub fn reps_unit(kind: Kind) -> &'static str {
    match kind {
        Kind::Duration => "秒",
        _ => "回",
    }
}

/// 部位チップ用の短い表記。"3d" / "今日"
pub fn short_elapsed(e: Elapsed) -> String {
    let days = match e {
        Elapsed::Exact(ms) => ms / 86_400_000,
        Elapsed::Days(d) => d,
    };
    if days == 0 {
        "今日".to_string()
    } else {
        format!("{days}d")
    }
}

/// チップの濃淡。**部位カラー × 経過濃淡の二重符号化を避けるため単色系に統一する。**
pub fn recency_class(e: Option<Elapsed>) -> &'static str {
    let Some(e) = e else { return "none" };
    let days = match e {
        Elapsed::Exact(ms) => ms / 86_400_000,
        Elapsed::Days(d) => d,
    };
    match days {
        0..=1 => "fresh",
        2..=3 => "recent",
        4..=6 => "stale",
        _ => "old",
    }
}

// ── DOM ヘルパ ──────────────────────────────────────────────────────────────

/// ホーム画面から起動された PWA か。
///
/// ★ iOS では Safari のタブと standalone PWA で `localStorage` が**共有されない**。
/// Safari で数日記録してからホーム画面に追加すると PWA 側は空の DB で起動し、それまでの
/// 記録が見えなくなる。エクスポート機能が無い v1 では回復不能なので警告を出す。
pub fn is_standalone() -> bool {
    window()
        .match_media("(display-mode: standalone)")
        .ok()
        .flatten()
        .is_some_and(|m| m.matches())
}

/// 次のフレームで指定 id の要素までスクロールする。
pub fn scroll_to_id(element_id: String) {
    request_animation_frame(move || {
        if let Some(el) = document().get_element_by_id(&element_id) {
            el.scroll_into_view();
        }
    });
}

// ── タブ ────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Today,
    Calendar,
    Progress,
    Menu,
}

/// 現在のタブ。
///
/// ボトムタブ以外からもタブを移す導線がある（カレンダーの「この日に記録する」）。
/// ここを `provide_context` しないと他画面から遷移する手段が無くなり、DOM を引いて
/// 合成クリックを投げるような**アプリの動作が `data-testid` に依存する**実装に落ちる。
#[derive(Clone, Copy)]
pub struct TabCtx(pub RwSignal<Tab>);

impl TabCtx {
    /// タブを切り替える。
    ///
    /// **切替のたびに「今日」を再評価する**のが要点。iOS のホーム画面 PWA は再起動されず
    /// 何日もレジュームされるので、mount 時に決めた日付を持ち回ると日付を跨いだ操作が
    /// 前日に記録される。`resync(false)` なので、ユーザーが明示的に選んだ過去日は残る。
    pub fn switch(&self, dates: DateCtx, to: Tab) {
        dates.resync(false);
        self.0.set(to);
    }
}

pub fn use_tab() -> TabCtx {
    use_context::<TabCtx>().expect("TabCtx が provide されていない")
}

#[component]
pub fn App() -> impl IntoView {
    let (initial, restore_note) = storage::load();
    let db = RwSignal::new(initial);
    provide_context(DbCtx(db));

    let start = today_local();
    let dates = DateCtx {
        today: RwSignal::new(start),
        selected: RwSignal::new(start),
    };
    provide_context(dates);

    let kb = KbCtx(RwSignal::new(false));
    provide_context(kb);

    let tab = RwSignal::new(Tab::Today);
    let tabs = TabCtx(tab);
    provide_context(tabs);

    let notice = RwSignal::new(restore_note);

    // Db の変更を購読して 400ms debounce で保存する
    Effect::new(move |_| {
        storage::save_debounced(db.get());
    });

    // ★ hidden で flush、visible 復帰で当日へリセット。
    //   visibilitychange は Document で発火するが bubbles: true なので window で捕捉できる。
    //   leptos に visibilitychange の typed event は無いので untyped を使う。
    let listener = window_event_listener_untyped("visibilitychange", move |_| {
        // 可視判定は document().hidden()（web-sys の VisibilityState feature が要らない）
        if document().hidden() {
            storage::flush();
        } else {
            dates.resync(true);
        }
    });
    on_cleanup(move || listener.remove());

    // ボトムタブも他画面からの遷移も同じ TabCtx::switch を通す（日付の再評価を一箇所にする）
    let switch = move |t: Tab| tabs.switch(dates, t);

    let tab_button = move |t: Tab, label: &'static str, testid: &'static str| {
        view! {
            <button
                class="tab-btn"
                class:active=move || tab.get() == t
                data-testid=testid
                on:click=move |_| switch(t)
            >
                {label}
            </button>
        }
    };

    view! {
        <div class="app" class:kb-open=move || kb.0.get()>
            {move || {
                notice
                    .get()
                    .map(|msg| {
                        view! {
                            <div class="notice" role="status" data-testid="restore-notice">
                                <span>{msg}</span>
                                <button
                                    class="icon-btn"
                                    aria-label="通知を閉じる"
                                    on:click=move |_| notice.set(None)
                                >
                                    "✕"
                                </button>
                            </div>
                        }
                    })
            }}

            <main class="screen">
                {move || match tab.get() {
                    Tab::Today => view! { <Today /> }.into_any(),
                    Tab::Calendar => view! { <Calendar /> }.into_any(),
                    Tab::Progress => view! { <Progress /> }.into_any(),
                    Tab::Menu => view! { <Menu /> }.into_any(),
                }}
            </main>

            <nav class="bottom-tabs" data-testid="bottom-tabs">
                {tab_button(Tab::Today, "今日", "tab-today")}
                {tab_button(Tab::Calendar, "カレンダー", "tab-calendar")}
                {tab_button(Tab::Progress, "推移", "tab-progress")}
                {tab_button(Tab::Menu, "種目", "tab-menu")}
            </nav>
        </div>
    }
}
