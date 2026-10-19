// Microphone and Speech are deliberately unreachable outside EZERBOT_RELEASE.
// The WebView bridge accepts main-frame, loopback-origin commands only (main.m).
#import <Cocoa/Cocoa.h>
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
#if defined(EZERBOT_RELEASE)
@property(nonatomic, strong) AVAudioRecorder *recorder;
@property(nonatomic, strong) SFSpeechRecognizer *recognizer;
@property(nonatomic, strong) SFSpeechRecognitionTask *recognitionTask;
#endif
- (void)command:(NSDictionary *)body reply:(void (^)(NSDictionary *))reply;
- (void)cancel;
@end

@implementation EzerbotVoice
- (NSDictionary *)snapshot {
    return @{ @"status":self.status ?: @"idle", @"token":self.token ?: @"",
        @"language":self.language ?: @"fr", @"transcript":self.transcript ?: @"",
        @"elapsed_ms":@(self.elapsed * 1000), @"level":@(self.level), @"too_long":@(self.tooLong),
        @"data":[self.status isEqual:@"ready"] ? ([self.audio base64EncodedStringWithOptions:0] ?: @"") : @"" };
}
- (void)removeTemporaryAudio {
    if (self.recordingURL) [[NSFileManager defaultManager] removeItemAtURL:self.recordingURL error:nil];
    self.recordingURL = nil;
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
    self.audio = nil; self.transcript = nil; self.status = @"idle"; self.level = 0;
}
- (void)transcribe {
    if (!self.audio.length) { self.status = @"failed"; return; }
    self.status = @"transcribing";
    NSUInteger generation = self.generation;
#if defined(EZERBOT_RELEASE)
    // No server fallback: unsupported/missing French assets are an explicit error.
    if (!self.recognizer.supportsOnDeviceRecognition) { self.status = @"offline_unavailable"; return; }
    SFSpeechURLRecognitionRequest *request = [[SFSpeechURLRecognitionRequest alloc] initWithURL:self.recordingURL];
    request.requiresOnDeviceRecognition = YES;
    request.shouldReportPartialResults = NO;
    __weak EzerbotVoice *weakSelf = self;
    self.recognitionTask = [self.recognizer recognitionTaskWithRequest:request resultHandler:^(SFSpeechRecognitionResult *result, NSError *error) {
        dispatch_async(dispatch_get_main_queue(), ^{
            EzerbotVoice *owner = weakSelf;
            if (!owner || owner.generation != generation || ![owner.status isEqual:@"transcribing"]) return;
            if (result.isFinal) {
                owner.transcript = [result.bestTranscription.formattedString stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet];
                owner.status = owner.transcript.length ? @"ready" : @"failed";
            } else if (error) { owner.status = @"failed"; }
        });
    }];
    // REA transcription deadline: 60 seconds. Timeout retains the WAV for retry.
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW, 60 * NSEC_PER_SEC), dispatch_get_main_queue(), ^{
        EzerbotVoice *owner = weakSelf;
        if (owner && owner.generation == generation && [owner.status isEqual:@"transcribing"]) {
            ++owner.generation; [owner.recognitionTask cancel]; owner.recognitionTask = nil; owner.status = @"failed";
        }
    });
#else
    (void)generation;
    self.transcript = @"Bonjour, voici mon mémo vocal pour préparer la réunion de demain.";
    self.status = @"ready";
#endif
}
- (void)stop {
    if (![self.status isEqual:@"recording"]) return;
    [self.timer invalidate]; self.timer = nil; self.level = 0;
#if defined(EZERBOT_RELEASE)
    self.elapsed = MIN(60, self.recorder.currentTime);
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
    if (error || ![self.recorder recordForDuration:60]) { self.status = @"no_microphone"; [self removeTemporaryAudio]; return; }
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
        owner.elapsed = MIN(60, -owner.started.timeIntervalSinceNow);
#if defined(EZERBOT_RELEASE)
        [owner.recorder updateMeters];
        owner.level = MAX(0, MIN(1, pow(10, [owner.recorder averagePowerForChannel:0] / 20.0)));
#else
        owner.level = 0.2 + fmod(owner.elapsed, 0.7);
#endif
        if (owner.elapsed >= 60) { owner.tooLong = YES; [owner stop]; }
    }];
}
- (void)command:(NSDictionary *)body reply:(void (^)(NSDictionary *))reply {
    NSString *command = body[@"command"];
    NSString *token = body[@"token"];
    if (![command isKindOfClass:NSString.class] || ![token isKindOfClass:NSString.class] || token.length > 128) { reply(@{@"status":@"invalid"}); return; }
    if ([command isEqual:@"start"]) {
        if (self.status && [@[@"authorizing",@"recording",@"transcribing"] containsObject:self.status]) { reply(@{@"status":@"busy"}); return; }
        [self cancel]; self.token = token;
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
    else if ([command isEqual:@"retry"] && [self.status isEqual:@"failed"]) { ++self.generation; [self transcribe]; }
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
