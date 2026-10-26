const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const { join } = require("node:path");
const { launchFixture } = require("./wrapper-chrome-fixture.cjs");
const { installUpdatesService, updateGeometry } = require("./updates-browser-fixture.cjs");

for (const theme of ["light", "dark"]) for (const [width, height] of [[1440, 900], [820, 560]]) {
  test(`les mises à jour préservent le rail en ${theme} à ${width}×${height}`, { timeout: 120000 }, async t => {
    const { evaluate } = await launchFixture(t, { documents: [], viewport: { width, height }, binary: process.env.EZERBOT_UPDATES_TEST_BINARY });
    const wait = async source => {
      for (let i = 0; i < 300; i++) {
        if (await evaluate(source)) return;
        await new Promise(resolve => setTimeout(resolve, 30));
      }
      assert.fail(`état absent: ${source}`);
    };
    await wait('Boolean(window.GrokbotPanels?.connected && document.querySelector(".update-corner"))');
    for (const asset of ["index.html", "styles.css"]) {
      const path = asset === "index.html" ? "/" : `/${asset}`;
      assert.equal(await evaluate(`fetch(${JSON.stringify(path)}).then(r=>r.text())`), fs.readFileSync(join(__dirname, asset), "utf8"));
    }
    await evaluate(`(${installUpdatesService})()`);
    await evaluate(`document.documentElement.dataset.theme=${JSON.stringify(theme)}`);
    const baseline = await evaluate(`(${updateGeometry})()`);
    async function check(phase) {
      const geometry = await evaluate(`(${updateGeometry})()`);
      assert.deepEqual(geometry.buttons, baseline.buttons, `${phase}: Notifications et Réglages se déplacent ${JSON.stringify(geometry)}`);
      assert.deepEqual(geometry.rail, baseline.rail, `${phase}: largeur du rail`);
      assert.ok(geometry.contained && geometry.hitTargets && geometry.legible && geometry.separate && geometry.railChildrenContained, `${phase}: débordement ou action inaccessible ${JSON.stringify(geometry)}`);
    }
    await evaluate('updatesFixture.set("disponible")');
    await check("disponible");
    await evaluate('updatesFixture.storage={backend:"file_fallback"}; GrokbotPanels.refresh()');
    assert.equal(await evaluate('document.querySelector(".storage-warning-marker").hidden'), false);
    await check("avertissement de stockage");
    await evaluate('updatesFixture.storage={backend:"file"}; GrokbotPanels.refresh()');
    await evaluate('document.querySelector(".update-corner-action").click()');
    await wait('document.querySelector(".update-corner-label").textContent === "Téléchargement…"');
    for (const phase of ["telechargement", "compilation", "installation", "redemarrage", "attente"]) {
      await evaluate(`updatesFixture.set(${JSON.stringify(phase)})`);
      assert.equal(await evaluate('document.querySelector(".update-corner-action").disabled'), true);
      await check(phase);
    }
    const error = "La compilation a échoué. L’application actuelle est restée en place. Réessayez depuis la barre latérale.";
    await evaluate(`updatesFixture.set("erreur", {message:${JSON.stringify(error)},last_error:${JSON.stringify(error)},last_attempt:"echec"})`);
    await check("erreur");
    await evaluate('document.getElementById("settings-button").click()');
    await wait('document.getElementById("settings-dialog").open');
    assert.equal(await evaluate('document.querySelector(".updates-error").textContent'), error);
    await evaluate('document.getElementById("settings-dialog").close(); document.querySelector(".update-corner-dismiss").click()');
    await check("erreur fermée");
    assert.equal(await evaluate('document.querySelector(".update-corner-action").hidden'), false);
    await evaluate('updatesFixture.failApply=true; document.querySelector(".update-corner-action").click()');
    await wait('document.querySelector(".update-corner-error-text").textContent === "Service de mise à jour indisponible."');
    await check("échec HTTP");
    await evaluate('updatesFixture.failApply=false; document.querySelector(".update-corner-dismiss").click(); document.querySelector(".update-corner-action").click()');
    await wait('document.querySelector(".update-corner-label").textContent === "Téléchargement…"');
    await check("nouvelle tentative");
    await evaluate('updatesFixture.set("erreur", {message:"Diagnostic ".repeat(160)})');
    assert.equal(await evaluate(`(() => {
      const host=document.getElementById('sidebar-updates');
      const text=document.querySelector('.update-corner-error-text');
      host.scrollTop=host.scrollHeight;
      const range=document.createRange(); range.selectNodeContents(text);
      return host.scrollHeight>host.clientHeight && [...range.getClientRects()].at(-1).bottom<=host.getBoundingClientRect().bottom;
    })()`), true);
    assert.deepEqual(await evaluate(`(${updateGeometry})().buttons`), baseline.buttons);
    await evaluate('document.getElementById("sidebar-updates").scrollTop=0; document.querySelector(".update-corner-dismiss").click()');
    await evaluate('updatesFixture.set("a_jour")');
    assert.equal(await evaluate('document.querySelector(".update-corner").hidden'), true);
    await check("repos");
    assert.equal(await evaluate('updatesFixture.calls.filter(c=>c.path==="/api/updates/apply").length'), 3);
    await evaluate('document.querySelector("[data-sidebar-destination=notifications]").click()');
    await wait('document.querySelector(".notifications-dialog").open');
  });
}
