# ADR-0011: localStorage の単一キーに JSON 全体を持つ

- **状態**: 採用
- **日付**: 2026-08-08
- **カテゴリ**: storage
- **関連**: [ADR-0012](0012-quarantine-on-parse-failure.md), [ADR-0013](0013-flush-on-visibilitychange.md), [ADR-0014](0014-defer-export-import.md)

## 背景

要件は「PWA として iPhone のホーム画面から起動し、完全オフラインで動く（iPhone 内のローカルストレージのみ）」。通信もアカウントも持たないので、永続化先はブラウザ内のストレージに限られる。

データ量を見積もると小さい。1 セット ≈ 30 bytes、週 4 回 × 5 種目 × 3.5 セットで年間約 3,700 セット ≈ 110 KB/年。10 年で約 1.1 MB で、Safari の 1 オリジンあたりの上限（約 5 MB）に対して余裕がある。

## 決定

**`localStorage` の単一キー `fitness-memo/v1` に `Db` 全体を JSON で持つ。**

```rust
const KEY: &str = "fitness-memo/v1";
pub fn load() -> (Db, Option<String>);   // (Db, 一度だけ出す通知)
pub fn save(db: &Db);
pub fn save_debounced(db: Db);           // 400ms debounce
pub fn flush();                          // pending を即時実行
```

`Db` は `RwSignal<Db>` として `provide_context()` で配り、`Effect::new` で変更を購読して `save_debounced()` を呼ぶ。読み書きは `src/storage.rs` の 4 関数だけに閉じ込める。

## 理由

- **単一キーなら部分書き込みによる不整合が起きない。** `localStorage.setItem` は 1 キーに対して原子的なので、「`exercises` は書けたが `sessions` は書けなかった」という中間状態が構造的に発生しない。複数キーに分けると `ExerciseLog.exercise_id` が存在しない種目を指す状態を作れてしまい、参照整合性を実行時に検証するコードが必要になる。
- **`localStorage` は同期 API なので、バックグラウンド遷移のハンドラ内で確実に書き切れる。** これは [ADR-0013](0013-flush-on-visibilitychange.md)（`visibilitychange` の hidden で flush）が成立するための前提である。非同期 API では「書き込みを開始したがプロセスが kill された」が起きうる。
- データ量が上限に対して 2 桁小さいので、容量効率のための設計を持ち込む必要がない。
- `serde` の derive でそのまま往復でき、`serde_json::from_str` / `to_string` の 2 行で済む。スキーマ移行も文字列 1 本を受け取る `core::migrate(raw)` に閉じる。
- 書き込み層が薄い 1 モジュールなので、[ADR-0014](0014-defer-export-import.md) で先送りしたエクスポート/インポートを後から 30 行程度で足せる。

`web-sys` の `Storage` feature は **leptos（tachys 0.2.16）が有効化していない**ので、`Cargo.toml` で自前に宣言する。宣言しないと `Window::local_storage()` はメソッド自体が生成されずコンパイルできない。逆に `set_timeout` などは tachys の feature に偶然乗って通ってしまうので、使う API は全て明示宣言する方針にした（[ADR-0003](../architecture/0003-wasm-target-scoped-dependencies.md)）。

## 結果（トレードオフ）

- **1 文字入力するたびに `Db` 全体をクローンし、400ms 後に全体を直列化する。** `Effect` が `db.get()` で購読しているため、キー入力ごとに `Db` のクローンが 1 回発生する。10 年分 1.1 MB でも 1 回の直列化は数 ms なので実測上問題にならないが、データ量が 1 桁増えたら差分書き込みを検討する必要がある。
- **容量超過（`QuotaExceededError`）は握りつぶしている。** `store.set_item` の `Err` を `let _ =` で捨てているので、上限に当たった場合は**無言で保存されない**。見積り上当たらないという判断だが、当たったときに気づけないのは正直に弱点である。
- **単一キーなので部分的な復元ができない。** JSON が壊れたら全体が読めなくなる。この一点が [ADR-0012](0012-quarantine-on-parse-failure.md)（パース失敗時は上書きせず退避する）を必須にしている。
- **Safari のプライベートブラウズでは `local_storage()` が例外を投げる。** `store()` で `Result` と `Option` の両方を畳んで `None` にし、`load()` は「この端末では記録を保存できません（プライベートブラウズ中かもしれません）」を通知として返す。黙って動いて全部消えるより良い。
- **他タブと同期しない。** `storage` イベントを購読していないので、同じブラウザで 2 タブ開くと後に書いた側が勝つ。個人用アプリで、しかも本来の利用形態は standalone PWA の 1 インスタンスなので許容する。
- iOS では Safari のタブと standalone PWA で `localStorage` が**共有されない**。これはこの決定の帰結ではなく iOS の仕様だが、影響が大きいので [ADR-0014](0014-defer-export-import.md) に記録した。

## 検討した代替案

**IndexedDB**: 容量上限がはるかに大きく、部分更新もできる。しかし API が非同期なので、バックグラウンド遷移の瞬間に書き込み完了を保証できず、iOS が PWA プロセスを頻繁に kill する環境（[ADR-0013](0013-flush-on-visibilitychange.md)）と相性が悪い。スキーマ・トランザクション管理のコードも増える。1.1 MB / 10 年という規模に対して過剰。却下。

**localStorage を日付ごとの複数キーに分ける**: 1 回の書き込み量が減り、破損時も 1 日分しか失わない。しかし部分書き込みによる参照整合性の破壊が起きうるうえ、`elapsed_since_last` や部位別グラフが全期間を走査するので毎回キー列挙とパースが必要になり、かえって重くなる。破損時の被害を小さくする目的は [ADR-0012](0012-quarantine-on-parse-failure.md) の退避で別の手段で満たした。却下。

**OPFS / File System Access API**: iOS Safari の対応が限定的で、standalone PWA での挙動も検証コストが高い。却下。

**サーバへの同期**: 「通信しない・アカウントを作らない」という要件の中心に反する。検討対象外。
