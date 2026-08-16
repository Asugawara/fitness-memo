import { test, expect } from '@playwright/test';

// 記録タブ（カレンダー + 選択日エディタ）と推移・設定タブの E2E。
// src/views/{day,calendar,mod,progress,chart,settings}.rs の data-testid を使う。
//
// 「前日の記録がある状態」は seedPastLogs() で localStorage に直接注入して作る。
// UI から過去日へ書き込む経路そのものの検証（ExerciseLog.at が null になること）は
// e2e/calendar.spec.mjs が担当する。ここは読み込み〜表示側を見る。

test.beforeEach(async ({ page }) => {
  // ★ baseURL がサブパス（例 /fitness-memo/）を持つとき、先頭 "/" は絶対パス参照として
  // ベースのパスを丸ごと破棄してしまう（new URL('/', 'http://h/sub/') === 'http://h/'）。
  // 相対参照の "./" でなければ E2E_BASE=/fitness-memo/ の重い側実行が壊れる
  await page.goto('./');
});

// hasText は部分一致なので "ベンチプレス" が "インクラインベンチプレス" にも
// マッチしてしまう。pick-exercise は全プリセットボタンで共有の testid なので、
// 名前での絞り込みは常に完全一致にする
function exactText(s) {
  return new RegExp(`^${s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}$`);
}

/**
 * フォーカスを外す。
 *
 * ★ 入力欄にフォーカスが残っていると `.app` に `kb-open` が付き、styles.css の
 *   `.kb-open .bottom-tabs { display: none }` でタブバーごと消える（iOS の
 *   キーボード対策なので設計どおり）。blur せずにタブを押すとクリックがタイムアウトする。
 */
async function blurActive(page) {
  await page.evaluate(() => {
    if (document.activeElement instanceof HTMLElement) document.activeElement.blur();
  });
}

/**
 * 設定タブの部位カード。
 *
 * ★ hasText を group-item に直接掛けない。開いている部位のカードは所属種目の名前も
 *   含むので、部位名が種目名の部分文字列だと誤マッチする。has: で group-name に
 *   絞ってから完全一致させる。
 */
function groupItem(page, name) {
  return page.getByTestId('group-item').filter({
    has: page.getByTestId('group-name').filter({ hasText: exactText(name) }),
  });
}

/**
 * 設定タブの節（`settings-row-*`）へ入る。
 *
 * ★ 設定タブの入口は節の一覧（adr/ux/settings-as-a-list-of-sections.md）なので、
 *   部位や種目を触るテストは必ずここを通す。
 * ★ **既に入っていることがある。** 開いている節は `SettingsPageCtx` が持っていて
 *   タブ往復では戻らない（`OpenGroupCtx` と同じ理由）ので、行が出ているときだけ押す。
 */
async function openSettingsSection(page, which) {
  await blurActive(page);
  await page.getByTestId('tab-settings').click();
  // ★ **別の節に居ることがある。** 開いている節は `SettingsPageCtx` が持っていて
  //   タブ往復では戻らないので、まずトップへ戻してから入る（「入っていない」
  //   ではなく「違う節に入っている」が抜けると、行が見えないまま素通りする）
  const back = page.getByTestId('settings-back');
  if (await back.isVisible()) await back.click();
  await page.getByTestId(`settings-row-${which}`).click();
}

/**
 * 設定タブの部位を開いてカードを返す。**「種目」節に入るところからやる。**
 *
 * ★ 種目一覧は既定で全部閉じている（adr/ux/menu-groups-as-single-open-accordion.md）。group-toggle はトグルなので、
 *   既に開いているものを押すと閉じてしまう。aria-expanded を見てから押す。
 */
async function openGroup(page, name) {
  await openSettingsSection(page, 'exercises');
  const item = groupItem(page, name);
  const toggle = item.getByTestId('group-toggle');
  if ((await toggle.getAttribute('aria-expanded')) !== 'true') await toggle.click();
  return item;
}

/** 「種目を追加」シートからプリセットを選び、追加されたカードを返す。 */
async function addExercise(page, name) {
  // ★ 入力欄にフォーカスが残ると .kb-open で追加ボタンごと隠れる（iOS でキーボードの
  //   裏に回るのを避ける仕様）。連続で種目を足すテストのために毎回 blur してから押す
  await blurActive(page);
  await page.getByTestId('add-exercise').click();
  await page
    .getByTestId('add-sheet')
    .getByTestId('pick-exercise')
    .filter({ hasText: exactText(name) })
    .click();
  return page.getByTestId('exercise-card');
}

/**
 * hidden への visibilitychange を発火させ、pending の debounce 保存を即時 flush する。
 * 単純な reload だけだと 400ms debounce と race して flaky になる。
 */
async function flushToStorage(page) {
  // page.goto() の解決は wasm(数十MBのdebugビルド)のロード完了を保証しない。
  // today 画面が出た時点なら App() の Effect::new が既に一度走っており
  // PENDING に初期 Db が積まれているので、それを待ってから flush する
  await page.getByTestId('screen-record').waitFor({ state: 'visible' });
  await page.evaluate(() => {
    Object.defineProperty(document, 'hidden', { value: true, configurable: true });
    document.dispatchEvent(new Event('visibilitychange', { bubbles: true }));
  });
}

/**
 * 投入済みプリセットの Db に daysAgo 日前のセッションを追加してから reload する。
 * calendar.rs が無い現状で「前日の記録が既にある」状態を作る唯一の手段。
 * 同じ daysAgo を複数渡すと同一セッションに複数種目のログとして積む
 * （1日に複数種目を記録するのは通常の使い方なので、テストデータでも再現する）。
 *
 * `exerciseName` / `sets` は省略できる。`bodyWeight` だけ渡すと「計量しただけの日」
 * になる（体重は毎日、トレーニングは週数回、という実際の使い方を再現するため）。
 *
 * `atHour` / `atMinute` を渡すと「その日のその時刻に当日入力した」状態になる（`at` より
 * 優先）。★ 日付キーと同じ日の時刻でないと意味のないデータになるので、生の epoch を渡す
 * `at` ではなくこちらを使うこと。
 */
async function seedPastLogs(page, entries) {
  await flushToStorage(page);
  await page.evaluate((entries) => {
    const KEY = 'fitness-memo/v3';
    const db = JSON.parse(localStorage.getItem(KEY));

    // Local::now().date_naive() と揃えるため UTC (toISOString) ではなく
    // ローカルタイムゾーンの年月日から日付キーを組み立てる
    const dateKey = (daysAgo) => {
      const d = new Date();
      d.setDate(d.getDate() - daysAgo);
      const y = d.getFullYear();
      const m = String(d.getMonth() + 1).padStart(2, '0');
      const day = String(d.getDate()).padStart(2, '0');
      return `${y}-${m}-${day}`;
    };

    // dateKey と同じ日のローカル hh:mm の epoch。dateKey と同じくブラウザの TZ で
    // 計算されるので Local::now() と必ず揃う
    const atOnDay = (daysAgo, hour, minute) => {
      const d = new Date();
      d.setDate(d.getDate() - daysAgo);
      d.setHours(hour, minute, 0, 0);
      return d.getTime();
    };

    for (const { daysAgo, exerciseName, sets, at = null, atHour, atMinute = 0, bodyWeight } of entries) {
      const key = dateKey(daysAgo);
      const stamp = atHour === undefined ? at : atOnDay(daysAgo, atHour, atMinute);
      const session = db.sessions[key] ?? { logs: [], body_weight: null, note: '' };
      if (exerciseName !== undefined) {
        const ex = db.exercises.find((e) => e.name === exerciseName);
        if (!ex) throw new Error(`preset exercise not found: ${exerciseName}`);
        session.logs.push({ exercise_id: ex.id, sets, at: stamp });
      }
      if (bodyWeight !== undefined) session.body_weight = bodyWeight;
      db.sessions[key] = session;
    }
    localStorage.setItem(KEY, JSON.stringify(db));
  }, entries);
  await page.reload();
}

test('1. 初回起動でプリセットが投入され記録タブが出る', async ({ page }) => {
  await expect(page.getByTestId('screen-record')).toBeVisible();
  await expect(page.getByTestId('tab-record')).toHaveClass(/active/);

  // まだ何も記録していないので経過時間は「—」、部位チップは6部位分
  await expect(page.getByTestId('elapsed')).toHaveText('—');
  await expect(page.getByTestId('group-chip')).toHaveCount(6);

  // プリセットが投入されている（胸=ベンチプレス、背中=懸垂）ことをシートで確認
  await page.getByTestId('add-exercise').click();
  const sheet = page.getByTestId('add-sheet');
  await expect(sheet).toBeVisible();
  await expect(sheet.getByTestId('pick-exercise').filter({ hasText: exactText('ベンチプレス') })).toBeVisible();
  await expect(sheet.getByTestId('pick-exercise').filter({ hasText: exactText('懸垂') })).toBeVisible();
});

test('2. 種目を追加してセットを2行入力すると指標表示が正しい（60×10 + 60×8 = 1,080）', async ({ page }) => {
  const card = await addExercise(page, 'ベンチプレス');
  const rows = card.getByTestId('set-row');

  await rows.nth(0).getByTestId('set-weight').fill('60');
  await rows.nth(0).getByTestId('set-reps').fill('10');

  await card.getByTestId('add-set').click();
  await rows.nth(1).getByTestId('set-weight').fill('60');
  await rows.nth(1).getByTestId('set-reps').fill('8');

  await expect(card.getByTestId('today-metric')).toHaveText('1,080');
});

test('3. hidden への visibilitychange を発火してからリロードしても入力が残る', async ({ page }) => {
  const card = await addExercise(page, 'ベンチプレス');
  const row0 = card.getByTestId('set-row').nth(0);
  await row0.getByTestId('set-weight').fill('60');
  await row0.getByTestId('set-reps').fill('10');

  // 400ms debounce の完了を待たず、hidden 遷移で flush() が即時保存することを検証する
  await flushToStorage(page);
  await page.reload();

  const reloadedCard = page.getByTestId('exercise-card');
  await expect(reloadedCard.getByTestId('today-metric')).toHaveText('600');
  await expect(reloadedCard.getByTestId('set-row').nth(0).getByTestId('set-weight')).toHaveValue('60');
  await expect(reloadedCard.getByTestId('set-row').nth(0).getByTestId('set-reps')).toHaveValue('10');
});

test('4. 前日にバックフィルした記録があると、経過表示が「昨日」になる', async ({ page }) => {
  // at: null（バックフィル済み）で注入する。時刻を持たない記録でも日付キーから
  // 日数が出ることの検証（4d が「at があっても日付キーが勝つ」側を見る）
  await seedPastLogs(page, [
    { daysAgo: 1, exerciseName: 'ベンチプレス', sets: [{ weight: 60, reps: 10 }] },
  ]);

  await expect(page.getByTestId('elapsed')).toHaveText('昨日');
});

// ★ 4b〜4d は「経過日数はローカル暦の日差であって経過ミリ秒 / 24h ではない」ことを固定する。
//   以前は views 側が Exact(ms) を 86_400_000 で割っていたため、繰り上がりが JST の 0 時では
//   なくトレーニング時刻の 24 時間後に起きていた（朝トレなら UTC 深夜に日付が変わって見える）。
//   4 と 7 が at: null のバックフィルしか作っていなかったのが、この穴を通した理由。

test('4b. 昨夜 20:00 の記録を翌朝 8:00 に開くと「昨日」（経過 12 時間を「今日」にしない）', async ({ page }) => {
  await seedPastLogs(page, [
    { daysAgo: 1, exerciseName: 'ベンチプレス', sets: [{ weight: 60, reps: 10 }], atHour: 20 },
  ]);

  // 端末時計を「今日の 8:00」に固定する。経過 12 時間・暦では 1 日
  const morning = new Date();
  morning.setHours(8, 0, 0, 0);
  await page.clock.install({ time: morning });
  await page.reload();
  await expect(page.getByTestId('screen-record')).toBeVisible();

  await expect(page.getByTestId('elapsed')).toHaveText('昨日');
  const chest = page.getByTestId('group-chip').filter({ hasText: '胸' });
  await expect(chest).toContainText('1d');
  await expect(chest).toHaveAttribute('data-recency', 'fresh');
});

test('4c. 2 日前の夜の記録は「2日前」/「2d」で、チップの濃淡が recent に落ちる', async ({ page }) => {
  await seedPastLogs(page, [
    { daysAgo: 2, exerciseName: 'ベンチプレス', sets: [{ weight: 60, reps: 10 }], atHour: 20 },
  ]);

  const morning = new Date();
  morning.setHours(8, 0, 0, 0);
  await page.clock.install({ time: morning });
  await page.reload();
  await expect(page.getByTestId('screen-record')).toBeVisible();

  await expect(page.getByTestId('elapsed')).toHaveText('2日前');
  const chest = page.getByTestId('group-chip').filter({ hasText: '胸' });
  await expect(chest).toContainText('2d');
  await expect(chest).toHaveAttribute('data-recency', 'recent');
});

test('4d. 日付キーが at に勝つ（at に now が漏れていても「昨日」のまま）', async ({ page }) => {
  await seedPastLogs(page, [
    { daysAgo: 1, exerciseName: 'ベンチプレス', sets: [{ weight: 60, reps: 10 }], at: Date.now() },
  ]);

  await expect(page.getByTestId('elapsed')).toHaveText('昨日');
  await expect(page.getByTestId('group-chip').filter({ hasText: '胸' })).toContainText('1d');
});

test('5. セットが空のときだけ「前回をコピー」が出て、押すと前日のセットがプリフィルされる', async ({ page }) => {
  await seedPastLogs(page, [
    { daysAgo: 1, exerciseName: 'ベンチプレス', sets: [{ weight: 50, reps: 8 }] },
  ]);

  const card = await addExercise(page, 'ベンチプレス');

  // セットが空の間は「前回をコピー」が出る
  await expect(card.getByTestId('copy-last')).toBeVisible();

  await card.getByTestId('copy-last').click();

  const row0 = card.getByTestId('set-row').nth(0);
  await expect(row0.getByTestId('set-weight')).toHaveValue('50');
  await expect(row0.getByTestId('set-reps')).toHaveValue('8');

  // セットが入った後は「前回をコピー」が消える（置換か追記かの曖昧さを無くす設計）
  await expect(card.getByTestId('copy-last')).toHaveCount(0);
});

test('6. 体重・体調メモを入力するとリロード後も残る', async ({ page }) => {
  await page.getByTestId('condition-toggle').click();
  await page.getByTestId('body-weight').fill('62.5');
  await page.getByTestId('condition-note').fill('絶好調');

  await flushToStorage(page);
  await page.reload();

  // body_weight が非空で復元されれば ConditionRow は自動的に開いた状態で描画される
  await expect(page.getByTestId('body-weight')).toHaveValue('62.5');
  await expect(page.getByTestId('condition-note')).toHaveValue('絶好調');
});

test('7. 部位チップは実施部位に日数、未実施の部位には「—」を表示する', async ({ page }) => {
  await seedPastLogs(page, [
    { daysAgo: 3, exerciseName: 'ベンチプレス', sets: [{ weight: 60, reps: 10 }] }, // 胸
  ]);

  await expect(page.getByTestId('group-chip').filter({ hasText: '胸' })).toContainText('3d');
  await expect(page.getByTestId('group-chip').filter({ hasText: '背中' })).toContainText('—');
  await expect(page.getByTestId('group-chip').filter({ hasText: '体幹' })).toContainText('—');
});

test('9. 推移タブの種目別グラフに2点描かれ、重量なしの記録も指標を持つ', async ({ page }) => {
  await seedPastLogs(page, [
    { daysAgo: 2, exerciseName: 'ベンチプレス', sets: [{ weight: 60, reps: 10 }] },
    { daysAgo: 1, exerciseName: 'ベンチプレス', sets: [{ weight: 60, reps: 8 }] },
    { daysAgo: 1, exerciseName: '懸垂', sets: [{ weight: 0, reps: 12 }] },
  ]);

  await page.getByTestId('tab-progress').click();
  await expect(page.getByTestId('screen-progress')).toBeVisible();

  // 既定選択は記録のある最初の種目=ベンチプレス。2点入っていること
  const chart = page.getByTestId('chart');
  await expect(chart).toHaveAttribute('data-points', '2');
  // SVG 属性は無検査で setAttribute されるため、viewBox や stroke-width を snake_case で
  // 書くとコンパイルは通るのに実行時に黙って無視される罠がある。polyline が実際に
  // 描画されていることまで確認する（属性名だけでなく描画結果を見る）。
  // ★ クラスで限定する。素の polyline だと「体重を seed していない」ことへの
  //   暗黙依存になり、第2軸を足した瞬間に意味が変わる
  await expect(chart.locator('polyline.chart-line')).toHaveCount(1);
  await expect(page.getByTestId('stat-best')).toHaveText('600');

  // ★ 重量を入れない記録（懸垂 12 回）も 0 に潰れず 12 になる。
  //   指標は「重量 × 回数、重量が空なら重量 1」の単一式なので、種目ごとに
  //   式を切り替えなくても自重種目が実質レップ数として意味を持つ
  await page.getByTestId('target-select').selectOption({ label: '懸垂' });
  await expect(page.getByTestId('stat-best')).toHaveText('12');
});

test('推移タブの指標セグメントでボリューム / セット数 / 回数を切り替えられる', async ({ page }) => {
  await seedPastLogs(page, [
    {
      daysAgo: 1,
      exerciseName: 'ベンチプレス',
      sets: [
        { weight: 60, reps: 10 },
        { weight: 60, reps: 8 },
      ],
    },
  ]);

  await page.getByTestId('tab-progress').click();
  const best = page.getByTestId('stat-best');
  const metrics = page.getByTestId('metric-select').getByTestId('metric-btn');

  // 既定はボリューム。60×10 + 60×8 = 1,080（単位表記なし）
  await expect(best).toHaveText('1,080');

  await metrics.filter({ hasText: 'セット数' }).click();
  await expect(best).toHaveText('2 セット');

  await metrics.filter({ hasText: '回数' }).click();
  await expect(best).toHaveText('18 回');

  // ★ 単位は指標だけで決まる。対象種目を切り替えても軸の意味は変わらない
  //   （旧 Kind 方式では種目ごとに単位が変わっていた）
  await page.getByTestId('target-select').selectOption({ label: '胸' });
  await expect(best).toContainText('回');
});

test('推移タブの候補には記録のある種目だけが出る', async ({ page }) => {
  await seedPastLogs(page, [
    { daysAgo: 1, exerciseName: 'ベンチプレス', sets: [{ weight: 60, reps: 10 }] },
  ]);

  await page.getByTestId('tab-progress').click();

  const options = page.getByTestId('target-select').locator('option');
  await expect(options.filter({ hasText: exactText('ベンチプレス') })).toHaveCount(1);
  // プリセットは 28 種目あるが、使っていないものは並べない
  await expect(options.filter({ hasText: exactText('スクワット') })).toHaveCount(0);
  await expect(options.filter({ hasText: exactText('プランク') })).toHaveCount(0);
  // 記録のある種目を持たない部位も出ない
  await expect(options.filter({ hasText: exactText('脚') })).toHaveCount(0);
  await expect(options.filter({ hasText: exactText('胸') })).toHaveCount(1);
});

test('記録が 1 件も無いと推移タブは空状態の説明を出す', async ({ page }) => {
  await page.getByTestId('tab-progress').click();
  await expect(page.getByTestId('progress-empty')).toContainText('まだ記録がありません');
});

// worker-d が実機相当の網羅スイープで見つけた真バグの退行テスト。Y軸ラベルは
// text-anchor="end" で x = X0 - 5.0（プロット領域の左マージン内）に描かれる前提だが、
// volume が6桁以上になるとラベル文字数が伸びて viewBox（幅320）の左端から溢れ、
// 先頭の桁が欠けて数値を誤読させる（例: "2,954,576" が "954,576" に見える）。
// 単なるレイアウト崩れではなく数値の誤表示なので、force のような回避策は使わず
// SVG のレンダリング結果（getBBox）そのものを viewBox と突き合わせて検証する。
async function expectNoChartLabelOverflowsViewBox(page) {
  const chart = page.getByTestId('chart');
  await expect(chart).toBeVisible();
  const overflowing = await chart.evaluate((svg) => {
    const [, , viewWidth] = svg.getAttribute('viewBox').split(' ').map(Number);
    return Array.from(svg.querySelectorAll('text.chart-label'))
      .map((t) => {
        const box = t.getBBox();
        return { text: t.textContent, x: box.x, right: box.x + box.width };
      })
      .filter((l) => l.x < 0 || l.right > viewWidth);
  });
  expect(overflowing).toEqual([]);
}

test('推移タブのグラフ Y 軸ラベルは volume が6桁でも viewBox に収まる（先頭の桁が欠けない）', async ({ page }) => {
  await seedPastLogs(page, [
    // 1800 × 500 = 900,000（6桁）。y_max=990,000, 中間=495,000 もいずれも6桁
    { daysAgo: 1, exerciseName: 'ベンチプレス', sets: [{ weight: 1800, reps: 500 }] },
  ]);

  await page.getByTestId('tab-progress').click();
  await expectNoChartLabelOverflowsViewBox(page);
});

test('推移タブのグラフ Y 軸ラベルは volume が7桁でも viewBox に収まる（先頭の桁が欠けない）', async ({ page }) => {
  await seedPastLogs(page, [
    // 50000 × 100 = 5,000,000（7桁）
    { daysAgo: 1, exerciseName: 'ベンチプレス', sets: [{ weight: 50000, reps: 100 }] },
  ]);

  await page.getByTestId('tab-progress').click();
  await expectNoChartLabelOverflowsViewBox(page);
});

// ── 体重の第2軸 ─────────────────────────────────────────────────────────────
// 体重は毎日、トレーニングは週数回。指標の折れ線に体重を控えめな点線で常時重ねる。

test('体重を記録すると推移グラフに点線と右軸が出る', async ({ page }) => {
  await seedPastLogs(page, [
    { daysAgo: 4, exerciseName: 'ベンチプレス', sets: [{ weight: 60, reps: 10 }], bodyWeight: 70 },
    { daysAgo: 3, bodyWeight: 70.4 },
    { daysAgo: 2, exerciseName: 'ベンチプレス', sets: [{ weight: 62.5, reps: 10 }], bodyWeight: 70.2 },
    { daysAgo: 1, bodyWeight: 70.6 },
  ]);

  await page.getByTestId('tab-progress').click();
  const chart = page.getByTestId('chart');
  await expect(chart).toHaveAttribute('data-points', '2');
  await expect(chart).toHaveAttribute('data-weight-points', '4');

  // 体重の線は独立した polyline。メインの線とは別に描かれる
  await expect(chart.getByTestId('chart-weight')).toBeVisible();
  await expect(chart.locator('polyline.chart-line-weight')).toHaveCount(1);
  await expect(chart.locator('polyline.chart-line')).toHaveCount(1);
  // 右軸ラベルはグリッド 3 本に対応して 3 個
  await expect(chart.locator('text.chart-label-weight')).toHaveCount(3);
});

test('体重が無ければ第2軸は描かず、グラフの見た目は従来のまま', async ({ page }) => {
  await seedPastLogs(page, [
    { daysAgo: 2, exerciseName: 'ベンチプレス', sets: [{ weight: 60, reps: 10 }] },
    { daysAgo: 1, exerciseName: 'ベンチプレス', sets: [{ weight: 62.5, reps: 10 }] },
  ]);

  await page.getByTestId('tab-progress').click();
  const chart = page.getByTestId('chart');
  await expect(chart).toHaveAttribute('data-weight-points', '0');
  await expect(chart.getByTestId('chart-weight')).toHaveCount(0);
  await expect(chart.locator('text.chart-label-weight')).toHaveCount(0);
  await expect(page.getByTestId('readout-weight')).toHaveCount(0);

  // 右端は従来の X1（=310）のまま。右軸のぶん縮めるのは体重があるときだけ
  const gridRight = await chart.evaluate(
    (svg) => svg.querySelector('line.chart-grid').getAttribute('x2'),
  );
  expect(gridRight).toBe('310.0');
});

test('3桁の体重でも右軸ラベルが viewBox に収まる', async ({ page }) => {
  await seedPastLogs(page, [
    { daysAgo: 2, exerciseName: 'ベンチプレス', sets: [{ weight: 60, reps: 10 }], bodyWeight: 137.5 },
    { daysAgo: 1, exerciseName: 'ベンチプレス', sets: [{ weight: 60, reps: 10 }], bodyWeight: 135.5 },
  ]);

  await page.getByTestId('tab-progress').click();
  // 右軸ラベルにも chart-label を付けてあるので、既存ヘルパが無改修で拾う
  await expect(page.getByTestId('chart').locator('text.chart-label-weight')).toHaveCount(3);
  await expectNoChartLabelOverflowsViewBox(page);
});

// ★ X ドメインを両系列の合併にしていないと、最後にトレした日より後の計量が
//   軸の外に落ちて見えなくなる。毎日計量して週数回トレする使い方では常に起きる
test('最後にトレした日より後の計量まで X 軸が伸びる', async ({ page }) => {
  await seedPastLogs(page, [
    { daysAgo: 5, exerciseName: 'ベンチプレス', sets: [{ weight: 60, reps: 10 }], bodyWeight: 70 },
    { daysAgo: 4, exerciseName: 'ベンチプレス', sets: [{ weight: 62.5, reps: 10 }], bodyWeight: 70.2 },
    { daysAgo: 1, bodyWeight: 70.6 }, // 計量しただけの日
  ]);

  await page.getByTestId('tab-progress').click();
  const chart = page.getByTestId('chart');

  const expected = (daysAgo) => {
    const d = new Date();
    d.setDate(d.getDate() - daysAgo);
    return `${d.getMonth() + 1}/${d.getDate()}`;
  };
  const labels = await chart.evaluate((svg) =>
    Array.from(svg.querySelectorAll('text.chart-label'))
      .filter((t) => !t.classList.contains('chart-label-weight'))
      .map((t) => t.textContent),
  );
  expect(labels).toContain(expected(5)); // 左端は最初の記録
  expect(labels).toContain(expected(1)); // 右端は「計量しただけの日」
});

test('グラフをタップすると読み取り欄に日付・指標・体重が並ぶ', async ({ page }) => {
  await seedPastLogs(page, [
    { daysAgo: 3, exerciseName: 'ベンチプレス', sets: [{ weight: 60, reps: 10 }], bodyWeight: 70.5 },
    // 2 点目は体重を記録していない日
    { daysAgo: 1, exerciseName: 'ベンチプレス', sets: [{ weight: 62.5, reps: 10 }] },
  ]);

  await page.getByTestId('tab-progress').click();
  const readout = page.getByTestId('chart-readout');
  const hits = page.getByTestId('chart-hit');

  // 既定は最新点。その日は体重が無いので kg は出ない
  await expect(readout).toContainText('625');
  await expect(page.getByTestId('readout-weight')).toHaveCount(0);

  // 体重を記録した日へ移すと併記される
  await hits.nth(0).click();
  await expect(readout).toContainText('600');
  await expect(page.getByTestId('readout-weight')).toHaveText('70.5 kg');
});

// ★ aggregate_weekly（合計）に通すと週内の体重が足し上がる。体重は平均でなければ意味を持たない。
//   ★ 特定の 2 日が同じ週に入ることに賭けない（週境界は今日の曜日で動く）。
//     8 日連続で入れれば、どこで切れても必ずどれかの週に 2 日以上入る
test('全期間の体重は合計ではなく週平均で集計される', async ({ page }) => {
  const entries = [
    { daysAgo: 30, exerciseName: 'ベンチプレス', sets: [{ weight: 60, reps: 10 }] },
    { daysAgo: 1, exerciseName: 'ベンチプレス', sets: [{ weight: 60, reps: 10 }] },
  ];
  // 全部同じ 70.0 にしておくと、平均は必ず 70.0、合計なら 140 以上の週ができる
  for (let daysAgo = 0; daysAgo <= 7; daysAgo++) entries.push({ daysAgo, bodyWeight: 70 });
  await seedPastLogs(page, entries);

  await page.getByTestId('tab-progress').click();
  await page.getByTestId('period-select').getByTestId('period-btn')
    .filter({ hasText: '全期間' }).click();
  await expect(page.getByTestId('weekly-note')).toContainText('体重は週平均');

  // 平均なら全週 70.0 なので、帯は 70 を中央にした 69.5〜70.5 に落ち着く。
  // 合計だと 140 以上の週ができるので、この 3 つのラベルにはならない
  const labels = await page.getByTestId('chart').evaluate((svg) =>
    Array.from(svg.querySelectorAll('text.chart-label-weight')).map((t) => t.textContent),
  );
  expect(labels).toEqual(['70.5', '70', '69.5']);
});

// ★ 週平均に落とす経路（体重 45 点超）はブラウザでも一度は通しておく。
//   単体テストで座標は固めてあるが、SVG 属性の綴りは実行時に黙って無視されるため
test('毎日の記録が続いても体重の破線がプロット領域からはみ出さない', async ({ page }) => {
  const entries = [
    { daysAgo: 50, exerciseName: 'ベンチプレス', sets: [{ weight: 60, reps: 10 }] },
    { daysAgo: 1, exerciseName: 'ベンチプレス', sets: [{ weight: 62.5, reps: 10 }] },
  ];
  for (let daysAgo = 0; daysAgo <= 55; daysAgo++) {
    entries.push({ daysAgo, bodyWeight: 70 + (daysAgo % 5) * 0.2 });
  }
  await seedPastLogs(page, entries);

  await page.getByTestId('tab-progress').click();
  const chart = page.getByTestId('chart');
  // 56 点 > WEIGHT_DENSE_POINTS(45) なので描画は週平均に落ちている
  await expect(chart).toHaveAttribute('data-weight-smoothed', 'true');

  const outside = await chart.evaluate((svg) => {
    const line = svg.querySelector('polyline.chart-line-weight');
    // グリッド線の x1/x2 がプロット領域そのもの
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

// ★ 体重が f32 の上限まで素通りするため、1 点でも極端な値が混じると帯が潰れて
//   NaN 座標になり、SVG のパースエラーで折れ線が丸ごと消える（例外も出ない）
test('異常な体重が混じってもグラフの線が消えない', async ({ page }) => {
  await seedPastLogs(page, [
    { daysAgo: 3, exerciseName: 'ベンチプレス', sets: [{ weight: 60, reps: 10 }], bodyWeight: 70 },
    { daysAgo: 2, bodyWeight: 3e38 },
    { daysAgo: 1, exerciseName: 'ベンチプレス', sets: [{ weight: 62.5, reps: 10 }], bodyWeight: 70.5 },
  ]);

  await page.getByTestId('tab-progress').click();
  const chart = page.getByTestId('chart');
  await expect(chart.locator('polyline.chart-line')).toHaveCount(1);

  const points = await chart.evaluate((svg) =>
    Array.from(svg.querySelectorAll('polyline')).map((p) => p.getAttribute('points')),
  );
  for (const p of points) {
    expect(p).not.toContain('NaN');
    expect(p).not.toContain('inf');
  }
  // 異常値の日は系列から外れるので、描かれる体重は 2 点
  await expect(chart).toHaveAttribute('data-weight-points', '2');
});

// ★「常に一緒に見られる」が要件なので、その種目を休んでいた期間でも体重は出す。
//   ただし左軸を出すと y_max が 1.0 に化けて "1 / 0.5 / 0" が並び、体重の目盛りだと誤読される
test('その種目の記録が無い期間でも体重の点線だけは出る', async ({ page }) => {
  await seedPastLogs(page, [
    // ベンチは 4 ヶ月前だけ。既定期間（3M）には入らない
    { daysAgo: 120, exerciseName: 'ベンチプレス', sets: [{ weight: 60, reps: 10 }] },
    { daysAgo: 3, bodyWeight: 70 },
    { daysAgo: 2, bodyWeight: 70.4 },
    { daysAgo: 1, bodyWeight: 70.2 },
  ]);

  await page.getByTestId('tab-progress').click();
  const chart = page.getByTestId('chart');
  await expect(chart).toHaveAttribute('data-points', '0');
  await expect(chart).toHaveAttribute('data-weight-points', '3');
  await expect(chart.locator('polyline.chart-line')).toHaveCount(0);
  await expect(chart.locator('polyline.chart-line-weight')).toHaveCount(1);

  // 左軸ラベルは出さず、右軸だけ出す
  const leftLabels = await chart.evaluate((svg) =>
    Array.from(svg.querySelectorAll('text.chart-label'))
      .filter((t) => !t.classList.contains('chart-label-weight'))
      .filter((t) => Number(t.getAttribute('x')) < 40)
      .map((t) => t.textContent),
  );
  expect(leftLabels).toEqual([]);
  await expect(chart.locator('text.chart-label-weight')).toHaveCount(3);

  await expect(page.getByTestId('chart-metric-empty')).toContainText('記録はありません');
});

test('10. 同じ日に同じ種目を再度追加してもカードは増えず既存カードのまま', async ({ page }) => {
  const card = await addExercise(page, 'ベンチプレス');
  const row0 = card.getByTestId('set-row').nth(0);
  await row0.getByTestId('set-weight').fill('70');
  await row0.getByTestId('set-reps').fill('5');

  await expect(page.getByTestId('exercise-card')).toHaveCount(1);

  // 既に追加済みの種目をもう一度ピックしても新規カードは作られない
  await blurActive(page);
  await page.getByTestId('add-exercise').click();
  await page
    .getByTestId('add-sheet')
    .getByTestId('pick-exercise')
    .filter({ hasText: exactText('ベンチプレス') })
    .click();

  await expect(page.getByTestId('exercise-card')).toHaveCount(1);
  // 新規カードへの置き換えではなく既存カードのままであることの確認（入力内容が保持される）
  await expect(row0.getByTestId('set-weight')).toHaveValue('70');
});

test('11. 設定タブでの改名・部位変更・新規追加が記録タブに反映され、アーカイブは推移タブから参照できる', async ({ page }) => {
  await page.getByTestId('tab-settings').click();
  await expect(page.getByTestId('screen-settings')).toBeVisible();

  // 改名 + 部位変更（肩→腕）を1つの種目に対して行う
  await openGroup(page, '肩');
  await page.getByTestId('exercise-name').filter({ hasText: exactText('サイドレイズ') }).click();
  const menuSheet = page.getByTestId('settings-sheet');
  await expect(menuSheet).toBeVisible();

  await page.getByTestId('exercise-rename').fill('サイドレイズ改');
  await page
    .getByTestId('exercise-groups')
    .getByTestId('group-option')
    .filter({ hasText: exactText('腕') })
    .click();

  // ★ 種目は「指標の種類」を持たない。加重 / 自重 / 時間の区別は種目名から読めるので
  //   選ばせる意味が無かった（指標は core::set_volume の単一式に統一されている）
  await expect(menuSheet.getByTestId('kind-option')).toHaveCount(0);
  await expect(menuSheet).not.toContainText('種類');

  await page.getByTestId('settings-sheet-close').click();

  // 新規部位 + 新規種目を追加する
  await page.getByTestId('settings-add-group').click();
  await page.getByTestId('new-group-name').fill('テスト部位');
  await page.getByTestId('new-group-submit').click();

  // ★ 作った部位は自動で開く。中身が空なので、閉じたままだと「＋ 種目を追加」が
  //   見えず行き止まりに見える（adr/ux/menu-groups-as-single-open-accordion.md）
  const testGroupItem = groupItem(page, 'テスト部位');
  await expect(testGroupItem.getByTestId('group-toggle')).toHaveAttribute('aria-expanded', 'true');
  await testGroupItem.getByTestId('settings-add-exercise').click();
  await page.getByTestId('new-exercise-name').fill('テスト種目');
  await page.getByTestId('new-exercise-submit').click();

  // 今日タブの「種目を追加」シートに、改名後の名前・新規種目の両方が反映されている
  await page.getByTestId('tab-record').click();
  await page.getByTestId('add-exercise').click();
  const addSheet = page.getByTestId('add-sheet');
  await expect(addSheet.getByTestId('pick-exercise').filter({ hasText: exactText('サイドレイズ改') })).toBeVisible();
  await expect(addSheet.getByTestId('pick-exercise').filter({ hasText: exactText('サイドレイズ') })).toHaveCount(0);
  await expect(addSheet.getByTestId('pick-exercise').filter({ hasText: exactText('テスト種目') })).toBeVisible();

  // ★ 推移の候補は「記録がある種目」だけなので、アーカイブ後も参照できることを
  //   確かめるには先に 1 件記録しておく必要がある
  await addSheet.getByTestId('pick-exercise').filter({ hasText: exactText('テスト種目') }).click();
  const testCard = page
    .getByTestId('exercise-card')
    .filter({ has: page.getByTestId('card-name').filter({ hasText: exactText('テスト種目') }) });
  await testCard.getByTestId('set-reps').first().fill('10');
  // 入力欄にフォーカスが残ると .kb-open でタブバーごと消える（iOS のキーボード対策）。
  // blur しないと次のタブ切り替えがヒットターゲット判定で落ちる
  await blurActive(page);

  // アーカイブ: 今日タブの追加シートには出なくなるが、推移タブのセレクタでは
  // 末尾の「アーカイブ済み」セクションから参照できる（過去ログの exercise_id 参照を
  // 保つための論理削除なので、参照できなくなると過去データが見えなくなる）
  await page.getByTestId('tab-settings').click();
  // 開閉状態はタブを跨いで保たれる（OpenGroupCtx が App 側にある）ので、
  // openGroup は aria-expanded を見て既に開いていれば押さない
  await openGroup(page, 'テスト部位');
  await page.getByTestId('exercise-name').filter({ hasText: exactText('テスト種目') }).click();
  await page.getByTestId('archive-exercise').click();
  // ★ <dialog> は常時マウントなので消えない。「閉じている」は toBeHidden で見る
  //   （閉じた dialog は UA の display:none が効く）
  await expect(page.getByTestId('settings-sheet')).toBeHidden();

  await page.getByTestId('tab-record').click();
  await page.getByTestId('add-exercise').click();
  await expect(
    page.getByTestId('add-sheet').getByTestId('pick-exercise').filter({ hasText: exactText('テスト種目') }),
  ).toHaveCount(0);
  await page.getByTestId('add-sheet-close').click();

  await page.getByTestId('tab-progress').click();
  const archivedOptions = page.getByTestId('target-select').locator('optgroup[label="アーカイブ済み"] option');
  await expect(archivedOptions.filter({ hasText: exactText('テスト種目') })).toHaveCount(1);

  // 部位グループの削除ガード: アーカイブ済み種目も所属種目として数えるので削除できない。
  // ここが漏れるとアーカイブ済み種目の group_id が宙に浮き、過去ログの部位帰属
  // （カレンダーのドット色・部位別グラフ・今日タブの部位チップ）が壊れる
  await page.getByTestId('tab-settings').click();
  // 部位の編集は右端の鉛筆から（group-name は <span> になったので押せない）
  await groupItem(page, 'テスト部位').getByTestId('group-edit').click();
  await page.getByTestId('delete-group').click();
  // ★ 所属種目がアーカイブ済み 1 件だけなので、文言のほうがそれを名指しする。
  //   ヘッダの「N 種目」は非アーカイブしか数えない（ここでは 0）ので、
  //   「種目が 1 件あるため」とだけ言うと画面のどこを見ても理由が読めない
  await expect(page.getByTestId('delete-blocked')).toContainText(
    'アーカイブ済み種目が 1 件あるため削除できません',
  );
  await expect(page.getByTestId('delete-blocked-archived')).toHaveCount(0);
});

test('設定タブは部位だけを並べ、部位を開くとその部位の種目が出る（1 つだけ開く）', async ({ page }) => {
  await openSettingsSection(page, 'exercises');

  // ★ 改修の本体。既定で種目は 1 つも出ていない。6 部位 28 種目が全部並ぶと、
  //   どの部位があるかを見るだけで長いスクロールが要る（adr/ux/menu-groups-as-single-open-accordion.md）
  await expect(page.getByTestId('group-item')).toHaveCount(6);
  await expect(page.getByTestId('exercise-item')).toHaveCount(0);
  await expect(page.getByTestId('settings-add-exercise')).toHaveCount(0);

  // 閉じていても種目数はヘッダに出る（開かないと中身の量が分からない、を避ける）
  await expect(groupItem(page, '胸').getByTestId('group-count')).toHaveText('5 種目');

  const chest = await openGroup(page, '胸');
  await expect(chest.getByTestId('exercise-item')).toHaveCount(5);
  await expect(chest.getByTestId('settings-add-exercise')).toBeVisible();
  // 他の部位は閉じたままなので、画面全体で数えても 5 件
  await expect(page.getByTestId('exercise-item')).toHaveCount(5);

  // 別の部位を開くと前のが閉じる
  const back = await openGroup(page, '背中');
  await expect(chest.getByTestId('exercise-item')).toHaveCount(0);
  await expect(back.getByTestId('exercise-item')).toHaveCount(5);
  await expect(page.getByTestId('exercise-item')).toHaveCount(5);

  // 同じ部位をもう一度押すと閉じる
  await back.getByTestId('group-toggle').click();
  await expect(page.getByTestId('exercise-item')).toHaveCount(0);
  await expect(back.getByTestId('group-toggle')).toHaveAttribute('aria-expanded', 'false');
});

test('並び替えの矢印は種目にも部位にも無く、部位ヘッダの標的は 2 つだけ', async ({ page }) => {
  await page.getByTestId('tab-settings').click();
  await openGroup(page, '胸');

  // 退行の固定。一覧に 44px のボタンを並べ直さない（adr/ux/menu-groups-as-single-open-accordion.md）
  await expect(page.getByTestId('exercise-up')).toHaveCount(0);
  await expect(page.getByTestId('exercise-down')).toHaveCount(0);
  await expect(page.getByTestId('group-up')).toHaveCount(0);
  await expect(page.getByTestId('group-down')).toHaveCount(0);

  // ヘッダのタップ標的は「開閉」と「編集」だけ。ここが増えると部位を見渡せなくなる
  await expect(groupItem(page, '胸').locator('.card-head button')).toHaveCount(2);
  // 種目の行は名前ボタン 1 つだけ
  await expect(page.getByTestId('exercise-item').first().locator('button')).toHaveCount(1);
});

test('アーカイブから戻すと、戻した先の部位が画面に入り、種目は元の位置に帰る', async ({ page }) => {
  // ★ ここだけビューポートを固定する。検証したいのは「戻す先が画面外にあっても
  //   画面へ入ってくる」ことなので、端末によって一覧が丸ごと収まってしまうと
  //   （Pixel 7 は 915px 高）前提そのものが作れず、テストが何も見なくなる
  await page.setViewportSize({ width: 390, height: 480 });
  await openSettingsSection(page, 'exercises');
  const chest = await openGroup(page, '胸');
  await expect(chest.getByTestId('exercise-name').first()).toHaveText('ベンチプレス');

  await page.getByTestId('exercise-name').filter({ hasText: exactText('ベンチプレス') }).click();
  await page.getByTestId('archive-exercise').click();
  // シートは常時マウントなので toHaveCount(0) にはならない（adr/ux/native-dialog-for-sheets.md）
  await expect(page.getByTestId('settings-sheet')).toBeHidden();

  // 戻す先が閉じていて、かつ画面の外にある状態を作る。アーカイブ済みセクションは
  // 一覧の一番下なので、そこまでスクロールすると先頭の胸は上へ抜ける
  await openGroup(page, '体幹');
  await page.getByTestId('archived-section').scrollIntoViewIfNeeded();
  await expect(groupItem(page, '胸').getByTestId('group-toggle')).toHaveAttribute(
    'aria-expanded',
    'false',
  );
  await expect(groupItem(page, '胸')).not.toBeInViewport();

  await page.getByTestId('unarchive-exercise').click();

  const restored = groupItem(page, '胸');
  await expect(restored.getByTestId('group-toggle')).toHaveAttribute('aria-expanded', 'true');
  // ★ toBeVisible() では駄目。あれは bounding box が空でないことしか見ないので、
  //   画面の外で開いていても通る。それでは折りたたみが生む唯一の実害
  //   （「種目がどこへ戻ったのか分からない」）を固定できていない
  await expect(restored).toBeInViewport();
  // ★ 元の位置（先頭）へ帰ること。並び替えの UI が無いので、ここで末尾へ落ちると
  //   記録タブ「種目を追加」シートでも最下段に固定されたまま二度と直せない
  await expect(restored.getByTestId('exercise-name').first()).toHaveText('ベンチプレス');
});

test('自動で開いた部位は画面の中に入る（新規作成・部位移動）', async ({ page }) => {
  // 上のテストと同じ理由でビューポートを固定する（末尾に作った部位が
  // fold の下に来る状況を、どの project でも同じに作るため）
  await page.setViewportSize({ width: 390, height: 480 });
  await openSettingsSection(page, 'exercises');

  // 新規部位は一覧の末尾に作られる。開くだけでスクロールしないと sticky な
  // .add-wrap の裏か fold の下に隠れて、「＋ 種目を追加」に到達できない
  await page.getByTestId('settings-add-group').click();
  await page.getByTestId('new-group-name').fill('有酸素');
  await page.getByTestId('new-group-submit').click();
  const cardio = groupItem(page, '有酸素');
  await expect(cardio.getByTestId('group-toggle')).toHaveAttribute('aria-expanded', 'true');
  await expect(cardio).toBeInViewport();
  await expect(cardio.getByTestId('settings-add-exercise')).toBeInViewport();

  // 部位を移すと移動先が開く（adr/ux/menu-groups-as-single-open-accordion.md の自動展開のうちテストが無かったもの）
  await page.evaluate(() => window.scrollTo(0, 0));
  await openGroup(page, '胸');
  await page.getByTestId('exercise-name').filter({ hasText: exactText('プッシュアップ') }).click();
  await page
    .getByTestId('exercise-groups')
    .getByTestId('group-option')
    .filter({ hasText: exactText('有酸素') })
    .click();
  await page.getByTestId('settings-sheet-close').click();
  await expect(cardio.getByTestId('group-toggle')).toHaveAttribute('aria-expanded', 'true');
  await expect(cardio).toBeInViewport();
  await expect(
    cardio.getByTestId('exercise-name').filter({ hasText: exactText('プッシュアップ') }),
  ).toBeInViewport();
  // 移動元は閉じる（同時に開くのは 1 つ）
  await expect(groupItem(page, '胸').getByTestId('group-toggle')).toHaveAttribute(
    'aria-expanded',
    'false',
  );
});

test('開いていた部位を削除するとアコーディオンが閉じる', async ({ page }) => {
  await openSettingsSection(page, 'exercises');
  await page.getByTestId('settings-add-group').click();
  await page.getByTestId('new-group-name').fill('空の部位');
  await page.getByTestId('new-group-submit').click();

  const empty = groupItem(page, '空の部位');
  await expect(empty.getByTestId('group-toggle')).toHaveAttribute('aria-expanded', 'true');

  await empty.getByTestId('group-edit').click();
  await page.getByTestId('delete-group').click();
  await page.getByTestId('delete-group-confirm').click();

  // 消えた GroupId をシグナルに残さない。残っていても表示は壊れないが、
  // 開いている部位が 1 つも無い状態が正しい
  await expect(page.getByTestId('group-item')).toHaveCount(6);
  await expect(page.getByTestId('exercise-item')).toHaveCount(0);
});

test('開いた部位はタブを往復しても開いたまま', async ({ page }) => {
  // ★ 筋トレ中は記録⇄種目の往復が常なので、戻るたびに部位を探して押し直すのは
  //   「最短距離」に反する。シグナルを App 側（OpenGroupCtx）に置くことで保つ
  await page.getByTestId('tab-settings').click();
  await openGroup(page, '肩');

  await page.getByTestId('tab-record').click();
  await expect(page.getByTestId('screen-record')).toBeVisible();
  await page.getByTestId('tab-settings').click();

  await expect(groupItem(page, '肩').getByTestId('group-toggle')).toHaveAttribute(
    'aria-expanded',
    'true',
  );
  await expect(page.getByTestId('exercise-item')).toHaveCount(4);
});

test('アイコンの SVG が描画されている', async ({ page }) => {
  // ★ SVG に <?xml ?> や DOCTYPE が混ざると、innerHTML の HTML フラグメントパーサが
  //   bogus comment にして**エラーも出さずに**アイコンが 1 つも出なくなる
  //   （adr/architecture/help-figures-as-included-svg.md /
  //   adr/architecture/lucide-icons-as-included-svg.md）。個数を固定して黙って消えるのを防ぐ
  await openSettingsSection(page, 'exercises');
  await expect(page.locator('[data-testid=group-toggle] .icon > svg')).toHaveCount(6);
  await expect(page.locator('[data-testid=group-edit] .icon > svg')).toHaveCount(6);

  await groupItem(page, '胸').getByTestId('group-edit').click();
  await expect(page.locator('[data-testid=settings-sheet-close] .icon > svg')).toHaveCount(1);
  await page.getByTestId('settings-sheet-close').click();

  // 記録タブの月移動も同じ機構
  await page.getByTestId('tab-record').click();
  await expect(page.locator('[data-testid=cal-prev] .icon > svg')).toHaveCount(1);
  await expect(page.locator('[data-testid=cal-next] .icon > svg')).toHaveCount(1);
});

test('設定タブにクラスなしの button を作らない', async ({ page }) => {
  // adr/ux/declare-color-scheme-for-ua-widgets.md。UA 既定のボタンは約 20px しかない
  await page.getByTestId('tab-settings').click();
  await openGroup(page, '胸');
  await expect(page.locator('[data-testid=screen-settings] button:not([class])')).toHaveCount(0);

  await groupItem(page, '胸').getByTestId('group-edit').click();
  await expect(page.locator('[data-testid=settings-sheet] button:not([class])')).toHaveCount(0);
});

test('12. 重量入力の中間状態("6.")でクラッシュせず、"6.5"まで打つと指標に反映される', async ({ page }) => {
  const card = await addExercise(page, 'ベンチプレス');
  const row0 = card.getByTestId('set-row').nth(0);
  const weight = row0.getByTestId('set-weight');

  await row0.getByTestId('set-reps').fill('2');

  await weight.fill('6.');
  // type="number" ではなく type="text"+inputmode の検証: 中間状態が正規化/クリアされない
  await expect(weight).toHaveValue('6.');

  await weight.fill('6.5');
  await expect(weight).toHaveValue('6.5');

  // 6.5 × 2 = 13。"6." で止まっていた/クラッシュしていれば 12 のままか反映されない
  await expect(card.getByTestId('today-metric')).toHaveText('13');
});

// ── 実使用フィードバック（記録中の操作コストと誤操作）の退行テスト ──────────

test('「+ セット」で直前行の重量がコピーされ、回数欄にフォーカスが移る', async ({ page }) => {
  const card = await addExercise(page, 'ベンチプレス');
  const rows = card.getByTestId('set-row');

  await rows.nth(0).getByTestId('set-weight').fill('60');
  await rows.nth(0).getByTestId('set-reps').fill('10');

  await card.getByTestId('add-set').click();

  // ★ 重量は打ち直さない（60×10 / 60×8 / 60×6 のように据え置くのが普通）
  await expect(rows.nth(1).getByTestId('set-weight')).toHaveValue('60');
  await expect(rows.nth(1).getByTestId('set-reps')).toHaveValue('');

  // ★ 回数欄にフォーカスが来ているので、そのまま打てる（入力欄をタップしない）
  await expect(rows.nth(1).getByTestId('set-reps')).toBeFocused();
  await page.keyboard.type('8');
  await expect(card.getByTestId('today-metric')).toHaveText('1,080');

  // 重量だけ入った空行はゴーストセットにならない（parse_reps が None を返す）
  await card.getByTestId('add-set').click();
  await expect(card.getByTestId('today-metric')).toHaveText('1,080');
});

test('カードが増えても「種目を追加」が常に画面内にあり、force なしで押せる', async ({ page }) => {
  for (const name of ['ベンチプレス', '懸垂', 'ショルダープレス', 'バーベルカール', 'スクワット']) {
    const card = await addExercise(page, name);
    await card.getByTestId('set-reps').first().fill('10');
  }
  await blurActive(page);
  await expect(page.getByTestId('exercise-card')).toHaveCount(5);

  // ★ sticky が効いていないとカード列の末尾まで押しやられ、ビューポート外になる。
  //   force を付けないので、隠れていればここでタイムアウトする
  const add = page.getByTestId('add-exercise');
  // kb_blur の 150ms debounce が解けるまで .kb-open が残るので、可視化を待ってから測る
  await expect(add).toBeVisible();
  const viewport = page.viewportSize();
  const box = await add.boundingBox();
  expect(box, '「種目を追加」が描画されていない').not.toBeNull();
  expect(box.y + box.height, 'ビューポートの下にはみ出している').toBeLessThanOrEqual(
    viewport.height + 1,
  );
  await add.click();
  await expect(page.getByTestId('add-sheet')).toBeVisible();
});

// ★ 中身の有無で分岐しない（adr/ux/set-delete-without-confirmation.md）。消えるのは 1 行で打ち直しは数秒、対して
//   確認は 1 種目 3〜5 行 × 打ち間違いの消し直しのぶんだけ踏むことになる
test('中身のあるセットも確認を挟まず 1 タップで消える', async ({ page }) => {
  const card = await addExercise(page, 'ベンチプレス');
  const row0 = card.getByTestId('set-row').nth(0);
  await row0.getByTestId('set-weight').fill('60');
  await row0.getByTestId('set-reps').fill('10');
  await expect(card.getByTestId('today-metric')).toHaveText('600');

  await card.getByTestId('add-set').click();
  await card.getByTestId('set-row').nth(1).getByTestId('set-reps').fill('8');
  await expect(card.getByTestId('today-metric')).toHaveText('1,080');

  // 1 タップで消え、確認は一度も出ない
  await card.getByTestId('set-row').nth(1).getByTestId('remove-set').click();
  await expect(page.getByTestId('remove-set-confirm')).toHaveCount(0);
  await expect(card.getByTestId('set-row')).toHaveCount(1);
  await expect(card.getByTestId('today-metric')).toHaveText('600');
});

test('最後の 1 行を消しても入力欄は空行として残る', async ({ page }) => {
  // ★ 行が 0 本になるとカードから入力欄ごと消えて、打ち直す先が無くなる
  const card = await addExercise(page, 'ベンチプレス');
  await card.getByTestId('set-weight').first().fill('60');
  await card.getByTestId('set-reps').first().fill('10');
  await expect(card.getByTestId('today-metric')).toHaveText('600');

  await card.getByTestId('set-row').first().getByTestId('remove-set').click();
  await expect(card.getByTestId('set-row')).toHaveCount(1);
  await expect(card.getByTestId('set-weight').first()).toHaveValue('');
  await expect(card.getByTestId('set-reps').first()).toHaveValue('');
  await expect(card.getByTestId('today-metric')).toHaveText('0');
});

test('セット削除の確認 UI はどこにも生えない', async ({ page }) => {
  // ★ 退行の固定。確認を戻すならこのテストを消す判断を通すこと
  const card = await addExercise(page, 'ベンチプレス');
  await card.getByTestId('set-weight').first().fill('60');
  await card.getByTestId('set-reps').first().fill('10');

  await card.getByTestId('set-row').first().getByTestId('remove-set').click();
  await expect(page.getByTestId('remove-set-confirm')).toHaveCount(0);
  await expect(page.getByTestId('remove-set-yes')).toHaveCount(0);
  await expect(page.getByTestId('remove-set-no')).toHaveCount(0);
  await expect(page.locator('.row-confirm')).toHaveCount(0);
});

test('「この日から外す」はフッタにあり、セットがあれば確認を経由する', async ({ page }) => {
  const card = await addExercise(page, 'ベンチプレス');
  await card.getByTestId('set-reps').first().fill('10');
  await blurActive(page);

  // ★ 見出しの右端に削除ボタンを置かない（追加しようとして消す事故の元だった）
  await expect(card.locator('.card-head button')).toHaveCount(0);
  // 導線はフッタの中。専用の行を持たせない（カードが縦に伸びる）
  await expect(card.locator('.card-foot [data-testid=close-card]')).toHaveCount(1);

  await card.getByTestId('close-card').click();
  await expect(page.getByTestId('close-card-warning')).toContainText('記録が消えます');
  await expect(page.getByTestId('exercise-card')).toHaveCount(1);

  await page.getByTestId('close-card-no').click();
  await expect(page.getByTestId('exercise-card')).toHaveCount(1);

  await card.getByTestId('close-card').click();
  await page.getByTestId('close-card-yes').click();
  await expect(page.getByTestId('exercise-card')).toHaveCount(0);
});

test('セットが空のカードは確認なしで外れる（消えるものが無いため）', async ({ page }) => {
  // シートで種目を押し間違えた直後の取り消し。ここに確認を挟むと邪魔なだけで、
  // 「この日の記録が消えます」は消えるものが無いので嘘になる
  const card = await addExercise(page, 'ベンチプレス');
  await expect(card).toHaveCount(1);

  await card.getByTestId('close-card').click();
  await expect(page.getByTestId('close-card-warning')).toHaveCount(0);
  await expect(page.getByTestId('exercise-card')).toHaveCount(0);
});

test('重量だけ入って回数が空のカードも「空」扱いで確認なしに外れる', async ({ page }) => {
  // parse_reps が None を返して保存されない = 消えるものが無い。
  // 行削除の「重量プリフィルのみの行は確認しない」と同じ判定にそろえる
  const card = await addExercise(page, 'ベンチプレス');
  await card.getByTestId('set-weight').first().fill('60');
  await blurActive(page);

  await card.getByTestId('close-card').click();
  await expect(page.getByTestId('close-card-warning')).toHaveCount(0);
  await expect(page.getByTestId('exercise-card')).toHaveCount(0);
});

test('カード削除の入口は静止時に警告色を持たない（警告色は確認だけの持ち物）', async ({
  page,
}) => {
  const card = await addExercise(page, 'ベンチプレス');
  await card.getByTestId('set-reps').first().fill('10');
  await blurActive(page);

  // ★ トークン値をベタ書きせず「同じ色か」で見る。ライト / ダークどちらでも成立する
  const atRest = await card.evaluate((el) => ({
    remove: getComputedStyle(el.querySelector('[data-testid=close-card]')).color,
    foot: getComputedStyle(el.querySelector('.card-foot')).color,
    rowRemove: getComputedStyle(el.querySelector('[data-testid=remove-set]')).color,
  }));
  expect(atRest.remove, 'フッタと同じ --muted であること').toBe(atRest.foot);
  expect(atRest.remove, '行削除の ✕ と同じ --muted であること').toBe(atRest.rowRemove);

  // 確認を開いたときだけ --warn が出る
  await card.getByTestId('close-card').click();
  const warnColor = await card.evaluate((el) => getComputedStyle(el.querySelector('.warn-box')).color);
  expect(warnColor, '確認は入口と違う色（警告色）で出ること').not.toBe(atRest.remove);
});

test('カード削除の入口は「+ セット」と同じ列に無い', async ({ page }) => {
  // ★ 目視でしか気づけない退行の固定用。カードの右端は
  //   「行の ✕ → + セット → sticky の種目を追加」が並ぶ動線の列なので、
  //   破壊的操作をそこへ戻したらここで落ちる
  const card = await addExercise(page, 'ベンチプレス');
  await blurActive(page);

  const remove = await card.getByTestId('close-card').boundingBox();
  const addSet = await card.getByTestId('add-set').boundingBox();
  expect(remove, '外す導線が描画されていない').not.toBeNull();
  expect(addSet, '「+ セット」が描画されていない').not.toBeNull();
  expect(remove.x + remove.width, '「+ セット」と横位置が重なっている').toBeLessThan(addSet.x);

  // タップ標的は縮めない（静かにするのは色と文字サイズだけ）
  expect(remove.height).toBeGreaterThanOrEqual(44);
});

// ── 種目メモ / セットメモ（adr/ux/exercise-and-set-notes-behind-one-toggle.md）──
//
// ★ src/lib.rs が views を wasm32 に cfg ゲートしているので `cargo test` はこの経路を
//   通らない。**E2E がこの決定の唯一のカバレッジ**。

/** メモ欄を開く。トグルなので開閉状態を見てから押す。 */
async function openNotes(page, card) {
  await blurActive(page);
  const toggle = card.getByTestId('note-toggle');
  if ((await toggle.getAttribute('aria-expanded')) !== 'true') await toggle.click();
}

test('メモは既定で閉じていて、入口 1 つで種目メモと全セットのメモ欄が一斉に開く', async ({ page }) => {
  const card = await addExercise(page, 'ベンチプレス');
  const rows = card.getByTestId('set-row');
  await rows.nth(0).getByTestId('set-reps').fill('10');
  await card.getByTestId('add-set').click();
  await rows.nth(1).getByTestId('set-reps').fill('8');
  await blurActive(page);

  // 既定は閉。入力欄はどこにも無い
  const toggle = card.getByTestId('note-toggle');
  await expect(toggle).toHaveAttribute('aria-expanded', 'false');
  await expect(card.getByTestId('exercise-note')).toHaveCount(0);
  await expect(card.getByTestId('set-note')).toHaveCount(0);

  // 1 タップで種目メモ 1 本 + セット行と同数のメモ欄が出る
  await toggle.click();
  await expect(toggle).toHaveAttribute('aria-expanded', 'true');
  await expect(card.getByTestId('exercise-note')).toHaveCount(1);
  await expect(card.getByTestId('set-note')).toHaveCount(await rows.count());

  // もう一度押すと畳む
  await toggle.click();
  await expect(toggle).toHaveAttribute('aria-expanded', 'false');
  await expect(card.getByTestId('exercise-note')).toHaveCount(0);
  await expect(card.getByTestId('set-note')).toHaveCount(0);
});

test('行ごとのメモのトグルは生えない（入口はカード 1 枚に 1 つ）', async ({ page }) => {
  const card = await addExercise(page, 'ベンチプレス');
  await card.getByTestId('set-reps').first().fill('10');
  await openNotes(page, card);

  await expect(card.getByTestId('note-toggle')).toHaveCount(1);
  // 行ごとの入口を足すと入口が N 倍になる。要件の退行検知
  await expect(page.getByTestId('set-note-toggle')).toHaveCount(0);
});

test('メモは閉じても薄字で残り、リロードしても消えない', async ({ page }) => {
  const card = await addExercise(page, 'ベンチプレス');
  const rows = card.getByTestId('set-row');
  await rows.nth(0).getByTestId('set-reps').fill('10');
  await card.getByTestId('add-set').click();
  await rows.nth(1).getByTestId('set-reps').fill('8');
  await openNotes(page, card);

  await card.getByTestId('exercise-note').fill('調子は普通');
  await rows.nth(1).getByTestId('set-note').fill('3セット目で肩に違和感');
  await blurActive(page);

  // 閉じても読める
  await card.getByTestId('note-toggle').click();
  await expect(card.getByTestId('exercise-note-read')).toHaveText('調子は普通');
  await expect(card.getByTestId('set-note-read')).toHaveText('3セット目で肩に違和感');

  await flushToStorage(page);
  await page.reload();
  const again = page.getByTestId('exercise-card');
  // ★ リロード後は既定で閉じる（ConditionRow の「値があれば開く」を継がない）
  await expect(again.getByTestId('set-note')).toHaveCount(0);
  await expect(again.getByTestId('exercise-note-read')).toHaveText('調子は普通');
  await expect(again.getByTestId('set-note-read')).toHaveText('3セット目で肩に違和感');
});

test('メモの薄字は opacity ではなく色とサイズで作る', async ({ page }) => {
  const card = await addExercise(page, 'ベンチプレス');
  await card.getByTestId('set-reps').first().fill('10');
  await openNotes(page, card);
  await card.getByTestId('set-note').first().fill('きつい');
  await blurActive(page);
  await card.getByTestId('note-toggle').click();

  // ★ トークン値をベタ書きせず「前回の記録（.last-row）と同じ色か」で見る。
  //   ライト / ダークどちらでも成立する
  const style = await card.evaluate((el) => {
    const read = getComputedStyle(el.querySelector('[data-testid=set-note-read]'));
    return {
      opacity: read.opacity,
      color: read.color,
      fontSize: read.fontSize,
      lastRow: getComputedStyle(el.querySelector('.last-row')).color,
    };
  });
  expect(style.opacity, 'opacity で薄さを作ると非テキストの 3:1 を割る').toBe('1');
  expect(style.color, '前回の記録と同じ --muted であること').toBe(style.lastRow);
  expect(style.fontSize).toBe('12px');
});

test('メモの薄字と入力欄は左端がそろう（開閉で文字が横に動かない）', async ({ page }) => {
  const card = await addExercise(page, 'ベンチプレス');
  await card.getByTestId('set-reps').first().fill('10');
  await openNotes(page, card);
  await card.getByTestId('set-note').first().fill('きつい');
  await blurActive(page);

  const open = await card.getByTestId('set-note').first().boundingBox();
  await card.getByTestId('note-toggle').click();
  const closed = await card.getByTestId('set-note-read').first().boundingBox();

  expect(Math.abs(open.x - closed.x), '開閉で文字の左端が動く').toBeLessThanOrEqual(1);
});

test('メモの入口はフッタにあり、外す導線とも合計とも重ならない', async ({ page }) => {
  const card = await addExercise(page, 'ベンチプレス');
  await card.getByTestId('set-reps').first().fill('10');
  await blurActive(page);

  // フッタの中にある（ヘッダに戻したら落ちる）
  await expect(card.locator('.card-foot [data-testid=note-toggle]')).toHaveCount(1);
  await expect(card.locator('.card-head button')).toHaveCount(0);

  const toggle = await card.getByTestId('note-toggle').boundingBox();
  const close = await card.getByTestId('close-card').boundingBox();
  const total = await card.getByTestId('today-metric').boundingBox();
  expect(toggle.x, '外す導線と横位置が重なっている').toBeGreaterThan(close.x + close.width);
  expect(toggle.x + toggle.width, '合計と横位置が重なっている').toBeLessThan(total.x);

  // タップ標的は縮めない
  expect(toggle.height).toBeGreaterThanOrEqual(44);
  expect(toggle.width).toBeGreaterThanOrEqual(44);
});

test('メモを開いてもトグルがあった座標に破壊的操作が来ない', async ({ page }) => {
  // フッタが下がることを受け入れた代わりに、押した場所に別の（しかも破壊的な）操作が
  // 滑り込まないことを機械で固定する
  const card = await addExercise(page, 'ベンチプレス');
  const rows = card.getByTestId('set-row');
  await rows.nth(0).getByTestId('set-reps').fill('10');
  await card.getByTestId('add-set').click();
  await rows.nth(1).getByTestId('set-reps').fill('8');
  await blurActive(page);

  const before = await card.getByTestId('note-toggle').boundingBox();
  const point = { x: before.x + before.width / 2, y: before.y + before.height / 2 };
  await card.getByTestId('note-toggle').click();

  const hit = await page.evaluate(
    ({ x, y }) => document.elementFromPoint(x, y)?.closest('[data-testid]')?.dataset.testid ?? null,
    point,
  );
  expect(['remove-set', 'close-card'], `トグルの座標に ${hit} が来た`).not.toContain(hit);
});

test('「+ セット」はメモをプリフィルしない（重量だけ引き継ぐ）', async ({ page }) => {
  const card = await addExercise(page, 'ベンチプレス');
  const rows = card.getByTestId('set-row');
  await rows.nth(0).getByTestId('set-weight').fill('60');
  await rows.nth(0).getByTestId('set-reps').fill('10');
  await openNotes(page, card);
  await rows.nth(0).getByTestId('set-note').fill('きつい');
  await blurActive(page);

  await card.getByTestId('add-set').click();

  // 重量は計画値なので引き継ぐ。メモは観測値なので引き継がない
  await expect(rows.nth(1).getByTestId('set-weight')).toHaveValue('60');
  await expect(rows.nth(1).getByTestId('set-note')).toHaveValue('');
});

test('「前回をコピー」は前回のメモを持ち込まない', async ({ page }) => {
  await seedPastLogs(page, [
    {
      daysAgo: 3,
      exerciseName: 'ベンチプレス',
      sets: [{ weight: 60, reps: 10, note: '肩に違和感' }],
    },
  ]);
  const card = await addExercise(page, 'ベンチプレス');
  await card.getByTestId('copy-last').click();

  // セットは複製されるがメモは来ない
  await expect(card.getByTestId('set-row')).toHaveCount(1);
  await expect(card.getByTestId('set-weight').first()).toHaveValue('60');
  await expect(card.getByTestId('set-note-read')).toHaveCount(0);
  await openNotes(page, card);
  await expect(card.getByTestId('set-note').first()).toHaveValue('');
});

test('1 日分のメニューコピーもメモを持ち込まない', async ({ page }) => {
  await seedPastLogs(page, [
    {
      daysAgo: 3,
      exerciseName: 'ベンチプレス',
      sets: [{ weight: 60, reps: 10, note: '肩に違和感' }],
    },
  ]);
  await page.getByTestId('menu-candidate').first().click();

  const card = page.getByTestId('exercise-card').first();
  await expect(card.getByTestId('set-weight').first()).toHaveValue('60');
  await expect(card.getByTestId('set-note-read')).toHaveCount(0);
  await expect(card.getByTestId('exercise-note-read')).toHaveCount(0);
});

test('セットのメモは回数が無ければ保存されず、保存されない旨が行に出る', async ({ page }) => {
  const card = await addExercise(page, 'ベンチプレス');
  await openNotes(page, card);
  await card.getByTestId('set-note').first().fill('メモだけ');

  // 黙って捨てない
  await expect(card.getByTestId('note-orphan')).toBeVisible();

  // 回数を入れると保存対象になり、警告は消える
  await card.getByTestId('set-reps').first().fill('10');
  await expect(card.getByTestId('note-orphan')).toHaveCount(0);

  // 回数を消すと再び出る。その状態でリロードするとメモは残らない
  await card.getByTestId('set-reps').first().fill('');
  await expect(card.getByTestId('note-orphan')).toBeVisible();
  await blurActive(page);
  await flushToStorage(page);
  await page.reload();
  await expect(page.getByTestId('set-note-read')).toHaveCount(0);
});

test('種目メモだけの種目もカードとして残り、外すときは確認を経由する', async ({ page }) => {
  // ★ core::dedupe_logs のフィルタが `!sets.is_empty()` に戻ったらここで落ちる。
  //   画面に出ているメモが次回起動で消える退行を捕まえる唯一の網
  const card = await addExercise(page, 'ベンチプレス');
  await openNotes(page, card);
  await card.getByTestId('exercise-note').fill('肩が痛いのでやめた');
  await blurActive(page);

  await flushToStorage(page);
  await page.reload();

  const again = page.getByTestId('exercise-card');
  await expect(again).toHaveCount(1);
  await expect(again.getByTestId('exercise-note-read')).toHaveText('肩が痛いのでやめた');

  // セットは 1 本も無いが、消えるものはある。確認なしに消してはいけない
  await again.getByTestId('close-card').click();
  await expect(again.getByTestId('close-card-warning')).toBeVisible();
});

test('メモ欄にフォーカスするとタブバーが隠れ、blur で戻る', async ({ page }) => {
  // kb_focus / kb_blur の付け忘れ検知。メモ欄は IME 付きキーボードが出るので
  // 付け忘れると実機でタブバーがキーボードの裏に残る
  const card = await addExercise(page, 'ベンチプレス');
  await card.getByTestId('set-reps').first().fill('10');
  await openNotes(page, card);

  await card.getByTestId('set-note').first().focus();
  await expect(page.locator('.app')).toHaveClass(/kb-open/);

  await blurActive(page);
  await expect(page.locator('.app')).not.toHaveClass(/kb-open/);

  // 種目メモも同じ
  await card.getByTestId('exercise-note').focus();
  await expect(page.locator('.app')).toHaveClass(/kb-open/);
  await blurActive(page);
  await expect(page.locator('.app')).not.toHaveClass(/kb-open/);
});

test('メモを使っていないデータの保存 JSON に note キーが増えていない', async ({ page }) => {
  // ★ skip_serializing_if の退行検知。ここが崩れると保存形式が全利用者ぶん変わり、
  //   calendar.spec.mjs / backup.spec.mjs の toEqual([{weight, reps}]) も落ちる
  const card = await addExercise(page, 'ベンチプレス');
  await card.getByTestId('set-weight').first().fill('60');
  await card.getByTestId('set-reps').first().fill('10');
  await blurActive(page);
  await flushToStorage(page);

  const raw = await page.evaluate(() => localStorage.getItem('fitness-memo/v3'));
  const session = Object.values(JSON.parse(raw).sessions)[0];
  expect(session.logs[0].sets).toEqual([{ weight: 60, reps: 10 }]);
  expect(session.logs[0]).not.toHaveProperty('note');
});

// 以下2件は計画の12ケースには無い追加の退行テスト。worker-d が実機相当の検証で見つけた
// バグ（.bottom-tabs / backdrop / .sheet が全て position:fixed なのに z-index を
// 省いていたため、DOM順で <nav class="bottom-tabs"> が前面に出ていた）の固定用。
// 目視でしか気づけない類の退行なので、force を付けないクリックで機械的に検出する。
//
// ★ adr/ux/native-dialog-for-sheets.md でシートを <dialog> + show_modal() に移したので、2 件とも今は UA が
//   構造的に保証している（top layer は z-index の外・背景は inert）。それでも残すのは、
//   「シートの下端が押せること」「シート表示中に裏のタブへ抜けないこと」が要件そのもので、
//   実装をどう変えても守られるべきだから。手書きの重なり順に戻れば再び落ちる。

test('「種目を追加」シート最下部（体幹の最後の種目）がタブバーに隠れずクリックできる', async ({ page }) => {
  await page.getByTestId('add-exercise').click();
  const sheet = page.getByTestId('add-sheet');
  await expect(sheet).toBeVisible();

  // force を付けない: z-index が外れてタブバーに覆われた瞬間、ヒットターゲット判定で
  // 落ちてこの click がタイムアウトする。プリセット順で体幹の最後（レッグレイズ）が
  // 対象になるが、対象が何であれ「シートの一番下」を踏むことが重要
  const lastPick = sheet.getByTestId('pick-exercise').last();
  await lastPick.scrollIntoViewIfNeeded();
  await lastPick.click();

  await expect(page.getByTestId('exercise-card')).toHaveCount(1);
});

test('「種目を追加」シート表示中は背景が inert で、誤タップで別タブへ遷移しない', async ({ page }) => {
  await page.getByTestId('add-exercise').click();
  await expect(page.getByTestId('add-sheet')).toBeVisible();

  // show_modal() で開いた <dialog> の背景は UA が inert にする。ここが効かなくなると
  // この click が素通りして推移タブへ遷移する（隠れた種目を狙ったタップが誤タブ遷移に
  // なり入力を見失う）
  await expect(page.getByTestId('tab-progress').click({ timeout: 1000 })).rejects.toThrow();

  await expect(page.getByTestId('add-sheet')).toBeVisible();
  await expect(page.getByTestId('screen-progress')).toHaveCount(0);
});

// ネイティブ <dialog> に移して初めて成立した挙動（adr/ux/native-dialog-for-sheets.md）。手書きの
// <div role="dialog"> では Esc もフォーカス復帰も無かったので、退行したら気づけるようにする。

test('シートは Esc で閉じ、閉じたあとフォーカスが開いたボタンへ戻る', async ({ page }) => {
  const add = page.getByTestId('add-exercise');
  // ★ キーボードで開く。WebKit（Safari）はボタンを**クリック**してもフォーカスを
  //   与えない（実測で activeElement は BODY）ので、click で開くと「戻す先」が
  //   そもそも存在しない。フォーカス復帰が意味を持つのはキーボード操作の経路なので、
  //   その経路で検証する
  await add.focus();
  await add.press('Enter');
  await expect(page.getByTestId('add-sheet')).toBeVisible();

  await page.keyboard.press('Escape');
  await expect(page.getByTestId('add-sheet')).toBeHidden();

  await expect(add).toBeFocused();

  // ★ close request で閉じたときに Rust 側のシグナルが真のまま残ると
  //   「閉じたのに二度と開かない」になる。開き直せることまで見る
  await add.click();
  await expect(page.getByTestId('add-sheet')).toBeVisible();
});

test('シートは背景タップで閉じ、シートの中を突いても閉じない', async ({ page }) => {
  // ★ 入場アニメーション（0.22s）の最中に boundingBox を取ると、まだ下へ translate
  //   された箱が返る（実測: 高さ 720 の画面で y=687 / height=562）。動きを止めて測る
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.getByTestId('add-exercise').click();
  const sheet = page.getByTestId('add-sheet');
  await expect(sheet).toBeVisible();

  const box = await sheet.boundingBox();
  // 中身（見出し帯）を突く。target が <dialog> 自身にならないので閉じない
  await page.mouse.click(box.x + box.width / 2, box.y + 8);
  await expect(sheet).toBeVisible();

  // 箱の外＝::backdrop。ここだけが閉じる
  await page.mouse.click(box.x + box.width / 2, box.y - 24);
  await expect(sheet).toBeHidden();
});

test('動きを減らす設定ではシートが上下に動かない', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await page.getByTestId('add-exercise').click();
  const sheet = page.getByTestId('add-sheet');
  await expect(sheet).toBeVisible();

  // 既定では translate: 0 0（= "0px"）で下から上がる。reduce では none に倒して
  // 黒幕のフェードだけ残す
  await expect(sheet).toHaveCSS('translate', 'none');
});

// ★ 「シートを開くと ✕ に青枠が出る」の退行ガード（adr/ux/native-dialog-for-sheets.md）。
//   show_modal() の dialog focusing steps は「中の最初のフォーカス可能要素」を選ぶので、
//   放っておくと sheet-head の唯一のボタン = ✕ が初期フォーカスになる。**WebKit はその
//   初期フォーカスを :focus-visible にマッチさせる**ため、指でタップして開いただけで
//   閉じるボタンにリングが出る。利用者からは「何も押していないのに青い印が出る」と見える。
//
//   ★ これは **iPhone Safari 固有**（実測: WebKit は matches(':focus-visible') が true、
//   Chromium は false）。**chromium だけで回しても守れていない**ので、この 3 本は
//   iPhone 15 Pro プロジェクトでこそ意味がある。
//
//   見るのは 2 つ。(a) フォーカスの居場所（Rust 側 = dialog へ引き取れているか）と、
//   (b) 実際に描かれる outline（CSS 側）。WebKit は今度は <dialog> 自身を
//   :focus-visible にマッチさせるので、(a) だけでは (b) を担保できない。

test('シートを開いた直後のフォーカスは ✕ ではなくシート自身に載る', async ({ page }) => {
  await page.getByTestId('add-exercise').click();
  const sheet = page.getByTestId('add-sheet');
  await expect(sheet).toBeVisible();

  await expect(sheet).toBeFocused();

  const close = page.getByTestId('add-sheet-close');
  await expect(close).not.toBeFocused();
  // 守りたいのは「開いただけで青枠が出ないこと」なので、要件の言葉（outline）でも見る
  await expect(close).toHaveCSS('outline-style', 'none');
});

test('シート自身にフォーカスが載ってもリングは描かれない', async ({ page }) => {
  // ★ キーボード経路で開く。:focus-visible は直前の要素のリング状態を引き継ぐので、
  //   リングが出る条件はクリック経路よりこちらが厳しい。厳しいほうで測る
  const add = page.getByTestId('add-exercise');
  await add.focus();
  await add.press('Enter');
  const sheet = page.getByTestId('add-sheet');
  await expect(sheet).toBeVisible();
  await expect(sheet).toBeFocused();

  // .sheet は inset: auto 0 0 / width: 100% なので、リングが出ると左右と下は画面端へ落ち、
  // **上辺だけが横一文字の青線**として残る。<dialog> は操作対象ではないので消してある。
  // ★ WebKit では実際に :focus-visible がマッチしている（Chromium はしない）。つまり
  //   このアサーションが実質的に働くのは iPhone 15 Pro プロジェクトのほう
  await expect(sheet).toHaveCSS('outline-style', 'none');
});

test('シートに入って最初の Tab で閉じるボタンにリングが戻る', async ({ page, browserName }) => {
  // ★ WebKit は既定で Tab をボタンに止めない（Safari の「Tab キーでWebページ上の各項目を
  //   ハイライト」がオフのときの挙動で、Playwright の WebKit もこれに従う）。守りたいのは
  //   「tabindex="-1" が Tab 順を変えていないこと」なので、Tab がボタンに止まるエンジンで見る
  test.skip(browserName !== 'chromium', 'WebKit の Tab 移動は full keyboard access 設定に依存する');

  const add = page.getByTestId('add-exercise');
  await add.focus();
  await add.press('Enter');
  await expect(page.getByTestId('add-sheet')).toBeVisible();

  // ★ tabindex="-1" は Tab 順を変えない。初期フォーカスを外した代償として
  //   「キーボードで閉じるボタンへ行けない」「行っても見えない」が起きていないか
  await page.keyboard.press('Tab');
  await expect(page.getByTestId('add-sheet-close')).toBeFocused();
});

// ── 見出しの階層とフォーカスリング（adr/ux/focus-ring-and-heading-order.md）──

test('記録タブの h1 は 1 個だけで、選択日は h2 にぶら下がる', async ({ page }) => {
  // カレンダーの月見出しが h1、選択日が h2、種目カードが h3。
  // 両方 h1 にすると 1 画面に見出しの階層が 2 本立ち、アウトラインで前後が読めない
  await expect(page.locator('main h1')).toHaveCount(1);
  await expect(page.getByTestId('cal-title')).toHaveJSProperty('tagName', 'H1');
  await expect(page.getByTestId('today-date')).toHaveJSProperty('tagName', 'H2');

  await page.getByTestId('add-exercise').click();
  await page.getByTestId('pick-exercise').first().click();
  await expect(page.getByTestId('card-name').first()).toHaveJSProperty('tagName', 'H3');
});

test('他タブの h1 もそれぞれ 1 個', async ({ page }) => {
  await page.getByTestId('tab-progress').click();
  await expect(page.locator('main h1')).toHaveCount(1);
  await page.getByTestId('tab-settings').click();
  await expect(page.locator('main h1')).toHaveCount(1);
});

test('フォーカスリングが出て、塗りボタンでは地色側で抜く', async ({ page }) => {
  const add = page.getByTestId('add-exercise'); // .primary（--accent のベタ塗り）
  await add.focus();

  const ring = await add.evaluate((el) => {
    const s = getComputedStyle(el);
    return { width: s.outlineWidth, style: s.outlineStyle, color: s.outlineColor };
  });
  // UA 既定任せにしない。--accent の塗りの上では既定のリングが沈む
  expect(ring.style).toBe('solid');
  expect(parseFloat(ring.width)).toBeGreaterThan(0);

  // 塗りと同じ色のリングだと境界が消えるので、--accent-text 側で抜いている
  const accent = await page.evaluate(() =>
    getComputedStyle(document.documentElement).getPropertyValue('--accent').trim(),
  );
  const asRgb = await page.evaluate((c) => {
    const d = document.createElement('div');
    d.style.color = c;
    document.body.appendChild(d);
    const v = getComputedStyle(d).color;
    d.remove();
    return v;
  }, accent);
  expect(ring.color).not.toBe(asRgb);
});

test('アイコンボタンのリングはグリフ側に出て、44px の標的からはみ出さない', async ({ page }) => {
  // ★ 入場アニメーション中に測ると箱が下へ translate された状態で返る（背景タップの
  //   テストと同じ理由）。動きを止めてから測る
  await page.emulateMedia({ reducedMotion: 'reduce' });
  // ★ キーボード経路で開いてから ✕ へ移す。クリックで開いて focus() を当てると
  //   :focus-visible にマッチせず（直前の操作がポインタなので）、リングが出ていない状態を
  //   「消えている」と読み違える。Tab で辿らないのは、WebKit が既定で Tab をボタンに
  //   止めないため（full keyboard access 設定に依存する。Tab 順自体は別のテストで見る）
  const add = page.getByTestId('add-exercise');
  await add.focus();
  await add.press('Enter');
  await expect(page.getByTestId('add-sheet')).toBeVisible();
  const close = page.getByTestId('add-sheet-close');
  await close.focus();

  const m = await close.evaluate((el) => {
    const icon = el.querySelector('.icon');
    const s = getComputedStyle(icon);
    return {
      btn: getComputedStyle(el).outlineStyle,
      style: s.outlineStyle,
      ring:
        icon.getBoundingClientRect().height +
        (parseFloat(s.outlineOffset) + parseFloat(s.outlineWidth)) * 2,
      btnH: el.getBoundingClientRect().height,
    };
  });
  // 44px の箱に沿わせると外形 52px になり、上辺がシートの角丸の外＝黒幕の上へ出る。
  // グリフ（20px）基準なら外形 32px で、どの親に入れても標的の内側に収まる
  expect(m.btn, 'ボタン側のリングは消してある').toBe('none');
  expect(m.style, 'グリフ側にリングが出ている').toBe('solid');
  expect(m.ring).toBeLessThanOrEqual(m.btnH - 8);
});

// ── タブ切替 ────────────────────────────────────────────────────────────────

// タブ切替に演出は無く、押した瞬間に入れ替わる（adr/ux/directional-tab-transitions.md は破棄）。
// ここで守るのは `TabCtx::switch` の同値ガード。`RwSignal::set` は同値でも購読者へ通知するので、
// ガードを外すと <main class="screen"> の中身が丸ごと作り直される。
//
// ★ 「画面のルート要素が同一か」では見ない。leptos は同じ型の view を rebuild するとき
//   ルートのノードを使い回すので、ガードを外しても screen-* 要素そのものは残る（実測）。
//   壊れるのは中身のほうで、確定前の入力が消える。だから入力の生存で見る。
test('同じタブをもう一度押しても、確定前の入力が消えない', async ({ page }) => {
  await addExercise(page, 'ベンチプレス');

  // ★ 回数は空のまま置く。commit() は parse_reps が通らない行を落とすので、この "62." は
  //   Db に一度も載らず DayEditor のローカル signal の上にしか無い（いちばん脆い状態）
  await page.getByTestId('set-weight').first().fill('62.');
  await blurActive(page);

  await page.getByTestId('tab-record').click();

  // ★ ガードを外すと DayEditor が Db から作り直され、載っていないカードごと消える。
  //   「値が違う」ではなく「要素が無い」として出るので、先に枚数を見て失敗を読めるようにする
  await expect(page.getByTestId('exercise-card')).toHaveCount(1);
  await expect(page.getByTestId('set-weight').first()).toHaveValue('62.');
});

// ★ `view-transition-name` の値では見ない。`none` はこのプロパティの初期値なので、
//   何も宣言しなければ常に `none` を返す＝**失敗しえないテストになる**（一度書いて気づいた）。
//   おまけに見る向きが逆で、`view-transition-name: bottom-tabs` は「タブバーを root の
//   スナップショットから外す」ための宣言だった。演出側だけが戻るとタブバーごと横に流れるのに、
//   その形では緑のまま通ってしまう。演出が走るかは startViewTransition の呼び出しでしか分からない。
test('タブを切り替えても View Transition は走らない', async ({ page }) => {
  await page.addInitScript(() => {
    window.__vt = 0;
    const orig = document.startViewTransition?.bind(document);
    if (orig) {
      document.startViewTransition = (opts) => {
        window.__vt++;
        return orig(opts);
      };
    }
  });
  await page.reload();

  await page.getByTestId('tab-settings').click();
  await expect(page.getByTestId('screen-settings')).toBeVisible();
  await page.getByTestId('tab-progress').click();
  await expect(page.getByTestId('screen-progress')).toBeVisible();

  expect(await page.evaluate(() => window.__vt)).toBe(0);

  // CSS 側も残っていないこと。呼び出しだけ戻っても UA 既定のクロスフェードは出るので、
  // 両側から塞ぐ（tab-slide-* の @keyframes と ::view-transition-* の規則）
  const vtRules = await page.evaluate(
    () =>
      [...document.styleSheets]
        .flatMap((s) => {
          try {
            return [...s.cssRules];
          } catch {
            return [];
          }
        })
        .filter((r) => /view-transition|tab-slide/.test(r.cssText)).length,
  );
  expect(vtRules).toBe(0);
});

// ── 1 日丸ごとのメニューコピー ──────────────────────────────────────────────

test('空の日にだけ過去メニューの候補が出て、1 タップで種目とセットが丸ごと入る', async ({ page }) => {
  await seedPastLogs(page, [
    {
      daysAgo: 2,
      exerciseName: 'ベンチプレス',
      sets: [
        { weight: 60, reps: 10 },
        { weight: 60, reps: 8 },
      ],
    },
    { daysAgo: 2, exerciseName: 'チェストフライ', sets: [{ weight: 15, reps: 12 }] },
  ]);

  const list = page.getByTestId('menu-copy');
  const candidates = page.getByTestId('menu-candidate');
  await expect(list).toBeVisible();
  await expect(candidates).toHaveCount(1);
  // 部位だけでなく種目名まで出す（胸の日が 2 つ並んだとき部位名では選び分けられない）
  await expect(candidates.first()).toContainText('胸');
  await expect(candidates.first()).toContainText('ベンチプレス');

  await candidates.first().click();

  const cards = page.getByTestId('exercise-card');
  await expect(cards).toHaveCount(2);

  // ★ today-metric ではなく入力欄の値を検証する。Db は正しいのに <For> がカードを
  //   使い回して入力欄が前の値のまま、という状態を today-metric は素通ししてしまう
  const rows = cards.nth(0).getByTestId('set-row');
  await expect(rows).toHaveCount(2);
  await expect(rows.nth(0).getByTestId('set-weight')).toHaveValue('60');
  await expect(rows.nth(0).getByTestId('set-reps')).toHaveValue('10');
  await expect(rows.nth(1).getByTestId('set-reps')).toHaveValue('8');
  await expect(cards.nth(1).getByTestId('set-row').nth(0).getByTestId('set-reps')).toHaveValue('12');

  // 1 種目でも入ったら候補は消える（adr/ux/copy-button-only-when-empty.md と同じ「空のときだけ」の考え方）
  await expect(list).toHaveCount(0);

  // signal に載っただけでなく Db にコミットされている
  await flushToStorage(page);
  await page.reload();
  await expect(page.getByTestId('exercise-card')).toHaveCount(2);
  await expect(page.getByTestId('menu-copy')).toHaveCount(0);
});

test('部位が同じでも種目構成が違えば別々の候補として並ぶ', async ({ page }) => {
  await seedPastLogs(page, [
    // 3 日前と 5 日前はどちらも「胸」だが種目が違う（A/B 法）。
    // 重複排除を部位集合でやると 1 件に潰れ、「直前の日をコピー」に退化する
    { daysAgo: 3, exerciseName: 'ベンチプレス', sets: [{ weight: 60, reps: 10 }] },
    { daysAgo: 5, exerciseName: 'ダンベルプレス', sets: [{ weight: 22.5, reps: 10 }] },
    // 7 日前は 3 日前と同じ構成なので畳まれる
    { daysAgo: 7, exerciseName: 'ベンチプレス', sets: [{ weight: 55, reps: 10 }] },
  ]);

  const candidates = page.getByTestId('menu-candidate');
  await expect(candidates).toHaveCount(2);
  await expect(candidates.nth(0)).toContainText('ベンチプレス');
  await expect(candidates.nth(1)).toContainText('ダンベルプレス');
});

test('コピーできる種目が残っていない日は候補に出ない（押せない行を作らない）', async ({ page }) => {
  // ★ 空セットの日はここでは作れない。seedPastLogs は最後に reload するので
  //   core::migrate が「空セットのログしか無いセッション」を丸ごと捨てる。
  //   その状態を仕込んでも検証しているのは migrate であって候補判定ではない
  //   （空セットの除外は core の recent_menus_skips_days_with_nothing_copyable で見る）
  await seedPastLogs(page, [
    { daysAgo: 3, exerciseName: 'スクワット', sets: [{ weight: 80, reps: 5 }] },
  ]);
  await expect(page.getByTestId('menu-candidate')).toHaveCount(1);

  // アーカイブするとコピー対象から外れる。件数だけ数えて行を出していると、
  // 「1種目」と表示されるのに押しても何も起きない死んだボタンになる
  await blurActive(page);
  await page.getByTestId('tab-settings').click();
  await openGroup(page, '脚');
  await page.getByTestId('exercise-name').filter({ hasText: exactText('スクワット') }).click();
  await page.getByTestId('archive-exercise').click();
  await expect(page.getByTestId('settings-sheet')).toBeHidden();

  await page.getByTestId('tab-record').click();
  await expect(page.getByTestId('menu-copy')).toHaveCount(0);
});

// ── トレーニングメニュー ────────────────────────────────────────────────────
// adr/ux/start-from-a-saved-routine.md
// adr/ux/routine-editor-drag-and-accordion.md

/**
 * メニュー編集シートの種目ピッカーを全部開く。
 *
 * ★ **既定は全部閉じている**（adr/ux/routine-editor-drag-and-accordion.md）。
 *   `routine-pick` は開いている部位にしか存在しないので、押す前に必ずここを通す。
 * ★ 設定タブの部位（1 つだけ開く）と違い**複数同時に開ける**ので、全部押して回れる。
 *   どの種目がどの部位かをテスト側で知らずに済むのが狙い。
 * ★ `scope` は `page` でもシートの Locator でもよい（どちらも getByTestId を持つ）。
 */
async function openAllPickGroups(scope) {
  const toggles = scope.getByTestId('routine-group-toggle');
  const n = await toggles.count();
  expect(n, 'ピッカーの部位が 1 つも出ていない').toBeGreaterThan(0);
  for (let i = 0; i < n; i++) {
    const toggle = toggles.nth(i);
    if ((await toggle.getAttribute('aria-expanded')) !== 'true') await toggle.click();
  }
}

/** メニュー編集シートの部位アコーディオン 1 つ。名前は完全一致で絞る。 */
function pickGroup(page, name) {
  return page.getByTestId('routine-group-toggle').filter({
    has: page.getByTestId('routine-group-name').filter({ hasText: exactText(name) }),
  });
}

/**
 * 「選択中」の行のハンドル（番号）を掴んで dy だけ動かして離す。
 *
 * ★ **`page.mouse` で出す。** Chromium / WebKit とも本物の PointerEvent
 *   （pointerType: "mouse", isPrimary: true, button: 0）になり、setPointerCapture が
 *   実際に働く。`dispatchEvent` で合成した PointerEvent では capture が
 *   NotFoundError で失敗し、実装が意図どおり「掴まない」ので検証にならない
 *   （e2e/reorder.spec.mjs の冒頭に同じ断り書きがある）。
 * ★ 固定の待ち時間を入れない。`data-drag="lift"` が付くのを待てば掴めたことまで
 *   一緒に確かめられ、待っている間は指が 1px も動かない。
 * ★ 指を離す前に `hold` を呼べる。ドラッグ中の見え方（番号の入れ替わり）を見るため。
 */
async function dragPicked(page, index, dy, hold) {
  const row = page.getByTestId('routine-picked-row').nth(index);
  const handle = row.getByTestId('routine-handle');
  await handle.scrollIntoViewIfNeeded();
  const box = await handle.boundingBox();
  expect(box, 'ハンドルが画面に出ていること').not.toBeNull();
  const x = box.x + box.width / 2;
  const y = box.y + box.height / 2;

  await page.mouse.move(x, y);
  await page.mouse.down();
  await expect(row).toHaveAttribute('data-drag', 'lift');
  await page.mouse.move(x, y + dy, { steps: 8 });
  if (hold) await hold();
  await page.mouse.up();
  await expect(row).not.toHaveAttribute('data-drag', 'lift');
}

/** 「選択中」の 1 行の高さ。ドラッグの dy はここから出す（CSS を書き写さない）。 */
async function pickedRowHeight(page) {
  const box = await page.getByTestId('routine-picked-row').first().boundingBox();
  expect(box, '「選択中」に行が無い').not.toBeNull();
  return box.height;
}

/** 設定タブでメニューを 1 本作る。`names` の順がそのまま展開順になる。 */
async function createRoutine(page, name, names) {
  await openSettingsSection(page, 'routines');
  await page.getByTestId('settings-add-routine').click();
  await expect(page.getByTestId('settings-sheet')).toBeVisible();
  await page.getByTestId('routine-name-input').fill(name);
  await openAllPickGroups(page);
  for (const n of names) {
    await page.getByTestId('routine-pick').filter({ hasText: exactText(n) }).click();
  }
  await page.getByTestId('routine-save').click();
  await expect(page.getByTestId('settings-sheet')).toBeHidden();
}

test('★ 保存したメニューは種目ごとに別々の日の「前回」からセットを引く', async ({ page }) => {
  // ★ この機能の核。ベンチは 3 日前、スクワットは 10 日前にやっている。
  //   1 日を丸ごと写す menu-candidate と違い、種目ごとに別の日から入る
  await seedPastLogs(page, [
    {
      daysAgo: 3,
      exerciseName: 'ベンチプレス',
      sets: [
        { weight: 60, reps: 10 },
        { weight: 60, reps: 8 },
      ],
    },
    { daysAgo: 10, exerciseName: 'スクワット', sets: [{ weight: 80, reps: 5 }] },
  ]);

  await createRoutine(page, '胸と脚の日', ['ベンチプレス', 'スクワット']);

  await page.getByTestId('tab-record').click();
  const routines = page.getByTestId('routine-candidate');
  await expect(routines).toHaveCount(1);
  await expect(routines.first()).toContainText('胸と脚の日');
  // 種目名まで出す（「胸の日」と「胸の日（軽め）」を名前だけで選び分けさせない）
  await expect(routines.first()).toContainText('ベンチプレス');
  await expect(routines.first()).toContainText('胸');

  await routines.first().click();

  const cards = page.getByTestId('exercise-card');
  await expect(cards).toHaveCount(2);
  // 並びはメニューの並び（タップ順）
  await expect(cards.nth(0)).toContainText('ベンチプレス');
  await expect(cards.nth(1)).toContainText('スクワット');

  // ★ 入力欄の値で見る。Db は正しいのに <For> がカードを使い回して入力欄が
  //   古いまま、という状態を today-metric は素通ししてしまう
  const bench = cards.nth(0).getByTestId('set-row');
  await expect(bench).toHaveCount(2);
  await expect(bench.nth(0).getByTestId('set-weight')).toHaveValue('60');
  await expect(bench.nth(0).getByTestId('set-reps')).toHaveValue('10');
  await expect(bench.nth(1).getByTestId('set-reps')).toHaveValue('8');

  const squat = cards.nth(1).getByTestId('set-row');
  await expect(squat).toHaveCount(1);
  await expect(squat.nth(0).getByTestId('set-weight')).toHaveValue('80');
  await expect(squat.nth(0).getByTestId('set-reps')).toHaveValue('5');

  // 1 種目でも入ったら候補は消える（menu-candidate と同じ「空のときだけ」）
  await expect(page.getByTestId('menu-copy')).toHaveCount(0);

  // signal に載っただけでなく Db にコミットされている
  await flushToStorage(page);
  await page.reload();
  await expect(page.getByTestId('exercise-card')).toHaveCount(2);
});

test('★ 「選択中」をドラッグで並べ替えると、記録タブのカードの並びがそうなる', async ({ page }) => {
  // adr/ux/routine-editor-drag-and-accordion.md
  // ★ この並びは記録タブでの**消化順**そのもの。外して入れ直す以外に直す手段が
  //   無かったので、途中に 1 種目を差し込むには後ろを全部やり直すことになっていた
  await seedPastLogs(page, [
    { daysAgo: 3, exerciseName: 'ベンチプレス', sets: [{ weight: 60, reps: 10 }] },
    { daysAgo: 3, exerciseName: 'スクワット', sets: [{ weight: 80, reps: 5 }] },
  ]);
  await createRoutine(page, '胸と脚の日', ['ベンチプレス', 'スクワット']);

  await page.getByTestId('routine-open').click();
  await expect(page.getByTestId('settings-sheet')).toBeVisible();
  await expect(page.getByTestId('routine-picked-name')).toHaveText(['ベンチプレス', 'スクワット']);

  // 1 行ぶん下へ動かせば隣の中点（高さの半分 + 隙間）は確実に越える
  await dragPicked(page, 0, await pickedRowHeight(page));
  await expect(page.getByTestId('routine-picked-name')).toHaveText(['スクワット', 'ベンチプレス']);
  // 番号も入れ替わっている（番号 ＝ 順番、が指を離した後も保たれる）
  await expect(page.getByTestId('routine-handle')).toHaveText(['1', '2']);

  await page.getByTestId('routine-save').click();
  await expect(page.getByTestId('settings-sheet')).toBeHidden();
  await expect(page.getByTestId('routine-names')).toHaveText('スクワット, ベンチプレス');

  // ★ 記録タブで開いたカードの並びがそうなる（この機能の目的そのもの）
  await page.getByTestId('tab-record').click();
  await page.getByTestId('routine-candidate').first().click();
  await expect(page.getByTestId('card-name')).toHaveText(['スクワット', 'ベンチプレス']);

  // 保存された Db の並びも同じ（signal に載っただけではない）
  await flushToStorage(page);
  const raw = await page.evaluate(() => localStorage.getItem('fitness-memo/v3'));
  const db = JSON.parse(raw);
  const byId = new Map(db.exercises.map((e) => [e.id, e.name]));
  expect(db.routines[0].exercises.map((id) => byId.get(id))).toEqual([
    'スクワット',
    'ベンチプレス',
  ]);
});

test('ドラッグ中は番号が入れ替わって見える（落ちる先が先に読める）', async ({ page }) => {
  // ★ ドラッグ中は Vec を入れ替えず transform だけで見せるので、模型の添字と
  //   見えている位置がずれる。そのまま描くと「2 番目に見えている行に 1 と書いてある」に
  //   なり、番号をハンドルにした前提（番号 ＝ 順番）が指を離すまで嘘になる。
  //   CSS の counter() ではこれが出せないので、テキストで描いている
  await createRoutine(page, '3 種目', ['ベンチプレス', 'スクワット', 'プランク']);
  await page.getByTestId('routine-open').click();
  await expect(page.getByTestId('routine-handle')).toHaveText(['1', '2', '3']);

  await dragPicked(page, 0, await pickedRowHeight(page), async () => {
    // ★ **指を離す前に**読む。掴んだ行は 2 番目に見えているので "2"
    await expect(page.getByTestId('routine-handle')).toHaveText(['2', '1', '3']);
    // 掴んだ行だけが持ち上がっていて、押しのけられた行には印が付かない
    await expect(page.locator('[data-testid=routine-picked-row][data-drag="lift"]')).toHaveCount(1);
  });

  await expect(page.getByTestId('routine-handle')).toHaveText(['1', '2', '3']);
  await expect(page.getByTestId('routine-picked-name')).toHaveText([
    'スクワット',
    'ベンチプレス',
    'プランク',
  ]);
});

test('ハンドルに触っただけでは並びが変わらない', async ({ page }) => {
  // ★ 閾値ではなく「落ちた先が元の位置なら signal に触らない」という分岐が保証する
  await createRoutine(page, '胸と脚の日', ['ベンチプレス', 'スクワット']);
  await page.getByTestId('routine-open').click();

  await dragPicked(page, 0, 0);
  await expect(page.getByTestId('routine-picked-name')).toHaveText(['ベンチプレス', 'スクワット']);
  // 指ブレ（数 px）でも動かない
  await dragPicked(page, 0, 3);
  await expect(page.getByTestId('routine-picked-name')).toHaveText(['ベンチプレス', 'スクワット']);
});

test('ドラッグの代わりに Alt + ↑↓ でも並べ替えられる', async ({ page }) => {
  // ★ 掴む場所（番号）は <button> にできないので、既にフォーカスできる行の ✕ に載せる。
  //   WCAG 2.1.1 の非ドラッグ経路であり、記録タブのカード / セット行と同じ作り
  await createRoutine(page, '胸と脚の日', ['ベンチプレス', 'スクワット']);
  await page.getByTestId('routine-open').click();

  await page.getByTestId('routine-remove').nth(1).focus();
  await page.keyboard.press('Alt+ArrowUp');
  await expect(page.getByTestId('routine-picked-name')).toHaveText(['スクワット', 'ベンチプレス']);

  // 端では何も起きない
  await page.getByTestId('routine-remove').nth(0).focus();
  await page.keyboard.press('Alt+ArrowUp');
  await expect(page.getByTestId('routine-picked-name')).toHaveText(['スクワット', 'ベンチプレス']);
});

test('★ シートの下端まで運ぶと「選択中」がシートの中で自動スクロールする', async ({ page }) => {
  // ★ **記録タブの実装をそのまま持ってこられなかった唯一の理由がここ。**
  //   この並びは `.sheet-body`（overflow-y: auto）という入れ子のスクロール容器の中に
  //   あるので、`window` を動かしても 1px も進まないし、帯を innerHeight で決めると
  //   上端の帯がシートの外に出る（views/drag.rs の Scroller）。
  await createRoutine(page, '長い日', [
    'ベンチプレス',
    'ダンベルプレス',
    'インクラインベンチプレス',
    'チェストフライ',
    'プッシュアップ',
    '懸垂',
    'ラットプルダウン',
    'デッドリフト',
  ]);
  await page.getByTestId('routine-open').click();
  await expect(page.getByTestId('routine-picked-row')).toHaveCount(8);

  const body = page.locator('#settings-sheet-body');
  await expect(body).toBeVisible();
  expect(
    await body.evaluate((el) => el.scrollHeight > el.clientHeight),
    'シートの中がスクロールする状態になっていない（この検証が成立しない）',
  ).toBe(true);
  // ★ 開き直したシートは必ず先頭から出る。`<dialog>` は常時マウントなので
  //   `.sheet-body` は scrollTop を覚えたままになり、これが無いと**見出しも名前欄も
  //   画面外**の状態で開く（実測: iPhone 15 Pro で 446）。views/mod.rs の `Sheet`
  expect(await body.evaluate((el) => el.scrollTop)).toBe(0);

  // 先頭を掴んで、シート下端の帯（内側 72px）に指を置いたまま待つ。
  // ★ 測る前に scrollIntoViewIfNeeded を通す。シートは 0.22s かけてせり上がるので、
  //   その前に boundingBox を読むと**別の場所を掴む**（actionability の stable 待ちが要る）
  const handle = page.getByTestId('routine-picked-row').first().getByTestId('routine-handle');
  await handle.scrollIntoViewIfNeeded();
  const box = await body.boundingBox();
  const hb = await handle.boundingBox();
  await page.mouse.move(hb.x + hb.width / 2, hb.y + hb.height / 2);
  await page.mouse.down();
  await expect(page.getByTestId('routine-picked-row').first()).toHaveAttribute('data-drag', 'lift');
  await page.mouse.move(hb.x + hb.width / 2, box.y + box.height - 12, { steps: 8 });

  // ★ 指は 1px も動かないままスクロールが進む（rAF ループが回っている証拠）
  await expect
    .poll(() => body.evaluate((el) => el.scrollTop), { timeout: 5000 })
    .toBeGreaterThan(50);
  await page.mouse.up();

  // 先頭がだいぶ後ろへ落ちている（何番目かは待ち時間で変わるので順位だけ見る）
  const names = await page.getByTestId('routine-picked-name').allTextContents();
  expect(names.indexOf('ベンチプレス')).toBeGreaterThan(1);
  expect(names).toHaveLength(8);
});

test('種目ピッカーは既定で全部閉じており、部位は複数同時に開ける', async ({ page }) => {
  // ★ 設定タブの「1 つだけ開く」（adr/ux/menu-groups-as-single-open-accordion.md）とは
  //   **別規則**。メニューを 1 本組む間は胸と脚を行き来するので、排他だと往復のたびに
  //   開き直すことになる
  await openSettingsSection(page, 'routines');
  await page.getByTestId('settings-add-routine').click();
  await expect(page.getByTestId('settings-sheet')).toBeVisible();

  const chest = pickGroup(page, '胸');
  const legs = pickGroup(page, '脚');

  // 既定で全部閉じている（開いている部位にしか種目ボタンは無い）
  await expect(page.getByTestId('routine-group-toggle')).toHaveCount(6);
  await expect(page.getByTestId('routine-pick')).toHaveCount(0);
  await expect(chest).toHaveAttribute('aria-expanded', 'false');

  await chest.click();
  await expect(chest).toHaveAttribute('aria-expanded', 'true');
  await expect(
    page.getByTestId('routine-pick').filter({ hasText: exactText('ベンチプレス') }),
  ).toHaveCount(1);
  // 開いていない部位の種目はまだ出ていない
  await expect(
    page.getByTestId('routine-pick').filter({ hasText: exactText('スクワット') }),
  ).toHaveCount(0);

  // ★ 2 つ目を開いても 1 つ目は閉じない
  await legs.click();
  await expect(chest).toHaveAttribute('aria-expanded', 'true');
  await expect(legs).toHaveAttribute('aria-expanded', 'true');

  // 開いたまま両方から選べる
  await page.getByTestId('routine-pick').filter({ hasText: exactText('ベンチプレス') }).click();
  await page.getByTestId('routine-pick').filter({ hasText: exactText('スクワット') }).click();
  await expect(page.getByTestId('routine-picked-name')).toHaveText(['ベンチプレス', 'スクワット']);

  // もう一度押すと閉じる（押した部位だけ）
  await chest.click();
  await expect(chest).toHaveAttribute('aria-expanded', 'false');
  await expect(legs).toHaveAttribute('aria-expanded', 'true');
  await expect(
    page.getByTestId('routine-pick').filter({ hasText: exactText('ベンチプレス') }),
  ).toHaveCount(0);
  // 閉じても選択は落ちない（「選択中」が真実源）
  await expect(page.getByTestId('routine-picked-name')).toHaveText(['ベンチプレス', 'スクワット']);
});

test('シートで開いた部位は、設定タブの種目一覧にも記録タブにも漏れない', async ({ page }) => {
  // ★ `OpenGroupCtx`（アプリ全体で 1 本）を使わず RoutineEditor ローカルに持つ理由。
  //   共有すると「メニューを組むために開いた胸」が種目一覧でも開きっぱなしになる
  await openSettingsSection(page, 'routines');
  await page.getByTestId('settings-add-routine').click();
  await pickGroup(page, '胸').click();
  await expect(pickGroup(page, '胸')).toHaveAttribute('aria-expanded', 'true');
  await page.getByTestId('settings-sheet-close').click();
  await expect(page.getByTestId('settings-sheet')).toBeHidden();

  await openSettingsSection(page, 'exercises');
  const toggles = page.getByTestId('group-toggle');
  await expect(toggles).toHaveCount(6);
  for (let i = 0; i < 6; i++) {
    await expect(toggles.nth(i)).toHaveAttribute('aria-expanded', 'false');
  }
});

test('メニューに履歴の無い種目が入っていても、空のカードとして出る', async ({ page }) => {
  // ★ 黙って飛ばすと「3 種目入れたのに 2 枚しか出ない」理由が画面から読めない。
  //   0 セットのログを書くと migrate が次回起動で落として「出ているのに消える」
  await seedPastLogs(page, [
    { daysAgo: 3, exerciseName: 'ベンチプレス', sets: [{ weight: 60, reps: 10 }] },
  ]);
  await createRoutine(page, '胸の日', ['ベンチプレス', 'チェストフライ']);

  await page.getByTestId('tab-record').click();
  await page.getByTestId('routine-candidate').first().click();

  const cards = page.getByTestId('exercise-card');
  await expect(cards).toHaveCount(2);
  await expect(cards.nth(1)).toContainText('チェストフライ');
  await expect(cards.nth(1)).toContainText('前回 —');
  // 空の 1 行が出るだけで、値は入らない
  await expect(cards.nth(1).getByTestId('set-row').nth(0).getByTestId('set-reps')).toHaveValue('');
});

test('メニューが 1 本も無いときの候補リストは今までと同じ見た目', async ({ page }) => {
  // ★ メニューを使わない利用者の画面を変えない。見出しを 2 つに割るのは
  //   出所が実際に 2 つあるときだけ
  await seedPastLogs(page, [
    { daysAgo: 3, exerciseName: 'ベンチプレス', sets: [{ weight: 60, reps: 10 }] },
  ]);

  const list = page.getByTestId('menu-copy');
  await expect(list).toBeVisible();
  await expect(list).toContainText('前回のメニューから始める');
  await expect(list).not.toContainText('トレーニングメニュー');
  await expect(page.getByTestId('routine-candidate')).toHaveCount(0);
});

test('メニューがあると候補は「トレーニングメニュー」と「最近の記録から」に分かれる', async ({ page }) => {
  await seedPastLogs(page, [
    { daysAgo: 3, exerciseName: 'ベンチプレス', sets: [{ weight: 60, reps: 10 }] },
  ]);
  await createRoutine(page, '胸の日', ['ベンチプレス']);

  await page.getByTestId('tab-record').click();
  const list = page.getByTestId('menu-copy');
  await expect(list).toContainText('トレーニングメニュー');
  await expect(list).toContainText('最近の記録から');
  await expect(list).not.toContainText('前回のメニューから始める');
  // メニューが先頭。curated なほうを先に読ませる
  await expect(page.getByTestId('routine-candidate')).toHaveCount(1);
  await expect(page.getByTestId('menu-candidate')).toHaveCount(1);
});

test('未来日にはメニューの候補も出さない', async ({ page }) => {
  // ★ ガードが日候補にしか効いていないと、まだやっていないトレーニングが
  //   「実施済み」としてカレンダーのドット・グラフに乗る
  await seedPastLogs(page, [
    { daysAgo: 3, exerciseName: 'ベンチプレス', sets: [{ weight: 60, reps: 10 }] },
  ]);
  await createRoutine(page, '胸の日', ['ベンチプレス']);

  await page.getByTestId('tab-record').click();
  await expect(page.getByTestId('routine-candidate')).toHaveCount(1);

  // 翌月へ送って未来の日を選ぶ（月内の未来日は今日の位置に依存するため）
  await page.getByTestId('cal-next').click();
  await page.getByTestId('cal-day').filter({ hasText: exactText('15') }).click();
  await expect(page.getByTestId('past-banner')).toBeVisible();
  await expect(page.getByTestId('menu-copy')).toHaveCount(0);
});

test('メニューは編集・削除でき、削除しても記録は消えない', async ({ page }) => {
  await seedPastLogs(page, [
    { daysAgo: 3, exerciseName: 'ベンチプレス', sets: [{ weight: 60, reps: 10 }] },
  ]);
  await createRoutine(page, '胸の日', ['ベンチプレス']);

  // 行全体が編集の入口（開閉と編集を分けない）
  await page.getByTestId('routine-open').click();
  await expect(page.getByTestId('settings-sheet')).toBeVisible();
  await page.getByTestId('routine-name-input').fill('プッシュの日');
  await openAllPickGroups(page);
  await page.getByTestId('routine-pick').filter({ hasText: exactText('チェストフライ') }).click();
  await page.getByTestId('routine-save').click();
  await expect(page.getByTestId('routine-name')).toHaveText('プッシュの日');
  await expect(page.getByTestId('routine-count')).toHaveText('2 種目');

  // 削除は確認を挟む（組んだ並びは元に戻せない）
  await page.getByTestId('routine-open').click();
  // ★ trash のアイコンが 1 つ描かれている。テキストリンクだけでは、シート最下部の
  //   この 1 行が「どこで消せるのか」として読み取れない
  //   （XML 宣言が混ざると svg が 0 個になる罠も同時に塞ぐ。
  //   adr/architecture/lucide-icons-as-included-svg.md）
  await expect(page.locator('[data-testid=delete-routine] .icon > svg')).toHaveCount(1);
  await page.getByTestId('delete-routine').click();
  await page.getByTestId('delete-routine-confirm').click();
  await expect(page.getByTestId('settings-sheet')).toBeHidden();
  await expect(page.getByTestId('routine-item')).toHaveCount(0);

  // ★ 記録は 1 件も消えない
  await page.getByTestId('tab-record').click();
  await expect(page.getByTestId('menu-candidate')).toHaveCount(1);
});

test('種目が 0 個のメニューも、名前が空のメニューも保存できない', async ({ page }) => {
  await openSettingsSection(page, 'routines');
  await page.getByTestId('settings-add-routine').click();

  // 名前だけ。記録タブに出せないので、作れても「押せない行」が残るだけになる
  await page.getByTestId('routine-name-input').fill('からっぽ');
  await page.getByTestId('routine-save').click();
  await expect(page.getByTestId('routine-invalid')).toHaveText('種目を 1 つ以上選んでください');

  // ★ 種目だけ（名前なし）も止める。無名を許すと「（名前なし）」の行が複数並び、
  //   誤タップ対策の柱にしている「行を見て選び分けられること」が痩せる
  await page.getByTestId('routine-name-input').fill('   ');
  await openAllPickGroups(page);
  await page.getByTestId('routine-pick').filter({ hasText: exactText('ベンチプレス') }).click();
  await page.getByTestId('routine-save').click();
  await expect(page.getByTestId('routine-invalid')).toHaveText('メニュー名を入れてください');

  await expect(page.getByTestId('settings-sheet')).toBeVisible();
  await expect(page.getByTestId('routine-item')).toHaveCount(0);
});

test('一部の種目だけアーカイブしたメニューは、開く数を出して差分の理由も出す', async ({ page }) => {
  // ★ 「2 種目」と書いてあるのに 1 枚しか開かない、が起きないこと。
  //   件数は core::expandable_count（＝記録タブが実際に開く数）を通す
  await seedPastLogs(page, [
    { daysAgo: 3, exerciseName: 'ベンチプレス', sets: [{ weight: 60, reps: 10 }] },
  ]);
  await createRoutine(page, '胸の日', ['ベンチプレス', 'チェストフライ']);
  await expect(page.getByTestId('routine-count')).toHaveText('2 種目');

  await openGroup(page, '胸');
  await page.getByTestId('exercise-name').filter({ hasText: exactText('チェストフライ') }).click();
  await page.getByTestId('archive-exercise').click();
  await expect(page.getByTestId('settings-sheet')).toBeHidden();

  await openSettingsSection(page, 'routines');

  // 件数は開く数に減り、名前は残したまま（アーカイブは可逆なので隠さない）、
  // 食い違いの理由を出す
  await expect(page.getByTestId('routine-count')).toHaveText('1 種目');
  await expect(page.getByTestId('routine-names')).toContainText('チェストフライ');
  await expect(page.getByTestId('routine-partial')).toHaveText(
    'アーカイブ済みの 1 種目は記録タブに出ません',
  );

  // 記録タブで実際に開くのも 1 枚
  await page.getByTestId('tab-record').click();
  await page.getByTestId('routine-candidate').first().click();
  await expect(page.getByTestId('exercise-card')).toHaveCount(1);
});

test('全種目をアーカイブしたメニューは記録タブに出ず、設定タブに理由が出る', async ({ page }) => {
  await seedPastLogs(page, [
    { daysAgo: 3, exerciseName: 'スクワット', sets: [{ weight: 80, reps: 5 }] },
  ]);
  await createRoutine(page, '脚の日', ['スクワット']);

  await openGroup(page, '脚');
  await page.getByTestId('exercise-name').filter({ hasText: exactText('スクワット') }).click();
  await page.getByTestId('archive-exercise').click();
  await expect(page.getByTestId('settings-sheet')).toBeHidden();

  await openSettingsSection(page, 'routines');

  // ★ 出ない理由が画面から読めること。無いと「作ったのに使えない」原因を探せない
  await expect(page.getByTestId('routine-unusable')).toBeVisible();

  await page.getByTestId('tab-record').click();
  await expect(page.getByTestId('routine-candidate')).toHaveCount(0);
});

test('メニューは書き出して読み戻しても残る', async ({ page }) => {
  await createRoutine(page, '胸の日', ['ベンチプレス', 'チェストフライ']);
  // flushToStorage は screen-record を待つので、記録タブへ戻してから呼ぶ
  await page.getByTestId('tab-record').click();
  await flushToStorage(page);

  const raw = await page.evaluate(() => localStorage.getItem('fitness-memo/v3'));
  const db = JSON.parse(raw);
  expect(db.routines).toHaveLength(1);
  expect(db.routines[0].name).toBe('胸の日');
  expect(db.routines[0].exercises).toHaveLength(2);

  // 書き出し形式 = 保存形式なので、この JSON がそのまま読み戻せる
  await page.reload();
  await expect(page.getByTestId('tab-settings')).toBeVisible();
  await openSettingsSection(page, 'routines');
  await expect(page.getByTestId('routine-name')).toHaveText('胸の日');
});

test('メニューを 1 本も作っていない保存データは今までとバイト単位で同じ', async ({ page }) => {
  // ★ routines を常に書くと、メニューを使わない利用者の JSON が変わる
  await seedPastLogs(page, [
    { daysAgo: 3, exerciseName: 'ベンチプレス', sets: [{ weight: 60, reps: 10 }] },
  ]);
  await flushToStorage(page);

  const raw = await page.evaluate(() => localStorage.getItem('fitness-memo/v3'));
  expect(raw).not.toContain('routines');
});

test('履歴ゼロでメニューを押したあとタブを往復すると、カードは消えるが候補は戻る', async ({ page }) => {
  // ★ 初回起動の利用者では**全種目**が履歴なしになるので、
  //   adr/ux/copy-whole-day-menu.md が却下した「タブ切替で消える」状況がそのまま起きる。
  //   受け入れているのは、失うのがタップ 1 回だけで、記録が 1 バイトも消えないから。
  //   その回復経路が本当に生きていることをここで固定する
  await createRoutine(page, '胸の日', ['ベンチプレス', 'チェストフライ']);

  await page.getByTestId('tab-record').click();
  await page.getByTestId('routine-candidate').first().click();
  await expect(page.getByTestId('exercise-card')).toHaveCount(2);
  // カードが出ている間は候補を出さない
  await expect(page.getByTestId('menu-copy')).toHaveCount(0);

  // 1 セットも打たずにタブを往復する
  await blurActive(page);
  await page.getByTestId('tab-progress').click();
  await page.getByTestId('tab-record').click();

  // カードは消える（空のログは保存しない仕様）が、記録は 1 件も生まれていない
  await expect(page.getByTestId('exercise-card')).toHaveCount(0);
  await flushToStorage(page);
  const raw = await page.evaluate(() => localStorage.getItem('fitness-memo/v3'));
  expect(JSON.parse(raw).sessions).toEqual({});

  // ★ 候補が戻るので、もう 1 タップで同じ状態に復帰できる
  await expect(page.getByTestId('routine-candidate')).toHaveCount(1);
  await page.getByTestId('routine-candidate').first().click();
  await expect(page.getByTestId('exercise-card')).toHaveCount(2);

  // 1 文字打てば以後は永続化される（実際の使い方はこちら）
  await page.getByTestId('exercise-card').first().getByTestId('set-reps').first().fill('10');
  await blurActive(page);
  await flushToStorage(page);
  await page.reload();
  await expect(page.getByTestId('exercise-card')).toHaveCount(1);
});

test('★ その日の記録から 1 タップでメニューを作れる', async ({ page }) => {
  // adr/ux/save-a-day-as-a-routine.md
  await seedPastLogs(page, [
    { daysAgo: 0, exerciseName: 'ベンチプレス', sets: [{ weight: 60, reps: 10 }] },
    { daysAgo: 0, exerciseName: 'スクワット', sets: [{ weight: 80, reps: 5 }] },
  ]);

  await page.getByTestId('day-to-routine').click();
  const sheet = page.getByTestId('day-routine-sheet');
  await expect(sheet).toBeVisible();

  // その日の種目が**その日のログ順で**初期選択になっている
  const picked = sheet.getByTestId('routine-picked').locator('li');
  await expect(picked).toHaveCount(2);
  await expect(picked.nth(0)).toContainText('ベンチプレス');
  await expect(picked.nth(1)).toContainText('スクワット');

  await page.getByTestId('routine-name-input').fill('全身の日');
  await page.getByTestId('routine-save').click();
  await expect(sheet).toBeHidden();

  // ★ 記録は 1 件も動かない（メニューを作るのは記録の操作ではない）
  await expect(page.getByTestId('exercise-card')).toHaveCount(2);
  await flushToStorage(page);
  const raw = await page.evaluate(() => localStorage.getItem('fitness-memo/v3'));
  const db = JSON.parse(raw);
  expect(db.routines).toHaveLength(1);
  expect(db.routines[0].name).toBe('全身の日');
  expect(db.routines[0].exercises).toHaveLength(2);

  // 設定タブにも、次の日の候補にも出る
  await openSettingsSection(page, 'routines');
  await expect(page.getByTestId('routine-name')).toHaveText('全身の日');
});

test('シートの中で種目を足し引きしてから保存できる', async ({ page }) => {
  // その日やった bonus 種目を外す／その日やらなかった種目を足す、が同じ画面でできる
  await seedPastLogs(page, [
    { daysAgo: 0, exerciseName: 'ベンチプレス', sets: [{ weight: 60, reps: 10 }] },
    { daysAgo: 0, exerciseName: 'プランク', sets: [{ weight: 0, reps: 60 }] },
  ]);

  await page.getByTestId('day-to-routine').click();
  const sheet = page.getByTestId('day-routine-sheet');
  // プランクを外して、チェストフライを足す
  await sheet.getByTestId('routine-remove').nth(1).click();
  await openAllPickGroups(sheet);
  await sheet.getByTestId('routine-pick').filter({ hasText: exactText('チェストフライ') }).click();
  await page.getByTestId('routine-name-input').fill('胸の日');
  await page.getByTestId('routine-save').click();
  await expect(sheet).toBeHidden();

  await openSettingsSection(page, 'routines');
  await expect(page.getByTestId('routine-names')).toHaveText('ベンチプレス, チェストフライ');
});

test('記録が無い日には「この日をメニューにする」を出さない', async ({ page }) => {
  // ★ 判定は cards ではなく core::day_exercises。「種目を追加」で出しただけで
  //   まだ何も打っていないカードで出すと、何も保存されないリンクになる
  await expect(page.getByTestId('day-to-routine')).toHaveCount(0);

  // 種目を追加しただけ（セットは空）ではまだ出ない
  await addExercise(page, 'ベンチプレス');
  await expect(page.getByTestId('exercise-card')).toHaveCount(1);
  await expect(page.getByTestId('day-to-routine')).toHaveCount(0);

  // 1 セット打つと出る
  const card = page.getByTestId('exercise-card').first();
  await card.getByTestId('set-weight').first().fill('60');
  await card.getByTestId('set-reps').first().fill('10');
  await blurActive(page);
  await expect(page.getByTestId('day-to-routine')).toBeVisible();
});

test('アーカイブ済み種目の記録はメニューに入れない', async ({ page }) => {
  // 「前回のメニューから始める」の候補と同じ copyable を通す
  await seedPastLogs(page, [
    { daysAgo: 0, exerciseName: 'ベンチプレス', sets: [{ weight: 60, reps: 10 }] },
    { daysAgo: 0, exerciseName: 'スクワット', sets: [{ weight: 80, reps: 5 }] },
  ]);

  await blurActive(page);
  await page.getByTestId('tab-settings').click();
  await openGroup(page, '脚');
  await page.getByTestId('exercise-name').filter({ hasText: exactText('スクワット') }).click();
  await page.getByTestId('archive-exercise').click();
  await expect(page.getByTestId('settings-sheet')).toBeHidden();

  await page.getByTestId('tab-record').click();
  await page.getByTestId('day-to-routine').click();
  const picked = page.getByTestId('day-routine-sheet').getByTestId('routine-picked').locator('li');
  await expect(picked).toHaveCount(1);
  await expect(picked.nth(0)).toContainText('ベンチプレス');
});

test('「この日をメニューにする」は sticky な帯に覆われず force なしで押せる', async ({ page }) => {
  // ★ .add-wrap は sticky で包含ブロックが .day。その中で帯より後ろに置くと、
  //   .day の末尾までスクロールしきるまで帯の下に潜りうる。だから .day の外に出して
  //   ある（InstallBanner と同じ回避、adr/ux/save-a-day-as-a-routine.md）。
  //   この位置関係を固定するテスト
  await seedPastLogs(page, [
    { daysAgo: 0, exerciseName: 'ベンチプレス', sets: [{ weight: 60, reps: 10 }] },
  ]);
  // カードを増やして、リンクが確実に折り返しの下へ行く状況を作る
  for (const name of ['ダンベルプレス', 'チェストフライ', 'プッシュアップ']) {
    await addExercise(page, name);
  }
  await blurActive(page);

  const link = page.getByTestId('day-to-routine');
  await link.scrollIntoViewIfNeeded();
  const box = await link.boundingBox();
  const add = await page.locator('.add-wrap').boundingBox();
  expect(box, 'リンクが描画されていない').not.toBeNull();
  // 「種目を追加」の下端よりリンクの上端が下にある（= 重なっていない）
  expect(box.y).toBeGreaterThanOrEqual(add.y + add.height);

  // force なしで押せる（actionability チェックを通る）
  await link.click();
  await expect(page.getByTestId('day-routine-sheet')).toBeVisible();
});

// ── 設定タブの節一覧（adr/ux/settings-as-a-list-of-sections.md）──────────────

test('★ 設定タブの入口は節の一覧で、中身は入るまで出ない', async ({ page }) => {
  await blurActive(page);
  await page.getByTestId('tab-settings').click();

  // トップは 4 行だけ。種目もメニューも 1 件も出ていない
  // （`.row` で数える。手順シートの <dialog> も同じ親に出るので `> *` だと 5 になる）
  await expect(page.getByTestId('settings-rows').locator('.row')).toHaveCount(4);
  await expect(page.getByTestId('group-item')).toHaveCount(0);
  await expect(page.getByTestId('routine-item')).toHaveCount(0);
  await expect(page.getByTestId('settings-add-group')).toHaveCount(0);
  await expect(page.getByTestId('settings-add-routine')).toHaveCount(0);
  // 入る前に件数が読める（入って初めて空だと分かる、を避ける）
  await expect(page.getByTestId('settings-row-exercises')).toContainText('28');
  await expect(page.getByTestId('settings-row-routines')).toContainText('0');

  // 節へ入ると h1 がその節名になる（h1 は常に 1 つ）
  await page.getByTestId('settings-row-exercises').click();
  await expect(page.locator('main h1')).toHaveCount(1);
  await expect(page.locator('main h1')).toHaveText('種目');
  await expect(page.getByTestId('group-item')).toHaveCount(6);
  await expect(page.getByTestId('settings-rows')).toHaveCount(0);

  // 「‹」でトップへ戻る
  await page.getByTestId('settings-back').click();
  await expect(page.locator('main h1')).toHaveText('設定');
  await expect(page.getByTestId('group-item')).toHaveCount(0);
});

test('開いていた節はタブを往復しても戻らない', async ({ page }) => {
  // ★ OpenGroupCtx と同じ理由。往復のたびにトップへ戻されると、入り直す手数が毎回要る
  await openSettingsSection(page, 'exercises');
  await expect(page.getByTestId('group-item')).toHaveCount(6);

  await blurActive(page);
  await page.getByTestId('tab-record').click();
  await page.getByTestId('tab-settings').click();

  await expect(page.locator('main h1')).toHaveText('種目');
  await expect(page.getByTestId('group-item')).toHaveCount(6);
});
