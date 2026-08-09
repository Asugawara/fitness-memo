# 書き出しは共有シートを主経路にし、iOS では `<a download>` を使わない

- **状態**: 採用
- **日付**: 2026-08-08
- **カテゴリ**: storage
- **関連**: [JSON エクスポート/インポートを v1 に入れない](defer-export-import.md)（代替案の評価を上書き）, [ID を 60 bit 乱数にし、プリセットには固定 ID を与える](../data-model/random-ids-for-safe-merge.md), [同一オリジン内の多層バックアップを採用しない](no-same-origin-redundancy.md)

## 背景

[JSON エクスポート/インポートを v1 に入れない](defer-export-import.md) がエクスポート / インポートを「次リリースの必須項目」として先送りしていた。それを実装するにあたり、**iPhone の standalone PWA からファイルを取り出す手段**を決める必要がある。

[JSON エクスポート/インポートを v1 に入れない](defer-export-import.md) は代替案として「Web Share API / ファイルダウンロードで自動バックアップ」を挙げ、「iOS の standalone PWA からのダウンロード挙動が安定せず、検証コストが高い」として却下していた。この判断を再検証した。

## 決定

**経路を iOS かどうかで分け、iOS では `<a download>` を構造的に選ばない。**

```rust
pub fn pick_route() -> Route {
    if is_ios() {
        if can_share_file() { Route::Share } else { Route::Clipboard }
    } else {
        Route::Download
    }
}
```

加えて:

- **`ShareData` には `files` だけを入れる**（`title` / `text` / `url` を混ぜない）
- **`share()` はクリックハンドラから同期的に呼ぶ**
- **`canShare` は 1 バイトのプローブ File でクリック直後に同期判定する**
- **`AbortError`（キャンセル）を成功扱いにしない**
- **`<input type="file">` に `accept` を付けない**
- **textarea への全文表示は残す**が、折りたたみの中に隠す

## 理由

### `<a download>` は 2026 年時点でも standalone で壊れている

Safari 26.0〜26.6 および 27 beta のリリースノートに download 関連の修正は一件も無い。2026 年だけで 6 件以上のマージ済み PR が `<a download>` から共有シートへ移行しており（training-tracker#22、FirearmLog#27、medical-records-keeper#6、coder/coder#27853 ほか）、逆方向（standalone で動いた）の報告は見つからなかった。

症状は「何も起きない」ではなく、それより悪い。**standalone では `download` 属性が無視され、WebView が href 自体へ遷移する。** 戻る UI が無いのでアプリを強制終了するまで復帰できない。しかも `click()` は成否を返さないので、アプリ側から検知してフォールバックすることもできない。

**この「検知できない」性質のため、`<a download>` を最後のフォールバックに置くことにも意味がない。**

### [JSON エクスポート/インポートを v1 に入れない](defer-export-import.md) の却下は理由としては正しく、まとめ方が誤っていた

維持すべき部分:
- 「standalone のダウンロード挙動が安定しない」は今も正しい
- 「検証コストが高い」も正しく、しかも想定より深刻。**Playwright の WebKit は download / share / clipboard のどれもジェスチャ要件を再現せず、実機で失敗する経路を素通りさせる**（`navigator.storage.persist` は逆に常に false を返す）。E2E を書いても「グリーンなのに実機で壊れている」を作り出す
- 「自動バックアップ」の却下も正しい。成否を検知できない手段で自動保存すると「守られている」という誤認だけが残る

覆すべき部分:
- **Web Share とファイルダウンロードを 1 つの選択肢として束ねたのが誤りだった。** `<a download>` は成否を**検知できない**が、`navigator.share` は **Promise で成否が返る**。束ねて却下したことで、成否が分かる唯一のファイル出力手段まで一緒に捨てていた

### `files` だけを渡す理由

WebKit の `WKShareSheet.mm` は、`text` / `url` / `title` を共有アイテム配列に**別要素として**積む。`UIActivityViewController` は全アイテムを受理できるアクティビティしか出さないので、文字列が混ざると「ファイルに保存」が候補から消える。

なお `canShare` に MIME の allowlist は無く（`Navigator.cpp`）、iOS 側の UTType は `File.type` ではなく**ファイル名の拡張子**から決まる（`WKShareSheet.mm`）。`.json` は登録済みの型なので通るはずだが、**この点は実機で未確認**（下記）。

### ジェスチャの窓

WebKit の transient activation は **5 秒**（Chromium の約 5 秒と同じだが、WebKit のジェスチャ転送上限は 1 秒という別の制約もある）。`share()` は `consumeTransientActivation()` を同期的に消費する。

このアプリは書き出しが同期処理（localStorage 読み → String → File 生成）なので、**クリックハンドラの中で一続きに書ける**。2 段階フロー（生成 → 別ボタン）は要らない。

### `accept` を付けない理由

iOS の `accept` は rdar://36726477 で壊れており、複数指定すると最初の型しか効かず残りが Files ピッカーで灰色になる。iCloud Drive 経由だとさらに悪化する。**付けないのが唯一の安全策**で、種別の検証は `core::parse_import` が行う（`Result` を返すので追加コストはゼロ）。

### textarea を残す理由

Web API を一切使わないので、iOS のどのバージョン・どの表示モードでも必ず動く。共有シートもクリップボードも駄目だった端末に残る最後の逃げ道になる。ただし UX が悪いので、既定では「うまくいかないとき」の折りたたみに隠す。

## 結果（トレードオフ）

- **`src/transfer.rs` を新設した。** JS interop をここに閉じ込め、`storage.rs` を「`localStorage` を読み書きするだけ」に保つ（あちらの薄さは [`visibilitychange` の hidden で debounce を flush する](flush-on-visibilitychange.md) の検証可能性を支えている）
- **web-sys features が 12 増え、`js-sys` / `wasm-bindgen` が直接依存に加わった。** [UI 依存を wasm32 の target 別 dependencies に置く](../architecture/wasm-target-scoped-dependencies.md) の「使う API は全て自前で宣言する」方針に従い全て明示した
- **`then2` に渡す `Closure` を `forget()` している。** 書き出しは年に数回の操作なので 2 つ分のリークは受け入れる
- **実機でしか確認できないことが残る**（下記）。E2E のコメントに「このファイルが緑でも iOS で動く保証はない」と明記した

## 実機で確認すべきこと

1. `.json` を共有シートに渡したとき「ファイルに保存」が出て、iCloud Drive に置けるか
2. `files` 単独と `files + title` で共有シートの内容が実際に変わるか（WebKit ソースからの推論であり、この環境では未検証）
3. `<input type="file">` のダイアログがアプリ復帰後に開くか（2019 年報告のバグの現存確認）
4. キーボード表示時に textarea が隠れないか

## 検討した代替案

**textarea 全文表示を基盤にする**: 確実性は 100% だが UX が最低で、しかも 2026 年の実プロジェクトは軒並み共有シートへ移行済み。折りたたみの中に後退させるのが妥当だった。

**`<a download>` を iOS でも最後のフォールバックとして残す**: 成否を検知できないので、フォールバック連鎖の一段として機能しない。踏むとアプリが固まるだけ。却下。

**`showSaveFilePicker`（File System Access API）**: WebKit に存在しない（caniuse で Safari 全バージョン未対応）。選択肢ですらない。

**Web Share Target（他アプリから共有で受け取る）**: WebKit Bugzilla #194593 が 2019 年から NEW のまま（最終コメント 2026-05-23）。インポート経路としては使えない。
