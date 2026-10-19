// Pure debug permit test. No NSApplication, screen capture, TCC or input posting.
#import <Cocoa/Cocoa.h>
#import <unistd.h>
#import "screen.h"
#include <assert.h>
int main(int argc, const char *argv[]) {
    @autoreleasepool {
        // Keep the production entry compiled/linked for symbol exclusion checks,
        // but never execute it as part of this pure test.
        if (argc > 1) return EZScreenMain(argc, argv);
        CGPoint point;
        assert(EZImagePoint(1000,700,1280,832,CGRectMake(0,0,1470,956),&point));
        assert(fabs(point.x-1148.4375)<0.001); assert(fabs(point.y-804.326923)<0.001);
        assert(EZImagePoint(640,400,1280,800,CGRectMake(100,50,2560,1600),&point));
        assert(point.x==1380 && point.y==850);
        assert(!EZImagePoint(1280,0,1280,800,CGRectMake(0,0,1470,956),&point));
        assert(!EZImagePoint(-1,0,1280,800,CGRectMake(0,0,1470,956),&point));
        assert(!EZImagePoint(0,0,0,0,CGRectZero,&point));
        ezDeadline = NSProcessInfo.processInfo.systemUptime + 60;
        assert(EZLive(ezGeneration));
        uint64_t queued = ezGeneration;
        EZStop(YES);
        assert(!EZLive(queued)); assert(ezPaused); assert(!ezRevoked);
        pthread_mutex_lock(&ezPermit); ezPaused = NO; ++ezGeneration; pthread_mutex_unlock(&ezPermit);
        assert(EZLive(ezGeneration)); assert(!EZLive(queued));
        NSDictionary *fixture = EZCapture();
        assert([fixture[@"fixture"] boolValue]); assert([fixture[@"image"] hasPrefix:@"data:image/png;base64,"]);
        EZAction(@{@"id":@1,@"name":@"computer_click",@"arguments":@{@"x":@10,@"y":@20,@"button":@"left"}},queued);
        EZStop(NO); assert(ezRevoked); assert(!EZLive(ezGeneration));
        EZAction(@{@"id":@2,@"name":@"computer_type",@"arguments":@{@"text":@"jamais injecté"}},ezGeneration);
        ezRevoked = NO; ezDeadline = NSProcessInfo.processInfo.systemUptime - 1;
        assert(!EZLive(ezGeneration));
    }
    return 0;
}
