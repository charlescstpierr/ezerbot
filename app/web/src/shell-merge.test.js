import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

const shell = readFileSync(new URL("../../native/main.m", import.meta.url), "utf8");
const method = (signature) => {
  const start = shell.indexOf(signature);
  assert.notEqual(start, -1, signature);
  const end = shell.indexOf("\n}\n", start);
  assert.notEqual(end, -1, signature);
  return shell.slice(start, end);
};

test("the Mac shell keeps microphone exclusivity and the floating panel in the same lifecycle", () => {
  assert.match(shell, /#import "voice-session\.h"/);
  assert.match(shell, /#import "floating\.h"/);
  assert.match(shell, /EzerbotVoiceSession \*voiceSession/);
  assert.match(shell, /EzerbotFloating \*floating/);
  assert.match(shell, /NSStatusItem \*statusItem/);

  const navigation = method("- (void)webView:(WKWebView *)webView didStartProvisionalNavigation:");
  assert.match(navigation, /\[self\.floating close\]/);
  assert.match(navigation, /\[self\.voice cancel\]; \[self\.voiceSession cancel\]/);
  assert.match(navigation, /self\.window\.title = @"Ezerbot"; self\.window\.titleVisibility = NSWindowTitleHidden/);

  const workspace = method("- (void)openWorkspace:");
  assert.match(workspace, /name:@"ezerbotVoiceSession"/);
  assert.match(workspace, /name:@"ezerbotFloating"/);
  assert.match(workspace, /window\.EzerbotFloatingMain\.openConversation/);

  const bridge = method("- (void)userContentController:");
  assert.match(bridge, /\[message\.name isEqual:@"ezerbotFloating"\]/);
  assert.match(bridge, /\[command isEqual:@"start"\] && \[EzerbotVoice captureStatusActive:self\.voiceSession\.status\]/);
  assert.match(bridge, /\[command isEqual:@"start"\] && \[EzerbotVoice captureStatusActive:self\.voice\.status\]/);
  assert.match(bridge, /recording \? @"● Réunion · Ezerbot" : @"Ezerbot"/);
  assert.match(bridge, /\[self\.voiceSession command:body reply:reply\]/);

  const stop = method("- (void)stopBackend");
  assert.match(stop, /\[self\.voice cancel\]/);
  assert.match(stop, /\[self\.voiceSession cancel\]/);
  assert.match(stop, /\[self\.floating invalidate\]/);

  const close = method("- (BOOL)windowShouldClose:");
  assert.match(close, /\[sender orderOut:nil\]; return NO/);
  const quit = method("- (BOOL)applicationShouldTerminateAfterLastWindowClosed:");
  assert.match(quit, /return NO/);
});
