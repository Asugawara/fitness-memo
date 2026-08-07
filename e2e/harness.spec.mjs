import { test, expect } from '@playwright/test';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

// worker-a の src/ がまだ無く trunk build が通らないため、実 dist/ の代わりに
// このファイル専用の fixture ディレクトリを static-server.mjs に向けて起動する。
// アプリの内容に一切依存せず、静的配信ロジックそのもの（MIME・サブパス・404）だけを検証する。
// 固定ポートを2つ常駐させるので、同一ファイル内・他プロジェクトとの並列実行は禁止する
// (playwright.config.mjs 側で projects を排他にしている)。
test.describe.configure({ mode: 'serial' });

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = join(HERE, '..');
const FIXTURE_DIST = join(HERE, 'fixtures', 'dist');

const ROOT_PORT = 4180; // E2E_BASE 未設定（"/"）を模した固定ポート
const SUBPATH_PORT = 4181; // E2E_BASE=/fitness-memo/ を模した固定ポート

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
  let rootProc;
  let subpathProc;

  test.beforeAll(async () => {
    rootProc = startServer({ port: ROOT_PORT, base: '/' });
    subpathProc = startServer({ port: SUBPATH_PORT, base: '/fitness-memo/' });
    await Promise.all([
      waitForReady(ROOT_PORT, rootProc),
      waitForReady(SUBPATH_PORT, subpathProc),
    ]);
  });

  test.afterAll(() => {
    rootProc?.kill();
    subpathProc?.kill();
  });

  test('1. index.html が 200 で返る', async ({ request }) => {
    const res = await request.get(`http://localhost:${ROOT_PORT}/`);
    expect(res.status()).toBe(200);
    expect(res.headers()['content-type']).toContain('text/html');
    expect(await res.text()).toContain('harness-fixture-body');
  });

  test('2. .wasm に content-type application/wasm が付く', async ({ request }) => {
    const res = await request.get(`http://localhost:${ROOT_PORT}/app.wasm`);
    expect(res.status()).toBe(200);
    expect(res.headers()['content-type']).toBe('application/wasm');
  });

  test('3. ディレクトリURL（トレイリングスラッシュ）で index.html が返る', async ({ request }) => {
    const byDirectory = await request.get(`http://localhost:${ROOT_PORT}/`);
    const byFilename = await request.get(`http://localhost:${ROOT_PORT}/index.html`);
    expect(byDirectory.status()).toBe(200);
    expect(await byDirectory.text()).toBe(await byFilename.text());
  });

  test('4. 存在しないパスは 404 になる', async ({ request }) => {
    const res = await request.get(`http://localhost:${ROOT_PORT}/does-not-exist.txt`);
    expect(res.status()).toBe(404);
  });

  test('5. E2E_BASE=/fitness-memo/ ではサブパス配下で 1・3 が成立し、ルート直下は 404 になる', async ({ request }) => {
    const html = await request.get(`http://localhost:${SUBPATH_PORT}/fitness-memo/`);
    expect(html.status()).toBe(200);
    expect(await html.text()).toContain('harness-fixture-body');

    const wasm = await request.get(`http://localhost:${SUBPATH_PORT}/fitness-memo/app.wasm`);
    expect(wasm.status()).toBe(200);
    expect(wasm.headers()['content-type']).toBe('application/wasm');

    const outsideBase = await request.get(`http://localhost:${SUBPATH_PORT}/`);
    expect(outsideBase.status()).toBe(404);
  });

  test('補足: トレイリングスラッシュ無しのベースパスは 301 で補完される', async ({ request }) => {
    const res = await request.get(`http://localhost:${SUBPATH_PORT}/fitness-memo`, { maxRedirects: 0 });
    expect(res.status()).toBe(301);
    expect(res.headers()['location']).toBe('/fitness-memo/');
  });
});
