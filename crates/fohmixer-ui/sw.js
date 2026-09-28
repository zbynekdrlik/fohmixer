// fohmixer service worker (#17): NETWORK-ONLY, registered on the https origin
// only (index.html). It never stores or replays an answer: on the internet
// path everything sits behind Cloudflare Access, and a cached page or API
// answer could bypass or confuse the Access login; on the LAN the mixer must
// show the live state. Its one job is to make the page an installable PWA.
// The WebSocket is not a fetch and never passes through here. Pattern:
// airuleset cli_webterm_pwa.py.
self.addEventListener('install', function () {
    self.skipWaiting();
});
self.addEventListener('activate', function (event) {
    event.waitUntil(self.clients.claim());
});
self.addEventListener('fetch', function (event) {
    event.respondWith(fetch(event.request));
});
