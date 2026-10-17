"use strict";

(() => {
  const panels = window.GrokbotPanels;
  if (!panels) return;
  let latestTasks = new Map();
  let latestSuspensions = new Map();
  let runs = new Map();
  let requests = new Map();
  let ready = false;
  let version = 0;
  // Metadata only: never retain tool arguments, outputs or opaque reasoning.
  window.GrokbotEngine = {
    latestTask: (id) => latestTasks.get(id),
    latestSuspension: (id) => latestSuspensions.get(id),
    run: (id) => runs.get(id) || (ready ? { status: "ended" } : undefined),
    request: (id) => requests.get(id),
    version: () => version,
  };
  panels.register({
    id: "engine",
    onState(state) {
      latestTasks = new Map();
      latestSuspensions = new Map();
      runs = new Map();
      requests = new Map();
      const observeRequest = (request) => {
        if (request?.id) requests.set(request.id, {
          run_id: request.run_id, status: request.status?.status, wake: request.receipt?.wake?.status,
        });
      };
      for (const conversation of state.conversations || []) {
        for (const message of conversation.messages || []) {
          const payload = message.payload;
          if (message.kind === "task" && payload?.run_id) {
            latestTasks.set(payload.run_id, message.id);
            runs.set(payload.run_id, { status: payload.status, turn_end: payload.turn_end });
          }
          if (message.kind === "tool" && payload?.run_id && payload.outcome?.status === "suspend") latestSuspensions.set(payload.run_id, message.id);
          if (["form", "approval", "secret"].includes(message.kind)) observeRequest(payload);
        }
      }
      for (const request of state.forms?.requests || []) observeRequest(request);
      // engine.runs is the complete NON-terminal list. Absence proves inactivity,
      // not success; terminal task snapshots above give a more precise reason.
      ready = Array.isArray(state.engine?.runs);
      for (const run of state.engine?.runs || []) runs.set(run.run_id, { status: run.status, turn_end: run.turn_end });
      version += 1;
      window.dispatchEvent(new Event("grokbot:engine-change"));
    },
    wantsPolling({ state }) {
      // Global lifecycle across ALL conversations, including hidden task/routine
      // threads. Never match qualified trigger.message_id or poll just because a
      // task is Working (REA goal-execution-autonomy.md §14).
      return (state.engine?.runs || []).some((run) =>
        ["running", "awaiting_children"].includes(run.status)
        || (["queued", "interrupted"].includes(run.status) && state.provider?.connected === true));
    },
  });
})();
