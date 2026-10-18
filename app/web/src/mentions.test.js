import assert from "node:assert/strict";
import { test } from "node:test";
import { Schema } from "@tiptap/pm/model";
import { Mention, readMentions, splitMentionText, mentionMenu } from "./mentions.js";

const schema = new Schema({ nodes: {
  doc: { content: "paragraph+" }, paragraph: { content: "inline*" }, text: { group: "inline" },
  botMention: { inline: true, atom: true, group: "inline", attrs: { id: {}, label: {} } },
  hardBreak: { inline: true, group: "inline" },
} });
const leafText = (node) => node.type.name === "botMention" ? `@${node.attrs.label}` : "\n";
function editor(label = "Même nom") {
  const mention = (id, label) => schema.node("botMention", { id, label });
  const doc = schema.node("doc", null, [schema.node("paragraph", null, [schema.text("  😀 "), mention("a", label), schema.text(" @autre-nom"), schema.node("hardBreak"), mention("b", "Même nom"), schema.text(" "), mention("@everyone", "everyone")])]);
  return { state: { doc }, getText: () => doc.textBetween(0, doc.content.size, "\n", leafText) };
}

test("identical labels, emoji, line breaks and trimming retain IDs and exact UTF-16 offsets", () => {
  const e = editor();
  const body = e.getText().trim();
  assert.deepEqual(readMentions(e), [
    { id: "a", label: "Même nom", offset: body.indexOf("@Même nom") },
    { id: "b", label: "Même nom", offset: body.lastIndexOf("@Même nom") },
    { id: "@everyone", label: "everyone", offset: body.indexOf("@everyone") },
  ]);
  assert.equal(readMentions(editor("Nom changé"))[0].id, "a");
});

test("sent chips use document offsets, not manual @name matches or current renamed names", () => {
  const body = "😀 @Aurore tapé puis @Aurore choisi";
  const offset = body.lastIndexOf("@Aurore");
  const parts = splitMentionText(body, [{ id: "bot-b", label: "Aurore", offset }]);
  assert.equal(parts[0].text, "😀 @Aurore tapé puis ");
  assert.equal(parts[0].mention, undefined);
  assert.equal(parts[1].mention.id, "bot-b");
  assert.equal(parts.map((p) => p.text).join(""), body);
  assert.deepEqual(splitMentionText(body, [{ id: "legacy", label: "Aurore" }]), [{ text: body }]);
  assert.deepEqual(splitMentionText(body, [{ id: "corrupt", label: "Aurore", offset: 1 }]), [{ text: body }]);
});

test("Escape dismisses only the unchanged query, not a newly typed @ at the same position", () => {
  class Element {
    constructor() { this.children = []; }
    append(...children) { this.children.push(...children); }
    replaceChildren() { this.children = []; }
    setAttribute() {}
    querySelector() { return { scrollIntoView() {} }; }
  }
  const original = globalThis.document;
  globalThis.document = { createElement: () => new Element() };
  try {
    const parent = new Element();
    const editor = { state: { doc: {}, selection: { empty: true, $from: { pos: 2, parentOffset: 1, parent: { textBetween: () => "@" } } } } };
    const menu = mentionMenu({ parentElement: parent }, () => [{ id: "b", name: "Boréal" }], async () => false);
    const popup = parent.children[0];
    menu.update(editor);
    assert.equal(popup.hidden, false);
    menu.keydown(editor, { key: "Escape", preventDefault() {} });
    menu.update(editor);
    assert.equal(popup.hidden, true);
    editor.state.doc = {}; // clear and type again, with the same caret/query
    menu.update(editor);
    assert.equal(popup.hidden, false);
  } finally {
    if (original === undefined) delete globalThis.document;
    else globalThis.document = original;
  }
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
