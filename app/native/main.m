#import <Cocoa/Cocoa.h>
#import <UniformTypeIdentifiers/UniformTypeIdentifiers.h>
#import <WebKit/WebKit.h>
#import "notifications.h"
#import "voice.h"
#import "voice-session.h"
#import "screen.h"
#import "floating.h"
#import "demonstration.h"
#import <errno.h>
#import <poll.h>
#include "shell_stop.h"
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
@property(nonatomic, strong) EzerbotVoiceSession *voiceSession;
@property(nonatomic, strong) EzerbotFloating *floating;
@property(nonatomic, strong) NSStatusItem *statusItem;
@property(nonatomic, copy) NSString *visibleConversationID;
@property(nonatomic) BOOL conversationVisible;
@property(nonatomic, copy) NSString *pendingNotificationID;
@property(nonatomic, strong) NSMutableArray<NSString *> *pendingDeepLinks;
@property(nonatomic) BOOL deepLinksReady;
@property(nonatomic) BOOL deepLinksFlushing;
@property(nonatomic) NSUInteger deepLinksEpoch;
@end

static int shell_task_still_running(const ShellChild *child) {
    NSTask *task = (__bridge NSTask *)child->context;
    return task.running ? 1 : 0;
}

static void shell_task_reap(const ShellChild *child) {
    NSTask *task = (__bridge NSTask *)child->context;
    [task waitUntilExit];
}

@implementation GrokbotApplication

- (void)applicationDidFinishLaunching:(NSNotification *)notification {
    (void)notification;
    self.notifier = [[EzerbotNotifier alloc] init];
    self.voice = [[EzerbotVoice alloc] init];
    self.voiceSession = [[EzerbotVoiceSession alloc] init];
#if defined(EZERBOT_RELEASE)
    [UNUserNotificationCenter currentNotificationCenter].delegate = self;
#endif
    self.floating = [[EzerbotFloating alloc] init];
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
    NSMenuItem *floating = [fileMenu addItemWithTitle:@"Ouvrir le panneau flottant" action:@selector(showFloating:) keyEquivalent:@""];
    floating.target = self;
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
    self.statusItem = [NSStatusBar.systemStatusBar statusItemWithLength:NSVariableStatusItemLength];
    self.statusItem.button.title = @"Ezerbot";
    self.statusItem.button.toolTip = @"Panneau flottant d’Ezerbot";
    NSMenu *statusMenu = [[NSMenu alloc] initWithTitle:@"Ezerbot"];
    NSMenuItem *openPanel = [statusMenu addItemWithTitle:@"Ouvrir le panneau flottant" action:@selector(showFloating:) keyEquivalent:@""];
    openPanel.target = self;
    NSMenuItem *openMain = [statusMenu addItemWithTitle:@"Ouvrir Ezerbot" action:@selector(showMain:) keyEquivalent:@""];
    openMain.target = self;
    [statusMenu addItem:[NSMenuItem separatorItem]];
    [statusMenu addItemWithTitle:@"Quitter Ezerbot" action:@selector(terminate:) keyEquivalent:@""];
    self.statusItem.menu = statusMenu;
}

- (void)showFloating:(id)sender { (void)sender; [self.floating show]; }
- (void)showMain:(id)sender {
    (void)sender; [self.window deminiaturize:nil]; [self.window makeKeyAndOrderFront:nil];
    [NSApp activateIgnoringOtherApps:YES];
}
- (BOOL)windowShouldClose:(NSWindow *)sender {
    if (sender == self.window) { [sender orderOut:nil]; return NO; }
    return YES;
}
- (BOOL)applicationShouldHandleReopen:(NSApplication *)application hasVisibleWindows:(BOOL)visible {
    (void)application; (void)visible; [self showMain:nil]; return YES;
}

- (void)showStatus:(NSString *)title detail:(NSString *)detail failed:(BOOL)failed {
    [self.voice cancel];
    [self.voiceSession cancel];
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
    [self showStatus:@"Ezerbot" detail:@"Ouverture de votre espace local… Une routine en cours enregistre son état avant l’ouverture." failed:NO];
    NSFileManager *files = [NSFileManager defaultManager];
    NSError *error = nil;
    NSURL *applicationSupport = nil;
#if !defined(EZERBOT_RELEASE)
    // HOME alone does not redirect Cocoa's Application Support lookup. A test
    // copy must opt in before any file is created in the owner's real space.
    NSString *testHome = NSProcessInfo.processInfo.environment[@"EZERBOT_TEST_HOME"];
    if (testHome.length) {
        if (![testHome isAbsolutePath] || ![testHome isEqual:NSProcessInfo.processInfo.environment[@"HOME"]]) {
            [self failStartup:@"Le HOME jetable du test est invalide." generation:generation];
            return;
        }
        applicationSupport = [NSURL fileURLWithPath:[testHome stringByAppendingPathComponent:@"Library/Application Support"] isDirectory:YES];
    } else
#endif
    {
        applicationSupport = [files URLForDirectory:NSApplicationSupportDirectory inDomain:NSUserDomainMask
                                  appropriateForURL:nil create:YES error:&error];
    }
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
    NSMutableDictionary *environment = [[[NSProcessInfo processInfo] environment] mutableCopy];
#ifdef EZERBOT_RELEASE
    // Recherche discrète au lancement. Le bouton des Réglages reste disponible sans cette variable.
    environment[@"EZERBOT_QUIET_UPDATE_CHECK"] = @"1";
#endif
    task.environment = environment;
    __weak GrokbotApplication *weakSelf = self;
    task.terminationHandler = ^(NSTask *finished) {
        dispatch_async(dispatch_get_main_queue(), ^{
            GrokbotApplication *strongSelf = weakSelf;
            if (!strongSelf || strongSelf.backendTask != finished || strongSelf.terminating) return;
            // 86 : le moteur a installé une nouvelle application et demande le redémarrage.
            if (finished.terminationReason == NSTaskTerminationReasonExit && finished.terminationStatus == 86) {
                [strongSelf relaunchUpdatedApplication];
                return;
            }
            [strongSelf failStartup:@"Le moteur s’est arrêté. Les détails sont dans le journal local."
                          generation:generation];
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
        NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:240];
        BOOL complete = NO;
        while (!complete && line.length <= 8192) {
            int remaining = (int)MAX(0, [deadline timeIntervalSinceNow] * 1000);
            if (remaining == 0) { problem = @"Le moteur reste occupé après quatre minutes. Réessayez après la fin de la routine."; break; }
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
    self.floating.backendURL = url;
    self.floating.mainWindow = self.window;
    self.floating.geometryPath = [self.supportPath stringByAppendingPathComponent:@"floating-panel.plist"];
    __weak GrokbotApplication *weakSelf = self;
    self.floating.openConversation = ^(NSString *identifier, void (^reply)(BOOL, NSString *)) {
        GrokbotApplication *shell = weakSelf;
        if (!shell || ![shell isWorkspaceURL:shell.webView.URL]) { reply(NO,@"La fenêtre principale n’est pas prête."); return; }
        [shell.webView callAsyncJavaScript:@"return await window.EzerbotFloatingMain.openConversation(id)"
            arguments:@{@"id":identifier} inFrame:nil inContentWorld:WKContentWorld.pageWorld completionHandler:^(id result, NSError *error) {
                NSDictionary *outcome = !error && [result isKindOfClass:NSDictionary.class] ? result : nil;
                if ([outcome[@"opened"] isEqual:@YES]) { [shell showMain:nil]; reply(YES,nil); }
                else reply(NO, [outcome[@"reason"] isKindOfClass:NSString.class] ? outcome[@"reason"] : nil);
            }];
    };
    WKWebViewConfiguration *configuration = [[WKWebViewConfiguration alloc] init];
    configuration.websiteDataStore = [WKWebsiteDataStore nonPersistentDataStore];
    [configuration.userContentController addScriptMessageHandlerWithReply:self contentWorld:WKContentWorld.pageWorld name:@"ezerbotNotifications"];
    [configuration.userContentController addScriptMessageHandlerWithReply:self contentWorld:WKContentWorld.pageWorld name:@"ezerbotVoice"];
    [configuration.userContentController addScriptMessageHandlerWithReply:self contentWorld:WKContentWorld.pageWorld name:@"ezerbotVoiceSession"];
    [configuration.userContentController addScriptMessageHandlerWithReply:self contentWorld:WKContentWorld.pageWorld name:@"ezerbotDeepLinks"];
    [configuration.userContentController addScriptMessageHandlerWithReply:self contentWorld:WKContentWorld.pageWorld name:@"ezerbotFloating"];
    [configuration.userContentController addScriptMessageHandlerWithReply:self contentWorld:WKContentWorld.pageWorld name:@"ezerbotImages"];
    [configuration.userContentController addScriptMessageHandlerWithReply:self contentWorld:WKContentWorld.pageWorld name:@"ezerbotRatings"];
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
    [self.floating close];
    self.deepLinksReady = NO; self.deepLinksFlushing = NO; ++self.deepLinksEpoch;
    [self.voice cancel]; [self.voiceSession cancel];
    self.window.title = @"Ezerbot"; self.window.titleVisibility = NSWindowTitleHidden;
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
    if ([message.name isEqual:@"ezerbotRatings"]) {
        NSUserDefaults *defaults = NSUserDefaults.standardUserDefaults;
        if ([command isEqual:@"read"]) {
            replyHandler([defaults stringForKey:@"ezerbot.notations"], nil); return;
        }
        if ([command isEqual:@"write"] && [body[@"value"] isKindOfClass:NSString.class]) {
            NSData *data = [body[@"value"] dataUsingEncoding:NSUTF8StringEncoding];
            if ([[NSJSONSerialization JSONObjectWithData:data options:0 error:nil] isKindOfClass:NSArray.class]) {
                [defaults setObject:body[@"value"] forKey:@"ezerbot.notations"];
                replyHandler(@YES, nil); return;
            }
        }
        replyHandler(nil, @"Notation invalide."); return;
    }
    if ([message.name isEqual:@"ezerbotFloating"]) {
        if ([command isEqual:@"show"]) { [self.floating show]; replyHandler(@YES,nil); return; }
        if ([command isEqual:@"configure"] && [body[@"shortcut"] isKindOfClass:NSString.class]) {
            replyHandler([self.floating configure:body[@"shortcut"]],nil); return;
        }
        replyHandler(nil,@"Commande du panneau inconnue."); return;
    }
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
    if ([message.name isEqual:@"ezerbotImages"]) {
        if (![command isEqual:@"copy"]) { replyHandler(nil, @"Commande d’image inconnue."); return; }
        NSString *mime = [body[@"mime"] isKindOfClass:NSString.class] ? [body[@"mime"] lowercaseString] : @"";
        NSString *data = [body[@"data"] isKindOfClass:NSString.class] ? body[@"data"] : @"";
        NSSet *types = [NSSet setWithArray:@[@"image/png", @"image/jpeg", @"image/gif", @"image/webp", @"image/bmp"]];
        if (![types containsObject:mime] || data.length == 0 || data.length > 27962028) { replyHandler(nil, @"Image illisible."); return; }
        NSData *bytes = [[NSData alloc] initWithBase64EncodedString:data options:NSDataBase64DecodingIgnoreUnknownCharacters];
        if (!bytes || bytes.length == 0 || bytes.length > 20 * 1024 * 1024) { replyHandler(nil, @"Image illisible."); return; }
        NSImage *image = [[NSImage alloc] initWithData:bytes];
        if (!image.isValid) { replyHandler(nil, @"Image illisible."); return; }
        NSPasteboard *board = NSPasteboard.generalPasteboard;
        [board clearContents];
        if (![board writeObjects:@[image]]) { replyHandler(nil, @"L’image n’a pas pu être copiée."); return; }
        replyHandler(@{@"ok": @YES}, nil);
        return;
    }
    if ([message.name isEqual:@"ezerbotVoice"]) {
        if ([command isEqual:@"start"] && [EzerbotVoice captureStatusActive:self.voiceSession.status]) { reply(@{@"status":@"microphone_busy"}); return; }
        [self.voice command:body reply:^(NSDictionary *result) {
            BOOL recording = self.voice.meeting && [@[@"authorizing",@"recording",@"paused",@"transcribing"] containsObject:self.voice.status];
            self.window.title = recording ? @"● Réunion · Ezerbot" : @"Ezerbot";
            self.window.titleVisibility = recording ? NSWindowTitleVisible : NSWindowTitleHidden;
            reply(result);
        }]; return;
    }
    if ([message.name isEqual:@"ezerbotVoiceSession"]) {
        if ([command isEqual:@"start"] && [EzerbotVoice captureStatusActive:self.voice.status]) { reply(@{@"status":@"microphone_busy"}); return; }
        [self.voiceSession command:body reply:reply]; return;
    }
    if ([command isEqual:@"screenSettings"]) {
        NSDictionary *panes = @{@"accessibility":@"Privacy_Accessibility", @"recording":@"Privacy_ScreenCapture", @"input":@"Privacy_ListenEvent"};
        NSString *pane = [body[@"pane"] isKindOfClass:NSString.class] ? panes[body[@"pane"]] : nil;
        if (!pane) { replyHandler(nil, @"Panneau système inconnu."); return; }
#if defined(EZERBOT_RELEASE)
        [[NSWorkspace sharedWorkspace] openURL:[NSURL URLWithString:[@"x-apple.systempreferences:com.apple.preference.security?" stringByAppendingString:pane]]];
        reply(@{@"status":@"ok"});
#else
        reply(@{@"status":@"recording"});
#endif
        return;
    }
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
    NSString *mime = response.MIMEType.lowercaseString ?: @"";
    NSString *filename = suggestedFilename.lastPathComponent;
    BOOL image = [mime hasPrefix:@"image/"];
    NSSavePanel *panel = [NSSavePanel savePanel];
    panel.canCreateDirectories = YES;
    if (image) {
        panel.title = @"Enregistrer l’image";
        panel.prompt = @"Enregistrer";
        UTType *type = [UTType typeWithMIMEType:response.MIMEType];
        if (type) panel.allowedContentTypes = @[type];
        panel.nameFieldStringValue = filename.length ? filename : @"image";
    } else {
        panel.title = @"Exporter l’espace Ezerbot";
        panel.prompt = @"Exporter";
        panel.allowedContentTypes = @[UTTypeJSON];
        panel.nameFieldStringValue = [filename.pathExtension.lowercaseString isEqualToString:@"json"]
                                      ? filename : @"ezerbot-workspace.json";
    }
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

- (void)relaunchUpdatedApplication {
    NSString *bundle = [[NSBundle mainBundle] bundlePath];
    NSTask *relaunch = [[NSTask alloc] init];
    relaunch.executableURL = [NSURL fileURLWithPath:@"/bin/bash"];
    // $0 est le nom, $1 le pid, $2 le bundle. on ignore SIGHUP pour survivre à la fermeture.
    relaunch.arguments = @[
        @"-c",
        @"trap '' HUP; while /bin/kill -0 \"$1\" 2>/dev/null; do /bin/sleep 0.2; done; exec /usr/bin/open \"$2\"",
        @"ezerbot-relaunch",
        [NSString stringWithFormat:@"%d", getpid()],
        bundle
    ];
    relaunch.standardOutput = [NSFileHandle fileHandleWithNullDevice];
    relaunch.standardError = [NSFileHandle fileHandleWithNullDevice];
    if ([relaunch launchAndReturnError:nil]) {
        self.terminating = YES;
        [NSApp terminate:nil];
    }
}

// Réunions : un bot Recall invité doit sortir de l'appel avant que le moteur
// ne soit arrêté. Le moteur n'a que SIGTERM puis une seconde, trop peu pour le réseau.
// Sans réunion, la réponse est immédiate ; la route attend au plus 12 s une invitation sans réponse, et un moteur muet ne retient pas la fermeture plus de 16 s.
- (void)leaveRecallMeetings {
    NSURL *base = self.backendURL;
    if (!base || !self.backendTask.running) return;
    NSURLComponents *components = [NSURLComponents componentsWithURL:base resolvingAgainstBaseURL:NO];
    components.path = @"/api/recall/shutdown";
    NSMutableURLRequest *request = [NSMutableURLRequest requestWithURL:components.URL];
    request.HTTPMethod = @"POST";
    request.HTTPBody = [@"{}" dataUsingEncoding:NSUTF8StringEncoding];
    [request setValue:@"application/json" forHTTPHeaderField:@"Content-Type"];
    [request setValue:@"local" forHTTPHeaderField:@"X-Grokbot-Request"];
    NSURLSessionConfiguration *configuration = NSURLSessionConfiguration.ephemeralSessionConfiguration;
    configuration.timeoutIntervalForRequest = 15;
    NSURLSession *session = [NSURLSession sessionWithConfiguration:configuration];
    dispatch_semaphore_t finished = dispatch_semaphore_create(0);
    [[session dataTaskWithRequest:request completionHandler:^(NSData *data, NSURLResponse *response, NSError *error) {
        (void)data; (void)response; (void)error;
        dispatch_semaphore_signal(finished);
    }] resume];
    dispatch_semaphore_wait(finished, dispatch_time(DISPATCH_TIME_NOW, 16 * NSEC_PER_SEC));
    [session invalidateAndCancel];
}

- (void)stopBackend {
    [self leaveRecallMeetings];
    [self.voice cancel];
    [self.voiceSession cancel];
    [self.floating invalidate];
    self.floating.backendURL = nil;
    NSTask *task = self.backendTask;
    self.backendTask = nil;
    task.terminationHandler = nil;
    if (task.running) {
        ShellChild child;
        child.pid = task.processIdentifier;
        child.context = (__bridge void *)task;
        child.still_running = shell_task_still_running;
        child.reap = shell_task_reap;
        shell_stop_child(&child);
    }
    [self.logHandle closeFile];
    self.logHandle = nil;
}

- (BOOL)applicationShouldTerminateAfterLastWindowClosed:(NSApplication *)sender {
    (void)sender;
    return NO;
}

- (void)applicationWillTerminate:(NSNotification *)notification {
    (void)notification;
    self.terminating = YES;
    ++self.startupGeneration;
    [self stopBackend];
}

@end

int main(int argc, const char *argv[]) {
    @autoreleasepool {
        if (argc > 1 && strcmp(argv[1], "--demonstration-helper") == 0) return EZDemoMain(argc, argv);
        if (argc > 1 && (strcmp(argv[1], "--screen-helper") == 0 || strcmp(argv[1], "--screen-permissions") == 0)) return EZScreenMain(argc, argv);
        if (argc == 3 && strcmp(argv[1], "--notify-background") == 0) {
            // Same signed app identity and privacy-safe notifier as the WebView;
            // no window, backend, permission prompt or personal workspace access.
            NSData *data = [[NSString stringWithUTF8String:argv[2]] dataUsingEncoding:NSUTF8StringEncoding];
            NSDictionary *event = [NSJSONSerialization JSONObjectWithData:data options:0 error:nil];
            if (![event isKindOfClass:NSDictionary.class] || ![event[@"id"] isKindOfClass:NSString.class]
                || ![event[@"conversation_id"] isKindOfClass:NSString.class]
                || ![@[@"Routine terminée", @"Échec de la routine", @"Autorisation demandée", @"Informations demandées", @"Secret demandé", @"Sous-tâche terminée", @"Réponse terminée"] containsObject:event[@"title"]]) return 1;
            EzerbotNotifier *notifier = [[EzerbotNotifier alloc] init];
            __block BOOL finished = NO;
            __block int code = 1;
            [notifier deliver:event reply:^(NSDictionary *result) {
                NSString *status = result[@"status"];
                dispatch_async(dispatch_get_main_queue(), ^{
                    code = [@[@"delivered", @"denied", @"undetermined", @"recording", @"disabled"] containsObject:status] ? 0 : 1;
                    finished = YES;
                });
            }];
            NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:5];
            while (!finished && deadline.timeIntervalSinceNow > 0) {
                [[NSRunLoop currentRunLoop] runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.02]];
            }
            return finished ? code : 1;
        }
        // The workspace and database created by the owned engine remain user-private.
        umask(0077);
        NSApplication *application = [NSApplication sharedApplication];
        [application setActivationPolicy:NSApplicationActivationPolicyRegular];
        GrokbotApplication *delegate = [[GrokbotApplication alloc] init];
        application.delegate = delegate;
        int quit_fd = -1;
        if (shell_stop_arm(&quit_fd) != 0) return 1;
        static dispatch_source_t quit_source = nil;
        quit_source = dispatch_source_create(DISPATCH_SOURCE_TYPE_READ, quit_fd, 0, dispatch_get_main_queue());
        if (quit_source == nil) return 1;
        dispatch_source_set_event_handler(quit_source, ^{
            if (shell_stop_take_quit(quit_fd) && !delegate.terminating) [NSApp terminate:nil];
        });
        dispatch_resume(quit_source);
        [application run];
    }
    return 0;
}
