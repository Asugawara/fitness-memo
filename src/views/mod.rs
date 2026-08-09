//! 画面の共通土台。ボトムタブ 3 つ + 全画面が依存する日付コンテキスト。

pub mod backup;
pub mod calendar;
pub mod chart;
pub mod day;
pub mod help;
pub mod icon;
pub mod menu;
pub mod progress;

use std::cell::Cell;
use std::time::Duration;

use chrono::{Datelike, Local, NaiveDate, Weekday};
use leptos::prelude::*;
// シートを閉じたときのフォーカス復帰で Element → HtmlElement に落とすのに使う
use wasm_bindgen::JsCast;

use crate::model::{Db, GroupId, SetEntry};
use crate::storage;
use crate::view_transition;

use calendar::Calendar;
use icon::icon;
use menu::Menu;
use progress::Progress;

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
    /// 記録タブが選んでいる日付。カレンダーの選択日と編集対象を兼ねる**唯一の真実源**。
    ///
    /// カレンダーと入力欄が同じ画面に載っているので、ここを二重に持つと
    /// 「グリッドで選んだ日」と「下の入力欄が書き込む日」がずれる。
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

    /// 日付を選ぶ。**同値ならシグナルを動かさない。**
    ///
    /// `RwSignal::set` は同値でも購読者へ通知するので、素で書くと選択中の日セルを
    /// もう一度タップしただけで `<ConditionRow />` が作り直され、体重欄に「62.」まで
    /// 打った中間状態が確定値へ巻き戻る。
    pub fn open(&self, date: NaiveDate) {
        if self.selected.get_untracked() != date {
            self.selected.set(date);
        }
    }

    /// 現在日付を引き直す。
    ///
    /// ★ **選択日は「当日を見ていたときだけ」新しい当日へ追従させる。**
    ///
    /// iOS のホーム画面 PWA は再起動されず何日もレジュームされるので、日付を跨いだ
    /// 操作が前日に記録されるのを防ぐ必要がある。ただし「明示的に選んだ過去日」まで
    /// 巻き戻してはいけない。以前は visible 復帰だけ無条件に当日へ戻していたが、
    /// それが安全だったのはカレンダーの選択日が別シグナルで resync の影響外に
    /// あったから。選択日を一本化した今それを残すと、7 月の記録を見ている最中に
    /// 通知からアプリへ戻るだけで月表示ごと今日へ飛ぶ。
    ///
    /// 誤記帳の防止はカレンダーのハイライトと `past-banner` が担う（日付が常時見える）。
    fn resync(&self) {
        let now = today_local();
        let prev = self.today.get_untracked();
        if prev != now {
            self.today.set(now);
        }
        let selected = self.selected.get_untracked();
        if selected == prev && selected != now {
            self.selected.set(now);
        }
    }
}

/// ソフトキーボードが出ているか。`true` の間はボトムタブを隠す。
#[derive(Clone, Copy)]
pub struct KbCtx(pub RwSignal<bool>);

/// 種目タブで開いている部位。**同時に開くのは 1 つ**
/// （adr/ux/menu-groups-as-single-open-accordion.md）。
///
/// ★ `Menu` の中ではなくここに置くのが要点。`match tab.get()` はタブを切り替えるたびに
///   `Menu` を作り直すので、コンポーネント内のシグナルにすると記録⇄種目を往復するたびに
///   全部閉じる。筋トレ中はその往復が常なので、戻るたびに部位を探して押し直すことになる。
/// ★ それでも**永続化はしない**。`Db` 由来の ID を UI 状態のキーに入れると `Db` から
///   部位が消えたときに宙に浮く（adr/storage/ui-state-in-separate-key.md が
///   前提の崩れる例として名指ししている形）。
///   ここはプロセス内の寿命に留める。
#[derive(Clone, Copy)]
pub struct OpenGroupCtx(pub RwSignal<Option<GroupId>>);

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

pub fn use_open_group() -> RwSignal<Option<GroupId>> {
    use_context::<OpenGroupCtx>()
        .expect("OpenGroupCtx が provide されていない")
        .0
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

/// 1 セットの表示。重量ありなら "60×10"、重量なしなら "12"。
///
/// 単位（回 / 秒）は添えない。プランクの 60 に「回」と付くほうが嘘になるし、
/// それが秒だと分かるのは種目名からで、表記から読むものではない。
pub fn fmt_set(s: &SetEntry) -> String {
    if s.weight > 0.0 {
        format!("{}×{}", fmt_weight(s.weight), s.reps)
    } else {
        format!("{}", s.reps)
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

/// ブラウザのタブと standalone で `localStorage` が分かれうる環境か（＝警告を出す対象か）。
///
/// ★ 「iOS を当てる」形にしないのが要点。iPadOS 13+ の Safari は既定で desktop-class の
/// UA（`Macintosh; Intel Mac OS X …`）を出すので、`iPhone` / `iPad` を探しにいくと
/// ストレージ分離が同じく起きる iPad が保護対象から落ちる。逆に Android Chrome は
/// タブと PWA でストレージを共有するので、そこに出す警告は事実として偽になる。
/// だから**除外側だけを列挙し、判定できない環境は警告を出す側に倒す**
/// （`is_standalone` が match_media の失敗を false に倒しているのと同じ方針）。
///
/// 副作用として PC ブラウザにも出る。これは受容する（このアプリは iPhone のホーム画面
/// から使うものなので、PC で開いた人にそう伝わるのは害ではない）。精度が要るように
/// なったら `maxTouchPoints` を足す。
pub fn storage_may_split() -> bool {
    window()
        .navigator()
        .user_agent()
        .map(|ua| !ua.contains("Android"))
        .unwrap_or(true)
}

/// 次のフレームで指定 id の要素までスクロールする（要素の上端を画面の上端に合わせる）。
pub fn scroll_to_id(element_id: String) {
    request_animation_frame(move || {
        if let Some(el) = document().get_element_by_id(&element_id) {
            el.scroll_into_view();
        }
    });
}

/// 次のフレームで指定 id の要素を、**画面に入っていなければ**入るところまで動かす。
///
/// ★ [`scroll_to_id`] との違いは `block: nearest`。既に見えている要素は 1px も動かさず、
/// はみ出しているときだけ最小限スクロールする。「開いた部位を必ず画面に入れる」ように
/// 使うのが目的で、`scroll_to_id` を使うと**タップして開いただけの部位まで**画面上端へ
/// 飛ぶ（アコーディオンを開くたびに視界がジャンプする）。
pub fn scroll_into_view_if_needed(element_id: String) {
    request_animation_frame(move || {
        if let Some(el) = document().get_element_by_id(&element_id) {
            let opts = web_sys::ScrollIntoViewOptions::new();
            opts.set_block(web_sys::ScrollLogicalPosition::Nearest);
            el.scroll_into_view_with_scroll_into_view_options(&opts);
        }
    });
}

// ── ボトムシート ────────────────────────────────────────────────────────────

/// 下から上がるシート。種目を追加 / 種目の編集 / ホーム画面に追加 / 書き出し読み込みの
/// 4 箇所で共有する。
///
/// ★ `<div role="dialog">` ではなく**ネイティブ `<dialog>` を `show_modal()` で開く**。
///   top layer に載るので z-index を持つ必要が無くなり、背景は UA が `inert` にする。
///   以前この 3 つを手で両立させようとして 2 件壊している（styles.css 冒頭のコメント）。
///   Esc（close request）も UA 側の仕事になる。経緯は adr/ux/native-dialog-for-sheets.md。
///
/// ★ **常時マウントする。** 開いている間だけ DOM に置く形にすると、閉じる際に
///   「top layer に載ったままの要素を DOM から消す」ことになり `close` イベントが
///   飛ばない。呼び出し側は `open` を倒すだけでよい。
///   その代わり、シートの中身は閉じている間も評価されるので、
///   **中で `with_untracked` を使うと開き直しても古い値のままになる**（day.rs の
///   「追加済み」表示がこれで壊れかけた）。中身は素直に追跡する形で書くこと。
#[component]
pub fn Sheet(
    /// 開いているか。
    #[prop(into)]
    open: Signal<bool>,
    /// 閉じる要求。✕ / Esc / 背景タップの 3 経路が全部ここに集まる。
    /// 呼び出し側の後始末（backup.rs のメモ消しなど）を奪わないので `Callback` で受ける。
    #[prop(into)]
    on_close: Callback<()>,
    /// 見出し。アクセシブル名も兼ねる。
    #[prop(into)]
    title: Signal<String>,
    testid: &'static str,
    close_testid: &'static str,
    children: Children,
) -> impl IntoView {
    let dialog: NodeRef<leptos::html::Dialog> = NodeRef::new();

    // 開く直前にフォーカスされていた要素。閉じたらここへ戻す（下の on:close を参照）。
    // web_sys の型は Send + Sync ではないので local 版で持つ
    let opener: StoredValue<Option<web_sys::HtmlElement>, LocalStorage> =
        StoredValue::new_local(None);

    // signal → DOM。★ 現在の開閉状態で門番する。`show_modal()` は既に開いている
    //   dialog に呼ぶと InvalidStateError を投げ、`close()` は閉じている dialog にも
    //   `close` イベントを飛ばすので、素直に呼ぶと on_close と往復する
    Effect::new(move |_| {
        let Some(d) = dialog.get() else { return };
        if open.get() {
            if !d.open() {
                // ★ 控えるのは show_modal() の**前**。呼んだ瞬間にフォーカスが
                //   シート内の先頭（✕ ボタン）へ移ってしまう。
                //   ★ <body> は捨てる。WebKit はボタンをタップしてもフォーカスを
                //   与えない（実測で activeElement は BODY）ので、指で開いた場合は
                //   「戻すべき場所」が存在しない。そこへ強引に focus を当てると、
                //   他のコントロールと違う挙動をシートだけが持つことになる
                opener.set_value(
                    document()
                        .active_element()
                        .filter(|e| e.tag_name() != "BODY")
                        .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok()),
                );
                let _ = d.show_modal();
            }
        } else if d.open() {
            d.close();
        }
    });

    view! {
        <dialog
            node_ref=dialog
            class="sheet"
            aria-label=move || title.get()
            data-testid=testid
            // Esc で閉じたときに呼び出し側のシグナルが真のまま残ると「閉じたのに
            // 二度と開かない」になる。UA 起因の close も必ずここを通す
            on:close=move |_| {
                on_close.run(());
                // ★ WebKit は <dialog> を閉じてもフォーカスを開いた要素へ戻さない。
                //   実測（Playwright / iPhone 15 Pro）で Esc 後の activeElement は BODY で、
                //   キーボードで操作している人はそのたびにページ先頭から辿り直しになる。
                //   Chromium は自前で戻すので二重に当たるが、同じ要素なので無害。
                //   主対象が iOS Safari である以上、UA 任せにはできない
                if let Some(el) = opener.get_value() {
                    let _ = el.focus();
                }
            }
            on:click=move |ev| {
                // ★ 背景タップで閉じる。`closedby="any"` は Safari 未対応なので使えない
                //   （adr/architecture/browser-support-policy.md）。backdrop へのクリックは **dialog 自身**が target に
                //   なるのでまずそこで絞り、さらに座標が箱の外かを見る。target だけで
                //   判定するとシート内の余白を突いたときにも閉じてしまう
                let Some(d) = dialog.get_untracked() else { return };
                let dialog_js: &wasm_bindgen::JsValue = d.as_ref();
                let on_dialog_itself = ev
                    .target()
                    .is_some_and(|t| {
                        let t: &wasm_bindgen::JsValue = t.as_ref();
                        t == dialog_js
                    });
                if !on_dialog_itself {
                    return;
                }
                let r = d.get_bounding_client_rect();
                let (x, y) = (ev.client_x() as f64, ev.client_y() as f64);
                let inside = r.left() <= x && x <= r.right() && r.top() <= y
                    && y <= r.bottom();
                if !inside {
                    on_close.run(());
                }
            }
        >
            <header class="sheet-head">
                <strong>{move || title.get()}</strong>
                // aria-label は残す。見た目は ✕ でも支援技術と E2E の role+name には
                // 「閉じる」で届く必要がある
                <button
                    class="icon-btn"
                    aria-label="閉じる"
                    data-testid=close_testid
                    on:click=move |_| on_close.run(())
                >
                    {icon(icon::X)}
                </button>
            </header>
            <div class="sheet-body">{children()}</div>
        </dialog>
    }
}

// ── タブ ────────────────────────────────────────────────────────────────────

/// ボトムタブ。
///
/// `Record` はカレンダーと選択日のエディタを 1 画面に載せたもの。
/// 以前は「今日」と「カレンダー」が別タブで、過去日を直すたびに往復していた。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Record,
    Progress,
    Menu,
}

impl Tab {
    /// タブバーでの並び順。遷移の向きはこれの大小で決める。
    fn order(self) -> u8 {
        match self {
            Tab::Record => 0,
            Tab::Progress => 1,
            Tab::Menu => 2,
        }
    }
}

/// 現在のタブ。
#[derive(Clone, Copy)]
pub struct TabCtx(pub RwSignal<Tab>);

impl TabCtx {
    /// タブを切り替える。
    ///
    /// **切替のたびに「今日」を再評価する**のが要点。iOS のホーム画面 PWA は再起動されず
    /// 何日もレジュームされるので、mount 時に決めた日付を持ち回ると日付を跨いだ操作が
    /// 前日に記録される。明示的に選んだ過去日は `resync` が保つ。
    pub fn switch(&self, dates: DateCtx, to: Tab) {
        dates.resync();

        // 同じタブなら何もしない。set は同値でも購読者へ通知するので、素で書くと
        // 押すたびに画面が丸ごとクロスフェードする
        let from = self.0.get_untracked();
        if from == to {
            return;
        }

        let direction = if to.order() > from.order() {
            view_transition::Direction::Forward
        } else {
            view_transition::Direction::Backward
        };
        let tab = self.0;
        view_transition::run(direction, move || tab.set(to));
    }
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

    // ★ 種目タブの外に置く（タブ往復で閉じないため）。理由は OpenGroupCtx を参照
    provide_context(OpenGroupCtx(RwSignal::new(None)));

    let tab = RwSignal::new(Tab::Record);
    let tabs = TabCtx(tab);
    provide_context(tabs);

    let notice = RwSignal::new(restore_note);

    // Db の変更を購読して 400ms debounce で保存する
    Effect::new(move |_| {
        storage::save_debounced(db.get());
    });

    // ★ hidden で flush、visible 復帰で「今日」を引き直す。
    //   visibilitychange は Document で発火するが bubbles: true なので window で捕捉できる。
    //   leptos に visibilitychange の typed event は無いので untyped を使う。
    let listener = window_event_listener_untyped("visibilitychange", move |_| {
        // 可視判定は document().hidden()（web-sys の VisibilityState feature が要らない）
        if document().hidden() {
            storage::flush();
        } else {
            dates.resync();
            // ★ 保存が失敗していたら伝える。黙っていると、書けていないのに動き続けて
            //   数週間分の入力を失ってから気づくことになる
            if storage::save_failed() {
                notice.set(Some(
                    "記録を保存できていません。種目タブの「データの書き出し / 読み込み」から今すぐ控えを取ってください"
                        .to_string(),
                ));
            }
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
                                    {icon(icon::X)}
                                </button>
                            </div>
                        }
                    })
            }}

            <main class="screen">
                {move || match tab.get() {
                    Tab::Record => view! { <Calendar /> }.into_any(),
                    Tab::Progress => view! { <Progress /> }.into_any(),
                    Tab::Menu => view! { <Menu /> }.into_any(),
                }}
            </main>

            <nav class="bottom-tabs" data-testid="bottom-tabs">
                {tab_button(Tab::Record, "記録", "tab-record")}
                {tab_button(Tab::Progress, "推移", "tab-progress")}
                {tab_button(Tab::Menu, "種目", "tab-menu")}
            </nav>
        </div>
    }
}
