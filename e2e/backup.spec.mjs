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

// ── 表計算（CSV / Google スプレッドシート）──────────────────────────────────
//
// ★ ここでも「文字列の中身」と「UI の分岐」しか見ていない。**取り込みは
//   page.route のモックで、実物の Google には当てていない。** 共有設定を変えないと
//   通らないうえ、外部サービスの死活でテストが赤くなるため。
//   モックが緑でも、実シートでの往復と iOS の共有シートに .csv が出るかは別に確かめる
//   （adr/storage/import-from-published-sheet-url.md「結果（トレードオフ）」）。

/** 貼られる形の URL。ID は 20 文字以上でないと ID と見なさない。 */
const SHEET_URL =
  'https://docs.google.com/spreadsheets/d/1BxiMVs0XRA5nFMdKvBdBZjgmUUqptlbs74OgvE2upms/edit#gid=0';

/**
 * Service Worker を登録させない。**取り込みのモックはこれが無いと成立しない。**
 *
 * ★ WebKit では、SW に制御されたページの fetch を Playwright が route できない
 *   （`pwa.spec.mjs` のオフライン検証が「setOffline は SW 発のリクエストに効かない」と
 *   書いているのと同じ壁）。外さないとモックが素通りして、**テストが実際の
 *   docs.google.com へ通信する**。落ちるだけならまだしも、外部サービスの死活で
 *   結果が変わるテストになってしまう。
 *
 *   `public/sw.js` は `url.origin !== self.location.origin` で早期 return するので、
 *   **この経路の挙動は SW の有無で変わらない**（adr/storage/import-from-published-sheet-url.md）。
 *   SW そのものの検証は `pwa.spec.mjs` の担当。
 */
async function withoutServiceWorker(page) {
  await page.addInitScript(() => {
    Object.defineProperty(navigator, 'serviceWorker', {
      get: () => undefined,
      configurable: true,
    });
  });
}

/**
 * docs.google.com への GET を横取りして中身を返す。
 *
 * 返す配列には横取りした URL が入る。**素通りしていないことを数で見る**ための口。
 */
async function mockSheet(page, { body, status = 200, contentType = 'text/csv' }) {
  await withoutServiceWorker(page);
  const calls = [];
  await page.route('**/docs.google.com/**', (route) => {
    calls.push(route.request().url());
    return route.fulfill({ status, contentType, body });
  });
  return calls;
}

/** URL 欄に貼って読み込む。 */
async function loadFromUrl(page) {
  await page.getByTestId('backup-pane-import').click();
  await page.getByTestId('backup-sheet-url').fill(SHEET_URL);
  await page.getByTestId('backup-sheet-load').click();
}

const CSV = [
  '日付,部位,種目,セット,重量kg,回数,セットメモ,種目メモ,体重kg,当日メモ',
  '2026-08-01,胸,ベンチプレス,1,60,10,,フォーム確認,70.5,調子よい',
  '2026-08-01,胸,ベンチプレス,2,62.5,8,きつい,フォーム確認,70.5,調子よい',
  '',
].join('\n');

/**
 * 記録を 1 日分入れて書き出しペインに戻る。
 *
 * ★ localStorage を直接書いて reload する形にしない。この 1 本のために 44MB の
 *   デバッグ wasm をもう一度読むことになり、ファイル単体で回したときに
 *   ワーカーが揃って詰まる。既存の取り込み経路を通せば読み直しが要らない。
 */
async function seedOneDay(page, { note, bodyWeight }) {
  const incoming = await page.evaluate(
    ([note, bodyWeight]) => {
      const base = JSON.parse(document.querySelector('[data-testid="backup-json"]').value);
      const bench = base.exercises.find((e) => e.name === 'ベンチプレス').id;
      base.sessions['2026-08-01'] = {
        logs: [{ exercise_id: bench, sets: [{ weight: 60, reps: 10 }], at: null }],
        body_weight: bodyWeight,
        note,
      };
      return JSON.stringify(base);
    },
    [note, bodyWeight],
  );
  await page.getByTestId('backup-pane-import').click();
  await paste(page, incoming);
  await page.getByTestId('backup-mode-replace').click();
  await page.getByTestId('backup-apply').click();
  await expect(page.getByTestId('backup-note')).toContainText('取り込みました');
  await page.getByTestId('backup-pane-export').click();
}

test('CSV は BOM 付きで、1 セット 1 行で出る', async ({ page, browserName }) => {
  // ★ Chromium 限定。iOS は共有シート経路なので download イベントが出ない
  //   （`transfer::pick_route` が構造的に `<a download>` を選ばない）
  test.skip(browserName !== 'chromium', 'download は Chromium 経路でしか通らない');
  await openSheet(page);
  await seedOneDay(page, { note: '調子よい', bodyWeight: 70.5 });

  const [download] = await Promise.all([
    page.waitForEvent('download'),
    page.getByTestId('backup-export-csv').click(),
  ]);
  expect(download.suggestedFilename()).toMatch(/^fitness-memo-\d{8}-\d{4}\.csv$/);

  const { readFile } = await import('node:fs/promises');
  const text = await readFile(await download.path(), 'utf8');

  // ★ BOM が無いと日本語環境の Excel が Shift_JIS と誤認して文字化けする
  expect(text.charCodeAt(0)).toBe(0xfeff);
  const lines = text.slice(1).trimEnd().split('\n');
  expect(lines[0]).toBe('日付,部位,種目,セット,重量kg,回数,セットメモ,種目メモ,体重kg,当日メモ');
  expect(lines[1]).toBe('2026-08-01,胸,ベンチプレス,1,60,10,,,70.5,調子よい');
});

test('スプレッドシートの URL から取り込める', async ({ page }) => {
  const calls = await mockSheet(page, { body: CSV });
  await openSheet(page);
  await loadFromUrl(page);

  // ★ 実物に当たっていないことを確かめる。素通りするとテストが外部サービスに依存する
  expect(calls).toHaveLength(1);
  expect(calls[0]).toContain('/export?format=csv');
  expect(calls[0], 'ブラウザで見えていた gid を引き継いでいない').toContain('gid=0');

  const confirm = page.getByTestId('backup-confirm');
  await expect(confirm).toContainText('記録 0 日');
  await expect(confirm).toContainText('記録 1 日');

  await page.getByTestId('backup-apply').click();
  await expect(page.getByTestId('backup-note')).toContainText('追加しました');

  const saved = await page.evaluate((k) => JSON.parse(localStorage.getItem(k)), KEY);
  const day = saved.sessions['2026-08-01'];
  expect(day.body_weight).toBe(70.5);
  expect(day.note).toBe('調子よい');
  expect(day.logs[0].sets).toEqual([
    { weight: 60, reps: 10 },
    { weight: 62.5, reps: 8, note: 'きつい' },
  ]);
  // ★ 取り込みは過去日のバックフィルなので時刻を持たない
  expect(day.logs[0].at).toBeNull();
});

test('表からの取り込みでは「置き換える」を出さない', async ({ page }) => {
  // ★ 退行の固定。CSV は ID・色・並び順を持たないので、置き換えると
  //   部位の色と並び順が黙って消える（adr/storage/csv-as-a-secondary-lossy-format.md）
  await mockSheet(page, { body: CSV });
  await openSheet(page);
  await loadFromUrl(page);

  await expect(page.getByTestId('backup-confirm')).toBeVisible();
  await expect(page.getByTestId('backup-mode-replace')).toHaveCount(0);
  await expect(page.getByTestId('backup-mode-merge')).toHaveCount(0);
  // 選ばせない代わりに、何が起きるかは必ず言う
  await expect(page.locator('[data-testid=backup-sheet]')).toContainText(
    '今の記録は 1 つも書き換えず、無い分だけ足します',
  );
});

test('JSON の取り込みでは「置き換える」を出し続ける', async ({ page }) => {
  await openSheet(page);
  const json = await page.getByTestId('backup-json').inputValue();
  await page.getByTestId('backup-pane-import').click();
  await paste(page, json);
  await expect(page.getByTestId('backup-mode-replace')).toBeVisible();
});

test('取り込めなかった行は件数と理由を出す', async ({ page }) => {
  // ★ 黙って 0 件取り込むことはしない（adr/ux/spreadsheet-import-asks-nothing.md）
  await mockSheet(page, {
    body: [
      '日付,部位,種目,セット,重量kg,回数',
      '2026-08-01,胸,ベンチプレス,1,60,10',
      '2026-08-01,Push Day,Bench Press,1,60,10',
      '08/13/2026,胸,ベンチプレス,1,60,10',
      '',
    ].join('\n'),
  });
  await openSheet(page);
  await loadFromUrl(page);

  const report = page.getByTestId('backup-sheet-report');
  await expect(report).toContainText('取り込めない行: 2 / 3 件');
  await expect(report).toContainText('Push Day');
  // 曖昧な日付は推測せず落とす（08/13 と 13/08 のどちらとも読めるため）
  await expect(report).toContainText('08/13/2026');
});

test('共有されていないシートは共有設定を直せと言う', async ({ page }) => {
  // ★ Google は非公開シートに CORS ヘッダ付きの 404 + text/html を返すので、
  //   fetch は成功する。だから「なぜか失敗しました」ではなく理由を言い切れる
  await mockSheet(page, {
    status: 404,
    contentType: 'text/html; charset=utf-8',
    body: '<!DOCTYPE html><html><body>Sorry, unable to open the file</body></html>',
  });
  await openSheet(page);
  await loadFromUrl(page);

  await expect(page.getByTestId('backup-note')).toContainText('リンクを知っている全員');
  await expect(page.getByTestId('backup-confirm')).toHaveCount(0);
});

test('200 でもログインページが返ったら共有設定を疑う', async ({ page }) => {
  // ★ content-type を ok() より先に見る理由。ok() だけ見ていると HTML を CSV として
  //   読ませて「見出しがありません」と誤診する
  await mockSheet(page, {
    contentType: 'text/html; charset=utf-8',
    body: '<!DOCTYPE html><html><body>Sign in</body></html>',
  });
  await openSheet(page);
  await loadFromUrl(page);
  await expect(page.getByTestId('backup-note')).toContainText('リンクを知っている全員');
});

test('オフラインならオフラインと言う', async ({ page }) => {
  await withoutServiceWorker(page);
  await page.addInitScript(() => {
    Object.defineProperty(navigator, 'onLine', { get: () => false, configurable: true });
  });
  await page.route('**/docs.google.com/**', (route) => route.abort('internetdisconnected'));
  await openSheet(page);
  await loadFromUrl(page);
  await expect(page.getByTestId('backup-note')).toContainText('オフライン');
});

test('シートを閉じたあとに応答が来ても、確認画面を復活させない', async ({ page }) => {
  // ★ BackupSheet は常時マウント（<dialog> を open で倒すだけ）なので、閉じた後に
  //   応答が着くと、次に開いたとき**頼んでいない取り込み**が確認待ちで出ている
  await withoutServiceWorker(page);
  await page.route('**/docs.google.com/**', async (route) => {
    await new Promise((r) => setTimeout(r, 1500));
    await route.fulfill({ status: 200, contentType: 'text/csv', body: CSV });
  });
  await openSheet(page);
  await loadFromUrl(page);
  await page.getByTestId('backup-sheet-close').click();

  await page.waitForTimeout(2500);
  await page.getByTestId('open-backup').click();
  await page.getByTestId('backup-pane-import').click();
  await expect(page.getByTestId('backup-confirm')).toHaveCount(0);
});

test('Google スプレッドシート以外の URL は貼る前に弾く', async ({ page }) => {
  await openSheet(page);
  await page.getByTestId('backup-pane-import').click();
  await page.getByTestId('backup-sheet-url').fill('https://example.com/a.csv');
  await page.getByTestId('backup-sheet-load').click();
  await expect(page.getByTestId('backup-note')).toContainText('docs.google.com');
});

test('スプレッドシートから貼ったセル（タブ区切り）も読める', async ({ page }) => {
  // 共有設定を変えずに済む最短経路。URL より先に閉じておきたい導線
  await openSheet(page);
  await page.getByTestId('backup-pane-import').click();
  await paste(
    page,
    ['日付\t種目\t重量kg\t回数', '2026-08-01\tベンチプレス\t60\t10'].join('\n'),
  );
  await expect(page.getByTestId('backup-confirm')).toContainText('記録 1 日');
  await expect(page.getByTestId('backup-mode-replace')).toHaveCount(0);
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

  // ★ 測る前に可視を待つ。`boundingBox()` は要素が付いた時点で返るので、
  //   ワーカーが揃って重い wasm を読んでいるときにレイアウト途中の値を拾う
  const tapTarget = async (id) => {
    const el = page.getByTestId(id);
    await expect(el, `${id} が描画されていない`).toBeVisible();
    const box = await el.boundingBox();
    expect(box.height, `${id} のタップ標的が 44px 未満`).toBeGreaterThanOrEqual(44);
  };

  for (const id of ['backup-export', 'backup-export-csv', 'backup-copy']) {
    await tapTarget(id);
  }

  await page.getByTestId('backup-pane-import').click();
  await expandDetails(page);
  // URL 欄も指で押せる大きさが要る
  for (const id of ['backup-sheet-url', 'backup-sheet-load', 'backup-paste-load']) {
    await tapTarget(id);
  }
});
