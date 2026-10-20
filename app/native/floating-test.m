// Debug-only Cocoa integration harness: no .app registration, event tap, real
// global shortcut, Accessibility request or Input Monitoring request.
#define main EzerbotApplicationMain
#import "main.m"
#undef main

static void EZWait(BOOL (^ready)(void)) {
    NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:25];
    while (!ready() && deadline.timeIntervalSinceNow > 0) [[NSRunLoop currentRunLoop] runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.02]];
    NSCAssert(ready(), @"Harness timed out");
}
static id EZJavaScript(WKWebView *view, NSString *script) {
    __block BOOL finished = NO; __block id result; __block NSError *failure;
    [view evaluateJavaScript:script completionHandler:^(id value, NSError *error) { result = value; failure = error; finished = YES; }];
    EZWait(^BOOL { return finished; });
    NSCAssert(!failure, @"JavaScript failed: %@", failure);
    return result;
}
static void EZShot(WKWebView *view, NSString *path) {
    __block BOOL finished = NO;
    [view takeSnapshotWithConfiguration:nil completionHandler:^(NSImage *image, NSError *error) {
        NSCAssert(image && !error, @"Snapshot failed: %@",error);
        NSBitmapImageRep *bitmap = [NSBitmapImageRep imageRepWithData:image.TIFFRepresentation];
        NSCAssert([[bitmap representationUsingType:NSBitmapImageFileTypePNG properties:@{}] writeToFile:path atomically:YES], @"Snapshot write failed");
        finished = YES;
    }];
    EZWait(^BOOL { return finished; });
}

int main(int argc, const char *argv[]) {
    @autoreleasepool {
        NSCAssert(argc == 3, @"Usage: floating-test http://127.0.0.1:port task-output-directory (isolated HOME required)");
        NSURL *url = [NSURL URLWithString:[NSString stringWithUTF8String:argv[1]]];
        NSString *output = [NSString stringWithUTF8String:argv[2]];
        NSCAssert([url.host isEqual:@"127.0.0.1"] && [url.scheme isEqual:@"http"] && url.port && !url.user && !url.password && !url.query && !url.fragment, @"Loopback only");
        NSCAssert([NSProcessInfo.processInfo.environment[@"HOME"] hasPrefix:output], @"Use a throwaway HOME inside the task output directory");
#if defined(EZERBOT_RELEASE)
#error The floating panel harness must never compile the global hotkey registration path.
#endif
        umask(0077);
        [NSApplication sharedApplication];
        [NSApp setActivationPolicy:NSApplicationActivationPolicyAccessory];
        GrokbotApplication *shell = [[GrokbotApplication alloc] init];
        shell.window = [[NSWindow alloc] initWithContentRect:NSMakeRect(0,0,1140,780) styleMask:NSWindowStyleMaskTitled | NSWindowStyleMaskResizable backing:NSBackingStoreBuffered defer:NO];
        shell.window.releasedWhenClosed = NO;
        shell.window.delegate = shell;
        shell.supportPath = NSProcessInfo.processInfo.environment[@"HOME"];
        shell.floating = [[EzerbotFloating alloc] init];
        [shell createMenus]; [shell openWorkspace:url];
        EZWait(^BOOL { return !shell.webView.loading && shell.webView.URL != nil; });
        EZJavaScript(shell.webView,@"document.documentElement.dataset.theme='dark'");
        // Wait for both workspace restoration and the shipping panel extension.
        __block BOOL restored = NO;
        for (int i=0; i<200 && !restored; i++) {
            restored = [EZJavaScript(shell.webView,@"Boolean(window.GrokbotPanels?.state?.floating && window.EzerbotFloatingMain)") boolValue];
            if (!restored) [[NSRunLoop currentRunLoop] runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.05]];
        }
        NSCAssert(restored,@"Main workspace restored");
        NSCAssert([EZJavaScript(shell.webView,@"window.GrokbotPanels.openConversation(window.GrokbotPanels.state.conversations.find(c=>c.kind==='chat').id)") boolValue], @"Seeded main conversation selected");
        NSDictionary *recorded = [shell.floating configure:@"cmd+option+space"];
        NSCAssert([recorded[@"status"] isEqual:@"recording"], @"No real shortcut registered");
        NSCAssert([[shell.floating configure:@"capture-all"][@"status"] isEqual:@"error"], @"Invalid shortcut rejected");
        [shell.window orderOut:nil];
        // The menu action is exactly the same entry point used by the global
        // shortcut in release, without installing any global input handler.
        NSMenuItem *menuItem = shell.statusItem.menu.itemArray.firstObject;
        [NSApp sendAction:menuItem.action to:menuItem.target from:menuItem];
        EzerbotFloating *floating = shell.floating;
        EZWait(^BOOL { return floating.webView.URL != nil && !floating.webView.loading; });
        NSCAssert([floating.panel isKindOfClass:NSPanel.class] && floating.panel.level == NSFloatingWindowLevel, @"Real always-on-top NSPanel");
        NSCAssert(!floating.panel.hidesOnDeactivate && !shell.window.visible, @"Works while main is hidden");
        NSCAssert((floating.panel.collectionBehavior & NSWindowCollectionBehaviorCanJoinAllSpaces) != 0, @"All workspaces");
        NSCAssert([floating isPanelURL:floating.webView.URL] && ![floating isPanelURL:[url URLByAppendingPathComponent:@"app.js"]] && ![floating isPanelURL:[NSURL URLWithString:@"https://example.invalid/floating"]], @"Separate panel trust boundary");
        EZWait(^BOOL { return [EZJavaScript(floating.webView,@"document.querySelectorAll('#floating-target option').length > 1") boolValue]; });
        EZJavaScript(floating.webView,@"window.__denied='pending'; window.webkit.messageHandlers.ezerbotFloating.postMessage({command:'configure',shortcut:'cmd+option+j'}).then(()=>window.__denied='unsafe',()=>window.__denied='denied'); true");
        EZWait(^BOOL { return ![EZJavaScript(floating.webView,@"window.__denied") isEqual:@"pending"]; });
        NSCAssert([EZJavaScript(floating.webView,@"window.__denied") isEqual:@"denied"], @"Panel cannot configure a global shortcut");
        EZJavaScript(floating.webView,@"document.querySelector('textarea').focus(); document.execCommand('insertText',false,'Message du panneau Cocoa, fenêtre principale masquée.'); true");
        EZWait(^BOOL { return [EZJavaScript(floating.webView,@"!document.querySelector('.floating-composer button').disabled") boolValue]; });
        EZJavaScript(floating.webView,@"document.querySelector('.floating-composer').requestSubmit(); true");
        EZWait(^BOOL { return [EZJavaScript(floating.webView,@"document.querySelector('.floating-question')?.textContent.includes('panneau Cocoa') && !document.querySelector('textarea').value") boolValue]; });
        NSCAssert(!shell.window.visible,@"Native send leaves main hidden");
        EZWait(^BOOL { return [EZJavaScript(shell.webView,@"document.querySelector('#messages').textContent.includes('panneau Cocoa')") boolValue]; });
        EZJavaScript(floating.webView,@"document.documentElement.dataset.theme='dark'");
        EZShot(floating.webView,[output stringByAppendingPathComponent:@"shots/native-panel-dark.png"]);
        NSRect moved = [floating visibleFrame:NSMakeRect(140,150,420,470)];
        [floating.panel setFrame:moved display:YES]; [floating savePosition];
        [floating.panel cancelOperation:nil];
        NSCAssert(!floating.panel.visible, @"Escape closes the panel");
        [floating show];
        NSCAssert(NSEqualPoints(floating.panel.frame.origin,moved.origin), @"Reopen remembers position");
        [floating invalidate];
        [floating show];
        EZWait(^BOOL { return floating.webView.URL != nil && !floating.webView.loading; });
        NSCAssert(NSEqualPoints(floating.panel.frame.origin,moved.origin), @"Position survives panel recreation");
        EZWait(^BOOL { return [EZJavaScript(floating.webView,@"document.querySelectorAll('#floating-target option').length > 1") boolValue]; });
        EZJavaScript(floating.webView,@"document.documentElement.dataset.theme='light'");
        EZShot(floating.webView,[output stringByAppendingPathComponent:@"shots/native-panel-light.png"]);
        [floating.panel setFrame:NSMakeRect(moved.origin.x,moved.origin.y,360,440) display:YES];
        NSCAssert([EZJavaScript(floating.webView,@"document.querySelector('footer').getBoundingClientRect().bottom <= innerHeight + 1 && document.body.scrollWidth <= innerWidth") boolValue], @"Composer and full-conversation action fit at the minimum native size");
        EZShot(floating.webView,[output stringByAppendingPathComponent:@"shots/native-panel-minimum.png"]);
        // The existing seeded conversation is selected from the actual dropdown.
        id target = EZJavaScript(floating.webView,@"document.querySelector('#floating-target option[value]:not([value=\"\"])')?.value || ''");
        NSCAssert([target isKindOfClass:NSString.class] && [target length], @"Seed one conversation before running the harness");
        NSData *json = [NSJSONSerialization dataWithJSONObject:@[target] options:0 error:nil];
        NSString *argument = [[NSString alloc] initWithData:json encoding:NSUTF8StringEncoding];
        NSString *script = [NSString stringWithFormat:@"window.__panelResult='pending'; window.webkit.messageHandlers.ezerbotFloating.postMessage({command:'openConversation',conversation_id:%@[0]}).then(()=>window.__panelResult='opened',()=>window.__panelResult='failed'); true",argument];
        EZJavaScript(floating.webView,script);
        __block NSString *result = @"pending";
        EZWait(^BOOL { result = EZJavaScript(floating.webView,@"window.__panelResult"); return ![result isEqual:@"pending"]; });
        NSCAssert([result isEqual:@"opened"] && shell.window.visible && !floating.panel.visible, @"Open in Ezerbot restores main and focuses exact conversation");
        [floating invalidate];
        [NSStatusBar.systemStatusBar removeStatusItem:shell.statusItem];
        [shell.webView.configuration.userContentController removeAllScriptMessageHandlers];
        shell.webView.navigationDelegate = nil; shell.webView.UIDelegate = nil;
        puts("NSPanel: menu, hidden main, recording-only shortcut, trust, move, Escape, persistent position, open conversation — OK");
    }
    return 0;
}
