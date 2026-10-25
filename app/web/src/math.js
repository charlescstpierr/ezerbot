import remarkMath from "remark-math";
import { loadBundle, loadedBundle } from "./bundles.js";

const katexOptions = { errorColor: "var(--color-muted-foreground)" };
const mathClasses = ["language-math", "math-inline", "math-display"];

// The elements rehype-katex typesets. Streamdown's sanitizer keeps only language-math.
function hasMath(node) {
  const names = node.properties?.className;
  if (Array.isArray(names) && mathClasses.some((name) => names.includes(name))) return true;
  return node.children?.some(hasMath) ?? false;
}

// remark-math stays in chat.js and marks the formulas; KaTeX waits in chat-math.js. Until it
// arrives the formula shows as its source; Thread.jsx renders again when it does.
function rehypeMath() {
  return (tree, file) => {
    const bundle = loadedBundle("math");
    if (bundle) return bundle.rehypeKatex(katexOptions)(tree, file);
    if (hasMath(tree)) loadBundle("math").catch(() => {});
  };
}

// Streamdown parses again only when the plugin changes, so each arrival gets a new object.
export function createMathPlugin() {
  return { name: "katex", type: "math", remarkPlugin: [remarkMath, { singleDollarTextMath: true }], rehypePlugin: [rehypeMath] };
}
