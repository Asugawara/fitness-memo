#!/bin/sh
set -eu
DIR="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$DIR/public/icons/icon.svg"
OUT="$DIR/public/icons"

rsvg-convert -w 192 -h 192 "$SRC" -o "$OUT/icon-192.png"
rsvg-convert -w 512 -h 512 "$SRC" -o "$OUT/icon-512.png"

# maskable はセーフゾーン中央80%に収める必要がある。icon.svg のグリフだけを抜き出し、
# 外側にも同色の全面背景を敷いた上でネストした <svg> で 80% にスケール・中央配置する
GLYPH=$(sed -n '/<g id="glyph"/,/<\/g>/p' "$SRC")
MASKABLE="$OUT/.maskable-512.svg"
{
  echo '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">'
  echo '<rect width="100" height="100" fill="#ffffff"/>'
  echo '<svg x="10" y="10" width="80" height="80" viewBox="0 0 100 100">'
  printf '%s\n' "$GLYPH"
  echo '</svg>'
  echo '</svg>'
} > "$MASKABLE"

rsvg-convert -w 512 -h 512 "$MASKABLE" -o "$OUT/icon-maskable-512.png"
rm -f "$MASKABLE"
