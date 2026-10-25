// Run with: node --test app/ui/panels/memory-links.test.cjs
// La règle des liens : deux souvenirs sont liés s'ils viennent de la même conversation. Rien n'est stocké.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const vm = require("node:vm");
const window = {};
vm.runInContext(fs.readFileSync(`${__dirname}/memory-links.js`, "utf8"), vm.createContext({ window, Map, Set, String, Math, Array }));
const { neighbors, pairs, radius } = window.EzerbotMemoryLinks;
const fact = (id, conversation, scope = "private") => ({ fact_id: id, scope, text: `souvenir ${id}`, provenance: { source: "automatic", conversation_id: conversation } });
const plain = (value) => JSON.parse(JSON.stringify(value)); // sort du contexte vm

test("deux souvenirs de la même conversation donnent un lien, vu des deux côtés", () => {
  const facts = [fact("a", "c1"), fact("b", "c1")];
  assert.deepEqual(plain(pairs(facts)), [["a", "b"]]);
  assert.deepEqual([...neighbors(facts)].map(([id, others]) => [id, plain(others)]), [["a", ["b"]], ["b", ["a"]]]);
});
test("un souvenir isolé n'a aucun lien", () => {
  const facts = [fact("a", "c1"), fact("b", "c2"), fact("c", null)];
  assert.deepEqual(plain(pairs(facts)), []);
  assert.equal(neighbors(facts).size, 0);
  assert.deepEqual(plain(pairs([fact("solo", "c1")])), []);
});
test("sans conversation (absente, vide ou blanche), des souvenirs ne sont pas liés entre eux", () => {
  const facts = [fact("a", null), fact("b", null), fact("c", ""), fact("d", "  "), { fact_id: "e", text: "sans provenance" }, { fact_id: "f", provenance: null }];
  assert.deepEqual(plain(pairs(facts)), []);
});
test("la portée ne compte pas : un souvenir privé, d'équipe et de sujet d'une même conversation sont liés deux à deux", () => {
  const facts = [fact("a", "c1", "private"), fact("b", "c1", "team"), fact("c", "c1", "topic"), fact("d", "c2")];
  assert.deepEqual(plain(pairs(facts)), [["a", "b"], ["a", "c"], ["b", "c"]]);
  assert.deepEqual(plain(neighbors(facts).get("b")), ["a", "c"]);
  assert.equal(neighbors(facts).has("d"), false);
});
test("deux conversations donnent deux groupes séparés ; l'ordre de la liste est gardé ; un identifiant numérique se lit comme du texte", () => {
  const facts = [fact(1, "c1"), fact("x", "c2"), fact("2", "c1"), fact("y", "c2")];
  assert.deepEqual(plain(pairs(facts)), [["1", "2"], ["x", "y"]]);
  assert.deepEqual(plain(neighbors(facts).get("1")), ["2"]);
});
test("la taille d'un point grandit un peu avec ses liens, sans jamais dépasser un plafond", () => {
  assert.equal(radius(0), 6, "un souvenir isolé garde la taille d'avant");
  assert.ok(radius(1) > radius(0) && radius(3) > radius(1));
  assert.equal(radius(6), radius(60), "plafond");
  assert.ok(radius(60) <= 9, "un peu plus gros, pas un autre objet");
  assert.equal(radius(undefined), 6);
});
test("absence de liste : aucun lien, pas d'erreur", () => {
  assert.deepEqual(plain(pairs(undefined)), []);
  assert.equal(neighbors(null).size, 0);
});
