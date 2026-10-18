#import <Cocoa/Cocoa.h>
#import <UniformTypeIdentifiers/UniformTypeIdentifiers.h>
#import <WebKit/WebKit.h>
#import "notifications.h"
#import "voice.h"
#import <errno.h>
#import <poll.h>
#import <signal.h>
#import <sys/stat.h>
#import <unistd.h>

@interface GrokbotApplication : NSObject <NSApplicationDelegate, NSWindowDelegate, WKNavigationDelegate, WKUIDelegate, WKDownloadDelegate, WKScriptMessageHandlerWithReply, UNUserNotificationCenterDelegate>
@property(nonatomic, strong) NSWindow *window;
@property(nonatomic, strong) WKWebView *webView;
@property(nonatomic, strong) NSTask *backendTask;
@property(nonatomic, strong) NSFileHandle *logHandle;
@property(nonatomic, strong) NSURL *backendURL;
@property(nonatomic, copy) NSString *supportPath;
@property(nonatomic) NSUInteger startupGeneration;
@property(nonatomic) BOOL terminating;
@property(nonatomic, strong) EzerbotNotifier *notifier;
@property(nonatomic, strong) EzerbotVoice *voice;
@property(nonatomic, copy) NSString *visibleConversationID;
@property(nonatomic) BOOL conversationVisible;
@property(nonatomic, copy) NSString *pendingNotificationID;
@property(nonatomic, strong) NSMutableArray<NSString *> *pendingDeepLinks;
@property(nonatomic) BOOL deepLinksReady;
@property(nonatomic) BOOL deepLinksFlushing;
@property(nonatomic) NSUInteger deepLinksEpoch;
@end

@implementation GrokbotApplication

- (void)applicationDidFinishLaunching:(NSNotification *)notification {
    (void)notification;
    self.notifier = [[EzerbotNotifier alloc] init];
    self.voice = [[EzerbotVoice alloc] init];
#if defined(EZERBOT_RELEASE)
    [UNUserNotificationCenter currentNotificationCenter].delegate = self;
#endif
    [self createMenus];
    self.window = [[NSWindow alloc] initWithContentRect:NSMakeRect(0, 0, 1140, 780)
                                            styleMask:NSWindowStyleMaskTitled | NSWindowStyleMaskClosable |
                                                      NSWindowStyleMaskMiniaturizable | NSWindowStyleMaskResizable |
                                                      NSWindowStyleMaskFullSizeContentView
                                              backing:NSBackingStoreBuffered defer:NO];
    self.window.title = @"Ezerbot";
    self.window.titleVisibility = NSWindowTitleHidden;
    self.window.titlebarAppearsTransparent = YES;
    self.window.minSize = NSMakeSize(820, 560);
    self.window.backgroundColor = [NSColor colorWithCalibratedRed:0.07 green:0.075 blue:0.09 alpha:1];
    self.window.delegate = self;
    self.window.releasedWhenClosed = NO;
    [self.window center];
    [self.window makeKeyAndOrderFront:nil];
    [NSApp activateIgnoringOtherApps:YES];
    for (NSString *argument in NSProcessInfo.processInfo.arguments) {
        if ([argument.lowercaseString hasPrefix:@"ezerbot://"]) [self receiveDeepLink:argument];
    }
    [self startBackend:nil];
}

// Cocoa may call openURLs before applicationDidFinishLaunching. Keep the raw
// URL until the local UI has installed its listener AND restored its workspace.
- (void)application:(NSApplication *)application openURLs:(NSArray<NSURL *> *)urls {
    (void)application;
    for (NSURL *url in urls) [self receiveDeepLink:url.absoluteString];
}

- (void)receiveDeepLink:(NSString *)raw {
    if (!self.pendingDeepLinks) self.pendingDeepLinks = [NSMutableArray array];
    if ([self.pendingDeepLinks containsObject:raw]) return;
    if (raw.length > 2048 || self.pendingDeepLinks.count >= 16) {
        NSAlert *alert = [[NSAlert alloc] init];
        alert.messageText = @"Impossible d’ouvrir ce lien";
        alert.informativeText = raw.length > 2048 ? @"Ce lien Ezerbot est trop long." : @"Trop de liens en attente. Rouvrez ce lien après le démarrage.";
        [alert runModal]; return;
    }
    [self.pendingDeepLinks addObject:raw];
    [self.window deminiaturize:nil]; [self.window makeKeyAndOrderFront:nil];
    [NSApp activateIgnoringOtherApps:YES];
    [self flushDeepLinks];
}

- (void)flushDeepLinks {
    if (!self.deepLinksReady || self.deepLinksFlushing || !self.pendingDeepLinks.count || ![self isWorkspaceURL:self.webView.URL]) return;
    NSString *raw = self.pendingDeepLinks.firstObject;
    NSData *json = [NSJSONSerialization dataWithJSONObject:@[raw] options:0 error:nil];
    NSString *argument = [[NSString alloc] initWithData:json encoding:NSUTF8StringEncoding];
    NSString *script = [NSString stringWithFormat:@"Boolean(window.EzerbotDeepLinks?.receive(%@[0]))", argument];
    NSUInteger epoch = self.deepLinksEpoch;
    self.deepLinksFlushing = YES;
    [self.webView evaluateJavaScript:script completionHandler:^(id result, NSError *error) {
        if (epoch != self.deepLinksEpoch) return;
        self.deepLinksFlushing = NO;
        if (!error && [result boolValue]) {
            [self.pendingDeepLinks removeObjectAtIndex:0];
            [self flushDeepLinks];
        }
    }];
}

- (void)createMenus {
    NSMenu *mainMenu = [[NSMenu alloc] initWithTitle:@""];
    NSMenuItem *appItem = [[NSMenuItem alloc] initWithTitle:@"Ezerbot" action:nil keyEquivalent:@""];
    NSMenu *appMenu = [[NSMenu alloc] initWithTitle:@"Ezerbot"];
    [appMenu addItemWithTitle:@"À propos d’Ezerbot" action:@selector(orderFrontStandardAboutPanel:) keyEquivalent:@""];
    [appMenu addItem:[NSMenuItem separatorItem]];
    [appMenu addItemWithTitle:@"Masquer Ezerbot" action:@selector(hide:) keyEquivalent:@"h"];
    NSMenuItem *hideOthers = [appMenu addItemWithTitle:@"Masquer les autres" action:@selector(hideOtherApplications:) keyEquivalent:@"h"];
    hideOthers.keyEquivalentModifierMask = NSEventModifierFlagCommand | NSEventModifierFlagOption;
    [appMenu addItemWithTitle:@"Tout afficher" action:@selector(unhideAllApplications:) keyEquivalent:@""];
    [appMenu addItem:[NSMenuItem separatorItem]];
    [appMenu addItemWithTitle:@"Quitter Ezerbot" action:@selector(terminate:) keyEquivalent:@"q"];
    appItem.submenu = appMenu;
    [mainMenu addItem:appItem];

    NSMenuItem *fileItem = [[NSMenuItem alloc] initWithTitle:@"Fichier" action:nil keyEquivalent:@""];
    NSMenu *fileMenu = [[NSMenu alloc] initWithTitle:@"Fichier"];
    [fileMenu addItemWithTitle:@"Fermer la fenêtre" action:@selector(performClose:) keyEquivalent:@"w"];
    NSMenuItem *reload = [fileMenu addItemWithTitle:@"Recharger" action:@selector(reloadPage:) keyEquivalent:@"r"];
    reload.target = self;
    fileItem.submenu = fileMenu;
    [mainMenu addItem:fileItem];

    NSMenuItem *editItem = [[NSMenuItem alloc] initWithTitle:@"Édition" action:nil keyEquivalent:@""];
    NSMenu *editMenu = [[NSMenu alloc] initWithTitle:@"Édition"];
    [editMenu addItemWithTitle:@"Annuler" action:@selector(undo:) keyEquivalent:@"z"];
    NSMenuItem *redo = [editMenu addItemWithTitle:@"Rétablir" action:@selector(redo:) keyEquivalent:@"z"];
    redo.keyEquivalentModifierMask = NSEventModifierFlagCommand | NSEventModifierFlagShift;
    [editMenu addItem:[NSMenuItem separatorItem]];
    [editMenu addItemWithTitle:@"Couper" action:@selector(cut:) keyEquivalent:@"x"];
    [editMenu addItemWithTitle:@"Copier" action:@selector(copy:) keyEquivalent:@"c"];
    [editMenu addItemWithTitle:@"Coller" action:@selector(paste:) keyEquivalent:@"v"];
    [editMenu addItemWithTitle:@"Tout sélectionner" action:@selector(selectAll:) keyEquivalent:@"a"];
    editItem.submenu = editMenu;
    [mainMenu addItem:editItem];
    NSApp.mainMenu = mainMenu;
}

- (void)showStatus:(NSString *)title detail:(NSString *)detail failed:(BOOL)failed {
    [self.voice cancel];
    self.deepLinksReady = NO; self.deepLinksFlushing = NO; ++self.deepLinksEpoch;
    self.webView.navigationDelegate = nil;
    self.webView.UIDelegate = nil;
    self.webView = nil;
    NSView *content = [[NSView alloc] initWithFrame:self.window.contentView.bounds];
    content.wantsLayer = YES;
    content.layer.backgroundColor = self.window.backgroundColor.CGColor;
    self.window.contentView = content;

    NSTextField *heading = [NSTextField labelWithString:title];
    heading.font = [NSFont systemFontOfSize:28 weight:NSFontWeightSemibold];
    heading.textColor = [NSColor whiteColor];
    NSTextField *description = [NSTextField wrappingLabelWithString:detail];
    description.font = [NSFont systemFontOfSize:14];
    description.textColor = [NSColor colorWithWhite:0.65 alpha:1];
    description.alignment = NSTextAlignmentCenter;
    NSStackView *stack = [NSStackView stackViewWithViews:@[heading, description]];
    stack.orientation = NSUserInterfaceLayoutOrientationVertical;
    stack.alignment = NSLayoutAttributeCenterX;
    stack.spacing = 18;
    if (failed) {
        NSButton *retry = [NSButton buttonWithTitle:@"Réessayer" target:self action:@selector(startBackend:)];
        retry.bezelStyle = NSBezelStyleRounded;
        NSButton *logs = [NSButton buttonWithTitle:@"Ouvrir les journaux" target:self action:@selector(openLogs:)];
        logs.bezelStyle = NSBezelStyleRounded;
        [stack addArrangedSubview:retry];
        [stack addArrangedSubview:logs];
    } else {
        NSProgressIndicator *spinner = [[NSProgressIndicator alloc] initWithFrame:NSMakeRect(0, 0, 24, 24)];
        spinner.style = NSProgressIndicatorStyleSpinning;
        [spinner startAnimation:nil];
        [stack addArrangedSubview:spinner];
    }
    stack.translatesAutoresizingMaskIntoConstraints = NO;
    [content addSubview:stack];
    [NSLayoutConstraint activateConstraints:@[
        [stack.centerXAnchor constraintEqualToAnchor:content.centerXAnchor],
        [stack.centerYAnchor constraintEqualToAnchor:content.centerYAnchor],
        [stack.widthAnchor constraintLessThanOrEqualToConstant:570],
        [stack.leadingAnchor constraintGreaterThanOrEqualToAnchor:content.leadingAnchor constant:40],
        [stack.trailingAnchor constraintLessThanOrEqualToAnchor:content.trailingAnchor constant:-40]
    ]];
}

- (void)failStartup:(NSString *)message generation:(NSUInteger)generation {
    if (generation != self.startupGeneration || self.terminating) return;
    ++self.startupGeneration;
    [self stopBackend];
    [self showStatus:@"Le moteur n’a pas démarré" detail:message failed:YES];
}

- (void)startBackend:(id)sender {
    (void)sender;
    [self stopBackend];
    NSUInteger generation = ++self.startupGeneration;
    self.backendURL = nil;
    [self showStatus:@"Ezerbot" detail:@"Ouverture de votre espace local…" failed:NO];
    NSFileManager *files = [NSFileManager defaultManager];
    NSError *error = nil;
    NSURL *applicationSupport = [files URLForDirectory:NSApplicationSupportDirectory inDomain:NSUserDomainMask
                                    appropriateForURL:nil create:YES error:&error];
    if (!applicationSupport) {
        [self failStartup:error.localizedDescription generation:generation];
        return;
    }
    self.supportPath = [[applicationSupport URLByAppendingPathComponent:@"Grokbot" isDirectory:YES] path];
    NSString *logsPath = [self.supportPath stringByAppendingPathComponent:@"logs"];
    if (![files createDirectoryAtPath:logsPath withIntermediateDirectories:YES
                           attributes:@{NSFilePosixPermissions:@0700} error:&error]) {
        [self failStartup:error.localizedDescription generation:generation];
        return;
    }
    NSString *logPath = [logsPath stringByAppendingPathComponent:@"engine.log"];
    NSDictionary *attributes = [files attributesOfItemAtPath:logPath error:nil];
    if ([attributes[NSFileSize] unsignedLongLongValue] > 1024 * 1024) {
        NSString *previous = [logsPath stringByAppendingPathComponent:@"engine.previous.log"];
        [files removeItemAtPath:previous error:nil];
        [files moveItemAtPath:logPath toPath:previous error:nil];
    }
    if (![files fileExistsAtPath:logPath] &&
        ![files createFileAtPath:logPath contents:nil attributes:@{NSFilePosixPermissions:@0600}]) {
        [self failStartup:@"Le journal local ne peut pas être créé." generation:generation];
        return;
    }
    self.logHandle = [NSFileHandle fileHandleForWritingAtPath:logPath];
    if (!self.logHandle) {
        [self failStartup:@"Le journal local ne peut pas être ouvert." generation:generation];
        return;
    }
    [self.logHandle seekToEndOfFile];
    NSString *entry = [NSString stringWithFormat:@"\n[%@] Ouverture d’Ezerbot\n", [NSDate date]];
    [self.logHandle writeData:[entry dataUsingEncoding:NSUTF8StringEncoding]];

    NSString *enginePath = [[[NSBundle mainBundle] executablePath].stringByDeletingLastPathComponent
                            stringByAppendingPathComponent:@"grokbot-wrapper"];
    if (![files isExecutableFileAtPath:enginePath]) {
        [self failStartup:@"Le moteur Rust manque dans l’application. Relancez tools/build-app.sh." generation:generation];
        return;
    }
    NSPipe *output = [NSPipe pipe];
    NSTask *task = [[NSTask alloc] init];
    task.executableURL = [NSURL fileURLWithPath:enginePath];
    task.arguments = @[@"--port", @"0", @"--db", [self.supportPath stringByAppendingPathComponent:@"workspace.sqlite"]];
    task.standardOutput = output;
    task.standardError = self.logHandle;
    task.currentDirectoryURL = [NSURL fileURLWithPath:self.supportPath isDirectory:YES];
    __weak GrokbotApplication *weakSelf = self;
    task.terminationHandler = ^(NSTask *finished) {
        dispatch_async(dispatch_get_main_queue(), ^{
            GrokbotApplication *strongSelf = weakSelf;
            if (strongSelf && strongSelf.backendTask == finished && !strongSelf.terminating) {
                [strongSelf failStartup:@"Le moteur s’est arrêté. Les détails sont dans le journal local."
                              generation:generation];
            }
        });
    };
    self.backendTask = task;
    if (![task launchAndReturnError:&error]) {
        [self failStartup:error.localizedDescription generation:generation];
        return;
    }
    NSFileHandle *reader = output.fileHandleForReading;
    dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0), ^{
        NSMutableData *line = [NSMutableData data];
        NSString *problem = nil;
        NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:15];
        BOOL complete = NO;
        while (!complete && line.length <= 8192) {
            int remaining = (int)MAX(0, [deadline timeIntervalSinceNow] * 1000);
            if (remaining == 0) { problem = @"Le moteur ne répond pas après 15 secondes."; break; }
            struct pollfd ready = {.fd = reader.fileDescriptor, .events = POLLIN};
            int result = poll(&ready, 1, remaining);
            if (result < 0 && errno == EINTR) continue;
            if (result <= 0) { problem = @"Le moteur n’a pas annoncé son adresse locale."; break; }
            unsigned char character;
            ssize_t length = read(reader.fileDescriptor, &character, 1);
            if (length < 0 && errno == EINTR) continue;
            if (length <= 0) { problem = @"Le moteur s’est arrêté avant l’ouverture de l’espace local."; break; }
            if (character == '\n') complete = YES;
            else [line appendBytes:&character length:1];
        }
        if (!complete && !problem) problem = @"L’adresse annoncée par le moteur est trop longue.";
        NSDictionary *announcement = complete ? [NSJSONSerialization JSONObjectWithData:line options:0 error:nil] : nil;
        NSString *urlString = [announcement isKindOfClass:[NSDictionary class]] &&
                              [announcement[@"url"] isKindOfClass:[NSString class]] ? announcement[@"url"] : nil;
        NSURLComponents *components = urlString ? [NSURLComponents componentsWithString:urlString] : nil;
        NSInteger port = components.port.integerValue;
        if (!problem && (!components || ![components.scheme isEqualToString:@"http"] ||
                         ![components.host isEqualToString:@"127.0.0.1"] || port < 1 || port > 65535 ||
                         components.user || components.password || components.query || components.fragment ||
                         (components.path.length > 0 && ![components.path isEqualToString:@"/"]))) {
            problem = @"Le moteur a annoncé une adresse locale invalide.";
        }
        if (!problem) {
            // Drain later stdout so the child can never block on a full pipe.
            reader.readabilityHandler = ^(NSFileHandle *handle) {
                if (handle.availableData.length == 0) handle.readabilityHandler = nil;
            };
        }
        dispatch_async(dispatch_get_main_queue(), ^{
            GrokbotApplication *strongSelf = weakSelf;
            if (!strongSelf || generation != strongSelf.startupGeneration || strongSelf.terminating) return;
            if (problem) [strongSelf failStartup:problem generation:generation];
            else [strongSelf openWorkspace:components.URL];
        });
    });
}

- (void)openWorkspace:(NSURL *)url {
    self.backendURL = url;
    WKWebViewConfiguration *configuration = [[WKWebViewConfiguration alloc] init];
    configuration.websiteDataStore = [WKWebsiteDataStore nonPersistentDataStore];
    [configuration.userContentController addScriptMessageHandlerWithReply:self contentWorld:WKContentWorld.pageWorld name:@"ezerbotNotifications"];
    [configuration.userContentController addScriptMessageHandlerWithReply:self contentWorld:WKContentWorld.pageWorld name:@"ezerbotVoice"];
    [configuration.userContentController addScriptMessageHandlerWithReply:self contentWorld:WKContentWorld.pageWorld name:@"ezerbotDeepLinks"];
    WKWebView *view = [[WKWebView alloc] initWithFrame:self.window.contentView.bounds configuration:configuration];
    view.navigationDelegate = self;
    view.UIDelegate = self;
    view.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
    view.allowsBackForwardNavigationGestures = NO;
    view.underPageBackgroundColor = self.window.backgroundColor;
    self.webView = view;
    self.window.contentView = view;
    [view loadRequest:[NSURLRequest requestWithURL:url]];
}

- (void)sendNotificationFocus {
    BOOL focused = NSApp.isActive && self.window.isKeyWindow;
    [self.webView evaluateJavaScript:[NSString stringWithFormat:@"window.EzerbotNotifications?.focus(%@)", focused ? @"true" : @"false"] completionHandler:nil];
}

- (void)windowDidBecomeKey:(NSNotification *)notification { (void)notification; [self sendNotificationFocus]; }
- (void)windowDidResignKey:(NSNotification *)notification { (void)notification; [self sendNotificationFocus]; }
- (void)applicationDidBecomeActive:(NSNotification *)notification { (void)notification; [self sendNotificationFocus]; }
- (void)applicationDidResignActive:(NSNotification *)notification { (void)notification; [self sendNotificationFocus]; }

- (void)activateNotification:(NSString *)eventID {
    self.pendingNotificationID = eventID;
    [self.window deminiaturize:nil];
    [self.window makeKeyAndOrderFront:nil];
    [NSApp activateIgnoringOtherApps:YES];
    [self flushNotificationClick];
}

- (void)flushNotificationClick {
    if (!self.pendingNotificationID || ![self isWorkspaceURL:self.webView.URL]) return;
    NSData *json = [NSJSONSerialization dataWithJSONObject:@[self.pendingNotificationID] options:0 error:nil];
    NSString *argument = [[NSString alloc] initWithData:json encoding:NSUTF8StringEncoding];
    NSString *script = [NSString stringWithFormat:@"Boolean(window.EzerbotNotifications && (window.EzerbotNotifications.activate(%@[0]), true))", argument];
    NSString *expected = self.pendingNotificationID;
    [self.webView evaluateJavaScript:script completionHandler:^(id result, NSError *error) {
        if (!error && [result boolValue] && [self.pendingNotificationID isEqualToString:expected]) self.pendingNotificationID = nil;
    }];
}

- (void)webView:(WKWebView *)webView didStartProvisionalNavigation:(WKNavigation *)navigation {
    (void)webView; (void)navigation;
    self.deepLinksReady = NO; self.deepLinksFlushing = NO; ++self.deepLinksEpoch;
}

- (void)webView:(WKWebView *)webView didFinishNavigation:(WKNavigation *)navigation {
    (void)webView; (void)navigation;
    [self sendNotificationFocus];
    [self flushNotificationClick];
}

- (void)userContentController:(WKUserContentController *)controller didReceiveScriptMessage:(WKScriptMessage *)message
                replyHandler:(void (^)(id, NSString *))replyHandler {
    (void)controller;
    if (message.webView != self.webView || !message.frameInfo.isMainFrame ||
        ![self isWorkspaceURL:message.frameInfo.request.URL] || ![self isWorkspaceURL:self.webView.URL] ||
        ![message.body isKindOfClass:NSDictionary.class] || self.terminating) {
        replyHandler(nil, @"Origine de notification refusée."); return;
    }
    NSDictionary *body = message.body;
    NSString *command = body[@"command"];
    if ([message.name isEqualToString:@"ezerbotDeepLinks"]) {
        if ([command isEqual:@"ready"]) {
            self.deepLinksReady = YES;
            [self flushDeepLinks]; replyHandler(@YES, nil); return;
        }
#if !defined(EZERBOT_RELEASE)
        // Direct debug entry point: exercise Cocoa forwarding without ever
        // registering a test bundle or asking LaunchServices to open a URL.
        if ([command isEqual:@"receive"] && [body[@"url"] isKindOfClass:NSString.class]) {
            [self receiveDeepLink:body[@"url"]]; replyHandler(@YES, nil); return;
        }
#endif
        replyHandler(nil, @"Commande de lien inconnue."); return;
    }
    void (^reply)(NSDictionary *) = ^(NSDictionary *result) {
        dispatch_async(dispatch_get_main_queue(), ^{ replyHandler(result, nil); });
    };
    if ([message.name isEqual:@"ezerbotVoice"]) { [self.voice command:body reply:reply]; return; }
    if ([command isEqual:@"presence"]) {
        self.visibleConversationID = [body[@"conversation_id"] isKindOfClass:NSString.class] ? body[@"conversation_id"] : nil;
        self.conversationVisible = [body[@"visible"] isKindOfClass:NSNumber.class] && [body[@"visible"] boolValue];
        reply(@{@"status":@"ok"}); return;
    }
    if ([command isEqual:@"status"]) { [self.notifier status:reply]; return; }
    if ([command isEqual:@"authorize"]) { [self.notifier authorize:reply]; return; }
#if !defined(EZERBOT_RELEASE)
    if ([command isEqual:@"recordings"]) { reply(@{@"status":@"recording", @"events":self.notifier.recordings}); return; }
    if ([command isEqual:@"activateRecorded"]) {
        for (NSDictionary *record in self.notifier.recordings) {
            if ([record[@"event"][@"id"] isEqual:body[@"id"]]) {
                [self activateNotification:body[@"id"]]; reply(@{@"status":@"ok"}); return;
            }
        }
        replyHandler(nil, @"Notification de test inconnue."); return;
    }
#endif
    if (![command isEqual:@"deliver"] || ![body[@"event"] isKindOfClass:NSDictionary.class]) {
        replyHandler(nil, @"Commande de notification inconnue."); return;
    }
    NSDictionary *event = body[@"event"];
    for (NSString *key in @[@"id", @"kind", @"conversation_id", @"card_id", @"title"]) {
        if (![event[key] isKindOfClass:NSString.class] ||
            ([event[key] length] == 0 && ![key isEqualToString:@"card_id"]) || [event[key] length] > 512) {
            replyHandler(nil, @"Notification invalide."); return;
        }
    }
    NSDictionary *titles = @{@"reply":@[@"Réponse terminée"], @"approval":@[@"Autorisation demandée"],
        @"form":@[@"Informations demandées"], @"secret":@[@"Secret demandé"],
        @"routine":@[@"Routine terminée", @"Échec de la routine"], @"delegation":@[@"Sous-tâche terminée"]};
    if (![titles[event[@"kind"]] containsObject:event[@"title"]]) { replyHandler(nil, @"Type de notification invalide."); return; }
    if (NSApp.isActive && self.window.isKeyWindow && self.conversationVisible && [self.visibleConversationID isEqualToString:event[@"conversation_id"]]) {
        reply(@{@"status":@"suppressed"}); return;
    }
    [self.notifier deliver:event reply:reply];
}

- (void)userNotificationCenter:(UNUserNotificationCenter *)center willPresentNotification:(UNNotification *)notification
        withCompletionHandler:(void (^)(UNNotificationPresentationOptions))completionHandler {
    (void)center;
    dispatch_async(dispatch_get_main_queue(), ^{
        BOOL visible = NSApp.isActive && self.window.isKeyWindow && self.conversationVisible &&
            [self.visibleConversationID isEqualToString:notification.request.content.userInfo[@"conversation_id"]];
        completionHandler(visible ? UNNotificationPresentationOptionNone : UNNotificationPresentationOptionBanner | UNNotificationPresentationOptionList);
    });
}

- (void)userNotificationCenter:(UNUserNotificationCenter *)center didReceiveNotificationResponse:(UNNotificationResponse *)response
        withCompletionHandler:(void (^)(void))completionHandler {
    (void)center;
    NSString *eventID = response.notification.request.content.userInfo[@"event_id"];
    dispatch_async(dispatch_get_main_queue(), ^{
        if ([eventID isKindOfClass:NSString.class] && eventID.length <= 512) [self activateNotification:eventID];
        completionHandler();
    });
}

- (BOOL)isWorkspaceURL:(NSURL *)url {
    return self.backendURL && [url.scheme isEqualToString:@"http"] &&
           [url.host isEqualToString:@"127.0.0.1"] &&
           [url.port isEqualToNumber:self.backendURL.port] && !url.user && !url.password;
}

- (BOOL)isExternalWebURL:(NSURL *)url {
    return [url.scheme isEqualToString:@"https"] || [url.scheme isEqualToString:@"http"];
}

- (BOOL)isWorkspaceBlobURL:(NSURL *)url {
    return [url.scheme isEqualToString:@"blob"] &&
           [self isWorkspaceURL:[NSURL URLWithString:[url.absoluteString substringFromIndex:5]]];
}

- (void)webView:(WKWebView *)webView decidePolicyForNavigationAction:(WKNavigationAction *)action
 decisionHandler:(void (^)(WKNavigationActionPolicy))decisionHandler {
    (void)webView;
    NSURL *url = action.request.URL;
    if (action.navigationType == WKNavigationTypeLinkActivated && [url.scheme.lowercaseString isEqualToString:@"ezerbot"]) {
        [self receiveDeepLink:url.absoluteString];
        decisionHandler(WKNavigationActionPolicyCancel); return;
    }
    if (action.shouldPerformDownload && ([self isWorkspaceURL:url] || [self isWorkspaceBlobURL:url])) {
        decisionHandler(WKNavigationActionPolicyDownload);
        return;
    }
    if ([self isWorkspaceURL:url] || [url.absoluteString isEqualToString:@"about:blank"]) {
        decisionHandler(WKNavigationActionPolicyAllow);
        return;
    }
    if (action.navigationType == WKNavigationTypeLinkActivated && [self isExternalWebURL:url]) {
        [[NSWorkspace sharedWorkspace] openURL:url];
    }
    decisionHandler(WKNavigationActionPolicyCancel);
}

- (WKWebView *)webView:(WKWebView *)webView createWebViewWithConfiguration:(WKWebViewConfiguration *)configuration
  forNavigationAction:(WKNavigationAction *)action windowFeatures:(WKWindowFeatures *)features {
    (void)configuration;
    (void)features;
    if (action.navigationType == WKNavigationTypeLinkActivated && [action.request.URL.scheme.lowercaseString isEqualToString:@"ezerbot"]) [self receiveDeepLink:action.request.URL.absoluteString];
    else if ([self isWorkspaceURL:action.request.URL]) [webView loadRequest:action.request];
    else if (action.navigationType == WKNavigationTypeLinkActivated && [self isExternalWebURL:action.request.URL]) {
        [[NSWorkspace sharedWorkspace] openURL:action.request.URL];
    }
    return nil;
}

- (void)webView:(WKWebView *)webView runOpenPanelWithParameters:(WKOpenPanelParameters *)parameters
 initiatedByFrame:(WKFrameInfo *)frame completionHandler:(void (^)(NSArray<NSURL *> *))completionHandler {
    // File access is user-mediated and limited to the local workspace, never an external frame.
    if (webView != self.webView || ![self isWorkspaceURL:frame.request.URL] || self.terminating) {
        completionHandler(nil);
        return;
    }
    NSOpenPanel *panel = [NSOpenPanel openPanel];
    panel.title = parameters.allowsDirectories ? @"Choisir un dossier" : @"Choisir des fichiers";
    panel.prompt = @"Choisir";
    panel.canChooseDirectories = parameters.allowsDirectories;
    panel.canChooseFiles = !parameters.allowsDirectories;
    panel.allowsMultipleSelection = parameters.allowsMultipleSelection;
    panel.treatsFilePackagesAsDirectories = NO;
    [panel beginSheetModalForWindow:self.window completionHandler:^(NSModalResponse result) {
        completionHandler(result == NSModalResponseOK ? panel.URLs : nil);
    }];
}

- (void)webView:(WKWebView *)webView navigationAction:(WKNavigationAction *)action didBecomeDownload:(WKDownload *)download {
    (void)webView;
    (void)action;
    download.delegate = self;
}

- (void)download:(WKDownload *)download decideDestinationUsingResponse:(NSURLResponse *)response
 suggestedFilename:(NSString *)suggestedFilename completionHandler:(void (^)(NSURL *))completionHandler {
    (void)download;
    (void)response;
    NSSavePanel *panel = [NSSavePanel savePanel];
    panel.title = @"Exporter l’espace Ezerbot";
    panel.prompt = @"Exporter";
    panel.allowedContentTypes = @[UTTypeJSON];
    panel.canCreateDirectories = YES;
    NSString *filename = suggestedFilename.lastPathComponent;
    panel.nameFieldStringValue = [filename.pathExtension.lowercaseString isEqualToString:@"json"]
                                  ? filename : @"ezerbot-workspace.json";
    [panel beginSheetModalForWindow:self.window completionHandler:^(NSModalResponse result) {
        completionHandler(result == NSModalResponseOK ? panel.URL : nil);
    }];
}

- (void)download:(WKDownload *)download didFailWithError:(NSError *)error resumeData:(NSData *)resumeData {
    (void)download;
    (void)resumeData;
    if (error.code == NSURLErrorCancelled || self.terminating) return;
    NSAlert *alert = [[NSAlert alloc] init];
    alert.messageText = @"L’export n’a pas pu être enregistré";
    alert.informativeText = error.localizedDescription;
    [alert beginSheetModalForWindow:self.window completionHandler:nil];
}

- (void)webView:(WKWebView *)webView didFailProvisionalNavigation:(WKNavigation *)navigation withError:(NSError *)error {
    (void)webView;
    (void)navigation;
    if (error.code == NSURLErrorCancelled || self.terminating) return;
    [self showStatus:@"L’espace local est indisponible" detail:error.localizedDescription failed:YES];
}

- (void)reloadPage:(id)sender {
    (void)sender;
    if (self.webView) [self.webView reload];
    else [self startBackend:nil];
}

- (void)openLogs:(id)sender {
    (void)sender;
    if (self.supportPath) [[NSWorkspace sharedWorkspace] openURL:
                          [NSURL fileURLWithPath:[self.supportPath stringByAppendingPathComponent:@"logs"] isDirectory:YES]];
}

- (void)stopBackend {
    [self.voice cancel];
    NSTask *task = self.backendTask;
    self.backendTask = nil;
    task.terminationHandler = nil;
    if (task.running) {
        [task terminate];
        NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:1];
        while (task.running && deadline.timeIntervalSinceNow > 0) usleep(20000);
        if (task.running) kill(task.processIdentifier, SIGKILL);
        [task waitUntilExit];
    }
    [self.logHandle closeFile];
    self.logHandle = nil;
}

- (BOOL)applicationShouldTerminateAfterLastWindowClosed:(NSApplication *)sender {
    (void)sender;
    return YES;
}

- (void)applicationWillTerminate:(NSNotification *)notification {
    (void)notification;
    self.terminating = YES;
    ++self.startupGeneration;
    [self stopBackend];
}

@end

int main(void) {
    @autoreleasepool {
        // The workspace and database created by the owned engine remain user-private.
        umask(0077);
        NSApplication *application = [NSApplication sharedApplication];
        [application setActivationPolicy:NSApplicationActivationPolicyRegular];
        GrokbotApplication *delegate = [[GrokbotApplication alloc] init];
        application.delegate = delegate;
        [application run];
    }
    return 0;
}
