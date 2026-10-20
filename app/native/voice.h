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
@property(nonatomic) BOOL meeting;
@property(nonatomic) NSUInteger maxSeconds;
@property(nonatomic) double pausedElapsed;
@property(nonatomic) NSUInteger segmentOffset;
@property(nonatomic, strong) NSData *pcm;
@property(nonatomic, strong) NSURL *segmentURL;
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
    if (self.segmentURL) [[NSFileManager defaultManager] removeItemAtURL:self.segmentURL error:nil];
    self.recordingURL = nil; self.segmentURL = nil; self.pcm = nil;
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
    self.transcript = @""; self.segmentOffset = 0;
    [self transcribeSegment];
}
- (void)transcribeSegment {
    NSUInteger generation = self.generation;
#if defined(EZERBOT_RELEASE)
    // No server fallback: unsupported/missing French assets are an explicit error.
    if (!self.recognizer.supportsOnDeviceRecognition) { self.status = @"offline_unavailable"; return; }
    NSURL *source = self.recordingURL;
    if (self.meeting) {
        // AVAudioRecorder emits mono PCM16 WAV. Locate the data chunk rather
        // than assuming Apple's header is the canonical 44 bytes.
        if (!self.pcm) {
            const uint8_t *bytes = self.audio.bytes;
            for (NSUInteger p = 12; p + 8 <= self.audio.length;) {
                uint32_t count = 0; memcpy(&count, bytes + p + 4, 4);
                if (count > self.audio.length - p - 8) break;
                if (!memcmp(bytes + p, "data", 4)) { self.pcm = [self.audio subdataWithRange:NSMakeRange(p+8,count)]; break; }
                p += 8 + count + count % 2;
            }
        }
        if (!self.pcm.length || self.segmentOffset >= self.pcm.length) { self.status = @"failed"; return; }
        NSUInteger count = MIN((NSUInteger)60*32000, self.pcm.length-self.segmentOffset);
        NSMutableData *wav = [NSMutableData dataWithLength:44];
        uint8_t *header = wav.mutableBytes;
        memcpy(header,"RIFF",4); uint32_t size=(uint32_t)count+36; memcpy(header+4,&size,4);
        memcpy(header+8,"WAVEfmt ",8); uint32_t fmt=16,rate=16000,byteRate=32000; uint16_t one=1,two=2,bits=16;
        memcpy(header+16,&fmt,4); memcpy(header+20,&one,2); memcpy(header+22,&one,2); memcpy(header+24,&rate,4);
        memcpy(header+28,&byteRate,4); memcpy(header+32,&two,2); memcpy(header+34,&bits,2); memcpy(header+36,"data",4);
        uint32_t length=(uint32_t)count; memcpy(header+40,&length,4);
        [wav appendData:[self.pcm subdataWithRange:NSMakeRange(self.segmentOffset,count)]];
        if (self.segmentURL) [[NSFileManager defaultManager] removeItemAtURL:self.segmentURL error:nil];
        self.segmentURL = [NSURL fileURLWithPath:[NSTemporaryDirectory() stringByAppendingPathComponent:[NSString stringWithFormat:@"ezerbot-meeting-%@.wav",NSUUID.UUID.UUIDString]]];
        if (![wav writeToURL:self.segmentURL options:NSDataWritingAtomic error:nil]) { self.status = @"failed"; return; }
        [[NSFileManager defaultManager] setAttributes:@{NSFilePosixPermissions:@0600} ofItemAtPath:self.segmentURL.path error:nil];
        self.segmentOffset += count; source = self.segmentURL;
    }
    SFSpeechURLRecognitionRequest *request = [[SFSpeechURLRecognitionRequest alloc] initWithURL:source];
    request.requiresOnDeviceRecognition = YES;
    request.shouldReportPartialResults = NO;
    __weak EzerbotVoice *weakSelf = self;
    self.recognitionTask = [self.recognizer recognitionTaskWithRequest:request resultHandler:^(SFSpeechRecognitionResult *result, NSError *error) {
        dispatch_async(dispatch_get_main_queue(), ^{
            EzerbotVoice *owner = weakSelf;
            if (!owner || owner.generation != generation || ![owner.status isEqual:@"transcribing"]) return;
            if (result.isFinal) {
                NSString *text = [result.bestTranscription.formattedString stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet];
                owner.transcript = [owner.transcript stringByAppendingFormat:@"%@%@",owner.transcript.length ? @"\n" : @"",text];
                if (owner.meeting && owner.segmentOffset < owner.pcm.length) {
                    ++owner.generation; [owner transcribeSegment];
                } else {
                    owner.status = owner.transcript.length ? @"ready" : @"failed";
                    if (owner.meeting && [owner.status isEqual:@"ready"]) [owner removeTemporaryAudio];
                }
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
        if (self.status && [@[@"authorizing",@"recording",@"paused",@"transcribing"] containsObject:self.status]) { reply(@{@"status":@"busy"}); return; }
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
        [self.recorder recordForDuration:MAX(0.1,self.maxSeconds-self.pausedElapsed)];
#endif
    }
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
