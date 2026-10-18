import assert from "node:assert/strict";
import { test } from "node:test";
import { Mention, readMentions } from "./mentions.js";

test("identical labels and renames never replace the structured destination IDs", () => {
  const nodes = [
    { type: { name: "botMention" }, attrs: { id: "a", label: "Même nom" } },
    { type: { name: "text" }, text: " @autre-nom" },
    { type: { name: "botMention" }, attrs: { id: "b", label: "Même nom" } },
    { type: { name: "botMention" }, attrs: { id: "@everyone", label: "everyone" } },
  ];
  const editor = { state: { doc: { descendants: (visit) => nodes.forEach(visit) } } };
  assert.deepEqual(readMentions(editor), [{ id: "a", label: "Même nom" }, { id: "b", label: "Même nom" }, { id: "@everyone", label: "everyone" }]);
  nodes[0].attrs.label = "Nom changé";
  assert.equal(readMentions(editor)[0].id, "a");
});

test("bot IDs cannot clobber shell DOM IDs and labels remain inert text", () => {
  const attributes = Mention.config.addAttributes.call(Mention);
  assert.equal(attributes.id.rendered, false);
  assert.equal(attributes.label.rendered, false);
  const rendered = Mention.config.renderHTML.call(Mention, { node: { attrs: { id: "send-button", label: "<script>" } }, HTMLAttributes: {} });
  assert.equal(rendered[1].id, undefined);
  assert.equal(rendered[1]["data-bot-mention"], "send-button");
  assert.equal(rendered[2], "@<script>");
  assert.equal(rendered[1].title, "Bot : send-button");
});
