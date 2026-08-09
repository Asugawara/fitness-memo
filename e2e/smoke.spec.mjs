import { test, expect } from '@playwright/test';

// 記録タブ（カレンダー + 選択日エディタ）と推移・種目タブの E2E。
// src/views/{day,calendar,mod,progress,chart,menu}.rs の data-testid を使う。
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

test('11. 種目タブでの改名・部位変更・新規追加が記録タブに反映され、アーカイブは推移タブから参照できる', async ({ page }) => {
  await page.getByTestId('tab-menu').click();
  await expect(page.getByTestId('screen-menu')).toBeVisible();

  // 改名 + 部位変更（肩→腕）を1つの種目に対して行う
  await page.getByTestId('exercise-name').filter({ hasText: exactText('サイドレイズ') }).click();
  const menuSheet = page.getByTestId('menu-sheet');
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

  await page.getByTestId('menu-sheet-close').click();

  // 新規部位 + 新規種目を追加する
  await page.getByTestId('menu-add-group').click();
  await page.getByTestId('new-group-name').fill('テスト部位');
  await page.getByTestId('new-group-submit').click();

  // group-item はカード全体（部位名 + 種目数 + 並び替えボタン）のテキストを含むので、
  // ここは exactText ではなく部分一致でよい（"テスト部位" は他と衝突しない固有名）
  const testGroupItem = page.getByTestId('group-item').filter({ hasText: 'テスト部位' });
  await testGroupItem.getByTestId('menu-add-exercise').click();
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
  await page.getByTestId('tab-menu').click();
  await page.getByTestId('exercise-name').filter({ hasText: exactText('テスト種目') }).click();
  await page.getByTestId('archive-exercise').click();
  // ★ <dialog> は常時マウントなので消えない。「閉じている」は toBeHidden で見る
  //   （閉じた dialog は UA の display:none が効く）
  await expect(page.getByTestId('menu-sheet')).toBeHidden();

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
  await page.getByTestId('tab-menu').click();
  await page.getByTestId('group-name').filter({ hasText: exactText('テスト部位') }).click();
  await page.getByTestId('delete-group').click();
  await expect(page.getByTestId('delete-blocked')).toContainText('種目が 1 件あるため削除できません');
  await expect(page.getByTestId('delete-blocked-archived')).toContainText('アーカイブ済み種目が 1 件あります');
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

// ★ 中身の有無で分岐しない（[ADR-0046]）。消えるのは 1 行で打ち直しは数秒、対して
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

// 以下2件は計画の12ケースには無い追加の退行テスト。worker-d が実機相当の検証で見つけた
// バグ（.bottom-tabs / backdrop / .sheet が全て position:fixed なのに z-index を
// 省いていたため、DOM順で <nav class="bottom-tabs"> が前面に出ていた）の固定用。
// 目視でしか気づけない類の退行なので、force を付けないクリックで機械的に検出する。
//
// ★ ADR-0050 でシートを <dialog> + show_modal() に移したので、2 件とも今は UA が
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

// ネイティブ <dialog> に移して初めて成立した挙動（ADR-0050）。手書きの
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

// ── 見出しの階層とフォーカスリング（ADR-0052）────────────────────────────────

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
  await page.getByTestId('tab-menu').click();
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

// ── タブ切替の View Transition（ADR-0051）────────────────────────────────────

// `document.startViewTransition` を包んで、渡された types を記録する。
// 向きの決定は今回入れたロジックそのものなので、そこだけを直接見る。
async function recordViewTransitions(page) {
  await page.addInitScript(() => {
    window.__vt = [];
    const orig = document.startViewTransition?.bind(document);
    if (!orig) return;
    document.startViewTransition = (opts) => {
      window.__vt.push(opts?.types?.[0] ?? null);
      return orig(opts);
    };
  });
  await page.reload();
  // 実装と同じ判定。types 形が無いブラウザでは遷移そのものを走らせない
  return page.evaluate(() =>
    CSS.supports('selector(:active-view-transition-type(forward))'),
  );
}

test('タブ切替は並び順どおりの向きで遷移する', async ({ page }) => {
  const supported = await recordViewTransitions(page);
  test.skip(!supported, 'このブラウザは View Transition の types 形に未対応');

  // 記録(0) → 種目(2) は前進
  await page.getByTestId('tab-menu').click();
  await expect(page.getByTestId('screen-menu')).toBeVisible();
  // 種目(2) → 推移(1) は後退
  await page.getByTestId('tab-progress').click();
  await expect(page.getByTestId('screen-progress')).toBeVisible();

  expect(await page.evaluate(() => window.__vt)).toEqual(['forward', 'backward']);
});

test('同じタブをもう一度押しても遷移は走らない', async ({ page }) => {
  const supported = await recordViewTransitions(page);
  test.skip(!supported, 'このブラウザは View Transition の types 形に未対応');

  await page.getByTestId('tab-menu').click();
  await expect(page.getByTestId('screen-menu')).toBeVisible();
  await page.getByTestId('tab-menu').click();

  // 素で set すると同値でも購読者へ通知が飛び、押すたびに画面が丸ごと動く
  expect(await page.evaluate(() => window.__vt)).toEqual(['forward']);
});

test('タブバーと通知は root のスナップショットから外れている', async ({ page }) => {
  // 付け忘れると画面全体と一緒にタブバーまで横へ流れる
  await expect(page.getByTestId('bottom-tabs')).toHaveCSS('view-transition-name', 'bottom-tabs');
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

  // 1 種目でも入ったら候補は消える（ADR-0021 と同じ「空のときだけ」の考え方）
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
  await page.getByTestId('tab-menu').click();
  await page.getByTestId('exercise-name').filter({ hasText: exactText('スクワット') }).click();
  await page.getByTestId('archive-exercise').click();
  await expect(page.getByTestId('menu-sheet')).toBeHidden();

  await page.getByTestId('tab-record').click();
  await expect(page.getByTestId('menu-copy')).toHaveCount(0);
});
