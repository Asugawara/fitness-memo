#!/bin/sh
# アイコン PNG を 1 枚のマスター画像から書き出す。
#
# マスターは assets/ に置く（public/icons/ に置くと index.html の
# `copy-dir public/icons` で 1024px 版まで配信物と SW のプリキャッシュに載る）。
#
# ★ マスターは「全面塗り・透過なし・角丸なし・主要素が中央 60% 以内」であること。
#   - 透過: iOS は透過を黒に合成するので背景が黒く抜ける
#   - 角丸: iOS が独自の角丸マスクをかけるので二重角丸になる
#   - 中央 60%: maskable のセーフゾーン（中央 80% の円）に収まる。ここが満たされていれば
#     maskable 用に縮小し直す必要がなく、any と同じ絵を使い回せる
set -eu
DIR="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$DIR/assets/icon-master.png"
OUT="$DIR/public/icons"

[ -f "$SRC" ] || { echo "マスター画像がありません: $SRC" >&2; exit 1; }

# 32 はブラウザのタブ（favicon）用。192 からの縮小をブラウザに任せると眠くなるので
# 専用に書き出す。192 はホーム画面 (apple-touch-icon) と manifest、512 は manifest 用。
#
# -depth 8 / -strip でメタデータと 16bit 深度を落とす（16bit のままだと約 3 倍になる）
for size in 32 192 512; do
  magick "$SRC" -resize "${size}x${size}" -depth 8 -strip "$OUT/icon-${size}.png"
done

cp "$OUT/icon-512.png" "$OUT/icon-maskable-512.png"
