// Local voice projection: only new assistant prose, never cards, secrets or old history.
export function spokenText(text) {
  return text.replace(/```[\s\S]*?(?:```|$)/g, " ").replace(/`([^`]+)`/g, "$1")
    .replace(/!\[[^\]]*\]\([^)]*\)/g, " ").replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
    .replace(/https?:\/\/\S+/g, " ").replace(/^[\s#>*|\-]+/gm, "")
    .replace(/[*_~|]/g, "").replace(/\s+/g, " ").trim();
}
export function createVoiceReader(messages = []) {
  const offsets = new Map(messages.map(m => [m.id, (m.body || "").length]));
  let muted = false;
  return {
    interrupt(messages = []) { muted = true; for (const m of messages) offsets.set(m.id, (m.body || "").length); },
    nextTurn() { muted = false; },
    read(messages, paused = false) {
      const chunks = [];
      for (const m of messages) {
        if (m.role !== "assistant" || ![undefined, null, "message", "reply"].includes(m.kind)) continue;
        const body = m.body || "", start = offsets.get(m.id) || 0;
        if (muted || ["failed", "stopped"].includes(m.state)) { offsets.set(m.id, body.length); continue; }
        if (paused) continue;
        // Mask entire code fences before slicing, preserving source offsets. A
        // closing fence arriving in a later delta must never expose code as speech.
        const projection = body.replace(/```[\s\S]*?(?:```|$)/g, text => " ".repeat(text.length));
        let tail = projection.slice(start), consumed = 0;
        // Read full sentences during streaming, including a sentence at a chunk's end.
        // Bound a punctuation-free sentence to the original client's 1,500 UTF-16 limit.
        while (tail.length) {
          const boundary = tail.match(/[.!?…](?:[”»"')]*)(?=\s|$)|\n\n/);
          let length = boundary ? boundary.index + boundary[0].length : 0;
          if (!length && m.state !== "streaming") length = tail.length;
          if (!length && tail.length < 1500) break;
          if (!length || length > 1500) {
            length = tail.lastIndexOf(" ", 1499);
            if (length < 1) length = 1500;
            if (/[\uD800-\uDBFF]/.test(tail[length - 1])) length--;
          }
          const text = spokenText(tail.slice(0, length));
          if (text) chunks.push(text);
          consumed += length; tail = tail.slice(length);
        }
        offsets.set(m.id, start + consumed);
      }
      return chunks;
    },
  };
}
export function voiceNeedsAction(messages) {
  return messages.some(m => ["approval", "form", "secret", "skill_proposal"].includes(m.kind) && m.payload?.status?.status === "pending");
}
