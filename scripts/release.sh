#!/usr/bin/env bash
#
# fitness-memo — 2 回目以降のリリース。origin/release への PR を作るところまで行う。
# 初回は scripts/bootstrap-release.sh（1 回だけ）。
#
# ─────────────────────────────────────────────────────────────────────────────
# 「docs/ が消えない」ことの根拠
#
# 2 回目以降の merge-base は「前回マージで取り込んだ main のコミット」で、そこに
# docs/ は無い。base に無く ours（release 側）にのみ存在するパスは 3-way マージで
# 「ours 側の追加」として保持される。さらに本スクリプトはマージ後に docs/ を作り
# 直すので毎回上書きされる。
#
# この保証を壊す経路は 2 つだけで、両方を塞いである:
#   - squash / rebase マージ → bootstrap-release.sh がリポジトリ設定で無効化済み
#   - main への docs/ 混入   → .githooks/pre-commit と本スクリプトの両方でガード
#
# ─────────────────────────────────────────────────────────────────────────────
# `set -o pipefail` は POSIX sh に無いので shebang は bash。

set -euo pipefail

REPO="Asugawara/fitness-memo"
MAIN_BRANCH="main"
RELEASE_BRANCH="release"
PUBLIC_URL="/fitness-memo/"
SITE_URL="https://asugawara.github.io/fitness-memo/"

STAGE="起動"
WORK_BRANCH=""

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
    if [ -n "$WORK_BRANCH" ]; then
      echo ""
      echo "   作業ブランチ ${WORK_BRANCH} に残っています。"
      echo "   マージ競合なら解決してから続きを手動で行うか、次で破棄してください:"
      echo "     git merge --abort                  # マージ中の場合"
      echo "     git switch ${MAIN_BRANCH}"
      echo "     git branch -D ${WORK_BRANCH}"
      echo "   （${MAIN_BRANCH} や origin/${RELEASE_BRANCH} は変更していません）"
    fi
  } >&2
}
trap on_exit EXIT

# ── 1. 前提の確認 ───────────────────────────────────────────────────────────

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

CURRENT="$(git rev-parse --abbrev-ref HEAD)"
[ "$CURRENT" = "$MAIN_BRANCH" ] \
  || die "${MAIN_BRANCH} で実行してください（現在: ${CURRENT}）"

if [ -n "$(git status --porcelain)" ]; then
  git status --short
  die "作業ツリーがクリーンではありません。コミットするか退避してください"
fi

# ★ main に docs/ が混入していないこと。
#   混入すると以後の main → release マージが modify/delete コンフリクトで停止する
if git cat-file -e "${MAIN_BRANCH}:docs" 2>/dev/null; then
  die "${MAIN_BRANCH} に docs/ がコミットされています。docs/ は release 専用です。main から取り除いてください"
fi

step "origin の取得と同期確認"
git fetch origin

git rev-parse --verify --quiet "refs/remotes/origin/${RELEASE_BRANCH}" >/dev/null \
  || die "origin/${RELEASE_BRANCH} がありません。先に scripts/bootstrap-release.sh を 1 回だけ実行してください"

LOCAL_MAIN="$(git rev-parse "$MAIN_BRANCH")"
REMOTE_MAIN="$(git rev-parse "origin/${MAIN_BRANCH}")"
if [ "$LOCAL_MAIN" != "$REMOTE_MAIN" ]; then
  echo "  local  ${MAIN_BRANCH}: ${LOCAL_MAIN}"
  echo "  origin/${MAIN_BRANCH}: ${REMOTE_MAIN}"
  die "${MAIN_BRANCH} が origin/${MAIN_BRANCH} と同期していません。push / pull してから再実行してください"
fi

echo "OK: ${MAIN_BRANCH} = $(git rev-parse --short HEAD) / origin と同期 / 作業ツリーはクリーン"

# ── 2. 本番と同じパス構成でビルド ───────────────────────────────────────────

step "trunk build --release --public-url ${PUBLIC_URL}"
trunk build --release --public-url "$PUBLIC_URL"

[ -f dist/index.html ] || die "dist/index.html が生成されていません"
# --public-url が効いていないビルドを公開すると全アセットが 404 になる
grep -q "$PUBLIC_URL" dist/index.html \
  || die "dist/index.html に ${PUBLIC_URL} が出てきません。--public-url が効いていない可能性があります"

# ── 3. 重い E2E（WebKit / iPhone エミュを含む全 project） ───────────────────

step "重い E2E（E2E_BASE=${PUBLIC_URL}）"
require_cmd npx "Node.js を入れてください"
[ -d node_modules ] || die "node_modules がありません。scripts/setup.sh を実行してください"

# ここで落ちたら公開しない。dist は本番と同じ public_url でビルド済みなので、
# サブパス配信そのものを検証できる
E2E_BASE="$PUBLIC_URL" npx playwright test \
  || die "E2E が失敗しました。リリースを中断します"

# ── 4. origin/release 起点で作業ブランチを作る ──────────────────────────────
#
# ローカル release の鮮度に依存しないよう、必ず origin/release から切る

step "作業ブランチの作成"
WORK_BRANCH="release-$(date +%Y%m%d-%H%M)"
git show-ref --verify --quiet "refs/heads/${WORK_BRANCH}" \
  && die "${WORK_BRANCH} が既に存在します。1 分待つか、不要なら git branch -D してください"

git switch -c "$WORK_BRANCH" "origin/${RELEASE_BRANCH}"
echo "作成しました: ${WORK_BRANCH} = $(git rev-parse --short HEAD)（origin/${RELEASE_BRANCH} 起点）"

# ── 5. main をマージ ────────────────────────────────────────────────────────
#
# bootstrap で release を main から派生させてあるので共通祖先があり、
# --allow-unrelated-histories は不要（初回と 2 回目以降で挙動が変わらない）。
# git merge が起動するのは pre-merge-commit フックで pre-commit ではないため、
# ここでの docs/ ガードの再発火は起きない

step "${MAIN_BRANCH} をマージ"
git merge --no-ff -m "merge ${MAIN_BRANCH}" "$MAIN_BRANCH" \
  || die "${MAIN_BRANCH} のマージが競合しました。${WORK_BRANCH} 上で解決してください"

# ── 6. docs/ を作り直してコミット ───────────────────────────────────────────

step "docs/ の作り直し"
rm -rf docs
cp -R dist docs
touch docs/.nojekyll

# ★ git add -A / git add . は使わない。docs のみを明示的に stage する
git add -- docs

if git diff --cached --quiet; then
  echo "docs/ に差分はありません（ビルド結果が前回と同一）"
  if [ "$(git rev-parse HEAD)" = "$(git rev-parse "origin/${RELEASE_BRANCH}")" ]; then
    step "リリースする変更がありません"
    git switch "$MAIN_BRANCH"
    git branch -D "$WORK_BRANCH"
    WORK_BRANCH=""
    echo "origin/${RELEASE_BRANCH} は既に最新です。作業ブランチは削除しました。"
    exit 0
  fi
  echo "main のマージ分だけをリリースします"
else
  # ★ --no-verify は必須。.githooks/pre-commit は「docs/ を stage したコミット」を
  #   ブランチに関係なく拒否する（main への混入を防ぐガード）ので、付けないと
  #   このコミット自体が弾かれる。加えて fmt/clippy/test/trunk build/playwright が
  #   再走し、直前に通したばかりの検証がリリースのたびに二重実行になる
  git commit --no-verify -m "deploy $(date +%F)"
  echo "コミットしました: $(git rev-parse --short HEAD)"
fi

# ── 7. push して PR を作る ──────────────────────────────────────────────────

step "push"
git push -u origin HEAD

step "PR の作成"
git diff --stat "origin/${RELEASE_BRANCH}...HEAD" -- docs | tail -1
gh pr create --base "$RELEASE_BRANCH" --fill

PR_URL="$(gh pr view --json url --jq .url 2>/dev/null || echo '')"

step "${MAIN_BRANCH} に戻る"
# docs/ は WORK_BRANCH 側にコミット済みなので、切り替えで作業ツリーから消える
git switch "$MAIN_BRANCH"
WORK_BRANCH=""

cat <<EOF

==> リリース PR を作成しました

  PR       : ${PR_URL:-（gh pr view で確認してください）}
  公開 URL : ${SITE_URL}

次の一手（ここから先は手動）:
  1. PR の内容を確認する（docs/ の差分がビルド成果物だけであること）
  2. **merge コミットでマージする**（squash / rebase はリポジトリ設定で無効化済み）
  3. マージすると Pages が自動デプロイされる。反映を確認:
       gh api repos/${REPO}/pages --jq '{status,build_type,source}'
       curl -sI ${SITE_URL} | head -3
  4. Service Worker は cache-first + skipWaiting なので、**更新の反映は次回起動から**。
     iPhone 側で一度アプリを開いて閉じ、もう一度開いて確認すること
EOF
