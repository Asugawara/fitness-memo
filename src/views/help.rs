//! ホーム画面への追加の案内。記録タブのバナーと、そこから開く手順シート。
//!
//! iOS では Safari のタブと standalone PWA で `localStorage` が**共有されない**。先に
//! Safari のタブで記録すると、ホーム画面に追加したあとで全部見えなくなる。エクスポート／
//! インポートが無い今は回復不能なので、記録を始める前に気付かせる必要がある。
//!
//! 導線を 2 層にしてある。
//!
//! - [`InstallBanner`]: 記録タブ末尾（「種目を追加」より下）。standalone でなく、かつ
//!   ストレージが分かれうる環境のときだけ出る。**手順を必要とする人は必ずこの状態に
//!   いる**（人に勧めるときも渡すのは URL なので、相手側にバナーが出る）ので、これが
//!   本命の導線。ただし初期表示では折り返しの下にあり、スクロールしないと見えない。
//! - [`InstallHelpLink`]: 種目タブ冒頭。standalone になったあとの読み返しと、
//!   [`super::storage_may_split`] が将来の UA 変更で壊れたときの逃げ道。
//!
//! ★ 図は `assets/help/*.svg` を `include_str!` で埋め込み `inner_html` で挿す。
//!   `public/` に置かないのは、`scripts/stamp-sw.sh` が配信物から Service Worker の
//!   プリキャッシュ一覧を作るため（`sw.js` の `cache.addAll` は 1 つ失敗すると install
//!   全体が失敗する原子的操作なので、シェルのエントリは増やしたくない）。
//!
//! ★ SVG ファイルに `<?xml … ?>` や DOCTYPE を書いてはいけない。`innerHTML` は HTML
//!   フラグメントパーサなので bogus comment になって画面に何も出ない。Illustrator や
//!   Inkscape は既定でこれを吐くので、書き出したものを貼るときは必ず削る。
//!
//! ★ 色は SVG 側に持たせず `public/styles.css` の `.hlp-*` クラスで与える。
//!   `fill="var(--accent)"` は presentation attribute では解決されない
//!   （`super::chart` と `.chart-*` が同じ作り）。

use leptos::prelude::*;

use crate::storage;

use super::{is_standalone, storage_may_split};

const STEP1_SVG: &str = include_str!("../../assets/help/step1-share.svg");
const STEP2_SVG: &str = include_str!("../../assets/help/step2-add.svg");
const STEP3_SVG: &str = include_str!("../../assets/help/step3-confirm.svg");

/// 記録タブ末尾の警告バナー。押すと手順シートが開く。
///
/// トーンは `.warn-box` 寄りの枠線で、`.notice` のような塗りにはしない。記録タブに
/// 常駐して毎回目に入るので、塗りは閉じられる一時通知のほうに残す。
#[component]
pub fn InstallBanner() -> impl IntoView {
    let open = RwSignal::new(false);

    // 環境の判定は一度きりの評価で足りる。タブを切り替えると `match tab.get()` が
    // Calendar ごと作り直すので、追加したあとに記録タブへ戻ってくれば消える
    let applicable = !is_standalone() && storage_may_split();

    // ★ 「閉じたか」だけシグナルにする。✕ を押した瞬間に消えてほしいが、
    //   localStorage を読み直すのは起動時の 1 回で足りる
    let dismissed = RwSignal::new(storage::install_hint_dismissed());

    view! {
        // ★ `dismissed.get()` を先に書く。`applicable && !dismissed.get()` にすると
        //   `applicable` が false のとき短絡してシグナルを購読せず、依存ゼロの
        //   クロージャになる。今は無害だが、後から `applicable` をシグナル化した
        //   ときに順序依存の罠として顕在化する
        {move || {
            (!dismissed.get() && applicable)
                .then(|| {
                    view! {
                        // ★ 箱は <div>。<button> の入れ子は不正な HTML なので、
                        //   「シートを開く」と「閉じる」を兄弟のボタンに分ける
                        <div class="install-hint" data-testid="install-hint">
                            <button
                                class="install-hint-body"
                                data-testid="install-hint-open"
                                on:click=move |_| open.set(true)
                            >
                                <span class="install-hint-text">
                                    <strong>"記録を付ける前にホーム画面に追加してください"</strong>
                                    "Safari のタブで付けた記録は引き継がれません"
                                </span>
                                // ★ aria-hidden を付けないこと。付けるとアクセシブル名から
                                //   落ち、支援技術には「押すと何が起きるか」を示す唯一の
                                //   語が届かなくなる（残るのは警告文だけになる）
                                <span class="install-hint-cta">"追加のしかた ›"</span>
                            </button>
                            // aria-label は残す。見た目は ✕ でも支援技術と E2E の
                            // role+name には言葉で届く必要がある
                            <button
                                class="icon-btn"
                                aria-label="この案内を今後表示しない"
                                data-testid="install-hint-dismiss"
                                on:click=move |_| {
                                    storage::dismiss_install_hint();
                                    dismissed.set(true);
                                }
                            >
                                "✕"
                            </button>
                        </div>
                        // シートも条件の中に置く。バナーが出ない状態では `open` を
                        // 立てる手段が無いので、外に出すと開けないシートが残るだけ
                        <InstallHelpSheet open=open />
                    }
                })
        }}
    }
}

/// 種目タブ冒頭の控えめなリンク。押すと同じ手順シートが開く。
///
/// ★ `storage_may_split` で絞らない。受動的なリンクなので無関係な環境に出ても軽い
///   無駄で済む一方、判定が壊れたときはここだけが手順への入口になる。
///
/// ★ 種目タブの**冒頭**に置くこと。末尾は `.add-wrap` が `position: sticky` で
///   居座るので後続の要素が帯に覆われ、`ArchivedSection` の有無で位置も動く。
#[component]
pub fn InstallHelpLink() -> impl IntoView {
    let open = RwSignal::new(false);

    view! {
        <p class="menu-help">
            <button
                class="link-btn"
                data-testid="install-help-link"
                on:click=move |_| open.set(true)
            >
                "ホーム画面への追加のしかた"
            </button>
        </p>
        <InstallHelpSheet open=open />
    }
}

/// 手順シート。`super::day` の「種目を追加」シートと同じ組み方にする。
///
/// z-index は `public/styles.css` が持つのでインライン `style` は書かない
/// （`super::menu` は歴史的にインラインで持っているが、あれは今は冗長）。
#[component]
fn InstallHelpSheet(open: RwSignal<bool>) -> impl IntoView {
    view! {
        {move || {
            open.get()
                .then(|| {
                    view! {
                        <div
                            class="sheet-backdrop"
                            data-testid="install-sheet-backdrop"
                            on:click=move |_| open.set(false)
                        ></div>
                        <div
                            class="sheet"
                            role="dialog"
                            aria-label="ホーム画面に追加"
                            data-testid="install-sheet"
                        >
                            <header class="sheet-head">
                                <strong>"ホーム画面に追加"</strong>
                                // aria-label は残す。見た目は ✕ でも支援技術と
                                // E2E の role+name には「閉じる」で届く必要がある
                                <button
                                    class="icon-btn"
                                    aria-label="閉じる"
                                    data-testid="install-sheet-close"
                                    on:click=move |_| open.set(false)
                                >
                                    "✕"
                                </button>
                            </header>
                            <div class="sheet-body">
                                <div class="warn-box">
                                    <p>
                                        "iPhone では、Safari のタブとホーム画面のアプリで記録の保存場所が分かれています。"
                                    </p>
                                    <p>
                                        "Safari のタブで付けた記録は、ホーム画面に追加したあとでは見えません。まだ記録していないなら、先に追加してください。"
                                    </p>
                                </div>

                                <p class="hlp-why muted">
                                    "追加すると、電波の届かないジムでも開けて、ホーム画面のアイコンから 1 タップで起動します。"
                                </p>

                                <section class="hlp-step">
                                    <h3>"1. 画面の下のまん中にある共有ボタンを押す"</h3>
                                    <div class="hlp-fig" inner_html=STEP1_SVG />
                                    <p class="hlp-note">
                                        <strong>"Safari で開いてください。"</strong>
                                        "他のブラウザだとこの手順は使えません。"
                                    </p>
                                </section>

                                <section class="hlp-step">
                                    <h3>"2. 「ホーム画面に追加」を選ぶ"</h3>
                                    <div class="hlp-fig" inner_html=STEP2_SVG />
                                    <p class="hlp-note">"リストを下にスクロールすると出てきます。"</p>
                                </section>

                                <section class="hlp-step">
                                    <h3>"3. 右上の「追加」を押す"</h3>
                                    <div class="hlp-fig" inner_html=STEP3_SVG />
                                </section>

                                <p class="hlp-cap muted">
                                    "図は iPhone を縦向きで使っているときの画面です。iPad では共有ボタンは画面の上のほうにあります。"
                                </p>

                                <section class="hlp-step">
                                    <h3>"追加できたかの確かめ方"</h3>
                                    <p class="hlp-note">
                                        "ホーム画面のアイコンから開くと、この注意書きが出なくなります。まだ出ているならブラウザのタブのままです。"
                                    </p>
                                </section>

                                // ★ エクスポート／インポートが入ったらこのブロックだけを
                                //   「Safari 側でエクスポート → アプリ側でインポート」に差し替える
                                <section class="hlp-step">
                                    <h3>"すでに Safari で記録してしまった場合"</h3>
                                    <p class="hlp-note">
                                        "移す方法は今のところありません。Safari のタブを開いたままにして、ホーム画面のアプリに手で入れ直してください。"
                                    </p>
                                </section>
                            </div>
                        </div>
                    }
                })
        }}
    }
}
