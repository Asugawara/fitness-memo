# タブ切替に方向つき View Transition を掛ける

- **状態**: 破棄（採用の当日に撤去。[破棄](#破棄) を参照）
- **日付**: 2026-08-09
- **カテゴリ**: ux
- **関連**: [ルーターを使わずタブを enum signal で切り替える](../architecture/no-router-tab-enum-signal.md), [UI 依存を wasm32 の target 別 dependencies に置く](../architecture/wasm-target-scoped-dependencies.md), [ブラウザサポートは Safari を基準にし、polyfill を入れない](../architecture/browser-support-policy.md)

## 背景

タブは 記録 / 推移 / 種目 の 3 つで、enum の signal を差し替えるだけで切り替わる（[ルーターを使わずタブを enum signal で切り替える](../architecture/no-router-tab-enum-signal.md)）。ルーターが無いぶん実装は短いが、**画面が瞬時に入れ替わるので前後関係が絵に出ない**。3 枚が横に並んでいるという構造は、タブバーの並びからしか読めなかった。

## 決定

`TabCtx::switch` を `document.startViewTransition()` で包み、`types` に `forward` / `backward` を渡す。向きは `Tab::order()`（記録 0 / 推移 1 / 種目 2）の大小で決める。CSS は `:active-view-transition-type()` で受け、`root` を左右にスライドさせる。

**差し込み口は `TabCtx::switch` の 1 箇所だけ。** ボトムタブも他画面からの遷移も既にここを通っている。

**同じタブへの切替では遷移を走らせない。** `RwSignal::set` は同値でも購読者へ通知するので、素で書くと押すたびに画面が丸ごとクロスフェードする。

**`.bottom-tabs` と `.notice` には固有の `view-transition-name` を与える。** 既定では画面全体が 1 枚の `root` として撮られるため、付けないとタブバーまで一緒に横へ流れる。

## 理由

**`web-sys` の `start_view_transition` は使わない。** あれは `#[cfg(web_sys_unstable_apis)]` の下にあり、使うには `.cargo/config.toml` の `rustflags` に cfg を足すことになる。それは web-sys の unstable 面を丸ごと開ける操作で、[UI 依存を wasm32 の target 別 dependencies に置く](../architecture/wasm-target-scoped-dependencies.md) の「使う API は全て自前で宣言する」と噛み合わない。`src/view_transition.rs` に extern を 1 本立てるほうが依存も影響範囲も小さい。

**`update` は Promise を返さなければならない。** ここが実装上いちばん重要な点で、着手前にレジストリのソースで確かめてある。

`startViewTransition({update})` の `update` は「呼ばれた時点で DOM が同期更新されている」ことを期待する。ところが leptos の signal → DOM 反映は `RenderEffect` が `any_spawner::Executor::spawn_local` に載せるので（`reactive_graph-0.2.14/src/effect/render_effect.rs:222,289`）、wasm では **microtask 送り**になる。同期クロージャを渡すと、まだ古い DOM のままスナップショットを撮り直すことになり、遷移が「何も変わらないアニメーション」になる。

`update` は Promise を返してよいので、`leptos::task::tick()`（`leptos-0.8.20/src/lib.rs:371`「Waits until the next tick of the current async executor」）を 1 回待ってから解決する。

**feature detection は `startViewTransition` の有無で行わない。** 引数にオブジェクトを渡す形（`{update, types}`）は、関数 1 個を渡す旧形より後から入った。**旧形しか無いブラウザにオブジェクトを渡すと `update` が呼ばれず、タブが切り替わらなくなる。** 見た目の劣化ではなく機能の停止なので、ここだけは厳しく判定する。`CSS.supports('selector(:active-view-transition-type(forward))')` が通ることは `types` が通ることと同時期なので、セレクタの対応で代表させ、駄目なら遷移を諦めて `update()` をその場で呼ぶ。

**向きを `Tab` の並び順から引くのは、それが利用者の見ている順序だから。** タブバーの左右の並びと遷移の向きが一致していないと、「戻った」つもりが右から入ってくることになって逆効果になる。

## 結果（トレードオフ）

**タブ切替に 0.2s の演出が乗る。** これは「筋トレ中に使うので最短距離で」という本プロジェクトの原則と真正面から緊張する。採用の判断は次の 2 点による。

- **操作をブロックしない。** View Transition はスナップショット同士のアニメーションで、DOM は既に新しい状態になっている。演出中でも次のタップは新しい画面に当たる
- **`prefers-reduced-motion: reduce` で完全に消える。** 演出が邪魔だと感じる利用者には最初から出ない

それでも「トレ中に毎回 0.2s の横スライドを見る」ことが煩わしいと分かったら、**この ADR ごと戻す**。差し込み口が `TabCtx::switch` の 1 箇所なので撤去は容易で、そのために 1 箇所へ集めてある。

**`wasm-bindgen-futures` が直接依存に増える。** 既に leptos 経由で依存グラフには居たが、[UI 依存を wasm32 の target 別 dependencies に置く](../architecture/wasm-target-scoped-dependencies.md) の方針に従って明示した。wasm32 の target 別 dependencies なので `cargo test` のホストビルドには影響しない。

**web-sys の feature 名は `css`（小文字）。** インタフェース名そのままが原則なので `Css` と書きたくなるが、CSS だけは小文字で登録されており、`Css` はビルドエラーになる。束縛も `web_sys::css::supports` で、型ではなくモジュール。Cargo.toml にコメントを残した。

**タブが 4 つ以上に増えたら `order()` を見直す必要がある。** 現在は enum の定義順と手で一致させている。ずれると向きだけが逆になり、コンパイルは通る。E2E で `['forward', 'backward']` を直接見ているのはそのため。

## 検討した代替案

**`.cargo/config.toml` に `--cfg=web_sys_unstable_apis` を足して web-sys の束縛を使う。** 3 行で済むが、web-sys の unstable API が全部開く。今後うっかり別の unstable API に触れても誰も気づけなくなる。extern 1 本のほうが影響範囲が閉じている。

**同期クロージャを渡す（`startViewTransition(|| tab.set(to))`）。** いちばん素直だが、上記のとおり leptos の DOM 反映が間に合わず「何も変わらないアニメーション」になる。

**CSS のアニメーションだけで表現する（View Transition を使わない）。** 新画面に `@keyframes` で入場アニメーションを付ける形。旧画面が消える様子を描けないので、方向が半分しか伝わらない。また leptos がタブごとにノードを作り直すため、退場側を残す仕組みを自前で持つことになる。

**`cross-document-transitions` を使う。** ルーターが無く 1 ドキュメントで完結しているので前提が成立しない（[ルーターを使わずタブを enum signal で切り替える](../architecture/no-router-tab-enum-signal.md)）。

## 破棄

**入れた当日に実機で使い、撤去した。** 上の「結果（トレードオフ）」に書いた撤去条件（「トレ中に毎回 0.2s の横スライドを見る」ことが煩わしいと分かったら **この ADR ごと戻す**）に、実使用が一発で当たった。

**採用時の 2 つの弁護は、どちらも的を外していた。**

- 「操作をブロックしない」は**手**の話で、問題は**目**だった。セット間の短い休憩でタブを行き来する場面では、横に流れる画面が視線を連れていく。次のタップが通ることと、視点を取られないことは別物
- 「`prefers-reduced-motion` で消える」は、**演出が邪魔だと感じる利用者に OS 設定を変えさせる**話だった。他のアプリのアニメーションまで一括で止める設定であって、このアプリのタブ切替 1 個のための逃げ道ではない

得られたはずの「3 枚が横に並んでいる」という手がかりは、実際にはタブバーの並びで足りていた。トレ中に毎回払う 0.2s に見合わない。

**撤去したもの。** `src/view_transition.rs`（ファイルごと）、`TabCtx::switch` の差し込みと `Tab::order()`、`public/styles.css` のタブ切替ブロック（`view-transition-name` 2 つ、`tab-slide-*` の `@keyframes` 4 本、`:active-view-transition-type()` の 4 規則、View Transition 用の `prefers-reduced-motion`）、`Cargo.toml` の `wasm-bindgen-futures` と web-sys の `css` feature、E2E の向き検証 3 本。**宣言どおり差し込み口が 1 箇所だったので、撤去は機械的に済んだ。**

**`TabCtx::switch` の同値ガードだけは残す。** これは View Transition と無関係に要る。`RwSignal::set` は同値でも購読者へ通知し、`{move || match tab.get() { ... }}` が `<main class="screen">` の中身を作り直すので、同じタブを再タップしただけで確定前の入力が飛ぶ（[記録タブをカレンダー + 選択日エディタの単一画面にする](record-tab-calendar-with-day-editor.md) が `DateCtx::open` について書いているのと同じ罠）。

E2E は「遷移の types が飛ばない」を見ていたので、これも置き換えが要る。**最初に書いた「画面のルート要素が作り直されない」（`screen-menu` に印を付けて再タップ後も残るか）は、ガードを外しても通ってしまった。** leptos は同じ型の view を rebuild するときルートのノードを使い回すので、そこは壊れない。壊れるのは中身で、セット入力欄に打った「62.」が消える。ガードを外して実測すると `set-weight` 自体が見つからなくなる。退行を実際に捕まえる形はこちらなので、**確定前の入力が生き残るか**を見るテストに差し替えた。

**残る教訓。** 「操作をブロックしないから邪魔ではない」は、このアプリでは通らない。演出の当否は**視線を取るかどうか**で判断する。
