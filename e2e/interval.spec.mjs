import { test, expect } from '@playwright/test';

// セット間のインターバル（adr/ux/interval-seconds-on-the-exercise.md）の E2E。
//
// ここでしか見られないのは **「種目に貼り付いて日をまたぐ」** ことと、
// **「ピンの下に置いても入口が増えていない」** こと。前者はセットメモ・種目メモとの
// 決定的な違いで、画面を日付ごと切り替えて初めて確かめられる。後者は
// `.card-head` / `.card-foot` のボタン数という構造の契約。
//
// 正規化（上限の丸め・0 の扱い）とマージ（未設定のときだけ埋める）と TSV の往復は
// ホストの `cargo test` が総当りしているので、ここでは追わない
// （pins.spec.mjs と src/core.rs の分担と同じ）。

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

/**
 * インターバル欄を開く。★ 専用のトグルは無く、ピンと種目メモと**同じ入口 1 つ**で
 * 一緒に開く（カードに 44px のタップ標的を新設できる場所が無いため）。
 */
async function openInterval(card) {
  await card.getByTestId('note-toggle').click();
  await expect(card.getByTestId('interval-box')).toBeVisible();
}

async function setInterval_(card, value) {
  await card.getByTestId('interval-value').fill(value);
}

// ── ★ 最優先: 種目に貼り付いて日をまたぐ ────────────────────────────────────

test('★ インターバルは種目に貼り付き、日を変えても同じ値が出る（メモとの違い）', async ({
  page,
}) => {
  const card = await addExercise(page, 'ベンチプレス');
  await openInterval(card);
  await setInterval_(card, '90');
  await blurActive(page);
  await flushToStorage(page);

  // その日のログではなく `Db.exercises` に載る
  expect(await savedInterval(page, 'ベンチプレス')).toBe(90);

  await page.reload();
  await expect(page.getByTestId('screen-record')).toBeVisible();
  await openDay(page, daysAgo(1));
  const again = await addExercise(page, 'ベンチプレス');

  // 別の日・リロード後でも、開かずに読める
  await expect(again.getByTestId('interval-read')).toHaveText('インターバル 90秒');
});

// ── 開閉と静止時の見え方 ────────────────────────────────────────────────────

test('インターバルはピン・種目メモと同じトグル 1 つで開く', async ({ page }) => {
  const card = await addExercise(page, 'ベンチプレス');

  await expect(card.getByTestId('interval-box')).toHaveCount(0);
  await card.getByTestId('note-toggle').click();

  await expect(card.getByTestId('interval-box')).toBeVisible();
  // ピンと種目メモも一緒に開いている（入口を分けていない）
  await expect(card.getByTestId('pin-box')).toBeVisible();
  await expect(card.getByTestId('exercise-note')).toBeVisible();
});

test('閉じてもインターバルは薄字で読める。空にすると薄字ごと消える', async ({ page }) => {
  const card = await addExercise(page, 'ベンチプレス');
  await openInterval(card);
  await setInterval_(card, '180');
  await blurActive(page);

  await card.getByTestId('note-toggle').click();
  await expect(card.getByTestId('interval-box')).toHaveCount(0);
  await expect(card.getByTestId('interval-read')).toHaveText('インターバル 180秒');

  // ★ 薄さは 12px + --muted で作る。opacity で薄くすると非テキストの 3:1 を割り、
  //   ダークで消える（adr/ux/exercise-and-set-notes-behind-one-toggle.md 決定 3）
  const style = await card.getByTestId('interval-read').evaluate((el) => {
    const cs = getComputedStyle(el);
    return { fontSize: cs.fontSize, opacity: cs.opacity };
  });
  expect(style).toEqual({ fontSize: '12px', opacity: '1' });

  // 空欄まで戻せる。戻せないと間違って打った秒数が薄字に永久に残る
  await card.getByTestId('note-toggle').click();
  await setInterval_(card, '');
  await blurActive(page);
  await flushToStorage(page);
  await card.getByTestId('note-toggle').click();

  await expect(card.getByTestId('interval-read')).toHaveCount(0);
  expect(await savedInterval(page, 'ベンチプレス')).toBeUndefined();
});

test('薄字はピンの下に出る（DOM 順が「準備 → 観測」）', async ({ page }) => {
  const card = await addExercise(page, 'ベンチプレス');
  await openInterval(card);
  await card.getByTestId('pin-add').click();
  await card.getByTestId('pin-value').fill('3');
  await setInterval_(card, '90');
  await blurActive(page);
  await card.getByTestId('note-toggle').click();

  // ★ 3 行の順序が「ピン → インターバル → 種目メモ」であること。順が入れ替わると
  //   「その種目の設定」と「その日のこと」の 2 段が崩れる
  const order = await card.evaluate((el) =>
    [...el.querySelectorAll('[data-testid]')]
      .map((n) => n.dataset.testid)
      .filter((t) => ['pin-read', 'interval-read'].includes(t)),
  );
  expect(order).toEqual(['pin-read', 'interval-read']);
});

// ── 入力 ────────────────────────────────────────────────────────────────────

test('数字キーパッドが出る欄で、整数の秒だけを受ける', async ({ page }) => {
  const card = await addExercise(page, 'ベンチプレス');
  await openInterval(card);
  const input = card.getByTestId('interval-value');

  // ★ type="number" は中間状態が読めないので使わない（adr/ux/text-input-not-number.md）。
  //   テンキーは inputmode が決める
  await expect(input).toHaveAttribute('type', 'text');
  await expect(input).toHaveAttribute('inputmode', 'numeric');
  // 999 秒（3 桁）まで。打てるのに保存で丸められる欄にしない
  await expect(input).toHaveAttribute('maxlength', '3');
});

test('0 秒は「未設定」ではなく入っている値', async ({ page }) => {
  const card = await addExercise(page, 'ベンチプレス');
  await openInterval(card);
  await setInterval_(card, '0');
  await blurActive(page);
  await flushToStorage(page);

  // ★ 0 を落とすとスーパーセット（休まず次のセットへ）の設定が黙って消える
  expect(await savedInterval(page, 'ベンチプレス')).toBe(0);
  await card.getByTestId('note-toggle').click();
  await expect(card.getByTestId('interval-read')).toHaveText('インターバル 0秒');
});

test('インターバルの入力欄にフォーカスするとタブバーが隠れる', async ({ page }) => {
  const card = await addExercise(page, 'ベンチプレス');
  await openInterval(card);
  await card.getByTestId('interval-value').focus();

  // kb_focus / kb_blur を付け忘れると iOS でタブバーが入力域に被る
  await expect(page.locator('.app')).toHaveClass(/kb-open/);
  await expect(page.getByTestId('bottom-tabs')).toBeHidden();
});

// ── 入口を増やしていないこと ────────────────────────────────────────────────

test('インターバルのために新しいタップ標的を 1 つも増やしていない', async ({ page }) => {
  const card = await addExercise(page, 'ベンチプレス');

  // ヘッダは掴み口。ここにボタンを置くとカードが枚数分縦に伸び、誤タップの列にも入る
  await expect(card.locator('.card-head button')).toHaveCount(0);
  // フッタは「この日から外す」「メモ」の 2 つのまま（インターバル専用のトグルを作っていない）
  await expect(card.locator('.card-foot button')).toHaveCount(2);

  // 開いても増えない（ピンの ＋ / ✕ 以外にボタンを足していない）
  await openInterval(card);
  await expect(card.getByTestId('interval-box').locator('button')).toHaveCount(0);
});
