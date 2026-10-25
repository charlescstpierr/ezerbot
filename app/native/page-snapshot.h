#import <Cocoa/Cocoa.h>
#import <WebKit/WebKit.h>
#import <poll.h>
#import <unistd.h>

static const NSUInteger EZPageSnapshotLimit = 6 * 1024 * 1024;
static BOOL EZPageLocalURL(NSURL *url) {
    NSString *host = url.host.lowercaseString;
    return [@[@"http", @"https"] containsObject:url.scheme.lowercaseString]
        && [@[@"localhost", @"127.0.0.1", @"::1"] containsObject:host]
        && url.user == nil && url.password == nil;
}

typedef NS_ENUM(NSUInteger, EZPageSnapshotState) { EZPageInitial, EZPageLoading, EZPageCapturing, EZPageFinished };
@interface EZPageSnapshot : NSObject <WKNavigationDelegate, WKUIDelegate>
@property(nonatomic, strong) WKWebView *view;
@property(nonatomic, strong) NSDictionary *result;
@property(nonatomic) EZPageSnapshotState state;
@property(nonatomic) BOOL html;
- (void)fail:(NSString *)message;
@end

@implementation EZPageSnapshot
- (void)fail:(NSString *)message {
    if (self.state == EZPageFinished) return;
    self.state = EZPageFinished;
    self.result = @{@"error":message};
    [self.view stopLoading];
}
- (void)webView:(WKWebView *)view decidePolicyForNavigationAction:(WKNavigationAction *)action decisionHandler:(void (^)(WKNavigationActionPolicy))reply {
    (void)view;
    BOOL allowed = self.html
        ? self.state == EZPageInitial && action.targetFrame.isMainFrame && [action.request.URL.absoluteString isEqual:@"about:blank"]
        : EZPageLocalURL(action.request.URL);
    if (!allowed || !action.targetFrame) {
        reply(WKNavigationActionPolicyCancel);
        [self fail:@"Cette navigation n'est pas autorisée pour la photo."];
        return;
    }
    if (self.state == EZPageInitial) self.state = EZPageLoading;
    reply(WKNavigationActionPolicyAllow);
}
- (void)webView:(WKWebView *)view decidePolicyForNavigationResponse:(WKNavigationResponse *)response decisionHandler:(void (^)(WKNavigationResponsePolicy))reply {
    (void)view;
    BOOL allowed = self.html ? response.isForMainFrame && [response.response.URL.absoluteString isEqual:@"about:blank"] : EZPageLocalURL(response.response.URL);
    reply(allowed && response.canShowMIMEType ? WKNavigationResponsePolicyAllow : WKNavigationResponsePolicyCancel);
    if (!allowed || !response.canShowMIMEType) [self fail:@"La réponse de cette adresse ne peut pas être photographiée."];
}
- (void)webView:(WKWebView *)view didReceiveServerRedirectForProvisionalNavigation:(WKNavigation *)navigation {
    (void)navigation;
    if (self.html || !EZPageLocalURL(view.URL)) [self fail:@"La redirection sort des adresses autorisées."];
}
- (WKWebView *)webView:(WKWebView *)view createWebViewWithConfiguration:(WKWebViewConfiguration *)configuration forNavigationAction:(WKNavigationAction *)action windowFeatures:(WKWindowFeatures *)features {
    (void)view; (void)configuration; (void)action; (void)features;
    [self fail:@"Une fenêtre supplémentaire ne peut pas être photographiée."];
    return nil;
}
- (void)webView:(WKWebView *)view didFailProvisionalNavigation:(WKNavigation *)navigation withError:(NSError *)error {
    (void)view; (void)navigation; (void)error;
    [self fail:@"Cette Page n'a pas pu être chargée."];
}
- (void)webView:(WKWebView *)view didFailNavigation:(WKNavigation *)navigation withError:(NSError *)error {
    [self webView:view didFailProvisionalNavigation:navigation withError:error];
}
- (void)webViewWebContentProcessDidTerminate:(WKWebView *)view {
    (void)view; [self fail:@"Le rendu de la Page a été interrompu."];
}
- (void)webView:(WKWebView *)view didReceiveAuthenticationChallenge:(NSURLAuthenticationChallenge *)challenge completionHandler:(void (^)(NSURLSessionAuthChallengeDisposition, NSURLCredential *))reply {
    (void)view;
    if ([challenge.protectionSpace.authenticationMethod isEqual:NSURLAuthenticationMethodServerTrust]) {
        reply(NSURLSessionAuthChallengePerformDefaultHandling, nil); return;
    }
    reply(NSURLSessionAuthChallengeCancelAuthenticationChallenge, nil);
    [self fail:@"Cette adresse demande une authentification."];
}
- (void)webView:(WKWebView *)view runJavaScriptAlertPanelWithMessage:(NSString *)message initiatedByFrame:(WKFrameInfo *)frame completionHandler:(void (^)(void))reply {
    (void)view; (void)message; (void)frame; reply();
}
- (void)webView:(WKWebView *)view runJavaScriptConfirmPanelWithMessage:(NSString *)message initiatedByFrame:(WKFrameInfo *)frame completionHandler:(void (^)(BOOL))reply {
    (void)view; (void)message; (void)frame; reply(NO);
}
- (void)webView:(WKWebView *)view runJavaScriptTextInputPanelWithPrompt:(NSString *)prompt defaultText:(NSString *)text initiatedByFrame:(WKFrameInfo *)frame completionHandler:(void (^)(NSString *))reply {
    (void)view; (void)prompt; (void)text; (void)frame; reply(nil);
}
- (void)webView:(WKWebView *)view requestMediaCapturePermissionForOrigin:(WKSecurityOrigin *)origin initiatedByFrame:(WKFrameInfo *)frame type:(WKMediaCaptureType)type decisionHandler:(void (^)(WKPermissionDecision))reply {
    (void)view; (void)origin; (void)frame; (void)type; reply(WKPermissionDecisionDeny);
}
- (void)webView:(WKWebView *)view didFinishNavigation:(WKNavigation *)navigation {
    (void)navigation;
    if (self.state != EZPageLoading) return;
    self.state = EZPageCapturing;
    WKSnapshotConfiguration *config = [WKSnapshotConfiguration new];
    config.rect = NSMakeRect(0, 0, 1280, 800);
    config.snapshotWidth = @1280;
    config.afterScreenUpdates = NO;
    [view takeSnapshotWithConfiguration:config completionHandler:^(NSImage *image, NSError *error) {
        if (self.state == EZPageFinished) return;
        if (!image || error) { [self fail:@"La photo de cette Page a échoué."]; return; }
        NSBitmapImageRep *bitmap = [[NSBitmapImageRep alloc] initWithBitmapDataPlanes:NULL pixelsWide:1280 pixelsHigh:800 bitsPerSample:8 samplesPerPixel:4 hasAlpha:YES isPlanar:NO colorSpaceName:NSDeviceRGBColorSpace bytesPerRow:0 bitsPerPixel:0];
        [NSGraphicsContext saveGraphicsState];
        [NSGraphicsContext setCurrentContext:[NSGraphicsContext graphicsContextWithBitmapImageRep:bitmap]];
        [image drawInRect:NSMakeRect(0, 0, 1280, 800)];
        [NSGraphicsContext restoreGraphicsState];
        NSData *png = [bitmap representationUsingType:NSBitmapImageFileTypePNG properties:@{}];
        if (!png || png.length > 4 * 1024 * 1024) { [self fail:@"La photo de cette Page est trop volumineuse."]; return; }
        self.result = @{@"image":[@"data:image/png;base64," stringByAppendingString:[png base64EncodedStringWithOptions:0]], @"width":@1280, @"height":@800};
        self.state = EZPageFinished;
    }];
}
@end

static NSDictionary *EZPageSnapshotRender(NSDictionary *request, NSDate *deadline) {
    BOOL html = [request[@"html"] isKindOfClass:NSString.class];
    BOOL url = [request[@"url"] isKindOfClass:NSString.class];
    if (![@[@"light", @"dark"] containsObject:request[@"theme"]] || ![request[@"width"] isEqual:@1280] || ![request[@"height"] isEqual:@800] || html == url || (request[@"html"] != nil && !html) || (request[@"url"] != nil && !url))
        return @{@"error":@"La photo attend un thème, 1280 par 800 et une seule Page ou adresse."};
    NSURL *target = url ? [NSURL URLWithString:request[@"url"]] : nil;
    if (url && !EZPageLocalURL(target)) return @{@"error":@"Seule une adresse locale sans identifiants peut être photographiée."};
    [NSApplication sharedApplication];
    [NSApp setActivationPolicy:NSApplicationActivationPolicyProhibited];
    EZPageSnapshot *job = [EZPageSnapshot new];
    job.html = html;
    WKWebViewConfiguration *configuration = [WKWebViewConfiguration new];
    configuration.websiteDataStore = [WKWebsiteDataStore nonPersistentDataStore];
    configuration.preferences.javaScriptCanOpenWindowsAutomatically = YES;
    configuration.mediaTypesRequiringUserActionForPlayback = WKAudiovisualMediaTypeAll;
    job.view = [[WKWebView alloc] initWithFrame:NSMakeRect(0, 0, 1280, 800) configuration:configuration];
    job.view.appearance = [NSAppearance appearanceNamed:[request[@"theme"] isEqual:@"dark"] ? NSAppearanceNameDarkAqua : NSAppearanceNameAqua];
    job.view.navigationDelegate = job;
    job.view.UIDelegate = job;
    // Block remote resources before WebKit can follow a redirect or load a frame.
    NSMutableArray *rules = [NSMutableArray arrayWithObject:@{@"trigger":@{@"url-filter":@".*"}, @"action":@{@"type":@"block"}}];
    if (url) for (NSString *host in @[@"localhost", @"127\\.0\\.0\\.1", @"\\[::1\\]"]) {
        NSString *filter = [NSString stringWithFormat:@"^https?://%@(:[0-9]+)?/", host];
        [rules addObject:@{@"trigger":@{@"url-filter":filter}, @"action":@{@"type":@"ignore-previous-rules"}}];
    }
    NSData *ruleData = [NSJSONSerialization dataWithJSONObject:rules options:0 error:nil];
    NSString *ruleJSON = [[NSString alloc] initWithData:ruleData encoding:NSUTF8StringEncoding];
    [[WKContentRuleListStore defaultStore] compileContentRuleListForIdentifier:html ? @"page-snapshot-html" : @"page-snapshot-local" encodedContentRuleList:ruleJSON completionHandler:^(WKContentRuleList *list, NSError *error) {
        if (job.state == EZPageFinished) return;
        if (!list || error) { [job fail:@"La protection des adresses n'a pas pu démarrer."]; return; }
        [job.view.configuration.userContentController addContentRuleList:list];
        if (html) [job.view loadHTMLString:request[@"html"] baseURL:nil];
        else [job.view loadRequest:[NSURLRequest requestWithURL:target]];
    }];
    while (!job.result && deadline.timeIntervalSinceNow > 0)
        [[NSRunLoop currentRunLoop] runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.01]];
    if (!job.result) [job fail:@"La Page a mis trop de temps à répondre."];
    job.view.navigationDelegate = nil;
    job.view.UIDelegate = nil;
    return job.result;
}

static int EZPageSnapshotMain(void) {
    NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:10];
    NSMutableData *input = [NSMutableData data];
    BOOL line = NO;
    while (!line && input.length < EZPageSnapshotLimit && deadline.timeIntervalSinceNow > 0) {
        struct pollfd fd = {STDIN_FILENO, POLLIN, 0};
        if (poll(&fd, 1, 20) <= 0) continue;
        unsigned char buffer[4096];
        ssize_t count = read(STDIN_FILENO, buffer, MIN(sizeof(buffer), EZPageSnapshotLimit - input.length));
        if (count <= 0) break;
        unsigned char *end = memchr(buffer, '\n', (size_t)count);
        [input appendBytes:buffer length:end ? (NSUInteger)(end - buffer) : (NSUInteger)count];
        line = end != NULL;
    }
    id request = line ? [NSJSONSerialization JSONObjectWithData:input options:0 error:nil] : nil;
    NSDictionary *result = [request isKindOfClass:NSDictionary.class] ? EZPageSnapshotRender(request, deadline) : @{@"error":@"Une ligne JSON valide est requise pour la photo."};
    NSData *output = [NSJSONSerialization dataWithJSONObject:result options:0 error:nil];
    fwrite(output.bytes, 1, output.length, stdout);
    fputc('\n', stdout);
    return result[@"error"] ? 1 : 0;
}
