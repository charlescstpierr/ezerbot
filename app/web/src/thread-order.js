const chatKinds = new Set([undefined, null, "message", "submission", "reply"]);

// The model can finish before the durable scheduler admits a return or appends
// its terminal notice. Poll until that group lifecycle settles as well.
export function pendingReply(message, group = false) {
  return message.state === "streaming" || (group && ["queued", "running"].includes(message.payload?.turn_status));
}

// The shell keeps a single reply for transport/retry. Render its text fragments
// around the host's call anchors instead of moving all text ahead of the tools.
// Leave unrelated/routed cards and user turns in their durable transcript order.
export function orderThread(messages) {
  // A group's placeholders are admitted before their calls arrive. Keep each
  // run's cards and fragments together at that durable turn anchor, otherwise
  // a later queued peer/return reply can overtake its author's tool-first turn.
  const runs = new Set(messages.filter((m) => chatKinds.has(m.kind) && m.payload?.turn_id && m.payload?.run_id).map((m) => m.payload.run_id));
  const owner = (m) => !chatKinds.has(m.kind) && (runs.has(m.payload?.run_id) ? m.payload.run_id : runs.has(m.payload?.parent_run_id) ? m.payload.parent_run_id : null);
  const cards = new Map();
  for (const message of messages) {
    const run = owner(message);
    if (run) cards.set(run, [...(cards.get(run) || []), message]);
  }
  messages = messages.flatMap((message) => owner(message) ? [] : [message, ...(message.payload?.turn_id ? cards.get(message.payload.run_id) || [] : [])]);
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
    // A turn's status belongs after its last call, not on pre-call text.
    if ((message.notice || ["streaming", "failed", "stopped"].includes(message.state))
      && (!visible.length || indices.has(visible.at(-1).before))) visible.push({ body: "" });
    visible.forEach((part, partIndex) => {
      const last = partIndex === visible.length - 1;
      const fragment = {
        ...message,
        id: last ? message.id : `${message.id}:part:${partIndex}`,
        // The transport id moves to the status fragment; the React key must not
        // move with it or previously read text remounts/animates on each call.
        key: `${message.id}#${partIndex}`,
        continuation: partIndex > 0,
        copyText: last ? message.body : null,
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
  // A human-input call is already represented by its request card. Match the
  // durable request and run, not just the tool name: failed calls and requests
  // routed to another conversation must keep their diagnostic tool card.
  const requests = new Map(messages.filter((message) => ["form", "secret"].includes(message.kind)
    && message.payload?.id).map((message) => [message.payload.id, message]));
  const ordered = messages.flatMap((message, index) => [
    ...(before.get(index) || []),
    ...(replies.has(index) ? [] : [message]),
    ...(after.get(index) || []),
  ]).filter((message) => {
    if (message.kind !== "tool") return true;
    const { call, outcome, run_id: runId } = message.payload || {};
    // These calls have no UI of their own (the task card carries their status).
    if (["begin_task", "complete_task"].includes(call?.name)) return false;
    const requestKind = { request_form: "form", request_secret: "secret" }[call?.name];
    const request = outcome?.status === "suspend" && outcome.cause?.kind === "human_input"
      ? requests.get(outcome.cause.id) : undefined;
    return !(requestKind && request?.kind === requestKind && runId && request.payload.run_id === runId);
  });

  // When a turn opens with cards, move only its heading above them. Text still
  // follows the call anchors, and a completed tool-only turn gets no empty reply.
  const headings = new Map();
  for (const index of replies) {
    const message = messages[index];
    const runId = message.payload.run_id;
    const first = ordered.findIndex((item) => item.key && item.payload?.run_id === runId);
    if (first < 0) {
      // In a group, a delivery-only turn still needs its distinct author above
      // its calls; it is not an anonymous tool card or an empty text response.
      if (message.payload.turn_id) {
        const opening = ordered.findIndex((item) => !chatKinds.has(item.kind) && item.payload?.run_id === runId);
        if (opening >= 0) headings.set(opening, { id: `${message.id}:heading`, kind: "turn-heading", created_at: message.created_at, payload: message.payload });
      }
      continue;
    }
    const opening = ordered.findIndex((item, position) => position < first && !chatKinds.has(item.kind)
      && (item.payload?.run_id === runId || item.payload?.parent_run_id === runId));
    if (opening < 0) continue;
    headings.set(opening, {
      id: `${message.id}:heading`, kind: "turn-heading", created_at: message.created_at, payload: message.payload,
    });
    ordered[first] = { ...ordered[first], continuation: true };
  }
  return ordered.flatMap((message, index) => headings.has(index) ? [headings.get(index), message] : [message]);
}
