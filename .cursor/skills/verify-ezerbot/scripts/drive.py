#!/usr/bin/env python3
"""Drive one or every mapped sphere through the browser harness.

Screenshots are PNG, dark and light, at 1140x780 and 820x560.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sqlite3
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from verify_lib import evidence_dir, load_run, png_size, session_id

SIZES = ((1140, 780), (820, 560))
THEMES = ("dark", "light")
CANVAS = {"dark": "#14161a", "light": "#ffffff"}
# EZERBOT_VERIFY_BASELINE=1 rejoue la sphère approbation-rappel sur un produit sans le rappel : mêmes gestes, aucune attente des éléments nouveaux, seules les captures « avant » comptent.
BASELINE = os.environ.get("EZERBOT_VERIFY_BASELINE") == "1"
SPHERES = ("accueil", "parametres-connecteurs", "modeles", "reunions-routines", "memoire", "bibliotheque", "images", "images-reference", "pages", "dictee", "voix-bots", "memo-vocal", "iphone-reponse", "voix-appel", "reponse-fil", "taches-de-fond", "approbation-non-bloquante", "approbation-tardive", "approbation-rappel", "sujets", "mode-yolo")


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


def waiting_toggle_js(on: bool) -> str:
    """Met l'interrupteur « Continuer sans attendre mes approbations » dans l'état voulu (sans clic s'il y est déjà)."""
    return f"""() => {{
      const input = document.querySelector('.waiting-settings [name="waiting-task"]');
      if (!input) throw new Error("interrupteur d'approbation absent");
      if (input.checked !== {str(on).lower()}) input.click();
      return true;
    }}"""


def waiting_step(evidence: Path, index: int) -> None:
    """actif : réglage par défaut ; inactif : interrupteur coupé, choix grisés et aide « 10 minutes »."""
    on = index == 0
    eval_js(evidence, waiting_toggle_js(on))
    wait_js(
        evidence,
        f"""() => {{
          const q = (name) => document.querySelector(`.waiting-settings [name="${{name}}"]`);
          const section = document.querySelector(".waiting-settings");
          if (!section || q("waiting-task").checked !== {str(on).lower()}) return null;
          if (q("waiting-validity").disabled === {str(on).lower()} || q("waiting-reminder").disabled === {str(on).lower()}) return null;
          const text = section.innerText || "";
          if ({str(not on).lower()} && !text.includes("pendant 10 minutes, comme avant")) return null;
          section.scrollIntoView({{block: "center"}});
          return {{ok: true}};
        }}""",
        "réglage d'approbation non mis à jour",
    )


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
              // Linear n'a ni champ ni secret non plus, et sa fiche sépare la lecture des effets par carte brouillon.
              const linear = panel.querySelector('[data-connector="linear"]');
              const linearFree = !!linear && !linear.querySelector("form") && linear.querySelectorAll('input:not([type="checkbox"])').length === 0;
              const linearActions = !!linear && /issues/i.test(linear.textContent) && /carte brouillon/i.test(linear.textContent);
              // Microsoft 365 non plus : sa fiche nomme Outlook et sépare la lecture du courriel par carte brouillon.
              const microsoft = panel.querySelector('[data-connector="microsoft365"]');
              const microsoftFree = !!microsoft && !microsoft.querySelector("form") && microsoft.querySelectorAll('input:not([type="checkbox"])').length === 0;
              const microsoftActions = !!microsoft && /Outlook/.test(microsoft.textContent) && /carte brouillon/i.test(microsoft.textContent);
              // Jira Cloud non plus, et sa fiche sépare la lecture de l'effet par carte brouillon.
              const jira = panel.querySelector('[data-connector="jira"]');
              const jiraFree = !!jira && !jira.querySelector("form") && jira.querySelectorAll('input:not([type="checkbox"])').length === 0;
              const jiraActions = !!jira && /issues/i.test(jira.textContent) && /carte brouillon/i.test(jira.textContent);
              return heading === "Connecteurs" && add && reunions && gmailFree && gmailReads && linearFree && linearActions && microsoftFree && microsoftActions && jiraFree && jiraActions ? {ok: true, heading, reunions, gmailFree, gmailReads, linearFree, linearActions, microsoftFree, microsoftActions, jiraFree, jiraActions} : null;
            }""",
            "connecteurs non prêts",
        )
    if sphere == "approbation-non-bloquante":
        eval_js(evidence, close_overlays_js())
        eval_js(evidence, """() => { document.getElementById("settings-button").click(); return document.getElementById("settings-dialog").open; }""")
        wait_js(evidence, """() => (document.querySelector(".waiting-settings") ? {ok: true} : null)""", "réglage d'approbation absent")
        # Les captures précédentes ont pu laisser le réglage désactivé : on repart de l'état par défaut.
        eval_js(evidence, waiting_toggle_js(True))
        return wait_js(
            evidence,
            """() => {
              const q = (name) => document.querySelector(`.waiting-settings [name="${name}"]`);
              const section = document.querySelector(".waiting-settings");
              if (!section || !q("waiting-task")?.checked || q("waiting-validity").disabled) return null;
              section.scrollIntoView({block: "center"});
              const text = section.innerText || "";
              const defaults = q("waiting-validity").value === "24h" && q("waiting-reminder").value === "1h" && !q("waiting-chat").checked;
              const options = [...q("waiting-validity").options].map((option) => option.value).join(",");
              return defaults && options === "1h,6h,24h,7d" && text.includes("Continuer sans attendre mes approbations") && text.includes("Rien n’est exécuté sans votre feu vert")
                ? {ok: true, validity: q("waiting-validity").value, reminder: q("waiting-reminder").value, chat: q("waiting-chat").checked} : null;
            }""",
            "réglage d'approbation non prêt",
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
    if sphere == "images-reference":
        # Parcours 2 : une photo jointe, « Le même style, en 16:9 », et l'image large à côté. Un seul passage crée la conversation.
        eval_js(evidence, close_overlays_js())
        if not eval_js(evidence, """() => !!document.querySelector('.gb-generated-image-card[data-image-state="done"]')"""):
            eval_js(
                evidence,
                """async () => {
                  const panels = window.GrokbotPanels;
                  const { conversation } = await panels.api("/api/conversations", {});
                  const canvas = document.createElement("canvas");
                  canvas.width = 240;
                  canvas.height = 180;
                  const paint = canvas.getContext("2d");
                  const sky = paint.createLinearGradient(0, 0, 0, 180);
                  sky.addColorStop(0, "#f2a65a");
                  sky.addColorStop(1, "#7a3e65");
                  paint.fillStyle = sky;
                  paint.fillRect(0, 0, 240, 180);
                  paint.fillStyle = "#fff3c4";
                  paint.beginPath();
                  paint.arc(170, 70, 26, 0, Math.PI * 2);
                  paint.fill();
                  paint.fillStyle = "#2a1a33";
                  paint.fillRect(0, 140, 240, 40);
                  const blob = await new Promise((resolve) => canvas.toBlob(resolve, "image/png"));
                  const upload = await fetch("/api/attachments", { method: "POST", headers: { "Content-Type": "image/png", "X-Grokbot-Request": "local" }, body: blob });
                  const photo = await upload.json();
                  if (!photo.id) throw new Error("photo non importée");
                  const draft = "Le même style, en 16:9";
                  for (let attempt = 0; attempt < 50; attempt++) {
                    const reply = await fetch("/api/attachments/precaption", { method: "POST", headers: { "Content-Type": "application/json", "X-Grokbot-Request": "local" }, body: JSON.stringify({ conversation_id: conversation.id, attachment_id: photo.id, draft }) });
                    const caption = await reply.json();
                    if (caption.caption?.status === "ready") break;
                    await new Promise((resolve) => setTimeout(resolve, 200));
                  }
                  await panels.api("/api/send", { conversation_id: conversation.id, message_id: "style-" + Date.now(), body: draft, attachments: [photo.id] });
                  await panels.refresh();
                  panels.openConversation(conversation.id);
                  return true;
                }""",
            )
        state = wait_js(
            evidence,
            """() => {
              const card = document.querySelector('.gb-generated-image-card[data-image-state="done"]');
              const image = card?.querySelector("img");
              const photo = [...document.querySelectorAll(".gb-image-grid img")].find((node) => !card.contains(node));
              if (!card || !image || !image.complete || !image.naturalWidth || !photo || !photo.complete) return null;
              const box = image.getBoundingClientRect();
              return {ok: true, natural: image.naturalWidth / image.naturalHeight, shown: box.width / box.height, photo: photo.naturalWidth / photo.naturalHeight, refused: !!document.querySelector('.gb-generated-image-card[data-image-state="failed"]')};
            }""",
            "image large et photo absentes",
            timeout=40,
        )
        if state["refused"] or abs(state["natural"] - 1.5) > 0.01 or abs(state["shown"] - state["natural"]) > 0.03:
            raise SystemExit(f"le cadre large ne se voit pas dans la carte: {state}")
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
    if sphere == "reponse-fil":
        # Une conversation neuve : une question, la réponse du faux bot, puis le fil ouvert sur la question.
        eval_js(evidence, close_overlays_js())
        eval_js(
            evidence,
            """async () => {
              const panels = window.GrokbotPanels;
              const { conversation } = await panels.api("/api/conversations", {});
              window.__filMain = conversation.id;
              window.__filQuestion = "question-" + Date.now();
              await panels.api("/api/send", {conversation_id: conversation.id, message_id: window.__filQuestion, body: "Quelle est la capitale du Canada ?"});
              return true;
            }""",
        )
        wait_js(
            evidence,
            """async () => {
              const panels = window.GrokbotPanels;
              await panels.refresh();
              const conversation = panels.state.conversations.find((item) => item.id === window.__filMain);
              return conversation?.messages.some((message) => message.role === "assistant" && message.body && message.state === "echoed") ? {ok: true} : null;
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
    if sphere == "mode-yolo":
        return prepare_mode_yolo(evidence)
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
    if sphere == "approbation-tardive":
        eval_js(evidence, close_overlays_js())
        # Le shell est permis et le chat direct continue sans attendre : la carte survit au tour du bot.
        # Le Mode YOLO, actif par défaut, lèverait la carte : la sphère joue « Demander avant d'agir ».
        return eval_js(
            evidence,
            """async () => {
              const panels = window.GrokbotPanels;
              if (panels.state.yolo) await panels.api("/api/yolo/mode", { mode: "ask" });
              await panels.api("/api/local/bot", { bot_id: "grokbot", shell: true });
              await panels.api("/api/approvals/waiting", { task: true, chat: true, validity: "24h", reminder: "1h" });
              await panels.refresh();
              return { ok: true, waiting: panels.state.forms.waiting };
            }""",
        )
    if sphere == "approbation-rappel":
        eval_js(evidence, close_overlays_js())
        settle_waiting_cards(evidence)
        # Même préparation que la décision tardive : la carte survit au tour du bot et reste en attente.
        return eval_js(
            evidence,
            """async () => {
              const panels = window.GrokbotPanels;
              await panels.api("/api/local/bot", { bot_id: "grokbot", shell: true });
              await panels.api("/api/approvals/waiting", { task: true, chat: true, validity: "24h", reminder: "1h" });
              await panels.refresh();
              return { ok: true, waiting: panels.state.forms.waiting };
            }""",
        )
    if sphere == "sujets":
        return prepare_topics(evidence)
    raise SystemExit(f"sphère inconnue: {sphere}")


def settle_waiting_cards(evidence: Path) -> None:
    """Le temps passe sur une base jetable : les cartes laissées par la combinaison précédente expirent, pour que chaque combinaison ait sa seule carte."""
    run = load_run(evidence)
    with sqlite3.connect(run["db"], timeout=15) as conn:
        conn.execute(
            "UPDATE forms_requests SET kind_json=json_set(kind_json,'$.expires_at',?) WHERE status='pending' AND json_extract(kind_json,'$.kind')='approval'",
            (int(time.time() * 1000) - 1000,),
        )
    deadline = time.time() + 15
    while time.time() < deadline:
        with sqlite3.connect(run["db"], timeout=15) as conn:
            if not conn.execute("SELECT COUNT(*) FROM forms_requests WHERE status='pending' AND json_extract(kind_json,'$.kind')='approval'").fetchone()[0]:
                return
        time.sleep(0.3)
    raise SystemExit("des cartes d'approbation restent en attente après leur échéance")


def age_waiting_card(evidence: Path, hours: int = 2, validity_hours: int = 24) -> None:
    """La seule carte en attente a été posée il y a `hours` h et vaut `validity_hours` h : l'horloge de la sphère, jamais celle de la base du capitaine."""
    run = load_run(evidence)
    created = int(time.time() * 1000) - hours * 3_600_000
    with sqlite3.connect(run["db"], timeout=15) as conn:
        pending = "status='pending' AND json_extract(kind_json,'$.kind')='approval'"
        moved = conn.execute(
            f"UPDATE wrapper_messages SET created_at=? WHERE kind='approval' AND message_id IN (SELECT entry_id FROM forms_requests WHERE {pending})",
            (created,),
        ).rowcount
        changed = conn.execute(
            f"UPDATE forms_requests SET kind_json=json_set(kind_json,'$.expires_at',?) WHERE {pending}",
            (created + validity_hours * 3_600_000,),
        ).rowcount
    if moved != 1 or changed != 1:
        raise SystemExit(f"{moved} message(s) et {changed} carte(s) en attente au lieu d'une")


WAIT_LINE = r"^En attente de vous depuis 2 h( 0\d)? · expire dans (22 h|21 h \d\d)$"


def reminder_step(evidence: Path, index: int) -> None:
    """rappel : la notification du rappel dans la liste, depuis un autre fil ; attente : sa carte avec la ligne d'ancienneté ; bandeau : la reprise de l'app."""
    if index == 0:
        late_ask(evidence)
        eval_js(
            evidence,
            """() => {
              const request = window.GrokbotPanels.state.forms.requests.find((item) => item.status?.status === "pending" && item.kind?.kind === "approval");
              window.__verifyWaitingConversation = request.route.target_conversation_id;
              return request.route.target_conversation_id;
            }""",
        )
        # Un autre fil est à l'écran avant que la carte vieillisse : le rappel n'est pas lu d'avance par un sondage du fil visible.
        eval_js(evidence, """() => { document.getElementById("new-chat").click(); return true; }""")
        wait_js(
            evidence,
            """() => {
              const editor = document.querySelector('.gb-editor[aria-label="Votre message"]');
              return editor && !document.querySelector('[data-card-kind="approval"]') ? {ok: true} : null;
            }""",
            "conversation neuve non prête",
        )
        age_waiting_card(evidence)
        eval_js(evidence, """() => { document.querySelector(".notifications-button").click(); return true; }""")
        if BASELINE:
            time.sleep(2)
            return
        state = wait_js(
            evidence,
            """async () => {
              await window.GrokbotPanels.refresh();
              const items = [...document.querySelectorAll("dialog.notifications-dialog[open] .notification-item")];
              const row = items.find((node) => node.innerText.includes("Une demande attend votre décision depuis 1 h"));
              return row ? {ok: true, count: items.length, text: row.innerText} : null;
            }""",
            "le rappel n'est pas dans la liste des notifications",
            timeout=40,
        )
        if "NE_JAMAIS" in state["text"] or "run_shell" in state["text"] or "printf" in state["text"]:
            raise SystemExit(f"le rappel dit autre chose que le temps: {state}")
        return
    if index == 1:
        if BASELINE:
            eval_js(evidence, close_overlays_js())
            eval_js(
                evidence,
                """() => { window.GrokbotPanels.openConversation(window.__verifyWaitingConversation); return true; }""",
            )
            wait_js(
                evidence,
                """() => {
                  const card = document.querySelector('[data-card-kind="approval"][data-request-status="pending"]');
                  if (!card) return null;
                  card.scrollIntoView({block: "center"});
                  return {ok: true};
                }""",
                "la carte en attente n'est pas à l'écran",
            )
            return
        eval_js(
            evidence,
            """() => {
              const row = [...document.querySelectorAll("dialog.notifications-dialog[open] .notification-item")].find((node) => node.innerText.includes("Une demande attend votre décision depuis 1 h"));
              row.click();
              return true;
            }""",
        )
        state = wait_js(
            evidence,
            f"""() => {{
              const line = document.querySelector("[data-approval-wait]");
              const card = document.querySelector('[data-card-kind="approval"][data-request-status="pending"]');
              if (!line || !card) return null;
              line.scrollIntoView({{block: "center"}});
              return {{ok: true, line: line.textContent, card: card.innerText, order: !!(line.compareDocumentPosition(card) & Node.DOCUMENT_POSITION_FOLLOWING), buttons: [...card.querySelectorAll("button")].map((node) => node.textContent)}};
            }}""",
            "la carte n'a pas sa ligne d'ancienneté",
            timeout=30,
        )
        if not re.match(WAIT_LINE, state["line"]) or not state["order"] or "Autoriser une fois" not in state["buttons"] or "En attente de vous" in state["card"]:
            raise SystemExit(f"la ligne d'ancienneté n'est pas celle attendue: {state}")
        return
    # Le bandeau ne naît qu'à la première lecture de l'état : la page est rechargée, comme une reprise de l'app.
    eval_js(evidence, close_overlays_js())
    eval_js(evidence, """() => { setTimeout(() => location.reload(), 50); return true; }""")
    wait_js(
        evidence,
        """() => {
          const editor = document.querySelector(".gb-editor");
          const badge = document.getElementById("mode-badge-text")?.textContent || "";
          return editor && badge === "ChatGPT" ? {badge} : null;
        }""",
        "Ezerbot n'est pas revenu après le rechargement",
        timeout=30,
    )
    if BASELINE:
        time.sleep(2)
        return
    state = wait_js(
        evidence,
        """() => {
          const banner = document.getElementById("approval-banner");
          if (!banner || banner.hidden) return null;
          return {ok: true, text: banner.innerText, close: banner.querySelector("button")?.getAttribute("aria-label"), link: !!banner.querySelector("a")};
        }""",
        "le bandeau de reprise n'est pas là",
        timeout=30,
    )
    if state["text"].strip() != "1 demande attend votre décision" or state["link"]:
        raise SystemExit(f"le bandeau n'est pas celui attendu: {state}")


# Âge, en heures, des fils de la sphère sujets : la barre latérale montre des temps relatifs.
TOPIC_AGES_HOURS = {"camion": 2, "adresse": 26, "declaration": 74, "cadeaux": 5, "crepes": 30}


def prepare_topics(evidence: Path) -> dict:
    """Deux sujets (Déménagement déplié, Impôts replié), cinq fils d'âges différents, et aucun fil vide laissé par un passage précédent."""
    eval_js(evidence, close_overlays_js())
    seeded = eval_js(
        evidence,
        """async () => {
          const panels = window.GrokbotPanels;
          await panels.refresh();
          if (panels.state.topics.items.some((topic) => topic.name === "Déménagement")) return {fresh: false};
          try { await panels.api("/api/bots", {id: "chef", name: "Chef", color: "cyan", instructions: "Organise le travail du groupe."}); } catch { /* déjà créé */ }
          const move = (await panels.api("/api/topics", {name: "Déménagement", description: "Camion, changement d’adresse et boîte postale."})).topic;
          const tax = (await panels.api("/api/topics", {name: "Impôts"})).topic;
          const make = async (title, extra) => (await panels.api("/api/conversations", {title, ...extra})).conversation.id;
          return {fresh: true, ids: {
            camion: await make("Louer un camion", {topic_id: move.id}),
            adresse: await make("Changer d’adresse", {topic_id: move.id, bot_ids: ["grokbot", "chef"]}),
            declaration: await make("Déclaration de revenus", {topic_id: tax.id}),
            cadeaux: await make("Idées de cadeaux"),
            crepes: await make("Recette de crêpes"),
          }};
        }""",
    )
    if seeded.get("fresh"):
        # Le temps passe sur une base jetable : chaque fil est vieilli sans toucher à ses messages.
        run = load_run(evidence)
        now = int(time.time() * 1000)
        with sqlite3.connect(run["db"], timeout=15) as conn:
            for key, hours in TOPIC_AGES_HOURS.items():
                changed = conn.execute(
                    "UPDATE wrapper_conversations SET updated_at=? WHERE id=?",
                    (now - hours * 3600 * 1000, seeded["ids"][key]),
                ).rowcount
                if changed != 1:
                    raise SystemExit(f"le fil {key} n'a pas pu être vieilli")
    eval_js(
        evidence,
        """async () => {
          const panels = window.GrokbotPanels;
          panels.leaveConversation();
          await panels.refresh();
          // Un « Nouveau fil » resté vide d'un passage précédent ne fausse pas la barre latérale.
          for (const conversation of panels.state.conversations) {
            if (panels.state.topics.membership[conversation.id] && conversation.title === "Nouvelle conversation" && !conversation.messages.length) {
              await panels.api("/api/conversations/delete", {id: conversation.id});
            }
          }
          await panels.refresh();
          return true;
        }""",
    )
    return wait_js(
        evidence,
        """() => {
          const folders = [...document.querySelectorAll("#topic-list .topic-folder")];
          const toggle = (name) => folders.find((folder) => folder.querySelector(".topic-name")?.textContent === name)?.querySelector(".topic-toggle");
          const move = toggle("Déménagement"), tax = toggle("Impôts");
          if (!move || !tax) return null;
          // Déménagement déplié, Impôts replié : l'état se lit sur aria-expanded.
          if (move.getAttribute("aria-expanded") !== "true") { move.click(); return null; }
          if (tax.getAttribute("aria-expanded") !== "false") { tax.click(); return null; }
          const times = [...document.querySelectorAll("#topic-list .topic-thread-time")].map((node) => node.textContent);
          const loose = [...document.querySelectorAll("#conversation-list .conversation-item")].map((node) => node.textContent);
          const page = document.getElementById("topic-page");
          return folders.length === 2 && times.length === 2 && loose.length === 2 && (!page || page.hidden) ? {ok: true, times, loose} : null;
        }""",
        "la barre latérale ne montre pas deux sujets (un déplié, un replié) et deux fils sans sujet",
    )


def topics_step(evidence: Path, index: int) -> None:
    """Avant chaque capture : 0 la barre latérale, 1 « Nouveau fil » vient de créer un fil rangé, 2 la page du sujet."""
    if index == 0:
        # Une fenêtre basse fait défiler la barre latérale : la capture se cale sur la section CONVERSATIONS.
        eval_js(
            evidence,
            """() => {
              const sidebar = document.getElementById("sidebar");
              sidebar.scrollTop = innerHeight < 700 ? sidebar.scrollHeight : 0;
              return true;
            }""",
        )
    if index == 1:
        eval_js(evidence, """() => { document.querySelector('#topic-list .topic-name').click(); return true; }""")
        wait_js(evidence, """() => { const page = document.getElementById("topic-page"); return page && !page.hidden && page.querySelector(".topic-new-thread") ? {ok: true} : null; }""", "la page du sujet ne s'ouvre pas")
        eval_js(evidence, """() => { document.querySelector("#topic-page .topic-new-thread").click(); return true; }""")
        state = wait_js(
            evidence,
            """() => {
              const active = document.querySelector("#topic-list .topic-thread.active");
              const title = document.getElementById("conversation-title").textContent;
              const page = document.getElementById("topic-page");
              return active && title === "Nouvelle conversation" && page.hidden ? {name: active.querySelector(".conversation-name").textContent} : null;
            }""",
            "« Nouveau fil » n'a pas ouvert un fil rangé dans le sujet",
        )
        if state["name"] != "Nouvelle conversation":
            raise SystemExit(f"le fil créé n'est pas celui attendu: {state}")
    if index == 2:
        # Le fil vide de la capture précédente est retiré, puis la page du sujet s'ouvre depuis l'accueil.
        eval_js(
            evidence,
            """async () => {
              const panels = window.GrokbotPanels;
              panels.leaveConversation();
              await panels.refresh();
              for (const conversation of panels.state.conversations) {
                if (panels.state.topics.membership[conversation.id] && conversation.title === "Nouvelle conversation" && !conversation.messages.length) {
                  await panels.api("/api/conversations/delete", {id: conversation.id});
                }
              }
              await panels.refresh();
              document.querySelector('#topic-list .topic-name').click();
              return true;
            }""",
        )
        state = wait_js(
            evidence,
            """() => {
              const page = document.getElementById("topic-page");
              if (!page || page.hidden) return null;
              return {
                title: page.querySelector(".topic-page-title")?.textContent,
                description: page.querySelector(".topic-page-description")?.textContent,
                threads: [...page.querySelectorAll(".topic-page-thread-title")].map((node) => node.textContent),
                memory: page.querySelector(".topic-memory h3")?.textContent,
                topbar: document.getElementById("conversation-title").textContent,
              };
            }""",
            "la page du sujet ne s'affiche pas",
        )
        if state["title"] != "Déménagement" or state["threads"] != ["Louer un camion", "Changer d’adresse"] or state["memory"] != "Mémoire du sujet" or state["topbar"] != "Déménagement":
            raise SystemExit(f"la page du sujet n'est pas celle attendue: {state}")


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


def late_ask(evidence: Path) -> dict:
    """Une conversation neuve : le bot demande la commande, son tour finit, la carte attend encore."""
    eval_js(evidence, """() => { document.getElementById("new-chat").click(); return true; }""")
    wait_js(
        evidence,
        """() => document.querySelector('.gb-editor[aria-label="Votre message"]') ? {ok: true} : null""",
        "conversation neuve non prête",
    )
    eval_js(
        evidence,
        """() => {
          const editor = document.querySelector('.gb-editor[aria-label="Votre message"]');
          editor.focus();
          if (!document.execCommand("insertText", false, "Envoie le résumé")) throw new Error("saisie refusée");
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
          const card = document.querySelector('[data-card-kind="approval"][data-request-status="pending"]');
          const buttons = card ? [...card.querySelectorAll("button")].map((node) => node.textContent) : [];
          const text = document.body.innerText;
          if (!card || !buttons.includes("Autoriser une fois") || !text.includes("J’attends son accord pour envoyer le résumé.")) return null;
          // La carte montre la commande (« Résumé envoyé » y figure) : seule une carte d'outil Terminée porte un résultat.
          const results = [...document.querySelectorAll('[data-card-kind="tool"][data-local-state="terminé"]')].filter((node) => node.innerText.includes("Résumé envoyé"));
          return {ok: true, buttons, sent: results.length > 0};
        }""",
        "la carte n'attend pas après la fin du tour du bot",
        timeout=40,
    )
    if state["sent"]:
        raise SystemExit(f"la commande est partie avant la décision: {state}")
    return state


def late_step(evidence: Path, index: int) -> None:
    """approuvee : « Autorisé une fois » ; reprise : le bot a repris et le résultat est arrivé ; expiree : carte grise, rien n'est parti."""
    if index == 1:
        # Même conversation que la capture précédente : le bas du fil montre le résultat puis la réponse du bot.
        eval_js(evidence, """() => { const last = [...document.querySelectorAll("[data-message-id]")].pop(); last?.scrollIntoView({block: "end"}); return true; }""")
        state = wait_js(
            evidence,
            """() => {
              const text = document.body.innerText;
              return text.includes("Tu as approuvé : envoyer le résumé. C’est parti.") ? {ok: true} : null;
            }""",
            "la réponse du bot après la décision n'est pas dans le fil",
        )
        return
    late_ask(evidence)
    if index == 0:
        eval_js(
            evidence,
            """() => {
              const card = document.querySelector('[data-card-kind="approval"][data-request-status="pending"]');
              [...card.querySelectorAll("button")].find((node) => node.textContent === "Autoriser une fois").click();
              return true;
            }""",
        )
        state = wait_js(
            evidence,
            """() => {
              const card = document.querySelector('[data-card-kind="approval"][data-request-status="resolved"]');
              const text = document.body.innerText;
              const results = [...document.querySelectorAll('[data-card-kind="tool"][data-local-state="terminé"]')].filter((node) => node.innerText.includes("Résumé envoyé"));
              if (!card || !text.includes("Tu as approuvé : envoyer le résumé") || results.length !== 1) return null;
              card.scrollIntoView({block: "start"});
              return {ok: true, card: card.innerText, results: results.length, buttons: card.querySelectorAll("button").length};
            }""",
            "le bot n'a pas repris avec le résultat après la décision tardive",
            timeout=40,
        )
        if "Autorisé une fois" not in state["card"] or "Le bot reprend dans la conversation." not in state["card"] or "La reprise a échoué" in state["card"] or state["buttons"]:
            raise SystemExit(f"la carte approuvée tard n'est pas celle attendue: {state}")
        return
    # Le temps passe sur une base jetable : l'échéance de la carte est ramenée à une seconde avant maintenant.
    run = load_run(evidence)
    with sqlite3.connect(run["db"], timeout=15) as conn:
        changed = conn.execute(
            "UPDATE forms_requests SET kind_json=json_set(kind_json,'$.expires_at',?) WHERE status='pending' AND json_extract(kind_json,'$.kind')='approval'",
            (int(time.time() * 1000) - 1000,),
        ).rowcount
    if changed != 1:
        raise SystemExit(f"{changed} carte(s) en attente au lieu d'une")
    state = wait_js(
        evidence,
        """async () => {
          await window.GrokbotPanels.refresh();
          const card = document.querySelector('[data-card-kind="approval"][data-request-status="expired"]');
          if (!card) return null;
          card.scrollIntoView({block: "start"});
          return {ok: true, card: card.innerText, grey: card.classList.contains("gb-approval-expired"), buttons: card.querySelectorAll("button").length, sent: [...document.querySelectorAll('[data-card-kind="tool"][data-local-state="terminé"]')].some((node) => node.innerText.includes("Résumé envoyé")) || document.body.innerText.includes("Tu as approuvé")};
        }""",
        "la carte n'est pas Expirée",
        timeout=40,
    )
    if "Cette demande a expiré. Aucune action n’a été faite." not in state["card"] or not state["grey"] or state["buttons"] or state["sent"]:
        raise SystemExit(f"la carte Expirée n'est pas celle attendue: {state}")
    # Un dernier rafraîchissement a pu replacer le fil : la carte grise doit être celle qu'on voit.
    time.sleep(0.6)
    eval_js(evidence, """() => { document.querySelector('[data-card-kind="approval"][data-request-status="expired"]').scrollIntoView({block: "center"}); return true; }""")


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


YOLO_PROMPTS = ("Liste mon dossier Documents", "Écris mes notes du jour")


def yolo_mode(evidence: Path):
    """« yolo » ou « ask » sur une build avec le Mode YOLO, None sur une build d'avant (la preuve « avant »)."""
    return eval_js(evidence, """() => window.GrokbotPanels?.state?.yolo?.mode ?? null""")


def send_in_new_chat(evidence: Path, text: str, new_chat: bool) -> None:
    if new_chat:
        eval_js(evidence, """() => { document.getElementById("new-chat").click(); return true; }""")
    wait_js(evidence, """() => document.querySelector('.gb-editor[aria-label="Votre message"]') ? {ok: true} : null""", "compositeur non prêt")
    eval_js(
        evidence,
        f"""() => {{
          const editor = document.querySelector('.gb-editor[aria-label="Votre message"]');
          editor.focus();
          if (!document.execCommand("insertText", false, {json.dumps(text, ensure_ascii=False)})) throw new Error("saisie refusée");
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


def prepare_mode_yolo(evidence: Path) -> dict:
    """Après : une commande et une écriture faites sans carte. Avant : les mêmes demandes, chacune derrière sa carte."""
    eval_js(evidence, close_overlays_js())
    mode = yolo_mode(evidence)
    run = load_run(evidence)
    work = Path(run["work"]) if run.get("work") else None
    if work is None:
        raise SystemExit("le dossier de travail du Mode YOLO est absent : relancer avec --sphere mode-yolo")
    if mode == "yolo":
        autos = """() => document.querySelectorAll('article[data-auto="yolo"]').length"""
        if eval_js(evidence, autos) < 2:
            first = eval_js(evidence, autos) == 0
            if first:
                send_in_new_chat(evidence, YOLO_PROMPTS[0], True)
                wait_js(evidence, """() => [...document.querySelectorAll('article[data-auto="yolo"]')].some((card) => card.textContent.includes("Commande exécutée")) ? {ok: true} : null""", "commande automatique absente", timeout=40)
            send_in_new_chat(evidence, YOLO_PROMPTS[1], False)
            wait_js(evidence, """() => [...document.querySelectorAll('article[data-auto="yolo"]')].some((card) => card.textContent.includes("Fichier créé")) ? {ok: true} : null""", "écriture automatique absente", timeout=40)
        state = wait_js(
            evidence,
            """() => {
              const cards = [...document.querySelectorAll('article[data-auto="yolo"]')];
              const badges = cards.every((card) => card.querySelector(".gb-auto-badge")?.textContent === "Fait automatiquement");
              const approvals = document.querySelectorAll('[data-card-kind="approval"]').length;
              return cards.length >= 2 && badges && approvals === 0 ? {ok: true, cards: cards.length, approvals} : null;
            }""",
            "cartes automatiques incomplètes ou carte d'approbation inattendue",
        )
        written = work / "Notes-du-jour.txt"
        if not written.is_file() or "valider le budget" not in written.read_text():
            raise SystemExit(f"le fichier écrit sans carte est absent ou incomplet: {written}")
        state["file"] = str(written.name)
        state["mode"] = mode
        return state
    if mode is not None:
        raise SystemExit(f"mode YOLO inattendu pour la preuve: {mode}")
    # Avant : le terminal et les fichiers sont éteints par défaut ; on les allume comme l'ancien parcours l'exigeait.
    ids = eval_js(evidence, """() => window.__yoloBefore || null""")
    if not ids:
        eval_js(
            evidence,
            """async () => {
              const panels = window.GrokbotPanels;
              await panels.api("/api/local/bot", { bot_id: "grokbot", shell: true, files: true });
              await panels.refresh();
              return true;
            }""",
        )
        ids = []
        for text in YOLO_PROMPTS:
            conversation = eval_js(
                evidence,
                """async () => {
                  const panels = window.GrokbotPanels;
                  const { conversation } = await panels.api("/api/conversations", {});
                  await panels.refresh();
                  panels.openConversation(conversation.id);
                  return conversation.id;
                }""",
            )
            ids.append(conversation)
            wait_js(evidence, f"""() => window.GrokbotPanels.state.conversations.find((c) => c.id === {json.dumps(conversation)}) ? {{ok: true}} : null""", "conversation absente")
            send_in_new_chat(evidence, text, False)
            wait_js(evidence, """() => document.querySelector('[data-card-kind="approval"]') ? {ok: true} : null""", "carte d'approbation absente", timeout=40)
        eval_js(evidence, f"""() => {{ window.__yoloBefore = {json.dumps(ids)}; return true; }}""")
    return {"ok": True, "mode": None, "conversations": ids}


def yolo_step(evidence: Path, index: int) -> None:
    mode = yolo_mode(evidence)
    eval_js(evidence, close_overlays_js())
    if index == 0:
        eval_js(evidence, """() => { document.getElementById("settings-button").click(); return document.getElementById("settings-dialog").open; }""")
        selector = '[data-panel="yolo"]' if mode else ".local-settings"
        wait_js(
            evidence,
            f"""() => {{
              const panel = document.querySelector({json.dumps(selector)});
              if (!panel) return null;
              panel.scrollIntoView({{block: "start"}});
              return {{ok: true, text: panel.innerText.slice(0, 120)}};
            }}""",
            "section Paramètres absente",
        )
        return
    if mode:
        wait_js(
            evidence,
            f"""() => {{
              const card = document.querySelectorAll('article[data-auto="yolo"]')[{index - 1}];
              if (!card) return null;
              card.scrollIntoView({{block: "center"}});
              return {{ok: true, text: card.innerText.slice(0, 200)}};
            }}""",
            "carte automatique absente",
        )
        return
    ids = eval_js(evidence, """() => window.__yoloBefore""")
    eval_js(evidence, f"""() => {{ window.GrokbotPanels.openConversation({json.dumps(ids[index - 1])}); return true; }}""")
    wait_js(
        evidence,
        """() => {
          const card = document.querySelector('[data-card-kind="approval"]');
          if (!card) return null;
          card.scrollIntoView({block: "center"});
          return {ok: true};
        }""",
        "carte d'approbation d'avant absente",
    )


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


def reply_in_thread_step(evidence: Path, index: int) -> None:
    """Avant chaque capture : 0 le fil principal, 1 le menu du message, 2 le sous-fil ouvert, 3 la réponse qui tient compte du parent, 4 retour au fil principal."""
    if index == 0:
        eval_js(evidence, """() => { window.GrokbotPanels.openConversation(window.__filMain); return true; }""")
        wait_js(
            evidence,
            """() => document.querySelector(`.gb-message[data-message-id="${window.__filQuestion}"]`) && document.getElementById("thread-banner").hidden ? {ok: true} : null""",
            "fil principal absent",
        )
    if index == 1:
        eval_js(
            evidence,
            """() => { document.querySelector(`.gb-message[data-message-id="${window.__filQuestion}"] button[aria-label="Actions du message"]`).click(); return true; }""",
        )
        wait_js(
            evidence,
            """() => [...document.querySelectorAll('[role="menu"][aria-label="Actions du message"] [role="menuitem"]')].some((item) => item.textContent === "Répondre en fil") ? {ok: true} : null""",
            "« Répondre en fil » absent du menu",
        )
    if index == 2:
        eval_js(
            evidence,
            """() => { [...document.querySelectorAll('[role="menuitem"]')].find((item) => item.textContent === "Répondre en fil").click(); return true; }""",
        )
        state = wait_js(
            evidence,
            """() => {
              const banner = document.getElementById("thread-banner");
              const title = document.getElementById("conversation-title").textContent;
              const quote = document.getElementById("thread-banner-quote").textContent;
              return !banner.hidden && title.startsWith("Fil : ") ? {title, quote} : null;
            }""",
            "sous-fil non ouvert",
        )
        if "Quelle est la capitale du Canada" not in state["quote"]:
            raise SystemExit(f"le bandeau ne cite pas le message parent: {state}")
    if index == 3:
        eval_js(
            evidence,
            """async () => {
              const panels = window.GrokbotPanels;
              const sub = Object.keys(panels.state.threads).find((id) => panels.state.threads[id].parent_message_id === window.__filQuestion);
              window.__filSub = sub;
              await panels.api("/api/send", {conversation_id: sub, message_id: "suite-" + Date.now(), body: "Et en un mot ?"});
              return true;
            }""",
        )
        wait_js(
            evidence,
            """async () => {
              const panels = window.GrokbotPanels;
              await panels.refresh();
              const sub = panels.state.conversations.find((item) => item.id === window.__filSub);
              const reply = sub?.messages.find((message) => message.role === "assistant" && message.state === "echoed" && message.body.includes("Quelle est la capitale du Canada"));
              const main = panels.state.conversations.find((item) => item.id === window.__filMain);
              return reply && main.messages.every((message) => message.body !== "Et en un mot ?") ? {ok: true} : null;
            }""",
            "la réponse du sous-fil ne tient pas compte du message parent",
            timeout=40,
        )
    if index == 4:
        eval_js(evidence, """() => { document.getElementById("thread-banner-open").click(); return true; }""")
        wait_js(
            evidence,
            """() => document.getElementById("thread-banner").hidden && document.querySelector(`.gb-message[data-message-id="${window.__filQuestion}"]`) ? {ok: true} : null""",
            "retour au fil principal absent",
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
    if sphere == "images-reference":
        return [f"photo-{theme}-{size}.png", f"resultat-{theme}-{size}.png"]
    if sphere == "memo-vocal":
        return [f"avant-{theme}-{size}.png", f"vitesse-{theme}-{size}.png", f"saut-{theme}-{size}.png", f"sans-temps-{theme}-{size}.png"]
    if sphere == "iphone-reponse":
        return [f"avant-{theme}-{size}.png", f"toucher-{theme}-{size}.png", f"feuille-{theme}-{size}.png", f"envoyee-{theme}-{size}.png"]
    if sphere == "reponse-fil":
        return [f"avant-{theme}-{size}.png", f"menu-{theme}-{size}.png", f"sous-fil-{theme}-{size}.png", f"reponse-{theme}-{size}.png", f"principal-{theme}-{size}.png"]
    if sphere == "sujets":
        return [f"barre-{theme}-{size}.png", f"nouveau-fil-{theme}-{size}.png", f"sujet-{theme}-{size}.png"]
    if sphere == "approbation-non-bloquante":
        return [f"actif-{theme}-{size}.png", f"inactif-{theme}-{size}.png"]
    if sphere == "approbation-tardive":
        return [f"approuvee-{theme}-{size}.png", f"reprise-{theme}-{size}.png", f"expiree-{theme}-{size}.png"]
    if sphere == "approbation-rappel":
        return [f"rappel-{theme}-{size}.png", f"attente-{theme}-{size}.png", f"bandeau-{theme}-{size}.png"]
    if sphere == "voix-bots":
        return [f"ezerbot-{theme}-{size}.png", f"chef-{theme}-{size}.png", f"refus-{theme}-{size}.png"]
    if sphere == "voix-appel":
        return [f"avant-{theme}-{size}.png", f"appel-{theme}-{size}.png", f"transcript-{theme}-{size}.png", f"fil-{theme}-{size}.png"]
    if sphere == "mode-yolo":
        return [f"parametres-{theme}-{size}.png", f"terminal-{theme}-{size}.png", f"ecriture-{theme}-{size}.png"]
    if sphere == "taches-de-fond":
        return [f"invite-{theme}-{size}.png", f"saisie-{theme}-{size}.png", f"mot-de-passe-{theme}-{size}.png"]
    if sphere == "dictee":
        return [f"microphone-{theme}-{size}.png", f"repli-{theme}-{size}.png", f"ecoute-{theme}-{size}.png", f"brouillon-{theme}-{size}.png"]
    if sphere == "parametres-connecteurs":
        return [f"{theme}-{size}.png", f"linear-{theme}-{size}.png", f"microsoft-{theme}-{size}.png", f"jira-{theme}-{size}.png"]
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
        if sphere == "parametres-connecteurs" and index in (1, 2, 3):
            # Un plan par fiche ajoutée au catalogue (Linear, Microsoft 365, Jira Cloud), que le titre Connecteurs en haut du premier laisse hors cadre.
            service = ["linear", "microsoft365", "jira"][index - 1]
            eval_js(
                evidence,
                f"""() => {{
                  const card = document.querySelector('[data-panel="mcp"] [data-connector="{service}"]');
                  if (!card) throw new Error("fiche {service} absente");
                  card.scrollIntoView({{block: "center"}});
                  return true;
                }}""",
            )
        if sphere == "pages" and index == 1:
            pages_request(evidence)
        if sphere == "images-reference":
            # La photo ne tient pas avec l'image dans une fenêtre minimale : un plan sur la photo, un plan sur le résultat.
            where = "start" if index == 0 else "end"
            eval_js(
                evidence,
                f"""() => {{
                  const card = document.querySelector('.gb-generated-image-card[data-image-state="done"]');
                  const photo = [...document.querySelectorAll(".gb-image-grid img")].find((node) => !card.contains(node));
                  ({"photo" if index == 0 else "card"}).scrollIntoView({{block: "{where}"}});
                  return true;
                }}""",
            )
        if sphere == "iphone-reponse":
            iphone_reply_step(evidence, index)
        if sphere == "mode-yolo":
            yolo_step(evidence, index)
        if sphere == "approbation-non-bloquante":
            waiting_step(evidence, index)
        if sphere == "approbation-tardive":
            late_step(evidence, index)
        if sphere == "approbation-rappel":
            reminder_step(evidence, index)
        if sphere == "taches-de-fond":
            background_task_step(evidence, index)
        if sphere == "reponse-fil":
            reply_in_thread_step(evidence, index)
        if sphere == "sujets":
            topics_step(evidence, index)
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
