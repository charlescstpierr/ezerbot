// Run with: node --test app/ui/panels/memory-graph.test.cjs
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const vm = require("node:vm");
const window = {};
vm.runInContext(fs.readFileSync(`${__dirname}/memory-graph.js`, "utf8"), vm.createContext({ window, Math, Set, Map, Object, Array, String, Number }));
const build = (...args) => JSON.parse(JSON.stringify(window.GrokbotMemoryGraph.buildGraph(...args))); // sort du contexte vm
const buildGraph = build;
const fact = (id, scope, topic) => ({ fact_id: id, bot_id: "b", scope, text: `souvenir ${id}`, topic_id: topic ? `t-${topic}` : null, topic_name: topic || null });
const FACTS = [fact("a", "private"), fact("b", "private"), fact("c", "team"), fact("d", "topic", "Voyage"), fact("e", "topic", "Voyage"), fact("f", "topic", "Maison")];
test("N souvenirs donnent N points, un centre et un groupe par sujet plus Personnel et Travail", () => {
  const graph = buildGraph(FACTS, { user: "Charles St-Pierre" });
  assert.equal(graph.nodes.length, 6);
  assert.deepEqual(graph.groups.map((group) => [group.label, group.count]), [["Personnel", 2], ["Travail", 1], ["Maison", 1], ["Voyage", 2]]);
  assert.deepEqual([graph.center.label, graph.center.initial], ["Charles", "C"]);
  assert.equal(graph.edges.length, 4 + 6, "vous → groupe, groupe → souvenir, rien d’autre");
  assert.deepEqual(graph.nodes.map((node) => node.scope), ["private", "private", "team", "topic", "topic", "topic"]);
});
test("un groupe sans souvenir n’est pas dessiné ; sans prénom connu, « Vous »", () => {
  const graph = buildGraph([fact("a", "private")]);
  assert.deepEqual(graph.groups.map((group) => group.label), ["Personnel"]);
  assert.deepEqual([graph.center.label, graph.center.initial], ["Vous", "V"]);
  assert.deepEqual([buildGraph([]).groups, buildGraph([]).nodes], [[], []]);
});
test("les positions sont déterministes, finies et sans deux points superposés", () => {
  const many = Array.from({ length: 40 }, (_, i) => fact(`m${i}`, i % 2 ? "team" : "topic", i % 3 ? "Voyage" : "Maison"));
  const graph = buildGraph(many);
  assert.deepEqual(buildGraph(many), graph);
  const spots = graph.nodes.map((node) => `${node.x},${node.y}`);
  assert.equal(new Set(spots).size, spots.length);
  for (const item of [graph.center, ...graph.groups, ...graph.nodes]) assert.ok(Number.isFinite(item.x) && Number.isFinite(item.y));
});
test("une portée inconnue est traitée comme privée", () => {
  assert.equal(buildGraph([{ fact_id: "x", scope: "autre", text: "t" }]).nodes[0].scope, "private");
});
test("jetons : les trois portées existent en clair et en sombre, aucune couleur codée en dur, aucun texte sous 11,5 px", () => {
  const styles = fs.readFileSync(`${__dirname}/../styles.css`, "utf8");
  const [light, dark] = [styles.slice(0, styles.indexOf('[data-theme="dark"]')), styles.slice(styles.indexOf('[data-theme="dark"]'), styles.indexOf('[data-accent="orange"]'))];
  for (const scope of ["private", "team", "topic"]) for (const block of [light, dark]) assert.match(block, new RegExp(`--scope-${scope}: #[0-9a-f]{6};`));
  const css = fs.readFileSync(`${__dirname}/memory-graph.css`, "utf8");
  for (const scope of ["private", "team", "topic"]) assert.match(css, new RegExp(`\\.memory-graph-node\\.is-${scope}[^{]*\\{ --tone: var\\(--scope-${scope}\\); \\}`));
  assert.doesNotMatch(css, /#[0-9a-fA-F]{3,8}\b|rgb\(|hsl\(/);
  for (const [, size] of css.matchAll(/font-size:\s*(?:calc\()?(\d+(?:\.\d+)?)px/g)) assert.ok(Number(size) >= 11.5, `${size}px`);
});
