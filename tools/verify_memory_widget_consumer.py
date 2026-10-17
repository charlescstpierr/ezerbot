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
    print(json.dumps({"status": "passed", "dedicated_static_checks": len(checks),
                      "sources": len(sources), "anchors": len(ledger["anchors"]),
                      "findings": len(ledger["findings"]),
                      "target_js_loaded_or_executed": False, "network_used": False}))


if __name__ == "__main__":
    verify(Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else DEFAULT)
