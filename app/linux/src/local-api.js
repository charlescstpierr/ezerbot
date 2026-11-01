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
  const ipc = window.ipc;
  const pending = new Map();
  let sequence = 0;
  const post = body => new Promise((resolve, reject) => {
    const id = ++sequence;
    pending.set(id, {resolve, reject});
    try { ipc.postMessage(JSON.stringify({launch, id, body})); }
    catch (error) { pending.delete(id); reject(error); }
  });
  const settle = (id, value, error) => {
    const call = pending.get(id);
    if (!call) return;
    pending.delete(id);
    if (error) call.reject(new Error(error)); else call.resolve(value);
  };
  const openConversation = (request, id) => Promise.resolve()
    .then(() => window.EzerbotFloatingMain.openConversation(id))
    .catch(() => ({opened: false}))
    .then(result => {
      post({
        command: 'conversationOpened', request, opened: result?.opened === true,
        reason: typeof result?.reason === 'string' ? result.reason : '',
      }).catch(() => {});
    });
  Object.defineProperty(window, 'EzerbotShell', {
    value: Object.freeze({
      platform: 'linux',
      unavailablePanels: Object.freeze([
        'voice', 'voice-conversation', 'voice-call', 'meetings', 'screen', 'demonstrations',
      ]),
      floating: Object.freeze({postMessage: post}),
      settle,
      openConversation,
    }),
    writable: false, configurable: false,
  });
})();
