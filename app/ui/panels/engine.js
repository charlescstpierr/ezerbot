"use strict";

(() => {
  const panels = window.GrokbotPanels;
  if (!panels) return;
  panels.register({
    id: "engine",
    wantsPolling({ state }) {
      // Track lifecycle, NOT TaskStatus::Working: a finished chat/tool turn does
      // not finish a task (REA goal-execution-autonomy.md §14). Human waits are
      // handled by forms' delivery receipts, without polling while users think.
      // Match runs by run_id/conversation_id, never the qualified trigger's
      // message_id (engine-cutover review r2 §5: "<conversation>/<message>").
      return (state.engine?.runs || []).some((run) =>
        ["queued", "running", "interrupted", "awaiting_children"].includes(run.status));
    },
  });
})();
