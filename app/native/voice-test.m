// Debug-only contract test: feeds synthetic WAV bytes, never opens an audio device.
#import "voice.h"
#include <assert.h>
#if defined(EZERBOT_RELEASE)
#error Voice tests must never request real permissions
#endif
int main(void) {
    @autoreleasepool {
        EzerbotVoice *voice = [[EzerbotVoice alloc] init];
        for (NSString *status in @[@"authorizing",@"recording",@"paused",@"transcribing",@"listening"]) assert([EzerbotVoice captureStatusActive:status]);
        for (NSString *status in @[@"idle",@"ready",@"failed",@"mic_denied"]) assert(![EzerbotVoice captureStatusActive:status]);
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
        // The production PCM splitter/progress ledger is compiled in debug too.
        NSData *longAudio = [EzerbotMeetingTranscription waveFromPCM:[NSMutableData dataWithLength:121*32000]];
        EzerbotMeetingTranscription *plan = [[EzerbotMeetingTranscription alloc] initWithAudio:longAudio];
        assert(plan.count == 3 && [plan nextAudio].length == 44+60*32000);
        [plan completeWithText:@"Décision A"];
        assert([plan nextAudio].length == 44+60*32000); [plan completeWithText:nil];
        assert([plan nextAudio].length == 44+32000); [plan completeWithText:@"Action C"];
        assert(![plan nextAudio] && plan.hasText && plan.missingCount == 1);
        assert([plan.transcript containsString:@"Décision A"] && [plan.transcript containsString:@"Action C"]);
        [plan retryMissing]; assert([plan nextAudio].length == 44+60*32000 && plan.currentIndex == 1);
        [plan completeWithText:@"Passage B récupéré"]; assert(![plan nextAudio] && plan.missingCount == 0);
        assert([plan.transcript isEqual:@"Décision A\nPassage B récupéré\nAction C"]);
        assert(![[EzerbotMeetingTranscription alloc] initWithAudio:[@"RIFF invalide" dataUsingEncoding:NSUTF8StringEncoding]]);
        NSMutableData *invalid = [[EzerbotMeetingTranscription waveFromPCM:[NSMutableData dataWithLength:32000]] mutableCopy];
        ((uint8_t *)invalid.mutableBytes)[22] = 2;
        assert(![[EzerbotMeetingTranscription alloc] initWithAudio:invalid]);
        // A deadline in the middle is nonfatal; retry visits ONLY that segment.
        EzerbotVoice *segmented = [[EzerbotVoice alloc] init]; segmented.meeting = YES; segmented.token = @"segments"; segmented.audio = longAudio;
        segmented.fakeSegmentTexts = @[@"Décision A",@"Passage B récupéré",@"Action C"];
        segmented.fakeSegmentTimeouts = [NSIndexSet indexSetWithIndex:1]; [segmented transcribe];
        assert([segmented.status isEqual:@"ready"] && segmented.transcriptionPlan.missingCount == 1);
        assert([segmented.transcript containsString:@"Décision A"] && [segmented.transcript containsString:@"Action C"]);
        assert([segmented.transcript containsString:@"[Passage non transcrit de 1:00 à 2:00]"]);
        segmented.fakeSegmentTimeouts = [NSIndexSet indexSet];
        [segmented command:@{@"command":@"retry",@"token":@"segments"} reply:reply];
        assert([result[@"status"] isEqual:@"ready"] && ![result[@"partial"] boolValue]);
        assert(([segmented.fakeRecognizedSegments isEqualToArray:@[@0,@1,@2,@1]]));
        assert([segmented.transcript isEqual:@"Décision A\nPassage B récupéré\nAction C"]);
        // A silent/failed one-second tail must not discard the first minute.
        EzerbotVoice *tail = [[EzerbotVoice alloc] init]; tail.meeting = YES; tail.token = @"tail";
        tail.audio = [EzerbotMeetingTranscription waveFromPCM:[NSMutableData dataWithLength:61*32000]];
        tail.fakeSegmentTexts = @[@"Première minute conservée",@""]; tail.fakeSegmentFailures = [NSIndexSet indexSetWithIndex:1];
        [tail transcribe]; assert([tail.status isEqual:@"ready"] && tail.transcriptionPlan.missingCount == 1);
        [tail command:@{@"command":@"retry",@"token":@"tail"} reply:reply];
        assert([result[@"partial"] boolValue] && [tail.transcript containsString:@"Première minute conservée"]);
        assert(CFGetTypeID((__bridge CFTypeRef)result[@"partial"]) == CFBooleanGetTypeID());
        assert(([tail.fakeRecognizedSegments isEqualToArray:@[@0,@1,@1]]));
        assert([tail.transcript containsString:@"1:00 à 1:01"]);
        // Fail only when no real text exists; a later retry can recover a part.
        EzerbotVoice *empty = [[EzerbotVoice alloc] init]; empty.meeting = YES; empty.token = @"empty";
        empty.audio = [EzerbotMeetingTranscription waveFromPCM:[NSMutableData dataWithLength:120*32000]];
        empty.fakeSegmentFailures = [NSIndexSet indexSetWithIndexesInRange:NSMakeRange(0,2)]; [empty transcribe];
        assert([empty.status isEqual:@"failed"] && empty.transcript.length == 0);
        empty.fakeSegmentFailures = [NSIndexSet indexSetWithIndex:1];
        [empty command:@{@"command":@"retry",@"token":@"empty"} reply:reply];
        assert([result[@"status"] isEqual:@"ready"] && [result[@"partial"] boolValue]);
        [segmented cancel]; [tail cancel]; [empty cancel];
        assert(!segmented.audio && !segmented.transcriptionPlan);
        puts("voice/meeting native debug: PCM chunks, partial failure, deadline, silent tail, targeted retry, consent, pause/resume, caps OK; no permissions");
    }
    return 0;
}
