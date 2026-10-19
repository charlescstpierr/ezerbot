// Cocoa helper for one ephemeral conversation lease. Included only by main.m.
// Debug compilation excludes EVERY TCC, capture, event-tap and CGEventPost path.
#import <ApplicationServices/ApplicationServices.h>
#import <pthread.h>
#import <sys/file.h>
#import <fcntl.h>
#import <libproc.h>
#import <Security/Security.h>

static pthread_mutex_t ezPermit = PTHREAD_MUTEX_INITIALIZER;
static BOOL ezRevoked = NO, ezPaused = NO;
static uint64_t ezGeneration = 1;
static double ezDeadline;
#if defined(EZERBOT_RELEASE)
static const int64_t ezEventTag = 0x455A4552424F54;
#endif
static NSPanel *ezOverlay;
static NSTextField *ezOverlayLabel;
static BOOL ezBusy = NO;
#if defined(EZERBOT_RELEASE)
static NSRunningApplication *ezOwner;
static NSInteger ezImageWidth, ezImageHeight;
static CGRect ezCaptureBounds;
#endif
// The image delivered to the model is the sole coordinate space. This pure
// transform is also tested in debug, without reading any real display.
static __attribute__((unused)) BOOL EZImagePoint(double x, double y, NSInteger width, NSInteger height, CGRect bounds, CGPoint *point) {
    if (!isfinite(x) || !isfinite(y) || width <= 0 || height <= 0 || x < 0 || y < 0 || x >= width || y >= height) return NO;
    *point = CGPointMake(bounds.origin.x + x * bounds.size.width / width, bounds.origin.y + y * bounds.size.height / height);
    return YES;
}
static void EZReply(NSDictionary *value) {
    @synchronized ([NSFileHandle fileHandleWithStandardOutput]) {
        NSData *data = [NSJSONSerialization dataWithJSONObject:value options:0 error:nil];
        [[NSFileHandle fileHandleWithStandardOutput] writeData:data];
        [[NSFileHandle fileHandleWithStandardOutput] writeData:[@"\n" dataUsingEncoding:NSUTF8StringEncoding]];
    }
}
static BOOL EZLive(uint64_t generation) {
    return !ezRevoked && !ezPaused && generation == ezGeneration && NSProcessInfo.processInfo.systemUptime < ezDeadline;
}
static void EZOverlayUpdate(void) {
    pthread_mutex_lock(&ezPermit);
    BOOL stopped = ezRevoked, paused = ezPaused;
    pthread_mutex_unlock(&ezPermit);
    if (stopped) [ezOverlay orderOut:nil];
    else {
        ezOverlayLabel.stringValue = paused ? @"Ezerbot en pause — Reprenez dans l’application" : @"Ezerbot contrôle l’écran — Échap pour arrêter";
        [ezOverlay orderFrontRegardless];
    }
}
static void EZStop(BOOL pause) {
    pthread_mutex_lock(&ezPermit);
    if (ezRevoked || (pause && ezPaused)) { pthread_mutex_unlock(&ezPermit); return; }
    if (pause) ezPaused = YES; else ezRevoked = YES;
    ++ezGeneration;
    pthread_mutex_unlock(&ezPermit);
    EZReply(@{@"event":pause ? @"paused" : @"revoked"});
    dispatch_async(dispatch_get_main_queue(), ^{ EZOverlayUpdate(); if (!pause) [NSApp terminate:nil]; });
}
// The exception is for a proven Ezerbot WINDOW, never merely “Ezerbot is
// frontmost”. Unknown ownership fails closed. Pure metadata fixtures exercise
// exactly this policy in debug, without an event tap or window-server access.
static __attribute__((unused)) BOOL EZInputForShellWindow(BOOL keyboard, pid_t ownerPID, pid_t targetPID, pid_t pointerWindowPID, BOOL focusedShellWindow) {
    if (ownerPID <= 0) return NO;
    return keyboard ? targetPID == ownerPID && focusedShellWindow : pointerWindowPID == ownerPID;
}
static __attribute__((unused)) void EZObserveHumanInput(BOOL escape, BOOL shellWindow) {
    if (escape) EZStop(NO);
    else if (!shellWindow) EZStop(YES);
}
#if defined(EZERBOT_RELEASE)
static BOOL EZParent(void) {
    char path[PROC_PIDPATHINFO_MAXSIZE] = {0};
    if (getppid() <= 1 || proc_pidpath(getppid(), path, sizeof(path)) <= 0) return NO;
    NSString *expected = [NSBundle.mainBundle.executablePath.stringByDeletingLastPathComponent stringByAppendingPathComponent:@"grokbot-wrapper"];
    if (![expected isEqualToString:[NSString stringWithUTF8String:path]]) return NO;
    struct proc_bsdinfo info = {0};
    if (proc_pidinfo(getppid(),PROC_PIDTBSDINFO,0,&info,sizeof(info)) != sizeof(info) || info.pbi_uid != getuid()) return NO;
    SecStaticCodeRef code = NULL;
    OSStatus status = SecStaticCodeCreateWithPath((__bridge CFURLRef)[NSURL fileURLWithPath:expected],kSecCSDefaultFlags,&code);
    if (status == errSecSuccess) status = SecStaticCodeCheckValidity(code,kSecCSStrictValidate,NULL);
    if (code) CFRelease(code);
    if (status != errSecSuccess) return NO;
    ezOwner = [NSRunningApplication runningApplicationWithProcessIdentifier:(pid_t)info.pbi_ppid];
    return [ezOwner.executableURL.path isEqualToString:NSBundle.mainBundle.executablePath];
}
static NSString *EZPermissions(void) {
    if (!AXIsProcessTrusted()) return @"Permission Accessibilité manquante. Autorisez Ezerbot dans Réglages Système › Confidentialité et sécurité › Accessibilité.";
    if (!CGPreflightScreenCaptureAccess()) return @"Permission Enregistrement de l’écran manquante. Autorisez Ezerbot dans Réglages Système › Confidentialité et sécurité › Enregistrement de l’écran.";
    if (!CGPreflightListenEventAccess()) return @"Permission Surveillance de l’entrée manquante. Autorisez Ezerbot dans Réglages Système › Confidentialité et sécurité › Surveillance de l’entrée pour garantir la reprise humaine.";
    return nil;
}
static BOOL EZEventForShellWindow(CGEventType type, CGEventRef event) {
    pid_t owner = ezOwner.processIdentifier;
    if (owner <= 0 || ezOwner.terminated) return NO;
    BOOL keyboard = type == kCGEventKeyDown || type == kCGEventKeyUp || type == kCGEventFlagsChanged;
    pid_t target = (pid_t)CGEventGetIntegerValueField(event,kCGEventTargetUnixProcessID);
    pid_t pointerOwner = 0;
    BOOL focusedShellWindow = NO;
    if (keyboard && target == owner && NSWorkspace.sharedWorkspace.frontmostApplication.processIdentifier == owner) {
        AXUIElementRef app = AXUIElementCreateApplication(owner); CFTypeRef window = NULL;
        // Never let a busy WebView stall the event tap. Missing/slow metadata
        // pauses instead of accidentally exempting an unrelated input.
        AXUIElementSetMessagingTimeout(app,0.05f);
        if (AXUIElementCopyAttributeValue(app,kAXFocusedWindowAttribute,&window) == kAXErrorSuccess && window && CFGetTypeID(window) == AXUIElementGetTypeID()) {
            pid_t windowOwner = 0;
            focusedShellWindow = AXUIElementGetPid((AXUIElementRef)window,&windowOwner) == kAXErrorSuccess && windowOwner == owner;
        }
        if (window) CFRelease(window); CFRelease(app);
    } else if (!keyboard) {
        // Annotated events identify the actual window receiving mouse input,
        // including through our click-through overlay. A click on another app
        // still pauses even while Ezerbot remains the frontmost application.
        int64_t number = CGEventGetIntegerValueField(event,kCGMouseEventWindowUnderMousePointerThatCanHandleThisEvent);
        if (number > 0 && number <= UINT32_MAX) {
            NSArray *windows = CFBridgingRelease(CGWindowListCopyWindowInfo(kCGWindowListOptionIncludingWindow,(CGWindowID)number));
            for (NSDictionary *window in windows) {
                CGRect bounds;
                if ([window[(id)kCGWindowNumber] longLongValue] == number && [window[(id)kCGWindowIsOnscreen] boolValue] && [window[(id)kCGWindowBounds] isKindOfClass:NSDictionary.class] && CGRectMakeWithDictionaryRepresentation((__bridge CFDictionaryRef)window[(id)kCGWindowBounds],&bounds) && CGRectContainsPoint(bounds,CGEventGetLocation(event))) {
                    pointerOwner = [window[(id)kCGWindowOwnerPID] intValue]; break;
                }
            }
        }
    }
    return EZInputForShellWindow(keyboard,owner,target,pointerOwner,focusedShellWindow);
}
static CGEventRef EZHumanInput(CGEventTapProxy proxy, CGEventType type, CGEventRef event, void *context) {
    (void)proxy; (void)context;
    if (type == kCGEventTapDisabledByTimeout || type == kCGEventTapDisabledByUserInput) { EZStop(NO); return event; }
    if (CGEventGetIntegerValueField(event,kCGEventSourceUserData) == ezEventTag) return event;
    BOOL escape = type == kCGEventKeyDown && CGEventGetIntegerValueField(event,kCGKeyboardEventKeycode) == 53;
    EZObserveHumanInput(escape, !escape && EZEventForShellWindow(type,event));
    return event;
}
// A complete down/up pair shares the permit lock. There are NO held inputs or
// sleeps under this lock: revoke linearizes before every subsequent pair.
static BOOL EZPostPair(CGEventRef down, CGEventRef up, uint64_t generation) {
    if (!down || !up) { if(down)CFRelease(down); if(up)CFRelease(up); return NO; }
    pthread_mutex_lock(&ezPermit);
    BOOL live = EZLive(generation);
    if (live) {
        CGEventSetIntegerValueField(down,kCGEventSourceUserData,ezEventTag);
        CGEventSetIntegerValueField(up,kCGEventSourceUserData,ezEventTag);
        CGEventPost(kCGHIDEventTap,down); CGEventPost(kCGHIDEventTap,up);
    }
    pthread_mutex_unlock(&ezPermit);
    CFRelease(down); CFRelease(up);
    return live;
}
static BOOL EZSecureFocus(void) {
    // Fail closed when focused-element metadata is unavailable. Password text
    // must be entered by the person, not persisted in a model argument/card.
    AXUIElementRef system = AXUIElementCreateSystemWide(); CFTypeRef element = NULL, subrole = NULL, role = NULL;
    AXError result = AXUIElementCopyAttributeValue(system,kAXFocusedUIElementAttribute,&element);
    BOOL secure = YES;
    if (result == kAXErrorSuccess && element) {
        AXError roleResult = AXUIElementCopyAttributeValue((AXUIElementRef)element,kAXRoleAttribute,&role);
        AXError subresult = AXUIElementCopyAttributeValue((AXUIElementRef)element,kAXSubroleAttribute,&subrole);
        BOOL textRole = roleResult == kAXErrorSuccess && role && (CFEqual(role,kAXTextFieldRole) || CFEqual(role,kAXTextAreaRole) || CFEqual(role,kAXComboBoxRole));
        BOOL known = subresult == kAXErrorSuccess || subresult == kAXErrorAttributeUnsupported || subresult == kAXErrorNoValue;
        secure = !textRole || !known || (subrole && CFEqual(subrole,kAXSecureTextFieldSubrole));
    }
    if (role) CFRelease(role); if (subrole) CFRelease(subrole); if (element) CFRelease(element); CFRelease(system);
    return secure;
}
#endif
static NSDictionary *EZCapture(void) {
#if defined(EZERBOT_RELEASE)
    NSString *permission = EZPermissions(); if(permission) return @{@"error":permission};
    CGImageRef image = CGDisplayCreateImage(CGMainDisplayID());
    if(!image) return @{@"error":@"La capture de l’écran est indisponible. Vérifiez Enregistrement de l’écran dans Réglages Système."};
    NSImage *source = [[NSImage alloc] initWithCGImage:image size:NSZeroSize];
    CGDirectDisplayID display = CGMainDisplayID(); CGRect bounds = CGDisplayBounds(display);
    NSInteger width = MIN(1280,CGImageGetWidth(image));
    NSInteger height = MAX(1,llround((double)width*CGImageGetHeight(image)/CGImageGetWidth(image)));
    NSSize size = NSMakeSize(width,height);
    NSBitmapImageRep *bitmap = [[NSBitmapImageRep alloc] initWithBitmapDataPlanes:NULL pixelsWide:(NSInteger)size.width pixelsHigh:(NSInteger)size.height bitsPerSample:8 samplesPerPixel:4 hasAlpha:YES isPlanar:NO colorSpaceName:NSDeviceRGBColorSpace bytesPerRow:0 bitsPerPixel:0];
    [NSGraphicsContext saveGraphicsState]; NSGraphicsContext.currentContext = [NSGraphicsContext graphicsContextWithBitmapImageRep:bitmap];
    [source drawInRect:NSMakeRect(0,0,size.width,size.height)]; [NSGraphicsContext restoreGraphicsState];
    NSData *png = [bitmap representationUsingType:NSBitmapImageFileTypePNG properties:@{}]; CFRelease(image);
    if(!png || png.length>4*1024*1024) return @{@"error":@"La capture dépasse la taille permise. Réduisez la résolution de l’écran."};
    ezImageWidth = width; ezImageHeight = height; ezCaptureBounds = bounds;
    return @{@"image":[@"data:image/png;base64," stringByAppendingString:[png base64EncodedStringWithOptions:0]],@"width":@(width),@"height":@(height)};
#else
    return @{@"image":@"data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a1ioAAAAASUVORK5CYII=",@"width":@1280,@"height":@800,@"fixture":@YES};
#endif
}
static void EZFinish(NSNumber *identifier, NSString *error) {
    pthread_mutex_lock(&ezPermit); BOOL live = EZLive(ezGeneration); pthread_mutex_unlock(&ezPermit);
    NSMutableDictionary *reply = [NSMutableDictionary dictionaryWithDictionary:error ? @{@"error":error} : !live ? @{@"error":@"Contrôle en pause ou arrêté. Un effet déjà livré ne peut pas être annulé."} : EZCapture()];
    reply[@"id"] = identifier; ezBusy = NO;
#if defined(EZERBOT_RELEASE)
    // Show the decision UI again, unless real input has taken over. Never steal
    // focus back from the person after a pause or revocation.
    if (live) [ezOwner activateWithOptions:NSApplicationActivateIgnoringOtherApps];
#endif
    EZReply(reply);
}
static void EZType(NSString *text, NSUInteger index, uint64_t generation, NSNumber *identifier) {
    pthread_mutex_lock(&ezPermit); BOOL live=EZLive(generation); pthread_mutex_unlock(&ezPermit);
    if(!live) {EZFinish(identifier,@"Saisie arrêtée par vous ou à l’expiration de la session.");return;}
    if(index >= text.length) {EZFinish(identifier,nil);return;}
    NSRange scalar = [text rangeOfComposedCharacterSequenceAtIndex:index];
#if defined(EZERBOT_RELEASE)
    if(EZSecureFocus()) {EZFinish(identifier,@"Champ sécurisé ou cible invérifiable : saisissez vous-même le mot de passe après avoir révoqué le contrôle.");return;}
    NSString *part = [text substringWithRange:scalar];
    UniChar characters[128];
    if(part.length>128) {EZFinish(identifier,@"Caractère composé trop long.");return;}
    [part getCharacters:characters range:NSMakeRange(0,part.length)];
    CGEventRef down=CGEventCreateKeyboardEvent(NULL,0,true), up=CGEventCreateKeyboardEvent(NULL,0,false);
    if(down)CGEventKeyboardSetUnicodeString(down,part.length,characters); if(up)CGEventKeyboardSetUnicodeString(up,part.length,characters);
    if(!EZPostPair(down,up,generation)) {EZFinish(identifier,@"Saisie arrêtée.");return;}
#endif
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW,2*NSEC_PER_MSEC),dispatch_get_main_queue(),^{EZType(text,NSMaxRange(scalar),generation,identifier);});
}
static void EZPerformAction(NSDictionary *request, uint64_t generation) {
    NSNumber *identifier = request[@"id"];
    pthread_mutex_lock(&ezPermit); BOOL live=EZLive(generation); pthread_mutex_unlock(&ezPermit);
    if(!live || ezBusy) {EZReply(@{@"id":identifier?:@0,@"error":@"Action en file annulée : contrôle en pause, occupé ou révoqué."});return;}
    ezBusy=YES;
    NSString *name=request[@"name"]; NSDictionary *args=request[@"arguments"];
#if defined(EZERBOT_RELEASE)
    NSString *permission=EZPermissions(); if(permission) {EZFinish(identifier,permission);return;}
#endif
    if([name isEqualToString:@"computer_type"]) {EZType(args[@"text"],0,generation,identifier);return;}
#if defined(EZERBOT_RELEASE)
    CGRect bounds=CGDisplayBounds(CGMainDisplayID());
    if([name isEqualToString:@"computer_move"] || [name isEqualToString:@"computer_click"]) {
        double x=[args[@"x"] doubleValue], y=[args[@"y"] doubleValue];
        if(!ezImageWidth || !CGRectEqualToRect(bounds,ezCaptureBounds)) {EZFinish(identifier,@"Prenez une nouvelle capture avant de déplacer la souris ou cliquer : les dimensions de l’écran ont changé ou sont inconnues.");return;}
        CGPoint point;
        if(!EZImagePoint(x,y,ezImageWidth,ezImageHeight,bounds,&point)) {EZFinish(identifier,@"Coordonnées hors de la dernière capture. Utilisez ses dimensions en pixels.");return;}
        BOOL right=[args[@"button"] isEqualToString:@"right"], move=[name isEqualToString:@"computer_move"];
        CGMouseButton button=right?kCGMouseButtonRight:kCGMouseButtonLeft;
        CGEventType down=move?kCGEventMouseMoved:right?kCGEventRightMouseDown:kCGEventLeftMouseDown;
        CGEventType up=move?kCGEventMouseMoved:right?kCGEventRightMouseUp:kCGEventLeftMouseUp;
        if(!EZPostPair(CGEventCreateMouseEvent(NULL,down,point,button),CGEventCreateMouseEvent(NULL,up,point,button),generation)) {EZFinish(identifier,@"Action arrêtée.");return;}
    } else if([name isEqualToString:@"computer_scroll"]) {
        CGEventRef event=CGEventCreateScrollWheelEvent(NULL,kCGScrollEventUnitPixel,1,[args[@"delta_y"] intValue]);
        // Two halves sum to the requested delta (no held scroll state).
        if(!event) {EZFinish(identifier,@"Défilement indisponible.");return;}
        pthread_mutex_lock(&ezPermit);
        if(EZLive(generation)) {CGEventSetIntegerValueField(event,kCGEventSourceUserData,ezEventTag);CGEventPost(kCGHIDEventTap,event);}
        pthread_mutex_unlock(&ezPermit); CFRelease(event);
    } else if([name isEqualToString:@"computer_key"]) {
        NSDictionary *keys=@{@"ENTER":@36,@"TAB":@48,@"ESC":@53,@"BACKSPACE":@51,@"DELETE":@117,@"UP":@126,@"DOWN":@125,@"LEFT":@123,@"RIGHT":@124,@"SPACE":@49};
        NSNumber *key=keys[args[@"key"]]; if(!key) {EZFinish(identifier,@"Touche non autorisée.");return;}
        if(!EZPostPair(CGEventCreateKeyboardEvent(NULL,key.unsignedShortValue,true),CGEventCreateKeyboardEvent(NULL,key.unsignedShortValue,false),generation)) {EZFinish(identifier,@"Touche arrêtée.");return;}
    } else if(![name isEqualToString:@"computer_screenshot"]) {EZFinish(identifier,@"Outil d’écran inconnu.");return;}
#endif
    EZFinish(identifier,nil);
}
static void EZAction(NSDictionary *request, uint64_t generation) {
#if defined(EZERBOT_RELEASE)
    // The approval UI must not become the input target. Hide only the proven
    // parent shell, reveal the exact desktop photographed for the model, and
    // allow its window ordering to settle. No other Ezerbot instance is touched.
    [ezOwner hide];
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW,150*NSEC_PER_MSEC),dispatch_get_main_queue(),^{EZPerformAction(request,generation);});
#else
    EZPerformAction(request,generation);
#endif
}
static int EZScreenMain(int argc, const char *argv[]) {
    BOOL permissions=argc==2 && strcmp(argv[1],"--screen-permissions")==0;
#if defined(EZERBOT_RELEASE)
    if(!EZParent()) return 77;
    if(permissions) {
        // The sole permission-request call site, reached only by explicit first
        // enable in Paramètres. The debug binary does not contain these calls.
        if(!AXIsProcessTrusted()) AXIsProcessTrustedWithOptions((__bridge CFDictionaryRef)@{(__bridge NSString *)kAXTrustedCheckOptionPrompt:@YES});
        if(!CGPreflightScreenCaptureAccess()) CGRequestScreenCaptureAccess();
        if(!CGPreflightListenEventAccess()) CGRequestListenEventAccess();
        return EZPermissions()?1:0;
    }
#else
    if(permissions) return 0;
#endif
    if(argc!=3) return 64;
    long duration=strtol(argv[2],NULL,10); if(duration<100||duration>900000) return 64;
    ezDeadline=NSProcessInfo.processInfo.systemUptime+(double)duration/1000;
    NSApplication *app=[NSApplication sharedApplication]; [app setActivationPolicy:NSApplicationActivationPolicyAccessory];
#if defined(EZERBOT_RELEASE)
    NSString *permission=EZPermissions(); if(permission){EZReply(@{@"error":permission});return 1;}
    NSString *lockPath=[NSTemporaryDirectory() stringByAppendingPathComponent:[NSString stringWithFormat:@"ezerbot-screen-%u.lock",getuid()]];
    int lock=open(lockPath.fileSystemRepresentation,O_CREAT|O_RDWR|O_NOFOLLOW,0600);
    if(lock<0||flock(lock,LOCK_EX|LOCK_NB)!=0){EZReply(@{@"error":@"Ce Mac est déjà contrôlé par une autre session Ezerbot."});return 1;}
    CGEventMask mask=CGEventMaskBit(kCGEventKeyDown)|CGEventMaskBit(kCGEventKeyUp)|CGEventMaskBit(kCGEventFlagsChanged)|CGEventMaskBit(kCGEventMouseMoved)|CGEventMaskBit(kCGEventLeftMouseDown)|CGEventMaskBit(kCGEventLeftMouseUp)|CGEventMaskBit(kCGEventRightMouseDown)|CGEventMaskBit(kCGEventRightMouseUp)|CGEventMaskBit(kCGEventOtherMouseDown)|CGEventMaskBit(kCGEventOtherMouseUp)|CGEventMaskBit(kCGEventScrollWheel)|CGEventMaskBit(kCGEventLeftMouseDragged)|CGEventMaskBit(kCGEventRightMouseDragged)|CGEventMaskBit(kCGEventOtherMouseDragged);
    CFMachPortRef tap=CGEventTapCreate(kCGAnnotatedSessionEventTap,kCGHeadInsertEventTap,kCGEventTapOptionListenOnly,mask,EZHumanInput,NULL);
    if(!tap){EZReply(@{@"error":@"La reprise humaine ne peut pas être surveillée. Vérifiez Accessibilité et Surveillance de l’entrée dans Réglages Système."});return 1;}
    CFRunLoopSourceRef source=CFMachPortCreateRunLoopSource(NULL,tap,0); CFRunLoopAddSource(CFRunLoopGetMain(),source,kCFRunLoopCommonModes);
#endif
    NSRect screen=NSScreen.mainScreen.frame;
    ezOverlay=[[NSPanel alloc] initWithContentRect:NSMakeRect(NSMidX(screen)-240,NSMaxY(screen)-80,480,44) styleMask:NSWindowStyleMaskBorderless backing:NSBackingStoreBuffered defer:NO];
    ezOverlay.level=NSScreenSaverWindowLevel; ezOverlay.hidesOnDeactivate=NO; ezOverlay.ignoresMouseEvents=YES;
    ezOverlay.collectionBehavior=NSWindowCollectionBehaviorCanJoinAllSpaces|NSWindowCollectionBehaviorFullScreenAuxiliary|NSWindowCollectionBehaviorStationary;
    ezOverlay.backgroundColor=[NSColor colorWithCalibratedRed:0.12 green:0.13 blue:0.16 alpha:0.98];
    ezOverlayLabel=[NSTextField labelWithString:@"Ezerbot contrôle l’écran — Échap pour arrêter"];
    ezOverlayLabel.textColor=NSColor.whiteColor; ezOverlayLabel.font=[NSFont systemFontOfSize:14 weight:NSFontWeightSemibold]; ezOverlayLabel.alignment=NSTextAlignmentCenter; ezOverlayLabel.frame=NSMakeRect(8,12,464,22); [ezOverlay.contentView addSubview:ezOverlayLabel]; EZOverlayUpdate();
    [NSTimer scheduledTimerWithTimeInterval:0.05 repeats:YES block:^(NSTimer *timer){if(NSProcessInfo.processInfo.systemUptime>=ezDeadline){[timer invalidate];EZStop(NO);}}];
    EZReply(@{@"ready":@YES});
    dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INTERACTIVE,0),^{
        FILE *input=stdin; char *line=NULL; size_t capacity=0;
        while(getline(&line,&capacity,input)>0) {
            @autoreleasepool {
                if(strlen(line)>32768) {EZStop(NO);break;}
                NSData *data=[[NSString stringWithUTF8String:line] dataUsingEncoding:NSUTF8StringEncoding];
                NSDictionary *request=[NSJSONSerialization JSONObjectWithData:data options:0 error:nil];
                if(![request isKindOfClass:NSDictionary.class]) {EZStop(NO);break;}
                NSString *command=request[@"command"];
                if([command isEqualToString:@"revoke"]) {EZStop(NO);break;}
                if([command isEqualToString:@"resume"]) {
                    pthread_mutex_lock(&ezPermit); if(!ezRevoked && ezPaused){ezPaused=NO;++ezGeneration;} pthread_mutex_unlock(&ezPermit);
                    EZReply(@{@"event":@"resumed"}); dispatch_async(dispatch_get_main_queue(),^{EZOverlayUpdate();});
                } else if([command isEqualToString:@"action"]) {
                    pthread_mutex_lock(&ezPermit); uint64_t generation=ezGeneration; pthread_mutex_unlock(&ezPermit);
                    dispatch_async(dispatch_get_main_queue(),^{EZAction(request,generation);});
                }
            }
        }
        free(line); EZStop(NO);
    });
    [app run];
    return 0;
}
