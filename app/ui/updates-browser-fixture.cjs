function installUpdatesService() {
  const originalFetch = window.fetch.bind(window);
  const idle = { version: "0.1.0", installed_short: "aaaaaaa", installed_commit: "a".repeat(40), available: false, running: false, phase: "a_jour", last_attempt: "aucune", last_error: "", message: "Ezerbot est à jour." };
  const service = window.updatesFixture = {
    state: { ...idle }, calls: [], failApply: false,
    async set(phase, extra = {}) {
      this.state = { ...idle, available: phase !== "a_jour", running: ["telechargement", "compilation", "installation", "redemarrage", "attente"].includes(phase), phase, ...extra };
      await GrokbotPanels.refresh();
    },
  };
  window.fetch = async (input, options) => {
    const path = new URL(typeof input === "string" ? input : input.url, location.href).pathname;
    if (path.startsWith("/api/updates/")) {
      service.calls.push({ path, body: options?.body ? JSON.parse(options.body) : null });
      if (path === "/api/updates/apply") {
        if (service.failApply) return new Response(JSON.stringify({ error: "Service de mise à jour indisponible." }), { status: 503, headers: { "Content-Type": "application/json" } });
        service.state = { ...idle, available: true, running: true, phase: "telechargement", last_attempt: "en_cours" };
      }
      return new Response(JSON.stringify(service.state), { headers: { "Content-Type": "application/json" } });
    }
    const response = await originalFetch(input, options);
    if (path !== "/api/state" || !response.ok) return response;
    const state = await response.json();
    return new Response(JSON.stringify({ ...state, updates: service.state }), { status: response.status, headers: response.headers });
  };
  return GrokbotPanels.refresh();
}

function updateGeometry() {
  const rect = el => el.getBoundingClientRect().toJSON();
  const inside = (a, b) => a.left >= b.left - 1 && a.right <= b.right + 1 && a.top >= b.top - 1 && a.bottom <= b.bottom + 1;
  const sidebar = document.getElementById("sidebar");
  const rail = document.querySelector(".sidebar-rail");
  const content = document.querySelector(".sidebar-content");
  const corner = document.querySelector(".update-corner");
  const controls = [...document.querySelectorAll(".rail-bottom button, .update-corner button")].filter(el => el.getClientRects().length);
  const text = [...corner.querySelectorAll(".update-corner-label, .update-corner-error-text")].filter(el => el.getClientRects().length);
  return {
    rail: rect(rail), buttons: [...document.querySelectorAll(".rail-bottom [data-sidebar-destination]")].map(rect),
    corner: rect(corner), content: rect(content),
    contained: controls.every(el => inside(rect(el), rect(sidebar))) && (corner.hidden || inside(rect(corner), rect(sidebar))),
    hitTargets: controls.every(el => el.contains(document.elementFromPoint(rect(el).x + rect(el).width / 2, rect(el).y + rect(el).height / 2))),
    legible: text.every(el => {
      const range = document.createRange(); range.selectNodeContents(el);
      return parseFloat(getComputedStyle(el).fontSize) >= 11.5 && [...range.getClientRects()].every(r => inside(r, rect(sidebar)));
    }),
    separate: corner.hidden || (rect(corner).left >= rect(rail).right && rect(corner).top >= rect(content).bottom - 1),
    label: corner.innerText, hidden: corner.hidden,
  };
}

module.exports = { installUpdatesService, updateGeometry };
