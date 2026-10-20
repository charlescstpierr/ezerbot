// Teach is a separate listen-only mode. Shares the CUA permission/identity and
// exclusive desktop lease, but can NEVER post input. Debug has no native capture
// or input observer at all (the Rust debug backend uses the drawn fixture).
static NSMutableArray *ezDemoInputs;
static BOOL ezDemoStopped;
static double ezDemoStarted;
#if defined(EZERBOT_RELEASE)
static BOOL EZDemoPrivateFocus(void) {
    AXUIElementRef system=AXUIElementCreateSystemWide(); CFTypeRef element=NULL,role=NULL,subrole=NULL;
    AXUIElementSetMessagingTimeout(system,0.05f);
    BOOL private=YES;
    if(AXUIElementCopyAttributeValue(system,kAXFocusedUIElementAttribute,&element)==kAXErrorSuccess && element && CFGetTypeID(element)==AXUIElementGetTypeID()) {
        AXUIElementSetMessagingTimeout((AXUIElementRef)element,0.05f);
        AXError rr=AXUIElementCopyAttributeValue((AXUIElementRef)element,kAXRoleAttribute,&role);
        AXError sr=AXUIElementCopyAttributeValue((AXUIElementRef)element,kAXSubroleAttribute,&subrole);
        BOOL known=sr==kAXErrorSuccess || sr==kAXErrorAttributeUnsupported || sr==kAXErrorNoValue;
        private=rr!=kAXErrorSuccess || !role || !known || (subrole && CFEqual(subrole,kAXSecureTextFieldSubrole));
    }
    if(role)CFRelease(role);if(subrole)CFRelease(subrole);if(element)CFRelease(element);CFRelease(system);
    return private;
}
#endif
static void EZDemoSample(BOOL stopped, NSString *error) {
    NSMutableDictionary *sample=[NSMutableDictionary dictionary];
#if defined(EZERBOT_RELEASE)
    // Unknown focus is private, too. Never read keyboard Unicode or AX values.
    // No snapshots while a password/unknown field is focused.
    if (!error && !EZDemoPrivateFocus()) [sample addEntriesFromDictionary:EZCapture()];
#else
    if (!error) [sample addEntriesFromDictionary:EZCapture()];
#endif
    sample[@"inputs"]=[ezDemoInputs copy] ?: @[]; [ezDemoInputs removeAllObjects];
    sample[@"elapsed_ms"]=@((NSProcessInfo.processInfo.systemUptime-ezDemoStarted)*1000);
    if (error) sample[@"error"]=error;
    sample[@"stopped"]=@(stopped);
    EZReply(sample);
}
static void EZDemoStop(NSString *error) {
    if(ezDemoStopped)return;ezDemoStopped=YES;
    EZDemoSample(YES,error);[ezOverlay orderOut:nil];[NSApp terminate:nil];
}
@interface EZDemoControls : NSObject
- (void)stop:(id)sender;
@end
@implementation EZDemoControls
- (void)stop:(id)sender { (void)sender; EZDemoStop(nil); }
@end
#if defined(EZERBOT_RELEASE)
static void EZDemoAppend(NSDictionary *event) {
    if (ezDemoStopped || ezDemoInputs.count>=200) return;
    if (![ezDemoInputs.lastObject isEqual:event]) [ezDemoInputs addObject:event];
}
static CGEventRef EZDemoInput(CGEventTapProxy proxy, CGEventType type, CGEventRef event, void *context) {
    (void)proxy;(void)context;
    if (ezDemoStopped) return event;
    if(type==kCGEventTapDisabledByTimeout || type==kCGEventTapDisabledByUserInput) {EZDemoStop(@"La surveillance des entrées a été interrompue. La démonstration est arrêtée.");return event;}
    if (type==kCGEventKeyDown) {
        // Only navigation labels are retained outside private focus. Printable
        // keycodes, modifiers, Unicode and counts are NEVER retained, anywhere.
        NSString *key=nil;
        if (!EZDemoPrivateFocus()) {
            switch(CGEventGetIntegerValueField(event,kCGKeyboardEventKeycode)) {
                case 36: key=@"Entrée";break;case 48:key=@"Tabulation";break;case 53:key=@"Échap";break;
                case 123:case 124:case 125:case 126:key=@"Flèche";break;default:break;
            }
        }
        EZDemoAppend(key ? @{@"kind":@"key",@"key":key} : @{@"kind":@"typing"});
    } else if(type==kCGEventLeftMouseDown || type==kCGEventRightMouseDown || type==kCGEventOtherMouseDown) {
        CGPoint p=CGEventGetLocation(event);
        EZDemoAppend(@{@"kind":@"click",@"x":@((NSInteger)p.x),@"y":@((NSInteger)p.y),@"button":@(CGEventGetIntegerValueField(event,kCGMouseEventButtonNumber))});
    }
    return event;
}
#endif
static int EZDemoMain(int argc,const char *argv[]) {
    if(argc!=3)return 64;
    long duration=strtol(argv[2],NULL,10);if(duration<100||duration>600000)return 64;
#if defined(EZERBOT_RELEASE)
    if(!EZParent())return 77;
    NSString *permission=EZPermissions();if(permission){EZReply(@{@"error":permission});return 1;}
    NSString *lockPath=[NSTemporaryDirectory() stringByAppendingPathComponent:[NSString stringWithFormat:@"ezerbot-screen-%u.lock",getuid()]];
    int lock=open(lockPath.fileSystemRepresentation,O_CREAT|O_RDWR|O_NOFOLLOW,0600);
    if(lock<0||flock(lock,LOCK_EX|LOCK_NB)!=0){EZReply(@{@"error":@"Une autre session utilise déjà l’écran de ce Mac."});return 1;}
#endif
    NSApplication *app=[NSApplication sharedApplication];[app setActivationPolicy:NSApplicationActivationPolicyAccessory];
    ezDemoInputs=[NSMutableArray array];ezDemoStarted=NSProcessInfo.processInfo.systemUptime;ezDemoStopped=NO;
#if defined(EZERBOT_RELEASE)
    CGEventMask mask=CGEventMaskBit(kCGEventKeyDown)|CGEventMaskBit(kCGEventLeftMouseDown)|CGEventMaskBit(kCGEventRightMouseDown)|CGEventMaskBit(kCGEventOtherMouseDown);
    CFMachPortRef tap=CGEventTapCreate(kCGAnnotatedSessionEventTap,kCGHeadInsertEventTap,kCGEventTapOptionListenOnly,mask,EZDemoInput,NULL);
    if(!tap){EZReply(@{@"error":@"Impossible d’observer la démonstration. Vérifiez Accessibilité et Surveillance de l’entrée."});return 1;}
    CFRunLoopSourceRef source=CFMachPortCreateRunLoopSource(NULL,tap,0);CFRunLoopAddSource(CFRunLoopGetMain(),source,kCFRunLoopCommonModes);
    [NSWorkspace.sharedWorkspace.notificationCenter addObserverForName:NSWorkspaceDidActivateApplicationNotification object:nil queue:NSOperationQueue.mainQueue usingBlock:^(NSNotification *note){
        NSRunningApplication *application=note.userInfo[NSWorkspaceApplicationKey];
        EZDemoAppend(@{@"kind":@"app",@"name":application.localizedName ?: @"Application"});
    }];
    NSRunningApplication *front=NSWorkspace.sharedWorkspace.frontmostApplication;
    EZDemoAppend(@{@"kind":@"app",@"name":front.localizedName ?: @"Application"});
#endif
    NSRect screen=NSScreen.mainScreen.frame;
    ezOverlay=[[NSPanel alloc] initWithContentRect:NSMakeRect(NSMidX(screen)-290,NSMaxY(screen)-80,580,48) styleMask:NSWindowStyleMaskBorderless backing:NSBackingStoreBuffered defer:NO];
    ezOverlay.level=NSScreenSaverWindowLevel;ezOverlay.hidesOnDeactivate=NO;ezOverlay.sharingType=NSWindowSharingNone;
    ezOverlay.collectionBehavior=NSWindowCollectionBehaviorCanJoinAllSpaces|NSWindowCollectionBehaviorFullScreenAuxiliary|NSWindowCollectionBehaviorStationary;
    ezOverlay.backgroundColor=[NSColor colorWithCalibratedRed:0.12 green:0.13 blue:0.16 alpha:0.98];
    NSTextField *label=[NSTextField labelWithString:@"Enregistrement de la démonstration — 10 min max."];
    label.textColor=NSColor.whiteColor;label.font=[NSFont systemFontOfSize:14 weight:NSFontWeightSemibold];label.frame=NSMakeRect(12,13,460,22);[ezOverlay.contentView addSubview:label];
    EZDemoControls *controls=[EZDemoControls new];NSButton *stop=[NSButton buttonWithTitle:@"Arrêter" target:controls action:@selector(stop:)];stop.frame=NSMakeRect(482,10,86,28);[ezOverlay.contentView addSubview:stop];[ezOverlay orderFrontRegardless];
    EZReply(@{@"ready":@YES});EZDemoSample(NO,nil);
    [NSTimer scheduledTimerWithTimeInterval:5 repeats:YES block:^(NSTimer *timer){if(ezDemoStopped){[timer invalidate];return;}EZDemoSample(NO,nil);}];
    [NSTimer scheduledTimerWithTimeInterval:0.1 repeats:YES block:^(NSTimer *timer){
        if(NSProcessInfo.processInfo.systemUptime-ezDemoStarted>=(double)duration/1000 || getppid()<=1){[timer invalidate];EZDemoStop(nil);}
    }];
    dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INTERACTIVE,0),^{
        // Any input or EOF ends recording. No arbitrary command/input interface.
        char line[128]; (void)fgets(line,sizeof(line),stdin);
        dispatch_async(dispatch_get_main_queue(),^{EZDemoStop(nil);});
    });
    [app run];(void)controls;return 0;
}
