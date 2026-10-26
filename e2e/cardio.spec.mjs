import { test, expect } from '@playwright/test';

// 有酸素（区間カード）の E2E。部位「有酸素」の種目は、行 = 区間のカードになる。
//
// ここで見るのは画面の配線: 「+ 区間」が直前の行を写して時間欄にフォーカスすること、
// 保存 JSON の形、前回をコピー、TSV の往復、375px 幅で 3 欄と ✕ が 1 行に収まること、
// 既存の利用者への部位の追加が 1 回だけであること。
// 換算・マージ・正規化は `cargo test` が持つ（interval.spec.mjs と同じ分担）。

test.beforeEach(async ({ page }) => {
  await page.goto('./');
  await expect(page.getByTestId('screen-record')).toBeVisible();
});

const STORAGE_KEY = 'fitness-memo/v3';
const TSV_MIME = 'text/tab-separated-values';

test.beforeEach(async ({ page }) => {
  // ★ baseURL がサブパス（/fitness-memo/）を持つとき先頭 "/" はベースを丸ごと捨てる
  await page.goto('./');
  await expect(page.getByTestId('screen-record')).toBeVisible();
});

// ── ヘルパ ──────────────────────────────────────────────────────────────────

function exactText(s) {
  return new RegExp(`^${s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}$`);
}

function dateKey(d) {
  const m = String(d.getMonth() + 1).padStart(2, '0');
  const day = String(d.getDate()).padStart(2, '0');
  return `${d.getFullYear()}-${m}-${day}`;
}

/** views/mod.rs の `fmt_date` と同じ "8/8 (金)" */
function fmtDate(d) {
  return `${d.getMonth() + 1}/${d.getDate()} (${['日', '月', '火', '水', '木', '金', '土'][d.getDay()]})`;
}

/** views/calendar.rs の `fmt_month` と同じ "2026年8月" */
function fmtMonth(d) {
  return `${d.getFullYear()}年${d.getMonth() + 1}月`;
}

function daysAgo(n) {
  const d = new Date();
  d.setDate(d.getDate() - n);
  return d;
}

/**
 * フォーカスを外す。★ 入力欄にフォーカスが残ると `.app` に `kb-open` が付き、
 * `.kb-open .add-wrap` などが消えて次の操作がタイムアウトする。
 */
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
  const raw = await page.evaluate((key) => localStorage.getItem(key), STORAGE_KEY);
  expect(raw, 'localStorage に Db が保存されていること').not.toBeNull();
  return JSON.parse(raw);
}

/**
 * 保存されたその種目のインターバル。`Exercise` に載っている＝日に属していないことの証拠。
 * 未設定は `skip_serializing_if` でキーごと消えるので `undefined` が返る。
 */
async function savedInterval(page, name) {
  const db = await readDb(page);
  return db.exercises.find((e) => e.name === name)?.interval_sec;
}

const dayCell = (page, date) =>
  page.locator(`[data-testid="cal-day"][data-date="${dateKey(date)}"]`);

async function openCalendarOn(page, date) {
  const now = new Date();
  const delta =
    (date.getFullYear() - now.getFullYear()) * 12 + (date.getMonth() - now.getMonth());
  const button = delta < 0 ? 'cal-prev' : 'cal-next';
  for (let i = 0; i < Math.abs(delta); i++) {
    await page.getByTestId(button).click();
  }
  await expect(page.getByTestId('cal-title')).toHaveText(fmtMonth(date));
}

async function openDay(page, date) {
  await openCalendarOn(page, date);
  await dayCell(page, date).click();
  await expect(page.getByTestId('today-date')).toHaveText(fmtDate(date));
}

function cardOf(page, name) {
  return page
    .getByTestId('exercise-card')
    .filter({ has: page.getByTestId('card-name').filter({ hasText: exactText(name) }) });
}

/**
 * 「種目を追加」シートの部位アコーディオンを、目当ての種目が出るまで順に開いて押す。
 *
 * ★ 部位は既定で全部閉じていて、**同時に開けるのは 1 つ**
 *   （adr/ux/record-add-sheet-groups-as-single-open-accordion.md）。押す前に
 *   `aria-expanded` を見ないと、開いている部位を閉じてしまう。
 * ★ 正典は e2e/smoke.spec.mjs の `openPickGroupFor`（e2e は spec 単体で読める作法なので
 *   共有モジュールを作らずコピーしてある）。
 */
async function pickFromAddSheet(page, name) {
  const sheet = page.getByTestId('add-sheet');
  const groups = sheet.getByTestId('pick-group');
  const n = await groups.count();
  expect(n, 'シートに部位が 1 つも出ていない').toBeGreaterThan(0);
  for (let i = 0; i < n; i++) {
    const group = groups.nth(i);
    const toggle = group.getByTestId('pick-group-toggle');
    if ((await toggle.getAttribute('aria-expanded')) !== 'true') await toggle.click();
    await expect(group.getByTestId('pick-exercise').first()).toBeVisible();
    const pick = group.getByTestId('pick-exercise').filter({ hasText: exactText(name) });
    if (await pick.count()) return pick.click();
  }
  throw new Error(`「種目を追加」シートに ${name} が無い`);
}

async function addExercise(page, name) {
  await blurActive(page);
  await page.getByTestId('add-exercise').click();
  await pickFromAddSheet(page, name);
  return cardOf(page, name);
}

const field = (card, id) => card.getByTestId('set-row').getByTestId(id);

async function fillRow(row, { min, kmh, incline }) {
  await row.getByTestId('cardio-min').fill(min);
  await row.getByTestId('cardio-kmh').fill(kmh);
  await row.getByTestId('cardio-incline').fill(incline);
}

test('トレッドミルは 分・km/h・% の区間として保存され、重量と回数は 0', async ({ page }) => {
  const card = await addExercise(page, 'トレッドミル');
  await fillRow(card.getByTestId('set-row').first(), { min: '15', kmh: '4.8', incline: '12' });
  await expect(card.getByTestId('cardio-total')).toContainText('15分');
  // 距離は 速度 × 時間（4.8 × 0.25h = 1.2km）
  await expect(card.getByTestId('cardio-total')).toContainText('1.2 km');

  await blurActive(page);
  await flushToStorage(page);
  const db = await readDb(page);
  const key = dateKey(new Date());
  const tm = db.exercises.find((e) => e.name === 'トレッドミル');
  expect(tm.group_id).toBe(db.groups.find((g) => g.name === '有酸素').id);
  const sets = db.sessions[key].logs.find((l) => l.exercise_id === tm.id).sets;
  // ★ 有酸素の行は weight / reps = 0。at は当日に打った行に入るので除いて見る
  expect(sets.map(({ at, ...s }) => s)).toEqual([
    { weight: 0, reps: 0, cardio: { sec: 900, kmh: 4.8, incline: 12 } },
  ]);
});

test('「+ 区間」は直前の行を写して時間欄にフォーカスする', async ({ page }) => {
  const card = await addExercise(page, 'トレッドミル');
  await fillRow(card.getByTestId('set-row').first(), { min: '15', kmh: '4.8', incline: '12' });
  await card.getByTestId('add-set').click();

  const rows = card.getByTestId('set-row');
  await expect(rows).toHaveCount(2);
  await expect(card.getByTestId('add-set')).toHaveText('+ 区間');
  const second = rows.nth(1);
  await expect(second.getByTestId('cardio-min')).toBeFocused();
  await expect(second.getByTestId('cardio-min')).toHaveValue('15');
  await expect(second.getByTestId('cardio-kmh')).toHaveValue('4.8');
  await expect(second.getByTestId('cardio-incline')).toHaveValue('12');
  // ドロップの口と重量の入れ忘れ警告は出ない
  await expect(card.getByTestId('drop-add')).toHaveCount(0);
  await expect(card.getByTestId('weight-missing')).toHaveCount(0);

  await second.getByTestId('cardio-incline').fill('15');
  await expect(card.getByTestId('cardio-total')).toContainText('30分');
});

test('全欄が空の行は保存されない', async ({ page }) => {
  const card = await addExercise(page, 'トレッドミル');
  await card.getByTestId('set-row').first().getByTestId('cardio-kmh').fill('4.8');
  await card.getByTestId('set-row').first().getByTestId('cardio-kmh').fill('');
  await blurActive(page);
  await flushToStorage(page);
  const db = await readDb(page);
  expect(db.sessions[dateKey(new Date())]?.logs ?? []).toEqual([]);
});

test('翌日に「前回をコピー」で区間の値が写る', async ({ page }) => {
  const card = await addExercise(page, 'トレッドミル');
  await fillRow(card.getByTestId('set-row').first(), { min: '15', kmh: '4.8', incline: '12' });
  await card.getByTestId('add-set').click();
  await card.getByTestId('set-row').nth(1).getByTestId('cardio-incline').fill('15');
  await blurActive(page);
  await flushToStorage(page);

  // 記録を昨日へずらして、今日を空にする
  await page.evaluate(
    ([k, today, yesterday]) => {
      const db = JSON.parse(localStorage.getItem(k));
      db.sessions[yesterday] = db.sessions[today];
      delete db.sessions[today];
      localStorage.setItem(k, JSON.stringify(db));
    },
    [STORAGE_KEY, dateKey(new Date()), dateKey(daysAgo(1))],
  );
  await page.goto('./');
  await expect(page.getByTestId('screen-record')).toBeVisible();

  const next = await addExercise(page, 'トレッドミル');
  await next.getByTestId('copy-last').click();
  const rows = next.getByTestId('set-row');
  await expect(rows).toHaveCount(2);
  await expect(rows.nth(0).getByTestId('cardio-incline')).toHaveValue('12');
  await expect(rows.nth(1).getByTestId('cardio-incline')).toHaveValue('15');
  await expect(rows.nth(1).getByTestId('cardio-min')).toHaveValue('15');
  await expect(next.getByTestId('cardio-total')).toContainText('30分');
});

test('TSV に書き出して読み戻すと区間がそのまま戻る', async ({ page }) => {
  await page.addInitScript(() => {
    Object.defineProperty(navigator, 'userAgent', {
      value: 'Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15',
      configurable: true,
    });
    Object.defineProperty(navigator, 'canShare', { value: () => true, configurable: true });
    Object.defineProperty(navigator, 'share', {
      configurable: true,
      value: async (data) => {
        window.__shared = { text: await data.files[0].text() };
      },
    });
  });
  await page.goto('./');
  const card = await addExercise(page, 'トレッドミル');
  await fillRow(card.getByTestId('set-row').first(), { min: '7.5', kmh: '4.8', incline: '0' });
  await blurActive(page);
  await flushToStorage(page);
  const before = await readDb(page);

  await page.getByTestId('tab-settings').click();
  await page.getByTestId('open-backup').click();
  await page.getByTestId('backup-export').click();
  await page.waitForFunction(() => !!window.__shared);
  const tsv = await page.evaluate(() => window.__shared.text);
  const header = tsv.split('\n')[0].split('\t');
  expect(header.slice(16)).toEqual(['有酸素秒', '有酸素m', '有酸素km/h', '有酸素傾斜%', '有酸素Lv']);
  // 有酸素の行は回数が空（旧版が回数 0 の行として読まない）
  const row = tsv.split('\n')[1].split('\t');
  expect(row[header.indexOf('回数')]).toBe('');
  expect(row[header.indexOf('有酸素秒')]).toBe('450');

  // 日付をずらして取り込む
  const today = dateKey(new Date());
  const moved = dateKey(daysAgo(30));
  await page.getByTestId('backup-file').setInputFiles({
    name: 'fitness-memo-20260801-1200.tsv',
    mimeType: TSV_MIME,
    buffer: Buffer.from(tsv.replaceAll(today, moved), 'utf-8'),
  });
  await page.getByTestId('backup-apply').click();
  const after = await readDb(page);
  const tm = after.exercises.find((e) => e.name === 'トレッドミル');
  const sets = (k) => after.sessions[k].logs.find((l) => l.exercise_id === tm.id).sets;
  expect(sets(moved).map(({ at, ...s }) => s)).toEqual(sets(today).map(({ at, ...s }) => s));
  expect(sets(moved)[0].cardio).toEqual({ sec: 450, kmh: 4.8, incline: 0 });
  expect(before.exercises.filter((e) => e.name === 'トレッドミル')).toHaveLength(1);
  expect(after.exercises.filter((e) => e.name === 'トレッドミル')).toHaveLength(1);
});

for (const width of [375, 393]) {
  test(`${width}px 幅でトレッドミルの 3 欄と ✕ が 1 行に収まる`, async ({ page }) => {
    await page.setViewportSize({ width, height: 800 });
    const card = await addExercise(page, 'トレッドミル');
    const row = card.getByTestId('set-row').first();
    await fillRow(row, { min: '120', kmh: '12.5', incline: '12' });
    await blurActive(page);
    const boxes = [];
    for (const id of ['cardio-min', 'cardio-kmh', 'cardio-incline', 'remove-set']) {
      const b = await row.getByTestId(id).boundingBox();
      expect(b, id).not.toBeNull();
      boxes.push(b);
    }
    const rowBox = await row.boundingBox();
    const centers = boxes.map((b) => b.y + b.height / 2);
    expect(Math.max(...centers) - Math.min(...centers), '同じ行にある').toBeLessThan(2);
    // 左から min → kmh → incline → ✕ の順で重ならず、行の中に収まる
    for (let i = 1; i < boxes.length; i++) {
      expect(boxes[i].x, `${i} 番目が前の欄と重ならない`).toBeGreaterThanOrEqual(
        boxes[i - 1].x + boxes[i - 1].width - 0.5,
      );
    }
    const last = boxes[boxes.length - 1];
    expect(last.x + last.width).toBeLessThanOrEqual(rowBox.x + rowBox.width + 0.5);
    expect(rowBox.height, '1 行の高さ（折り返していない）').toBeLessThan(60);
  });
}

test('既存の利用者には部位「有酸素」が 1 回だけ足され、消したら戻らない', async ({ page, context }) => {
  // 有酸素より前の版が書いたデータ（部位は胸だけ・種目 1 つ）
  // ★ 起動済みのページから書くと、閉じるときの保存が上書きする。次の起動の最初に 1 度だけ入れる
  await page.addInitScript((k) => {
    if (sessionStorage.getItem('seeded')) return;
    sessionStorage.setItem('seeded', '1');
    localStorage.removeItem('fitness-memo/ui/v1');
    localStorage.setItem(
      k,
      JSON.stringify({
        schema: 3,
        groups: [{ id: '000000000010', name: '胸', color: '#e0524a', order: 0 }],
        exercises: [
          { id: '000000000011', name: 'ベンチプレス', group_id: '000000000010', order: 0 },
        ],
        sessions: {},
      }),
    );
  }, STORAGE_KEY);
  await page.goto('./');
  await expect(page.getByTestId('screen-record')).toBeVisible();
  let db = await readDb(page);
  expect(db.groups.map((g) => g.name)).toEqual(['胸', '有酸素']);
  expect(db.exercises.filter((e) => e.group_id === db.groups[1].id)).toHaveLength(6);

  // 部位を消した状態を、新しいページの起動前に作る（開いたままのページの保存に上書きされない）
  await flushToStorage(page);
  await page.close();
  const next = await context.newPage();
  await next.addInitScript((k) => {
    if (sessionStorage.getItem('removed')) return;
    sessionStorage.setItem('removed', '1');
    const d = JSON.parse(localStorage.getItem(k));
    const g = d.groups.find((x) => x.name === '有酸素');
    d.groups = d.groups.filter((x) => x.id !== g.id);
    d.exercises = d.exercises.filter((e) => e.group_id !== g.id);
    localStorage.setItem(k, JSON.stringify(d));
  }, STORAGE_KEY);
  await next.goto('./');
  await expect(next.getByTestId('screen-record')).toBeVisible();
  db = await readDb(next);
  expect(db.groups.map((g) => g.name), '印があるので足し直さない').toEqual(['胸']);
});

test('推移タブで有酸素の種目を選ぶと指標が「時間 / 距離」になる', async ({ page }) => {
  const card = await addExercise(page, 'トレッドミル');
  await fillRow(card.getByTestId('set-row').first(), { min: '30', kmh: '4.8', incline: '12' });
  await blurActive(page);
  await flushToStorage(page);
  await page.getByTestId('tab-progress').click();
  await expect(page.getByTestId('metric-btn')).toHaveText(['時間', '距離']);
  await expect(page.getByTestId('metric-btn').first()).toHaveClass(/active/);
  await page.getByTestId('metric-btn').nth(1).click();
  await expect(page.getByTestId('stat-best')).toContainText('2.4 km');
});
