#import <Cocoa/Cocoa.h>
#import <UserNotifications/UserNotifications.h>

// The only class allowed to touch UserNotifications. Without the explicit
// release compiler flag it CANNOT request permission or post a system alert.
@interface EzerbotNotifier : NSObject
@property(nonatomic, strong) NSMutableArray<NSDictionary *> *recordings;
@property(nonatomic, copy) NSString *fakeMode;
- (void)authorize:(void (^)(NSDictionary *))reply;
- (void)status:(void (^)(NSDictionary *))reply;
- (void)deliver:(NSDictionary *)event reply:(void (^)(NSDictionary *))reply;
@end

@implementation EzerbotNotifier
- (instancetype)init {
    self = [super init];
    if (self) {
        _recordings = [NSMutableArray array];
#if !defined(EZERBOT_RELEASE)
        _fakeMode = NSProcessInfo.processInfo.environment[@"GROKBOT_NOTIFICATION_FAKE"] ?: @"disabled";
#endif
    }
    return self;
}
- (NSDictionary *)fakeResult {
    NSString *status = [@[@"recording", @"denied", @"error"] containsObject:self.fakeMode] ? self.fakeMode : @"disabled";
    return @{@"status":status};
}
- (void)status:(void (^)(NSDictionary *))reply {
#if defined(EZERBOT_RELEASE)
    [[UNUserNotificationCenter currentNotificationCenter] getNotificationSettingsWithCompletionHandler:^(UNNotificationSettings *settings) {
        NSString *status = @"unknown";
        switch (settings.authorizationStatus) {
            case UNAuthorizationStatusNotDetermined: status = @"undetermined"; break;
            case UNAuthorizationStatusDenied: status = @"denied"; break;
            case UNAuthorizationStatusAuthorized: case UNAuthorizationStatusProvisional: status = @"granted"; break;
            default: break;
        }
        if ([status isEqualToString:@"granted"] && settings.alertSetting == UNNotificationSettingDisabled) status = @"denied";
        reply(@{@"status":status});
    }];
#else
    reply([self fakeResult]);
#endif
}
- (void)authorize:(void (^)(NSDictionary *))reply {
#if defined(EZERBOT_RELEASE)
    [self status:^(NSDictionary *result) {
        if (![result[@"status"] isEqualToString:@"undetermined"]) { reply(result); return; }
        // Called ONLY by the explicit Notifications switch, never startup/delivery.
        [[UNUserNotificationCenter currentNotificationCenter] requestAuthorizationWithOptions:UNAuthorizationOptionAlert
            completionHandler:^(BOOL granted, NSError *error) {
                reply(@{@"status":error ? @"error" : (granted ? @"granted" : @"denied")});
            }];
    }];
#else
    [self.recordings addObject:@{@"operation":@"authorize"}];
    reply([self fakeResult]);
#endif
}
- (void)deliver:(NSDictionary *)event reply:(void (^)(NSDictionary *))reply {
#if defined(EZERBOT_RELEASE)
    [self status:^(NSDictionary *result) {
        if (![result[@"status"] isEqualToString:@"granted"]) { reply(result); return; }
        UNMutableNotificationContent *content = [[UNMutableNotificationContent alloc] init];
        content.title = event[@"title"];
        content.body = @"Ouvrez Ezerbot pour consulter cet événement.";
        content.userInfo = @{@"event_id":event[@"id"], @"conversation_id":event[@"conversation_id"]};
        content.threadIdentifier = event[@"conversation_id"];
        UNNotificationRequest *request = [UNNotificationRequest requestWithIdentifier:event[@"id"] content:content trigger:nil];
        [[UNUserNotificationCenter currentNotificationCenter] addNotificationRequest:request withCompletionHandler:^(NSError *error) {
            reply(@{@"status":error ? @"error" : @"delivered"});
        }];
    }];
#else
    if ([self.fakeMode isEqualToString:@"recording"]) {
        [self.recordings addObject:@{@"operation":@"deliver", @"event":event}];
    }
    reply([self fakeResult]);
#endif
}
@end
