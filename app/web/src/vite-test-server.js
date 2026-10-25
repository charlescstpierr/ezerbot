import { mkdtemp, rm } from "node:fs/promises";
import { rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";

// The one Vite config of the web tests. `node --test app/web/src/*.test.js` runs every
// file in its own process, at the same time, and each file starts a Vite server to load
// the JSX it renders. Servers that look alike must not share anything on disk or on a port:
//  - the dependency cache. Vite optimizes dependencies at startup and swaps `deps_temp_*`
//    into `deps`; two servers in the same cacheDir race on that rename (ENOENT/ENOTEMPTY),
//    or leave a server waiting for ever. Each server gets a cacheDir of its own.
//  - the HMR WebSocket. In middleware mode Vite still listens on 24678. Nothing here
//    hot-reloads, so the WebSocket server is off, and so is the file watcher.
// `close()` also removes the cacheDir and may be called any number of times. If a file
// fails before its `after(() => vite.close())` runs, the cacheDir is still removed on exit.
export async function createTestServer({ root = fileURLToPath(new URL("..", import.meta.url)), plugins = [], ...options } = {}) {
  const cacheDir = await mkdtemp(join(tmpdir(), "ezerbot-vite-test-"));
  const removeAtExit = () => rmSync(cacheDir, { recursive: true, force: true });
  process.once("exit", removeAtExit);
  let vite;
  try {
    vite = await createServer({
      root,
      configFile: false,
      cacheDir,
      plugins,
      server: { middlewareMode: true, hmr: false, ws: false, watch: null },
      logLevel: "error",
      optimizeDeps: { noDiscovery: true },
      ...options,
    });
  } catch (error) {
    process.removeListener("exit", removeAtExit);
    await rm(cacheDir, { recursive: true, force: true });
    throw error;
  }
  const close = vite.close.bind(vite);
  let closing;
  vite.close = () => (closing ??= close().finally(async () => {
    process.removeListener("exit", removeAtExit);
    await rm(cacheDir, { recursive: true, force: true });
  }));
  return vite;
}
