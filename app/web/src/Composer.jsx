import { useEffect } from "react";
import { EditorContent, useEditor } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";
import Placeholder from "@tiptap/extension-placeholder";
import { ArrowUp } from "lucide-react";

// Rich-text message field: Markdown shortcuts while typing (**gras**, `code`, listes, ```).
export function Composer({ disabled = false, placeholder = "Qu’avez-vous en tête ?", onSend }) {
  const editor = useEditor({
    extensions: [StarterKit.configure({ heading: false, horizontalRule: false }), Placeholder.configure({ placeholder })],
    editorProps: {
      attributes: { class: "gb-editor", "aria-label": "Votre message", role: "textbox", "aria-multiline": "true" },
      handleKeyDown(view, event) {
        if (event.key !== "Enter" || event.shiftKey || event.isComposing) return false;
        if (view.state.selection.$from.parent.type.name === "codeBlock") return false;
        event.preventDefault();
        submit();
        return true;
      },
    },
  });
  const empty = !editor || editor.isEmpty;

  function submit() {
    if (!editor || editor.isEmpty || disabled) return;
    onSend(editor.getText({ blockSeparator: "\n" }).trim());
    editor.commands.clearContent(true);
  }

  useEffect(() => { editor?.setEditable(!disabled); }, [editor, disabled]);

  return (
    <form className="gb-composer" onSubmit={(event) => { event.preventDefault(); submit(); }}>
      <EditorContent editor={editor} />
      <div className="gb-composer-bottom">
        <span className="gb-composer-hint">Entrée pour envoyer · ⇧ Entrée pour une nouvelle ligne</span>
        <button type="submit" className="gb-send" aria-label="Envoyer le message" disabled={empty || disabled}><ArrowUp /></button>
      </div>
    </form>
  );
}
