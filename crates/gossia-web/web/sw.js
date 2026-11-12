// The service worker of the web version (DESIGN.md, The web version on
// GitHub Pages, 17c): it keeps the page, the module and the icons in a cache
// named after the build, so that Gossia starts without a connection, and a
// new build is fetched in the background and used on the next start.
// cargo xtask web writes the build and the list of files in place of the
// two marks below.
'use strict';

const BUILD = '__GOSSIA_BUILD__';
const FILES = __GOSSIA_FILES__;
const CACHE = `gossia-${BUILD}`;

// A new build: its files, all of them fresh from the server (not from the
// browser's HTTP cache, which may still hold the last build's), before it
// waits to be used.
self.addEventListener('install', (event) => {
  event.waitUntil((async () => {
    const cache = await caches.open(CACHE);
    await cache.addAll(FILES.map((file) => new Request(file, { cache: 'no-cache' })));
  })());
});

// In use: the caches of other builds go, and the pages open are served from
// this one.
self.addEventListener('activate', (event) => {
  event.waitUntil((async () => {
    for (const key of await caches.keys()) {
      if (key.startsWith('gossia-') && key !== CACHE) await caches.delete(key);
    }
    await self.clients.claim();
  })());
});

// The page asks for the waiting build to be used: at once (Restart now), or
// when Gossia starts and no other window of it is open, since an open one
// keeps running the build it started with.
self.addEventListener('message', (event) => {
  const type = event.data && event.data.type;
  if (type === 'activate') {
    self.skipWaiting();
  } else if (type === 'activate-if-alone') {
    event.waitUntil((async () => {
      const windows = await self.clients.matchAll({ type: 'window', includeUncontrolled: true });
      if (windows.length <= 1) self.skipWaiting();
      else if (event.source) event.source.postMessage({ type: 'not-alone' });
    })());
  }
});

// The site's own files from the cache (a page's address with a query is the
// page), anything else from the network: data source requests go to other
// sites and are never cached.
self.addEventListener('fetch', (event) => {
  const request = event.request;
  if (request.method !== 'GET' || new URL(request.url).origin !== self.location.origin) return;
  event.respondWith((async () => {
    const cache = await caches.open(CACHE);
    const hit = await cache.match(request, { ignoreSearch: true });
    return hit || fetch(request);
  })());
});
