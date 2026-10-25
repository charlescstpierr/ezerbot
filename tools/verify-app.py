#!/usr/bin/env python3
"""Inspect the native app, or run isolated welcome/composer browser regressions."""
from datetime import datetime, timezone
from pathlib import Path
import argparse
import hashlib
import json
import os
import plistlib
import re
import selectors
import subprocess
import sys
import tempfile
import urllib.request
import urllib.error
import uuid
from local_api import BrowserAuthentication, LaunchIdentity, launch_isolated_chrome
from bundle_inspection import BundleInspection, BundleInspectionError

ROOT = Path(__file__).resolve().parents[1]
APP = ROOT / 'app/build/Ezerbot.app'


def verify_ui_regressions():
    """Drive the real shell/editor against a disposable, simulated Rust wrapper."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ui-regressions', action='store_true')
    parser.add_argument('--report', type=Path, help='Optional report path (otherwise stdout only)')
    args = parser.parse_args()
    session = os.environ.get('CHROME_DEVTOOLS_AXI_SESSION', 'grokbot-ui-regressions')
    env = dict(os.environ, CHROME_DEVTOOLS_AXI_SESSION=f'{session}-ui-{os.getpid()}-{uuid.uuid4().hex[:8]}')
    env.pop('CHROME_DEVTOOLS_AXI_AUTO_CONNECT', None)
    # Assets are embedded with include_str!: always rebuild with the parent HOME/toolchain.
    cargo = os.environ.get('EZERBOT_VERIFY_CARGO', str(Path.home() / '.cargo/bin/cargo'))
    gate = os.environ.get('EZERBOT_VERIFY_CARGO_GATE')
    subprocess.run(([gate] if gate else []) + [cargo, 'build', '--locked', '-p', 'grokbot-wrapper'],
                   cwd=ROOT, check=True)

    def chrome(*arguments):
        result = subprocess.run(['chrome-devtools-axi', *arguments], text=True, env=env, timeout=60,
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        if authentication:
            authentication.check()
        if result.returncode or re.search(r'^error:', result.stdout, re.MULTILINE):
            raise AssertionError(result.stdout)
        return result.stdout

    with tempfile.TemporaryDirectory(prefix='grokbot-newchat-qa-') as directory:
        home = Path(directory)
        child_env = dict(os.environ, HOME=directory, CFFIXED_USER_HOME=directory)
        # No account/token is copied, and no real provider endpoint is used.
        child_env.pop('GROKBOT_FAKE_OPENAI', None)
        with (home / 'wrapper.log').open('w+') as log:
            identity = LaunchIdentity()
            child = identity.spawn([str(ROOT / 'target/debug/grokbot-wrapper'), '--port', '0',
                                      '--db', str(home / 'workspace.sqlite')], cwd=ROOT, env=child_env,
                                     stdout=subprocess.PIPE, stderr=log, text=True)
            page_id = None
            browser = None
            authentication = None
            try:
                with selectors.DefaultSelector() as selector:
                    selector.register(child.stdout, selectors.EVENT_READ)
                    if not selector.select(15):
                        raise TimeoutError('Isolated wrapper did not announce its URL')
                url = json.loads(child.stdout.readline())['url']
                if not re.fullmatch(r'http://127\.0\.0\.1:\d+', url):
                    raise AssertionError('Wrapper must bind to IPv4 loopback')
                with identity.open(url + '/app.js', timeout=10) as response:
                    if response.read() != (ROOT / 'app/ui/app.js').read_bytes():
                        raise AssertionError('Wrapper embeds a different app.js than the current worktree')
                browser, browser_url, browser_ws = launch_isolated_chrome(home / 'chrome-profile', log)
                authentication = BrowserAuthentication(identity, url, browser_ws)
                env['CHROME_DEVTOOLS_AXI_BROWSER_URL'] = browser_url
                pages = chrome('pages')
                previous = re.search(r'^\s*(\d+),[^\n]*,true\s*$', pages, re.MULTILINE)
                if not previous:
                    previous = re.search(r'^\s*(\d+),', pages, re.MULTILINE)
                    if previous:
                        chrome('selectpage', previous.group(1))
                old_page_ids = set(re.findall(r'^\s*(\d+),', pages, re.MULTILINE))
                try:
                    chrome('newpage', url)
                except AssertionError as error:
                    if 'Navigation timeout' not in str(error):
                        raise
                    # A timed-out navigation may still have created its page. Retry
                    # that page once instead of leaking another tab with the same URL.
                    created = set(re.findall(r'^\s*(\d+),', chrome('pages'), re.MULTILINE)) - old_page_ids
                    if len(created) != 1:
                        raise
                    page_id = created.pop()
                    chrome('selectpage', page_id)
                    chrome('open', url)
                page = re.search(r'^\s*(\d+),' + re.escape(url) + r'/?\s*,true\s*$', chrome('pages'), re.MULTILINE)
                if not page:
                    raise AssertionError('Could not identify the isolated browser page')
                page_id = page.group(1)
                result = chrome('eval', r'''async () => {
  const checks = [];
  const check = (name, passed) => {
    if (!passed) throw new Error(name);
    checks.push({name, passed: true});
  };
  const $ = id => document.getElementById(id);
  const api = async (path, body) => {
    const response = await fetch(path, body === undefined ? {} : {
      method: 'POST', headers: {'Content-Type': 'application/json', 'X-Grokbot-Request': 'local'},
      body: JSON.stringify(body),
    });
    if (!response.ok) throw new Error(`${path}: ${response.status}`);
    return response.json();
  };
  const until = async predicate => {
    const deadline = Date.now() + 10000;
    while (!(await predicate())) {
      if (Date.now() > deadline) throw new Error(`UI did not settle: ${predicate}`);
      await new Promise(resolve => setTimeout(resolve, 50));
    }
  };
  const editor = () => document.querySelector('.gb-editor').editor;
  const empty = () => JSON.stringify(editor().getJSON()) === JSON.stringify({type:'doc', content:[{type:'paragraph'}]});
  const type = async text => {
    document.querySelector('.gb-editor').focus();
    document.execCommand('insertText', false, text);
    // Tiptap observes the DOM edit in a microtask before firing onInput.
    await new Promise(resolve => setTimeout(resolve, 0));
  };
  const choose = name => [...document.querySelectorAll('.sidebar-bot')].find(node => node.textContent.trim() === name).click();
  const chip = () => document.querySelector('.sidebar-bot[aria-pressed=true]').textContent.trim();
  const cmdN = () => document.dispatchEvent(new KeyboardEvent('keydown', {key:'n', metaKey:true, bubbles:true, cancelable:true}));
  const saved = () => api('/api/state');
  const send = async body => {
    await type(body);
    check('typed message enables send', !$('send-button').disabled);
    $('composer-form').requestSubmit();
    await until(() => !$('new-chat').disabled);
    const state = await saved();
    const conversation = state.conversations.find(item => item.id === localStorage.getItem('grokbot.selected'));
    check('sent text is preserved', conversation.messages.some(item => item.role === 'user' && item.body === body));
    check('send clears to one valid empty paragraph', empty());
    return conversation;
  };
  await until(() => window.GrokbotPanels?.state.bots.length > 0);
  check('startup has one valid empty paragraph', empty());
  check('fresh workspace has no conversations', (await saved()).conversations.length === 0);
  await api('/api/bots', {id:'qa-redacteur', name:'Rédacteur', color:'violet', shape:'etoile', description:'Relit vos textes.', instructions:'Sois bref.'});
  await window.GrokbotPanels.refresh();
  choose('Rédacteur'); cmdN(); $('new-chat').click(); cmdN();
  await until(() => !$('new-chat').disabled);
  check('new-chat and Cmd+N do not persist empty threads', (await saved()).conversations.length === 0);
  choose('Ezerbot');
  check('visible product name and browser title are Ezerbot', document.title === 'Ezerbot' && $('home-link').textContent.trim() === 'ezerbot.' && $('home-link').getAttribute('aria-label') === 'Ezerbot, accueil');
  check('welcome chip and breadcrumb agree', chip() === 'Ezerbot' && $('workspace-label').textContent === 'Ezerbot');
  // Exercise the shipped thread renderer with pending form delivery payloads. Actual
  // model-tool suspension/resume belongs to the Rust forms integration harness.
  const formHost = document.createElement('section');
  formHost.style.cssText = 'position:fixed;inset:20px;z-index:9999;background:var(--background)';
  document.body.append(formHost);
  for (const [requester, expected] of [
    ['grokbot', 'Demande d’Ezerbot'], ['qa-redacteur', 'Demande de Rédacteur'],
    ['Émile', 'Demande d’Émile'], ['Hugo', 'Demande de Hugo'],
  ]) {
    const id = `branding-form-${requester}`;
    window.GrokbotChat.renderThread(formHost, {messages:[{id, kind:'form', role:'assistant', payload:{
      entry_id:id, route:{requesting_bot_id:requester}, status:{status:'pending'},
      kind:{kind:'form', fields:[{name:'city', label:'Votre ville', required:true, field_type:{kind:'text'}}]},
    }}]});
    await until(() => formHost.querySelector(`[data-message-id="${id}"]`));
    check(`${requester} form requester uses correct French elision`, formHost.innerText.includes(expected) && !formHost.innerText.includes('Demande de Ezerbot'));
  }
  window.GrokbotChat.renderThread(formHost, {messages:[]});
  await until(() => !formHost.querySelector('[data-card-kind]'));
  formHost.remove();
  // Composer bot menu: a 40-character name stays inside at both welcome sizes and themes.
  // Focus survives a refresh, arrows/Home/End move, and Escape returns to the trigger.
  const longName = 'ABCDEFGHIJKLMNOPQRSTUVWXYZABCDEFGHIJKLMN';
  const mediumName = 'Coach sportif du matin';
  await api('/api/bots', {id:'qa-long', name:longName, color:'cyan', shape:'galet', description:'', instructions:''});
  await api('/api/bots', {id:'qa-coach', name:mediumName, color:'orange', shape:'goutte', description:'', instructions:''});
  await window.GrokbotPanels.refresh();
  choose(longName);
  const menuFits = () => {
    const menu = $('bot-menu');
    const menuBox = menu.getBoundingClientRect();
    const list = menu.querySelector('.bot-menu-list');
    const buttons = [...menu.querySelectorAll('button')];
    const inside = buttons.every(button => {
      const box = button.getBoundingClientRect();
      const childrenInside = [...button.children].every(child => {
        const childBox = child.getBoundingClientRect();
        return childBox.right <= box.right + 0.5 && childBox.left >= box.left - 0.5;
      });
      const label = button.querySelector('.bot-menu-name, .bot-menu-action > span');
      const readable = Boolean(label && label.clientWidth > 12 && label.textContent.trim());
      const point = document.elementFromPoint(box.left + box.width / 2, box.top + box.height / 2);
      return box.width > 0 && box.right <= menuBox.right + 0.5 && box.left >= menuBox.left - 0.5
        && childrenInside && readable && (point === button || button.contains(point));
    });
    const checkedIcon = menu.querySelector('[aria-checked="true"] .icon')?.getBoundingClientRect();
    const iconInside = Boolean(checkedIcon && checkedIcon.width > 0 && checkedIcon.right <= menuBox.right + 0.5 && checkedIcon.left >= menuBox.left - 0.5);
    const actionText = menu.querySelector('.bot-menu-action span')?.textContent || '';
    const noHorizontalScroll = !list || list.scrollWidth <= list.clientWidth + 1;
    return inside && iconInside && noHorizontalScroll && menuBox.right <= innerWidth + 0.5
      && actionText.startsWith('Modifier ') && actionText.includes(longName);
  };
  // Largeur réelle du composeur : 720 px à 1140×780, 530 px à 820×560.
  // Le panneau a une bordure de 2 px. Entre 761 et 910, la zone retire 28 px, pas 44.
  const applyFrame = (width) => {
    let style = document.getElementById('qa-menu-frame');
    if (!style) { style = document.createElement('style'); style.id = 'qa-menu-frame'; document.head.append(style); }
    const sidebar = width <= 1100 ? 244 : 270;
    const main = width - sidebar - 16;
    const area = width > 1100 ? Math.min(720, main - 2 - 54) : width <= 910 && width >= 761 ? main - 2 - 28 : width <= 760 ? main - 44 : main - 2 - 44;
    style.textContent = `.composer-area{width:${area}px!important;max-width:none!important}`;
  };
  const setTheme = theme => { document.documentElement.dataset.theme = theme; };
  const paintedIcon = node => {
    if (!node) return false;
    const box = node.getBoundingClientRect();
    const style = getComputedStyle(node);
    return box.width >= 8 && box.height >= 8 && style.display !== 'none' && style.visibility !== 'hidden'
      && Number.parseFloat(style.opacity) > 0 && style.stroke !== 'none' && (Number.parseFloat(style.strokeWidth) || 0) > 0;
  };
  const openMenu = () => { if (!$('bot-menu').hidden) $('composer-bot').click(); $('composer-bot').click(); };
  const insideBot = (node) => {
    const bot = $('composer-bot').getBoundingClientRect();
    const box = node.getBoundingClientRect();
    return box.width > 0 && box.height > 0 && box.left >= bot.left - 0.5 && box.right <= bot.right + 0.5
      && box.top >= bot.top - 0.5 && box.bottom <= bot.bottom + 0.5;
  };
  const modelReadable = () => {
    const label = document.querySelector('[data-picker="model"] b');
    return !!label && label.getBoundingClientRect().width >= 40;
  };
  const clearOfControls = (label) => {
    const nameBox = label.getBoundingClientRect();
    const voice = document.querySelector('.voice-controls')?.getBoundingClientRect();
    const send = $('send-button').getBoundingClientRect();
    const tools = document.querySelector('.composer-tools')?.getBoundingClientRect();
    if (!voice || voice.width === 0) throw new Error('voice controls missing');
    const toolsClear = !tools || tools.width === 0 || tools.right <= voice.left + 0.5;
    return nameBox.width > 0 && nameBox.right <= voice.left + 0.5 && nameBox.right <= send.left + 0.5 && toolsClear;
  };
  const crowdTools = () => {
    const tools = $('composer-tools');
    tools.replaceChildren();
    for (const [label, value, picker] of [['Modèle', 'GPT-6 Sol', 'model'], ['Effort', 'Moyen', 'effort']]) {
      const button = document.createElement('button');
      button.type = 'button';
      button.className = 'tool-pill';
      button.dataset.picker = picker;
      const small = document.createElement('small');
      small.textContent = label;
      const name = document.createElement('b');
      name.textContent = value;
      const icon = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
      icon.setAttribute('class', 'icon');
      icon.setAttribute('aria-hidden', 'true');
      const use = document.createElementNS('http://www.w3.org/2000/svg', 'use');
      use.setAttribute('href', '#i-chevron');
      icon.append(use);
      button.append(small, name, icon);
      tools.append(button);
    }
  };
  window.__qaRunFrame = async (width, height) => {
    if (innerWidth !== width || innerHeight !== height) throw new Error(`viewport is ${innerWidth}×${innerHeight}, expected ${width}×${height}`);
    applyFrame(width);
    await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    for (const theme of ['dark', 'light']) {
      setTheme(theme);
      const plus = document.querySelector('#composer-model .attachment-choose:not(.meeting-start) .icon');
      const chevron = document.querySelector('.composer-chevron');
      check(`attach plus and composer chevron stay visible at ${width}×${height} ${theme}`, paintedIcon(plus) && paintedIcon(chevron));
      choose('Ezerbot');
      crowdTools();
      const ezerbotName = $('composer-bot-name');
      check(`composer shows the full Ezerbot name at ${width}×${height} ${theme}`, ezerbotName.textContent === 'Ezerbot' && ezerbotName.scrollWidth <= ezerbotName.clientWidth + 1 && clearOfControls(ezerbotName));
      check(`bot name and chevron stay inside the button at ${width}×${height} ${theme}`, insideBot(ezerbotName) && insideBot(document.querySelector('#composer-bot .composer-chevron')) && modelReadable());
      choose('Rédacteur');
      const writerName = $('composer-bot-name');
      check(`composer shows the full Rédacteur name at ${width}×${height} ${theme}`, writerName.textContent === 'Rédacteur' && writerName.scrollWidth <= writerName.clientWidth + 1 && clearOfControls(writerName));
      choose(mediumName);
      crowdTools();
      const mediumLabel = $('composer-bot-name');
      check(`composer shows the full medium name at ${width}×${height} ${theme}`, mediumLabel.textContent === mediumName && mediumLabel.scrollWidth <= mediumLabel.clientWidth + 1 && clearOfControls(mediumLabel));
      check(`crowded bot name, chevron and model stay usable at ${width}×${height} ${theme}`, insideBot(mediumLabel) && insideBot(document.querySelector('#composer-bot .composer-chevron')) && modelReadable());
      choose(longName);
      crowdTools();
      const longLabel = $('composer-bot-name');
      check(`maximal composer name stays left of the voice controls and send button at ${width}×${height} ${theme}`, longLabel.textContent === longName && clearOfControls(longLabel));
      check(`maximal name and chevron stay inside the bot button at ${width}×${height} ${theme}`, insideBot(longLabel) && insideBot(document.querySelector('#composer-bot .composer-chevron')) && modelReadable());
      check(`maximal composer name ellipsizes when the bar is full at ${width}×${height} ${theme}`, longLabel.scrollWidth > longLabel.clientWidth + 1);
      openMenu();
      check(`bot menu keeps a maximal name inside at ${width}×${height} ${theme}`, menuFits());
    }
  };
  window.__qaFinish = async () => {
  document.getElementById('qa-menu-frame')?.remove();
  setTheme('dark');
  openMenu();
  check('opened bot menu focuses the chosen bot', document.activeElement?.getAttribute('aria-checked') === 'true');
  await window.GrokbotPanels.refresh();
  check('bot menu refresh keeps focus on the same option', !$('bot-menu').hidden && document.activeElement?.getAttribute('aria-checked') === 'true');
  const menuItems = () => [...$('bot-menu').querySelectorAll('button')];
  const before = menuItems().indexOf(document.activeElement);
  const press = key => document.activeElement.dispatchEvent(new KeyboardEvent('keydown', {key, bubbles:true, cancelable:true}));
  press('ArrowDown');
  check('ArrowDown moves to the next menu item', menuItems().indexOf(document.activeElement) === (before + 1) % menuItems().length);
  press('ArrowUp');
  check('ArrowUp returns to the previous menu item', menuItems().indexOf(document.activeElement) === before);
  press('Home');
  check('Home moves to the first menu item', document.activeElement === menuItems()[0]);
  press('End');
  check('End moves to the last menu item', document.activeElement === menuItems().at(-1));
  press('Escape');
  check('Escape closes the bot menu and returns focus to its trigger', $('bot-menu').hidden && document.activeElement === $('composer-bot'));
  choose('Ezerbot');
  const first = await send('Qui me répond ?');
  check('R → Cmd+N → E sends to E', first.bot_id === 'grokbot' && $('workspace-label').textContent === 'Ezerbot');
  check('reply and conversation page title use Ezerbot', $('messages').innerText.includes('Ezerbot') && document.title.endsWith(' · Ezerbot'));
  await type('Brouillon du fil');
  cmdN(); await type('Brouillon accueil'); cmdN();
  check('repeated new-chat preserves the welcome draft', editor().getText() === 'Brouillon accueil');
  window.GrokbotPanels.openConversation(first.id);
  check('existing conversation draft is restored', editor().getText() === 'Brouillon du fil');
  cmdN();
  check('welcome draft is restored after leaving a thread', editor().getText() === 'Brouillon accueil');
  editor().commands.selectAll(); editor().commands.deleteSelection();
  choose('Rédacteur');
  check('opposite choice updates breadcrumb', chip() === 'Rédacteur' && $('workspace-label').textContent === 'Rédacteur');
  const second = await send('Réponse du rédacteur');
  check('E → new-chat → R sends to R', second.bot_id === 'qa-redacteur' && $('messages').innerText.includes('Rédacteur'));
  check('only sent conversations are persisted', (await saved()).conversations.length === 2);
  check('sent welcome draft is consumed', localStorage.getItem('grokbot.draft.new') === null);
  cmdN();
  check('new-chat restores an empty composer without warning content', empty());
  // Freeze creation to prove a mid-send chip click cannot switch the recipient.
  choose('Ezerbot'); await type('Choix figé pendant l’envoi');
  const originalFetch = window.fetch;
  let release;
  const gate = new Promise(resolve => { release = resolve; });
  window.fetch = async (...arguments_) => {
    if (arguments_[0] === '/api/conversations') await gate;
    return originalFetch(...arguments_);
  };
  $('composer-form').requestSubmit();
  check('submitted composer is cleared before awaiting conversation creation', empty());
  choose('Rédacteur');
  check('bot choice is frozen while send creates its conversation', chip() === 'Ezerbot' && $('workspace-label').textContent === 'Ezerbot');
  await type('Suite tapée pendant la création');
  release();
  await until(() => !$('new-chat').disabled);
  window.fetch = originalFetch;
  const inFlightConversation = (await saved()).conversations.find(item => item.id === localStorage.getItem('grokbot.selected'));
  check('in-flight send keeps the original recipient', inFlightConversation.bot_id === 'grokbot');
  check('text typed during creation remains a saved unsent draft', editor().getText() === 'Suite tapée pendant la création' && localStorage.getItem(`grokbot.draft.${inFlightConversation.id}`) === editor().getText() && localStorage.getItem('grokbot.draft.new') === null && localStorage.getItem('grokbot.draft.new.document') === null);
  check('in-flight edits are not appended to the submitted message', inFlightConversation.messages.some(item => item.role === 'user' && item.body === 'Choix figé pendant l’envoi'));
  cmdN();
  check('a draft transferred to its thread does not reappear in a new conversation', empty() && localStorage.getItem('grokbot.draft.new.document') === null);
  await type('Message à restaurer');
  const beforeFailure = (await saved()).conversations.length;
  let failCreation;
  const failureGate = new Promise(resolve => { failCreation = resolve; });
  window.fetch = async (...arguments_) => {
    if (arguments_[0] === '/api/conversations') {
      await failureGate;
      return new Response(JSON.stringify({error:'Création refusée pour le test'}), {status:503, headers:{'Content-Type':'application/json'}});
    }
    return originalFetch(...arguments_);
  };
  $('composer-form').requestSubmit();
  await type('Suite à conserver'); failCreation();
  await until(() => !$('new-chat').disabled);
  window.fetch = originalFetch;
  check('failed creation restores both submitted and subsequent draft text', editor().getText({blockSeparator:'\n'}) === 'Message à restaurer\nSuite à conserver' && localStorage.getItem('grokbot.draft.new') === 'Message à restaurer\nSuite à conserver');
  check('failed creation does not persist an empty thread', (await saved()).conversations.length === beforeFailure);
  editor().commands.selectAll(); editor().commands.deleteSelection();
  // Old versions could already have persisted an empty thread with a different bot.
  const legacy = (await api('/api/conversations', {bot_id:'qa-redacteur'})).conversation;
  await window.GrokbotPanels.refresh();
  window.GrokbotPanels.openConversation(legacy.id);
  check('legacy empty thread shows its actual bot', chip() === 'Rédacteur' && $('workspace-label').textContent === 'Rédacteur');
  await type('Brouillon ancien'); choose('Ezerbot');
  check('switching a legacy empty thread moves its draft to the chosen bot', localStorage.getItem('grokbot.selected') === '' && editor().getText() === 'Brouillon ancien' && chip() === 'Ezerbot' && $('workspace-label').textContent === 'Ezerbot');
  check('moving a legacy draft removes its old key', localStorage.getItem(`grokbot.draft.${legacy.id}`) === null);
  $('composer-form').requestSubmit();
  await until(() => !$('new-chat').disabled);
  check('legacy draft is sent to the newly chosen bot', (await saved()).conversations.find(item => item.id === localStorage.getItem('grokbot.selected')).bot_id === 'grokbot');
  $('confirm-delete').click();
  await until(() => !$('new-chat').disabled);
  check('deletion restores a valid empty composer', empty());
  // An empty legacy draft must never erase an existing welcome draft.
  await type('Brouillon accueil A');
  window.GrokbotPanels.openConversation(legacy.id);
  check('sent legacy text is not restored as a duplicate', empty());
  choose('Ezerbot');
  check('empty legacy draft preserves welcome text and storage', editor().getText() === 'Brouillon accueil A' && localStorage.getItem('grokbot.draft.new') === 'Brouillon accueil A');
  check('empty legacy draft has no residual storage key', localStorage.getItem(`grokbot.draft.${legacy.id}`) === null);
  // When both drafts contain text, keep both in order and consume the old key.
  window.GrokbotPanels.openConversation(legacy.id);
  await type('Brouillon ancien B'); choose('Ezerbot');
  const merged = 'Brouillon accueil A\nBrouillon ancien B';
  check('two nonempty drafts are preserved without replacement', editor().getText({blockSeparator:'\n'}) === merged && localStorage.getItem('grokbot.draft.new') === merged);
  check('merged legacy draft is moved rather than copied', localStorage.getItem(`grokbot.draft.${legacy.id}`) === null);
  $('composer-form').requestSubmit();
  await until(() => !$('new-chat').disabled);
  const mergedConversation = (await saved()).conversations.find(item => item.id === localStorage.getItem('grokbot.selected'));
  check('merged text is sent exactly once to the chosen bot', mergedConversation.bot_id === 'grokbot' && mergedConversation.messages.filter(item => item.role === 'user' && item.body === merged).length === 1);
  window.GrokbotPanels.openConversation(legacy.id);
  check('reopening legacy thread after send cannot resend the moved draft', empty() && $('send-button').disabled);
  // Creating a bot from an old empty thread follows the same safe draft transition.
  cmdN(); await type('Brouillon accueil C');
  window.GrokbotPanels.openConversation(legacy.id); await type('Brouillon ancien D');
  const beforeCreate = (await saved()).conversations.length;
  document.querySelector('.add-bot').click();
  $('bot-name').value = 'Traducteur';
  $('bot-description').value = 'Traduit vos textes.';
  $('bot-instructions').value = 'Traduis en français.';
  $('bot-form').requestSubmit();
  await until(() => !$('bot-dialog').open);
  const newBot = window.GrokbotPanels.state.bots.find(bot => bot.name === 'Traducteur');
  const createdDraft = 'Brouillon accueil C\nBrouillon ancien D';
  check('new bot is selected from a legacy empty thread', newBot && chip() === 'Traducteur' && $('workspace-label').textContent === 'Traducteur' && localStorage.getItem('grokbot.selected') === '' && localStorage.getItem('grokbot.bot') === newBot.id);
  check('new-bot creation preserves both drafts and clears the old key', editor().getText({blockSeparator:'\n'}) === createdDraft && localStorage.getItem('grokbot.draft.new') === createdDraft && localStorage.getItem(`grokbot.draft.${legacy.id}`) === null);
  check('new-bot creation does not persist another empty conversation', (await saved()).conversations.length === beforeCreate);
  $('composer-form').requestSubmit();
  await until(() => !$('new-chat').disabled);
  const newBotConversation = (await saved()).conversations.find(item => item.id === localStorage.getItem('grokbot.selected'));
  check('created bot receives the preserved draft', newBotConversation.bot_id === newBot.id && newBotConversation.messages.some(item => item.role === 'user' && item.body === createdDraft));
  // Real image input: the attachment panel owns staging and binds images to a thread.
  const stageImage = async () => {
    const canvas = document.createElement('canvas'); canvas.width = 4; canvas.height = 4;
    const context = canvas.getContext('2d'); context.fillStyle = '#639'; context.fillRect(0, 0, 4, 4);
    const image = await new Promise(resolve => canvas.toBlob(resolve, 'image/png'));
    const transfer = new DataTransfer(); transfer.items.add(new File([image], 'photo-regression.png', {type:'image/png'}));
    const picker = document.querySelector('.attachment-picker'); picker.files = transfer.files;
    picker.dispatchEvent(new Event('change', {bubbles:true}));
  };
  cmdN(); choose('Ezerbot'); await type('Décris cette photo');
  const beforeImage = (await saved()).conversations.length;
  let allowUpload;
  const uploadGate = new Promise(resolve => { allowUpload = resolve; });
  window.fetch = async (...arguments_) => {
    if (arguments_[0] === '/api/attachments') await uploadGate;
    return originalFetch(...arguments_);
  };
  await stageImage(); choose('Rédacteur');
  check('pending image upload already pins the displayed bot', chip() === 'Ezerbot' && $('workspace-label').textContent === 'Ezerbot' && Boolean(document.querySelector('.attachment-chip')));
  check('blocked switch explains how to retain or remove the image', $('toast').textContent.includes('Retirez les pièces jointes') && $('toast').textContent.includes('Ezerbot'));
  allowUpload();
  await until(() => localStorage.getItem('grokbot.selected') && document.querySelector('.attachment-chip')?.dataset.status !== 'pending');
  window.fetch = originalFetch;
  const imageConversationId = localStorage.getItem('grokbot.selected');
  choose('Rédacteur');
  check('ready staged image cannot be abandoned by a bot chip', chip() === 'Ezerbot' && $('workspace-label').textContent === 'Ezerbot' && localStorage.getItem('grokbot.bot') === 'grokbot' && localStorage.getItem('grokbot.selected') === imageConversationId && document.querySelectorAll('.attachment-chip').length === 1 && editor().getText() === 'Décris cette photo');
  document.querySelector('.add-bot').click(); $('bot-name').value = 'Bot image'; $('bot-form').requestSubmit();
  await until(() => !$('bot-dialog').open);
  check('creating a bot also retains staged image and current recipient', chip() === 'Ezerbot' && $('workspace-label').textContent === 'Ezerbot' && localStorage.getItem('grokbot.bot') === 'grokbot' && localStorage.getItem('grokbot.selected') === imageConversationId && document.querySelectorAll('.attachment-chip').length === 1);
  check('new-bot toast explains why image recipient is retained', $('toast').textContent.includes('Bot image est prêt') && $('toast').textContent.includes('Retirez les pièces jointes'));
  $('composer-form').requestSubmit();
  await until(() => !$('new-chat').disabled);
  const imageState = await saved();
  const imageConversation = imageState.conversations.find(item => item.id === imageConversationId);
  check('image is sent to the bot displayed on chip and breadcrumb', imageConversation.bot_id === 'grokbot' && imageConversation.messages.some(item => item.role === 'user' && item.body === 'Décris cette photo' && item.attachments?.length === 1));
  check('sending staged image reuses its thread without leaving another empty one', imageState.conversations.length === beforeImage + 1 && imageState.conversations.filter(item => !item.messages.length).length === 1);
  check('accepted image is removed from the composer', !document.querySelector('.attachment-chip'));
  cmdN(); choose('Bot image');
  check('newly created bot can be chosen after image is sent', chip() === 'Bot image' && $('workspace-label').textContent === 'Bot image');
  await stageImage();
  await until(() => localStorage.getItem('grokbot.selected') && document.querySelector('.attachment-chip')?.dataset.status !== 'pending');
  document.querySelector('.attachment-remove').click(); choose('Rédacteur');
  check('explicit image removal permits changing the bot', !document.querySelector('.attachment-chip') && chip() === 'Rédacteur' && $('workspace-label').textContent === 'Rédacteur');
  return {scope:'Real app.js, bundled Tiptap editor and isolated simulated wrapper', checks};
  };
  return true;
}''')
                for width, height in ((1140, 780), (820, 560)):
                    chrome('resize', str(width), str(height))
                    frame = chrome('eval', f'async () => {{ await window.__qaRunFrame({width}, {height}); return true; }}')
                    if not re.search(r'^result: ', frame, re.MULTILINE):
                        raise AssertionError(frame)
                result = chrome('eval', 'async () => window.__qaFinish()', '--full')
                value = re.search(r'^result: (.+)$', result, re.MULTILINE)
                if not value:
                    raise AssertionError(result)
                ui_report = json.loads(value.group(1))
                if isinstance(ui_report, str):
                    ui_report = json.loads(ui_report)
                console = chrome('console')
                if '[tiptap warn]' in console or 'Unknown node type' in console:
                    raise AssertionError(console)
                ui_report['checks'].append({'name': 'rebuilt wrapper embeds the current worktree app.js', 'passed': True})
                ui_report['checks'].append({'name': 'no Tiptap invalid-content warning across startup, navigation, send and deletion', 'passed': True})
                ui_report['status'] = 'passed'
                if args.report:
                    args.report.write_text(json.dumps(ui_report, ensure_ascii=False, indent=2) + '\n')
                print(json.dumps(ui_report, ensure_ascii=False, indent=2))
            finally:
                if authentication:
                    authentication.close()
                if browser:
                    try:
                        subprocess.run(['chrome-devtools-axi', 'stop'], env=env, timeout=30,
                                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
                    except (OSError, subprocess.TimeoutExpired):
                        pass
                    finally:
                        if browser.poll() is None:
                            browser.terminate()
                            try:
                                browser.wait(timeout=5)
                            except subprocess.TimeoutExpired:
                                browser.kill()
                                browser.wait(timeout=5)
                if child.poll() is None:
                    child.terminate()
                    try:
                        child.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        child.kill()
                        child.wait(timeout=5)
                child.stdout.close()



if '--ui-regressions' in sys.argv:
    verify_ui_regressions()
    raise SystemExit(0)


parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--app', type=Path, default=APP, help='Bundle to launch and inspect in a private HOME')
parser.add_argument('--report', type=Path, default=ROOT / 'evidence/wrapper-native-verification.json')
args = parser.parse_args()
APP = args.app.resolve()


def run(arguments):
    return subprocess.check_output(arguments, text=True, timeout=30).strip()


def process_command(pid):
    return run(['/bin/ps', '-p', str(pid), '-o', 'command='])


def require(name, condition):
    report['checks'].append({'name': name, 'passed': bool(condition)})
    if not condition:
        raise AssertionError(name)


def runtime_snapshot(inspected):
    owner = inspected.pid
    host = str(APP / 'Contents/MacOS/Ezerbot')
    engine = str(APP / 'Contents/MacOS/grokbot-wrapper')
    require('inspected bundle process remains alive', inspected.process.poll() is None)
    require('launched process runs this bundle inspection command',
            process_command(owner) == host + ' --inspect-private')
    require('running process is named Ezerbot', run(['/bin/ps', '-p', str(owner), '-o', 'ucomm=']) == 'Ezerbot')
    children = run(['/usr/bin/pgrep', '-P', str(owner)]).splitlines()
    owned = [int(pid) for pid in children if process_command(pid).startswith(engine + ' ')]
    require('native host owns one Rust engine', len(owned) == 1)
    engine_pid = owned[0]
    require('inspected engine is the direct native child',
            run(['/bin/ps', '-p', str(engine_pid), '-o', 'ppid=']) == str(owner))
    sockets = run(['/usr/sbin/lsof', '-nP', '-a', '-p', str(engine_pid), '-iTCP', '-sTCP:LISTEN', '-Fn'])
    listeners = [line[1:] for line in sockets.splitlines() if line.startswith('n')]
    require('engine listens only on IPv4 loopback', len(listeners) == 1
            and re.fullmatch(r'127\.0\.0\.1:[1-9][0-9]{0,4}', listeners[0]))
    runtime = {'host_pid': owner, 'engine_pid': engine_pid, 'url': 'http://' + listeners[0]}
    require('private readiness matches the OS engine and listener', inspected.ready == {'ready': True, **runtime})
    return runtime


report = {
    'scope': 'Built native host, owned Rust child, loopback assets and window metadata; no model or native export interaction',
    'verified_at_utc': datetime.now(timezone.utc).isoformat(),
    'checks': [],
}
try:
    with (APP / 'Contents/Info.plist').open('rb') as file:
        info = plistlib.load(file)
    require('obsolete generated Grokbot bundle is absent', not (ROOT / 'app/build/Grokbot.app').exists())
    require('bundle and About display Ezerbot', info['CFBundleName'] == info['CFBundleDisplayName'] == 'Ezerbot')
    require('bundle identity stays compatible', info['CFBundleIdentifier'] == 'local.grokbot.workspace')
    require('native executable is Ezerbot', info['CFBundleExecutable'] == 'Ezerbot')
    require('system-added menus use French', info.get('CFBundleDevelopmentRegion') == 'fr')
    require('icon name stays compatible', info['CFBundleIconFile'] == 'Grokbot')
    require('obsolete native executable is absent', not (APP / 'Contents/MacOS/Grokbot').exists())
    require('minimum macOS version supports dialog and UUID', info['LSMinimumSystemVersion'] == '12.3')
    run(['/usr/bin/codesign', '--verify', '--deep', '--strict', str(APP)])
    require('bundle signature verifies', True)
    args.report.parent.mkdir(parents=True, exist_ok=True)
    with BundleInspection(APP, args.report.parent) as inspected:
        runtime = runtime_snapshot(inspected)
        owner = runtime['host_pid']
        engine_pid = runtime['engine_pid']
        url = runtime['url']
        report['runtime'] = runtime.copy()
        for endpoint, source in [('/', 'index.html'), ('/styles.css', 'styles.css'), ('/app.js', 'app.js'),
                             ('/mascot.css', 'mascot.css'), ('/mascot.js', 'mascot.js'),
                             ('/chat.css', 'chat.css'), ('/chat.js', 'chat.js')]:
            with urllib.request.urlopen(url + endpoint, timeout=10) as response:
                actual = response.read()
            require(f'bundled {source} matches current source', actual == (ROOT / 'app/ui' / source).read_bytes())
        for extension in ['js', 'css']:
            expected = b''.join((b';(function () {\n' + path.read_bytes() + b'\n})();\n')
                            if extension == 'js' else path.read_bytes() + b'\n'
                            for path in sorted((ROOT / 'app/ui/panels').glob(f'*.{extension}'))
                            if path.is_file())
            with urllib.request.urlopen(url + f'/panels.{extension}', timeout=10) as response:
                actual = response.read()
            require(f'bundled panels.{extension} matches current sources', actual == expected)
        try:
            with urllib.request.urlopen(url + '/api/state', timeout=10):
                refused = False
        except urllib.error.HTTPError as error:
            refused = error.code == 403
        require('native launch identity stays private to the native host', refused)
        require('bundle runtime remains unchanged before authenticated state', runtime_snapshot(inspected) == runtime)
        state = inspected.state(engine_pid, url)
        require('inspected bundle authenticates its own engine state', state['status'] == 200)
        require('inspected bundle engine advertises a known mode', state['mode'] in ('simulated', 'chatgpt'))
        require('inspected bundle keeps its launch identity out of its journals', state['journals_safe'] is True)
        require('bundle runtime remains unchanged after authenticated state', runtime_snapshot(inspected) == runtime)
        report['runtime']['authenticated_state'] = state

        # Query only this application's window metadata, without screen capture or Accessibility.
        source = r'''#import <Cocoa/Cocoa.h>
#import <ApplicationServices/ApplicationServices.h>
int main(int argc, const char **argv) {
    @autoreleasepool {
        if (argc != 2) return 2;
        pid_t owner = (pid_t)atoi(argv[1]);
        NSArray *windows = CFBridgingRelease(CGWindowListCopyWindowInfo(kCGWindowListOptionAll, kCGNullWindowID));
        NSMutableArray *matching = [NSMutableArray array];
        for (NSDictionary *window in windows) {
            if ([window[(id)kCGWindowOwnerPID] intValue] != owner || [window[(id)kCGWindowLayer] intValue] != 0) continue;
            [matching addObject:@{@"on_screen": window[(id)kCGWindowIsOnscreen] ?: @NO,
                                  @"bounds": window[(id)kCGWindowBounds] ?: @{}}];
        }
        NSData *data = [NSJSONSerialization dataWithJSONObject:matching options:0 error:nil];
        fwrite(data.bytes, 1, data.length, stdout);
    }
    return 0;
}
'''
        with tempfile.TemporaryDirectory(prefix='grokbot-window-qa-', dir=args.report.parent) as directory:
            helper = Path(directory) / 'windows.m'
            binary = Path(directory) / 'windows'
            helper.write_text(source)
            run(['/usr/bin/clang', '-fobjc-arc', '-Wall', '-Wextra', '-Werror', str(helper),
             '-framework', 'Cocoa', '-framework', 'ApplicationServices', '-o', str(binary)])
            windows = json.loads(run([str(binary), str(owner)]))
        report['runtime']['windows'] = windows
        require('native window exists at supported size', any(window['bounds'].get('Width', 0) >= 820
                                                        and window['bounds'].get('Height', 0) >= 560 for window in windows))
    require('inspected bundle exits normally after private EOF', inspected.process.returncode == 0)
    require('isolated inspection HOME is removed', not inspected.home.exists())
    stopped = subprocess.run(['/bin/ps', '-p', str(engine_pid), '-o', 'pid='], text=True,
                             stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, timeout=5)
    require('owned Rust engine exits after private EOF', stopped.returncode == 1 and not stopped.stdout.strip())
    tracked = [APP / 'Contents/MacOS/Ezerbot', APP / 'Contents/MacOS/grokbot-wrapper',
               APP / 'Contents/Info.plist', ROOT / 'app/native/main.m', ROOT / 'app/native/inspection.h',
               ROOT / 'tools/build-app.sh', ROOT / 'tools/bundle_inspection.py', ROOT / 'tools/verify-app.py']
    report['sha256'] = {os.path.relpath(path, ROOT): hashlib.sha256(path.read_bytes()).hexdigest() for path in tracked}
    report['status'] = 'passed'
except Exception as error:
    report['status'] = 'failed'
    report['error'] = str(error) if isinstance(error, (AssertionError, BundleInspectionError)) else 'Bundle verification failed'
args.report.parent.mkdir(parents=True, exist_ok=True)
args.report.write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
print(json.dumps(report, ensure_ascii=False, indent=2))
raise SystemExit(0 if report['status'] == 'passed' else 1)
