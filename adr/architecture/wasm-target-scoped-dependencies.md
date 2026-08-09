# UI 依存を wasm32 の target 別 dependencies に置く

- **状態**: 採用
- **日付**: 2026-08-08
- **カテゴリ**: architecture
- **関連**: [Rust + Leptos (CSR) + trunk を採用する](rust-leptos-csr-trunk.md), [CI を `.githooks/pre-commit` で回す](../deploy/ci-in-pre-commit.md)

## 背景

CI は `.githooks/pre-commit` でローカル実行する（[CI を `.githooks/pre-commit` で回す](../deploy/ci-in-pre-commit.md)）。その中で `cargo test` を**ホストターゲット**（aarch64-apple-darwin）で走らせ、`core.rs` の純ロジックを検証する。

素直に `[dependencies]` に `leptos` を書くと、`cargo test` がホスト向けに leptos の依存グラフ全体をビルドする。ここに 2 つの問題がある。

1. **ビルド時間**: leptos + tachys + reactive_graph + wasm-bindgen 一式をホスト向けにコンパイルする。pre-commit の目標は 60 秒以内である
2. **未検証の前提**: 「`csr` feature がホストターゲットでコンパイルできるか」は誰も保証していない。`csr` は `wasm-bindgen` を前提にした feature であり、ホストで通るかどうかは leptos のバージョンごとに変わりうる。ここが崩れると `cargo test` が丸ごと落ちる

## 決定

**UI 系の依存を `[target.'cfg(target_arch = "wasm32")'.dependencies]` に置き、モジュールも `cfg` gate する。**

```toml
[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
chrono = "0.4"

[target.'cfg(target_arch = "wasm32")'.dependencies]
leptos = { version = "0.8.20", features = ["csr"] }
console_error_panic_hook = "0.1"
web-sys = { version = "0.3", features = [ /* … */ ] }
```

```rust
// src/lib.rs
pub mod core;
pub mod model;
pub mod presets;

#[cfg(target_arch = "wasm32")] pub mod storage;
#[cfg(target_arch = "wasm32")] pub mod views;
```

**`src/main.rs` も `fn main` を丸ごと cfg で分岐する。**

```rust
#[cfg(target_arch = "wasm32")]
fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(fitness_memo::views::App);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {}
```

さらに **`web-sys` の feature は使う API を全て自前で宣言する。**

## 理由

- **`cargo test` がホスト向けに leptos をビルドしなくなる。** 依存が `serde` / `serde_json` / `chrono` の 3 つだけになるので、テストは秒で終わる。pre-commit の 60 秒目標に効く。
- **「`csr` がホストで通るか」という前提そのものが消える。** 検証していない仮定に依存しないのが最も安全な扱いである。
- **`main.rs` の cfg 分岐は必須である。** オプションなしの `cargo test` は lib に加えて **bin もテストハーネスとしてビルドする**。`lib.rs` 側で `views` を gate しても、`main.rs` が `views::App` を無条件参照していればホストビルドが E0433 で落ち、**pre-commit が常に失敗する**。ここを踏むと原因が「テストコードではなく bin のビルド」なので分かりにくい。
- **`web-sys` の feature を自前で宣言するのは feature unification の罠を避けるため。** leptos 0.8.20 の実効 `web-sys` feature は tachys 0.2.16 が握っており、`Window` / `Document` / `Event` / `HtmlInputElement` / `Element` は有効化するが **`Storage` と `MediaQueryList` は有効化しない**。つまり `Window::local_storage()` と `Window::match_media()` はメソッド自体が生成されない。逆に `set_timeout` 系や入力値の読み取りは tachys の feature に偶然乗って通ってしまう。**偶然通っている API は leptos のパッチ更新で無言に壊れる**ので、使うものは全部書く。
- 副作用として、ロジック層（`model` / `core` / `presets`）が UI に依存できない構造が強制される。`core.rs` に `leptos` を import しようとするとホストビルドが落ちるので、「純ロジックは `core.rs`、画面は結果を並べるだけ」という分離がコンパイラに守られる。

## 結果（トレードオフ）

- **rust-analyzer が既定で `views` / `storage` を解析対象外にする。** ホスト cfg で解析するため、UI コードの補完・型チェック・エラー表示が効かない。対処は 1 行で、`.vscode/settings.json` などに `"rust-analyzer.cargo.target": "wasm32-unknown-unknown"` を書く。この設定にすると逆にホスト側の解析を失うが、`core` / `model` / `presets` はターゲット非依存なので失うものがない。
- **`cargo clippy` は必ず `--target wasm32-unknown-unknown` を付けないと UI コードを一切見ない。** 付け忘れると「clippy が通った」のに `views` が未検査という状態になる。pre-commit では明示している。
- **`cargo build`（ターゲット指定なし）は空の `main` をビルドするだけで、アプリは何も入っていない。** 知らないと「ビルドは通るのに動かない」と混乱する。ビルドは `trunk build` で行う。
- **`web-sys` の feature を手で管理するコストが継続的に発生する。** 新しい API を使うたびに `Cargo.toml` を触る必要があり、しかもエラーメッセージは「メソッドが存在しない」なので原因に気づきにくい。この手間は「偶然通っていた API が無言で壊れる」よりましと判断した。実際 `Element`（`scroll_into_view`）と `MediaQueryList`（standalone 判定）は後から追加されている。
- `chrono` は両ターゲットで使うので `[dependencies]` に残る。default features（`wasmbind`）を削ると wasm32 で `Local::now()` が UTC になるので、ここは絶対に触らない（[セッションをローカル日付文字列で BTreeMap に持つ](../data-model/session-keyed-by-local-date.md)）。

## 検討した代替案

**`[dependencies]` に leptos を置き、`cargo test --lib` で bin を避ける**: `main.rs` の cfg 分岐が不要になる。しかし「ホストで `csr` がビルドできる」前提に依存したままで、ホスト向けの leptos ビルド時間も残る。しかもオプション付きのテストコマンドを全員が守る運用になり、素の `cargo test` が落ちる状態は放置できない。却下。

**feature flag（`ui` feature）で切り替える**: ターゲットではなく feature で分ける。`--features ui` の指定漏れで「UI がビルドされていないのに気づかない」経路ができ、`trunk` 側にも feature 指定が必要になる。ターゲットで分けるほうが自動的で漏れがない。却下。

**ワークスペースを分けて `core` クレートと `app` クレートにする**: 分離が最も明確で、`cargo test -p core` が自然になる。しかしファイル構成が 2 クレートに増え、この規模（`src/` 10 ファイル）には過剰。target 別 dependencies で同じ効果が得られる。却下。

**`web-sys` の feature を leptos 任せにする**: `Cargo.toml` が短くなる。しかし `Storage` と `MediaQueryList` が実際に足りず、`storage.rs` と standalone 判定がコンパイルできない。選択肢として成立しない。
