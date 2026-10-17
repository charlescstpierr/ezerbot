#!/usr/bin/env python3
"""Inspect the built, running Mac app without changing its workspace."""
from datetime import datetime, timezone
from pathlib import Path
import hashlib
import json
import plistlib
import re
import subprocess
import tempfile
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
APP = ROOT / 'app/build/Grokbot.app'


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
    for extension, separator in [('js', b'\n;\n'), ('css', b'\n')]:
        expected = b''.join(path.read_bytes() + separator
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
