// fohmixer service worker (#17): NETWORK-ONLY, registered on the https origin
// only (index.html). It never stores or replays an answer: on the internet
// path everything sits behind Cloudflare Access, and a cached page or API
// answer could bypass or confuse the Access login; on the LAN the mixer must
// show the live state. Its one job is to make the page an installable PWA.
// The WebSocket is not a fetch and never passes through here. Pattern:
// airuleset cli_webterm_pwa.py.
//
// Only this origin's requests pass through it (#90). Another origin's (the
// AbleSet tab's probe image) stays the page's own: the worker's fetch of it
// would fall under the worker's own policy, whose connect-src is 'self', and
// fail before it left the browser, while the page's img-src allows it.
self.addEventListener('install', function () {
    self.skipWaiting();
});
self.addEventListener('activate', function (event) {
    event.waitUntil(self.clients.claim());
});
self.addEventListener('fetch', function (event) {
    if (new URL(event.request.url).origin !== self.location.origin) {
        return;
    }
    event.respondWith(fetch(event.request));
});
