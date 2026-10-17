"use strict";

(() => {
  const panels = window.GrokbotPanels;
  if (!panels) return;
  // REA reports/server-context-residual.md RC01: 1s debounce, 20 MiB,
  // png/jpeg/gif/webp/bmp, 150s deadline and controller-wide SKIPPED backoff.
  // The wrapper owns the five-pure-human projection and the caption cache.
  const MAX_BYTES = 20 * 1024 * 1024;
  const MAX_IMAGES = 10; // Matches the wrapper's per-message limit.
  const DEADLINE = 150_000;
  const types = new Set(["image/png", "image/jpeg", "image/gif", "image/webp", "image/bmp"]);
  const extensions = { png: "image/png", jpg: "image/jpeg", jpeg: "image/jpeg", gif: "image/gif", webp: "image/webp", bmp: "image/bmp" };
  const buckets = new Map();
  const views = new WeakMap();
  let context, tray, list, picker, pause, cooldown = 0, cooldownTimer;
  function backoff(until) {
    cooldown = Math.max(cooldown, until);
  }
  function remainingWait(milliseconds) {
    const seconds = Math.ceil(milliseconds / 1000);
    return `${Math.floor(seconds / 60)} min ${String(seconds % 60).padStart(2, "0")} s`;
  }

  function node(tag, className, text) {
    const result = document.createElement(tag);
    if (className) result.className = className;
    if (text !== undefined) result.textContent = text;
    return result;
  }
  const key = (ctx) => ctx.conversation?.id || `new:${ctx.bot?.id || "grokbot"}`;
  function bucket(ctx = context) {
    const id = key(ctx);
    if (!buckets.has(id)) buckets.set(id, { key: id, bot: ctx.bot?.id, conversation: ctx.conversation?.id, items: [] });
    const group = buckets.get(id);
    // Registration precedes the first /api/state response.
    group.bot ||= ctx.bot?.id;
    return group;
  }
  const active = (group) => group.key === key(context);
  const present = (group, item) => group.items.includes(item);
  function changed() { render(); panels.composerChanged(); }
  function discard(item) {
    clearTimeout(item.timer);
    item.controller?.abort();
    URL.revokeObjectURL(item.preview);
    item.revision++;
  }
  function remove(group, item) {
    discard(item);
    group.items = group.items.filter((entry) => entry !== item);
    changed();
  }
  function captionText(item) {
    if (item.upload === "pending") return "Import en cours…";
    if (item.upload === "failed") return "Import impossible";
    return { waiting: "Description en attente…", pending: "Description en cours…", ready: "Description prête", skipped: "Description ignorée", paused: cooldown > Date.now() ? "Description en pause" : "Description à préparer", failed: "Description indisponible" }[item.caption];
  }
  function render() {
    if (!list || !context) return;
    clearTimeout(cooldownTimer);
    const remaining = Math.max(0, cooldown - Date.now());
    const explanation = remaining > 0 ? `Prédescriptions en pause · reprise dans ${remainingWait(remaining)}` : "";
    let waiting = false;
    const group = bucket();
    const chips = group.items.map((item) => {
      if (!views.has(item)) views.set(item, createChip(group, item));
      const { chip, status, retry } = views.get(item);
      chip.dataset.status = item.upload === "ready" ? item.caption : item.upload;
      const label = captionText(item);
      if (status.textContent !== label) status.textContent = label;
      status.title = item.error || "";
      retry.hidden = !(item.upload === "failed" || ["failed", "skipped", "paused"].includes(item.caption));
      retry.disabled = !retry.hidden && item.upload === "ready" && remaining > 0;
      retry.title = retry.disabled ? explanation : "Réessayer la préparation de cette image";
      if (retry.disabled) retry.setAttribute("aria-describedby", pause.id);
      else retry.removeAttribute("aria-describedby");
      waiting ||= retry.disabled;
      return chip;
    });
    pause.hidden = !waiting;
    const visibleExplanation = waiting ? explanation : "";
    if (pause.textContent !== visibleExplanation) pause.textContent = visibleExplanation;
    // Keep the remaining time and expiry accurate even without server polling.
    // Tick only while the active tray actually shows a paused retry.
    if (waiting) cooldownTimer = setTimeout(render, Math.min(1000, remaining));
    const visible = new Set(chips);
    for (const child of Array.from(list.children)) if (!visible.has(child)) child.remove();
    // Do not detach/reappend an unchanged chip: even moving the same node can
    // blur its focused button in WebKit. Status regions also remain stable.
    chips.forEach((chip, index) => {
      if (list.children[index] !== chip) list.insertBefore(chip, list.children[index] || null);
    });
  }
  function createChip(group, item) {
    const chip = node("li", "attachment-chip");
    const image = node("img", "attachment-preview");
    image.src = item.preview;
    image.alt = `Aperçu de ${item.file.name || "l’image collée"}`;
    const copy = node("div", "attachment-copy");
    const name = node("strong", "", item.file.name || "Image collée");
    name.title = item.file.name || "Image collée";
    const status = node("span", "attachment-status");
    status.setAttribute("role", "status");
    const retry = node("button", "attachment-retry", "Réessayer");
    retry.type = "button";
    retry.addEventListener("click", () => {
      if (item.upload === "failed") upload(group, item);
      else schedule(group, item, true);
    });
    copy.append(name, status, retry);
    const close = node("button", "attachment-remove", "×");
    close.type = "button";
    close.setAttribute("aria-label", `Retirer ${item.file.name || "l’image"}`);
    close.addEventListener("click", () => remove(group, item));
    chip.append(image, copy, close);
    return { chip, status, retry };
  }

  // The API requires an existing conversation (wrapper review r1/r2). Create it
  // on staging, not on send. Reuse the shell's saved-draft handoff when selecting
  // it, so even edits made while creation/refresh is in flight are preserved.
  async function ensureConversation(group) {
    if (group.conversation && group.key === group.conversation) return;
    if (group.creating) return group.creating;
    group.creating = (async () => {
      if (!group.conversation) {
        const { conversation } = await panels.api("/api/conversations", { bot_id: group.bot });
        group.conversation = conversation.id;
        buckets.set(conversation.id, group);
      }
      await panels.refresh();
      // Never pull someone back from a conversation/bot selected in the meantime.
      if (active(group) && !context.conversation && context.bot?.id === group.bot) {
        const draft = panels.draft();
        // If storage is unavailable, do not switch and erase a nonempty draft.
        if (draft) localStorage.setItem(`grokbot.draft.${group.conversation}`, draft);
        if (!panels.openConversation(group.conversation)) throw new Error("Ouvrez la conversation créée pour préparer l’image.");
        // The shell re-saved this handed-off draft under the new-chat key;
        // ownership has moved to the conversation, so home must start empty.
        try { localStorage.removeItem("grokbot.draft.new"); } catch { /* Already saved under the conversation key. */ }
      }
      if (buckets.get(group.key) === group) buckets.delete(group.key);
      group.key = group.conversation;
      buckets.set(group.key, group);
    })();
    try { await group.creating; }
    finally { group.creating = null; changed(); }
  }

  function mime(file) {
    if (types.has(file.type)) return file.type;
    if (file.type && !file.type.startsWith("image/") && file.type !== "application/octet-stream") return null;
    return extensions[file.name?.split(".").pop().toLowerCase()] || null;
  }
  function onFiles(files, event, ctx) {
    // Office/iWork copies include a PNG rendition of the intended text. Browser
    // "Copy Image" also carries its URL, and RTFD may carry an object mark.
    const text = event?.clipboardData?.getData("text/plain").replace(/\uFFFC/g, "").trim();
    if (text && !/^(?:[a-z][a-z0-9+.-]*:\/\/|data:|blob:)\S*$/i.test(text)) return false;
    const candidates = files.filter((file) => mime(file));
    if (!candidates.length) { panels.notify("Joignez une image PNG, JPEG, GIF, WebP ou BMP."); return false; }
    const group = bucket(ctx);
    for (const file of candidates) {
      if (!file.size || file.size > MAX_BYTES) { panels.notify("Les images doivent être non vides et ne pas dépasser 20 Mio."); continue; }
      if (group.items.length >= MAX_IMAGES) { panels.notify("Un message peut contenir dix images au maximum."); break; }
      const item = { file, preview: URL.createObjectURL(file), upload: "pending", caption: "waiting", revision: 0 };
      group.items.push(item);
      upload(group, item);
    }
    changed();
    return true;
  }

  async function upload(group, item) {
    item.upload = "pending";
    item.error = null;
    const controller = new AbortController();
    item.controller = controller;
    const timeout = setTimeout(() => controller.abort(), DEADLINE);
    changed();
    try {
      const response = await fetch("/api/attachments", {
        method: "POST", headers: { "Content-Type": mime(item.file), "X-Grokbot-Request": "local" },
        body: item.file, signal: controller.signal,
      });
      const attachment = await response.json();
      if (!response.ok) throw new Error(attachment.error || "L’import de l’image a échoué.");
      if (!/^attachment_[0-9a-f]{64}$/.test(attachment.id)) throw new Error("Réponse d’import illisible.");
      if (!present(group, item)) return;
      if (group.items.some((other) => other !== item && other.id === attachment.id)) {
        remove(group, item);
        panels.notify("Cette image est déjà jointe.");
        return;
      }
      item.id = attachment.id;
      // Keep submission blocked until the new conversation is selectable.
      await ensureConversation(group);
      if (!present(group, item)) return;
      item.upload = "ready";
      schedule(group, item);
    } catch (error) {
      if (present(group, item)) { item.upload = "failed"; item.error = error.message; }
    } finally {
      clearTimeout(timeout);
      if (item.controller === controller) item.controller = null;
      changed();
    }
  }

  function signature(ctx) {
    return JSON.stringify([ctx.draft, ctx.state.preferences?.model || null, Boolean(ctx.state.provider?.connected), Boolean(ctx.state.provider?.models_ready)]);
  }
  function schedule(group, item, retry = false) {
    clearTimeout(item.timer);
    item.revision++;
    item.controller?.abort();
    item.polling = false;
    item.retry = false;
    item.caption = "waiting";
    item.error = null;
    if (active(group)) {
      item.draft = panels.draft();
      item.signature = signature({ ...context, draft: item.draft });
      item.model = context.state.preferences?.model || null;
      const revision = item.revision;
      item.timer = setTimeout(() => {
        item.timer = null;
        if (present(group, item) && active(group) && revision === item.revision) {
          item.started = Date.now();
          prepare(group, item, retry);
        }
      }, 1000);
    }
    changed();
  }
  async function prepare(group, item, retry = false) {
    if (item.polling || !present(group, item)) return;
    if (Date.now() - item.started >= DEADLINE) {
      item.caption = "failed";
      item.error = "Délai de prédescription dépassé.";
      changed();
      return;
    }
    // Poll an already-started job to learn its real outcome; a new image that
    // never reached the service is paused, not refused/ignored.
    if (cooldown > Date.now() && item.caption !== "pending") { item.caption = "paused"; changed(); return; }
    const revision = item.revision;
    item.caption = "pending";
    item.polling = true;
    const controller = new AbortController();
    item.controller = controller;
    const timeout = setTimeout(() => controller.abort(), Math.max(1, DEADLINE - (Date.now() - item.started)));
    changed();
    try {
      // Poll exactly the original draft/model cache key. A changed draft gets
      // its own debounced request; stale completions must not update its chip.
      const response = await fetch("/api/attachments/precaption", {
        method: "POST", headers: { "Content-Type": "application/json", "X-Grokbot-Request": "local" },
        body: JSON.stringify({ conversation_id: group.conversation, attachment_id: item.id, draft: item.draft, ...(item.model ? { model: item.model } : {}), retry }),
        signal: controller.signal,
      });
      const result = await response.json();
      if (!present(group, item) || revision !== item.revision) return;
      // Capacity/catalog refusals precede any claim. Retain an explicit retry
      // until admitted, then poll normally. DEADLINE bounds the transient wait;
      // other conflicts must surface their actual error rather than time out.
      if (response.status === 429 || (response.status === 409 && !context.state.provider?.models_ready)) {
        item.retry = retry;
        return;
      }
      if (!response.ok) throw new Error(result.error || "La description n’est pas disponible.");
      item.retry = false;
      const status = result.caption?.status;
      if (!["pending", "ready", "skipped", "failed"].includes(status)) throw new Error("État de description illisible.");
      backoff(Math.max(Number(result.retry_after) || 0, status === "skipped" ? Number(result.caption.retry_after) || 0 : 0));
      item.caption = status;
      item.error = status === "failed" ? result.caption.reason : null;
    } catch (error) {
      if (present(group, item) && revision === item.revision) { item.caption = "failed"; item.error = error.message; }
    } finally {
      clearTimeout(timeout);
      if (revision === item.revision) { item.polling = false; item.controller = null; }
      changed();
    }
  }

  function sync(ctx = context) {
    context = ctx;
    const group = bucket();
    const current = signature({ ...context, draft: panels.draft() });
    for (const item of group.items) {
      if (item.upload !== "ready") continue;
      if (item.signature !== current) schedule(group, item);
    }
    render();
  }
  panels.register({
    id: "attachments",
    composer: {
      mount(container, ctx) {
        context = ctx;
        tray = node("div", "attachments-panel");
        picker = node("input", "attachment-picker");
        picker.type = "file";
        picker.multiple = true;
        picker.accept = [...types, ".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp"].join(",");
        picker.tabIndex = -1;
        picker.setAttribute("aria-label", "Choisir des images");
        const choose = node("button", "attachment-choose", "+ Joindre une image");
        choose.type = "button";
        choose.addEventListener("click", () => picker.click());
        picker.addEventListener("change", () => { onFiles(Array.from(picker.files), null, context); picker.value = ""; });
        // One controller-wide notice, read in document order and referenced by
        // disabled retries for screen-reader browsing. Not a live region: do not
        // interrupt readers with a countdown announcement every second.
        pause = node("p", "attachment-pause");
        pause.id = "attachment-pause";
        pause.hidden = true;
        list = node("ul", "attachment-list");
        list.setAttribute("aria-label", "Images à envoyer");
        tray.append(choose, picker, pause, list);
        container.append(tray);
        const form = document.getElementById("composer-form");
        form.addEventListener("input", () => sync());
        // Includes rich-editor/programmatic changes (suggestions, undo, drafts).
        new MutationObserver(() => sync()).observe(document.getElementById("composer-input"), { childList: true, characterData: true, subtree: true });
        form.addEventListener("submit", (event) => {
          if (document.getElementById("send-button").dataset.mode === "stop") return;
          if (bucket().items.some((item) => item.upload !== "ready")) {
            event.preventDefault();
            event.stopImmediatePropagation();
            panels.notify("Attendez l’import des images, ou retirez celles en échec, avant d’envoyer.");
          }
        }, true);
        window.addEventListener("beforeunload", () => { for (const group of new Set(buckets.values())) group.items.forEach(discard); });
      },
      onFiles,
      onSent({ messageId, extras }, ctx) {
        // onSent may be delivered by refresh while another conversation is open.
        const conversation = ctx.state.conversations.find((entry) => entry.messages.some((message) => message.id === messageId));
        const group = buckets.get(conversation?.id);
        if (!group) return;
        const sent = new Set(extras.attachments || []);
        group.items = group.items.filter((item) => {
          if (!sent.has(item.id)) return true;
          discard(item);
          return false;
        });
        changed();
      },
    },
    onState(state, ctx) {
      backoff(Number(state.attachments?.retry_after) || 0);
      sync(ctx);
      const group = bucket();
      for (const item of group.items) {
        if (item.caption === "pending" && !item.polling) prepare(group, item, Boolean(item.retry));
        else if (item.caption === "waiting" && !item.timer && item.upload === "ready") schedule(group, item);
      }
    },
    wantsPolling: () => Boolean(context && bucket().items.some((item) => item.caption === "pending")),
    sendExtras: (ctx) => ({ attachments: bucket(ctx).items.filter((item) => item.upload === "ready").map((item) => item.id) }),
  });
})();
