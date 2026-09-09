// README の「画面」セクションと、設定タブのマニュアルの図を撮り直す。
//
//   trunk build
//   node scripts/shots.mjs                          # README 3 枚 + マニュアル 10 枚
//   node scripts/shots.mjs --only=manual            # マニュアル 10 枚だけ（★ pre-commit が呼ぶのはこれ）
//   node scripts/shots.mjs --only=readme            # README 3 枚だけ（UI を触った PR の最後に手で叩く）
//   node scripts/shots.mjs --only=manual:copy-last  # 1 図だけ（ja / en 両方）。反復用
//   node scripts/shots.mjs --check                  # 一時ディレクトリに撮ってバイト比較。差があれば exit 1
//
// 環境変数は scripts/static-server.mjs と同名同義にしてある:
//   SHOT_PORT … 撮影用サーバのポート（既定 4274）
//   DIST_DIR  … 配信するディレクトリ名（既定 dist。release.sh の dist-release 用）
//   E2E_BASE  … 配信のベースパス（既定 /。Pages 相当は /fitness-memo/）
//
// ★ 手で撮らないのは、撮り直しのたびに端末・データ・テーマがばらつくから。
//   タブ構成や UI を変えたら同じコマンドでまとめて更新できるようにしてある。
//
// ★ **決定性がこのスクリプトの設計の中心。** マニュアルの図は pre-commit が撮り直して
//   自動で stage するので、UI を 1 文字も変えていないのに違うバイト列が出ると、
//   コミットのたびに blob が積まれて git が膨張し始める。だから次を全部固定する:
//   時計（`SHOT_NOW`）/ タイムゾーン / locale / colorScheme / deviceScaleFactor /
//   `package.json` の `@playwright/test` の exact pin（Chromium が上がると全枚数 churn する）。
//
// - 端末は E2E と同じ iPhone 15 Pro
// - display-mode: standalone を偽装する。ホーム画面から起動した状態が本来の姿で、
//   ブラウザで開いたときだけ出る「ホーム画面に追加してください」のバナー
//   （calendar.rs の is_standalone 分岐）は図に写すと嘘になる
// - 記録は localStorage へ直接注入する（E2E の seedPastLogs と同じ手口）。
//   空のアプリを撮ってもカレンダーのドットもグラフも出ない

import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { join } from 'node:path';
import { mkdtemp, readFile, readdir, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { chromium, devices } from '@playwright/test';

const REPO_ROOT = join(fileURLToPath(new URL('.', import.meta.url)), '..');
const PORT = Number(process.env.SHOT_PORT || 4274);
// static-server.mjs:18-22 と同じ正規化。ここが食い違うと goto だけが 404 になる
const BASE_PATH = (() => {
  let b = process.env.E2E_BASE || '/';
  if (!b.startsWith('/')) b = `/${b}`;
  if (!b.endsWith('/')) b += '/';
  return b;
})();
const BASE = `http://localhost:${PORT}${BASE_PATH}`;

/**
 * 撮影時の「今」。**これが全体を成立させる。**
 *
 * ★ 下の SEED は `daysAgo` 基準なので、時計を固定しないと **UI を 1 文字も変えていない
 *   「日を跨いだコミット」でも**カレンダーの当日位置・経過表示・X 軸の右端が動く。
 *   pre-commit が自動 stage する設計なので、それは毎日のノイズ diff になる。
 *
 * ★ **オフセット（+09:00）を必ず付ける。** 付けないと Node のローカル TZ で解釈され、
 *   マシン TZ が UTC のとき JST 19:30 になって `at` と経過表示が動く。
 *
 * 木曜・月半ば（28 日前が前月に入る日）を選んである。
 */
const SHOT_NOW = '2026-03-12T10:30:00+09:00';

/** `SHOT_NOW` から見た `daysAgo` 日前の日付キー。**JST 固定**（マシン TZ に依らない）。 */
const JST_DAY = new Intl.DateTimeFormat('en-CA', {
  timeZone: 'Asia/Tokyo',
  year: 'numeric',
  month: '2-digit',
  day: '2-digit',
});
const dayKey = (daysAgo) => JST_DAY.format(new Date(Date.parse(SHOT_NOW) - daysAgo * 86_400_000));

/** 撮影日そのもの。記録タブの図はここを選ぶ */
const DAY_TODAY = dayKey(0);
/**
 * **体重だけがあってセットが無い日。** `empty-day` の被写体。
 *
 * ★ SEED は `daysAgo` 1 にログを持たず、BODY_WEIGHTS は 0〜28 の毎日を持つ。
 *   カードが 1 枚でもあると候補リストは消える（day.rs:404-414 の `candidates`）ので、ここが唯一の穴。
 */
const DAY_WEIGHT_ONLY = dayKey(1);

/**
 * 撮る画面。ファイル名の連番は README の並び順。
 *
 * `center` は「この要素が画面の中央に来るまでスクロールしてから撮る」指定。
 * 記録タブだけ指定があるのは、先頭で撮るとカレンダーしか入らず、この画面の要である
 * 「カレンダーと入力欄が縦に並んで 1 画面」（adr/ux/record-tab-calendar-with-day-editor.md）が写らないため。
 *
 * `settleChart` は折れ線の描き終わりを待つ指定。**README でだけ残している一律待ち**で、
 * マニュアルの図は全部条件待ちにしてある。
 *
 * ★ 設定タブは**トップ（節の一覧）をそのまま撮る**。タップして最初に出るのがこれで、
 *   各節の件数まで写る（adr/ux/settings-as-a-list-of-sections.md）。かつては部位を 1 つ
 *   開いた絵にしていたが、その一覧は節の中へ移ったので、開いた絵は「設定」の代表では
 *   なくなった。中身は README の本文が説明する。
 */
const SHOTS = [
  { file: '1-record.png', testid: 'tab-record', screen: 'screen-record', center: 'today-date' },
  { file: '2-progress.png', testid: 'tab-progress', screen: 'screen-progress', settleChart: true },
  { file: '3-menu.png', testid: 'tab-settings', screen: 'screen-settings' },
];

/**
 * 撮影用の記録。3〜4 日おきに部位を回す、ありがちな 4 週間分。
 * 当日ぶんも入れて「記録タブに入力済みのカードが並んでいる」状態にする。
 */
const SEED = [
  { daysAgo: 25, name: 'ベンチプレス', sets: [[50, 10], [50, 8], [50, 8]] },
  { daysAgo: 24, name: '懸垂', sets: [[0, 8], [0, 7], [0, 6]] },
  { daysAgo: 21, name: 'スクワット', sets: [[60, 10], [60, 10], [60, 8]] },
  { daysAgo: 18, name: 'ベンチプレス', sets: [[52.5, 10], [52.5, 9], [52.5, 8]] },
  { daysAgo: 17, name: 'ショルダープレス', sets: [[20, 12], [20, 10], [20, 10]] },
  { daysAgo: 14, name: '懸垂', sets: [[0, 9], [0, 8], [0, 7]] },
  { daysAgo: 11, name: 'ベンチプレス', sets: [[55, 10], [55, 9], [55, 8]] },
  { daysAgo: 10, name: 'スクワット', sets: [[70, 10], [70, 9], [70, 8]] },
  { daysAgo: 7, name: 'ベンチプレス', sets: [[57.5, 10], [57.5, 9], [57.5, 7]] },
  { daysAgo: 6, name: '懸垂', sets: [[0, 10], [0, 9], [0, 8]] },
  { daysAgo: 3, name: 'ベンチプレス', sets: [[60, 10], [60, 9], [60, 8]] },
  { daysAgo: 2, name: 'ラットプルダウン', sets: [[45, 12], [45, 10], [45, 10]] },
  { daysAgo: 0, name: 'ベンチプレス', sets: [[60, 10], [60, 10], [60, 8]] },
  { daysAgo: 0, name: 'ダンベルプレス', sets: [[22.5, 12], [22.5, 10]] },
];

/**
 * 図 `exercise-memo` の被写体。**ピンとインターバルは種目に貼り付く設定**なので
 * （adr/ux/machine-pins-on-the-exercise.md / adr/ux/interval-seconds-on-the-exercise.md）
 * UI 操作ではなくシードで立てる。打鍵で作ると 1 文字ごとに db が動いて絵が揺れる。
 *
 * ★ 当日にカードがある種目を選ぶ。無いと `note-toggle` が画面に出ない。
 * ★ 閉じていても薄字（`.pin-read` / `.interval-read`）で読めるので、
 *   **README の記録タブにもこの 2 行が写る**。章 4 が説明する挙動そのものなので許容する。
 */
const PINNED = { name: 'ベンチプレス', pins: ['3', '7'], interval_sec: 90 };

/**
 * 撮影用のトレーニングメニュー（adr/ux/start-from-a-saved-routine.md）。
 *
 * ★ 置かないと設定タブに「まだありません」の 1 行しか写らず、この画面の目玉が
 *   README から読めない。上の SEED と同じ分割（胸 / 背中）で 2 本だけ置く。
 *
 * ★ ID は予約領域（1024 未満）の外に固定で書く。撮り直しても同じ絵になるように
 *   するためで、`IdGen` を通さないのはここが撮影用の直接注入だから（実アプリの
 *   採番経路ではない）。12 文字の base32 でないと `core::migrate` が弾く。
 *
 * ★ `name_en` を持つのは **`Routine.name` が言語追従しないから**。利用者が付けた名前
 *   なので `src/views/day.rs:419` が素通しで描く。差し替えないと `empty-day` の
 *   en の図に「胸の日」が日本語で写る（種目と部位は `ex_name` / `grp_name` を通るので英語になる）。
 */
const ROUTINES = [
  {
    id: '00000000zzz1',
    name: '胸の日',
    name_en: 'Chest day',
    names: ['ベンチプレス', 'ダンベルプレス', 'チェストフライ'],
  },
  {
    id: '00000000zzz2',
    name: '背中の日',
    name_en: 'Back day',
    names: ['懸垂', 'ラットプルダウン', 'デッドリフト'],
  },
];

/**
 * 撮影用の体重（`daysAgo` 28 → 0 の毎日）。推移タブの第2軸に出る（adr/ux/body-weight-second-axis-always-on.md）。
 *
 * ★ 記録日だけでなく**毎日**入れる。体重は毎日でトレーニングは週数回、という
 *   実際の使い方でしか X 軸の合併ドメイン（最後にトレした日より後の計量まで軸が
 *   伸びる）が絵に出ない。値は乱数にせず直書きして、撮り直しても同じ絵になるようにする。
 */
const BODY_WEIGHTS = [
  68.4, 68.6, 68.3, 68.5, 68.9, 68.7, 68.6, 69.0, 68.8, 69.1,
  68.9, 69.2, 69.4, 69.1, 69.3, 69.6, 69.4, 69.2, 69.5, 69.8,
  69.6, 69.9, 70.1, 69.8, 70.0, 70.3, 70.1, 70.4, 70.2,
].map((kg, i, all) => ({ daysAgo: all.length - 1 - i, kg }));

/** マニュアルの図を出す言語。`public/manual/<lang>/<id>.webp` の `<lang>` になる */
const LANGS = ['ja', 'en'];

/**
 * WebP の品質。**明示が必須。**
 *
 * ★ 省略すると Playwright の既定 100 = lossless になり、Chromium の lossless WebP は
 *   **PNG の約 2 倍**（実測 1179×600: PNG 45,169 / lossless 88,950 / q90 前後で 2 万台）。
 *   README の PNG を pre-commit から外した理由（1 回 383KB の churn）と同じ規模の
 *   膨張がそのまま戻る。
 */
const WEBP_QUALITY = 90;

/**
 * マニュアルの図。`targets` の外接矩形 + `pad` を `clip` に渡して撮る。
 * こうすると **`src/` に撮影専用の testid を 1 つも足さずに済む**。
 *
 * ★ **順序に意味がある。** `copy-last` は UI 操作で当日に空のカードを 1 枚生やすので
 *   最後に置く。`empty-day` は別の日を選ぶので、その次に当日へ戻ると `load_cards` が
 *   db から引き直して空カードが消える（`pick()` は db を書かない）。
 *   この順なら `--only=manual:<id>` の単発撮影と全枚数撮影が同じバイト列になる。
 */
const FIGS = [
  { id: 'chart-readout', pad: 10, setup: setupReadout },
  // ★ 上寄せ。中央寄せだと下端が sticky の「種目を追加」の帯（画面下 528px 付近に
  //   貼り付く）に潜り込む
  { id: 'exercise-memo', pad: 10, align: 'top', setup: setupMemoOpen },
  { id: 'empty-day', pad: 8, setup: setupEmptyDay },
  // ★ シートは position: fixed で画面幅いっぱい（styles.css の `.sheet`）なので
  //   pad を足すと必ず viewport の左右をはみ出す。ここだけ 0。
  //   スクロールしても動かないので中央寄せもしない（背後だけが動いて無駄）
  { id: 'accordions', pad: 0, scroll: false, setup: setupAccordions },
  { id: 'copy-last', pad: 8, setup: setupCopyLast },
];

// ── 引数 ──────────────────────────────────────────────────────────────────────

const argv = process.argv.slice(2);
const check = argv.includes('--check');
const only = argv.find((a) => a.startsWith('--only='))?.slice('--only='.length) ?? null;
for (const a of argv) {
  if (a !== '--check' && !a.startsWith('--only=')) {
    throw new Error(`知らない引数: ${a}（--only=readme|manual|manual:<id> / --check）`);
  }
}

let wantReadme = true;
let wantManual = true;
let figFilter = null;
if (only !== null) {
  const [kind, id, ...rest] = only.split(':');
  if (rest.length > 0) throw new Error(`--only= の形が違う: ${only}`);
  if (kind === 'readme') {
    if (id) throw new Error('--only=readme に図の指定は付かない');
    wantManual = false;
  } else if (kind === 'manual') {
    wantReadme = false;
    if (id) {
      if (!FIGS.some((f) => f.id === id)) {
        throw new Error(`知らない図: ${id}（${FIGS.map((f) => f.id).join(' / ')}）`);
      }
      figFilter = id;
    }
  } else {
    throw new Error(`--only= は readme / manual / manual:<id> のどれか: ${only}`);
  }
}

const figs = FIGS.filter((f) => figFilter === null || f.id === figFilter);

// ── 出力先 ────────────────────────────────────────────────────────────────────

// `--check` は**リポジトリに 1 バイトも書かない**。一時ディレクトリへ撮ってから比べる
const outRoot = check ? await mkdtemp(join(tmpdir(), 'fitness-memo-shots-')) : REPO_ROOT;
const readmeOut = (file) => join(outRoot, 'assets', file);
const manualOut = (lang, id) => join(outRoot, 'public', 'manual', lang, `${id}.webp`);
const manualRepo = (lang, id) => join(REPO_ROOT, 'public', 'manual', lang, `${id}.webp`);

// ── ページ側に流し込む init script ────────────────────────────────────────────

/**
 * `display-mode: standalone` の偽装。
 *
 * ★ CDP の Emulation.setEmulatedMedia に display-mode を渡しても
 *   matchMedia('(display-mode: standalone)') は false のままだった。
 *   代わりにその問い合わせだけを常に真になるクエリ（'all'）へ差し替える。
 *   本物の MediaQueryList を返すので matches 以外の面も壊れない。
 */
function fakeStandalone() {
  const orig = window.matchMedia.bind(window);
  window.matchMedia = (q) =>
    (String(q).includes('display-mode: standalone') ? orig('all') : orig(q));
}

/**
 * `localStorage` への注入。init script は**文書ごとに毎回**走るので、reload しても
 * 同じ状態から始まる。
 *
 * ★ init script は `about:blank` の初期文書でも走る。そこは opaque origin なので
 *   `localStorage` へ触ると SecurityError を投げる。origin を見てから try/catch で囲む。
 */
function seedStorage({ ui, db }) {
  if (location.origin === 'null') return;
  try {
    // ★ storage.rs の UI_KEY / KEY と一致していること。schema 世代ごとにキーを切る運用
    //   （adr/storage/storage-key-per-schema-generation.md）なので、ここが古いと
    //   getItem が null を返して黙って落ちる
    localStorage.setItem('fitness-memo/ui/v1', ui);
    if (db) localStorage.setItem('fitness-memo/v3', db);
  } catch {
    // opaque origin。本物の文書で撮り直される
  }
}

/**
 * UI 状態。**`manual_hint_dismissed: true` を全 context に書く。**
 *
 * ★ このスクリプトは standalone を偽装するので `ManualHint` の出現条件
 *   （`applicable && !install_hint_dismissed()`）が真になり、記録タブの手掛かりが
 *   **図に焼き込まれる**。README の 1-record.png は `today-date` を中央に置く
 *   viewport 撮りなので、可視域に置いた手掛かりは確実に写り込む。
 *
 * ★ `fitness-memo/ui/v1` は `Db` とは別のキー（adr/storage/ui-state-in-separate-key.md）。
 *   `UiState` の全フィールドが `#[serde(default)]` なので、まだ実装されていない
 *   フィールドを先に書いても既存の版が黙って無視するだけで害はない。
 */
const UI_STATE = JSON.stringify({ manual_hint_dismissed: true });

// ── 小道具 ────────────────────────────────────────────────────────────────────

/** 条件が満たされるまで待つ。**一律待ちを置かないため**の道具 */
async function waitFor(fn, what, timeoutMs = 10_000) {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    if (await fn()) return;
    if (Date.now() > deadline) throw new Error(`待ち切れなかった: ${what}`);
    await new Promise((r) => setTimeout(r, 50));
  }
}

/**
 * rAF を 2 フレーム待つ。
 *
 * ★ `pick()` は `scroll_to_id` → **rAF の次フレーム**で `scroll_into_view()` する
 *   （day.rs:385-394 の `pick` → `views::mod` の `scroll_to_id`）。そのフレームより先に `boundingBox()` を
 *   測ると座標が 1 フレームぶんずれ、run ごとに違う絵が出る。fake clock 下でも
 *   rAF は実時間で発火するので、待てば必ず追いつく。
 */
const settle = (page) =>
  page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));

/**
 * 撮る対象の**外接矩形**を viewport の中央（`align: 'top'` なら上端 + `margin`）へ入れる。
 *
 * ★ 対象 1 つを `scrollIntoView({ block: 'center' })` するのでは足りない。図は複数の
 *   要素の外接矩形なので、真ん中の 1 つを中央に置いても上下がはみ出しうる。
 * ★ スクロール量を整数へ丸める。小数のスクロール位置はサブピクセルの描画差を生み、
 *   同じ UI から違うバイト列が出る。
 * ★ 端で止まって狙った位置に来ないことはある。その場合も clipOf の viewport 判定と
 *   assertNoStickyOverlap が守る。
 */
async function scrollUnion(page, targets, { align = 'center', margin = 0 } = {}) {
  const boxes = [];
  for (const t of targets) {
    const b = await t.boundingBox();
    if (!b) throw new Error('クリップの対象が画面に無い');
    boxes.push(b);
  }
  const top = Math.min(...boxes.map((b) => b.y));
  const bottom = Math.max(...boxes.map((b) => b.y + b.height));
  const delta =
    align === 'top'
      ? Math.round(top - margin)
      : Math.round((top + bottom) / 2 - (await page.evaluate(() => innerHeight)) / 2);
  if (delta !== 0) {
    await page.evaluate((d) => window.scrollBy({ top: d, behavior: 'instant' }), delta);
    await settle(page);
  }
}

/**
 * sticky な「種目を追加」の帯が図に被っていないこと。
 *
 * ★ `.add-wrap` は `bottom: タブバー + 8px` に貼り付く（styles.css:1020-1034）ので、
 *   viewport に収まっていても**下の方を撮ると帯の地色が図に焼き込まれる**。
 *   clipOf の viewport 判定では捕まらない静かな壊れ方なので、別に見る。
 * ★ シート（`<dialog>`）を撮る図は帯を手前から覆い隠すので対象外。
 */
async function assertNoStickyOverlap(page, clip, figId) {
  if ((await page.locator('dialog[open]').count()) > 0) return;
  const bar = page.locator('.add-wrap');
  // ★ **count() で先に確かめる。** 要素が無いまま boundingBox() を呼ぶと
  //   既定のタイムアウト 30 秒まで待ってから投げる（推移タブには帯が無いので、
  //   catch で握り潰すと撮影が黙って 30 秒延びる。実測で踏んだ）
  if ((await bar.count()) === 0) return;
  const box = await bar.boundingBox();
  if (!box) return;
  const hit =
    box.x < clip.x + clip.width &&
    box.x + box.width > clip.x &&
    box.y < clip.y + clip.height &&
    box.y + box.height > clip.y;
  if (hit) {
    throw new Error(
      `図 ${figId} に sticky の「種目を追加」の帯が被る: 帯 ${JSON.stringify(box)} / clip ${JSON.stringify(clip)}`,
    );
  }
}

/**
 * 開きっぱなしのシートを閉じる。
 *
 * ★ `Sheet` はネイティブ `<dialog>` を `show_modal()` で開くので、開いている間は
 *   **背後のタブもカレンダーも押せない**（top layer が pointer events を奪う）。
 *   図と図の間で状態を持ち越さないよう、タブへ移る前に必ず畳む。
 *   Esc は ✕ / 背景タップと同じ `on_close` に集まる（views/mod.rs の `Sheet`）。
 */
async function closeAnySheet(page) {
  const open = page.locator('dialog[open]');
  if ((await open.count()) === 0) return;
  await page.keyboard.press('Escape');
  await open.first().waitFor({ state: 'hidden' });
  await settle(page);
}

/** タブを切り替えて先頭へ戻す。タブを跨いでもスクロール位置は持ち越されるので毎回戻す */
async function gotoTab(page, testid, screen) {
  await closeAnySheet(page);
  await page.getByTestId(testid).click();
  await page.getByTestId(screen).waitFor({ state: 'visible' });
  await page.evaluate(() => window.scrollTo(0, 0));
  await settle(page);
}

/** カレンダーの日を選ぶ */
async function selectDay(page, date) {
  const day = page.locator(`[data-testid="cal-day"][data-date="${date}"]`);
  await day.waitFor({ state: 'visible' });
  await day.click();
  await settle(page);
}

/**
 * `targets` の外接矩形 + `pad` を clip にする。
 *
 * ★ **自分で整数化してから渡す。** Playwright は `trimClipToSize` の後
 *   `enclosingIntSize`（= floor）で **width/height だけ**を丸め、**x/y は小数のまま**
 *   CDP へ渡す。右と下の pad が最大 1px 欠け、サブピクセルの揺れで寸法が 1px 動く。
 *   `enclosingIntRect` 相当（floor x/y, ceil x2/y2）にすれば入力が整数になって揺れない。
 *
 * ★ **viewport をはみ出したら落とす。** `trimClipToSize` は黙って切るので、
 *   欠けた図が静かにできる。左右も切られるので x も見る
 *   （既存の standalone 検証と同じ「撮ってから気づかない」ための作法）。
 */
async function clipOf(page, targets, pad, viewport) {
  const boxes = [];
  for (const t of targets) {
    const b = await t.boundingBox();
    if (!b) throw new Error('クリップの対象が画面に無い');
    boxes.push(b);
  }
  const left = Math.min(...boxes.map((b) => b.x)) - pad;
  const top = Math.min(...boxes.map((b) => b.y)) - pad;
  const right = Math.max(...boxes.map((b) => b.x + b.width)) + pad;
  const bottom = Math.max(...boxes.map((b) => b.y + b.height)) + pad;
  const x = Math.floor(left);
  const y = Math.floor(top);
  const clip = { x, y, width: Math.ceil(right) - x, height: Math.ceil(bottom) - y };
  const over =
    clip.x < 0 ||
    clip.y < 0 ||
    clip.x + clip.width > viewport.width ||
    clip.y + clip.height > viewport.height;
  if (over) {
    throw new Error(
      `クリップが viewport(${viewport.width}×${viewport.height}) をはみ出す: ${JSON.stringify(clip)}`,
    );
  }
  return clip;
}

/** 日本語（かな・カナ・漢字・半角カナ）が 1 文字でも含まれるか */
const JAPANESE = /[ぁ-ゟ゠-ヿ㐀-䶿一-鿿ｦ-ﾟ]/;

/**
 * **en の図に日本語が写らないこと**を図ごとに確かめる。
 *
 * ★ ctx C の `Db` は ja 由来（`Exercise.name` は日本語のまま保存されている）で、
 *   表示だけが `ex_name` / `grp_name` を通って英語になる。誰かが `Exercise.name` を
 *   素通しで描いたら en の図が黙って日本語になる。
 *
 * ★ `card-name` 1 本では足りない。あれは記録タブの見出しなので、`ex_name` / `grp_name`
 *   を通していない画面（`routine-candidate` の見出しなど）を捕まえられない。
 *   **撮る図に実際に写るものだけを、図ごとに見る。**
 */
async function assertEnglish(targets, figId) {
  for (const t of targets) {
    const text = (await t.evaluate((el) => el.textContent ?? '')).replace(/\s+/g, ' ').trim();
    const hit = text.match(JAPANESE);
    if (hit) {
      throw new Error(`en の図 ${figId} に日本語が写る（${hit[0]}）: ${text.slice(0, 120)}`);
    }
  }
}

/** 偽装が効いていないと「ホーム画面に追加してください」バナーが写り込む */
async function assertStandalone(page, label) {
  const ok = await page.evaluate(() => window.matchMedia('(display-mode: standalone)').matches);
  if (!ok) throw new Error(`${label}: display-mode: standalone の偽装が効いていない`);
}

// ── シードから引く ID とインデックス ──────────────────────────────────────────

/**
 * setup が使う ID を**シード JSON から引く**。
 *
 * ★ ID を手書きしない。12 文字 Crockford base32 のタイポが別の種目に当たると、
 *   誤った絵が黙って出る（落ちてくれない）。
 * ★ 名前でも引かない。ctx C は表示が英語なので、ラベル一致では解決できない。
 */
function seedIndex(json) {
  const db = JSON.parse(json);
  const byName = (name) => {
    const e = db.exercises.find((x) => x.name === name);
    if (!e) throw new Error(`プリセットに無い種目: ${name}`);
    return e;
  };
  /**
   * 「種目を追加」シートでの (部位, 種目) の並び順。
   *
   * ★ シートは**中身が 0 の部位を出さない**（day.rs:696）ので、出る部位だけを数える。
   *   部位は `order` 順、種目も部位内で `order` 順（day.rs:690）。
   */
  const pickIndex = (name) => {
    const ex = byName(name);
    const shown = [...db.groups]
      .sort((a, b) => a.order - b.order)
      .filter((g) => db.exercises.some((e) => e.group_id === g.id && !e.archived));
    const group = shown.findIndex((g) => g.id === ex.group_id);
    const inGroup = db.exercises
      .filter((e) => e.group_id === ex.group_id && !e.archived)
      .sort((a, b) => a.order - b.order);
    const exercise = inGroup.findIndex((e) => e.id === ex.id);
    if (group < 0 || exercise < 0) throw new Error(`シートに出ない種目: ${name}`);
    return { group, exercise };
  };
  return { byName, pickIndex };
}

/**
 * ctx C（en）へ流す JSON。**`Routine.name` だけ英語に差し替える。**
 *
 * ★ 種目・部位の名前は差し替えない。`src/presets.rs:50-53` が「名前は言語で変わるが
 *   ID は変わらない」を保証し、`views::mod` の `ex_name` / `grp_name`（mod.rs:244, 249）が
 *   表示を `cur_lang()` に追従させる。**シードの英訳は要らない。**
 */
function englishRoutines(json) {
  const db = JSON.parse(json);
  const en = new Map(ROUTINES.map((r) => [r.id, r.name_en]));
  for (const r of db.routines ?? []) {
    const name = en.get(r.id);
    if (name) r.name = name;
  }
  return JSON.stringify(db);
}

// ── 図ごとの setup ────────────────────────────────────────────────────────────
//
// setup は「撮る直前の状態」を作り、clip の対象になる locator の配列を返す。
// スクロール（scrollUnion）とクリップ（clipOf）は shootManual が共通に行うので、
// setup は**位置を決めない**。

/**
 * 章 2「グラフをタップすると別の点へ動く」。
 *
 * ★ **`selectOption` は value（ID 文字列）で指定する。** progress.rs:584-588 / 614 が
 *   `<option value=id.to_string()>` なので、ラベル「胸」は en コンテキストで解決できない。
 * ★ **`chart-hit` は band ごとに複数ある。** 素の `getByTestId('chart-hit').click()` は
 *   strict mode violation になるので中央の 1 本を押す。
 * ★ **読み取り欄は既定で出ている**（chart.rs:88-92 の Effect が最新点を選ぶ）ので、
 *   「可視待ち」では何も検証できない。`chart-cursor` の `line` の `x1` が動いたことを見る。
 */
async function setupReadout(page, { ids }) {
  await gotoTab(page, 'tab-progress', 'screen-progress');
  const bench = ids.byName(PINNED.name);
  await page.getByTestId('group-select').selectOption(String(bench.group_id));
  await page.getByTestId('exercise-select').selectOption(String(bench.id));
  const chart = page.getByTestId('chart');
  await chart.waitFor({ state: 'visible' });

  const cursorX = () => page.getByTestId('chart-cursor').locator('line').getAttribute('x1');
  const atLatest = await cursorX();
  const hits = page.getByTestId('chart-hit');
  await waitFor(async () => (await hits.count()) >= 3, 'chart-hit が 3 本以上出る');
  const n = await hits.count();
  await hits.nth(Math.floor(n / 2)).click();
  await waitFor(async () => (await cursorX()) !== atLatest, 'chart-cursor が最新点から動く');

  return [chart, page.getByTestId('chart-readout')];
}

/**
 * 章 4「＋ メモ 1 つで 4 つが一斉に開く」。**一番効く図。**
 *
 * ★ `note-toggle` は**カードごとにある**ので、必ずカードへスコープを絞ってから押す。
 */
async function setupMemoOpen(page, { ids }) {
  await gotoTab(page, 'tab-record', 'screen-record');
  await selectDay(page, DAY_TODAY);
  const card = page.locator(`#card-${ids.byName(PINNED.name).id}`);
  await card.waitFor({ state: 'visible' });
  await card.getByTestId('note-toggle').click();
  for (const t of ['set-note', 'pin-box', 'interval-box', 'exercise-note']) {
    await card.getByTestId(t).first().waitFor({ state: 'visible' });
  }
  // ★ **セット行から種目メモまで。** 4 つ目（セット行ごとのメモ欄）はセット行の中に
  //   あるので、`pin-box` / `interval-box` / `exercise-note` の 3 つだけでは章 4 の主張
  //   （1 つのトグルで 4 つが一斉に開く）が図に写らない。
  // ★ **フッタの「＋ メモ」までは伸ばせない。** カード全体は 635px で 393×659 に入らず、
  //   フッタ（540-593）は sticky の帯（528-594）の裏にある。押す場所は本文が説明する。
  // ★ 種目メモの「メモ」ラベルは入力欄と同じ行（.ex-note は flex）なので、
  //   入力欄を対象にすれば外接矩形に入る。
  return [card.getByTestId('set-row').first(), card.getByTestId('exercise-note')];
}

/**
 * 章 5「空の日だけ候補リストが出る」。
 *
 * ★ 体重だけの日を選ぶ。カードが 1 枚でもあると候補は消える（day.rs:404-414 の `candidates`）。
 */
async function setupEmptyDay(page) {
  await gotoTab(page, 'tab-record', 'screen-record');
  await selectDay(page, DAY_WEIGHT_ONLY);
  await page.getByTestId('menu-copy').waitFor({ state: 'visible' });

  // ★ **リストの末尾までは撮らない。** 「種目を追加」の帯は sticky で
  //   `bottom: タブバー + 8px` に貼り付く（styles.css の `.add-wrap`）ので、
  //   候補が 6 件並ぶこの日は**最後の 1 枚が帯の地色に半分隠れる**。
  //   そのまま撮ると図が壊れて見えるので、2 つの出所が読める最小限で切る。
  const recent = page.getByTestId('menu-candidate');
  await waitFor(async () => (await recent.count()) >= 2, '「最近の記録から」が 2 件以上出る');
  return [page.locator('.menu-copy .menu-copy-label').first(), recent.nth(1)];
}

/**
 * 章 6「部位の折りたたみは 1 つだけ開く」。被写体は**記録タブの「種目を追加」シート**。
 *
 * ★ メニュー編集シートは撮らない。あそこだけ `ex_name` / `grp_name` を通しておらず
 *   （別 PR で直す既存バグ）、en の図に日本語が写る。
 */
async function setupAccordions(page, { ids }) {
  await gotoTab(page, 'tab-record', 'screen-record');
  await selectDay(page, DAY_TODAY);
  await page.getByTestId('add-exercise').click();
  const sheet = page.getByTestId('add-sheet');
  await sheet.waitFor({ state: 'visible' });
  const { group } = ids.pickIndex(PINNED.name);
  await page.getByTestId('pick-group-toggle').nth(group).click();
  const opened = page.getByTestId('pick-group').nth(group).getByTestId('pick-exercise');
  await waitFor(async () => (await opened.count()) > 0, '部位が 1 つ開く');
  await settle(page);
  return [sheet];
}

/**
 * 章 3「前回をコピーはその日のセットが空のときだけ出る」。
 *
 * ★ **`sets: []` のログをシードする手口は使えない。** `storage::load` が必ず
 *   `core::migrate` → `normalize` → `dedupe_logs` を通し、`.filter(|l| !l.is_empty())`
 *   が**セット空・メモ空のログを落とす**ので、カードが生えず `copy-last` も出ない。
 *   `add-exercise` → `pick-exercise` の **UI 操作**で当日の空カードを作る。
 *
 * ★ **選ぶ種目は当日にセットを持たないもの**に限る。シードは当日にベンチプレスと
 *   ダンベルプレスを持つので、その 2 つでは `show_copy`（day.rs:943）が偽になる。
 *
 * ★ `pick()` の rAF スクロールと競合する。**2 フレーム待ってから** shootManual の
 *   `scrollUnion` が位置を上書きするので、run ごとに絵が変わらない。
 */
async function setupCopyLast(page, { ids }) {
  const name = '懸垂';
  await gotoTab(page, 'tab-record', 'screen-record');
  await selectDay(page, DAY_TODAY);
  await page.getByTestId('add-exercise').click();
  await page.getByTestId('add-sheet').waitFor({ state: 'visible' });
  const { group, exercise } = ids.pickIndex(name);
  await page.getByTestId('pick-group-toggle').nth(group).click();
  await page.getByTestId('pick-group').nth(group).getByTestId('pick-exercise').nth(exercise).click();
  await page.getByTestId('add-sheet').waitFor({ state: 'hidden' });
  await settle(page);

  const card = page.locator(`#card-${ids.byName(name).id}`);
  await card.getByTestId('copy-last').waitFor({ state: 'visible' });
  return [card];
}

// ── 撮影 ──────────────────────────────────────────────────────────────────────

async function shootReadme(page) {
  const out = [];
  for (const { file, testid, screen, center, settleChart } of SHOTS) {
    await gotoTab(page, testid, screen);
    if (center) {
      await page
        .getByTestId(center)
        .first()
        .evaluate((el) => el.scrollIntoView({ block: 'center', behavior: 'instant' }));
      await settle(page);
    }
    // ★ ここだけ一律待ちを残す。折れ線の落ち着きに条件を書けるフックが無い
    if (settleChart) await page.waitForTimeout(300);
    const path = readmeOut(file);
    await page.screenshot({ path, animations: 'disabled' });
    out.push({ shot: path, repo: join(REPO_ROOT, 'assets', file), label: `assets/${file}` });
    if (!check) console.log(`撮影: assets/${file}`);
  }
  return out;
}

async function shootManual(page, cx) {
  const out = [];
  for (const fig of figs) {
    const targets = await fig.setup(page, cx);
    if (fig.scroll !== false) {
      await scrollUnion(page, targets, { align: fig.align, margin: fig.pad + 4 });
    }
    if (cx.lang === 'en') await assertEnglish(targets, fig.id);
    const clip = await clipOf(page, targets, fig.pad, cx.viewport);
    await assertNoStickyOverlap(page, clip, fig.id);
    const path = manualOut(cx.lang, fig.id);
    await page.screenshot({
      path, // 拡張子 .webp から type を推論する
      clip,
      quality: WEBP_QUALITY,
      // ★ `.sheet` の 0.22s の translate と backdrop のフェードが途中で写るのを止める
      animations: 'disabled',
    });
    const label = `public/manual/${cx.lang}/${fig.id}.webp`;
    out.push({ shot: path, repo: manualRepo(cx.lang, fig.id), label });
    if (!check) console.log(`撮影: ${label}（${clip.width}×${clip.height}）`);
  }
  return out;
}

// ── 実行 ──────────────────────────────────────────────────────────────────────

async function waitForServer(url, timeoutMs = 10_000) {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    try {
      const res = await fetch(url);
      if (res.ok) return;
    } catch {
      // まだ listen していない
    }
    if (Date.now() > deadline) throw new Error(`static-server が ${url} で応答しない`);
    await new Promise((r) => setTimeout(r, 150));
  }
}

const server = spawn(process.execPath, [join(REPO_ROOT, 'scripts/static-server.mjs')], {
  cwd: REPO_ROOT,
  env: {
    ...process.env,
    PORT: String(PORT),
    E2E_BASE: BASE_PATH,
    DIST_DIR: process.env.DIST_DIR || 'dist',
  },
  stdio: 'inherit',
});
// 例外で落ちてもサーバを残さない（孤児プロセスが次のビルドのポートを塞ぐ）
const stopServer = () => server.kill();
process.on('exit', stopServer);
process.on('SIGINT', () => process.exit(130));

/**
 * コンテキストを 1 つ作る。**全 context で共通に固定するもの**をここに集める
 * （1 つでも忘れると、その言語の図だけが違う条件で撮られる）。
 */
async function newContext(browser, { locale, deviceScaleFactor, db }) {
  const context = await browser.newContext({
    ...devices['iPhone 15 Pro'],
    ...(deviceScaleFactor === undefined ? {} : { deviceScaleFactor }),
    // ★ locale を固定する。既定の en-US だとアプリが英語 UI + 英語プリセットで起動し、
    //   SEED が日本語の種目名で引いているので**撮影自体が落ちる**
    locale,
    // ★ 現状は locale だけを固定していて TZ はマシン依存だった。`at` と経過表示が動く
    timezoneId: 'Asia/Tokyo',
    // ★ 既定に頼らない。dark で撮れる事故を塞ぐ
    //   （adr/ux/declare-color-scheme-for-ua-widgets.md が「shots.mjs はライトで撮る」）
    colorScheme: 'light',
  });
  // ★ `goto` より前。context 単位に効き、init script として登録される
  await context.clock.setFixedTime(new Date(SHOT_NOW));
  await context.addInitScript(fakeStandalone);
  await context.addInitScript(seedStorage, { ui: UI_STATE, db: db ?? null });
  const page = await context.newPage();
  await page.emulateMedia({ colorScheme: 'light' });
  return { context, page };
}

let browser;
let failed = false;
try {
  await waitForServer(BASE);
  browser = await chromium.launch();

  // ── ctx A: シード投入 + README ──────────────────────────────────────────────
  // deviceScaleFactor は devices の値（3）のまま。README は GitHub が任意サイズで
  // 描くので、マニュアルの図（dsf 2）とは事情が違う
  const { page: pageA } = await newContext(browser, { locale: 'ja-JP' });
  await pageA.goto(BASE);
  // プリセット投入 (App の Effect) が走った後でないと exercises が空
  await pageA.getByTestId('screen-record').waitFor({ state: 'visible' });
  await assertStandalone(pageA, 'ctx A');

  // 400ms debounce の保存を先に確定させてから読み書きする
  await pageA.evaluate(() => {
    Object.defineProperty(document, 'hidden', { value: true, configurable: true });
    document.dispatchEvent(new Event('visibilitychange', { bubbles: true }));
  });

  const seedJson = await pageA.evaluate(
    ({ seed, weights, routines, pinned }) => {
      const KEY = 'fitness-memo/v3';
      const db = JSON.parse(localStorage.getItem(KEY));
      // Local::now().date_naive() と揃えるため UTC ではなくローカル日付で組み立てる
      // （時計は setFixedTime で、TZ は timezoneId で固定してある）
      const dateKey = (daysAgo) => {
        const d = new Date();
        d.setDate(d.getDate() - daysAgo);
        const p = (n) => String(n).padStart(2, '0');
        return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
      };
      for (const { daysAgo, name, sets } of seed) {
        const ex = db.exercises.find((e) => e.name === name);
        if (!ex) throw new Error(`プリセットに無い種目: ${name}`);
        const key = dateKey(daysAgo);
        const session = db.sessions[key] ?? { logs: [], body_weight: null, note: '' };
        session.logs.push({
          exercise_id: ex.id,
          sets: sets.map(([weight, reps]) => ({ weight, reps })),
          // at は当日ぶんだけ埋める（過去日バックフィルは null。ExerciseLog.at の意味）
          at: daysAgo === 0 ? Date.now() : null,
        });
        db.sessions[key] = session;
      }
      // トレーニングしていない日にも体重は入る（Session::is_empty はそれを空扱いしない）
      for (const { daysAgo, kg } of weights) {
        const key = dateKey(daysAgo);
        const session = db.sessions[key] ?? { logs: [], body_weight: null, note: '' };
        session.body_weight = kg;
        db.sessions[key] = session;
      }
      const pex = db.exercises.find((e) => e.name === pinned.name);
      if (!pex) throw new Error(`プリセットに無い種目: ${pinned.name}`);
      pex.pins = pinned.pins;
      pex.interval_sec = pinned.interval_sec;
      db.routines = routines.map(({ id, name, names }) => ({
        id,
        name,
        exercises: names.map((n) => {
          const ex = db.exercises.find((e) => e.name === n);
          if (!ex) throw new Error(`プリセットに無い種目: ${n}`);
          return ex.id;
        }),
      }));
      localStorage.setItem(KEY, JSON.stringify(db));
      // ★ Node 側へ返す。ctx B / C はこれを init script で流し込むので、
      //   boot → 注入 → reload の 2 ロードが 1 ロードで済む
      return localStorage.getItem(KEY);
    },
    { seed: SEED, weights: BODY_WEIGHTS, routines: ROUTINES, pinned: PINNED },
  );

  await pageA.reload();
  await pageA.getByTestId('screen-record').waitFor({ state: 'visible' });

  const ids = seedIndex(seedJson);
  const viewport = pageA.viewportSize();

  // ── ctx B / C: マニュアル ja / en ──────────────────────────────────────────
  //
  // ★ dsf 2 は README（devices 既定の 3）からの意図した逸脱。マニュアルの図は
  //   `.man-fig { max-width: 365px }` に収まり撮影幅も CSS 361px 前後なので、
  //   dsf 2 で既に 2 倍密度。dsf 3 は 2.25 倍のバイト数を払って画面上で区別が付かない。
  const manualCtx = [];
  if (wantManual) {
    for (const lang of LANGS) {
      const { page } = await newContext(browser, {
        locale: lang === 'ja' ? 'ja-JP' : 'en-US',
        deviceScaleFactor: 2,
        db: lang === 'en' ? englishRoutines(seedJson) : seedJson,
      });
      await page.goto(BASE);
      await page.getByTestId('screen-record').waitFor({ state: 'visible' });
      await assertStandalone(page, `ctx ${lang}`);
      manualCtx.push({ page, lang, ids, viewport });
    }
    // 言語の出し分けが効いていること。ここが偽なら figure ごとの assert 以前の問題
    const en = manualCtx.find((c) => c.lang === 'en');
    const first = await en.page.getByTestId('card-name').first().textContent();
    if (first?.trim() !== 'Bench Press') {
      throw new Error(`ctx en の表示が英語になっていない: ${first}`);
    }
  }

  // ── フェーズ 2: 3 コンテキストは完全に独立なので並列で撮る ──────────────────
  const shot = (
    await Promise.all([
      wantReadme ? shootReadme(pageA) : Promise.resolve([]),
      ...manualCtx.map((cx) => shootManual(cx.page, cx)),
    ])
  ).flat();

  // ── --check: バイト比較 ────────────────────────────────────────────────────
  if (check) {
    const diffs = [];
    for (const { shot: a, repo: b, label } of shot) {
      const [got, want] = await Promise.all([
        readFile(a).catch(() => null),
        readFile(b).catch(() => null),
      ]);
      if (!got) diffs.push(`${label}: 撮れていない`);
      else if (!want) diffs.push(`${label}: 手元に無い（コミットされていない）`);
      else if (!got.equals(want)) {
        diffs.push(`${label}: 内容が違う（撮影 ${got.length}B / 手元 ${want.length}B）`);
      }
    }
    // 図を消したのにファイルが残っている場合も差分として扱う（全枚数を撮ったときだけ）
    if (wantManual && figFilter === null) {
      for (const lang of LANGS) {
        const dir = join(REPO_ROOT, 'public', 'manual', lang);
        for (const f of await readdir(dir).catch(() => [])) {
          if (!FIGS.some((fig) => `${fig.id}.webp` === f)) {
            diffs.push(`public/manual/${lang}/${f}: 図の一覧に無い（孤児）`);
          }
        }
      }
    }
    if (diffs.length > 0) {
      console.error('スクリーンショットが実装とずれています:');
      for (const d of diffs) console.error(`  ${d}`);
      console.error('`node scripts/shots.mjs` で撮り直してコミットしてください');
      failed = true;
    } else {
      console.log(`--check: ${shot.length} 枚とも一致`);
    }
  }
} finally {
  await browser?.close();
  stopServer();
  if (check) await rm(outRoot, { recursive: true, force: true });
}

if (failed) process.exit(1);
