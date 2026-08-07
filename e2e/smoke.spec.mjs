import { test, expect } from '@playwright/test';

// 計画の smoke ケースのうち 1・2・3・4・5・6・7・10・12 を実装（8・9・11 は
// カレンダー/推移/種目タブが揃う Wave 3・4 で追記される）。src/views/today.rs,
// src/views/mod.rs の data-testid を使う。
//
// ケース4・5・7 は「前日の記録」を前提にするが、calendar.rs（過去日を選ぶ導線）
// がまだ無く、today タブ単体には dates.selected を today 以外にする UI 操作が
// 存在しない（mod.rs の DateCtx::open は calendar.rs からの呼び出しを想定した
// pub fn だが、現時点でどこからも呼ばれていない）。そのため「UI からバックフィル
// する」という書き込み経路そのものは検証できず、seedPastLog() で localStorage に
// バックフィル済み（at: null）のデータを直接注入し、読み込み〜表示側だけを検証する。

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

/**
 * hidden への visibilitychange を発火させ、pending の debounce 保存を即時 flush する。
 * 単純な reload だけだと 400ms debounce と race して flaky になる。
 */
async function flushToStorage(page) {
  // page.goto() の解決は wasm(数十MBのdebugビルド)のロード完了を保証しない。
  // today 画面が出た時点なら App() の Effect::new が既に一度走っており
  // PENDING に初期 Db が積まれているので、それを待ってから flush する
  await page.getByTestId('screen-today').waitFor({ state: 'visible' });
  await page.evaluate(() => {
    Object.defineProperty(document, 'hidden', { value: true, configurable: true });
    document.dispatchEvent(new Event('visibilitychange', { bubbles: true }));
  });
}

/**
 * 投入済みプリセットの Db に daysAgo 日前のセッションを1件追加してから reload する。
 * calendar.rs が無い現状で「前日の記録が既にある」状態を作る唯一の手段。
 */
async function seedPastLog(page, { daysAgo, exerciseName, sets, at = null }) {
  await flushToStorage(page);
  await page.evaluate(
    ({ daysAgo, exerciseName, sets, at }) => {
      const KEY = 'fitness-memo/v1';
      const db = JSON.parse(localStorage.getItem(KEY));
      const ex = db.exercises.find((e) => e.name === exerciseName);
      if (!ex) throw new Error(`preset exercise not found: ${exerciseName}`);

      // Local::now().date_naive() と揃えるため UTC (toISOString) ではなく
      // ローカルタイムゾーンの年月日から日付キーを組み立てる
      const d = new Date();
      d.setDate(d.getDate() - daysAgo);
      const y = d.getFullYear();
      const m = String(d.getMonth() + 1).padStart(2, '0');
      const day = String(d.getDate()).padStart(2, '0');

      db.sessions[`${y}-${m}-${day}`] = {
        logs: [{ exercise_id: ex.id, sets, at }],
        body_weight: null,
        note: '',
      };
      localStorage.setItem(KEY, JSON.stringify(db));
    },
    { daysAgo, exerciseName, sets, at },
  );
  await page.reload();
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

  // 400ms debounce の完了を待たず、hidden 遷移で flush() が即時保存することを検証する
  await flushToStorage(page);
  await page.reload();

  const reloadedCard = page.getByTestId('exercise-card');
  await expect(reloadedCard.getByTestId('today-metric')).toHaveText('600 kg·回');
  await expect(reloadedCard.getByTestId('set-row').nth(0).getByTestId('set-weight')).toHaveValue('60');
  await expect(reloadedCard.getByTestId('set-row').nth(0).getByTestId('set-reps')).toHaveValue('10');
});

test('4. 前日にバックフィルした記録があると、今日タブの経過表示が「昨日」になる', async ({ page }) => {
  // at: null（バックフィル済み）で注入する。これが at: Some(now) だと
  // 「たった今」になってしまい、要件「最後のトレーニングから」の出力が嘘になる
  await seedPastLog(page, {
    daysAgo: 1,
    exerciseName: 'ベンチプレス',
    sets: [{ weight: 60, reps: 10 }],
  });

  await expect(page.getByTestId('elapsed')).toHaveText('昨日');
});

test('5. セットが空のときだけ「前回をコピー」が出て、押すと前日のセットがプリフィルされる', async ({ page }) => {
  await seedPastLog(page, {
    daysAgo: 1,
    exerciseName: 'ベンチプレス',
    sets: [{ weight: 50, reps: 8 }],
  });

  const card = await addExercise(page, 'ベンチプレス');

  // セットが空の間は「前回をコピー」が出る
  await expect(card.getByTestId('copy-last')).toBeVisible();

  await card.getByTestId('copy-last').click();

  const row0 = card.getByTestId('set-row').nth(0);
  await expect(row0.getByTestId('set-weight')).toHaveValue('50');
  await expect(row0.getByTestId('set-reps')).toHaveValue('8');

  // セットが入った後は「前回をコピー」が消える（置換か追記かの曖昧さを無くす設計）
  await expect(card.getByTestId('copy-last')).toHaveCount(0);
});

test('6. 体重・体調メモを入力するとリロード後も残る', async ({ page }) => {
  await page.getByTestId('condition-toggle').click();
  await page.getByTestId('body-weight').fill('62.5');
  await page.getByTestId('condition-note').fill('絶好調');

  await flushToStorage(page);
  await page.reload();

  // body_weight が非空で復元されれば ConditionRow は自動的に開いた状態で描画される
  await expect(page.getByTestId('body-weight')).toHaveValue('62.5');
  await expect(page.getByTestId('condition-note')).toHaveValue('絶好調');
});

test('7. 部位チップは実施部位に日数、未実施の部位には「—」を表示する', async ({ page }) => {
  await seedPastLog(page, {
    daysAgo: 3,
    exerciseName: 'ベンチプレス', // 胸
    sets: [{ weight: 60, reps: 10 }],
  });

  await expect(page.getByTestId('group-chip').filter({ hasText: '胸' })).toContainText('3d');
  await expect(page.getByTestId('group-chip').filter({ hasText: '背中' })).toContainText('—');
  await expect(page.getByTestId('group-chip').filter({ hasText: '体幹' })).toContainText('—');
});

test('10. 同じ日に同じ種目を再度追加してもカードは増えず既存カードのまま', async ({ page }) => {
  const card = await addExercise(page, 'ベンチプレス');
  const row0 = card.getByTestId('set-row').nth(0);
  await row0.getByTestId('set-weight').fill('70');
  await row0.getByTestId('set-reps').fill('5');

  await expect(page.getByTestId('exercise-card')).toHaveCount(1);

  // 既に追加済みの種目をもう一度ピックしても新規カードは作られない
  await page.getByTestId('add-exercise').click();
  await page
    .getByTestId('add-sheet')
    .getByTestId('pick-exercise')
    .filter({ hasText: exactText('ベンチプレス') })
    .click();

  await expect(page.getByTestId('exercise-card')).toHaveCount(1);
  // 新規カードへの置き換えではなく既存カードのままであることの確認（入力内容が保持される）
  await expect(row0.getByTestId('set-weight')).toHaveValue('70');
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
