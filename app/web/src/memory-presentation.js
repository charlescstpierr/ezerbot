// Keep attribution and inactive receipts consistent without rendering old fact
// text. Facts are literal user quotations, never a statement in the bot's voice.
export function memoryPresentation(fact, botName = "Ezerbot", isGroup = false) {
  const label = fact.replaced ? "Mis à jour :" : "Mémorisé :";
  const groupLabel = fact.replaced ? `Mis à jour par ${botName} :` : `Mémorisé par ${botName} :`;
  const receipt = fact.cancelled
    ? fact.replaced ? "Mise à jour annulée — l’ancien fait est rétabli." : "Mémorisation annulée."
    : fact.updated ? "Ce fait a été mis à jour." : "Ce fait a été oublié.";
  return { label: isGroup ? groupLabel : label, quote: fact.active ? `« ${fact.text} »` : "", receipt };
}
