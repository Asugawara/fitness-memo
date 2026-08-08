import { test, expect } from '@playwright/test';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

// Playwright の WebKit は Service Worker を公式サポートしていない
// ("Service workers are only supported on Chromium-based browsers")。
// SW の activated 判定とオフライン起動は Service Worker API に直接依存するため
// Chromium 系（chromium / Pixel 7）限定で検証し、webkit（iPhone 15 Pro）では
// test.skip する。破損 JSON の退避キー検証は localStorage と DOM だけで SW を
// 使わないため、重い側に WebKit を入れている本来の目的（iOS Safari 特有の
// localStorage 挙動差分を踏むこと）に合わせて iPhone 15 Pro でも実行する。

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
const BASE = process.env.E2E_BASE || '/';

function normalizeBase(base) {
  let b = base.startsWith('/') ? base : `/${base}`;
  if (!b.endsWith('/')) b += '/';
  return b;
}

function skipOnWebkit(browserName) {
  test.skip(browserName === 'webkit', 'WebKit は Playwright で Service Worker 未対応');
}

/** dist/ を配信する static-server.mjs を専用ポートで起動し、応答するまで待つ。 */
async function startDedicatedServer(port) {
  const proc = spawn(process.execPath, ['scripts/static-server.mjs'], {
    cwd: REPO_ROOT,
    env: { ...process.env, PORT: String(port), E2E_BASE: BASE },
    stdio: ['ignore', 'ignore', 'pipe'],
  });
  let stderr = '';
  proc.stderr.on('data', (chunk) => {
    stderr += chunk;
  });

  const deadline = Date.now() + 5000;
  while (Date.now() < deadline) {
    if (proc.exitCode !== null) {
      throw new Error(`dedicated static-server (port ${port}) が起動前に終了しました:\n${stderr}`);
    }
    try {
      await fetch(`http://localhost:${port}/`);
      return proc;
    } catch {
      await new Promise((r) => setTimeout(r, 100));
    }
  }
  throw new Error(`dedicated static-server (port ${port}) が応答しませんでした:\n${stderr}`);
}

/** navigator.serviceWorker.ready を待ち、activated への遷移まで見届ける。 */
async function waitForSwActivated(page) {
  return page.evaluate(async () => {
    const reg = await navigator.serviceWorker.ready;
    if (reg.active?.state === 'activated') return 'activated';
    return new Promise((resolve) => {
      reg.active.addEventListener('statechange', () => {
        if (reg.active.state === 'activated') resolve('activated');
      });
    });
  });
}

/** hidden への visibilitychange を発火させ、pending の debounce 保存を即時 flush する。 */
async function flushToStorage(page) {
  await page.getByTestId('screen-record').waitFor({ state: 'visible' });
  await page.evaluate(() => {
    Object.defineProperty(document, 'hidden', { value: true, configurable: true });
    document.dispatchEvent(new Event('visibilitychange', { bubbles: true }));
  });
}

async function addBenchPress(page) {
  await page.getByTestId('add-exercise').click();
  await page
    .getByTestId('add-sheet')
    .getByTestId('pick-exercise')
    .filter({ hasText: /^ベンチプレス$/ })
    .click();
  return page.getByTestId('exercise-card');
}

test('manifest が取得でき display=standalone かつ id がある', async ({ page }) => {
  await page.goto('./');

  const href = await page.locator('link[rel="manifest"]').getAttribute('href');
  const res = await page.request.get(new URL(href, page.url()).toString());
  expect(res.ok()).toBeTruthy();

  const manifest = await res.json();
  expect(manifest.display).toBe('standalone');
  expect(manifest.id).toBeTruthy();
});

test('ボトムタブが viewport 内にあり、横スクロールが発生しない', async ({ page }) => {
  // この検証はレイアウト崩れの検知であって、env(safe-area-inset-*) の実効果は
  // Chromium でも Playwright の WebKit でも常に 0px を返すため確認できない（実機でのみ確認可能）
  await page.goto('./');

  const overflowsHorizontally = await page.evaluate(
    () => document.documentElement.scrollWidth > document.documentElement.clientWidth,
  );
  expect(overflowsHorizontally).toBe(false);

  const viewport = page.viewportSize();
  const box = await page.getByTestId('bottom-tabs').boundingBox();
  expect(box).not.toBeNull();
  expect(box.x).toBeGreaterThanOrEqual(0);
  expect(box.y + box.height).toBeLessThanOrEqual(viewport.height + 1);
  expect(box.x + box.width).toBeLessThanOrEqual(viewport.width + 1);
});

test('SW が activated になる', async ({ page, browserName }) => {
  skipOnWebkit(browserName);

  await page.goto('./');
  const state = await waitForSwActivated(page);
  expect(state).toBe('activated');
});

test('破損した JSON を注入すると退避キーが作られ、復元失敗の通知が出る', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('fitness-memo/v2', '{not valid json');
  });
  await page.goto('./');

  await expect(page.getByTestId('restore-notice')).toContainText('復元できませんでした');
  await expect(page.getByTestId('screen-record')).toBeVisible();

  const backupKeys = await page.evaluate(() =>
    Object.keys(localStorage).filter((k) => k.startsWith('fitness-memo/v2.bak-')),
  );
  expect(backupKeys.length).toBeGreaterThan(0);
});

// ★ 保存キーは schema 非互換の変更のたびに切る（storage.rs のモジュールコメント参照）。
//   旧版は新 JSON を読めない（消したフィールドが必須のまま）ので、キーを共有したまま
//   出すとロールバック時に「記録が全消し」に見える。ここで検証するのは 2 点:
//   1. v1 しか無い端末で起動すると、その記録がそのまま見えること（引き継ぎ）
//   2. 引き継いだ後も **v1 が消えていないこと**（旧版へ戻ったときの退路）
test('旧キー v1 の記録を引き継いで v2 に書き、v1 は消さない', async ({ page }) => {
  // kind 付き（schema 1）の旧形式。新モデルには kind フィールドが無いが、
  // serde は未知フィールドを無視するので読める
  await page.addInitScript(() => {
    localStorage.setItem(
      'fitness-memo/v1',
      JSON.stringify({
        schema: 1,
        next_id: 100,
        groups: [{ id: 1, name: '胸', color: '#e0524a', order: 0 }],
        exercises: [
          { id: 10, name: 'レガシーベンチ', group_id: 1, kind: 'Weighted', order: 0 },
        ],
        sessions: {
          '2020-01-02': { logs: [{ exercise_id: 10, sets: [{ weight: 60, reps: 10 }], at: null }] },
        },
      }),
    );
  });
  await page.goto('./');
  await expect(page.getByTestId('screen-record')).toBeVisible();
  // 破損扱いになっていない（プリセットへのフォールバックが起きていない）
  await expect(page.getByTestId('restore-notice')).toHaveCount(0);

  const keys = await page.evaluate(() => ({
    v1: localStorage.getItem('fitness-memo/v1'),
    v2: localStorage.getItem('fitness-memo/v2'),
  }));

  expect(keys.v1, 'v1 は読み取り専用で残す（旧版へ戻ったときの退路）').not.toBeNull();
  expect(keys.v2, 'v2 へ書き写されている').not.toBeNull();

  const migrated = JSON.parse(keys.v2);
  expect(migrated.exercises.map((e) => e.name)).toContain('レガシーベンチ');
  expect(migrated.sessions['2020-01-02'].logs[0].sets).toEqual([{ weight: 60, reps: 10 }]);
  // 旧形式のプリセットで上書きされていない（引き継ぎであって初期化ではない）
  expect(migrated.groups.map((g) => g.name)).toEqual(['胸']);
});

test('オフラインでも起動し記録が読める（SW の navigate 分岐の検証）', async (
  { page, context, browserName },
  testInfo,
) => {
  skipOnWebkit(browserName);

  // ★ precache のキーは "./index.html" であって "./"（ディレクトリURL）ではない。
  // ここでディレクトリURLへナビゲートするのが最重要: sw.js の navigate 分岐が
  // 無いとオフライン起動だけが例外経路頼みになり、オンラインでは気づけない
  //
  // ポートは 4600 起点（e2e/harness.spec.mjs は 4180 起点で workerInfo.parallelIndex
  // ごとに2つ使うので、ワーカー数が増えても重ならないよう十分離す）
  const port = 4600 + testInfo.parallelIndex;
  const base = normalizeBase(BASE);
  const url = `http://localhost:${port}${base}`;
  const server = await startDedicatedServer(port);

  // ★ 途中で assertion が失敗しても専用サーバーを確実に kill する。
  // finally が無いと port を握ったまま孤児プロセス化し、以後の実行を邪魔する
  try {
    await page.goto(url);
    await waitForSwActivated(page);

    const card = await addBenchPress(page);
    const row0 = card.getByTestId('set-row').nth(0);
    await row0.getByTestId('set-weight').fill('60');
    await row0.getByTestId('set-reps').fill('10');
    await flushToStorage(page);

    // setOffline に加えて実サーバー自体も落とす。setOffline は SW 発のリクエストに
    // 効かない既知の問題があるため、両方を併用してオフラインを再現する
    await context.setOffline(true);
    server.kill();

    const response = await page.goto(url, { waitUntil: 'load' });
    expect(response?.fromServiceWorker()).toBe(true);

    await expect(page.getByTestId('exercise-card')).toHaveCount(1);
    await expect(page.getByTestId('today-metric')).toHaveText('600');

    await context.setOffline(false);
  } finally {
    server.kill();
  }
});
