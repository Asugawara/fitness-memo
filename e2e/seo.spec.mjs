import { test, expect } from '@playwright/test';

// 検索エンジンと SNS のスクレイパだけが読む head のメタデータを固定する（adr/seo/crawler-metadata-and-hardcoded-origin.md）。
//
// ★ canonical / og:url / og:image は**本番 URL のハードコード**なので、期待値は
//   環境非依存の定数になる。dist（E2E_BASE=/）でも dist-release（E2E_BASE=/fitness-memo/）
//   でも同じ assert が通るのはそのため。相対 URL に書き戻すと X / Slack / LINE で
//   カードが出なくなるので、「絶対 URL であること」をここで不変条件にする。
const SITE = 'https://asugawara.github.io/fitness-memo/';

/** <meta name="..."> / <meta property="..."> の content を取る。 */
function metaContent(page, selector) {
  return page.locator(selector).getAttribute('content');
}

test('description と canonical と OGP と Twitter Card が揃っている', async ({ page }) => {
  await page.goto('./');

  // description は検索結果のスニペットに出る。<title> は fitness-memo のままなので
  // （アプリ名 4 点セットを崩さないため）、日本語のキーワードはここが主な置き場になる
  const description = await metaContent(page, 'meta[name="description"]');
  expect(description).toContain('筋トレ');
  expect(description.length).toBeGreaterThan(50);
  // 日本語のスニペットはモバイルで全角 50〜60 字ほどで切られる。全部は出ない前提だが、
  // 冒頭 1 文で「何のアプリか」が完結する長さに収める（上限が無いと際限なく伸びる）
  expect(description.length).toBeLessThan(120);

  // ★ 絶対 URL であることが本題。相対に書き戻すとスクレイパが解決できない
  await expect(page.locator('link[rel="canonical"]')).toHaveAttribute('href', SITE);
  expect(await metaContent(page, 'meta[property="og:url"]')).toBe(SITE);
  expect(await metaContent(page, 'meta[property="og:image"]')).toBe(`${SITE}og.png`);

  expect(await metaContent(page, 'meta[property="og:type"]')).toBe('website');
  expect(await metaContent(page, 'meta[property="og:locale"]')).toBe('ja_JP');
  expect(await metaContent(page, 'meta[property="og:site_name"]')).toBe('fitness-memo');

  // OGP 画像はアイコンだけで情報量が無いので、「何のアプリか」を伝えるのは
  // カードの見出し文＝ og:title だけになる。日本語の説明が入っていること
  const ogTitle = await metaContent(page, 'meta[property="og:title"]');
  expect(ogTitle).toContain('fitness-memo');
  expect(ogTitle).toContain('筋トレ');

  // og:description は「何のアプリか」ではなく中身を説明する担当（それは og:title が持つ）。
  // Slack / LINE / Discord / Facebook のカードは長い説明を途中で切るので、文が途切れない
  // 長さに収まっていることを見る（X は 2023 年以降そもそも説明文を描画しない）
  const ogDescription = await metaContent(page, 'meta[property="og:description"]');
  expect(ogDescription.length).toBeGreaterThan(30);
  expect(ogDescription.length).toBeLessThan(120);

  // summary_large_image でないと 1200x630 が小さな正方形にトリミングされる
  expect(await metaContent(page, 'meta[name="twitter:card"]')).toBe('summary_large_image');

  // 4 点セットは据え置き（e2e/pwa.spec.mjs が本体を握っている。ここは念のため）
  await expect(page).toHaveTitle('fitness-memo');
});

test('og:image が実在し、宣言した寸法と実ファイルが一致する', async ({ page }) => {
  await page.goto('./');

  const res = await page.request.get('./og.png');
  expect(res.ok(), 'og.png が配信物に載っていない（index.html の copy-file 宣言を確認）').toBeTruthy();
  expect(res.headers()['content-type']).toContain('image/png');

  // ★ PNG の IHDR を直に読む。meta の宣言だけ直して画像を差し替え忘れる（逆も）事故を止める。
  //   PNG は 8B シグネチャ + 4B 長さ + 4B "IHDR" のあとに width / height が並ぶ
  const buf = await res.body();
  const width = buf.readUInt32BE(16);
  const height = buf.readUInt32BE(20);
  expect(width).toBe(Number(await metaContent(page, 'meta[property="og:image:width"]')));
  expect(height).toBe(Number(await metaContent(page, 'meta[property="og:image:height"]')));
  // OGP のカードが横長で出る比率。ここが崩れると正方形にトリミングされる
  expect({ width, height }).toEqual({ width: 1200, height: 630 });
});

test('JSON-LD が JSON として読め、url が canonical と一致する', async ({ page }) => {
  await page.goto('./');

  const raw = await page.locator('script[type="application/ld+json"]').textContent();
  // JSON.parse に失敗する = 構造化データとして丸ごと無視されている状態。
  // 目視では気づけないので機械で見る
  const ld = JSON.parse(raw);

  expect(ld['@context']).toBe('https://schema.org');
  expect(ld['@type']).toBe('WebApplication');
  expect(ld.url, 'JSON-LD の url が canonical とズレると別ページ扱いになる').toBe(SITE);
  expect(ld.name).toBe('fitness-memo');
  expect(ld.alternateName).toBe('筋トレメモ');
  expect(ld.inLanguage).toBe('ja');
});

// ★ このファイルで一番効くテスト。scripts/stamp-sw.sh の除外は find の 1 語なので
//   リファクタで消えやすく、消えても**挙動は正常なまま**（クローラ専用の画像が
//   全ユーザーのオフラインシェルに載って太るだけ）で誰も気づかない。
test('og.png は SW のオフラインシェルに入っていない', async ({ page }) => {
  await page.goto('./');
  const sw = await (await page.request.get('./sw.js')).text();

  // まず置換自体が走っていること（プレースホルダが残っていたら以下の assert が空振りする）
  expect(sw).not.toContain('__BUILD_ID__');
  expect(sw).not.toContain('__SHELL__');
  // SHELL が空でないこと（空配列なら og.png が無いのは当たり前で、何も検証できていない）
  expect(sw).toContain('"./manifest.webmanifest"');

  expect(sw, 'クローラ専用の og.png を precache すると全ユーザーが無駄に再DL する').not.toContain(
    'og.png',
  );
});

test('JS 無効時のフォールバック文が noscript に入っている', async ({ page }) => {
  await page.goto('./');

  // scripting 有効な文書では noscript の中身は要素に parse されず生テキストになるので、
  // innerText ではなく textContent を見る（display:none なので innerText は空）
  const fallback = await page.locator('noscript').textContent();
  expect(fallback).toContain('筋トレメモ');
  expect(fallback).toContain('fitness-memo');
});
