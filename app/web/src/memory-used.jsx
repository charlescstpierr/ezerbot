import { useId, useState } from "react";
import { Brain } from "lucide-react";
import { MEMORY_UNAVAILABLE, memoryUsedLabel, recalledIds } from "./memory-used.js";

// Ouvre l'écran Mémoire sur les souvenirs de la réponse ; renvoie la raison, en français, quand rien ne s'ouvre.
export async function openMemory(ids, onOpenMemory, message) {
  if (typeof onOpenMemory !== "function") return MEMORY_UNAVAILABLE;
  try {
    const result = await onOpenMemory(ids, message);
    return result?.ok === false ? result.message || "L’écran Mémoire n’a pas pu s’ouvrir." : null;
  } catch {
    return "L’écran Mémoire n’a pas pu s’ouvrir.";
  }
}

// « 1 souvenir utilisé » sous la réponse, à côté de « Sources » ; rien quand aucun souvenir n'a servi.
export function MemoryUsedPill({ message, onOpenMemory }) {
  const ids = recalledIds(message);
  const [problem, setProblem] = useState(null);
  const id = useId();
  if (!ids.length) return null;
  return (
    <>
      <button type="button" className="gb-memory-used" title="Voir dans la Mémoire" aria-describedby={problem ? id : undefined} onClick={async () => setProblem(await openMemory(ids, onOpenMemory, message))}>
        <Brain aria-hidden="true" />{memoryUsedLabel(ids.length)}
      </button>
      {problem && <span className="gb-source-problem" id={id} role="status">{problem}</span>}
    </>
  );
}
