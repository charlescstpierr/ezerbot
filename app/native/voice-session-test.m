// Recording-only debug synthesizer. Never run this test with EZERBOT_RELEASE.
#import "voice.h"
#import "voice-session.h"
#include <assert.h>
#if defined(EZERBOT_RELEASE)
#error Voice tests must not request permissions or play audio
#endif
int main(void) {
    @autoreleasepool {
        EzerbotVoiceSession *voice = [[EzerbotVoiceSession alloc] init];
        __block NSDictionary *result;
        void (^reply)(NSDictionary *) = ^(NSDictionary *value) { result = value; };
        [voice command:@{@"command":@"options",@"token":@""} reply:reply];
        assert([result[@"voices"] count] == 1);
        [voice command:@{@"command":@"start",@"token":@"one"} reply:reply];
        assert([result[@"status"] isEqual:@"listening"]);
        [voice command:@{@"command":@"speak",@"token":@"one",@"text":@"Bonjour.",@"voice":@"fake-fr-CA",@"speed":@1,@"barge":@0} reply:reply];
        assert(voice.spoken.count == 1 && [result[@"speaking"] boolValue]);
        [voice command:@{@"command":@"pause",@"token":@"one"} reply:reply];
        assert([result[@"paused"] boolValue] && ![result[@"speaking"] boolValue]);
        [voice command:@{@"command":@"resume",@"token":@"one"} reply:reply];
        assert(![result[@"paused"] boolValue] && [result[@"speaking"] boolValue]);
        [voice command:@{@"command":@"feed",@"token":@"one",@"text":@"Attends",@"final_result":@YES} reply:reply];
        assert(![result[@"speaking"] boolValue] && [result[@"utterance_id"] unsignedIntegerValue] == 1);
        [voice command:@{@"command":@"speak",@"token":@"one",@"text":@"Trop tard.",@"voice":@"fake-fr-CA",@"speed":@1,@"barge":@0} reply:reply];
        assert(voice.spoken.count == 1); // Stale streaming text cannot restart speech after barge-in.
        [voice command:@{@"command":@"cancel",@"token":@"one"} reply:reply];
        assert([result[@"status"] isEqual:@"idle"] && ![result[@"speaking"] boolValue]);
        assert([result[@"transcript"] length] == 0);
        [voice command:@{@"command":@"status",@"token":@"old"} reply:reply];
        assert([result[@"status"] isEqual:@"stale"]);
        puts("voice session native debug: French options, recording synthesis, live transcript, immediate barge-in, stale chunk, cleanup OK; no permissions or audio");
    }
}
