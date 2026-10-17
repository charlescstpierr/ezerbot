const chatKinds = new Set([undefined, null, "message", "submission", "reply"]);

// The shell keeps a single reply for transport/retry. Render its text fragments
// around the host's call anchors instead of moving all text ahead of the tools.
// Leave unrelated/routed cards and user turns in their durable transcript order.
export function orderThread(messages) {
  const before = new Map();
  const after = new Map();
  const replies = new Set();
  const indices = new Map(messages.map((message, index) => [message.id, index]));
  const add = (slots, index, message) => slots.set(index, [...(slots.get(index) || []), message]);
  messages.forEach((message, index) => {
    const { run_id: runId, parts } = message.payload || {};
    if (message.role !== "assistant" || !chatKinds.has(message.kind) || !runId || !Array.isArray(parts)) return;
    replies.add(index);
    let tail = index;
    messages.forEach((card, cardIndex) => {
      if (!chatKinds.has(card.kind) && (card.payload?.run_id === runId || card.payload?.parent_run_id === runId)) tail = Math.max(tail, cardIndex);
    });
    // A tool-only completion has no assistant header. Preserve genuine errors,
    // notices, Stop/Retry controls and live typing even when there is no text.
    const visible = parts.filter((part) => part.body?.trim());
    if (!visible.length && (message.notice || ["streaming", "failed", "stopped"].includes(message.state))) visible.push({ body: "" });
    visible.forEach((part, partIndex) => {
      const last = partIndex === visible.length - 1;
      const fragment = {
        ...message,
        id: last ? message.id : `${message.id}:part:${partIndex}`,
        body: part.body,
        state: last ? message.state : "echoed",
        notice: last ? message.notice : undefined,
        error: last ? message.error : undefined,
      };
      const anchor = indices.get(part.before);
      if (anchor !== undefined) add(before, anchor, fragment);
      else add(after, tail, fragment);
    });
  });
  return messages.flatMap((message, index) => [
    ...(before.get(index) || []),
    ...(replies.has(index) ? [] : [message]),
    ...(after.get(index) || []),
  ]);
}
