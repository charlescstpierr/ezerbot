// Real on-device recognition, no debug transcript and no microphone access.
#define EZERBOT_RELEASE 1
#import "voice.h"
#include <assert.h>
int main(int argc, const char *argv[]) {
    @autoreleasepool {
        assert(argc == 3);
        assert(SFSpeechRecognizer.authorizationStatus == SFSpeechRecognizerAuthorizationStatusAuthorized);
        EzerbotVoice *voice = [[EzerbotVoice alloc] init];
        NSString *english = @"en-US";
        NSString *french = @"fr-CA";
        NSArray *languages = @[english, french];
        NSArray *expected = @[@"hello world", @"bonjour le monde"];
        for (NSUInteger index = 0; index < languages.count; ++index) {
            voice.language = languages[index];
            voice.recognizer = [[SFSpeechRecognizer alloc] initWithLocale:[NSLocale localeWithLocaleIdentifier:voice.language]];
            assert(voice.recognizer.supportsOnDeviceRecognition);
            voice.recordingURL = [NSURL fileURLWithPath:[NSString stringWithUTF8String:argv[index + 1]]];
            voice.audio = [NSData dataWithContentsOfURL:voice.recordingURL];
            assert(voice.audio.length > 44);
            voice.transcript = @"";
            voice.status = @"transcribing";
            [voice transcribe]; // Production request with requiresOnDeviceRecognition=YES.
            NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:30];
            while ([voice.status isEqual:@"transcribing"] && deadline.timeIntervalSinceNow > 0)
                [NSRunLoop.currentRunLoop runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.05]];
            NSLog(@"%@ : %@", voice.status, voice.transcript);
            assert([voice.status isEqual:@"ready"]);
            NSString *text = [[voice.transcript lowercaseString] stringByTrimmingCharactersInSet:NSCharacterSet.punctuationCharacterSet];
            assert([text isEqual:expected[index]]);
        }
        // Do not cancel: the source is a checked-in fixture, not a temporary recording.
    }
    return 0;
}
