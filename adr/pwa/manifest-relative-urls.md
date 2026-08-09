# manifest の URL を全て相対にする

- **状態**: 採用
- **日付**: 2026-08-08
- **カテゴリ**: pwa
- **関連**: [fetch ハンドラで navigate を明示分岐する](sw-explicit-navigate-branch.md), [GitHub Pages の branch deploy（`release` / `docs`）を使う](../deploy/github-pages-branch-deploy.md)

## 背景

このアプリは 2 つの異なるパス構成で動く必要がある。

| 環境 | ベースパス |
|---|---|
| ローカル / E2E | `http://localhost:4173/` |
| 本番（GitHub Pages） | `https://asugawara.github.io/fitness-memo/` |

`trunk build --public-url /fitness-memo/` でリリースビルドを作るので、HTML から参照する js / wasm / css のパスは trunk が書き換えてくれる。しかし **`manifest.webmanifest` は `copy-file` の不透明コピー**なので、trunk は中身を書き換えない。

つまり manifest 内の `start_url` / `scope` / `icons[].src` を絶対パス（`/fitness-memo/`）で書くと、ローカルで PWA として成立しなくなる。ビルドごとに書き換える仕組みを入れるか、環境ごとに 2 ファイル持つことになる。

## 決定

**manifest の URL を全て `./` 起点の相対にする。**

```json
{
  "id": "./",
  "name": "筋トレメモ",
  "short_name": "筋トレ",
  "start_url": "./",
  "scope": "./",
  "display": "standalone",
  "icons": [
    { "src": "./icons/icon-192.png",          "sizes": "192x192", "type": "image/png", "purpose": "any" },
    { "src": "./icons/icon-512.png",          "sizes": "512x512", "type": "image/png", "purpose": "any" },
    { "src": "./icons/icon-maskable-512.png", "sizes": "512x512", "type": "image/png", "purpose": "maskable" }
  ]
}
```

あわせて 2 つを決める。

- **`id` を必ず入れる**
- **maskable は独立エントリで `purpose: "maskable"` 単独指定にする**

## 理由

- **manifest 内の相対 URL は manifest 自身の URL を基準に解決される。** manifest をアプリルートに置く本構成なら、`localhost/` でも `/fitness-memo/` でも同じ 1 ファイルが正しく解決される。iOS も同じ挙動である。ビルド時の書き換えも環境別ファイルも不要になる。
- **`scope` を明示している。** iOS で相対 URL が問題になるのは `scope` を省略した場合であり、本計画は明示しているので該当しない。省略すると `start_url` のディレクトリが暗黙の scope になり、環境によって解釈が揺れる。
- **`id` を入れないと `start_url` がアプリ識別子になる。** その場合、将来 URL 構成を変えた瞬間に**別アプリとして扱われる**（ホーム画面のアイコンが更新されず、ストレージも引き継がれない）。`id: "./"` を明示しておけば、`start_url` の変更でアプリ同一性を失わない。
- **maskable を独立エントリにするのは見栄えのため。** 1 エントリに `"purpose": "any maskable"` と併記すると、**マスク前提の余白入り画像が通常アイコンとしても使われる**。maskable はセーフゾーン（中央 80%）にグリフを収める必要があるので、そのまま通常アイコンに使うとグリフが小さく見える。`scripts/gen-icons.sh` は `icon.svg` のグリフを 80% にスケールした専用の maskable を生成しており、用途を分けている前提で作られている。
- **この決定は [fetch ハンドラで navigate を明示分岐する](sw-explicit-navigate-branch.md) と同じ性質に乗っている。** `sw.js` 内の `./index.html` も `self.location` 基準で解決され、両方の環境で正しく動く。「相対 URL に統一してベースパスの差を吸収する」という一貫した方針である。

## 結果（トレードオフ）

- **`start_url` が `./` なので、起動 URL はディレクトリ URL（トレイリングスラッシュ）になる。** これが Service Worker の precache キー（`./index.html`）と一致しないため、`fetch` ハンドラでナビゲーションを明示分岐する必要が生じた（[fetch ハンドラで navigate を明示分岐する](sw-explicit-navigate-branch.md)）。相対 URL の代償がここに現れている。
- **manifest がビルド成果物としてハッシュ化されない。** `copy-file` なので `manifest.webmanifest` という固定名で配信される。更新は Service Worker の `BUILD_ID` 入れ替えで届くが、ブラウザが manifest を独自にキャッシュする挙動には依存が残る（アイコンや名前の変更が即時に反映されない可能性）。個人用アプリで manifest を頻繁に変えないので許容する。
- **相対パスの正しさをローカルで検証しても、本番のサブパスで壊れる経路がゼロではない。** `scripts/static-server.mjs` が `E2E_BASE=/fitness-memo/` でサブパス配信を再現できるようにしてあり、重い側 E2E は本番と同じパス構成で manifest 取得を検証する。これが唯一の防波堤である。
- **iOS は manifest の一部を無視する。** `theme_color` や `background_color` の扱いは Safari 独自で、`apple-mobile-web-app-*` の meta タグも併記している（`index.html`）。manifest だけで完結しないのは iOS の制約であり、この決定の帰結ではない。
- アイコンが 3 ファイル（192 / 512 / maskable-512）になり、`gen-icons.sh` に maskable 生成の分岐が必要になった。1 エントリ併記なら 2 ファイルで済んだ。

## 検討した代替案

**絶対パス（`/fitness-memo/…`）で書く**: 本番では最も明快。しかしローカルと E2E で PWA として成立しなくなり、`start_url` が 404 になる。環境ごとに 2 ファイル持つか、ビルド時に置換する仕組み（`stamp-sw.sh` のような後処理）が必要になる。相対で済むなら不要な機構である。却下。

**`stamp-sw.sh` と同様に manifest もビルド時に置換する**: 絶対パスの利点を保ちつつ環境差を吸収できる。しかし置換対象が増えるほど「置換漏れで壊れる」経路が増える。manifest は相対で正しく動くので、置換する理由がない。却下。

**`"purpose": "any maskable"` を 1 エントリに併記する**: アイコンファイルが 2 つで済む。しかしマスク用の余白入り画像が通常アイコンにも使われ、ホーム画面でグリフが小さく見える。却下。

**`id` を省略する**: 現状は動く。しかし将来 URL を変えたときに別アプリ扱いになり、**ストレージが引き継がれない**（バックアップ手段がない v1 では全損に等しい。[JSON エクスポート/インポートを v1 に入れない](../storage/defer-export-import.md)）。1 行のコストで回避できるリスクなので省略しない。
