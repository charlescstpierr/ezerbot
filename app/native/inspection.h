#import <pwd.h>

static NSString *EzerbotInspectionHome(int argc) {
    struct stat input, output, errorOutput;
    if (argc != 2 || fstat(STDIN_FILENO, &input) != 0 || !S_ISFIFO(input.st_mode) || input.st_nlink != 0 ||
        fstat(STDOUT_FILENO, &output) != 0 || !S_ISFIFO(output.st_mode) || output.st_nlink != 0 ||
        fstat(STDERR_FILENO, &errorOutput) != 0 || !S_ISREG(errorOutput.st_mode)) return nil;
    NSDictionary *environment = NSProcessInfo.processInfo.environment;
    NSString *home = environment[@"HOME"];
    struct passwd *owner = getpwuid(getuid());
    NSString *actual = owner ? [NSString stringWithUTF8String:owner->pw_dir] : nil;
    NSString *support = [actual stringByAppendingPathComponent:@"Library/Application Support/Grokbot"];
    if (!actual || !home.isAbsolutePath || ![home isEqual:environment[@"CFFIXED_USER_HOME"]] ||
        ![home isEqual:environment[@"EZERBOT_TEST_HOME"]] || [home isEqual:actual] || [home isEqual:@"/"] ||
        [actual hasPrefix:[home stringByAppendingString:@"/"]] || [home isEqual:support] ||
        [home hasPrefix:[support stringByAppendingString:@"/"]] ||
        ![home isEqual:home.stringByStandardizingPath.stringByResolvingSymlinksInPath]) return nil;
    struct stat directory, journal;
    NSString *log = [home stringByAppendingPathComponent:@"shell.err"];
    if (lstat(home.fileSystemRepresentation, &directory) != 0 || !S_ISDIR(directory.st_mode) ||
        directory.st_uid != getuid() || (directory.st_mode & 0777) != 0700 ||
        lstat(log.fileSystemRepresentation, &journal) != 0 || !S_ISREG(journal.st_mode) ||
        journal.st_uid != getuid() || journal.st_dev != errorOutput.st_dev || journal.st_ino != errorOutput.st_ino) return nil;
    NSString *isolatedSupport = [home stringByAppendingPathComponent:@"Library/Application Support/Grokbot"];
    for (NSString *path in @[isolatedSupport, [isolatedSupport stringByAppendingPathComponent:@"logs"],
                             [isolatedSupport stringByAppendingPathComponent:@"logs/engine.log"],
                             [isolatedSupport stringByAppendingPathComponent:@"logs/engine.previous.log"],
                             [isolatedSupport stringByAppendingPathComponent:@"workspace.sqlite"],
                             [isolatedSupport stringByAppendingPathComponent:@"workspace.codex-auth.json"],
                             [isolatedSupport stringByAppendingPathComponent:@"workspace.secrets.json"]]) {
        if (![path isEqual:path.stringByResolvingSymlinksInPath]) return nil;
    }
    if (fcntl(STDIN_FILENO, F_SETFD, FD_CLOEXEC) < 0 || fcntl(STDOUT_FILENO, F_SETFD, FD_CLOEXEC) < 0 ||
        fcntl(STDOUT_FILENO, F_SETNOSIGPIPE, 1) < 0) return nil;
    return home;
}

@interface EzerbotPrivateInspection : NSObject
@property(nonatomic, strong) GrokbotApplication *shell;
@property(nonatomic, strong) dispatch_source_t timer;
@property(nonatomic) NSTimeInterval deadline;
@property(nonatomic) BOOL ready;
@property(nonatomic) BOOL pending;
@property(nonatomic) NSUInteger generation;
@property(nonatomic, copy) NSString *identity;
@property(nonatomic) dev_t engineLogDevice;
@property(nonatomic) ino_t engineLogInode;
- (instancetype)initWithShell:(GrokbotApplication *)shell;
- (void)start;
@end

@implementation EzerbotPrivateInspection

- (instancetype)initWithShell:(GrokbotApplication *)shell {
    if ((self = [super init])) self.shell = shell;
    return self;
}

- (void)writeReply:(NSDictionary *)reply {
    NSData *data = [NSJSONSerialization dataWithJSONObject:reply options:0 error:nil];
    if (!data || data.length > 4096) { [NSApp terminate:nil]; return; }
    NSMutableData *line = [data mutableCopy];
    [line appendBytes:"\n" length:1];
    const unsigned char *bytes = line.bytes;
    NSUInteger remaining = line.length;
    while (remaining) {
        ssize_t count = write(STDOUT_FILENO, bytes, remaining);
        if (count < 0 && errno == EINTR) continue;
        if (count <= 0) { [NSApp terminate:nil]; return; }
        bytes += count;
        remaining -= (NSUInteger)count;
    }
}

- (NSDictionary *)runtime {
    GrokbotApplication *shell = self.shell;
    return @{@"host_pid":@(getpid()), @"engine_pid":@(shell.backendTask.processIdentifier),
             @"url":shell.backendURL.absoluteString};
}

- (BOOL)available {
    GrokbotApplication *shell = self.shell;
    return !shell.terminating && shell.backendTask.running && shell.localAPI && shell.webView &&
        shell.backendURL && [shell isWorkspaceURL:shell.webView.URL];
}

- (BOOL)journalsSafe {
    if (self.identity.length != 64) return NO;
    NSString *home = self.shell.inspectionHome;
    NSString *support = [home stringByAppendingPathComponent:@"Library/Application Support/Grokbot"];
    NSString *previous = [support stringByAppendingPathComponent:@"logs/engine.previous.log"];
    NSString *shellLog = [home stringByAppendingPathComponent:@"shell.err"];
    NSString *engineLog = [support stringByAppendingPathComponent:@"logs/engine.log"];
    for (NSString *path in @[shellLog, engineLog, previous]) {
        if (![path isEqual:path.stringByResolvingSymlinksInPath]) return NO;
        int descriptor = open(path.fileSystemRepresentation, O_RDONLY | O_NOFOLLOW | O_CLOEXEC);
        if (descriptor < 0) {
            if (errno == ENOENT && [path isEqual:previous]) continue;
            return NO;
        }
        struct stat status;
        BOOL safe = fstat(descriptor, &status) == 0 && S_ISREG(status.st_mode) && status.st_uid == getuid();
        if ([path isEqual:shellLog]) {
            struct stat standardError;
            safe = safe && fstat(STDERR_FILENO, &standardError) == 0 &&
                standardError.st_dev == status.st_dev && standardError.st_ino == status.st_ino;
        }
        if ([path isEqual:engineLog]) {
            safe = safe && status.st_dev == self.engineLogDevice && status.st_ino == self.engineLogInode;
        }
        NSMutableData *data = [NSMutableData data];
        unsigned char buffer[8192];
        while (safe) {
            ssize_t count = read(descriptor, buffer, sizeof buffer);
            if (count < 0 && errno == EINTR) continue;
            if (count < 0) { safe = NO; break; }
            if (!count) break;
            if (data.length + (NSUInteger)count > 16 * 1024 * 1024) { safe = NO; break; }
            [data appendBytes:buffer length:(NSUInteger)count];
        }
        close(descriptor);
        NSString *text = safe ? [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding] : nil;
        if (!text || [text containsString:self.identity] ||
            [text rangeOfString:@"eyJ[A-Za-z0-9_-]{8,}|Bearer\\s+\\S+|refresh_token|access_token|id_token"
                         options:NSRegularExpressionSearch | NSCaseInsensitiveSearch].location != NSNotFound) return NO;
    }
    return YES;
}

- (void)inspectState {
    if (!self.ready || self.pending || ![self available] || self.shell.startupGeneration != self.generation || ![self journalsSafe]) {
        [self writeReply:@{@"error":@"state_failed"}];
        return;
    }
    self.pending = YES;
    GrokbotApplication *shell = self.shell;
    NSTask *engine = shell.backendTask;
    EzerbotLocalAPI *api = shell.localAPI;
    WKWebView *view = shell.webView;
    NSUInteger generation = shell.startupGeneration;
    NSDictionary *runtime = [self runtime];
    [view callAsyncJavaScript:
        @"const response = await fetch('/api/state', {cache:'no-store'});"
         "if (response.status !== 200) throw new Error('state_failed');"
         "const state = await response.json();"
         "if (!['simulated','chatgpt'].includes(state.mode)) throw new Error('state_failed');"
         "return {status:200, mode:state.mode};"
        arguments:@{} inFrame:nil inContentWorld:WKContentWorld.pageWorld completionHandler:^(id result, NSError *error) {
            self.pending = NO;
            if (shell.terminating) return;
            if (![self available] || shell.backendTask != engine || shell.localAPI != api || shell.webView != view ||
                shell.startupGeneration != generation || ![runtime isEqual:[self runtime]]) {
                [self writeReply:@{@"error":@"stale_runtime"}];
                return;
            }
            NSDictionary *state = !error && [result isKindOfClass:NSDictionary.class] ? result : nil;
            NSString *mode = state[@"mode"];
            if (![state[@"status"] isEqual:@200] || ![@[@"simulated", @"chatgpt"] containsObject:mode ?: NSNull.null] || ![self journalsSafe]) {
                [self writeReply:@{@"error":@"state_failed"}];
                return;
            }
            NSMutableDictionary *reply = [runtime mutableCopy];
            reply[@"status"] = @200;
            reply[@"mode"] = mode;
            reply[@"journals_safe"] = @YES;
            [self writeReply:reply];
        }];
}

- (void)pollReady {
    if (NSProcessInfo.processInfo.systemUptime >= self.deadline) {
        dispatch_source_cancel(self.timer);
        [self writeReply:@{@"error":@"inspection_failed"}];
        [NSApp terminate:nil];
        return;
    }
    if (self.pending || ![self available]) return;
    self.pending = YES;
    WKWebView *view = self.shell.webView;
    NSTask *engine = self.shell.backendTask;
    NSUInteger generation = self.shell.startupGeneration;
    [view evaluateJavaScript:
        @"Boolean(document.readyState === 'complete' && window.EzerbotLocal && document.querySelector('#composer-tools'))"
        completionHandler:^(id result, NSError *error) {
            self.pending = NO;
            if (self.ready || self.shell.terminating || error || ![result isEqual:@YES] || ![self available] ||
                self.shell.webView != view || self.shell.backendTask != engine || self.shell.startupGeneration != generation) return;
            struct stat engineLog;
            if (!self.shell.logHandle || fstat(self.shell.logHandle.fileDescriptor, &engineLog) != 0 || !S_ISREG(engineLog.st_mode)) {
                [self writeReply:@{@"error":@"inspection_failed"}];
                [NSApp terminate:nil];
                return;
            }
            self.engineLogDevice = engineLog.st_dev;
            self.engineLogInode = engineLog.st_ino;
            self.ready = YES;
            self.generation = self.shell.startupGeneration;
            self.identity = self.shell.localAPI.launchSecret;
            dispatch_source_cancel(self.timer);
            NSMutableDictionary *reply = [[self runtime] mutableCopy];
            reply[@"ready"] = @YES;
            [self writeReply:reply];
        }];
}

- (void)readCommands {
    char line[6];
    NSUInteger length = 0;
    for (;;) {
        char byte;
        ssize_t count = read(STDIN_FILENO, &byte, 1);
        if (count < 0 && errno == EINTR) continue;
        if (count <= 0) {
            dispatch_async(dispatch_get_main_queue(), ^{ [NSApp terminate:nil]; });
            return;
        }
        if (byte == '\n' && length == 5 && memcmp(line, "state", 5) == 0) {
            length = 0;
            dispatch_async(dispatch_get_main_queue(), ^{ [self inspectState]; });
        } else if (byte == '\n' || length >= 5) {
            dispatch_async(dispatch_get_main_queue(), ^{
                [self writeReply:@{@"error":@"invalid_command"}];
                [NSApp terminate:nil];
            });
            return;
        } else line[length++] = byte;
    }
}

- (void)start {
    __weak EzerbotPrivateInspection *weakSelf = self;
    self.shell.inspectionJournalsSafe = ^{ return [weakSelf journalsSafe]; };
    self.deadline = NSProcessInfo.processInfo.systemUptime + 240;
    self.timer = dispatch_source_create(DISPATCH_SOURCE_TYPE_TIMER, 0, 0, dispatch_get_main_queue());
    dispatch_source_set_timer(self.timer, DISPATCH_TIME_NOW, 100 * NSEC_PER_MSEC, 10 * NSEC_PER_MSEC);
    dispatch_source_set_event_handler(self.timer, ^{ [self pollReady]; });
    dispatch_resume(self.timer);
    dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INITIATED, 0), ^{ [self readCommands]; });
}

@end
