#!/usr/bin/env python3
"""Linux proof: verdict rules, prerequisites report, cleanup of the run folder."""

from __future__ import annotations

import hashlib
import json
import os
import struct
import subprocess
import sys
import tempfile
import unittest
import zlib
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import doctor  # noqa: E402
import linux_verdict  # noqa: E402

SCRIPTS = Path(__file__).resolve().parent
SHA = "a" * 40


def png(width: int = 4, height: int = 3, raw: bytes | None = None) -> bytes:
    def chunk(kind: bytes, data: bytes) -> bytes:
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    if raw is None:
        raw = b"".join(b"\x00" + b"\x00" * width * 3 for _ in range(height))
    ihdr = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b"")


class VerdictTests(unittest.TestCase):
    def evidence(self, *, error: str | None = None, failing: str | None = None, manifest: bool = True, omit: str | None = None) -> Path:
        root = Path(tempfile.mkdtemp(prefix="linux-verdict-"))
        self.addCleanup(lambda: subprocess.run(["rm", "-rf", str(root)], check=True))
        captures = root / "captures"
        captures.mkdir()
        features = []
        views = []
        for feature, view in linux_verdict.EXPECTED:
            status = "fail" if view == failing else "pass"
            if status == "pass":
                (captures / f"{view}.png").write_bytes(png())
                views.append({"id": view, "file": f"{view}.png", "sha256": hashlib.sha256(png()).hexdigest(), "size": len(png())})
            features.append({"feature": feature, "view": view, "file": f"{view}.png", "status": status})
        features = [item for item in features if item["view"] != omit]
        views = [item for item in views if item["id"] != omit]
        (captures / "result.json").write_text(json.dumps({"sha": SHA, "error": error, "features": features}))
        if manifest:
            (captures / "manifest.json").write_text(json.dumps({"sha": SHA, "renderer": "r", "chrome_version": "c", "views": views}))
        return root

    def test_all_features_captured_is_pass(self) -> None:
        status, lines = linux_verdict.verdict(self.evidence(), SHA)
        self.assertEqual(status, "PASS")
        self.assertEqual(lines[-1], f"PASS {SHA}")

    def test_broken_feature_is_fix_and_keeps_earlier_captures(self) -> None:
        evidence = self.evidence(error="settings-connectors readiness failed.", failing="settings-connectors")
        status, lines = linux_verdict.verdict(evidence, SHA)
        self.assertEqual(status, "FIX")
        self.assertEqual(lines[-1], f"FIX {SHA}")
        self.assertIn("| `accueil` | `home-dark` | `captures/home-dark.png` | ok |", lines)
        self.assertIn("| `parametres-connecteurs` | `settings-connectors` | `captures/settings-connectors.png` | ÉCHEC |", lines)
        self.assertIn("Entrées : `accueil` ok, `reponse-fil` ok, `parametres-connecteurs` ÉCHEC", lines)

    def test_missing_or_unreadable_manifest_marks_nothing_ok(self) -> None:
        for broken in ("absent", "illisible"):
            with self.subTest(broken=broken):
                evidence = self.evidence(manifest=broken == "illisible")
                if broken == "illisible":
                    (evidence / "captures" / "manifest.json").write_text('{"views": [')
                status, lines = linux_verdict.verdict(evidence, SHA)
                self.assertEqual(status, "FIX")
                self.assertIn("## Erreur d’outil", lines)
                self.assertIn("`manifest.json` absent ou illisible : captures non vérifiées.", lines)
                rows = [line for line in lines if line.startswith("| `")]
                self.assertEqual(len(rows), len(linux_verdict.EXPECTED))
                self.assertTrue(all(row.endswith("| non vérifié |") for row in rows), rows)
                self.assertFalse(any(" ok" in line for line in lines if line.startswith(("| `", "Entrées"))), lines)

    def test_partial_manifest_verifies_earlier_views_and_never_passes(self) -> None:
        evidence = self.evidence(error="thread-menu readiness failed.", failing="thread-menu")
        path = evidence / "captures" / "manifest.json"
        manifest = json.loads(path.read_text())
        manifest["complete"] = False
        path.write_text(json.dumps(manifest))
        status, lines = linux_verdict.verdict(evidence, SHA)
        self.assertEqual(status, "FIX")
        self.assertIn("| `accueil` | `home-dark` | `captures/home-dark.png` | ok |", lines)
        self.assertIn("## Échec", lines)
        self.assertNotIn("## Erreur d’outil", lines)
        (evidence / "captures" / "result.json").write_text(json.dumps({"sha": SHA, "error": None, "features": [{"feature": f, "view": v, "file": f"{v}.png", "status": "pass"} for f, v in linux_verdict.EXPECTED]}))
        self.assertEqual(linux_verdict.verdict(evidence, SHA)[0], "FIX")

    def test_failed_driver_exit_is_fix_even_with_complete_captures(self) -> None:
        status, lines = linux_verdict.verdict(self.evidence(), SHA, drive_status=134)
        self.assertEqual(status, "FIX")
        self.assertIn("Le pilote a quitté avec le code 134.", lines)

    def test_missing_result_is_a_tool_error_fix(self) -> None:
        evidence = self.evidence()
        (evidence / "captures" / "result.json").unlink()
        status, lines = linux_verdict.verdict(evidence, SHA, drive_status=1)
        self.assertEqual(status, "FIX")
        self.assertIn("## Erreur d’outil", lines)

    def test_each_omitted_view_is_fix(self) -> None:
        for _, view in linux_verdict.EXPECTED:
            with self.subTest(view=view):
                status, lines = linux_verdict.verdict(self.evidence(omit=view), SHA)
                self.assertEqual(status, "FIX")
                self.assertTrue(any(line.endswith(f"`captures/{view}.png` | ÉCHEC |") for line in lines))

    def test_registry_matches_the_driver(self) -> None:
        listed = subprocess.run(
            ["node", "-e", "const m=require(process.argv[1]); console.log(JSON.stringify(m.selectedViews().map(v => [v.feature, v.id])))", str(SCRIPTS / "linux_drive.cjs")],
            capture_output=True, text=True, check=True,
        )
        self.assertEqual([tuple(item) for item in json.loads(listed.stdout)], list(linux_verdict.EXPECTED))

    def test_truncated_result_is_a_tool_error_fix(self) -> None:
        evidence = self.evidence()
        (evidence / "captures" / "result.json").write_text('{"sha": "' + SHA[:10])
        status, lines = linux_verdict.verdict(evidence, SHA)
        self.assertEqual(status, "FIX")
        self.assertIn("## Erreur d’outil", lines)
        self.assertIn("`result.json` illisible (JSONDecodeError).", lines)

    def test_truncated_manifest_is_a_tool_error_fix(self) -> None:
        evidence = self.evidence()
        (evidence / "captures" / "manifest.json").write_text('{"views": [')
        status, lines = linux_verdict.verdict(evidence, SHA)
        self.assertEqual(status, "FIX")
        self.assertIn("`manifest.json` illisible (JSONDecodeError).", lines)

    def test_malformed_manifest_views_is_a_tool_error_fix(self) -> None:
        evidence = self.evidence()
        (evidence / "captures" / "manifest.json").write_text(json.dumps({"sha": SHA, "views": [1]}))
        self.assertEqual(linux_verdict.verdict(evidence, SHA)[0], "FIX")

    def test_patch_is_copied_on_the_verdict_line(self) -> None:
        status, lines = linux_verdict.verdict(self.evidence(), SHA, patch="c" * 40)
        self.assertEqual(lines[-1], f"PASS {SHA} patch={'c' * 40}")

    def rewrite_result(self, evidence: Path, change) -> None:
        path = evidence / "captures" / "result.json"
        data = json.loads(path.read_text())
        change(data["features"][0])
        path.write_text(json.dumps(data))

    def test_feature_without_status_is_a_tool_error_fix(self) -> None:
        evidence = self.evidence()
        self.rewrite_result(evidence, lambda item: item.pop("status"))
        status, lines = linux_verdict.verdict(evidence, SHA)
        self.assertEqual(status, "FIX")
        self.assertIn("`result.json` : liste `features` invalide.", lines)

    def test_non_string_identifier_is_a_tool_error_fix(self) -> None:
        for key in ("feature", "view"):
            with self.subTest(key=key):
                evidence = self.evidence()
                self.rewrite_result(evidence, lambda item: item.__setitem__(key, ["accueil"]))
                status, lines = linux_verdict.verdict(evidence, SHA)
                self.assertEqual(status, "FIX")
                self.assertIn("## Erreur d’outil", lines)

    def test_missing_or_mistyped_sha_is_a_tool_error_fix(self) -> None:
        for change in (lambda data: data.pop("sha"), lambda data: data.__setitem__("sha", None), lambda data: data.__setitem__("sha", [])):
            evidence = self.evidence()
            path = evidence / "captures" / "result.json"
            data = json.loads(path.read_text())
            change(data)
            path.write_text(json.dumps(data))
            status, lines = linux_verdict.verdict(evidence, SHA)
            self.assertEqual(status, "FIX")
            self.assertIn("`result.json` : `sha` absent ou invalide.", lines)

    def test_pathological_json_is_a_tool_error_fix(self) -> None:
        for text in ('{"sha": ' + "9" * 5000 + "}", "[" * 10000 + "]" * 10000):
            evidence = self.evidence()
            (evidence / "captures" / "result.json").write_text(text)
            status, lines = linux_verdict.safe_verdict(evidence, SHA)
            self.assertEqual(status, "FIX")
            self.assertEqual(lines[-1], f"FIX {SHA}")

    def test_unencodable_error_text_still_writes_the_report(self) -> None:
        evidence = self.evidence()
        path = evidence / "captures" / "result.json"
        data = json.loads(path.read_text())
        data["error"] = "\ud800"
        path.write_text(json.dumps(data))
        status, lines = linux_verdict.safe_verdict(evidence, SHA)
        self.assertEqual(status, "FIX")
        written = linux_verdict.write_report(evidence / "report.md", lines).decode("utf-8")
        self.assertTrue(written.rstrip().endswith(f"FIX {SHA}"))

    def assert_mismatch_is_fix(self, data: bytes) -> None:
        # The capture step recorded sha256 + size of the bytes it wrote; any other bytes are a tool error.
        evidence = self.evidence()
        (evidence / "captures" / "home-dark.png").write_bytes(data)
        status, lines = linux_verdict.verdict(evidence, SHA)
        self.assertEqual(status, "FIX")
        self.assertIn("`captures/home-dark.png` ne correspond pas à l’empreinte du manifeste.", lines)
        self.assertIn("## Erreur d’outil", lines)

    def test_truncated_png_is_fix(self) -> None:
        self.assert_mismatch_is_fix(png()[:33])

    def test_altered_byte_is_fix(self) -> None:
        data = bytearray(png())
        data[-20] ^= 0x01
        self.assert_mismatch_is_fix(bytes(data))

    def test_png_without_iend_is_fix(self) -> None:
        data = png()
        self.assert_mismatch_is_fix(data[: data.rindex(b"IEND") - 4])

    def test_not_a_png_is_fix(self) -> None:
        self.assert_mismatch_is_fix(b"pas un png")

    def mutate_manifest(self, mutate) -> list[str]:
        evidence = self.evidence()
        path = evidence / "captures" / "manifest.json"
        manifest = json.loads(path.read_text())
        mutate(manifest["views"])
        path.write_text(json.dumps(manifest))
        status, lines = linux_verdict.verdict(evidence, SHA)
        self.assertEqual(status, "FIX")
        self.assertIn("## Erreur d’outil", lines)
        return lines

    def test_missing_hash_or_size_is_fix(self) -> None:
        for key in ("sha256", "size"):
            with self.subTest(key=key):
                self.mutate_manifest(lambda views, key=key: views[0].pop(key))

    def test_view_missing_from_manifest_is_fix(self) -> None:
        lines = self.mutate_manifest(lambda views: views.pop(0))
        self.assertIn("`captures/home-dark.png` ne correspond pas à l’empreinte du manifeste.", lines)

    def test_matching_hash_but_undecodable_image_is_fix(self) -> None:
        # Pillow stays a sanity check: bytes the capture step itself recorded must still decode.
        evidence = self.evidence()
        data = png(raw=b"")
        (evidence / "captures" / "home-dark.png").write_bytes(data)
        path = evidence / "captures" / "manifest.json"
        manifest = json.loads(path.read_text())
        manifest["views"][0].update(sha256=hashlib.sha256(data).hexdigest(), size=len(data))
        path.write_text(json.dumps(manifest))
        status, lines = linux_verdict.verdict(evidence, SHA)
        self.assertEqual(status, "FIX")
        self.assertTrue(any(line.startswith("`captures/home-dark.png` illisible (") for line in lines), lines)

    def test_hidden_or_offscreen_target_is_not_clickable(self) -> None:
        script = r"""
const { pointerTarget } = require(process.argv[1]);
globalThis.innerWidth = 1440; globalThis.innerHeight = 900;
const make = (box, visible = true, extra = {}) => { const el = { disabled: false, checkVisibility: () => visible, getBoundingClientRect: () => ({ ...box, right: box.left + box.width, bottom: box.top + box.height }), contains: node => node === el, ...extra }; return el; };
const box = { left: 10, top: 10, width: 100, height: 20 };
let hit = null;
globalThis.document = { elementFromPoint: () => hit };
const visible = make(box); hit = visible;
const out = { visible: pointerTarget(visible) };
hit = null;
out.hidden = pointerTarget(make(box, false));
out.zero = pointerTarget(make({ ...box, width: 0, height: 0 }));
out.offscreen = pointerTarget(make({ ...box, top: 2000 }));
out.covered = pointerTarget(make(box));
out.disabled = pointerTarget(make(box, true, { disabled: true }));
out.absent = pointerTarget(null);
console.log(JSON.stringify(out));
"""
        result = subprocess.run(["node", "-e", script, str(SCRIPTS.parents[3] / "tools" / "linux-captures.cjs")], capture_output=True, text=True, check=True)
        out = json.loads(result.stdout)
        self.assertEqual(out.pop("visible"), {"x": 60, "y": 20})
        self.assertEqual(out, dict.fromkeys(out))

    def test_driver_clicks_reply_in_thread_with_the_pointer(self) -> None:
        text = (SCRIPTS / "linux_drive.cjs").read_text()
        self.assertNotIn("'Répondre en fil').click()", text)
        result = subprocess.run(["node", "-e", "const m=require(process.argv[1]); console.log(JSON.stringify(m.DRIVEN.filter(v => v.click).map(v => v.id)))", str(SCRIPTS / "linux_drive.cjs")], capture_output=True, text=True, check=True)
        self.assertTrue({"thread-menu", "thread-open", "settings-connectors-add"} <= set(json.loads(result.stdout)))

    def test_transparent_target_is_not_clickable(self) -> None:
        # checkVisibility must be asked about opacity: an opacity:0 element is not a target.
        script = r"""
const { pointerTarget } = require(process.argv[1]);
globalThis.innerWidth = 1440; globalThis.innerHeight = 900;
const el = { disabled: false, checkVisibility: options => !options?.opacityProperty, getBoundingClientRect: () => ({ left: 10, top: 10, width: 100, height: 20, right: 110, bottom: 30 }), contains: () => true };
globalThis.document = { elementFromPoint: () => el };
console.log(JSON.stringify(pointerTarget(el)));
"""
        result = subprocess.run(["node", "-e", script, str(SCRIPTS.parents[3] / "tools" / "linux-captures.cjs")], capture_output=True, text=True, check=True)
        self.assertEqual(result.stdout.strip(), "null")

    def test_every_view_goes_through_the_visibility_helper(self) -> None:
        text = (SCRIPTS / "linux_drive.cjs").read_text()
        self.assertNotRegex(text, r"\.click\(")
        self.assertNotIn("getBoundingClientRect", text)
        result = subprocess.run(["node", "-e", "const m=require(process.argv[1]); console.log(JSON.stringify(m.selectedViews().map(v => ({id: v.id, click: v.click ?? null, visible: v.visible ?? null, script: (v.action ?? '') + (v.after ?? '')}))))", str(SCRIPTS / "linux_drive.cjs")], capture_output=True, text=True, check=True)
        for view in json.loads(result.stdout):
            with self.subTest(view=view["id"]):
                self.assertTrue(view["visible"], "vue sans contrôle de visibilité")
                self.assertNotIn(".click(", view["script"])
                self.assertNotIn(".click(", view["click"] or "")
        self.assertIn("ADD_MCP", text.split('id: "settings-connectors-add"')[1].split("},")[0])

    def test_transparent_settings_target_is_fix(self) -> None:
        evidence = self.evidence(error="settings-connectors-add target not visible.", failing="settings-connectors-add")
        status, lines = linux_verdict.verdict(evidence, SHA)
        self.assertEqual(status, "FIX")
        self.assertIn("| `parametres-connecteurs` | `settings-connectors-add` | `captures/settings-connectors-add.png` | ÉCHEC |", lines)

    PAGES = {
        "home-dark": {"#conversation-title": ["Nouvelle conversation"], "#welcome": ["Bienvenue"], ".gb-editor": [""]},
        "conversation-dark": {"#conversation-title": ["Préparer la semaine"], "#chat-scroll .gb-user": ["Bonjour, prépare une courte liste pour organiser ma semaine."], "#chat-scroll .gb-assistant": ["Voici votre document."]},
        "thread-reply": {"#thread-banner-quote": ["Bonjour, prépare une courte liste"], "#chat-scroll .gb-user": ["Et en un mot ?"], "#chat-scroll .gb-assistant": ["Voici votre document."]},
        "settings-connectors": {"#settings-dialog[open] [data-panel=mcp] h3": ["Connecteurs"], "#settings-dialog[open] [data-panel=mcp] summary": ["Ajouter un serveur MCP"]},
    }

    def ready(self, view: str, page: dict, hidden: str | None = None) -> bool:
        # Evaluates the driver's real `ready` expression against a fake page; `hidden` is rendered display:none
        # (checkVisibility false, innerText empty) while its textContent would still hold the text.
        script = r"""
const [drive, view, page, hidden] = [require(process.argv[1]), process.argv[2], JSON.parse(process.argv[3]), process.argv[4]];
globalThis.innerWidth = 1440; globalThis.innerHeight = 900;
const node = (selector, text) => ({ textContent: text, innerText: selector === hidden ? "" : text, checkVisibility: () => selector !== hidden,
  getBoundingClientRect: () => selector === hidden ? { left: 0, top: 0, width: 0, height: 0, right: 0, bottom: 0 } : { left: 10, top: 10, width: 100, height: 20, right: 110, bottom: 30 } });
globalThis.document = { querySelectorAll: selector => (page[selector] ?? []).map(text => node(selector, text)) };
globalThis.GrokbotPanels = { state: { conversations: [] } };
const found = drive.selectedViews().find(item => item.id === view);
console.log(JSON.stringify(Boolean(eval(found.ready))));
"""
        result = subprocess.run(["node", "-e", script, str(SCRIPTS / "linux_drive.cjs"), view, json.dumps(page), hidden or ""], capture_output=True, text=True, check=True)
        return json.loads(result.stdout)

    def test_visible_content_is_ready_and_hidden_content_is_not(self) -> None:
        cases = {"thread-reply": "#chat-scroll .gb-assistant", "conversation-dark": "#chat-scroll .gb-assistant", "settings-connectors": "#settings-dialog[open] [data-panel=mcp] summary", "home-dark": "#welcome"}
        for view, hidden in cases.items():
            with self.subTest(view=view):
                self.assertTrue(self.ready(view, self.PAGES[view]))
                self.assertFalse(self.ready(view, self.PAGES[view], hidden), f"{view} prêt avec {hidden} masqué")

    def test_hidden_bot_reply_is_fix(self) -> None:
        for view in ("thread-reply", "conversation-dark", "settings-connectors"):
            with self.subTest(view=view):
                feature = dict((v, f) for f, v in linux_verdict.EXPECTED)[view]
                evidence = self.evidence(error=f"{view} readiness failed. {view} readiness timed out.", failing=view)
                status, lines = linux_verdict.verdict(evidence, SHA)
                self.assertEqual(status, "FIX")
                self.assertIn(f"| `{feature}` | `{view}` | `captures/{view}.png` | ÉCHEC |", lines)

    def test_no_view_asserts_on_hidden_or_page_wide_text(self) -> None:
        text = (SCRIPTS / "linux_drive.cjs").read_text()
        for forbidden in ("textContent", "innerHTML", "outerHTML", "document.body", "document.documentElement"):
            self.assertNotIn(forbidden, text)
        result = subprocess.run(["node", "-e", "const m=require(process.argv[1]); console.log(JSON.stringify(m.DRIVEN.map(v => [v.id, v.ready])))", str(SCRIPTS / "linux_drive.cjs")], capture_output=True, text=True, check=True)
        for view, ready in json.loads(result.stdout):
            with self.subTest(view=view):
                self.assertIn("expectVisibleText(", ready)
                self.assertNotRegex(ready, r"getElementById\(|querySelector\(|\.innerText|\.hidden")

    def test_hidden_reply_target_is_fix(self) -> None:
        # What the driver writes when the « Répondre en fil » item never becomes visible.
        evidence = self.evidence(error="thread-open target not visible.", failing="thread-open")
        status, lines = linux_verdict.verdict(evidence, SHA)
        self.assertEqual(status, "FIX")
        self.assertIn("| `reponse-fil` | `thread-open` | `captures/thread-open.png` | ÉCHEC |", lines)
        self.assertIn("`thread-open target not visible.`", lines)

    def assert_features_rejected(self, mutate) -> None:
        evidence = self.evidence()
        path = evidence / "captures" / "result.json"
        result = json.loads(path.read_text())
        mutate(result["features"])
        path.write_text(json.dumps(result))
        status, lines = linux_verdict.verdict(evidence, SHA)
        self.assertEqual(status, "FIX")
        self.assertIn("`result.json` : liste `features` invalide.", lines)

    def test_duplicate_feature_entry_is_fix(self) -> None:
        self.assert_features_rejected(lambda items: items.append(dict(items[0])))

    def test_duplicate_entry_with_other_status_is_fix(self) -> None:
        self.assert_features_rejected(lambda items: items.insert(0, dict(items[0], status="fail")))

    def test_unknown_status_is_fix(self) -> None:
        self.assert_features_rejected(lambda items: items[0].update(status="ok"))

    def test_tampered_capture_is_fix(self) -> None:
        evidence = self.evidence()
        (evidence / "captures" / "home-dark.png").write_bytes(png(5, 3))
        self.assertEqual(linux_verdict.verdict(evidence, SHA)[0], "FIX")

    def test_malformed_entries_are_fix(self) -> None:
        for mutate in (lambda items: items[0].pop("status"), lambda items: items[0].update(feature=["accueil"]), lambda items: items.append("x")):
            with self.subTest(mutate=mutate):
                self.assert_features_rejected(mutate)

    def test_other_commit_is_refused(self) -> None:
        with self.assertRaises(SystemExit):
            linux_verdict.verdict(self.evidence(), "b" * 40)

    def test_mapped_features_exist_in_the_feature_map(self) -> None:
        for name in ("accueil", "reponse-fil", "parametres-connecteurs"):
            self.assertTrue((linux_verdict.FEATURES_DIR / f"{name}.md").is_file(), name)


@unittest.skipUnless(sys.platform.startswith("linux"), "prérequis Linux")
class LinuxPrerequisitesTests(unittest.TestCase):
    def test_missing_tool_is_reported(self) -> None:
        env = {**os.environ, "PATH": "/nonexistent", "EZERBOT_VERIFY_CARGO": "/nonexistent/cargo"}
        result = subprocess.run([sys.executable, str(SCRIPTS / "doctor.py"), "--prerequisites"], env=env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 1)
        self.assertIn("echec  xvfb-run", result.stdout)
        self.assertIn("echec  cargo: /nonexistent/cargo", result.stdout)
        self.assertEqual(result.stdout.strip().splitlines()[-1], "doctor: echec")

    def test_no_mac_path_is_needed(self) -> None:
        text = (SCRIPTS / "prove-linux.sh").read_text() + (SCRIPTS / "linux_drive.cjs").read_text()
        self.assertNotIn("/Users", text)
        self.assertNotIn("/Applications", text)

    def test_explicit_chrome_is_honored(self) -> None:
        scratch = Path(tempfile.mkdtemp(prefix="linux-chrome-"))
        self.addCleanup(lambda: subprocess.run(["rm", "-rf", str(scratch)], check=True))
        chrome = scratch / "mon-chrome"
        chrome.write_text("#!/bin/sh\n")
        chrome.chmod(0o755)
        for variable in ("EZERBOT_VERIFY_CHROME", "CHROME_BIN"):
            with self.subTest(variable=variable):
                env = {key: value for key, value in os.environ.items() if key not in ("EZERBOT_VERIFY_CHROME", "CHROME_BIN")}
                env[variable] = str(chrome)
                result = subprocess.run([sys.executable, str(SCRIPTS / "doctor.py"), "--chrome"], env=env, capture_output=True, text=True)
                self.assertEqual(result.stdout.strip(), str(chrome))
                env[variable] = str(scratch / "absent")
                result = subprocess.run([sys.executable, str(SCRIPTS / "doctor.py"), "--prerequisites"], env=env, capture_output=True, text=True)
                self.assertIn("echec  chrome:", result.stdout)

    def test_launcher_symlink_keeps_its_own_name(self) -> None:
        # Snap launchers (/snap/bin/chromium -> /usr/bin/snap) dispatch on the name they are called by.
        scratch = Path(tempfile.mkdtemp(prefix="linux-snap-"))
        self.addCleanup(lambda: subprocess.run(["rm", "-rf", str(scratch)], check=True))
        target = scratch / "snap"
        target.write_text("#!/bin/sh\n")
        target.chmod(0o755)
        (scratch / "chromium").symlink_to(target)
        env = {key: value for key, value in os.environ.items() if key not in ("EZERBOT_VERIFY_CHROME", "CHROME_BIN")}
        env["PATH"] = f"{scratch}:/usr/bin:/bin"
        env["EZERBOT_VERIFY_CHROME"] = str(scratch / "chromium")
        result = subprocess.run([sys.executable, str(SCRIPTS / "doctor.py"), "--chrome"], env=env, capture_output=True, text=True)
        self.assertEqual(result.stdout.strip(), str(scratch / "chromium"))

    def test_doctor_lists_prerequisites(self) -> None:
        names = {line.split(":")[0].split()[-1] for line in subprocess.run(
            [sys.executable, str(SCRIPTS / "doctor.py"), "--prerequisites"], capture_output=True, text=True
        ).stdout.splitlines() if ":" in line}
        self.assertTrue({"xvfb-run", "Xvfb", "xauth", "chrome", "node-websocket", "cargo", "disque"} <= names)

    def test_evidence_inside_repo_is_refused(self) -> None:
        result = subprocess.run(["bash", str(SCRIPTS / "prove-linux.sh"), "--evidence", str(SCRIPTS.parents[3] / "inside")], capture_output=True, text=True)
        self.assertEqual(result.returncode, 2)
        self.assertIn("hors du dépôt", result.stderr)

    def test_equals_form_selects_the_same_path(self) -> None:
        env = {key: value for key, value in os.environ.items() if key != "EZERBOT_VERIFY_BROWSER"}
        inside = str(SCRIPTS.parents[3] / "inside")
        for args in (["--sphere=linux", f"--evidence={inside}"], [f"--evidence={inside}"], ["--sphere", "linux", f"--evidence={inside}"]):
            with self.subTest(args=args):
                result = subprocess.run(["bash", str(SCRIPTS / "prove.sh"), *args], env=env, capture_output=True, text=True)
                self.assertEqual(result.returncode, 2)
                self.assertIn("hors du dépôt", result.stderr)

    def test_unknown_or_empty_option_is_refused(self) -> None:
        for args in (["--evidance", "/tmp/x"], ["--verbose"], ["accueil"], ["--sphere"], ["--sphere="]):
            with self.subTest(args=args):
                result = subprocess.run(["bash", str(SCRIPTS / "prove.sh"), *args], capture_output=True, text=True, timeout=30)
                self.assertEqual(result.returncode, 2)
                self.assertIn("prove.sh:", result.stderr)
                self.assertIn("Usage", result.stderr)

    def test_all_and_named_spheres_keep_the_prove_py_path(self) -> None:
        # `all` (and every existing sphere) must behave as on main: prove.py, not the Xvfb subset.
        env = {key: value for key, value in os.environ.items() if key != "EZERBOT_VERIFY_BROWSER"}
        env["EZERBOT_VERIFY_CARGO"] = "/nonexistent/cargo"
        for sphere in ("all", "accueil"):
            with self.subTest(sphere=sphere):
                scratch = Path(tempfile.mkdtemp(prefix="linux-all-"))
                self.addCleanup(lambda path=scratch: subprocess.run(["rm", "-rf", str(path)], check=True))
                result = subprocess.run(["bash", str(SCRIPTS / "prove.sh"), "--sphere", sphere, "--evidence", str(scratch / "ev")], env=env, capture_output=True, text=True, timeout=120)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("cargo introuvable: /nonexistent/cargo", result.stdout + result.stderr)
                self.assertTrue((scratch / "ev" / "stand-in.md").is_file())

    def test_sphere_linux_and_default_use_the_xvfb_path(self) -> None:
        env = {key: value for key, value in os.environ.items() if key != "EZERBOT_VERIFY_BROWSER"}
        for extra in (["--sphere", "linux"], []):
            with self.subTest(extra=extra):
                result = subprocess.run(["bash", str(SCRIPTS / "prove.sh"), *extra, "--evidence", str(SCRIPTS.parents[3] / "inside")], env=env, capture_output=True, text=True)
                self.assertEqual(result.returncode, 2)
                self.assertIn("hors du dépôt", result.stderr)

    def test_uncovered_sphere_is_refused_by_the_xvfb_path(self) -> None:
        for sphere in ("modeles", "all"):
            result = subprocess.run(["bash", str(SCRIPTS / "prove-linux.sh"), "--sphere", sphere], capture_output=True, text=True)
            self.assertEqual(result.returncode, 2)
            self.assertIn("seul « linux »", result.stderr)

    def test_explicit_patch_uses_the_linux_path(self) -> None:
        env = {key: value for key, value in os.environ.items() if key != "EZERBOT_VERIFY_BROWSER"}
        result = subprocess.run(["bash", str(SCRIPTS / "prove.sh"), "--patch", "d" * 40, "--evidence", str(SCRIPTS.parents[3] / "inside")], env=env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 2)
        self.assertIn("hors du dépôt", result.stderr)

    def test_patch_from_environment_is_honored(self) -> None:
        env = {key: value for key, value in os.environ.items() if key != "EZERBOT_VERIFY_BROWSER"}
        env["EZERBOT_VERIFY_PATCH_ID"] = "pas-un-patch"
        result = subprocess.run(["bash", str(SCRIPTS / "prove.sh")], env=env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 2)
        self.assertIn("patch-id invalide", result.stderr)

    def failing_build(self, tool: str, extra_env: dict[str, str]) -> tuple[subprocess.CompletedProcess[str], Path]:
        repo = SCRIPTS.parents[3]
        dirty = subprocess.run(["git", "-C", str(repo), "status", "--porcelain", "--", "crates", "app/ui", "app/web", "Cargo.toml", "Cargo.lock", "vendor", "tools/linux-captures.cjs", ".cursor/skills/verify-ezerbot/scripts"], capture_output=True, text=True, check=True).stdout
        if dirty:
            self.skipTest("checkout modifié : prove-linux.sh refuse avant le build")
        scratch = Path(tempfile.mkdtemp(prefix="linux-build-"))
        self.addCleanup(lambda: subprocess.run(["rm", "-rf", str(scratch)], check=True))
        fake = scratch / "bin"
        fake.mkdir()
        (fake / tool).write_text("#!/bin/sh\nexit 7\n")
        (fake / tool).chmod(0o755)
        env = {key: value for key, value in os.environ.items() if key != "EZERBOT_VERIFY_BROWSER"}
        env.update(extra_env)
        env["PATH"] = f"{fake}:{env['PATH']}"
        evidence = scratch / "evidence"
        result = subprocess.run(["bash", str(SCRIPTS / "prove.sh"), "--evidence", str(evidence)], env=env, capture_output=True, text=True, timeout=600)
        return result, evidence

    def test_busy_lock_is_refused_at_once_and_cleans_up(self) -> None:
        import fcntl

        repo = SCRIPTS.parents[3]
        dirty = subprocess.run(["git", "-C", str(repo), "status", "--porcelain", "--", "crates", "app/ui", "app/web", "Cargo.toml", "Cargo.lock", "vendor", "tools/linux-captures.cjs", ".cursor/skills/verify-ezerbot/scripts"], capture_output=True, text=True, check=True).stdout
        if dirty:
            self.skipTest("checkout modifié : prove-linux.sh refuse avant le verrou")
        target = repo / "target" / "verify-linux"
        target.mkdir(parents=True, exist_ok=True)
        holder = open(target / ".verify-build.lock", "w")
        self.addCleanup(holder.close)
        fcntl.flock(holder, fcntl.LOCK_EX | fcntl.LOCK_NB)
        scratch = Path(tempfile.mkdtemp(prefix="linux-lock-"))
        self.addCleanup(lambda: subprocess.run(["rm", "-rf", str(scratch)], check=True))
        before = set(Path("/tmp").glob("ezv.*"))
        result = subprocess.run(["bash", str(SCRIPTS / "prove-linux.sh"), "--evidence", str(scratch / "ev")], capture_output=True, text=True, timeout=60)
        self.assertEqual(result.returncode, 2)
        self.assertIn("un autre run tient", result.stderr)
        self.assertEqual(set(Path("/tmp").glob("ezv.*")) - before, set())

    def test_failed_web_build_writes_a_fix_report_with_the_patch(self) -> None:
        result, evidence = self.failing_build("npm", {"EZERBOT_VERIFY_PATCH_ID": "e" * 40})
        self.assertEqual(result.returncode, 1, result.stderr)
        report = (evidence / "report.md").read_text().splitlines()
        sha = subprocess.run(["git", "-C", str(SCRIPTS), "rev-parse", "HEAD"], capture_output=True, text=True, check=True).stdout.strip()
        self.assertEqual(report[-1], f"FIX {sha} patch={'e' * 40}")
        self.assertIn("L’étape `npm ci` a échoué (code 7) ; aucune vue n’a été pilotée.", report)

    def test_failed_cargo_build_writes_a_fix_report(self) -> None:
        scratch = Path(tempfile.mkdtemp(prefix="linux-cargo-"))
        self.addCleanup(lambda: subprocess.run(["rm", "-rf", str(scratch)], check=True))
        cargo = scratch / "cargo"
        cargo.write_text("#!/bin/sh\nexit 9\n")
        cargo.chmod(0o755)
        result, evidence = self.failing_build("unused-tool", {"EZERBOT_VERIFY_CARGO": str(cargo)})
        self.assertEqual(result.returncode, 1, result.stderr)
        report = (evidence / "report.md").read_text().splitlines()
        self.assertTrue(report[-1].startswith("FIX "))
        self.assertIn("L’étape `cargo build` a échoué (code 9) ; aucune vue n’a été pilotée.", report)

    def test_used_evidence_folder_is_refused(self) -> None:
        used = Path(tempfile.mkdtemp(prefix="linux-used-"))
        self.addCleanup(lambda: subprocess.run(["rm", "-rf", str(used)], check=True))
        (used / "captures").mkdir()
        (used / "captures" / "result.json").write_text("{}")
        result = subprocess.run(["bash", str(SCRIPTS / "prove-linux.sh"), "--evidence", str(used)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 2)
        self.assertIn("déjà utilisé", result.stderr)

    def assert_dead(self, pid: int) -> None:
        state = Path(f"/proc/{pid}/stat")
        alive = state.exists() and state.read_text().split(") ")[-1][0] != "Z"
        self.assertFalse(alive, f"pid {pid} vivant")

    def test_failed_step_kills_its_descendants_and_removes_the_run_folder(self) -> None:
        scratch = Path(tempfile.mkdtemp(prefix="linux-lib-"))
        self.addCleanup(lambda: subprocess.run(["rm", "-rf", str(scratch)], check=True))
        script = (f'set -euo pipefail\nsource {SCRIPTS / "linux_run_lib.sh"}\necho "$run" > {scratch}/run\n'
                  f"step . bash -c 'sleep 300 & echo $! > {scratch}/pid; exit 7'\n")
        result = subprocess.run(["bash", "-c", script], capture_output=True, text=True, timeout=60)
        self.assertEqual(result.returncode, 7)
        self.assert_dead(int((scratch / "pid").read_text()))
        run = Path((scratch / "run").read_text().strip())
        self.assertTrue(run.name.startswith("ezv."))
        self.assertFalse(run.exists())
        self.assertIn(f"nettoyage: {run} supprimé", result.stdout)

    def test_detached_run_process_is_killed_after_the_step(self) -> None:
        scratch = Path(tempfile.mkdtemp(prefix="linux-lib-"))
        self.addCleanup(lambda: subprocess.run(["rm", "-rf", str(scratch)], check=True))
        spawn = ("import subprocess,sys; p=subprocess.Popen(['sleep','300'], start_new_session=True); "
                 "open(sys.argv[1],'w').write(str(p.pid))")
        script = (f'set -euo pipefail\nsource {SCRIPTS / "linux_run_lib.sh"}\necho "$run" > {scratch}/run\n'
                  f'step . env TMPDIR="$run/tmp" python3 -c "{spawn}" {scratch}/pid\n'
                  f'cat /proc/$(cat {scratch}/pid)/stat > {scratch}/after-step 2>/dev/null || true\n')
        result = subprocess.run(["bash", "-c", script], capture_output=True, text=True, timeout=60)
        self.assertEqual(result.returncode, 0, result.stderr)
        pid = int((scratch / "pid").read_text())
        self.assert_dead(pid)
        after = (scratch / "after-step").read_text()
        self.assertTrue(after == "" or after.split(") ")[-1][0] == "Z", "le processus détaché a survécu à l'étape")

    def test_detached_process_with_large_environment_is_killed_by_cleanup(self) -> None:
        # HOME first then ~1 MB of environment: an early-exiting `grep -q` would SIGPIPE `tr` under pipefail.
        scratch = Path(tempfile.mkdtemp(prefix="linux-lib-"))
        self.addCleanup(lambda: subprocess.run(["rm", "-rf", str(scratch)], check=True))
        spawn = ("import subprocess,sys; env={'HOME': sys.argv[2] + '/home', **{f'BIG{i}': 'x' * 100000 for i in range(10)}}; "
                 "p=subprocess.Popen(['sleep','300'], start_new_session=True, env=env); open(sys.argv[1],'w').write(str(p.pid))")
        script = (f'set -euo pipefail\nsource {SCRIPTS / "linux_run_lib.sh"}\n'
                  f'python3 -c "{spawn}" {scratch}/pid "$run"\nexit 0\n')
        result = subprocess.run(["bash", "-c", script], capture_output=True, text=True, timeout=60)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assert_dead(int((scratch / "pid").read_text()))

    def test_unrelated_process_is_left_alone(self) -> None:
        other = subprocess.Popen(["sleep", "300"], start_new_session=True)
        self.addCleanup(lambda: (other.kill(), other.wait()))
        script = f'set -euo pipefail\nsource {SCRIPTS / "linux_run_lib.sh"}\nstep . true\n'
        subprocess.run(["bash", "-c", script], capture_output=True, text=True, timeout=60, check=True)
        self.assertIsNone(other.poll())

    def test_interrupted_step_is_killed_by_the_trap(self) -> None:
        scratch = Path(tempfile.mkdtemp(prefix="linux-lib-"))
        self.addCleanup(lambda: subprocess.run(["rm", "-rf", str(scratch)], check=True))
        script = (f'set -euo pipefail\nsource {SCRIPTS / "linux_run_lib.sh"}\necho "$run" > {scratch}/run\n'
                  f"(sleep 1; kill -TERM $$) &\nstep . bash -c 'sleep 300 & echo $! > {scratch}/pid; wait'\n")
        result = subprocess.run(["bash", "-c", script], capture_output=True, text=True, timeout=60)
        self.assertEqual(result.returncode, 143)
        self.assert_dead(int((scratch / "pid").read_text()))
        self.assertFalse(Path((scratch / "run").read_text().strip()).exists())


if __name__ == "__main__":
    unittest.main()
