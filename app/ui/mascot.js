"use strict";

(() => {
  const template = document.getElementById("mascot-template");
  const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)");
  const companions = [];
  const labels = {
    loading: "Ouverture de votre espace…", idle: "À votre rythme.",
    attentive: "Prêt pour votre message.", working: "Je garde le fil…",
    uncertain: "On vérifie ensemble.", offline: "Je vous attends ici.",
  };
  let sequence = 0;
  let context = { connection: "loading", activity: null, pending: null, drafting: false };
  let mood = "loading";
  let celebration;
  let celebrationTimer;
  let frame;
  let current = { x: 0, y: 0 };
  let target = { x: 0, y: 0 };
  let destroyed = false;

  function create(reactive = false) {
    const node = template.content.firstElementChild.cloneNode(true);
    const suffix = ++sequence;
    for (const definition of node.querySelectorAll("[id]")) {
      const original = definition.id;
      definition.id = `${original}-${suffix}`;
      for (const painted of node.querySelectorAll(`[fill="url(#${original})"]`)) {
        painted.setAttribute("fill", `url(#${definition.id})`);
      }
    }
    node.dataset.reactive = String(reactive);
    node.dataset.mood = reactive ? mood : "idle";
    if (reactive) companions.push(node);
    return node;
  }

  for (const mount of document.querySelectorAll("[data-mascot]")) {
    mount.append(create(mount.dataset.mascot !== "brand"));
  }

  function baseMood() {
    if (context.connection === "offline") return "offline";
    if (context.connection === "loading") return "loading";
    if (context.activity) return "working";
    if (context.pending) return "uncertain";
    return context.drafting ? "attentive" : "idle";
  }

  function applyMood() {
    if (destroyed) return;
    const base = baseMood();
    if (!["idle", "attentive"].includes(base) && celebration) {
      clearTimeout(celebrationTimer);
      celebration = null;
    }
    const next = celebration ? "happy" : base;
    if (next !== mood) {
      mood = next;
      for (const companion of companions) companion.dataset.mood = mood;
      document.documentElement.dataset.companionMood = mood;
      aim(mood === "attentive" ? 3 : mood === "uncertain" ? 9 : 0, mood === "attentive" ? 8 : mood === "offline" ? 6 : 0);
    }
    const status = document.getElementById("companion-status");
    if (status) status.textContent = celebration || (context.pending === "queued" ? "Votre message attend son envoi." : labels[base]);
  }

  function paintLook() {
    for (const companion of companions) {
      companion.querySelector(".mascot-gaze").setAttribute("transform", `translate(${current.x.toFixed(2)} ${current.y.toFixed(2)})`);
    }
  }

  function moveLook() {
    frame = null;
    if (destroyed || document.hidden || reducedMotion.matches) return;
    current.x += (target.x - current.x) * .2;
    current.y += (target.y - current.y) * .2;
    paintLook();
    if (Math.abs(target.x - current.x) + Math.abs(target.y - current.y) > .15) frame = requestAnimationFrame(moveLook);
  }

  function aim(x, y) {
    target = reducedMotion.matches ? { x: 0, y: 0 } : { x, y };
    if (document.hidden || reducedMotion.matches) {
      cancelAnimationFrame(frame);
      frame = null;
      current = { ...target };
      paintLook();
    } else if (!frame && !destroyed) frame = requestAnimationFrame(moveLook);
  }

  function followPointer(event) {
    if (event.pointerType === "touch" || document.hidden || reducedMotion.matches || !["idle", "happy"].includes(mood)) return;
    const hero = document.querySelector("[data-mascot=hero]");
    const box = hero.getBoundingClientRect();
    const anchor = box.width ? box : document.querySelector("[data-mascot=sidebar]").getBoundingClientRect();
    if (!anchor.width) return;
    const x = (event.clientX - anchor.left - anchor.width / 2) / 28;
    const y = (event.clientY - anchor.top - anchor.height / 2) / 35;
    aim(Math.max(-11, Math.min(11, x)), Math.max(-7, Math.min(9, y)));
  }

  function resetLook() { aim(0, 0); }

  function setContext(next) {
    context = next;
    applyMood();
  }

  function cheer(label) {
    if (!["idle", "attentive"].includes(baseMood()) || destroyed) return;
    clearTimeout(celebrationTimer);
    celebration = label;
    applyMood();
    celebrationTimer = setTimeout(() => { celebration = null; applyMood(); }, 1400);
  }

  function wave() { cheer("Un petit coucou."); }
  function celebrate() { cheer("C’est bien enregistré."); }

  function syncMotion() {
    document.documentElement.dataset.mascotPaused = String(document.hidden || reducedMotion.matches);
    if (document.hidden || reducedMotion.matches) {
      cancelAnimationFrame(frame);
      frame = null;
      if (reducedMotion.matches) { current = { x: 0, y: 0 }; paintLook(); }
    } else aim(target.x, target.y);
  }

  function onPageHide(event) { if (event.persisted) syncMotion(); else destroy(); }

  function destroy() {
    if (destroyed) return;
    destroyed = true;
    clearTimeout(celebrationTimer);
    cancelAnimationFrame(frame);
    document.removeEventListener("pointermove", followPointer);
    document.documentElement.removeEventListener("pointerleave", resetLook);
    document.removeEventListener("visibilitychange", syncMotion);
    reducedMotion.removeEventListener("change", syncMotion);
    document.querySelector("[data-mascot=hero]").removeEventListener("click", wave);
    window.removeEventListener("pagehide", onPageHide);
    window.removeEventListener("pageshow", syncMotion);
    document.documentElement.dataset.mascotPaused = "true";
    companions.length = 0;
  }

  document.addEventListener("pointermove", followPointer, { passive: true });
  document.documentElement.addEventListener("pointerleave", resetLook);
  document.addEventListener("visibilitychange", syncMotion);
  reducedMotion.addEventListener("change", syncMotion);
  document.querySelector("[data-mascot=hero]").addEventListener("click", wave);
  window.addEventListener("pagehide", onPageHide);
  window.addEventListener("pageshow", syncMotion);
  window.GrokbotMascot = { create, setContext, celebrate, destroy };
  syncMotion();
  applyMood();
})();
