import { test, expect } from '@playwright/test';

// セットごとの時刻（SetEntry.at）と日ヘッダの開始–終了（day-span）。
// adr/data-model/set-at-typed-today-only.md /
// adr/ux/day-time-span-in-the-header-and-set-times-behind-the-memo-toggle.md
//
// ここでしか見られないのは 7 つ。
//
// - **当日に重量か回数を打った行にだけ `SetEntry.at` が入り、保存 JSON のキー集合が
//   `['at','reps','weight']` に固定されること。** `stamp_set_at` の分岐そのものは
//   core.rs のユニットテストが持つが、`day.rs` の `on:input` から実際に呼ばれ、
//   `fitness-memo/v3` に届くかは E2E にしか出ない
// - **「前回をコピー」で入った行には `at` が付かず、打ち直した行にだけ付くこと。**
//   `day-span` の 0 件 → 1 件という見え方の変化ごと確認する
// - **メモを開いたときだけ `set-at` が出て、打った行と手つかずの行で表示が分かれること**
// - **一度入った時刻がリロード後も残り、同じ値を打ち直しても動かないこと**
// - **過去日はセットに `at` が付かず、`day-span` も出ないこと。**
//   `seedPastLogs` を、セットの要素にも `at` を持たせられる形で使う
// - **過去日に UI で新規記録してもセットに `at` キーが増えないこと**
//   （`calendar.spec.mjs` の同趣旨のログ側の主張の、セット側の対応）
// - **`day-span` があるとき `.day-head` が 1 行に収まること。** iPhone 15 Pro
//   プロジェクトで実際に幅が詰まったときに意味がある
//
// 期待する表示文字列は保存済みの `at`（epoch ms）を Node 側で
// `Intl.DateTimeFormat` で組んで完全一致させる（分境界の flake が消え
// 「表示 = 保存値」まで主張できる）。

const STORAGE_KEY = 'fitness-memo/v3';
const WEEKDAY_JA = ['日', '月', '火', '水', '木', '金', '土'];

test.beforeEach(async ({ page }) => {
  // ★ baseURL がサブパス（/fitness-memo/）を持つとき先頭 "/" はベースを丸ごと捨てる
  await page.goto('./');
  await expect(page.getByTestId('screen-record')).toBeVisible();
});

// ── ヘルパ（共有モジュールは意図的に持たない。各スペックに複製する） ──────────

function exactText(s) {
  return new RegExp(`^${s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}$`);
}

async function blurActive(page) {
  await page.evaluate(() => {
    if (document.activeElement instanceof HTMLElement) document.activeElement.blur();
  });
}

/** 「種目を追加」シートからプリセットを選び、追加されたカードを返す。 */
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

/**
 * hidden への visibilitychange を発火させ、pending の debounce 保存を即時 flush する。
 * 単純な reload だけだと 400ms debounce と race して flaky になる。
 */
async function flushToStorage(page) {
  await page.getByTestId('screen-record').waitFor({ state: 'visible' });
  await page.evaluate(() => {
    Object.defineProperty(document, 'hidden', { value: true, configurable: true });
    document.dispatchEvent(new Event('visibilitychange', { bubbles: true }));
  });
}

async function readDb(page) {
  await flushToStorage(page);
  return page.evaluate((key) => JSON.parse(localStorage.getItem(key)), STORAGE_KEY);
}

/** メモ欄を開く。トグルなので開閉状態を見てから押す。 */
async function openNotes(page, card) {
  await blurActive(page);
  const toggle = card.getByTestId('note-toggle');
  if ((await toggle.getAttribute('aria-expanded')) !== 'true') await toggle.click();
}

/** views/mod.rs の `fmt_clock` と同じ "HH:MM"（JST）。表示文字列の唯一の正典。 */
function clockOf(atMs) {
  return new Intl.DateTimeFormat('en-GB', {
    hour: '2-digit',
    minute: '2-digit',
    hourCycle: 'h23',
    timeZone: 'Asia/Tokyo',
  }).format(new Date(atMs));
}

// ── カレンダー日付ヘルパ（e2e/calendar.spec.mjs から複製） ────────────────────

function dateKey(d) {
  const m = String(d.getMonth() + 1).padStart(2, '0');
  const day = String(d.getDate()).padStart(2, '0');
  return `${d.getFullYear()}-${m}-${day}`;
}

function fmtDate(d) {
  return `${d.getMonth() + 1}/${d.getDate()} (${WEEKDAY_JA[d.getDay()]})`;
}

function fmtMonth(d) {
  return `${d.getFullYear()}年${d.getMonth() + 1}月`;
}

function daysAgo(n) {
  const d = new Date();
  d.setDate(d.getDate() - n);
  return d;
}

/** その日のローカル hh:mm の epoch。`dateKey` と同じくブラウザ / Node 両方の TZ で計算される。 */
function atOnDay(daysAgoN, hour, minute) {
  const d = daysAgo(daysAgoN);
  d.setHours(hour, minute, 0, 0);
  return d.getTime();
}

function shownMonthIndex(title) {
  const m = title.match(/^(\d+)年(\d+)月$/);
  if (!m) throw new Error(`cal-title の書式が想定外: ${title}`);
  return Number(m[1]) * 12 + (Number(m[2]) - 1);
}

async function switchTab(page, testid) {
  await blurActive(page);
  await page.getByTestId(testid).click();
}

async function openCalendarOn(page, date) {
  await switchTab(page, 'tab-record');
  await expect(page.getByTestId('screen-record')).toBeVisible();

  const want = date.getFullYear() * 12 + date.getMonth();
  const shown = shownMonthIndex(await page.getByTestId('cal-title').textContent());
  const delta = want - shown;
  const button = delta < 0 ? 'cal-prev' : 'cal-next';
  for (let i = 0; i < Math.abs(delta); i++) {
    await page.getByTestId(button).click();
  }
  await expect(page.getByTestId('cal-title')).toHaveText(fmtMonth(date));
}

const dayCell = (page, date) =>
  page.locator(`[data-testid="cal-day"][data-date="${dateKey(date)}"]`);

async function openDay(page, date) {
  await openCalendarOn(page, date);
  await dayCell(page, date).click();
  await expect(page.getByTestId('today-date')).toHaveText(fmtDate(date));
}

async function fillSet(card, index, { weight, reps }) {
  const row = card.getByTestId('set-row').nth(index);
  if (weight !== undefined) await row.getByTestId('set-weight').fill(String(weight));
  await row.getByTestId('set-reps').fill(String(reps));
}

/**
 * 投入済みプリセットの Db に daysAgo 日前のセッションを追加してから reload する。
 * `e2e/smoke.spec.mjs` の同名関数の複製。**このファイルでの拡張点は呼び出し側にある** —
 * `sets` の要素にそのまま `at`（`atOnDay` で組んだ epoch ms）を含めれば、その値が
 * `SetEntry.at` としてそのまま保存される（このファイル自体は `sets` を素通しするだけ
 * で、要素の形を決め打ちしていない）。
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

    for (const { daysAgo, exerciseName, sets, at = null } of entries) {
      const key = dateKey(daysAgo);
      const session = db.sessions[key] ?? { logs: [], body_weight: null, note: '' };
      if (exerciseName !== undefined) {
        const ex = db.exercises.find((e) => e.name === exerciseName);
        if (!ex) throw new Error(`preset exercise not found: ${exerciseName}`);
        session.logs.push({ exercise_id: ex.id, sets, at });
      }
      db.sessions[key] = session;
    }
    localStorage.setItem(KEY, JSON.stringify(db));
  }, entries);
  await page.reload();
}

// ── 1. 当日に回数を打つ ───────────────────────────────────────────────────────

test('1. 当日に回数を打つとそのセットに at が入り、day-span がその HH:MM になる', async ({
  page,
}) => {
  const before = Date.now();
  const card = await addExercise(page, 'ベンチプレス');
  await card.getByTestId('set-weight').first().fill('60');
  await card.getByTestId('set-reps').first().fill('10');
  await blurActive(page);

  const db = await readDb(page);
  const set = Object.values(db.sessions)[0].logs[0].sets[0];
  expect(Object.keys(set).sort(), 'キー集合は at・reps・weight だけ').toEqual([
    'at',
    'reps',
    'weight',
  ]);
  expect(typeof set.at, 'at は epoch ms').toBe('number');
  expect(set.at).toBeGreaterThanOrEqual(before - 60_000);
  expect(set.at).toBeLessThanOrEqual(Date.now() + 60_000);

  await expect(page.getByTestId('day-span')).toHaveText(clockOf(set.at));
});

// ── 2. 「前回をコピー」は at を運ばない ────────────────────────────────────────

test('2. 「前回をコピー」は at を運ばず、打ち直した行だけ時刻が付く', async ({ page }) => {
  await seedPastLogs(page, [
    {
      daysAgo: 3,
      exerciseName: 'ベンチプレス',
      sets: [
        { weight: 60, reps: 10 },
        { weight: 60, reps: 8 },
      ],
    },
  ]);
  const card = await addExercise(page, 'ベンチプレス');
  await card.getByTestId('copy-last').click();
  await blurActive(page);

  // コピー直後: 全行 at キー無し、day-span は 0 件（ログの at はコピー時刻のまま数値）。
  // ★ 過去日（コピー元）のセッションと今日のセッションが両方存在するので、
  //   `dateKey` で今日のぶんを名指しする（`sets.length` などの形で引き当てない）
  const todayKey = dateKey(new Date());
  let db = await readDb(page);
  let session = db.sessions[todayKey];
  expect(session.logs[0].sets.every((s) => !('at' in s)), 'コピー直後は at キーが無い').toBe(
    true,
  );
  expect(typeof session.logs[0].at, 'ログの at はコピー時刻のまま数値').toBe('number');
  await expect(page.getByTestId('day-span')).toHaveCount(0);

  // 1 行だけ回数を打ち直す → その行だけ at、day-span はその HH:MM
  await card.getByTestId('set-row').nth(0).getByTestId('set-reps').fill('9');
  await blurActive(page);

  db = await readDb(page);
  session = db.sessions[todayKey];
  const [first, second] = session.logs[0].sets;
  expect(typeof first.at, '打ち直した行には at が入る').toBe('number');
  expect('at' in second, '手つかずの行には at キーが無い').toBe(false);
  await expect(page.getByTestId('day-span')).toHaveText(clockOf(first.at));
});

// ── 3. メモを開いたときだけ set-at が出る ──────────────────────────────────────

test('3. メモを開くと set-at が行数ぶん出て、打った行だけ時刻が読める', async ({ page }) => {
  await seedPastLogs(page, [
    {
      daysAgo: 3,
      exerciseName: 'ベンチプレス',
      sets: [
        { weight: 60, reps: 10 },
        { weight: 60, reps: 8 },
      ],
    },
  ]);
  const card = await addExercise(page, 'ベンチプレス');
  await card.getByTestId('copy-last').click();
  await blurActive(page);
  // コピーのままの 2 行目には触らず、1 行目だけ打ち直す
  await card.getByTestId('set-row').nth(0).getByTestId('set-reps').fill('9');
  await blurActive(page);

  await expect(page.getByTestId('set-at')).toHaveCount(0);

  await openNotes(page, card);
  const setAt = card.getByTestId('set-at');
  await expect(setAt).toHaveCount(2);

  const db = await readDb(page);
  const session = db.sessions[dateKey(new Date())];
  const [first] = session.logs[0].sets;

  await expect(setAt.nth(0)).toHaveText(clockOf(first.at));
  await expect(setAt.nth(1)).toHaveText('');

  // 開いた set-note の左端 x が閉じた set-note-read と 1px 以内
  // （e2e/smoke.spec.mjs「メモの薄字と入力欄は左端がそろう」と同じ主張。
  //   .set-at を右に追加した後もメモ欄の左端は動かないことを確認する）。
  // ★ set-note-read は空メモの行には描画されないので、閉じる前に何か書く
  await card.getByTestId('set-note').first().fill('きつい');
  await blurActive(page);
  const open = await card.getByTestId('set-note').first().boundingBox();
  const toggle = card.getByTestId('note-toggle');
  await toggle.click();
  const closed = await card.getByTestId('set-note-read').first().boundingBox();
  expect(Math.abs(open.x - closed.x), '.set-at を足して開閉で文字の左端が動く').toBeLessThanOrEqual(
    1,
  );

  await expect(page.getByTestId('set-at')).toHaveCount(0);
});

// ── 4. 一度入った時刻はリロードや打ち直しで動かない ─────────────────────────────

test('4. 一度入った時刻はリロード後も残り、同じ回数を打ち直しても動かない', async ({ page }) => {
  const card = await addExercise(page, 'ベンチプレス');
  await card.getByTestId('set-weight').first().fill('60');
  await card.getByTestId('set-reps').first().fill('10');
  await blurActive(page);

  let db = await readDb(page);
  const firstAt = Object.values(db.sessions)[0].logs[0].sets[0].at;

  await page.reload();
  await expect(page.getByTestId('screen-record')).toBeVisible();
  await expect(page.getByTestId('set-at')).toHaveCount(0);
  await expect(page.getByTestId('day-span')).toHaveText(clockOf(firstAt));

  const reloadedCard = page.getByTestId('exercise-card');
  await reloadedCard.getByTestId('set-reps').first().fill('10');
  await blurActive(page);

  db = await readDb(page);
  const secondAt = Object.values(db.sessions)[0].logs[0].sets[0].at;
  expect(secondAt, '同じ値を打ち直しても at は据え置き').toBe(firstAt);
});

// ── 5. 過去日: セットの at はセットの at だけから出る ────────────────────────────

test('5. 過去日のセット時刻は day-span に出て、打ち直しても増えない', async ({ page }) => {
  const target = daysAgo(3);
  await seedPastLogs(page, [
    {
      daysAgo: 3,
      exerciseName: 'ベンチプレス',
      sets: [
        { weight: 60, reps: 10, at: atOnDay(3, 19, 2) },
        { weight: 60, reps: 8, at: atOnDay(3, 19, 14) },
      ],
    },
  ]);

  await openDay(page, target);
  await expect(page.getByTestId('day-span')).toHaveText('19:02–19:14');

  const card = page.getByTestId('exercise-card');
  await card.getByTestId('set-row').nth(0).getByTestId('set-reps').fill('9');
  await blurActive(page);

  // ★ 過去日は is_today が偽なので stamp_set_at は据え置く。打ち直しても
  //   day-span は動かない
  await expect(page.getByTestId('day-span')).toHaveText('19:02–19:14');

  const db = await readDb(page);
  const sets = db.sessions[dateKey(target)].logs[0].sets;
  expect(sets[0].reps, '回数自体は打ち直せる').toBe(9);
  expect(sets[0].at, '過去日は打ち直しても at が動かない').toBe(atOnDay(3, 19, 2));

  // 過去日で新規に足した行には at キーが増えない（次のケースと同じ主張をここでも見る）
  await card.getByTestId('add-set').click();
  await card.getByTestId('set-row').nth(2).getByTestId('set-reps').fill('6');
  await blurActive(page);
  const after = await readDb(page);
  const newRow = after.sessions[dateKey(target)].logs[0].sets[2];
  expect('at' in newRow, '過去日の新規行には at キーが無い').toBe(false);
});

// ── 6. 過去日に UI で新規記録してもセットに at が付かない ─────────────────────────

test('6. 過去日に UI で新規記録すると、セットに at キーが増えない', async ({ page }) => {
  const target = daysAgo(5);

  await openDay(page, target);
  await expect(page.getByTestId('past-banner')).toBeVisible();

  const card = await addExercise(page, 'スクワット');
  await fillSet(card, 0, { weight: 80, reps: 5 });
  await blurActive(page);

  const db = await readDb(page);
  const set = db.sessions[dateKey(target)].logs[0].sets[0];
  expect(
    'at' in set,
    '過去日バックフィルの ExerciseLog.at は null になる（calendar.spec.mjs）が、' +
      'セット単位の at はそもそもキーごと出てはいけない',
  ).toBe(false);
  await expect(page.getByTestId('day-span')).toHaveCount(0);
});

// ── 7. day-span があるとき .day-head は 1 行に収まる（iPhone 15 Pro で意味がある） ──

test('7. day-span があっても .day-head は 1 行に収まる', async ({ page }) => {
  const card = await addExercise(page, 'ベンチプレス');
  await card.getByTestId('set-weight').first().fill('60');
  await card.getByTestId('set-reps').first().fill('10');
  await blurActive(page);

  await expect(page.getByTestId('day-span')).toBeVisible();

  const { headH, h2H } = await page.evaluate(() => {
    const head = document.querySelector('.day-head');
    const h2 = head.querySelector('h2');
    return {
      headH: head.getBoundingClientRect().height,
      h2H: h2.getBoundingClientRect().height,
    };
  });
  expect(Math.abs(headH - h2H), '.day-head が 2 行に折り返している').toBeLessThanOrEqual(1);
});
