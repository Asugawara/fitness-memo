// データの書き出し / 読み込み。
//
// ★ ここで検証できるのは「文字列の中身」と「UI の分岐」だけ。
//   Playwright の WebKit は download / share / clipboard のどれもジェスチャ要件を
//   再現せず、実機なら失敗する経路でも素通りさせる（`navigator.storage.persist` は
//   逆に常に false を返す）。**このファイルが緑でも iOS で動く保証はない。**
//   共有シートが実際に「ファイルに保存」を出すかは実機でしか確認できない。
import { expect, test } from '@playwright/test';

const KEY = 'fitness-memo/v3';

/** 種目タブを開いてバックアップシートを出す。 */
async function openSheet(page) {
  await page.goto('./');
  // ★ 初回起動直後は 400ms の debounce 保存がまだ走っていない。ここを待たないと
  //   localStorage が空のままで、控え（.pre-）も取れない状態を見ることになる
  await page.waitForFunction((k) => !!localStorage.getItem(k), KEY);
  await page.getByTestId('tab-menu').click();
  await page.getByTestId('open-backup').click();
  await expect(page.getByTestId('backup-sheet')).toBeVisible();
}

/** 貼り付け欄と「うまくいかないとき」は折りたたみの中にある。 */
async function expandDetails(page) {
  await page.evaluate(() => {
    document
      .querySelectorAll('[data-testid="backup-sheet"] details')
      .forEach((d) => (d.open = true));
  });
}

/** 貼り付けて確認画面まで進める。 */
async function paste(page, text) {
  await expandDetails(page);
  await page.getByTestId('backup-paste').fill(text);
  await page.getByTestId('backup-paste-load').click();
}

test('書き出した JSON はそのまま読み戻せる形で出ている', async ({ page }) => {
  await openSheet(page);

  const raw = await page.getByTestId('backup-json').inputValue();
  const parsed = JSON.parse(raw);

  expect(parsed.schema).toBe(3);
  expect(parsed.groups.map((g) => g.name)).toEqual(['胸', '背中', '肩', '腕', '脚', '体幹']);
  expect(parsed.exercises).toHaveLength(28);
  // ID は 12 文字の文字列。数値だと JSON.parse/stringify の往復で 2^53 超えが
  // 丸められ、参照が静かに壊れる
  for (const ex of parsed.exercises) {
    expect(typeof ex.id).toBe('string');
    expect(ex.id).toHaveLength(12);
  }
  // localStorage の中身と一致している（保存形式 = 書き出し形式）
  const stored = await page.evaluate((k) => localStorage.getItem(k), KEY);
  expect(JSON.parse(stored)).toEqual(parsed);
});

test('置き換えは前後の件数を見せ、控えを取ってから実行し、元に戻せる', async ({ page }) => {
  await openSheet(page);

  // 現在の DB に 1 日分の記録を足したものを取り込ませる
  const incoming = await page.evaluate(() => {
    const base = JSON.parse(document.querySelector('[data-testid="backup-json"]').value);
    const bench = base.exercises.find((e) => e.name === 'ベンチプレス').id;
    base.sessions['2026-08-01'] = {
      logs: [
        {
          exercise_id: bench,
          sets: [
            { weight: 60, reps: 10 },
            { weight: 60, reps: 8 },
          ],
          at: null,
        },
      ],
      body_weight: 70.5,
      note: '調子よい',
    };
    return JSON.stringify(base);
  });

  await page.getByTestId('backup-pane-import').click();
  await paste(page, incoming);

  // ★ 現在と読込後を両方出す。片方だけでは「0 日のファイルで全消し」が止まらない
  const confirm = page.getByTestId('backup-confirm');
  await expect(confirm).toContainText('記録 0 日');
  await expect(confirm).toContainText('記録 1 日');

  await page.getByTestId('backup-apply').click();
  await expect(page.getByTestId('backup-note')).toContainText('取り込みました');

  // 実行前の控えが残っている
  const preKeys = await page.evaluate(() =>
    Object.keys(localStorage).filter((k) => k.includes('.pre-')),
  );
  expect(preKeys).toHaveLength(1);
  const saved = await page.evaluate((k) => JSON.parse(localStorage.getItem(k)), KEY);
  expect(Object.keys(saved.sessions)).toContain('2026-08-01');

  // ★ 「元に戻す」はシートを開いている間だけの導線（リロードすると消える）。
  //   取り込み直後に間違いに気づくのが典型なので、そこで戻せれば足りる。
  //   リロード後は下の退避データ一覧から救う
  await page.getByTestId('backup-undo').click();
  await expect(page.getByTestId('backup-note')).toContainText('元に戻しました');
  const restored = await page.evaluate((k) => JSON.parse(localStorage.getItem(k)), KEY);
  expect(Object.keys(restored.sessions)).not.toContain('2026-08-01');
});

// ADR-0012 が「退避データを UI から読む手段がない ... iPhone 単体では実質的に
// 救出不可能」と自認していた穴。ここが塞がっていることを見る。
test('保管中のデータは一覧に出て、中身を見て取り込み直せる', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem(
      'fitness-memo/v3.bak-1700000000000',
      JSON.stringify({
        schema: 3,
        groups: [{ id: '00000000000g', name: '胸', color: '#e0524a', order: 0 }],
        exercises: [{ id: '00000000000h', name: '救出テスト', group_id: '00000000000g', order: 0 }],
        sessions: {
          '2024-03-03': {
            logs: [{ exercise_id: '00000000000h', sets: [{ weight: 50, reps: 5 }], at: null }],
            body_weight: null,
            note: '',
          },
        },
      }),
    );
  });
  await openSheet(page);

  const quarantine = page.getByTestId('backup-quarantine');
  await expect(quarantine).toContainText('保管中のデータ');
  await page.evaluate(() => {
    document.querySelector('[data-testid="backup-quarantine"]').open = true;
  });
  await expect(quarantine).toContainText('読み込み失敗の退避');

  await page.getByTestId('backup-restore').first().click();
  // 読み込みペインへ移り、確認画面に退避データの中身が出る
  await expect(page.getByTestId('backup-confirm')).toContainText('記録 1 日');

  await page.getByTestId('backup-apply').click();
  const saved = await page.evaluate((k) => JSON.parse(localStorage.getItem(k)), KEY);
  expect(saved.exercises.map((e) => e.name)).toContain('救出テスト');
});

test('壊れた JSON は取り込まれず、今のデータが 1 バイトも変わらない', async ({ page }) => {
  await openSheet(page);
  const before = await page.evaluate((k) => localStorage.getItem(k), KEY);

  await page.getByTestId('backup-pane-import').click();

  for (const [bad, expected] of [
    ['{"schema":3,"groups":', 'データが途中で切れている'],
    ['[1,2,3]', 'このアプリの記録ではない'],
    ['{"schema":99,"groups":[],"exercises":[],"sessions":{}}', '新しい版'],
  ]) {
    await paste(page, bad);
    await expect(page.getByTestId('backup-note')).toContainText(expected);
    // 確認画面まで進んでいない = 取り込みボタンが無い
    await expect(page.getByTestId('backup-confirm')).toHaveCount(0);
  }

  expect(await page.evaluate((k) => localStorage.getItem(k), KEY)).toBe(before);
});

test('保存できなくなったら黙って動き続けず警告を出す', async ({ page }) => {
  // 本体キーへの書き込みだけを容量超過にする
  await page.addInitScript(() => {
    const original = Storage.prototype.setItem;
    Storage.prototype.setItem = function (key, value) {
      if (key === 'fitness-memo/v3') {
        throw new DOMException('quota', 'QuotaExceededError');
      }
      return original.call(this, key, value);
    };
  });
  await page.goto('./');

  // 何か入力して保存を走らせる（debounce 400ms + flush）
  await page.getByTestId('tab-menu').click();
  await page.getByTestId('open-backup').click();
  await page.getByTestId('backup-sheet-close').click();
  await page.evaluate(() => {
    document.dispatchEvent(new Event('visibilitychange', { bubbles: true }));
  });
  // hidden → visible の順で発火させ、visible 側で警告を拾わせる
  await page.evaluate(() => {
    Object.defineProperty(document, 'hidden', { value: true, configurable: true });
    document.dispatchEvent(new Event('visibilitychange', { bubbles: true }));
    Object.defineProperty(document, 'hidden', { value: false, configurable: true });
    document.dispatchEvent(new Event('visibilitychange', { bubbles: true }));
  });

  await expect(page.getByTestId('restore-notice')).toContainText('保存できていません');
});
