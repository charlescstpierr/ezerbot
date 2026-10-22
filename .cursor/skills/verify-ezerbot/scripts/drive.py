#!/usr/bin/env python3
"""Drive one or every mapped sphere through the browser harness.

Screenshots are PNG, dark and light, at 1140x780 and 820x560.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from verify_lib import evidence_dir, load_run, png_size, session_id

SIZES = ((1140, 780), (820, 560))
THEMES = ("dark", "light")
CANVAS = {"dark": "#14161a", "light": "#ffffff"}
SPHERES = ("accueil", "parametres-connecteurs", "modeles", "reunions-routines", "memoire", "bibliotheque", "images", "pages")


def browser(evidence: Path, *command: str) -> str:
    script = Path(__file__).with_name("browser.py")
    result = subprocess.run(
        [sys.executable, str(script), "--evidence", str(evidence), *command],
        text=True,
        capture_output=True,
        env=os.environ.copy(),
    )
    if result.returncode != 0:
        raise SystemExit((result.stderr or result.stdout or "commande navigateur échouée").strip())
    return result.stdout


def parse_eval_output(output: str):
    """Decode `result: <json>`. axi often wraps the object in a JSON string."""
    match = re.search(r"^result:\s*(.*)$", output, re.M)
    if not match:
        raise SystemExit(f"résultat navigateur illisible: {output}")
    try:
        value = json.loads(match.group(1))
    except json.JSONDecodeError as error:
        raise SystemExit(f"résultat navigateur illisible: {output}") from error
    while isinstance(value, str):
        try:
            decoded = json.loads(value)
        except json.JSONDecodeError:
            break
        value = decoded
    return value


def eval_js(evidence: Path, source: str):
    return parse_eval_output(browser(evidence, "eval", source))


def wait_js(evidence: Path, source: str, message: str, timeout: float = 20):
    deadline = time.time() + timeout
    last = None
    while time.time() < deadline:
        try:
            value = eval_js(evidence, source)
            if value:
                return value
            last = value
        except SystemExit as error:
            last = str(error)
        time.sleep(0.2)
    raise SystemExit(f"{message}: {last}")


def theme_js(theme: str, close: bool) -> str:
    return f"""async () => {{
      const theme = {json.dumps(theme)};
      const closeAfter = {str(close).lower()};
      const dialog = document.getElementById("settings-dialog");
      if (!dialog.open) document.getElementById("settings-button").click();
      if (!dialog.open) throw new Error("Paramètres fermés");
      const choice = document.querySelector(`[data-theme-choice="${{theme}}"]`);
      choice.click();
      const deadline = Date.now() + 8000;
      while (document.documentElement.dataset.theme !== theme) {{
        if (Date.now() > deadline) throw new Error("thème non appliqué: " + theme);
        await new Promise((resolve) => setTimeout(resolve, 50));
      }}
      const pressed = choice.getAttribute("aria-pressed") === "true";
      if (closeAfter) {{
        document.querySelector("#settings-dialog .dialog-footer [data-close-dialog]").click();
        while (dialog.open) await new Promise((resolve) => setTimeout(resolve, 50));
      }}
      return {{theme: document.documentElement.dataset.theme, pressed, closed: !dialog.open}};
    }}"""


def close_overlays_js() -> str:
    return """async () => {
      for (let guard = 0; guard < 8; guard++) {
        const open = [...document.querySelectorAll("dialog")].filter((dialog) => dialog.open);
        if (!open.length) return true;
        const dialog = open[open.length - 1];
        const close = dialog.querySelector("[data-close-dialog]")
          || [...dialog.querySelectorAll("button")].find((button) => /Fermer|Terminé|Annuler/.test(button.textContent || button.getAttribute("aria-label") || ""));
        if (!close) throw new Error("dialogue sans fermeture");
        close.click();
        await new Promise((resolve) => setTimeout(resolve, 80));
      }
      throw new Error("des dialogues restent ouverts");
    }"""


def metrics_js() -> str:
    return """() => ({
      theme: document.documentElement.dataset.theme,
      canvas: getComputedStyle(document.documentElement).getPropertyValue("--canvas").trim().toLowerCase(),
      width: window.innerWidth,
      height: window.innerHeight,
      title: document.title
    })"""


def prepare_sphere(evidence: Path, sphere: str) -> dict:
    if sphere == "accueil":
        eval_js(evidence, close_overlays_js())
        eval_js(evidence, """() => { document.getElementById("home-link").click(); return true; }""")
        return wait_js(
            evidence,
            """() => {
              const welcome = document.getElementById("welcome");
              const heading = welcome?.querySelector("h2")?.innerText || "";
              const title = document.getElementById("conversation-title")?.textContent || "";
              const editor = document.querySelector(".gb-editor[aria-label='Votre message']");
              const brand = document.getElementById("home-link")?.innerText || "";
              const ok = !!(welcome && !welcome.hidden && heading.includes("Une idée en tête") && title === "Nouvelle conversation" && editor && /ezerbot/i.test(brand));
              return ok ? {ok, heading, title} : null;
            }""",
            "accueil non prêt",
        )
    if sphere == "parametres-connecteurs":
        eval_js(evidence, close_overlays_js())
        eval_js(evidence, """() => { document.getElementById("settings-button").click(); return document.getElementById("settings-dialog").open; }""")
        return wait_js(
            evidence,
            """() => {
              const panel = document.querySelector('[data-panel="mcp"]');
              if (!panel) return null;
              panel.scrollIntoView({block: "start"});
              const heading = panel.querySelector("h3")?.textContent || "";
              const add = [...panel.querySelectorAll("summary")].some((node) => node.textContent.includes("Ajouter un serveur MCP"));
              return heading === "Connecteurs" && add ? {ok: true, heading} : null;
            }""",
            "connecteurs non prêts",
        )
    if sphere == "modeles":
        eval_js(evidence, close_overlays_js())
        return wait_js(
            evidence,
            """async () => {
              const pill = document.querySelector('[data-picker="model"]');
              if (!pill || pill.disabled) return null;
              if (document.getElementById("picker").hidden) pill.click();
              const items = [...document.querySelectorAll('#picker [role="menuitemradio"]')].map((node) => node.innerText);
              const title = document.querySelector("#picker h4")?.textContent || "";
              if (title !== "Modèle" || !items.some((item) => item.includes("GPT Test"))) return null;
              return {ok: true, title, items};
            }""",
            "menu Modèle non prêt",
        )
    if sphere == "reunions-routines":
        eval_js(evidence, close_overlays_js())
        eval_js(evidence, """() => { document.getElementById("settings-button").click(); return document.getElementById("settings-dialog").open; }""")
        meetings = wait_js(
            evidence,
            """() => {
              const retention = document.getElementById("meeting-retention");
              if (!retention) return null;
              retention.scrollIntoView({block: "center"});
              const heading = retention.closest("section, .setting-section")?.querySelector("h3")?.textContent || "";
              const routines = document.querySelector('[data-panel="routines"] h3')?.textContent || "";
              return heading.includes("Réunions") && routines === "Routines" ? {ok: true, heading, routines} : null;
            }""",
            "réglages réunions non prêts",
        )
        return meetings
    if sphere == "memoire":
        eval_js(evidence, close_overlays_js())
        eval_js(evidence, """() => { document.getElementById("settings-button").click(); return document.getElementById("settings-dialog").open; }""")
        wait_js(
            evidence,
            """() => {
              const panel = document.querySelector('[data-panel="memory"]');
              if (!panel) return null;
              const heading = panel.querySelector("h3")?.textContent || "";
              const toggle = panel.querySelector('input[name="automatic-grokbot"]');
              return heading === "Mémoire automatique" && toggle ? {ok: true} : null;
            }""",
            "mémoire automatique non prête",
        )
        eval_js(
            evidence,
            """() => {
              const edit = document.querySelector('[aria-label="Modifier Ezerbot"]');
              if (!edit) throw new Error("Modifier Ezerbot absent");
              edit.click();
              return true;
            }""",
        )
        return wait_js(
            evidence,
            """() => {
              const dialog = document.getElementById("bot-dialog");
              const section = document.querySelector('[aria-label="Mémoire du bot"]');
              if (!dialog?.open || !section) return null;
              section.scrollIntoView({block: "start"});
              const search = document.querySelector('[aria-label="Rechercher dans la mémoire"]');
              const text = section.innerText || "";
              return search && text.includes("faits actifs") ? {ok: true, excerpt: text.slice(0, 240)} : null;
            }""",
            "mémoire du bot non prête",
        )
    if sphere == "bibliotheque":
        eval_js(evidence, close_overlays_js())
        eval_js(evidence, """() => { document.getElementById("settings-button").click(); return document.getElementById("settings-dialog").open; }""")
        wait_js(
            evidence,
            """() => (document.querySelector('[aria-label="Modifier Ezerbot"]') ? {ok: true} : null)""",
            "liste des bots non prête",
        )
        eval_js(
            evidence,
            """() => {
              const edit = document.querySelector('[aria-label="Modifier Ezerbot"]');
              if (!edit) throw new Error("Modifier Ezerbot absent");
              edit.click();
              return true;
            }""",
        )
        return wait_js(
            evidence,
            """() => {
              const dialog = document.getElementById("bot-dialog");
              const section = document.querySelector('[aria-label="Bibliothèque du bot"]');
              if (!dialog?.open || !section) return null;
              section.scrollIntoView({block: "start"});
              const drop = section.querySelector('[aria-label="Déposer un fichier dans la bibliothèque"]');
              const text = section.innerText || "";
              return drop && text.includes("Plafond : 20 Mo") ? {ok: true, excerpt: text.slice(0, 240)} : null;
            }""",
            "bibliothèque du bot non prête",
        )
    if sphere == "images":
        eval_js(evidence, close_overlays_js())
        done = """() => {
          const card = document.querySelector('.gb-generated-image-card[data-image-state="done"]');
          const image = card?.querySelector("img");
          if (!card || !image || !image.complete || !image.naturalWidth) return null;
          card.scrollIntoView({block: "center"});
          return {ok: true, width: image.naturalWidth, height: image.naturalHeight, cells: card.querySelectorAll(".gb-image-cell").length};
        }"""
        if not eval_js(evidence, """() => !!document.querySelector('.gb-generated-image-card[data-image-state="done"]')"""):
            eval_js(evidence, """() => { document.getElementById("new-chat").click(); return true; }""")
            wait_js(
                evidence,
                """() => {
                  const editor = document.querySelector('.gb-editor[aria-label="Votre message"]');
                  return editor ? {ok: true} : null;
                }""",
                "compositeur non prêt",
            )
            eval_js(
                evidence,
                """() => {
                  const editor = document.querySelector('.gb-editor[aria-label="Votre message"]');
                  editor.focus();
                  if (!document.execCommand("insertText", false, "Dessine un phare au crépuscule")) throw new Error("saisie refusée");
                  return true;
                }""",
            )
            wait_js(
                evidence,
                """() => {
                  const send = document.getElementById("send-button");
                  if (!send || send.disabled) return null;
                  send.click();
                  return {ok: true};
                }""",
                "envoi impossible",
            )
        state = wait_js(evidence, done, "image générée absente", timeout=40)
        if state.get("cells") != 1:
            raise SystemExit(f"l’image générée n’a pas son agrandissement: {state}")
        viewer = eval_js(
            evidence,
            """async () => {
              document.querySelector(".gb-generated-image-card .gb-image-cell").click();
              await new Promise((resolve) => setTimeout(resolve, 300));
              const dialog = document.querySelector("dialog.gb-image-viewer");
              const labels = [...(dialog?.querySelectorAll(".gb-image-action") || [])].map((button) => button.textContent.trim());
              const open = !!dialog?.open;
              dialog?.querySelector(".gb-image-close")?.click();
              await new Promise((resolve) => setTimeout(resolve, 200));
              return {open, labels};
            }""",
        )
        if not viewer.get("open") or viewer.get("labels") != ["Enregistrer l’image…", "Copier l’image"]:
            raise SystemExit(f"la visionneuse de l’image générée est incomplète: {viewer}")
        state["viewer"] = viewer["labels"]
        return state
    if sphere == "pages":
        # Une conversation neuve à chaque passage : la capture « avant » la montre sans Page, « après » est prise à l’envoi.
        eval_js(evidence, close_overlays_js())
        eval_js(evidence, """() => { document.getElementById("new-chat").click(); return true; }""")
        return wait_js(
            evidence,
            """() => {
              const editor = document.querySelector('.gb-editor[aria-label="Votre message"]');
              const stage = document.querySelector(".page-stage");
              return editor && (!stage || stage.hidden) ? {ok: true} : null;
            }""",
            "conversation neuve non prête",
        )
    raise SystemExit(f"sphère inconnue: {sphere}")


def pages_request(evidence: Path) -> dict:
    eval_js(
        evidence,
        """() => {
          const editor = document.querySelector('.gb-editor[aria-label="Votre message"]');
          editor.focus();
          if (!document.execCommand("insertText", false, "Fais-moi une page des forfaits")) throw new Error("saisie refusée");
          return true;
        }""",
    )
    wait_js(
        evidence,
        """() => {
          const send = document.getElementById("send-button");
          if (!send || send.disabled) return null;
          send.click();
          return {ok: true};
        }""",
        "envoi impossible",
    )
    state = wait_js(
        evidence,
        """() => {
          const stage = document.querySelector(".page-stage");
          const choices = stage && !stage.hidden ? [...stage.querySelectorAll(".page-canvas button.page-choice")] : [];
          if (choices.length !== 3) return null;
          return {ok: true, text: stage.innerText.slice(0, 400)};
        }""",
        "Page absente à côté du fil",
        timeout=40,
    )
    if "9 €" not in state["text"] or "u20ac" in state["text"]:
        raise SystemExit(f"l’échappement Unicode du prix est mal décodé: {state['text']}")
    if "Tout < 30 € par mois" not in state["text"]:
        raise SystemExit(f"les références de caractères JSX sont mal décodées: {state['text']}")
    chosen = eval_js(
        evidence,
        """async () => {
          const stage = document.querySelector(".page-stage");
          const choices = [...stage.querySelectorAll(".page-canvas button.page-choice")];
          choices[1].click();
          await new Promise((resolve) => setTimeout(resolve, 700));
          const again = [...stage.querySelectorAll(".page-canvas button.page-choice")];
          return {text: stage.innerText, focused: again.indexOf(document.activeElement)};
        }""",
    )
    if "Offre choisie : Confort" not in chosen["text"] or chosen["focused"] != 1:
        raise SystemExit(f"le clic sur « Choisir » n’a pas gardé son choix et son focus: {chosen}")
    state["choice"] = "Confort"
    return state


def shot_names(sphere: str, theme: str, width: int, height: int) -> list[str]:
    size = f"{width}x{height}"
    if sphere == "reunions-routines":
        return [f"reunions-{theme}-{size}.png", f"routines-{theme}-{size}.png"]
    if sphere == "pages":
        return [f"avant-{theme}-{size}.png", f"apres-{theme}-{size}.png"]
    return [f"{theme}-{size}.png"]


def capture(evidence: Path, sphere: str, theme: str, width: int, height: int, names: list[str]) -> list[Path]:
    written: list[Path] = []
    directory = evidence / "spheres" / sphere
    for index, name in enumerate(names):
        if sphere == "reunions-routines" and index == 0:
            eval_js(
                evidence,
                """() => {
                  const retention = document.getElementById("meeting-retention");
                  retention?.scrollIntoView({block: "center"});
                  return true;
                }""",
            )
        if sphere == "pages" and index == 1:
            pages_request(evidence)
        if sphere == "reunions-routines" and index == 1:
            eval_js(
                evidence,
                """() => {
                  const button = [...document.querySelectorAll('[data-panel="routines"] button')]
                    .find((node) => node.textContent.includes("Gérer les routines"));
                  if (!button) throw new Error("bouton des routines absent");
                  button.click();
                  return true;
                }""",
            )
            wait_js(
                evidence,
                """() => {
                  const title = document.getElementById("routines-title");
                  const dialog = document.querySelector("dialog.routines-dialog");
                  return dialog?.open && title?.textContent === "Routines" ? {ok: true} : null;
                }""",
                "dialogue Routines non ouvert",
            )
        eval_js(evidence, "() => { window.scrollTo(0, 0); return true; }")
        path = directory / name
        browser(evidence, "screenshot", str(path))
        actual = png_size(path)
        if actual != (width, height):
            raise SystemExit(f"{path} mesure {actual[0]}x{actual[1]}, attendu {width}x{height}")
        text = eval_js(evidence, "() => document.body.innerText.slice(0, 4000)")
        path.with_suffix(".txt").write_text(text if isinstance(text, str) else json.dumps(text, ensure_ascii=False))
        written.append(path)
    return written


def load_findings(evidence: Path) -> list[dict]:
    path = evidence / "findings.json"
    if not path.is_file():
        return []
    return json.loads(path.read_text())


def save_findings(evidence: Path, findings: list[dict]) -> None:
    (evidence / "findings.json").write_text(json.dumps(findings, indent=2, ensure_ascii=False) + "\n")


def record_tool_error(evidence: Path, error: BaseException) -> None:
    """A crash of this driver is not an Ezerbot defect."""
    message = " ".join(f"{type(error).__name__}: {error}".split())
    payload = {"kind": "outil", "message": message}
    (evidence / "tool-error.json").write_text(json.dumps(payload, indent=2, ensure_ascii=False) + "\n")


def drive_sphere(evidence: Path, sphere: str, findings: list[dict]) -> None:
    for theme in THEMES:
        eval_js(evidence, close_overlays_js())
        observed = eval_js(evidence, theme_js(theme, sphere == "accueil"))
        if observed.get("theme") != theme or not observed.get("pressed"):
            raise SystemExit(f"thème {theme} non confirmé: {observed}")
        if sphere != "accueil" and observed.get("closed"):
            eval_js(evidence, """() => { document.getElementById("settings-button").click(); return true; }""")
        for width, height in SIZES:
            browser(evidence, "resize", str(width), str(height))
            state = prepare_sphere(evidence, sphere)
            metrics = eval_js(evidence, metrics_js())
            if metrics.get("width") != width or metrics.get("height") != height:
                raise SystemExit(f"fenêtre {metrics.get('width')}x{metrics.get('height')} au lieu de {width}x{height}")
            if metrics.get("theme") != theme or metrics.get("canvas") != CANVAS[theme]:
                raise SystemExit(f"thème visuel inattendu: {metrics}")
            names = shot_names(sphere, theme, width, height)
            written = capture(evidence, sphere, theme, width, height, names)
            print(json.dumps({"sphere": sphere, "theme": theme, "size": [width, height], "state": state, "shots": [str(path) for path in written]}, ensure_ascii=False))


def main() -> None:
    parser = argparse.ArgumentParser(description="Parcourt une sphère Ezerbot et capture les preuves")
    parser.add_argument("--evidence", type=Path)
    parser.add_argument("--sphere", default="accueil")
    args = parser.parse_args()
    evidence = args.evidence or evidence_dir(session_id())
    os.environ["EZERBOT_VERIFY_EVIDENCE_DIR"] = str(evidence)
    spheres = list(SPHERES) if args.sphere == "all" else [args.sphere]
    unknown = [sphere for sphere in spheres if sphere not in SPHERES]
    if unknown:
        raise SystemExit(f"sphère inconnue: {', '.join(unknown)}")
    run = load_run(evidence)
    findings = load_findings(evidence)
    try:
        browser(evidence, "open", run["url"])
        wait_js(
            evidence,
            """() => {
              const editor = document.querySelector(".gb-editor");
              const badge = document.getElementById("mode-badge-text")?.textContent || "";
              return editor && badge === "ChatGPT" ? {badge} : null;
            }""",
            "Ezerbot n'est pas connecté au faux OpenAI",
            timeout=25,
        )
        for sphere in spheres:
            drive_sphere(evidence, sphere, findings)
    except SystemExit as error:
        findings.append(
            {
                "screenshot": "",
                "defect": str(error),
                "severity": "bloquant",
                "fix": "Rejouer la sphère après correction du parcours ou du produit.",
            }
        )
        save_findings(evidence, findings)
        raise
    except Exception as error:
        record_tool_error(evidence, error)
        save_findings(evidence, findings)
        print(f"erreur d'outil: {type(error).__name__}: {error}", file=sys.stderr)
        raise SystemExit(1) from error
    save_findings(evidence, findings)
    print(f"drive: ok {', '.join(spheres)}")


if __name__ == "__main__":
    main()
