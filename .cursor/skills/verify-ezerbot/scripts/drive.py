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
SPHERES = ("accueil", "parametres-connecteurs", "modeles", "reunions-routines", "memoire", "bibliotheque", "images", "pages", "dictee", "voix-bots", "memo-vocal", "iphone-reponse", "voix-appel", "taches-de-fond")


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
              // La carte Réunions n'a qu'un champ de clé masqué (aucune clé enregistrée sur une base neuve).
              const keys = [...panel.querySelectorAll('[data-connector="reunions"] input')];
              const reunions = keys.length === 1 && keys[0].type === "password";
              // Gmail et GitHub n'ont ni champ d'identifiant ni de secret ; sans client de build, la carte Gmail le dit.
              const gmail = panel.querySelector('[data-connector="gmail"]');
              const gmailFree = !!gmail && !gmail.querySelector("form") && gmail.querySelectorAll('input:not([type="checkbox"])').length === 0;
              // Le même consentement Google lit l'agenda et Drive : la carte Gmail le dit, sans second formulaire.
              const gmailReads = !!gmail && /agenda/i.test(gmail.textContent) && /Drive/.test(gmail.textContent);
              return heading === "Connecteurs" && add && reunions && gmailFree && gmailReads ? {ok: true, heading, reunions, gmailFree, gmailReads} : null;
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
    if sphere == "dictee":
        # Tous les micros sont branchés au départ : la preuve du repli débranche le micro choisi puis le rebranche.
        eval_js(evidence, close_overlays_js())
        eval_js(evidence, """async () => { await window.GrokbotPanels.api("/api/voice/debug", {command: "configure", unplugged: []}); document.getElementById("settings-button").click(); return document.getElementById("settings-dialog").open; }""")
        return wait_js(
            evidence,
            """() => {
              const panel = document.querySelector('[data-panel="voice"].voice-microphone-settings');
              const options = panel ? [...panel.querySelectorAll(".voice-microphone-option")] : [];
              if (options.length !== 3) return null;
              panel.scrollIntoView({block: "center"});
              return {ok: true, heading: panel.querySelector("h3").textContent, options: options.map((node) => node.dataset.id)};
            }""",
            "réglage du microphone non prêt",
        )
    if sphere == "voix-bots":
        # Deux bots sans choix : Ezerbot et Chef. Les voix de test sont attribuées dès que la liste est lue.
        eval_js(evidence, close_overlays_js())
        eval_js(
            evidence,
            """async () => {
              const panels = window.GrokbotPanels;
              await panels.api("/api/bots", {id: "bot-chef", name: "Chef", color: "orange", instructions: "Réponds en recettes."});
              await panels.refresh();
              return true;
            }""",
        )
        wait_js(
            evidence,
            """() => {
              const voice = window.GrokbotPanels?.state?.voice;
              return voice?.voices?.length === 2 && voice.bots?.grokbot?.voice && voice.bots["bot-chef"]?.voice ? {ok: true} : null;
            }""",
            "voix des bots non attribuées",
        )
        return open_bot_voice(evidence, "Ezerbot")
    if sphere == "iphone-reponse":
        # Une conversation neuve avec une vraie réponse du faux bot, puis l'accueil : la notification « reply: » reste non lue.
        eval_js(evidence, close_overlays_js())
        eval_js(
            evidence,
            """async () => {
              const panels = window.GrokbotPanels;
              const { conversation } = await panels.api("/api/conversations", {});
              await panels.api("/api/send", {conversation_id: conversation.id, message_id: "question-" + Date.now(), body: "Bonjour, qui es-tu ?"});
              window.__replyConversation = conversation.id;
              return true;
            }""",
        )
        wait_js(
            evidence,
            """async () => {
              const panels = window.GrokbotPanels;
              await panels.refresh();
              const conversation = panels.state.conversations.find((item) => item.id === window.__replyConversation);
              const reply = conversation?.messages.find((message) => message.role === "assistant" && message.body && message.state !== "streaming");
              if (!reply) return null;
              const { items } = await panels.api("/api/notifications");
              const item = items.find((entry) => entry.kind === "reply" && entry.conversation_id === window.__replyConversation);
              if (!item) return null;
              window.__replyNotification = item.id;
              document.getElementById("home-link").click();
              return {ok: true, notification: item.id};
            }""",
            "réponse du bot absente",
            timeout=40,
        )
        return {"ok": True}
    if sphere == "memo-vocal":
        # Un mémo de vingt secondes déjà envoyé, ouvert dans son fil. Il n’est créé qu’une fois par passage complet.
        eval_js(evidence, close_overlays_js())
        eval_js(
            evidence,
            """async () => {
              const panels = window.GrokbotPanels;
              if (document.querySelector(".gb-voice-memo .gb-memo-word")) return true;
              const { conversation } = await panels.api("/api/conversations", {});
              const token = "memo-" + Date.now();
              await panels.api("/api/voice/debug", {command: "start", token, purpose: "memo", scenario: "long_memo"});
              const memo = await panels.api("/api/voice/debug", {command: "stop", token});
              const kept = await panels.api("/api/voice/memos", {conversation_id: conversation.id, upload_id: token, data: memo.data, transcript: memo.transcript, language: memo.language, words: memo.words});
              await panels.api("/api/send", {conversation_id: conversation.id, message_id: "message-" + token, body: "Voici mon mémo vocal.", attachments: [kept.id]});
              await panels.refresh();
              panels.openConversation(conversation.id);
              return true;
            }""",
        )
        wait_js(
            evidence,
            """() => {
              const audio = document.querySelector(".gb-voice-memo audio");
              const words = document.querySelectorAll(".gb-voice-memo .gb-memo-word");
              return audio && words.length > 10 && audio.duration > 19 ? {ok: true} : null;
            }""",
            "mémo de vingt secondes absent du fil",
        )
        return memo_reset(evidence)
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
    if sphere == "voix-appel":
        # Une conversation neuve à chaque passage, puis l’appel lancé depuis l’en-tête : le fil ne porte que le transcript de ce passage.
        eval_js(evidence, close_overlays_js())
        eval_js(
            evidence,
            """async () => {
              const panels = window.GrokbotPanels;
              if (window.EzerbotVoiceCall?.active()) {
                [...document.querySelectorAll(".voice-call-bar button")].find((node) => node.textContent === "Quitter").click();
                const deadline = Date.now() + 8000;
                while (window.EzerbotVoiceCall.active() && Date.now() < deadline) await new Promise((resolve) => setTimeout(resolve, 100));
              }
              const { conversation } = await panels.api("/api/conversations", {});
              await panels.refresh();
              if (!panels.openConversation(conversation.id)) throw new Error("conversation de l’appel non ouverte");
              return true;
            }""",
        )
        return wait_js(
            evidence,
            """() => {
              const button = document.querySelector(".voice-call-toggle");
              return button && !button.disabled && button.getAttribute("aria-label") === "Appel vocal" ? {ok: true, title: button.title} : null;
            }""",
            "bouton Appel vocal non prêt",
        )
    if sphere == "taches-de-fond":
        # Le shell est accordé au bot, et aucune tâche d'un passage précédent ne tourne encore.
        eval_js(evidence, close_overlays_js())
        return eval_js(
            evidence,
            """async () => {
              const panels = window.GrokbotPanels;
              await panels.refresh();
              let stopped = 0;
              for (const conversation of panels.state.conversations || []) {
                for (const message of conversation.messages || []) {
                  const output = message.payload?.outcome?.output;
                  if (message.kind === "tool" && output?.background === true && output?.status === "en cours") {
                    await panels.api("/api/local/tasks/stop", { run_id: message.payload.run_id, call_id: message.payload.call?.call_id });
                    stopped += 1;
                  }
                }
              }
              await panels.api("/api/local/bot", { bot_id: "grokbot", shell: true });
              return { ok: true, stopped };
            }""",
        )
    raise SystemExit(f"sphère inconnue: {sphere}")


def task_request(evidence: Path, text: str, prompt: str) -> dict:
    """Une conversation neuve, la demande, l'approbation « Autoriser une fois », puis la carte En cours avec son invite visible."""
    eval_js(evidence, """() => { document.getElementById("new-chat").click(); return true; }""")
    wait_js(
        evidence,
        """() => document.querySelector('.gb-editor[aria-label="Votre message"]') ? {ok: true} : null""",
        "conversation neuve non prête",
    )
    eval_js(
        evidence,
        f"""() => {{
          const editor = document.querySelector('.gb-editor[aria-label="Votre message"]');
          editor.focus();
          if (!document.execCommand("insertText", false, {json.dumps(text)})) throw new Error("saisie refusée");
          return true;
        }}""",
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
    wait_js(
        evidence,
        """() => {
          const button = [...document.querySelectorAll('[data-card-kind="approval"] button')].find((node) => node.textContent === "Autoriser une fois" && !node.disabled);
          if (!button) return null;
          button.click();
          return {ok: true};
        }""",
        "approbation du shell absente",
        timeout=40,
    )
    return wait_js(
        evidence,
        f"""() => {{
          const card = document.querySelector('[data-card-kind="tool"][data-local-state="en cours"]');
          const field = card?.querySelector(".gb-local-input input");
          const details = card?.querySelector("details");
          if (!card || !field || !details) return null;
          details.open = true;
          return card.innerText.includes({json.dumps(prompt)}) ? {{ok: true, text: card.innerText.slice(0, 400)}} : null;
        }}""",
        "carte En cours avec son invite et son champ absente",
        timeout=40,
    )


def task_send(evidence: Path, line: str) -> None:
    eval_js(
        evidence,
        f"""() => {{
          const card = document.querySelector('[data-card-kind="tool"] .gb-local-input')?.closest("article");
          const field = card.querySelector(".gb-local-input input");
          Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set.call(field, {json.dumps(line)});
          field.dispatchEvent(new Event("input", {{bubbles: true}}));
          card.querySelector(".gb-local-input").requestSubmit();
          return true;
        }}""",
    )


def background_task_step(evidence: Path, index: int) -> None:
    """Avant chaque capture : 0 la carte En cours devant son invite, 1 « oui » envoyé et Terminée, 2 un mot de passe refusé."""
    if index == 0:
        state = task_request(evidence, "Lance le script de saisie", "Continuer ? [oui/non]")
        if "Arrêter" not in state["text"] or "En cours" not in state["text"]:
            raise SystemExit(f"la carte En cours n'offre pas Arrêter: {state}")
    if index == 1:
        task_send(evidence, "oui")
        state = wait_js(
            evidence,
            """() => {
              const card = document.querySelector('[data-card-kind="tool"][data-local-state="terminé"]');
              if (!card) return null;
              card.querySelector("details").open = true;
              const text = card.innerText;
              return text.includes("reponse=oui") ? {ok: true, text, field: !!card.querySelector(".gb-local-input")} : null;
            }""",
            "le programme n'a pas lu « oui »",
            timeout=30,
        )
        # L'invite n'a pas de fin de ligne : la ligne recopiée la suit, comme dans un terminal.
        if "Continuer ? [oui/non] oui\nreponse=oui" not in state["text"] or state["field"] or "Code de sortie : 0" not in state["text"]:
            raise SystemExit(f"« oui » n'est pas recopié dans la sortie d'une carte Terminée sans champ: {state}")
        refused = eval_js(
            evidence,
            """async () => {
              const panels = window.GrokbotPanels;
              await panels.refresh();
              const message = panels.state.conversations.flatMap((conversation) => conversation.messages || []).find((item) => item.kind === "tool" && item.payload?.call?.call_id === "verify-task-saisie" && item.payload?.outcome?.output?.status === "terminé");
              const before = message.payload.outcome.output.stdout;
              let error = "";
              try { await panels.api("/api/local/tasks/input", { run_id: message.payload.run_id, call_id: "verify-task-saisie", line: "trop tard" }); }
              catch (failure) { error = failure.message; }
              await panels.refresh();
              const again = panels.state.conversations.flatMap((conversation) => conversation.messages || []).find((item) => item.payload?.call?.call_id === "verify-task-saisie" && item.payload?.outcome?.output?.status === "terminé");
              return { error, unchanged: again.payload.outcome.output.stdout === before };
            }""",
        )
        if "n’est plus en cours" not in refused["error"] or not refused["unchanged"]:
            raise SystemExit(f"une saisie après Terminée n'est pas refusée en français sans changer la sortie: {refused}")
    if index == 2:
        task_request(evidence, "Lance le script du mot de passe", "Mot de passe :")
        task_send(evidence, "secret-123")
        state = wait_js(
            evidence,
            """() => {
              const card = document.querySelector('[data-card-kind="tool"][data-local-state="en cours"]');
              const alert = card?.querySelector('.gb-local-failure[role="alert"]');
              if (!alert) return null;
              return {ok: true, alert: alert.innerText, text: card.innerText, field: card.querySelector(".gb-local-input input")?.value};
            }""",
            "le refus du mot de passe n'est pas sur la carte",
            timeout=30,
        )
        if "mot de passe : la saisie n’a pas été envoyée" not in state["alert"]:
            raise SystemExit(f"le refus ne dit pas que rien n'est parti: {state}")
        if "secret-123" in state["text"] or state["field"] != "secret-123":
            raise SystemExit(f"la ligne refusée est apparue dans la sortie au lieu de rester dans le champ: {state}")


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


def open_bot_voice(evidence: Path, name: str) -> dict:
    """Ouvre « Modifier <bot> » depuis les Paramètres et rend la voix affichée dans la section Voix du bot."""
    eval_js(evidence, close_overlays_js())
    eval_js(evidence, """() => { document.getElementById("settings-button").click(); return document.getElementById("settings-dialog").open; }""")
    wait_js(evidence, f"""() => (document.querySelector('[aria-label="Modifier {name}"]') ? {{ok: true}} : null)""", "liste des bots non prête")
    eval_js(evidence, f"""() => {{ document.querySelector('[aria-label="Modifier {name}"]').click(); return true; }}""")
    return wait_js(
        evidence,
        """() => {
          const section = document.querySelector(".voice-bot-settings");
          const current = section?.querySelector(".voice-bot-current")?.textContent || "";
          if (!document.getElementById("bot-dialog")?.open || !current.includes("Voix de")) return null;
          section.scrollIntoView({block: "center"});
          const speed = section.querySelector('[name="bot-voice-speed"]');
          return {ok: true, current, hint: section.querySelector(".voice-bot-hint").textContent, max: speed.max, language: section.querySelector('[name="bot-voice-language"]').value};
        }""",
        f"section Voix de {name} non prête",
    )


def voice_bots_step(evidence: Path, index: int) -> None:
    """Avant chaque capture : 0 voix d’Ezerbot, 1 voix de Chef (autre), 2 vitesse 2,5 refusée pour Chef."""
    if index == 0:
        return
    if index == 1:
        first = eval_js(evidence, """() => document.querySelector(".voice-bot-current").textContent""")
        second = open_bot_voice(evidence, "Chef")
        if not first.startswith("Voix de Ezerbot") or first.split(" : ")[1] == second["current"].split(" : ")[1] or "attribuée automatiquement" not in second["hint"]:
            raise SystemExit(f"les deux bots n’annoncent pas deux voix différentes: {first} / {second}")
    if index == 2:
        before = eval_js(evidence, """() => document.querySelector(".voice-bot-current").textContent""")
        state = eval_js(
            evidence,
            """async () => {
              const section = document.querySelector(".voice-bot-settings");
              const speed = section.querySelector('[name="bot-voice-speed"]');
              speed.value = "2.5"; speed.dispatchEvent(new Event("input"));
              [...section.querySelectorAll("button")].find((node) => node.textContent === "Enregistrer la voix").click();
              await new Promise((resolve) => setTimeout(resolve, 600));
              return {notice: section.querySelector(".voice-settings-notice").textContent, current: section.querySelector(".voice-bot-current").textContent};
            }""",
        )
        if "Vitesse refusée" not in state["notice"] or state["current"] != before:
            raise SystemExit(f"la vitesse au-delà de 2 n’est pas refusée telle quelle: {state}")


def voice_call_step(evidence: Path, index: int) -> None:
    """Avant chaque capture : 0 le mode voix seul puis quitté (il garde sa phrase), 1 l’appel lancé (phrase de transmission), 2 le transcript affiché pendant que le bot parle, 3 le fil après Quitter."""
    if index == 0:
        # Le mode voix dit encore que l’audio reste sur le Mac, et l’appel est refusé tant qu’il tient le micro.
        mode = eval_js(
            evidence,
            """async () => {
              document.querySelector(".voice-session-toggle").click();
              const deadline = Date.now() + 8000;
              while (!document.querySelector(".voice-live-transcript")?.textContent.includes("L’audio reste sur ce Mac.") && Date.now() < deadline) await new Promise((resolve) => setTimeout(resolve, 100));
              const phrase = document.querySelector(".voice-live-transcript")?.textContent || "";
              const call = document.querySelector(".voice-call-toggle");
              const refused = {disabled: call.disabled, title: call.title};
              [...document.querySelectorAll(".voice-session-bar button")].find((node) => node.textContent === "Quitter").click();
              while (window.EzerbotVoiceMode.active() && Date.now() < deadline) await new Promise((resolve) => setTimeout(resolve, 100));
              return {phrase, refused, over: !window.EzerbotVoiceMode.active()};
            }""",
        )
        if "L’audio reste sur ce Mac." not in mode["phrase"] or not mode["refused"]["disabled"] or not mode["over"]:
            raise SystemExit(f"le mode voix a perdu sa phrase, ou l’appel n’est pas refusé pendant le mode voix: {mode}")
    if index == 1:
        eval_js(evidence, """() => { document.querySelector(".voice-call-toggle").click(); return true; }""")
        state = wait_js(
            evidence,
            """() => {
              const bar = document.querySelector(".voice-call-bar");
              const state = bar?.querySelector(".voice-call-state")?.textContent || "";
              if (!bar || bar.hidden || !state.startsWith("Appel en cours")) return null;
              const toggle = document.querySelector(".voice-call-toggle");
              const mode = document.querySelector(".voice-session-toggle");
              return {ok: true, notice: bar.querySelector(".voice-call-notice").textContent, label: bar.querySelector(".voice-call-transcript-toggle").textContent,
                transcriptHidden: bar.querySelector(".voice-call-transcript").hidden, pressed: toggle.getAttribute("aria-pressed"), modeDisabled: mode.disabled, modeTitle: mode.title,
                fromHeader: toggle.parentElement.classList.contains("topbar-actions")};
            }""",
            "appel non démarré",
        )
        if (
            state["notice"] != "L’audio de cet appel est transmis au fournisseur de voix le temps de l’appel. Il n’est pas gardé sur le Mac."
            or state["label"] != "Afficher le transcript" or not state["transcriptHidden"]
            or not state["modeDisabled"] or state["modeTitle"] != "Indisponible pendant l’appel vocal"
        ):
            raise SystemExit(f"l’écran de l’appel est incorrect: {state}")
    if index == 2:
        said = eval_js(
            evidence,
            """async () => {
              const panels = window.GrokbotPanels;
              const deadline = Date.now() + 8000;
              while (!panels.state.voice_call?.active && Date.now() < deadline) await new Promise((resolve) => setTimeout(resolve, 100));
              const id = panels.state.voice_call.active.id;
              const feed = (body) => panels.api("/api/voice/call/debug", {command: "feed", token: id, ...body});
              const label = () => document.querySelector(".voice-call-state").textContent;
              const waitFor = async (test) => { const deadline = Date.now() + 8000; while (!test() && Date.now() < deadline) await new Promise((resolve) => setTimeout(resolve, 100)); return test(); };
              await feed({text: "Bonjour Ezerbot, tu m’entends ?", final_result: true});
              const spoke = await waitFor(() => label().includes("Le bot parle"));
              // Parler par-dessus coupe la voix du bot.
              await feed({text: "Attends"});
              const cutBySpeech = await waitFor(() => label().startsWith("Appel en cours"));
              // Échap la coupe aussi.
              await feed({text: "Dis-moi une longue histoire.", final_result: true});
              const spokeAgain = await waitFor(() => label().includes("Le bot parle"));
              document.dispatchEvent(new KeyboardEvent("keydown", {key: "Escape", bubbles: true, cancelable: true}));
              const cutByEscape = await waitFor(() => label().startsWith("Appel en cours"));
              // Le transcript s’affiche, puis se masque, puis s’affiche pour la capture, bot en train de parler.
              const toggle = document.querySelector(".voice-call-transcript-toggle");
              toggle.click();
              const shown = !document.querySelector(".voice-call-transcript").hidden && toggle.textContent === "Masquer le transcript";
              toggle.click();
              const hidden = document.querySelector(".voice-call-transcript").hidden && toggle.textContent === "Afficher le transcript";
              toggle.click();
              await feed({text: "Merci, parle-moi du Mac.", final_result: true});
              await waitFor(() => label().includes("Le bot parle"));
              await waitFor(() => document.querySelectorAll(".voice-call-line").length >= 6);
              return {spoke, cutBySpeech, spokeAgain, cutByEscape, shown, hidden, lines: document.querySelectorAll(".voice-call-line").length, text: document.querySelector(".voice-call-transcript").innerText};
            }""",
        )
        if not all(said[key] for key in ("spoke", "cutBySpeech", "spokeAgain", "cutByEscape", "shown", "hidden")) or "Bonjour Ezerbot" not in said["text"]:
            raise SystemExit(f"l’appel ne coupe pas la voix du bot ou n’affiche pas son transcript: {said}")
    if index == 3:
        eval_js(evidence, """() => { [...document.querySelectorAll(".voice-call-bar button")].find((node) => node.textContent === "Quitter").click(); return true; }""")
        state = wait_js(
            evidence,
            """() => {
              const card = document.querySelector('[data-card-kind="voice_call"]');
              if (!card || window.EzerbotVoiceCall.active()) return null;
              card.scrollIntoView({block: "center"});
              return {ok: true, label: card.querySelector(".gb-engine-label").textContent, lines: card.querySelectorAll(".gb-voice-call-lines li").length, audio: card.querySelectorAll("audio, a[href*='audio']").length, text: card.innerText};
            }""",
            "transcript absent du fil",
        )
        if state["label"] != "Appel vocal" or state["lines"] < 6 or state["audio"] != 0 or "Bonjour Ezerbot" not in state["text"]:
            raise SystemExit(f"le fil ne garde pas le transcript marqué Appel vocal: {state}")


MEMO_SPEED_JS = """(text) => {
  const input = document.querySelector(".gb-voice-memo .gb-memo-speed input");
  input.focus();
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set.call(input, text);
  input.dispatchEvent(new Event("input", {bubbles: true}));
  input.blur();
  return new Promise((resolve) => setTimeout(() => {
    const audio = document.querySelector(".gb-voice-memo audio");
    resolve({shown: input.value, rate: audio.playbackRate, refusal: document.querySelector(".gb-memo-refusal")?.textContent || ""});
  }, 200));
}"""


MEMO_READY_JS = """() => document.querySelector(".gb-voice-memo audio") && document.querySelector(".gb-voice-memo .gb-memo-speed input") ? {ok: true} : null"""


def memo_speed(evidence: Path, text: str) -> dict:
    # Le fil peut se redessiner entre deux commandes : on attend le lecteur avant de saisir.
    wait_js(evidence, MEMO_READY_JS, "lecteur du mémo absent")
    return eval_js(evidence, f"() => ({MEMO_SPEED_JS})({json.dumps(text)})")


def memo_reset(evidence: Path) -> dict:
    """Le mémo repart de zéro : vitesse 1, lecture arrêtée au début, aucune phrase d’erreur."""
    memo_speed(evidence, "1")
    return wait_js(
        evidence,
        """() => {
          const audio = document.querySelector(".gb-voice-memo audio");
          if (!audio) return null;
          audio.pause(); audio.currentTime = 0;
          document.querySelector(".gb-voice-memo").scrollIntoView({block: "center"});
          return {ok: true, speed: document.querySelector(".gb-voice-memo .gb-memo-speed input").value, words: document.querySelectorAll(".gb-voice-memo .gb-memo-word").length, duration: audio.duration};
        }""",
        "lecteur du mémo absent",
    )


def memo_step(evidence: Path, index: int) -> None:
    """Avant chaque capture : 0 le mémo à la vitesse 1, 1 vitesse 2 puis 3 refusée, 2 clic sur un mot du milieu, 3 clic sur un mot sans horodatage."""
    wait_js(evidence, MEMO_READY_JS, "lecteur du mémo absent")
    if index == 0:
        state = memo_reset(evidence)
        if state["speed"] != "1":
            raise SystemExit(f"le mémo ne part pas de la vitesse 1: {state}")
    if index == 1:
        faster = memo_speed(evidence, "2")
        refused = memo_speed(evidence, "3")
        if faster["rate"] != 2 or refused["rate"] != 2 or refused["shown"] != "2" or "de 0,5 à 2" not in refused["refusal"]:
            raise SystemExit(f"la vitesse 3 n’est pas refusée telle quelle: {faster} / {refused}")
        slower = memo_speed(evidence, "0,5")
        if slower["rate"] != 0.5 or slower["refusal"]:
            raise SystemExit(f"la vitesse 0,5 n’est pas suivie par la lecture: {slower}")
        memo_speed(evidence, "2")
        memo_speed(evidence, "3")
    if index == 2:
        jump = eval_js(
            evidence,
            """() => {
              const audio = document.querySelector(".gb-voice-memo audio");
              audio.currentTime = 0;
              const words = [...document.querySelectorAll(".gb-voice-memo button.gb-memo-word")];
              const word = words[Math.floor(words.length / 2)];
              word.click();
              return {text: word.textContent, start: Number(word.dataset.startMs) / 1000, at: audio.currentTime, rate: audio.playbackRate, paused: audio.paused};
            }""",
        )
        if jump["at"] != jump["start"] or jump["start"] < 5 or jump["rate"] != 2:
            raise SystemExit(f"le clic sur un mot n’a pas placé la lecture au début de ce mot: {jump}")
    if index == 3:
        state = eval_js(
            evidence,
            """async () => {
              const audio = document.querySelector(".gb-voice-memo audio");
              audio.pause(); audio.currentTime = 3;
              const word = document.querySelector(".gb-voice-memo .gb-memo-word.is-untimed");
              word.click();
              await new Promise((resolve) => setTimeout(resolve, 300));
              return {text: word.textContent, at: audio.currentTime, notice: document.querySelector(".gb-memo-notice")?.textContent || ""};
            }""",
        )
        if state["at"] != 3 or state["notice"] != "Ce mot n’a pas d’horodatage : la lecture continue.":
            raise SystemExit(f"un mot sans horodatage a déplacé la lecture ou ne le dit pas: {state}")


def iphone_reply_step(evidence: Path, index: int) -> None:
    """Avant chaque capture : 0 le fil avec la réponse du bot, 1 un toucher sur la notification et le bouton Répondre, 2 la feuille ouverte par ce bouton, 3 « Vu depuis la notification » envoyé deux fois."""
    if index == 0:
        eval_js(evidence, """() => { window.GrokbotPanels.openConversation(window.__replyConversation); return true; }""")
        wait_js(
            evidence,
            """() => document.querySelector("dialog.reply-sheet[open]") ? null : [...document.querySelectorAll("[data-message-id]")].some((node) => node.innerText.includes("Bonjour, qui es-tu")) ? {ok: true} : null""",
            "fil de la réponse absent",
        )
    if index == 1:
        eval_js(evidence, """() => { document.getElementById("home-link").click(); return true; }""")
        state = eval_js(
            evidence,
            """async () => {
              await window.EzerbotNotifications.activate(window.__replyNotification, {push: true});
              await new Promise((resolve) => setTimeout(resolve, 400));
              const button = document.querySelector(".reply-offer button");
              return {label: button?.textContent || "", sheet: !!document.querySelector("dialog.reply-sheet[open]")};
            }""",
        )
        if not state["label"].startswith("Répondre") or state["sheet"]:
            raise SystemExit(f"un toucher simple n'offre pas Répondre sur la conversation: {state}")
    if index == 2:
        state = eval_js(
            evidence,
            """async () => {
              document.querySelector(".reply-offer button").click();
              await new Promise((resolve) => setTimeout(resolve, 400));
              const dialog = document.querySelector("dialog.reply-sheet");
              const field = dialog?.querySelector("textarea");
              return {open: !!dialog?.open, title: dialog?.querySelector("h2")?.textContent, focused: document.activeElement === field, offer: !!document.querySelector(".reply-offer")};
            }""",
        )
        if not state["open"] or not state["title"].startswith("Répondre à ") or not state["focused"] or state["offer"]:
            raise SystemExit(f"la feuille de réponse n'est pas prête, champ prêt: {state}")
    if index == 3:
        state = eval_js(
            evidence,
            """async () => {
              const dialog = document.querySelector("dialog.reply-sheet");
              const field = dialog.querySelector("textarea");
              field.value = "Vu depuis la notification";
              const form = dialog.querySelector("form");
              form.requestSubmit(); form.requestSubmit();
              await new Promise((resolve) => setTimeout(resolve, 1500));
              await window.GrokbotPanels.refresh();
              const conversation = window.GrokbotPanels.state.conversations.find((item) => item.id === window.__replyConversation);
              const sent = conversation.messages.filter((message) => message.role === "user" && message.body === "Vu depuis la notification").length;
              return {closed: !dialog.open, sent};
            }""",
        )
        if not state["closed"] or state["sent"] != 1:
            raise SystemExit(f"la réponse n'est pas arrivée une seule fois dans le fil: {state}")
        eval_js(evidence, """() => { window.GrokbotPanels.openConversation(window.__replyConversation); return true; }""")
        wait_js(
            evidence,
            """() => [...document.querySelectorAll("[data-message-id]")].some((node) => node.innerText.includes("Vu depuis la notification")) ? {ok: true} : null""",
            "message absent du fil",
        )


def dictation_step(evidence: Path, index: int) -> None:
    """Avant chaque capture : 0 choix du micro USB, 1 micro retiré, 2 dictée en cours, 3 texte dans le brouillon."""
    if index == 0:
        state = eval_js(
            evidence,
            """async () => {
              const option = document.querySelector('.voice-microphone-option[data-id="fake-usb"] input');
              option.click();
              await new Promise((resolve) => setTimeout(resolve, 600));
              const marked = [...document.querySelectorAll(".voice-microphone-option")].find((node) => node.dataset.selected === "true");
              return {chosen: marked?.dataset.id, mark: marked?.querySelector(".voice-microphone-mark")?.textContent};
            }""",
        )
        if state.get("chosen") != "fake-usb" or state.get("mark") != "Choisi":
            raise SystemExit(f"le micro choisi n’est pas marqué: {state}")
    if index == 1:
        eval_js(evidence, close_overlays_js())
        eval_js(evidence, """async () => { await window.GrokbotPanels.api("/api/voice/debug", {command: "configure", unplugged: ["fake-usb"]}); document.getElementById("settings-button").click(); return true; }""")
        state = wait_js(
            evidence,
            """() => {
              const panel = document.querySelector('[data-panel="voice"].voice-microphone-settings');
              const notice = panel?.querySelector(".voice-settings-notice")?.textContent || "";
              const marked = panel ? [...panel.querySelectorAll(".voice-microphone-option")].find((node) => node.dataset.selected === "true") : null;
              if (!notice.includes("n’est plus disponible")) return null;
              panel.scrollIntoView({block: "center"});
              return {ok: true, notice, chosen: marked?.dataset.id, ids: [...panel.querySelectorAll(".voice-microphone-option")].map((node) => node.dataset.id)};
            }""",
            "phrase de repli absente",
        )
        if state["notice"] != "Le microphone choisi n’est plus disponible. Ezerbot utilise le microphone par défaut." or state["chosen"] != "" or "fake-usb" in state["ids"]:
            raise SystemExit(f"repli du micro incorrect: {state}")
        eval_js(evidence, """async () => { await window.GrokbotPanels.api("/api/voice/debug", {command: "configure", unplugged: []}); return true; }""")
    if index == 2:
        eval_js(evidence, close_overlays_js())
        eval_js(evidence, """() => { document.getElementById("voice-microphone").click(); return true; }""")
        state = wait_js(
            evidence,
            """() => {
              const text = document.querySelector(".voice-status")?.textContent || "";
              const time = document.querySelector(".voice-time")?.textContent || "";
              return text.startsWith("Écoute en cours · Microphone intégré (test)") && time.endsWith("/ 5:00") ? {ok: true, text, time} : null;
            }""",
            "dictée non démarrée sur le micro par défaut",
        )
    if index == 3:
        eval_js(evidence, """() => { document.getElementById("voice-microphone").click(); return true; }""")
        wait_js(
            evidence,
            """() => {
              const text = document.querySelector(".voice-status")?.textContent || "";
              const editor = document.querySelector('.gb-editor[aria-label="Votre message"]')?.innerText || "";
              const sent = document.querySelectorAll('[data-message-id]').length;
              return text.startsWith("Transcription ajoutée au brouillon") && editor.includes("mémo vocal") && !sent ? {ok: true} : null;
            }""",
            "texte absent du brouillon, ou message parti tout seul",
        )


def shot_names(sphere: str, theme: str, width: int, height: int) -> list[str]:
    size = f"{width}x{height}"
    if sphere == "reunions-routines":
        return [f"reunions-{theme}-{size}.png", f"routines-{theme}-{size}.png"]
    if sphere == "pages":
        return [f"avant-{theme}-{size}.png", f"apres-{theme}-{size}.png"]
    if sphere == "memo-vocal":
        return [f"avant-{theme}-{size}.png", f"vitesse-{theme}-{size}.png", f"saut-{theme}-{size}.png", f"sans-temps-{theme}-{size}.png"]
    if sphere == "iphone-reponse":
        return [f"avant-{theme}-{size}.png", f"toucher-{theme}-{size}.png", f"feuille-{theme}-{size}.png", f"envoyee-{theme}-{size}.png"]
    if sphere == "voix-bots":
        return [f"ezerbot-{theme}-{size}.png", f"chef-{theme}-{size}.png", f"refus-{theme}-{size}.png"]
    if sphere == "voix-appel":
        return [f"avant-{theme}-{size}.png", f"appel-{theme}-{size}.png", f"transcript-{theme}-{size}.png", f"fil-{theme}-{size}.png"]
    if sphere == "taches-de-fond":
        return [f"invite-{theme}-{size}.png", f"saisie-{theme}-{size}.png", f"mot-de-passe-{theme}-{size}.png"]
    if sphere == "dictee":
        return [f"microphone-{theme}-{size}.png", f"repli-{theme}-{size}.png", f"ecoute-{theme}-{size}.png", f"brouillon-{theme}-{size}.png"]
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
        if sphere == "iphone-reponse":
            iphone_reply_step(evidence, index)
        if sphere == "taches-de-fond":
            background_task_step(evidence, index)
        if sphere == "dictee":
            dictation_step(evidence, index)
        if sphere == "voix-bots":
            voice_bots_step(evidence, index)
        if sphere == "voix-appel":
            voice_call_step(evidence, index)
        if sphere == "memo-vocal":
            memo_step(evidence, index)
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
