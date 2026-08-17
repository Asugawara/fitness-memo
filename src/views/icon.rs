//! アイコン。lucide の SVG を `assets/icons/` から `include_str!` で埋め込む。
//!
//! 出典は lucide（<https://lucide.dev>、ISC / 一部 MIT）。`assets/icons/LICENSE` を参照。
//!
//! 機構は `adr/architecture/help-figures-as-included-svg.md` の
//! ヘルプの図と同じで、`public/` ではなく `assets/` に置く。`public/` に置いたものは
//! 配信物になり `scripts/stamp-sw.sh` が作る SW のプリキャッシュ一覧に載る。
//! `cache.addAll` は **1 つ失敗すると install 全体が落ちる**原子的操作なので、
//! エントリを増やすことは失敗確率を上げることを意味する。
//!
//! SVG ファイル側の規則:
//!
//! - **`viewBox` だけを持つ。** lucide 既定の `width` / `height` / `fill` / `stroke` /
//!   `stroke-width` / `stroke-linecap` / `stroke-linejoin` は**全部剥がして**
//!   `public/styles.css` の `.icon > svg` に移してある。線の太さと色を 2 箇所で
//!   管理しないための分担で、`adr/architecture/help-figures-as-included-svg.md` の
//!   「色は CSS に寄せる」と同じ
//! - **`<?xml … ?>` と DOCTYPE を書かない。** `inner_html` は HTML フラグメント
//!   パーサ経路なので、混ざると bogus comment になり**エラーも出さずにアイコンが
//!   1 つも出ない**（E2E で `.icon > svg` の個数を固定してある）
//! - **`role="img"` / `aria-label` を付けない。** アイコンは必ず `aria-label` を持つ
//!   ボタンの中に置くので、名前はボタン側が持つ。SVG は装飾として `aria-hidden` にする
//!
//! `chevron-down` は持たない。`CHEVRON_RIGHT` を CSS で 90 度回して使う。

use leptos::prelude::*;

pub const CHEVRON_RIGHT: &str = include_str!("../../assets/icons/chevron-right.svg");
pub const CHEVRON_LEFT: &str = include_str!("../../assets/icons/chevron-left.svg");
pub const PENCIL: &str = include_str!("../../assets/icons/pencil.svg");
pub const X: &str = include_str!("../../assets/icons/x.svg");
/// エクスポート（端末の外へ出す）。★ 矢印の向きが上下で対になるので [`DOWNLOAD`] と
/// 並べたとき 20px でも区別が付く。`file-output` / `file-input` の対は横矢印どうしで
/// 紛らわしい
pub const UPLOAD: &str = include_str!("../../assets/icons/upload.svg");
/// インポート（アプリの中へ取り込む）。
pub const DOWNLOAD: &str = include_str!("../../assets/icons/download.svg");
/// ★ **削除にだけ使う。** 「選択中」の ✕（[`X`]）と取り違えないこと — あちらは
/// 「このメニューから外す」で種目そのものは 1 つも消えないので、trash にすると
/// 押した人には**種目を消した**ように読める。
pub const TRASH_2: &str = include_str!("../../assets/icons/trash-2.svg");

/// アイコン 1 個。
///
/// ★ ラッパを関数にしているのは `aria-hidden` の付け忘れを構造的に防ぐため。
/// 1 箇所でも落ちると、そのボタンのアクセシブル名にアイコンが混ざって壊れる。
///
/// ★ `inner_html` は子を持たない要素にしか付かない（`HtmlElement<E, At, ()>` 境界）ので
/// 自己閉じで書く。子を足そうとするとコンパイルエラーになる。
pub fn icon(svg: &'static str) -> impl IntoView {
    view! { <span class="icon" aria-hidden="true" inner_html=svg /> }
}
