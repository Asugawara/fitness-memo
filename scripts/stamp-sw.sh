#!/bin/sh
set -eu
STAGE="$TRUNK_STAGING_DIR"
# find の列挙順は FS 依存なので必ず sort を挟む（同一内容で BUILD_ID が変わると全クライアントが無駄に再DL）
# ハッシュ入力に sw.js テンプレと本スクリプト自身を含める（ロジック変更でキャッシュ名が変わるように）
BUILD_ID=$(
  { (cd "$STAGE" && find . -type f ! -name sw.js -print0 | LC_ALL=C sort -z | xargs -0 shasum -a 256)
    shasum -a 256 public/sw.js scripts/stamp-sw.sh
  } | shasum -a 256 | cut -c1-16
)
SHELL_LIST=$(cd "$STAGE" && find . -type f ! -name sw.js | LC_ALL=C sort | awk '{printf "\"%s\",", $0}')
sed -e "s|__BUILD_ID__|$BUILD_ID|" -e "s|\"__SHELL__\"|$SHELL_LIST|" "$STAGE/sw.js" > "$STAGE/sw.js.new"
mv "$STAGE/sw.js.new" "$STAGE/sw.js"
