const STOPPED = new Set(["interrupted", "cancelled", "failed", "finished", "ended"]);

export function runView(stored, live) {
  const status = live && live !== "ended" ? live : stored || live;
  return { status, stopped: STOPPED.has(status) };
}
