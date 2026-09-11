// 設定タブの使い方マニュアル節と、記録タブの初回手掛かり（ManualHint）。
//
// `smoke.spec.mjs` の機能別分割（history / i18n / progress / pins / interval）に
// 従い新規ファイルにする。共有ヘルパーモジュールは無いので、要るものはここに
// 自前で持つ（既存 spec の作法。e2e/pwa.spec.mjs の comment 参照）。
//
// testid は `src/views/manual.rs` を正とする。章ごとの区別は testid では付いて
// いない（`man-toggle` / `man-group` / `man-body` はどの章でも同じ文字列）ので、
// 各章の `<section class="man-group">` に振ってある `id="man-<chapter id>"`
// （`src/manual.rs` の `Chapter::id`）で章を特定する。
import { test, expect } from '@playwright/test';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
const BASE = process.env.E2E_BASE || '/';

// 図を持つ章（計画「3. 章立て」）。無い 5 章（progress-target / labels / reorder /
// backup / whats-new）のうち reorder / backup はテスト 8 が個別に見る。
const FIG_CHAPTERS = [
  'chart-readout',
  'copy-last',
  'exercise-memo',
  'drop-sets',
  'empty-day',
  'accordions',
];

function normalizeBase(base) {
  let b = base.startsWith('/') ? base : `/${base}`;
  if (!b.endsWith('/')) b += '/';
  return b;
}

function skipOnWebkit(browserName) {
  test.skip(browserName === 'webkit', 'WebKit は Playwright で Service Worker 未対応');
}

async function blurActive(page) {
  await page.evaluate(() => {
    if (document.activeElement instanceof HTMLElement) document.activeElement.blur();
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

/** 設定タブ → 使い方マニュアル節。別の節に居ても一度トップへ戻ってから入る。 */
async function openManual(page) {
  await blurActive(page);
  await page.getByTestId('tab-settings').click();
  const back = page.getByTestId('settings-back');
  if (await back.isVisible()) await back.click();
  await page.getByTestId('settings-row-manual').click();
}

/** 指定した章の `<section class="man-group">` を返す（`src/manual.rs` の `Chapter::id`）。 */
function chapterGroup(page, id) {
  return page.locator(`#man-${id}`);
}

/** 指定した章を開く。既に開いていれば何もしない（押すと閉じるトグルなので）。 */
async function openChapter(page, id) {
  const group = chapterGroup(page, id);
  const toggle = group.getByTestId('man-toggle');
  if ((await toggle.getAttribute('aria-expanded')) !== 'true') await toggle.click();
  await expect(group.getByTestId('man-body')).toBeVisible();
  return group;
}

/** 設定 > 言語 のサブページを開く。 */
async function openLanguage(page) {
  await blurActive(page);
  await page.getByTestId('tab-settings').click();
  const back = page.getByTestId('settings-back');
  if (await back.isVisible()) await back.click();
  await page.getByTestId('settings-row-language').click();
}

/** 言語ボタン（endonym で引かず data-lang で引く。e2e/i18n.spec.mjs と同じ作法）。 */
function langButton(page, tag) {
  return page.getByTestId('lang-btn').and(page.locator(`[data-lang="${tag}"]`));
}

/**
 * standalone 起動をエミュレートする（e2e/pwa.spec.mjs の `fakeStandalone` と同じ）。
 * install バナーの表示条件 `!is_standalone() && storage_may_split()` を偽にして、
 * `chromium` project（Desktop Chrome UA・非 standalone なので既定では真になる）でも
 * `ManualHint` 側の条件（install バナーが出ていない）を作れるようにする。
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

/**
 * dist/ を配信する static-server.mjs を専用ポートで起動する（テスト 10 専用）。
 *
 * ★ 共有 webServer（playwright.config.mjs）を使わないのは、`context.setOffline(true)`
 *   だけでは SW 自身が発する fetch を止められない既知の問題があり
 *   （e2e/pwa.spec.mjs の「オフラインでも起動し記録が読める」の先例）、
 *   実サーバーを本当に kill する必要があるため。共有サーバーを kill すると他の
 *   テストを巻き添えにする。
 */
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

/** `<img>` が読み込み終わる（成功 / 失敗いずれか）まで待つ。 */
async function waitForImageSettled(locator) {
  await locator.evaluate(
    (el) =>
      new Promise((resolve) => {
        if (el.complete) return resolve();
        el.addEventListener('load', resolve, { once: true });
        el.addEventListener('error', resolve, { once: true });
      }),
  );
}

/**
 * WebP 1 枚の宣言寸法を読む。`src/manual.rs` の `webp_size` と同じロジック
 * （チャンク走査）を JS に移植したもの。出力は常に拡張形式
 * `VP8X(10) + ICCP(456) + VP8L`/`"VP8 "` になるので、offset 固定では読めない
 * （byte 20 は VP8X の flags で VP8L の signature ではない）。
 */
function webpSize(buf) {
  if (buf.length < 12 || buf.toString('latin1', 0, 4) !== 'RIFF' || buf.toString('latin1', 8, 12) !== 'WEBP') {
    return null;
  }
  let pos = 12;
  while (pos + 8 <= buf.length) {
    const fourcc = buf.toString('latin1', pos, pos + 4);
    const size = buf.readUInt32LE(pos + 4);
    const dataStart = pos + 8;
    const dataEnd = dataStart + size;
    if (dataEnd > buf.length) return null;

    if (fourcc === 'VP8X' && size >= 10) {
      return {
        width: buf.readUIntLE(dataStart + 4, 3) + 1,
        height: buf.readUIntLE(dataStart + 7, 3) + 1,
      };
    }
    if (fourcc === 'VP8L' && size >= 5 && buf[dataStart] === 0x2f) {
      const bits = buf.readUInt32LE(dataStart + 1);
      return {
        width: (bits & 0x3fff) + 1,
        height: ((bits >>> 14) & 0x3fff) + 1,
      };
    }
    if (
      fourcc === 'VP8 ' &&
      size >= 10 &&
      buf[dataStart + 3] === 0x9d &&
      buf[dataStart + 4] === 0x01 &&
      buf[dataStart + 5] === 0x2a
    ) {
      return {
        width: buf.readUInt16LE(dataStart + 6) & 0x3fff,
        height: buf.readUInt16LE(dataStart + 8) & 0x3fff,
      };
    }

    // チャンクは偶数境界にパディングされる
    pos = dataEnd + (size % 2);
  }
  return null;
}

// ★ 先頭 `/` は禁止。baseURL がサブパス（/fitness-memo/）を持つとき、絶対パス参照は
//   そのベースを破棄してしまう。ファイル共通の `beforeEach` にしないのは、9 の
//   `fakeStandalone` が「addInitScript の後に goto」の順を必要とするため
//   （先に共通 goto を挟むと二重ナビゲーションになる）。

// 1. 導線と h1 ─────────────────────────────────────────────────────────────

test('導線: 設定 → 使い方 で h1 がマニュアル名になり、戻ると設定に戻る', async ({ page }) => {
  await page.goto('./');
  await openManual(page);

  await expect(page.locator('main h1')).toHaveCount(1);
  await expect(page.locator('main h1')).toHaveText('活用方法');

  await page.getByTestId('settings-back').click();
  await expect(page.locator('main h1')).toHaveCount(1);
  await expect(page.locator('main h1')).toHaveText('設定');
});

// 2. アコーディオン ─────────────────────────────────────────────────────────

test('アコーディオン: 既定で全部閉じ、1 つ開くと前が閉じる', async ({ page }) => {
  await page.goto('./');
  await openManual(page);

  const toggles = page.getByTestId('man-toggle');
  await expect(toggles).toHaveCount(12);
  await expect(page.getByTestId('man-body')).toHaveCount(0);
  const states = await toggles.evaluateAll((els) => els.map((el) => el.getAttribute('aria-expanded')));
  expect(states).toEqual(Array(12).fill('false'));

  await openChapter(page, 'chart-readout');
  await expect(page.getByTestId('man-body')).toHaveCount(1);

  // 別の章を開くと前が閉じる
  await openChapter(page, 'accordions');
  await expect(page.getByTestId('man-body')).toHaveCount(1);
  await expect(chapterGroup(page, 'accordions').getByTestId('man-toggle')).toHaveAttribute(
    'aria-expanded',
    'true',
  );
  await expect(chapterGroup(page, 'chart-readout').getByTestId('man-toggle')).toHaveAttribute(
    'aria-expanded',
    'false',
  );
  await expect(page.locator('[data-testid="man-toggle"][aria-expanded="true"]')).toHaveCount(1);
});

// 3. 図が全部読める ─────────────────────────────────────────────────────────

test('図が全部読める: 図を持つ章を順に開き、img が全部 200 / image/webp で読める', async ({ page }) => {
  await page.goto('./');
  await openManual(page);

  // ★ アコーディオンは 1 章しか開けないので、開いている `man-fig` は毎回 1 個だけ。
  //   URL はチャプターごとにその場で集める（ループの後でまとめて集めると、
  //   その時点で開いている最後の章の 1 件しか拾えない）
  const srcs = [];
  for (const id of FIG_CHAPTERS) {
    await openChapter(page, id);
    const img = chapterGroup(page, id).getByTestId('man-fig');
    await expect(img).toBeVisible();
    await waitForImageSettled(img);
    const naturalWidth = await img.evaluate((el) => el.naturalWidth);
    expect(naturalWidth, `${id} の図が読めていない`).toBeGreaterThan(0);
    srcs.push(await img.evaluate((el) => el.src));
  }

  // URL の一覧はハードコードせず、画面から集めたものをそのまま取得して確かめる
  expect(srcs).toHaveLength(FIG_CHAPTERS.length);
  expect(new Set(srcs).size, '同じ URL が重複している').toBe(srcs.length);
  for (const src of srcs) {
    const res = await page.request.get(src);
    expect(res.ok(), `${src} が配信物に無い`).toBeTruthy();
    expect(res.headers()['content-type']).toBe('image/webp');
  }
});

// 4. 宣言寸法 = 実ファイル ──────────────────────────────────────────────────

test('宣言寸法 = 実ファイル: img の width/height と WebP 実測が一致する', async ({ page }) => {
  await page.goto('./');
  await openManual(page);

  for (const id of FIG_CHAPTERS) {
    await openChapter(page, id);
    const img = chapterGroup(page, id).getByTestId('man-fig');
    await expect(img).toBeVisible();

    const [src, declaredWidth, declaredHeight] = await Promise.all([
      img.evaluate((el) => el.src),
      img.evaluate((el) => Number(el.getAttribute('width'))),
      img.evaluate((el) => Number(el.getAttribute('height'))),
    ]);

    const res = await page.request.get(src);
    expect(res.ok(), `${id} の WebP が配信物に無い`).toBeTruthy();
    const actual = webpSize(await res.body());
    expect(actual, `${id} の WebP をチャンク走査で解けなかった`).not.toBeNull();
    expect({ width: actual.width, height: actual.height }, id).toEqual({
      width: declaredWidth,
      height: declaredHeight,
    });
  }
});

// 5. SW シェルの除外 ────────────────────────────────────────────────────────

test('SW のシェルに manual/ が入っていない', async ({ page }) => {
  await page.goto('./');
  const sw = await (await page.request.get('./sw.js')).text();

  // まず置換自体が走っていること（プレースホルダが残っていたら以下の assert が空振りする）
  expect(sw).not.toContain('__BUILD_ID__');
  expect(sw).not.toContain('__SHELL__');
  // SHELL が空でないこと（空配列なら manual/ が無いのは当たり前で、何も検証できていない）
  expect(sw).toContain('"./manifest.webmanifest"');

  expect(sw, 'マニュアルを開かない人のオフラインシェルに図を載せてはならない').not.toContain('manual/');
});

// 6. 言語の出し分け ─────────────────────────────────────────────────────────

test('言語の出し分け: en に切り替えると manual/en/、ja に戻すと manual/ja/', async ({ page }) => {
  const id = FIG_CHAPTERS[0];

  await page.goto('./');
  await openManual(page);
  await openChapter(page, id);
  await expect(chapterGroup(page, id).getByTestId('man-fig')).toHaveAttribute('src', /manual\/ja\//);

  // config が locale: 'ja-JP' で起動するので、英語への切り替えは必ず UI 操作で行う
  // （ja ボタンは同値ガードで効かないため、ja → en → ja と実際に押して確かめる）
  await openLanguage(page);
  await langButton(page, 'en').click();

  await page.getByTestId('settings-back').click();
  await page.getByTestId('settings-row-manual').click();
  await openChapter(page, id);
  await expect(chapterGroup(page, id).getByTestId('man-fig')).toHaveAttribute('src', /manual\/en\//);

  await openLanguage(page);
  await langButton(page, 'ja').click();

  await page.getByTestId('settings-back').click();
  await page.getByTestId('settings-row-manual').click();
  await openChapter(page, id);
  await expect(chapterGroup(page, id).getByTestId('man-fig')).toHaveAttribute('src', /manual\/ja\//);
});

// 7. タブ往復で章が開いたまま ───────────────────────────────────────────────

test('タブ往復で開いた章が保たれ、可視域に入っている', async ({ page }) => {
  await page.goto('./');
  await openManual(page);
  await openChapter(page, 'backup');
  await expect(page.getByTestId('man-body')).toHaveCount(1);

  await page.getByTestId('tab-record').click();
  await page.getByTestId('tab-settings').click();

  await expect(page.locator('main h1')).toHaveText('活用方法');
  await expect(page.getByTestId('man-body')).toHaveCount(1);
  const group = chapterGroup(page, 'backup');
  await expect(group.getByTestId('man-toggle')).toHaveAttribute('aria-expanded', 'true');

  const box = await group.boundingBox();
  const viewport = page.viewportSize();
  expect(box).not.toBeNull();
  expect(box.y).toBeGreaterThanOrEqual(0);
  expect(box.y).toBeLessThanOrEqual(viewport.height);
});

// 8. 図が無い章も本文がある ─────────────────────────────────────────────────

test('図が無い章にも本文がある', async ({ page }) => {
  await page.goto('./');
  await openManual(page);

  for (const id of ['reorder', 'backup']) {
    const group = await openChapter(page, id);
    const body = group.getByTestId('man-body');
    const items = body.locator('li');
    expect(await items.count(), `${id} に本文が無い`).toBeGreaterThan(0);
    await expect(body.getByTestId('man-fig')).toHaveCount(0);
  }
});

// 9. 記録タブの手掛かり（ManualHint） ───────────────────────────────────────

test.describe('記録タブの手掛かり（ManualHint）', () => {
  test('install バナーと同時には出ない（既定の環境）', async ({ page }) => {
    // ★ project 名では分岐しない。install バナー側の条件（!is_standalone() &&
    //   storage_may_split()）は UA に依存し、`storage_may_split()` は UA に
    //   "Android" を含むと偽になる（src/views/mod.rs）。だから chromium /
    //   iPhone 15 Pro では install バナーが出て ManualHint が出ないが、
    //   Pixel 7（Android UA）では逆に install バナーが出ず ManualHint が出る。
    //   見たいのは「2 つが同時には出ない」という排他そのものなので、
    //   どちらが出るかで分岐して確かめる（project 名の決め打ちで一方だけを
    //   期待すると、UA 次第で他方が出ている project で必ず落ちる）
    //
    // ★ 軽い側（pre-commit）は chromium と harness だけを走らせるので、
    //   Pixel 7 だけで踏むこの手の壊れ方は release.sh の重い側で初めて出る
    await page.goto('./');
    // ★ `isVisible()` は待たない即時チェックなので、goto 直後にそのまま呼ぶと
    //   Calendar がまだ描画し終わる前を拾って両方とも「無い」判定になりうる
    //   （実測: chromium / iPhone 15 Pro で再現、Pixel 7 は再現しないこともある —
    //   スケジューリング次第で化けるレースなので project では見分けられない）。
    //   `screen-record` は install-hint / manual-hint と同じ Calendar の
    //   view! で同期に描かれるので、これが見えた時点でどちらかは確定している
    await expect(page.getByTestId('screen-record')).toBeVisible();
    const install = page.getByTestId('install-hint');
    const hint = page.getByTestId('manual-hint');
    if (await install.isVisible()) {
      await expect(hint).toHaveCount(0);
    } else {
      await expect(hint).toBeVisible();
    }
  });

  test.describe('install バナーが出ない環境（standalone を偽装）', () => {
    test.beforeEach(async ({ page }) => {
      await fakeStandalone(page);
      await page.goto('./');
    });

    test('初回は出て、可視域に入っている', async ({ page }) => {
      await expect(page.getByTestId('install-hint')).toHaveCount(0);
      const hint = page.getByTestId('manual-hint');
      await expect(hint).toBeVisible();

      const box = await hint.boundingBox();
      const viewport = page.viewportSize();
      expect(box.y).toBeGreaterThanOrEqual(0);
      expect(box.y + box.height).toBeLessThanOrEqual(viewport.height);
    });

    test('✕ で消え、リロードしても復活しない', async ({ page }) => {
      await expect(page.getByTestId('manual-hint')).toBeVisible();
      await page.getByTestId('manual-hint-dismiss').click();
      await expect(page.getByTestId('manual-hint')).toHaveCount(0);

      await page.reload();
      await expect(page.getByTestId('screen-record')).toBeVisible();
      await expect(page.getByTestId('manual-hint')).toHaveCount(0);
    });

    test('記録は UI 専用キーに入り、Db のキーには混ざらない', async ({ page }) => {
      await page.getByTestId('manual-hint-dismiss').click();
      await expect(page.getByTestId('manual-hint')).toHaveCount(0);
      await flushToStorage(page);

      const stored = await page.evaluate(() => ({
        db: localStorage.getItem('fitness-memo/v3'),
        ui: localStorage.getItem('fitness-memo/ui/v1'),
      }));
      expect(stored.ui).toContain('manual_hint_dismissed');
      // Db が書かれていること自体も確認する（null だと次の assert が空振りする）
      expect(stored.db).toContain('"schema"');
      expect(stored.db).not.toContain('manual_hint');
    });

    test('CTA を押すと設定タブのマニュアル節へ飛び、手掛かりが消える', async ({ page }) => {
      await page.getByTestId('manual-hint-cta').click();

      await expect(page.getByTestId('tab-settings')).toHaveClass(/active/);
      await expect(page.locator('main h1')).toHaveCount(1);
      await expect(page.locator('main h1')).toHaveText('活用方法');

      await page.getByTestId('tab-record').click();
      await expect(page.getByTestId('manual-hint')).toHaveCount(0);
    });
  });
});

// 10. オフラインで本文が出て、図の欠けが伝わる ──────────────────────────────

test('オフラインで開くと本文は読めるが図は欠け、断りが伝わる', async ({ page, context, browserName }, testInfo) => {
  skipOnWebkit(browserName);

  // 共有 webServer を kill できないので専用サーバーを使う（上の startDedicatedServer 参照）
  const port = 4650 + testInfo.parallelIndex;
  const url = `http://localhost:${port}${normalizeBase(BASE)}`;
  const server = await startDedicatedServer(port);

  try {
    await page.goto(url);
    await waitForSwActivated(page);

    // ★ setOffline に加えて実サーバー自体も落とす。setOffline は SW 発の fetch に
    //   効かない既知の問題があるため（e2e/pwa.spec.mjs 参照）
    await context.setOffline(true);
    server.kill();

    // シェル（本文・章のコード）はここで初めてオフラインから読み直す。
    // 図はこの前に一度もリクエストしていないので、キャッシュ経由で偶然読める心配が無い
    await page.reload();

    const id = FIG_CHAPTERS[0];
    await openManual(page);
    await openChapter(page, id);

    const fig = chapterGroup(page, id).getByTestId('man-fig');
    await waitForImageSettled(fig);
    expect(await fig.evaluate((el) => el.naturalWidth), '図はシェル外なのでオフラインでは読めないはず').toBe(0);

    await expect(chapterGroup(page, id).getByTestId('man-body').locator('li').first()).toBeVisible();
    await expect(page.getByTestId('man-section')).toContainText('圏外では図が表示されません');

    await context.setOffline(false);
  } finally {
    server.kill();
  }
});
