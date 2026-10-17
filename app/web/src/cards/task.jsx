export const kind = "task";

// REA goal-execution-autonomy.md §14: WORKING / NEEDS_INPUT / DONE
// are independent of FINISHED / STOPPED / SUPERSEDED and tool completion.
const taskLabels = { working: "En cours", needs_input: "En attente de vous", done: "Terminé" };
const turnLabels = {
  finished: "Tour terminé",
  stopped: "Tour arrêté",
  superseded: "Tour remplacé par une relance",
  budget_exhausted: "Budget du tour épuisé",
};
const runLabels = {
  queued: "Tour en file d’attente",
  running: "Tour en cours",
  awaiting_approval: "En attente d’une approbation dans la conversation du bot principal",
  awaiting_human_input: "En attente d’une réponse dans la conversation du bot principal",
  awaiting_children: "En attente de sous-tâches",
  interrupted: "Tour interrompu · reprise en attente",
  finished: "Tour terminé",
  failed: "Tour en échec",
  cancelled: "Tour annulé",
};

export function Card({ message }) {
  const { task_status: task, turn_end: turnEnd, status } = message.payload || {};
  const label = Object.hasOwn(taskLabels, task?.status) ? taskLabels[task.status] : null;
  // A chat has no task_status; never manufacture a forever-Working task for it.
  if (!label) {
    return <article className="gb-message gb-engine-card" data-message-id={message.id} data-card-kind={kind}>Statut de tâche indisponible.</article>;
  }
  const done = task.status === "done";
  const ended = Boolean(turnEnd) || ["finished", "failed", "cancelled"].includes(status);
  const turnLabel = Object.hasOwn(turnLabels, turnEnd) ? turnLabels[turnEnd]
    : Object.hasOwn(runLabels, status) ? runLabels[status] : "État du tour indisponible";
  return (
    <article className="gb-message gb-engine-card" data-message-id={message.id} data-card-kind={kind} data-task-status={task.status}>
      <header className="gb-engine-heading"><strong>Tâche</strong><span className="gb-engine-label">{label}</span></header>
      <p className="gb-engine-note">{turnLabel}</p>
      {!done && ended && <p className="gb-engine-note">La tâche reste ouverte : sa fin n’a pas été déclarée.</p>}
      {done && <p className="gb-engine-text">{typeof task.summary === "string" && task.summary.trim() ? task.summary : "Tâche terminée sans résumé."}</p>}
    </article>
  );
}
