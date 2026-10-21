// Microphone and Speech are deliberately unreachable outside EZERBOT_RELEASE.
// The WebView bridge accepts main-frame, loopback-origin commands only (main.m).
#import <Cocoa/Cocoa.h>
#import "meeting-transcription.h"
#if defined(EZERBOT_RELEASE)
#import <AVFoundation/AVFoundation.h>
#import <Speech/Speech.h>
#endif

@interface EzerbotVoice : NSObject
@property(nonatomic, copy) NSString *token;
@property(nonatomic, copy) NSString *status;
@property(nonatomic, copy) NSString *language;
@property(nonatomic, copy) NSString *transcript;
@property(nonatomic, strong) NSData *audio;
@property(nonatomic, strong) NSURL *recordingURL;
@property(nonatomic, strong) NSDate *started;
@property(nonatomic, strong) NSTimer *timer;
@property(nonatomic) NSUInteger generation;
@property(nonatomic) BOOL tooLong;
@property(nonatomic) double elapsed;
@property(nonatomic) double level;
@property(nonatomic) BOOL meeting;
@property(nonatomic) NSUInteger maxSeconds;
@property(nonatomic) double pausedElapsed;
@property(nonatomic, strong) EzerbotMeetingTranscription *transcriptionPlan;
@property(nonatomic, strong) NSURL *segmentURL;
#if defined(EZERBOT_RELEASE)
@property(nonatomic, strong) AVAudioRecorder *recorder;
@property(nonatomic, strong) SFSpeechRecognizer *recognizer;
@property(nonatomic, strong) SFSpeechRecognitionTask *recognitionTask;
#else
@property(nonatomic, copy) NSArray<NSString *> *fakeSegmentTexts;
@property(nonatomic, copy) NSIndexSet *fakeSegmentFailures;
@property(nonatomic, copy) NSIndexSet *fakeSegmentTimeouts;
@property(nonatomic, strong) NSMutableArray<NSNumber *> *fakeRecognizedSegments;
#endif
+ (BOOL)captureStatusActive:(NSString *)status;
- (void)command:(NSDictionary *)body reply:(void (^)(NSDictionary *))reply;
- (void)cancel;
- (void)transcribe;
@end

@implementation EzerbotVoice
+ (BOOL)captureStatusActive:(NSString *)status {
    return [@[@"authorizing",@"recording",@"paused",@"transcribing",@"listening"] containsObject:status ?: @""];
}
- (NSDictionary *)snapshot {
    return @{ @"status":self.status ?: @"idle", @"token":self.token ?: @"",
        @"language":self.language ?: @"fr", @"transcript":self.transcript ?: @"",
        @"elapsed_ms":@(self.elapsed * 1000), @"level":@(self.level), @"too_long":@(self.tooLong),
        @"missing_segments":@(self.transcriptionPlan.missingCount),
        @"partial":@((BOOL)(self.transcriptionPlan.hasText && self.transcriptionPlan.missingCount > 0)),
        @"data":[self.status isEqual:@"ready"] ? ([self.audio base64EncodedStringWithOptions:0] ?: @"") : @"" };
}
- (void)removeTemporaryAudio {
    if (self.recordingURL) [[NSFileManager defaultManager] removeItemAtURL:self.recordingURL error:nil];
    if (self.segmentURL) [[NSFileManager defaultManager] removeItemAtURL:self.segmentURL error:nil];
    self.recordingURL = nil; self.segmentURL = nil;
}
- (void)cancel {
    ++self.generation;
    [self.timer invalidate]; self.timer = nil;
#if defined(EZERBOT_RELEASE)
    [self.recorder stop]; self.recorder = nil;
    [self.recognitionTask cancel]; self.recognitionTask = nil;
    self.recognizer = nil;
#endif
    [self removeTemporaryAudio];
    self.audio = nil; self.transcriptionPlan = nil; self.transcript = nil; self.status = @"idle"; self.level = 0;
}
- (void)transcribe {
    if (!self.audio.length) { self.status = @"failed"; return; }
    self.status = @"transcribing"; self.transcript = @"";
    if (self.meeting) {
        self.transcriptionPlan = [[EzerbotMeetingTranscription alloc] initWithAudio:self.audio];
        if (!self.transcriptionPlan) { self.status = @"failed"; return; }
#if !defined(EZERBOT_RELEASE)
        self.fakeRecognizedSegments = [NSMutableArray array];
#endif
    }
    [self transcribeSegment];
}
- (void)completeMeetingSegment:(NSString *)text {
    // Both Speech errors and deadlines use this path. Previous successful text
    // survives, the missing passage is marked, and the next segment is attempted.
    ++self.generation;
#if defined(EZERBOT_RELEASE)
    [self.recognitionTask cancel]; self.recognitionTask = nil;
#endif
    [self.transcriptionPlan completeWithText:text];
    self.transcript = self.transcriptionPlan.transcript;
    [self transcribeSegment];
}
- (void)segmentTimedOut {
    if (![self.status isEqual:@"transcribing"]) return;
    if (self.meeting) { [self completeMeetingSegment:nil]; return; }
    ++self.generation;
#if defined(EZERBOT_RELEASE)
    [self.recognitionTask cancel]; self.recognitionTask = nil;
#endif
    self.status = @"failed";
}
- (void)transcribeSegment {
    NSData *segment = nil;
    if (self.meeting) {
        segment = [self.transcriptionPlan nextAudio];
        if (!segment) {
            self.transcript = self.transcriptionPlan.transcript;
            self.status = self.transcriptionPlan.hasText ? @"ready" : @"failed";
            [self removeTemporaryAudio]; // Retry uses the in-memory progress ledger.
            return;
        }
    }
    NSUInteger generation = self.generation;
#if defined(EZERBOT_RELEASE)
    // Strictly on-device, never fall back to a server.
    if (!self.recognizer.supportsOnDeviceRecognition) {
        if (self.meeting) { [self completeMeetingSegment:nil]; return; }
        self.status = @"offline_unavailable"; return;
    }
    NSURL *source = self.recordingURL;
    if (self.meeting) {
        if (self.segmentURL) [[NSFileManager defaultManager] removeItemAtURL:self.segmentURL error:nil];
        self.segmentURL = [NSURL fileURLWithPath:[NSTemporaryDirectory() stringByAppendingPathComponent:[NSString stringWithFormat:@"ezerbot-meeting-%@.wav",NSUUID.UUID.UUIDString]]];
        if (![segment writeToURL:self.segmentURL options:NSDataWritingAtomic error:nil]) { [self completeMeetingSegment:nil]; return; }
        [[NSFileManager defaultManager] setAttributes:@{NSFilePosixPermissions:@0600} ofItemAtPath:self.segmentURL.path error:nil];
        source = self.segmentURL;
    }
    SFSpeechURLRecognitionRequest *request = [[SFSpeechURLRecognitionRequest alloc] initWithURL:source];
    request.requiresOnDeviceRecognition = YES; request.shouldReportPartialResults = NO;
    __weak EzerbotVoice *weakSelf = self;
    self.recognitionTask = [self.recognizer recognitionTaskWithRequest:request resultHandler:^(SFSpeechRecognitionResult *result, NSError *error) {
        dispatch_async(dispatch_get_main_queue(), ^{
            EzerbotVoice *owner = weakSelf;
            if (!owner || owner.generation != generation || ![owner.status isEqual:@"transcribing"]) return;
            if (result.isFinal) {
                NSString *text = [result.bestTranscription.formattedString stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet];
                if (owner.meeting) [owner completeMeetingSegment:text];
                else { owner.transcript = text; owner.status = text.length ? @"ready" : @"failed"; }
            } else if (error) {
                if (owner.meeting) [owner completeMeetingSegment:nil]; else owner.status = @"failed";
            }
        });
    }];
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW, 60 * NSEC_PER_SEC), dispatch_get_main_queue(), ^{
        EzerbotVoice *owner = weakSelf;
        if (owner && owner.generation == generation) [owner segmentTimedOut];
    });
#else
    (void)generation;
    if (self.meeting) {
        NSUInteger index = self.transcriptionPlan.currentIndex;
        [self.fakeRecognizedSegments addObject:@(index)];
        if ([self.fakeSegmentTimeouts containsIndex:index]) [self segmentTimedOut];
        else if ([self.fakeSegmentFailures containsIndex:index]) [self completeMeetingSegment:nil];
        else [self completeMeetingSegment:index < self.fakeSegmentTexts.count ? self.fakeSegmentTexts[index] : @"Bonjour, voici mon mémo vocal pour préparer la réunion de demain."];
    } else {
        self.transcript = @"Bonjour, voici mon mémo vocal pour préparer la réunion de demain."; self.status = @"ready";
    }
#endif
}
- (void)stop {
    if (![@[@"recording",@"paused"] containsObject:self.status]) return;
    [self.timer invalidate]; self.timer = nil; self.level = 0;
#if defined(EZERBOT_RELEASE)
    self.elapsed = MIN(self.maxSeconds, MAX(self.elapsed, self.recorder.currentTime));
    [self.recorder stop]; self.recorder = nil;
    self.audio = [NSData dataWithContentsOfURL:self.recordingURL];
#endif
    [self transcribe];
}
- (void)beginRecording:(NSUInteger)generation {
    if (generation != self.generation) return;
#if defined(EZERBOT_RELEASE)
    if (![AVCaptureDevice defaultDeviceWithMediaType:AVMediaTypeAudio]) { self.status = @"no_microphone"; return; }
    self.recordingURL = [NSURL fileURLWithPath:[NSTemporaryDirectory() stringByAppendingPathComponent:[NSString stringWithFormat:@"ezerbot-voice-%@.wav", NSUUID.UUID.UUIDString]]];
    NSDictionary *settings = @{AVFormatIDKey:@(kAudioFormatLinearPCM),AVSampleRateKey:@16000,AVNumberOfChannelsKey:@1,
        AVLinearPCMBitDepthKey:@16,AVLinearPCMIsFloatKey:@NO,AVLinearPCMIsBigEndianKey:@NO};
    NSError *error = nil;
    self.recorder = [[AVAudioRecorder alloc] initWithURL:self.recordingURL settings:settings error:&error];
    self.recorder.meteringEnabled = YES;
    if (error || ![self.recorder recordForDuration:self.maxSeconds]) { self.status = @"no_microphone"; [self removeTemporaryAudio]; return; }
    [[NSFileManager defaultManager] setAttributes:@{NSFilePosixPermissions:@0600} ofItemAtPath:self.recordingURL.path error:nil];
#else
    NSString *fixturePath = NSProcessInfo.processInfo.environment[@"GROKBOT_FAKE_AUDIO_WAV"];
    NSURL *fixture = fixturePath.length ? [NSURL fileURLWithPath:fixturePath] : [NSBundle.mainBundle URLForResource:@"voice-fixture" withExtension:@"wav"];
    self.audio = fixture ? [NSData dataWithContentsOfURL:fixture] : nil;
    if (!self.audio.length) { self.status = @"failed"; return; }
#endif
    self.status = @"recording"; self.started = NSDate.date;
    __weak EzerbotVoice *weakSelf = self;
    self.timer = [NSTimer scheduledTimerWithTimeInterval:0.1 repeats:YES block:^(NSTimer *timer) {
        EzerbotVoice *owner = weakSelf;
        if (!owner || owner.generation != generation) { [timer invalidate]; return; }
        if (![owner.status isEqual:@"recording"]) return;
        owner.elapsed = MIN(owner.maxSeconds, -owner.started.timeIntervalSinceNow);
#if defined(EZERBOT_RELEASE)
        [owner.recorder updateMeters];
        owner.level = MAX(0, MIN(1, pow(10, [owner.recorder averagePowerForChannel:0] / 20.0)));
#else
        owner.level = 0.2 + fmod(owner.elapsed, 0.7);
#endif
        if (owner.elapsed >= owner.maxSeconds) { owner.tooLong = YES; [owner stop]; }
    }];
}
- (void)command:(NSDictionary *)body reply:(void (^)(NSDictionary *))reply {
    NSString *command = body[@"command"];
    NSString *token = body[@"token"];
    if (![command isKindOfClass:NSString.class] || ![token isKindOfClass:NSString.class] || token.length > 128) { reply(@{@"status":@"invalid"}); return; }
    if ([command isEqual:@"start"]) {
        if (self.status && [@[@"authorizing",@"recording",@"paused",@"transcribing"] containsObject:self.status]) { reply(@{@"status":self.meeting && ![body[@"purpose"] isEqual:@"meeting"] ? @"meeting_busy" : @"busy"}); return; }
        BOOL meeting = [body[@"purpose"] isEqual:@"meeting"];
        if (meeting && ![body[@"consent"] isEqual:@YES]) { reply(@{@"status":@"consent_required"}); return; }
        [self cancel]; self.token = token; self.meeting = meeting; self.maxSeconds = meeting ? 600 : 60;
        NSString *language = body[@"language"];
        self.language = [language isKindOfClass:NSString.class] && language.length > 0 && language.length <= 32 ? language : @"fr";
        self.elapsed = 0; self.tooLong = NO;
        NSUInteger generation = self.generation;
        self.status = @"authorizing";
#if defined(EZERBOT_RELEASE)
        self.recognizer = [[SFSpeechRecognizer alloc] initWithLocale:[NSLocale localeWithLocaleIdentifier:self.language]];
        // The UI currently declares « fr ». Prefer an installed French variant,
        // not a silent network fallback (fr-CA assets are present on our test Mac).
        if ([self.language isEqual:@"fr"] && !self.recognizer.supportsOnDeviceRecognition) {
            for (NSString *candidate in @[@"fr-CA", @"fr-FR"]) {
                SFSpeechRecognizer *recognizer = [[SFSpeechRecognizer alloc] initWithLocale:[NSLocale localeWithLocaleIdentifier:candidate]];
                if (recognizer.supportsOnDeviceRecognition) { self.recognizer = recognizer; self.language = candidate; break; }
            }
        }
        if (!self.recognizer.supportsOnDeviceRecognition) { self.status = @"offline_unavailable"; reply([self snapshot]); return; }
        __weak EzerbotVoice *weakSelf = self;
        [SFSpeechRecognizer requestAuthorization:^(SFSpeechRecognizerAuthorizationStatus status) {
            dispatch_async(dispatch_get_main_queue(), ^{
                EzerbotVoice *owner = weakSelf;
                if (!owner || owner.generation != generation) return;
                if (status != SFSpeechRecognizerAuthorizationStatusAuthorized) { owner.status = @"speech_denied"; return; }
                [AVCaptureDevice requestAccessForMediaType:AVMediaTypeAudio completionHandler:^(BOOL granted) {
                    dispatch_async(dispatch_get_main_queue(), ^{
                        EzerbotVoice *current = weakSelf;
                        if (!current || current.generation != generation) return;
                        if (!granted) { current.status = @"mic_denied"; return; }
                        [current beginRecording:generation];
                    });
                }];
            });
        }];
#else
        [self beginRecording:generation];
#endif
    } else if (![token isEqual:self.token]) { reply(@{@"status":@"stale"}); return; }
    else if ([command isEqual:@"cancel"]) [self cancel];
    else if ([command isEqual:@"stop"]) [self stop];
    else if ([command isEqual:@"pause"] && self.meeting && [self.status isEqual:@"recording"]) {
        self.pausedElapsed = self.elapsed; self.status = @"paused"; self.level = 0;
#if defined(EZERBOT_RELEASE)
        [self.recorder pause];
#endif
    }
    else if ([command isEqual:@"resume"] && self.meeting && [self.status isEqual:@"paused"]) {
        self.started = [NSDate dateWithTimeIntervalSinceNow:-self.pausedElapsed]; self.status = @"recording";
#if defined(EZERBOT_RELEASE)
        // Resume the same recording; the elapsed-time watchdog still enforces
        // the total cap independently of AVAudioRecorder's duration semantics.
        if (![self.recorder record]) { [self stop]; }
#endif
    }
    else if ([command isEqual:@"retry"] && ([self.status isEqual:@"failed"] || (self.meeting && [self.status isEqual:@"ready"] && self.transcriptionPlan.missingCount))) {
        ++self.generation;
        if (self.meeting && self.transcriptionPlan) {
            [self.transcriptionPlan retryMissing]; self.status = @"transcribing"; [self transcribeSegment];
        } else [self transcribe];
    }
    else if ([command isEqual:@"settings"]) {
#if defined(EZERBOT_RELEASE)
        NSString *pane = [self.status isEqual:@"speech_denied"] ? @"Privacy_SpeechRecognition" : @"Privacy_Microphone";
        NSString *url = [self.status isEqual:@"offline_unavailable"] ? @"x-apple.systempreferences:com.apple.Keyboard-Settings.extension?Dictation" : [@"x-apple.systempreferences:com.apple.preference.security?" stringByAppendingString:pane];
        [NSWorkspace.sharedWorkspace openURL:[NSURL URLWithString:url]];
#endif
    }
    reply([self snapshot]);
}
@end
