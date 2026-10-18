import StarterKit from "@tiptap/starter-kit";
import Placeholder from "@tiptap/extension-placeholder";
import { Mention } from "./mentions.js";

// Formatting is off on purpose: the message is sent exactly as typed, Markdown source included.
const plainText = Object.fromEntries(["blockquote", "bold", "bulletList", "code", "codeBlock", "heading", "horizontalRule", "italic", "link", "listItem", "listKeymap", "orderedList", "strike", "underline"].map((name) => [name, false]));

export function editorOptions({ placeholder, onSubmit }) {
  return {
    extensions: [StarterKit.configure(plainText), Placeholder.configure({ placeholder }), Mention],
    enableInputRules: false,
    enablePasteRules: false,
    editorProps: {
      attributes: { class: "gb-editor", "aria-label": "Votre message", role: "textbox", "aria-multiline": "true", spellcheck: "true" },
      handleKeyDown(view, event) {
        if (event.key !== "Enter" || event.shiftKey || event.isComposing) return false;
        event.preventDefault();
        onSubmit();
        return true;
      },
    },
  };
}

export const readText = (editor) => editor.getText({ blockSeparator: "\n" });

export function writeText(editor, text) {
  const lines = text ? text.split("\n") : [];
  editor.commands.setContent(lines.map((line) => ({ type: "paragraph", content: line ? [{ type: "text", text: line }] : [] })), { emitUpdate: false });
}
