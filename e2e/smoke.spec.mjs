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
 */
async function seedPastLogs(page, entries) {
  await flushToStorage(page);
  await page.evaluate((entries) => {
    const KEY = 'fitness-memo/v2';
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

    for (const { daysAgo, exerciseName, sets, at = null } of entries) {
      const ex = db.exercises.find((e) => e.name === exerciseName);
      if (!ex) throw new Error(`preset exercise not found: ${exerciseName}`);
      const key = dateKey(daysAgo);
      const session = db.sessions[key] ?? { logs: [], body_weight: null, note: '' };
      session.logs.push({ exercise_id: ex.id, sets, at });
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
  // at: null（バックフィル済み）で注入する。これが at: Some(now) だと
  // 「たった今」になってしまい、要件「最後のトレーニングから」の出力が嘘になる
  await seedPastLogs(page, [
    { daysAgo: 1, exerciseName: 'ベンチプレス', sets: [{ weight: 60, reps: 10 }] },
  ]);

  await expect(page.getByTestId('elapsed')).toHaveText('昨日');
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
  // 描画されていることまで確認する（属性名だけでなく描画結果を見る）
  await expect(chart.locator('polyline')).toHaveCount(1);
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
  await expect(page.getByTestId('menu-sheet')).toHaveCount(0);

  await page.getByTestId('tab-record').click();
  await page.getByTestId('add-exercise').click();
  await expect(
    page.getByTestId('add-sheet').getByTestId('pick-exercise').filter({ hasText: exactText('テスト種目') }),
  ).toHaveCount(0);
  // sheet-backdrop はビューポート全体を覆うが、クリック位置の中心はシート本体の
  // 裏に隠れて弾かれる。「閉じる」ボタンには testid が無いので role+text で取る
  await page.getByRole('button', { name: '閉じる' }).click();

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

test('中身のあるセットの削除は確認を挟み、「やめる」で残る', async ({ page }) => {
  const card = await addExercise(page, 'ベンチプレス');
  const row0 = card.getByTestId('set-row').nth(0);
  await row0.getByTestId('set-weight').fill('60');
  await row0.getByTestId('set-reps').fill('10');

  await row0.getByTestId('remove-set').click();
  await expect(page.getByTestId('remove-set-confirm')).toBeVisible();
  // 確認中は行がまだ生きている
  await expect(card.getByTestId('today-metric')).toHaveText('600');

  await page.getByTestId('remove-set-no').click();
  await expect(page.getByTestId('remove-set-confirm')).toHaveCount(0);
  await expect(row0.getByTestId('set-weight')).toHaveValue('60');

  await row0.getByTestId('remove-set').click();
  await page.getByTestId('remove-set-yes').click();
  await expect(card.getByTestId('today-metric')).toHaveText('0');
});

test('空のセット行は確認なしで消える（消えるものが無いため）', async ({ page }) => {
  const card = await addExercise(page, 'ベンチプレス');
  await card.getByTestId('set-reps').first().fill('10');
  await card.getByTestId('add-set').click();
  await expect(card.getByTestId('set-row')).toHaveCount(2);

  // 2 行目は重量プリフィルのみで回数が空 = 中身なし扱い
  await card.getByTestId('set-row').nth(1).getByTestId('remove-set').click();
  await expect(page.getByTestId('remove-set-confirm')).toHaveCount(0);
  await expect(card.getByTestId('set-row')).toHaveCount(1);
});

test('「この種目を外す」はカード末尾にあり、確認を経由する', async ({ page }) => {
  const card = await addExercise(page, 'ベンチプレス');
  await card.getByTestId('set-reps').first().fill('10');
  await blurActive(page);

  // ★ 見出しの右端に削除ボタンを置かない（追加しようとして消す事故の元だった）
  await expect(card.locator('.card-head button')).toHaveCount(0);

  await card.getByTestId('close-card').click();
  await expect(page.getByTestId('close-card-warning')).toContainText('記録が消えます');
  await expect(page.getByTestId('exercise-card')).toHaveCount(1);

  await page.getByTestId('close-card-no').click();
  await expect(page.getByTestId('exercise-card')).toHaveCount(1);

  await card.getByTestId('close-card').click();
  await page.getByTestId('close-card-yes').click();
  await expect(page.getByTestId('exercise-card')).toHaveCount(0);
});

// 以下2件は計画の12ケースには無い追加の退行テスト。worker-d が実機相当の検証で見つけた
// バグ（.bottom-tabs / .sheet-backdrop / .sheet が全て position:fixed なのに z-index を
// 省いていたため、DOM順で <nav class="bottom-tabs"> が前面に出ていた）の固定用。
// 目視でしか気づけない類の退行なので、force を付けないクリックで機械的に検出する。

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

test('「種目を追加」シート表示中はバックドロップがタブバーを覆い、誤タップで別タブへ遷移しない', async ({ page }) => {
  await page.getByTestId('add-exercise').click();
  await expect(page.getByTestId('add-sheet')).toBeVisible();

  // z-index が外れてバックドロップがタブバーを覆えなくなると、この click が素通りして
  // 推移タブへ遷移してしまう（隠れた種目を狙ったタップが誤タブ遷移になり入力を見失う）
  await expect(page.getByTestId('tab-progress').click({ timeout: 1000 })).rejects.toThrow();

  await expect(page.getByTestId('add-sheet')).toBeVisible();
  await expect(page.getByTestId('screen-progress')).toHaveCount(0);
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
  await seedPastLogs(page, [
    // 空セットだけの日はそもそもコピー対象が無い
    { daysAgo: 2, exerciseName: 'ベンチプレス', sets: [] },
    { daysAgo: 3, exerciseName: 'スクワット', sets: [{ weight: 80, reps: 5 }] },
  ]);
  await expect(page.getByTestId('menu-candidate')).toHaveCount(1);

  // アーカイブするとコピー対象から外れる。件数だけ数えて行を出していると、
  // 「1種目」と表示されるのに押しても何も起きない死んだボタンになる
  await blurActive(page);
  await page.getByTestId('tab-menu').click();
  await page.getByTestId('exercise-name').filter({ hasText: exactText('スクワット') }).click();
  await page.getByTestId('archive-exercise').click();
  await expect(page.getByTestId('menu-sheet')).toHaveCount(0);

  await page.getByTestId('tab-record').click();
  await expect(page.getByTestId('menu-copy')).toHaveCount(0);
});
