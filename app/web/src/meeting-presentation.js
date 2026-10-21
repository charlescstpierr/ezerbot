export function formatMeetingDuration(milliseconds) {
  const value = Number(milliseconds);
  const seconds = Number.isFinite(value) && value > 0 ? Math.ceil(value / 1000) : 0;
  if (seconds < 60) return `${seconds} s`;
  const minutes = Math.floor(seconds / 60), remainder = seconds % 60;
  return remainder ? `${minutes} min ${String(remainder).padStart(2, "0")} s` : `${minutes} min`;
}
