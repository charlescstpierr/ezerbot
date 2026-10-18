// clang -fobjc-arc -Wall -Wextra -Werror -framework Cocoa -framework UserNotifications \
//   app/native/notifications-test.m -o /tmp/ezerbot-notifications-test
// No NSApplication or UserNotifications service is opened by this test.
#if defined(EZERBOT_RELEASE)
#error Notification tests must never be built with the real adapter
#endif
#import "notifications.h"

int main(void) {
    @autoreleasepool {
        EzerbotNotifier *notifier = [[EzerbotNotifier alloc] init];
        notifier.fakeMode = @"recording";
        __block NSString *status;
        [notifier authorize:^(NSDictionary *result) { status = result[@"status"]; }];
        NSCAssert([status isEqualToString:@"recording"], @"fake authorization");
        NSDictionary *event = @{@"id":@"reply:test", @"kind":@"reply", @"title":@"Réponse terminée",
            @"conversation_id":@"test", @"card_id":@"reply-test"};
        [notifier deliver:event reply:^(NSDictionary *result) { status = result[@"status"]; }];
        NSCAssert(notifier.recordings.count == 2, @"authorization and one recorded delivery");
        NSCAssert([notifier.recordings.lastObject[@"event"] isEqual:event], @"exact destination retained");
        for (NSString *mode in @[@"denied", @"error", @"disabled"]) {
            notifier.fakeMode = mode;
            [notifier authorize:^(NSDictionary *result) { status = result[@"status"]; }];
            NSCAssert([status isEqual:mode], @"denial/error status");
            NSUInteger count = notifier.recordings.count;
            [notifier deliver:event reply:^(NSDictionary *result) { status = result[@"status"]; }];
            NSCAssert([status isEqual:mode] && notifier.recordings.count == count, @"denial cannot post");
        }
        puts("Notifications natives : faux enregistrement, refus, erreur et désactivation validés.");
    }
    return 0;
}
