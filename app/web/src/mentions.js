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
  editor.state.doc.descendants((node) => {
    if (node.type.name === "botMention") mentions.push({ id: node.attrs.id, label: node.attrs.label });
  });
  return mentions;
}

export function mentionMenu(element, getCandidates, addMember) {
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
    if (!match || dismissedAt === `${$from.pos}:${before}`) { hide(); return; }
    range = { from: $from.pos - match[1].length - 1, to: $from.pos, doc: editor.state.doc };
    const query = match[1].toLocaleLowerCase("fr");
    if (query !== previousQuery || range.from !== previousFrom) current = 0;
    previousQuery = query; previousFrom = range.from;
    choices = getCandidates().filter((c) => c.name.toLocaleLowerCase("fr").includes(query) || c.id.toLocaleLowerCase("fr").includes(query)).slice(0, 12);
    current = Math.min(current, Math.max(0, choices.length - 1));
    menu.replaceChildren();
    choices.forEach((candidate, index) => {
      const button = document.createElement("button");
      button.type = "button";
      button.setAttribute("role", "option");
      button.setAttribute("aria-selected", String(current === index));
      const name = document.createElement("strong");
      name.textContent = candidate.outside ? `Ajouter ${candidate.name} et mentionner` : candidate.name;
      const id = document.createElement("small");
      id.textContent = candidate.id === "@everyone" ? "Tous les membres actifs à l’envoi" : candidate.id;
      button.append(name, id);
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
      dismissedAt = `${$from.pos}:${$from.parent.textBetween(0, $from.parentOffset, "\n", "\ufffc")}`;
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
