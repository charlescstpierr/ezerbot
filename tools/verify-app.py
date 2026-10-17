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

ROOT = Path(__file__).resolve().parents[1]
APP = ROOT / 'app/build/Ezerbot.app'


def verify_ui_regressions():
    """Drive the real shell/editor against a disposable, simulated Rust wrapper."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--ui-regressions', action='store_true')
    parser.add_argument('--report', type=Path, help='Optional report path (otherwise stdout only)')
    args = parser.parse_args()
    env = dict(os.environ, CHROME_DEVTOOLS_AXI_SESSION=os.environ.get('CHROME_DEVTOOLS_AXI_SESSION', 'grokbot-ui-regressions'))
    # Assets are embedded with include_str!: always rebuild with the parent HOME/toolchain.
    subprocess.run([str(Path.home() / '.cargo/bin/cargo'), 'build', '--locked', '-p', 'grokbot-wrapper'],
                   cwd=ROOT, check=True)

    def chrome(*arguments):
        result = subprocess.run(['chrome-devtools-axi', *arguments], text=True, env=env, timeout=60,
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        if result.returncode or re.search(r'^error:', result.stdout, re.MULTILINE):
            raise AssertionError(result.stdout)
        return result.stdout

    with tempfile.TemporaryDirectory(prefix='grokbot-newchat-qa-') as directory:
        home = Path(directory)
        child_env = dict(os.environ, HOME=directory, CFFIXED_USER_HOME=directory)
        # No account/token is copied, and no real provider endpoint is used.
        child_env.pop('GROKBOT_FAKE_OPENAI', None)
        with (home / 'wrapper.log').open('w+') as log:
            child = subprocess.Popen([str(ROOT / 'target/debug/grokbot-wrapper'), '--port', '0',
                                      '--db', str(home / 'workspace.sqlite')], cwd=ROOT, env=child_env,
                                     stdout=subprocess.PIPE, stderr=log, text=True)
            page_id = None
            previous_page_id = None
            try:
                with selectors.DefaultSelector() as selector:
                    selector.register(child.stdout, selectors.EVENT_READ)
                    if not selector.select(15):
                        raise TimeoutError('Isolated wrapper did not announce its URL')
                url = json.loads(child.stdout.readline())['url']
                if not re.fullmatch(r'http://127\.0\.0\.1:\d+', url):
                    raise AssertionError('Wrapper must bind to IPv4 loopback')
                with urllib.request.urlopen(url + '/app.js', timeout=10) as response:
                    if response.read() != (ROOT / 'app/ui/app.js').read_bytes():
                        raise AssertionError('Wrapper embeds a different app.js than the current worktree')
                pages = chrome('pages')
                previous = re.search(r'^\s*(\d+),[^\n]*,true\s*$', pages, re.MULTILINE)
                if not previous:
                    previous = re.search(r'^\s*(\d+),', pages, re.MULTILINE)
                    if previous:
                        chrome('selectpage', previous.group(1))
                previous_page_id = previous.group(1) if previous else None
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
  const choose = name => [...document.querySelectorAll('.bot-chip')].find(node => node.textContent.trim() === name).click();
  const chip = () => document.querySelector('.bot-chip[aria-pressed=true]').textContent.trim();
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
  check('text typed during creation remains a saved unsent draft', editor().getText() === 'Suite tapée pendant la création' && localStorage.getItem(`grokbot.draft.${inFlightConversation.id}`) === editor().getText() && localStorage.getItem('grokbot.draft.new') === null);
  check('in-flight edits are not appended to the submitted message', inFlightConversation.messages.some(item => item.role === 'user' && item.body === 'Choix figé pendant l’envoi'));
  cmdN(); await type('Message à restaurer');
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
  check('blocked switch explains how to retain or remove the image', $('toast').textContent.includes('Retirez l’image jointe') && $('toast').textContent.includes('Ezerbot'));
  allowUpload();
  await until(() => localStorage.getItem('grokbot.selected') && document.querySelector('.attachment-chip')?.dataset.status !== 'pending');
  window.fetch = originalFetch;
  const imageConversationId = localStorage.getItem('grokbot.selected');
  choose('Rédacteur');
  check('ready staged image cannot be abandoned by a bot chip', chip() === 'Ezerbot' && $('workspace-label').textContent === 'Ezerbot' && localStorage.getItem('grokbot.bot') === 'grokbot' && localStorage.getItem('grokbot.selected') === imageConversationId && document.querySelectorAll('.attachment-chip').length === 1 && editor().getText() === 'Décris cette photo');
  document.querySelector('.add-bot').click(); $('bot-name').value = 'Bot image'; $('bot-form').requestSubmit();
  await until(() => !$('bot-dialog').open);
  check('creating a bot also retains staged image and current recipient', chip() === 'Ezerbot' && $('workspace-label').textContent === 'Ezerbot' && localStorage.getItem('grokbot.bot') === 'grokbot' && localStorage.getItem('grokbot.selected') === imageConversationId && document.querySelectorAll('.attachment-chip').length === 1);
  check('new-bot toast explains why image recipient is retained', $('toast').textContent.includes('Bot image est prêt') && $('toast').textContent.includes('Retirez l’image jointe'));
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
}''')
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
                child.terminate()
                try:
                    child.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait(timeout=5)
                child.stdout.close()
                if page_id:
                    subprocess.run(['chrome-devtools-axi', 'closepage', page_id], env=env, timeout=30,
                                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
                if previous_page_id:
                    subprocess.run(['chrome-devtools-axi', 'selectpage', previous_page_id], env=env, timeout=30,
                                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)


if '--ui-regressions' in sys.argv:
    verify_ui_regressions()
    raise SystemExit(0)


def run(arguments):
    return subprocess.check_output(arguments, text=True, timeout=30).strip()


def process_command(pid):
    return run(['/bin/ps', '-p', str(pid), '-o', 'command='])


def require(name, condition):
    report['checks'].append({'name': name, 'passed': bool(condition)})
    if not condition:
        raise AssertionError(name)


report = {
    'scope': 'Built native host, owned Rust child, loopback assets and window metadata; no model or native export interaction',
    'verified_at_utc': datetime.now(timezone.utc).isoformat(),
    'checks': [],
}
try:
    with (APP / 'Contents/Info.plist').open('rb') as file:
        info = plistlib.load(file)
    require('bundle and About display Ezerbot', info['CFBundleName'] == info['CFBundleDisplayName'] == 'Ezerbot')
    require('bundle identity and executable stay compatible', info['CFBundleIdentifier'] == 'local.grokbot.workspace' and info['CFBundleExecutable'] == 'Grokbot')
    require('minimum macOS version supports dialog and UUID', info['LSMinimumSystemVersion'] == '12.3')
    run(['/usr/bin/codesign', '--verify', '--deep', '--strict', str(APP)])
    require('bundle signature verifies', True)
    host = str(APP / 'Contents/MacOS/Grokbot')
    candidates = run(['/usr/bin/pgrep', '-x', 'Grokbot']).splitlines()
    host_commands = {host, str(Path(host).relative_to(ROOT))}
    owners = [int(pid) for pid in candidates if process_command(pid) in host_commands]
    require('this bundle is running', len(owners) == 1)
    owner = owners[0]
    children = run(['/usr/bin/pgrep', '-P', str(owner)]).splitlines()
    engine = str(APP / 'Contents/MacOS/grokbot-wrapper')
    owned = [int(pid) for pid in children if process_command(pid).startswith(engine + ' ')]
    require('native host owns one Rust engine', len(owned) == 1)
    engine_pid = owned[0]
    sockets = run(['/usr/sbin/lsof', '-nP', '-a', '-p', str(engine_pid), '-iTCP', '-sTCP:LISTEN', '-Fn'])
    listeners = [line[1:] for line in sockets.splitlines() if line.startswith('n')]
    require('engine listens only on IPv4 loopback', len(listeners) == 1 and re.fullmatch(r'127\.0\.0\.1:\d+', listeners[0]))
    url = 'http://' + listeners[0]
    report['runtime'] = {'host_pid': owner, 'engine_pid': engine_pid, 'url': url}
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
    with urllib.request.urlopen(url + '/api/state', timeout=10) as response:
        state = json.load(response)
    # Le mode suit la connexion de l'utilisateur : simulé sans compte, ChatGPT une fois connecté.
    require('running engine advertises a known mode', state['mode'] in ('simulated', 'chatgpt'))

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
    with tempfile.TemporaryDirectory(prefix='grokbot-window-qa-') as directory:
        helper = Path(directory) / 'windows.m'
        binary = Path(directory) / 'windows'
        helper.write_text(source)
        run(['/usr/bin/clang', '-fobjc-arc', '-Wall', '-Wextra', '-Werror', str(helper),
             '-framework', 'Cocoa', '-framework', 'ApplicationServices', '-o', str(binary)])
        windows = json.loads(run([str(binary), str(owner)]))
    report['runtime']['windows'] = windows
    require('native window exists at supported size', any(window['bounds'].get('Width', 0) >= 820
                                                        and window['bounds'].get('Height', 0) >= 560 for window in windows))
    tracked = [APP / 'Contents/MacOS/Grokbot', APP / 'Contents/MacOS/grokbot-wrapper',
               APP / 'Contents/Info.plist', ROOT / 'app/native/main.m', ROOT / 'tools/build-app.sh']
    report['sha256'] = {str(path.relative_to(ROOT)): hashlib.sha256(path.read_bytes()).hexdigest() for path in tracked}
    report['status'] = 'passed'
except Exception as error:
    report['status'] = 'failed'
    report['error'] = str(error)
(ROOT / 'evidence/wrapper-native-verification.json').write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
print(json.dumps(report, ensure_ascii=False, indent=2))
raise SystemExit(0 if report['status'] == 'passed' else 1)
