(() => {
  'use strict';
  const [backend, resourceOrigin, launch] = __EZERBOT_PARAMETERS__;
  const URLClass = window.URL, RequestClass = window.Request, HeadersClass = window.Headers;
  const localOrigin = new URLClass(backend).origin;
  const isLocal = url => url.origin === localOrigin && url.protocol === 'http:' &&
    url.hostname === '127.0.0.1' && !url.username && !url.password;
  if (!isLocal(new URLClass(window.location.href))) return;
  const originalFetch = window.fetch.bind(window);
  window.fetch = function(input, init) {
    try {
      const request = new RequestClass(input, init);
      const url = new URLClass(request.url);
      const headers = new HeadersClass(request.headers);
      headers.delete('X-Ezerbot-Launch');
      if (isLocal(url)) {
        headers.set('X-Ezerbot-Launch', launch);
        return originalFetch(new RequestClass(request, {headers, redirect: 'error', credentials: 'omit'}));
      }
      return originalFetch(new RequestClass(request, {headers}));
    } catch (_) { return Promise.reject(new TypeError('Request unavailable.')); }
  };
  const resourceURL = raw => {
    try {
      const url = new URLClass(raw, document.baseURI);
      if (isLocal(url) && url.pathname.startsWith('/api/')) {
        return resourceOrigin + url.pathname + url.search + url.hash;
      }
    } catch (_) {}
    return raw;
  };
  Object.defineProperty(window, 'EzerbotLocal', {
    value: Object.freeze({resourceURL}), writable: false, configurable: false,
  });
  Object.defineProperty(window, 'EzerbotShell', {
    value: Object.freeze({
      platform: 'linux',
      unavailablePanels: Object.freeze([
        'voice', 'voice-conversation', 'voice-call', 'meetings', 'screen', 'demonstrations', 'floating',
      ]),
    }),
    writable: false, configurable: false,
  });
})();
