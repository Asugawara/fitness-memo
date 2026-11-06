# ± の箱の位置に「キーボードの上」を 4 つ目として足す（既定は欄の上のまま）

- **状態**: 採用
- **日付**: 2026-11-06
- **カテゴリ**: ux
- **関連**: [タップで数値を空にし、触っている欄の真上に ± ボタンを浮かせる](tap-clears-the-number-and-steps-above-the-field.md)（この ADR が「キーボードの真上のバー」を却下した。この ADR はその却下を覆さず、選択肢として採る）, [± の箱は設定で持つ手の側の端へ 2 段で寄せられる](stepper-on-the-thumb-side.md)（同じ 2 段の並びを使う）, [キーボード表示中はボトムタブを隠す](../pwa/hide-tabs-when-keyboard-open.md)（iOS の visual viewport の不具合を踏んだ前提）

## 背景

± の箱は欄の真上に浮く（前の ADR）。前の行と前回の記録（`last-row`）を覆うので、見たい行が隠れる。端へ寄せる 2 段（前の ADR）は覆いを減らしたが、持つ手から遠い −5 に届く位置を狙ったもので、キーボードの真上ではない。ユーザーの要望は「スマホでキーボードのすぐ上に出せないか」。

## 決定

1. **位置は 4 択。** 設定 › ± ボタン › 位置 に「キーボードの上」を末尾に足す。並びは「左端 / 欄の上 / 右端 / キーボードの上」。**既定は「欄の上」のまま**（未設定の画面と保存データは変えない）。
2. **箱は画面幅いっぱい・下端に付く。** 高さはキーボードの真上。並びは端寄せの 2 段と同じで、上の段に ＋、下の段に −（`+1.25 −1.25 +2.5 −2.5 +5 −5` の列流し）。高さは約 100px で、キーボード表示中に見える範囲がその分狭くなる。
3. **下端の位置は `--kb-inset` で決める。** `--kb-inset` ＝ `innerHeight − visualViewport.height − visualViewport.offsetTop`（0 未満にしない）。`visualViewport` の `resize` / `scroll` で書き直す。`visualViewport` が無ければ箱は画面の下端に付く。
4. **欄を覆うときは差だけ下へ送る。** 欄の下端が箱の上端より下なら、その差 + 8px だけスクロールする。判定は両方の `getBoundingClientRect()` で行うので、iOS の座標系の違いに依らない。`resize` でも判定し直す（iOS はキーボードを開き切ってから `resize` を送るため）。
5. `Above` / `Left` / `Right` の挙動は変えない。

## 理由

前の ADR（[タップで数値を空にし…](tap-clears-the-number-and-steps-above-the-field.md)）が却下した理由は、`visualViewport` に追従させる必要があり、[キーボード表示中はボトムタブを隠す](../pwa/hide-tabs-when-keyboard-open.md) が避けた iOS の不具合（iOS 26 で `visualViewport.offsetTop` がキーボードを閉じても 0 に戻らない）を踏む、というものだった[^1]。

この不具合は「閉じた後も画面に残る fixed 要素」で起きる。タブバーは閉じた後も出ているので踏む。一方この箱は**フォーカス中（＝キーボードが出ている間）しか存在しない**。開くたびに `--kb-inset` を読み直すので、閉じた後のずれが残る場面が無い。

既定を変えず選択肢として足すのは、キーボードの開閉中の動きが iPhone 実機でしか確かめられないため。実機で良ければ既定にするか、端寄せを残すかは別の PR で決める。

## 結果（トレードオフ）

- キーボードの開閉中の動きは **iPhone 実機でしか確かめられない**（Playwright にキーボードが無い）。E2E は「キーボードが食った高さ」を `--kb-inset` に反映できること、欄を覆わないことまでを見る。
- 外付けキーボードなどで画面キーボードが出ないときは、箱は画面の下端に付く。
- ピンチズーム中は箱が visual viewport の下端に付く（E2E 18 で確かめる、Chromium のみ）。
- 箱が前の行と前回の記録を覆うことは無くなるが、キーボードの上 約 100px は箱に取られ、その分スクロールできる範囲が狭くなる。

## 検討した代替案

- **既定にする。** 実機で確かめる前に、既定の画面の動きが変わる。却下。実機で良ければ別の PR で既定にする。
- **端寄せを廃止する。** 「持つ手の側の端」を選んだ人の操作を奪う。却下。
- **お知らせ（`RELEASES`）に書く。** 同じ結果に別の操作で至るだけ（[お知らせの項目は図付きの利用者向け機能だけ](whats-new-notes-are-user-facing-features-with-figures.md)）。

## 参考

[^1]: Apple Developer Forums [thread 800125](https://developer.apple.com/forums/thread/800125)（iOS 26 で `visualViewport.offsetTop` がキーボードを閉じても 0 に戻らない。WebKit bug 297779）。対処の定石は `bottom = innerHeight − vv.height − vv.offsetTop`（[mathix.dev](https://mathix.dev/blog/fix-html-elements-on-top-of-the-ios-keyboard-using-html-css-js)）。
