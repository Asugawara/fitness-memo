import { expect, test } from '@playwright/test';

// 新機能のお知らせバナー（画面最上段）と、そこから開くシート。
// src/views/whatsnew.rs の data-testid を使う。
//
// ★ 期待値は文言だけ手書きし、構造（件数・並び順）は DOM から読む。最新の id や
//   項目数を直書きすると、リリースのたびにこの spec を書き換えることになる
//   （i18n.rs の RELEASES に足すだけで済むようにするための決め事）。

const DB_KEY = 'fitness-memo/v3';
const UI_KEY = 'fitness-memo/ui/v1';

test.beforeEach(async ({ page }) => {
  // ★ baseURL がサブパス（例 /fitness-memo/）を持つとき、先頭 "/" は絶対パス参照として
  // ベースのパスを丸ごと破棄してしまう（new URL('/', 'http://h/sub/') === 'http://h/'）。
  // 相対参照の "./" でなければ E2E_BASE=/fitness-memo/ の重い側実行が壊れる
  await page.goto('./');
});

/**
 * hidden への visibilitychange を発火させ、pending の debounce 保存を即時 flush する。
 * Db（fitness-memo/v3）の保存は 400ms debounce なので、書かれた状態にしてから中身を見る。
 */
async function flushToStorage(page) {
  await page.getByTestId('screen-record').waitFor({ state: 'visible' });
  await page.evaluate(() => {
    Object.defineProperty(document, 'hidden', { value: true, configurable: true });
    document.dispatchEvent(new Event('visibilitychange', { bubbles: true }));
  });
}

/**
 * `<img>` が読み込み終わる（成功 / 失敗いずれか）まで待つ。
 * `e2e/manual.spec.mjs` の同名ヘルパーと同じ実装。共有モジュールは持たない
 * （`manual.spec.mjs` 冒頭のコメントが明記する既存 spec の作法）。
 */
async function waitForImageSettled(locator) {
  await locator.evaluate(
    (el) =>
      new Promise((resolve) => {
        if (el.complete) return resolve();
        el.addEventListener('load', resolve, { once: true });
        el.addEventListener('error', resolve, { once: true });
      }),
  );
}

/** UI 専用キーの生の JSON を読む。 */
async function uiState(page) {
  return page.evaluate((key) => JSON.parse(localStorage.getItem(key) ?? '{}'), UI_KEY);
}

/**
 * UI 専用キーを丸ごと置き換える。**マージしない。**
 *
 * ★ `UiState` の全フィールドが `#[serde(default)]` なので、渡さなかったキーは
 *   「まだ何も記録していない」に倒れる。既存利用者（このキー自体はあるが
 *   `release_seen` フィールドを持たない）を再現するには、マージではなく置き換えで
 *   ないと成立しない —— このファイルの起動（`beforeEach`）で 1 度アプリが起動し、
 *   `release_seen` の基準値が既に書き込まれているため。
 */
async function setUiState(page, obj) {
  await page.evaluate(
    ({ key, obj }) => localStorage.setItem(key, JSON.stringify(obj)),
    { key: UI_KEY, obj },
  );
}

test('1. 空の localStorage で起動するとバナーは出ず、release_seen に基準値が書かれる', async ({
  page,
}) => {
  // ★ 否定（バナーが出ないこと）を先に見ると、wasm 起動前に数えて常に 0 で
  //   通ってしまう（pwa.spec.mjs の install-hint と同じ罠）。先に record 画面の
  //   可視待ちを置いて、アプリが起動しきったことを保証する
  await expect(page.getByTestId('screen-record')).toBeVisible();
  await expect(page.getByTestId('whatsnew-banner')).toHaveCount(0);

  const ui = await uiState(page);
  expect(typeof ui.release_seen).toBe('number');
});

test('2. 既存利用者には一切出ない。基準値だけ書かれ、他の設定は元のまま', async ({ page }) => {
  // ★ 仕様「未読履歴なしの人（この機能が乗る前からの利用者）には一切出さない」を
  //   直接固定する唯一のテスト。release_seen を持たない UI 状態が「既存利用者」
  await setUiState(page, { lang: 'en', install_hint_dismissed: true, history: 2 });
  await page.reload();
  await expect(page.getByTestId('screen-record')).toBeVisible();

  await expect(page.getByTestId('whatsnew-banner')).toHaveCount(0);

  const ui = await uiState(page);
  expect(typeof ui.release_seen).toBe('number');
  expect(ui.lang, '既存の言語設定が巻き添えで消えていない').toBe('en');
  expect(ui.install_hint_dismissed, '既存のインストール案内の既読が残っている').toBe(true);
  expect(ui.history, '既存の表示件数設定が残っている').toBe(2);
});

test('3. 未読があるとき、バナー本文の件数がシート内の項目数と一致する', async ({ page }) => {
  await setUiState(page, { release_seen: 1 });
  await page.reload();
  await expect(page.getByTestId('screen-record')).toBeVisible();

  await expect(page.getByTestId('whatsnew-banner')).toBeVisible();
  await page.getByTestId('whatsnew-banner-open').click();
  const sheet = page.getByTestId('whatsnew-sheet');
  await expect(sheet).toBeVisible();

  const liCount = await sheet.locator('li').count();
  expect(liCount).toBeGreaterThan(0);
  await expect(page.getByTestId('whatsnew-banner-open')).toContainText(String(liCount));
});

test('4. 未読が複数リリースぶんあるとまとめて出て、新しい順に厳密減少し、閉じると先頭の番号が既読になる', async ({
  page,
}) => {
  await setUiState(page, { release_seen: 0 });
  await page.reload();
  await expect(page.getByTestId('screen-record')).toBeVisible();

  await page.getByTestId('whatsnew-banner-open').click();
  const sheet = page.getByTestId('whatsnew-sheet');
  // ★ `<Sheet>` は条件の外に常時マウントされているので、<section> は閉じていても
  //   DOM にある。可視待ちを入れないと「一度も開かなくても ids が取れる」テストになり、
  //   下の toBeHidden() と開閉の対も成立しない
  await expect(sheet).toBeVisible();

  const releases = sheet.getByTestId('whatsnew-release');
  const ids = await releases.evaluateAll((els) => els.map((el) => Number(el.dataset.id)));

  // ★ 出荷時点でリリースを 2 本仕込んである（1 本だと「まとめて見せる」が
  //   作れない）。将来リリースが増えてもここは書き換えなくてよい
  expect(ids.length).toBeGreaterThan(1);
  for (let i = 1; i < ids.length; i++) {
    expect(ids[i], '新しい順に厳密減少していること').toBeLessThan(ids[i - 1]);
  }

  await page.getByTestId('whatsnew-sheet-close').click();
  await expect(sheet).toBeHidden();

  const ui = await uiState(page);
  expect(ui.release_seen, '既読になる番号は先頭（最新）の id').toBe(ids[0]);
});

test('5. シートを閉じるとバナーが消え、リロードしても出ない', async ({ page }) => {
  // release_seen は「未読が残る値」であればよい（1 でも 0 でも経路は同じ）
  await setUiState(page, { release_seen: 1 });
  await page.reload();
  await expect(page.getByTestId('screen-record')).toBeVisible();

  await page.getByTestId('whatsnew-banner-open').click();
  const sheet = page.getByTestId('whatsnew-sheet');
  await expect(sheet).toBeVisible();

  await page.getByTestId('whatsnew-sheet-close').click();
  await expect(sheet).toBeHidden();
  await expect(page.getByTestId('whatsnew-banner')).toHaveCount(0);

  await page.reload();
  await expect(page.getByTestId('screen-record')).toBeVisible();
  await expect(page.getByTestId('whatsnew-banner')).toHaveCount(0);
});

test('6. ✕ で閉じても同じく既読になり、リロードしても出ない', async ({ page }) => {
  await setUiState(page, { release_seen: 0 });
  await page.reload();
  await expect(page.getByTestId('screen-record')).toBeVisible();

  // 期待値（先頭の id）を DOM から学習する。**開くだけなら release_seen は動かない**
  await page.getByTestId('whatsnew-banner-open').click();
  const topId = Number(
    await page
      .getByTestId('whatsnew-sheet')
      .getByTestId('whatsnew-release')
      .first()
      .getAttribute('data-id'),
  );

  // ✕ 単体の経路を検証したいので、学習用に開いたシートは閉じずにリロードして
  // 最初からやり直す（シートを閉じると mark_seen が走ってしまう）
  await page.reload();
  await expect(page.getByTestId('screen-record')).toBeVisible();
  await expect(page.getByTestId('whatsnew-banner')).toBeVisible();

  await page.getByTestId('whatsnew-banner-dismiss').click();
  await expect(page.getByTestId('whatsnew-banner')).toHaveCount(0);

  const ui = await uiState(page);
  expect(ui.release_seen).toBe(topId);

  await page.reload();
  await expect(page.getByTestId('screen-record')).toBeVisible();
  await expect(page.getByTestId('whatsnew-banner')).toHaveCount(0);
});

test('7. release_seen に整数の異常値が入っていても落ちず、他の設定は残る', async ({ page }) => {
  // 最新の id を DOM から学習する（テスト 4 と同じ方法）。開くだけなら
  // release_seen は動かないので、この後のループに影響しない
  await setUiState(page, { release_seen: 0 });
  await page.reload();
  await expect(page.getByTestId('screen-record')).toBeVisible();
  await page.getByTestId('whatsnew-banner-open').click();
  const topId = Number(
    await page
      .getByTestId('whatsnew-sheet')
      .getByTestId('whatsnew-release')
      .first()
      .getAttribute('data-id'),
  );

  // ★ "abc" のような文字列は入れない。`ui_state()`（storage.rs）は 1 フィールドでも
  //   型が合わなければ `UiState` 全体を `Default` に落とすので、文字列を入れると
  //   `lang` まで消えて必ず赤になる（history / lang の既存 spec も正常値の型しか
  //   入れていないのと同じ理由で、ここでも整数の範囲だけを崩す）
  for (const bad of [-1, 0, 9999, Number.MAX_SAFE_INTEGER]) {
    await setUiState(page, { lang: 'en', install_hint_dismissed: true, release_seen: bad });
    await page.reload();
    await expect(page.getByTestId('screen-record')).toBeVisible();

    const ui = await uiState(page);
    expect(ui.lang, `release_seen=${bad} で lang が巻き添えで消えていない`).toBe('en');
    expect(
      ui.install_hint_dismissed,
      `release_seen=${bad} で install_hint_dismissed が残っている`,
    ).toBe(true);

    // ★ `core::unseen_releases` は `id > last_seen` を未読とする。異常値でも
    //   この比較規則どおりにバナーの有無が決まること（黙って壊れない、を実際に見る）
    if (bad >= topId) {
      await expect(
        page.getByTestId('whatsnew-banner'),
        `release_seen=${bad} でバナーが出ている`,
      ).toHaveCount(0);
    } else {
      await expect(
        page.getByTestId('whatsnew-banner'),
        `release_seen=${bad} でバナーが出ていない`,
      ).toBeVisible();
    }
  }
});

test('8. release_seen は Db のキーには入らず、UI 専用キーだけに入る', async ({ page }) => {
  await setUiState(page, { release_seen: 0 });
  await page.reload();
  await expect(page.getByTestId('screen-record')).toBeVisible();

  await flushToStorage(page);

  const stored = await page.evaluate(
    ({ dbKey, uiKey }) => ({
      db: localStorage.getItem(dbKey),
      ui: localStorage.getItem(uiKey),
    }),
    { dbKey: DB_KEY, uiKey: UI_KEY },
  );
  expect(stored.ui).toContain('release_seen');
  // Db が書かれていること自体も確認する（null だと下の assert が空振りする）
  expect(stored.db).toContain('"schema"');
  expect(stored.db).not.toContain('release_seen');
});

test('9. バナーの両ボタンのタップ標的が44px以上', async ({ page }) => {
  await setUiState(page, { release_seen: 0 });
  await page.reload();
  await expect(page.getByTestId('screen-record')).toBeVisible();
  await expect(page.getByTestId('whatsnew-banner')).toBeVisible();

  for (const testId of ['whatsnew-banner-open', 'whatsnew-banner-dismiss']) {
    const box = await page.getByTestId(testId).boundingBox();
    expect(box, `${testId} の boundingBox が取れない`).not.toBeNull();
    expect(Math.round(box.height), `${testId} の高さ`).toBeGreaterThanOrEqual(44);
  }
});

test('10. バナーが出ている状態で言語を切り替えても、既読が巻き戻らない', async ({ page }) => {
  await setUiState(page, { release_seen: 0 });
  await page.reload();
  await expect(page.getByTestId('screen-record')).toBeVisible();
  await expect(page.getByTestId('whatsnew-banner')).toBeVisible();

  // ★ 言語切替は App の描画クロージャごと部分木を作り直すので、WhatsNewBanner も
  //   作り直される（unseen prop は起動時に評価した値のまま動かない）
  await page.getByTestId('tab-settings').click();
  await page.getByTestId('settings-row-language').click();
  await page.getByTestId('lang-btn').and(page.locator('[data-lang="en"]')).click();

  // まだ既読にしていないので、作り直された後もバナーは残る
  await expect(page.getByTestId('whatsnew-banner')).toBeVisible();

  await page.getByTestId('whatsnew-banner-dismiss').click();
  await expect(page.getByTestId('whatsnew-banner')).toHaveCount(0);

  // ★ ここが `dismissed` の初期値を `release_seen` から求めている理由。`false` 固定に
  //   すると、この切り替えで作り直された瞬間に既読済みのバナーが復活する
  await page.getByTestId('lang-btn').and(page.locator('[data-lang="ja"]')).click();
  await expect(page.getByTestId('whatsnew-banner')).toHaveCount(0);
});

// ── 図（src/views/whatsnew.rs の `revealed` ラッチ / `on:error` / ダーク断り） ──
//
// ★ **11 と 14 は Service Worker を block する。** E2E サーバー（4173）は
//   `index.html` が SW を登録し、`sw.js` は `skipWaiting` + `clients.claim` で
//   初回ロード直後から同一オリジンの全 GET を仲介する。Playwright は
//   **SW が仲介したリクエストを `page.route()` でインターセプトできない**
//   （`node_modules/playwright-core/types/types.d.ts` の `page.route` の NOTE:
//   "will not intercept requests intercepted by Service Worker … We recommend
//   disabling Service Workers by setting `serviceWorkers` to `'block'`"。
//   microsoft/playwright#1090 も同じ制約を記録している）。
//   `e2e/manual.spec.mjs:528` と `e2e/pwa.spec.mjs:391` も同じ壁を踏んでいる
//   （`setOffline` が SW 発の fetch に効かない）。SW の activate との競走なので
//   block しないと flaky になる（chromium / Pixel 7）。

/**
 * 図を持つ項目が 1 つでもあれば、項目ごとに `<img>`（`whatsnew-fig`）か代替の
 * `<p>`（`whatsnew-fig-missing`）のどちらか 1 つが必ず DOM にある
 * （`<img>` は `revealed` と同時にマウントされ、`on:error` で `<p>` に**置き換わる**）。
 *
 * ★ skip 判定はこの和集合で行う。**`whatsnew-fig` 単独の `count()` は使わない** —
 *   404 で `<p>` に差し替わった図が「無い」扱いになり、本来 FAIL すべき状況
 *   （宣言した図が読めない）が skip という緑の結果に化ける
 *   （差し替え前に `count()` が走れば `naturalWidth` 0 で FAIL、差し替え後なら skip、
 *   とタイミング次第で結果が変わっていた）。
 */
function figOrFallback(page) {
  return page.getByTestId('whatsnew-fig').or(page.getByTestId('whatsnew-fig-missing'));
}

test.describe('11: 図はシートを開くまで取りに行かない', () => {
  test.use({ serviceWorkers: 'block' });

  test('起動直後は whatsnew/ へのリクエストが 0 件、開くと 1 件以上', async ({ page }) => {
    await setUiState(page, { release_seen: 0 });

    const requests = [];
    // ★ reload の前に listener を張る。後から張ると起動直後のリクエストを
    //   数え漏らす（reload はナビゲーションなので listener は張り直しになる）
    page.on('request', (req) => {
      if (req.url().includes('whatsnew/')) requests.push(req.url());
    });

    await page.reload();
    await expect(page.getByTestId('screen-record')).toBeVisible();
    await expect(page.getByTestId('whatsnew-fig')).toHaveCount(0);
    expect(requests.length, '開く前に whatsnew/ へのリクエストが飛んでいる').toBe(0);

    await page.getByTestId('whatsnew-banner-open').click();
    test.skip((await figOrFallback(page).count()) === 0, '図を持つ項目が無い');

    const figs = page.getByTestId('whatsnew-fig');
    await expect(figs.first()).toBeVisible();
    const srcs = await figs.evaluateAll((els) => els.map((el) => el.getAttribute('src')));
    for (const src of srcs) {
      expect(src.startsWith('/'), `src が絶対パス参照になっている: ${src}`).toBe(false);
      expect(src).toContain('whatsnew/ja/');
    }
    // ★ Chromium は SW 経由の fetch を二重に数えうるので toBe(1) にしない。
    //   正の側（開けば取りに行く）を見る
    expect(requests.length, '開いた後は whatsnew/ へのリクエストが 1 件以上').toBeGreaterThanOrEqual(1);
  });
});

test('12. 宣言寸法 = 実ファイル: img の naturalWidth/Height が width/height 属性と一致する', async ({
  page,
}) => {
  await setUiState(page, { release_seen: 0 });
  await page.reload();
  await expect(page.getByTestId('screen-record')).toBeVisible();

  await page.getByTestId('whatsnew-banner-open').click();
  test.skip((await figOrFallback(page).count()) === 0, '図を持つ項目が無い');

  // 読めなかった図があれば、以下の naturalWidth 検査を待たずここで明示的に落とす
  await expect(page.getByTestId('whatsnew-fig-missing')).toHaveCount(0);

  const figs = page.getByTestId('whatsnew-fig');
  const count = await figs.count();
  for (let i = 0; i < count; i++) {
    const img = figs.nth(i);
    await img.scrollIntoViewIfNeeded();
    await waitForImageSettled(img);

    const [naturalWidth, naturalHeight, width, height] = await Promise.all([
      img.evaluate((el) => el.naturalWidth),
      img.evaluate((el) => el.naturalHeight),
      img.evaluate((el) => Number(el.getAttribute('width'))),
      img.evaluate((el) => Number(el.getAttribute('height'))),
    ]);
    expect(naturalWidth, `${i} 枚目が読めていない`).toBeGreaterThan(0);
    expect(naturalWidth, `${i} 枚目の naturalWidth`).toBe(width);
    expect(naturalHeight, `${i} 枚目の naturalHeight`).toBe(height);
  }
});

test('13. SW のシェルに whatsnew/ が入っていない', async ({ page }) => {
  const sw = await (await page.request.get('./sw.js')).text();

  // まず置換自体が走っていること（プレースホルダが残っていたら以下の assert が空振りする）
  expect(sw).not.toContain('__BUILD_ID__');
  expect(sw).not.toContain('__SHELL__');
  // SHELL が空でないこと（空配列なら whatsnew/ が無いのは当たり前で、何も検証できていない）
  expect(sw).toContain('"./manifest.webmanifest"');

  expect(sw, 'バナーを開かない人のオフラインシェルに図を載せてはならない').not.toContain('whatsnew/');
});

test.describe('14: 図が取れないとき代替表示になる', () => {
  test.use({ serviceWorkers: 'block' });

  test('本文は読めて断りが出る（図・SW を経由しない）', async ({ page }) => {
    await setUiState(page, { release_seen: 0 });
    // ★ シートを開くまで図は fetch されないので、abort を先に張っても
    //   キャッシュ経由で偶然読める心配は無い（SW を block しているので route が届く）
    await page.route('**/whatsnew/**', (route) => route.abort());

    await page.reload();
    await expect(page.getByTestId('screen-record')).toBeVisible();

    await page.getByTestId('whatsnew-banner-open').click();
    test.skip((await figOrFallback(page).count()) === 0, '図を持つ項目が無い');

    // ★ 専用サーバーを kill する重い形（manual.spec.mjs:516-556）は使わない。
    //   図はシートを開くまで一切 fetch されないので、abort だけで足りる
    const missing = page.getByTestId('whatsnew-fig-missing');
    await expect(missing.first()).toBeVisible();
    await expect(page.getByTestId('whatsnew-fig')).toHaveCount(0);
    await expect(page.getByTestId('whatsnew-release').first().locator('li').first()).toBeVisible();
  });
});

test('14b. ダークの断りはダークのときだけ出る', async ({ page }) => {
  await setUiState(page, { release_seen: 0 });
  await page.reload();
  await expect(page.getByTestId('screen-record')).toBeVisible();
  await page.getByTestId('whatsnew-banner-open').click();

  const note = page.getByTestId('whatsnew-light-note');
  test.skip((await note.count()) === 0, '図を持つ項目が無い');

  await page.emulateMedia({ colorScheme: 'dark' });
  await expect(note).toBeVisible();

  await page.emulateMedia({ colorScheme: 'light' });
  await expect(note).toBeHidden();
});

test('15. 言語切替後にバナーを開き直すと図の src が en/ になる', async ({ page }) => {
  await setUiState(page, { release_seen: 0 });
  await page.reload();
  await expect(page.getByTestId('screen-record')).toBeVisible();

  // ★ 言語切替で WhatsNewBanner ごと作り直され、`revealed` は false に戻る
  //   （テスト 10 と同じ理由）。開き直さないと図がまだ DOM に無い
  await page.getByTestId('tab-settings').click();
  await page.getByTestId('settings-row-language').click();
  await page.getByTestId('lang-btn').and(page.locator('[data-lang="en"]')).click();

  await expect(page.getByTestId('whatsnew-banner')).toBeVisible();
  await page.getByTestId('whatsnew-banner-open').click();

  test.skip((await figOrFallback(page).count()) === 0, '図を持つ項目が無い');
  const figs = page.getByTestId('whatsnew-fig');
  await expect(figs.first()).toHaveAttribute('src', /whatsnew\/en\//);
});
