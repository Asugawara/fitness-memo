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
