// A piece of the chat that most messages never need lives in its own file next to chat.js.
// The server serves a fixed list of files under `script-src 'self'`, so each piece is one
// classic script with a fixed name that sets one global, never an import() chunk.
export const bundles = {
  code: { file: "chat-code.js", global: "GrokbotCode" },
};

const loading = new Map();

export function loadedBundle(name) {
  return globalThis[bundles[name].global];
}

// One <script> per piece. A failed load forgets itself, so a later message retries.
export function loadBundle(name) {
  if (!loading.has(name)) loading.set(name, inject(bundles[name]).catch((error) => {
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
