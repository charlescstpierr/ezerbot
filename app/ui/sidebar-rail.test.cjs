const { test } = require("node:test");
const assert = require("node:assert/strict");
const { execFileSync } = require("node:child_process");
const { launchFixture } = require("./wrapper-chrome-fixture.cjs");

test("le rail ouvre les destinations existantes et garde la liste utilisable", { timeout: 120000 }, async t => {
  const { evaluate, database } = await launchFixture(t, { documents: [], viewport: { width: 820, height: 560 } });
  const wait = async source => {
    const deadline = Date.now() + 15000;
    while (Date.now() < deadline) {
      if (await evaluate(source)) return;
      await new Promise(resolve => setTimeout(resolve, 30));
    }
    assert.fail(`état attendu absent: ${source}`);
  };
  await wait('Boolean(window.GrokbotPanels?.connected && document.querySelector(".memory-entry"))');
  assert.deepEqual(await evaluate('[...document.querySelectorAll("[data-sidebar-destination]")].map(b => b.getAttribute("aria-label"))'),
    ["Conversations", "Sujets", "Routines", "Notifications", "Réglages"]);
  assert.equal(await evaluate('document.querySelector(".sidebar-pages").hidden'), true);
  await evaluate(`(async () => {
    const post = async (path, body) => {
      const r = await fetch(path, {method:'POST', headers:{'Content-Type':'application/json'}, body:JSON.stringify(body)});
      if (!r.ok) throw new Error(path + ' HTTP ' + r.status);
      return r.json();
    };
    await post('/api/bots', {id:'atelier',name:'Atelier',color:'cyan'});
    await post('/api/topics', {name:'Lancement'});
    for (let i=0;i<35;i++) await post('/api/conversations', {title:'Projet '+i,bot_id:'atelier'});
    await GrokbotPanels.refresh();
  })()`);
  await wait('document.querySelectorAll(".sidebar-bot").length === 2 && document.querySelectorAll("#conversation-list .conversation-item").length === 35');
  const dates = [15, 14, 13, 1].map(day => new Date(2030, 4, day, 12).getTime());
  execFileSync("python3", ["-c", `import json,sqlite3,sys
with sqlite3.connect(sys.argv[1]) as db:
    for i,stamp in enumerate(json.loads(sys.argv[2])):
        db.execute("UPDATE wrapper_conversations SET updated_at=? WHERE title=?", (stamp, "Projet "+str(i)))
`, database, JSON.stringify(dates)]);
  assert.deepEqual(await evaluate(`(async () => {
    const RealDate = Date;
    window.Date = class extends RealDate { constructor(...args) { super(...(args.length ? args : [${dates[0]}])); } static now() { return ${dates[0]}; } };
    try { await GrokbotPanels.refresh(); return [...document.querySelectorAll('#conversation-list .list-group-title')].map(e=>e.textContent); }
    finally { window.Date=RealDate; }
  })()`), ["Aujourd’hui", "Hier", "Cette semaine", "Précédemment"]);
  await evaluate('[...document.querySelectorAll(".sidebar-bot")].find(b => b.lastElementChild.textContent === "Atelier").click()');
  await evaluate('document.querySelector("[data-sidebar-destination=routines]").click()');
  await wait('document.querySelector(".routines-dialog")?.open');
  assert.equal(await evaluate('document.querySelector(".routines-dialog select").value'), "atelier");
  await wait('document.querySelector("[data-sidebar-destination=routines]").getAttribute("aria-pressed") === "true"');
  await evaluate('document.querySelector(".routines-dialog").close()');
  await evaluate('document.querySelector("[data-sidebar-destination=notifications]").click()');
  await wait('document.querySelector(".notifications-dialog").open && document.querySelector("[data-sidebar-destination=notifications]").getAttribute("aria-pressed") === "true"');
  await evaluate('document.querySelector(".notifications-dialog").close()');
  await evaluate('document.getElementById("settings-button").click()');
  await wait('document.getElementById("settings-dialog").open && document.getElementById("settings-button").getAttribute("aria-pressed") === "true"');
  await evaluate('document.getElementById("settings-dialog").close()');
  await evaluate('document.querySelector(".memory-entry").click()');
  await wait('!document.querySelector(".memory-space").hidden');
  await evaluate('document.querySelector("[data-sidebar-destination=topics]").click()');
  await wait('document.activeElement.id === "new-topic" && document.querySelector(".memory-space").hidden && document.querySelector("[data-sidebar-destination=topics]").getAttribute("aria-pressed") === "true"');
  await evaluate('document.querySelector(".topic-name").click()');
  await wait('document.querySelector(".main-panel").dataset.view === "topic"');
  await evaluate('document.querySelector("[data-sidebar-destination=conversations]").click()');
  await wait('!document.querySelector(".main-panel").dataset.view && document.activeElement.id === "conversation-list"');
  await evaluate('document.dispatchEvent(new KeyboardEvent("keydown", {key:"k",metaKey:true,bubbles:true}))');
  assert.equal(await evaluate('document.activeElement.id'), "search");
  await evaluate('document.dispatchEvent(new KeyboardEvent("keydown", {key:"n",metaKey:true,bubbles:true}))');
  assert.equal(await evaluate('document.getElementById("composer-bot-name").textContent'), "Atelier");
  await evaluate('document.querySelector("#conversation-list .conversation-item").click()');
  await wait('Boolean(document.querySelector("#conversation-list [aria-current=page]"))');
  const selected = await evaluate('document.querySelector("#conversation-list [aria-current=page]").dataset.conversationId');
  await evaluate('document.querySelector("[data-sidebar-destination=conversations]").click()');
  assert.equal(await evaluate('document.querySelector("#conversation-list [aria-current=page]").dataset.conversationId'), selected);
  const geometry = await evaluate(`(() => {
    const content = document.querySelector('.sidebar-content');
    content.scrollTop = content.scrollHeight;
    const last = document.querySelector('#conversation-list .conversation-item:last-child');
    const r = last.getBoundingClientRect();
    const s = content.getBoundingClientRect();
    const rail = document.querySelector('.sidebar-rail').getBoundingClientRect();
    return {scrolls:content.scrollHeight>content.clientHeight, lastVisible:r.top>=s.top && r.bottom<=s.bottom,
      separate:rail.right<=s.left, local:/\\bLocal\\b/i.test(document.getElementById('sidebar').textContent),
      small:[...document.querySelectorAll('#sidebar *')].filter(e=>e.getClientRects().length && [...e.childNodes].some(n=>n.nodeType===3 && n.textContent.trim()) && parseFloat(getComputedStyle(e).fontSize)<11.5).map(e=>e.className)};
  })()`);
  assert.deepEqual(geometry, {scrolls:true,lastVisible:true,separate:true,local:false,small:[]});
  await evaluate(`(async () => {
    const r = await fetch('/api/send', {method:'POST', headers:{'Content-Type':'application/json'}, body:JSON.stringify({
      conversation_id:${JSON.stringify(selected)},message_id:crypto.randomUUID(),
      body:'write_page:'+JSON.stringify({name:'lancement.page.tsx',title:'Plan du lancement',source:'export default function Plan() { return (<article><h1>Plan du lancement</h1><p>Préparer puis annoncer.</p></article>); }'})
    })});
    if (!r.ok) throw new Error('envoi Page HTTP '+r.status);
  })()`);
  await wait('!document.querySelector(".sidebar-pages").hidden && document.querySelector(".page-link")?.textContent === "Plan du lancement"');
  await evaluate('document.querySelector(".page-link").click()');
  await wait('!document.querySelector(".page-stage").hidden && document.querySelector(".page-canvas").textContent.includes("Préparer puis annoncer.")');
  await evaluate('[...document.querySelectorAll(".sidebar-bot")].find(b => b.lastElementChild.textContent === "Ezerbot").click()');
  await wait('document.querySelector(".sidebar-pages").hidden');
});
