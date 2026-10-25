// Souvenirs rappelés pour une réponse : le moteur les enregistre (#266) dans `payload.recalled_fact_ids`,
// déjà débarrassés des souvenirs oubliés. Cette lecture est la frontière : au-delà, on ne revalide rien.
export const MEMORY_UNAVAILABLE = "L’écran Mémoire n’est pas disponible ici.";

// Les identifiants, dans l'ordre du rappel, sans doublon ni valeur que l'écran Mémoire ne saurait retrouver.
export function recalledIds(message) {
  const list = message?.payload?.recalled_fact_ids;
  if (!Array.isArray(list)) return [];
  return [...new Set(list.filter((id) => typeof id === "string" && id.trim()))];
}

export function memoryUsedLabel(count) {
  return count < 2 ? `${count} souvenir utilisé` : `${count} souvenirs utilisés`;
}
