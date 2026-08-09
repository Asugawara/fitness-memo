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

  // ★ 「元に戻す」は取り込みと同じだけ破壊的（戻す先より後の記録が消える）ので
  //   確認を挟む。1 回目は実行せず、何に戻るかを出すだけ
  await page.getByTestId('backup-undo').click();
  await expect(page.getByTestId('backup-note')).toContainText('もう一度押すと実行します');
  const notYet = await page.evaluate((k) => JSON.parse(localStorage.getItem(k)), KEY);
  expect(Object.keys(notYet.sessions), '1 回目のタップで実行された').toContain('2026-08-01');

  await page.getByTestId('backup-undo').click();
  await expect(page.getByTestId('backup-note')).toContainText('元に戻しました');
  const restored = await page.evaluate((k) => JSON.parse(localStorage.getItem(k)), KEY);
  expect(Object.keys(restored.sessions)).not.toContain('2026-08-01');

  // ★ 巻き戻し**前**の状態も退避されている。復旧操作そのものが全損経路にならない
  const preKeysAfterUndo = await page.evaluate(() =>
    Object.keys(localStorage).filter((k) => k.includes('.pre-')),
  );
  expect(preKeysAfterUndo.length, '戻す前の状態が保管されていない').toBeGreaterThan(1);
});

// ★ シートを閉じたら「元に戻す」を持ち越さない。iOS の PWA は何日もレジュームされる
//   ので、残しておくと数日後に誤タップされ、その間の記録が消える。
test('シートを閉じると「元に戻す」は消える', async ({ page }) => {
  await openSheet(page);
  const incoming = await page.evaluate(() => {
    const base = JSON.parse(document.querySelector('[data-testid="backup-json"]').value);
    base.sessions['2026-08-01'] = {
      logs: [],
      body_weight: 70,
      note: 'あとで消す',
    };
    return JSON.stringify(base);
  });

  await page.getByTestId('backup-pane-import').click();
  await paste(page, incoming);
  await page.getByTestId('backup-apply').click();
  await expect(page.getByTestId('backup-undo')).toBeVisible();

  await page.getByTestId('backup-sheet-close').click();
  await page.getByTestId('open-backup').click();
  await page.getByTestId('backup-pane-import').click();
  await expect(page.getByTestId('backup-undo')).toHaveCount(0);
});

// adr/storage/quarantine-on-parse-failure.md が「退避データを UI から読む手段がない ... iPhone 単体では実質的に
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

// ── 見えることの検証（adr/ux/declare-color-scheme-for-ua-widgets.md）────
//
// ★ 「コピー」がダークで読めなかった原因は色の付け忘れではなく、`color-scheme` を
//   宣言していないこと。`button { color: inherit }` が UA の文字色だけ上書きし、
//   UA が描く背景はライトのまま取り残されて、コントラストが約 1.02:1 になっていた。
//   トークン値はベタ書きせずコントラスト比で見るので、ライト / ダーク両方で成立する。

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
 * ★ 背景が透明なら祖先を辿る。クラスを外して UA 既定へ戻すと backgroundColor は
 *   `rgba(0, 0, 0, 0)` を返すので、辿らないと body の色を拾って**通ってしまう**。
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

for (const scheme of ['dark', 'light']) {
  test(`${scheme} でシート内のボタンの文字が背景から読める`, async ({ page }) => {
    await page.emulateMedia({ colorScheme: scheme });
    await openSheet(page);
    await expandDetails(page);

    // 書き出しの救済経路。ここが読めないと、いちばん困っているときに押せない
    expect(
      await contrastRatio(page.getByTestId('backup-copy')),
      `${scheme} で「コピー」が背景に埋もれている`,
    ).toBeGreaterThanOrEqual(4.5);

    await page.getByTestId('backup-pane-import').click();
    await expandDetails(page);
    expect(
      await contrastRatio(page.getByTestId('backup-paste-load')),
      `${scheme} で「読み込む」が背景に埋もれている`,
    ).toBeGreaterThanOrEqual(4.5);
  });
}

test('シートの中にクラスなしの button を作らない', async ({ page }) => {
  // ★ 退行の固定。UA 既定の chrome に任せた瞬間にダークで消え、44px も割る。
  //   色ではなく構造で見るので、どのテーマで回しても落ちる
  await openSheet(page);
  await expandDetails(page);
  await expect(page.locator('[data-testid=backup-sheet] button:not([class])')).toHaveCount(0);

  await page.getByTestId('backup-pane-import').click();
  await expandDetails(page);
  await expect(page.locator('[data-testid=backup-sheet] button:not([class])')).toHaveCount(0);
});

// ★ 上の 2 本は .secondary が自前で背景を持つので、`color-scheme` を消しても通る。
//   宣言が守っているのは**自前で色を持てないもの** — input[type=file] の
//   「ファイルを選択」と select のネイティブピッカー。そこは getComputedStyle で
//   覗けないので、宣言そのものと、素の <button> を代理にした効果を別々に見る。
test('color-scheme を宣言している', async ({ page }) => {
  await page.goto('./');
  const declared = await page.evaluate(
    () => getComputedStyle(document.documentElement).colorScheme,
  );
  expect(declared, 'ライトとダークの両方を宣言していない').toContain('dark');
  expect(declared).toContain('light');
});

test('UA が描くコントロールがテーマに追従する', async ({ page, browserName }) => {
  // ★ Chromium 限定。WebKit のネイティブ form control は自前のテーマが描くので
  //   getComputedStyle に出ない（ダークでも backgroundColor は rgb(255,255,255) を
  //   返す）。宣言そのものは上の 1 本が全エンジンで見ている。
  test.skip(browserName !== 'chromium', 'WebKit / Firefox は UA 描画が computed style に出ない');
  await page.goto('./');

  const uaBackground = async (scheme) => {
    await page.emulateMedia({ colorScheme: scheme });
    return page.evaluate(() => {
      const probe = document.createElement('button');
      document.body.appendChild(probe);
      const bg = getComputedStyle(probe).backgroundColor;
      probe.remove();
      return bg;
    });
  };

  const dark = luminance(parseRgb(await uaBackground('dark')));
  const light = luminance(parseRgb(await uaBackground('light')));
  expect(dark, 'ダークで UA 既定の背景がライトのまま取り残されている').toBeLessThan(light);
});

test('シート内のボタンは 44px のタップ標的を持つ', async ({ page }) => {
  await openSheet(page);
  await expandDetails(page);

  for (const id of ['backup-export', 'backup-copy']) {
    const box = await page.getByTestId(id).boundingBox();
    expect(box, `${id} が描画されていない`).not.toBeNull();
    expect(box.height, `${id} のタップ標的が 44px 未満`).toBeGreaterThanOrEqual(44);
  }
});
