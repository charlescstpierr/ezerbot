import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { conversationReplyError, presentThread, threadActivity } from "./thread-presentation.js";
import { replyProblemState } from "./connection-view.js";

const engineReasons = [
  "Résultat inconnu après interruption. Action non relancée ; vérifiez son effet avant de réessayer.",
  "Appel externe interrompu ; son effet peut déjà avoir eu lieu.",
  "L’appel externe a été interrompu après son autorisation. Son effet est incertain ; vérifiez avant de réessayer.",
];
const tool = (payload) => ({ id: "run:3", role: "assistant", kind: "tool", payload: { run_id: "run", call: { call_id: "call", name: "run_shell", arguments: { command: "printf trace" } }, ...payload } });
const task = (id, payload) => ({ id, role: "assistant", kind: "task", body: "", payload: { run_id: "run", status: "finished", routine: true, task_status: { status: "working" }, ...payload } });
const reply = { id: "reply", role: "assistant", kind: "message", body: "Je consulte le dossier.", state: "echoed", payload: { run_id: "run" } };
const notes = (messages) => presentThread(messages).messages.filter((message) => message.presentation).map((message) => message.presentation.text);

test("une action au résultat inconnu reste inconnue, sans trace d’outil", () => {
  for (const reason of engineReasons) {
    const [note, ...others] = notes([tool({ code: "uncertain", outcome: { status: "failed", reason } })]);
    assert.deepEqual(others, []);
    assert.match(note, /résultat est inconnu/);
    assert.match(note, /Vérifiez son effet avant de réessayer/);
    assert.doesNotMatch(note, /n’a pas abouti|run_shell|printf|Commande/);
  }
});

test("le résultat inconnu passe avant un autre libellé de l’outil ou de l’approbation", () => {
  const outcome = { status: "failed", reason: engineReasons[0] };
  const [image] = notes([tool({ code: "uncertain", call: { name: "generate_image" }, outcome })]);
  assert.doesNotMatch(image, /n’a pas pu être créée/);
  const [approved] = notes([tool({ code: "uncertain", approval_status: "cancelled", outcome })]);
  assert.doesNotMatch(approved, /Aucune action n’a été faite/);
});

test("un échec établi reste un échec et un résultat établi reste silencieux", () => {
  assert.deepEqual(notes([tool({ code: "failed", outcome: { status: "failed", reason: "Commande introuvable." } })]), ["La commande n’a pas abouti."]);
  assert.deepEqual(notes([tool({ outcome: { status: "failed", reason: "Fin confirmée après coup." } })]), ["La commande n’a pas abouti."]);
  assert.deepEqual(notes([tool({ code: "ok", outcome: { status: "done", output: { status: "terminé" } } })]), []);
});

test("une routine arrêtée par le budget garde son avertissement dans le fil", () => {
  const [note, ...others] = notes([task("run:9", { event_kind: "turn_ended", turn_end: "budget_exhausted" })]);
  assert.deepEqual(others, []);
  assert.match(note, /budget de ce tour est épuisé/);
  assert.match(note, /tâche n’est pas terminée/);
});

test("l’avertissement de budget arrive une fois, après les dernières phrases du bot", () => {
  const stopped = { turn_end: "budget_exhausted", task_status: { status: "needs_input" } };
  const { messages } = presentThread([task("run:2", { ...stopped, event_kind: "task_changed" }), reply, task("run:9", { ...stopped, event_kind: "turn_ended" })]);
  assert.deepEqual(messages.map((message) => message.id), ["reply", "run:9"]);
  assert.equal(messages[1].presentation.kind, "problem");
});

test("seul un tour épuisé sur une tâche ouverte avertit", () => {
  assert.deepEqual(notes([task("run:9", { event_kind: "turn_ended", turn_end: "finished" })]), []);
  const done = presentThread([task("run:9", { event_kind: "turn_ended", turn_end: "budget_exhausted", task_status: { status: "done", summary: "Rapport envoyé." } })]).messages;
  assert.deepEqual(done.map((message) => [message.presentation.kind, message.presentation.text]), [["completion", "Rapport envoyé."]]);
});

test("la consigne d’une délégation reste lisible dans le fil de la tâche", () => {
  const brief = { id: "brief", role: "user", kind: "delegation_brief", body: "Résume ce dossier.", attachments: [{ id: "piece" }], payload: { source: "delegation" } };
  assert.deepEqual(presentThread([brief]).messages, [brief]);
});

test("une remise à zéro du contexte reste une phrase visible, sans carte", () => {
  const body = "Nouvelle conversation — la mémoire est conservée.";
  assert.deepEqual(notes([{ id: "reset", role: "assistant", kind: "reset", body, payload: { reset: true } }]), [body]);
});

const source = (path) => readFileSync(new URL(`../../../crates/wrapper/src/${path}`, import.meta.url), "utf8");
const sentences = (text) => [...text.matchAll(/"((?:[^"\\]|\\.)*)"/g)].map(([, literal]) => literal.replace(/\{[^}]*\}/g, ""))
  .filter((literal) => /^[A-ZÉ].* .*[.…]$/u.test(literal));
const provider = source("provider.rs");
const engine = source("features/engine.rs").split("\n").filter((line) => /^const (INTERRUPTED|CONTEXT_TOO_LARGE)|let group_error|SET error=\?2/.test(line)).join("\n");
const emitted = [...new Set([
  ...sentences(provider.slice(provider.indexOf("pub(crate) fn failure("), provider.indexOf("fn text("))),
  ...sentences(source("provider/reliable.rs").split("mod tests")[0]),
  ...sentences(engine),
])];

test("chaque diagnostic rédigé par le moteur reste tel quel dans le fil", () => {
  assert.ok(emitted.length >= 20, `${emitted.length} phrases lues dans le moteur`);
  for (const error of emitted) assert.equal(conversationReplyError({ error }), error);
});

test("un modèle refusé garde la consigne d’en choisir un autre", () => {
  const error = "OpenAI n’accepte pas ce modèle « o9 » avec votre connexion. Choisissez un autre modèle avec le bouton « Modèle » sous la zone de message, puis relancez la réponse ou envoyez un nouveau message.";
  const view = replyProblemState({ error_kind: "other", error: conversationReplyError({ error }) });
  assert.match(view.error, /Choisissez un autre modèle/);
  assert.equal(view.advice, null);
});

test("un dépassement de contexte demande une conversation plus courte, jamais un nouveau message", () => {
  const error = "Le fil dépasse le budget de contexte disponible. Ouvrez une conversation plus courte ; aucun ancien message n’a été supprimé ni résumé automatiquement.";
  const view = replyProblemState({ error_kind: "context", error: conversationReplyError({ error_kind: "context", error }) }, { isGroup: true });
  assert.equal(view.error, error);
  assert.equal(view.advice, null);
  assert.equal(view.retry, false);
});

test("la limite d’usage garde la phrase du moteur", () => {
  const error = "Ezerbot a atteint la limite d’utilisation que lui accorde votre abonnement ChatGPT. Votre quota Codex peut rester disponible. Pour ajuster cette limite : ChatGPT → Réglages → Utilisation.";
  assert.equal(conversationReplyError({ error_kind: "limit", error }), error);
});

test("le détail brut d’un fournisseur laisse sa phrase et disparaît seul", () => {
  assert.equal(conversationReplyError({ error: "OpenAI a refusé la demande (erreur 400) : Invalid 'input[3].call_id'." }), "OpenAI a refusé la demande (erreur 400).");
  assert.equal(conversationReplyError({ error: "OpenAI a signalé une erreur pendant la réponse : {\"code\":\"x\"}" }), "OpenAI a signalé une erreur pendant la réponse.");
  assert.equal(conversationReplyError({ error: "Le service d’OpenAI est indisponible (HTTP 503). Réessayez dans quelques minutes. Détail : <html>busy</html>" }), "Le service d’OpenAI est indisponible (HTTP 503). Réessayez dans quelques minutes.");
  assert.equal(conversationReplyError({ error: "OpenAI a refusé la demande (erreur 429) : You exceeded your quota." }), "OpenAI a refusé la demande (erreur 429) : You exceeded your quota.");
});

test("un diagnostic purement technique ou absent devient une phrase", () => {
  for (const error of ['{"command":"ls","cwd":"/tmp"}', "/opt/ezerbot/run failed", "", null, undefined]) assert.equal(conversationReplyError({ error }), "La réponse n’a pas abouti. Réessayez.");
});

const host = (name, outcome, extra = {}) => ({ id: "mcp-host-a:2", role: "assistant", kind: "tool", payload: { run_id: "mcp-host-a", call: { call_id: "mcp-host-a-call", name, arguments: { id: "generated_1", action: "save", path: "/home/charles/copie.pdf" } }, outcome, ...extra } });
const resolved = (run_id, tool, status, extra = {}) => ({ id: `approval-${run_id}`, role: "assistant", kind: "approval", payload: { run_id, kind: { tool }, status: { status }, ...extra } });

test("« Enregistrer sous… » laisse un accusé sans aucun bot pour le dire", () => {
  const save = host("document_action", { status: "done", output: { message: "Copie enregistrée.", path: "/home/charles/copie.pdf" } });
  const { messages, interventions } = presentThread([resolved("mcp-host-a", "document_action", "resolved"), save]);
  assert.deepEqual(interventions, []);
  assert.deepEqual(messages.map((message) => [message.id, message.presentation?.kind, message.presentation?.text]), [["mcp-host-a:2", "notice", "Copie enregistrée."]]);
  assert.doesNotMatch(JSON.stringify(messages[0].presentation), /copie\.pdf|document_action/);
});

test("une action des paramètres garde sa consigne et la raison de son échec", () => {
  const code = "Ouvrez github.com/login/device dans votre navigateur, puis entrez le code affiché dans Paramètres › Connecteurs.";
  assert.deepEqual(notes([host("mcp_start_github", { status: "done", output: { message: code, outils: [] } })]), [code]);
  const reason = "Ce fichier existe déjà : choisissez un autre nom.";
  assert.deepEqual(notes([host("document_action", { status: "failed", reason })]), [reason]);
  assert.deepEqual(notes([host("document_action", { status: "failed", reason: "os error 13 at /home/charles/copie.pdf" })]), ["L’action sur le document n’a pas abouti."]);
});

test("le modèle qui agit garde la parole : son outil réussi reste silencieux", () => {
  const save = { ...tool({ outcome: { status: "done", output: { message: "Copie enregistrée." } } }) };
  save.payload.call = { call_id: "call", name: "document_action", arguments: {} };
  assert.deepEqual(notes([save]), []);
});

test("une action refusée se dit une seule fois quand sa demande est dans le fil", () => {
  const refused = tool({ approval_status: "denied", outcome: { status: "failed", reason: "L’action a été refusée." } });
  assert.deepEqual(notes([resolved("run", "run_shell", "denied"), refused]), ["L’action a été refusée. Elle n’a pas été exécutée."]);
  assert.deepEqual(notes([refused]), ["L’action a été refusée et n’a pas été exécutée."]);
});

test("un autre appel du même outil garde son propre échec à côté d’un refus", () => {
  const other = tool({ outcome: { status: "failed", reason: "Commande introuvable." } });
  assert.deepEqual(notes([resolved("run", "run_shell", "denied"), other]), ["L’action a été refusée. Elle n’a pas été exécutée.", "La commande n’a pas abouti."]);
});

test("un refus automatique garde le motif du relecteur", () => {
  const review = { answer: { decision: "deny", reason: "Cette commande efface des fichiers hors du projet." } };
  assert.deepEqual(notes([resolved("run", "run_shell", "denied", { review })]), ["L’action a été refusée automatiquement : Cette commande efface des fichiers hors du projet."]);
});

test("les refus déjà rédigés pour l’utilisateur gardent leur consigne", () => {
  const off = "Cet outil est désactivé pour ce bot dans Paramètres › Terminal et fichiers sur ce Mac.";
  assert.deepEqual(notes([tool({ outcome: { status: "failed", reason: off } })]), [off]);
  assert.match(notes([tool({ outcome: { status: "failed", reason: "Une autorisation permanente refuse cette action." } })])[0], /Paramètres › Autorisations permanentes/);
  assert.match(notes([tool({ outcome: { status: "failed", reason: "not executed: stopped" } })])[0], /Action non exécutée/);
});

test("un connecteur à rétablir le dit sans bouton ni nom d’outil", () => {
  const [note] = notes([tool({ connector: { server: "GitHub", tool: "lister", recoverable: true }, outcome: { status: "failed", reason: "transport closed" } })]);
  assert.equal(note, "Le connecteur GitHub ne répond plus. Ouvrez Paramètres › Connecteurs pour le rétablir.");
});

test("une question de tâche reste lisible dans le fil, une seule fois", () => {
  const ask = tool({ outcome: { status: "done", output: { status: "signalée" } } });
  ask.payload.call = { call_id: "call", name: "attendre_reponse", arguments: { question: "Quel dossier dois-je ranger ?" } };
  assert.deepEqual(notes([ask]), ["Quel dossier dois-je ranger ?"]);
  assert.deepEqual(notes([{ ...reply, body: "Je bloque. Quel dossier dois-je ranger ?" }, ask]), []);
});

test("un résumé de contexte se dit en une phrase, sans son contenu", () => {
  const [note] = notes([{ id: "summary", role: "assistant", kind: "summary", body: "trace", payload: { text: "trace" } }]);
  assert.match(note, /résumé le début/);
  assert.doesNotMatch(note, /trace/);
});

test("une entrée inconnue reste visible plutôt que disparaître", () => {
  const entry = { id: "new", role: "assistant", kind: "future_card", body: "" };
  assert.deepEqual(presentThread([entry]).messages, [entry]);
});

test("une attente se dit seulement quand la demande en cours vise une autre conversation", () => {
  const runs = [{ run_id: "child", bot_id: "helper", conversation_id: "c", status: "awaiting_approval" }, { run_id: "work", bot_id: "grokbot", conversation_id: "c", status: "running" }, { run_id: "kids", conversation_id: "c", status: "awaiting_children" }];
  const shown = (requests) => threadActivity({ runs, requests, conversationId: "c", engineConnected: true }).map((run) => [run.run_id, run.status]);
  const request = (target, status) => ({ run_id: "child", status: { status }, route: { target_conversation_id: target } });
  const running = [["work", "running"]];
  assert.deepEqual(shown([request("principal", "pending")]), [["child", "awaiting_approval"], ...running]);
  assert.deepEqual(shown([request("c", "pending")]), running, "la demande est dans ce fil");
  assert.deepEqual(shown([request("principal", "resolved")]), running, "une décision prise n’envoie nulle part");
  assert.deepEqual(shown([request("c", "resolved"), request("principal", "pending")]), [["child", "awaiting_approval"], ...running], "un ancien formulaire ne cache pas la nouvelle attente");
  assert.deepEqual(shown(undefined), running);
});
