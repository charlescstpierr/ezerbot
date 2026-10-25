// chat-mermaid.js resolves `import("katex")` here instead of shipping a second KaTeX. Mermaid
// only calls it for $$…$$ labels, and src/mermaid.js loads chat-math.js before those render.
export default {
  renderToString: (...args) => globalThis.GrokbotMath.katex.renderToString(...args),
};
