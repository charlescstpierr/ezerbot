// Run Vite for app/web, then PLAYWRIGHT_MODULE=/path/to/playwright/index.mjs node --test tools/verify-sources.mjs.
import test from 'node:test';
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE || 'playwright');
import assert from 'node:assert/strict';
test('pastilles, carte, compte déconnecté et source introuvable', async t => {
const browser = await chromium.launch({executablePath:process.env.CHROMIUM_PATH || '/opt/ungoogled-chromium/chrome',headless:true,args:['--no-sandbox']});
t.after(() => browser.close());
const page=await browser.newPage({viewport:{width:820,height:560}});
page.on('pageerror',e=>console.error(e));
for(const dark of [false,true]){
 await page.goto(`http://localhost:5173/sources-preview.html?${dark?'dark':''}`);
 await page.getByRole('button',{name:'[1]',exact:true}).click();
 await assert.equal(await page.getByRole('dialog').getByText('Budget du trimestre').count(),1);
 await assert.match(await page.getByRole('dialog').textContent(),/Alice Martin.*2026-10-23/);
 await page.evaluate(() => { window.open = url => { window.openedSource = url; }; });
 await page.route('**/api/state',route=>route.fulfill({json:{mcp:{catalogue:[{id:'gmail',account:'alice@example.com',status:'connected'}]}}}));
 await page.getByRole('button',{name:'Ouvrir la source'}).click();
 await page.waitForFunction(() => window.openedSource?.endsWith('/mail1'));
 await page.unroute('**/api/state');
 await page.route('**/api/state',route=>route.fulfill({json:{mcp:{catalogue:[]}}}));
 await page.getByRole('button',{name:'Ouvrir la source'}).click();
 await page.getByRole('status').waitFor();
 assert.match(await page.getByRole('status').textContent(),/Ce compte n'est plus connecté/);
 await page.getByRole('dialog').press('Escape');
 assert.equal(await page.getByRole('button',{name:'[1]',exact:true}).evaluate(el=>el===document.activeElement),true);
 await page.locator('summary').click();
 await page.screenshot({path:`artifacts/issue-213/sources-${dark?'sombre':'clair'}.png`});
}
await page.goto('http://localhost:5173/sources-preview.html?missing');
const missing=page.getByTitle('Source introuvable');await missing.waitFor();assert.equal(await missing.evaluate(el=>getComputedStyle(el).textDecorationLine),'line-through');
await page.screenshot({path:'artifacts/issue-213/source-introuvable.png'});
await browser.close();console.log('UI sources : pastilles, carte, compte déconnecté, source introuvable et captures OK');

});
