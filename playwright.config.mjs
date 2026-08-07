import { defineConfig, devices } from '@playwright/test';

const PORT = Number(process.env.PORT || 4173);
const BASE = normalizeBase(process.env.E2E_BASE || '/');

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
  },
  // dist の内容は static-server.mjs がリクエストごとに読むので、古いビルドを掴んだまま
  // 使い回されないよう毎回起動する
  webServer: {
    command: 'node scripts/static-server.mjs',
    port: PORT,
    env: { PORT: String(PORT), E2E_BASE: BASE },
    reuseExistingServer: false,
  },
  projects: [
    { name: 'chromium', use: { ...devices['Desktop Chrome'] } },
    { name: 'iPhone 15 Pro', use: { ...devices['iPhone 15 Pro'] } },
    { name: 'Pixel 7', use: { ...devices['Pixel 7'] } },
  ],
});
