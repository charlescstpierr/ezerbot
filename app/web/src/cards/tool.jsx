import { useState, useSyncExternalStore } from "react";
import { localCallView, localStatusLabel, localFailureView, localLimits, localToolLabel } from "../local-call.js";
import { runView } from "../run-status.js";
import { LocalResult } from "../local-result.js";
import { ScreenCard, screenTools } from "../screen.jsx";
import { GeneratedDocument, DocumentTool } from "../generated-document.jsx";
import { GeneratedImage } from "../generated-image.jsx";
import { AUTOMATIC_LABEL, automaticLine, isAutomatic, sourceLabel } from "../yolo-view.js";

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

export function Data({ value }) {
  const text = typeof value === "string" ? value : JSON.stringify(value, null, 2);
  return <pre className="gb-engine-data">{text ?? "Non disponible"}</pre>;
}

export function Disclosure({ message, title, label, automatic, line, children }) {
  const [open, setOpen] = useState(false);
  return (
    <article className={`gb-message gb-engine-card${automatic ? " gb-auto-card" : ""}`} data-message-id={message.id} data-card-kind={message.kind} data-auto={automatic ? "yolo" : undefined}>
      {line && <p className="gb-auto-line" title={line.title}>{line.text}</p>}
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

export function TaskInputView({ line, onLine, onSubmit, sending, disabled = false, notice }) {
  return <>
    <form className="gb-local-input" onSubmit={onSubmit}>
      <input type="text" name="task-input" value={line} disabled={disabled} maxLength={511} autoComplete="off" spellCheck={false} placeholder="Une ligne pour le programme" aria-label="Saisie pour la tâche" onChange={(event) => onLine?.(event.target.value)} />
      <button type="submit" className="gb-retry" disabled={sending || disabled}>Envoyer</button>
    </form>
    {notice && <p className="gb-local-failure" data-tone="warning" role="alert">{notice}</p>}
  </>;
}

// The field empties once its line is sent, unless Charles has typed more since.
export const settleDraft = (current, sent) => current === sent ? "" : current;

// One line to the program's standard input. A refusal (finished task, password
// prompt) is the engine's French sentence, kept on the card with the line.
export function TaskInput({ onSend, disabled = false }) {
  const [line, setLine] = useState("");
  const [sending, setSending] = useState(false);
  const [notice, setNotice] = useState("");
  const submit = async (event) => {
    event.preventDefault();
    if (sending || disabled) return;
    setSending(true);
    const sent = line;
    try { await onSend(sent); setLine((current) => settleDraft(current, sent)); setNotice(""); }
    catch (error) { setNotice(error?.message || "La saisie n’a pas été envoyée."); }
    finally { setSending(false); }
  };
  return <TaskInputView line={line} onLine={setLine} onSubmit={submit} sending={sending} disabled={disabled} notice={notice} />;
}

function LocalCard({ message, run, onAction, busy }) {
  const { call, outcome, authorized_by: authority, approval_status: approvalStatus, run_status: storedStatus } = message.payload;
  const output = outcome?.output;
  const { status: runStatus, stopped } = runView(storedStatus, run?.status);
  const { args, preview, shellArguments } = localCallView(call);
  const pending = !outcome && !stopped;
  if (!outcome && runStatus === "awaiting_approval") return <article className="gb-message gb-engine-card gb-local-card gb-local-waiting" data-message-id={message.id} data-card-kind="tool"><p>Accord requis dans la carte d’approbation.</p></article>;
  const failure = outcome?.status === "failed" ? localFailureView(outcome.reason, approvalStatus, runStatus) : null;
  const running = output?.background === true && output?.status === "en cours";
  const status = failure?.label || (output?.status ? localStatusLabel(output.status) : outcome ? "Terminé" : pending ? "En cours" : "Arrêtée");
  const tone = failure?.tone || (output?.status === "échec" ? "danger" : ["annulé", "délai dépassé", "arrêtée"].includes(output?.status) ? "warning" : running || !outcome ? "neutral" : "success");
  const automatic = isAutomatic(authority);
  const command = <pre className="gb-local-command">{typeof (call.name === "run_shell" ? args.command : args.path) === "string" ? preview : "Paramètres de l’action indisponibles ou invalides."}</pre>;
  return <article className={`gb-message gb-engine-card gb-local-card${automatic ? " gb-auto-card" : ""}`} data-message-id={message.id} data-card-kind="tool" data-auto={automatic ? "yolo" : undefined} data-local-state={outcome?.status === "failed" ? "failed" : output?.status || (pending && runStatus === "running" ? "running" : "")}>
    <div className="gb-local-header">{automatic && <span className="gb-auto-badge">{AUTOMATIC_LABEL}</span>}<strong>{localToolLabel(call.name)}</strong><span className="gb-local-status" data-tone={tone}>{status}</span></div>
    {authority?.kind === "rule" && <p>Autorisée automatiquement par une règle « Toujours » (Paramètres › Autorisations permanentes).</p>}
    {authority?.kind === "folder_grant" && <p>Lecture autorisée dans un dossier accordé.</p>}
    {automatic ? <p className="gb-auto-line" title={sourceLabel(authority)}>{automaticLine({ call, outcome })}</p> : command}
    {failure && <p className="gb-local-failure" data-tone={failure.tone} role={failure.tone === "warning" ? "alert" : "status"}>{failure.message}</p>}
    {call.name === "run_shell" && shellArguments && <p>Dossier : {args.cwd} · {localLimits(args)}</p>}
    {pending && <button type="button" className="gb-retry gb-local-stop" disabled={busy} onClick={() => onAction?.("local.stop")}>Annuler l’exécution</button>}
    {running && <button type="button" className="gb-retry gb-local-stop" disabled={busy} onClick={() => onAction?.("local.task.stop")}>Arrêter</button>}
    {running && !(typeof window !== "undefined" && window.EzerbotIPhone?.remote) && <TaskInput onSend={(line) => onAction?.("local.task.input", { line })} />}
    {output?.exit_code != null && <p>Code de sortie : {output.exit_code}</p>}
    {output?.signal != null && <p>Arrêtée de force (signal {output.signal}).</p>}
    {output?.truncated && call.name === "run_shell" && <p>Sortie tronquée : la limite a été atteinte.</p>}
    {output?.output_incomplete && <p>Sortie incomplète : la collecte a été arrêtée pour ne pas bloquer l’annulation.</p>}
    {outcome && <details><summary>{failure ? "Afficher les détails" : "Afficher la sortie"}</summary>{automatic && command}{failure ? <Data value={call.arguments} /> : call.name === "run_shell" ? <><h4>Sortie standard</h4><Data value={output.stdout || "(vide)"} /><h4>Erreurs</h4><Data value={output.stderr || "(vide)"} /></> : <LocalResult name={call.name} output={output} arguments={args} />}</details>}
  </article>;
}

export function Card({ message, onAction, busy }) {
  const view = useEngine();
  const { run_id: runId, call, outcome, connector, authorized_by, requesting_bot, approval_status } = message.payload || {};
  if (["begin_task", "complete_task", "propose_skill"].includes(call?.name)) return null;
  if (!call?.call_id || typeof call.name !== "string") {
    return <article className="gb-message gb-engine-card" data-message-id={message.id} data-card-kind={kind}>Appel d’outil indisponible.</article>;
  }
  const run = view?.run(runId);
  const storedStatus = message.payload?.run_status;
  const { status: runStatus, stopped: ended } = runView(storedStatus, run?.status);
  if (screenTools[call.name]) return <ScreenCard message={message} />;
  if (call.name === "create_document" && outcome?.status === "done" && outcome.output?.id?.startsWith("generated_")) return <GeneratedDocument file={outcome.output} message={message} />;
  if (["create_document", "document_action"].includes(call.name)) return <DocumentTool message={message} run={run} />;
  if (call.name === "generate_image") return <GeneratedImage message={message} onAction={onAction} />;
  if (["run_shell", "read_file", "list_directory", "write_file"].includes(call.name)) return <LocalCard message={message} run={run} onAction={onAction} busy={busy} />;
  const background = call.name === "routines_background_configure";
  const publication = call.name === "routines_webhook_publish";
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
  const automatic = isAutomatic(authorized_by);
  return (
    <Disclosure message={message} title={connector ? `Connecteur · ${connector.server} · ${connector.tool}` : background ? "Routines en arrière-plan" : publication ? "Webhooks de routines" : `Outil · ${call.name}`} label={automatic ? `${AUTOMATIC_LABEL} · ${label}` : label} automatic={automatic} line={automatic ? { text: automaticLine({ call, outcome, connector }), title: sourceLabel(authorized_by) } : undefined}>
      {!background && !publication && <p className="gb-engine-note">Identifiant d’appel : <code>{call.call_id}</code></p>}
      {authorized_by?.kind === "rule" && <p className="gb-engine-note">Autorisé automatiquement par la règle « Toujours » ({requesting_bot || "Bot demandeur"} · {connector ? `${connector.server} · ${connector.tool}` : call.name}).</p>}
      {background || publication ? <p>{background ? (call.arguments?.enabled ? "Installer l’agent de routines" : "Retirer l’agent de routines") : (call.arguments?.enabled ? "Publier les webhooks sur Internet" : "Retirer la publication des webhooks")}</p> : <><h4>Arguments</h4><Data value={call.arguments} /></>}
      {outcome?.status === "done" && <><h4>Résultat</h4>{connector && Array.isArray(outcome.output?.content)
        ? <>{outcome.output.content.some(part => part?.type === "text" && typeof part.text === "string")
          ? outcome.output.content.filter(part => part?.type === "text" && typeof part.text === "string").map((part, index) => <Data key={index} value={part.text} />)
          : <p className="gb-engine-note">Le résultat ne contient pas de texte. Consultez les données techniques ci-dessous.</p>}
          <details><summary>Données techniques du résultat</summary><Data value={outcome.output} /></details></>
        : background || publication ? <p>{outcome.output?.message}</p> : <Data value={outcome.output} />}</>}
      {outcome?.status === "failed" && <><h4>{refused ? "Raison" : "Raison de l’échec"}</h4><Data value={outcome.reason} />{connector?.recoverable === true && !refused && <button type="button" className="gb-retry" onClick={() => openConnectors(connector)}>Ouvrir les connecteurs</button>}</>}
      {note && <p className="gb-engine-note">{note}</p>}
    </Disclosure>
  );
}
