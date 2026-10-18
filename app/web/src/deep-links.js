// REA desktop.deep_links: listener before readiness, 16 queued links and a 2s
// canonical dedup window. Volatile navigation only: no automatic send or replay
// after an application restart. The Rust resolver is the grammar/access authority.
export function createDeepLinkQueue({ resolve, navigate, report, generation = () => 0, now = Date.now, capacity = 16 }) {
  const pending = [], seen = new Map();
  let ready = false, draining = false;
  async function drain() {
    if (!ready || draining) return;
    draining = true;
    try {
      while (ready && pending.length) {
        const entry = pending.shift(), { raw } = entry;
        try {
          const expected = entry.expected ?? generation();
          const link = entry.link || await resolve(raw);
          if (link.duplicate) continue;
          if (generation() !== expected) { report("Navigation annulée : vous avez changé de vue pendant l’ouverture du lien."); continue; }
          if (!ready) { pending.unshift({ raw, link, expected }); break; }
          const at = now();
          for (const [key, time] of seen) if (at - time >= 2000) seen.delete(key);
          if (seen.has(link.canonical)) continue;
          if (seen.size >= capacity) { report("Trop de liens reçus. Réessayez dans quelques secondes."); continue; }
          seen.set(link.canonical, at);
          await navigate(link.destination);
        } catch (error) { report(error.message || "Ce lien n’a pas pu être ouvert."); }
      }
    } finally { draining = false; }
  }
  return {
    receive(raw) {
      if (typeof raw !== "string" || raw.length > 2048) { report("Ce lien Ezerbot est invalide ou trop long."); return true; }
      if (pending.some((entry) => entry.raw === raw)) return true;
      if (pending.length >= capacity) { report("Trop de liens en attente. Rouvrez ce lien après le démarrage."); return true; }
      pending.push({ raw }); void drain();
      return true;
    },
    setReady(value) { ready = Boolean(value); void drain(); },
    // Deterministic test checkpoint, not a separate routing implementation.
    async idle() { while (draining) await new Promise((done) => setTimeout(done, 0)); },
  };
}
