# ADR-0013: `visibilitychange` の hidden で debounce を flush する

- **状態**: 採用
- **日付**: 2026-08-08
- **カテゴリ**: storage
- **関連**: [ADR-0011](0011-localstorage-single-key-json.md), [ADR-0017](../pwa/0017-sw-update-on-visible.md)

## 背景

セット入力は 1 文字ごとに `Db` を更新する（[ADR-0023](../ux/0023-text-input-not-number.md) のとおり入力中は文字列 signal で持ち、確定ボタンを置かない）。毎回 `localStorage` に全体を直列化すると無駄なので 400ms の debounce を掛けた。

```rust
pub fn save_debounced(db: Db) {
    PENDING.with_borrow_mut(|pending| *pending = Some(db));
    cancel_pending_timer();
    if let Ok(handle) = set_timeout_with_handle(flush, DEBOUNCE) { /* … */ }
}
```

ここで、ジムでの実際の操作を考える。**最終セットの回数を打ち込み、そのままスワイプでホーム画面に戻る。** これがこのアプリで最も普通の終わり方である。

このとき debounce の 400ms は発火しない可能性が高い。

- バックグラウンドに入ると **JS タイマーは凍結される**（スロットリングではなく停止に近い）
- iOS は **PWA プロセスを頻繁に kill する**。メモリ圧が高い端末では数秒で落ちる
- 凍結されたタイマーは、プロセスが kill された時点で永久に発火しない

つまり対策しないと、**最後に打ち込んだ入力が消える**。しかも「たいてい残っているが時々消える」という最悪の形で現れる。

## 決定

**`visibilitychange` を購読し、hidden へ遷移した時点で `flush()` を呼ぶ。**

```rust
let listener = window_event_listener_untyped("visibilitychange", move |_| {
    if document().hidden() {
        storage::flush();          // pending を即時書き込み
    } else {
        dates.resync(true);        // 当日へ引き直す
    }
});
```

`flush()` は進行中のタイマーを clear し、`PENDING` に溜まっている最新の `Db` を同期的に `setItem` する。

```rust
pub fn flush() {
    cancel_pending_timer();
    if let Some(db) = PENDING.with_borrow_mut(Option::take) {
        save(&db);
    }
}
```

## 理由

- **`visibilitychange` の hidden は、iOS でページが離れるときに最も信頼できるイベントである。** WebKit は `pagehide` / `beforeunload` / `unload` をバックグラウンド遷移や強制終了で発火しないことがあり、これらを主にすると同じバグが残る。`pagehide` は補助にもならないので使っていない。
- **`localStorage` が同期 API なので、hidden ハンドラの中で書き切れる。** これが [ADR-0011](0011-localstorage-single-key-json.md) で IndexedDB を採らなかった理由と直結している。非同期ストレージだと「書き込みを開始したがプロセスが kill された」が起きうるので、この対策自体が成立しない。
- **`visibilitychange` は `Document` で発火するが `bubbles: true` なので window で捕捉できる。** そのため `window_event_listener_untyped` 1 本で済む。leptos 0.8.20 に `leptos::ev::visibilitychange` という typed event は存在しない（docs.rs で 404 を確認）ので untyped を使う。
- **可視判定は `document().hidden()` で行う。** `document().visibility_state()` を使うと `web-sys` の `VisibilityState` feature が必要になるが、`hidden()` は `Document` feature だけで足りる。使う feature を増やさない方針（[ADR-0003](../architecture/0003-wasm-target-scoped-dependencies.md)）に合わせた。
- **visible 復帰側では日付を引き直す。** 同じイベントの裏側で、レジュームされ続ける PWA が「mount 時に決めた今日」を持ち続ける問題を解く（[ADR-0005](../data-model/0005-session-keyed-by-local-date.md)）。1 つのリスナーに 2 つの責務が乗るが、どちらも「アプリが前面/背面を行き来する瞬間にやるべきこと」なので同じ場所にあるのが自然と判断した。

## 結果（トレードオフ）

- **debounce 幅 400ms を安全側に詰める必要がなくなった。** flush が無ければ「短くして取りこぼしを減らす」しかなく、入力ごとの書き込みに近づいていた。
- **`PENDING` と `HANDLE` を `thread_local!` のグローバル状態で持っている。** wasm はシングルスレッドなので実害はないが、モジュールに隠れた可変状態があるということでもある。`storage` の 4 関数の外からは触れないので、スコープは閉じている。
- **アプリがクラッシュした場合（プロセス kill が hidden 遷移を伴わない場合）は最大 400ms 分が失われる。** これは残るリスクとして受け入れる。`visibilitychange` すら来ない強制終了を守る手段は、入力ごとの同期書き込みしかない。
- **タイマーが張れない環境では即時保存にフォールバックする。** `set_timeout_with_handle` が `Err` を返したら `flush()` を直接呼ぶので、debounce が効かないだけで保存は落ちない。
- **E2E で単純なリロードを使うと flaky になる。** リロードは 400ms の debounce と競合するため、`smoke.spec.mjs`（ケース 3）では**明示的に `visibilitychange`(hidden) を発火させてからリロードする**。この順序を守らないとテストが「たまに落ちる」形になり、本番のバグと区別できなくなる。
- 実機検証項目に「最終セット入力直後にスワイプでホームへ戻り、再度開いて入力が残っていること」を入れた。DevTools ではプロセス kill が再現しないので、ここは実機でしか見えない。

## 検討した代替案

**`beforeunload` / `unload` で保存する**: デスクトップブラウザでは定番だが、iOS Safari では発火しないことがあり、しかも WebKit は `unload` を段階的に廃止する方向にある。信頼できないので却下。

**`pagehide` を主にする**: `unload` より iOS でましだが、バックグラウンド遷移では発火しないケースが報告されている。`visibilitychange` を主として、`pagehide` は補助にも入れなかった（2 経路あると「どちらで保存されたか」の検証が増えるだけで、hidden より早いタイミングは存在しない）。

**debounce をやめて入力ごとに同期保存する**: 取りこぼしがゼロになる。しかし 1 文字ごとに `Db` 全体を直列化して `setItem` するので、10 年分のデータで入力のレスポンスが落ちる。debounce + flush で同じ安全性が得られるので却下。

**`setInterval` で定期保存する**: 実装は簡単だが、バックグラウンドではタイマーが凍結されるので**まさに守りたいケースで効かない**。却下。
