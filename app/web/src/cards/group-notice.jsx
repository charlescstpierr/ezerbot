export const kind = "group_notice";
export function Card({ message }) {
  return <aside className="gb-group-notice" role="status"><strong>{message.body.startsWith("Limite") ? "Limite de la discussion" : "Discussion de groupe"}</strong><p>{message.body}</p></aside>;
}
