// データの書き出し / 読み込み。
//
// ★ ここで検証できるのは「文字列の中身」と「UI の分岐」だけ。
//   Playwright の WebKit は download / share / clipboard のどれもジェスチャ要件を
//   再現せず、実機なら失敗する経路でも素通りさせる（`navigator.storage.persist` は
//   逆に常に false を返す）。**このファイルが緑でも iOS で動く保証はない。**
//   共有シートが実際に `.tsv` へ「ファイルに保存」を出すか、隠した <input type=file> を
//   click() したとき standalone PWA でピッカーが開いて復帰後も状態が生きているかは、
//   実機でしか確認できない。
//
// ★ TSV の中身そのもの（列の並び・往復・冪等・入力の癖）は `cargo test` の
//   `core::tests::export_tsv_*` / `tsv_import_*` が持つ。ここは配線だけを見る。
import { expect, test } from '@playwright/test';

const KEY = 'fitness-memo/v3';
const TSV_MIME = 'text/tab-separated-values';

/** 設定タブを開いてバックアップシートを出す。 */
async function openSheet(page) {
  await page.goto('./');
  // ★ 初回起動直後は 400ms の debounce 保存がまだ走っていない。ここを待たないと
  //   localStorage が空のままで、控え（.pre-）も取れない状態を見ることになる
  await page.waitForFunction((k) => !!localStorage.getItem(k), KEY);
  await page.getByTestId('tab-settings').click();
  await page.getByTestId('open-backup').click();
  await expect(page.getByTestId('backup-sheet')).toBeVisible();
}

/**
 * 共有シートを差し替えて、渡された `ShareData` を `window.__shared` に控える。
 *
 * ★ UA も iPhone にする。`transfer::pick_route` は iOS のときだけ Share を選ぶので、
 *   ここを偽らないと chromium では Download 経路に落ちる。
 */
async function stubShare(page) {
  await page.addInitScript(() => {
    Object.defineProperty(navigator, 'userAgent', {
      value: 'Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15',
      configurable: true,
    });
    Object.defineProperty(navigator, 'canShare', { value: () => true, configurable: true });
    Object.defineProperty(navigator, 'share', {
      configurable: true,
      value: async (data) => {
        const file = data.files?.[0];
        window.__shared = {
          keys: Object.keys(data),
          name: file?.name,
          type: file?.type,
          text: file ? await file.text() : null,
        };
      },
    });
  });
}

/** 隠した input にファイルを流し込む。★ setInputFiles は attached しか要求しない。 */
async function importFile(page, text, name = 'fitness-memo-20260801-1200.tsv') {
  await page.getByTestId('backup-file').setInputFiles({
    name,
    mimeType: TSV_MIME,
    buffer: Buffer.from(text, 'utf-8'),
  });
}

/** 1 日分の記録が入った TSV。取り込むと必ず「増える」側になる。 */
const ONE_DAY_TSV =
  '日付\t部位\t種目\tセット\t重量kg\t回数\t体重kg\tセットメモ\t種目メモ\t体調メモ\t時刻\n' +
  '2026-08-01\t胸\tベンチプレス\t1\t60\t10\t70.5\t\t\t調子よい\t\n' +
  '2026-08-01\t胸\tベンチプレス\t2\t60\t8\t\t\t\t\t\n';

// ── 書き出し ────────────────────────────────────────────────────────────────

// ★ iOS の主経路の**形**を見る唯一のテスト。files 以外を混ぜると
//   UIActivityViewController から「ファイルに保存」が消える
//   （adr/storage/share-sheet-over-download.md）。
test('書き出しは共有シートに files だけを .tsv で渡す', async ({ page }) => {
  await stubShare(page);
  await openSheet(page);

  await page.getByTestId('backup-export').click();
  await expect(page.getByTestId('backup-note')).toContainText('書き出しました');

  await page.waitForFunction(() => !!window.__shared);
  const shared = await page.evaluate(() => window.__shared);
  expect(shared.keys, 'files 以外が混ざると「ファイルに保存」が消える').toEqual(['files']);
  expect(shared.name).toMatch(/^fitness-memo-\d{8}-\d{4}\.tsv$/);
  expect(shared.type).toBe(TSV_MIME);
  // 見出しは外部仕様（cargo test がバイト一致で固定しているのと同じ並び）
  expect(shared.text.split('\n')[0]).toBe(
    '日付\t部位\t種目\tセット\t重量kg\t回数\t体重kg\tセットメモ\t種目メモ\t体調メモ\t時刻',
  );
  // 保存形式は JSON のまま（書き出し形式とは別。adr/storage/tsv-export-for-spreadsheets.md）
  const stored = await page.evaluate((k) => localStorage.getItem(k), KEY);
  expect(JSON.parse(stored).schema).toBe(3);
});

test('ダウンロード経路は 1 タップで .tsv を落とす', async ({ page, browserName }) => {
  // ★ chromium 限定。WebKit の Playwright は download のジェスチャ要件を再現しない
  test.skip(browserName !== 'chromium', 'download の検証は chromium だけ');
  await openSheet(page);

  const [download] = await Promise.all([
    page.waitForEvent('download'),
    page.getByTestId('backup-export').click(),
  ]);
  expect(download.suggestedFilename()).toMatch(/^fitness-memo-\d{8}-\d{4}\.tsv$/);
});

test('書き出した TSV はそのまま読み戻せる', async ({ page }) => {
  await stubShare(page);
  await openSheet(page);

  // 1 日分入れてから書き出す
  await importFile(page, ONE_DAY_TSV);
  await page.getByTestId('backup-apply').click();
  await expect(page.getByTestId('backup-note')).toContainText('取り込みました');

  await page.getByTestId('backup-export').click();
  // ★ share() は Promise。控えが積まれるまで待たないと undefined を掴む
  await page.waitForFunction(() => !!window.__shared);
  const tsv = await page.evaluate(() => window.__shared.text);

  // 日付をずらして読み戻すと、その分だけ増える
  await importFile(page, tsv.replaceAll('2026-08-01', '2026-09-01'));
  await expect(page.getByTestId('backup-confirm')).toContainText('1 日分');
  await page.getByTestId('backup-apply').click();

  const saved = await page.evaluate((k) => JSON.parse(localStorage.getItem(k)), KEY);
  expect(Object.keys(saved.sessions)).toEqual(
    expect.arrayContaining(['2026-08-01', '2026-09-01']),
  );
});

// ── 取り込み ────────────────────────────────────────────────────────────────

// ★ 「現在」と「取り込み後」を両方出す。取り込み後は**マージ済みの結果**で、
//   取り込むファイル自身の要約ではない（足すだけなので、それは嘘になる）。
test('取り込みは現在と取り込み後を並べ、今の記録を減らさない', async ({ page }) => {
  await openSheet(page);
  await importFile(page, ONE_DAY_TSV);

  const confirm = page.getByTestId('backup-confirm');
  await expect(confirm).toContainText('記録 0 日');
  await expect(confirm).toContainText('記録 1 日');
  await expect(confirm).toContainText('を追加します');

  // 記録 0 日のファイルでも「現在」が減らない = 全消しが構造的に起きない
  await page.getByTestId('backup-apply').click();
  await page.getByTestId('backup-import').click();
  await importFile(page, '日付\t部位\t種目\tセット\t重量kg\t回数\n\t胸\t自作マシン\t\t\t\n');
  await expect(confirm).toContainText('記録 1 日');
  await expect(confirm).not.toContainText('記録 0 日');
});

test('取り込みは控えを取ってから実行する', async ({ page }) => {
  await openSheet(page);
  await importFile(page, ONE_DAY_TSV);
  await page.getByTestId('backup-apply').click();
  await expect(page.getByTestId('backup-note')).toContainText('取り込みました');

  const preKeys = await page.evaluate(() =>
    Object.keys(localStorage).filter((k) => k.includes('.pre-')),
  );
  expect(preKeys).toHaveLength(1);
  const saved = await page.evaluate((k) => JSON.parse(localStorage.getItem(k)), KEY);
  expect(Object.keys(saved.sessions)).toContain('2026-08-01');
});

// ★ 「元に戻す」は取り込みと同じだけ破壊的（戻す先より後の記録が消える）ので確認を挟む。
//   1 回目は実行せず、何に戻るかを出すだけ。
test('「元に戻す」は 2 回押さないと実行されず、戻す前も保管する', async ({ page }) => {
  await openSheet(page);
  await importFile(page, ONE_DAY_TSV);
  await page.getByTestId('backup-apply').click();
  await expect(page.getByTestId('backup-undo')).toBeVisible();

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
  await importFile(page, ONE_DAY_TSV);
  await page.getByTestId('backup-apply').click();
  await expect(page.getByTestId('backup-undo')).toBeVisible();

  await page.getByTestId('backup-sheet-close').click();
  await page.getByTestId('open-backup').click();
  await expect(page.getByTestId('backup-undo')).toHaveCount(0);
});

test('壊れたファイルは取り込まれず、今のデータが 1 バイトも変わらない', async ({ page }) => {
  await openSheet(page);
  const before = await page.evaluate((k) => localStorage.getItem(k), KEY);

  for (const [bad, expected] of [
    ['あ\tい\nう\tえ\n', 'このアプリの記録ではない'],
    ['日付\t部位\t重量kg\n', '見出しが読めません'],
    ['日付\t部位\t種目\tセット\t重量kg\t回数\n', '取り込める記録が入っていません'],
    ['{"schema":3,"groups":', 'データが途中で切れている'],
  ]) {
    await importFile(page, bad);
    await expect(page.getByTestId('backup-note')).toContainText(expected);
    // 確認画面まで進んでいない = 取り込みボタンが無い
    await expect(page.getByTestId('backup-confirm')).toHaveCount(0);
  }

  expect(await page.evaluate((k) => localStorage.getItem(k), KEY)).toBe(before);
});

// ★ input.value を戻さないと 2 回目の change が飛ばない。貼り付けという逃げ道が
//   無くなったので、「やめる → もう一度同じファイル」で詰む。
test('同じファイルを続けて 2 回選んでも確認画面が出る', async ({ page }) => {
  await openSheet(page);

  await importFile(page, ONE_DAY_TSV);
  await expect(page.getByTestId('backup-confirm')).toBeVisible();
  await page.getByTestId('backup-cancel').click();
  await expect(page.getByTestId('backup-confirm')).toHaveCount(0);

  await importFile(page, ONE_DAY_TSV);
  await expect(page.getByTestId('backup-confirm'), '2 回目の change が飛んでいない').toBeVisible();
});

// ★ `toBeHidden()` では見ない。opacity:0 の 1px 要素は Playwright の定義では
//   「visible」で、それは**意図どおり**（display:none にすると iOS で click() が
//   無視されうるので避けている。adr/ux/hidden-file-input-behind-a-button.md）。
//   見たいのは「利用者の目にもタップにも触れないこと」なので、実測で確かめる。
test('「読み込む」は押せるボタンで、input は目に触れない', async ({ page }) => {
  await openSheet(page);

  const input = page.getByTestId('backup-file');
  expect(await input.count(), 'input が DOM に無いと setInputFiles が使えない').toBe(1);
  const style = await input.evaluate((el) => {
    const s = getComputedStyle(el);
    return { opacity: s.opacity, pointerEvents: s.pointerEvents, display: s.display };
  });
  expect(style.opacity).toBe('0');
  expect(style.pointerEvents).toBe('none');
  expect(style.display, 'display:none にすると iOS で click() が無視されうる').not.toBe('none');
  const inputBox = await input.boundingBox();
  expect(inputBox.height, 'input が場所を取っている').toBeLessThanOrEqual(1);
  // シート内の file input は 1 つだけ（隠したものと出したものが二重にならない）
  expect(await page.locator('[data-testid=backup-sheet] input[type=file]').count()).toBe(1);

  const box = await page.getByTestId('backup-import').boundingBox();
  expect(box, '「読み込む」が描画されていない').not.toBeNull();
  expect(box.height, 'タップ標的が 44px 未満').toBeGreaterThanOrEqual(44);
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
  await page.getByTestId('tab-settings').click();

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

    for (const id of ['backup-export', 'backup-import']) {
      expect(
        await contrastRatio(page.getByTestId(id)),
        `${scheme} で「${id}」が背景に埋もれている`,
      ).toBeGreaterThanOrEqual(4.5);
    }
  });
}

test('シートの中にクラスなしの button を作らない', async ({ page }) => {
  // ★ 退行の固定。UA 既定の chrome に任せた瞬間にダークで消え、44px も割る。
  //   色ではなく構造で見るので、どのテーマで回しても落ちる
  await openSheet(page);
  const bare = page.locator('[data-testid=backup-sheet] button:not([class])');
  await expect(bare).toHaveCount(0);

  // 確認状態でも見る（要素の入れ替わりが大きいので、待機状態だけでは足りない）
  await importFile(page, ONE_DAY_TSV);
  await expect(page.getByTestId('backup-confirm')).toBeVisible();
  await expect(bare).toHaveCount(0);
});

// ★ 上の 2 本は .secondary が自前で背景を持つので、`color-scheme` を消しても通る。
//   宣言が守っているのは**自前で色を持てないもの** — select のネイティブピッカーと
//   スクロールバー。そこは getComputedStyle で覗けないので、宣言そのものと、
//   素の <button> を代理にした効果を別々に見る。
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

  for (const id of ['backup-export', 'backup-import']) {
    const box = await page.getByTestId(id).boundingBox();
    expect(box, `${id} が描画されていない`).not.toBeNull();
    expect(box.height, `${id} のタップ標的が 44px 未満`).toBeGreaterThanOrEqual(44);
  }

  await importFile(page, ONE_DAY_TSV);
  const apply = await page.getByTestId('backup-apply').boundingBox();
  expect(apply.height, '「取り込む」のタップ標的が 44px 未満').toBeGreaterThanOrEqual(44);
});
