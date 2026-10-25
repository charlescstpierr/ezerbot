#import <Cocoa/Cocoa.h>
#import <WebKit/WebKit.h>
#import "local-api.h"
#if defined(EZERBOT_RELEASE)
#import <Carbon/Carbon.h>
#endif

@interface EzerbotFloatingWindow : NSPanel
@property(nonatomic, copy) void (^dismiss)(void);
@end
@implementation EzerbotFloatingWindow
- (BOOL)canBecomeKeyWindow { return YES; }
- (BOOL)canBecomeMainWindow { return NO; }
- (void)cancelOperation:(id)sender { (void)sender; if (self.dismiss) self.dismiss(); }
@end

@interface EzerbotFloating : NSObject <NSWindowDelegate, WKNavigationDelegate, WKScriptMessageHandlerWithReply>
@property(nonatomic, strong) EzerbotFloatingWindow *panel;
@property(nonatomic, weak) NSWindow *mainWindow;
@property(nonatomic, strong) WKWebView *webView;
@property(nonatomic, strong) NSURL *backendURL;
@property(nonatomic, strong) EzerbotLocalAPI *localAPI;
@property(nonatomic, copy) NSString *geometryPath;
@property(nonatomic, copy) NSString *shortcut;
@property(nonatomic, copy) void (^openConversation)(NSString *, void (^)(BOOL, NSString *));
#if defined(EZERBOT_RELEASE)
@property(nonatomic) EventHotKeyRef hotKey;
@property(nonatomic) EventHandlerRef hotKeyHandler;
#endif
- (NSDictionary *)configure:(NSString *)shortcut;
- (void)show;
- (void)close;
- (void)invalidate;
@end

#if defined(EZERBOT_RELEASE)
static OSStatus EZFloatingHotKey(EventHandlerCallRef next, EventRef event, void *context) {
    (void)next;
    EventHotKeyID identity = {0};
    if (GetEventParameter(event, kEventParamDirectObject, typeEventHotKeyID, NULL, sizeof(identity), NULL, &identity) != noErr || identity.signature != 'EZFP') return eventNotHandledErr;
    EzerbotFloating *owner = (__bridge EzerbotFloating *)context;
    [owner show];
    return noErr;
}
#endif

@implementation EzerbotFloating
- (NSDictionary *)configure:(NSString *)shortcut {
    NSArray *choices = @[@"disabled", @"cmd+option+space", @"ctrl+option+space", @"cmd+shift+space", @"cmd+option+j"];
    if (![choices containsObject:shortcut]) return @{@"status":@"error"};
    if ([self.shortcut isEqual:shortcut]) {
#if defined(EZERBOT_RELEASE)
        return @{@"status":self.hotKey ? @"registered" : @"disabled"};
#else
        return @{@"status":@"recording"};
#endif
    }
#if defined(EZERBOT_RELEASE)
    // Carbon registers one discrete shortcut, not a keyboard event tap. It does
    // not request Accessibility or Input Monitoring. Debug never reaches here.
    if (!self.hotKeyHandler) {
        EventTypeSpec kind = { kEventClassKeyboard, kEventHotKeyPressed };
        if (InstallApplicationEventHandler(EZFloatingHotKey, 1, &kind, (__bridge void *)self, &_hotKeyHandler) != noErr) return @{@"status":@"error"};
    }
    EventHotKeyRef replacement = NULL;
    if (![shortcut isEqual:@"disabled"]) {
        UInt32 modifiers = [shortcut hasPrefix:@"ctrl"] ? controlKey | optionKey : [shortcut containsString:@"shift"] ? cmdKey | shiftKey : cmdKey | optionKey;
        UInt32 key = [shortcut hasSuffix:@"+j"] ? kVK_ANSI_J : kVK_Space;
        EventHotKeyID identity = { 'EZFP', 1 };
        OSStatus result = RegisterEventHotKey(key, modifiers, identity, GetApplicationEventTarget(), 0, &replacement);
        // Keep the previous registration alive on conflict; never persist a
        // shortcut which we could not acquire.
        if (result != noErr) return @{@"status":result == eventHotKeyExistsErr ? @"conflict" : @"error"};
    }
    if (self.hotKey) UnregisterEventHotKey(self.hotKey);
    self.hotKey = replacement;
#endif
    self.shortcut = shortcut;
#if defined(EZERBOT_RELEASE)
    return @{@"status":self.hotKey ? @"registered" : @"disabled"};
#else
    return @{@"status":@"recording"};
#endif
}

- (BOOL)isPanelURL:(NSURL *)url {
    return self.backendURL && [url.scheme isEqual:@"http"] && [url.host isEqual:@"127.0.0.1"] &&
        [url.port isEqual:self.backendURL.port] && [url.path isEqual:@"/floating"] && !url.user && !url.password && !url.query && !url.fragment;
}

- (NSRect)visibleFrame:(NSRect)frame {
    NSScreen *screen = NSScreen.mainScreen;
    for (NSScreen *candidate in NSScreen.screens) {
        if (NSIntersectsRect(candidate.visibleFrame, frame)) { screen = candidate; break; }
    }
    NSRect bounds = screen.visibleFrame;
    if (NSIsEmptyRect(bounds)) return NSMakeRect(80, 80, 420, 588);
    frame.size.width = MIN(MAX(360, frame.size.width), bounds.size.width);
    frame.size.height = MIN(MAX(520, frame.size.height), bounds.size.height);
    frame.origin.x = MIN(MAX(NSMinX(bounds), frame.origin.x), NSMaxX(bounds) - frame.size.width);
    frame.origin.y = MIN(MAX(NSMinY(bounds), frame.origin.y), NSMaxY(bounds) - frame.size.height);
    return frame;
}

- (void)show {
    if (!self.backendURL) return;
    if (!self.panel) {
        EzerbotFloatingWindow *panel = [[EzerbotFloatingWindow alloc] initWithContentRect:NSMakeRect(80, 80, 420, 560)
            styleMask:NSWindowStyleMaskTitled | NSWindowStyleMaskClosable | NSWindowStyleMaskResizable | NSWindowStyleMaskNonactivatingPanel
            backing:NSBackingStoreBuffered defer:NO];
        panel.title = @"Panneau · Ezerbot";
        panel.releasedWhenClosed = NO;
        panel.floatingPanel = YES; panel.hidesOnDeactivate = NO;
        panel.becomesKeyOnlyIfNeeded = NO; panel.level = NSFloatingWindowLevel;
        panel.collectionBehavior = NSWindowCollectionBehaviorCanJoinAllSpaces | NSWindowCollectionBehaviorFullScreenAuxiliary;
        panel.minSize = NSMakeSize(360, 520); panel.maxSize = NSMakeSize(650, 850);
        __weak EzerbotFloating *weakSelf = self;
        panel.dismiss = ^{ [weakSelf close]; };
        NSDictionary *saved = self.geometryPath ? [NSDictionary dictionaryWithContentsOfFile:self.geometryPath] : nil;
        NSRect frame = panel.frame;
        if ([saved[@"x"] isKindOfClass:NSNumber.class] && [saved[@"y"] isKindOfClass:NSNumber.class]) {
            double x = [saved[@"x"] doubleValue], y = [saved[@"y"] doubleValue];
            if (isfinite(x) && isfinite(y)) frame.origin = NSMakePoint(x, y);
        }
        [panel setFrame:[self visibleFrame:frame] display:NO];
        panel.delegate = self;
        WKWebViewConfiguration *configuration = [[WKWebViewConfiguration alloc] init];
        configuration.websiteDataStore = [WKWebsiteDataStore nonPersistentDataStore];
        [self.localAPI installInConfiguration:configuration];
        [configuration.userContentController addScriptMessageHandlerWithReply:self contentWorld:WKContentWorld.pageWorld name:@"ezerbotFloating"];
        self.webView = [[WKWebView alloc] initWithFrame:panel.contentView.bounds configuration:configuration];
        self.webView.navigationDelegate = self;
        self.webView.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
        panel.contentView = self.webView;
        self.panel = panel;
        [self.webView loadRequest:[NSURLRequest requestWithURL:[self.backendURL URLByAppendingPathComponent:@"floating"]]];
    } else {
        [self.panel setFrame:[self visibleFrame:self.panel.frame] display:YES];
        [self.webView evaluateJavaScript:@"window.EzerbotFloating?.open()" completionHandler:nil];
    }
    // Unhiding the application must not resurrect its full workspace merely
    // because the user requested the compact panel from another application.
    if (NSApp.isHidden) [self.mainWindow orderOut:nil];
    [NSApp unhideWithoutActivation];
    [self.panel makeKeyAndOrderFront:nil];
}
- (void)savePosition {
    if (!self.panel || !self.geometryPath) return;
    NSPoint point = self.panel.frame.origin;
    [@{@"x":@(point.x), @"y":@(point.y)} writeToFile:self.geometryPath atomically:YES];
}
- (void)windowDidMove:(NSNotification *)notification { (void)notification; [self savePosition]; }
- (void)windowDidResize:(NSNotification *)notification { (void)notification; [self savePosition]; }
- (BOOL)windowShouldClose:(NSWindow *)sender { (void)sender; [self close]; return NO; }
- (void)close { [self savePosition]; [self.panel orderOut:nil]; }
- (void)invalidate {
    [self close];
    [self.webView.configuration.userContentController removeScriptMessageHandlerForName:@"ezerbotFloating" contentWorld:WKContentWorld.pageWorld];
    self.webView.navigationDelegate = nil; self.webView = nil;
    self.panel.delegate = nil; self.panel.dismiss = nil; self.panel = nil;
#if defined(EZERBOT_RELEASE)
    if (self.hotKey) UnregisterEventHotKey(self.hotKey);
    if (self.hotKeyHandler) RemoveEventHandler(self.hotKeyHandler);
    self.hotKey = NULL; self.hotKeyHandler = NULL;
#endif
    self.shortcut = nil;
}
- (void)webView:(WKWebView *)view decidePolicyForNavigationAction:(WKNavigationAction *)action decisionHandler:(void (^)(WKNavigationActionPolicy))decision {
    (void)view;
    decision([self isPanelURL:action.request.URL] ? WKNavigationActionPolicyAllow : WKNavigationActionPolicyCancel);
}
- (void)userContentController:(WKUserContentController *)controller didReceiveScriptMessage:(WKScriptMessage *)message replyHandler:(void (^)(id, NSString *))reply {
    (void)controller;
    if (message.webView != self.webView || !message.frameInfo.isMainFrame || ![self isPanelURL:message.frameInfo.request.URL] || ![self isPanelURL:self.webView.URL] || ![message.body isKindOfClass:NSDictionary.class]) {
        reply(nil, @"Origine du panneau refusée."); return;
    }
    NSDictionary *body = message.body;
    if ([body[@"command"] isEqual:@"close"]) { [self close]; reply(@YES,nil); return; }
    if ([body[@"command"] isEqual:@"appearance"] && [@[@"dark",@"light"] containsObject:body[@"theme"]]) {
        self.panel.appearance = [NSAppearance appearanceNamed:[body[@"theme"] isEqual:@"dark"] ? NSAppearanceNameDarkAqua : NSAppearanceNameAqua];
        reply(@YES,nil); return;
    }
    NSString *identifier = body[@"conversation_id"];
    NSCharacterSet *invalid = [[NSCharacterSet characterSetWithCharactersInString:@"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_"] invertedSet];
    if (![body[@"command"] isEqual:@"openConversation"] || ![identifier isKindOfClass:NSString.class] || identifier.length == 0 || identifier.length > 128 || [identifier rangeOfCharacterFromSet:invalid].location != NSNotFound || !self.openConversation) {
        reply(nil, @"Commande du panneau invalide."); return;
    }
    self.openConversation(identifier, ^(BOOL opened, NSString *error) {
        if (opened) { [self close]; reply(@YES,nil); }
        else reply(nil,error ?: @"");
    });
}
@end
