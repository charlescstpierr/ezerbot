"use strict";

(() => {
  const template = document.getElementById("mascot-template");
  const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)");
  const companions = [];
  // Chaque silhouette partage le viewBox 320×280 et entoure les yeux, qui ne bougent pas.
  const shapes = {
    galet: { label: "Galet",
      d: "M68 217C37 194 39 144 55 103C72 61 110 37 151 37C198 33 249 60 270 104C290 145 291 193 267 219C241 248 195 251 153 249C117 248 88 234 68 217Z" },
    goutte: { label: "Goutte",
      d: "M166.2 28.3C181.2 22.4 193.9 32.2 203.9 47.8C233.9 92.4 283.8 139.8 276.3 193.3C269.6 240.8 224.4 260.7 161.1 251.8C95.7 242.6 51.8 210.2 58.7 160.7C67.1 101.3 130.1 55.6 148.7 38C153.3 33.6 159.8 30.5 166.2 28.3Z" },
    nuage: { label: "Nuage",
      d: "M254 252L92 252A56 56 0 0 1 78 141.8A16 16 0 0 0 90 126.7A52 52 0 0 1 143.7 76A16 16 0 0 0 157.7 68.6A62 62 0 0 1 263.9 132.6A16 16 0 0 0 272.1 155.4A50 50 0 0 1 254 252Z" },
    carre: { label: "Carré",
      d: "M130.5 38.6L196.4 34C244.6 30.6 276.3 58.1 279.6 106.3L283.8 166.2C287.2 214.4 259.7 246.1 211.5 249.4L145.6 254C97.4 257.4 65.7 229.9 62.4 181.7L58.2 121.8C54.8 73.6 82.3 41.9 130.5 38.6Z" },
    fantome: { label: "Fantôme",
      d: "M65.8 219.6C63.5 175.7 62.5 155.7 61.9 145.7C58.6 81.8 104.2 35.3 165.1 32.2C226.1 29 276.3 70.4 279.6 134.3C280.2 144.3 281.2 164.3 283.5 208.2C284.9 234.2 271.7 250.9 258.5 251.6C244.9 252.3 243.8 232.3 230.2 233C216.6 233.7 217.7 253.7 204.1 254.4C190.5 255.1 189.4 235.2 175.8 235.9C162.2 236.6 163.3 256.6 149.6 257.3C136 258 135 238 121.4 238.7C107.8 239.4 108.8 259.4 95.2 260.1C82 260.8 67.2 245.6 65.8 219.6Z" },
    etoile: { label: "Étoile",
      d: "M186.8 37.8L203.1 58.4C212.3 70.1 227 78.6 241.7 80.7L267.8 84.5C293.6 88.3 300.5 103.9 286.1 125.6L271.5 147.5C263.2 159.9 259.7 176.5 262.2 191.1L266.6 217.1C271 242.8 258.3 254.2 233.3 247.1L207.9 240C193.6 236 176.7 237.8 163.5 244.7L140.2 256.9C117.1 269 102.4 260.5 101.3 234.5L100.2 208.1C99.6 193.3 92.7 177.8 82.1 167.4L63.2 149C44.6 130.8 48.2 114.1 72.6 105.1L97.3 95.9C111.2 90.7 123.8 79.4 130.4 66.1L142.1 42.4C153.7 19.1 170.6 17.3 186.8 37.8Z" },
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

  function setShape(node, shape) {
    const { d } = shapes[shape] || shapes.galet;
    node.querySelector(".mascot-shell").setAttribute("d", d);
    node.querySelector(".mascot-sheen").setAttribute("d", d);
    node.dataset.shape = shapes[shape] ? shape : "galet";
  }

  function create(reactive = false, shape = "galet") {
    const node = template.content.firstElementChild.cloneNode(true);
    setShape(node, shape);
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
    const anchor = box.width ? box : document.querySelector("[data-mascot=composer]").getBoundingClientRect();
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
  window.GrokbotMascot = { create, setShape, shapes, setContext, celebrate, destroy };
  syncMotion();
  applyMood();
})();
