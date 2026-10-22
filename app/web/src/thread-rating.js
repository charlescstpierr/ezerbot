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

// La copie en mémoire sert les exports synchrones ; elle ne reçoit un vote
// qu’une fois le Mac d’accord. Les votes passent un par un : chacun repart de
// la dernière copie validée, donc un échec ne laisse rien derrière lui.
export async function createNativeRatingTable(bridge) {
  let committed = await bridge.postMessage({ command: "read" });
  let queue = Promise.resolve();
  const table = createRatingTable({
    getItem() { return committed; },
    async setItem(_key, next) {
      await bridge.postMessage({ command: "write", value: next });
      committed = next;
    },
  });
  const serial = (record) => (input) => {
    const saved = queue.then(() => record(input));
    queue = saved.catch(() => {});
    return saved;
  };
  return { ...table, recordGood: serial(table.recordGood), recordBad: serial(table.recordBad) };
}
