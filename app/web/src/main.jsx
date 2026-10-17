import { createRoot } from "react-dom/client";
import { Editor } from "@tiptap/core";
import "./chat.css";
import { Thread } from "./Thread.jsx";
import { editorOptions, readText, writeText } from "./editor.js";

const roots = new WeakMap();

// The shell (app.js) owns state and transport; this bundle draws the thread and the message field.
window.GrokbotChat = {
  renderThread(container, props) {
    if (!roots.has(container)) roots.set(container, createRoot(container));
    roots.get(container).render(<Thread {...props} />);
  },

  // Returns a stand-in for the former <textarea>.
  mountInput(element, { placeholder, onInput, onSubmit }) {
    const editor = new Editor({ element, ...editorOptions({ placeholder, onSubmit }), onUpdate: onInput });
    return {
      get value() { return readText(editor); },
      set value(text) { writeText(editor, text); },
      focus() { editor.commands.focus("end"); },
      setInvalid(invalid) { editor.view.dom.setAttribute("aria-invalid", String(invalid)); },
    };
  },
};
