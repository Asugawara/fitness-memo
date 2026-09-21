import { test, expect } from '@playwright/test';

// 記録タブ: 種目を外す / メモを開閉 / 外す確認 でカードの高さが変わったときに
// 視界が飛ばないことを見る（adr/ux/viewport-after-card-height-changes.md）。
//
// 主対象は iOS Safari（standalone PWA）。WebKit には scroll anchoring が無いので
// 生のジャンプが出るが、Chromium は overflow-anchor: auto で部分的に隠すため、
// 既定の describe では body{overflow-anchor:none} を注入して症状を再現する。
// 本番の overflow-anchor には触らない（決めた挙動 6）。
//
// e2e はスペックごとにヘルパを複製する規約（e2e/label.spec.mjs:24-25）。
// 以下のヘルパは e2e/smoke.spec.mjs のものと中身が同じでも import しない。

// 主対象は iOS Safari の viewport（iPhone 15 Pro = 393×659）。chromium の既定
// デスクトップ viewport（1280×720 相当）では「フッタが帯の裏に隠れる」
// 「カードが領域より高い」がそもそも起きず V3 系の前提が成立しないため、
// 全テストで同じ小さい viewport に揃える（`iPhone 15 Pro` project の値と同じ）。
test.use({ viewport: { width: 393, height: 659 } });

function exactText(s) {
  return new RegExp(`^${s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}$`);
}

async function blurActive(page) {
  await page.evaluate(() => {
    if (document.activeElement instanceof HTMLElement) document.activeElement.blur();
  });
}

async function flushToStorage(page) {
  await page.getByTestId('screen-record').waitFor({ state: 'visible' });
  await page.evaluate(() => {
    Object.defineProperty(document, 'hidden', { value: true, configurable: true });
    document.dispatchEvent(new Event('visibilitychange', { bubbles: true }));
  });
}

/**
 * localStorage シード + reload で当日カードを作る。UI 操作を挟むと `pick()` の
 * scroll_to_id が動いてしまい、置きたい位置に自前で置けなくなるため。
 * `daysAgo: 0` で当日、reload 後のカード順 = `session.logs` 順。
 */
async function seedPastLogs(page, entries) {
  await flushToStorage(page);
  await page.evaluate((entries) => {
    const KEY = 'fitness-memo/v3';
    const db = JSON.parse(localStorage.getItem(KEY));

    const dateKey = (daysAgo) => {
      const d = new Date();
      d.setDate(d.getDate() - daysAgo);
      const y = d.getFullYear();
      const m = String(d.getMonth() + 1).padStart(2, '0');
      const day = String(d.getDate()).padStart(2, '0');
      return `${y}-${m}-${day}`;
    };

    for (const { daysAgo, exerciseName, sets } of entries) {
      const key = dateKey(daysAgo);
      const session = db.sessions[key] ?? { logs: [], body_weight: null, note: '' };
      const ex = db.exercises.find((e) => e.name === exerciseName);
      if (!ex) throw new Error(`preset exercise not found: ${exerciseName}`);
      session.logs.push({ exercise_id: ex.id, sets, at: null });
      db.sessions[key] = session;
    }
    localStorage.setItem(KEY, JSON.stringify(db));
  }, entries);
  await page.reload();
}

/** n セットぶんの sets 配列（値は座標調整用のダミー）。 */
function sets(n) {
  return Array.from({ length: n }, (_, i) => ({ weight: 40, reps: 10 - (i % 5) }));
}

/** `.add-wrap` の上端（帯の上端）。毎回読み直す — .kb-open で消えると 0 は返らない前提。 */
async function bandTop(page) {
  const box = await page.locator('.add-wrap').boundingBox();
  expect(box, '.add-wrap が画面に無い').not.toBeNull();
  return box.y;
}

// rAF を 2 フレーム待つ。scripts/shots.mjs:417 の settle と同じ書き方をこの spec にも
// 複製する（e2e はスペックごとにヘルパを複製する規約）。スクロール補正は rAF 後に起きる。
const settle = (page) =>
  page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));

async function scrollWindowTo(page, y) {
  await page.evaluate((y) => window.scrollTo(0, Math.round(y)), y);
}

/** カードの上端を画面上端の scroll-margin-top（12px）ぴったりに置く。 */
async function topAlign(page, card) {
  const box = await card.boundingBox();
  await page.evaluate((d) => window.scrollBy(0, d), box.y - 12);
}

test.describe('iOS 再現（overflow-anchor: none を注入）', () => {
  test.beforeEach(async ({ page }) => {
    // ★ addInitScript 時点で document.head は無いので <style> を append すると
    //   黙って失敗する。constructable stylesheet なら document だけで足り、
    //   seedPastLogs の reload 後も効く
    await page.addInitScript(() => {
      const s = new CSSStyleSheet();
      s.replaceSync('body{overflow-anchor:none}');
      document.adoptedStyleSheets = [s];
    });
    await page.goto('./');
    await page.getByTestId('screen-record').waitFor({ state: 'visible' });
  });

  test('V0. 注入が効いている', async ({ page, browserName }) => {
    test.skip(browserName !== 'chromium', 'chromium のみで注入経路を確認する');
    const anchor = await page.evaluate(() => getComputedStyle(document.body).overflowAnchor);
    expect(anchor).toBe('none');
  });

  test('V1. 途中の種目を外すと直下のカードが上端へ来る', async ({ page }) => {
    await seedPastLogs(page, [
      { daysAgo: 0, exerciseName: 'ベンチプレス', sets: sets(3) },
      { daysAgo: 0, exerciseName: 'インクラインベンチプレス', sets: sets(3) },
      { daysAgo: 0, exerciseName: '懸垂', sets: sets(3) },
      { daysAgo: 0, exerciseName: 'ラットプルダウン', sets: sets(3) },
    ]);
    await blurActive(page);

    const cards = page.getByTestId('exercise-card');
    await expect(cards).toHaveCount(4);

    // 2 枚目の上端を y≈200 に置く
    const second = cards.nth(1);
    const before = await second.boundingBox();
    await scrollWindowTo(page, before.y - 200 + (await page.evaluate(() => window.scrollY)));
    await expect
      .poll(async () => (await second.boundingBox()).y, { message: '2 枚目を y≈200 へ置けない' })
      .toBeGreaterThan(190);

    await second.getByTestId('close-card').click();
    const warnBox = second.locator('.warn-box');
    await expect.poll(async () => warnBox.isVisible()).toBe(true);

    const yesBox = await second.getByTestId('close-card-yes').boundingBox();
    const point = { x: yesBox.x + yesBox.width / 2, y: yesBox.y + yesBox.height / 2 };
    await page.getByTestId('close-card-yes').click();

    // 3 枚目（外した位置に来たカード）の上端が 12 ± 1
    const third = cards.nth(1);
    await expect
      .poll(async () => (await third.boundingBox()).y, { message: '直下のカードが上端へ来ない' })
      .toBeGreaterThanOrEqual(11);
    await expect
      .poll(async () => (await third.boundingBox()).y)
      .toBeLessThanOrEqual(13);

    const hit = await page.evaluate(
      ({ x, y }) => document.elementFromPoint(x, y)?.closest('[data-testid]')?.dataset.testid ?? null,
      point,
    );
    expect(['remove-set', 'close-card'], `「はい」の座標に ${hit} が来た`).not.toContain(hit);

    test.info().annotations.push({
      type: 'V1 実測',
      description: `はいの座標 = (${point.x.toFixed(1)}, ${point.y.toFixed(1)}), elementFromPoint testid = ${hit}`,
    });
  });

  test('V2. 末尾の種目を外すとクランプに任せる', async ({ page }) => {
    await seedPastLogs(page, [
      { daysAgo: 0, exerciseName: 'ベンチプレス', sets: sets(3) },
      { daysAgo: 0, exerciseName: 'インクラインベンチプレス', sets: sets(3) },
    ]);
    await blurActive(page);

    const cards = page.getByTestId('exercise-card');
    const last = cards.nth(1);
    const before = await page.evaluate(() => window.scrollY);
    await last.getByTestId('close-card').click();
    await page.getByTestId('close-card-yes').click();

    await expect.poll(async () => page.evaluate(() => document.readyState)).toBe('complete');
    const after = await page.evaluate(() => ({
      y: window.scrollY,
      max: document.documentElement.scrollHeight - window.innerHeight,
    }));
    expect(after.y).toBeLessThanOrEqual(Math.max(before, after.max) + 1);
  });

  test('V3a. 収まっている状態でメモを開いても位置は不変', async ({ page }) => {
    // 1 セット。iPhone 15 Pro の viewport（659px）では 2 セット以上を開くと
    // 高さがそれだけで領域（innerHeight − 12 − reveal）を超える実測になるため、
    // 「収まっている」前提を成立させるには 1 セットにする（V3c/d/e の各コメントも参照）。
    // 2 枚目（スクワット）は本題ではなく、1 枚目を画面上端へ topAlign するためのスクロール
    // の「のりしろ」。1 枚だけだと文書がその日の内容ぶんしか無く、必要な量スクロール
    // できず topAlign がクランプされる
    await seedPastLogs(page, [
      { daysAgo: 0, exerciseName: 'ベンチプレス', sets: sets(1) },
      { daysAgo: 0, exerciseName: 'スクワット', sets: sets(6) },
    ]);
    await blurActive(page);

    const card = page.getByTestId('exercise-card').first();
    await topAlign(page, card);
    const before = await card.boundingBox();
    const scrollBefore = await page.evaluate(() => window.scrollY);

    await card.getByTestId('note-toggle').click();
    // DOM 更新（Leptos の microtask）を確実に待つ。click() の resolve 直後に測ると
    // 開閉の途中の高さを掴むことがある
    await expect(card.getByTestId('exercise-note')).toBeVisible();
    await settle(page);

    const after = await card.boundingBox();
    const scrollAfter = await page.evaluate(() => window.scrollY);
    expect(Math.abs(after.y - before.y)).toBeLessThanOrEqual(1);
    expect(Math.abs(scrollAfter - scrollBefore)).toBeLessThanOrEqual(1);
  });

  test('V3b. フッタが帯の裏に来るとき開くと隠れたぶんだけ見せる', async ({ page }) => {
    await seedPastLogs(page, [{ daysAgo: 0, exerciseName: 'ベンチプレス', sets: sets(2) }]);
    await blurActive(page);

    const card = page.getByTestId('exercise-card').first();
    const foot = card.locator('.card-foot');
    const band = await bandTop(page);
    const footBefore = await foot.boundingBox();
    // フッタ下端が bandTop + 20 に来るよう置く
    const delta = footBefore.y + footBefore.height - (band + 20);
    await page.evaluate((d) => window.scrollBy(0, d), delta);

    await card.getByTestId('note-toggle').click();

    await expect
      .poll(async () => {
        const f = await foot.boundingBox();
        const b = await bandTop(page);
        return f.y + f.height <= b - 7 && f.y >= 0;
      }, { message: 'フッタが帯の裏 / 画面外のまま' })
      .toBe(true);
  });

  test('V3c. 収まる高さで開いて閉じるとフッタの画面位置が不変', async ({ page }) => {
    // 1 セット + topAlign 用の 2 枚目（V3a と同じ理由）
    await seedPastLogs(page, [
      { daysAgo: 0, exerciseName: 'ベンチプレス', sets: sets(1) },
      { daysAgo: 0, exerciseName: 'スクワット', sets: sets(6) },
    ]);
    await blurActive(page);

    const card = page.getByTestId('exercise-card').first();
    await topAlign(page, card);
    const foot = card.locator('.card-foot');
    const toggle = card.getByTestId('note-toggle');

    await toggle.click();
    // DOM 更新を確実に待ってから測る（click() の resolve 直後は開閉の途中のことがある）
    await expect(card.getByTestId('exercise-note')).toBeVisible();
    await settle(page);
    // 前提: 開いた状態で card.y >= 12 かつ card.y + card.height + reveal <= innerHeight
    const reveal = await card.evaluate((el) => parseFloat(getComputedStyle(el).scrollMarginBottom));
    const openBox = await card.boundingBox();
    const innerHeight = await page.evaluate(() => window.innerHeight);
    expect(openBox.y, '前提: カードの上端が画面外').toBeGreaterThanOrEqual(11);
    expect(openBox.y + openBox.height + reveal, '前提: 開いた状態が領域に入らない').toBeLessThanOrEqual(
      innerHeight,
    );

    const footBefore = await foot.boundingBox();
    await toggle.click();

    await expect
      .poll(async () => (await foot.boundingBox()).y, { message: 'フッタの画面位置が動いた' })
      .toBeGreaterThanOrEqual(footBefore.y - 1);
    await expect
      .poll(async () => (await foot.boundingBox()).y)
      .toBeLessThanOrEqual(footBefore.y + 1);
    const cardAfter = await card.boundingBox();
    expect(cardAfter.y).toBeGreaterThanOrEqual(12 - 1);
  });

  test('V3d. 見出しが画面外に残るときは閉じるとカードを収める', async ({ page }) => {
    // 2 枚目（スクワット）は本題ではなく、文書の下に十分なスクロール量を作るための
    // 「のりしろ」。1 枚だけだと文書がその日の内容ぶんしか無く、開いた状態で深く
    // スクロールしたあとカードが縮んだときに、ブラウザの max-scroll クランプが
    // 自然に上端へ寄せてしまい、閉じる側の補正ロジックを試さずに緑になってしまう
    await seedPastLogs(page, [
      { daysAgo: 0, exerciseName: 'ベンチプレス', sets: sets(3) },
      { daysAgo: 0, exerciseName: 'スクワット', sets: sets(6) },
    ]);
    await blurActive(page);

    const card = page.getByTestId('exercise-card').first();
    const foot = card.locator('.card-foot');
    const toggle = card.getByTestId('note-toggle');
    await toggle.click();
    await expect(card.getByTestId('exercise-note')).toBeVisible();
    await settle(page);

    // フッタを y≈100 に置く（見出しは画面外になる）
    const footOpen = await foot.boundingBox();
    await page.evaluate((d) => window.scrollBy(0, d), footOpen.y - 100);
    await expect
      .poll(async () => (await foot.boundingBox()).y)
      .toBeGreaterThan(90);

    await toggle.click();

    await expect
      .poll(async () => (await card.boundingBox()).y, { message: '見出しが画面外のまま' })
      .toBeGreaterThanOrEqual(11);
    await expect.poll(async () => (await card.boundingBox()).y).toBeLessThanOrEqual(13);
    const band = await bandTop(page);
    const footAfter = await foot.boundingBox();
    expect(footAfter.y + footAfter.height).toBeLessThanOrEqual(band - 7);
  });

  test('V3e. 収まらない高さのカードは下端揃えになる（見出しは出ない）', async ({ page }) => {
    // 2 枚目は V3d と同じ理由の「のりしろ」（max-scroll クランプで緑になるのを防ぐ）
    await seedPastLogs(page, [
      { daysAgo: 0, exerciseName: 'ベンチプレス', sets: sets(12) },
      { daysAgo: 0, exerciseName: 'スクワット', sets: sets(6) },
    ]);
    await blurActive(page);

    const card = page.getByTestId('exercise-card').first();
    const foot = card.locator('.card-foot');
    const toggle = card.getByTestId('note-toggle');
    await toggle.click();
    await expect(card.getByTestId('exercise-note')).toBeVisible();
    await settle(page);

    const footOpen = await foot.boundingBox();
    await page.evaluate((d) => window.scrollBy(0, d), footOpen.y - 100);

    await toggle.click();

    // ★ band は poll のたびに読み直す。click 直後は DOM が縮んで scrollY が一時的に
    //   max-scroll へクランプされ、.add-wrap が in-flow 位置（画面上で 16px 高い）に
    //   居る中間状態を通る。ここで 1 回だけ読んで固定すると、rAF の補正が遅れる
    //   並列実行下でその古い値と最終状態のフッタ下端を比べることになり、
    //   f.y + f.height <= band - 7 が永久に成立しない（V3b / V4 と同じ書き方に揃える）
    await expect
      .poll(async () => {
        const f = await foot.boundingBox();
        const band = await bandTop(page);
        return f.y + f.height <= band - 7 && f.y >= 0;
      }, { message: 'フッタが画面外（負）のまま' })
      .toBe(true);
    const cardAfter = await card.boundingBox();
    expect(cardAfter.y, '下端揃えなら見出しは出ない').toBeLessThan(12);
  });

  test('V4. 確認箱が帯の裏に来るとき隠れたぶんだけ見せる', async ({ page }) => {
    // 2 枚目（スクワット）は本題ではなく、確認箱を消してもページ末尾に
    // max-scroll クランプされない高さを作る「のりしろ」（V3d と同じ理由）
    await seedPastLogs(page, [
      { daysAgo: 0, exerciseName: 'ベンチプレス', sets: sets(2) },
      { daysAgo: 0, exerciseName: 'スクワット', sets: sets(6) },
    ]);
    await blurActive(page);

    const card = page.getByTestId('exercise-card').first();
    const foot = card.locator('.card-foot');
    const band = await bandTop(page);
    const footBefore = await foot.boundingBox();
    // フッタを帯の裏（viewport 内・bandTop より下）に置く
    const delta = footBefore.y - (band + 10);
    await page.evaluate((d) => window.scrollBy(0, d), delta);

    // click は帯を素通しするので mouse.click で押す
    const closeBox = await card.getByTestId('close-card').boundingBox();
    await page.mouse.click(closeBox.x + closeBox.width / 2, closeBox.y + closeBox.height / 2);

    const warnBox = card.locator('.warn-box');
    await expect
      .poll(async () => {
        const w = await warnBox.boundingBox();
        const f = await foot.boundingBox();
        return w && f && w.y + w.height <= (await bandTop(page)) - 7 && f.y >= 0 && w.y > 12;
      }, { message: '確認箱が帯の裏 / 画面外のまま' })
      .toBe(true);

    // 「いいえ」を選び確認箱が消えたあとの scrollY を基準にする。確認箱が見える
    // 状態からの遷移で取らないと、消えた直後の max-scroll クランプを基準に
    // 取り込んでしまい、環境ごとのクランプ量の差でテストが不安定になる
    await card.getByTestId('close-card-no').click();
    await expect(warnBox).toBeHidden();
    await settle(page);
    const scrollBefore = await page.evaluate(() => window.scrollY);

    // 既に見える位置なので、再度外して確認箱が出ても scrollY は動かない
    await card.getByTestId('close-card').click();
    await expect.poll(async () => warnBox.isVisible()).toBe(true);
    await settle(page);
    const scrollAfter = await page.evaluate(() => window.scrollY);
    expect(Math.abs(scrollAfter - scrollBefore)).toBeLessThanOrEqual(1);
  });

  test('V5. --reveal-bottom と .add-wrap の実測高さ', async ({ page }) => {
    await seedPastLogs(page, [{ daysAgo: 0, exerciseName: 'ベンチプレス', sets: sets(2) }]);
    await blurActive(page);

    const addWrapHeight = (await page.locator('.add-wrap').boundingBox()).height;
    expect(addWrapHeight).toBe(66);

    const card = page.getByTestId('exercise-card').first();
    const values = await card.evaluate((el) => {
      const foot = el.querySelector('.card-foot');
      const warn = document.createElement('div');
      warn.className = 'warn-box';
      el.appendChild(warn);
      const v = {
        card: parseFloat(getComputedStyle(el).scrollMarginBottom),
        foot: parseFloat(getComputedStyle(foot).scrollMarginBottom),
        warn: parseFloat(getComputedStyle(warn).scrollMarginBottom),
      };
      warn.remove();
      return v;
    });

    for (const [name, v] of Object.entries(values)) {
      expect(v, `${name} の scrollMarginBottom`).toBeGreaterThanOrEqual(130);
    }

    test.info().annotations.push({
      type: 'V5 実測',
      description: `scrollMarginBottom = ${JSON.stringify(values)}, addWrapHeight = ${addWrapHeight}`,
    });
  });
});

test.describe('Chromium 既定（anchoring あり、注入なし）', () => {
  test.beforeEach(async ({ page, browserName }) => {
    test.skip(browserName !== 'chromium', 'anchoring との共存は chromium だけで見る');
    await page.goto('./');
    await page.getByTestId('screen-record').waitFor({ state: 'visible' });
  });

  test('V3c（注入なし）. 収まる高さで開いて閉じるとフッタの画面位置が不変', async ({ page }) => {
    await seedPastLogs(page, [{ daysAgo: 0, exerciseName: 'ベンチプレス', sets: sets(2) }]);
    await blurActive(page);

    const card = page.getByTestId('exercise-card').first();
    const foot = card.locator('.card-foot');
    const toggle = card.getByTestId('note-toggle');
    await toggle.click();

    const footBefore = await foot.boundingBox();
    await toggle.click();

    await expect
      .poll(async () => (await foot.boundingBox()).y, { message: 'フッタの画面位置が動いた（anchoring と二重補正？）' })
      .toBeGreaterThanOrEqual(footBefore.y - 1);
    await expect
      .poll(async () => (await foot.boundingBox()).y)
      .toBeLessThanOrEqual(footBefore.y + 1);
  });

  test('V3d（注入なし）. 見出しが画面外に残るときは閉じるとカードを収める', async ({ page }) => {
    await seedPastLogs(page, [
      { daysAgo: 0, exerciseName: 'ベンチプレス', sets: sets(3) },
      { daysAgo: 0, exerciseName: 'スクワット', sets: sets(6) },
    ]);
    await blurActive(page);

    const card = page.getByTestId('exercise-card').first();
    const foot = card.locator('.card-foot');
    const toggle = card.getByTestId('note-toggle');
    await toggle.click();
    await expect(card.getByTestId('exercise-note')).toBeVisible();
    await settle(page);

    const footOpen = await foot.boundingBox();
    await page.evaluate((d) => window.scrollBy(0, d), footOpen.y - 100);

    await toggle.click();

    await expect
      .poll(async () => (await card.boundingBox()).y, { message: '見出しが画面外のまま' })
      .toBeGreaterThanOrEqual(11);
    await expect.poll(async () => (await card.boundingBox()).y).toBeLessThanOrEqual(13);
  });
});
