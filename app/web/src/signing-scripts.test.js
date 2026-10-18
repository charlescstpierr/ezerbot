import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import test from "node:test";

test("build signing diagnostic keeps the explicit absent-identity path ad hoc without a real keychain or a build", { skip: process.platform !== "darwin" }, () => {
  const home = mkdtempSync(join(tmpdir(), "ezerbot-signing-diagnostic-"));
  try {
    const security = join(home, "security-mock");
    const calls = join(home, "calls");
    writeFileSync(security, '#!/bin/bash\nprintf "%s\\n" "$@" > "$SIGNING_TEST_CALLS"\nprintf "Policy: Code Signing\\n  Matching identities\\n     0 identities found\\n  Valid identities only\\n     0 valid identities found\\n"\n', { mode: 0o700 });
    for (const keychain of [undefined, join(home, "not-opened.keychain-db")]) {
      const env = { ...process.env, HOME: home, CFFIXED_USER_HOME: home, CARGO: "/usr/bin/false", EZERBOT_SECURITY_TOOL: security, SIGNING_TEST_CALLS: calls };
      delete env.EZERBOT_SIGNING_KEYCHAIN;
      if (keychain) env.EZERBOT_SIGNING_KEYCHAIN = keychain;
      const result = spawnSync("/bin/bash", [fileURLToPath(new URL("../../../tools/build-app.sh", import.meta.url)), "--signature-info"], { encoding: "utf8", env });
      assert.equal(result.status, 0, result.stderr);
      assert.match(result.stdout, /Signature ad hoc/);
      assert.match(result.stdout, /secrets restent dans le coffre fichier/);
      assert.equal(readFileSync(calls, "utf8"), `find-identity\n-p\ncodesigning\n${keychain ? `${keychain}\n` : ""}`);
    }
  } finally { rmSync(home, { recursive: true, force: true }); }
});
