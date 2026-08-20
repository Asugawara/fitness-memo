// README の「画面」セクション用スクリーンショットを撮り直す。
//
//   trunk build && node scripts/shots.mjs   （dist/ をそのまま配信して撮る）
//
// ★ 手で撮らないのは、撮り直しのたびに端末・データ・テーマがばらつくから。
//   タブ構成や UI を変えたら同じコマンドで 3 枚まとめて更新できるようにしてある。
//
// - 端末は E2E と同じ iPhone 15 Pro（devices の deviceScaleFactor をそのまま使う）
// - display-mode: standalone を CDP で偽装する。ホーム画面から起動した状態が
//   本来の姿で、ブラウザで開いたときだけ出る「ホーム画面に追加してください」の
//   バナー（calendar.rs の is_standalone 分岐）は README に写すと嘘になる
// - 記録は localStorage へ直接注入する（E2E の seedPastLogs と同じ手口）。
//   空のアプリを撮ってもカレンダーのドットもグラフも出ない

import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { join } from 'node:path';
import { chromium, devices } from '@playwright/test';

const REPO_ROOT = join(fileURLToPath(new URL('.', import.meta.url)), '..');
const PORT = Number(process.env.SHOT_PORT || 4274);
const BASE = `http://localhost:${PORT}/`;

/**
 * 撮る画面。ファイル名の連番は README の並び順。
 *
 * `center` は「この要素が画面の中央に来るまでスクロールしてから撮る」指定。
 * 記録タブだけ指定があるのは、先頭で撮るとカレンダーしか入らず、この画面の要である
 * 「カレンダーと入力欄が縦に並んで 1 画面」（adr/ux/record-tab-calendar-with-day-editor.md）が写らないため。
 *
 * ★ 設定タブは**トップ（節の一覧）をそのまま撮る**。タップして最初に出るのがこれで、
 *   各節の件数まで写る（adr/ux/settings-as-a-list-of-sections.md）。かつては部位を 1 つ
 *   開いた絵にしていたが、その一覧は節の中へ移ったので、開いた絵は「設定」の代表では
 *   なくなった。中身は README の本文が説明する。
 */
const SHOTS = [
  { file: '1-record.png', testid: 'tab-record', screen: 'screen-record', center: 'today-date' },
  { file: '2-progress.png', testid: 'tab-progress', screen: 'screen-progress' },
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
 * 撮影用のトレーニングメニュー（adr/ux/start-from-a-saved-routine.md）。
 *
 * ★ 置かないと設定タブに「まだありません」の 1 行しか写らず、この画面の目玉が
 *   README から読めない。上の SEED と同じ分割（胸 / 背中）で 2 本だけ置く。
 *
 * ★ ID は予約領域（1024 未満）の外に固定で書く。撮り直しても同じ絵になるように
 *   するためで、`IdGen` を通さないのはここが撮影用の直接注入だから（実アプリの
 *   採番経路ではない）。12 文字の base32 でないと `core::migrate` が弾く。
 */
const ROUTINES = [
  { id: '00000000zzz1', name: '胸の日', names: ['ベンチプレス', 'ダンベルプレス', 'チェストフライ'] },
  { id: '00000000zzz2', name: '背中の日', names: ['懸垂', 'ラットプルダウン', 'デッドリフト'] },
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
  env: { ...process.env, PORT: String(PORT), E2E_BASE: '/' },
  stdio: 'inherit',
});
// 例外で落ちてもサーバを残さない（孤児プロセスが次のビルドのポートを塞ぐ）
const stopServer = () => server.kill();
process.on('exit', stopServer);
process.on('SIGINT', () => process.exit(130));

let browser;
try {
  await waitForServer(BASE);
  browser = await chromium.launch();
  // ★ locale を固定する。既定の en-US だとアプリが英語 UI + 英語プリセットで起動し、
  //   下の SEED が日本語の種目名で引いているので**撮影自体が落ちる**
  //   （落ちなくても README のスクショが日本語版と食い違う）
  const context = await browser.newContext({
    ...devices['iPhone 15 Pro'],
    locale: 'ja-JP',
  });

  // ★ CDP の Emulation.setEmulatedMedia に display-mode を渡しても
  //   matchMedia('(display-mode: standalone)') は false のままだった。
  //   代わりにその問い合わせだけを常に真になるクエリ（'all'）へ差し替える。
  //   本物の MediaQueryList を返すので matches 以外の面も壊れない
  await context.addInitScript(() => {
    const orig = window.matchMedia.bind(window);
    window.matchMedia = (q) => (String(q).includes('display-mode: standalone') ? orig('all') : orig(q));
  });

  const page = await context.newPage();
  await page.goto(BASE);
  // プリセット投入 (App の Effect) が走った後でないと exercises が空
  await page.getByTestId('screen-record').waitFor({ state: 'visible' });

  // 400ms debounce の保存を先に確定させてから読み書きする
  await page.evaluate(() => {
    Object.defineProperty(document, 'hidden', { value: true, configurable: true });
    document.dispatchEvent(new Event('visibilitychange', { bubbles: true }));
  });

  await page.evaluate(({ seed, weights, routines }) => {
    // ★ storage.rs の KEY と一致していること。schema 世代ごとにキーを切る運用
    //   （adr/storage/storage-key-per-schema-generation.md）なので、ここが古いと getItem が null を返して黙って落ちる
    const KEY = 'fitness-memo/v3';
    const db = JSON.parse(localStorage.getItem(KEY));
    // Local::now().date_naive() と揃えるため UTC ではなくローカル日付で組み立てる
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
  }, { seed: SEED, weights: BODY_WEIGHTS, routines: ROUTINES });

  await page.reload();

  // ★ 偽装が効いていないと「ホーム画面に追加してください」バナーが写り込む。
  //   撮ってから気づくと差し替えが面倒なので、撮る前に落とす
  const standalone = await page.evaluate(
    () => window.matchMedia('(display-mode: standalone)').matches,
  );
  if (!standalone) throw new Error('display-mode: standalone の偽装が効いていない');

  for (const { file, testid, screen, center } of SHOTS) {
    await page.getByTestId(testid).click();
    await page.getByTestId(screen).waitFor({ state: 'visible' });
    // タブを切り替えてもスクロール位置は持ち越されるので、毎回先頭へ戻してから決める
    await page.evaluate(() => window.scrollTo(0, 0));
    if (center) {
      await page
        .getByTestId(center)
        .first()
        .evaluate((el) => el.scrollIntoView({ block: 'center', behavior: 'instant' }));
    }
    // 折れ線とスクロールの落ち着きに 1 フレーム以上待つ
    await page.waitForTimeout(300);
    const path = join(REPO_ROOT, 'assets', file);
    await page.screenshot({ path });
    console.log(`撮影: assets/${file}`);
  }
} finally {
  await browser?.close();
  stopServer();
}
