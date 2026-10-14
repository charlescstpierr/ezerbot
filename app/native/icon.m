#import <Cocoa/Cocoa.h>

static void DrawIcon(CGFloat size) {
    CGFloat scale = size / 1024.0;
    NSAffineTransform *transform = [NSAffineTransform transform];
    [transform scaleBy:scale];
    [transform concat];
    NSBezierPath *base = [NSBezierPath bezierPathWithRoundedRect:NSMakeRect(52, 52, 920, 920) xRadius:214 yRadius:214];
    NSShadow *shadow = [[NSShadow alloc] init];
    shadow.shadowColor = [NSColor colorWithCalibratedRed:0.35 green:0.23 blue:0.17 alpha:0.22];
    shadow.shadowBlurRadius = 22 * scale;
    shadow.shadowOffset = NSMakeSize(0, -8 * scale);
    [NSGraphicsContext saveGraphicsState];
    [shadow set];
    [[NSColor colorWithCalibratedRed:239.0/255 green:228.0/255 blue:214.0/255 alpha:1] setFill];
    [base fill];
    [NSGraphicsContext restoreGraphicsState];
    NSGradient *background = [[NSGradient alloc]
        initWithStartingColor:[NSColor colorWithCalibratedRed:0.98 green:0.95 blue:0.90 alpha:1]
                  endingColor:[NSColor colorWithCalibratedRed:0.86 green:0.72 blue:0.63 alpha:1]];
    [background drawInBezierPath:base angle:315];
    [NSGraphicsContext saveGraphicsState];
    [base addClip];
    NSGradient *terracotta = [[NSGradient alloc]
        initWithStartingColor:[NSColor colorWithCalibratedRed:214.0/255 green:120.0/255 blue:83.0/255 alpha:0.95]
                  endingColor:[NSColor colorWithCalibratedRed:214.0/255 green:120.0/255 blue:83.0/255 alpha:0]];
    [terracotta drawFromCenter:NSMakePoint(840, 185) radius:0
                     toCenter:NSMakePoint(840, 185) radius:760
                      options:NSGradientDrawsBeforeStartingLocation | NSGradientDrawsAfterEndingLocation];
    [NSGraphicsContext restoreGraphicsState];
    base.lineWidth = 2;
    [[NSColor colorWithCalibratedRed:0.66 green:0.48 blue:0.36 alpha:0.25] setStroke];
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
    pebbleShadow.shadowColor = [NSColor colorWithCalibratedRed:0.35 green:0.22 blue:0.17 alpha:0.32];
    pebbleShadow.shadowBlurRadius = 44 * scale;
    pebbleShadow.shadowOffset = NSMakeSize(18 * scale, -36 * scale);
    [pebbleShadow set];
    [[NSColor colorWithWhite:0.94 alpha:1] setFill];
    [pebble fill];
    [NSGraphicsContext restoreGraphicsState];
    NSGradient *pearl = [[NSGradient alloc] initWithColors:@[
        [NSColor colorWithCalibratedRed:1 green:0.995 blue:0.985 alpha:1],
        [NSColor colorWithCalibratedRed:0.965 green:0.960 blue:0.952 alpha:1],
        [NSColor colorWithCalibratedRed:0.87 green:0.87 blue:0.87 alpha:1],
        [NSColor colorWithCalibratedRed:0.76 green:0.77 blue:0.79 alpha:1]
    ]];
    [pearl drawInBezierPath:pebble relativeCenterPosition:NSMakePoint(-0.48, 0.56)];
    pebble.lineWidth = 2;
    [[NSColor colorWithWhite:1 alpha:0.52] setStroke];
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
    [[NSColor colorWithCalibratedRed:0.12 green:0.13 blue:0.14 alpha:1] setStroke];
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
