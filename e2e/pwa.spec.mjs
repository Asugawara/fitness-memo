import { test, expect, devices } from '@playwright/test';
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

  // ★ ホーム画面のアイコン下に出るのは short_name / apple-mobile-web-app-title であって
  //   <title> ではない。4 箇所揃っていないと DOM だけ英語で iPhone は日本語のまま残る
  expect(manifest.name).toBe('fitness-memo');
  expect(manifest.short_name).toBe('fitness-memo');
  await expect(page).toHaveTitle('fitness-memo');
  await expect(page.locator('meta[name="apple-mobile-web-app-title"]')).toHaveAttribute(
    'content',
    'fitness-memo',
  );
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
    localStorage.setItem('fitness-memo/v3', '{not valid json');
  });
  await page.goto('./');

  await expect(page.getByTestId('restore-notice')).toContainText('復元できませんでした');
  await expect(page.getByTestId('screen-record')).toBeVisible();

  const backupKeys = await page.evaluate(() =>
    Object.keys(localStorage).filter((k) => k.startsWith('fitness-memo/v3.bak-')),
  );
  expect(backupKeys.length).toBeGreaterThan(0);
});

// ★ 保存キーは schema 非互換の変更のたびに切る（storage.rs のモジュールコメント参照）。
//   旧版は新 JSON を読めない（消したフィールドが必須のまま）ので、キーを共有したまま
//   出すとロールバック時に「記録が全消し」に見える。ここで検証するのは 2 点:
//   1. v1 しか無い端末で起動すると、その記録がそのまま見えること（引き継ぎ）
//   2. 引き継いだ後も **v1 が消えていないこと**（旧版へ戻ったときの退路）
test('旧キー v1 の記録を引き継いで v3 に書き、v1 は消さない', async ({ page }) => {
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
    v3: localStorage.getItem('fitness-memo/v3'),
  }));

  expect(keys.v1, 'v1 は読み取り専用で残す（旧版へ戻ったときの退路）').not.toBeNull();
  expect(keys.v3, 'v3 へ書き写されている').not.toBeNull();

  const migrated = JSON.parse(keys.v3);
  expect(migrated.exercises.map((e) => e.name)).toContain('レガシーベンチ');
  expect(migrated.sessions['2020-01-02'].logs[0].sets).toEqual([{ weight: 60, reps: 10 }]);
  // 旧形式のプリセットで上書きされていない（引き継ぎであって初期化ではない）
  expect(migrated.groups.map((g) => g.name)).toEqual(['胸']);
});

// ★ 旧キーは「全損に対する唯一の退路」（ADR-0034）。現行キーが壊れたときに
//   そこへ降りられなければ、退路を用意した意味が無い。
//   「最初に中身があったキーで打ち切る」実装だと、健全な v2 が残っているのに
//   プリセットが表示され、直後の保存でそれが確定してしまう。
test('v3 が壊れていても健全な v2 があればそこから復元する', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('fitness-memo/v3', '{壊れている');
    localStorage.setItem(
      'fitness-memo/v2',
      JSON.stringify({
        schema: 2,
        next_id: 100,
        groups: [{ id: 1, name: '胸', color: '#e0524a', order: 0 }],
        exercises: [{ id: 10, name: '生き残りベンチ', group_id: 1, order: 0 }],
        sessions: {
          '2020-01-02': { logs: [{ exercise_id: 10, sets: [{ weight: 60, reps: 10 }], at: null }] },
        },
      }),
    );
  });
  await page.goto('./');
  await expect(page.getByTestId('screen-record')).toBeVisible();

  // プリセットに落ちていない = v2 から復元できている
  const db = await page.evaluate(() => JSON.parse(localStorage.getItem('fitness-memo/v3')));
  expect(db.exercises.map((e) => e.name)).toContain('生き残りベンチ');
  expect(db.groups.map((g) => g.name)).toEqual(['胸']);

  // 壊れていた v3 は退避されている（黙って捨てない）
  const backups = await page.evaluate(() =>
    Object.keys(localStorage).filter((k) => k.startsWith('fitness-memo/v3.bak-')),
  );
  expect(backups.length).toBeGreaterThan(0);

  // 何が起きたかを黙らない
  await expect(page.getByTestId('restore-notice')).toContainText('バックアップから復元');
});

// ★ ロールバック中に旧版が旧キーへ書いた記録は、新版へ戻ると現行キーが採用されるので
//   画面から消える。自動マージはしない（同じ日を両方で編集していると正が決まらない）が、
//   **消えていないことは伝える**。無言の欠落が一番たちが悪い。
test('旧世代のほうが新しい記録を持っていると通知が出る', async ({ page }) => {
  await page.addInitScript(() => {
    // v3 は現行形式（ID は 12 文字の文字列）。胸 = 0x10 → "00000000000g"、
    // ベンチプレス = 0x11 → "00000000000h"
    localStorage.setItem(
      'fitness-memo/v3',
      JSON.stringify({
        schema: 3,
        groups: [{ id: '00000000000g', name: '胸', color: '#e0524a', order: 0 }],
        exercises: [
          { id: '00000000000h', name: 'ベンチプレス', group_id: '00000000000g', order: 0 },
        ],
        sessions: {
          '2020-01-02': {
            logs: [{ exercise_id: '00000000000h', sets: [{ weight: 60, reps: 10 }], at: null }],
          },
        },
      }),
    );
    localStorage.setItem(
      'fitness-memo/v2',
      JSON.stringify({
        schema: 2,
        next_id: 100,
        groups: [{ id: 1, name: '胸', color: '#e0524a', order: 0 }],
        exercises: [{ id: 10, name: 'ベンチプレス', group_id: 1, order: 0 }],
        sessions: {
          '2020-03-09': { logs: [{ exercise_id: 10, sets: [{ weight: 60, reps: 10 }], at: null }] },
        },
      }),
    );
  });

  await page.goto('./');
  await expect(page.getByTestId('restore-notice')).toContainText('2020-03-09');

  // 採用しているのは v3 のまま（旧世代で上書きしない）
  const kept = await page.evaluate(() => JSON.parse(localStorage.getItem('fitness-memo/v3')));
  expect(Object.keys(kept.sessions)).toEqual(['2020-01-02']);
});

// ★ v2 → v3 は ID を連番 u32 から乱数の文字列へ張り替える世代。ここで見るのは
//   「ログが指す種目が入れ替わっていないこと」— 連番のままエクスポートを出すと
//   起きる壊れ方（別種目の履歴が混ざる）を、移行そのものがやらないことの検証。
test('旧キー v2 の連番 ID を張り替えて v3 に書き、参照が入れ替わらない', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem(
      'fitness-memo/v2',
      JSON.stringify({
        schema: 2,
        next_id: 100,
        groups: [{ id: 1, name: 'わたしの部位', color: '#e0524a', order: 0 }],
        exercises: [
          { id: 2, name: 'さきの種目', group_id: 1, order: 0 },
          { id: 3, name: 'あとの種目', group_id: 1, order: 1 },
        ],
        sessions: {
          '2020-01-02': {
            logs: [
              { exercise_id: 2, sets: [{ weight: 60, reps: 10 }], at: null },
              { exercise_id: 3, sets: [{ weight: 30, reps: 12 }], at: null },
            ],
          },
        },
      }),
    );
  });
  await page.goto('./');
  await expect(page.getByTestId('screen-record')).toBeVisible();
  await expect(page.getByTestId('restore-notice')).toHaveCount(0);

  const keys = await page.evaluate(() => ({
    v2: localStorage.getItem('fitness-memo/v2'),
    v3: localStorage.getItem('fitness-memo/v3'),
  }));
  expect(keys.v2, 'v2 は消さない（移行直前の状態が正常なまま凍結される）').not.toBeNull();
  expect(keys.v3).not.toBeNull();

  const migrated = JSON.parse(keys.v3);
  expect(migrated.schema).toBe(3);
  // ID は文字列になっている。数値のままだと JSON.parse/stringify の往復で
  // 2^53 超えが丸められ、参照が静かに壊れる
  for (const ex of migrated.exercises) {
    expect(typeof ex.id, `${ex.name} の ID が文字列でない`).toBe('string');
    expect(ex.id).toHaveLength(12);
  }

  // ★ ログが指す種目を**名前**で確かめる。張り替えが一貫していなければここで落ちる
  const nameOf = (id) => migrated.exercises.find((e) => e.id === id)?.name;
  const logs = migrated.sessions['2020-01-02'].logs;
  expect(nameOf(logs[0].exercise_id)).toBe('さきの種目');
  expect(nameOf(logs[1].exercise_id)).toBe('あとの種目');
  // 種目 → 部位の参照も張り替わっている
  const groupIds = new Set(migrated.groups.map((g) => g.id));
  expect(migrated.exercises.every((e) => groupIds.has(e.group_id))).toBe(true);
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

// ── ホーム画面に追加の案内 ───────────────────────────────────────────────────
//
// 表示条件は `views::storage_may_split()`（UA に "Android" を含まない）と
// `views::is_standalone()` の AND。project ごとの UA に依存させると chromium
// （= pre-commit が回す唯一のブラウザ）でどちらのケースを踏むかが暗黙になるので、
// UA は test.use で明示して全 project で同じ分岐を通す。

// ★ 以下の否定テストは `toHaveCount(0)` で「出ない」ことを見るので、testid を改名すると
//   全部が無言で常に通るようになる。実在の担保は同ファイルの肯定テストが持っている:
//   `install-hint` は「バナーは『種目を追加』より下に…」が boundingBox で、
//   `install-hint-open` / `install-hint-dismiss` は各々をクリックするテストが握っている。
//   testid を変えるときは、肯定側が落ちることを確認してから否定側を直すこと。

const UA_IPHONE = devices['iPhone 15 Pro'].userAgent;
const UA_ANDROID = devices['Pixel 7'].userAgent;

// ★ iPadOS 13+ の Safari は既定でこの desktop-class UA を出す（"iPhone" も "iPad" も
//   含まない）。「iOS を当てる」判定にするとストレージ分離が同じく起きる iPad が
//   保護対象から落ちるため、この UA でもバナーが出ることを退行テストとして固定する。
const UA_IPAD_DESKTOP =
  'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15';

/**
 * standalone 起動をエミュレートする。
 *
 * ★ Playwright に display-mode のエミュレーションは無い（`emulateMedia` は colorScheme /
 *   contrast / forcedColors / media / reducedMotion だけ）。wasm-bindgen の生成コードは
 *   `matchMedia` の戻り値を instanceof で検査せず `.matches` を読むだけなので、該当クエリに
 *   限ってプレーンオブジェクトを返せば `is_standalone()` が true になる。全置換にすると
 *   leptos が将来 matchMedia を使い始めたときに巻き込むので、クエリ一致で分岐する。
 */
async function fakeStandalone(page) {
  await page.addInitScript(() => {
    const orig = window.matchMedia.bind(window);
    window.matchMedia = (q) =>
      q === '(display-mode: standalone)'
        ? {
            matches: true,
            media: q,
            onchange: null,
            addEventListener() {},
            removeEventListener() {},
            addListener() {},
            removeListener() {},
            dispatchEvent: () => false,
          }
        : orig(q);
  });
}

test.describe('ストレージが分かれうる環境（iPhone の UA）', () => {
  test.use({ userAgent: UA_IPHONE });

  test('記録タブの警告バナーを押すと手順シートが開く', async ({ page }) => {
    await page.goto('./');
    await page.getByTestId('install-hint-open').click();

    const sheet = page.getByTestId('install-sheet');
    await expect(sheet).toBeVisible();

    // include_str! + inner_html の経路が生きているかの検証。SVG に XML 宣言や DOCTYPE が
    // 混ざると HTML フラグメントパーサが bogus comment にして図が 1 枚も出なくなる
    await expect(sheet.locator('.hlp-fig > svg')).toHaveCount(3);
  });

  test('✕ を押すとバナーが消え、リロードしても復活しない', async ({ page }) => {
    await page.goto('./');
    await expect(page.getByTestId('install-hint')).toBeVisible();

    await page.getByTestId('install-hint-dismiss').click();
    await expect(page.getByTestId('install-hint')).toHaveCount(0);

    await page.reload();
    await expect(page.getByTestId('screen-record')).toBeVisible();
    await expect(page.getByTestId('install-hint')).toHaveCount(0);
  });

  // ADR-0040 が「消しても手順自体は失われない」ことを ✕ を入れる条件にしているので固定する
  test('✕ で消しても種目タブから手順シートを開ける', async ({ page }) => {
    await page.goto('./');
    await page.getByTestId('install-hint-dismiss').click();
    await expect(page.getByTestId('install-hint')).toHaveCount(0);

    await page.getByTestId('tab-menu').click();
    await page.getByTestId('install-help-link').click();

    await expect(page.getByTestId('install-sheet')).toBeVisible();
  });

  // UI のフラグを Db と混ぜないこと（ADR-0014 の「Db の JSON がそのままエクスポート
  // 形式」という前提を守る）。混ぜると export に UI 状態が混入する
  test('✕ の記録は Db のキーではなく UI 専用キーに入る', async ({ page }) => {
    await page.goto('./');
    await page.getByTestId('install-hint-dismiss').click();
    await expect(page.getByTestId('install-hint')).toHaveCount(0);

    // Db の保存は 400ms debounce なので、書かれた状態にしてから中身を見る
    await flushToStorage(page);

    const stored = await page.evaluate(() => ({
      db: localStorage.getItem('fitness-memo/v3'),
      ui: localStorage.getItem('fitness-memo/ui/v1'),
    }));
    expect(stored.ui).toContain('install_hint_dismissed');
    // Db が書かれていること自体も確認する（null だと下の assert が空振りする）
    expect(stored.db).toContain('"schema"');
    expect(stored.db).not.toContain('install_hint');
  });

  test('バナーは「種目を追加」より下にあり、sticky な帯に覆われない', async ({ page }) => {
    await page.goto('./');

    const hint = await page.getByTestId('install-hint').boundingBox();
    const add = await page.locator('.add-wrap').boundingBox();
    expect(hint).not.toBeNull();
    expect(add).not.toBeNull();
    // 「種目を追加」の下端よりバナーの上端が下にある（= 重なっていない）
    expect(hint.y).toBeGreaterThanOrEqual(add.y + add.height);
  });

  test('✕ と backdrop のどちらでもシートが閉じる', async ({ page }) => {
    await page.goto('./');
    const sheet = page.getByTestId('install-sheet');

    await page.getByTestId('install-hint-open').click();
    await page.getByTestId('install-sheet-close').click();
    await expect(sheet).toBeHidden();

    await page.getByTestId('install-hint-open').click();
    // backdrop は inset:0 なので中央はシート本体に覆われている。左上を突く
    await page.getByTestId('install-sheet-backdrop').click({ position: { x: 8, y: 8 } });
    await expect(sheet).toBeHidden();
  });

  test('standalone で起動していればバナーは出ない', async ({ page }) => {
    await fakeStandalone(page);
    await page.goto('./');

    await expect(page.getByTestId('screen-record')).toBeVisible();
    await expect(page.getByTestId('install-hint')).toHaveCount(0);
  });
});

test.describe('iPad の desktop-class UA', () => {
  test.use({ userAgent: UA_IPAD_DESKTOP });

  test('UA から OS を特定できなくてもバナーは出る', async ({ page }) => {
    await page.goto('./');
    await expect(page.getByTestId('install-hint')).toBeVisible();
  });
});

test.describe('ストレージが分かれない環境（Android の UA）', () => {
  test.use({ userAgent: UA_ANDROID });

  test('バナーは出ない（Android はタブと PWA でストレージを共有する）', async ({ page }) => {
    await page.goto('./');

    await expect(page.getByTestId('screen-record')).toBeVisible();
    await expect(page.getByTestId('install-hint')).toHaveCount(0);
  });

  test('種目タブの導線は残り、同じシートが開く', async ({ page }) => {
    await page.goto('./');
    await page.getByTestId('tab-menu').click();
    await page.getByTestId('install-help-link').click();

    await expect(page.getByTestId('install-sheet')).toBeVisible();
  });
});
