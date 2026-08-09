# Rust + Leptos (CSR) + trunk を採用する

- **状態**: 採用
- **日付**: 2026-08-08
- **カテゴリ**: architecture

## 背景

iPhone から使う個人用の筋トレ記録 PWA を新規に作る。完全オフラインで動き、通信もアカウントも持たない。開発マシンには Rust が未インストールで、Node v24 は導入済みだった。

利用者から「プログラミング言語は Rust がよいのではないか」という希望が出ていた。断定ではなく相談の形だったため、トレードオフを提示したうえで確認した。

## 決定

**Rust + leptos 0.8.20（CSR モード）+ trunk 0.21.14** を採用する。

## 理由

- 利用者が Rust を書きたいという動機は正当で、この規模のアプリなら技術的に破綻しない。
- 型安全なドメインモデル（`Kind` / `Elapsed` / `Db`）が、この後 [`at` を `Option<i64>` にし当日入力時のみ埋める](../data-model/at-optional-same-day-only.md)・0007 で扱う「静かに壊れる」バグ群を型で防いでくれる。
- WASM のバンドルサイズは gzip で 300〜500KB 程度になるが、**オフライン PWA なので初回ダウンロードのみ**。Service Worker がキャッシュしたあとは影響しない。
- グラフは自前 SVG（[グラフライブラリを使わず SVG を自前で描く](no-chart-library-hand-rolled-svg.md)）、日付は chrono、永続化は localStorage で、いずれも Rust から素直に書ける。

## 結果（トレードオフ）

- ツールチェーン導入に 10〜20 分かかる。`brew install rustup` は現行 formula が `rustup-init` を提供せず keg_only なので使えず、公式インストーラを使う必要があった。
- グラフ・日付 UI・カレンダーグリッドを自前で書く量が TypeScript 版より多い。
- ホストターゲットでのコンパイル可否という前提が生まれたが、[UI 依存を wasm32 の target 別 dependencies に置く](wasm-target-scoped-dependencies.md) の target 分離で解消した。
- iOS Safari でのデバッグは JS より手数が増えるため、E2E（Playwright の WebKit）を厚めに敷いて補う。

## 検討した代替案

**TypeScript + Svelte**: バンドル約 30KB、開発速度は体感 2 倍、iOS Safari のデバッグも容易、日付・グラフのエコシステムが豊富。Node v24 が既にあるので即着手できた。技術的にはこちらの方が素直だったが、利用者の明示的な希望と、この規模なら Rust でも破綻しないという判断で採らなかった。
