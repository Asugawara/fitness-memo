# ADR-0002: ルーターを使わずタブを enum signal で切り替える

- **状態**: 採用
- **日付**: 2026-08-08
- **カテゴリ**: architecture
- **関連**: [ADR-0001](0001-rust-leptos-csr-trunk.md), [ADR-0025](../deploy/0025-github-pages-branch-deploy.md)

> **改訂。** タブは 4 つ（今日 / カレンダー / 推移 / 種目）から 3 つ（記録 / 推移 / 種目）に
> なり、`Tab::Today` は無くなった。enum signal で切り替えるという決定自体は変わらない。
> → [ADR-0035](../ux/0035-record-tab-calendar-with-day-editor.md)

## 背景

画面はボトムタブ 4 つ（今日 / カレンダー / 推移 / 種目）。leptos には `leptos_router` があり、SPA なら素直に `/today` `/calendar` … とパスを割り当てる構成になる。

しかし配信先は GitHub Pages の branch deploy（[ADR-0025](../deploy/0025-github-pages-branch-deploy.md)）である。**GitHub Pages には SPA フォールバックの設定がない。** `/fitness-memo/progress` を直接叩くと、そのパスにファイルが無いので 404 が返る（`404.html` を置いてリライトを偽装する定番の回避策はあるが、ステータスコードは 404 のままで、Service Worker のキャッシュキーとも噛み合わない）。

## 決定

**ルーターを使わない。タブは `enum` の signal で切り替える。**

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab { Today, Calendar, Progress, Menu }

let tab = RwSignal::new(Tab::Today);

<main class="screen">
    {move || match tab.get() {
        Tab::Today => view! { <Today /> }.into_any(),
        Tab::Calendar => view! { <Calendar /> }.into_any(),
        Tab::Progress => view! { <Progress /> }.into_any(),
        Tab::Menu => view! { <Menu /> }.into_any(),
    }}
</main>
```

URL は常にアプリのルート 1 つだけ。`leptos_router` を依存に入れない。

## 理由

- **URL が 1 つなら Pages の 404 問題が構造的に起きない。** `start_url` も `scope` も `./` で、Service Worker の precache キーも `./index.html` の 1 つに収束する（[ADR-0016](../pwa/0016-sw-explicit-navigate-branch.md)）。パスが増えるとオフライン起動で「どのパスからでも index.html を返す」フォールバックが必要になり、それは SRI との相性が悪い経路を開く。
- **タブ切り替えに URL を使う必要がない。** このアプリには共有もディープリンクも履歴も要らない。standalone PWA には**アドレスバーも戻るボタンも無い**ので、URL は利用者から一切見えない。
- **画面間の状態が context で共有される。** 日付コンテキスト（`DateCtx`）と `Db`（`DbCtx`）を `provide_context` で配っており、カレンダーの「この日を編集」は `dates.open(date)` してタブを切り替えるだけで成立する。ルーターだとこれをパスパラメータで表現し、パースと検証を挟むことになる。
- **依存が 1 つ減る。** [ADR-0001](0001-rust-leptos-csr-trunk.md) で WASM サイズを気にしている構成なので、使わない機能を入れない。
- **`match` が網羅性を保証する。** タブを増やしたら `match` がコンパイルエラーになる。ルーターの文字列パスは増やし忘れても通る。

## 結果（トレードオフ）

- **ブラウザの戻るボタンでタブが戻らない。** Mac の Chrome で開発中に「戻る」を押すとアプリを離れる。standalone PWA には戻るボタンが無いので実機では問題にならないが、開発時とデスクトップ利用時は不便である。
- **リロードすると必ず今日タブに戻る。** タブの選択状態を永続化していない。今日タブが最頻の入口なので実用上は自然だが、推移タブを見ている途中でリロードすると位置を失う。
- **ディープリンクができない。** 「この種目のグラフ」を URL で指せない。共有機能を持たないアプリなので現状では失うものがないが、将来 URL 共有を入れるならルーターの導入が必要になる。
- **画面ごとの状態がタブ切替で失われる。** `match` で `<Progress />` を破棄しているので、推移タブの期間選択や種目選択はタブを離れると初期値に戻る。維持するなら状態を `App` レベルの signal に持ち上げる必要があり、それはルーターの有無とは独立した話である。現状は初期値が妥当なので放置している。
- E2E はタブボタン（`data-testid="tab-today"` など）を押して遷移する。URL を assert できないので、画面の判定は `data-testid="screen-today"` のような画面ルート要素で行う。

## 検討した代替案

**`leptos_router` + `404.html` によるリライト**: 一般的な SPA on Pages の構成。`404.html` に同じアプリを置けば任意のパスで起動できる。しかし HTTP ステータスが 404 のまま返るので Service Worker のキャッシュ戦略と噛み合わず（404 応答をキャッシュするか否かの判断が増える）、オフライン起動の経路も 1 本増える。得るもの（見えない URL）に対して増えるリスクが大きい。却下。

**`leptos_router` のハッシュルーティング（`#/progress`）**: Pages の 404 問題は起きない。しかし iOS の PWA では `start_url` とハッシュの組み合わせが「同じアプリか別か」の判定に影響しうるうえ、やはり URL は誰にも見えない。依存を増やす理由が残らないので却下。

**タブを整数インデックスで持つ**: `enum` と同等に動くが、`match` の網羅性チェックが効かず、範囲外の値を作れてしまう。`enum` にコストは無いので却下。
