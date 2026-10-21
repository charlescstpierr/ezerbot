import { useState, useSyncExternalStore } from "react";
import { localCallView, localStatusLabel, localFailureView, localLimits, localToolLabel } from "../local-call.js";
import { LocalResult } from "../local-result.js";
import { ScreenCard, screenTools } from "../screen.jsx";
import { GeneratedDocument, DocumentTool } from "../generated-document.jsx";

export const kind = "tool";

const engine = () => typeof window === "undefined" ? undefined : window.GrokbotEngine;
const subscribe = (changed) => {
  window.addEventListener("grokbot:engine-change", changed);
  return () => window.removeEventListener("grokbot:engine-change", changed);
};
export function useEngine() {
  useSyncExternalStore(subscribe, () => engine()?.version() ?? 0, () => 0);
  return engine();
}

// Shared presentation only. Never dump whole engine events: they can contain
// opaque provider reasoning. All external text stays text, never HTML/Markdown.
export function Data({ value }) {
  const text = typeof value === "string" ? value : JSON.stringify(value, null, 2);
  return <pre className="gb-engine-data">{text ?? "Non disponible"}</pre>;
}

export function Disclosure({ message, title, label, children }) {
  const [open, setOpen] = useState(false);
  return (
    <article className="gb-message gb-engine-card" data-message-id={message.id} data-card-kind={message.kind}>
      <details onToggle={(event) => setOpen(event.currentTarget.open)}>
        <summary><strong>{title}</strong>{label && <span className="gb-engine-label">{label}</span>}</summary>
        {open && <div className="gb-engine-details">{children}</div>}
      </details>
    </article>
  );
}

function suspension(message, view) {
  const { run_id: runId, outcome } = message.payload;
  const run = view?.run(runId);
  const human = outcome.cause?.kind === "human_input";
  const request = human ? view?.request(outcome.cause.id) : undefined;
  const current = !view?.latestSuspension(runId) || view.latestSuspension(runId) === message.id;
  if (request?.run_id === runId) {
    const labels = { denied: "Demande refusée", expired: "Demande expirée", cancelled: "Demande annulée" };
    if (Object.hasOwn(labels, request.status)) return { label: labels[request.status] };
    if (request.wake === "delivered") return { label: "Réponse reçue · appel repris" };
    if (request.wake === "failed") return { label: "Reprise non confirmée" };
    if (request.status === "resolved") return { label: "Réponse enregistrée · reprise en attente" };
  }
  if (["cancelled", "failed"].includes(run?.status)) return { label: "Appel interrompu", note: "Le run est terminé. Aucune réponse n’est demandée pour cet appel." };
  const waiting = current && (human
    ? ["awaiting_human_input", "awaiting_approval"].includes(run?.status)
    : outcome.cause?.kind === "children" && run?.status === "awaiting_children");
  if (waiting) return human
    ? { label: "En attente de vous", note: "Répondez dans la carte de demande de la conversation du bot principal." }
    : { label: "En attente de sous-tâches", note: "Cet appel attend ses sous-tâches. Leur fin ne termine pas la tâche principale." };
  // A missing active run proves neither delivery nor child success. Without a
  // receipt or a precise lifecycle snapshot, say only that the wait has ended.
  if (run?.status === "ended") return { label: "Attente terminée", note: "Le run n’est plus actif. Aucune nouvelle réponse n’est demandée par cet appel." };
  if (!current || ["running", "interrupted", "finished"].includes(run?.status)) return { label: human ? "Appel repris" : "Attente de sous-tâches terminée" };
  return { label: "État de l’appel indisponible" };
}

export function openConnectors(connector) {
  const dialog = document.getElementById("settings-dialog");
  if (!dialog?.open) document.getElementById("settings-button")?.click();
  requestAnimationFrame(() => {
    const section = document.querySelector(".mcp-section");
    const id = connector?.catalogue_id || window.GrokbotPanels?.state?.mcp?.catalogue?.find(item => item.name === connector?.server)?.id;
    const card = id && [...(section?.querySelectorAll("[data-connector]") || [])].find(item => item.dataset.connector === id);
    const target = card || section;
    target?.scrollIntoView({ block: "start" });
    (card?.querySelector('[data-connector-action="connect"]') || target?.querySelector(".mcp-actions a, button, summary"))?.focus({ preventScroll: true });
  });
}

function LocalCard({ message, run, onAction, busy }) {
  const { call, outcome, authorized_by: authority, approval_status: approvalStatus, run_status: storedStatus } = message.payload;
  const output = outcome?.output;
  const runStatus = run?.status && run.status !== "ended" ? run.status : storedStatus || run?.status;
  const { args, preview, shellArguments } = localCallView(call);
  const pending = !outcome && !["ended", "finished", "cancelled", "failed", "interrupted"].includes(runStatus);
  if (!outcome && runStatus === "awaiting_approval") return <article className="gb-message gb-engine-card gb-local-card gb-local-waiting" data-message-id={message.id} data-card-kind="tool"><p>Accord requis dans la carte d’approbation.</p></article>;
  const failure = outcome?.status === "failed" ? localFailureView(outcome.reason, approvalStatus, runStatus) : null;
  const status = failure?.label || (output?.status ? localStatusLabel(output.status) : outcome ? "Terminé" : pending ? "En cours" : "Arrêtée");
  const tone = failure?.tone || (output?.status === "échec" ? "danger" : ["annulé", "délai dépassé"].includes(output?.status) ? "warning" : outcome ? "success" : "neutral");
  return <article className="gb-message gb-engine-card gb-local-card" data-message-id={message.id} data-card-kind="tool" data-local-state={outcome?.status === "failed" ? "failed" : output?.status || (pending && runStatus === "running" ? "running" : "")}>
    <div className="gb-local-header"><strong>{localToolLabel(call.name)}</strong><span className="gb-local-status" data-tone={tone}>{status}</span></div>
    {authority?.kind === "rule" && <p>Autorisée automatiquement par une règle « Toujours » (Paramètres › Autorisations permanentes).</p>}
    {authority?.kind === "folder_grant" && <p>Lecture autorisée dans un dossier accordé.</p>}
    <pre className="gb-local-command">{typeof (call.name === "run_shell" ? args.command : args.path) === "string" ? preview : "Paramètres de l’action indisponibles ou invalides."}</pre>
    {failure && <p className="gb-local-failure" data-tone={failure.tone} role={failure.tone === "warning" ? "alert" : "status"}>{failure.message}</p>}
    {call.name === "run_shell" && shellArguments && <p>Dossier : {args.cwd} · {localLimits(args)}</p>}
    {pending && <button type="button" className="gb-retry gb-local-stop" disabled={busy} onClick={() => onAction?.("local.stop")}>Annuler l’exécution</button>}
    {output?.exit_code != null && <p>Code de sortie : {output.exit_code}</p>}
    {output?.signal != null && <p>Arrêtée de force (signal {output.signal}).</p>}
    {output?.truncated && call.name === "run_shell" && <p>Sortie tronquée : la limite a été atteinte.</p>}
    {output?.output_incomplete && <p>Sortie incomplète : la collecte a été arrêtée pour ne pas bloquer l’annulation.</p>}
    {outcome && <details><summary>{failure ? "Afficher les détails" : "Afficher la sortie"}</summary>{failure ? <Data value={call.arguments} /> : call.name === "run_shell" ? <><h4>Sortie standard</h4><Data value={output.stdout || "(vide)"} /><h4>Erreurs</h4><Data value={output.stderr || "(vide)"} /></> : <LocalResult name={call.name} output={output} arguments={args} />}</details>}
  </article>;
}

export function Card({ message, onAction, busy }) {
  const view = useEngine();
  const { run_id: runId, call, outcome, connector, authorized_by, requesting_bot, approval_status } = message.payload || {};
  // These calls already have a dedicated, user-facing decision/status card.
  if (["begin_task", "complete_task", "propose_skill"].includes(call?.name)) return null;
  if (!call?.call_id || typeof call.name !== "string") {
    return <article className="gb-message gb-engine-card" data-message-id={message.id} data-card-kind={kind}>Appel d’outil indisponible.</article>;
  }
  const run = view?.run(runId);
  const storedStatus = message.payload?.run_status;
  const runStatus = run?.status && run.status !== "ended" ? run.status : storedStatus || run?.status;
  if (screenTools[call.name]) return <ScreenCard message={message} />;
  if (call.name === "create_document" && outcome?.status === "done" && outcome.output?.id?.startsWith("generated_")) return <GeneratedDocument file={outcome.output} message={message} />;
  if (["create_document", "document_action"].includes(call.name)) return <DocumentTool message={message} run={run} />;
  if (["run_shell", "read_file", "list_directory", "write_file"].includes(call.name)) return <LocalCard message={message} run={run} onAction={onAction} busy={busy} />;
  const background = call.name === "routines_background_configure";
  const ended = ["ended", "finished", "cancelled", "failed", "interrupted"].includes(runStatus);
  let label = outcome?.status === "done" ? (outcome.output?.isError ? "Erreur de l’outil" : "Appel terminé") : outcome?.status === "failed" ? "Échec de l’appel" : "Résultat en attente";
  const decisionLabels = { denied: "Refusé", cancelled: "Annulé", expired: "Approbation expirée" };
  const refused = outcome?.status === "failed" && Object.hasOwn(decisionLabels, approval_status);
  if (refused) label = decisionLabels[approval_status];
  let note;
  if (outcome?.status === "suspend") ({ label, note } = suspension(message, view));
  if (!outcome) {
    label = ended ? "Non exécuté" : run?.status === "awaiting_approval" ? "En attente d’approbation" : "Résultat en attente";
    note = ended ? "Aucun résultat enregistré ; le run n’est plus actif."
      : run?.status === "awaiting_approval" ? "L’approbation se décide dans la conversation du bot principal."
        : "Aucun résultat enregistré pour cet appel.";
  }
  return (
    <Disclosure message={message} title={connector ? `Connecteur · ${connector.server} · ${connector.tool}` : call.name === "routines_background_configure" ? "Routines en arrière-plan" : `Outil · ${call.name}`} label={label}>
      {!background && <p className="gb-engine-note">Identifiant d’appel : <code>{call.call_id}</code></p>}
      {authorized_by?.kind === "rule" && <p className="gb-engine-note">Autorisé automatiquement par la règle « Toujours » ({requesting_bot || "Bot demandeur"} · {connector ? `${connector.server} · ${connector.tool}` : call.name}).</p>}
      {background ? <p>{call.arguments?.enabled ? "Installer l’agent de routines" : "Retirer l’agent de routines"}</p> : <><h4>Arguments</h4><Data value={call.arguments} /></>}
      {outcome?.status === "done" && <><h4>Résultat</h4>{connector && Array.isArray(outcome.output?.content)
        ? <>{outcome.output.content.some(part => part?.type === "text" && typeof part.text === "string")
          ? outcome.output.content.filter(part => part?.type === "text" && typeof part.text === "string").map((part, index) => <Data key={index} value={part.text} />)
          : <p className="gb-engine-note">Le résultat ne contient pas de texte. Consultez les données techniques ci-dessous.</p>}
          <details><summary>Données techniques du résultat</summary><Data value={outcome.output} /></details></>
        : background ? <p>{outcome.output?.message}</p> : <Data value={outcome.output} />}</>}
      {outcome?.status === "failed" && <><h4>{refused ? "Raison" : "Raison de l’échec"}</h4><Data value={outcome.reason} />{connector?.recoverable === true && !refused && <button type="button" className="gb-retry" onClick={() => openConnectors(connector)}>Ouvrir les connecteurs</button>}</>}
      {note && <p className="gb-engine-note">{note}</p>}
    </Disclosure>
  );
}
