#import "local-api.h"
#import <JavaScriptCore/JavaScriptCore.h>
#import <arpa/inet.h>
#import <assert.h>
#import <poll.h>
#import <sys/socket.h>
#import <unistd.h>

@interface LocalTestController : NSObject
@property(nonatomic, strong) WKUserScript *script;
@end
@implementation LocalTestController
- (void)addUserScript:(WKUserScript *)script { self.script = script; }
@end

@interface LocalTestConfiguration : NSObject
@property(nonatomic, strong) LocalTestController *userContentController;
@property(nonatomic, strong) id<WKURLSchemeHandler> handler;
@property(nonatomic, copy) NSString *scheme;
@end
@implementation LocalTestConfiguration
- (void)setURLSchemeHandler:(id<WKURLSchemeHandler>)handler forURLScheme:(NSString *)scheme {
    self.handler = handler; self.scheme = scheme;
}
@end

@interface LocalTestTask : NSObject <WKURLSchemeTask>
@property(nonatomic, copy) NSURLRequest *request;
@property(nonatomic, strong) NSHTTPURLResponse *response;
@property(nonatomic, strong) NSMutableData *data;
@property(nonatomic) NSUInteger responses;
@property(nonatomic) NSUInteger finishes;
@property(nonatomic) NSUInteger failures;
@property(nonatomic) BOOL stopped;
@property(nonatomic, copy) void (^onResponse)(void);
@property(nonatomic, copy) void (^onFailure)(void);
@end
@implementation LocalTestTask
- (instancetype)init {
    self = [super init];
    if (self) _data = [NSMutableData data];
    return self;
}
- (void)didReceiveResponse:(NSURLResponse *)response {
    assert(!self.stopped && !self.finishes && !self.failures && !self.responses);
    assert([response isKindOfClass:NSHTTPURLResponse.class]);
    self.response = (NSHTTPURLResponse *)response; self.responses++;
    if (self.onResponse) self.onResponse();
}
- (void)didReceiveData:(NSData *)data {
    assert(!self.stopped && !self.finishes && !self.failures && self.responses == 1);
    [self.data appendData:data];
}
- (void)didFinish {
    assert(!self.stopped && !self.finishes && !self.failures && self.responses == 1);
    self.finishes++;
}
- (void)didFailWithError:(NSError *)error {
    assert(!self.stopped && !self.finishes && !self.failures);
    assert([error.domain isEqual:@"EzerbotLocalAPI"] && error.userInfo.count == 1);
    assert([error.localizedDescription isEqual:@"Local resource unavailable."]);
    self.failures++;
    if (self.onFailure) self.onFailure();
}
@end

static void LocalWait(BOOL (^ready)(void)) {
    NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:5];
    while (!ready() && deadline.timeIntervalSinceNow > 0) {
        [[NSRunLoop currentRunLoop] runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.01]];
    }
    assert(ready());
}

static void LocalDrain(void) {
    NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:0.15];
    while (deadline.timeIntervalSinceNow > 0) {
        [[NSRunLoop currentRunLoop] runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.01]];
    }
}

@interface LocalTestServer : NSObject
@property(nonatomic) int listener;
@property(nonatomic) uint16_t port;
@property(nonatomic, copy) NSString *expectedHeader;
@property(nonatomic, copy) NSString *redirectLocation;
@property(nonatomic) NSUInteger requests;
@property(nonatomic) BOOL allAuthorized;
@property(nonatomic) BOOL sawRange;
@property(nonatomic) BOOL sawIfRange;
@property(nonatomic) BOOL sawCookie;
@property(nonatomic) BOOL sawHead;
@property(nonatomic) BOOL sawFinal;
@property(nonatomic, strong) dispatch_queue_t queue;
@property(nonatomic, strong) dispatch_source_t source;
@property(nonatomic, strong) dispatch_semaphore_t closed;
@end

@implementation LocalTestServer
- (instancetype)init {
    self = [super init];
    if (!self) return nil;
    _listener = socket(AF_INET, SOCK_STREAM, 0);
    assert(_listener >= 0);
    struct sockaddr_in address = {0};
    address.sin_family = AF_INET;
    address.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    assert(bind(_listener, (struct sockaddr *)&address, sizeof(address)) == 0);
    socklen_t length = sizeof(address);
    assert(getsockname(_listener, (struct sockaddr *)&address, &length) == 0);
    _port = ntohs(address.sin_port);
    assert(listen(_listener, 8) == 0);
    _allAuthorized = YES;
    _queue = dispatch_queue_create("ezerbot.local-api-test.server", DISPATCH_QUEUE_SERIAL);
    _closed = dispatch_semaphore_create(0);
    _source = dispatch_source_create(DISPATCH_SOURCE_TYPE_READ, _listener, 0, _queue);
    int listener = _listener;
    dispatch_semaphore_t closed = _closed;
    dispatch_source_set_cancel_handler(_source, ^{ close(listener); dispatch_semaphore_signal(closed); });
    __weak LocalTestServer *weakSelf = self;
    dispatch_source_set_event_handler(_source, ^{ [weakSelf acceptRequest]; });
    dispatch_resume(_source);
    return self;
}
- (void)acceptRequest {
    int connection = accept(self.listener, NULL, NULL);
    assert(connection >= 0);
    int noSignal = 1;
    assert(setsockopt(connection, SOL_SOCKET, SO_NOSIGPIPE, &noSignal, sizeof(noSignal)) == 0);
    NSMutableData *bytes = [NSMutableData data];
    while (bytes.length < 16384) {
        struct pollfd descriptor = {.fd=connection, .events=POLLIN};
        assert(poll(&descriptor, 1, 2000) > 0);
        char buffer[2048];
        ssize_t count = recv(connection, buffer, sizeof(buffer), 0);
        if (count <= 0) { close(connection); return; }
        [bytes appendBytes:buffer length:(NSUInteger)count];
        NSString *text = [[NSString alloc] initWithData:bytes encoding:NSUTF8StringEncoding];
        if ([text containsString:@"\r\n\r\n"]) break;
    }
    NSString *text = [[NSString alloc] initWithData:bytes encoding:NSUTF8StringEncoding];
    NSArray<NSString *> *lines = [text componentsSeparatedByString:@"\r\n"];
    NSArray<NSString *> *first = [lines.firstObject componentsSeparatedByString:@" "];
    assert(first.count == 3);
    NSMutableDictionary<NSString *, NSString *> *headers = [NSMutableDictionary dictionary];
    for (NSString *line in lines) {
        NSRange separator = [line rangeOfString:@":"];
        if (separator.location == NSNotFound) continue;
        NSString *key = [[line substringToIndex:separator.location] lowercaseString];
        headers[key] = [[line substringFromIndex:separator.location + 1] stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceCharacterSet];
    }
    NSString *path = first[1];
    BOOL head = [first[0] isEqual:@"HEAD"];
    BOOL ranged = [headers[@"range"] isEqual:@"bytes=1-2"];
    @synchronized(self) {
        self.requests++;
        self.allAuthorized = self.allAuthorized && [headers[@"x-ezerbot-launch"] isEqual:self.expectedHeader];
        self.sawRange = self.sawRange || ranged;
        self.sawIfRange = self.sawIfRange || [headers[@"if-range"] isEqual:@"resource-version"];
        self.sawCookie = self.sawCookie || headers[@"cookie"] != nil;
        self.sawHead = self.sawHead || head;
        self.sawFinal = self.sawFinal || [path hasPrefix:@"/api/final"];
    }
    NSString *response;
    if ([path hasPrefix:@"/api/redirect"]) {
        response = [NSString stringWithFormat:@"HTTP/1.1 302 Found\r\nLocation: %@\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", self.redirectLocation];
    } else if (ranged) {
        response = @"HTTP/1.1 206 Partial Content\r\nContent-Type: application/octet-stream\r\nContent-Length: 2\r\nContent-Range: bytes 1-2/4\r\nAccept-Ranges: bytes\r\nSet-Cookie: local-test=present\r\nConnection: close\r\n\r\nbc";
    } else {
        response = [@"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: 4\r\nSet-Cookie: local-test=present\r\nConnection: close\r\n\r\n" stringByAppendingString:head ? @"" : @"abcd"];
    }
    NSData *output = [response dataUsingEncoding:NSUTF8StringEncoding];
    const char *cursor = output.bytes;
    NSUInteger remaining = output.length;
    while (remaining) {
        ssize_t count = send(connection, cursor, remaining, 0);
        if (count < 0) break;
        assert(count > 0);
        cursor += count; remaining -= (NSUInteger)count;
    }
    close(connection);
    EzerbotClearLaunchBytes(bytes.mutableBytes, bytes.length);
}
- (void)stop {
    dispatch_source_cancel(self.source);
    assert(dispatch_semaphore_wait(self.closed, dispatch_time(DISPATCH_TIME_NOW, 3 * NSEC_PER_SEC)) == 0);
    self.source = nil;
    self.expectedHeader = nil;
}
@end

static LocalTestTask *LocalResourceTask(uint16_t port, NSString *path, NSString *method) {
    LocalTestTask *task = [[LocalTestTask alloc] init];
    NSURL *url = [NSURL URLWithString:[NSString stringWithFormat:@"ezerbot-resource://127.0.0.1:%u%@", port, path]];
    NSMutableURLRequest *request = [NSMutableURLRequest requestWithURL:url];
    request.HTTPMethod = method;
    task.request = request;
    return task;
}

static LocalTestConfiguration *LocalInstall(EzerbotLocalAPI *api) {
    LocalTestConfiguration *configuration = [[LocalTestConfiguration alloc] init];
    configuration.userContentController = [[LocalTestController alloc] init];
    [api installInConfiguration:(WKWebViewConfiguration *)(id)configuration];
    assert(configuration.handler == api && [configuration.scheme isEqual:@"ezerbot-resource"]);
    assert(configuration.userContentController.script.injectionTime == WKUserScriptInjectionTimeAtDocumentStart);
    assert(configuration.userContentController.script.forMainFrameOnly);
    return configuration;
}

static JSContext *LocalScriptContext(NSString *base, NSString *expectedHeader) {
    JSContext *context = [[JSContext alloc] init];
    context[@"nativeURL"] = ^NSDictionary *(NSString *raw, NSString *relative) {
        NSURL *url = relative.length ? [NSURL URLWithString:raw relativeToURL:[NSURL URLWithString:relative]].absoluteURL : [NSURL URLWithString:raw];
        NSURLComponents *parts = url ? [NSURLComponents componentsWithURL:url resolvingAgainstBaseURL:YES] : nil;
        assert(parts.scheme && parts.host);
        NSString *authority = parts.port && parts.port.integerValue != 80 ? [NSString stringWithFormat:@"%@:%@", parts.host, parts.port] : parts.host;
        NSString *origin = [NSString stringWithFormat:@"%@://%@", parts.scheme, authority];
        return @{@"href":url.absoluteString, @"origin":origin, @"protocol":[parts.scheme stringByAppendingString:@":"],
            @"hostname":parts.host, @"username":parts.user ?: @"", @"password":parts.password ?: @"",
            @"pathname":parts.percentEncodedPath.length ? parts.percentEncodedPath : @"/",
            @"search":parts.percentEncodedQuery ? [@"?" stringByAppendingString:parts.percentEncodedQuery] : @"",
            @"hash":parts.percentEncodedFragment ? [@"#" stringByAppendingString:parts.percentEncodedFragment] : @""};
    };
    context[@"base"] = base;
    context[@"expectedHeader"] = expectedHeader;
    [context evaluateScript:@"var window = this; var document = {baseURI:base}; var location = {href:base}; var sent = [];\n"
        "class URL { constructor(raw, relative) { Object.assign(this, nativeURL(String(raw), relative || '')); } }\n"
        "class Headers {\n"
        "  constructor(initial) { this.values = {}; if (initial instanceof Headers) Object.assign(this.values, initial.values); "
            "else if (initial) for (const key of Object.keys(initial)) this.set(key, initial[key]); }\n"
        "  set(key, value) { this.values[key.toLowerCase()] = String(value); }\n"
        "  get(key) { return this.values[key.toLowerCase()] || null; }\n"
        "  delete(key) { delete this.values[key.toLowerCase()]; }\n"
        "}\n"
        "class Request {\n"
        "  constructor(input, init = {}) {\n"
        "    const prior = input instanceof Request ? input : {url:new URL(input, document.baseURI).href};\n"
        "    Object.assign(this, {method:'GET', redirect:'follow', credentials:'same-origin', body:null}, prior, init);\n"
        "    this.headers = new Headers(init.headers || prior.headers);\n"
        "  }\n"
        "}\n"
        "window.URL = URL; window.Headers = Headers; window.Request = Request;\n"
        "window.fetch = request => { sent.push(request); return Promise.resolve(request); };\n"];
    assert(!context.exception);
    return context;
}

static void LocalCheckScript(EzerbotLocalAPI *api, NSString *header) {
    LocalTestConfiguration *configuration = LocalInstall(api);
    JSContext *context = LocalScriptContext([api.backendURL.absoluteString stringByAppendingString:@"/workspace"], header);
    [context evaluateScript:configuration.userContentController.script.source];
    assert(!context.exception);
    JSValue *result = [context evaluateScript:@"(() => {\n"
        "fetch('/api/state', {method:'POST', body:'body', headers:{'X-Other':'kept'}, redirect:'follow'});\n"
        "const first = sent.pop();\n"
        "if (first.headers.get('X-Ezerbot-Launch') !== expectedHeader || first.headers.get('X-Other') !== 'kept' || "
            "first.method !== 'POST' || first.body !== 'body' || first.redirect !== 'error' || first.credentials !== 'omit') return false;\n"
        "fetch(new Request('/assets/app.js', {headers:{'X-Other':'before'}}), {headers:{'X-Other':'override'}});\n"
        "const asset = sent.pop();\n"
        "if (asset.headers.get('X-Ezerbot-Launch') !== expectedHeader || asset.headers.get('X-Other') !== 'override') return false;\n"
        "for (const target of ['http://127.0.0.1:32124/api/state', 'https://127.0.0.1:32123/api/state', "
            "'http://localhost:32123/api/state', 'http://user@127.0.0.1:32123/api/state', 'https://example.invalid/api/state']) {\n"
        "  fetch(target, {headers:{'X-Ezerbot-Launch':expectedHeader}});\n"
        "  const outside = sent.pop();\n"
        "  if (outside.headers.get('X-Ezerbot-Launch') !== null || outside.redirect !== 'follow') return false;\n"
        "}\n"
        "if (EzerbotLocal.resourceURL('/api/file?download=1#part') !== 'ezerbot-resource://127.0.0.1:32123/api/file?download=1#part') return false;\n"
        "for (const raw of ['/assets/app.js', 'https://example.invalid/api/file', 'http://user@127.0.0.1:32123/api/file', 'http://127.0.0.1:32124/api/file']) "
            "if (EzerbotLocal.resourceURL(raw) !== raw) return false;\n"
        "return !EzerbotLocal.resourceURL('/api/file').includes(expectedHeader);\n"
        "})()"];
    assert(!context.exception && result.toBool);
    JSContext *outside = LocalScriptContext(@"https://example.invalid/workspace", header);
    [outside evaluateScript:configuration.userContentController.script.source];
    assert(!outside.exception);
    assert([[outside evaluateScript:@"typeof EzerbotLocal === 'undefined'"] toBool]);
    assert([[outside evaluateScript:@"fetch('https://example.invalid/api/state'); sent.pop().headers === undefined"] toBool]);
}

int main(void) {
    @autoreleasepool {
        NSData *first = EzerbotNewLaunchSecret(), *second = EzerbotNewLaunchSecret();
        assert(first.length == 32 && second.length == 32 && ![first isEqual:second]);
        assert([[EzerbotLocalAPI alloc] initWithSecretData:[NSMutableData dataWithLength:31]] == nil);
        EzerbotLocalAPI *api = [[EzerbotLocalAPI alloc] initWithSecretData:first];
        assert(api);
        api.backendURL = [NSURL URLWithString:@"https://127.0.0.1:32123"];
        assert(!api.backendURL);
        api.backendURL = [NSURL URLWithString:@"http://127.0.0.1:32123/"];
        assert(api.backendURL);
        api.backendURL = [NSURL URLWithString:@"http://127.0.0.1:32124"];
        assert(api.backendURL.port.integerValue == 32123);
        NSURLRequest *request = [NSURLRequest requestWithURL:[NSURL URLWithString:@"http://127.0.0.1:32123/api/state"]];
        NSMutableURLRequest *authorized = [api authorizeRequest:request];
        NSString *header = [authorized valueForHTTPHeaderField:@"X-Ezerbot-Launch"];
        NSCharacterSet *invalidHex = [[NSCharacterSet characterSetWithCharactersInString:@"0123456789abcdef"] invertedSet];
        assert(header.length == 64 && [header rangeOfCharacterFromSet:invalidHex].location == NSNotFound);
        assert(![request valueForHTTPHeaderField:@"X-Ezerbot-Launch"] && !authorized.HTTPShouldHandleCookies);
        for (NSString *url in @[@"https://127.0.0.1:32123/api/state", @"http://127.0.0.1:32124/api/state",
            @"http://localhost:32123/api/state", @"http://127.0.0.1/api/state", @"http://[::1]:32123/api/state",
            @"http://user@127.0.0.1:32123/api/state", @"http://:pass@127.0.0.1:32123/api/state",
            @"http://2130706433:32123/api/state", @"ezerbot-resource://127.0.0.1:32123/api/state"]) {
            assert(![api authorizeRequest:[NSURLRequest requestWithURL:[NSURL URLWithString:url]]]);
        }
        LocalCheckScript(api, header);
        [api invalidate];
        assert(![api authorizeRequest:request]);
        EzerbotLocalAPI *rotated = [[EzerbotLocalAPI alloc] initWithSecretData:second];
        rotated.backendURL = [NSURL URLWithString:@"http://127.0.0.1:32123"];
        assert(![[[rotated authorizeRequest:request] valueForHTTPHeaderField:@"X-Ezerbot-Launch"] isEqual:header]);
        [rotated invalidate];

        LocalTestServer *server = [[LocalTestServer alloc] init];
        LocalTestServer *sink = [[LocalTestServer alloc] init];
        EzerbotLocalAPI *resources = [[EzerbotLocalAPI alloc] initWithSecretData:first];
        WKWebView *view = (WKWebView *)(id)resources;
        resources.backendURL = [NSURL URLWithString:[NSString stringWithFormat:@"http://127.0.0.1:%u", server.port]];
        server.expectedHeader = [[resources authorizeRequest:[NSURLRequest requestWithURL:resources.backendURL]] valueForHTTPHeaderField:@"X-Ezerbot-Launch"];
        sink.expectedHeader = server.expectedHeader;
        LocalTestTask *full = LocalResourceTask(server.port, @"/api/file", @"GET");
        assert([resources isResourceURL:full.request.URL]);
        [resources webView:view startURLSchemeTask:full];
        LocalWait(^BOOL { return full.finishes + full.failures == 1; });
        assert(full.finishes == 1 && full.responses == 1 && [full.data isEqualToData:[@"abcd" dataUsingEncoding:NSUTF8StringEncoding]]);
        assert([full.response.URL isEqual:full.request.URL] && full.response.statusCode == 200);
        assert(![full.response valueForHTTPHeaderField:@"Set-Cookie"]);
        assert([[full.response valueForHTTPHeaderField:@"Access-Control-Allow-Origin"] isEqual:resources.backendURL.absoluteString]);

        LocalTestTask *range = LocalResourceTask(server.port, @"/api/file", @"GET");
        NSMutableURLRequest *rangeRequest = [range.request mutableCopy];
        [rangeRequest setValue:@"bytes=1-2" forHTTPHeaderField:@"Range"];
        [rangeRequest setValue:@"resource-version" forHTTPHeaderField:@"If-Range"];
        [rangeRequest setValue:@"local-test=present" forHTTPHeaderField:@"Cookie"];
        range.request = rangeRequest;
        [resources webView:view startURLSchemeTask:range];
        LocalWait(^BOOL { return range.finishes + range.failures == 1; });
        assert(range.finishes == 1 && range.response.statusCode == 206 && [range.data isEqualToData:[@"bc" dataUsingEncoding:NSUTF8StringEncoding]]);
        assert([[range.response valueForHTTPHeaderField:@"Content-Range"] isEqual:@"bytes 1-2/4"]);

        LocalTestTask *head = LocalResourceTask(server.port, @"/api/file", @"HEAD");
        [resources webView:view startURLSchemeTask:head];
        LocalWait(^BOOL { return head.finishes + head.failures == 1; });
        assert(head.finishes == 1 && head.data.length == 0);

        for (NSString *location in @[[NSString stringWithFormat:@"http://127.0.0.1:%u/api/final", sink.port], @"/api/final"]) {
            server.redirectLocation = location;
            LocalTestTask *redirect = LocalResourceTask(server.port, @"/api/redirect", @"GET");
            [resources webView:view startURLSchemeTask:redirect];
            LocalWait(^BOOL { return redirect.finishes + redirect.failures == 1; });
            assert(redirect.failures == 1 && redirect.responses == 0);
        }
        for (NSString *path in @[@"/assets/app.js", @"/api/../private", @"/api/%2e%2e/private", @"/api/%2e%2e%2fprivate"]) {
            LocalTestTask *invalid = LocalResourceTask(server.port, path, @"GET");
            assert(![resources isResourceURL:invalid.request.URL]);
            [resources webView:view startURLSchemeTask:invalid];
            assert(invalid.failures == 1 && invalid.responses == 0);
        }
        LocalTestTask *post = LocalResourceTask(server.port, @"/api/file", @"POST");
        [resources webView:view startURLSchemeTask:post];
        assert(post.failures == 1);
        LocalTestTask *otherPort = LocalResourceTask(sink.port, @"/api/file", @"GET");
        assert(![resources isResourceURL:otherPort.request.URL]);
        [resources webView:view startURLSchemeTask:otherPort];
        assert(otherPort.failures == 1);
        for (NSString *format in @[@"ezerbot-resource://user@127.0.0.1:%u/api/file",
            @"ezerbot-resource://:pass@127.0.0.1:%u/api/file", @"ezerbot-resource://localhost:%u/api/file",
            @"http://127.0.0.1:%u/api/file"]) {
            NSURL *outside = [NSURL URLWithString:[NSString stringWithFormat:format, server.port]];
            assert(![resources isResourceURL:outside]);
        }

        LocalTestTask *duringResponse = LocalResourceTask(server.port, @"/api/file", @"GET");
        __weak LocalTestTask *responseTask = duringResponse;
        duringResponse.onResponse = ^{
            responseTask.stopped = YES;
            [resources webView:view stopURLSchemeTask:responseTask];
        };
        [resources webView:view startURLSchemeTask:duringResponse];
        LocalWait(^BOOL { return duringResponse.responses == 1; });
        LocalDrain();
        assert(duringResponse.data.length + duringResponse.finishes + duringResponse.failures == 0);
        duringResponse.onResponse = nil;

        LocalTestTask *cancelled = LocalResourceTask(server.port, @"/api/file", @"GET");
        [resources webView:view startURLSchemeTask:cancelled];
        cancelled.stopped = YES;
        [resources webView:view stopURLSchemeTask:cancelled];
        LocalDrain();
        assert(cancelled.responses + cancelled.finishes + cancelled.failures == 0);
        @synchronized(server) {
            assert(server.requests >= 5 && server.allAuthorized && server.sawRange && server.sawIfRange && server.sawHead);
            assert(!server.sawCookie && !server.sawFinal);
        }
        @synchronized(sink) { assert(sink.requests == 0); }
        LocalTestTask *invalidated = LocalResourceTask(server.port, @"/api/file", @"GET");
        LocalTestTask *sibling = LocalResourceTask(server.port, @"/api/file", @"GET");
        invalidated.onFailure = ^{
            if (!sibling.failures) { sibling.stopped = YES; [resources webView:view stopURLSchemeTask:sibling]; }
        };
        sibling.onFailure = ^{
            if (!invalidated.failures) { invalidated.stopped = YES; [resources webView:view stopURLSchemeTask:invalidated]; }
        };
        [resources webView:view startURLSchemeTask:invalidated];
        [resources webView:view startURLSchemeTask:sibling];
        [resources invalidate];
        assert(invalidated.failures + sibling.failures == 1 && invalidated.finishes + sibling.finishes == 0);
        assert(![resources isResourceURL:full.request.URL]);
        [resources invalidate];
        LocalDrain();
        assert(invalidated.failures + sibling.failures == 1);
        invalidated.onFailure = nil; sibling.onFailure = nil;
        [server stop]; [sink stop];
        puts("Local API transport and JavaScript checks passed.");
    }
    return 0;
}
