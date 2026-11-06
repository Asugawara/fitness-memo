// ± ボタンとタップで空にする入力（adr/ux/tap-clears-the-number-and-steps-above-the-field.md）。
//
// ここでしか見られないのは 4 つ。
//
// - **タップで空になり、薄字で元の値が残ること。** 何も打たずに離れれば戻り、
//   打って全部消して離れたときだけ空のまま残る（判定は signal が持つ）
// - **± の 1 回の押下で値が変わり、保存され、フォーカスは入力欄に残ること**
// - **設定の枠と種目の上書きが箱の並びに効くこと**（「設定 → カード」の配線）
// - **設定を変えても他の設定が消えないこと**（`UiState` の巻き添え）
//
// 幅の解釈・増減の規則は `core.rs` のユニットテストが持つ。
import { expect, test } from '@playwright/test';

const STORAGE_KEY = 'fitness-memo/v3';
const UI_KEY = 'fitness-memo/ui/v1';

test.beforeEach(async ({ page }) => {
  await page.goto('./');
  await expect(page.getByTestId('screen-record')).toBeVisible();
});

// ── ヘルパ（共有モジュールは持たない。各スペックに複製する） ──────────────

function exactText(s) {
  return new RegExp(`^${s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}$`);
}

async function blurActive(page) {
  await page.evaluate(() => {
    if (document.activeElement instanceof HTMLElement) document.activeElement.blur();
  });
}

/** hidden への visibilitychange で pending の debounce 保存を即時 flush する。 */
async function flushToStorage(page) {
  await page.evaluate(() => {
    Object.defineProperty(document, 'hidden', { value: true, configurable: true });
    document.dispatchEvent(new Event('visibilitychange', { bubbles: true }));
  });
}

async function readDb(page) {
  await flushToStorage(page);
  return page.evaluate((key) => JSON.parse(localStorage.getItem(key)), STORAGE_KEY);
}

async function uiState(page) {
  await flushToStorage(page);
  return page.evaluate((key) => JSON.parse(localStorage.getItem(key) ?? '{}'), UI_KEY);
}

/** 「種目を追加」シートで部位を開いて種目を押す（drop.spec.mjs の同名ヘルパと同じ形）。 */
async function addExercise(page, name) {
  await blurActive(page);
  await page.getByTestId('add-exercise').click();
  const sheet = page.getByTestId('add-sheet');
  const groups = sheet.getByTestId('pick-group');
  const n = await groups.count();
  expect(n, 'シートに部位が 1 つも出ていない').toBeGreaterThan(0);
  for (let i = 0; i < n; i += 1) {
    const group = groups.nth(i);
    const toggle = group.getByTestId('pick-group-toggle');
    if ((await toggle.getAttribute('aria-expanded')) !== 'true') await toggle.click();
    await expect(group.getByTestId('pick-exercise').first()).toBeVisible();
    const pick = group.getByTestId('pick-exercise').filter({ hasText: exactText(name) });
    if (await pick.count()) {
      await pick.click();
      return page.getByTestId('exercise-card');
    }
  }
  throw new Error(`「種目を追加」シートに ${name} が無い`);
}

/** 種目を出して 1 セット目に 60×10 を入れ、欄の外をタップして離す。 */
async function cardWithOneSet(page) {
  const card = await addExercise(page, 'ベンチプレス');
  const row = card.getByTestId('set-row').nth(0);
  await row.getByTestId('set-weight').fill('60');
  await row.getByTestId('set-reps').fill('10');
  await blurActive(page);
  return { card, row };
}

/** 設定タブの ± ボタンの節に入り、`act` を実行して記録タブへ戻る。 */
async function inSteps(page, act) {
  await blurActive(page);
  await page.getByTestId('tab-settings').click();
  const back = page.getByTestId('settings-back');
  if (await back.isVisible()) await back.click();
  await page.getByTestId('settings-row-steps').click();
  await act();
  await blurActive(page);
  await page.getByTestId('settings-back').click();
  await page.getByTestId('tab-record').click();
  await expect(page.getByTestId('screen-record')).toBeVisible();
}

/** 箱の中のボタンを data-delta で引く（ラベルの文字は見ない）。 */
const stepBtn = (scope, delta) => scope.locator(`[data-testid="step-btn"][data-delta="${delta}"]`);

// ── 入力の動き ───────────────────────────────────────────────────────────────

test('1. ★ タップで空になり薄字で元の値が残る。打てば置き換わる', async ({ page }) => {
  const { row } = await cardWithOneSet(page);
  const weight = row.getByTestId('set-weight');
  await expect(weight).toHaveValue('60');

  await weight.click();
  await expect(weight).toHaveValue('');
  await expect(weight).toHaveAttribute('placeholder', '60');

  await weight.pressSequentially('62.5');
  await expect(weight).toHaveValue('62.5');
  await blurActive(page);
  await expect(weight).toHaveValue('62.5');
});

test('2. ★ 何も打たずに離れると元の値に戻り、時刻も保存も動かない', async ({ page }) => {
  const { row } = await cardWithOneSet(page);
  const before = await readDb(page);
  const setBefore = Object.values(before.sessions)[0].logs[0].sets[0];

  const weight = row.getByTestId('set-weight');
  await weight.click();
  await expect(weight).toHaveValue('');
  await blurActive(page);
  await expect(weight).toHaveValue('60');
  await expect(weight).not.toHaveAttribute('placeholder', '60');

  const after = await readDb(page);
  const setAfter = Object.values(after.sessions)[0].logs[0].sets[0];
  expect(setAfter).toEqual(setBefore);
});

test('3. ★ 打ってから全部消して離れると空のまま残る', async ({ page }) => {
  const { row } = await cardWithOneSet(page);
  const weight = row.getByTestId('set-weight');
  await weight.click();
  await weight.pressSequentially('7');
  await weight.press('Backspace');
  await blurActive(page);
  await expect(weight).toHaveValue('');
});

// ── ± の箱 ───────────────────────────────────────────────────────────────────

test('4. ★ +2.5 は 1 回で値を変え、保存され、フォーカスは入力欄に残る', async ({ page }) => {
  const { row } = await cardWithOneSet(page);
  const weight = row.getByTestId('set-weight');
  await weight.click();
  await expect(weight).toHaveValue('');

  await stepBtn(row, '+2.5').click();
  await expect(weight).toHaveValue('62.5');
  await expect(weight).toBeFocused();

  await blurActive(page);
  await page.reload();
  await expect(page.getByTestId('screen-record')).toBeVisible();
  await expect(page.getByTestId('exercise-card').getByTestId('set-row').nth(0).getByTestId('set-weight')).toHaveValue('62.5');
});

test('5. ★ 回数 1 で −1 を押しても 1、重量 0.5 で −1.25 を押すと 0', async ({ page }) => {
  const { row } = await cardWithOneSet(page);
  const reps = row.getByTestId('set-reps');
  await reps.click();
  await expect(reps).toHaveValue('');
  await reps.pressSequentially('1');
  await stepBtn(row, '-1').click();
  await expect(reps).toHaveValue('1');

  const weight = row.getByTestId('set-weight');
  await weight.click();
  await weight.pressSequentially('0.5');
  await stepBtn(row, '-1.25').click();
  await expect(weight).toHaveValue('0');
});

test('6. ★ 設定で重量を「2」1 枠にすると箱は −2 / +2 だけ、全部空なら箱が出ない', async ({ page }) => {
  // 既定の 3 枠に戻したうえで、重量を「2」だけにする
  await inSteps(page, async () => {
    const slots = page.getByTestId('steps-weight-input');
    await slots.nth(0).fill('2');
    await slots.nth(0).blur();
    await slots.nth(1).fill('');
    await slots.nth(1).blur();
    await slots.nth(2).fill('');
    await slots.nth(2).blur();
  });

  const { row } = await cardWithOneSet(page);
  await row.getByTestId('set-weight').click();
  await expect(stepBtn(row, '-2')).toHaveCount(1);
  await expect(stepBtn(row, '+2')).toHaveCount(1);
  await expect(row.locator('.num-wrap').first().getByTestId('step-btn')).toHaveCount(2);

  // 全部空にすると、重量欄には箱が出ない（タップで空になる動きは残る）
  await inSteps(page, async () => {
    await page.getByTestId('steps-weight-input').nth(0).fill('');
    await page.getByTestId('steps-weight-input').nth(0).blur();
  });
  await row.getByTestId('set-weight').click();
  await expect(row.getByTestId('set-weight')).toHaveValue('');
  await expect(row.locator('.num-wrap').first().getByTestId('step-btn')).toHaveCount(0);
});

test('7. 設定を変えても history / drop_pct が消えない', async ({ page }) => {
  // 他の設定を先に書いておき、± の節の保存でそれらが消えないことを見る
  await page.evaluate((key) => {
    localStorage.setItem(key, JSON.stringify({ history: 2, drop_pct: 25, install_hint_dismissed: true }));
  }, UI_KEY);
  await page.reload();
  await expect(page.getByTestId('screen-record')).toBeVisible();

  await inSteps(page, async () => {
    await page.getByTestId('steps-reps-input').nth(0).fill('5');
    await page.getByTestId('steps-reps-input').nth(0).blur();
  });
  const ui = await uiState(page);
  expect(ui.reps_steps).toBe('5,,');
  expect(ui.history).toBe(2);
  expect(ui.drop_pct).toBe(25);
});

// ── 種目ごとの上書き ─────────────────────────────────────────────────────────

test('8. ★ 種目の編集で 1 / 2 を入れると、その種目だけ箱が変わり、JSON に種目の分だけ入る', async ({ page }) => {
  // ベンチプレスを 1 度だけ使う（使っていない種目の JSON に出ないことを見るため）
  const { row } = await cardWithOneSet(page);
  await expect(row).toBeVisible();

  await page.getByTestId('tab-settings').click();
  const back = page.getByTestId('settings-back');
  if (await back.isVisible()) await back.click();
  await page.getByTestId('settings-row-exercises').click();
  const item = page.getByTestId('group-item').filter({
    has: page.getByTestId('group-name').filter({ hasText: exactText('胸') }),
  });
  const toggle = item.getByTestId('group-toggle');
  if ((await toggle.getAttribute('aria-expanded')) !== 'true') await toggle.click();
  await item.getByTestId('exercise-name').filter({ hasText: exactText('ベンチプレス') }).click();

  const inputs = page.getByTestId('ex-step-input');
  await inputs.nth(0).fill('1');
  await inputs.nth(0).blur();
  await inputs.nth(1).fill('2');
  await inputs.nth(1).blur();
  await page.getByTestId('settings-sheet-close').click();

  await blurActive(page);
  await page.getByTestId('settings-back').click();
  await page.getByTestId('tab-record').click();

  const bench = page.getByTestId('exercise-card').first();
  const benchRow = bench.getByTestId('set-row').nth(0);
  await benchRow.getByTestId('set-weight').click();
  const weightBox = benchRow.locator('.num-wrap').first();
  await expect(stepBtn(weightBox, '-2')).toHaveCount(1);
  await expect(stepBtn(weightBox, '-1')).toHaveCount(1);
  await expect(stepBtn(weightBox, '+1')).toHaveCount(1);
  await expect(stepBtn(weightBox, '+2')).toHaveCount(1);
  await expect(weightBox.getByTestId('step-btn')).toHaveCount(4);

  const db = await readDb(page);
  const ex = db.exercises.find((e) => e.name === 'ベンチプレス');
  expect(ex.weight_steps).toEqual([1, 2]);
  const unused = db.exercises.filter((e) => e.name !== 'ベンチプレス');
  for (const e of unused) expect(e).not.toHaveProperty('weight_steps');
});

// ── 端に寄せる 2 段（adr/ux/stepper-on-the-thumb-side.md）─────────────────────

/** 設定の「位置」を押す（`side` は left / above / right）。 */
async function setSide(page, side) {
  await inSteps(page, async () => {
    await page.locator(`[data-testid="steps-side-btn"][data-side="${side}"]`).click();
  });
}

/** 箱の矩形（表示されるまで待つ）。箱は欄にフォーカスがあるあいだだけ出る。 */
async function boxRect(locator) {
  await expect(locator).toBeVisible();
  return locator.boundingBox();
}

test('9. ★ 右端：箱は行の右端に付き、＋ の真上に − が来る。history と位置が保存される', async ({ page }) => {
  await page.evaluate((key) => {
    localStorage.setItem(key, JSON.stringify({ history: 2, drop_pct: 25 }));
  }, UI_KEY);
  await page.reload();
  await expect(page.getByTestId('screen-record')).toBeVisible();

  await setSide(page, 'right');
  const { row } = await cardWithOneSet(page);
  await row.getByTestId('set-weight').click();

  const box = row.getByTestId('stepper').first();
  await expect(box).toHaveAttribute('data-side', 'right');
  const rowRect = await row.boundingBox();
  const boxRectNow = await boxRect(box);
  expect(Math.abs(boxRectNow.x + boxRectNow.width - (rowRect.x + rowRect.width))).toBeLessThanOrEqual(1);

  const plus = stepBtn(box, '+2.5').first();
  const minus = stepBtn(box, '-2.5').first();
  const pr = await plus.boundingBox();
  const mr = await minus.boundingBox();
  expect(Math.abs(pr.x - mr.x)).toBeLessThanOrEqual(1);
  expect(pr.y).toBeLessThan(mr.y);

  const ui = await uiState(page);
  expect(ui.stepper_side).toBe('right');
  expect(ui.history).toBe(2);
  expect(ui.drop_pct).toBe(25);
});

test('10. ★ 右端の ＋2.5 は 1 回で値を変え、フォーカスは欄に残る', async ({ page }) => {
  await setSide(page, 'right');
  const { row } = await cardWithOneSet(page);
  const weight = row.getByTestId('set-weight');
  await weight.click();
  await expect(weight).toHaveValue('');

  await stepBtn(row, '+2.5').click();
  await expect(weight).toHaveValue('62.5');
  await expect(weight).toBeFocused();
});

test('11. ★ 左端：箱の左端は行の左端に付く。ドロップの段でも同じ x', async ({ page }) => {
  await setSide(page, 'left');
  const { card, row } = await cardWithOneSet(page);
  const rowRect = await row.boundingBox();

  await row.getByTestId('set-weight').click();
  const mainBox = await boxRect(row.getByTestId('stepper').first());
  expect(Math.abs(mainBox.x - rowRect.x)).toBeLessThanOrEqual(1);
  await blurActive(page);

  await row.getByTestId('drop-add').click();
  const dropWeight = row.getByTestId('drop-weight');
  await dropWeight.click();
  const dropRow = card.getByTestId('drop-row');
  const dropBox = await boxRect(dropRow.getByTestId('stepper').first());
  expect(Math.abs(dropBox.x - rowRect.x)).toBeLessThanOrEqual(1);
});

test('12. ★ 右端：1 行目の欄が画面の上へ寄っていても、箱の上端は画面内に入る', async ({ page }) => {
  // 種目を 1 つだけにすると、種目を足した直後はページの末尾に居て、その位置から先へは
  // スクロールできない。画面の高さを 400px にすると、1 行目の欄は上端から約 70〜82px に来る。
  //
  // ★ 500px ではなく 400px なのは Pixel 7 のため。Pixel 7 の UA ではカードの下の
  //   install-hint が出ない（storage_may_split() が false）ので、カードより下の余白が
  //   chromium / iPhone より約 98px 短く、末尾までスクロールしても欄が 170.5px までしか
  //   上がらない（実測。500px だと前提の 104px に届かない）。400px なら 3 project とも
  //   104px を割る（Pixel 7 は 70.5px）
  await page.setViewportSize({ width: 390, height: 400 });
  await setSide(page, 'right');
  const card = await addExercise(page, 'ベンチプレス');
  const weight = card.getByTestId('set-row').nth(0).getByTestId('set-weight');

  // 前提：欄の上端が 104px 未満になっていないと、この試験は何も見ていない
  const before = await weight.boundingBox();
  expect(before.y, '前提：欄の上端が 104px 未満になっていること').toBeLessThan(104);

  await weight.click();
  const box = card.getByTestId('set-row').nth(0).getByTestId('stepper').first();
  await expect.poll(async () => (await box.boundingBox())?.y ?? -1).toBeGreaterThanOrEqual(0);
});

test('13. 既定（未設定）は data-side="above" で、重量の箱の左端は欄の左端に揃う（今のまま）', async ({ page }) => {
  const { row } = await cardWithOneSet(page);
  const weight = row.getByTestId('set-weight');
  await weight.click();
  const box = row.getByTestId('stepper').first();
  await expect(box).toHaveAttribute('data-side', 'above');
  const boxR = await boxRect(box);
  const wr = await weight.boundingBox();
  expect(Math.abs(boxR.x - wr.x)).toBeLessThanOrEqual(1);
  expect((await uiState(page)).stepper_side).toBeNull();
});

// ── キーボードの上（adr/ux/stepper-above-the-keyboard.md）────────────────────
//
// 箱は画面幅いっぱいの 2 段（上 ＋ / 下 −）。Playwright にはキーボードが無いので、
// キーボードが出ている状態は `--kb-inset` が 0 の「画面の下端」で見る。

/** 箱の中のボタンを DOM の順に `data-delta` で返す。 */
const deltasOf = (box) => box.locator('[data-testid="step-btn"]').evaluateAll((els) => els.map((e) => e.dataset.delta));

test('14. ★ キーボードの上：位置ボタンは 375px に収まり、箱は画面幅いっぱい・下端に付く。＋が上段、−が下段', async ({ page }) => {
  await page.setViewportSize({ width: 375, height: 700 });
  await page.evaluate((key) => {
    localStorage.setItem(key, JSON.stringify({ history: 2, drop_pct: 25 }));
  }, UI_KEY);
  await page.reload();
  await expect(page.getByTestId('screen-record')).toBeVisible();

  await inSteps(page, async () => {
    const group = page.getByTestId('steps-side');
    const fits = await group.evaluate((el) => el.scrollWidth <= el.clientWidth);
    expect(fits, '位置ボタンの 4 つが 375px に収まる').toBe(true);
    await page.locator('[data-testid="steps-side-btn"][data-side="keyboard"]').click();
  });

  const { row } = await cardWithOneSet(page);
  await row.getByTestId('set-weight').click();

  const box = row.getByTestId('stepper').first();
  await expect(box).toHaveAttribute('data-side', 'keyboard');
  const r = await boxRect(box);
  const vp = await page.evaluate(() => ({ w: window.innerWidth, h: window.innerHeight }));
  expect(Math.abs(r.x), '箱は左端から').toBeLessThanOrEqual(1);
  expect(Math.abs(r.width - vp.w), '箱の幅 = innerWidth').toBeLessThanOrEqual(1);
  expect(Math.abs(r.y + r.height - vp.h), '箱の下端 = innerHeight').toBeLessThanOrEqual(1);

  expect(await deltasOf(box)).toEqual(['+1.25', '-1.25', '+2.5', '-2.5', '+5', '-5']);
  const plus = stepBtn(box, '+2.5');
  const minus = stepBtn(box, '-2.5');
  const pr = await plus.boundingBox();
  const mr = await minus.boundingBox();
  expect(pr.y, '＋ が上の段').toBeLessThan(mr.y);
  expect(Math.abs(pr.x - mr.x), '同じ幅の ＋ と − は同じ列').toBeLessThanOrEqual(1);

  const ui = await uiState(page);
  expect(ui.stepper_side).toBe('keyboard');
  expect(ui.history).toBe(2);
  expect(ui.drop_pct).toBe(25);
});

test('15. ★ キーボードの上の ＋2.5 は 1 回で値を変え、フォーカスは欄に残る', async ({ page }) => {
  await setSide(page, 'keyboard');
  const { row } = await cardWithOneSet(page);
  const weight = row.getByTestId('set-weight');
  await weight.click();
  await expect(weight).toHaveValue('');

  await stepBtn(row, '+2.5').click();
  await expect(weight).toHaveValue('62.5');
  await expect(weight).toBeFocused();
});

test('16. ★ 画面の一番下にある欄にフォーカスしても、欄の下端が箱の上端より上に出る', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 400 });
  await setSide(page, 'keyboard');
  const card = await addExercise(page, 'ベンチプレス');
  const weight = card.getByTestId('set-row').nth(0).getByTestId('set-weight');
  const box = card.getByTestId('set-row').nth(0).getByTestId('stepper').first();
  // 欄の下に文書の高さが要る。1 種目だけだと欄は画面の上の方までしか上がらないので、
  // 下にもう 1 種目足してから、1 種目目の欄を「スクロールで画面の一番下」へ送る
  await addExercise(page, 'スクワット');
  await weight.evaluate((el) => el.scrollIntoView({ block: 'end' }));

  // 前提：欄が箱の置かれる帯（画面下端から 96px）に重なる位置にある
  const before = await weight.boundingBox();
  expect(before.y + before.height, '前提：欄が箱の帯に重なる').toBeGreaterThan(400 - 96);

  await weight.click();
  await expect(box).toBeVisible();
  const fieldR = await weight.boundingBox();
  const boxR = await box.boundingBox();
  expect(fieldR.y + fieldR.height, '欄の下端 ≤ 箱の上端').toBeLessThanOrEqual(boxR.y + 1);
});

test('17. 回数の箱・ドロップの段の箱も、画面幅いっぱい・下端に付く（＋1 が上段）', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 700 });
  await setSide(page, 'keyboard');
  const { card, row } = await cardWithOneSet(page);
  const vp = await page.evaluate(() => ({ w: window.innerWidth, h: window.innerHeight }));

  // 回数欄の箱（重量欄の箱は同じ行にあるが、フォーカス中の欄の箱だけ出る）
  const repsWrap = row.locator('.num-wrap').nth(1);
  await row.getByTestId('set-reps').click();
  const repsBox = await boxRect(repsWrap.getByTestId('stepper'));
  expect(Math.abs(repsBox.width - vp.w)).toBeLessThanOrEqual(1);
  expect(Math.abs(repsBox.y + repsBox.height - vp.h)).toBeLessThanOrEqual(1);
  // 最小の値の組（既定は 1）は ＋ が上、− が下で、最初の組が +1 / −1
  const repsDeltas = await deltasOf(repsWrap.getByTestId('stepper'));
  expect(repsDeltas.slice(0, 2)).toEqual(['+1', '-1']);
  await blurActive(page);

  await row.getByTestId('drop-add').click();
  await card.getByTestId('drop-row').getByTestId('drop-weight').click();
  const dropBox = await boxRect(card.getByTestId('drop-row').getByTestId('stepper').first());
  expect(Math.abs(dropBox.width - vp.w)).toBeLessThanOrEqual(1);
  expect(Math.abs(dropBox.y + dropBox.height - vp.h)).toBeLessThanOrEqual(1);
});

test('18. ★ キーボードの上（Chromium）：visual viewport を縮めると、箱の下端が visual viewport の下端に付く', async ({ page, browserName }) => {
  test.skip(browserName !== 'chromium', 'CDP の setPageScaleFactor は Chromium のみ');
  await setSide(page, 'keyboard');
  const { row } = await cardWithOneSet(page);
  await row.getByTestId('set-weight').click();
  const box = row.getByTestId('stepper').first();
  await expect(box).toBeVisible();

  const cdp = await page.context().newCDPSession(page);
  await cdp.send('Emulation.setPageScaleFactor', { pageScaleFactor: 2 });

  // 前提：visual viewport が実際に縮んでいること（縮まなければ、この試験は何も見ていない）
  await expect.poll(() => page.evaluate(() => window.visualViewport.height)).toBeLessThan(
    await page.evaluate(() => window.innerHeight) - 1,
  );
  await expect.poll(() => page.evaluate(() => parseFloat(document.documentElement.style.getPropertyValue('--kb-inset')) || 0)).toBeGreaterThan(0);

  const bottom = await page.evaluate(() => window.visualViewport.offsetTop + window.visualViewport.height);
  const r = await boxRect(box);
  expect(Math.abs(r.y + r.height - bottom)).toBeLessThanOrEqual(1);
});
