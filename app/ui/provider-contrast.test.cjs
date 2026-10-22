// Run with: node --test app/ui/provider-contrast.test.cjs
const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { test } = require("node:test");

function linear(channel) {
  const value = channel / 255;
  return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
}

function luminance(hex) {
  const rgb = parseInt(hex.slice(1), 16);
  return 0.2126 * linear((rgb >> 16) & 255) + 0.7152 * linear((rgb >> 8) & 255) + 0.0722 * linear(rgb & 255);
}

function contrast(foreground, background) {
  const lighter = Math.max(luminance(foreground), luminance(background));
  const darker = Math.min(luminance(foreground), luminance(background));
  return (lighter + 0.05) / (darker + 0.05);
}

test("OK — ChatGPT répond clears 4.5:1 in both themes on the dialog background", () => {
  const css = readFileSync(`${__dirname}/styles.css`, "utf8");
  const light = css.match(/:root \{([^}]+)\}/)[1];
  const dark = css.match(/\[data-theme="dark"\] \{([^}]+)\}/)[1];
  const token = (block, name) => block.match(new RegExp(`${name}:\\s*(#[0-9a-fA-F]{6})`))?.[1];
  const success = css.match(/\.provider-card p\.provider-success \{([^}]+)\}/)[1];
  const dialog = css.match(/\.dialog \{([^}]+)\}/)[1];
  assert.match(success, /color:\s*var\(--ok-text\)/);
  // The text sits on the dialog; it must be measured on that real background, not on an assumed white.
  assert.match(dialog, /background:\s*var\(--pane\)/);
  for (const [theme, block] of [["light", light], ["dark", dark]]) {
    const foreground = token(block, "--ok-text");
    const background = token(block, "--pane");
    const ratio = contrast(foreground, background);
    assert.ok(ratio >= 4.5, `${theme} ${foreground} on ${background} is ${ratio.toFixed(2)}:1`);
  }
  assert.equal(token(dark, "--ok-text"), "#35c58a", "the dark theme stays unchanged");
});
