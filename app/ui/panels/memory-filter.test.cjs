// node --test app/ui/panels/memory-filter.test.cjs
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const vm = require("node:vm");
const window = {};
vm.runInContext(fs.readFileSync(`${__dirname}/memory-filter.js`, "utf8"), vm.createContext({ window, String, Set, Map, Object, Array, JSON }));
const out = (value) => JSON.parse(JSON.stringify(value)); // sort du contexte vm
const filtre = (...args) => out(window.EzerbotMemoryFilter.filtre(...args)), botChoices = (...args) => out(window.EzerbotMemoryFilter.botChoices(...args)), { normalize } = window.EzerbotMemoryFilter;
const fact = (id, bot, scope, text) => ({ fact_id: id, bot_id: bot, scope, text });
const FACTS = [fact(1, "a", "private", "Voyage à Lisbonne"), fact(2, "b", "team", "Ma nièce est élève"), fact(3, "a", "topic", "ÉLÈVE de Lisbonne"), fact(4, "a", "autre", "Sans portée connue")];
test("filtre : accents et casse ignorés, ids rendus dans l’ordre des souvenirs", () => {
  assert.deepEqual(filtre(FACTS, { q: "lisbonne" }), ["1", "3"]);
  assert.deepEqual(filtre(FACTS, { q: "ELEVE" }), ["2", "3"]);
  assert.deepEqual(filtre(FACTS, { q: "  élève " }), ["2", "3"]);
  assert.deepEqual(filtre(FACTS, {}), ["1", "2", "3", "4"], "rien de filtré : tout correspond");
  assert.equal(normalize("Éléphant À Noël"), "elephant a noel");
});
test("filtre : bot, portée et recherche se combinent ; une portée inconnue compte comme privée", () => {
  assert.deepEqual(filtre(FACTS, { bot: "a" }), ["1", "3", "4"]);
  assert.deepEqual(filtre(FACTS, { bot: "a", scope: "private" }), ["1", "4"]);
  assert.deepEqual(filtre(FACTS, { bot: "a", scope: "topic", q: "lisbonne" }), ["3"]);
  assert.deepEqual(filtre(FACTS, { bot: "b", scope: "topic" }), []);
  assert.deepEqual(filtre(null, { q: "x" }), []);
});
test("menu des bots : ceux de l’état (sauf supprimés) plus ceux que les souvenirs citent, triés", () => {
  const label = (item) => `libellé ${item.bot_id}`;
  const choices = botChoices(FACTS, [{ id: "z", name: "Zoé" }, { id: "a", name: "Ada" }, { id: "gone", name: "Parti", deleted: true }], label);
  assert.deepEqual(choices, [{ id: "a", label: "libellé a" }, { id: "b", label: "libellé b" }, { id: "z", label: "Zoé" }]);
});
test("jetons : aucune couleur codée en dur, aucun texte sous 11,5 px", () => {
  const css = fs.readFileSync(`${__dirname}/memory-filter.css`, "utf8");
  assert.doesNotMatch(css, /#[0-9a-fA-F]{3,8}\b|rgb\(|hsl\(/);
  for (const [, size] of css.matchAll(/font-size:\s*(\d+(?:\.\d+)?)px/g)) assert.ok(Number(size) >= 11.5, `${size}px`);
});
