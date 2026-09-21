#!/bin/sh
set -eu
STAGE="$TRUNK_STAGING_DIR"
# find の列挙順は FS 依存なので必ず sort を挟む（同一内容で BUILD_ID が変わると全クライアントが無駄に再DL）
# ハッシュ入力に sw.js テンプレと本スクリプト自身を含める（ロジック変更でキャッシュ名が変わるように）
#
# ★ $STAGE の配信物のうち、Service Worker のオフラインシェルに載せるものを列挙する。
#   **SHELL と BUILD_ID は必ずこの関数を通す。** 片方だけ除外すると「中身が同一の
#   シェル」に新しいキャッシュ世代が切られ、全クライアントが無駄に再DL する。
#
#   除外してよいのは「アプリが一度も参照しないファイル」、または「参照はするが、
#   欠けても本文だけで完結する補助資産」だけ。
#   - **og.png**（前者）: 取りに来るのはクローラと SNS のスクレイパだけで、
#     アプリは一度も参照しない（adr/seo/crawler-metadata-and-hardcoded-origin.md）
#   - **public/manual/**（後者）: マニュアルの章を開いたときだけ <img> で参照される、
#     アプリが参照する初めてのシェル外配信物。次の 5 条件を全部満たすものだけ
#     後者に載せてよいと基準を改めた（基準の改訂自体は
#     adr/seo/crawler-metadata-and-hardcoded-origin.md と adr/pwa/sw-atomic-shell-swap.md
#     への追記で記録する。ここには規則だけを書く）:
#       1. 起動経路・既定画面から参照されない（利用者が明示的に開いたときだけ fetch される）
#       2. <img> / <a> からの参照に限る（script / style / wasm は SRI と起動に絡むので不可）
#       3. SHELL と BUILD_ID の両方から外す
#       4. sw.js に載っていないことを E2E で固定する
#       5. オフラインで開いたときに本文が出て、欠けが利用者に伝わることを E2E で固定する
#
#   どちらも SHELL から外すのは、参照しない／参照が薄いファイルを約 1.0MB の
#   オフラインシェルに恒久的に上乗せしないため。BUILD_ID からも外すのは、
#   差し替えたときに「中身が同一のシェル」に新しいキャッシュ世代が切られ、
#   全クライアントが無駄に再DL するのを防ぐため（シェルに入らないものはシェルの
#   同一性にも影響させない）。
#   除外が消えても挙動は正常なまま太る／欠けるだけで誰も気づかないので、
#   e2e/seo.spec.mjs が「SW のシェルに og.png が無い」ことを、
#   （本 PR で追加する）e2e/manual.spec.mjs が「manual/ が無い」ことを機械で固定する。
#
#   ★ .* も除外する。copy-dir は隠しファイルもコピーするので、たとえば
#     public/icons/.DS_Store が残っているとそのまま SHELL に載るが、docs/ 側は
#     .gitignore で落ちるため Pages で 404 → cache.addAll が拒否 → install 失敗で
#     全端末が旧版に固定される（無言。ローカル E2E では通る）
shell_files() {
  (cd "$STAGE" && find . -type f \
      ! -name sw.js \
      ! -name og.png \
      ! -path './manual/*' \
      ! -name '.*' \
      "$@")
}
BUILD_ID=$(
  # ★ xargs は shell_files() の外（このパイプラインの一部）で走るので、
  #   shell_files() 内の `cd "$STAGE"` はサブシェル止まりでここまで届かない。
  #   `find` が吐く相対パス（"./…"）を shasum が解決できるよう、ここでも
  #   $STAGE へ cd してから読む（cd し忘れると全ファイルが no such file になる）
  { shell_files -print0 | LC_ALL=C sort -z | (cd "$STAGE" && xargs -0 shasum -a 256)
    shasum -a 256 public/sw.js scripts/stamp-sw.sh
  } | shasum -a 256 | cut -c1-16
)
SHELL_LIST=$(shell_files | LC_ALL=C sort | awk '{printf "\"%s\",", $0}')
sed -e "s|__BUILD_ID__|$BUILD_ID|" -e "s|\"__SHELL__\"|$SHELL_LIST|" "$STAGE/sw.js" > "$STAGE/sw.js.new"
mv "$STAGE/sw.js.new" "$STAGE/sw.js"
