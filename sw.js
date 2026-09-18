/* SpeakLab service worker —— 让页面能离线打开、能「添加到主屏幕」。
 *
 * 设计上有两条硬规矩，都是踩过坑才定下来的：
 *
 * 1. **绝对不缓存 /api/ 下的任何东西。**
 *    后端有云游戏串流（WebSocket + 一路 JPEG 帧）和练习记录读写。
 *    如果被 SW 拦下来返回旧数据，用户会看到过期画面，而且报错极难查。
 *    所以带 /api/ 的请求直接放行，连碰都不碰。
 *
 * 2. **导航请求走「网络优先」。**
 *    这是给开发用的仓库——改了 index.html 就应该立刻看到。
 *    缓存优先的话，每次改完都要手动清 SW，非常折磨。
 *    网络不通时才回退到缓存里的 index.html（这才是离线可用的意义）。
 *
 * 静态资源（css/js/图片/图标）反过来走「缓存优先」，因为它们改动少，
 * 从缓存拿能省掉每次首屏的几百毫秒。
 */

const VERSION = 'v1';
const CACHE = 'speaklab-' + VERSION;

// 首屏必需的东西。这里列全，装完就能直接离线打开。
const PRECACHE = [
  './',
  './index.html',
  './manifest.json',
  './cloud/cloud.css',
  './cloud/cloud.js',
  './cloud/posters/genshin.webp',
  './cloud/posters/starrail.webp',
  './cloud/posters/mingchao.jpg',
  './dino/dino.css',
  './dino/dino.js',
  './dino/offline-sprite-1x.png',
  './dino/offline-sprite-2x.png',
  './icons/icon-192.png',
  './icons/icon-512.png',
  './icons/icon-maskable-192.png',
  './icons/icon-maskable-512.png',
  './icons/apple-touch-icon.png',
  './icons/favicon-32.png',
];

self.addEventListener('install', (e) => {
  e.waitUntil((async () => {
    const cache = await caches.open(CACHE);
    // 逐个 add，一个失败不影响其余；addAll 是全有全无，太脆。
    const results = await Promise.allSettled(
      PRECACHE.map((url) => cache.add(new Request(url, { cache: 'reload' }))),
    );
    const failed = results
      .map((r, i) => (r.status === 'rejected' ? PRECACHE[i] : null))
      .filter(Boolean);
    if (failed.length) {
      console.warn('[sw] 这些没缓存上（不影响离线主流程）:', failed);
    }
    await self.skipWaiting();
  })());
});

self.addEventListener('activate', (e) => {
  e.waitUntil((async () => {
    // 清掉旧版本缓存
    const names = await caches.keys();
    await Promise.all(
      names.filter((n) => n.startsWith('speaklab-') && n !== CACHE).map((n) => caches.delete(n)),
    );
    await self.clients.claim();
  })());
});

const isApi = (url) => url.pathname.startsWith('/api/');

self.addEventListener('fetch', (e) => {
  const req = e.request;
  if (req.method !== 'GET') return;

  const url = new URL(req.url);

  // 跨域资源不碰（页面本身零第三方依赖，出现跨域多半是用户自己加的）
  if (url.origin !== self.location.origin) return;

  // 规矩 1：API 一律放行，不缓存
  if (isApi(url)) return;

  // 规矩 2：导航请求网络优先，失败回退缓存
  if (req.mode === 'navigate') {
    e.respondWith((async () => {
      try {
        const fresh = await fetch(req);
        const cache = await caches.open(CACHE);
        cache.put('./index.html', fresh.clone());
        return fresh;
      } catch {
        const cache = await caches.open(CACHE);
        return (await cache.match('./index.html'))
          || (await cache.match('./'))
          || new Response('离线，且没有缓存副本。', {
            status: 503,
            headers: { 'content-type': 'text/plain; charset=utf-8' },
          });
      }
    })());
    return;
  }

  // 其余静态资源：缓存优先，后台顺便更新
  e.respondWith((async () => {
    const cache = await caches.open(CACHE);
    const hit = await cache.match(req);
    if (hit) {
      //  stale-while-revalidate
      fetch(req).then((res) => {
        if (res && res.ok) cache.put(req, res.clone());
      }).catch(() => {});
      return hit;
    }
    try {
      const res = await fetch(req);
      if (res && res.ok && res.type === 'basic') cache.put(req, res.clone());
      return res;
    } catch (err) {
      throw err;
    }
  })());
});
