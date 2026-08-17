//! ドラッグで並び替えるための **view 側の共有部品**（adr/ux/drag-to-reorder-in-record-tab.md）。
//!
//! 幾何そのものは [`crate::reorder`] がターゲット非依存で持っている。ここに置くのは
//! それを DOM に繋ぐ側 — 掴む / 測る / 端でスクロールする / 畳む — で、**`web_sys` を
//! 知っているのはこちらだけ**である。
//!
//! ## なぜ `views::day` から出したか
//!
//! 掴めるものが記録タブのカードとセット行の 2 つだった間は day.rs の private で足りていた。
//! メニュー編集シート（`views::routine`）の「選択中」が 3 つ目になった時点で、`Drag` /
//! `Press` / `capture` / `holds` / `release` / `alt_arrow` / 端の自動スクロールを丸ごと
//! もう 1 実装書くか、共有するかの二択になる。**もう 1 実装は書かない** —
//! `views::routine` がシートを 2 つ書かないのと同じ理由で、片方だけ直る事故が必ず起きる。
//!
//! ## 座標系は容器ごとに違う（[`Scroller`]）
//!
//! ここが day.rs からそのまま持ってこられなかった唯一の場所である。記録タブは
//! **ページ全体**がスクロールするが、メニュー編集シートの「選択中」は
//! `.sheet-body`（`overflow-y: auto`）という**入れ子のスクロール容器**の中にある。
//! `window().scroll_y()` はシートの中をいくらスクロールしても 1px も動かないので、
//! そのままでは指の移動量を取りこぼす。[`Scroller`] が「スクロール量・端の帯・
//! スクロールのさせ方」の 3 つをまとめて差し替える。

use std::cell::Cell;
use std::time::Duration;

use leptos::prelude::*;
// pointerdown の target を Element に落として setPointerCapture するのに使う
use wasm_bindgen::JsCast;
use web_sys::PointerEvent;

use crate::reorder::{self, Slots};

/// 長押し待ちの間に指がこれ以上動いたら、そのジェスチャは捨てる。
pub const PRESS_SLOP_PX: f64 = 10.0;

/// `.card-head` を押してからカードのドラッグが効き始めるまで。
///
/// ★ **細いハンドル（`.set-no` / `.rtn-no`）は 0 で、全幅のハンドルだけ待つ。**
/// `touch-action: none` を当てた要素からはページをスクロールできないので、全幅の
/// `.card-head` を即時開始にすると縦フリック 1 回で種目の順が変わる（閾値は隣の
/// カードの半分 ≈ 87px、フリックは 100〜300px 動く）。しかも画面が動かないので
/// 「スクロールが効かなかった」と「並びが変わった」が同時に起きる。250ms 待てば
/// フリックは**何も起きない**で終わる。約 29px 幅のハンドルはそこを起点に縦フリックが
/// 始まる確率が低く、待たせるほうが損になる。
pub const PRESS_DELAY_CARD: Duration = Duration::from_millis(250);

/// 画面端の自動スクロール速度（px/frame）。60fps で約 840px/s。
///
/// **3 つのハンドルすべてに要る。**
/// - カードは 5〜8 枚 × 150〜400px で、可視域（約 660px）に収まらない
/// - セット行は閉じていれば 8 行 × 約 50px で収まるが、**メモ欄を開くと 1 行 96px**
///   になり、8 行で 768px（実測）。1 本目を 8 本目まで運ぶ指が画面の外に出る
/// - メニューの「選択中」は 1 行 44px で、10 種目のメニューが `.sheet-body` の
///   可視域（78vh から見出しと保存帯を引いた残り）を超える
///
/// 無いと「動かす → 指を離す → スクロール」を数回繰り返すことになる。
const EDGE_STEP_PX: f64 = 14.0;

thread_local! {
    /// 端の自動スクロールのループが走っているか。**再入防止。**
    /// 同時に掴めるハンドルは 1 つなので 1 本で足りる（`views::mod` の `KB_TIMER` と同じ形）。
    pub static EDGE_SCROLLING: Cell<bool> = const { Cell::new(false) };
    /// 長押し待ち。**画面には何も出ないので signal ではない。**
    ///
    /// ★ 掴む側のコンポーネントの中（`StoredValue`）に置かない。記録タブは `mod.rs` の
    ///   `match tab.get()` の枝なので、**タブを切り替えた瞬間に所有者ごと破棄される**。
    ///   長押しの途中で切り替えると、生き残ったタイマーが破棄済みの値を触って panic する
    ///   （wasm では unreachable に落ちてアプリが死ぬ）。ここなら所有者を持たない。
    pub static PRESS: Cell<Option<Press>> = const { Cell::new(None) };
}

/// 長押し待ちを畳む。タイマーが生きていれば止める。**冪等。**
pub fn end_press() {
    if let Some(p) = PRESS.take()
        && let Some(timer) = p.timer
    {
        timer.clear();
    }
}

/// 長押し待ち。**まだ何も動かさない**ので signal ではなく [`PRESS`] に置く。
#[derive(Clone, Copy, Debug)]
pub struct Press {
    pub pointer_id: i32,
    pub down_y: f64,
    pub last_y: f64,
    /// `None` は「slop を超えたので死んだ」。`pointerup` まで生き返らせない
    /// （じわじわ動かして後から armed になるのを防ぐ）
    pub timer: Option<TimeoutHandle>,
}

// ── スクロール容器 ──────────────────────────────────────────────────────────

/// ドラッグの座標が乗っているスクロール容器。
///
/// ★ **要素そのものを持たず DOM id を持つ。** `web_sys` の型は `Send + Sync` では
/// ないので、[`Drag`] に載せると `RwSignal<Option<Drag>>` が `LocalStorage` 版でしか
/// 作れなくなり、掴む側 3 か所の型が全部連鎖して変わる（`views::mod` の `opener` が
/// `StoredValue<_, LocalStorage>` なのは同じ制約）。id なら `String` なので普通に載る。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scroller {
    /// ページ全体。記録タブのカードとセット行。
    Window,
    /// 入れ子のスクロール容器。`String` は DOM id で、`views::Sheet` が
    /// `{testid}-body` で振っている。
    Element(String),
}

impl Scroller {
    /// 掴んだ並びの要素（DOM id で指定）から容器を決める。
    /// **祖先に `.sheet-body` があればそちら、無ければページ全体。**
    ///
    /// ★ 呼び側に「自分がどのシートの中にいるか」を渡させない。`views::routine` の
    /// `RoutineEditor` は設定タブと記録タブの 2 つのシートから開かれるので、
    /// prop で受ける形にすると**片方だけ間違った容器を指しても画面上は動いて見える**
    /// （スクロールしない限り差が出ない）。DOM から引けば配線を間違えようがない。
    ///
    /// ★ `PointerEvent` の `target` からではなく **id から**引く。掴む場所が長押しの
    /// 要る全幅のハンドルだと、実際に掴むのは 250ms 後のタイマーの中でイベントが手元に
    /// 無い（`Press` は `Cell` に入るので `String` を持てず、イベントも持ち越せない）。
    /// 並びの要素の id は測るために必ず作っているので、それを使い回す。
    pub fn of_id(element_id: &str) -> Self {
        document()
            .get_element_by_id(element_id)
            .and_then(|el| el.closest(".sheet-body").ok().flatten())
            .map(|el| el.id())
            .filter(|id| !id.is_empty())
            .map_or(Self::Window, Self::Element)
    }

    fn element(&self) -> Option<web_sys::Element> {
        match self {
            Self::Window => None,
            Self::Element(id) => document().get_element_by_id(id),
        }
    }

    /// 現在のスクロール量。viewport 座標を容器の内容座標へ直すのに足す。
    ///
    /// `Element` 側が `i32` に落ちるのは `web_sys` の束縛の都合。閾値は最小でも
    /// 25px あるので 1px 未満の丸めは効かない。
    pub fn offset(&self) -> f64 {
        match self {
            Self::Window => window().scroll_y().unwrap_or(0.0),
            Self::Element(_) => self.element().map_or(0.0, |el| f64::from(el.scroll_top())),
        }
    }

    /// 端の自動スクロールの帯の**内側の縁**（viewport 座標）。測れなければ `None`。
    ///
    /// ★ ページ側で `--tabbar` を読まないのは、タブバー（56px）の上端でちょうど最大速度に
    /// なる位置に帯が来るため。指がタブバーの上まで行っても pointer capture が効いて
    /// いるので、そこは「最大速度の続き」で正しい。
    ///
    /// ★ シート側は `innerHeight` ではなく `.sheet-body` の箱から出す。シートは
    /// `max-height: 78vh` で下端に貼るので、**画面上端の帯はシートの外**にあり、
    /// そこへ指が入ることは（modal なので）そもそも無い。
    fn band(&self) -> Option<(f64, f64)> {
        match self {
            Self::Window => {
                let h = window().inner_height().ok().and_then(|v| v.as_f64())?;
                Some(reorder::edge_band(0.0, h))
            }
            Self::Element(_) => {
                let r = self.element()?.get_bounding_client_rect();
                Some(reorder::edge_band(r.top(), r.bottom()))
            }
        }
    }

    fn scroll_by(&self, dy: f64) {
        match self {
            Self::Window => window().scroll_by_with_x_and_y(0.0, dy),
            Self::Element(_) => {
                if let Some(el) = self.element() {
                    el.scroll_by_with_x_and_y(0.0, dy);
                }
            }
        }
    }
}

// ── 実測 ────────────────────────────────────────────────────────────────────

/// 指定 id の要素群を **容器の内容座標**で測る。
///
/// 1 つでも見つからなければ `None` を返し、呼び側はドラッグを始めない。欠けた箱で
/// 幾何を組むと、押しのけ量が 1 つずれた並びが「それらしく」動いてしまう。
///
/// ★ viewport 座標にしない。iOS は慣性スクロール中の `pointerdown` で `pointercancel` を
/// 送らないことがあり、ドラッグ中に容器が動くと viewport 基準のスナップショットは
/// 陳腐化する。内容座標なら毎 `pointermove` で [`Scroller::offset`] を足し直すだけで
/// 整合が保てる（[`crate::reorder`] のモジュール doc）。
///
/// ★ 呼ぶのは掴んだ 1 回だけ。`pointermove` から呼ぶとレイアウトを 60Hz で強制同期させる。
pub fn measure_slots(ids: &[String], scroller: &Scroller) -> Option<Slots> {
    let sy = scroller.offset();
    let doc = document();
    let mut slots = Vec::with_capacity(ids.len());
    for id in ids {
        let rect = doc.get_element_by_id(id)?.get_bounding_client_rect();
        slots.push(reorder::Slot {
            top: rect.top() + sy,
            height: rect.height(),
        });
    }
    Some(Slots::new(slots))
}

// ── ドラッグ中の状態 ────────────────────────────────────────────────────────

/// ドラッグ中の状態。**掴んだ瞬間のスナップショット**で、指を離すまで測り直さない。
///
/// ★ ドラッグ中に元の `Vec` を入れ替えない。`translateY(dy)` の基準は「掴んだ瞬間の
/// レイアウト位置」なので、`<For>` が DOM を move した瞬間に基準が 1 スロット飛び、
/// 高さがバラバラだと進み幅と戻り幅が一致せず振動が収束しない。
/// 加えて tachys の keyed diff は `insertBefore` で入れ直すので pointer capture を
/// 落としうるうえ、入れ替えの 2 つのうちどちらが move されるかは指定できない。
#[derive(Clone, Debug, PartialEq)]
pub struct Drag {
    /// 掴んだポインタ。2 本目の指の `pointermove` を弾く
    pub pointer_id: i32,
    /// 掴んだ要素の、掴んだ時点での位置
    pub from: usize,
    /// 今指を離したら入る位置。`from` と同じなら何も確定しない
    pub to: usize,
    /// ドラッグが効き始めた瞬間の指の位置（容器の内容座標）
    start_y: f64,
    /// 直近の指の位置（**viewport 座標**）。端の自動スクロールが読む
    client_y: f64,
    /// 掴んだ要素に当てる `translateY`
    lift: f64,
    slots: Slots,
    /// この並びが乗っているスクロール容器。**掴んだときに決めて固定する**
    scroller: Scroller,
}

impl Drag {
    pub fn start(
        pointer_id: i32,
        from: usize,
        client_y: f64,
        slots: Slots,
        scroller: Scroller,
    ) -> Self {
        Self {
            pointer_id,
            from,
            to: from,
            start_y: client_y + scroller.offset(),
            client_y,
            lift: 0.0,
            slots,
            scroller,
        }
    }

    /// 指が動いたぶんを反映する。**レイアウトを読まない**（掴んだときの箱だけで決まる）。
    ///
    /// 容器がスクロールしただけでも呼ぶ（`client_y` は据え置きで `offset()` が動く）。
    pub fn advance(&mut self, client_y: f64) {
        self.client_y = client_y;
        let dy = client_y + self.scroller.offset() - self.start_y;
        self.to = self.slots.drop_index(self.from, dy);
        self.lift = self.slots.lift(self.from, dy);
    }

    /// 模型上 `i` 番目の要素が、いま画面で何番目に見えているか。
    pub fn seen_at(&self, i: usize) -> usize {
        reorder::visual_index(self.from, self.to, i)
    }

    /// 並びの `i` 番目に当てる `transform`。動かないときは `None`。
    ///
    /// `None` を返すとインラインスタイルごと消えるので、静止時の DOM に 1 文字も残らない。
    pub fn transform(&self, i: usize) -> Option<String> {
        let px = if i == self.from {
            self.lift
        } else {
            self.slots.offset(self.from, self.to, i)?
        };
        Some(format!("translateY({px}px)"))
    }
}

// ── 掴む / 畳む ─────────────────────────────────────────────────────────────

/// 掴む資格を確かめ、ポインタを捕まえる。
///
/// ★ **`prevent_default()` を呼ぶ。** これはスクロール対策ではない（それは
/// `touch-action: none` の仕事で、`pointerdown` の preventDefault では iOS の
/// スクロールは止まらない）。止めたいのは **WebKit の選択ドラッグ**で、これを許すと
/// 指が通り過ぎた入力欄に**フォーカスが移ってしまう**。実測（iPhone 15 Pro / WebKit）:
///
/// | 段階 | `.app` | `activeElement` |
/// |---|---|---|
/// | ドラッグ前 | `app` | BODY |
/// | `pointerdown` 後 | `app` | BODY |
/// | **指を動かした後** | **`app kb-open`** | **`set-reps`** |
/// | 指を離した後 | `app kb-open` | BODY |
///
/// 最後の行が致命的で、`<For>` が DOM を move した拍子にフォーカスが**`focusout` を
/// 出さずに**消えるため `kb_blur` が走らず、`.kb-open` が立ちっぱなしになる
/// （＝ `styles.css` の `.kb-open .bottom-tabs { display: none }` でタブバーが消えたまま
/// 戻らない）。Chromium では再現しない。
///
/// 互換 `mousedown` が止まるとフォーカスが外れなくなるので、[`blur_active`] で自分で外す。
///
/// ★ `current_target()` ではなく `target()` を使う。leptos の `delegation` feature は
/// 今 OFF（`csr` に含まれない）なのでハンドラは要素へ直付けされるが、ON になると
/// `current_target` は黙って壊れる。capture 先が子要素（`.card-head` の中の `<h3>`）に
/// なっても、`pointermove` はハンドラのある親まで bubble するので実害が無い。
///
/// ★ capture できなければ掴まない。capture 無しで始めると、指がハンドルの外へ出た
/// 瞬間に `pointermove` も `pointerup` も届かなくなり、**持ち上がったまま固まる**。
pub fn capture(ev: &PointerEvent) -> bool {
    if !ev.is_primary() || ev.button() != 0 {
        return false;
    }
    let ok = ev
        .target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
        .is_some_and(|el| el.set_pointer_capture(ev.pointer_id()).is_ok());
    if ok {
        ev.prevent_default();
        blur_active();
    }
    ok
}

/// 今フォーカスがある要素を外す。
///
/// 掴んだらキーボードは引っ込むべきで、[`capture`] の `prevent_default()` が互換
/// `mousedown` ごと止めてしまうぶんを自分で埋める。`focusout` が出るので
/// `views::mod` の `kb_blur` が普通に走る。
fn blur_active() {
    if let Some(el) = document()
        .active_element()
        .and_then(|el| el.dyn_into::<web_sys::HtmlElement>().ok())
    {
        let _ = el.blur();
    }
}

/// このポインタが掴んでいる最中か。
pub fn holds(drag: RwSignal<Option<Drag>>, ev: &PointerEvent) -> bool {
    drag.with_untracked(|d| d.as_ref().is_some_and(|d| d.pointer_id == ev.pointer_id()))
}

/// ドラッグを畳む。既に畳んでいれば何もしない（`lostpointercapture` は
/// 通常の `pointerup` の後にも飛ぶので、冪等でないと余計な再描画が走る）。
pub fn release(drag: RwSignal<Option<Drag>>) {
    if drag.with_untracked(Option::is_some) {
        drag.set(None);
    }
}

/// `Alt` + ↑↓ を「1 つ上へ / 1 つ下へ」に読む。
///
/// ★ ドラッグの代わりの経路。掴む場所（`.set-no` / `.card-head` / `.rtn-no`）を
/// `<button>` にできないので（`<header>` は `<h3>` を含む・行番号はコントロールではない）、
/// 既にフォーカスできる要素にこれを足す。WCAG 2.1.1 が求める非ドラッグ経路であると同時に、
/// E2E で「並び替えが落ちること」を座標に依存せず書ける経路でもある。
///
/// ★ `Alt` 付きにするのは、素の ↑↓ が入力欄のカーソル操作と衝突するから。
pub fn alt_arrow(ev: &web_sys::KeyboardEvent) -> Option<bool> {
    if !ev.alt_key() {
        return None;
    }
    match ev.key().as_str() {
        "ArrowUp" => Some(true),
        "ArrowDown" => Some(false),
        _ => None,
    }
}

/// 端の自動スクロールの 1 フレーム。`drag` が畳まれたら自分で止まる。
///
/// ★ `pointermove` では成立しない。**指が止まっていてもスクロールし続ける**必要がある
/// （画面外の要素まで運ぶのが目的なので、指は端に置いたまま待つのが普通の使い方）。
fn edge_scroll_tick(drag: RwSignal<Option<Drag>>) {
    // ★ `try_` で読む。掴めるものは全部タブ切替やシートの開閉で破棄されるので、
    //   ドラッグ中に切り替えるとこの signal はもう無い。`get_untracked` だと panic して
    //   wasm ごと落ちる。**フラグを下ろすのは早期 return の全経路で**。1 つでも漏らすと
    //   再入防止のフラグが立ちっぱなしになり、以後この機能が二度と動かない
    let Some(Some((client_y, scroller))) =
        drag.try_with_untracked(|d| d.as_ref().map(|d| (d.client_y, d.scroller.clone())))
    else {
        EDGE_SCROLLING.set(false);
        return;
    };
    if let Some((top, bottom)) = scroller.band() {
        let step = reorder::edge_scroll_step(client_y, top, bottom, EDGE_STEP_PX);
        if step != 0.0 {
            scroller.scroll_by(step);
            // スクロールしたぶん dy が変わる。指が 1px も動いていなくても追随させる
            drag.update(|d| {
                if let Some(d) = d {
                    d.advance(d.client_y);
                }
            });
        }
    }
    // ★ 端に居ないフレームでも回し続ける（指が後から端へ入ってくる）。読むのは
    //   `client_y` の 1 つだけなので、止める価値のあるコストは乗っていない
    request_animation_frame(move || edge_scroll_tick(drag));
}

/// 端の自動スクロールを回し始める。**既に回っていれば何もしない。**
///
/// 再入すると 2 本のループが同じ容器をスクロールして倍の速さで流れる。
pub fn start_edge_scroll(drag: RwSignal<Option<Drag>>) {
    if !EDGE_SCROLLING.replace(true) {
        edge_scroll_tick(drag);
    }
}
