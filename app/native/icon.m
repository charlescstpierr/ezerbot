#import <Cocoa/Cocoa.h>

static void DrawIcon(CGFloat size) {
    CGFloat scale = size / 1024.0;
    NSAffineTransform *transform = [NSAffineTransform transform];
    [transform scaleBy:scale];
    [transform concat];
    NSBezierPath *base = [NSBezierPath bezierPathWithRoundedRect:NSMakeRect(52, 52, 920, 920) xRadius:214 yRadius:214];
    NSShadow *shadow = [[NSShadow alloc] init];
    shadow.shadowColor = [NSColor colorWithCalibratedWhite:0 alpha:0.3];
    shadow.shadowBlurRadius = 22 * scale;
    shadow.shadowOffset = NSMakeSize(0, -8 * scale);
    [NSGraphicsContext saveGraphicsState];
    [shadow set];
    [[NSColor colorWithCalibratedRed:20.0/255 green:22.0/255 blue:26.0/255 alpha:1] setFill];
    [base fill];
    [NSGraphicsContext restoreGraphicsState];
    NSGradient *background = [[NSGradient alloc]
        initWithStartingColor:[NSColor colorWithCalibratedRed:27.0/255 green:30.0/255 blue:36.0/255 alpha:1]
                  endingColor:[NSColor colorWithCalibratedRed:10.0/255 green:11.0/255 blue:13.0/255 alpha:1]];
    [background drawInBezierPath:base angle:315];
    base.lineWidth = 2;
    [[NSColor colorWithCalibratedWhite:1 alpha:0.09] setStroke];
    [base stroke];

    // A new vector drawing: an asymmetric pearl, softly lit from the upper left.
    NSBezierPath *pebble = [NSBezierPath bezierPath];
    [pebble moveToPoint:NSMakePoint(196, 389)];
    [pebble curveToPoint:NSMakePoint(228, 677) controlPoint1:NSMakePoint(177, 499) controlPoint2:NSMakePoint(190, 602)];
    [pebble curveToPoint:NSMakePoint(501, 819) controlPoint1:NSMakePoint(281, 778) controlPoint2:NSMakePoint(390, 826)];
    [pebble curveToPoint:NSMakePoint(756, 717) controlPoint1:NSMakePoint(611, 821) controlPoint2:NSMakePoint(705, 777)];
    [pebble curveToPoint:NSMakePoint(839, 497) controlPoint1:NSMakePoint(813, 651) controlPoint2:NSMakePoint(844, 577)];
    [pebble curveToPoint:NSMakePoint(767, 294) controlPoint1:NSMakePoint(839, 406) controlPoint2:NSMakePoint(814, 337)];
    [pebble curveToPoint:NSMakePoint(549, 213) controlPoint1:NSMakePoint(716, 239) controlPoint2:NSMakePoint(632, 215)];
    [pebble curveToPoint:NSMakePoint(328, 237) controlPoint1:NSMakePoint(466, 197) controlPoint2:NSMakePoint(384, 207)];
    [pebble curveToPoint:NSMakePoint(196, 389) controlPoint1:NSMakePoint(248, 264) controlPoint2:NSMakePoint(206, 316)];
    [pebble closePath];
    [NSGraphicsContext saveGraphicsState];
    NSShadow *pebbleShadow = [[NSShadow alloc] init];
    pebbleShadow.shadowColor = [NSColor colorWithCalibratedWhite:0 alpha:0.5];
    pebbleShadow.shadowBlurRadius = 44 * scale;
    pebbleShadow.shadowOffset = NSMakeSize(18 * scale, -36 * scale);
    [pebbleShadow set];
    [[NSColor colorWithCalibratedRed:0.25 green:0.27 blue:0.31 alpha:1] setFill];
    [pebble fill];
    [NSGraphicsContext restoreGraphicsState];
    NSGradient *pearl = [[NSGradient alloc] initWithColors:@[
        [NSColor colorWithCalibratedRed:0.40 green:0.43 blue:0.49 alpha:1],
        [NSColor colorWithCalibratedRed:0.29 green:0.31 blue:0.36 alpha:1],
        [NSColor colorWithCalibratedRed:0.22 green:0.24 blue:0.28 alpha:1],
        [NSColor colorWithCalibratedRed:0.16 green:0.17 blue:0.21 alpha:1]
    ]];
    [pearl drawInBezierPath:pebble relativeCenterPosition:NSMakePoint(-0.48, 0.56)];
    pebble.lineWidth = 2;
    [[NSColor colorWithWhite:1 alpha:0.16] setStroke];
    [pebble stroke];

    NSBezierPath *leftEye = [NSBezierPath bezierPath];
    [leftEye moveToPoint:NSMakePoint(393, 565)];
    [leftEye lineToPoint:NSMakePoint(438, 489)];
    leftEye.lineWidth = 74;
    leftEye.lineCapStyle = NSLineCapStyleRound;
    NSBezierPath *rightEye = [NSBezierPath bezierPath];
    [rightEye moveToPoint:NSMakePoint(580, 640)];
    [rightEye lineToPoint:NSMakePoint(625, 564)];
    rightEye.lineWidth = 62;
    rightEye.lineCapStyle = NSLineCapStyleRound;
    [[NSColor colorWithCalibratedRed:230.0/255 green:1 blue:61.0/255 alpha:1] setStroke];
    [leftEye stroke];
    [rightEye stroke];
}

int main(int argc, const char *argv[]) {
    @autoreleasepool {
        if (argc != 2) return 1;
        NSString *directory = [NSString stringWithUTF8String:argv[1]];
        NSArray<NSNumber *> *sizes = @[@16, @32, @128, @256, @512];
        for (NSNumber *number in sizes) {
            NSUInteger size = number.unsignedIntegerValue;
            for (NSUInteger factor = 1; factor <= 2; factor++) {
                NSUInteger pixels = size * factor;
                NSBitmapImageRep *bitmap = [[NSBitmapImageRep alloc]
                    initWithBitmapDataPlanes:NULL pixelsWide:pixels pixelsHigh:pixels bitsPerSample:8 samplesPerPixel:4
                    hasAlpha:YES isPlanar:NO colorSpaceName:NSDeviceRGBColorSpace bytesPerRow:0 bitsPerPixel:0];
                [NSGraphicsContext saveGraphicsState];
                [NSGraphicsContext setCurrentContext:[NSGraphicsContext graphicsContextWithBitmapImageRep:bitmap]];
                DrawIcon(pixels);
                [NSGraphicsContext restoreGraphicsState];
                NSString *name = [NSString stringWithFormat:@"icon_%lux%lu%@.png", (unsigned long)size,
                                  (unsigned long)size, factor == 2 ? @"@2x" : @""];
                NSData *png = [bitmap representationUsingType:NSBitmapImageFileTypePNG properties:@{}];
                if (![png writeToFile:[directory stringByAppendingPathComponent:name] atomically:YES]) return 1;
            }
        }
    }
    return 0;
}
