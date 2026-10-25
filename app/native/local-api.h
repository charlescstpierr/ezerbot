#import <Cocoa/Cocoa.h>
#import <Security/Security.h>
#import <WebKit/WebKit.h>

NS_ASSUME_NONNULL_BEGIN

static inline void EzerbotClearLaunchBytes(void *bytes, NSUInteger count) {
    volatile unsigned char *cursor = bytes;
    while (count--) *cursor++ = 0;
}

static inline NSData * _Nullable EzerbotNewLaunchSecret(void) {
    NSMutableData *data = [NSMutableData dataWithLength:32];
    if (SecRandomCopyBytes(kSecRandomDefault, data.length, data.mutableBytes) != errSecSuccess) {
        EzerbotClearLaunchBytes(data.mutableBytes, data.length);
        return nil;
    }
    return data;
}

@interface EzerbotLocalResource : NSObject
@property(nonatomic, strong) id<WKURLSchemeTask> schemeTask;
@property(nonatomic, strong) NSURLSessionDataTask *networkTask;
@property(nonatomic) BOOL receivedResponse;
@end
@implementation EzerbotLocalResource
@end

@interface EzerbotLocalAPI : NSObject <WKURLSchemeHandler, NSURLSessionDataDelegate>
@property(nonatomic, copy, nullable) NSURL *backendURL;
- (nullable instancetype)initWithSecretData:(NSData *)secretData;
- (instancetype)init NS_UNAVAILABLE;
- (nullable NSMutableURLRequest *)authorizeRequest:(NSURLRequest *)request;
- (BOOL)isResourceURL:(NSURL *)url;
- (void)installInConfiguration:(WKWebViewConfiguration *)configuration;
- (void)invalidate;
@end

@interface EzerbotLocalAPI ()
@property(nonatomic, copy, nullable) NSString *launchSecret;
@property(nonatomic, strong, nullable) NSURLSession *resourceSession;
@property(nonatomic, strong) NSMutableDictionary<NSNumber *, EzerbotLocalResource *> *resources;
@property(nonatomic) BOOL invalidated;
@end

@implementation EzerbotLocalAPI
@synthesize backendURL = _backendURL;

- (nullable instancetype)initWithSecretData:(NSData *)secretData {
    if (secretData.length != 32) return nil;
    self = [super init];
    if (!self) return nil;
    const unsigned char *bytes = secretData.bytes;
    const char digits[] = "0123456789abcdef";
    char hex[64];
    for (NSUInteger index = 0; index < secretData.length; index++) {
        hex[index * 2] = digits[bytes[index] >> 4];
        hex[index * 2 + 1] = digits[bytes[index] & 15];
    }
    _launchSecret = [[NSString alloc] initWithBytes:hex length:sizeof(hex) encoding:NSASCIIStringEncoding];
    EzerbotClearLaunchBytes(hex, sizeof(hex));
    _resources = [NSMutableDictionary dictionary];
    return self;
}

- (void)setBackendURL:(nullable NSURL *)url {
    if (_backendURL || self.invalidated) return;
    NSURLComponents *components = url ? [NSURLComponents componentsWithURL:url resolvingAgainstBaseURL:NO] : nil;
    if (![components.scheme isEqual:@"http"] || ![components.host isEqual:@"127.0.0.1"] ||
        !components.port || components.port.integerValue < 1 || components.port.integerValue > 65535 ||
        components.user || components.password || components.query || components.fragment ||
        (components.path.length && ![components.path isEqual:@"/"])) return;
    components.path = @"";
    _backendURL = components.URL;
}

- (BOOL)isBackendURL:(nullable NSURL *)url {
    return self.backendURL && [url.scheme isEqual:@"http"] && [url.host isEqual:@"127.0.0.1"] &&
        [(url.port ?: @80) isEqual:self.backendURL.port] && !url.user && !url.password;
}

- (nullable NSMutableURLRequest *)authorizeRequest:(NSURLRequest *)request {
    if (self.invalidated || !self.launchSecret || ![self isBackendURL:request.URL]) return nil;
    NSMutableURLRequest *authorized = [request mutableCopy];
    [authorized setValue:self.launchSecret forHTTPHeaderField:@"X-Ezerbot-Launch"];
    authorized.HTTPShouldHandleCookies = NO;
    authorized.cachePolicy = NSURLRequestReloadIgnoringLocalCacheData;
    return authorized;
}

- (void)installInConfiguration:(WKWebViewConfiguration *)configuration {
    if (self.invalidated || !self.backendURL || !self.launchSecret) return;
    NSString *origin = self.backendURL.absoluteString;
    NSString *resourceOrigin = [NSString stringWithFormat:@"ezerbot-resource://127.0.0.1:%@", self.backendURL.port];
    NSData *parameters = [NSJSONSerialization dataWithJSONObject:@[origin, resourceOrigin, self.launchSecret] options:0 error:nil];
    NSString *arguments = [[NSString alloc] initWithData:parameters encoding:NSUTF8StringEncoding];
    NSString *source = [NSString stringWithFormat:
        @"(() => { 'use strict';\n"
        "const [backend, resourceOrigin, launch] = %@;\n"
        "const URLClass = window.URL, RequestClass = window.Request, HeadersClass = window.Headers;\n"
        "const localOrigin = new URLClass(backend).origin;\n"
        "const isLocal = url => url.origin === localOrigin && url.protocol === 'http:' && "
            "url.hostname === '127.0.0.1' && !url.username && !url.password;\n"
        "if (!isLocal(new URLClass(window.location.href))) return;\n"
        "const originalFetch = window.fetch.bind(window);\n"
        "window.fetch = function(input, init) {\n"
        "  try {\n"
        "    const request = new RequestClass(input, init);\n"
        "    const url = new URLClass(request.url);\n"
        "    const headers = new HeadersClass(request.headers);\n"
        "    headers.delete('X-Ezerbot-Launch');\n"
        "    if (isLocal(url)) {\n"
        "      headers.set('X-Ezerbot-Launch', launch);\n"
        "      return originalFetch(new RequestClass(request, {headers, redirect:'error', credentials:'omit'}));\n"
        "    }\n"
        "    return originalFetch(new RequestClass(request, {headers}));\n"
        "  } catch (_) { return Promise.reject(new TypeError('Request unavailable.')); }\n"
        "};\n"
        "const resourceURL = raw => {\n"
        "  try {\n"
        "    const url = new URLClass(raw, document.baseURI);\n"
        "    if (isLocal(url) && url.pathname.startsWith('/api/')) "
            "return resourceOrigin + url.pathname + url.search + url.hash;\n"
        "  } catch (_) {}\n"
        "  return raw;\n"
        "};\n"
        "Object.defineProperty(window, 'EzerbotLocal', {value:Object.freeze({resourceURL}), writable:false, configurable:false});\n"
        "})();", arguments];
    WKUserScript *script = [[WKUserScript alloc] initWithSource:source injectionTime:WKUserScriptInjectionTimeAtDocumentStart forMainFrameOnly:YES];
    [configuration.userContentController addUserScript:script];
    [configuration setURLSchemeHandler:self forURLScheme:@"ezerbot-resource"];
}

- (nullable NSMutableURLRequest *)resourceRequest:(NSURLRequest *)request {
    NSURL *url = request.URL;
    NSString *method = request.HTTPMethod ?: @"GET";
    NSURLComponents *components = url ? [NSURLComponents componentsWithURL:url resolvingAgainstBaseURL:NO] : nil;
    if (![components.scheme isEqual:@"ezerbot-resource"] || ![components.host isEqual:@"127.0.0.1"] ||
        ![components.port isEqual:self.backendURL.port] || components.user || components.password ||
        ![components.percentEncodedPath hasPrefix:@"/api/"] ||
        ![components.path hasPrefix:@"/api/"] || [components.path containsString:@"\\"] ||
        (![method isEqual:@"GET"] && ![method isEqual:@"HEAD"])) return nil;
    for (NSString *part in components.path.pathComponents) {
        if ([part isEqual:@"."] || [part isEqual:@".."]) return nil;
    }
    components.scheme = @"http";
    components.fragment = nil;
    NSMutableURLRequest *forward = [NSMutableURLRequest requestWithURL:components.URL];
    forward.HTTPMethod = method;
    for (NSString *header in @[@"Range", @"If-Range"]) {
        [forward setValue:[request valueForHTTPHeaderField:header] forHTTPHeaderField:header];
    }
    [forward setValue:@"identity" forHTTPHeaderField:@"Accept-Encoding"];
    return [self authorizeRequest:forward];
}

- (BOOL)isResourceURL:(NSURL *)url {
    return [self resourceRequest:[NSURLRequest requestWithURL:url]] != nil;
}

- (NSError *)resourceError {
    return [NSError errorWithDomain:@"EzerbotLocalAPI" code:1 userInfo:@{NSLocalizedDescriptionKey:@"Local resource unavailable."}];
}

- (void)webView:(WKWebView *)webView startURLSchemeTask:(id<WKURLSchemeTask>)schemeTask {
    (void)webView;
    NSMutableURLRequest *request = [self resourceRequest:schemeTask.request];
    if (!request) { [schemeTask didFailWithError:[self resourceError]]; return; }
    if (!self.resourceSession) {
        NSURLSessionConfiguration *configuration = NSURLSessionConfiguration.ephemeralSessionConfiguration;
        configuration.HTTPShouldSetCookies = NO;
        configuration.HTTPCookieStorage = nil;
        configuration.URLCredentialStorage = nil;
        configuration.URLCache = nil;
        configuration.requestCachePolicy = NSURLRequestReloadIgnoringLocalCacheData;
        configuration.connectionProxyDictionary = @{};
        self.resourceSession = [NSURLSession sessionWithConfiguration:configuration delegate:self delegateQueue:NSOperationQueue.mainQueue];
    }
    EzerbotLocalResource *resource = [[EzerbotLocalResource alloc] init];
    resource.schemeTask = schemeTask;
    resource.networkTask = [self.resourceSession dataTaskWithRequest:request];
    self.resources[@(resource.networkTask.taskIdentifier)] = resource;
    [resource.networkTask resume];
}

- (void)webView:(WKWebView *)webView stopURLSchemeTask:(id<WKURLSchemeTask>)schemeTask {
    (void)webView;
    for (NSNumber *key in self.resources.allKeys) {
        EzerbotLocalResource *resource = self.resources[key];
        if (resource.schemeTask == schemeTask) {
            [self.resources removeObjectForKey:key];
            [resource.networkTask cancel];
            return;
        }
    }
}

- (void)URLSession:(NSURLSession *)session task:(NSURLSessionTask *)task
    willPerformHTTPRedirection:(NSHTTPURLResponse *)response newRequest:(NSURLRequest *)request
    completionHandler:(void (^)(NSURLRequest * _Nullable))completionHandler {
    (void)session; (void)task; (void)response; (void)request;
    completionHandler(nil);
}

- (void)URLSession:(NSURLSession *)session dataTask:(NSURLSessionDataTask *)task
    didReceiveResponse:(NSURLResponse *)response completionHandler:(void (^)(NSURLSessionResponseDisposition))completionHandler {
    if (session != self.resourceSession) { completionHandler(NSURLSessionResponseCancel); return; }
    NSNumber *key = @(task.taskIdentifier);
    EzerbotLocalResource *resource = self.resources[key];
    if (!resource) { completionHandler(NSURLSessionResponseCancel); return; }
    NSHTTPURLResponse *http = [response isKindOfClass:NSHTTPURLResponse.class] ? (NSHTTPURLResponse *)response : nil;
    if (!http || ![self isBackendURL:http.URL] || (http.statusCode >= 300 && http.statusCode < 400)) {
        [self.resources removeObjectForKey:key];
        completionHandler(NSURLSessionResponseCancel);
        [resource.schemeTask didFailWithError:[self resourceError]];
        return;
    }
    NSMutableDictionary<NSString *, NSString *> *headers = [NSMutableDictionary dictionary];
    for (NSString *name in @[@"Content-Type", @"Content-Length", @"Content-Range", @"Accept-Ranges", @"Content-Disposition", @"Cache-Control", @"ETag", @"Last-Modified"]) {
        NSString *value = [http valueForHTTPHeaderField:name];
        if (value) headers[name] = value;
    }
    if ([http valueForHTTPHeaderField:@"Content-Encoding"].length) [headers removeObjectForKey:@"Content-Length"];
    headers[@"Access-Control-Allow-Origin"] = self.backendURL.port.integerValue == 80 ? @"http://127.0.0.1" : self.backendURL.absoluteString;
    NSHTTPURLResponse *forward = [[NSHTTPURLResponse alloc] initWithURL:resource.schemeTask.request.URL
        statusCode:http.statusCode HTTPVersion:@"HTTP/1.1" headerFields:headers];
    resource.receivedResponse = YES;
    [resource.schemeTask didReceiveResponse:forward];
    completionHandler(self.resources[key] == resource ? NSURLSessionResponseAllow : NSURLSessionResponseCancel);
}

- (void)URLSession:(NSURLSession *)session dataTask:(NSURLSessionDataTask *)task didReceiveData:(NSData *)data {
    if (session != self.resourceSession) return;
    EzerbotLocalResource *resource = self.resources[@(task.taskIdentifier)];
    if (resource) [resource.schemeTask didReceiveData:data];
}

- (void)URLSession:(NSURLSession *)session task:(NSURLSessionTask *)task didCompleteWithError:(nullable NSError *)error {
    if (session != self.resourceSession) return;
    NSNumber *key = @(task.taskIdentifier);
    EzerbotLocalResource *resource = self.resources[key];
    if (!resource) return;
    [self.resources removeObjectForKey:key];
    if (error || !resource.receivedResponse) [resource.schemeTask didFailWithError:[self resourceError]];
    else [resource.schemeTask didFinish];
}

- (void)invalidate {
    self.invalidated = YES;
    [self.resourceSession invalidateAndCancel];
    self.resourceSession = nil;
    self.launchSecret = nil;
    for (NSNumber *key in self.resources.allKeys) {
        EzerbotLocalResource *resource = self.resources[key];
        if (!resource) continue;
        [self.resources removeObjectForKey:key];
        [resource.schemeTask didFailWithError:[self resourceError]];
    }
}
@end

NS_ASSUME_NONNULL_END
