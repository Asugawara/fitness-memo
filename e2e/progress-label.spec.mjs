import { expect, test } from '@playwright/test';

// ラベルの色 + 推移タブのラベル絞り込み
// （adr/ux/label-colour-on-the-progress-dots.md）の E2E。
//
// ここでしか見られないのは 5 つ。
//
// 1. **色ピッカーが `Db` まで届くこと。** `<input type="color">` は Playwright が
//    value 設定 + input/change で埋める特殊な入力で、`on:input` の配線が実際に
//    走るかはブラウザに聞くしかない
// 2. **チップが「グラフの点数」と「記録テーブルの行数」を同時に切り替えること。**
//    `core.rs` の unit test は `pick_points` の意味論までしか言えない。2 つの Memo を
//    同じ `filter` に繋いだかは画面でしか確かめられない
// 3. **点の色が本当に CSS に載ること。** `--dot` は継承変数なので、置き場所を
//    1 つ間違える（`svg` や `.chart-wrap` に置く）と全点が同色になるが、
//    Rust 側はコンパイルを通る。computed style を読む以外に検出手段が無い
// 4. **既定色が重ならず、`--accent` とも衝突しないこと。** パレットを部位と分けた
//    理由そのもの（2 本目が無ラベル点と見分けられない、を型で消したかった）
// 5. **`.lbl` の退行の見張り。** 推移タブのために `.dot` を足したので、`.lbl` を
//    flex 化してしまうと記録タブの 1 文字チップの文字が左寄せになる。既存 E2E は
//    チップの寸法しか見ていないのでこの退行を捕まえられない
//
// 正規化・パレットの規則・週集約でのラベルの畳み方は `src/core.rs` の unit test が
// 総当りしているので、ここでは追わない（label.spec.mjs と core.rs の分担と同じ）。

const STORAGE_KEY = 'fitness-memo/v3';

test.beforeEach(async ({ page }) => {
  // ★ baseURL がサブパス（/fitness-memo/）を持つとき先頭 "/" はベースを丸ごと捨てる
  await page.goto('./');
  await expect(page.getByTestId('screen-record')).toBeVisible();
});

// ── ヘルパ（e2e/label.spec.mjs からコピー）──────────────────────────────────
//
// ★ spec 単体で読める作法（pins.spec.mjs / interval.spec.mjs と同じ）。

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
 *   引く）。
 */
async function defineHPS(page) {
  const sheet = await openExerciseEditor(page, '胸', 'ベンチプレス');
  await addLabels(page, sheet, ['H', 'P', 'S']);
  await backToRecord(page);
}

// ── ここからこの spec 固有 ──────────────────────────────────────────────────

/**
 * H(9 日前) / P(7 日前) / ラベルなし(5 日前) / P(2 日前) + 別種目 1 件。
 *
 * ★ **4 点に留める。** `chart_layout::DENSE_POINTS`（40 点）を超えると最新点しか
 *   描かれず、点の色が読めない（ADR の結果に書いた数字そのもの）。
 * ★ プッシュアップは「ラベル定義が 0 本の種目」の枝を見るために要る。
 */
const PROGRESS_HISTORY = [
  { daysAgo: 9, exerciseName: 'ベンチプレス', label: 'H', sets: [{ weight: 70, reps: 10 }] },
  { daysAgo: 7, exerciseName: 'ベンチプレス', label: 'P', sets: [{ weight: 100, reps: 3 }] },
  { daysAgo: 5, exerciseName: 'ベンチプレス', sets: [{ weight: 80, reps: 8 }] },
  { daysAgo: 2, exerciseName: 'ベンチプレス', label: 'P', sets: [{ weight: 105, reps: 3 }] },
  { daysAgo: 3, exerciseName: 'プッシュアップ', sets: [{ weight: 0, reps: 20 }] },
];

/**
 * 既にあるセッションに体重を足す。
 *
 * ★ 体重があると、指標の系列が空でもグラフは描かれる（第2軸だけが残る）。
 *   `chart-metric-empty` の枝はこの状態でしか出ないので、そこだけで使う。
 */
async function seedBodyWeights(page, entries) {
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
      for (const { daysAgo: ago, kg } of entries) {
        const k = dateKey(ago);
        const session = db.sessions[k] ?? { logs: [], body_weight: null, note: '' };
        session.body_weight = kg;
        db.sessions[k] = session;
      }
      localStorage.setItem(key, JSON.stringify(db));
    },
    { entries, key: STORAGE_KEY },
  );
  await page.reload();
  await expect(page.getByTestId('screen-record')).toBeVisible();
}

/** `Db` に保存されているラベル定義（`{name, color}` の配列）。 */
async function storedLabels(page, exerciseName = 'ベンチプレス') {
  await flushToStorage(page);
  return await page.evaluate(
    ({ key, exerciseName }) => {
      const db = JSON.parse(localStorage.getItem(key));
      const ex = db.exercises.find((e) => e.name === exerciseName);
      // `labels` は `skip_serializing_if = "Vec::is_empty"` なので 0 本なら欄ごと無い
      return (ex.labels ?? []).map((l) => ({ name: l.name, color: l.color }));
    },
    { key: STORAGE_KEY, exerciseName },
  );
}

async function openProgress(page) {
  await blurActive(page);
  await page.getByTestId('tab-progress').click();
  await expect(page.getByTestId('screen-progress')).toBeVisible();
}

function chips(page) {
  return page.getByTestId('progress-label-chip');
}

/** ラベル名 → 推移タブのチップ。「すべて」は `data-label-any` で引く。 */
function chip(page, name) {
  return chips(page).filter({ hasText: exactText(name) });
}

function anyChip(page) {
  return page.locator('[data-testid=progress-label-chip][data-label-any]');
}

/** `#rrggbb` → `rgb(r, g, b)`（computed style の綴りに合わせる）。 */
function hexToRgb(hex) {
  const n = parseInt(hex.slice(1), 16);
  return `rgb(${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255})`;
}

/** 選択マーカーではない素の点の `fill`（系列順 = 日付昇順）。 */
async function dotFills(page) {
  return await page
    .locator('[data-testid=chart] circle.chart-dot:not(.selected)')
    .evaluateAll((els) => els.map((el) => getComputedStyle(el).fill));
}

/** `--accent` を computed style と同じ `rgb(...)` の綴りで返す。 */
async function accentRgb(page) {
  return await page.evaluate(() => {
    const hex = getComputedStyle(document.documentElement).getPropertyValue('--accent').trim();
    const n = parseInt(hex.slice(1), 16);
    return `rgb(${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255})`;
  });
}

/** H / P / S を定義し、履歴を入れて推移タブを開く（ほぼ全テストの前提）。 */
async function setupProgress(page) {
  await defineHPS(page);
  await seedPastLogs(page, PROGRESS_HISTORY);
  await openProgress(page);
  // 既定の対象は記録がある先頭の種目 = 胸 / ベンチプレス
  await expect(page.getByTestId('chart')).toHaveAttribute('data-points', '4');
}

// ── 1. 色ピッカーが Db まで届く ─────────────────────────────────────────────

test('1. ラベルの色を選ぶと保存データに入る', async ({ page }) => {
  const sheet = await openExerciseEditor(page, '胸', 'ベンチプレス');
  await addLabels(page, sheet, ['H', 'P']);

  // ★ `<input type="color">` は `fill` が value 設定 + input/change で埋める。
  //   アプリ側は `on:input` で即 commit（`group-color` と同じ）
  await sheet.getByTestId('label-color').nth(1).fill('#123456');

  const labels = await storedLabels(page);
  expect(labels.map((l) => l.name)).toEqual(['H', 'P']);
  expect(labels[1].color).toBe('#123456');
  // ★ 隣の行を巻き込んでいない（`<For>` の key と `change_label_color` の絞り込み）
  expect(labels[0].color).not.toBe('#123456');
});

// ── 2. チップ行の有無と既定 ─────────────────────────────────────────────────

test('2. チップ行はラベル定義がある種目にだけ出る', async ({ page }) => {
  await setupProgress(page);

  // 「すべて」+ H / P / S の 4 個。先頭が点いている
  await expect(chips(page)).toHaveCount(4);
  await expect(anyChip(page)).toHaveAttribute('aria-pressed', 'true');
  await expect(chip(page, 'P')).toHaveAttribute('aria-pressed', 'false');

  // ★ ラベル定義が 0 本の種目では行ごと出さない（「すべて」しか無い行は
  //   操作でも情報でもない）
  await page.getByTestId('exercise-select').selectOption({ label: 'プッシュアップ' });
  await expect(page.getByTestId('progress-label-row')).toHaveCount(0);
});

// ── 3. グラフと記録テーブルが同時に動く ─────────────────────────────────────

test('3. チップでグラフの点数と記録テーブルの行数が同時に絞られる', async ({ page }) => {
  await setupProgress(page);
  const chart = page.getByTestId('chart');
  await expect(page.getByTestId('record-row')).toHaveCount(4);

  await chip(page, 'P').click();
  await expect(chart).toHaveAttribute('data-points', '2');
  await expect(page.getByTestId('record-row')).toHaveCount(2);
  await expect(chip(page, 'P')).toHaveAttribute('aria-pressed', 'true');
  await expect(anyChip(page)).toHaveAttribute('aria-pressed', 'false');

  // 記録が 1 件も無いラベルでは、種目ごと空とは別の文言を出す（戻し方が読める）。
  // ★ 体重を 1 件も入れていないので `layout` は空 = `<svg>` ごと出ない
  //   （`chart-empty` に置き換わる）。行き止まりの説明はテーブル側が担う
  await chip(page, 'S').click();
  await expect(page.getByTestId('chart-empty')).toBeVisible();
  await expect(chart).toHaveCount(0);
  await expect(page.getByTestId('records-empty')).toHaveText(
    'この期間、このラベルの記録はありません',
  );

  // 「すべて」で戻る
  await anyChip(page).click();
  await expect(chart).toHaveAttribute('data-points', '4');
  await expect(page.getByTestId('record-row')).toHaveCount(4);
});

// ── 4. 点の色 ───────────────────────────────────────────────────────────────

test('4. データ点がラベルの色になり、ラベルなしは --accent のまま', async ({ page }) => {
  await setupProgress(page);
  const labels = await storedLabels(page);
  const colorOf = (name) => hexToRgb(labels.find((l) => l.name === name).color);
  const accent = await accentRgb(page);

  // 日付昇順: H(9 日前) / P(7 日前) / なし(5 日前) / P(2 日前)
  expect(await dotFills(page)).toEqual([colorOf('H'), colorOf('P'), accent, colorOf('P')]);

  // ★ `--dot` は継承変数。`svg` や `.chart-wrap` に置いていたら全点が同色になる
  expect(new Set(await dotFills(page)).size, '全点が同色になっている').toBe(3);

  // 折れ線は色を変えない（ラベルは「その日の狙い」、線は狙いをまたいだ推移）
  const line = await page
    .locator('[data-testid=chart] polyline.chart-line')
    .evaluate((el) => getComputedStyle(el).stroke);
  expect(line).toBe(accent);

  // 絞っても残った点の色は変わらない
  await chip(page, 'P').click();
  expect(await dotFills(page)).toEqual([colorOf('P'), colorOf('P')]);
});

// ── 5. 44px ─────────────────────────────────────────────────────────────────

test('5. 推移タブのチップも 44px を割らない', async ({ page }) => {
  await setupProgress(page);
  const n = await chips(page).count();
  expect(n).toBe(4);
  for (let i = 0; i < n; i++) {
    const box = await chips(page).nth(i).boundingBox();
    // ★ **判定側を丸める。** Pixel 7 の DPR 2.625 では 43.99993896484375 になる
    expect(Math.round(box.height), `チップ ${i} の高さ`).toBeGreaterThanOrEqual(44);
    expect(Math.round(box.width), `チップ ${i} の幅`).toBeGreaterThanOrEqual(44);
  }
});

// ── 6. 構造の契約 ───────────────────────────────────────────────────────────

test('6. チップ行を足しても推移タブに素のボタンが増えない', async ({ page }) => {
  await setupProgress(page);
  // ★ UA 既定のボタンは約 20px しかない（adr/ux/declare-color-scheme-for-ua-widgets.md）
  await expect(page.locator('[data-testid=screen-progress] button:not([class])')).toHaveCount(0);
});

// ── 7. 種目をまたいで絞りを持ち越さない ─────────────────────────────────────

test('7. 種目を切り替えると「すべて」に戻る', async ({ page }) => {
  await setupProgress(page);
  await chip(page, 'P').click();
  await expect(chip(page, 'P')).toHaveAttribute('aria-pressed', 'true');

  // ★ ラベルは種目ごとに独立した ID。持ち越すと門番が `Any` へ倒すので
  //   「どのチップも点いていないのに隠れた選択が残る」状態になる
  await page.getByTestId('exercise-select').selectOption({ label: 'プッシュアップ' });
  await expect(page.getByTestId('progress-label-row')).toHaveCount(0);

  await page.getByTestId('exercise-select').selectOption({ label: 'ベンチプレス' });
  await expect(anyChip(page)).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByTestId('chart')).toHaveAttribute('data-points', '4');
});

// ── 8. 既定色 ───────────────────────────────────────────────────────────────

test('8. 新しいラベルの既定色は重ならず、--accent とも衝突しない', async ({ page }) => {
  await defineHPS(page);
  const colors = (await storedLabels(page)).map((l) => l.color);
  expect(colors).toHaveLength(3);

  for (const c of colors) expect(c, `色が #rrggbb でない: ${c}`).toMatch(/^#[0-9a-f]{6}$/i);
  expect(new Set(colors).size, `既定色が重複している: ${colors}`).toBe(3);

  // ★ **パレットを部位（`COLOR_CHOICES`）と分けた理由そのもの。** `#2f7fd1`（背中）は
  //   `--accent` と見分けが付かず、HPS の 2 本目が無ラベル点に紛れる
  await openProgress(page);
  const accent = await accentRgb(page);
  for (const c of colors) {
    expect(hexToRgb(c), `既定色が --accent と同色: ${c}`).not.toBe(accent);
  }
});

// ── 9. `.lbl` の退行の見張り ────────────────────────────────────────────────

test('9. 記録タブの 1 文字チップの文字は中央のまま', async ({ page }) => {
  await defineHPS(page);
  const card = await addExercise(page, 'ベンチプレス');
  const h = card.getByTestId('label-chip').filter({ hasText: exactText('H') });
  await expect(h).toBeVisible();

  // ★ **`.lbl` を flex コンテナにすると、`min-width: 44px` で広がった 1 文字チップの
  //   文字が中央から左寄せへ退行する。** 推移タブのために足した `.dot` は
  //   `.lbl > .dot { display: inline-block }` で blockify してあり、`.lbl` 自体の
  //   `display` は触っていない。既存 E2E はチップの寸法しか見ていないのでここで見る
  const gap = await h.evaluate((el) => {
    const range = document.createRange();
    range.selectNodeContents(el);
    const text = range.getBoundingClientRect();
    const chip = el.getBoundingClientRect();
    return Math.abs(text.left + text.width / 2 - (chip.left + chip.width / 2));
  });
  expect(gap, `1 文字チップの文字が中央から ${gap}px ずれている`).toBeLessThanOrEqual(2);
});

// ── 10. 絞って空になったときの文言（グラフ側） ──────────────────────────────

test('10. 体重の線だけ残ったときも、絞りが理由なら文言がそう言う', async ({ page }) => {
  await defineHPS(page);
  await seedPastLogs(page, PROGRESS_HISTORY);
  // ★ 体重があると指標が 0 件でもグラフは描かれる（第2軸だけが残る）。
  //   `chart-metric-empty` はこの状態でしか出ない枝
  await seedBodyWeights(page, [
    { daysAgo: 9, kg: 70 },
    { daysAgo: 2, kg: 70.5 },
  ]);
  await openProgress(page);

  const note = page.getByTestId('chart-metric-empty');
  // 絞っていなければ 4 点あるので注記そのものが出ない
  await expect(note).toHaveCount(0);

  // ★ 「この種目の記録はありません」だと嘘になる（種目には在る）。
  //   「すべて」に戻せば見えることが読めないと行き止まり
  await chip(page, 'S').click();
  await expect(note).toHaveText('この期間、このラベルの記録はありません');

  await anyChip(page).click();
  await expect(note).toHaveCount(0);
});

// ── 11. 選択中チップの文字が読める ──────────────────────────────────────────
//
// ★ **退行の再現があるので入れる。** `.lbl.on`（ベタ塗り + `--accent-text`）と
//   `.selectors .lbl`（推移タブだけ地色を `--surface` に上げる）は詳細度が同じ (0,2,0) で、
//   ソース順で後ろが勝つ。門番（`:not(.on)`）を外すと選択中チップだけ地色が戻り、
//   文字色 `--accent-text`（light では #ffffff）だけが残って**白地に白文字**になる。
//   点灯しているかどうかは `aria-pressed` で読めるので E2E は緑のままで、
//   図（`public/manual/*/progress-labels.webp`）に丸だけのチップが焼き込まれていた。
// ★ 記録タブのチップ（`.selectors` の外）も同時に見る。門番を「`.lbl.on` を後ろへ
//   動かす」形で入れると今度はそちらのベタ塗りが崩れうるので、両面を 1 本で固定する。
// ★ 作法は e2e/backup.spec.mjs の「シート内のボタンの文字が背景から読める」と同じ
//   （トークン値をベタ書きせずコントラスト比で見るので、ライト / ダーク両方で成立する）。

/** `rgb(r, g, b)` / `rgba(...)` を [r,g,b] にする。 */
function parseRgb(value) {
  const nums = value.match(/[\d.]+/g);
  return nums ? nums.slice(0, 3).map(Number) : null;
}

/** WCAG の相対輝度。 */
function luminance([r, g, b]) {
  const lin = [r, g, b].map((v) => {
    const c = v / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * lin[0] + 0.7152 * lin[1] + 0.0722 * lin[2];
}

/**
 * 要素の文字色と**実効**背景色のコントラスト比。
 *
 * ★ 背景が透明なら祖先を辿る。辿らないと `rgba(0, 0, 0, 0)` を拾って**通ってしまう**。
 */
async function contrastRatio(locator) {
  const pair = await locator.evaluate((el) => {
    const color = getComputedStyle(el).color;
    let node = el;
    while (node) {
      const bg = getComputedStyle(node).backgroundColor;
      if (bg && bg !== 'transparent' && !/rgba\(0, 0, 0, 0\)/.test(bg)) {
        return { color, background: bg };
      }
      node = node.parentElement;
    }
    return { color, background: 'rgb(255, 255, 255)' };
  });
  const fg = luminance(parseRgb(pair.color));
  const bg = luminance(parseRgb(pair.background));
  const [hi, lo] = fg > bg ? [fg, bg] : [bg, fg];
  return (hi + 0.05) / (lo + 0.05);
}

for (const scheme of ['light', 'dark']) {
  test(`11. ${scheme} で選択中のラベルチップの文字が背景から読める`, async ({ page }) => {
    await page.emulateMedia({ colorScheme: scheme });
    await setupProgress(page);

    // 既定で点いているのは「すべて」
    await expect(anyChip(page)).toHaveAttribute('aria-pressed', 'true');
    expect(
      await contrastRatio(anyChip(page)),
      `${scheme} で推移タブの「すべて」が背景に埋もれている`,
    ).toBeGreaterThanOrEqual(4.5);

    // 名前付きのラベルを選んだときも同じこと
    await chip(page, 'H').click();
    await expect(chip(page, 'H')).toHaveAttribute('aria-pressed', 'true');
    expect(
      await contrastRatio(chip(page, 'H')),
      `${scheme} で推移タブの「H」が背景に埋もれている`,
    ).toBeGreaterThanOrEqual(4.5);

    // ★ 記録タブのチップは `.selectors` の外。ここが道連れになっていないこと
    //   （門番を「`.lbl.on` を後ろへ動かす」形で入れると、今度はこちらが崩れうる）
    await page.getByTestId('tab-record').click();
    await expect(page.getByTestId('screen-record')).toBeVisible();
    const card = await addExercise(page, 'ベンチプレス');
    const any = card.locator('[data-testid=label-chip][data-label-any]');
    await expect(any).toHaveAttribute('aria-pressed', 'true');
    expect(
      await contrastRatio(any),
      `${scheme} で記録タブの「指定なし」が背景に埋もれている`,
    ).toBeGreaterThanOrEqual(4.5);
  });
}
