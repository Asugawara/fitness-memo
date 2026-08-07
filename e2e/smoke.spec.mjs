import { test, expect } from '@playwright/test';

// 計画の smoke ケースのうち 1・2・3・12 のみ実装（残りはカレンダー/推移/種目タブが
// 揃う Wave 3・4 で追記される）。src/views/today.rs, src/views/mod.rs の data-testid
// を使う。

test.beforeEach(async ({ page }) => {
  await page.goto('/');
});

// hasText は部分一致なので "ベンチプレス" が "インクラインベンチプレス" にも
// マッチしてしまう。pick-exercise は全プリセットボタンで共有の testid なので、
// 名前での絞り込みは常に完全一致にする
function exactText(s) {
  return new RegExp(`^${s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}$`);
}

/** 「種目を追加」シートからプリセットを選び、追加されたカードを返す。 */
async function addExercise(page, name) {
  await page.getByTestId('add-exercise').click();
  await page
    .getByTestId('add-sheet')
    .getByTestId('pick-exercise')
    .filter({ hasText: exactText(name) })
    .click();
  return page.getByTestId('exercise-card');
}

test('1. 初回起動でプリセットが投入され「今日」タブが出る', async ({ page }) => {
  await expect(page.getByTestId('screen-today')).toBeVisible();
  await expect(page.getByTestId('tab-today')).toHaveClass(/active/);

  // まだ何も記録していないので経過時間は「—」、部位チップは6部位分
  await expect(page.getByTestId('elapsed')).toHaveText('—');
  await expect(page.getByTestId('group-chip')).toHaveCount(6);

  // プリセットが投入されている（胸=ベンチプレス、背中=懸垂）ことをシートで確認
  await page.getByTestId('add-exercise').click();
  const sheet = page.getByTestId('add-sheet');
  await expect(sheet).toBeVisible();
  await expect(sheet.getByTestId('pick-exercise').filter({ hasText: exactText('ベンチプレス') })).toBeVisible();
  await expect(sheet.getByTestId('pick-exercise').filter({ hasText: exactText('懸垂') })).toBeVisible();
});

test('2. 種目を追加してセットを2行入力すると指標表示が正しい（60×10 + 60×8 = 1,080）', async ({ page }) => {
  const card = await addExercise(page, 'ベンチプレス');
  const rows = card.getByTestId('set-row');

  await rows.nth(0).getByTestId('set-weight').fill('60');
  await rows.nth(0).getByTestId('set-reps').fill('10');

  await card.getByTestId('add-set').click();
  await rows.nth(1).getByTestId('set-weight').fill('60');
  await rows.nth(1).getByTestId('set-reps').fill('8');

  await expect(card.getByTestId('today-metric')).toHaveText('1,080 kg·回');
});

test('3. hidden への visibilitychange を発火してからリロードしても入力が残る', async ({ page }) => {
  const card = await addExercise(page, 'ベンチプレス');
  const row0 = card.getByTestId('set-row').nth(0);
  await row0.getByTestId('set-weight').fill('60');
  await row0.getByTestId('set-reps').fill('10');

  // 400ms debounce の完了を待たず、hidden 遷移で flush() が即時保存することを検証する。
  // 単純な reload だけだと debounce と race して flaky になる（計画の注記どおり）
  await page.evaluate(() => {
    Object.defineProperty(document, 'hidden', { value: true, configurable: true });
    document.dispatchEvent(new Event('visibilitychange', { bubbles: true }));
  });

  await page.reload();

  const reloadedCard = page.getByTestId('exercise-card');
  await expect(reloadedCard.getByTestId('today-metric')).toHaveText('600 kg·回');
  await expect(reloadedCard.getByTestId('set-row').nth(0).getByTestId('set-weight')).toHaveValue('60');
  await expect(reloadedCard.getByTestId('set-row').nth(0).getByTestId('set-reps')).toHaveValue('10');
});

test('12. 重量入力の中間状態("6.")でクラッシュせず、"6.5"まで打つと指標に反映される', async ({ page }) => {
  const card = await addExercise(page, 'ベンチプレス');
  const row0 = card.getByTestId('set-row').nth(0);
  const weight = row0.getByTestId('set-weight');

  await row0.getByTestId('set-reps').fill('2');

  await weight.fill('6.');
  // type="number" ではなく type="text"+inputmode の検証: 中間状態が正規化/クリアされない
  await expect(weight).toHaveValue('6.');

  await weight.fill('6.5');
  await expect(weight).toHaveValue('6.5');

  // 6.5 × 2 = 13。"6." で止まっていた/クラッシュしていれば 12 のままか反映されない
  await expect(card.getByTestId('today-metric')).toHaveText('13 kg·回');
});
