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
  mountInput(element, { placeholder, onInput, onSubmit, onFiles }) {
    const options = editorOptions({ placeholder, onSubmit });
    const editor = new Editor({
      element, ...options, onUpdate: onInput,
      editorProps: {
        ...options.editorProps,
        handlePaste: (_view, event) => Boolean(onFiles?.(event.clipboardData?.files, event)),
        handleDrop: (_view, event) => Boolean(onFiles?.(event.dataTransfer?.files, event)),
      },
    });
    return {
      get value() { return readText(editor); },
      set value(text) { writeText(editor, text); },
      focus() { editor.commands.focus("end"); },
      setInvalid(invalid) { editor.view.dom.setAttribute("aria-invalid", String(invalid)); },
    };
  },
};
