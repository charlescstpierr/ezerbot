import { createRoot } from "react-dom/client";
import { Editor } from "@tiptap/core";
import "./chat.css";
import { Thread } from "./Thread.jsx";
import { editorOptions, readText, writeText } from "./editor.js";
import { createDocumentIcon, formatAttachmentSize, documentBudgetMessage } from "./attachment-presentation.js";
import { mentionMenu, readMentions } from "./mentions.js";
import { pendingReply } from "./thread-order.js";

const roots = new WeakMap();

// The shell (app.js) owns state and transport; this bundle draws the thread and the message field.
window.GrokbotChat = {
  formatAttachmentSize,
  documentBudgetMessage,
  createDocumentIcon: () => createDocumentIcon(document),
  isPendingReply: pendingReply,
  renderThread(container, props) {
    if (!roots.has(container)) roots.set(container, createRoot(container));
    roots.get(container).render(<Thread {...props} />);
  },

  // Returns a stand-in for the former <textarea>.
  mountInput(element, { placeholder, onInput, onSubmit, onFiles, getMentionCandidates = () => [], onAddMember = async () => false, createMentionAvatar, mentionSummary }) {
    const menu = mentionMenu(element, getMentionCandidates, onAddMember, createMentionAvatar, mentionSummary);
    let currentPlaceholder = placeholder;
    const options = editorOptions({ placeholder: () => currentPlaceholder, onSubmit });
    const editor = new Editor({
      element, ...options, onUpdate: ({ editor }) => { onInput(); menu.update(editor); },
      onSelectionUpdate: ({ editor }) => menu.update(editor),
      onBlur: () => setTimeout(() => menu.hide(), 150),
      editorProps: {
        ...options.editorProps,
        handleKeyDown: (view, event) => menu.keydown(editor, event) || options.editorProps.handleKeyDown(view, event),
        handlePaste: (_view, event) => Boolean(onFiles?.(event.clipboardData?.files, event)),
        handleDrop: (_view, event) => Boolean(onFiles?.(event.dataTransfer?.files, event)),
      },
    });
    return {
      get value() { return readText(editor); },
      set value(text) { menu.hide(); writeText(editor, text); },
      get mentions() { return readMentions(editor); },
      get document() { return editor.getJSON(); },
      set document(value) { menu.hide(); editor.commands.setContent(value, { emitUpdate: false }); },
      focus() { if (editor.isEditable) editor.commands.focus("end"); },
      setReadOnly(readOnly) {
        if (readOnly) menu.hide();
        if (editor.isEditable === readOnly) editor.setEditable(!readOnly, false);
        editor.view.dom.setAttribute("aria-disabled", String(readOnly));
      },
      setPlaceholder(text) {
        if (currentPlaceholder === text) return;
        currentPlaceholder = text;
        editor.view.dispatch(editor.state.tr);
      },
      setInvalid(invalid) { editor.view.dom.setAttribute("aria-invalid", String(invalid)); },
    };
  },
};
