import { loadBundle } from "./bundles.js";

// Same plugin as @streamdown/mermaid, whose code waits in chat-mermaid.js until a message
// holds a diagram. Streamdown shows its loading state while render is pending.
export function createMermaidPlugin(options) {
  let plugin;
  const ready = () => loadBundle("mermaid").then((bundle) => (plugin ??= bundle.createMermaidPlugin(options)));
  return {
    name: "mermaid",
    type: "diagram",
    language: "mermaid",
    getMermaid(config) {
      return {
        initialize: (next) => { config = next; },
        render: async (id, source) => (await ready()).getMermaid(config).render(id, source),
      };
    },
  };
}
