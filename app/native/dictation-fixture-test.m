// Real on-device recognition, no debug transcript and no microphone access.
// tools/test-dictation-fixture.sh installs one signed copy at a fixed path: macOS remembers the Speech
// permission for that exact binary, so it is granted once, whoever launches the test.
//   dictation-fixture-test <en-US.wav> <fr-CA.wav>   asks for Speech once if needed, then transcribes
//   dictation-fixture-test --status                  prints the Speech status, never asks for it
#define EZERBOT_RELEASE 1
#import "voice.h"
#include <assert.h>
#include <errno.h>
#include <mach-o/dyld.h>
#include <spawn.h>
#include <stdatomic.h>
#include <stdbool.h>
#include <sys/wait.h>

extern char **environ;
// Private libSystem calls, declared as in LLDB (PosixSpawnResponsible.h). A child spawned with the
// disclaim flag is its own responsible process: TCC asks about, and remembers, this binary instead of
// the Terminal or agent shell that launched it.
int responsibility_spawnattrs_setdisclaim(posix_spawnattr_t *attrs, bool disclaim);
pid_t responsibility_get_pid_responsible_for_pid(pid_t pid);

static const char *kDisclaimed = "EZERBOT_DICTATION_DISCLAIMED";
static const int kAnswerSeconds = 45; // Under the 60 s budget for a run without permission.
static atomic_bool answered;

static const char *statusName(SFSpeechRecognizerAuthorizationStatus status) {
    switch (status) {
    case SFSpeechRecognizerAuthorizationStatusNotDetermined: return "notDetermined";
    case SFSpeechRecognizerAuthorizationStatusDenied: return "denied";
    case SFSpeechRecognizerAuthorizationStatusRestricted: return "restricted";
    case SFSpeechRecognizerAuthorizationStatusAuthorized: return "authorized";
    }
    return "inconnu";
}

static int refuse(const char *path, SFSpeechRecognizerAuthorizationStatus status, const char *why) {
    fprintf(stderr,
            "ÉCHEC : permission Speech non accordée à %s (statut : %s, %s).\n"
            "Marche à suivre : devant le Mac, relancer ce test et répondre OK à l'invite macOS "
            "« ezerbot-dictation-fixture-test » souhaite accéder à la reconnaissance vocale. "
            "Si l'accès a été refusé : Réglages Système > Confidentialité et sécurité > Reconnaissance vocale, "
            "activer ezerbot-dictation-fixture-test.\n",
            path, statusName(status), why);
    return 1;
}

// Runs this same binary again as its own responsible process and returns its exit code.
static int respawnDisclaimed(const char *path, const char *argv[]) {
    posix_spawnattr_t attrs;
    int error = posix_spawnattr_init(&attrs);
    if (!error) error = responsibility_spawnattrs_setdisclaim(&attrs, true);
    if (error) {
        fprintf(stderr, "ÉCHEC : %s ne peut pas devenir son propre processus responsable (%s).\n", path, strerror(error));
        return 1;
    }
    setenv(kDisclaimed, "1", 1);
    pid_t child;
    error = posix_spawn(&child, path, NULL, &attrs, (char *const *)argv, environ);
    posix_spawnattr_destroy(&attrs);
    if (error) {
        fprintf(stderr, "ÉCHEC : lancement de %s impossible (%s).\n", path, strerror(error));
        return 1;
    }
    int status;
    while (waitpid(child, &status, 0) < 0)
        if (errno != EINTR) return 1;
    if (WIFEXITED(status)) return WEXITSTATUS(status);
    fprintf(stderr, "ÉCHEC : %s arrêté par le signal %d.\n", path, WTERMSIG(status));
    return 128 + WTERMSIG(status);
}

int main(int argc, const char *argv[]) {
    @autoreleasepool {
        char found[PATH_MAX], path[PATH_MAX];
        uint32_t size = sizeof found;
        bool located = _NSGetExecutablePath(found, &size) == 0 && realpath(found, path);
        assert(located);
        bool statusOnly = argc == 2 && strcmp(argv[1], "--status") == 0;
        if (!statusOnly) assert(argc == 3);
        pid_t me = getpid(), responsible = responsibility_get_pid_responsible_for_pid(me);
        if (!getenv(kDisclaimed)) {
            if (statusOnly) printf("lanceur : pid %d, processus responsable : pid %d\n", me, responsible);
            fflush(stdout);
            return respawnDisclaimed(path, argv);
        }
        if (responsible != me) {
            fprintf(stderr, "ÉCHEC : %s (pid %d) n'est pas son propre processus responsable (pid %d) : "
                            "la permission irait à l'app qui l'a lancé. Aucune demande faite.\n", path, me, responsible);
            return 1;
        }
        SFSpeechRecognizerAuthorizationStatus status = SFSpeechRecognizer.authorizationStatus;
        if (statusOnly) {
            printf("binaire : %s\nprocessus responsable : pid %d (lui-même)\nstatut Speech : %s\n", path, responsible, statusName(status));
            fflush(stdout);
            return status == SFSpeechRecognizerAuthorizationStatusAuthorized ? 0 : refuse(path, status, "--status ne demande rien");
        }
        if (status == SFSpeechRecognizerAuthorizationStatusNotDetermined) {
            fprintf(stderr, "Demande de la permission Speech pour %s : répondre à l'invite macOS dans les %d s.\n", path, kAnswerSeconds);
            [SFSpeechRecognizer requestAuthorization:^(SFSpeechRecognizerAuthorizationStatus answer) {
                (void)answer;
                atomic_store(&answered, true);
            }];
            NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:kAnswerSeconds];
            while (!atomic_load(&answered) && deadline.timeIntervalSinceNow > 0)
                [NSRunLoop.currentRunLoop runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.05]];
            status = SFSpeechRecognizer.authorizationStatus;
            if (status == SFSpeechRecognizerAuthorizationStatusNotDetermined)
                return refuse(path, status, "pas de réponse à l'invite dans le délai");
        }
        if (status != SFSpeechRecognizerAuthorizationStatusAuthorized) return refuse(path, status, "accès refusé ou restreint");
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
