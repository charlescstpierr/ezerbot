import { useEffect, useState } from "react";

// Mermaid's colour parser only reads hex and comma-separated rgb().
function hex(value) {
  const context = document.createElement("canvas").getContext("2d");
  context.fillStyle = value.trim();
  return context.fillStyle;
}

function read() {
  const root = document.documentElement;
  const style = getComputedStyle(root);
  const token = (name) => hex(style.getPropertyValue(name));
  return {
    key: `${root.dataset.theme}-${root.dataset.accent}`,
    dark: root.dataset.theme === "dark",
    text: token("--text"),
    muted: token("--muted"),
    pane: token("--pane"),
    soft: token("--soft"),
    canvas: token("--canvas"),
    accent: token("--accent-ink"),
  };
}

export function useAppearance() {
  const [appearance, setAppearance] = useState(read);
  useEffect(() => {
    const observer = new MutationObserver(() => setAppearance(read()));
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme", "data-accent"] });
    return () => observer.disconnect();
  }, []);
  return appearance;
}
