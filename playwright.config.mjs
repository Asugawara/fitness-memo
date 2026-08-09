import { defineConfig, devices } from '@playwright/test';

// ★ Node 側とブラウザ側のタイムゾーンを**両方**揃える。片方だけでは意味がない。
//   calendar.spec.mjs の dateKey / fmtDate / fmtMonth / daysAgo は Node 側で日付を
//   組み立てており（実質全テストが依存）、ブラウザだけ timezoneId で JST にすると
//   host TZ が UTC の CI では両ゾーンが別の暦日になる時間帯で全部落ちる。逆に Node だけ
//   揃えてもアプリの Local::now() は動かない。
//
//   JST の開発機では完全な no-op で、CI や海外マシンでのみ効く。日本は DST が無いので
//   夏時間の縁も生じない。Node は実行時の process.env.TZ 変更を反映するので
//   （v16.2+。ESM の import 巻き上げでこの行は @playwright/test の評価後に走るが問題ない）、
//   spec 側の new Date() には確実に効く。
const TZ = process.env.E2E_TZ || 'Asia/Tokyo';
process.env.TZ = TZ;

const PORT = Number(process.env.PORT || 4173);
const BASE = normalizeBase(process.env.E2E_BASE || '/');
// dist ディレクトリの切り替え。release.sh は `trunk build --dist dist-release` の
// 成果物を `DIST_DIR=dist-release` で配信させることで、他ワーカーが並行して既定の
// dist/ に trunk build し続けていても、そこに引きずられずに検証できる
// (未設定時は "dist"。scripts/static-server.mjs 側の同名の仕組みを参照)
//
// ★ 同じ罠は `trunk serve` でも踏む。watch が走るたびに既定の dist/ を書き換えるので、
//   起動したまま E2E を回すと配信中の wasm が差し替わり、「screen-record が現れない」
//   型の失敗が実行ごとに 1〜2 件出る（アプリのバグではない）。E2E の前に必ず止めるか、
//   DIST_DIR を分けること。実測: trunk serve あり 35〜38 秒で 1〜2 件 fail /
//   なし 17 秒で 174 件 pass。
const DIST_DIR = process.env.DIST_DIR || 'dist';

function normalizeBase(base) {
  let b = base.startsWith('/') ? base : `/${base}`;
  if (!b.endsWith('/')) b += '/';
  return b;
}

export default defineConfig({
  testDir: './e2e',
  fullyParallel: true,
  reporter: 'list',
  use: {
    baseURL: `http://localhost:${PORT}${BASE}`,
    timezoneId: TZ,
  },
  // dist の内容は static-server.mjs がリクエストごとに読むので、古いビルドを掴んだまま
  // 使い回されないよう毎回起動する
  webServer: {
    command: 'node scripts/static-server.mjs',
    port: PORT,
    env: { PORT: String(PORT), E2E_BASE: BASE, DIST_DIR },
    reuseExistingServer: false,
  },
  projects: [
    // harness.spec.mjs は dist/ に依存せず、自前で static-server.mjs を固定ポートで
    // 起動する（e2e/harness.spec.mjs 参照）。他 project と並列実行するとポートが
    // 衝突するので、この project だけに限定し、他 project からは除外する
    { name: 'harness', testMatch: /harness\.spec\.mjs$/ },
    { name: 'chromium', testIgnore: /harness\.spec\.mjs$/, use: { ...devices['Desktop Chrome'] } },
    { name: 'iPhone 15 Pro', testIgnore: /harness\.spec\.mjs$/, use: { ...devices['iPhone 15 Pro'] } },
    { name: 'Pixel 7', testIgnore: /harness\.spec\.mjs$/, use: { ...devices['Pixel 7'] } },
  ],
});
