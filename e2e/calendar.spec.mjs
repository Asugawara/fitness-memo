import { test, expect } from '@playwright/test';

// カレンダータブ（src/views/calendar.rs）の E2E。計画の smoke ケース 8 を含む。
//
// このファイルの存在意義が 2 つある。
//
// 1. 要件「カレンダーに対して、どの日付にどの筋トレ項目をしたかを追加できる」の検証。
//    記録が無い日をタップ →「この日に記録する」→ その日付で今日タブが開く → 実際に
//    記録できる、という一連が成立しなければ要件を満たさない。
//
// 2. **`ExerciseLog.at: Option<i64>` の設計そのものの検証。**
//    smoke のケース 4・7 は「前日の記録がある状態」を localStorage への直接注入で
//    作っていた（calendar.rs が入るまで、today タブ単体に過去日へ移動する導線が
//    無かったため）。つまり「UI から過去日に書き込んだときに at が入らない」という
//    書き込み側の不変条件は、これまで一度も検証されていない。
//    ここに now が入ると「最後のトレーニングから」が「たった今」になり、明示要件
//    （最後のトレーニングからの経過時間）の出力が嘘になる。
//
// 既知の制約: 日付は実行時刻の「今日」を基準にする。テストが日を跨いだ瞬間に走ると
// Node 側の new Date() とブラウザ側の Local::now() が 1 日ズレうる（smoke も同じ）。

const STORAGE_KEY = 'fitness-memo/v1';
const WEEKDAY_JA = ['日', '月', '火', '水', '木', '金', '土'];

test.beforeEach(async ({ page }) => {
  // ★ baseURL がサブパス（/fitness-memo/）を持つとき先頭 "/" はベースを丸ごと捨てる。
  //   相対参照の "./" でなければ E2E_BASE 指定の重い側実行が壊れる
  await page.goto('./');
  await expect(page.getByTestId('screen-today')).toBeVisible();
});

// ── 日付ヘルパ ──────────────────────────────────────────────────────────────

/** `Local::now().date_naive()` と同じローカル日付キー。UTC(toISOString) は使わない */
function dateKey(d) {
  const m = String(d.getMonth() + 1).padStart(2, '0');
  const day = String(d.getDate()).padStart(2, '0');
  return `${d.getFullYear()}-${m}-${day}`;
}

/** views/mod.rs の `fmt_date` と同じ "8/8 (金)" */
function fmtDate(d) {
  return `${d.getMonth() + 1}/${d.getDate()} (${WEEKDAY_JA[d.getDay()]})`;
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

// ── 操作ヘルパ ──────────────────────────────────────────────────────────────

// hasText は部分一致なので "ベンチプレス" が "インクラインベンチプレス" にも
// マッチする。pick-exercise は全プリセット共有の testid なので常に完全一致にする
function exactText(s) {
  return new RegExp(`^${s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}$`);
}

/**
 * タブを切り替える。
 *
 * ★ 入力欄にフォーカスが残っていると `.app` に `kb-open` が付き、styles.css の
 *   `.kb-open .bottom-tabs { display: none }` でタブバーごと消える（iOS の
 *   キーボード対策なので設計どおり）。blur せずに押すとクリックがタイムアウトする。
 *   150ms の解除 debounce は click の自動待機が吸収する。
 */
async function switchTab(page, testid) {
  await page.evaluate(() => {
    if (document.activeElement instanceof HTMLElement) document.activeElement.blur();
  });
  await page.getByTestId(testid).click();
}

/** 「種目を追加」シートからプリセットを選び、その種目のカードを返す。 */
async function addExercise(page, name) {
  await page.getByTestId('add-exercise').click();
  await page
    .getByTestId('add-sheet')
    .getByTestId('pick-exercise')
    .filter({ hasText: exactText(name) })
    .click();
  // カード名で絞る（複数カードを並べるテストがあるため nth では特定できない）
  return page
    .getByTestId('exercise-card')
    .filter({ has: page.getByTestId('card-name').filter({ hasText: exactText(name) }) });
}

/** カードの index 行目にセットを入力する。weight 省略時はレップだけ入れる。 */
async function fillSet(card, index, { weight, reps }) {
  const row = card.getByTestId('set-row').nth(index);
  if (weight !== undefined) await row.getByTestId('set-weight').fill(String(weight));
  await row.getByTestId('set-reps').fill(String(reps));
}

/**
 * hidden への visibilitychange で pending の debounce 保存を即時 flush する。
 * 400ms 待つだけだと debounce と race して flaky になる。
 */
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

function shownMonthIndex(title) {
  const m = title.match(/^(\d+)年(\d+)月$/);
  if (!m) throw new Error(`cal-title の書式が想定外: ${title}`);
  return Number(m[1]) * 12 + (Number(m[2]) - 1);
}

/**
 * カレンダータブを開き、指定日の月まで前後ナビで寄せる。
 *
 * カレンダーは `dates.selected` の月から開くので、過去日を編集した後は既にその月に
 * いることがある。必要な回数だけ押してから、最後にタイトルで着地を待つ
 * （クリック直後は DOM が更新前のことがあるので、逐次読み取りはしない）。
 */
async function openCalendarOn(page, date) {
  await switchTab(page, 'tab-calendar');
  await expect(page.getByTestId('screen-calendar')).toBeVisible();

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

/** カレンダーの空日から今日タブをその日付で開く。 */
async function openDayInToday(page, date) {
  await openCalendarOn(page, date);
  await dayCell(page, date).click();
  await page.getByTestId('cal-open-day').click();
  await expect(page.getByTestId('screen-today')).toBeVisible();
  await expect(page.getByTestId('today-date')).toHaveText(fmtDate(date));
}

// ── ★ 最優先: at: Option<i64> の書き込み側の検証 ────────────────────────────

test('★ UI から過去日に記録すると ExerciseLog.at が null になり、経過表示が日数粒度に落ちる', async ({
  page,
}) => {
  const yesterday = daysAgo(1);

  await openDayInToday(page, yesterday);
  await expect(page.getByTestId('past-banner')).toBeVisible();

  const card = await addExercise(page, 'ベンチプレス');
  await fillSet(card, 0, { weight: 60, reps: 10 });
  await expect(card.getByTestId('today-metric')).toHaveText('600 kg·回');

  await flushToStorage(page);

  // ── ここが本題。保存された JSON を直接見る ────────────────────────────
  const db = await readDb(page);
  const session = db.sessions[dateKey(yesterday)];
  expect(session, `${dateKey(yesterday)} のセッションが保存されていること`).toBeTruthy();
  expect(session.logs).toHaveLength(1);
  expect(session.logs[0].sets).toEqual([{ weight: 60, reps: 10 }]);

  // ★ 過去日バックフィルは at を埋めない。ここに now が入ると
  //   「最後のトレーニングから」が「たった今」になり、明示要件の出力が嘘になる
  expect(
    session.logs[0].at,
    '過去日にバックフィルした ExerciseLog の at は None（null）でなければならない',
  ).toBeNull();

  // 表示側も日粒度に落ちていること（Elapsed::Days 分岐）
  await page.getByTestId('back-to-today').click();
  await expect(page.getByTestId('past-banner')).toHaveCount(0);
  await expect(page.getByTestId('elapsed')).toHaveText('昨日');
});

test('当日に記録したときは at に epoch ms が入る（過去日との対照）', async ({ page }) => {
  const before = Date.now();

  const card = await addExercise(page, 'ベンチプレス');
  await fillSet(card, 0, { weight: 60, reps: 10 });
  await flushToStorage(page);

  const db = await readDb(page);
  const log = db.sessions[dateKey(new Date())].logs[0];

  // at が「常に null」なら要件「最後のトレーニングから 2日5時間」の時間粒度が出ない。
  // 過去日で null・当日で数値、の両方が揃って初めて Option の設計が効いている
  expect(typeof log.at, 'at は当日入力では epoch ms').toBe('number');
  expect(log.at).toBeGreaterThan(before - 60_000);
  expect(log.at).toBeLessThan(Date.now() + 60_000);
});

// ── ★ 要件の核心: カレンダーから記録を追加する ──────────────────────────────

test('★ 記録が無い日をタップして「この日に記録する」から追加すると、その日が実施日になる', async ({
  page,
}) => {
  const target = daysAgo(3);

  await openCalendarOn(page, target);
  const cell = dayCell(page, target);
  await expect(cell).toHaveAttribute('data-trained', 'false');
  await cell.click();

  // 記録が無い日でも必ず「記録なし」と導線が出る（ここが要件の入口）
  await expect(page.getByTestId('cal-empty')).toHaveText('記録なし');
  await expect(page.getByTestId('cal-logs')).toHaveCount(0);
  const open = page.getByTestId('cal-open-day');
  await expect(open).toHaveAttribute('data-mode', 'new');
  await expect(open).toHaveText('この日に記録する');
  await open.click();

  // 今日タブがその日付で開き、過去日を編集中であることが分かる
  await expect(page.getByTestId('screen-today')).toBeVisible();
  await expect(page.getByTestId('tab-today')).toHaveClass(/active/);
  await expect(page.getByTestId('today-date')).toHaveText(fmtDate(target));
  await expect(page.getByTestId('past-banner')).toContainText(`${fmtDate(target)} を編集中`);

  const card = await addExercise(page, 'スクワット');
  await fillSet(card, 0, { weight: 80, reps: 5 });
  await expect(card.getByTestId('today-metric')).toHaveText('400 kg·回');

  // カレンダーに戻るとその日が実施日になり、サマリと「この日を編集」に変わる
  await openCalendarOn(page, target);
  await expect(dayCell(page, target)).toHaveAttribute('data-trained', 'true');
  await expect(dayCell(page, target).getByTestId('cal-dot')).toHaveCount(1);

  await dayCell(page, target).click();
  const log = page.getByTestId('cal-log');
  await expect(log).toHaveCount(1);
  await expect(log).toContainText('スクワット');
  await expect(log).toContainText('80×5');
  await expect(log).toContainText('400 kg·回');
  await expect(page.getByTestId('cal-empty')).toHaveCount(0);
  await expect(page.getByTestId('cal-open-day')).toHaveAttribute('data-mode', 'edit');
});

test('記録がある日は「この日を編集」で開き直せ、既存のセットが復元される', async ({ page }) => {
  const target = daysAgo(2);

  await openDayInToday(page, target);
  const card = await addExercise(page, 'ベンチプレス');
  await fillSet(card, 0, { weight: 55, reps: 12 });
  await flushToStorage(page);

  await openCalendarOn(page, target);
  await dayCell(page, target).click();
  const open = page.getByTestId('cal-open-day');
  await expect(open).toHaveText('この日を編集');
  await open.click();

  await expect(page.getByTestId('today-date')).toHaveText(fmtDate(target));
  const restored = page.getByTestId('exercise-card');
  await expect(restored).toHaveCount(1);
  await expect(restored.getByTestId('set-weight').first()).toHaveValue('55');
  await expect(restored.getByTestId('set-reps').first()).toHaveValue('12');
  await expect(restored.getByTestId('today-metric')).toHaveText('660 kg·回');
});

// ── 月ナビ・グリッド ────────────────────────────────────────────────────────

test('前月・翌月ナビで月が移動し、年をまたいでも壊れない', async ({ page }) => {
  await switchTab(page, 'tab-calendar');
  const title = page.getByTestId('cal-title');

  const now = new Date();
  await expect(title).toHaveText(fmtMonth(now));

  // new Date(y, -1, 1) / new Date(y, 12, 1) は JS 側が年を正規化する
  await page.getByTestId('cal-prev').click();
  await expect(title).toHaveText(fmtMonth(new Date(now.getFullYear(), now.getMonth() - 1, 1)));

  await page.getByTestId('cal-next').click();
  await expect(title).toHaveText(fmtMonth(now));

  await page.getByTestId('cal-next').click();
  await expect(title).toHaveText(fmtMonth(new Date(now.getFullYear(), now.getMonth() + 1, 1)));

  // 年跨ぎ（翌年 1 月 → 前年 12 月）。shift_month の div_euclid/rem_euclid の検証
  await openCalendarOn(page, new Date(now.getFullYear() + 1, 0, 1));
  await page.getByTestId('cal-prev').click();
  await expect(title).toHaveText(`${now.getFullYear()}年12月`);

  // どの月でもグリッドの列がずれない（前後にはみ出す分は空きマスで埋める）
  const cells = await page.locator('[data-testid="cal-grid"] > *').count();
  expect(cells % 7, `グリッドのマス数は 7 の倍数: ${cells}`).toBe(0);

  // ★ 日曜始まり。1 日のマスは「その月の 1 日の曜日番号」と同じ位置に来る。
  //   core::week_start / aggregate_weekly も週の起点が日曜であることに依存しているので、
  //   月曜始まりへ退行するとここで落ちる（空きマスには data-date が無い）
  await openCalendarOn(page, now);
  const firstIndex = await page.evaluate(() => {
    const all = [...document.querySelectorAll('[data-testid="cal-grid"] > *')];
    return all.findIndex((el) => el.getAttribute('data-date')?.endsWith('-01'));
  });
  expect(firstIndex, '1 日が日曜始まりの列に置かれていない').toBe(
    new Date(now.getFullYear(), now.getMonth(), 1).getDay(),
  );
});

// ── ドット・月フッタ・サマリ ────────────────────────────────────────────────

test('実施日に部位カラーのドットが出て、4 部位を記録しても最大 3 個で頭打ちになる', async ({
  page,
}) => {
  const today = new Date();

  // 胸 → 背中 → 肩 → 腕 の 4 部位。ドットは部位の並び順で先頭 3 色に切り詰められる
  for (const name of ['ベンチプレス', '懸垂', 'ショルダープレス', 'バーベルカール']) {
    const card = await addExercise(page, name);
    await fillSet(card, 0, { reps: 10 });
  }

  await openCalendarOn(page, today);
  const cell = dayCell(page, today);
  await expect(cell).toHaveAttribute('data-trained', 'true');
  await expect(cell.getByTestId('cal-dot')).toHaveCount(3);

  const colors = await cell
    .getByTestId('cal-dot')
    .evaluateAll((els) => els.map((el) => getComputedStyle(el).backgroundColor));
  expect(new Set(colors).size, `部位ごとに別の色が出る: ${colors.join(' ')}`).toBe(3);

  // 記録の無い日にはドットが出ない
  await expect(dayCell(page, daysAgo(1))).toHaveAttribute('data-trained', 'false');
  await expect(dayCell(page, daysAgo(1)).getByTestId('cal-dot')).toHaveCount(0);
});

test('月フッタが実施日数・合計・セット数を正しく出す', async ({ page }) => {
  const today = new Date();

  const bench = await addExercise(page, 'ベンチプレス');
  await fillSet(bench, 0, { weight: 60, reps: 10 });
  await bench.getByTestId('add-set').click();
  await fillSet(bench, 1, { weight: 60, reps: 8 });

  // 懸垂は Kind::Bodyweight。セット数には入るが kg·回 の合計には folding しない
  // （単位の違う指標を足すと意味を失うため）
  const pullup = await addExercise(page, '懸垂');
  await fillSet(pullup, 0, { reps: 12 });

  await openCalendarOn(page, today);
  await expect(page.getByTestId('cal-trained-days')).toHaveText('1 日');
  await expect(page.getByTestId('cal-volume')).toHaveText('1,080 kg·回');
  await expect(page.getByTestId('cal-sets')).toHaveText('3');
});

test('記録がある日のサマリに種目・セット・指標・体重・メモが出る', async ({ page }) => {
  const today = new Date();

  const card = await addExercise(page, 'ベンチプレス');
  await fillSet(card, 0, { weight: 60, reps: 10 });

  await page.getByTestId('condition-toggle').click();
  await page.getByTestId('body-weight').fill('62.5');
  await page.getByTestId('condition-note').fill('絶好調');

  await openCalendarOn(page, today);
  await dayCell(page, today).click();

  await expect(page.getByTestId('cal-detail-date')).toHaveText(fmtDate(today));
  const log = page.getByTestId('cal-log');
  await expect(log).toHaveCount(1);
  await expect(log).toContainText('ベンチプレス');
  await expect(log).toContainText('胸');
  await expect(log).toContainText('60×10');
  await expect(log).toContainText('600 kg·回');
  await expect(page.getByTestId('cal-body-weight')).toHaveText('体重 62.5 kg');
  await expect(page.getByTestId('cal-note')).toHaveText('絶好調');
});

// ── レイアウト ──────────────────────────────────────────────────────────────

test('iPhone 15 Pro 幅でカレンダーが横スクロールしない', async ({ page }) => {
  await page.setViewportSize({ width: 393, height: 852 });

  // 長い種目名はサマリ行で溢れやすいので、あえてこれを選ぶ
  const card = await addExercise(page, 'トライセプスエクステンション');
  await fillSet(card, 0, { weight: 27.5, reps: 12 });

  const today = new Date();
  await openCalendarOn(page, today);
  await dayCell(page, today).click();
  await expect(page.getByTestId('cal-log')).toHaveCount(1);

  const overflow = await page.evaluate(
    () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
  );
  expect(overflow, '横スクロールが発生している').toBeLessThanOrEqual(0);
});
