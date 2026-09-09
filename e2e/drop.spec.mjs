// ドロップセット（adr/data-model/drop-sets-as-stages-under-the-main-set.md /
// adr/ux/drop-sets-as-a-box-under-the-main-set.md）。
//
// ここでしか見られないのは 6 つ。
//
// - **段が保存 JSON に載り、段の無いセットにキーが増えないこと。**
//   `skip_serializing_if` はバイト一致の cargo test も見ているが、実際に画面から
//   入れた段が `fitness-memo/v3` に届くかは E2E にしか出ない
// - **`↓` が 1 行目に乗り、行の高さを変えないこと。** メモを開かずに段を足せる形に
//   するために 44px の標的を 1 つ増やしたので、「縦は増えていない」が言えなければ
//   adr/ux/exercise-and-set-notes-behind-one-toggle.md の決定を覆した根拠が消える
// - **メモを開かずに段を足して打てること。** 足せるのに打てない状態を作らない
// - **落とし幅の設定が、1 段目の重量に先に入ること。** 計算は `core` のユニット
//   テストが持つが、「設定 → カード」の配線が繋がっているかは E2E にしか出ない
// - **段があるときだけ箱が出ること。** 空の箱を常に出すと 44px × セット行数ぶん
//   カードが伸びる
// - **設定が推移タブ全体（グラフ・統計・記録一覧）に効き、値が `fitness-memo/ui/v1`
//   に入って `Db` に混ざらないこと**
//
// 集計の規則そのもの（3 指標 × 含める/除く、既定が「含めない」、落とし幅の丸めと
// clamp、取り込みでの合流）は `core.rs` のユニットテストが持つ。
import { expect, test } from '@playwright/test';

const STORAGE_KEY = 'fitness-memo/v3';
const UI_KEY = 'fitness-memo/ui/v1';

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

/** hidden への visibilitychange で pending の debounce 保存を即時 flush する。 */
async function flushToStorage(page) {
  await page.evaluate(() => {
    Object.defineProperty(document, 'hidden', { value: true, configurable: true });
    document.dispatchEvent(new Event('visibilitychange', { bubbles: true }));
  });
}

/**
 * 「種目を追加」シートで部位を開いて種目を押す。
 *
 * ★ シートは部位のアコーディオン（既定で全部閉じている）なので、`pick-exercise` を
 *   直に押すと見えないまま待って落ちる
 *   （adr/ux/record-add-sheet-groups-as-single-open-accordion.md）。
 */
async function addExercise(page, name) {
  // ★ 入力欄にフォーカスが残ると .kb-open で追加ボタンごと隠れる
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

/** 設定タブでドロップセットの扱いを選ぶ（`exclude` / `include`）。 */
async function setDrops(page, which) {
  await blurActive(page);
  await page.getByTestId('tab-settings').click();
  // ★ 既に別の節に入っていることがある（`SettingsPageCtx` はタブ往復で戻らない）
  const back = page.getByTestId('settings-back');
  if (await back.isVisible()) await back.click();
  await page.getByTestId('settings-row-drop-sets').click();
  await page.getByTestId('drop-sets-btn').and(page.locator(`[data-drops="${which}"]`)).click();
  await page.getByTestId('settings-back').click();
  // ★ 記録タブへ戻る。設定タブに居たまま次の操作へ進むと `add-exercise` が無い
  //   （history.spec.mjs の `setHistoryCount` と同じ作法）
  await page.getByTestId('tab-record').click();
  await expect(page.getByTestId('screen-record')).toBeVisible();
}

/** 設定タブで落とし幅（%）を入れる。 */
async function setDropPct(page, pct) {
  await blurActive(page);
  await page.getByTestId('tab-settings').click();
  const back = page.getByTestId('settings-back');
  if (await back.isVisible()) await back.click();
  await page.getByTestId('settings-row-drop-sets').click();
  await page.getByTestId('drop-pct').fill(String(pct));
  // ★ 入力欄にフォーカスが残ると `.kb-open` でタブ帯が隠れる
  await blurActive(page);
  await page.getByTestId('settings-back').click();
  await page.getByTestId('tab-record').click();
  await expect(page.getByTestId('screen-record')).toBeVisible();
}

async function openProgress(page) {
  await blurActive(page);
  await page.getByTestId('tab-progress').click();
  await expect(page.getByTestId('screen-progress')).toBeVisible();
}

// ── 記録 ────────────────────────────────────────────────────────────────────

/** 種目カードを出して 1 セット目に 60×10 を入れる。 */
async function cardWithOneSet(page) {
  const card = await addExercise(page, 'ベンチプレス');
  const row = card.getByTestId('set-row').nth(0);
  await row.getByTestId('set-weight').fill('60');
  await row.getByTestId('set-reps').fill('10');
  await blurActive(page);
  return card;
}

test('1. ★ メモを開かずに段を足して打てる', async ({ page }) => {
  // ★ この機能の要。`↓` は常に 1 行目に出ていて、押すと段の入力欄がその場で開く
  const card = await cardWithOneSet(page);
  const row = card.getByTestId('set-row').nth(0);

  await expect(card.getByTestId('note-toggle')).toHaveAttribute('aria-expanded', 'false');
  await row.getByTestId('drop-add').click();
  // 1 段目の重量は既定の落とし幅 20% で先に入る（60 → 48）
  await expect(row.getByTestId('drop-weight')).toHaveValue('48');
  await row.getByTestId('drop-reps').fill('5');
  // メモは一度も開いていない
  await expect(card.getByTestId('note-toggle')).toHaveAttribute('aria-expanded', 'false');

  const db = await readDb(page);
  expect(Object.values(db.sessions)[0].logs[0].sets).toEqual([
    { weight: 60, reps: 10, drops: [{ weight: 48, reps: 5 }] },
  ]);
});

test('2. 段の無いセットの保存 JSON にキーが増えない', async ({ page }) => {
  // ★ `skip_serializing_if` の回帰検出器。ここが崩れると、段を使っていない
  //   利用者の保存データが今までと変わる（smoke.spec.mjs の note 版と同じ主張）
  await cardWithOneSet(page);

  const db = await readDb(page);
  expect(Object.values(db.sessions)[0].logs[0].sets).toEqual([{ weight: 60, reps: 10 }]);
});

test('3. 段を消すと保存 JSON からキーが消える', async ({ page }) => {
  const card = await cardWithOneSet(page);
  const row = card.getByTestId('set-row').nth(0);
  await row.getByTestId('drop-add').click();
  await row.getByTestId('drop-reps').fill('5');
  await row.getByTestId('drop-remove').click();

  const db = await readDb(page);
  expect(Object.values(db.sessions)[0].logs[0].sets).toEqual([{ weight: 60, reps: 10 }]);
});

test('4. ★ `＋` は 1 行目に乗り、行の高さを変えない', async ({ page }) => {
  // ★ 44px の標的を 1 つ増やしたので、「縦は 1px も増えていない」が言えなければ
  //   adr/ux/exercise-and-set-notes-behind-one-toggle.md 決定 1 を覆した根拠が消える。
  //   iPhone 幅で回数欄の右端から ✕ の左端まで 94px 空いているのが前提
  const card = await cardWithOneSet(page);
  const row = card.getByTestId('set-row').nth(0);

  const add = await row.getByTestId('drop-add').boundingBox();
  const x = await row.getByTestId('remove-set').boundingBox();
  const reps = await row.getByTestId('set-reps').boundingBox();

  // ✕ と同じ行の同じ高さに乗っている（2 行目に落ちていない）
  expect(add.y + add.height / 2).toBeCloseTo(x.y + x.height / 2, 0);
  // 回数欄と ✕ のあいだに収まっている
  expect(add.x).toBeGreaterThanOrEqual(reps.x + reps.width - 1);
  expect(add.x + add.width).toBeLessThanOrEqual(x.x + 1);
  // 行は入力欄 1 段ぶんのまま
  const h = (await row.boundingBox()).height;
  expect(h, `行が ${h}px = 2 段になっている`).toBeLessThan(reps.height + 12);
});

test('5. `＋` は 44px より小さくならない', async ({ page }) => {
  // ★ 静かにするのは色と文字サイズだけで、当たり判定ではない
  //   （adr/ux/destructive-affordance-quiet-at-rest.md。note-toggle と同じ検査）
  const card = await cardWithOneSet(page);
  const box = await card.getByTestId('drop-add').nth(0).boundingBox();
  expect(box.height).toBeGreaterThanOrEqual(44);
  expect(box.width).toBeGreaterThanOrEqual(44);
});

test('6. 段があるときだけ行が出る', async ({ page }) => {
  // ★ 空の行を常に出すと `44px × セット行数` ぶんカードが伸びる。
  //   やらない日のほうが多い操作にその縦を払わない
  const card = await cardWithOneSet(page);
  await expect(card.getByTestId('drop-row')).toHaveCount(0);

  await card.getByTestId('set-row').nth(0).getByTestId('drop-add').click();
  await expect(card.getByTestId('drop-row')).toHaveCount(1);
});

test('7. ★ 段の ✕ はメインセットの ✕ と縦 1 列に並ぶ', async ({ page }) => {
  // ★ 横に流して折り返す形だと ✕ が段ごとに違う x に来て、どの行のものか読めない
  const card = await cardWithOneSet(page);
  const row = card.getByTestId('set-row').nth(0);
  // ★ ＋ はメインセット行と段の行の両方に出る。**先頭**（メインセット行）を押す
  await row.getByTestId('drop-add').first().click();
  await row.getByTestId('drop-add').first().click();

  const mainX = await row.getByTestId('remove-set').boundingBox();
  const stageXs = await row.getByTestId('drop-remove').all();
  expect(stageXs).toHaveLength(2);
  for (const [i, el] of stageXs.entries()) {
    const b = await el.boundingBox();
    expect(b.x + b.width / 2, `${i} 段目の ✕ が揃っていない`).toBeCloseTo(
      mainX.x + mainX.width / 2,
      0,
    );
  }
});

test('8. メモを開いても段の入力欄は変わらない', async ({ page }) => {
  // ★ 段の入力欄は `note_open` を見ない（足せるのに打てない状態を作らない）
  const card = await cardWithOneSet(page);
  const row = card.getByTestId('set-row').nth(0);
  await row.getByTestId('drop-add').click();
  await row.getByTestId('drop-reps').fill('5');

  await card.getByTestId('note-toggle').click();
  await expect(row.getByTestId('drop-weight')).toHaveValue('48');
  await card.getByTestId('note-toggle').click();
  await expect(row.getByTestId('drop-weight')).toHaveValue('48');
});

test('9. 段だけ足して回数が空の行は、落ちる理由が行に出る', async ({ page }) => {
  // ★ `commit` の `parse_reps` で落ちる行。黙って捨てない（メモだけの行と同じ扱い）
  const card = await addExercise(page, 'ベンチプレス');
  const row = card.getByTestId('set-row').nth(0);
  await row.getByTestId('set-weight').fill('60');
  await blurActive(page);
  await row.getByTestId('drop-add').click();

  await expect(row.getByTestId('note-orphan')).toBeVisible();
});

test('10. 段はリロードしても残る', async ({ page }) => {
  const card = await cardWithOneSet(page);
  const row = card.getByTestId('set-row').nth(0);
  await row.getByTestId('drop-add').click();
  await row.getByTestId('drop-reps').fill('5');

  await flushToStorage(page);
  await page.reload();
  await expect(page.getByTestId('screen-record')).toBeVisible();

  const back = page.getByTestId('exercise-card').getByTestId('set-row').nth(0);
  await expect(back.getByTestId('drop-weight')).toHaveValue('48');
  await expect(back.getByTestId('drop-reps')).toHaveValue('5');
});

// ── 落とし幅（%）────────────────────────────────────────────────────────────

test('11. 落とし幅を変えると 1 段目の重量が変わる', async ({ page }) => {
  await setDropPct(page, 12.5);
  const card = await cardWithOneSet(page);
  const row = card.getByTestId('set-row').nth(0);
  await row.getByTestId('drop-add').click();

  // 60 の 12.5% 引き = 52.5
  await expect(row.getByTestId('drop-weight')).toHaveValue('52.5');
  const ui = await uiState(page);
  expect(ui.drop_pct).toBe(12.5);
});

test('12. ★ 落とし幅を掛けるのは 1 段目だけ。2 段目以降は前の段をコピーする', async ({
  page,
}) => {
  // ★ 落とし幅をもう一度掛けない。実際の刻みは段ごとに変わる（20% → 20% とは
  //   限らない）ので、当たらない数字を作ることになる。前の段をそのまま持ってくれば
  //   「同じか、そこから下げる」のどちらでも打ち直しが最小になる
  //   （`add_row` が前の行の重量を引き継ぐのと同じ規則）
  const card = await cardWithOneSet(page);
  const row = card.getByTestId('set-row').nth(0);
  await row.getByTestId('drop-add').first().click();
  await row.getByTestId('drop-reps').nth(0).fill('5');
  await row.getByTestId('drop-add').first().click();

  // 60 の 20% 引き = 48。2 段目はその 48 をコピー（38.4 ではない）
  await expect(row.getByTestId('drop-weight').nth(0)).toHaveValue('48');
  await expect(row.getByTestId('drop-weight').nth(1)).toHaveValue('48');

  // 1 段目を打ち直してから足すと、その値がコピーされる
  await row.getByTestId('drop-weight').nth(1).fill('40');
  await row.getByTestId('drop-add').first().click();
  await expect(row.getByTestId('drop-weight').nth(2)).toHaveValue('40');
});

test('13. 重量の無いメインセットには何も入れない', async ({ page }) => {
  // ★ 掛ける相手が無いので、0 を入れるより空のまま打たせる
  const card = await addExercise(page, 'プッシュアップ');
  const row = card.getByTestId('set-row').nth(0);
  await row.getByTestId('set-reps').fill('20');
  await blurActive(page);
  await row.getByTestId('drop-add').click();

  await expect(row.getByTestId('drop-weight')).toHaveValue('');
});

test('14. 段は上限に達すると足せなくなる', async ({ page }) => {
  // ★ 押しても何も起きないボタンを作らない（`.pin-add` と同じ規則）
  const card = await cardWithOneSet(page);
  const row = card.getByTestId('set-row').nth(0);
  for (let i = 0; i < 4; i += 1) await row.getByTestId('drop-add').first().click();

  await expect(row.getByTestId('drop-row')).toHaveCount(4);
  // ★ 上限に達したら**どの行の ＋ も**出さない（押しても何も起きないボタンを作らない）
  await expect(row.getByTestId('drop-add')).toHaveCount(0);
});

// ── 推移タブ ────────────────────────────────────────────────────────────────

/** メインセット 60×10 と 60×6（段 50×5 / 40×4）の 1 日。除く 960 / 含む 1,370。 */
async function seedMainAndStages(page) {
  await seedPastLogs(page, [
    {
      daysAgo: 2,
      exerciseName: 'ベンチプレス',
      sets: [
        { weight: 60, reps: 10 },
        {
          weight: 60,
          reps: 6,
          drops: [
            { weight: 50, reps: 5 },
            { weight: 40, reps: 4 },
          ],
        },
      ],
    },
  ]);
}

test('15. 既定はグラフ・統計・記録一覧すべてから段が外れる', async ({ page }) => {
  await seedMainAndStages(page);
  await openProgress(page);

  await expect(page.getByTestId('stat-best')).toHaveText('960');
  const row = page.getByTestId('record-row').nth(0);
  await expect(row).toContainText('60×10  60×6');
  await expect(row).not.toContainText('50×5');
  await expect(row).toContainText('960');
  // 外したことを黙らない
  await expect(page.getByTestId('drops-hidden')).toBeVisible();
});

test('16. 「含める」に切り替えると推移タブ全体の数字が変わる', async ({ page }) => {
  await seedMainAndStages(page);
  await setDrops(page, 'include');
  await openProgress(page);

  await expect(page.getByTestId('stat-best')).toHaveText('1,370');
  const row = page.getByTestId('record-row').nth(0);
  // 段は ↓ を挟んでメインセットに続けて出す
  await expect(row).toContainText('60×6↓50×5↓40×4');
  await expect(row).toContainText('1,370');
  await expect(page.getByTestId('drops-hidden')).toHaveCount(0);
});

test('17. 設定は ui キーに入り、記録の JSON に混ざらない', async ({ page }) => {
  await seedMainAndStages(page);
  await setDrops(page, 'include');

  const ui = await uiState(page);
  expect(ui.drops).toBe(1);
  const db = await readDb(page);
  expect(JSON.stringify(db), '`Db` に設定が混ざっている').not.toContain('"drops":1');

  await page.getByTestId('tab-settings').click();
  await expect(page.getByTestId('settings-row-drop-sets')).toContainText('含める');
});

test('18. 件数の設定と巻き添えで消し合わない', async ({ page }) => {
  // ★ `UiState` は 1 フィールドの deserialize が落ちると全体が飛ぶ。
  //   書き込みが read-modify-write になっていることを見る
  await page.getByTestId('tab-settings').click();
  await page.getByTestId('settings-row-history').click();
  await page.getByTestId('history-btn').and(page.locator('[data-count="2"]')).click();
  await page.getByTestId('settings-back').click();

  await setDrops(page, 'include');
  await setDropPct(page, 15);

  const ui = await uiState(page);
  expect(ui.drops).toBe(1);
  expect(ui.drop_pct).toBe(15);
  expect(ui.history, '件数の設定が巻き添えで消えている').toBe(2);
});

// ── 記録タブとの関係 ────────────────────────────────────────────────────────

test('19. 記録タブの合計は設定にかかわらず段も数える', async ({ page }) => {
  // ★ 設定は「推移の見せ方」であって記録ではない
  //   （adr/data-model/metric-is-a-view-setting.md）。その日やった仕事の合計から
  //   落としてはいけない
  const card = await cardWithOneSet(page);
  const row = card.getByTestId('set-row').nth(0);
  await row.getByTestId('drop-add').click();
  await row.getByTestId('drop-reps').fill('5');
  await blurActive(page);

  // 60×10 + 48×5 = 840。既定（推移では除外）でも当日合計は落とさない
  await expect(card.getByTestId('today-metric')).toHaveText('840');
});
