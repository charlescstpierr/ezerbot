/* Mémoire : vue Graphe. Vous au centre, un groupe par sujet (plus « Personnel » et « Travail »), un point par souvenir.
 * `buildGraph` est pure (données → positions) ; `view` la dessine en SVG. Seuls les liens vous → groupe → souvenir existent. */
(() => {
  "use strict";
  const SVG = "http://www.w3.org/2000/svg", CX = 300, CY = 220, RANK = { private: 0, team: 1, topic: 2 };
  const NAMES = { private: "Personnel", team: "Travail", topic: "Sujet" }, SCOPE_LABELS = { private: "Privé", team: "Équipe", topic: "Sujet" };
  const round = (value) => Math.round(value * 10) / 10;
  const scopeOf = (fact) => (fact.scope === "team" || fact.scope === "topic" ? fact.scope : "private");
  function buildGraph(facts, { user = "", bots = [] } = {}) {
    const first = String(user || "").trim().split(/\s+/)[0] || "Vous";
    const found = new Map();
    for (const fact of facts || []) {
      const scope = scopeOf(fact);
      const id = scope === "topic" ? `topic:${fact.topic_id ?? fact.topic_name ?? ""}` : scope;
      if (!found.has(id)) found.set(id, { id, scope, label: scope === "topic" ? fact.topic_name || NAMES.topic : NAMES[scope], facts: [] });
      found.get(id).facts.push(fact);
    }
    // Un groupe sans souvenir n'est pas dessiné : une pastille seule ne dirait rien.
    const list = [...found.values()].sort((a, b) => RANK[a.scope] - RANK[b.scope] || a.label.localeCompare(b.label, "fr"));
    const ring = Math.max(130, 24 * list.length);
    const graph = { center: { id: "user", x: CX, y: CY, label: first, initial: first.charAt(0).toLocaleUpperCase("fr") }, groups: [], nodes: [], edges: [] };
    list.forEach((group, i) => {
      const angle = -Math.PI / 2 + (2 * Math.PI * i) / list.length;
      const gx = CX + ring * Math.cos(angle), gy = CY + ring * Math.sin(angle);
      graph.groups.push({ id: group.id, scope: group.scope, label: group.label, count: group.facts.length, x: round(gx), y: round(gy) });
      graph.edges.push({ from: "user", to: group.id });
      group.facts.forEach((fact, j) => {
        // Les souvenirs s'ouvrent en éventail vers l'extérieur, sept par rang.
        const rank = Math.floor(j / 7), size = Math.min(7, group.facts.length - rank * 7), theta = angle + 0.42 * ((j % 7) - (size - 1) / 2), reach = 72 + 20 * rank;
        const bot = (bots || []).find((item) => item.id === fact.bot_id);
        graph.nodes.push({ id: String(fact.fact_id), group: group.id, scope: group.scope, bot: bot?.name || "Bot", label: String(fact.text ?? ""), x: round(gx + reach * Math.cos(theta)), y: round(gy + reach * Math.sin(theta)) });
        graph.edges.push({ from: group.id, to: String(fact.fact_id) });
      });
    });
    const xs = [graph.center, ...graph.groups, ...graph.nodes].map((item) => item.x), ys = [graph.center, ...graph.groups, ...graph.nodes].map((item) => item.y);
    const x = Math.min(...xs) - 70, y = Math.min(...ys) - 50;
    graph.bounds = { x, y, w: Math.max(...xs) + 70 - x, h: Math.max(...ys) + 70 - y };
    return graph;
  }
  const excerpt = (text, max = 22) => { // le texte d'un résultat, raccourci à un mot entier
    const chars = [...String(text).replace(/\s+/g, " ").trim()];
    if (chars.length <= max) return chars.join("");
    const cut = chars.slice(0, max).join(""), space = cut.lastIndexOf(" ");
    return `${(space > max / 2 ? cut.slice(0, space) : cut).replace(/[\s,;:.!?-]+$/, "")}…`;
  };
  let observer = null;
  const kept = { zoom: 1, ox: 0, oy: 0 }; // le cadrage survit au redessin, par exemple à chaque frappe de la recherche
  // `options.matches` : ids des souvenirs trouvés (Set) ; ils ressortent avec leur texte, le reste s'estompe. Absent : rien ne change.
  function view(facts, options) {
    const graph = buildGraph(facts, options), matches = options?.matches || null;
    // Le texte d'un résultat déborde de son point et garde sa taille à l'écran : la place qu'on lui laisse se compte en pixels (s = pixels par unité),
    // suit sa longueur et reste plafonnée à 35 % du cadre de chaque côté, pour que le calcul soit borné sur un cadre étroit.
    const room = (s, rect) => {
      let { x, y, w, h } = graph.bounds, x1 = x + w, y1 = y + h;
      const pad = (px, side) => Math.min(px, (rect?.[side] || Infinity) * 0.35) / s;
      for (const n of graph.nodes) if (matches?.has(n.id)) { const wide = pad(10 * [...excerpt(n.label)].length + 12, "width"); x = Math.min(x, n.x - wide); y = Math.min(y, n.y - pad(40, "height")); x1 = Math.max(x1, n.x + wide); y1 = Math.max(y1, n.y + pad(50, "height")); }
      return { x, y, w: x1 - x, h: y1 - y };
    };
    let box = room(1);
    const make = (tag, attrs = {}, text) => {
      const el = document.createElementNS(SVG, tag);
      for (const [key, value] of Object.entries(attrs)) el.setAttribute(key, value);
      if (text !== undefined) el.textContent = text;
      return el;
    };
    const html = (tag, className, text) => { const el = document.createElement(tag); el.className = className; if (text !== undefined) el.textContent = text; return el; };
    const svg = make("svg", { class: "memory-graph-canvas", role: "group", "aria-label": `Graphe de la mémoire : ${graph.nodes.length} souvenirs, ${graph.groups.length} groupes` });
    const links = new Map(), drawn = new Map(), lines = [];
    for (const edge of graph.edges) {
      const line = make("line", { class: "memory-graph-edge" });
      lines.push([edge, line]);
      for (const [a, b] of [[edge.from, edge.to], [edge.to, edge.from]]) links.set(a, [...(links.get(a) || []), b]);
    }
    const at = new Map([[graph.center.id, graph.center], ...graph.groups.map((item) => [item.id, item]), ...graph.nodes.map((item) => [item.id, item])]);
    for (const [edge, line] of lines) for (const [key, id] of [["1", edge.from], ["2", edge.to]]) { line.setAttribute(`x${key}`, at.get(id).x); line.setAttribute(`y${key}`, at.get(id).y); }
    // Au repos, seuls les éléments sans rapport avec la recherche sont estompés : le centre, les groupes et les liens qui mènent aux résultats restent.
    const base = new Set();
    if (matches) {
      const keep = new Set(["user", ...graph.nodes.filter((item) => matches.has(item.id)).flatMap((item) => [item.id, item.group])]);
      for (const key of [...graph.groups, ...graph.nodes].map((item) => item.id)) if (!keep.has(key)) base.add(key);
    }
    const highlight = (id) => {
      const lit = new Set(id ? [id, ...(links.get(id) || [])] : []);
      for (const [key, el] of drawn) { el.classList.toggle("is-active", key === id); el.classList.toggle("is-dim", id ? !lit.has(key) : base.has(key)); }
      for (const [edge, line] of lines) line.classList.toggle("is-dim", id ? edge.from !== id && edge.to !== id : base.has(edge.from) || base.has(edge.to));
    };
    const add = (item, kind, scope, radius, name, caption) => {
      const el = make("g", { class: `memory-graph-node memory-graph-${kind} is-${scope}`, tabindex: "0", role: "img", "aria-label": name, "data-id": item.id });
      el.append(make("circle", { cx: item.x, cy: item.y, r: radius }));
      if (kind === "point") el.append(make("title", {}, name));
      if (caption) el.append(make("text", { class: "memory-graph-label", x: item.x, y: item.y + radius, dy: "1.4em" }, caption));
      for (const on of ["mouseenter", "focus"]) el.addEventListener(on, () => highlight(item.id));
      for (const off of ["mouseleave", "blur"]) el.addEventListener(off, () => highlight(null));
      drawn.set(item.id, el);
      return el;
    };
    const center = add(graph.center, "center", "self", 30, graph.center.label, graph.center.label);
    center.append(make("text", { class: "memory-graph-initial", x: CX, y: CY }, graph.center.initial));
    const short = (text) => (text.length > 18 ? `${text.slice(0, 17)}…` : text);
    const edgeLayer = make("g"), nodeLayer = make("g");
    edgeLayer.append(...lines.map(([, line]) => line));
    nodeLayer.append(center,
      ...graph.groups.map((group) => add(group, "group", group.scope, 20, `${group.label}, ${group.count} souvenir${group.count > 1 ? "s" : ""}`, short(group.label))),
      ...graph.nodes.map((node) => {
        const hit = Boolean(matches?.has(node.id)), el = add(node, "point", node.scope, 6, `${node.bot}, ${SCOPE_LABELS[node.scope]} : ${node.label.slice(0, 120)}`);
        el.classList.toggle("is-match", hit);
        if (hit) { // le texte se pose vers l'extérieur de son groupe, loin de l'étiquette du groupe
          const g = at.get(node.group), d = Math.hypot(node.x - g.x, node.y - g.y) || 1, ux = (node.x - g.x) / d, uy = (node.y - g.y) / d, side = Math.abs(ux) > 0.5 ? Math.sign(ux) : 0;
          el.append(make("text", { class: "memory-graph-label", x: node.x + side * 10, y: node.y, dy: Math.abs(uy) > 0.5 ? (uy > 0 ? "1.6em" : "-0.9em") : "0.35em", style: `text-anchor:${side > 0 ? "start" : side < 0 ? "end" : "middle"}` }, excerpt(node.label)));
        }
        return el;
      }));
    svg.append(edgeLayer, nodeLayer);
    highlight(null);
    // Zoom : le viewBox se resserre autour du centre de la vue ; le texte garde sa taille à l'écran (--k = unités par pixel).
    let { zoom, ox, oy } = kept, scale = 1, drag = null;
    const apply = () => {
      const rect = svg.getBoundingClientRect?.();
      if (matches && rect?.width) { box = room(1, rect); for (let i = 0; i < 6; i++) box = room(Math.min(rect.width / box.w, rect.height / box.h), rect); } // à neuf à chaque fois, en quelques passes : la place dépend de l'échelle
      const w = box.w / zoom, h = box.h / zoom;
      scale = rect?.width ? Math.min(rect.width / w, rect.height / h) : 1;
      svg.setAttribute("viewBox", [box.x + box.w / 2 + ox - w / 2, box.y + box.h / 2 + oy - h / 2, w, h].map(round).join(" "));
      svg.setAttribute("style", `--k:${Number((1 / scale).toFixed(3))}`);
      Object.assign(kept, { zoom, ox, oy });
    };
    const step = (factor) => () => { zoom = Math.min(4, Math.max(0.5, zoom * factor)); apply(); };
    svg.addEventListener("pointerdown", (event) => { drag = event; svg.setPointerCapture?.(event.pointerId); });
    svg.addEventListener("pointermove", (event) => { if (!drag) return; ox -= (event.clientX - drag.clientX) / scale; oy -= (event.clientY - drag.clientY) / scale; drag = event; apply(); });
    for (const end of ["pointerup", "pointercancel"]) svg.addEventListener(end, () => { drag = null; });
    const tools = html("div", "memory-graph-tools");
    for (const [label, text, action] of [["Zoom avant", "+", step(1.25)], ["Zoom arrière", "−", step(0.8)], ["Recadrer", "Recadrer", () => { zoom = 1; ox = oy = 0; apply(); }]]) {
      const button = html("button", "memory-graph-tool", text);
      button.type = "button";
      button.setAttribute("aria-label", label);
      button.addEventListener("click", action);
      tools.append(button);
    }
    const legend = html("ul", "memory-graph-legend");
    legend.setAttribute("aria-label", "Légende");
    for (const scope of ["private", "team", "topic"]) {
      const item = html("li", "memory-graph-key");
      item.append(html("span", `memory-graph-swatch is-${scope}`), html("span", "memory-graph-key-name", SCOPE_LABELS[scope]));
      legend.append(item);
    }
    const root = html("div", "memory-graph");
    root.append(svg, tools, legend);
    apply();
    window.requestAnimationFrame?.(apply);
    // Un seul observateur à la fois : celui du graphe remplacé est déconnecté, il ne retient pas un SVG détaché.
    observer?.disconnect();
    observer = window.ResizeObserver ? new window.ResizeObserver(apply) : null;
    observer?.observe(svg);
    return root;
  }
  window.GrokbotMemoryGraph = { buildGraph, view };
})();
