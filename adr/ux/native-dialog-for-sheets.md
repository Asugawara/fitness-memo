# シートをネイティブ `<dialog>` にし、手動の重なり順から降りる

- **状態**: 採用
- **日付**: 2026-08-09
- **カテゴリ**: ux
- **関連**: [キーボード表示中はボトムタブを隠す](../pwa/hide-tabs-when-keyboard-open.md), [ホーム画面への追加の案内を記録タブ末尾のバナー + 手順シートにする](install-guide-banner-and-sheet.md), [ブラウザサポートは Safari を基準にし、polyfill を入れない](../architecture/browser-support-policy.md)

## 背景

下から上がるシートは 4 箇所ある。種目を追加（`day.rs`）、部位 / 種目の編集（`menu.rs`）、ホーム画面に追加（`help.rs`）、データの書き出し / 読み込み（`backup.rs`）。

4 つとも `<div class="sheet-backdrop">` + `<div class="sheet" role="dialog">` の手書きで、**重なり順を人間が管理していた**。これは実際に 2 件の不具合を出している（`public/styles.css` 冒頭に当時のコメントが残っている）。

1. シート下端がタブバーに隠れ、「種目を追加」の最後の種目（レッグレイズ）が押せない。利用者には「体幹の種目は 3 つしかない」ように見えた
2. `inset: 0` の backdrop もタブバーを覆えず、シート表示中にタブが押せた。隠れた種目を狙ったタップが別タブへの遷移になり、入力を見失う（こちらが重症）

修正は `.bottom-tabs: 10` / `.sheet-backdrop: 19` / `.sheet: 20` という**明示的な数値の割り当て**だった。動いてはいたが、`position: fixed` の要素を足すたびに同じ表に並べて考え直す必要があり、忘れれば同じ事故が再発する構造が残っていた。実際 `menu.rs` だけはインライン `style="z-index:20"` を持っており、規則が 2 か所に分裂していた。

手書きゆえに欠けていたものが他にもあった。

- 背景が `inert` にならない。支援技術からは裏のタブバーもカード列も読めるままで、キーボードの Tab はシートの外へ出ていける
- Esc で閉じない
- 閉じたときにフォーカスが元へ戻らない

## 決定

**4 箇所すべてをネイティブ `<dialog>` にし、`show_modal()` で開く。** 枠（`<dialog>` / 見出し / ✕ / 本文の器）は `views::Sheet` の 1 コンポーネントに集約し、4 箇所はそれを使う。

**シートは常時マウントする。** 開いている間だけ DOM に置く形にすると、閉じるときに「top layer に載ったままの要素を DOM から消す」ことになり `close` イベントが飛ばない。

**`z-index` は持たせない。** top layer は z-index の外側にあり、常に通常フローより前面に出る。`styles.css` の重なり順の表からシートの行を消し、残るのは `.add-wrap: 9` < `.bottom-tabs: 10` の 2 つだけになった。

**背景タップで閉じる挙動は自前で書く。** `closedby="any"` は Safari 未対応のため使わない（[ブラウザサポートは Safari を基準にし、polyfill を入れない](../architecture/browser-support-policy.md)）。`<dialog>` 自身の click で、(a) `event.target` が `<dialog>` そのものであること、(b) クリック座標が `getBoundingClientRect()` の外にあること、の両方を見る。

**閉じたときのフォーカス復帰も自前で持つ。** ただし**開く直前のフォーカスが `<body>` だったときは何もしない**。

## 理由

**UA に任せられるものを人間が持たない。** 「シートが最前面にあること」「背景が触れないこと」「Esc で閉じること」は、`showModal()` を呼べば全部 UA の仕事になる。手で保証していた 2 件の不具合は、構造的に起きなくなった。

**座標の判定を省くと iPhone で誤爆する。** `event.target` の同一性だけで判定すると、`<dialog>` 自身の padding 上のタップでも閉じてしまう。このシートは `padding: 0 0 env(safe-area-inset-bottom)` を持っており、**iPhone ではホームインジケータの帯がまさにその padding**。指が下端に触れただけでシートが閉じることになる。座標比較 4 項はそのために要る。

**フォーカス復帰で `<body>` を捨てるのは、Safari の作法に合わせるため。** Playwright（iPhone 15 Pro プロファイル）で実測したところ、WebKit は次の 2 つの性質を持っていた。

| 操作 | WebKit の `document.activeElement` |
|---|---|
| ボタンを**クリック**した直後 | `BODY`（Safari はボタンにフォーカスを与えない） |
| `<dialog>` を Esc で閉じた直後 | `BODY`（開いた要素へ戻さない） |

後者はキーボード利用者にとって明確な後退なので自前で戻す。しかし前者があるため、指でタップして開いた場合は「戻すべき場所」がそもそも存在しない。そこへ強引に `focus()` を当てると、**シートだけがアプリの他のコントロールと違う挙動**になる（タップしただけでフォーカスリングが出る）。だから `<body>` は捨て、キーボードで開いた経路だけを救う。

**常時マウントは「中身の書き方」に条件を課す。** シートの中身は閉じている間も評価されるため、`with_untracked` を使うと開き直しても古い値のままになる。実際 `day.rs` の「追加済み」表示（`.pick.added`）がこれで壊れかけた。開くたびに作り直されることを当てにできないので、中身は素直に追跡する形で書く。この条件は `views::Sheet` のドキュメントコメントに書いた。

## 結果（トレードオフ）

**`data-testid="*-sheet-backdrop"` が消えた。** backdrop は `::backdrop` 疑似要素になり DOM ノードを持たない。E2E は `page.mouse.click()` で座標を直接突く形に書き換えた。

**「閉じている」の判定が `toHaveCount(0)` から `toBeHidden()` に変わった。** 常時マウントなので要素は消えない。閉じた `<dialog>` には UA の `display: none` が効く。

**開閉アニメーションが付いた。** `@starting-style` + `transition-behavior: allow-discrete` で 0.22s。`@media (prefers-reduced-motion: reduce)` では上下動をやめ、黒幕のフェードだけ 0.1s 残す。

**アニメーションは E2E の測定を壊しうる。** `toBeVisible()` は `display: flex` が付いた時点で通るので、その直後に `boundingBox()` を取るとスライド途中の箱が返る（実測で高さ 720 の画面に対し `y=687` / `height=562`、つまり画面外まで伸びた状態）。座標を測るテストは `emulateMedia({ reducedMotion: 'reduce' })` で動きを止めてから測る。

**`.kb-open` との関係は変えていない。** [キーボード表示中はボトムタブを隠す](../pwa/hide-tabs-when-keyboard-open.md) の「キーボード表示中はタブバーと追加ボタンを隠す」はそのまま。シートは隠す対象ではない（入力欄がその中にある）。**ただし top layer の `<dialog>` が iOS standalone のキーボードとどう干渉するかは DevTools で再現しない。** 実機確認が要る項目として残る。

**`web-sys` の feature が 3 つ増えた。** `HtmlDialogElement` / `MouseEvent` / `DomRect`。tachys が `dialog` 要素の定義で `HtmlDialogElement` を参照しているが、[UI 依存を wasm32 の target 別 dependencies に置く](../architecture/wasm-target-scoped-dependencies.md) の「使う API は全て自前で宣言する」に従って明示した。

## 検討した代替案

**`z-index` の割り当てを続け、`inert` 属性だけ自前で足す。** 変更は小さいが、`inert` を付ける対象（`.app` の中のシート以外全部）を DOM 構造に依存して選ぶことになり、`<dialog>` が無償でくれるものを手で組み直すだけになる。Esc とフォーカス管理も別途要る。

**invoker commands（`command="show-modal"`）で宣言的に開く。** JS も `NodeRef` も要らなくなるが、Safari 26.2 が要る（[ブラウザサポートは Safari を基準にし、polyfill を入れない](../architecture/browser-support-policy.md)）。また常時マウントとの相性で結局 Rust 側の signal と同期が要る。

**`popover` 属性を使う。** light dismiss が無償で付くのが魅力だが、`popover` は背景を `inert` にしない。今回いちばん直したかった 2 件目の不具合（裏のタブが押せる）が残るので採れない。

**条件レンダリングのまま `NodeRef` で `show_modal()` を呼ぶ。** 中身が閉じている間 DOM に載らないので `with_untracked` の制約も無い。ただし閉じるときに「top layer にある要素を DOM から消す」ことになり、`close` イベントが飛ばずフォーカス復帰も走らない。開閉アニメーションの退場側も出せない。
