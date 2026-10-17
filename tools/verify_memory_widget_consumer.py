#!/usr/bin/env python3
"""Verify the memory-widget documentary chain offline, without loading target JS."""
from pathlib import Path
import hashlib
import json
import runpy
import sys

ROOT = Path(__file__).resolve().parents[1]
DEFAULT = ROOT / "evidence/memory-widget-consumer.json"
VIEW = "dist/renderer/assets/chunk-view-BnlWmtVC.js"
APP = "dist/renderer/assets/index.eager-app-BfVhIMYf.js"
ROW = "dist/renderer/assets/chunk-team-setup-row-C-xk3_Xs.js"
MAIN = "dist/electron-main/main-app.cjs"
PROTO = "dist/electron-main/proto.cjs"


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def verify(path=DEFAULT):
    helper = runpy.run_path(str(ROOT / "tools/evidence_ledger.py"))
    helper["verify"](path)
    ledger = json.loads(path.read_text())
    manifest = {
        r["path"]: r["sha256"]
        for r in json.loads((ROOT / "artifacts/manifest.json").read_text())["entries"]
    }
    sources = {s["id"]: s for s in ledger["sources"]}
    texts = {}
    for rel, source in sources.items():
        original = ROOT / "artifacts/client" / rel
        formatted = ROOT / "artifacts/formatted" / rel
        assert Path(source["original_path"]) == original
        assert Path(source["formatted_path"]) == formatted
        assert digest(original.read_bytes()) == manifest[rel]
        assert b"\r" not in formatted.read_bytes()
        texts[rel] = formatted.read_text()

    def section(rel, start, end):
        text = texts[rel]
        a = text.index(start)
        return text[a:text.index(end, a + len(start))]

    def contains(label, text, *needles):
        for needle in needles:
            assert needle in text, (label, needle)
        checks.append(label)

    checks = []
    contains("lazy-dispatch", texts[VIEW], 'function _s(t)',
             'Et("grok_bot_multiplayer") ? i.jsx(Cn, { ...t }) : i.jsx(Pn, { ...t })')
    contains("import-projection", texts[VIEW], 'jX as qs', 'jY as Is')
    contains("export-projection", texts[APP], 'Lme as jX', 'qKe as jY')
    mapping = section(APP, "function qKe(e, t)", "function jKe(e, t)")
    contains("exact-normalized-text-first-theme", mapping,
             'a.replace(/\\s+/g, " ").trim()',
             'r.has(c) || r.set(c, s[l] ?? "")', 'r.get(n(a.text))',
             'if (l === void 0) continue;', 'c.push(a)')
    modern = section(VIEW, "function Cn(t)", "function ka(t)")
    contains("fact-id-selection", modern, 'q.useState(A)', 'A = new Set()',
             'E.has(N.factId) && !G.has(N.factId)',
             'F ? te.add(we.factId) : te.delete(we.factId)',
             'const N = $e.map(ka)')
    contains("selection-is-not-done", modern,
             'isChecked: N.memories.some((F) => E.has(F.factId))',
             'isDone: N.memories.every((F) => G.has(F.factId))',
             'for (const je of he) te.add(je)')
    contains("processed-row-lock", texts[ROW], 'Je as h',
             'isAdded: b || h', 'isLocked: b',
             'if (t || i)', 'readOnly: !0')
    contains("owner-and-loaded-guards", modern,
             'le = !r && l != null && m?.viewerIsOwner !== !1',
             'if (!le || l == null || ye || !K) return;')
    contains("empty-list-auto-keep", modern,
             'isEmpty: L && l != null && l !== "" && Is(n, F).length === 0',
             'Y.current || ((Y.current = !0), me())',
             'resolution: { kind: "answered", value: Ht }, value: Ht')
    empty = section(VIEW, "function _n(t)", "function ja()")
    contains("empty-effect-needs-open-actionable", empty,
             'n && l && s && d()', 'us(a)')
    contains("promotion-before-widget-receipt", modern,
             'Ae = await wa(() => h(l, he))',
             'C(F === Ns ? j : F), V(te)',
             'if ((C(F), V(te), w(new Set()), D({ kind: "idle" }), !L))',
             'Oe = we === 0 ? Ht : ml(we)',
             'R.send({ resolution: { kind: "answered", value: Oe }, value: Oe })')
    assert modern.index('Ae = await wa(() => h(l, he))') < modern.index('Oe = we === 0 ? Ht : ml(we)')
    checks.append("mutation-receipt-order")
    tap = section(VIEW, "function Kt(t)", "function at(t)")
    contains("optimistic-widget-before-ack", tap,
             'o.record("widget", w, A.resolution), c()',
             'j = (C) => p({ agentId: s, entryId: n, value: C })',
             'if (C?.accepted !== !1) return;',
             'o.clear("widget", w)', 'f({ answer: A, fromStatus: r, onFail: E })')
    contains("retry-status-and-epoch", tap,
             'u !== null && u.fromStatus === r ? u : null',
             'B !== m.current', 'x(h.answer, h.onFail)')
    contains("retry-receipt-without-repromotion", modern,
             'if ($e.length === 0)', 'if (R.retry()) return;')
    legacy = section(VIEW, "function Pn(t)", "function Nn(t)")
    contains("legacy-optimism-rollback", legacy,
             'x ?? new Set(n.borderline ?? [])',
             'u.record("widget", O, { kind: "answered", value: C })',
             'u.clear("widget", O), a()', 'D?.accepted === !1 && U()')
    assert 'promoteToTeam' not in legacy and 'factId' not in legacy
    checks.append("legacy-no-fact-id-mutation")
    contains("legacy-index-decision", section(VIEW, "function Nn(t)", "function Ln(t)"),
             '[...O.keys()].filter((z) => !n.has(z))',
             'd(Ml(x, u))', 'd(Ht)')
    contains("prefix-only-display", section(APP, "function Lme(e, t)", "function KKe(e, t)"),
             't.value.startsWith(ey) || t.value.startsWith(iO)', '"shared"', '"kept"')
    contains("memory-resource-error-refresh", section(APP, "function GU(e)", "function VU(e)"),
             'lifetime: "revalidate"', 't.snapshotsFor(n).refresh(), r')
    contains("active-promotion-rpc", texts[MAIN],
             'promoteGrokBotMemoriesToTeam: ({ agentId: r, factIds: o })',
             '{ agentId: o, factIds: [...i] }',
             'keptPrivateCount: a.keptPrivateCount')
    contains("active-widget-rpc", section(MAIN, 'case "respondToWidget": {', 'case "submitSecret": {'),
             'await e.respondGrokBotWidget(', 'agentId: t.args.agentId',
             'entryId: t.args.entryId', 'value: t.args.value',
             'value: { accepted: r.accepted }')
    contains("widget-contract-kept-separate", texts[PROTO],
             'RespondGrokBotWidgetRequest|1 agent_id 9|2 entry_id 9|3 value 9|4 session_id 9',
             'RespondGrokBotWidgetResponse|1 accepted 8|2 refusal #0?',
             'PromoteGrokBotMemoriesToTeamResponse|1 created #0*|2 already_in_team #0*|3 kept_private_count 13')
    contains("local-answer-map", section(APP, 'class qIe {', 'const B3 ='),
             'byAgent = new Map()', 'record(t, n, s)', 'reset()', 'pruneForAgent(t, n)')
    assert len(ledger["findings"]) == 9
    assert {f["id"] for f in ledger["findings"]} == {f"MW{i:02d}" for i in range(1, 10)}
    for f in ledger["findings"]:
        assert f["anchors"] and f["limitation_fr"] and not f["runtime_observed"]
    assert digest((ROOT / ledger["report"]).read_bytes()) == ledger["report_sha256"]
    assert digest(Path(__file__).read_bytes()) == ledger["verifier_sha256"]
    assert ledger["target_execution_performed"] is False
    assert ledger["network_calls_performed"] is False
    assert len(ledger["derived_copies_created"]) == 1
    copy = ledger["derived_copies_created"][0]
    assert copy["relative_path"] == ROW
    assert copy["original_sha256"] == manifest[ROW]
    assert copy["formatted_sha256"] == sources[ROW]["formatted_sha256"]
    assert copy["command"] == ["npx", "--offline", "prettier@3.6.2",
                               "artifacts/client/" + ROW, "--ignore-path",
                               "/dev/null", "--end-of-line", "lf"]
    for rel, baseline in ledger["preexisting_formatted_baselines"].items():
        prior = json.loads((ROOT / baseline["prior_ledger"]).read_text())
        prior_source = next(s for s in prior["sources"] if s["id"] == rel)
        assert prior_source["formatted_sha256"] == baseline["formatted_sha256"]
        assert sources[rel]["formatted_sha256"] == baseline["formatted_sha256"]
    print(json.dumps({"status": "passed", "dedicated_static_checks": len(checks),
                      "sources": len(sources), "anchors": len(ledger["anchors"]),
                      "findings": len(ledger["findings"]),
                      "target_js_loaded_or_executed": False, "network_used": False}))


def build():
    """Rebuild only the assigned ledger from immutable, saved source passages."""
    helper = runpy.run_path(str(ROOT / "tools/evidence_ledger.py"))
    ledger = helper["Ledger"](
        "reports/memory-widget-consumer.md",
        "Suivi statique .70 du consommateur mémoire : sélection, mutation, projection, reçu, erreurs et relecture. .71 non requis.",
    )
    router = "dist/renderer/assets/index-D6ReTfRE.js"
    coordinator = "dist/node-agent-coordinator/main.cjs"
    specs = [
        ("loader", router, 123258, 123274, 'import("./chunk-view-BnlWmtVC.js")'),
        ("dispatch", VIEW, 5485, 5510, 'case "memories":'),
        ("gate", VIEW, 5677, 5683, 'Et("grok_bot_multiplayer") ? i.jsx(Cn, { ...t }) : i.jsx(Pn, { ...t })'),
        ("import", VIEW, 105, 113, 'jX as qs'),
        ("export", APP, 81403, 81412, 'Lme as jX'),
        ("match", APP, 47179, 47197, 'r.has(c) || r.set(c, s[l] ?? "")'),
        ("sets", VIEW, 3444, 3480, 'const [E, w] = q.useState(A)'),
        ("selection", VIEW, 3728, 3745, 'F ? te.add(we.factId) : te.delete(we.factId)'),
        ("group", VIEW, 3798, 3818, 'isDone: N.memories.every((F) => G.has(F.factId))'),
        ("lock", ROW, 1198, 1230, 'isLocked: b'),
        ("lock_effect", ROW, 1029, 1084, 'if (t || i)'),
        ("row_export", ROW, 1475, 1492, 'Je as h'),
        ("detail", VIEW, 3860, 3900, 'factId: N.factId'),
        ("projection", APP, 47199, 47214, 't.value.startsWith(ey) || t.value.startsWith(iO)'),
        ("tap", VIEW, 853, 916, 'o.record("widget", w, A.resolution), c()'),
        ("retry", VIEW, 918, 951, 'x(h.answer, h.onFail)'),
        ("locals", APP, 73217, 73280, 'class qIe {'),
        ("prune", APP, 48814, 48840, 'e.localAnswers.pruneForAgent(s, [o])'),
        ("promote", VIEW, 3690, 3740, 'Ae = await wa(() => h(l, he))'),
        ("resource", APP, 13067, 13088, 'throw (t.snapshotsFor(n).refresh(), r)'),
        ("ipc", MAIN, 108511, 108519, 'e.grokBot.plugins.promoteMemoriesToTeam(r, o)'),
        ("rpc", MAIN, 103051, 103068, '{ agentId: o, factIds: [...i] }'),
        ("wire", PROTO, 54712, 54810, 'PromoteGrokBotMemoriesToTeamResponse|1 created #0*|2 already_in_team #0*|3 kept_private_count 13'),
        ("result", VIEW, 3942, 4024, 'keptPrivateCount: t.keptPrivateCount + e.keptPrivateCount'),
        ("empty", VIEW, 3480, 3521, 'Y.current || ((Y.current = !0), me())'),
        ("empty_effect", VIEW, 3924, 3945, 'n && l && s && d()'),
        ("empty_retry", VIEW, 3524, 3563, 'onClick: () => R.retry()'),
        ("reopen", VIEW, 3585, 3618, 'F = le && !N && (L || _e)'),
        ("owner", APP, 3495, 3520, 'return t && q6(e) && e?.isGroup !== !0'),
        ("load", VIEW, 3460, 3476, '$ = ui(L || d ? l : null)'),
        ("hook", VIEW, 206, 241, 'r = () => l?.refresh()'),
        ("legacy", VIEW, 5690, 5723, 'x ?? new Set(n.borderline ?? [])'),
        ("legacy_send", VIEW, 5808, 5836, 'D?.accepted === !1 && U()'),
        ("positions", VIEW, 5863, 5909, '[...O.keys()].filter((z) => !n.has(z))'),
        ("buttons", VIEW, 5994, 6050, 'A = () => d(Ml(x, u))'),
        ("encoders", APP, 47134, 47168, 'return e.length === t ? ey : `${yg}${e.join(",")}`'),
        ("widget_rpc", MAIN, 110735, 110753, 'await e.respondGrokBotWidget('),
        ("widget_wire", PROTO, 56080, 56133, 'RespondGrokBotWidgetResponse|1 accepted 8|2 refusal #0?'),
        ("widget_args", coordinator, 39342, 39348, 'args: x({ entryId: u(), value: u(), agentId: u() })'),
        ("action", APP, 12208, 12230, 'respondToWidget: t.respondToWidget'),
        ("delegate", APP, 69998, 70005, 'respondToWidget: (A) => t.respondToWidget(A)'),
        ("map_create", APP, 73675, 73708, 'u = new qIe()'),
    ]
    anchors = {name: ledger.excerpt(rel, start, end, literal)
               for name, rel, start, end, literal in specs}
    findings = [
        ("MW01", "Consommateur lazy et binding de Lme résolus pour Cn/Pn selon le gate.",
         "loader dispatch gate import export", "Gate effectif non observé ; raccordement déjà partiellement établi par RC02–RC04.", "corroboration"),
        ("MW02", "Pn sélectionne des positions ; Cn démarre vide et sélectionne des factIds par égalité de texte normalisé, pouvant raccorder plusieurs faits identiques.",
         "match sets selection positions legacy encoders", "Données distantes ; pas de classement sémantique ni de conversion universelle index→ID.", "new_detail"),
        ("MW03", "isDone vient des IDs traités, y compris gardés privés, et verrouille la coche. Gardes et détail ne prouvent pas une autorisation distante.",
         "group promote lock lock_effect row_export sets detail legacy_send", "ACL serveur absentes ; absence de deuxième confirmation bornée aux handlers cités.", "new_detail"),
        ("MW04", "shared dérive d’une réponse optimiste avant reçu ; map locale dont le pruning reconnaît send-message, pas team-shared.",
         "projection tap legacy_send locals prune map_create", "Pas de stockage durable démontré ; projection sans preuve d’écriture ni d’oubli.", "new_detail"),
        ("MW05", "Promotion active par factIds avant reçu ; celui-ci contient created+alreadyInTeam ou keep si zéro, sans faits.",
         "promote resource ipc rpc wire result encoders", "Résultat RPC distinct d’une transaction commune ou d’une relecture de l’équipe.", "new_detail"),
        ("MW06", "Succès partiels conservés ; reçu rejeté retire la résolution locale, sans annuler la promotion. Retry peut renvoyer seulement le reçu.",
         "promote tap retry result", "Reprise dans cette instance ; pas de journal durable ni garantie après réponse perdue/remontage.", "new_detail"),
        ("MW07", "Carte proposée ouverte/actionnable sans faits raccordés : envoi automatique possible de memories:keep une fois dans l’instance.",
         "empty empty_effect empty_retry match hook", "Aucun fait raccordé n’implique pas une liste de mémoire vide ; aucun runtime observé.", "new_detail"),
        ("MW08", "Réouverture résolue sur bot Temporal d’équipe propriétaire ; promotion hors proposed sans nouveau reçu. GU rafraîchit explicitement sur erreur, pas sur succès.",
         "reopen owner load promote resource hook", "Autres relectures possibles par lifecycle/reconnexion ; visibilité serveur non observée.", "new_detail"),
        ("MW09", "RespondGrokBotWidget reçoit agentId, entryId et value ; sessionId partagé n’est pas renseigné dans ce chemin.",
         "projection action delegate widget_rpc widget_wire widget_args", "Acceptation RPC distincte du réveil, du tour et du rechargement de mémoire par le modèle.", "new_detail"),
    ]
    for ident, claim, names, limit, novelty in findings:
        ledger.finding(ident, claim, [anchors[n] for n in names.split()], limit)
        ledger.findings[-1]["novelty"] = novelty
    questions = [
        ("Consommateur lazy de Lme", "answered-client-side", ["MW01"]),
        ("Sélection index / factIds", "answered-client-side", ["MW02"]),
        ("Promotion ou réponse widget", "answered-client-side", ["MW05", "MW09"]),
        ("Affichage shared / kept / done", "answered-client-side", ["MW03", "MW04"]),
        ("Droits et confirmations", "partially_answered", ["MW03"]),
        ("Erreurs, retry et relecture", "answered-client-side", ["MW06", "MW07", "MW08"]),
        ("Réponse consommée par le bot", "partially_answered", ["MW09"]),
        ("Écriture réelle et rétention", "unresolved", ["MW05", "MW09"]),
    ]
    checklist = [{"question": q, "status": s, "findings": f} for q, s, f in questions]
    ledger.write(str(DEFAULT.relative_to(ROOT)), checklist, [{
        "scope": "Seven cited original/LF files in 0.70.2; no 0.71 comparison.",
        "patterns": ["Lme", "memories:share", "memories:promoted:", "respondToWidget", "promoteToTeam", "isDone", "pruneForAgent"],
        "negative_boundaries_fr": [
            "Pas de promotion factId dans Pn.",
            "Pas de seconde confirmation dans les handlers Cn/Pn cités.",
            "Pas de sessionId transmis par le handler widget cité.",
            "Pas de refresh explicite sur succès dans GU.promoteToTeam.",
        ],
    }])
    data = json.loads(DEFAULT.read_text())
    source = next(s for s in data["sources"] if s["id"] == ROW)
    baselines = {}
    for prior_path in ["evidence/server-context-residual.json", "evidence/goal-context-memory-learning.json"]:
        old = json.loads((ROOT / prior_path).read_text())
        assert old["rea_parent"] == data["rea_parent"]
        for s in old["sources"]:
            current = next((n for n in data["sources"] if n["id"] == s["id"]), None)
            if current is not None:
                assert s["original_sha256"] == current["original_sha256"]
                assert s["formatted_sha256"] == current["formatted_sha256"]
                baselines[s["id"]] = {"prior_ledger": prior_path, "formatted_sha256": s["formatted_sha256"]}
    data.update({
        "named_anchors": anchors,
        "derived_copies_created": [{
            "relative_path": ROW, "original_sha256": source["original_sha256"],
            "formatted_sha256": source["formatted_sha256"], "created_only_if_absent": True,
            "command": ["npx", "--offline", "prettier@3.6.2", "artifacts/client/" + ROW,
                        "--ignore-path", "/dev/null", "--end-of-line", "lf"],
            "line_endings": "LF", "formatter_stderr": "",
        }],
        "comparison_version_071_used": False, "target_js_loaded_or_executed": False,
        "rea_bundle_path": "evidence/rea-bundle.json", "rea_parent_reused": True,
        "prior_findings_read": ["reports/goal-context-memory-learning.md CM09–CM10", "reports/server-context-residual.md RC02–RC04"],
        "report_sha256": digest((ROOT / data["report"]).read_bytes()),
        "verifier_path": "tools/verify_memory_widget_consumer.py",
        "verifier_sha256": digest(Path(__file__).read_bytes()),
        "preexisting_formatted_baselines": baselines,
    })
    DEFAULT.write_text(json.dumps(data, ensure_ascii=False, indent=2) + "\n")


if __name__ == "__main__":
    if sys.argv[1:] == ["--rebuild-ledger"]:
        build()
        verify()
    else:
        verify(Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else DEFAULT)
