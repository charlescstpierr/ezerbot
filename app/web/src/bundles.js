// A piece of the chat that most messages never need lives in its own file next to chat.js.
// The server serves a fixed list of files under `script-src 'self'`, so each piece is one
// classic script with a fixed name that sets one global, never an import() chunk.
export const bundles = {
  code: { file: "chat-code.js", global: "GrokbotCode" },
  mermaid: { file: "chat-mermaid.js", global: "GrokbotMermaid" },
  math: { file: "chat-math.js", global: "GrokbotMath" },
};

const loading = new Map();
const listeners = new Set();
let arrived = 0;

// For React's useSyncExternalStore: the count changes each time a piece arrives.
export const subscribeBundles = (listener) => {
  listeners.add(listener);
  return () => listeners.delete(listener);
};
export const arrivedBundles = () => arrived;

export function loadedBundle(name) {
  return globalThis[bundles[name].global];
}

// One <script> per piece. A failed load forgets itself, so a later message retries.
export function loadBundle(name) {
  if (!loading.has(name)) loading.set(name, inject(bundles[name]).then((bundle) => {
    arrived += 1;
    for (const listener of listeners) listener();
    return bundle;
  }, (error) => {
    loading.delete(name);
    throw error;
  }));
  return loading.get(name);
}

function inject({ file, global }) {
  return new Promise((resolve, reject) => {
    const script = document.createElement("script");
    const fail = () => {
      script.remove();
      reject(new Error(`/${file} n’a pas pu être chargé`));
    };
    script.src = `/${file}`;
    script.onload = () => (globalThis[global] ? resolve(globalThis[global]) : fail());
    script.onerror = fail;
    document.head.append(script);
  });
}
