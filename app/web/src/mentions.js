import { Node, mergeAttributes } from "@tiptap/core";

// Atomic nodes keep identity independent of spelling, duplicate names and renames.
export const Mention = Node.create({
  name: "botMention", group: "inline", inline: true, atom: true,
  addAttributes() { return { id: { default: null, rendered: false }, label: { default: "", rendered: false } }; },
  parseHTML() { return [{ tag: "span[data-bot-mention]", getAttrs: (node) => ({ id: node.dataset.botMention, label: node.dataset.label }) }]; },
  renderHTML({ node, HTMLAttributes }) { return ["span", mergeAttributes(HTMLAttributes, { "data-bot-mention": node.attrs.id, "data-label": node.attrs.label, title: node.attrs.id === "@everyone" ? "Tous les membres actifs" : `Bot : ${node.attrs.id}`, class: "gb-mention" }), `@${node.attrs.label}`]; },
  renderText({ node }) { return `@${node.attrs.label}`; },
});

export function readMentions(editor) {
  const mentions = [];
  const text = editor.getText({ blockSeparator: "\n" });
  const leading = text.length - text.trimStart().length;
  editor.state.doc.descendants((node, position) => {
    if (node.type.name === "botMention") {
      const prefix = editor.state.doc.textBetween(0, position, "\n", (leaf) => leaf.type.name === "botMention" ? `@${leaf.attrs.label}` : leaf.type.name === "hardBreak" ? "\n" : "");
      mentions.push({ id: node.attrs.id, label: node.attrs.label, offset: prefix.length - leading });
    }
  });
  return mentions;
}

// Offsets are durable document metadata, never a name-based recipient lookup.
// Legacy plain mentions stay plain rather than guessing which @name was atomic.
export function splitMentionText(body = "", mentions = []) {
  const parts = [];
  let from = 0;
  for (const mention of mentions) {
    const token = `@${mention.label}`;
    if (!Number.isInteger(mention.offset) || mention.offset < from || body.slice(mention.offset, mention.offset + token.length) !== token) continue;
    if (mention.offset > from) parts.push({ text: body.slice(from, mention.offset) });
    parts.push({ text: token, mention });
    from = mention.offset + token.length;
  }
  if (from < body.length) parts.push({ text: body.slice(from) });
  return parts;
}

export function mentionMenu(element, getCandidates, addMember, createAvatar, summary) {
  const menu = document.createElement("div");
  menu.className = "gb-mention-menu";
  menu.setAttribute("role", "listbox");
  menu.setAttribute("aria-label", "Mentionner un bot");
  menu.hidden = true;
  element.parentElement.append(menu);
  let range = null;
  let current = 0;
  let choices = [];
  let dismissedAt = null;
  let previousQuery = null;
  let previousFrom = null;
  function hide() { menu.hidden = true; range = null; previousQuery = null; previousFrom = null; }
  async function choose(editor, candidate) {
    const saved = range;
    hide();
    if (candidate.outside && !await addMember(candidate.id)) return;
    // Do not apply an async choice to a newly edited document or another thread.
    if (!saved || editor.state.doc !== saved.doc) return;
    editor.chain().focus().insertContentAt({ from: saved.from, to: saved.to }, [{ type: "botMention", attrs: { id: candidate.id, label: candidate.name } }, { type: "text", text: " " }]).run();
  }
  function update(editor) {
    const { $from, empty } = editor.state.selection;
    const before = $from.parent.textBetween(0, $from.parentOffset, "\n", "\ufffc");
    const match = empty && before.match(/(?:^|\s)@([^@\s]*)$/u);
    const selectionKey = `${$from.pos}:${before}`;
    if (dismissedAt && (dismissedAt.doc !== editor.state.doc || dismissedAt.key !== selectionKey)) dismissedAt = null;
    if (!match || dismissedAt?.key === selectionKey) { hide(); return; }
    range = { from: $from.pos - match[1].length - 1, to: $from.pos, doc: editor.state.doc };
    const query = match[1].toLocaleLowerCase("fr");
    if (query !== previousQuery || range.from !== previousFrom) current = 0;
    previousQuery = query; previousFrom = range.from;
    choices = getCandidates().filter((c) => c.name.toLocaleLowerCase("fr").includes(query) || c.id.toLocaleLowerCase("fr").includes(query)).slice(0, 12);
    current = Math.min(current, Math.max(0, choices.length - 1));
    menu.replaceChildren();
    let outside = false;
    choices.forEach((candidate, index) => {
      if (candidate.outside && !outside) {
        const separator = document.createElement("div"); separator.className = "gb-mention-separator"; separator.setAttribute("role", "presentation"); separator.textContent = "Ajouter au groupe"; menu.append(separator); outside = true;
      }
      const button = document.createElement("button");
      button.type = "button";
      button.setAttribute("role", "option");
      button.setAttribute("aria-selected", String(current === index));
      const everyone = candidate.id === "@everyone";
      button.title = everyone ? "Tous les membres actifs à l’envoi" : `Bot : ${candidate.id}`;
      button.setAttribute("aria-label", everyone ? "@everyone, tous les membres actifs" : `${candidate.outside ? "Ajouter et mentionner" : "Mentionner"} ${candidate.name} (${candidate.id})`);
      const avatar = document.createElement("span"); avatar.className = "gb-mention-avatar"; avatar.setAttribute("aria-hidden", "true");
      if (everyone) avatar.textContent = "@";
      else { const figure = createAvatar?.(candidate); if (figure) avatar.append(figure); }
      const copy = document.createElement("span"); copy.className = "gb-mention-copy";
      const name = document.createElement("strong");
      name.textContent = everyone ? "@everyone" : candidate.outside ? `Ajouter ${candidate.name} et mentionner` : candidate.name;
      const description = document.createElement("small");
      description.textContent = everyone ? "Tous les membres actifs à l’envoi" : summary?.(candidate) || candidate.description || "Bot personnel";
      copy.append(name, description); button.append(avatar, copy);
      button.onmousedown = (event) => event.preventDefault();
      button.onclick = () => choose(editor, candidate);
      menu.append(button);
    });
    if (!choices.length) menu.textContent = "Aucun bot correspondant.";
    menu.hidden = false;
    menu.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: "nearest" });
  }
  function keydown(editor, event) {
    if (menu.hidden || event.isComposing) return false;
    if (event.key === "Escape") {
      const { $from } = editor.state.selection;
      dismissedAt = { doc: editor.state.doc, key: `${$from.pos}:${$from.parent.textBetween(0, $from.parentOffset, "\n", "\ufffc")}` };
      hide(); event.preventDefault(); return true;
    }
    if (["ArrowDown", "ArrowUp"].includes(event.key)) {
      current = (current + (event.key === "ArrowDown" ? 1 : choices.length - 1)) % (choices.length || 1);
      update(editor); event.preventDefault(); return true;
    }
    if ((event.key === "Enter" || event.key === "Tab") && choices[current]) {
      event.preventDefault(); choose(editor, choices[current]); return true;
    }
    return false;
  }
  return { update, keydown, hide };
}
