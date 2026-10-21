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

test("OK — ChatGPT répond clears 4.5:1 in both themes", () => {
  const css = readFileSync(`${__dirname}/styles.css`, "utf8");
  const light = css.match(/:root \{([^}]+)\}/)[1];
  const dark = css.match(/\[data-theme="dark"\] \{([^}]+)\}/)[1];
  const token = (block) => block.match(/--ok-text:\s*(#[0-9a-fA-F]{6})/)?.[1];
  const success = css.match(/\.provider-card p\.provider-success \{([^}]+)\}/)[1];
  assert.match(success, /color:\s*var\(--ok-text\)/);
  assert.equal(token(light), "#157a41");
  assert.equal(token(dark), "#35c58a");
  const lightRatio = contrast(token(light), "#ffffff");
  const darkRatio = contrast(token(dark), "#1b1e24");
  assert.ok(lightRatio >= 4.5, `light ${token(light)} on #ffffff is ${lightRatio.toFixed(2)}:1`);
  assert.ok(darkRatio >= 4.5, `dark ${token(dark)} on #1b1e24 is ${darkRatio.toFixed(2)}:1`);
});
