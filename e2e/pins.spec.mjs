import { test, expect } from '@playwright/test';

// マシンのピン（adr/ux/machine-pins-on-the-exercise.md）の E2E。
//
// ここでしか見られないのは **「種目に貼り付いて日をまたぐ」** こと。セットメモと種目メモは
// その日のログに属するので日を変えれば消えるが、ピンは同じ値が出続ける。この違いは
// `Db` の形を見るだけでは主張できず、画面を日付ごと切り替えて初めて確かめられる。
//
// 正規化（空白分割・上限・重複）とマージ（空のときだけ埋める）はホストの `cargo test` が
// 総当りしているので、ここでは追わない（reorder.spec.mjs と src/reorder.rs の分担と同じ）。

const STORAGE_KEY = 'fitness-memo/v3';

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

/** 保存されたその種目のピン。`Exercise` に載っている＝日に属していないことの証拠。 */
async function savedPins(page, name) {
  const db = await readDb(page);
  return db.exercises.find((e) => e.name === name)?.pins ?? [];
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

async function addExercise(page, name) {
  await blurActive(page);
  await page.getByTestId('add-exercise').click();
  await page
    .getByTestId('add-sheet')
    .getByTestId('pick-exercise')
    .filter({ hasText: exactText(name) })
    .click();
  return cardOf(page, name);
}

/**
 * ピン欄を開く。★ 専用のトグルは無く、種目メモと**同じ入口 1 つ**で一緒に開く
 * （カードに 44px のタップ標的を新設できる場所が無いため）。
 */
async function openPins(card) {
  await card.getByTestId('note-toggle').click();
  await expect(card.getByTestId('pin-box')).toBeVisible();
}

async function addPin(card, value) {
  await card.getByTestId('pin-add').click();
  await card.getByTestId('pin-value').last().fill(value);
}

// ── ★ 最優先: 種目に貼り付いて日をまたぐ ────────────────────────────────────

test('★ ピンは種目に貼り付き、日を変えても同じ値が出る（メモとの違い）', async ({ page }) => {
  const card = await addExercise(page, 'ベンチプレス');
  await openPins(card);
  await addPin(card, '3');
  await addPin(card, '5');
  await blurActive(page);
  await flushToStorage(page);

  // その日のログではなく `Db.exercises` に載る
  expect(await savedPins(page, 'ベンチプレス')).toEqual(['3', '5']);

  await page.reload();
  await expect(page.getByTestId('screen-record')).toBeVisible();
  await openDay(page, daysAgo(1));
  const again = await addExercise(page, 'ベンチプレス');

  // 別の日・リロード後でも、開かずに読める
  await expect(again.getByTestId('pin-read')).toHaveText('ピン 3・5');
});

// ── 開閉と静止時の見え方 ────────────────────────────────────────────────────

test('ピンは種目メモと同じトグル 1 つで開く', async ({ page }) => {
  const card = await addExercise(page, 'ベンチプレス');

  await expect(card.getByTestId('pin-box')).toHaveCount(0);
  await card.getByTestId('note-toggle').click();

  await expect(card.getByTestId('pin-box')).toBeVisible();
  // 種目メモも一緒に開いている（入口を分けていない）
  await expect(card.getByTestId('exercise-note')).toBeVisible();
});

test('閉じてもピンは薄字で読める。全部消すと薄字ごと消える', async ({ page }) => {
  const card = await addExercise(page, 'ベンチプレス');
  await openPins(card);
  await addPin(card, '3');
  await addPin(card, '5');
  await blurActive(page);

  await card.getByTestId('note-toggle').click();
  await expect(card.getByTestId('pin-box')).toHaveCount(0);
  await expect(card.getByTestId('pin-read')).toHaveText('ピン 3・5');

  // ★ 薄さは 12px + --muted で作る。opacity で薄くすると非テキストの 3:1 を割り、
  //   ダークで消える（adr/ux/exercise-and-set-notes-behind-one-toggle.md 決定 3）
  const style = await card.getByTestId('pin-read').evaluate((el) => {
    const cs = getComputedStyle(el);
    return { fontSize: cs.fontSize, opacity: cs.opacity };
  });
  expect(style).toEqual({ fontSize: '12px', opacity: '1' });

  await card.getByTestId('note-toggle').click();
  await card.getByTestId('pin-remove').first().click();
  await card.getByTestId('pin-remove').first().click();
  await card.getByTestId('note-toggle').click();

  await expect(card.getByTestId('pin-read')).toHaveCount(0);
});

// ── 編集 ────────────────────────────────────────────────────────────────────

test('✕ は押した 1 本だけを消す', async ({ page }) => {
  const card = await addExercise(page, 'ベンチプレス');
  await openPins(card);
  await addPin(card, '3');
  await addPin(card, '5');
  await addPin(card, '2');
  await blurActive(page);

  await card.getByTestId('pin-remove').nth(1).click();
  await flushToStorage(page);

  await expect(card.getByTestId('pin-value')).toHaveCount(2);
  expect(await savedPins(page, 'ベンチプレス')).toEqual(['3', '2']);
});

test('空にしたピンは保存されない', async ({ page }) => {
  const card = await addExercise(page, 'ベンチプレス');
  await openPins(card);
  await addPin(card, '3');
  // 値を入れないチップ。画面には残るが `Db` には落ちない（セットの空行と同じ規則）
  await card.getByTestId('pin-add').click();
  await blurActive(page);
  await flushToStorage(page);

  await expect(card.getByTestId('pin-value')).toHaveCount(2);
  expect(await savedPins(page, 'ベンチプレス')).toEqual(['3']);
});

test('ピンは 8 本まで。上限に達すると ＋ が消える', async ({ page }) => {
  const card = await addExercise(page, 'ベンチプレス');
  await openPins(card);
  for (let i = 0; i < 8; i++) {
    await card.getByTestId('pin-add').click();
  }

  await expect(card.getByTestId('pin-value')).toHaveCount(8);
  // ★ 押しても何も起きないボタンを残さない
  await expect(card.getByTestId('pin-add')).toHaveCount(0);
});

test('ピンの入力欄にフォーカスするとタブバーが隠れる', async ({ page }) => {
  const card = await addExercise(page, 'ベンチプレス');
  await openPins(card);
  await card.getByTestId('pin-add').click();
  await card.getByTestId('pin-value').first().focus();

  // kb_focus / kb_blur を付け忘れると iOS でタブバーが入力域に被る
  await expect(page.locator('.app')).toHaveClass(/kb-open/);
  await expect(page.getByTestId('bottom-tabs')).toBeHidden();
});

// ── 入口を増やしていないこと ────────────────────────────────────────────────

test('ピンのために新しいタップ標的を 1 つも増やしていない', async ({ page }) => {
  const card = await addExercise(page, 'ベンチプレス');

  // ヘッダは掴み口。ここにボタンを置くとカードが枚数分縦に伸び、誤タップの列にも入る
  await expect(card.locator('.card-head button')).toHaveCount(0);
  // フッタは「この日から外す」「メモ」の 2 つのまま（ピン専用のトグルを作っていない）
  await expect(card.locator('.card-foot button')).toHaveCount(2);
});
