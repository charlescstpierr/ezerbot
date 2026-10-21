// Run with: node --test app/ui/toast-copy.test.cjs
// The browser regression must quote the same attachment sentence as the app.
const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { test } = require("node:test");

const app = readFileSync(`${__dirname}/app.js`, "utf8");
const browser = readFileSync(`${__dirname}/../../tools/verify-app.py`, "utf8");
const phrase = "Retirez les pièces jointes";

test("browser toast checks use the composer's attachment sentence", () => {
  assert.match(app, /Retirez les pièces jointes pour changer de bot/);
  assert.match(app, /est prêt\. Retirez les pièces jointes pour le choisir/);
  const names = [
    "blocked switch explains how to retain or remove the image",
    "new-bot toast explains why image recipient is retained",
  ];
  for (const name of names) {
    const check = browser.split("\n").find((line) => line.includes(`check('${name}'`));
    assert.ok(check, name);
    assert.ok(check.includes(phrase), `${name} must expect ${phrase}`);
    assert.equal(check.includes("Retirez l’image jointe"), false);
  }
  assert.match(browser, /pending image upload already pins the displayed bot/);
  assert.match(browser, /ready staged image cannot be abandoned by a bot chip/);
  assert.match(browser, /creating a bot also retains staged image and current recipient/);
  assert.match(browser, /explicit image removal permits changing the bot/);
});
