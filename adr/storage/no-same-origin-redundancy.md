# 同一オリジン内の多層バックアップを採用しない

- **状態**: 採用
- **日付**: 2026-08-08
- **カテゴリ**: storage
- **関連**: [localStorage の単一キーに JSON 全体を持つ](localstorage-single-key-json.md), [パース失敗時は上書きせず退避する](quarantine-on-parse-failure.md), [書き出しは共有シートを主経路にし、iOS では `<a download>` を使わない](share-sheet-over-download.md)

## 背景

要件は「データ喪失を二重三重に防ぎたい。ただしサーバーは持たず、ブラウザだけで完結・オフラインで完結する」である。

素直に読むと「localStorage に世代スナップショットを持つ」「IndexedDB にもミラーを書く」といった実装になる。**どちらも iOS では保護にならず、むしろ有害**であることが分かったので、採用しない理由を記録する。

## 決定

**同一オリジン内に冗長化の層を作らない。** 具体的に不採用とするもの:

- localStorage への世代スナップショット（日次 / 週次 / 月次のローテーション）
- IndexedDB / Cache Storage / OPFS へのミラー書き込み

代わりに、**端末外へ出す経路**（[書き出しは共有シートを主経路にし、iOS では `<a download>` を使わない](share-sheet-over-download.md)）と、**同一オリジン内では「自分のコードのバグ」だけを守る層**（[パース失敗時は上書きせず退避する](quarantine-on-parse-failure.md) の退避 + 取り込み前の `.pre-` 退避）に絞る。

## 理由

### localStorage と IndexedDB は WebKit では同一の障害ドメインにある

消失経路ごとに見ると、両者は**常に一緒に消える**。

- **ITP の 7 日ルール**の対象リストは "Indexed DB, LocalStorage, Media keys, SessionStorage, Service Worker registrations and cache" — 同じリストに並んでいる
- **Storage Policy のクォータ / eviction** の対象も "localStorage, Cache API, IndexedDB, Service Worker, and File System" — 同じ origin quota を共有し、同じ LRU で evict される
- **WebKit Bug 266559** は「Safari 17.x が全サイトの LocalStorage **と** IndexedDB を消去する」バグだった。原因が `OriginStorageManager::deleteData()` という **origin 単位の削除 API** だったことが決定的で、WebKit の内部では両者が同じ origin storage の管理下にある
- **「履歴とWebサイトデータを消去」**も両方を同時に消す

**片方だけが残る証拠は見つからなかった。** したがって「localStorage が壊れても IndexedDB が残る」という冗長化は成立しない。

### 世代スナップショットは本体の保存を殺しうる

localStorage の上限は実測で**約 5 MiB、オリジン単位で全キー合計**（256KB チャンクを別キーに書き続けて 4.75MB で `QuotaExceededError`）。キー単位ではない。

`Db` が 10 年で 1.24 MB に育つと想定すると、4 世代持てば 6 MB を超えて上限に当たる。そのとき起きるのは:

1. スナップショットの書き込みが失敗する（想定内として握りつぶす）
2. **その後、本体キーの書き込みも失敗する**
3. アプリは正常に見えたまま、新しい記録が一切永続化されなくなる
4. 次回起動時、利用者は「数週間分の記録が消えた」状態を見る

**バックアップを足したことによってデータ喪失が起きる**という最悪の帰結で、しかも起きたことに気づけない。

### では同一オリジン内の層は何を守るのか

守れるのは「**自分のコードのバグ**」と「**利用者の誤操作**」だけ。ITP・容量逼迫・履歴消去・機種変更のいずれに対しても無力である。

この範囲は既に [パース失敗時は上書きせず退避する](quarantine-on-parse-failure.md)（パース失敗時の退避）がカバーしており、今回そこに「取り込み前の `.pre-` 退避」を足した。これ以上の層を積んでも、守れる範囲は増えずに容量リスクだけが増える。

**冗長性は端末外にしか作れない。**

## 結果（トレードオフ）

- 「二重三重の保護」という要望に対して、**同一オリジン内では二重にしない**という答えを返している。UI の文言でも「端末の中だけに置くと機種変更で消えます」と明示し、端末外への保存を促す
- IndexedDB を採らないので、5 MiB の上限は残る。10 年で 1.24 MB の見積りに対して 4 倍の余裕があり、当面は動機がない。仮に上限に近づいたら、そのときは**移行先として** IndexedDB を検討する（ミラーではなく）
- ただし iOS の IndexedDB は 14.6（`open()` が永久にハングする）、15.2.1（PWA で internal error）、17.x（上記 266559）と繰り返し壊れており、移行にも相応のリスクがある
- **`save()` の握りつぶしを撤回した**（[localStorage の単一キーに JSON 全体を持つ](localstorage-single-key-json.md) に追記）。層を増やさない代わりに、書けなくなったことを検知して伝える

## 検討した代替案

**世代を 1 つに絞り、`Db` サイズが上限の 1/3 を超えたら自動削除する**: 容量リスクは緩和できるが、守れる範囲（自分のバグ / 誤操作）は `.bak-` / `.pre-` と変わらない。複雑さに見合わない。

**IndexedDB を「起動時チェックポイント」として使う**: hidden ハンドラに置かなければ [`visibilitychange` の hidden で debounce を flush する](flush-on-visibilitychange.md) の前提は壊れない。ただし増えるカバレッジは「localStorage だけが選択的に壊れた」という起きないケースのみで、対価として async 経路と 7 つの feature、そして iOS 固有の破損バグを買うことになる。却下。

**`navigator.storage.persist()`**: 呼ぶこと自体は無害でコストもほぼゼロだが、効くのは「ストレージ逼迫時の LRU eviction」だけで、履歴消去・機種変更・Safari↔PWA 分離のいずれも防がない。Playwright では常に false を返すので E2E で検証もできない。**今回は入れない**（入れるとしても、その戻り値をバックアップ要否の判断に使ってはいけない）。
