// Full duplex local conversation. Debug builds cannot link to capture or synthesis paths.
@interface EzerbotVoiceSession : NSObject
@property(nonatomic, copy) NSString *token;
@property(nonatomic, copy) NSString *status;
@property(nonatomic, copy) NSString *transcript;
@property(nonatomic, copy) NSString *finalText;
@property(nonatomic) NSUInteger generation;
@property(nonatomic) NSUInteger recognitionGeneration;
@property(nonatomic) NSUInteger utterance;
@property(nonatomic) NSUInteger barged;
@property(nonatomic) BOOL speaking;
@property(nonatomic) BOOL paused;
@property(nonatomic) BOOL muted;
@property(nonatomic) BOOL speechDetected;
@property(nonatomic) NSUInteger loudBuffers;
@property(nonatomic) BOOL tapInstalled;
@property(nonatomic, strong) NSTimer *silence;
@property(nonatomic, strong) id changeObserver;
@property(nonatomic, strong) NSMutableArray<NSString *> *spoken;
@property(nonatomic, copy) NSDictionary *microphone;
#if defined(EZERBOT_RELEASE)
@property(nonatomic) AudioDeviceID chosenDevice;
@property(nonatomic, strong) AVAudioEngine *engine;
@property(nonatomic, strong) SFSpeechRecognizer *recognizer;
@property(atomic, strong) SFSpeechAudioBufferRecognitionRequest *request;
@property(nonatomic, strong) SFSpeechRecognitionTask *task;
@property(nonatomic, strong) AVSpeechSynthesizer *synth;
@property(nonatomic, strong) AVAudioPlayerNode *player;
@property(nonatomic, strong) AVAudioFormat *playbackFormat;
@property(nonatomic, strong) AVAudioConverter *converter;
@property(nonatomic, strong) NSMutableArray<AVSpeechUtterance *> *speechQueue;
@property(nonatomic) NSUInteger speechGeneration;
@property(nonatomic) NSUInteger pendingBuffers;
@property(nonatomic) BOOL synthesisFinished;
#else
@property(nonatomic, strong) EzerbotVoice *fakeSource;
#endif
- (void)command:(NSDictionary *)body reply:(void (^)(NSDictionary *))reply;
- (void)cancel;
@end

@implementation EzerbotVoiceSession
- (NSDictionary *)snapshot {
    return @{ @"token":self.token ?: @"", @"status":self.status ?: @"idle", @"transcript":self.transcript ?: @"",
        @"utterance_id":@(self.utterance), @"final_text":self.finalText ?: @"", @"barged":@(self.barged),
        @"speaking":@(self.speaking && !self.paused), @"paused":@(self.paused), @"muted":@(self.muted),
        @"microphone_id":self.microphone[@"id"] ?: @"", @"microphone_name":self.microphone[@"name"] ?: @"", @"microphone_fallback":self.microphone[@"fallback"] ?: @NO };
}
- (void)interrupt {
#if defined(EZERBOT_RELEASE)
    ++self.speechGeneration;
    [self.synth stopSpeakingAtBoundary:AVSpeechBoundaryImmediate];
    [self.player stop]; [self.speechQueue removeAllObjects];
    self.pendingBuffers = 0; self.converter = nil;
#endif
    self.speaking = NO; self.paused = NO;
}
- (void)cancel {
    ++self.generation;
    [self interrupt];
#if defined(EZERBOT_RELEASE)
    [self teardownEngine];
    self.recognizer = nil; self.synth = nil;
#else
    [self.fakeSource cancel]; self.fakeSource = nil;
#endif
    self.transcript = nil; self.finalText = nil; self.status = @"idle"; self.muted = NO;
}
#if defined(EZERBOT_RELEASE)
- (void)teardownEngine {
    ++self.recognitionGeneration;
    [self.silence invalidate]; self.silence = nil;
    if (self.changeObserver) [NSNotificationCenter.defaultCenter removeObserver:self.changeObserver]; self.changeObserver = nil;
    [self.engine stop];
    if (self.engine && self.tapInstalled) [self.engine.inputNode removeTapOnBus:0];
    self.tapInstalled = NO; self.engine = nil;
    [self.request endAudio]; self.request = nil;
    [self.task cancel]; self.task = nil;
    self.player = nil; self.playbackFormat = nil;
}
- (void)fail:(NSString *)status {
    [self cancel]; self.status = status;
}
// Speech is rendered through the SAME engine's output, so voice processing has
// the actual TTS reference signal. AVSpeechSynthesizer never plays a separate
// system-output stream that the microphone could mistake for the user.
- (void)finishSentenceIfDrained {
    if (!self.synthesisFinished || self.pendingBuffers) return;
    if (self.speechQueue.count) [self.speechQueue removeObjectAtIndex:0];
    self.speaking = self.speechQueue.count > 0;
    if (self.speaking) [self synthesizeSentence];
}
- (void)playBuffer:(AVAudioPCMBuffer *)buffer generation:(NSUInteger)generation {
    if (!buffer.frameLength || generation != self.speechGeneration) return;
    ++self.pendingBuffers;
    __weak EzerbotVoiceSession *weakSelf = self;
    [self.player scheduleBuffer:buffer completionCallbackType:AVAudioPlayerNodeCompletionDataPlayedBack completionHandler:^(AVAudioPlayerNodeCompletionCallbackType type) {
        (void)type;
        dispatch_async(dispatch_get_main_queue(), ^{
            EzerbotVoiceSession *owner = weakSelf;
            if (!owner || owner.speechGeneration != generation) return;
            if (owner.pendingBuffers) --owner.pendingBuffers;
            [owner finishSentenceIfDrained];
        });
    }];
    if (!self.paused) [self.player play];
}
- (void)synthesizeSentence {
    if (!self.speechQueue.count) return;
    self.synthesisFinished = NO; self.converter = nil;
    NSUInteger generation = self.speechGeneration;
    __weak EzerbotVoiceSession *weakSelf = self;
    [self.synth writeUtterance:self.speechQueue.firstObject toBufferCallback:^(AVAudioBuffer *audio) {
        dispatch_async(dispatch_get_main_queue(), ^{
            EzerbotVoiceSession *owner = weakSelf;
            if (!owner || owner.speechGeneration != generation || !owner.engine) return;
            if (![audio isKindOfClass:AVAudioPCMBuffer.class]) { [owner fail:@"tts_unavailable"]; return; }
            AVAudioPCMBuffer *input = (AVAudioPCMBuffer *)audio;
            BOOL end = input.frameLength == 0;
            if (!owner.converter && !end) owner.converter = [[AVAudioConverter alloc] initFromFormat:input.format toFormat:owner.playbackFormat];
            if (owner.converter) {
                AVAudioFrameCount capacity = end ? 4096 : (AVAudioFrameCount)ceil(input.frameLength * owner.playbackFormat.sampleRate / input.format.sampleRate) + 4096;
                AVAudioPCMBuffer *output = [[AVAudioPCMBuffer alloc] initWithPCMFormat:owner.playbackFormat frameCapacity:capacity];
                __block BOOL supplied = NO;
                NSError *error = nil;
                AVAudioConverterOutputStatus converted = [owner.converter convertToBuffer:output error:&error withInputFromBlock:^AVAudioBuffer *(AVAudioPacketCount count, AVAudioConverterInputStatus *status) {
                    (void)count;
                    if (end) { *status = AVAudioConverterInputStatus_EndOfStream; return nil; }
                    if (supplied) { *status = AVAudioConverterInputStatus_NoDataNow; return nil; }
                    supplied = YES; *status = AVAudioConverterInputStatus_HaveData; return input;
                }];
                if (error || converted == AVAudioConverterOutputStatus_Error) { [owner fail:@"tts_unavailable"]; return; }
                [owner playBuffer:output generation:generation];
            } else if (!end) { [owner fail:@"tts_unavailable"]; return; }
            if (end) { owner.synthesisFinished = YES; [owner finishSentenceIfDrained]; }
        });
    }];
}
- (void)userSpeech {
    if (self.speechDetected) return;
    self.speechDetected = YES; ++self.barged; [self interrupt];
}
- (void)recognize {
    self.speechDetected = NO; self.loudBuffers = 0;
    NSUInteger generation = self.generation, cycle = ++self.recognitionGeneration;
    [self.task cancel]; self.task = nil;
    self.request = [[SFSpeechAudioBufferRecognitionRequest alloc] init];
    self.request.requiresOnDeviceRecognition = YES; self.request.shouldReportPartialResults = YES;
    self.transcript = @"";
    __weak EzerbotVoiceSession *weakSelf = self;
    self.task = [self.recognizer recognitionTaskWithRequest:self.request resultHandler:^(SFSpeechRecognitionResult *result, NSError *error) {
        dispatch_async(dispatch_get_main_queue(), ^{
            EzerbotVoiceSession *owner = weakSelf;
            if (!owner || owner.generation != generation || owner.recognitionGeneration != cycle) return;
            if (result) {
                NSString *text = [result.bestTranscription.formattedString stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet];
                if (text.length && ![text isEqual:owner.transcript]) {
                    // Recognition is fed by the voice-processing (echo-cancelled) input.
                    // Interrupt here, not on the next WebView polling frame.
                    if (!owner.transcript.length) [owner userSpeech];
                    owner.transcript = text;
                    [owner.silence invalidate];
                    owner.silence = [NSTimer scheduledTimerWithTimeInterval:0.8 repeats:NO block:^(NSTimer *timer) {
                        (void)timer;
                        EzerbotVoiceSession *current = weakSelf;
                        if (!current || current.generation != generation || current.recognitionGeneration != cycle) return;
                        current.finalText = current.transcript; ++current.utterance;
                        [current recognize];
                    }];
                }
                if (result.isFinal) {
                    [owner.silence invalidate]; owner.silence = nil;
                    if (text.length) { owner.finalText = text; ++owner.utterance; }
                    [owner recognize];
                }
            } else if (error) { [owner fail:@"failed"]; }
        });
    }];
    // Rotate Speech's bounded requests without stopping the audio engine or losing a phrase.
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW, 50 * NSEC_PER_SEC), dispatch_get_main_queue(), ^{
        EzerbotVoiceSession *owner = weakSelf;
        if (!owner || owner.generation != generation || owner.recognitionGeneration != cycle) return;
        if (owner.transcript.length) { owner.finalText = owner.transcript; ++owner.utterance; }
        [owner.silence invalidate]; owner.silence = nil; [owner recognize];
    });
}
// A muted engine is output-only: AVAudioEngine opens the input unit (and the
// microphone) only when inputNode is accessed, so that path never touches it.
- (BOOL)buildEngine:(BOOL)listening {
    NSUInteger generation = self.generation;
    self.engine = [[AVAudioEngine alloc] init];
    NSError *error = nil;
    AVAudioFormat *format = nil;
    if (listening) {
        // The microphone chosen in the settings. Without a choice, the system default serves.
        if (self.chosenDevice != kAudioObjectUnknown) {
            AudioDeviceID device = self.chosenDevice; AudioUnit unit = self.engine.inputNode.audioUnit;
            if (unit) AudioUnitSetProperty(unit, kAudioOutputUnitProperty_CurrentDevice, kAudioUnitScope_Global, 0, &device, sizeof(device));
        }
        // No raw microphone fallback while the speaker is active: avoid self-transcription.
        if (![self.engine.inputNode setVoiceProcessingEnabled:YES error:&error]) { [self fail:@"audio_failed"]; return NO; }
        format = [self.engine.inputNode outputFormatForBus:0];
        if (!format.sampleRate || !format.channelCount) { [self fail:@"no_microphone"]; return NO; }
    }
    self.player = [[AVAudioPlayerNode alloc] init];
    [self.engine attachNode:self.player];
    double rate = [self.engine.outputNode outputFormatForBus:0].sampleRate;
    self.playbackFormat = [[AVAudioFormat alloc] initStandardFormatWithSampleRate:rate > 0 ? rate : 48000 channels:1];
    [self.engine connect:self.player to:self.engine.mainMixerNode format:self.playbackFormat];
    __weak EzerbotVoiceSession *weakSelf = self;
    if (listening) {
        [self recognize];
        [self.engine.inputNode installTapOnBus:0 bufferSize:1024 format:format block:^(AVAudioPCMBuffer *buffer, AVAudioTime *when) {
            (void)when;
            EzerbotVoiceSession *owner = weakSelf;
            [owner.request appendAudioPCMBuffer:buffer];
            // Three echo-cancelled input buffers (~64 ms at 48 kHz) interrupt speech
            // before the recognizer can produce its first partial transcript.
            float *samples = buffer.floatChannelData ? buffer.floatChannelData[0] : NULL;
            if (samples && owner.speaking && !owner.paused && buffer.frameLength) {
                double energy = 0;
                for (AVAudioFrameCount i = 0; i < buffer.frameLength; ++i) energy += samples[i] * samples[i];
                owner.loudBuffers = sqrt(energy / buffer.frameLength) > 0.025 ? owner.loudBuffers + 1 : 0;
                if (owner.loudBuffers == 3) dispatch_async(dispatch_get_main_queue(), ^{
                    EzerbotVoiceSession *current = weakSelf;
                    if (current && current.generation == generation && current.speaking) [current userSpeech];
                });
            } else { owner.loudBuffers = 0; }
        }];
        self.tapInstalled = YES;
    }
    [self.engine prepare];
    if (![self.engine startAndReturnError:&error]) { [self fail:@"audio_failed"]; return NO; }
    // Capture route changes and unplugged microphones fail closed, with a French UI error.
    // Muted, nothing is captured: closing the microphone can itself change the output
    // (a Bluetooth headset leaves its call profile), so the output engine is rebuilt.
    self.changeObserver = [NSNotificationCenter.defaultCenter addObserverForName:AVAudioEngineConfigurationChangeNotification object:self.engine queue:NSOperationQueue.mainQueue usingBlock:^(NSNotification *note) {
        EzerbotVoiceSession *owner = weakSelf;
        if (!owner || owner.generation != generation || note.object != owner.engine) return;
        if (owner.muted) [owner rebuildEngine]; else [owner fail:@"audio_failed"];
    }];
    return YES;
}
- (void)capture:(NSUInteger)generation {
    if (self.generation != generation) return;
    if (![AVCaptureDevice defaultDeviceWithMediaType:AVMediaTypeAudio]) { [self fail:@"no_microphone"]; return; }
    self.speechQueue = [NSMutableArray array];
    if (![self buildEngine:!self.muted]) return;
    self.status = @"listening";
    if (!self.synth) self.synth = [[AVSpeechSynthesizer alloc] init];
}
// The bot keeps talking: its queue stays and the current sentence restarts through the new engine.
- (void)rebuildEngine {
    if (!self.engine) return;
    ++self.speechGeneration;
    [self.synth stopSpeakingAtBoundary:AVSpeechBoundaryImmediate]; [self.player stop];
    self.pendingBuffers = 0; self.converter = nil;
    [self teardownEngine];
    if (![self buildEngine:!self.muted]) return;
    if (self.speechQueue.count) [self synthesizeSentence];
}
#else
- (void)startFakeSource {
    self.fakeSource = [[EzerbotVoice alloc] init];
    [self.fakeSource command:@{@"command":@"start",@"token":self.token,@"language":@"fr"} reply:^(NSDictionary *result) {
        self.status = [result[@"status"] isEqual:@"recording"] ? @"listening" : result[@"status"];
    }];
}
#endif
- (void)command:(NSDictionary *)body reply:(void (^)(NSDictionary *))reply {
    NSString *command = body[@"command"], *token = body[@"token"];
    if (![command isKindOfClass:NSString.class] || ![token isKindOfClass:NSString.class] || token.length > 128) { reply(@{@"status":@"invalid"}); return; }
    if ([command isEqual:@"options"]) {
        NSMutableArray *voices = [NSMutableArray array];
#if defined(EZERBOT_RELEASE)
        // Every voice the Mac can pronounce, French first: a bot without a choice is given the first free one.
        for (AVSpeechSynthesisVoice *voice in AVSpeechSynthesisVoice.speechVoices) [voices addObject:@{@"id":voice.identifier,@"name":voice.name,@"language":voice.language}];
        [voices sortWithOptions:NSSortStable usingComparator:^NSComparisonResult(NSDictionary *a, NSDictionary *b) {
            BOOL french = [a[@"language"] hasPrefix:@"fr"], otherFrench = [b[@"language"] hasPrefix:@"fr"];
            return french == otherFrench ? NSOrderedSame : french ? NSOrderedAscending : NSOrderedDescending;
        }];
#else
        [voices addObject:@{@"id":@"fake-fr-CA",@"name":@"Voix française de test (Canada)",@"language":@"fr-CA"}];
        [voices addObject:@{@"id":@"fake-fr-FR",@"name":@"Voix française de test (France)",@"language":@"fr-FR"}];
#endif
        reply(@{@"voices":voices}); return;
    }
    if ([command isEqual:@"start"]) {
        [self cancel]; self.token = token; self.utterance = 0; self.barged = 0;
        NSString *requested = body[@"microphone"];
        self.microphone = [EzerbotMicrophones resolve:[requested isKindOfClass:NSString.class] && requested.length <= 256 ? requested : @""];
#if defined(EZERBOT_RELEASE)
        self.chosenDevice = [EzerbotMicrophones deviceForResolved:self.microphone];
#endif
        self.spoken = [NSMutableArray array]; self.status = @"authorizing";
#if defined(EZERBOT_RELEASE)
        NSUInteger generation = self.generation;
        for (NSString *locale in @[@"fr-CA",@"fr-FR",@"fr"]) {
            SFSpeechRecognizer *recognizer = [[SFSpeechRecognizer alloc] initWithLocale:[NSLocale localeWithLocaleIdentifier:locale]];
            if (recognizer.supportsOnDeviceRecognition) { self.recognizer = recognizer; break; }
        }
        if (!self.recognizer) { self.status = @"offline_unavailable"; reply([self snapshot]); return; }
        __weak EzerbotVoiceSession *weakSelf = self;
        [SFSpeechRecognizer requestAuthorization:^(SFSpeechRecognizerAuthorizationStatus status) {
            dispatch_async(dispatch_get_main_queue(), ^{
                EzerbotVoiceSession *owner = weakSelf;
                if (!owner || owner.generation != generation) return;
                if (status != SFSpeechRecognizerAuthorizationStatusAuthorized) { owner.status = @"speech_denied"; return; }
                [AVCaptureDevice requestAccessForMediaType:AVMediaTypeAudio completionHandler:^(BOOL granted) {
                    dispatch_async(dispatch_get_main_queue(), ^{
                        EzerbotVoiceSession *current = weakSelf;
                        if (!current || current.generation != generation) return;
                        if (!granted) { current.status = @"mic_denied"; return; }
                        [current capture:generation];
                    });
                }];
            });
        }];
#else
        [self startFakeSource];
#endif
    } else if (![token isEqual:self.token]) { reply(@{@"status":@"stale"}); return; }
    else if ([command isEqual:@"cancel"]) [self cancel];
    else if ([command isEqual:@"interrupt"]) { [self interrupt]; self.transcript = @""; }
    else if ([command isEqual:@"pause"]) {
        self.paused = YES;
#if defined(EZERBOT_RELEASE)
        [self.player pause];
#endif
    } else if ([command isEqual:@"resume"]) {
        self.paused = NO;
#if defined(EZERBOT_RELEASE)
        if (self.pendingBuffers) [self.player play];
#endif
    } else if ([command isEqual:@"mute"] || [command isEqual:@"unmute"]) {
        BOOL mute = [command isEqual:@"mute"];
        if (mute != self.muted) {
            self.muted = mute;
            if (mute) self.transcript = @"";
#if defined(EZERBOT_RELEASE)
            [self rebuildEngine];
#else
            if (mute) { [self.fakeSource cancel]; self.fakeSource = nil; } else [self startFakeSource];
#endif
        }
    } else if ([command isEqual:@"speak"]) {
        NSString *text = body[@"text"], *identifier = body[@"voice"], *language = body[@"language"] ?: @"";
        NSNumber *speed = body[@"speed"], *barge = body[@"barge"];
        if (![text isKindOfClass:NSString.class] || !text.length || text.length > 1500 || ![identifier isKindOfClass:NSString.class] || ![language isKindOfClass:NSString.class] || language.length > 32 || ![speed isKindOfClass:NSNumber.class] || (barge && ![barge isKindOfClass:NSNumber.class]) || !isfinite(speed.doubleValue) || speed.doubleValue < 0.5 || speed.doubleValue > 2) { reply(@{@"status":@"invalid"}); return; }
        if (![self.status isEqual:@"listening"] || (barge && barge.unsignedIntegerValue != self.barged)) { reply([self snapshot]); return; }
#if defined(EZERBOT_RELEASE)
        // No voice chosen: the Mac's voice for the bot's language, French when the language is Auto.
        AVSpeechSynthesisVoice *voice = identifier.length ? [AVSpeechSynthesisVoice voiceWithIdentifier:identifier] : [AVSpeechSynthesisVoice voiceWithLanguage:language.length ? language : @"fr-CA"];
        if (!voice && !identifier.length) voice = [AVSpeechSynthesisVoice voiceWithLanguage:language.length ? language : @"fr-FR"];
        if (!voice || (language.length && ![voice.language.lowercaseString hasPrefix:language.lowercaseString])) { [self fail:@"tts_unavailable"]; reply([self snapshot]); return; }
        AVSpeechUtterance *utterance = [AVSpeechUtterance speechUtteranceWithString:text];
        utterance.voice = voice; utterance.rate = AVSpeechUtteranceDefaultSpeechRate * speed.floatValue;
        [self.speechQueue addObject:utterance];
        if (self.speechQueue.count == 1) [self synthesizeSentence];
#else
        [self.spoken addObject:text];
#endif
        self.speaking = YES;
    } else if ([command isEqual:@"settings"]) {
#if defined(EZERBOT_RELEASE)
        NSString *pane = [body[@"reason"] isEqual:@"speech_denied"] ? @"Privacy_SpeechRecognition" : @"Privacy_Microphone";
        NSString *url = [body[@"reason"] isEqual:@"offline_unavailable"] ? @"x-apple.systempreferences:com.apple.Keyboard-Settings.extension?Dictation" : [@"x-apple.systempreferences:com.apple.preference.security?" stringByAppendingString:pane];
        [NSWorkspace.sharedWorkspace openURL:[NSURL URLWithString:url]];
#endif
    }
#if defined(EZERBOT_DEBUG)
    else if ([command isEqual:@"fail"]) {
        [self cancel]; self.status = @"failed";
    }
    else if ([command isEqual:@"feed"]) {
        NSString *text = body[@"text"];
        if (!self.muted && [text isKindOfClass:NSString.class]) {
            self.transcript = text; ++self.barged; [self interrupt];
            if ([body[@"final_result"] isKindOfClass:NSNumber.class] && [body[@"final_result"] boolValue]) { self.finalText = text; ++self.utterance; }
        }
    }
#endif
    reply([self snapshot]);
}
@end
