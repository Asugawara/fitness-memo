// 体重の線を期間ごとに「日ごと / 週平均」で選べる設定
// （adr/ux/weight-line-daily-or-weekly-per-period.md）。
//
// ここでしか見られないのは 6 つ。
//
// - **既定は 3M・6M が日ごと、1Y が週平均であること。** 点数トリガ（旧
//   `WEIGHT_DENSE_POINTS`）を廃止したので、日々計量していても 3M では日ごとのまま
//   描かれるはずで、`data-weight-smoothed` の実測でしか確かめられない
// - **設定で切り替わり、期間ごとに独立していること。** 3M を週平均にしても
//   1Y の選択は変わらない
// - **1M は常に日ごと、「全期間」は常に週単位で、設定の影響を受けないこと。**
//   `views::progress` の `weight_line` Memo が触っていない経路
// - **保存先が `fitness-memo/ui/v1` で、3 フィールドとも明示で書かれ、
//   他の設定（表示数・ドロップセット）と巻き添えで消し合わないこと**
// - **`Db` の JSON に設定が混ざらないこと**
// - **週平均に落としたときも破線がプロット領域からはみ出さないこと**
//   （`chart_layout` の単体テストで座標は固めてあるが、SVG 属性の綴りは
//   実行時に黙って無視されるため）
//
// 集計の規則そのもの（週平均へ落とすかどうかの判定・座標）は `chart_layout.rs` の
// ユニットテストが持つ。
import { expect, test } from '@playwright/test';

const STORAGE_KEY = 'fitness-memo/v3';
const UI_KEY = 'fitness-memo/ui/v1';

test.beforeEach(async ({ page }) => {
  // ★ baseURL がサブパス（/fitness-memo/）を持つとき先頭 "/" はベースを丸ごと捨てる
  await page.goto('./');
  await expect(page.getByTestId('screen-record')).toBeVisible();
});

// ── ヘルパ（共有モジュールは意図的に持たない。drop.spec.mjs から複製） ──────────

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

/** 過去のログを localStorage へ直接注入する（smoke.spec.mjs の同名ヘルパと同じ形）。 */
async function seedPastLogs(page, entries) {
  await flushToStorage(page);
  await page.evaluate(
    ({ entries, key }) => {
      const db = JSON.parse(localStorage.getItem(key));
      const dateKey = (n) => {
        const d = new Date();
        d.setDate(d.getDate() - n);
        const m = String(d.getMonth() + 1).padStart(2, '0');
        const day = String(d.getDate()).padStart(2, '0');
        return `${d.getFullYear()}-${m}-${day}`;
      };
      for (const { daysAgo, exerciseName, sets, bodyWeight } of entries) {
        const k = dateKey(daysAgo);
        const session = db.sessions[k] ?? { logs: [], body_weight: null, note: '' };
        if (exerciseName !== undefined) {
          const ex = db.exercises.find((e) => e.name === exerciseName);
          if (!ex) throw new Error(`preset exercise not found: ${exerciseName}`);
          session.logs.push({ exercise_id: ex.id, sets, at: null });
        }
        if (bodyWeight !== undefined) session.body_weight = bodyWeight;
        db.sessions[k] = session;
      }
      localStorage.setItem(key, JSON.stringify(db));
    },
    { entries, key: STORAGE_KEY },
  );
  await page.reload();
  await expect(page.getByTestId('screen-record')).toBeVisible();
}

/**
 * 設定タブの「体重の線」の節に入り、`act` を実行して記録タブへ戻る。
 * `drop.spec.mjs` の `inDropSettings` を写して `settings-row-weight-line` に向ける。
 */
async function inWeightLineSettings(page, act) {
  await blurActive(page);
  await page.getByTestId('tab-settings').click();
  const back = page.getByTestId('settings-back');
  if (await back.isVisible()) await back.click();
  await page.getByTestId('settings-row-weight-line').click();
  await act();
  await blurActive(page);
  await page.getByTestId('settings-back').click();
  await page.getByTestId('tab-record').click();
  await expect(page.getByTestId('screen-record')).toBeVisible();
}

/** ある期間の 2 択（`daily` / `weekly`）を選ぶ。 */
const setWeightLine = (page, period, which) =>
  inWeightLineSettings(page, () =>
    page
      .getByTestId('weight-line-btn')
      .and(page.locator(`[data-period="${period}"][data-line="${which}"]`))
      .click(),
  );

async function openProgress(page) {
  await blurActive(page);
  await page.getByTestId('tab-progress').click();
  await expect(page.getByTestId('screen-progress')).toBeVisible();
}

const periodButton = (page, label) =>
  page.getByTestId('period-select').getByTestId('period-btn').filter({ hasText: label });

test('1. 既定の 3M は日ごと', async ({ page }) => {
  const entries = [
    { daysAgo: 50, exerciseName: 'ベンチプレス', sets: [{ weight: 60, reps: 10 }] },
    { daysAgo: 1, exerciseName: 'ベンチプレス', sets: [{ weight: 60, reps: 10 }] },
  ];
  for (let daysAgo = 0; daysAgo <= 55; daysAgo++) {
    entries.push({ daysAgo, bodyWeight: 70 + (daysAgo % 2) });
  }
  await seedPastLogs(page, entries);

  await openProgress(page);
  const chart = page.getByTestId('chart');
  // このテストは修正前（main の実装、点数トリガ）では `true` になって落ちる
  await expect(chart).toHaveAttribute('data-weight-smoothed', 'false');
  await expect(chart).toHaveAttribute('data-weight-points', '56');
});

test('2. 既定の 1Y は週平均', async ({ page }) => {
  const entries = [
    { daysAgo: 100, exerciseName: 'ベンチプレス', sets: [{ weight: 60, reps: 10 }] },
    { daysAgo: 1, exerciseName: 'ベンチプレス', sets: [{ weight: 60, reps: 10 }] },
  ];
  for (let daysAgo = 0; daysAgo <= 119; daysAgo++) {
    entries.push({ daysAgo, bodyWeight: 70 + (daysAgo % 2) });
  }
  await seedPastLogs(page, entries);

  await openProgress(page);
  await periodButton(page, '1Y').click();
  const chart = page.getByTestId('chart');
  await expect(chart).toHaveAttribute('data-weight-smoothed', 'true');
  // 120 日 = 17 週 + 1 日なので開始曜日に関係なく 18 バケット
  await expect(chart).toHaveAttribute('data-weight-points', '18');

  // 読み取り欄は既定で最新点（daysAgo 1）の実測。daysAgo 1 は奇数 → 71。
  // 隣り合う日が必ず違う値なので 2 点以上の週の平均は 70 と 71 の間の非整数になり、
  // 実測と一致しえない
  await expect(page.getByTestId('readout-weight')).toHaveText('71 kg');
});

test('3. 設定で切り替わる', async ({ page }) => {
  const entries = [
    { daysAgo: 50, exerciseName: 'ベンチプレス', sets: [{ weight: 60, reps: 10 }] },
    { daysAgo: 1, exerciseName: 'ベンチプレス', sets: [{ weight: 60, reps: 10 }] },
  ];
  for (let daysAgo = 0; daysAgo <= 55; daysAgo++) {
    entries.push({ daysAgo, bodyWeight: 70 + (daysAgo % 2) });
  }
  await seedPastLogs(page, entries);

  await setWeightLine(page, '3m', 'weekly');
  await openProgress(page);
  const chart = page.getByTestId('chart');
  await expect(chart).toHaveAttribute('data-weight-smoothed', 'true');
  const points = Number(await chart.getAttribute('data-weight-points'));
  expect(points).toBeLessThan(56);

  await setWeightLine(page, '1y', 'daily');
  await openProgress(page);
  await periodButton(page, '1Y').click();
  await expect(chart).toHaveAttribute('data-weight-smoothed', 'false');
  await expect(chart).toHaveAttribute('data-weight-points', '56');

  await page.getByTestId('tab-settings').click();
  await expect(page.getByTestId('settings-row-weight-line')).toContainText('週平均 3M');
});

test('4. 1M は常に日ごと、全期間は設定に関係なく注記', async ({ page }) => {
  const entries = [
    { daysAgo: 50, exerciseName: 'ベンチプレス', sets: [{ weight: 60, reps: 10 }] },
    { daysAgo: 1, exerciseName: 'ベンチプレス', sets: [{ weight: 60, reps: 10 }] },
  ];
  for (let daysAgo = 0; daysAgo <= 55; daysAgo++) {
    entries.push({ daysAgo, bodyWeight: 70 + (daysAgo % 2) });
  }
  await seedPastLogs(page, entries);

  await setWeightLine(page, '3m', 'weekly');
  await setWeightLine(page, '6m', 'weekly');
  await setWeightLine(page, '1y', 'weekly');

  await openProgress(page);
  await periodButton(page, '1M').click();
  await expect(page.getByTestId('chart')).toHaveAttribute('data-weight-smoothed', 'false');

  await page.getByTestId('period-select').getByTestId('period-btn')
    .filter({ hasText: '全期間' }).click();
  await expect(page.getByTestId('weekly-note')).toContainText('体重は週平均');
});

test('5. 保存先', async ({ page }) => {
  // 事前に表示数を 2 に、ドロップを include にしておき、巻き添えで消えていないことを見る
  // （drop.spec.mjs 18 と同型）
  await page.getByTestId('tab-settings').click();
  await page.getByTestId('settings-row-history').click();
  await page.getByTestId('history-btn').and(page.locator('[data-count="2"]')).click();
  await page.getByTestId('settings-back').click();
  await page.getByTestId('settings-row-drop-sets').click();
  await page.getByTestId('drop-sets-btn').and(page.locator('[data-drops="include"]')).click();
  await page.getByTestId('settings-back').click();
  await page.getByTestId('tab-record').click();
  await expect(page.getByTestId('screen-record')).toBeVisible();

  await setWeightLine(page, '3m', 'weekly');

  const ui = await uiState(page);
  expect(ui.weight_weekly_3m).toBe(1);
  expect(ui.weight_weekly_6m).toBe(0);
  expect(ui.weight_weekly_1y).toBe(1);
  expect(ui.history, '件数の設定が巻き添えで消えている').toBe(2);
  expect(ui.drops, 'ドロップの設定が巻き添えで消えている').toBe(1);

  const db = await readDb(page);
  expect(JSON.stringify(db), '`Db` に設定が混ざっている').not.toContain('weight_weekly');
});

test('6. はみ出さない', async ({ page }) => {
  const entries = [
    { daysAgo: 50, exerciseName: 'ベンチプレス', sets: [{ weight: 60, reps: 10 }] },
    { daysAgo: 1, exerciseName: 'ベンチプレス', sets: [{ weight: 60, reps: 10 }] },
  ];
  for (let daysAgo = 0; daysAgo <= 55; daysAgo++) {
    entries.push({ daysAgo, bodyWeight: 70 + (daysAgo % 2) });
  }
  await seedPastLogs(page, entries);
  await setWeightLine(page, '3m', 'weekly');

  await openProgress(page);
  const chart = page.getByTestId('chart');
  await expect(chart).toHaveAttribute('data-weight-smoothed', 'true');

  const outside = await chart.evaluate((svg) => {
    const line = svg.querySelector('polyline.chart-line-weight');
    const grid = svg.querySelector('line.chart-grid');
    const [x0, x1] = [Number(grid.getAttribute('x1')), Number(grid.getAttribute('x2'))];
    return line
      .getAttribute('points')
      .split(' ')
      .map((p) => Number(p.split(',')[0]))
      .filter((x) => !(x >= x0 - 0.05 && x <= x1 + 0.05));
  });
  expect(outside).toEqual([]);
});
