# GitHub の Social preview 画像を `assets/` に生成し、アップロードは手作業と割り切る

- **状態**: 採用
- **日付**: 2026-08-19
- **カテゴリ**: seo
- **関連**: [クローラ向けメタデータを本番 URL のハードコードで持ち、オフラインシェルから外す](crawler-metadata-and-hardcoded-origin.md), [静的メタデータを英語に統一し、`<html lang>` は実行時に切り替える](static-metadata-in-english.md), [README を英語で正とし、日本語版を `README.ja.md` に置く](../process/readme-in-english-with-japanese-mirror.md)

## 背景

このリポジトリの URL を Slack や X に貼ると、GitHub の自動生成カード
（リポジトリ名・説明・スター数・言語が焼き込まれた画像）が出る。
アプリのアイコンを出したいという要望があった。

GitHub の Social preview は Settings → General からアップロードする。
推奨 1280x640、PNG / JPG / GIF、1MB 未満。**REST にも GraphQL にもこの API は無い。**

## 決定

**`scripts/gen-og.sh` を拡張して 1280x640 を `assets/social-preview.png` に書き出す。
アップロードは Web UI からの手作業と割り切り、両 README に `[!IMPORTANT]` で書く。**

```sh
magick -size 1280x640 "xc:$BG" \
  \( "$SRC" -resize 448x448 \) -gravity center -composite \
  -depth 8 -strip "$OUT_GH"
```

あわせて**リポジトリの description を英語にする**（こちらは `gh repo edit` で API がある）。

## 理由

- **`public/` ではなく `assets/` に置く。** GitHub はアップロードされた画像を
  `repository-images.githubusercontent.com` に再ホストするので、**この 1 枚を HTTP で
  取りに来る者は一人もいない。** `public/` に置くと `scripts/stamp-sw.sh` の除外
  （現在 `og.png` の 1 語だけ）を 2 語に増やすことになり、
  [クローラ向けメタデータを本番 URL のハードコードで持ち、オフラインシェルから外す](crawler-metadata-and-hardcoded-origin.md) が
  言う「precache は全走査で作るので漏れは構造的に起きない」に開けた唯一の穴が広がる。
  `assets/icon-master.png` や `assets/icons/LICENSE` と同じ「人間向けのビルド入力」の棚。
- **新規スクリプトではなく `gen-og.sh` の拡張にする。** README が既に
  「アイコンを差し替えたら `gen-icons.sh` と `gen-og.sh` を両方回す」と書いており、
  3 本目は忘れられる。同じマスター・同じ背景色・同じ「文字を焼かない」規約なので、
  同じスクリプトに置くのが自然。
- **448px = 640 の 70%。** `og.png` の 440/630（69.8%）と同じ比率なので、2 枚が同じ絵に見える。
- **文字は焼き込まない。** `-font` はホスト依存のフォント名になり、
  「マスター 1 枚から決定的に書き出す」規約が別マシンで壊れる。
- **リポジトリの description を放置すると目に見えて不整合になる。** これはリポジトリの
  リンクを貼ったときの `og:description` そのもので、README とサイトを英語にして
  ここだけ日本語だと、カードの中で言語が割れる。トピック 8 件は既に英語なので無変更。

## 結果（トレードオフ）

- **生成物なのにデプロイが人手なので、リポジトリの中身と GitHub 上の絵が黙って食い違いうる。**
  README の手順が唯一の防波堤で、機械では検知できない。アイコンを差し替えた人が
  再アップロードを忘れると、古い絵が出続ける。
- **GitHub の自動生成カードを失う。** 今まではリポジトリ名・説明・スター数・言語が
  画像に焼き込まれていた。単色のアイコンに置き換えるとそれが消える。
  ただし `og:title` / `og:description` は GitHub 側が出し続けるので、
  Slack / X / Discord では画像の**隣**にテキストが出る。情報が全部消えるわけではない。
- **`assets/` に 21KB のバイナリが 1 つ増えた。** 配信物には入らないので
  オフラインシェルにもリポジトリの clone 以外にも影響しない。

## 検討した代替案

**`public/og.png`（1200x630）をそのままアップロードする**: 新規ファイルが 0 枚で済む。
GitHub は 1200x630 も受け付けるし、2:1 でないぶん僅かに切られるが背景が単色なので
視覚差はほぼ無い。**本気で最小差分にしたいならこれで十分。**
却下したのは、推奨サイズちょうどのほうが鮮明で、生成のコストが 4 行だから。

**`public/social-preview.png` に置く**: `assets/` と `public/` の使い分けを考えずに済む。
しかし `stamp-sw.sh` の除外が 2 語に増え、E2E の見張り（`not.toContain('og.png')`）も
2 本必要になる。「アプリが一度も参照しないファイルだけ除外に載せてよい」という制約を、
そもそも使わずに済ませられる。却下。

**新規スクリプト `gen-social.sh` を作る**: 責務が分かれて読みやすい。
しかし回し忘れる 3 本目になる。README の「両方回す」が「3 本とも回す」になるだけ悪化する。却下。

**API でアップロードする**: 存在しない。却下。

**何も設定せず GitHub の自動カードのままにする**: 変更ゼロで、情報量はむしろ多い。
しかしユーザーの要望に反する。却下。
