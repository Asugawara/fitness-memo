import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { extname, join, normalize, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

// STATIC_ROOT はテストハーネス(e2e/harness.spec.mjs)が絶対パスの fixture
// ディレクトリに差し替えるためのフック。最優先。
// DIST_DIR はリポジトリルート直下の「dist という名前」を切り替えるフック。
// 例えば release.sh が `trunk build --dist dist-release` で本番相当のビルドを
// 既定の dist/ とは別の場所に出力し、`DIST_DIR=dist-release` で配信させれば、
// 他ワーカーが並行して既定の dist/ に trunk build し続けていても影響を受けない
// (未設定時は "dist")
const REPO_ROOT = join(fileURLToPath(new URL('.', import.meta.url)), '..');
const ROOT = normalize(process.env.STATIC_ROOT || join(REPO_ROOT, process.env.DIST_DIR || 'dist'));
const PORT = Number(process.env.PORT || 4173);
const BASE = normalizeBase(process.env.E2E_BASE || '/');

function normalizeBase(base) {
  let b = base.startsWith('/') ? base : `/${base}`;
  if (!b.endsWith('/')) b += '/';
  return b;
}

const MIME = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.mjs': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  '.webmanifest': 'application/manifest+json; charset=utf-8',
  '.wasm': 'application/wasm',
  '.png': 'image/png',
  '.svg': 'image/svg+xml',
  '.ico': 'image/x-icon',
};

const server = createServer(async (req, res) => {
  if (req.method !== 'GET') {
    res.writeHead(405).end();
    return;
  }

  const { pathname } = new URL(req.url, 'http://localhost');

  // GitHub Pages はディレクトリを末尾スラッシュへ 301 する。sw.js の scope 判定はこれに依存する
  if (pathname === BASE.slice(0, -1)) {
    res.writeHead(301, { Location: BASE }).end();
    return;
  }
  if (!pathname.startsWith(BASE)) {
    res.writeHead(404).end('Not Found');
    return;
  }

  let rel = decodeURIComponent(pathname.slice(BASE.length));
  if (rel === '' || rel.endsWith('/')) rel += 'index.html';

  const filePath = normalize(join(ROOT, rel));
  if (filePath !== ROOT && !filePath.startsWith(ROOT + sep)) {
    res.writeHead(403).end('Forbidden');
    return;
  }

  try {
    const data = await readFile(filePath);
    res.writeHead(200, { 'Content-Type': MIME[extname(filePath)] || 'application/octet-stream' });
    res.end(data);
  } catch {
    res.writeHead(404).end('Not Found');
  }
});

server.listen(PORT, () => {
  console.log(`static-server: http://localhost:${PORT}${BASE}`);
});
