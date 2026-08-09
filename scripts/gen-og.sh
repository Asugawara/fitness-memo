#!/bin/sh
# OGP 画像（1200x630）をアイコンのマスター画像から書き出す。
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
BG='#101114'

[ -f "$SRC" ] || { echo "マスター画像がありません: $SRC" >&2; exit 1; }

# 440px はキャンバスの上下に 95px ずつ余白が残る大きさ。Slack や X のカードは
# 幅を詰めて表示するので、これ以上小さくするとアイコンの絵が潰れる。
#
# -depth 8 / -strip でメタデータと 16bit 深度を落とす（gen-icons.sh と同じ理由）
magick -size 1200x630 "xc:$BG" \
  \( "$SRC" -resize 440x440 \) -gravity center -composite \
  -depth 8 -strip "$OUT"
