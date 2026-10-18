// Keep attribution and inactive receipts consistent without rendering old fact
// text. Facts are literal user quotations, never a statement in the bot's voice.
export function memoryPresentation(fact, botName = "Ezerbot", isGroup = false) {
  const label = fact.replaced ? "Mis à jour :" : "Mémorisé :";
  const groupLabel = fact.replaced ? `Mis à jour par ${botName} :` : `Mémorisé par ${botName} :`;
  const receipt = fact.cancelled
    ? fact.replaced ? fact.restored_active ? "Mise à jour annulée — l’ancien fait est rétabli." : "Mise à jour annulée." : "Mémorisation annulée."
    : fact.updated ? "Ce fait a été mis à jour." : "Ce fait a été oublié.";
  return { label: isGroup ? groupLabel : label, quote: fact.active ? `« ${fact.text} »` : "", receipt };
}

// Group addressing may produce up to 18 private receipts for one human turn.
// Keep three visible and one native disclosure for the rest, preserving each
// original message/id so its actions still target the correct bot and fact.
export function collapseMemoryReceipts(messages, isGroup) {
  if (!isGroup) return messages;
  const roots = new Map();
  for (const message of messages) {
    const root = message.kind === "memory_auto" && message.payload?.memory_root_id;
    if (root) roots.set(root, [...(roots.get(root) || []), message]);
  }
  const hidden = new Set(), disclosures = new Map();
  for (const [root, receipts] of roots) {
    if (receipts.length <= 3) continue;
    const extra = receipts.slice(3);
    for (const receipt of extra) hidden.add(receipt);
    disclosures.set(receipts.at(-1), { id: `memory-overflow:${root}`, kind: "memory_overflow", memories: extra });
  }
  return messages.flatMap((message) => disclosures.has(message) ? [disclosures.get(message)] : hidden.has(message) ? [] : [message]);
}
