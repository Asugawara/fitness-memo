import { test, expect } from '@playwright/test';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

// worker-a の src/ がまだ無く trunk build が通らないため、実 dist/ の代わりに
// このファイル専用の fixture ディレクトリを static-server.mjs に向けて起動する。
// アプリの内容に一切依存せず、静的配信ロジックそのもの（MIME・サブパス・404）だけを検証する。
// 固定ポートを2つ常駐させるので、同一ファイル内・他プロジェクトとの並列実行は禁止する
// (playwright.config.mjs 側で projects を排他にしている)。
//
// ★ serial モードは「同じ describe インスタンス内」の直列化しか保証しない。
// --repeat-each を付けると同じ describe が複数インスタンス化され、それぞれの
// beforeAll が別ワーカーで同時に走ることがある。ポートを固定値のままにすると
// リピート間でポートが衝突する（実際に --repeat-each=2 を全 project で回して踏んだ）。
// worker ごとに一意な workerInfo.parallelIndex からポートを導出して衝突を避ける。
test.describe.configure({ mode: 'serial' });

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = join(HERE, '..');
const FIXTURE_DIST = join(HERE, 'fixtures', 'dist');

function startServer({ port, base }) {
  const proc = spawn(process.execPath, ['scripts/static-server.mjs'], {
    cwd: REPO_ROOT,
    env: { ...process.env, PORT: String(port), E2E_BASE: base, STATIC_ROOT: FIXTURE_DIST },
    stdio: ['ignore', 'ignore', 'pipe'],
  });
  let stderr = '';
  proc.stderr.on('data', chunk => { stderr += chunk; });
  proc.getStderr = () => stderr;
  return proc;
}

async function waitForReady(port, proc, timeoutMs = 5000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (proc.exitCode !== null) {
      throw new Error(`static-server (port ${port}) が起動前に終了しました:\n${proc.getStderr()}`);
    }
    try {
      await fetch(`http://localhost:${port}/`);
      return;
    } catch {
      await new Promise(r => setTimeout(r, 100));
    }
  }
  throw new Error(`static-server (port ${port}) が ${timeoutMs}ms 以内に応答しませんでした:\n${proc.getStderr()}`);
}

test.describe('static-server ハーネス（アプリ非依存 / fixture dist）', () => {
  let rootPort;
  let subpathPort;
  let rootProc;
  let subpathProc;

  test.beforeAll(async ({}, workerInfo) => {
    // parallelIndex は「今アクティブなワーカー」の間で常に一意なので、他ワーカーや
    // 他の repeat インスタンスと同時に走っても衝突しない
    rootPort = 4180 + workerInfo.parallelIndex * 2;
    subpathPort = rootPort + 1;
    rootProc = startServer({ port: rootPort, base: '/' });
    subpathProc = startServer({ port: subpathPort, base: '/fitness-memo/' });
    await Promise.all([
      waitForReady(rootPort, rootProc),
      waitForReady(subpathPort, subpathProc),
    ]);
  });

  test.afterAll(() => {
    rootProc?.kill();
    subpathProc?.kill();
  });

  test('1. index.html が 200 で返る', async ({ request }) => {
    const res = await request.get(`http://localhost:${rootPort}/`);
    expect(res.status()).toBe(200);
    expect(res.headers()['content-type']).toContain('text/html');
    expect(await res.text()).toContain('harness-fixture-body');
  });

  test('2. .wasm に content-type application/wasm が付く', async ({ request }) => {
    const res = await request.get(`http://localhost:${rootPort}/app.wasm`);
    expect(res.status()).toBe(200);
    expect(res.headers()['content-type']).toBe('application/wasm');
  });

  test('3. ディレクトリURL（トレイリングスラッシュ）で index.html が返る', async ({ request }) => {
    const byDirectory = await request.get(`http://localhost:${rootPort}/`);
    const byFilename = await request.get(`http://localhost:${rootPort}/index.html`);
    expect(byDirectory.status()).toBe(200);
    expect(await byDirectory.text()).toBe(await byFilename.text());
  });

  test('4. 存在しないパスは 404 になる', async ({ request }) => {
    const res = await request.get(`http://localhost:${rootPort}/does-not-exist.txt`);
    expect(res.status()).toBe(404);
  });

  test('5. E2E_BASE=/fitness-memo/ ではサブパス配下で 1・3 が成立し、ルート直下は 404 になる', async ({ request }) => {
    const html = await request.get(`http://localhost:${subpathPort}/fitness-memo/`);
    expect(html.status()).toBe(200);
    expect(await html.text()).toContain('harness-fixture-body');

    const wasm = await request.get(`http://localhost:${subpathPort}/fitness-memo/app.wasm`);
    expect(wasm.status()).toBe(200);
    expect(wasm.headers()['content-type']).toBe('application/wasm');

    const outsideBase = await request.get(`http://localhost:${subpathPort}/`);
    expect(outsideBase.status()).toBe(404);
  });

  test('補足: トレイリングスラッシュ無しのベースパスは 301 で補完される', async ({ request }) => {
    const res = await request.get(`http://localhost:${subpathPort}/fitness-memo`, { maxRedirects: 0 });
    expect(res.status()).toBe(301);
    expect(res.headers()['location']).toBe('/fitness-memo/');
  });
});
