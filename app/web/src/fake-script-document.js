// The part of `document` that src/bundles.js touches: it appends <script> tags to <head>.
// `arrive` plays a script that ran and set its global, `fail` one the network refused.
export function installScriptDocument() {
  const appended = [];
  globalThis.document = {
    createElement: (tagName) => {
      const element = { tagName, removed: false, remove: () => { element.removed = true; } };
      return element;
    },
    head: { append: (element) => appended.push(element) },
  };
  return {
    appended,
    live: (src) => appended.filter((script) => script.src === src && !script.removed),
    arrive(script, global, value) {
      globalThis[global] = value;
      script.onload();
    },
    fail: (script) => script.onerror(),
  };
}
