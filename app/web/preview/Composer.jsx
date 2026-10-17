import { useEffect, useRef } from "react";
import { EditorContent, useEditor } from "@tiptap/react";
import { ArrowUp } from "lucide-react";
import { editorOptions, readText } from "../src/editor.js";

export function Composer({ disabled = false, onSend }) {
  const submit = useRef(() => {});
  const editor = useEditor(editorOptions({ placeholder: "Qu’avez-vous en tête ?", onSubmit: () => submit.current() }));
  submit.current = () => {
    if (!editor || editor.isEmpty || disabled) return;
    onSend(readText(editor).trim());
    editor.commands.clearContent(true);
  };
  useEffect(() => { editor?.setEditable(!disabled); }, [editor, disabled]);

  return (
    <form className="gb-composer" onSubmit={(event) => { event.preventDefault(); submit.current(); }}>
      <EditorContent editor={editor} />
      <div className="gb-composer-bottom">
        <span className="gb-composer-hint">Entrée pour envoyer · ⇧ Entrée pour une nouvelle ligne</span>
        <button type="submit" className="gb-send" aria-label="Envoyer le message" disabled={!editor || editor.isEmpty || disabled}><ArrowUp /></button>
      </div>
    </form>
  );
}
