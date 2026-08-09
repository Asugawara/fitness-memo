# ADR-0049: ブラウザサポートは Safari を基準にし、polyfill を入れない

- **状態**: 採用
- **日付**: 2026-08-09
- **カテゴリ**: architecture
- **関連**: [ADR-0001](0001-rust-leptos-csr-trunk.md), [ADR-0015](../pwa/0015-sw-atomic-shell-swap.md), [ADR-0050](../ux/0050-native-dialog-for-sheets.md)

## 背景

新しい Web 機能を使うかどうかの判断が、これまで案件ごとの場当たりだった。判断基準が書かれていないと、次に同じ問いが来たときにまた一から調べ直すことになるし、人によって答えが変わる。

このアプリの実使用環境ははっきりしている。**iPhone のホーム画面から起動する standalone PWA** で、開発時に Chrome を見ることはあっても、記録を付けるのは常に iOS Safari である。

一方で、機能の可用性はブラウザごとにばらつく。実際に今回調べた範囲でも次のように割れていた。

| 機能 | Safari | 判断が要る理由 |
|---|---|---|
| `<dialog>` + `showModal()` | 対応 | Baseline widely available |
| `@starting-style` / `transition-behavior` | 17.4 / 17.5 | Baseline newly available（2024-08） |
| View Transitions | 18（2024-09） | Baseline newly available（2025-10） |
| `<dialog closedby>` | **未対応** | Chrome 134 / Firefox 141 のみ |
| `overlay` プロパティ | **未対応** | Chromium のみ |
| invoker commands（`command` / `commandfor`） | 26.2（2025-12） | 出たばかり |

## 決定

**1. Safari を基準にする。** Baseline Newly available はフォールバックなしでそのまま使う。

**2. Safari 未対応の機能は使わない。** どうしても要るなら **20 行以内の自前フォールバック**に留める。それを超えるなら機能ごと諦めて別の設計にする。

**3. polyfill は入れない。**

**4. 未対応環境での劣化は「動くが地味になる」に倒す。** 機能が消えても記録・保存・閲覧は成立させる。

## 理由

**polyfill を置く場所が無い。** このアプリは完全オフラインで動く PWA で、Service Worker がシェル一式をキャッシュしている（[ADR-0015](../pwa/0015-sw-atomic-shell-swap.md)）。CDN から動的 import する形はオフライン起動で落ちるので選べない。self-host すれば動くが、今度は JS を自前で持つことになり、シェルのサイズと更新手順（`scripts/stamp-sw.sh` のキャッシュ名）に恒久的な負債が増える。

**npm はランタイム依存を持っていない。** `package.json` の依存は Playwright だけで、配信物に JS ライブラリは 1 つも載っていない。ここを崩すと「ビルド無しで CSS と Rust だけ読めば全部わかる」という現在の可読性が失われる。

**Safari が最も遅いので、Safari を満たせば他は満たせる。** 逆向き（Chromium 基準）にすると、実使用環境でだけ壊れるという最悪の失敗の仕方をする。

**「20 行以内」の線引きは実例から引いた。** [ADR-0050](../ux/0050-native-dialog-for-sheets.md) の背景タップ判定がちょうどこの規模（`event.target` の同一性 + `getBoundingClientRect()` との座標比較）で、これは読めば意図が分かる。これが 100 行の互換レイヤーになるなら、その機能は時期尚早と判断する。

## 結果（トレードオフ）

**`<dialog closedby="any">` を使えない。** 背景タップで閉じる挙動を自前で書くことになった（[ADR-0050](../ux/0050-native-dialog-for-sheets.md)）。Safari が対応したら消せるコードとして残る。

**invoker commands を使わない。** シートの開閉は Rust 側の signal と `NodeRef` で配線する。宣言的に書けたほうが短いが、Safari 26.2 は出たばかりで、手元の端末が到達している保証が無い。

**`overlay` は「書くが当てにしない」。** `transition-property` に並べてあるが Safari では効かない。未知の値として無視されるだけで害は無く、Chromium での退出アニメーションが正しくなる。**当てにした設計はしない**のが条件。

**この方針は実測で覆せる。** 対象端末の iOS が上がって Safari の対応が進めば、そのつど個別に見直す。方針の意図は「新しい機能を避ける」ことではなく、**判断の基準を 1 か所に書いておく**ことにある。

## 検討した代替案

**Baseline Widely available のみに絞る。** 最も安全だが、`@starting-style`（2024-08 Baseline）も View Transitions（2025-10 Baseline）も落ちる。実使用端末の Safari が対応済みの機能を、対象外のブラウザのために使わないのは本末転倒。

**最新機能を積極採用し polyfill を self-host する。** 上記のとおりシェルサイズと更新手順に恒久的な負債が増える。個人用の記録アプリに対して割に合わない。

**ブラウザごとに分岐を書く。** 分岐は増える一方で消えない。「Safari で成立する 1 本の実装」を探すほうが結果的に短くなる。
