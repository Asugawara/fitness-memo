#!/usr/bin/env bash
#
# fitness-memo — GitHub Pages の初回ブートストラップ。**1 回だけ**実行する。
# 2 回目以降のリリースは scripts/release.sh を使う。
#
# ─────────────────────────────────────────────────────────────────────────────
# 実行順を入れ替えないこと
#
#   1. main を push
#   2. マージ方式を merge コミットのみに固定
#   3. docs/ 入りの release を push
#   4. **その後で** Pages を有効化
#
# GitHub Docs は branch deploy の前提として「publishing source にするブランチが
# 事前にリポジトリへ存在すること」を要求する。/docs を選んだのに docs/ が無い状態で
# 有効化すると "missing /docs folder" のビルドエラーになる。
#
# ─────────────────────────────────────────────────────────────────────────────
# release ブランチは main から派生させる。**`git switch --orphan` は使わない。**
#
#   1. unborn ブランチはコミットを 1 つ作るまで ref が生まれず push できない
#   2. main と共通祖先を持たないので `git merge` が
#      `fatal: refusing to merge unrelated histories` で必ず失敗する。
#      `--allow-unrelated-histories` を足すと通るが、**初回だけ必要で 2 回目以降は不要**
#      という非対称な挙動になり、release.sh との整合が取れなくなる
#   3. `git switch --orphan` は tracked ファイルを全部消す = `.gitignore` も消える。
#      その状態で `git add -A` すると ignore されなくなった target/（数 GB）や
#      node_modules/ が stage され、push 失敗か公開リポジトリへの巨大ゴミ混入になる
#
# main から派生させれば 1〜3 のすべてが最初から起きない。release は
# 「main の内容 + docs/」になり、これは計画のブランチ設計そのもの。
#
# ─────────────────────────────────────────────────────────────────────────────
# `set -o pipefail` は POSIX sh に無いので shebang は bash。
# 他の scripts/*.sh（#!/bin/sh）とは意図的に異なる。

# ─────────────────────────────────────────────────────────────────────────────
# ビルド成果物を dist/ ではなく dist-release/ に出す理由
#
# 同じチェックアウトで複数人（複数エージェント）が並行作業すると、dist/ はポート 4173 と
# 同じく共有資源になる。static-server はリクエストごとに dist/ を読み直すので、他の作業者が
# `trunk build`（debug・public_url=/）を走らせた瞬間に、リリース検証中の release 成果物が
# 足元から差し替わる。厄介なのは、この事故が「なぜか落ちる」という極めて分かりにくい形で
# 出ることだ。trunk は index.html に SRI の integrity（sha384）を埋めるため、参照先の実体
# だけが debug ビルドに入れ替わると **ハッシュ不一致でブラウザが wasm の取得だけを静かに
# 拒否する**。HTML も CSS も manifest も Service Worker も 200 で返り続けるので、manifest の
# 検証や SW 登録のテストは何事もなく通り、**wasm が要る（= 画面が描画される）テストだけが
# タイムアウトする**。症状が E2E コード側のバグにしか見えず、延々と追う羽目になる。
# そこで出力先自体を分けて根本から断つ（release.sh も同じ DIST_DIR を使い、E2E には
# DIST_DIR=<同じディレクトリ> を渡して同一の成果物を見せる）。
#
# ─────────────────────────────────────────────────────────────────────────────

set -euo pipefail

REPO="Asugawara/fitness-memo"
REMOTE_URL="https://github.com/${REPO}.git"
MAIN_BRANCH="main"
RELEASE_BRANCH="release"
PUBLIC_URL="/fitness-memo/"
SITE_URL="https://asugawara.github.io/fitness-memo/"

# リリース専用の出力先。debug ビルドの dist/ と混ざらないよう必ず分ける
# （.gitignore に /dist-release が必要。preflight で確認する）
DIST_DIR="dist-release"

# Pages のビルド完了を待つ回数と間隔
POLL_TRIES=20
POLL_INTERVAL=15

STAGE="起動"

die() {
  echo "error: $*" >&2
  exit 1
}

step() {
  STAGE="$1"
  echo ""
  echo "==> $1"
}

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "$1 が見つかりません。$2"
}

on_exit() {
  local code=$?
  [ "$code" -eq 0 ] && return 0
  {
    echo ""
    echo "!! 失敗しました (exit ${code}) / 失敗箇所: ${STAGE}"
    echo "   現在のブランチ: $(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo '?')"
    echo ""
    echo "   このスクリプトは冪等ではない。再実行の前に必ず現在の状態を確認すること:"
    echo "     git status --short"
    echo "     git branch -a && git remote -v"
    echo "     gh api repos/${REPO} --jq '{allow_merge_commit,allow_squash_merge,allow_rebase_merge}'"
    echo "     gh api repos/${REPO}/pages --jq '{status,source}' 2>/dev/null || echo 'Pages 未設定'"
    echo ""
    echo "   release ブランチ上で止まっている場合、docs/ は未コミットのまま残っている"
    echo "   可能性がある。main に戻る前に確認すること。"
  } >&2
}
trap on_exit EXIT

# ── 0. 前提の確認 ───────────────────────────────────────────────────────────

step "前提の確認"

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
[ "$(git rev-parse --show-toplevel 2>/dev/null || echo '')" = "$ROOT" ] \
  || die "${ROOT} が git リポジトリのルートではありません"

require_cmd git "https://git-scm.com/"
require_cmd gh "brew install gh"
require_cmd trunk "brew install trunk"
require_cmd cargo 'export PATH="$HOME/.cargo/bin:$PATH" を試してください'

gh auth status >/dev/null 2>&1 || die "gh が未認証です。gh auth login を実行してください"

# ★ core.hooksPath が .githooks を指していないと pre-commit は一度も発火しない。
#   ワークフローファイルを書かない構成では pre-commit が唯一の防波堤なので、設定が
#   抜けたまま積み上がったコミットは「誰も検証していないコード」になる。実際に一度
#   見落として 23 コミットが素通りしたので、**初回公開の前に**必ず確認する
HOOKS_PATH="$(git config --get core.hooksPath || true)"
[ -n "$HOOKS_PATH" ] \
  || die "core.hooksPath が未設定です。pre-commit が発火していません。sh scripts/setup.sh を実行してください"
[ "$(cd "$HOOKS_PATH" 2>/dev/null && pwd || echo '')" = "${ROOT}/.githooks" ] \
  || die "core.hooksPath が '${HOOKS_PATH}' を指しています（想定: .githooks）。sh scripts/setup.sh を実行してください"
[ -x "${ROOT}/.githooks/pre-commit" ] \
  || die ".githooks/pre-commit に実行権限がありません。sh scripts/setup.sh を実行してください"

# dist-release/ が ignore されていないと、生成物が untracked として現れ、
# 直後の「作業ツリーがクリーンか」の確認や再実行が落ちる。
# 追跡対象になっている場合は .gitignore を足しても効かない（ignore は追跡済みファイルに
# 作用しない）ので、原因を分けて案内する
if git ls-files --error-unmatch "$DIST_DIR" >/dev/null 2>&1; then
  die "${DIST_DIR}/ が git の追跡対象になっています。git rm -r --cached ${DIST_DIR} で外してから /${DIST_DIR} を .gitignore に追加してください"
fi
git check-ignore -q "$DIST_DIR" \
  || die "${DIST_DIR}/ が .gitignore に入っていません。/${DIST_DIR} を追加してください"

# main にいること
CURRENT="$(git rev-parse --abbrev-ref HEAD)"
[ "$CURRENT" = "$MAIN_BRANCH" ] \
  || die "${MAIN_BRANCH} で実行してください（現在: ${CURRENT}）"

# main に最低 1 コミットあること
git rev-parse --verify --quiet "${MAIN_BRANCH}^{commit}" >/dev/null \
  || die "${MAIN_BRANCH} にコミットがありません"

# 作業ツリーがクリーンであること
# （.gitignore 済みの dist/ dist-release/ target/ は無視される）
if [ -n "$(git status --porcelain)" ]; then
  git status --short
  die "作業ツリーがクリーンではありません。コミットするか退避してください"
fi

# main に docs/ が混入していないこと。
# 混入すると以後の main → release マージが modify/delete コンフリクトで停止する
if git cat-file -e "${MAIN_BRANCH}:docs" 2>/dev/null; then
  die "${MAIN_BRANCH} に docs/ がコミットされています。docs/ は release 専用です"
fi

# release ブランチがローカルに無いこと（= まだブートストラップしていない）
if git show-ref --verify --quiet "refs/heads/${RELEASE_BRANCH}"; then
  die "ローカルに ${RELEASE_BRANCH} ブランチが既にあります。初回専用のスクリプトです。scripts/release.sh を使ってください"
fi

echo "OK: ${MAIN_BRANCH} = $(git rev-parse --short HEAD) / 作業ツリーはクリーン"

# ── 1. 実行前の最終確認 ─────────────────────────────────────────────────────

step "実行内容の確認"

cat <<EOF
このスクリプトは以下を行います。**公開リポジトリへの push を含みます。**

  リポジトリ : ${REPO} (public)
  公開 URL   : ${SITE_URL}

  1. git remote add origin ${REMOTE_URL}
  2. git push -u origin ${MAIN_BRANCH}            ← ソースが公開されます（不可逆）
  3. マージ方式を merge コミットのみに固定（squash / rebase を無効化）
  4. ${MAIN_BRANCH} から ${RELEASE_BRANCH} を作成
  5. trunk build --release --public-url ${PUBLIC_URL} --dist ${DIST_DIR}
  6. ${DIST_DIR} を docs へコピーし docs/.nojekyll を置いてコミット・push
  7. GitHub Pages を有効化（branch=${RELEASE_BRANCH}, path=/docs）

注意: E2E は実行しません。先に次を通しておくことを強く推奨します。
  trunk build --release --public-url ${PUBLIC_URL} --dist ${DIST_DIR}
  DIST_DIR=${DIST_DIR} E2E_BASE=${PUBLIC_URL} npx playwright test
EOF

if [ "${CONFIRM:-}" != "yes" ]; then
  printf '\n続行しますか？ yes と入力してください: '
  read -r answer || answer=""
  [ "$answer" = "yes" ] || die "中止しました"
fi

# ── 2. remote 設定と main の初回 push ───────────────────────────────────────

step "remote の設定"

if git remote get-url origin >/dev/null 2>&1; then
  EXISTING="$(git remote get-url origin)"
  [ "$EXISTING" = "$REMOTE_URL" ] \
    || die "origin が別 URL を指しています: ${EXISTING}（想定: ${REMOTE_URL}）"
  echo "origin は設定済み: ${EXISTING}"
else
  git remote add origin "$REMOTE_URL"
  echo "origin を追加しました: ${REMOTE_URL}"
fi

# origin に release が既にあるならブートストラップ済み
if git ls-remote --exit-code --heads origin "$RELEASE_BRANCH" >/dev/null 2>&1; then
  die "origin に ${RELEASE_BRANCH} ブランチが既に存在します。ブートストラップ済みです。scripts/release.sh を使ってください"
fi

step "${MAIN_BRANCH} の初回 push"
git push -u origin "$MAIN_BRANCH"

# ── 3. マージ方式を merge コミットのみに固定 ────────────────────────────────
#
# squash / rebase マージをすると main のコミットが release の祖先に入らず
# merge-base が古いまま固定され、以後 docs/ の modify/delete コンフリクトが多発する。
# CLAUDE.md の規約だけでは担保されないのでリポジトリ設定で塞ぐ。

step "マージ方式を merge コミットのみに固定"
gh api -X PATCH "repos/${REPO}" \
  -F allow_squash_merge=false \
  -F allow_rebase_merge=false \
  -F allow_merge_commit=true \
  --jq '{allow_merge_commit,allow_squash_merge,allow_rebase_merge}'

# ── 4. release ブランチの作成（main から派生・orphan は使わない） ───────────

step "${RELEASE_BRANCH} ブランチを ${MAIN_BRANCH} から作成"
git switch -c "$RELEASE_BRANCH" "$MAIN_BRANCH"
echo "作成しました: ${RELEASE_BRANCH} = $(git rev-parse --short HEAD)"

# ── 5. 本番と同じパス構成でビルド ───────────────────────────────────────────

step "trunk build --release --public-url ${PUBLIC_URL} --dist ${DIST_DIR}"
trunk build --release --public-url "$PUBLIC_URL" --dist "$DIST_DIR"

[ -f "${DIST_DIR}/index.html" ] || die "${DIST_DIR}/index.html が生成されていません"
# --public-url が効いていないビルドを公開すると全アセットが 404 になる
grep -q "$PUBLIC_URL" "${DIST_DIR}/index.html" \
  || die "${DIST_DIR}/index.html に ${PUBLIC_URL} が出てきません。--public-url が効いていない可能性があります"

# ── 6. docs/ を作ってコミット・push ─────────────────────────────────────────

step "docs/ の作成"

# ここで消すのは release ブランチ上の docs/ のみ。main には docs/ が無いことを
# 前提の確認で保証済み
rm -rf docs
# ★ コピー元は dist/ ではなく DIST_DIR（冒頭の注記を参照）
cp -R "$DIST_DIR" docs
# Jekyll のビルドを止める（_ で始まるファイルが落とされるのを防ぐ）
touch docs/.nojekyll

# ★ git add -A / git add . は使わない。docs のみを明示的に stage する
git add -- docs

git diff --cached --quiet && die "docs/ に stage された差分がありません"
echo "stage したファイル数: $(git diff --cached --name-only | wc -l | tr -d ' ')"

# ★ --no-verify は必須。.githooks/pre-commit は「docs/ を stage したコミット」を
#   ブランチに関係なく拒否する（main への混入を防ぐガード）ので、付けないと
#   このコミット自体が弾かれる。加えて fmt/clippy/test/trunk build/playwright が
#   再走してリリースのたびに二重実行になる
step "コミットと push"
git commit --no-verify -m "release: bootstrap Pages"
git push -u origin "$RELEASE_BRANCH"

step "${MAIN_BRANCH} に戻る"
git switch "$MAIN_BRANCH"
echo "現在のブランチ: $(git rev-parse --abbrev-ref HEAD)"

# ── 7. Pages の有効化（docs/ 入り release を push した後） ──────────────────

step "GitHub Pages の有効化"

PAGES_JSON='{"build_type":"legacy","source":{"branch":"'"${RELEASE_BRANCH}"'","path":"/docs"}}'

if gh api "repos/${REPO}/pages" >/dev/null 2>&1; then
  echo "Pages は設定済みでした。source を更新します"
  printf '%s' "$PAGES_JSON" | gh api -X PUT "repos/${REPO}/pages" --input -
else
  printf '%s' "$PAGES_JSON" | gh api -X POST "repos/${REPO}/pages" --input -
fi

# ★ リポジトリ設定の Actions は無効化しないこと。
#   branch deploy を選んでも GitHub Pages は内部で必ず
#   "pages build and deployment" ワークフローを実行する。Actions を無効にすると
#   Error: Actor is not allowed to trigger Actions workflows でデプロイが止まる。

# ── 8. 状態の確認 ───────────────────────────────────────────────────────────

step "Pages の状態を確認"
gh api "repos/${REPO}/pages" --jq '{status,build_type,source,html_url}'

status=""
for ((i = 1; i <= POLL_TRIES; i++)); do
  status="$(gh api "repos/${REPO}/pages" --jq '.status // "null"' 2>/dev/null || echo "unavailable")"
  case "$status" in
    built)
      echo "Pages のビルドが完了しました"
      break
      ;;
    errored)
      die "Pages のビルドが失敗しました。GitHub の Actions タブで 'pages build and deployment' のログを確認してください"
      ;;
    *)
      printf '  status=%s (%d/%d) %d 秒待機\n' "$status" "$i" "$POLL_TRIES" "$POLL_INTERVAL"
      sleep "$POLL_INTERVAL"
      ;;
  esac
done

echo ""
if [ "$status" = "built" ]; then
  echo "HTTP ヘッダ:"
  curl -sI "$SITE_URL" | head -3 || true
else
  echo "まだ built になっていません（最後の status=${status}）。"
  echo "配信開始まで数分かかることがあります。時間をおいて次で確認してください:"
  echo "  gh api repos/${REPO}/pages --jq '{status,build_type,source}'"
  echo "  curl -sI ${SITE_URL} | head -3"
fi

cat <<EOF

==> ブートストラップ完了

  公開 URL: ${SITE_URL}

次の一手:
  - iPhone 実機でホーム画面に追加して動作確認（機内モード起動 / キーボード時の
    ボトムタブ / 日跨ぎでヘッダが当日に戻ること / 重量欄に "6." を打てること）
  - 以降のリリースは scripts/release.sh（PR を作ってマージする運用）

やってはいけないこと:
  - リポジトリ設定で Actions を無効化する（branch deploy の内部実行基盤）
  - main に docs/ をコミットする（以後のマージが停止する）
EOF
