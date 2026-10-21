// Pure PCM segmentation and progress ledger, shared by release Speech and debug
// fixtures. No device, permission, filesystem or Speech API is reachable here.
#import <Foundation/Foundation.h>
#include <stdint.h>
#include <string.h>

@interface EzerbotMeetingTranscription : NSObject
@property(nonatomic, strong, readonly) NSData *pcm;
@property(nonatomic, readonly) NSUInteger currentIndex;
@property(nonatomic, readonly) NSUInteger count;
@property(nonatomic, readonly) NSUInteger missingCount;
@property(nonatomic, readonly) BOOL hasText;
@property(nonatomic, copy, readonly) NSString *transcript;
+ (NSData *)waveFromPCM:(NSData *)pcm;
- (instancetype)initWithAudio:(NSData *)audio;
- (NSData *)nextAudio;
- (void)completeWithText:(NSString *)text;
- (void)retryMissing;
@end

@interface EzerbotMeetingTranscription ()
@property(nonatomic, strong, readwrite) NSData *pcm;
@property(nonatomic, readwrite) NSUInteger currentIndex;
@property(nonatomic, strong) NSMutableArray<NSString *> *texts;
@property(nonatomic, strong) NSMutableIndexSet *pending;
@property(nonatomic, strong) NSMutableIndexSet *missing;
@end

@implementation EzerbotMeetingTranscription
+ (NSData *)waveFromPCM:(NSData *)pcm {
    if (!pcm.length || pcm.length % 2 || pcm.length > 601 * 32000) return nil;
    NSMutableData *wav = [NSMutableData dataWithLength:44];
    uint8_t *header = wav.mutableBytes;
    memcpy(header,"RIFF",4); uint32_t size=(uint32_t)pcm.length+36; memcpy(header+4,&size,4);
    memcpy(header+8,"WAVEfmt ",8); uint32_t fmt=16,rate=16000,byteRate=32000; uint16_t one=1,two=2,bits=16;
    memcpy(header+16,&fmt,4); memcpy(header+20,&one,2); memcpy(header+22,&one,2); memcpy(header+24,&rate,4);
    memcpy(header+28,&byteRate,4); memcpy(header+32,&two,2); memcpy(header+34,&bits,2); memcpy(header+36,"data",4);
    uint32_t length=(uint32_t)pcm.length; memcpy(header+40,&length,4);
    [wav appendData:pcm]; return wav;
}
- (instancetype)initWithAudio:(NSData *)audio {
    self = [super init];
    if (!self || audio.length < 44 || audio.length > 601*32000+4096) return nil;
    const uint8_t *bytes = audio.bytes;
    uint32_t (^u32)(NSUInteger) = ^uint32_t(NSUInteger p) { return (uint32_t)bytes[p] | (uint32_t)bytes[p+1]<<8 | (uint32_t)bytes[p+2]<<16 | (uint32_t)bytes[p+3]<<24; };
    uint16_t (^u16)(NSUInteger) = ^uint16_t(NSUInteger p) { return (uint16_t)(bytes[p] | (uint16_t)bytes[p+1]<<8); };
    if (memcmp(bytes,"RIFF",4) || memcmp(bytes+8,"WAVE",4) || (NSUInteger)u32(4)+8 != audio.length) return nil;
    BOOL format = NO; NSUInteger cursor = 12;
    while (cursor + 8 <= audio.length) {
        NSUInteger length = u32(cursor+4), start = cursor+8;
        if (length > audio.length-start) return nil;
        if (!memcmp(bytes+cursor,"fmt ",4)) {
            if (format || length < 16 || u16(start)!=1 || u16(start+2)!=1 || u32(start+4)!=16000 || u32(start+8)!=32000 || u16(start+12)!=2 || u16(start+14)!=16) return nil;
            format = YES;
        } else if (!memcmp(bytes+cursor,"data",4)) {
            if (_pcm || !length || length%2 || length>601*32000) return nil;
            _pcm = [audio subdataWithRange:NSMakeRange(start,length)];
        }
        cursor = start + length + length%2;
    }
    if (!format || !_pcm.length || cursor != audio.length) return nil;
    _texts = [NSMutableArray array];
    NSUInteger count = (_pcm.length + 60*32000 - 1)/(60*32000);
    for (NSUInteger i=0; i<count; i++) [_texts addObject:@""];
    _pending = [NSMutableIndexSet indexSetWithIndexesInRange:NSMakeRange(0,count)];
    _missing = [NSMutableIndexSet indexSet]; _currentIndex = NSNotFound;
    return self;
}
- (NSUInteger)count { return self.texts.count; }
- (NSUInteger)missingCount { return self.missing.count; }
- (BOOL)hasText {
    for (NSString *text in self.texts) if (text.length) return YES;
    return NO;
}
- (NSData *)nextAudio {
    NSUInteger index = self.pending.firstIndex;
    if (index == NSNotFound) { self.currentIndex = NSNotFound; return nil; }
    self.currentIndex = index;
    NSUInteger start = index*60*32000, length = MIN((NSUInteger)60*32000,self.pcm.length-start);
    return [EzerbotMeetingTranscription waveFromPCM:[self.pcm subdataWithRange:NSMakeRange(start,length)]];
}
- (void)completeWithText:(NSString *)text {
    NSUInteger index = self.currentIndex;
    if (index == NSNotFound || ![self.pending containsIndex:index]) return;
    NSString *clean = [text stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet] ?: @"";
    self.texts[index] = clean;
    [self.pending removeIndex:index];
    if (clean.length) [self.missing removeIndex:index]; else [self.missing addIndex:index];
    self.currentIndex = NSNotFound;
}
- (void)retryMissing {
    // Confirmed text is immutable across retries; only missing indexes re-enter
    // the queue. No rewind to byte zero and no silent loss of acquired segments.
    if (self.currentIndex == NSNotFound) [self.pending addIndexes:self.missing];
}
- (NSString *)transcript {
    if (!self.hasText) return @"";
    NSMutableArray<NSString *> *parts = [NSMutableArray array];
    for (NSUInteger i=0; i<self.texts.count; i++) {
        NSString *text = self.texts[i];
        if (!text.length) {
            NSUInteger start = i*60, end = MIN((i+1)*60,self.pcm.length/32000);
            text = [NSString stringWithFormat:@"[Passage non transcrit de %lu:%02lu à %lu:%02lu]",(unsigned long)(start/60),(unsigned long)(start%60),(unsigned long)(end/60),(unsigned long)(end%60)];
        }
        [parts addObject:text];
    }
    return [parts componentsJoinedByString:@"\n"];
}
@end
