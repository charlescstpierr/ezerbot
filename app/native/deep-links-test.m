// Compile as a plain command-line executable, never a registered .app bundle:
// clang -fobjc-arc -Wall -Wextra -Werror -mmacosx-version-min=12.3 \
//   -framework Cocoa -framework WebKit -framework UniformTypeIdentifiers \
//   -framework UserNotifications app/native/deep-links-test.m -o /tmp/ezerbot-links-test
#define main EzerbotApplicationMain
#import "main.m"
#undef main

@interface LinkTestView : NSObject
@property(nonatomic, strong) NSURL *URL;
@property(nonatomic, strong) NSMutableArray<NSString *> *scripts;
@property(nonatomic, copy) void (^completion)(id, NSError *);
@property(nonatomic) BOOL hold;
@end
@implementation LinkTestView
- (void)evaluateJavaScript:(NSString *)script completionHandler:(void (^)(id, NSError *))completion {
    [self.scripts addObject:script];
    if (self.hold) self.completion = completion;
    else completion(@YES, nil);
}
@end

int main(void) {
    @autoreleasepool {
        GrokbotApplication *shell = [[GrokbotApplication alloc] init];
        NSURL *link = [NSURL URLWithString:@"ezerbot://app/v1/open?text=Bonjour%20%22ami%22"];
        [shell application:(NSApplication *)(id)shell openURLs:@[link, link]];
        NSCAssert(shell.pendingDeepLinks.count == 1, @"Queue before startup and raw duplicates");
        LinkTestView *view = [[LinkTestView alloc] init];
        view.URL = [NSURL URLWithString:@"http://127.0.0.1:18798"];
        view.scripts = [NSMutableArray array];
        shell.backendURL = view.URL;
        shell.webView = (WKWebView *)(id)view;
        [shell flushDeepLinks];
        NSCAssert(view.scripts.count == 0, @"No delivery before renderer readiness");
        shell.deepLinksReady = YES;
        [shell flushDeepLinks];
        NSCAssert(view.scripts.count == 1 && shell.pendingDeepLinks.count == 0, @"Ready drains once");
        NSCAssert([view.scripts.firstObject containsString:@"EzerbotDeepLinks?.receive"], @"Same web entry point");
        view.hold = YES;
        [shell receiveDeepLink:@"ezerbot://app/v1/agent?id=grokbot"];
        void (^oldCompletion)(id, NSError *) = view.completion;
        [shell webView:shell.webView didStartProvisionalNavigation:nil];
        oldCompletion(@YES, nil);
        NSCAssert(shell.pendingDeepLinks.count == 1, @"Reload invalidates stale acknowledgment");
        view.hold = NO; shell.deepLinksReady = YES; [shell flushDeepLinks];
        NSCAssert(shell.pendingDeepLinks.count == 0, @"Queued link survives reload");
        view.URL = [NSURL URLWithString:@"https://example.invalid"];
        [shell receiveDeepLink:@"ezerbot://app/v1/settings?id=appearance"];
        NSCAssert(shell.pendingDeepLinks.count == 1, @"Never deliver into an external origin");
        shell.webView = nil; // It is a deliberately minimal test double.
        puts("Cocoa deep links: cold boot, dedup, readiness, reload, origin — OK");
    }
    return 0;
}
