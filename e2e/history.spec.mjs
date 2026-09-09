import { test, expect } from '@playwright/test';

// 種目カードの「前回までの記録」（adr/ux/past-records-by-date-with-a-count-setting.md）の E2E。
//
// ここでしか見られないのは 3 つ。
//
// 1. **表示件数という表示設定が、コピーと「重量未入力」の判定を変えないこと。**
//    どちらも `history` の先頭 1 件しか見ない規約で、ホストの `cargo test` は
//    `last_logs_before` の先頭が `last_log_before` と一致することまでしか言えない。
//    Memo をどう繋いだかは画面でしか確かめられない
// 2. **メモを出さないこと、しかしコピーは今までどおり運ぶこと。** 「画面に出さない」と
//    「運ばない」は別で、後者を壊すと adr/ux/copy-carries-the-notes.md の決定が消える
// 3. **設定が `Db` ではなく UI キーに入ること。** 型では強制できない
//    （adr/storage/ui-state-in-separate-key.md）
//
// 丸め（範囲外 / 未設定）と履歴の抽出そのものは `src/core.rs` の unit test が
// 総当りしているので、ここでは追わない（pins.spec.mjs と src/reorder.rs の分担と同じ）。

const STORAGE_KEY = 'fitness-memo/v3';
const UI_KEY = 'fitness-memo/ui/v1';

test.beforeEach(async ({ page }) => {
  // ★ baseURL がサブパス（/fitness-memo/）を持つとき先頭 "/" はベースを丸ごと捨てる
  await page.goto('./');
  await expect(page.getByTestId('screen-record')).toBeVisible();
});

// ── ヘルパ ──────────────────────────────────────────────────────────────────

function exactText(s) {
  return new RegExp(`^${s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}$`);
}

/** views/mod.rs の `fmt_date` と同じ "8/8 (金)" */
function fmtDate(d) {
  return `${d.getMonth() + 1}/${d.getDate()} (${['日', '月', '火', '水', '木', '金', '土'][d.getDay()]})`;
}

function daysAgo(n) {
  const d = new Date();
  d.setDate(d.getDate() - n);
  return d;
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

/**
 * 過去のログを localStorage へ直接注入する（smoke.spec.mjs の同名ヘルパと同じ形）。
 * `sets` は生で流すので `{weight, reps, note}` でセットメモも仕込める。
 */
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
      for (const { daysAgo: ago, exerciseName, sets, exerciseNote } of entries) {
        const k = dateKey(ago);
        const ex = db.exercises.find((e) => e.name === exerciseName);
        if (!ex) throw new Error(`preset exercise not found: ${exerciseName}`);
        const session = db.sessions[k] ?? { logs: [], body_weight: null, note: '' };
        const log = { exercise_id: ex.id, sets, at: null };
        if (exerciseNote !== undefined) log.note = exerciseNote;
        session.logs.push(log);
        db.sessions[k] = session;
      }
      localStorage.setItem(key, JSON.stringify(db));
    },
    { entries, key: STORAGE_KEY },
  );
  await page.reload();
  await expect(page.getByTestId('screen-record')).toBeVisible();
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
 * 「前回までの記録」の件数を選ぶ。
 *
 * ★ **既に別の節に入っていることがある。** 開いている節は `SettingsPageCtx` が持っていて
 *   タブ往復では戻らない（smoke.spec.mjs の `openSettingsSection` と同じ注意）。
 */
async function setHistoryCount(page, n) {
  await blurActive(page);
  await page.getByTestId('tab-settings').click();
  const back = page.getByTestId('settings-back');
  if (await back.isVisible()) await back.click();
  await page.getByTestId('settings-row-history').click();
  await page.getByTestId('history-btn').and(page.locator(`[data-count="${n}"]`)).click();
  await page.getByTestId('settings-back').click();
  await page.getByTestId('tab-record').click();
  await expect(page.getByTestId('screen-record')).toBeVisible();
}

/**
 * 直近 60 日から「日が 1 桁の日」と「2 桁の日」を 1 つずつ選ぶ。
 *
 * ★ 日付は今日からの相対で決まるので、`8/3` と `8/17` のような**桁数の違う組**を
 *   固定日付でハードコードすると年をまたいだ瞬間に意味が変わる。走って探す。
 */
function daysAgoWithDifferentWidths() {
  let one = null;
  let two = null;
  for (let n = 1; n <= 60 && (one === null || two === null); n++) {
    const day = daysAgo(n).getDate();
    if (day < 10) {
      if (one === null) one = n;
    } else if (two === null) two = n;
  }
  expect(one, '直近 60 日に日が 1 桁の日がある').not.toBeNull();
  expect(two, '直近 60 日に日が 2 桁の日がある').not.toBeNull();
  return { one, two };
}

/** 3 週ぶんのベンチプレス。数値を全部変えてあるので「どれを見たか」が判別できる。 */
const THREE_WEEKS = [
  { daysAgo: 21, exerciseName: 'ベンチプレス', sets: [{ weight: 50, reps: 10 }] },
  { daysAgo: 14, exerciseName: 'ベンチプレス', sets: [{ weight: 55, reps: 9 }] },
  { daysAgo: 7, exerciseName: 'ベンチプレス', sets: [{ weight: 60, reps: 8 }] },
];

// ── 表示 ────────────────────────────────────────────────────────────────────

test('1. 前回の記録は相対日数ではなく日付で出る', async ({ page }) => {
  await seedPastLogs(page, [
    { daysAgo: 3, exerciseName: 'ベンチプレス', sets: [{ weight: 60, reps: 10 }] },
  ]);
  const card = await addExercise(page, 'ベンチプレス');

  await expect(card.getByTestId('last-log')).toHaveText(fmtDate(daysAgo(3)));
  // 相対日数はヒーローと部位チップに残るが、カードからは消える
  await expect(card.getByTestId('last-row')).not.toContainText('3日前');
  await expect(card.getByTestId('last-row')).not.toContainText('前回');
});

test('2. 既定は 1 件で、3 にすると 3 日ぶんが新しい順に並ぶ', async ({ page }) => {
  await seedPastLogs(page, THREE_WEEKS);

  const card = await addExercise(page, 'ベンチプレス');
  await expect(card.getByTestId('last-row'), '既定は前回 1 件だけ').toHaveCount(1);
  await expect(card.getByTestId('last-log')).toHaveText(fmtDate(daysAgo(7)));

  await setHistoryCount(page, 3);
  // ★ セットを 1 つも入れていないカードは `Db` に無いので、タブを往復すると消える
  //   （`write_log` はセットもメモも空なら書かない）。置き直してから数える
  await addExercise(page, 'ベンチプレス');

  const rows = cardOf(page, 'ベンチプレス').getByTestId('last-row');
  await expect(rows).toHaveCount(3);
  // ★ 新しい順。先頭が「前回」であることが copy / 重量警告の土台になっている
  await expect(cardOf(page, 'ベンチプレス').getByTestId('last-log')).toHaveText([
    fmtDate(daysAgo(7)),
    fmtDate(daysAgo(14)),
    fmtDate(daysAgo(21)),
  ]);
});

test('3. 件数を増やしても記録が足りなければあるぶんだけ出る', async ({ page }) => {
  await seedPastLogs(page, THREE_WEEKS.slice(1)); // 2 日ぶんしか無い
  await setHistoryCount(page, 3);

  const card = await addExercise(page, 'ベンチプレス');
  await expect(card.getByTestId('last-row')).toHaveCount(2);
});

test('4. 記録が 1 件も無い種目は「記録なし」で、コピーボタンも出ない', async ({ page }) => {
  await setHistoryCount(page, 3);
  const card = await addExercise(page, 'ベンチプレス');

  await expect(card.getByTestId('last-log')).toHaveText('記録なし');
  await expect(card.getByTestId('copy-last')).toHaveCount(0);
  // ★ 空でも .last-row は 1 個描く（薄字テストが querySelector('.last-row') を引く）
  await expect(card.locator('.last-row')).toHaveCount(1);
});

// ── 間隔 ────────────────────────────────────────────────────────────────────

test('5. 日付の桁数が違ってもセット列と指標の列が揃う', async ({ page }) => {
  // ★ これが日付表記にした目的そのもの。エントリごとに幅を決めると
  //   "8/3 (月)" と "8/17 (月)" でセットの開始位置がずれ、縦に数字を読めなくなる
  const { one, two } = daysAgoWithDifferentWidths();
  await seedPastLogs(page, [
    { daysAgo: Math.max(one, two), exerciseName: 'ベンチプレス', sets: [{ weight: 55, reps: 9 }] },
    { daysAgo: Math.min(one, two), exerciseName: 'ベンチプレス', sets: [{ weight: 60, reps: 10 }] },
  ]);
  await setHistoryCount(page, 2);
  await addExercise(page, 'ベンチプレス');

  const cols = await cardOf(page, 'ベンチプレス').evaluate((el) => {
    const rows = [...el.querySelectorAll('.last-row')];
    const x = (r, sel) => r.querySelector(sel).getBoundingClientRect();
    // ★ span はグリッドの列幅いっぱいに伸びるので、箱ではなく**文字の実描画幅**を測る
    const textWidth = (node) => {
      const range = document.createRange();
      range.selectNodeContents(node);
      return Math.round(range.getBoundingClientRect().width);
    };
    return {
      whenTextWidths: rows.map((r) => textWidth(r.querySelector('.when'))),
      setsLeft: rows.map((r) => Math.round(x(r, '.sets').left)),
      metricRight: rows.map((r) => Math.round(x(r, '.metric').right)),
      gap: rows[1].getBoundingClientRect().top - rows[0].getBoundingClientRect().bottom,
    };
  });

  expect(
    new Set(cols.whenTextWidths).size,
    '日付そのものの幅は桁数ぶん違っている（テストが有効であることの確認）',
  ).toBe(2);
  expect(new Set(cols.setsLeft).size, 'セット列の左端は揃う').toBe(1);
  expect(new Set(cols.metricRight).size, '指標列の右端も揃う').toBe(1);
  expect(cols.gap, 'エントリ間は row-gap ぶん空く').toBeCloseTo(8, 0);
});

test('5b. 件数 1（既定）ではエントリ間の余白が当たらない', async ({ page }) => {
  // ★ 選ばなかった人の画面が 1px も動かないことの回帰。`row-gap` は「間」だけに
  //   入るので、1 件のときは高さに 1px も乗らない
  await seedPastLogs(page, [
    { daysAgo: 7, exerciseName: 'ベンチプレス', sets: [{ weight: 60, reps: 10 }] },
  ]);
  const card = await addExercise(page, 'ベンチプレス');

  await expect(card.getByTestId('last-row')).toHaveCount(1);
  const m = await card.evaluate((el) => {
    const rows = el.querySelector('.last-rows');
    return {
      rowMargin: getComputedStyle(el.querySelector('.last-row')).marginTop,
      wrapperTop: getComputedStyle(rows).marginTop,
      // gap は「間」にしか入らないので、1 件のときは高さに寄与しない
      height: Math.round(rows.getBoundingClientRect().height),
      lineHeight: Math.round(
        el.querySelector('.last-row').getBoundingClientRect().height,
      ),
    };
  });
  expect(m.rowMargin, 'エントリ側に margin を持たせない').toBe('0px');
  expect(m.wrapperTop, '見出しからの 4px は .last-rows が持つ').toBe('4px');
  expect(m.height, '1 件のときは row-gap が高さに乗らない').toBe(m.lineHeight);
});

test('6. 前回までの記録にメモは出さない（数値の列を崩さない）', async ({ page }) => {
  await seedPastLogs(page, [
    {
      daysAgo: 7,
      exerciseName: 'ベンチプレス',
      exerciseNote: '前回のメモ',
      sets: [
        { weight: 60, reps: 10 },
        { weight: 60, reps: 8, note: '最後潰れた' },
      ],
    },
  ]);
  await setHistoryCount(page, 3);
  const card = await addExercise(page, 'ベンチプレス');

  await expect(card.getByTestId('last-row')).toHaveCount(1);
  await expect(card.getByTestId('last-row')).not.toContainText('前回のメモ');
  await expect(card.getByTestId('last-row')).not.toContainText('最後潰れた');

  // ★ ただしコピーは今までどおりメモを運ぶ（adr/ux/copy-carries-the-notes.md）。
  //   「画面に出さない」と「運ばない」は別の話
  await card.getByTestId('copy-last').click();
  await card.getByTestId('note-toggle').click();
  await expect(card.getByTestId('exercise-note')).toHaveValue('前回のメモ');
  await expect(card.getByTestId('set-note').nth(1)).toHaveValue('最後潰れた');
});

// ── 先頭 1 件しか見ない判断 ─────────────────────────────────────────────────

test('7. 件数を 3 にしても「前回をコピー」は最新の 1 日ぶんだけを入れる', async ({ page }) => {
  await seedPastLogs(page, THREE_WEEKS);
  await setHistoryCount(page, 3);

  const card = await addExercise(page, 'ベンチプレス');
  await expect(card.getByTestId('last-row')).toHaveCount(3);
  await card.getByTestId('copy-last').click();

  // 3 週ぶん流し込まれてはいけない。最新（7 日前）の 1 セットだけ
  await expect(card.getByTestId('set-row')).toHaveCount(1);
  const row0 = card.getByTestId('set-row').nth(0);
  await expect(row0.getByTestId('set-weight')).toHaveValue('60');
  await expect(row0.getByTestId('set-reps')).toHaveValue('8');
});

test('8. 件数を 3 にしても「重量未入力」は最新の 1 日ぶんだけで判定する', async ({ page }) => {
  // 直近は自重（重量なし）、3 週間前だけ加重。件数 1 なら警告は出ない
  await seedPastLogs(page, [
    { daysAgo: 21, exerciseName: '懸垂', sets: [{ weight: 10, reps: 5 }] },
    { daysAgo: 7, exerciseName: '懸垂', sets: [{ weight: 0, reps: 12 }] },
  ]);
  await setHistoryCount(page, 3);

  const card = await addExercise(page, '懸垂');
  await expect(card.getByTestId('last-row')).toHaveCount(2);

  await card.getByTestId('set-reps').nth(0).fill('12');
  await blurActive(page);

  // ★ ここが `iter().any(..)` だと 3 週間前の加重が効いて警告が戻ってくる
  await expect(
    card.getByTestId('weight-missing'),
    '表示件数が警告の出方を変えてはいけない',
  ).toHaveCount(0);
});

// ── 保存 ────────────────────────────────────────────────────────────────────

test('9. 件数はリロードしても残り、Db には混ざらない', async ({ page }) => {
  await seedPastLogs(page, THREE_WEEKS);
  await setHistoryCount(page, 3);
  await flushToStorage(page);

  const stored = await page.evaluate(
    ({ dbKey, uiKey }) => ({
      db: localStorage.getItem(dbKey),
      ui: JSON.parse(localStorage.getItem(uiKey) ?? '{}'),
    }),
    { dbKey: STORAGE_KEY, uiKey: UI_KEY },
  );
  expect(stored.ui.history, 'UI 専用キーに入る').toBe(3);
  expect(stored.db, 'Db には混ざらない（書き出しに載らない）').not.toContain('history');

  await page.reload();
  await expect(page.getByTestId('screen-record')).toBeVisible();
  // 空カードはリロードで消えるので置き直す（設定が残っていることが見たいので十分）
  await addExercise(page, 'ベンチプレス');
  await expect(cardOf(page, 'ベンチプレス').getByTestId('last-row')).toHaveCount(3);
});

test('10. 件数を変えても言語とバナーの設定が巻き添えで消えない', async ({ page }) => {
  // ★ `UiState` は 1 フィールドの deserialize が落ちると全体が飛ぶ。
  //   `history` を範囲の広い整数で受けているのはそれを避けるため
  // ★ 英語を選ぶ。Playwright の locale は ja-JP なので、日本語を押しても
  //   同値ガード（settings.rs）で保存されず `lang` が書かれない
  await page.getByTestId('tab-settings').click();
  await page.getByTestId('settings-row-language').click();
  await page.getByTestId('lang-btn').and(page.locator('[data-lang="en"]')).click();
  await page.getByTestId('settings-back').click();

  await setHistoryCount(page, 2);
  await flushToStorage(page);

  const ui = await page.evaluate(
    (key) => JSON.parse(localStorage.getItem(key) ?? '{}'),
    UI_KEY,
  );
  expect(ui.history).toBe(2);
  expect(ui.lang, '言語の選択が巻き添えで消えていない').toBe('en');
});
