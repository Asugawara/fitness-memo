#!/bin/sh
set -eu
STAGE="$TRUNK_STAGING_DIR"
# find の列挙順は FS 依存なので必ず sort を挟む（同一内容で BUILD_ID が変わると全クライアントが無駄に再DL）
# ハッシュ入力に sw.js テンプレと本スクリプト自身を含める（ロジック変更でキャッシュ名が変わるように）
#
# ★ og.png は SHELL と BUILD_ID の**両方**から外す。取りに来るのはクローラと SNS の
#   スクレイパだけで、アプリは一度も参照しない（ADR-0048）。
#   - SHELL から外す: 参照しないものを約 1.0MB のオフラインシェルに恒久的に上乗せしない
#   - BUILD_ID からも外す: SHELL だけ外して BUILD_ID に残すと、og.png を差し替えた
#     ときに「中身が同一のシェル」に対して新しいキャッシュ世代が切られ、全クライアントが
#     無駄に再DL する。シェルに入らないものはシェルの同一性にも影響させない
#   除外が消えても挙動は正常なまま太るだけで誰も気づかないので、
#   e2e/seo.spec.mjs が「SW のシェルに og.png が無い」ことを機械で固定している
BUILD_ID=$(
  { (cd "$STAGE" && find . -type f ! -name sw.js ! -name og.png -print0 | LC_ALL=C sort -z | xargs -0 shasum -a 256)
    shasum -a 256 public/sw.js scripts/stamp-sw.sh
  } | shasum -a 256 | cut -c1-16
)
SHELL_LIST=$(cd "$STAGE" && find . -type f ! -name sw.js ! -name og.png | LC_ALL=C sort | awk '{printf "\"%s\",", $0}')
sed -e "s|__BUILD_ID__|$BUILD_ID|" -e "s|\"__SHELL__\"|$SHELL_LIST|" "$STAGE/sw.js" > "$STAGE/sw.js.new"
mv "$STAGE/sw.js.new" "$STAGE/sw.js"
