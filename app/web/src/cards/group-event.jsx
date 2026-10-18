export const kind = "group_event";
export function Card({ message }) {
  return <p className="gb-group-event" role="status">{message.body}</p>;
}
