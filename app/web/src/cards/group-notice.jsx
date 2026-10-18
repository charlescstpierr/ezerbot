export const kind = "group_notice";
export function Card({ message }) {
  return <aside className="gb-group-notice" role="status"><strong>{message.payload?.terminal ? "Fin de la chaîne" : "Discussion de groupe"}</strong><p>{message.body}</p></aside>;
}
