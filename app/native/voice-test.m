// Debug-only contract test: feeds synthetic WAV bytes, never opens an audio device.
#import "voice.h"
#include <assert.h>
#if defined(EZERBOT_RELEASE)
#error Voice tests must never request real permissions
#endif
int main(void) {
    @autoreleasepool {
        EzerbotVoice *voice = [[EzerbotVoice alloc] init];
        __block NSDictionary *result;
        void (^reply)(NSDictionary *) = ^(NSDictionary *value) { result = value; };
        [voice command:@{@"command":@"start",@"token":@"one",@"language":@"fr"} reply:reply];
        assert([result[@"status"] isEqual:@"recording"]);
        [NSRunLoop.currentRunLoop runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.2]];
        [voice command:@{@"command":@"status",@"token":@"one"} reply:reply];
        assert([result[@"level"] doubleValue] > 0);
        [voice command:@{@"command":@"stop",@"token":@"stale"} reply:reply];
        assert([result[@"status"] isEqual:@"stale"]);
        [voice command:@{@"command":@"stop",@"token":@"one"} reply:reply];
        assert([result[@"status"] isEqual:@"ready"]);
        assert([result[@"transcript"] hasPrefix:@"Bonjour"]);
        assert([result[@"data"] length] > 44);
        [voice command:@{@"command":@"cancel",@"token":@"one"} reply:reply];
        assert([result[@"status"] isEqual:@"idle"]);
        assert([result[@"data"] length] == 0);
        [voice command:@{@"command":@"start",@"token":@"two",@"language":@"fr"} reply:reply];
        voice.started = [NSDate dateWithTimeIntervalSinceNow:-61];
        [NSRunLoop.currentRunLoop runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.2]];
        [voice command:@{@"command":@"status",@"token":@"two"} reply:reply];
        assert([result[@"status"] isEqual:@"ready"] && [result[@"too_long"] boolValue]);
        [voice cancel];
        [voice command:@{@"command":@"start",@"token":@"meeting",@"purpose":@"meeting"} reply:reply];
        assert([result[@"status"] isEqual:@"consent_required"]);
        [voice command:@{@"command":@"start",@"token":@"meeting",@"purpose":@"meeting",@"consent":@YES} reply:reply];
        assert([result[@"status"] isEqual:@"recording"] && voice.maxSeconds == 600);
        [voice command:@{@"command":@"start",@"token":@"dictation"} reply:reply];
        assert([result[@"status"] isEqual:@"meeting_busy"]);
        [voice command:@{@"command":@"pause",@"token":@"meeting"} reply:reply];
        assert([result[@"status"] isEqual:@"paused"]);
        [voice command:@{@"command":@"resume",@"token":@"meeting"} reply:reply];
        assert([result[@"status"] isEqual:@"recording"]);
        voice.started = [NSDate dateWithTimeIntervalSinceNow:-601];
        [NSRunLoop.currentRunLoop runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.2]];
        [voice command:@{@"command":@"status",@"token":@"meeting"} reply:reply];
        assert([result[@"status"] isEqual:@"ready"] && [result[@"too_long"] boolValue]);
        [voice cancel];
        puts("voice/meeting native debug: fixture, consent, pause/resume, level, stop, stale, cancel, caps OK; no permissions");
    }
    return 0;
}
