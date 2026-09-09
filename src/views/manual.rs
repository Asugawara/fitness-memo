//! 設定タブの使い方マニュアル節と、記録タブに初回だけ出す手掛かり。
//!
//! 章の骨格は [`crate::manual::MANUAL_CHAPTERS`]、文言は [`crate::i18n::Manual`] /
//! [`crate::i18n::ChapterText`] にある。ここは両者を突き合わせて描くだけで、
//! 表示用の日本語リテラルは 1 つも持たない
//! （`rg '"[^"]*[ぁ-んァ-ヶ一-龠]' src/ -g '!src/i18n.rs'` で検証済みの規約）。
//!
//! ★ 図はまだ 1 枚も無い（[`crate::manual::MANUAL_CHAPTERS`] は全章 `has_fig: false`）。
//!   `<img>` を描く分岐は書いてあるが `ChapterText::fig` が全章 `None` の間は実行されない。
//!   コミット #4 で図と実測寸法が入ると、コードを変えずに効き始める設計になっている。

use leptos::prelude::*;

use crate::manual::MANUAL_CHAPTERS;
use crate::storage;

use super::icon::{self, icon};
use super::{
    SettingsPage, Tab, cur_lang, is_standalone, scroll_into_view_if_needed, storage_may_split, t,
    use_dates, use_open_manual, use_settings_page, use_tab,
};

fn chapter_dom_id(id: &str) -> String {
    format!("man-{id}")
}

/// 使い方マニュアル節の本体。`SettingsPage::Manual` から呼ばれる。
///
/// ★ h1 は呼び出し側（`settings.rs` の `back_head`）が持つので、ここは h2 から始まる
///   （h1 → h2 の 1 本、adr/ux/focus-ring-and-heading-order.md）。
#[component]
pub fn ManualSection() -> impl IntoView {
    let open = use_open_manual();
    let m = &t().manual;

    // ★ 開いた章は必ず画面に入れる。`settings.rs` の `open_group` と同じ理由 —
    //   `open.set(..)` するだけでは、末尾の章を開いたときに見出しが画面外のまま
    //   本文だけが下に伸びる。`block: nearest` なので、既に見えている章は動かさない
    Effect::new(move |_| {
        if let Some(i) = open.get()
            && let Some(ch) = MANUAL_CHAPTERS.get(i)
        {
            scroll_into_view_if_needed(chapter_dom_id(ch.id));
        }
    });

    view! {
        <div class="man-section" data-testid="man-section">
            <p class="man-intro">{m.intro}</p>
            <p class="muted man-note">{m.offline_note}</p>
            <p class="muted man-note man-light-note">{m.light_note}</p>
            <p class="muted man-note">{m.see_install_help}</p>

            <div data-testid="man-chapters">
                {MANUAL_CHAPTERS
                    .iter()
                    .zip(m.chapters)
                    .enumerate()
                    .map(|(i, (ch, text))| {
                        view! {
                            <section
                                class="man-group"
                                id=chapter_dom_id(ch.id)
                                data-testid="man-group"
                            >
                                <h2>
                                    <button
                                        class="grp-toggle"
                                        aria-expanded=move || {
                                            if open.get() == Some(i) { "true" } else { "false" }
                                        }
                                        data-testid="man-toggle"
                                        on:click=move |_| {
                                            open
                                                .update(|o| {
                                                    *o = (*o != Some(i)).then_some(i);
                                                });
                                        }
                                    >
                                        {icon(icon::CHEVRON_RIGHT)}
                                        <span class="grp-name">{text.title}</span>
                                    </button>
                                </h2>
                                {move || {
                                    (open.get() == Some(i))
                                        .then(|| {
                                            view! {
                                                <div class="man-body" data-testid="man-body">
                                                    {text
                                                        .body
                                                        .iter()
                                                        .map(|p| view! { <p>{*p}</p> })
                                                        .collect::<Vec<_>>()}
                                                    {text
                                                        .fig
                                                        .map(|(w, h)| {
                                                            view! {
                                                                <figure class="man-fig">
                                                                    <img
                                                                        src=format!(
                                                                            "manual/{}/{}.webp",
                                                                            cur_lang().tag(),
                                                                            ch.id,
                                                                        )
                                                                        width=w
                                                                        height=h
                                                                        alt=text.fig_alt
                                                                        loading="lazy"
                                                                        decoding="async"
                                                                        data-testid="man-fig"
                                                                    />
                                                                </figure>
                                                            }
                                                        })}
                                                </div>
                                            }
                                        })
                                }}
                            </section>
                        }
                    })
                    .collect::<Vec<_>>()}
            </div>
        </div>
    }
}

/// 記録タブに初回だけ出す、マニュアルへの手掛かり。
///
/// ★ 呼び出し側（`calendar.rs`）で **`<DayEditor />` の直前**に置くこと。末尾
///   （`InstallBanner` の近く）は上端 685px で可視域外になる実測があり
///   （adr/ux/install-guide-banner-and-sheet.md:78）、「気づきづらい説明が
///   気づきづらい場所にある」矛盾をそのまま引き継ぐ。
///
/// ★ install バナーが実際に出ているときは出さない。`applicable`
///   （`!is_standalone() && storage_may_split()`）単独では判定しない — それだと
///   install バナーを ✕ で消した人・PC ブラウザ・Playwright の chromium project で
///   永久に出ない。`help::InstallBanner` と同じ式をここでも独立に評価する
///   （バナー自身の `dismissed` シグナルはコンポーネントが別なので見えない。
///   `InstallBanner` の環境判定自身が「タブ往復で作り直されるまでは一度きりの
///   評価で足りる」設計なので、ここでも同じ粒度で揃える）。
#[component]
pub fn ManualHint() -> impl IntoView {
    let dates = use_dates();
    let tabs = use_tab();
    let page = use_settings_page();
    let m = &t().manual;

    let install_showing =
        !storage::install_hint_dismissed() && !is_standalone() && storage_may_split();

    let dismissed = RwSignal::new(storage::manual_hint_dismissed());

    let go_to_manual = move |_| {
        storage::dismiss_manual_hint();
        dismissed.set(true);
        tabs.switch(dates, Tab::Settings);
        page.set(SettingsPage::Manual);
    };

    view! {
        {move || {
            (!dismissed.get() && !install_showing)
                .then(|| {
                    view! {
                        <div class="man-hint" data-testid="manual-hint">
                            <p>{m.hint_body}</p>
                            <div class="man-hint-actions">
                                <button
                                    class="man-hint-cta"
                                    data-testid="manual-hint-cta"
                                    on:click=go_to_manual
                                >
                                    {m.hint_cta}
                                </button>
                                <button
                                    class="icon-btn"
                                    aria-label=m.hint_dismiss
                                    data-testid="manual-hint-dismiss"
                                    on:click=move |_| {
                                        storage::dismiss_manual_hint();
                                        dismissed.set(true);
                                    }
                                >
                                    {icon(icon::X)}
                                </button>
                            </div>
                        </div>
                    }
                })
        }}
    }
}
