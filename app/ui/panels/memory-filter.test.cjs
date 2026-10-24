const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const vm = require("node:vm");
const window = {};
vm.runInContext(fs.readFileSync(`${__dirname}/memory-filter.js`, "utf8"), vm.createContext({ window, String, Set, Map, Object, Array, JSON }));
const out = (value) => JSON.parse(JSON.stringify(value)); // sort du contexte vm
const filtre = (...args) => out(window.EzerbotMemoryFilter.filtre(...args)), botChoices = (...args) => out(window.EzerbotMemoryFilter.botChoices(...args));
const fact = (id, bot, scope, text) => ({ fact_id: id, bot_id: bot, scope, text });
const FACTS = [fact(1, "a", "private", "Voyage à Lisbonne"), fact(2, "b", "team", "Ma nièce est élève"), fact(3, "a", "topic", "ÉLÈVE de Lisbonne"), fact(4, "a", "autre", "Sans portée connue")];
test("filtre : accents et casse ignorés, ids rendus dans l’ordre des souvenirs", () => {
  assert.deepEqual(filtre(FACTS, { q: "lisbonne" }), ["1", "3"]);
  assert.deepEqual(filtre(FACTS, { q: "ELEVE" }), ["2", "3"]);
  assert.deepEqual(filtre(FACTS, { q: "  élève " }), ["2", "3"]);
});
test("filtre : bot, portée et recherche se combinent ; une portée inconnue compte comme privée", () => {
  assert.deepEqual(filtre(FACTS, { bot: "a" }), ["1", "3", "4"]);
  assert.deepEqual(filtre(FACTS, { bot: "a", scope: "private" }), ["1", "4"]);
  assert.deepEqual(filtre(FACTS, { bot: "a", scope: "topic", q: "lisbonne" }), ["3"]);
  assert.deepEqual(filtre(FACTS, { bot: "b", scope: "topic" }), []);
});
test("menu des bots : ceux de l’état (sauf supprimés) plus ceux que les souvenirs citent, triés", () => {
  const label = (item) => `libellé ${item.bot_id}`;
  const choices = botChoices(FACTS, [{ id: "z", name: "Zoé" }, { id: "a", name: "Ada" }, { id: "gone", name: "Parti", deleted: true }], label);
  assert.deepEqual(choices, [{ id: "a", label: "libellé a" }, { id: "b", label: "libellé b" }, { id: "z", label: "Zoé" }]);
});
