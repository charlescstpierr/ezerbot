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
  // A same-origin frame reaches this page's objects, this bridge included, and the
  // handler cannot tell which frame spoke. A frame may keep a reference to the bridge
  // and call it after its element is gone, so once a frame that can reach this page
  // has been attached, the bridge stays refused until the page reloads. Every call
  // re-checks: live frames, frame elements in the page, and attachments recorded
  // since the last check. A sandboxed frame (opaque origin) cannot reach it.
  const framesOf = window.frames;
  const FRAME_TAGS = new Set(['IFRAME', 'FRAME', 'OBJECT', 'EMBED']);
  const opaque = element => element.tagName !== 'OBJECT' && element.tagName !== 'EMBED' &&
    element.hasAttribute?.('sandbox') &&
    !String(element.getAttribute('sandbox')).toLowerCase().split(/\s+/).includes('allow-same-origin');
  let revoked = false;
  const inspect = node => {
    if (!node || node.nodeType !== 1) return;
    if (FRAME_TAGS.has(node.tagName) && !opaque(node)) revoked = true;
    for (const inner of node.querySelectorAll?.('iframe,frame,object,embed') || []) {
      if (!opaque(inner)) revoked = true;
    }
  };
  const record = records => {
    for (const change of records) {
      if (change.type === 'attributes') inspect(change.target);
      for (const node of change.addedNodes || []) inspect(node);
    }
  };
  const Observer = window.MutationObserver;
  const observer = typeof Observer === 'function' ? new Observer(record) : null;
  observer?.observe(document, {childList: true, subtree: true, attributes: true, attributeFilter: ['sandbox']});
  const authorized = () => {
    if (observer) record(observer.takeRecords());
    for (let index = 0; index < framesOf.length; index++) {
      try { if (framesOf[index].document) revoked = true; } catch (_) {}
    }
    for (const element of document.querySelectorAll?.('iframe,frame,object,embed') || []) inspect(element);
    return !revoked;
  };
  const post = body => new Promise((resolve, reject) => {
    if (!authorized()) { reject(new Error('Commande refusée : un cadre de la page a pu accéder au pont. Rechargez la page.')); return; }
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
