// Selection for one share card. Preselect eligible work facts once per message,
// then only drop identifiers that are shared, sensitive, or no longer listed.
export function reconcileShareSelection(previous, message) {
  const facts = message.payload?.facts || [];
  if (!previous || previous.messageId !== message.id) {
    return {
      messageId: message.id,
      selected: new Set(
        facts
          .filter((fact) => fact.class === "work" && !fact.sensitive && !fact.shared)
          .map((fact) => fact.fact_id),
      ),
    };
  }
  const keep = new Set(
    facts.filter((fact) => !fact.sensitive && !fact.shared).map((fact) => fact.fact_id),
  );
  return {
    messageId: message.id,
    selected: new Set([...previous.selected].filter((id) => keep.has(id))),
  };
}
