// « En attente de vous depuis 2 h · expire dans 22 h » : la ligne grise au-dessus d'une
// carte d'approbation qui attend (#217). Elle ne dit que le temps : jamais l'outil, les
// arguments ni un secret.
const MINUTE = 60_000;

function duration(ms, round) {
  const minutes = Math[round](ms / MINUTE);
  if (minutes < 1) return "moins d’une minute";
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return minutes % 60 ? `${hours} h ${String(minutes % 60).padStart(2, "0")}` : `${hours} h`;
  const days = Math.floor(hours / 24);
  return hours % 24 ? `${days} j ${hours % 24} h` : `${days} j`;
}

// Null quand la carte ne peut pas dire son âge ou que son échéance est passée : la carte
// reste alors telle qu'elle était et l'état se rafraîchit seul.
export function approvalWaitLine(createdAt, expiresAt, now) {
  if (!Number.isFinite(createdAt) || createdAt <= 0 || !Number.isFinite(expiresAt) || expiresAt <= now) return null;
  return `En attente de vous depuis ${duration(Math.max(0, now - createdAt), "floor")} · expire dans ${duration(expiresAt - now, "ceil")}`;
}
