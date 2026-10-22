// The microphones Ezerbot can listen to, shared by dictation and the voice mode.
// Debug builds list fixtures and never reach CoreAudio.
#pragma once
#import <Foundation/Foundation.h>
#if defined(EZERBOT_RELEASE)
#import <CoreAudio/CoreAudio.h>
#endif

@interface EzerbotMicrophones : NSObject
// @[@{@"id":uid,@"name":name}]: input devices only, the system default first.
+ (NSArray<NSDictionary *> *)available;
// The microphone Ezerbot listens to for the requested id (empty = system default).
// @{@"id",@"name",@"fallback"}: fallback is YES when a chosen microphone is gone.
+ (NSDictionary *)resolve:(NSString *)requested;
#if defined(EZERBOT_RELEASE)
// The device to record from, or kAudioObjectUnknown when the system default serves.
+ (AudioDeviceID)deviceForResolved:(NSDictionary *)resolved;
#else
@property(class, nonatomic, copy) NSArray<NSString *> *fakeUnplugged;
#endif
@end

@implementation EzerbotMicrophones
#if defined(EZERBOT_RELEASE)
static AudioObjectPropertyAddress EzerbotAddress(AudioObjectPropertySelector selector, AudioObjectPropertyScope scope) {
    return (AudioObjectPropertyAddress){selector, scope, kAudioObjectPropertyElementMain};
}
static NSString *EzerbotString(AudioObjectID object, AudioObjectPropertySelector selector) {
    AudioObjectPropertyAddress address = EzerbotAddress(selector, kAudioObjectPropertyScopeGlobal);
    CFStringRef value = NULL; UInt32 size = sizeof(value);
    if (AudioObjectGetPropertyData(object, &address, 0, NULL, &size, &value) != noErr || !value) return nil;
    return CFBridgingRelease(value);
}
static AudioDeviceID EzerbotDefaultInput(void) {
    AudioObjectPropertyAddress address = EzerbotAddress(kAudioHardwarePropertyDefaultInputDevice, kAudioObjectPropertyScopeGlobal);
    AudioDeviceID device = kAudioObjectUnknown; UInt32 size = sizeof(device);
    if (AudioObjectGetPropertyData(kAudioObjectSystemObject, &address, 0, NULL, &size, &device) != noErr) return kAudioObjectUnknown;
    return device;
}
static BOOL EzerbotHasInput(AudioDeviceID device) {
    AudioObjectPropertyAddress streams = EzerbotAddress(kAudioDevicePropertyStreams, kAudioObjectPropertyScopeInput);
    UInt32 size = 0;
    if (AudioObjectGetPropertyDataSize(device, &streams, 0, NULL, &size) != noErr || !size) return NO;
    // The aggregate device voice processing creates is not a microphone to choose.
    AudioObjectPropertyAddress transport = EzerbotAddress(kAudioDevicePropertyTransportType, kAudioObjectPropertyScopeGlobal);
    UInt32 kind = 0; UInt32 kindSize = sizeof(kind);
    if (AudioObjectGetPropertyData(device, &transport, 0, NULL, &kindSize, &kind) == noErr && (kind == kAudioDeviceTransportTypeAggregate || kind == kAudioDeviceTransportTypeAutoAggregate)) return NO;
    return YES;
}
+ (NSArray<NSDictionary *> *)available {
    AudioObjectPropertyAddress devices = EzerbotAddress(kAudioHardwarePropertyDevices, kAudioObjectPropertyScopeGlobal);
    UInt32 size = 0;
    if (AudioObjectGetPropertyDataSize(kAudioObjectSystemObject, &devices, 0, NULL, &size) != noErr || !size) return @[];
    NSMutableData *ids = [NSMutableData dataWithLength:size];
    if (AudioObjectGetPropertyData(kAudioObjectSystemObject, &devices, 0, NULL, &size, ids.mutableBytes) != noErr) return @[];
    AudioDeviceID *list = ids.mutableBytes; NSUInteger count = size / sizeof(AudioDeviceID);
    AudioDeviceID fallback = EzerbotDefaultInput();
    NSMutableArray *microphones = [NSMutableArray array];
    for (NSUInteger i = 0; i < count; i++) {
        if (!EzerbotHasInput(list[i])) continue;
        NSString *uid = EzerbotString(list[i], kAudioDevicePropertyDeviceUID), *name = EzerbotString(list[i], kAudioObjectPropertyName);
        if (!uid.length || uid.length > 256 || !name.length) continue;
        NSDictionary *entry = @{@"id":uid,@"name":name};
        if (list[i] == fallback) [microphones insertObject:entry atIndex:0]; else [microphones addObject:entry];
    }
    return microphones;
}
+ (NSDictionary *)resolve:(NSString *)requested {
    NSArray<NSDictionary *> *microphones = [self available];
    NSDictionary *chosen = nil;
    for (NSDictionary *entry in microphones) if (requested.length && [entry[@"id"] isEqual:requested]) chosen = entry;
    NSDictionary *effective = chosen ?: microphones.firstObject;
    return @{@"id":effective[@"id"] ?: @"", @"name":effective[@"name"] ?: @"", @"fallback":@((BOOL)(requested.length && !chosen))};
}
+ (AudioDeviceID)deviceForResolved:(NSDictionary *)resolved {
    NSString *uid = resolved[@"id"];
    if (!uid.length) return kAudioObjectUnknown;
    AudioObjectPropertyAddress address = EzerbotAddress(kAudioHardwarePropertyTranslateUIDToDevice, kAudioObjectPropertyScopeGlobal);
    CFStringRef qualifier = (__bridge CFStringRef)uid;
    AudioDeviceID device = kAudioObjectUnknown; UInt32 size = sizeof(device);
    if (AudioObjectGetPropertyData(kAudioObjectSystemObject, &address, sizeof(qualifier), &qualifier, &size, &device) != noErr) return kAudioObjectUnknown;
    // The default microphone is recorded as it always was.
    return device == EzerbotDefaultInput() ? kAudioObjectUnknown : device;
}
#else
static NSArray<NSString *> *EzerbotFakeUnplugged;
+ (NSArray<NSString *> *)fakeUnplugged { return EzerbotFakeUnplugged ?: @[]; }
+ (void)setFakeUnplugged:(NSArray<NSString *> *)ids { EzerbotFakeUnplugged = [ids copy]; }
+ (NSArray<NSDictionary *> *)available {
    NSMutableArray *microphones = [NSMutableArray array];
    for (NSArray<NSString *> *fake in @[@[@"fake-built-in",@"Microphone intégré (test)"],@[@"fake-usb",@"Microphone USB (test)"]]) {
        if (![self.fakeUnplugged containsObject:fake[0]]) [microphones addObject:@{@"id":fake[0],@"name":fake[1]}];
    }
    return microphones;
}
+ (NSDictionary *)resolve:(NSString *)requested {
    NSArray<NSDictionary *> *microphones = [self available];
    NSDictionary *chosen = nil;
    for (NSDictionary *entry in microphones) if (requested.length && [entry[@"id"] isEqual:requested]) chosen = entry;
    NSDictionary *effective = chosen ?: microphones.firstObject;
    return @{@"id":effective[@"id"] ?: @"", @"name":effective[@"name"] ?: @"", @"fallback":@((BOOL)(requested.length && !chosen))};
}
#endif
@end
