# `<input type="file">` を視覚的に隠し、ボタンから `click()` する

- **状態**: 採用
- **日付**: 2026-08-15
- **カテゴリ**: ux
- **関連**: [書き出し / 読み込みを 1 画面に畳む](one-screen-export-import.md), [書き出しは共有シートを主経路にし、iOS では `<a download>` を使わない](../storage/share-sheet-over-download.md), [UA が描くコントロールのために `color-scheme` を宣言する](declare-color-scheme-for-ua-widgets.md)（受益者から 1 件外れる）

## 背景

読み込みは素の `<input type="file">` をそのまま画面に置いていた。UA が描く「ファイルを選択 / 選択されていません」が、周りの `.primary` / `.secondary` ボタンから浮く。文字列は消せず（`::file-selector-button` は Safari 16.4+ でもラベル文字列には効かない）、タップ標的の高さも制御できない。

[書き出し / 読み込みを 1 画面に畳む](one-screen-export-import.md) で貼り付け欄を消した結果、**ファイル選択が取り込みの唯一の経路**になった。浮いたコントロールを唯一の入口にしておけない。

## 決定

**input を視覚的に隠し、`.secondary` の「読み込む」から `click()` で開く。**

```rust
<button class="secondary wide" data-testid="backup-import" on:click=open_picker>"読み込む"</button>
...
<input type="file" class="file-input" tabindex="-1" aria-hidden="true"
       data-testid="backup-file" node_ref=file_ref on:change=on_file />
```

```css
.file-input {
  position: absolute;
  width: 1px; height: 1px;
  padding: 0; border: 0;
  opacity: 0;
  pointer-events: none;
}
```

規則を 5 つ:

1. **`display: none` にしない**
2. **`tabindex="-1"` + `aria-hidden="true"`**
3. **`click()` はクリックハンドラから同期的に呼ぶ**
4. **`<Show>` の外に常時マウントする**
5. **読み終えたら `value` を空にする**

## 理由

### 1. `display: none` にしない

iOS では「レイアウトツリーに無い input への `click()`」が無視された報告が複数ある。そして **`click()` は成否を返さない**。これは [`<a download>` を却下したのと同じ性質の危険](../storage/share-sheet-over-download.md)で、踏んでも「何も起きない」だけが残り、アプリ側から検知してフォールバックすることもできない。

1px + `opacity: 0` で残せば、レイアウトツリーには居るのでこのリスクごと消える。`pointer-events: none` を足すのは、1px の当たり判定が別のボタンを食わないため。

### 2. `tabindex="-1"` + `aria-hidden="true"`

隠した input が Tab 順に残ると「見えないのにフォーカスが止まる」幽霊タブストップになる。操作は `<button>` 側が全部持つので、input は完全に裏方にする。

### 3. `click()` は同期

WebKit の `HTMLInputElement::click()` は `UserGestureIndicator::processingUserGesture()` を見る。leptos の `on:click` はイベントディスパッチ中に同期で走るので、その中から `file_ref.get_untracked().click()` を呼べば活性が生きている。`share()` と違って `await` を挟む余地も無い（ピッカーを開くだけ）。

### 4. `<Show>` の外に常時マウントする

`<Show>` の中に置くと、待機状態 ↔ 確認状態の遷移のたびに要素が作り直され、**`NodeRef` が無効化されて `click()` が空振りする**。空振りは「何も起きない」なので、やはり画面には出ない。

### 5. 読み終えたら `value` を空にする（既存バグの修正）

`on_file` は `input.value` をリセットしていなかった。**同じファイルを 2 回選ぶと 2 回目の `change` が飛ばない**（値が変わっていないため）。

今までは「貼り付け」という別経路があったので露見しにくかったが、ファイル選択が唯一の入口になると **「確認画面でやめる → もう一度同じファイル」で詰む**。しかも何も起きない理由が画面に出ない。

`read_file_text` が `files().get(0)` を同期で掴んだ**後**に `set_value("")` すれば、読み出し自体には影響しない。E2E に退行ガードを 1 本置いた。

### Playwright は隠したままでも `setInputFiles` できる

`playwright-core` の `_setInputFiles` はアクショナビリティ検査が **`attached` のみ**で、`visible` も `stable` も要求しない。したがって `data-testid` は **input 自身**に付けるのが最善（曖昧さがゼロ）。

**逆に、E2E で `toBeHidden()` を主張してはいけない。** `opacity: 0` の 1px 要素は Playwright の定義では「visible」で、それは意図どおり（規則 1）。見たいのは「利用者の目にもタップにも触れないこと」なので、`opacity` / `pointer-events` / `display` / 実測の高さで確かめる。

### `color-scheme` の受益者が 1 件減る

[UA が描くコントロールのために `color-scheme` を宣言する](declare-color-scheme-for-ua-widgets.md) は「効く先」に `input[type=file]` の「ファイルを選択」を挙げていた。隠した結果、受益者は `select` のネイティブピッカーとスクロールバーだけになる。**宣言自体は残りの 2 つに要るので消さない。** E2E の 2 本（`color-scheme を宣言している` / `UA が描くコントロールがテーマに追従する`）もそのまま残る。

## 結果（トレードオフ）

- **`accept` は付けないままにする。** iOS の `accept` は rdar://36726477 で壊れており、付けると Files ピッカーで目当てのファイルが灰色になる。種別の検証は `core::parse_import` がやる（`Result` を返すので追加コストがゼロ）
- **`.sheet-body input[type="file"]` の CSS が要らなくなった**代わりに `.file-input` が増えた（差し引き 1 ルール）
- **実機で未確認**: 隠した input を `click()` したとき、standalone PWA でピッカーが開き、**復帰後にアプリの状態が生きているか**（2019 年報告のバグの現存）。`.tsv` を iCloud Drive / Google Drive 経由で選んだとき `FileReader` が読めるかも同様

## 検討した代替案

**`<label class="secondary" for="...">` で包む。** JS ゼロで、ジェスチャの議論すら発生しない。却下理由が 3 つ: (1) `min-height: 44px` が inline 要素に効かないので `display: inline-flex` を足す CSS が別途要る、(2) キーボード活性化はフォーカス可能な隠し input 側に頼ることになり `aria-hidden` を付けられない、(3) アプリ内の操作要素が全部 `<button class=...>` である一貫性が崩れる（[クラスなしの `<button>` を作らない](declare-color-scheme-for-ua-widgets.md) を構造で見る E2E の前提でもある）。

**`::file-selector-button` で整形して素の input を出したまま。** Safari 16.4+ でしか効かず、しかもラベル文字列（「ファイルを選択」「選択されていません」）は消せない。整形しきれない。

**`showOpenFilePicker`（File System Access API）。** WebKit に存在しない。選択肢ですらない。
