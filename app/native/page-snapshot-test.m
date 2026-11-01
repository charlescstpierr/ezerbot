#import "page-snapshot.h"
#import <arpa/inet.h>
#import <sys/socket.h>

@interface EZSnapshotFrame : WKFrameInfo
@property(nonatomic) BOOL main;
@end
@implementation EZSnapshotFrame
- (BOOL)isMainFrame { return self.main; }
@end
@interface EZSnapshotAction : WKNavigationAction
@property(nonatomic, strong) NSURLRequest *testRequest;
@property(nonatomic, strong) WKFrameInfo *frame;
@end
@implementation EZSnapshotAction
- (NSURLRequest *)request { return self.testRequest; }
- (WKFrameInfo *)targetFrame { return self.frame; }
@end
@interface EZSnapshotResponse : WKNavigationResponse
@property(nonatomic, strong) NSURLResponse *testResponse;
@end
@implementation EZSnapshotResponse
- (NSURLResponse *)response { return self.testResponse; }
- (BOOL)isForMainFrame { return YES; }
- (BOOL)canShowMIMEType { return YES; }
@end

static NSDictionary *render(NSString *key, NSString *value, NSString *theme) {
    return EZPageSnapshotRender(@{key:value, @"theme":theme, @"width":@1280, @"height":@800}, [NSDate dateWithTimeIntervalSinceNow:10]);
}
static NSData *image(NSDictionary *result) {
    NSCAssert(!result[@"error"], @"render failed: %@", result[@"error"]);
    NSString *data = result[@"image"];
    NSData *png = [[NSData alloc] initWithBase64EncodedString:[data substringFromIndex:22] options:0];
    NSBitmapImageRep *bitmap = [NSBitmapImageRep imageRepWithData:png];
    NSCAssert(bitmap.pixelsWide == 1280 && bitmap.pixelsHigh == 800, @"exact pixel dimensions");
    NSColor *corner = [[bitmap colorAtX:10 y:10] colorUsingColorSpace:NSColorSpace.deviceRGBColorSpace];
    NSColor *inside = [[bitmap colorAtX:100 y:100] colorUsingColorSpace:NSColorSpace.deviceRGBColorSpace];
    NSCAssert(![corner isEqual:inside], @"real content must differ from the background");
    return png;
}

int main(void) {
    @autoreleasepool {
        (void)EZPageSnapshotMain;
        NSString *html = @"<style>:root{color-scheme:light dark}body{background:Canvas;color:CanvasText;margin:0}div{position:absolute;left:80px;top:80px;width:200px;height:200px;background:#f00}</style><h1>Vraie Page</h1><div></div>";
        NSData *light = image(render(@"html", html, @"light"));
        NSData *dark = image(render(@"html", html, @"dark"));
        NSCAssert(![light isEqual:dark], @"forced themes produce distinct pixels");
        for (NSWindow *window in NSApp.windows) NSCAssert(!window.visible, @"no visible window");
        WKWebView *policyView = [[WKWebView alloc] init];
        for (NSString *raw in @[@"https://example.com/", @"file:///etc/passwd", @"data:text/html,x", @"javascript:alert(1)", @"ftp://localhost/", @"http://user@localhost/", @"http://user:pass@127.0.0.1/", @"http://localhost.example/", @"http://localhost./", @"http://127.0.0.1.nip.io/", @"http://127.0.0.2/", @"http://[::2]/"]) {
            NSCAssert(render(@"url", raw, @"light")[@"error"], @"initial URL refused");
            for (NSNumber *main in @[@YES, @NO]) {
                EZPageSnapshot *job = [EZPageSnapshot new];
                EZSnapshotAction *action = [EZSnapshotAction new];
                EZSnapshotFrame *frame = [EZSnapshotFrame new]; frame.main = main.boolValue;
                action.frame = frame; action.testRequest = [NSURLRequest requestWithURL:[NSURL URLWithString:raw]];
                [job webView:policyView decidePolicyForNavigationAction:action decisionHandler:^(WKNavigationActionPolicy policy) {
                    NSCAssert(policy == WKNavigationActionPolicyCancel, @"link and subframe refused");
                }];
                NSCAssert(job.result[@"error"], @"link error");
            }
            EZPageSnapshot *job = [EZPageSnapshot new];
            EZSnapshotResponse *response = [EZSnapshotResponse new];
            response.testResponse = [[NSURLResponse alloc] initWithURL:[NSURL URLWithString:raw] MIMEType:@"text/html" expectedContentLength:0 textEncodingName:@"utf-8"];
            [job webView:policyView decidePolicyForNavigationResponse:response decisionHandler:^(WKNavigationResponsePolicy policy) {
                NSCAssert(policy == WKNavigationResponsePolicyCancel, @"final response refused");
            }];
        }
        for (NSString *host in @[@"localhost", @"127.0.0.1", @"[::1]"]) NSCAssert(EZPageLocalURL([NSURL URLWithString:[@"http://" stringByAppendingString:host]]), @"local host accepted");
        EZPageSnapshot *popup = [EZPageSnapshot new];
        NSCAssert(![popup webView:policyView createWebViewWithConfiguration:[WKWebViewConfiguration new] forNavigationAction:[EZSnapshotAction new] windowFeatures:[WKWindowFeatures new]] && popup.result[@"error"], @"popup refused");
        for (NSString *destination in @[@"http://localhost/", @"about:blank", @"https://example.com/"]) {
            EZPageSnapshot *job = [EZPageSnapshot new]; job.html = YES; job.state = EZPageLoading;
            EZSnapshotAction *action = [EZSnapshotAction new];
            EZSnapshotFrame *frame = [EZSnapshotFrame new]; frame.main = YES;
            action.frame = frame; action.testRequest = [NSURLRequest requestWithURL:[NSURL URLWithString:destination]];
            [job webView:policyView decidePolicyForNavigationAction:action decisionHandler:^(WKNavigationActionPolicy policy) { NSCAssert(policy == WKNavigationActionPolicyCancel, @"HTML permits only its initial page"); }];
        }
        int listener = socket(AF_INET, SOCK_STREAM, 0);
        struct sockaddr_in address = {.sin_family=AF_INET, .sin_addr.s_addr=htonl(INADDR_LOOPBACK)};
        NSCAssert(bind(listener, (struct sockaddr *)&address, sizeof(address)) == 0 && listen(listener, 8) == 0, @"local server");
        socklen_t length = sizeof(address); getsockname(listener, (struct sockaddr *)&address, &length);
        NSString *base = [NSString stringWithFormat:@"http://127.0.0.1:%u", ntohs(address.sin_port)];
        int remote = socket(AF_INET, SOCK_STREAM, 0);
        struct sockaddr_in remoteAddress = {.sin_family=AF_INET};
        remoteAddress.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
        NSCAssert(bind(remote, (struct sockaddr *)&remoteAddress, sizeof(remoteAddress)) == 0 && listen(remote, 8) == 0, @"denied host observer");
        length = sizeof(remoteAddress); getsockname(remote, (struct sockaddr *)&remoteAddress, &length);
        NSString *denied = [NSString stringWithFormat:@"http://localhost.:%u/", ntohs(remoteAddress.sin_port)];
        dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0), ^{
            for (;;) {
                int client = accept(listener, NULL, NULL);
                if (client < 0) break;
                char bytes[4096] = {0}; read(client, bytes, sizeof(bytes)-1);
                NSString *request = [NSString stringWithUTF8String:bytes];
                NSString *response = [request hasPrefix:@"GET /remote "] ? [NSString stringWithFormat:@"HTTP/1.1 302 Found\r\nLocation: http://localhost.:%u/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", ntohs(remoteAddress.sin_port)]
                    : [request hasPrefix:@"GET /redirect "] ? [NSString stringWithFormat:@"HTTP/1.1 302 Found\r\nLocation: %@/ok\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", base]
                    : [NSString stringWithFormat:@"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: %lu\r\nConnection: close\r\n\r\n%@", (unsigned long)[html lengthOfBytesUsingEncoding:NSUTF8StringEncoding], html];
                for (NSString *path in @[@"link", @"frame", @"popup"]) {
                    if (![request hasPrefix:[NSString stringWithFormat:@"GET /%@ ", path]]) continue;
                    NSString *body = [path isEqual:@"frame"] ? [NSString stringWithFormat:@"<iframe src='%@'></iframe>", denied] : [NSString stringWithFormat:@"<script>%@('%@')</script>", [path isEqual:@"popup"] ? @"window.open" : @"location.assign", denied];
                    response = [NSString stringWithFormat:@"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: %lu\r\nConnection: close\r\n\r\n%@", (unsigned long)[body lengthOfBytesUsingEncoding:NSUTF8StringEncoding], body];
                }
                NSData *data = [response dataUsingEncoding:NSUTF8StringEncoding]; write(client, data.bytes, data.length); close(client);
            }
        });
        image(render(@"url", [base stringByAppendingString:@"/ok"], @"light"));
        image(render(@"url", [base stringByAppendingString:@"/redirect"], @"dark"));
        NSDictionary *redirect = render(@"url", [base stringByAppendingString:@"/remote"], @"light");
        NSCAssert(redirect[@"error"] && ![redirect[@"error"] containsString:@"chargée"], @"nonlocal redirect rejected by policy, not a connection error: %@", redirect[@"error"]);
        for (NSString *path in @[@"link", @"frame", @"popup"]) render(@"url", [base stringByAppendingFormat:@"/%@", path], @"light");
        NSString *localObserver = [NSString stringWithFormat:@"http://127.0.0.1:%u/", ntohs(remoteAddress.sin_port)];
        for (NSString *markup in @[[NSString stringWithFormat:@"<script>location='%@'</script>", localObserver], [NSString stringWithFormat:@"<iframe src='%@'></iframe>", localObserver]]) NSCAssert(render(@"html", markup, @"light")[@"error"], @"HTML navigation and subframe refused live");
        render(@"html", [NSString stringWithFormat:@"<script>window.open('%@')</script>", localObserver], @"light");
        for (NSWindow *window in NSApp.windows) NSCAssert(!window.visible, @"popup never opens a visible window");
        struct pollfd observer = {remote, POLLIN, 0};
        NSCAssert(poll(&observer, 1, 0) == 0, @"no request reaches a denied host or leaves an HTML page, including redirect, link, subframe or popup");
        shutdown(listener, SHUT_RDWR); close(listener); close(remote);
        for (NSDictionary *invalid in @[@{@"theme":@"sepia", @"width":@1280, @"height":@800, @"html":html}, @{@"theme":@"dark", @"width":@16, @"height":@10, @"html":html}, @{@"theme":@"light", @"width":@1280, @"height":@800, @"html":html, @"url":@"http://localhost/"}, @{@"theme":@"light", @"width":@1280, @"height":@800, @"html":html, @"url":@1}]) NSCAssert(EZPageSnapshotRender(invalid, [NSDate dateWithTimeIntervalSinceNow:1])[@"error"], @"invalid protocol input");
        NSCAssert(EZPageSnapshotRender(@{}, [NSDate date])[@"error"], @"invalid input");
        NSCAssert(EZPageSnapshotRender(@{@"theme":@"light", @"width":@1280, @"height":@800, @"html":html}, [NSDate date])[@"error"], @"deadline enforced");
        puts("page-snapshot-test: PASS (themes, real pixels, dimensions, initial/link/subframe/popup/final/redirect, deadline, no visible window)");
    }
    return 0;
}
