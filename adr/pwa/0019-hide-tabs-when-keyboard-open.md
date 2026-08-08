# ADR-0019: キーボード表示中はボトムタブを隠す

- **状態**: 採用
- **日付**: 2026-08-08
- **カテゴリ**: pwa
- **関連**: [ADR-0023](../ux/0023-text-input-not-number.md)

## 背景

ボトムタブは `position: fixed; bottom: 0; padding-bottom: env(safe-area-inset-bottom)` で置いている。iOS で標準的な構成である。

しかし iOS Safari / standalone PWA は**キーボード表示時に layout viewport が縮まず visual viewport だけが変化する**。そのため `position: fixed; bottom: 0` の要素はキーボードの背後に隠れるか、宙に浮いた位置に残る。iOS 17 / iOS 26 でも未解決の著名なバグ群である。

回避策として知られる `<meta name="viewport" content="… interactive-widget=resizes-visual">` は **standalone モードでは無視される**。

このアプリの中核操作は「テンキーでセットを打ち込む」ことなので、**毎セットごとにタブバーが入力域に被る**。さらに悪いことに、**DevTools のレスポンシブモードではキーボードが再現されないので、リリースまで発見されない。**

## 決定

**入力欄がフォーカスされている間はボトムタブを `display: none` で隠す。**

判定は Rust 側の `KbCtx`（`RwSignal<bool>`）で持ち、入力欄の `on:focusin` / `on:focusout` から更新する。

```rust
pub fn kb_focus(kb: KbCtx) {          // focusin
    if let Some(handle) = KB_TIMER.take() { handle.clear(); }
    if !kb.0.get_untracked() { kb.0.set(true); }
}

pub fn kb_blur(kb: KbCtx) {           // focusout
    if let Some(handle) = KB_TIMER.take() { handle.clear(); }
    let open = kb.0;
    match set_timeout_with_handle(move || open.set(false), Duration::from_millis(150)) {
        Ok(handle) => KB_TIMER.set(Some(handle)),
        Err(_) => open.set(false),
    }
}
```

```css
.kb-open .bottom-tabs { display: none; }
```

## 理由

- **最小で確実な対策である。** 「隠す」は 1 行の CSS で、iOS のバージョン差にも依存しない。位置を補正する方向（`visualViewport` の `resize` / `scroll` を購読して `bottom` を動かす）は、iOS 26 系の「キーボードを閉じても `visualViewport` の値が戻らない」バグに自前で対処する羽目になる。
- **キーボードが出ている間にタブを切り替える必要がない。** セットを打ち込んでいる最中にカレンダーへ行きたい、という操作は存在しない。隠して失う機能が実質ない。
- **`focusin` / `focusout` を使うのは、キーボードの表示を直接検知する API が無いため。** iOS には「キーボードが出た」を通知する Web API が無い（`visualViewport` の高さ変化から推測するしかない）。入力欄のフォーカスをプロキシにするのが確実で、しかもフォーカスとキーボードは iOS では実質同義である。
- **`focusout` に 150ms の遅延を入れたのが実装上の要点である。** セット行の重量欄から回数欄へタップで移動すると `focusout` → `focusin` が連続して発火する。遅延なしだと**その間の 1 フレームでタブバーが再表示され、画面が跳ねる**。遅延中に `focusin` が来たらタイマーを clear するので、連続移動では一度も再表示されない。
- **タイマーは leptos の `set_timeout_with_handle` / `TimeoutHandle::clear` を使う**（0.8.20 に実在）。`wasm-bindgen` の `Closure` を自前で扱う必要がなくなり、`web-sys` の feature も増えない（[ADR-0003](../architecture/0003-wasm-target-scoped-dependencies.md)）。
- 判定を CSS クラス（`.kb-open`）で表現し、`App` のルート div に `class:kb-open` で付ける。隠す対象を CSS 側で選べるので、後から「シートも隠す」等の調整が Rust に波及しない。

## 結果（トレードオフ）

- **入力中は画面下部からタブが消えるので、レイアウトが動く。** タブバーの高さぶん本文の余白（`padding-bottom: calc(56px + env(safe-area-inset-bottom))`）は body に残したままなので、コンテンツ自体はずれない。消えるのはタブバーだけである。
- **`focusin` / `focusout` を全ての入力欄に書く必要がある。** 現在は今日タブのセット入力・体重・メモ、種目タブの名前入力に付けている。**新しい入力欄を追加したときに付け忘れると、そこだけタブバーが被る**。コンポーネント側の規律に依存しているのが弱点で、共通の入力コンポーネントに包めば構造的に守れた。
- **`display: none` なので、キーボードを閉じた瞬間にタブバーが戻る。** 150ms の遅延があるため、体感としては「入力を終えると少し遅れて戻る」。跳ねるより良い挙動と判断した。
- **`thread_local!` のグローバルタイマーを 1 つ持つ。** 画面をまたいで共有されるが、キーボードは 1 つしかないので競合しない。
- **DevTools では検証できない。** レスポンシブモードにキーボードが無いので、`.kb-open` が付くこと自体は E2E で確認できるが、「タブバーが入力欄を塞がない」は**実機でしか見えない**。計画の実機検証項目に「キーボードを出した状態でボトムタブが入力欄を塞がない」を最重要項目として明記した。
- **Safari 26.1 beta で fixed-position のキーボードバグが修正されたとされる。** 正式版に載れば `.kb-open` は不要になるが、当面は入れておく。将来この ADR は「置換済み」になりうる。
- 副作用として、シートを開いた状態で入力するとタブバーが消えるため、シートの下端がタブバーに隠れる問題も入力中は起きない。ただし入力していないときは隠れるので、シート側に `z-index` を持たせる必要が別途ある。

## 検討した代替案

**`interactive-widget=resizes-visual` を meta viewport に付ける**: 仕様上はこれが正しい解である。しかし **standalone モードでは無視される**ため、このアプリの主要な利用形態で効かない。Safari のタブでは効くので付けても害はないが、対策としては成立しない。

**`visualViewport` の変化を購読してタブバーの位置を補正する**: タブバーを消さずにキーボードの上へ載せられる。しかし iOS 26 系に「キーボードを閉じても `visualViewport.offsetTop` が元に戻らない」バグがあり、自前でリセット処理を書く羽目になる。バグの回避のためにバグを踏む構造なので却下。

**ボトムタブを `position: sticky` にする**: fixed の問題は回避できるが、スクロールしないと見えない位置に行くことがあり、タブバーとして機能しなくなる。却下。

**キーボード表示中はタブバーを `visibility: hidden` にする**: レイアウトを保ったまま消せる。しかしタブバーは fixed なのでレイアウトに影響せず、`display: none` と結果が同じ。より確実な `display: none` を選んだ。

**入力を確定ボタン方式にしてキーボードの表示時間を短くする**: 被る時間は減るが、なくならない。しかも手数が増えて「最短手数で回す」という要件に反する。却下。
