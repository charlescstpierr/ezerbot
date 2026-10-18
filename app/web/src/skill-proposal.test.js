import assert from "node:assert/strict";
import { test } from "node:test";
import { skillDiff } from "./skill-proposal.js";

const skill = { display_name: "Résumé", description: "Trois puces", body: "Lire.\nRésumer.", files: [{ path: "exemple.md", content: "Ancien" }] };
test("skill diff includes metadata, body and companion-file changes", () => {
  const updated = { ...skill, description: "Quatre puces", body: "Lire.\nVérifier.\nRésumer.", files: [{ path: "exemple.md", content: "Nouveau" }] };
  const diff = skillDiff(skill, updated);
  assert.ok(diff.some((line) => line.kind === "removed" && line.text === "Description : Trois puces"));
  assert.ok(diff.some((line) => line.kind === "added" && line.text === "Description : Quatre puces"));
  assert.ok(diff.some((line) => line.kind === "added" && line.text === "Vérifier."));
  assert.ok(!diff.some((line) => line.kind === "removed" && line.text === "Lire."));
  assert.ok(diff.some((line) => line.kind === "removed" && line.text === "Ancien"));
  assert.ok(diff.some((line) => line.kind === "added" && line.text === "Nouveau"));
  assert.deepEqual(skillDiff(skill, skill), []);
});
test("skill diff handles very long repeated content with bounded space", () => {
  const before = { ...skill, body: "Ligne\n".repeat(40000) };
  const after = { ...before, body: `${before.body}Fin` };
  const diff = skillDiff(before, after);
  assert.ok(diff.length < 15);
  assert.ok(diff.some((line) => line.kind === "added" && line.text === "Fin"));
});
