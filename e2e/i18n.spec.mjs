// 日本語 / 英語の切り替え。
//
// ★ **このファイルだけ locale を en-US にする。** 他の spec は
//   `playwright.config.mjs` の `locale: 'ja-JP'` で日本語に固定してある
//   （既定の en-US だと英語 UI で起動して、既存の日本語の期待値が総崩れになる）。
//   ここは逆に「何も設定していない英語圏の端末で開いたら何が出るか」を見る。
//
// ★ 期待値に i18n の表を引かない。実装と同じ表から取ると、表そのものの間違いを
//   検出できなくなる。綴りは手で書く。
import { expect, test } from '@playwright/test';

const KEY = 'fitness-memo/v3';
const UI_KEY = 'fitness-memo/ui/v1';
const TSV_MIME = 'text/tab-separated-values';

test.use({ locale: 'en-US' });

/** 初回起動を待つ（400ms debounce の保存が済むまで localStorage は空）。 */
async function boot(page) {
  await page.goto('./');
  await page.waitForFunction((k) => !!localStorage.getItem(k), KEY);
}

/** 設定 > 言語 のサブページを開く。 */
async function openLanguage(page) {
  await page.getByTestId('tab-settings').click();
  const back = page.getByTestId('settings-back');
  if (await back.isVisible()) await back.click();
  await page.getByTestId('settings-row-language').click();
  await expect(page.getByTestId('lang-select')).toBeVisible();
}

/** 完全一致で引くための正規表現（部分一致で別の種目を掴まないため）。 */
function exactText(str) {
  return new RegExp(`^${str.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}$`);
}

/** 種目の節を開き、指定した部位のアコーディオンを展開する。 */
async function openGroup(page, name) {
  await page.getByTestId('tab-settings').click();
  // ★ 開いている節はタブを往復しても保たれる（SettingsPageCtx は App が持つ）ので、
  //   節の中に居るなら先にトップへ戻す
  const back = page.getByTestId('settings-back');
  if (await back.isVisible()) await back.click();
  await page.getByTestId('settings-row-exercises').click();
  const item = page
    .getByTestId('group-item')
    .filter({ has: page.getByTestId('group-name').filter({ hasText: exactText(name) }) });
  const toggle = item.getByTestId('group-toggle');
  if ((await toggle.getAttribute('aria-expanded')) !== 'true') await toggle.click();
  return item;
}

/** 言語ボタン（endonym で引かず data-lang で引く）。 */
function langButton(page, tag) {
  return page.getByTestId('lang-btn').and(page.locator(`[data-lang="${tag}"]`));
}

test('英語圏の端末で開くと、何も設定しなくても英語で起動する', async ({ page }) => {
  await boot(page);

  await expect(page.getByTestId('tab-record')).toHaveText('Record');
  await expect(page.getByTestId('tab-progress')).toHaveText('Progress');
  await expect(page.getByTestId('tab-settings')).toHaveText('Settings');

  // ★ <html lang> は静的には en で、実行時にアプリが上書きする。支援技術の
  //   読み上げ音声がこの属性で決まるので、切り替えに追随しないと使えなくなる
  await expect(page.locator('html')).toHaveAttribute('lang', 'en');

  // 明示的に選んでいないので、保存された言語は無い（＝ブラウザ言語に従っている）
  const ui = await page.evaluate((k) => localStorage.getItem(k), UI_KEY);
  expect(ui === null || !JSON.parse(ui).lang).toBe(true);
});

test('英語で初期化した端末には、英語のプリセットが入る', async ({ page }) => {
  await boot(page);
  const chest = await openGroup(page, 'Chest');
  await expect(chest.getByTestId('exercise-name').first()).toHaveText('Bench Press');
});

test('設定で選んだ言語はブラウザ言語より優先され、再読み込みしても残る', async ({ page }) => {
  await boot(page);
  await openLanguage(page);

  // サブページの h1 は節名。画面の h1 は常に 1 つ
  await expect(page.locator('main h1')).toHaveCount(1);
  await expect(page.locator('main h1')).toHaveText('Language');

  await langButton(page, 'ja').click();

  // 画面が丸ごと作り直されて日本語になる
  await expect(page.getByTestId('tab-record')).toHaveText('記録');
  await expect(page.locator('html')).toHaveAttribute('lang', 'ja');
  // 切り替えた人はそのページに留まる（トップへ飛ばされない）
  await expect(page.locator('main h1')).toHaveText('言語');

  const saved = await page.evaluate((k) => JSON.parse(localStorage.getItem(k)), UI_KEY);
  expect(saved.lang).toBe('ja');

  await page.reload();
  await expect(page.getByTestId('tab-record')).toHaveText('記録');
  await expect(page.locator('html')).toHaveAttribute('lang', 'ja');
});

test('★ 未改名のプリセットは言語に追従し、改名したものは利用者の名前のまま', async ({ page }) => {
  await boot(page);

  // 英語で入ったプリセットを 1 つだけ改名する
  await openGroup(page, 'Chest');
  await page.getByTestId('exercise-name').filter({ hasText: exactText('Bench Press') }).click();
  await expect(page.getByTestId('settings-sheet')).toBeVisible();
  await page.getByTestId('exercise-rename').fill('My Bench');
  await page.getByTestId('settings-sheet-close').click();

  await openLanguage(page);
  await langButton(page, 'ja').click();

  // 部位名は日本語になる（未改名なので追従する）
  const chest = await openGroup(page, '胸');
  await expect(chest.getByTestId('group-name')).toHaveText('胸');
  // ★ 改名したものは利用者が付けた名前のまま
  await expect(chest.getByTestId('exercise-name').first()).toHaveText('My Bench');
  // ★ 触っていないものは日本語に追従する
  await expect(chest.getByTestId('exercise-name').nth(1)).toHaveText('ダンベルプレス');

  // 英語へ戻すと、未改名のものだけ英語に戻る
  await openLanguage(page);
  await langButton(page, 'en').click();
  const chestEn = await openGroup(page, 'Chest');
  await expect(chestEn.getByTestId('exercise-name').first()).toHaveText('My Bench');
  await expect(chestEn.getByTestId('exercise-name').nth(1)).toHaveText('Dumbbell Press');
});

test('自分で追加した種目は言語を切り替えても変わらない', async ({ page }) => {
  await boot(page);

  const chest = await openGroup(page, 'Chest');
  await chest.getByTestId('settings-add-exercise').click();
  await page.getByTestId('new-exercise-name').fill('My Own Lift');
  await page.getByTestId('new-exercise-submit').click();

  await openLanguage(page);
  await langButton(page, 'ja').click();

  const chestJa = await openGroup(page, '胸');
  await expect(
    chestJa.getByTestId('exercise-name').filter({ hasText: exactText('My Own Lift') }),
  ).toHaveCount(1);
});

test('言語を切り替えても記録は残る', async ({ page }) => {
  await boot(page);

  await page.getByTestId('add-exercise').click();
  await page
    .getByTestId('add-sheet')
    .getByTestId('pick-exercise')
    .filter({ hasText: exactText('Bench Press') })
    .click();
  await page.getByTestId('set-weight').first().fill('60');
  await page.getByTestId('set-reps').first().fill('10');
  await page.getByTestId('set-reps').first().blur();

  await openLanguage(page);
  await langButton(page, 'ja').click();

  await page.getByTestId('tab-record').click();
  await expect(page.getByTestId('set-weight').first()).toHaveValue('60');
  await expect(page.getByTestId('set-reps').first()).toHaveValue('10');
});

test('英語で書き出した TSV は英語の見出しで、日本語の見出しのファイルも取り込める', async ({
  page,
}) => {
  await boot(page);
  await page.evaluate(() => {
    window.__shared = [];
    navigator.share = async (data) => {
      window.__shared.push(data);
    };
    navigator.canShare = () => true;
    Object.defineProperty(navigator, 'userAgent', {
      value: 'Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15',
      configurable: true,
    });
  });

  await page.getByTestId('tab-settings').click();
  await page.getByTestId('open-backup').click();
  await page.getByTestId('backup-export').click();
  await page.waitForFunction(() => window.__shared?.length > 0);

  const tsv = await page.evaluate(async () => {
    const f = window.__shared[0].files[0];
    return await f.text();
  });
  expect(tsv.split('\n')[0]).toBe(
    'Date\tMuscle group\tExercise\tSet\tWeight kg\tReps\tBody weight kg\tSet note\tExercise note\tDay note\tTime\tRoutine\tPins',
  );

  // ★ 過去に日本語で書き出したファイルが、英語に切り替えた端末でも読める。
  //   ここが落ちると、言語を変えた人の手元のファイルが開けなくなる
  await page.getByTestId('backup-file').setInputFiles({
    name: 'fitness-memo-20260801-1200.tsv',
    mimeType: TSV_MIME,
    buffer: Buffer.from('日付\t部位\t種目\tセット\t重量kg\t回数\n2026-08-01\t胸\tベンチプレス\t1\t60\t10\n', 'utf-8'),
  });
  await expect(page.getByTestId('backup-confirm')).toBeVisible();
  await page.getByTestId('backup-apply').click();
  await expect(page.getByTestId('backup-note')).toContainText('Imported');
});

test('英語でも取り込めないファイルの理由が英語で出る', async ({ page }) => {
  await boot(page);
  await page.getByTestId('tab-settings').click();
  await page.getByTestId('open-backup').click();

  await page.getByTestId('backup-file').setInputFiles({
    name: 'x.tsv',
    mimeType: TSV_MIME,
    buffer: Buffer.from('a\tb\nc\td\n', 'utf-8'),
  });
  // ★ ここは以前 `cur_lang()` をイベントハンドラから呼んで落ちていた経路。
  //   文言が出ること自体が回帰テストになっている
  await expect(page.getByTestId('backup-note')).toContainText('does not look like a log');
  await expect(page.getByTestId('backup-confirm')).toHaveCount(0);
});
