# ADR-0047: `color-scheme` を宣言し、クラスなしの `<button>` を作らない

- **状態**: 採用
- **日付**: 2026-08-09
- **カテゴリ**: ux
- **関連**: [ADR-0038](../storage/0038-share-sheet-over-download.md), [ADR-0014](../storage/0014-defer-export-import.md)

## 背景

実使用のフィードバックは **「データをエクスポートする際にテキストエリアにある情報をコピーするんだけれども、ダークモードで『コピー』という文字が見えなくなっていて、非常に不便」**。

「コピー」は書き出しシートの「うまくいかないとき」の中にある。共有シートもダウンロードも駄目だった端末に残る**最後の逃げ道**（[ADR-0038](../storage/0038-share-sheet-over-download.md)）なので、いちばん困っているときに押せないのが困る。

**色の付け忘れではなかった。** `public/styles.css` の `color:` 宣言 54 個はすべて `var(--…)` か `inherit` で、`:root` の 2 ブロックの外にハードコードした色は 1 つも無い（唯一の literal は `.sheet-backdrop` の `rgba(0, 0, 0, 0.35)` で意図的）。

原因は 2 つが噛み合ったところにあった。

**1. `color-scheme` をどこにも宣言していなかった。** リポジトリ全体で `color-scheme` の出現は `@media (prefers-color-scheme: dark)` だけ。CSS プロパティも `<meta name="color-scheme">` も無い。

**2. その状態で `button { color: inherit }` を書いていた。**

```css
button,
input {
  font: inherit;
  color: inherit;      /* ← UA の ButtonText を上書きする */
  touch-action: manipulation;
}
```

結果、**文字色だけがテーマに追従し、UA が描く背景はライトのまま取り残される。**

| | 文字色 | UA 背景（`ButtonFace`） | コントラスト |
|---|---|---|---|
| ライト | `--text` `#16171a` | `#efefef` | 15.59:1 |
| ダーク | `--text` `#eceef1` | `#efefef`（**追従しない**） | **1.01:1** |

修正後は `.secondary` の `--surface` に載るので、ダークで 15.27:1 になる。

そして **`class` を 1 つも持たない `<button>` はアプリ全体でちょうど 2 つだけ**で、両方このシートの中にあった。

- `backup.rs` の「コピー」← 報告された箇所
- `backup.rs` の「読み込む」（読み込みペインの貼り付け欄の下）← **同じ原因で同じく読めない**

他のボタンは全部 `primary` / `secondary` / `opt` / `link-btn` / `icon-btn` / `pick` / `seg-btn` / `tab-btn` / `menu-cand` / `cal-day` のどれかを持っていて無事だった。**報告された 1 箇所は、規約から外れた 2 箇所のうちの 1 つ**だった。

## 決定

**1. `:root` に `color-scheme: light dark;` を宣言する。**

UA が描くものを全部テーマに追従させる。効く先は自前で色を持てないもの:

- `.sheet-body input[type="file"]` の「ファイルを選択」ボタン（`width` / `margin` / `font-size` しか指定していない）
- `.target-select` のネイティブ `<option>` ピッカー
- スクロールバー

**見た目の退行は無い。** 自前で `background` を持つ入力欄はいずれも影響を受けない: `.text-input`（`--bg`）、`.cond-fields input`（`--surface`）、`.target-select`（`--surface`）、`.field input[type="color"]`（`--bg`）、`.json-box`（`--bg`）。`body` も `background: var(--bg)` を持つので canvas も変わらない。

**2. クラスを持たない `<button>` を作らない。**

「コピー」と「読み込む」に `.secondary` を付けた。`min-height: 44px` / `padding: 0 18px` / `border-radius: 10px` / `border: var(--line)` / `background: var(--surface)`、文字色は `--text` を継承する。

## 理由

**片方だけでは残りが出る。**

`color-scheme` だけだと、2 つのボタンは **44px のタップ標的を持たないまま**（UA 既定は約 20px）アプリのボタン規約からも外れ続ける。同じ `.sheet-actions` に並ぶ「ファイルとして書き出す」が `.primary` の 44px なので、隣に UA 既定の小さいボタンが並ぶ形も残る。

`.secondary` だけだと、`input[type="file"]` と `<select>` のポップアップが取り残される。ここは**自前で色を持たせられない**（`appearance: none` で自前描画に逃げない理由は `.target-select` のコメントに既に書いてある — 矢印を背景画像で描くと色をハードコードすることになり、まさにダークで破綻する）。

**`color-scheme` は既存の方針と噛み合う。** このリポジトリは「ネイティブのコントロールをできるだけそのまま使う」で通してきた（`.target-select` の `appearance: auto`、`input[type="text"]` を使う [ADR-0023](0023-text-input-not-number.md)）。`color-scheme` はその選択の**前提条件**で、宣言しないままネイティブに任せるのは中途半端だった。

## 保証するので、破る入力のテストを書く

ダークのテストはこれまで **1 本も無かった**（`emulateMedia` の呼び出しはゼロ、`playwright.config.mjs` の 4 project も `colorScheme` 未指定）。`e2e/backup.spec.mjs` に 5 本置いた。

- **コントラスト比 ≥ 4.5:1**（`colorScheme: 'dark'` と `'light'` の 2 本）。「コピー」と「読み込む」の文字色と**実効**背景色から相対輝度で計算する。背景が透明なら祖先を辿る — 辿らないと UA 既定へ戻したときに `rgba(0, 0, 0, 0)` から body の色を拾って**通ってしまう**。トークン値をベタ書きしないので両テーマで同じテストが成立する
- **`[data-testid=backup-sheet] button:not([class])` が 0 件**。クラスなしボタンを新たに作った瞬間に落ちる。色ではなく構造で見るので、どのテーマで回しても落ちる
- **`color-scheme` を宣言していること**。`getComputedStyle(document.documentElement).colorScheme` に `light` と `dark` が両方あること。全エンジンで回る
- **UA 描画が実際にテーマへ追従すること**。素の `<button>` を差し込んで UA 既定の背景をダーク / ライトで測り、ダークのほうが暗いことを要求する。`getComputedStyle` では覗けない `input[type="file"]` の代理

**最後の 1 本は Chromium 限定にした。** WebKit のネイティブ form control は自前のテーマが描くので computed style に出ない — 実測でダークの `backgroundColor` が `rgb(255, 255, 255)`、ライトが約 `rgb(200, 200, 200)` と、明暗が**逆に**出る。エンジンの描画実装を測っているのであってアプリの退行ではないので、ここで落とす意味がない。宣言そのものは 1 つ前のテストが全エンジンで見ている。

**実際に壊して確かめた。**

| 戻したもの | 落ちるテスト |
|---|---|
| `color-scheme` の 1 行 | 「color-scheme を宣言している」と「UA が描くコントロールがテーマに追従する」 |
| `class="secondary"` | 「クラスなしの button を作らない」と「44px のタップ標的」 |

`class="secondary"` を外してもコントラストのテストは通る。**`color-scheme` があれば UA が描く背景もダークになるから**で、これは根本の層が効いていることの裏付けでもある。2 つの層はそれぞれ別のテストが見ている。

## 結果（トレードオフ）

**ダークでスクロールバーの見た目が変わる。** desktop Chromium で暗くなる。意図した変化で、`scripts/shots.mjs` はライトで撮るのでスクリーンショットには影響しない。

**「コピー」と「読み込む」が 44px になり、シートが少し縦に伸びる。** トレ中の画面ではなく復旧のための画面なので、押しやすさを取る。

**`--hero` は定義されているが 1 箇所も参照されていない。** 今回の調査で見つけたが色の問題ではないので触っていない。
