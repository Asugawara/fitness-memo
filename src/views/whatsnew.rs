//! 新機能のお知らせバナー。
//!
//! リリースのたびに機能を足しているが、それを利用者に伝える手段が今のアプリには
//! 無い。画面最上段に細いバナーを出し、押すと未読のリリースをまとめて 1 枚のシートで
//! 見せる。閉じたら二度と出さない（adr/ux/whats-new-banner-above-the-screen.md）。
//!
//! お知らせ本文は [`crate::i18n::RELEASES`] に持つ。お知らせは文言そのものなので
//! `i18n.rs` の外に置かない規約の対象で、[`crate::presets::Names`] のような
//! 「データ」の例外には当たらない。未読の切り出しは [`core::unseen_releases`]
//! （ホストの `cargo test` が届く側）に任せる。

use leptos::prelude::*;

use crate::core;
use crate::i18n::ReleaseNote;
use crate::storage;

use super::icon::{self, icon};
use super::{Sheet, cur_lang, t};

/// 起動時に 1 回だけ呼ぶ。既読を記録していなければ基準値を書き、未読を返す。
///
/// ★ **`None`（未設定）に基準値を書かないと `None` が永久に残り、次のリリースも
///   出なくなる**（`None` は「全部既読」の意味なので）。`storage::save_release_seen`
///   が書けない環境（Safari のプライベートブラウズ等）では以後も何も出せないが、
///   `store()` が使えない環境そのものが無害に倒れる設計（`storage.rs` 冒頭）に
///   合わせて許容する。
///
/// ★ 呼ぶのは `App` 本体で 1 回だけ（`mod.rs` の言語切替クロージャの**外**）。あの
///   クロージャは言語を切り替えるたびに走り直すので、中に置くと「起動時 1 回」が
///   偶然の性質になる（結果は冪等なので壊れはしないが、意図が読み取れなくなる）。
pub fn bootstrap() -> &'static [ReleaseNote] {
    let seen = storage::release_seen();
    if seen.is_none() {
        if let Some(id) = core::latest_release_id(crate::i18n::RELEASES) {
            storage::save_release_seen(id);
        }
        return &[];
    }
    core::unseen_releases(crate::i18n::RELEASES, seen)
}

/// 画面最上段のバナー。未読が無ければ何も描かない。
///
/// ★ `unseen` は [`bootstrap`] が起動時に 1 回評価した値をそのまま prop で受ける。
///   言語切替のたびにこのコンポーネント自体は作り直されるが（`mod.rs` の言語切替
///   クロージャの内側に置かれるため）、`unseen` の値そのものは動かない。
#[component]
pub fn WhatsNewBanner(unseen: &'static [ReleaseNote]) -> impl IntoView {
    if unseen.is_empty() {
        return ().into_any();
    }

    let open = RwSignal::new(false);
    // ★ 初期値は「今の `release_seen` でまだ未読が残っているか」を素直に見る
    //   （`views::help::InstallBanner` の `dismissed` が起動時の
    //   `storage::install_hint_dismissed()` を読むのと同じ理由）。`false` 固定だと、
    //   閉じた直後に言語を切り替えたときこのコンポーネントごと作り直されて
    //   `dismissed` が false へ巻き戻り、既読済みのバナーが同じセッション内で
    //   もう一度出てしまう。
    let dismissed =
        RwSignal::new(core::unseen_releases(unseen, storage::release_seen()).is_empty());

    // 既読を確定させる。✕ とシートを閉じる操作の両方がここを通る。
    //
    // ★ `unseen` は新しい順なので、先頭が確定させるべき最新番号
    //   （`core::unseen_releases` の prefix 切り出しに対応する）
    let mark_seen = move || {
        if let Some(id) = core::latest_release_id(unseen) {
            storage::save_release_seen(id);
        }
        dismissed.set(true);
    };

    let n_items: usize = unseen.iter().map(|r| r.items(cur_lang()).len()).sum();

    view! {
        // ★ `help.rs` の `InstallBanner` は `!dismissed.get() && applicable` で
        //   **順序に意味がある**（`applicable` を左に書くと false のとき短絡して
        //   `.get()` に到達せず、依存ゼロのクロージャになる）。ここは単項なので
        //   その制約は無い。環境判定を足して `&&` にするときは必ず左に置くこと。
        {move || {
            (!dismissed.get())
                .then(|| {
                    view! {
                        // ★ 箱は <div>。<button> の入れ子は不正な HTML なので、
                        //   「シートを開く」と「閉じる」を兄弟のボタンに分ける（help.rs と同じ）
                        <div class="whatsnew" data-testid="whatsnew-banner">
                            <button
                                class="whatsnew-body"
                                data-testid="whatsnew-banner-open"
                                on:click=move |_| open.set(true)
                            >
                                <span class="whatsnew-text">
                                    {cur_lang().whatsnew_banner(n_items)}
                                </span>
                                // ★ aria-hidden を付けないこと。付けると支援技術から
                                //   「押すと何が起きるか」を示す唯一の語が消える（help.rs と同じ）
                                <span class="whatsnew-cta">{t().releases.banner_cta}</span>
                            </button>
                            // aria-label は残す。見た目は ✕ でも支援技術と E2E の
                            // role+name には言葉で届く必要がある
                            <button
                                class="icon-btn"
                                aria-label=t().releases.banner_dismiss
                                data-testid="whatsnew-banner-dismiss"
                                on:click=move |_| mark_seen()
                            >
                                {icon(icon::X)}
                            </button>
                        </div>
                    }
                })
        }}
        // ★ **`<Sheet>` は条件の外に常時置く。** `help.rs` の `InstallHelpSheet` は
        //   条件の内側に置いているが、あちらは ✕ とシートを閉じる操作が別物で、
        //   シートを閉じても表示条件の `dismissed` が動かないから成立している
        //   （`applicable` は環境判定なのでシートの開閉と無関係）。
        //   この機能は「閉じる ＝ 既読 ＝ バナーが消える」なので同じ形にすると、
        //   `on_close` の中で `dismissed` が真になった瞬間に条件が偽へ切り替わり、
        //   **開いている（退場中の）`<dialog>` が DOM から消える** —— `Sheet` の doc と
        //   adr/ux/native-dialog-for-sheets.md が名指しした不具合クラスそのもの。
        //   退場アニメーション（styles.css の `.sheet` の `translate` 0.22s +
        //   `allow-discrete`）が中断し、フォーカス復帰（`Sheet` の `on:close`）も走らない。
        //   正解の形は `views::routine::SaveDayAsRoutine`（条件付きにするのは
        //   その手前の `<div class="day-foot">` だけで、`<Sheet>` 自体は外に出す）。
        <Sheet
            open=open
            on_close=Callback::new(move |_| {
                open.set(false);
                mark_seen();
            })
            title=t().releases.sheet_title.to_string()
            testid="whatsnew-sheet"
            close_testid="whatsnew-sheet-close"
        >
            {unseen
                .iter()
                .map(|r| {
                    let items = r.items(cur_lang());
                    view! {
                        // ★ 新しい順のまま並べる（`unseen` 自体が新しい順）
                        <section data-testid="whatsnew-release" data-id=r.id.to_string()>
                            <h3>{r.date}</h3>
                            <ul>
                                {items
                                    .iter()
                                    .map(|item| view! { <li>{*item}</li> })
                                    .collect::<Vec<_>>()}
                            </ul>
                        </section>
                    }
                })
                .collect::<Vec<_>>()}
        </Sheet>
    }
    .into_any()
}
