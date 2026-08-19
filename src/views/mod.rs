//! 画面の共通土台。ボトムタブ 3 つ + 全画面が依存する日付コンテキスト。

pub mod backup;
pub mod calendar;
pub mod chart;
pub mod day;
pub mod drag;
pub mod help;
pub mod icon;
pub mod progress;
pub mod routine;
pub mod settings;

use std::cell::Cell;
use std::time::Duration;

use chrono::{Datelike, Local, NaiveDate};
use leptos::prelude::*;
// シートを閉じたときのフォーカス復帰で Element → HtmlElement に落とすのに使う
use wasm_bindgen::JsCast;

use crate::i18n::{self, Lang, S};
use crate::model::{Db, GroupId, SetEntry};
use crate::storage;

use calendar::Calendar;
use icon::icon;
use progress::Progress;
use settings::Settings;

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

/// 設定タブで開いている部位。**同時に開くのは 1 つ**
/// （adr/ux/menu-groups-as-single-open-accordion.md）。
///
/// ★ `Settings` の中ではなくここに置くのが要点。`match tab.get()` はタブを切り替える
///   たびに `Settings` を作り直すので、コンポーネント内のシグナルにすると記録⇄設定を
///   往復するたびに全部閉じる。筋トレ中はその往復が常なので、戻るたびに部位を探して
///   押し直すことになる。
/// ★ それでも**永続化はしない**。`Db` 由来の ID を UI 状態のキーに入れると `Db` から
///   部位が消えたときに宙に浮く（adr/storage/ui-state-in-separate-key.md が
///   前提の崩れる例として名指ししている形）。
///   ここはプロセス内の寿命に留める。
#[derive(Clone, Copy)]
pub struct OpenGroupCtx(pub RwSignal<Option<GroupId>>);

/// 設定タブで開いているページ（adr/ux/settings-as-a-list-of-sections.md）。
///
/// ★ `OpenGroupCtx` とまったく同じ理由でここに置く。`Settings` の中に持つと、
///   記録⇄設定を往復するたびに一覧のトップへ戻され、入っていた節をもう一度
///   開き直すことになる。**永続化はしないのも同じ**（プロセス内の寿命に留める）。
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum SettingsPage {
    #[default]
    Root,
    Routines,
    Exercises,
    Language,
}

#[derive(Clone, Copy)]
pub struct SettingsPageCtx(pub RwSignal<SettingsPage>);

pub fn use_settings_page() -> RwSignal<SettingsPage> {
    use_context::<SettingsPageCtx>()
        .expect("SettingsPageCtx が provide されていない")
        .0
}

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

/// UI の言語（adr/ux/language-follows-the-browser-then-the-setting.md）。
///
/// ★ 他のコンテキストと違い、**文言はこのシグナルを購読しない**（[`t`] は
///   `get_untracked`）。264 箇所の文字列を全部 `move ||` で包むのは現実的でないので、
///   反映経路を `App` の 1 箇所に集約している（`App` の該当箇所のコメントを参照）。
#[derive(Clone, Copy)]
pub struct LangCtx(pub RwSignal<Lang>);

pub fn use_lang() -> RwSignal<Lang> {
    use_context::<LangCtx>()
        .expect("LangCtx が provide されていない")
        .0
}

thread_local! {
    /// 現在の言語のプロセス内キャッシュ。`App` が [`LangCtx`] と同期させる。
    ///
    /// ★ **コンテキストではなくここから読む理由。** `use_context` はリアクティブな
    ///   owner の中でしか引けない。文言が要る場所はイベントハンドラ（書き出しボタン、
    ///   ファイル選択のコールバック）にもあり、そこで `use_context` を呼ぶと
    ///   `None` が返って `expect` で落ちる — 実際に「壊れたファイルの取り込み」で
    ///   踏んだ。読み取りを owner から独立させて、この失敗クラスごと無くす。
    ///
    /// ★ グローバル可変状態だが **UI 層に閉じている**。`core` / `presets` / `i18n` は
    ///   今までどおり `Lang` を引数で受けるので、ホストの `cargo test` が
    ///   言語ごとの挙動を検証できる性質は失われない
    ///   （adr/architecture/i18n-hand-rolled-string-table.md）。
    static CURRENT_LANG: Cell<Lang> = const { Cell::new(Lang::Ja) };
}

/// 現在の文言表。**コンポーネント本体の先頭で 1 回だけ引く。**
///
/// ★ 購読しないのが要点。ここで購読すると「文言を読んだだけの」コンポーネントが
///   軒並み lang の購読者になり、切り替え時の作り直しが `App` の境界と二重に走る。
///   反映を起こすのは `App` の `lang.get()` 1 箇所だけにしてある。
pub fn t() -> &'static S {
    cur_lang().strings()
}

/// 現在の言語。`fmt_date` のように `Lang` そのものを要求する関数へ渡すために引く。
///
/// ★ 名前が `lang` でないのは、Rust の組み込み属性 `#[lang = ".."]` と綴りが衝突して
///   呼び出しが属性に解決されてしまうため。
pub fn cur_lang() -> Lang {
    CURRENT_LANG.get()
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

/// 曜日の短縮表記。
///
/// ★ 表は `i18n::Cal::weekdays` に一本化してある（`views/calendar.rs` の `WEEKDAYS` と
///   ここの `weekday_ja` に同じ並びが二重にあった）。**日曜始まり**なので
///   `num_days_from_sunday()` の 0..=6 がそのまま添字になる。
pub fn weekday(d: NaiveDate, lang: Lang) -> &'static str {
    lang.strings().cal.weekdays[d.weekday().num_days_from_sunday() as usize]
}

/// "8/8 (金)" / "Aug 8 (Fri)"
///
/// ★ **英語で `8/8` を使わない。** 米式 M/D と英式 D/M は見た目が同じで意味が違うので、
///   8 月 8 日以外は読み手のロケール次第で別の日に読める。月名の略記にすれば
///   曖昧さが構造的に消える。カッコ付き曜日の形は日本語と揃えてあるので、
///   CSS もレイアウトも動かない。
pub fn fmt_date(d: NaiveDate, lang: Lang) -> String {
    let wd = weekday(d, lang);
    match lang {
        Lang::Ja => format!("{}/{} ({})", d.month(), d.day(), wd),
        Lang::En => format!(
            "{} {} ({})",
            lang.strings().cal.months_short[d.month0() as usize],
            d.day(),
            wd
        ),
    }
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

// 重量・レップの整形とパースは `core` にある（書き出しの TSV が同じ関数を通るので、
// ホストの `cargo test` で検証できる側に置いた）。ここから使い続けられるよう再輸出する。
pub use crate::core::{fmt_weight, parse_reps, parse_weight};

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

/// ブラウザが申告する言語。**`ja` で始まるものだけ日本語**、それ以外は英語。
///
/// ★ `navigator.languages`（優先順の配列）は見ない。1 本目で足りるし、web-sys の
///   `languages()` は `js_sys::Array` を返すので走査のコードが要る。
/// ★ 判定本体は `i18n::from_bcp47` に置いてある（ホストの `cargo test` で検証するため）。
pub fn browser_lang() -> Lang {
    window()
        .navigator()
        .language()
        .map_or(Lang::En, |tag| i18n::from_bcp47(&tag))
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
///
/// ★ **開いたときのフォーカスは `<dialog>` 自身に置く。** `show_modal()` は中の最初の
///   フォーカス可能要素（= ✕）を選ぶが、そこへ残すと **iPhone Safari で**「タップした
///   だけで青枠が出る」になるので `tabindex="-1"` の dialog へ引き取っている。
///   **中身の入力欄にフォーカスは当てない。** iOS は `focus()` でキーボードを出すので、
///   シートを開いた瞬間に画面の下半分が埋まる（adr/pwa/hide-tabs-when-keyboard-open.md）。
///   呼び出し側で「開いたら入力欄へ」をやりたくなったら、まずここを読むこと。
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
                //   シート内の先頭（✕ ボタン）へ移ってしまう（その先は下の focus() で
                //   <dialog> 自身へ引き取る）。
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
                // ★ 初期フォーカスを <dialog> 自身へ引き取る（WAI-ARIA APG の
                //   「モーダルを開いたらダイアログにフォーカス」）。放っておくと
                //   dialog focusing steps が「中の最初のフォーカス可能要素」＝ ✕ を選び、
                //   **指でタップして開いただけで閉じるボタンに青枠が出る**。
                //   利用者からは「何も押していないのに印が出た」と見える。
                //
                //   ★ これは **iPhone Safari で起きる**。Playwright 実測（iPhone 15 Pro /
                //   Desktop Chrome / Pixel 7）で、フォーカスが ✕ に移るのは両エンジン共通だが、
                //   `:focus-visible` にマッチするのは **WebKit だけ**（Chromium は false で
                //   リングが出ない）。「Safari はボタンをタップしてもフォーカスを与えない」
                //   （下の opener のコメント）とは別の経路で、show_modal() の初期フォーカスは
                //   UA が能動的に与えるものであり、WebKit はそれをキーボード相当と判定する。
                //
                //   ★ show_modal() と**同じタスクの中**で移すこと。dialog focusing steps は
                //   show_modal() の一部として同期に走り、描画はタスク終了後なので、
                //   ✕ にリングが 1 フレーム出ることは無い。ここを上の
                //   scroll_into_view_if_needed のように request_animation_frame で
                //   遅らせると本当に 1 フレーム出る。
                //
                //   ★ フォーカスがここに載ることと、リングが**描かれない**ことは別の話。
                //   実測では WebKit が今度は **<dialog> 自身**を :focus-visible にマッチさせる
                //   ので、これだけでは「✕ の青枠」が「シート上辺の青線」に化けるだけになる。
                //   リングは styles.css の `.sheet:focus-visible { outline: none }` が受け持つ
                let _ = d.focus();
                // ★ **開くたびに中身を先頭へ戻す。** `<dialog>` は常時マウント
                //   （このコンポーネントの doc）なので `.sheet-body` の DOM 要素は
                //   閉じても生き続け、`scrollTop` を覚えたままになる。前回の操作で
                //   下まで送っていると、次に開いたシートは**見出しも入力欄も画面外**の
                //   状態で現れる。実測（Playwright / iPhone 15 Pro）で、メニュー編集
                //   シートを一度閉じて開き直すと `scrollTop` が 446 のまま出た
                //   （Chromium は中身が空になる 1 tick で 0 に潰れるので出ない）。
                //   ★ これはシート 5 枚すべての話で、長いシートほど強く効く。
                if let Some(body) = d.query_selector(".sheet-body").ok().flatten() {
                    body.set_scroll_top(0);
                }
            }
        } else if d.open() {
            d.close();
        }
    });

    view! {
        <dialog
            node_ref=dialog
            class="sheet"
            // ★ 上の focus() の受け皿。tabindex="-1" は「Tab では止まらないが focus() では
            //   受けられる」なので **Tab 順は 1 つも変わらない**（シートに入って最初の
            //   Tab は今までどおり ✕）。これが無いと「開いた modal dialog が focusable か」が
            //   エンジン任せになり、focus() が黙って no-op になりうる
            tabindex="-1"
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
                    aria-label=t().common.close
                    data-testid=close_testid
                    on:click=move |_| on_close.run(())
                >
                    {icon(icon::X)}
                </button>
            </header>
            // ★ **id を振る。** `.sheet-body` は `overflow-y: auto` の入れ子のスクロール
            //   容器で、この中でドラッグする画面（`views::routine` の「選択中」）は
            //   `window().scroll_y()` ではなくここの `scrollTop` を読む必要がある
            //   （`views::drag::Scroller`）。シートは全部常時マウントなので id は
            //   衝突してはならず、`testid` が既にシートごとに一意なのでそれを使う。
            //   `Scroller::of` は `closest(".sheet-body")` で辿ってこの id を拾う
            <div class="sheet-body" id=format!("{testid}-body")>
                {children()}
            </div>
        </dialog>
    }
}

// ── タブ ────────────────────────────────────────────────────────────────────

/// ボトムタブ。
///
/// `Record` はカレンダーと選択日のエディタを 1 画面に載せたもの。
/// 以前は「今日」と「カレンダー」が別タブで、過去日を直すたびに往復していた。
///
/// `Settings` は旧「種目タブ」。トレーニングメニューを作る場所を足した時点で、
/// 種目マスタ・部位・書き出し読み込み・ホーム画面への追加を抱える画面になったので
/// 名前を実態に合わせた（adr/ux/start-from-a-saved-routine.md）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Record,
    Progress,
    Settings,
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
        // 押すたびに <main class="screen"> の中身が丸ごと作り直され、入力中の値が飛ぶ
        if self.0.get_untracked() == to {
            return;
        }
        self.0.set(to);
    }
}

#[component]
pub fn App() -> impl IntoView {
    // ★ **言語を最初に決める。** `storage::load` はプリセットを投入する言語と起動時通知の
    //   文言の両方にこれを使うので、`Db` より先でなければならない。
    //   解決順序は「明示的に選ばれた値 → ブラウザの申告 → 英語」
    //   （adr/ux/language-follows-the-browser-then-the-setting.md）
    let initial_lang = storage::saved_lang().unwrap_or_else(browser_lang);
    // ★ シグナルより先にキャッシュを埋める。`storage::load` も、この後に走る
    //   どのコンポーネントも `cur_lang()` から読む
    CURRENT_LANG.set(initial_lang);
    let lang = RwSignal::new(initial_lang);
    provide_context(LangCtx(lang));

    let (initial, restore_note) = storage::load(lang.get_untracked());
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

    // ★ 設定タブの外に置く（タブ往復で閉じない / トップへ戻らないため）。
    //   理由は OpenGroupCtx と SettingsPage を参照
    provide_context(OpenGroupCtx(RwSignal::new(None)));
    provide_context(SettingsPageCtx(RwSignal::new(SettingsPage::default())));

    let tab = RwSignal::new(Tab::Record);
    let tabs = TabCtx(tab);
    provide_context(tabs);

    let notice = RwSignal::new(restore_note);

    // Db の変更を購読して 400ms debounce で保存する
    Effect::new(move |_| {
        storage::save_debounced(db.get());
    });

    // ★ `<html lang>` を UI に合わせる。**任意ではなく必須** — スクリーンリーダーは
    //   この属性から読み上げ音声を選ぶので、日本語の画面が `lang="en"` のままだと
    //   英語音声で読まれて使い物にならない。CJK のフォント選択にも効く。
    //
    //   index.html の静的な値は `en`（クローラが見るのはそれで、英語の description /
    //   og:locale と揃えてある — adr/seo/static-metadata-in-english.md）。
    //   ここは実行時の上書きで、切り替えのたびに追随する
    Effect::new(move |_| {
        let l = lang.get();
        // ★ キャッシュを先に更新する。この後の作り直しで各画面が `cur_lang()` を読む
        CURRENT_LANG.set(l);
        if let Some(root) = document().document_element() {
            let _ = root.set_attribute("lang", l.tag());
        }
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
                    lang.get_untracked()
                        .strings()
                        .common
                        .save_failed
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
            // ★ **ここが言語の唯一の反映経路。** 各画面は `t()` で文言を
            //   非リアクティブに読む（264 箇所を `move ||` で包まないため）ので、
            //   切り替えたら中身を丸ごと作り直す。
            //
            //   ★ 失われるのはコンポーネント局所のシグナルだけ（編集シートの開閉、
            //     カレンダーの表示月、推移タブの対象/指標/期間）。`DbCtx` / `DateCtx` /
            //     `TabCtx` / `SettingsPageCtx` / `OpenGroupCtx` は `App` が持っているので
            //     残り、切り替えた人は「設定 > 言語」に留まる。
            //   ★ 押したボタン自身がこの中で破棄されるが、leptos は現在のイベント
            //     ハンドラを抜けてから破棄するので安全（設定の「‹ 設定」で実証済み）。
            {move || {
                let t = lang.get().strings();
                view! {
                    {move || {
                        notice
                            .get()
                            .map(|msg| {
                                view! {
                                    <div class="notice" role="status" data-testid="restore-notice">
                                        <span>{msg}</span>
                                        <button
                                            class="icon-btn"
                                            aria-label=t.common.close_notice
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
                            Tab::Settings => view! { <Settings /> }.into_any(),
                        }}
                    </main>

                    <nav class="bottom-tabs" data-testid="bottom-tabs">
                        {tab_button(Tab::Record, t.common.tab_record, "tab-record")}
                        {tab_button(Tab::Progress, t.common.tab_progress, "tab-progress")}
                        {tab_button(Tab::Settings, t.common.tab_settings, "tab-settings")}
                    </nav>
                }
            }}
        </div>
    }
}
