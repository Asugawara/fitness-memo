import { test, expect } from '@playwright/test';

// 種目ごとのユーザー定義ラベル
// （adr/data-model/labels-on-the-exercise-and-a-mark-on-the-log.md /
//  adr/ux/label-chips-switch-the-history-and-the-copy.md）の E2E。
//
// ここでしか見られないのは 5 つ。
//
// 1. **タブ横断の配線。** 設定タブで作った定義が記録タブのカードにチップとして出る。
//    `Db` を挟んだ 2 画面の往復なので、ホストの `cargo test` には見えない
// 2. **チップが履歴とコピーの両方を同時に切り替えること。** `core.rs` の unit test は
//    `LabelFilter` の意味論までしか言えない。Memo をどう繋いだかは画面でしか確かめられない
// 3. **ラベルだけでは `Db` に 1 バイトも書かれないこと。** 設計の要点そのもので、
//    `localStorage` を直接読む以外に可視化する手段が無い
// 4. **定義 0 の種目でカードの幾何が 1px も動かないこと**（既存利用者の体験不変）。
//    subgrid の列数と `data-labels` の有無が噛み合っているかは実測しかない
// 5. **桁揃えと 44px の実測。** どちらも計画では推定値で、ブラウザに聞かないと分からない。
//    ★ 折り返しは**高さ**で見る（`getClientRects().length` は grid item では常に 1 で、
//      `max-width` を消しても通ってしまう）。9 番は viewport を 393×852 に固定する
//      （`.screen` の `max-width: 640px` により、Desktop Chrome では余裕がありすぎる）
//
// 正規化・上限・merge 規則・TSV 往復・フィルタの意味論は `src/core.rs` の unit test が
// 総当りしているので、ここでは追わない（history.spec.mjs と pins.spec.mjs の分担と同じ）。

const STORAGE_KEY = 'fitness-memo/v3';

test.beforeEach(async ({ page }) => {
  // ★ baseURL がサブパス（/fitness-memo/）を持つとき先頭 "/" はベースを丸ごと捨てる
  await page.goto('./');
  await expect(page.getByTestId('screen-record')).toBeVisible();
});

// ── ヘルパ（history.spec.mjs / smoke.spec.mjs からコピー）───────────────────
//
// ★ spec 単体で読める作法（pins.spec.mjs / interval.spec.mjs と同じ）。

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

function cardOf(page, name) {
  return page
    .getByTestId('exercise-card')
    .filter({ has: page.getByTestId('card-name').filter({ hasText: exactText(name) }) });
}

/**
 * 「種目を追加」シートから 1 つ選ぶ。
 *
 * ★ シートは部位のアコーディオンで、**開くまで種目が DOM に無い**
 *   （adr/ux/record-add-sheet-groups-as-single-open-accordion.md）。同時に 1 つしか
 *   開かないので「全部開く」ができず、見つかるまで順に開いて回る。
 * ★ 押す前に `aria-expanded` を見ないと、開いている部位を閉じてしまう。
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

function groupItem(page, name) {
  return page.getByTestId('group-item').filter({
    has: page.getByTestId('group-name').filter({ hasText: exactText(name) }),
  });
}

async function openSettingsSection(page, which) {
  await blurActive(page);
  await page.getByTestId('tab-settings').click();
  // ★ **別の節に居ることがある。** 開いている節は `SettingsPageCtx` が持っていて
  //   タブ往復では戻らないので、まずトップへ戻してから入る
  const back = page.getByTestId('settings-back');
  if (await back.isVisible()) await back.click();
  await page.getByTestId(`settings-row-${which}`).click();
}

async function openGroup(page, name) {
  await openSettingsSection(page, 'exercises');
  const item = groupItem(page, name);
  const toggle = item.getByTestId('group-toggle');
  if ((await toggle.getAttribute('aria-expanded')) !== 'true') await toggle.click();
  return item;
}

/** 設定タブの種目編集シートを開く。 */
async function openExerciseEditor(page, group, name) {
  const item = await openGroup(page, group);
  await item.getByTestId('exercise-name').filter({ hasText: exactText(name) }).click();
  const sheet = page.getByTestId('settings-sheet');
  await expect(sheet).toBeVisible();
  return sheet;
}

/** シートを閉じて記録タブへ戻る。 */
async function backToRecord(page) {
  await blurActive(page);
  await page.getByTestId('settings-sheet-close').click();
  // ★ `<dialog>` の close は非同期に飛ぶ。明示的に待つ（再実行で誤魔化さない）
  await expect(page.getByTestId('settings-sheet')).toBeHidden();
  await page.getByTestId('tab-record').click();
  await expect(page.getByTestId('screen-record')).toBeVisible();
}

/**
 * 種目編集シートでラベルを作る。**`on:change` 書き込みなので fill だけでは足りない** —
 * blur（または Enter）まで通す。
 */
async function addLabels(page, sheet, names) {
  for (const [i, name] of names.entries()) {
    await sheet.getByTestId('label-add').click();
    const input = sheet.getByTestId('label-name').nth(i);
    await input.fill(name);
    await input.blur();
  }
  await expect(sheet.getByTestId('label-name')).toHaveCount(names.length);
}

/** ラベル名 → 記録タブのチップ。「指定なし」は `data-label-any` で引く。 */
function chip(card, name) {
  return card.getByTestId('label-chip').filter({ hasText: exactText(name) });
}

function anyChip(card) {
  return card.locator('[data-testid=label-chip][data-label-any]');
}

/**
 * 過去のログを localStorage へ直接注入する。**ラベルは名前で引いて ID に直す**
 * （画面が作った定義の ID は外から予測できない）。
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
      for (const { daysAgo: ago, exerciseName, sets, label } of entries) {
        const k = dateKey(ago);
        const ex = db.exercises.find((e) => e.name === exerciseName);
        if (!ex) throw new Error(`exercise not found: ${exerciseName}`);
        const session = db.sessions[k] ?? { logs: [], body_weight: null, note: '' };
        const log = { exercise_id: ex.id, sets, at: null };
        if (label !== undefined) {
          const l = (ex.labels ?? []).find((x) => x.name === label);
          if (!l) throw new Error(`label not found on ${exerciseName}: ${label}`);
          log.label = l.id;
        }
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

/**
 * ベンチプレスに H / P / S を定義して記録タブへ戻る。
 *
 * ★ **カードを足すのはこれの後。** セットを 1 本も打っていないカードはタブ往復で
 *   消える（`DayEditor` はタブ切替で破棄され、`load_cards` は `session.logs` から
 *   引く）。既存の空カードと同じ階級で、これは仕様。
 */
async function defineHPS(page) {
  const sheet = await openExerciseEditor(page, '胸', 'ベンチプレス');
  await addLabels(page, sheet, ['H', 'P', 'S']);
  await backToRecord(page);
}

/** H が 9 日前（70×10 ×3）、P が 2 日前（100×3 ×2）。 */
const HPS_HISTORY = [
  {
    daysAgo: 9,
    exerciseName: 'ベンチプレス',
    label: 'H',
    sets: [
      { weight: 70, reps: 10 },
      { weight: 70, reps: 10 },
      { weight: 70, reps: 10 },
    ],
  },
  {
    daysAgo: 2,
    exerciseName: 'ベンチプレス',
    label: 'P',
    sets: [
      { weight: 100, reps: 3 },
      { weight: 100, reps: 3 },
    ],
  },
];

// ── 1. タブ横断の配線 ───────────────────────────────────────────────────────

test('1. 設定でラベルを作ると記録タブのカードにチップ行が出る', async ({ page }) => {
  // ベースライン: 定義が 0 本ならブロックごと出ない
  const card = await addExercise(page, 'ベンチプレス');
  await expect(card.locator('.lbl-row')).toHaveCount(0);

  await defineHPS(page);

  const again = await addExercise(page, 'ベンチプレス');
  // 「指定なし」+ H / P / S の 4 個
  await expect(again.getByTestId('label-chip')).toHaveCount(4);
  await expect(anyChip(again)).toHaveAttribute('aria-pressed', 'true');
  for (const name of ['H', 'P', 'S']) {
    await expect(chip(again, name)).toHaveAttribute('aria-pressed', 'false');
  }
});

// ── 2. チップが履歴とコピーを同時に切り替える ───────────────────────────────

test('2. チップを押すと履歴の行とコピーの中身が同時に切り替わる', async ({ page }) => {
  await defineHPS(page);
  await seedPastLogs(page, HPS_HISTORY);
  const card = await addExercise(page, 'ベンチプレス');
  // 「指定なし」= 従来どおり全ログから前回（2 日前の P）
  await expect(card.getByTestId('last-log')).toHaveText(fmtDate(daysAgo(2)));
  // ★ 履歴行にラベル名が出る
  await expect(card.getByTestId('last-label')).toHaveText('P');

  // [H] へ切り替えると 9 日前に飛ぶ
  await chip(card, 'H').click();
  await expect(card.getByTestId('last-log')).toHaveText(fmtDate(daysAgo(9)));
  await expect(card.getByTestId('last-label')).toHaveText('H');
  await expect(chip(card, 'H')).toHaveAttribute('aria-pressed', 'true');
  await expect(anyChip(card)).toHaveAttribute('aria-pressed', 'false');

  // コピーの中身も同時に切り替わっている
  await card.getByTestId('copy-last').click();
  await expect(card.getByTestId('set-row')).toHaveCount(3);
  await expect(card.getByTestId('set-row').nth(0).getByTestId('set-weight')).toHaveValue('70');
  await expect(card.getByTestId('set-row').nth(0).getByTestId('set-reps')).toHaveValue('10');
  // チップは [H] のまま（コピーで選択が動かない）
  await expect(chip(card, 'H')).toHaveAttribute('aria-pressed', 'true');
});

// ── 3. フォールバックしない ─────────────────────────────────────────────────

test('3. 履歴の無いラベルは「記録なし」+ コピーボタンなし。「指定なし」で戻る', async ({
  page,
}) => {
  await defineHPS(page);
  await seedPastLogs(page, HPS_HISTORY);
  const card = await addExercise(page, 'ベンチプレス');
  await chip(card, 'S').click();

  // ★ ラベルなしの前回へ落ちない（落とすとコピーが「表示と違うもの」を流し込む）
  await expect(card.getByTestId('last-log')).toHaveText('記録なし');
  await expect(card.getByTestId('copy-last')).toHaveCount(0);

  // 回復手段は常に可視の「指定なし」1 タップ
  await anyChip(card).click();
  await expect(card.getByTestId('last-log')).toHaveText(fmtDate(daysAgo(2)));
  await expect(card.getByTestId('copy-last')).toHaveCount(1);
});

// ── 4. 選択の永続とラベルだけでは書かないこと ───────────────────────────────

test('4. 選択はセットがあるときだけリロードを越え、ラベルだけでは Db に何も書かない', async ({
  page,
}) => {
  await defineHPS(page);
  const card = await addExercise(page, 'ベンチプレス');
  await chip(card, 'P').click();

  // ★ **ラベルだけの状態では `sessions` にログが生えていない。**
  //   設計の唯一の可視証明（作ると dedupe_logs が次回起動で消して食い違う）
  await flushToStorage(page);
  const logs = await page.evaluate((key) => {
    const db = JSON.parse(localStorage.getItem(key));
    return Object.values(db.sessions).flatMap((s) => s.logs);
  }, STORAGE_KEY);
  expect(logs, 'ラベルだけでログを作ってはいけない').toEqual([]);

  // セットを打てば選択はリロードを越える
  const row0 = card.getByTestId('set-row').nth(0);
  await row0.getByTestId('set-weight').fill('100');
  await row0.getByTestId('set-reps').fill('3');
  await blurActive(page);
  await flushToStorage(page);
  await page.reload();
  await expect(page.getByTestId('screen-record')).toBeVisible();

  const back = cardOf(page, 'ベンチプレス');
  await expect(chip(back, 'P')).toHaveAttribute('aria-pressed', 'true');

  // ★ **`[指定なし]` に戻すと、リロード後も「指定なし」のまま**（`write_log` の
  //   `Some(log)` 枝が無条件代入であることの証明。`if label.is_some()` にすると
  //   `Db` に届かず `[P]` に戻る）
  await anyChip(back).click();
  await blurActive(page);
  await flushToStorage(page);
  await page.reload();
  await expect(page.getByTestId('screen-record')).toBeVisible();

  const again = cardOf(page, 'ベンチプレス');
  await expect(anyChip(again)).toHaveAttribute('aria-pressed', 'true');
  await expect(chip(again, 'P')).toHaveAttribute('aria-pressed', 'false');
});

// ── 5. 定義 0 の種目は 1px も動かない ───────────────────────────────────────

test('5. ラベル 0 の種目はチップ行を描かず、カードの幾何が 1px も動かない', async ({ page }) => {
  const card = await addExercise(page, 'スクワット');

  await expect(card.locator('.lbl-row')).toHaveCount(0);
  await expect(card.locator('.last-rows')).not.toHaveAttribute('data-labels', /.*/);
  await expect(card.getByTestId('last-label')).toHaveCount(0);

  const geometry = await card.locator('.last-rows').evaluate((el) => {
    const s = getComputedStyle(el);
    return { marginTop: s.marginTop, columns: s.gridTemplateColumns.split(' ').length };
  });
  expect(geometry.marginTop, '.last-rows の margin-top が変わっている').toBe('4px');
  expect(geometry.columns, '定義 0 の種目で 4 列になっている').toBe(3);
});

// ── 6. 44px ─────────────────────────────────────────────────────────────────

test('6. チップは 44px を割らない', async ({ page }) => {
  await defineHPS(page);
  const card = await addExercise(page, 'ベンチプレス');
  const chips = card.getByTestId('label-chip');
  const n = await chips.count();
  expect(n).toBe(4);
  for (let i = 0; i < n; i++) {
    const box = await chips.nth(i).boundingBox();
    // ★ **判定側を丸める。** Pixel 7 の DPR 2.625 では 43.99993896484375 になる
    expect(Math.round(box.height), `チップ ${i} の高さ`).toBeGreaterThanOrEqual(44);
    expect(Math.round(box.width), `チップ ${i} の幅`).toBeGreaterThanOrEqual(44);
  }
});

// ── 7. 構造の契約 ───────────────────────────────────────────────────────────

test('7. チップ行を足しても構造の契約が動かない', async ({ page }) => {
  await defineHPS(page);
  const card = await addExercise(page, 'ベンチプレス');
  // ★ `.card-head` はドラッグの掴み口。ボタンを入れるとヘッダを割ることになる
  await expect(card.locator('.card-head button')).toHaveCount(0);
  // メモのトグルと「この日から外す」の 2 つだけ
  await expect(card.locator('.card-foot button')).toHaveCount(2);

  // ★ UA 既定のボタンは約 20px しかない（adr/ux/declare-color-scheme-for-ua-widgets.md）
  const sheet = await openExerciseEditor(page, '胸', 'ベンチプレス');
  await expect(page.locator('[data-testid=settings-sheet] button:not([class])')).toHaveCount(0);
  // 削除の確認を開いた状態でも同じ
  await sheet.getByTestId('label-remove').first().click();
  await expect(sheet.getByTestId('label-delete-confirm')).toBeVisible();
  await expect(page.locator('[data-testid=settings-sheet] button:not([class])')).toHaveCount(0);
});

// ── 8. 削除しても記録は消えない ─────────────────────────────────────────────

test('8. ラベルを削除しても記録は消えず、削除済みを選んでいたカードは「指定なし」に落ちる', async ({
  page,
}) => {
  await defineHPS(page);
  await seedPastLogs(page, HPS_HISTORY);

  // 今日 [P] でセットを打つ
  const card = await addExercise(page, 'ベンチプレス');
  await chip(card, 'P').click();
  const row0 = card.getByTestId('set-row').nth(0);
  await row0.getByTestId('set-weight').fill('105');
  await row0.getByTestId('set-reps').fill('3');
  await blurActive(page);

  // 設定で P を削除（確認の warn-box を通す）
  const sheet = await openExerciseEditor(page, '胸', 'ベンチプレス');
  const p = sheet.getByTestId('label-name').nth(1);
  await expect(p).toHaveValue('P');
  await sheet.getByTestId('label-remove').nth(1).click();
  await expect(sheet.getByTestId('label-delete-confirm')).toContainText('同じ名前');
  await sheet.getByTestId('label-delete-yes').click();
  await expect(sheet.getByTestId('label-name')).toHaveCount(2);
  await backToRecord(page);

  const back = cardOf(page, 'ベンチプレス');
  // 定義は H / S の 2 本 + 「指定なし」
  await expect(back.getByTestId('label-chip')).toHaveCount(3);
  // ★ **セットがあるのに「記録なし」にならず、「指定なし」が点灯する**
  await expect(anyChip(back)).toHaveAttribute('aria-pressed', 'true');
  await expect(back.getByTestId('last-log')).toHaveText(fmtDate(daysAgo(2)));
  // 過去の P の記録は無傷（「指定なし」で見える）
  await expect(back.getByTestId('last-row')).toContainText('100×3');
  // 今日打ったセットも消えていない
  await expect(back.getByTestId('set-row').nth(0).getByTestId('set-weight')).toHaveValue('105');
});

// ── 9. 桁揃えの実測 ─────────────────────────────────────────────────────────
//
// ★ **viewport を 393×852 に固定する。** `.screen` は `max-width: 640px` なので、
//   既定の `chromium`（Desktop Chrome = 1280px）ではカード内側が約 586px になり
//   `.sets` に余裕がありすぎて折り返し判定が永久に発火しない。列幅の主張は
//   「393px 幅のカードで」という前提つきのものなので（ADR 決定 6 の表）、
//   どの project から走っても同じ幅で測る。
test.describe('9. 桁揃え（393×852 固定）', () => {
  test.use({ viewport: { width: 393, height: 852 } });

  test('12 文字のラベルでも .sets が折り返さず、日付の桁が縦に揃う', async ({ page }) => {
    // ★ 12 文字（`MAX_LABEL_LEN`）の和文。cap が効いていないとここで `.sets` が割れる
    const sheet = await openExerciseEditor(page, '胸', 'ベンチプレス');
    await addLabels(page, sheet, ['あいうえおかきくけこさし', 'P']);
    await backToRecord(page);

    // ★ **`100×3` を 4 セットにする。** これが cap の受け入れ条件そのもの（HPS の
    //   Power 日として最も普通の記録）。実測で `.sets` は cap 無しで 80px /
    //   `6em` で 152px / `4em` で 176px になり、176px でしか 1 行に収まらない。
    //
    // ★ **2 日前は `label` を落として「定義はあるがその日は None」の行を作る。**
    //   空 span を省く実装ではその行の `.when` が 1 列目に落ちて x がずれるので、
    //   下の桁揃え判定がそのまま門番になる（計画の落とし穴 4）。
    await seedPastLogs(page, [
      {
        daysAgo: 9,
        exerciseName: 'ベンチプレス',
        label: 'あいうえおかきくけこさし',
        sets: [
          { weight: 100, reps: 3 },
          { weight: 100, reps: 3 },
          { weight: 100, reps: 3 },
          { weight: 100, reps: 3 },
        ],
      },
      {
        daysAgo: 2,
        exerciseName: 'ベンチプレス',
        sets: [
          { weight: 100, reps: 3 },
          { weight: 100, reps: 3 },
          { weight: 100, reps: 3 },
          { weight: 100, reps: 3 },
        ],
      },
    ]);

    // 3 件出して 2 行以上を並べる（既定は 1 件）
    await blurActive(page);
    await page.getByTestId('tab-settings').click();
    const back = page.getByTestId('settings-back');
    if (await back.isVisible()) await back.click();
    await page.getByTestId('settings-row-history').click();
    await page.getByTestId('history-btn').and(page.locator('[data-count="3"]')).click();
    await page.getByTestId('settings-back').click();
    await page.getByTestId('tab-record').click();
    await expect(page.getByTestId('screen-record')).toBeVisible();

    const card = await addExercise(page, 'ベンチプレス');
    await expect(card.getByTestId('last-row')).toHaveCount(2);
    await expect(card.locator('.last-rows')).toHaveAttribute('data-labels', 'true');

    // ★ **折り返しは高さで見る。`getClientRects().length` では見られない** —
    //   `.last-row` は grid なので子の span は grid item として blockify され、
    //   インライン内容が何行に折り返しても矩形は常に 1 個になる（`max-width` を
    //   消しても通ってしまう）。1 行 = 12px × line-height 1.5 = 18px なので 20px で足りる。
    const setHeights = await card
      .getByTestId('last-row')
      .locator('.sets')
      .evaluateAll((els) => els.map((el) => el.getBoundingClientRect().height));
    expect(setHeights.length).toBe(2);
    for (const h of setHeights) {
      expect(h, `.sets が折り返している（列幅の cap が効いていない）: ${setHeights}`).toBeLessThan(
        20,
      );
    }

    // ★ 決定 6（subgrid で桁を縦に揃える）が生きていること。日付列の左端が全行で一致する。
    //   **ラベル無しの行でも空 span を描いている**ことがここで担保される
    const xs = await card.getByTestId('last-row').locator('.when').evaluateAll((els) =>
      els.map((el) => Math.round(el.getBoundingClientRect().x)),
    );
    expect(xs.length).toBe(2);
    expect(new Set(xs).size, `日付の桁が揃っていない: ${xs}`).toBe(1);

    // ★ ラベル span は**行数と同じ数だけ**ある（`label: None` の行でも空 span を描く）
    const labels = await card.getByTestId('last-label').evaluateAll((els) =>
      els.map((el) => ({
        text: el.textContent,
        w: Math.round(el.getBoundingClientRect().width),
        clipped: el.scrollWidth > el.clientWidth,
      })),
    );
    expect(labels.length, 'ラベル span の数が行数と一致しない').toBe(xs.length);
    expect(labels.filter((l) => l.text === '').length, '空 span が描かれていない').toBe(1);

    // ★ `ellipsis` が実際に効いていること（cap 以下の幅に収まり、中身が溢れている）
    const long = labels.find((l) => l.text !== '');
    expect(long.w, `ラベル列が cap（4em = 48px）を超えている: ${long.w}px`).toBeLessThanOrEqual(48);
    expect(long.clipped, 'ellipsis が効いていない（cap が当たっていない）').toBe(true);
  });
});

// ── 10. 改名で履歴が外れない ────────────────────────────────────────────────

test('10. ラベルを改名しても履歴が付いてくる', async ({ page }) => {
  await defineHPS(page);
  await seedPastLogs(page, HPS_HISTORY);

  // P で 2 日前が引けることを先に確かめる
  const card = await addExercise(page, 'ベンチプレス');
  await chip(card, 'P').click();
  await expect(card.getByTestId('last-log')).toHaveText(fmtDate(daysAgo(2)));

  // 「P」→「Power」。**ID は変わらない**（`LabelRow` が `LabelId` を持つことの証明）
  const sheet = await openExerciseEditor(page, '胸', 'ベンチプレス');
  const p = sheet.getByTestId('label-name').nth(1);
  await p.fill('Power');
  await p.blur();
  await backToRecord(page);

  // ★ セットを打っていないので往復でカードは消える。足し直して履歴が残っているか見る
  //   （選択はカードごと消えたので「指定なし」からやり直す）
  const back = await addExercise(page, 'ベンチプレス');
  await chip(back, 'Power').click();
  await expect(back.getByTestId('last-log')).toHaveText(fmtDate(daysAgo(2)));
  await expect(back.getByTestId('last-label')).toHaveText('Power');
});

// ── 重複の門番 ──────────────────────────────────────────────────────────────

test('同じ名前のラベルは新規追加でも改名でも入らない', async ({ page }) => {
  const sheet = await openExerciseEditor(page, '胸', 'ベンチプレス');
  await addLabels(page, sheet, ['H', 'P']);

  // 改名で衝突させる
  const p = sheet.getByTestId('label-name').nth(1);
  await p.fill('H');
  await p.blur();
  await expect(sheet.getByTestId('duplicate-label')).toBeVisible();
  // 直前の保存値へ戻る
  await expect(p).toHaveValue('P');

  // 新規追加で衝突させる
  await sheet.getByTestId('label-add').click();
  const third = sheet.getByTestId('label-name').nth(2);
  await third.fill('H');
  await third.blur();
  await expect(sheet.getByTestId('duplicate-label')).toBeVisible();

  await backToRecord(page);
  const card = await addExercise(page, 'ベンチプレス');
  // 「指定なし」+ H / P。空欄の 3 本目は `set_labels` が落とす
  await expect(card.getByTestId('label-chip')).toHaveCount(3);
});

test('ラベル名は前後の空白を落として保存する', async ({ page }) => {
  // ★ 生値のまま保存すると TSV の往復でラベルが 2 本に割れる —
  //   `core::resolve_label` は `name.trim()` してから厳密比較するので `"P "` と
  //   一致せず新しい ID を採番し、過去ログは旧 ID・取り込んだログは新 ID に付く。
  //   UI の重複ガードは trim 比較なので、利用者はこの経路以外からこの状態を作れない
  const sheet = await openExerciseEditor(page, '胸', 'ベンチプレス');
  await sheet.getByTestId('label-add').click();
  const input = sheet.getByTestId('label-name').first();
  await input.fill('  P  ');
  await input.blur();

  // 入力欄も揃える（見えている値と `Db` を食い違わせない）
  await expect(input).toHaveValue('P');

  await flushToStorage(page);
  const stored = await page.evaluate((key) => {
    const db = JSON.parse(localStorage.getItem(key));
    // `labels` は `skip_serializing_if = "Vec::is_empty"` なので 0 本なら欄ごと無い
    return db.exercises.find((e) => e.name === 'ベンチプレス').labels ?? [];
  }, STORAGE_KEY);
  expect(stored.map((l) => l.name), '前後の空白が保存されている').toEqual(['P']);

  // 「高重量 低レップ」のような**中の**空白は落とさない（1 セル = 1 名前）
  await sheet.getByTestId('label-add').click();
  const second = sheet.getByTestId('label-name').nth(1);
  await second.fill('高重量 低レップ');
  await second.blur();
  await expect(second).toHaveValue('高重量 低レップ');

  await backToRecord(page);
  const card = await addExercise(page, 'ベンチプレス');
  await expect(chip(card, 'P')).toHaveCount(1);
  await expect(chip(card, '高重量 低レップ')).toHaveCount(1);
});
