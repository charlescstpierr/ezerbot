// Menu d’un message : une réaction est une bascule, et Renvoyer / Supprimer
// n’existent que tant que l’envoi n’est pas livré.
export const REACTIONS = ["👍", "❤️", "😂", "🎉", "👀", "🙏"];

const UNDELIVERED = new Set(["queued", "dispatching", "unknown", "refused", "prepared"]);
const IN_FLIGHT = new Set(["queued", "dispatching"]);

export function toggleReaction(current, emoji) {
  return current === emoji ? null : emoji;
}

export function messageReaction(message) {
  const value = message?.payload?.reaction ?? message?.reaction ?? null;
  return typeof value === "string" && value ? value : null;
}

export function messageActions(message, { busy = false } = {}) {
  const userSend = message?.role === "user" && [undefined, null, "message", "submission"].includes(message.kind);
  const failed = userSend && UNDELIVERED.has(message.state) && !(busy && IN_FLIGHT.has(message.state));
  return {
    status: failed ? (message.state === "refused" ? "Échec de l’envoi" : "Pas encore livré") : null,
    copy: true,
    resend: failed,
    delete: failed,
    edit: false,
    confirm: false,
    reaction: messageReaction(message),
  };
}
