#!/bin/sh
# 共有用の画像 2 枚をアイコンのマスター画像から書き出す。
#
#   public/og.png             1200x630  アプリのページを SNS に貼ったときのカード
#   assets/social-preview.png 1280x640  GitHub のリポジトリを貼ったときのカード
#
# ★ この 1 枚を取りに来るのはクローラと SNS のスクレイパだけで、アプリからは一度も
#   参照しない。だから scripts/stamp-sw.sh の SHELL と BUILD_ID の両方から除外して
#   ある（除外を消すと約 1.0MB のオフラインシェルに恒久的に上乗せされる）。
#   e2e/seo.spec.mjs が「SW のシェルに og.png が入っていない」ことを固定している。
#
# ★ 出力はハッシュの付かない固定名（copy-file）なので、絵を差し替えても URL は変わらない。
#   SNS は OG 画像を URL をキーにキャッシュするため、再デプロイしても各社では古い絵が
#   出続ける。差し替えたら Facebook の Sharing Debugger と X の Card Validator で
#   再スクレイプさせること。
#
# ★ 文字は焼き込まない。日本語を入れると -font に環境依存のフォント名を書くことに
#   なり、別マシンで再生成できなくなる（gen-icons.sh の「マスター 1 枚から決定的に
#   書き出す」規約から外れる）。カードの見出し文は og:title が担う。
#
# ★ 背景は styles.css の --hero（ライト）と同じ中性の暗色。manifest の
#   background_color (#7a1020) はアイコン下端のグラデーションと同色なので、
#   それを使うとアイコンの下辺が背景に溶けて四角に見えなくなる。
#
# アイコンを差し替えたら gen-icons.sh と一緒にこれも回す。
set -eu
DIR="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$DIR/assets/icon-master.png"
OUT="$DIR/public/og.png"
OUT_GH="$DIR/assets/social-preview.png"
BG='#101114'

[ -f "$SRC" ] || { echo "マスター画像がありません: $SRC" >&2; exit 1; }

# 440px はキャンバスの上下に 95px ずつ余白が残る大きさ。Slack や X のカードは
# 幅を詰めて表示するので、これ以上小さくするとアイコンの絵が潰れる。
#
# -depth 8 / -strip でメタデータと 16bit 深度を落とす（gen-icons.sh と同じ理由）
magick -size 1200x630 "xc:$BG" \
  \( "$SRC" -resize 440x440 \) -gravity center -composite \
  -depth 8 -strip "$OUT"

# GitHub の Social preview（Settings → General）。推奨 1280x640 / 1MB 未満。
#
# ★ **配信物ではないので public/ ではなく assets/ に出す。** GitHub は
#   repository-images.githubusercontent.com に再ホストするので、この 1 枚を HTTP で
#   取りに来る者は一人もいない。public/ に置くと stamp-sw.sh の除外（今は og.png の
#   1 語だけ）を 2 語に増やすことになり、「precache は全走査で作るので漏れは
#   構造的に起きない」に開けた唯一の穴が広がる。assets/icon-master.png や
#   assets/icons/LICENSE と同じ「人間向けのビルド入力」の棚に置く。
#
# ★ **アップロードは手作業。** GitHub は REST にも GraphQL にもこの API を持たない。
#   このファイルを作り直しただけでは GitHub 上の絵は変わらない（README 参照）。
#
# 448px = 640 の 70%。og.png の 440/630（69.8%）と同じ比率なので 2 枚は同じ絵に見える
magick -size 1280x640 "xc:$BG" \
  \( "$SRC" -resize 448x448 \) -gravity center -composite \
  -depth 8 -strip "$OUT_GH"
