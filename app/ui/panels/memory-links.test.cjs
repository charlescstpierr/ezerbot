// Run with: node --test app/ui/panels/memory-links.test.cjs
// La règle des liens : deux souvenirs sont liés s'ils viennent de la même conversation. Rien n'est stocké.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const vm = require("node:vm");
const window = {};
vm.runInContext(fs.readFileSync(`${__dirname}/memory-links.js`, "utf8"), vm.createContext({ window, Map, Set, String, Math, Array }));
const { counts, pairs, of, radius } = window.EzerbotMemoryLinks;
const fact = (id, conversation, scope = "private") => ({ fact_id: id, scope, text: `souvenir ${id}`, provenance: { source: "automatic", conversation_id: conversation } });
const plain = (value) => JSON.parse(JSON.stringify(value)); // sort du contexte vm
const ids = (list) => plain(list).map((item) => item.fact_id);

test("deux souvenirs de la même conversation sont liés : un trait, et chacun est lié à l'autre", () => {
  const facts = [fact("a", "c1"), fact("b", "c1")];
  assert.deepEqual(plain(pairs(facts)), [["a", "b"]]);
  assert.deepEqual(ids(of(facts, facts[0])), ["b"]);
  assert.deepEqual(ids(of(facts, facts[1])), ["a"]);
  assert.deepEqual([...counts(facts)], [["a", 1], ["b", 1]]);
});
test("un souvenir isolé n'a aucun lien", () => {
  const facts = [fact("a", "c1"), fact("b", "c2"), fact("c", null)];
  assert.deepEqual(plain(pairs(facts)), []);
  assert.equal(counts(facts).size, 0);
  assert.deepEqual(ids(of(facts, facts[0])), []);
  assert.deepEqual(plain(pairs([fact("solo", "c1")])), []);
});
test("sans conversation (absente, vide ou blanche), des souvenirs ne sont pas liés entre eux", () => {
  const facts = [fact("a", null), fact("b", null), fact("c", ""), fact("d", "  "), { fact_id: "e", text: "sans provenance" }, { fact_id: "f", provenance: null }];
  assert.deepEqual(plain(pairs(facts)), []);
  assert.equal(counts(facts).size, 0);
  for (const item of facts) assert.deepEqual(ids(of(facts, item)), []);
});
test("la portée ne compte pas : un souvenir privé, d'équipe et de sujet d'une même conversation sont tous liés entre eux", () => {
  const facts = [fact("a", "c1", "private"), fact("b", "c1", "team"), fact("c", "c1", "topic"), fact("d", "c2")];
  assert.deepEqual(ids(of(facts, facts[1])), ["a", "c"], "chacun voit les deux autres, pas lui-même");
  assert.deepEqual(ids(of(facts, facts[0])), ["b", "c"]);
  assert.deepEqual(ids(of(facts, facts[3])), []);
  assert.deepEqual([...counts(facts)], [["a", 2], ["b", 2], ["c", 2]], "trois liés ensemble : deux liens chacun ; d est isolé");
});
test("les traits d'une conversation la relient tout entière, sans tracer chaque paire", () => {
  const facts = [fact("a", "c1"), fact("x", "c2"), fact("b", "c1"), fact("y", "c2"), fact("c", "c1")];
  assert.deepEqual(plain(pairs(facts)), [["a", "b"], ["b", "c"], ["x", "y"]], "n − 1 traits par conversation, dans l'ordre de la liste");
});
test("deux conversations restent séparées ; un identifiant numérique se lit comme du texte", () => {
  const facts = [fact(1, "c1"), fact("x", "c2"), fact("2", "c1"), fact("y", "c2")];
  assert.deepEqual(ids(of(facts, facts[0])), ["2"]);
  assert.deepEqual(ids(of(facts, facts[1])), ["y"]);
  assert.equal(counts(facts).get("1"), 1);
});
test("une conversation de 4 000 souvenirs reste bornée : n − 1 traits, et lire un souvenir isolé ne coûte pas le carré", () => {
  const many = Array.from({ length: 4000 }, (_, i) => fact(`m${i}`, "gros"));
  assert.equal(pairs(many).length, 3999);
  assert.equal(counts(many).get("m7"), 3999, "mais chacun est bien lié aux 3 999 autres");
  const alone = fact("seul", "autre"), started = Date.now();
  for (let i = 0; i < 50; i += 1) assert.deepEqual(ids(of([...many, alone], alone)), []);
  assert.ok(Date.now() - started < 2000, "50 lectures d'un souvenir isolé parmi 4 000 en moins de 2 s");
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
  assert.equal(counts(null).size, 0);
  assert.deepEqual(ids(of(undefined, fact("a", "c1"))), []);
});
