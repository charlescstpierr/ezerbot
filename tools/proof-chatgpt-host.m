#define main EzerbotOriginalMain
#import "../app/native/main.m"
#undef main
#import <pwd.h>

#if defined(EZERBOT_RELEASE)
#error The operator proof requires the isolated non-release shell.
#endif

static const NSUInteger ProofLineLimit = 1024 * 1024;
static const NSUInteger ProofJournalLimit = 16 * 1024 * 1024;

static BOOL ProofWithin(NSString *path, NSString *directory) {
    return [path hasPrefix:[directory stringByAppendingString:@"/"]];
}

static BOOL ProofUnsafeString(NSString *text, NSString *identity) {
    if (identity.length && [text containsString:identity]) return YES;
    static NSRegularExpression *pattern;
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        pattern = [NSRegularExpression regularExpressionWithPattern:
            @"eyJ[A-Za-z0-9_-]{8,}|Bearer\\s+\\S+|refresh_token|access_token|id_token"
            options:NSRegularExpressionCaseInsensitive error:nil];
    });
    return !pattern || [pattern firstMatchInString:text options:0 range:NSMakeRange(0, text.length)] != nil;
}

static BOOL ProofUnsafeObject(id object, NSString *identity) {
    if ([object isKindOfClass:NSString.class]) return ProofUnsafeString(object, identity);
    if ([object isKindOfClass:NSDictionary.class]) {
        for (NSString *key in object) {
            if (ProofUnsafeString(key, identity) || ProofUnsafeObject(object[key], identity)) return YES;
        }
    } else if ([object isKindOfClass:NSArray.class]) {
        for (id value in object) if (ProofUnsafeObject(value, identity)) return YES;
    }
    return NO;
}

static NSString *ProofHome(void) {
    NSDictionary *environment = NSProcessInfo.processInfo.environment;
    NSString *home = environment[@"HOME"];
    if (!home.isAbsolutePath || ![home isEqual:environment[@"CFFIXED_USER_HOME"]] ||
        ![home isEqual:environment[@"EZERBOT_TEST_HOME"]] ||
        ![home isEqual:home.stringByStandardizingPath.stringByResolvingSymlinksInPath]) return nil;
    struct passwd *owner = getpwuid(getuid());
    NSString *actualHome = owner ? [NSString stringWithUTF8String:owner->pw_dir] : nil;
    NSString *actualSupport = [actualHome stringByAppendingPathComponent:@"Library/Application Support/Grokbot"];
    if (!actualHome || [home isEqual:actualHome] || ProofWithin(actualHome, home) || [home isEqual:@"/"] ||
        [home isEqual:actualSupport] || ProofWithin(home, actualSupport)) return nil;
    struct stat status;
    if (lstat(home.fileSystemRepresentation, &status) != 0 || !S_ISDIR(status.st_mode) ||
        status.st_uid != getuid() || (status.st_mode & 0777) != 0700) return nil;
    NSString *support = [home stringByAppendingPathComponent:@"Library/Application Support/Grokbot"];
    for (NSString *path in @[support, [support stringByAppendingPathComponent:@"logs/engine.log"],
                             [support stringByAppendingPathComponent:@"logs/engine.previous.log"],
                             [support stringByAppendingPathComponent:@"workspace.sqlite"],
                             [support stringByAppendingPathComponent:@"workspace.codex-auth.json"]]) {
        if (![path isEqual:path.stringByResolvingSymlinksInPath]) return nil;
    }
    return home;
}

@interface EzerbotOperatorProof : NSObject
@property(nonatomic, strong) GrokbotApplication *shell;
@property(nonatomic, copy) NSString *home;
@property(nonatomic, copy) NSString *supportPath;
@property(atomic, copy) NSString *identity;
@property(nonatomic, strong) NSLock *outputLock;
@property(nonatomic, strong) dispatch_source_t readinessTimer;
@property(atomic, strong) NSURLSession *activeSession;
@property(atomic) BOOL ready;
@property(atomic) BOOL closing;
@property(nonatomic) BOOL probePending;
@property(nonatomic) NSTimeInterval startupDeadline;
@property(nonatomic) int exitStatus;
- (BOOL)writeReply:(NSDictionary *)reply;
- (BOOL)journalsSafe;
- (void)finishWithReply:(BOOL)reply status:(int)status;
- (void)pollWorkspace;
- (void)readCommands;
- (NSDictionary *)performRequest:(NSDictionary *)command;
@end

@implementation EzerbotOperatorProof

- (BOOL)writeReply:(NSDictionary *)reply {
    NSData *encoded = [NSJSONSerialization dataWithJSONObject:reply options:0 error:nil];
    NSString *text = encoded ? [[NSString alloc] initWithData:encoded encoding:NSUTF8StringEncoding] : nil;
    if (!text || encoded.length >= ProofLineLimit || ProofUnsafeString(text, self.identity) ||
        ProofUnsafeObject(reply, self.identity)) {
        encoded = [@"{\"error\":\"Unsafe response refused.\"}" dataUsingEncoding:NSUTF8StringEncoding];
    }
    NSMutableData *line = [encoded mutableCopy];
    [line appendBytes:"\n" length:1];
    [self.outputLock lock];
    const unsigned char *bytes = line.bytes;
    NSUInteger remaining = line.length;
    while (remaining) {
        ssize_t count = write(STDOUT_FILENO, bytes, remaining);
        if (count < 0 && errno == EINTR) continue;
        if (count <= 0) break;
        remaining -= (NSUInteger)count;
        bytes += count;
    }
    [self.outputLock unlock];
    return remaining == 0;
}

- (BOOL)journalsSafe {
    NSString *support = self.supportPath;
    if (!support || !ProofWithin(support, self.home) ||
        ![support isEqual:support.stringByResolvingSymlinksInPath] || !self.identity.length) return NO;
    NSString *engine = [support stringByAppendingPathComponent:@"logs/engine.log"];
    NSString *stderrPath = [self.home stringByAppendingPathComponent:@"shell.err"];
    for (NSString *path in @[engine, stderrPath]) {
        if (!ProofWithin(path.stringByResolvingSymlinksInPath, self.home)) return NO;
        int descriptor = open(path.fileSystemRepresentation, O_RDONLY | O_NOFOLLOW | O_CLOEXEC);
        if (descriptor < 0) return NO;
        struct stat status;
        BOOL valid = fstat(descriptor, &status) == 0 && S_ISREG(status.st_mode) && status.st_uid == getuid();
        if ([path isEqual:stderrPath]) {
            struct stat standardError;
            valid = valid && fstat(STDERR_FILENO, &standardError) == 0 &&
                standardError.st_dev == status.st_dev && standardError.st_ino == status.st_ino;
        }
        NSMutableData *contents = [NSMutableData data];
        unsigned char buffer[8192];
        while (valid) {
            ssize_t count = read(descriptor, buffer, sizeof buffer);
            if (count < 0 && errno == EINTR) continue;
            if (count < 0) { valid = NO; break; }
            if (count == 0) break;
            if (contents.length + (NSUInteger)count > ProofJournalLimit) { valid = NO; break; }
            [contents appendBytes:buffer length:(NSUInteger)count];
        }
        close(descriptor);
        NSString *text = valid ? [[NSString alloc] initWithData:contents encoding:NSUTF8StringEncoding] : nil;
        if (!text || ProofUnsafeString(text, self.identity)) return NO;
    }
    return YES;
}

- (void)finishWithReply:(BOOL)reply status:(int)status {
    if (self.closing) return;
    self.closing = YES;
    if (self.readinessTimer) dispatch_source_cancel(self.readinessTimer);
    [self.activeSession invalidateAndCancel];
    BOOL safe = !self.identity.length || [self journalsSafe];
    self.shell.terminating = YES;
    ++self.shell.startupGeneration;
    [self.shell stopBackend];
    safe = (!self.identity.length || [self journalsSafe]) && safe;
    self.exitStatus = safe ? status : 1;
    if (reply) {
        BOOL written = [self writeReply:safe ? @{@"ok":@YES} : @{@"error":@"Journal safety check failed."}];
        if (!written) self.exitStatus = 1;
    }
    [self.shell.webView stopLoading];
    [self.shell.window close];
    [NSApp stop:nil];
    [NSApp postEvent:[NSEvent otherEventWithType:NSEventTypeApplicationDefined location:NSZeroPoint
        modifierFlags:0 timestamp:0 windowNumber:0 context:nil subtype:0 data1:0 data2:0] atStart:NO];
}

- (void)pollWorkspace {
    if (self.ready || self.closing) return;
    if (NSProcessInfo.processInfo.systemUptime >= self.startupDeadline) {
        [self writeReply:@{@"error":@"Workspace startup failed."}];
        [self finishWithReply:NO status:1];
        return;
    }
    GrokbotApplication *shell = self.shell;
    if (!shell.backendURL || !shell.webView || !shell.localAPI || !shell.backendTask.running) return;
    NSString *expected = [self.home stringByAppendingPathComponent:@"Library/Application Support/Grokbot"];
    if (![shell.supportPath isEqual:expected] || ![expected isEqual:expected.stringByResolvingSymlinksInPath]) {
        [self writeReply:@{@"error":@"Workspace isolation failed."}];
        [self finishWithReply:NO status:1];
        return;
    }
    if (!self.identity) {
        NSURLRequest *authorized = [shell.localAPI authorizeRequest:[NSURLRequest requestWithURL:shell.backendURL]];
        NSString *identity = [authorized valueForHTTPHeaderField:@"X-Ezerbot-Launch"];
        if (identity.length != 64) return;
        self.identity = identity;
        self.supportPath = shell.supportPath;
    }
    if (self.probePending) return;
    self.probePending = YES;
    [shell.webView evaluateJavaScript:
        @"Boolean(document.readyState === 'complete' && window.EzerbotLocal && document.querySelector('#settings-button') && document.querySelector('#composer-tools'))"
        completionHandler:^(id result, NSError *error) {
            self.probePending = NO;
            if (self.closing || self.ready || error || ![result isEqual:@YES] ||
                ![shell isWorkspaceURL:shell.webView.URL]) return;
            self.ready = YES;
            dispatch_source_cancel(self.readinessTimer);
            if (![self writeReply:@{@"ready":@YES}]) [self finishWithReply:NO status:1];
        }];
}

- (NSDictionary *)performRequest:(NSDictionary *)command {
    NSString *method = command[@"method"];
    NSString *path = command[@"path"];
    id body = command[@"body"];
    NSDictionary *routes = @{
        @"GET":@[@"/api/state", @"/api/usage"],
        @"POST":@[@"/api/preferences", @"/api/conversations", @"/api/send", @"/api/provider/test",
                   @"/api/usage/refresh", @"/api/provider/signout"]
    };
    if (![method isKindOfClass:NSString.class] || ![path isKindOfClass:NSString.class] ||
        ![routes[method] containsObject:path] || command.count < 2 || command.count > 3) {
        return @{@"error":@"Request refused."};
    }
    for (NSString *key in command) {
        if (![@[@"method", @"path", @"body"] containsObject:key]) return @{@"error":@"Request refused."};
    }
    if (body == NSNull.null) body = nil;
    if ((body && ![body isKindOfClass:NSDictionary.class]) || ([method isEqual:@"GET"] && body)) {
        return @{@"error":@"Request body refused."};
    }
    NSData *payload = [method isEqual:@"POST"] ?
        [NSJSONSerialization dataWithJSONObject:body ?: @{} options:0 error:nil] : nil;
    if ([method isEqual:@"POST"] && !payload) return @{@"error":@"Request body refused."};
    __block NSMutableURLRequest *request;
    __block EzerbotLocalAPI *api;
    dispatch_sync(dispatch_get_main_queue(), ^{
        if (!self.ready || self.closing) return;
        api = self.shell.localAPI;
        NSURLComponents *url = [NSURLComponents componentsWithURL:self.shell.backendURL resolvingAgainstBaseURL:NO];
        url.path = path;
        NSMutableURLRequest *unsignedRequest = [NSMutableURLRequest requestWithURL:url.URL];
        unsignedRequest.HTTPMethod = method;
        unsignedRequest.HTTPBody = payload;
        unsignedRequest.timeoutInterval = 25;
        [unsignedRequest setValue:@"local" forHTTPHeaderField:@"X-Grokbot-Request"];
        [unsignedRequest setValue:@"application/json" forHTTPHeaderField:@"Accept"];
        if (payload) [unsignedRequest setValue:@"application/json" forHTTPHeaderField:@"Content-Type"];
        request = [api authorizeRequest:unsignedRequest];
    });
    if (!request || !api || ![[request valueForHTTPHeaderField:@"X-Ezerbot-Launch"] isEqual:self.identity]) {
        return @{@"error":@"Workspace unavailable."};
    }
    NSURLSessionConfiguration *configuration = NSURLSessionConfiguration.ephemeralSessionConfiguration;
    configuration.timeoutIntervalForRequest = 25;
    configuration.timeoutIntervalForResource = 28;
    configuration.HTTPShouldSetCookies = NO;
    configuration.HTTPCookieStorage = nil;
    configuration.URLCredentialStorage = nil;
    configuration.URLCache = nil;
    configuration.requestCachePolicy = NSURLRequestReloadIgnoringLocalCacheData;
    configuration.connectionProxyDictionary = @{};
    NSURLSession *session = [NSURLSession sessionWithConfiguration:configuration delegate:api delegateQueue:nil];
    self.activeSession = session;
    dispatch_semaphore_t finished = dispatch_semaphore_create(0);
    __block NSDictionary *answer = @{@"error":@"Local request failed."};
    NSString *identity = self.identity;
    [[session dataTaskWithRequest:request completionHandler:^(NSData *data, NSURLResponse *response, NSError *error) {
        NSHTTPURLResponse *http = [response isKindOfClass:NSHTTPURLResponse.class] ? (NSHTTPURLResponse *)response : nil;
        if (!error && http && [http.URL isEqual:request.URL] && http.statusCode >= 200 &&
            http.statusCode < 600 && !(http.statusCode >= 300 && http.statusCode < 400) && data.length < ProofLineLimit) {
            NSString *text = [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding];
            id object = text && !ProofUnsafeString(text, identity) ?
                [NSJSONSerialization JSONObjectWithData:data options:0 error:nil] : nil;
            if ([object isKindOfClass:NSDictionary.class] && !ProofUnsafeObject(object, identity)) {
                answer = @{@"status":@(http.statusCode), @"body":object};
            }
        }
        dispatch_semaphore_signal(finished);
    }] resume];
    BOOL completed = dispatch_semaphore_wait(finished, dispatch_time(DISPATCH_TIME_NOW, 30 * NSEC_PER_SEC)) == 0;
    [session invalidateAndCancel];
    self.activeSession = nil;
    return completed ? answer : @{@"error":@"Local request timed out."};
}

- (void)readCommands {
    NSMutableData *line = [NSMutableData data];
    unsigned char buffer[4096];
    while (!self.closing) {
        ssize_t count = read(STDIN_FILENO, buffer, sizeof buffer);
        if (count < 0 && errno == EINTR) continue;
        if (count <= 0) {
            int status = count == 0 && line.length == 0 ? 0 : 1;
            dispatch_async(dispatch_get_main_queue(), ^{ [self finishWithReply:NO status:status]; });
            return;
        }
        for (ssize_t index = 0; index < count && !self.closing; index++) {
            if (buffer[index] != '\n') {
                if (line.length >= ProofLineLimit) {
                    [self writeReply:@{@"error":@"Request too large."}];
                    dispatch_async(dispatch_get_main_queue(), ^{ [self finishWithReply:NO status:2]; });
                    return;
                }
                [line appendBytes:buffer + index length:1];
                continue;
            }
            @autoreleasepool {
                id command = [NSJSONSerialization JSONObjectWithData:line options:0 error:nil];
                [line setLength:0];
                NSDictionary *reply;
                if (![command isKindOfClass:NSDictionary.class]) reply = @{@"error":@"Invalid command."};
                else if (command[@"command"]) {
                    if ([command count] != 1) reply = @{@"error":@"Invalid command."};
                    else if ([command[@"command"] isEqual:@"quit"]) {
                        dispatch_async(dispatch_get_main_queue(), ^{ [self finishWithReply:YES status:0]; });
                        return;
                    } else if ([command[@"command"] isEqual:@"check-journals"] && self.ready) {
                        reply = [self journalsSafe] ? @{@"ok":@YES} : @{@"error":@"Journal safety check failed."};
                    } else reply = @{@"error":@"Command refused."};
                } else reply = [self performRequest:command];
                if (!self.closing && ![self writeReply:reply]) {
                    dispatch_async(dispatch_get_main_queue(), ^{ [self finishWithReply:NO status:1]; });
                    return;
                }
            }
        }
    }
}

@end

int main(int argc, const char *argv[]) {
    (void)argv;
    @autoreleasepool {
        struct stat input, output, errorOutput;
        if (argc != 1 || fstat(STDIN_FILENO, &input) != 0 || !S_ISFIFO(input.st_mode) ||
            fstat(STDOUT_FILENO, &output) != 0 || !S_ISFIFO(output.st_mode) ||
            fstat(STDERR_FILENO, &errorOutput) != 0 || !S_ISREG(errorOutput.st_mode)) return 2;
        NSString *home = ProofHome();
        if (!home) return 2;
        NSString *stderrPath = [home stringByAppendingPathComponent:@"shell.err"];
        struct stat journal;
        if (lstat(stderrPath.fileSystemRepresentation, &journal) != 0 || !S_ISREG(journal.st_mode) ||
            journal.st_dev != errorOutput.st_dev || journal.st_ino != errorOutput.st_ino) return 2;
        if (fcntl(STDOUT_FILENO, F_SETNOSIGPIPE, 1) < 0) return 2;
        umask(0077);
        int quitFD = -1;
        if (shell_stop_arm(&quitFD) != 0) return 1;
        EzerbotOperatorProof *host = EzerbotOperatorProof.new;
        host.home = home;
        host.outputLock = NSLock.new;
        host.shell = GrokbotApplication.new;
        host.startupDeadline = NSProcessInfo.processInfo.systemUptime + 240;
        NSApplication *application = NSApplication.sharedApplication;
        [application setActivationPolicy:NSApplicationActivationPolicyAccessory];
        application.delegate = host.shell;
        dispatch_source_t quitSource = dispatch_source_create(DISPATCH_SOURCE_TYPE_READ, quitFD, 0, dispatch_get_main_queue());
        if (!quitSource) return 1;
        dispatch_source_set_event_handler(quitSource, ^{
            if (shell_stop_take_quit(quitFD)) [host finishWithReply:NO status:0];
        });
        dispatch_resume(quitSource);
        [application finishLaunching];
        host.readinessTimer = dispatch_source_create(DISPATCH_SOURCE_TYPE_TIMER, 0, 0, dispatch_get_main_queue());
        dispatch_source_set_timer(host.readinessTimer, DISPATCH_TIME_NOW, 100 * NSEC_PER_MSEC, 10 * NSEC_PER_MSEC);
        dispatch_source_set_event_handler(host.readinessTimer, ^{ [host pollWorkspace]; });
        dispatch_resume(host.readinessTimer);
        dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0), ^{ [host readCommands]; });
        [application run];
        if (!host.closing) [host finishWithReply:NO status:1];
        dispatch_source_cancel(quitSource);
        close(quitFD);
        return host.exitStatus;
    }
}
