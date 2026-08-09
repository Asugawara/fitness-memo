const CACHE = `fitness-memo-a0f87bf88011fbc4`;
const SHELL = ["./fitness-memo-671fb15769f8600e.js","./fitness-memo-671fb15769f8600e_bg.wasm","./icons/icon-192.png","./icons/icon-32.png","./icons/icon-512.png","./icons/icon-maskable-512.png","./index.html","./manifest.webmanifest","./styles-f2216b35228cd0fd.css",];

self.addEventListener('install', e => e.waitUntil((async () => {
  const c = await caches.open(CACHE);
  await c.addAll(SHELL.map(u => new Request(u, { cache: 'reload' })));  // HTTP キャッシュを迂回
  await self.skipWaiting();
})()));

self.addEventListener('activate', e => e.waitUntil((async () => {
  // 自分の prefix だけ消す。caches.keys() はオリジン全体（asugawara.github.io）を返すので、
  // 無差別に消すと同じアカウントの他プロジェクトの Pages サイトのキャッシュを壊す
  for (const k of await caches.keys()) {
    if (k.startsWith('fitness-memo-') && k !== CACHE) await caches.delete(k);
  }
  await self.clients.claim();
})()));

self.addEventListener('fetch', e => {
  const url = new URL(e.request.url);
  if (e.request.method !== 'GET' || url.origin !== self.location.origin) return;

  // 自分の CACHE に限定して match する（他世代・他サイトのキャッシュを見ない）。
  // ミスはネットワークへ素通しし、応答を cache.put しない —— GitHub Pages は
  // max-age=600 を返すため、格納すると v2 キャッシュに HTTP キャッシュ由来の
  // v1 index.html が固定化され、次のデプロイまで白画面になる。
  // 非ナビゲーションに index.html をフォールバックしないのは、trunk の SRI が
  // 有効で js/wasm に text/html を返すと integrity 不一致で即死するため。
  const fromCache = key => caches.open(CACHE).then(c => c.match(key)).then(r => r || fetch(e.request));

  // "/fitness-memo/"（ディレクトリURL）も "?sw=off" もここで 1 つの key に収束する
  e.respondWith(e.request.mode === 'navigate' ? fromCache('./index.html') : fromCache(e.request));
});
