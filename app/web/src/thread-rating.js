// Notation locale du Mac. Rien ici n’appelle le réseau et rien n’entre dans le fil.

export const RATING_KEY = "ezerbot.notations";

export const ratingLabels = {
  good: "Bon résultat",
  bad: "Mauvais résultat",
  title: "Comment s’améliorer ?",
  reasons: [
    { id: "pas-demande", label: "Pas ce que j’ai demandé" },
    { id: "inexact", label: "Inexact" },
    { id: "qualite", label: "Qualité faible" },
    { id: "lent", label: "Trop lent" },
  ],
  details: "Détails",
  send: "Envoyer",
  close: "Fermer",
};

function readRows(storage) {
  try {
    const parsed = JSON.parse(storage.getItem(RATING_KEY) || "[]");
    return Array.isArray(parsed) ? parsed : [];
  } catch {
    return [];
  }
}

export function createRatingTable(storage, { now = () => new Date().toISOString(), createId = () => globalThis.crypto.randomUUID() } = {}) {
  const write = (rows) => storage.setItem(RATING_KEY, JSON.stringify(rows));
  const append = (row) => {
    const rows = [...readRows(storage), row];
    const saved = write(rows);
    return saved?.then ? saved.then(() => row) : row;
  };
  return {
    list() { return readRows(storage); },
    recordGood({ conversationId, messageId, runId }) {
      return append({
        id: createId(),
        conversation_id: conversationId,
        message_id: messageId,
        run_id: runId || null,
        vote: "bon",
        motif: null,
        details: "",
        created_at: now(),
      });
    },
    recordBad({ conversationId, messageId, runId, motif, details }) {
      const reason = ratingLabels.reasons.find((item) => item.id === motif);
      return append({
        id: createId(),
        conversation_id: conversationId,
        message_id: messageId,
        run_id: runId || null,
        vote: "mauvais",
        motif: reason?.label || null,
        details: String(details || ""),
        created_at: now(),
      });
    },
    exportDocument() {
      return { notations: readRows(storage) };
    },
  };
}

// La copie en mémoire sert les exports synchrones ; chaque écriture attend le Mac.
export async function createNativeRatingTable(bridge) {
  let value = await bridge.postMessage({ command: "read" });
  let pending = Promise.resolve();
  return createRatingTable({
    getItem() { return value; },
    setItem(_key, next) {
      value = next;
      const saved = pending.then(() => bridge.postMessage({ command: "write", value: next }));
      pending = saved.catch(() => {});
      return saved;
    },
  });
}
