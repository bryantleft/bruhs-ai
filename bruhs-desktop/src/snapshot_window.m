#import <AppKit/AppKit.h>
#import <ApplicationServices/ApplicationServices.h>
#import <QuartzCore/QuartzCore.h>

typedef struct {
    uint32_t windowID;
    int32_t ownerPID;
    double x, y, width, height;
} SnapshotTarget;

// Resolve the frontmost application's visible window only on a capture shortcut.
bool bruhs_front_snapshot_target(SnapshotTarget *target) {
    @autoreleasepool {
        pid_t pid = NSWorkspace.sharedWorkspace.frontmostApplication.processIdentifier;
        if (pid <= 0) return false;
        NSArray *windows = CFBridgingRelease(CGWindowListCopyWindowInfo(
            kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements, kCGNullWindowID));
        for (NSDictionary *window in windows) {
            if ([window[(id)kCGWindowOwnerPID] intValue] != pid ||
                [window[(id)kCGWindowLayer] intValue] != 0 ||
                [window[(id)kCGWindowAlpha] doubleValue] <= 0) continue;
            CGRect r;
            if (!CGRectMakeWithDictionaryRepresentation((__bridge CFDictionaryRef)window[(id)kCGWindowBounds], &r) ||
                r.size.width < 1 || r.size.height < 1) continue;
            *target = (SnapshotTarget){[window[(id)kCGWindowNumber] unsignedIntValue], pid,
                                       r.origin.x, r.origin.y, r.size.width, r.size.height};
            return true;
        }
        return false;
    }
}

static id attribute(AXUIElementRef el, CFStringRef name) {
    CFTypeRef result = NULL;
    if (AXUIElementCopyAttributeValue(el, name, &result) != kAXErrorSuccess) return nil;
    return CFBridgingRelease(result);
}

// Limit depth, node count and output. Secure text fields never contribute text.
static void collect_labels(AXUIElementRef el, NSMutableOrderedSet<NSString *> *lines,
                           unsigned depth, unsigned *visited, CFAbsoluteTime deadline) {
    if (CFAbsoluteTimeGetCurrent() > deadline || depth > 16 || *visited >= 250 || lines.count >= 150) return;
    (*visited)++;
    if ([attribute(el, kAXSubroleAttribute) isEqual:(__bridge NSString *)kAXSecureTextFieldSubrole]) return;
    for (NSString *name in @[(id)kAXTitleAttribute, (id)kAXDescriptionAttribute]) {
        id s = attribute(el, (__bridge CFStringRef)name);
        if ([s isKindOfClass:NSString.class] && [s length] > 0 && [s length] <= 500)
            [lines addObject:s];
    }
    if ([attribute(el,kAXRoleAttribute) isEqual:(id)kAXStaticTextRole]) {
        id text = attribute(el,kAXValueAttribute);
        if ([text isKindOfClass:NSString.class] && [text length] > 0 && [text length] <= 500)
            [lines addObject:text];
    }
    NSArray *children = attribute(el,kAXChildrenAttribute);
    if (![children isKindOfClass:NSArray.class]) return;
    for (id child in children) {
        if (*visited >= 250 || CFAbsoluteTimeGetCurrent() > deadline) break;
        if (CFGetTypeID((__bridge CFTypeRef)child) == AXUIElementGetTypeID())
            collect_labels((__bridge AXUIElementRef)child,lines,depth+1,visited,deadline);
    }
}

char *bruhs_snapshot_control_text(SnapshotTarget target) {
    @autoreleasepool {
        SnapshotTarget current;
        if (!AXIsProcessTrusted() || !bruhs_front_snapshot_target(&current) ||
            current.windowID != target.windowID || current.ownerPID != target.ownerPID) return NULL;
        AXUIElementRef app = AXUIElementCreateApplication(target.ownerPID);
        AXUIElementSetMessagingTimeout(app, 0.2);
        id focused = attribute(app, kAXFocusedWindowAttribute);
        CFRelease(app);
        if (!focused || CFGetTypeID((__bridge CFTypeRef)focused) != AXUIElementGetTypeID()) return NULL;
        AXUIElementRef window = (__bridge AXUIElementRef)focused;
        id pos = attribute(window,kAXPositionAttribute);
        id size = attribute(window,kAXSizeAttribute);
        CGPoint origin; CGSize extent;
        if (!pos || !size || CFGetTypeID((__bridge CFTypeRef)pos) != AXValueGetTypeID() ||
            CFGetTypeID((__bridge CFTypeRef)size) != AXValueGetTypeID() ||
            !AXValueGetValue((__bridge AXValueRef)pos,kAXValueCGPointType,&origin) ||
            !AXValueGetValue((__bridge AXValueRef)size,kAXValueCGSizeType,&extent) ||
            fabs(origin.x - target.x) > 1 || fabs(origin.y - target.y) > 1 ||
            fabs(extent.width - target.width) > 1 || fabs(extent.height - target.height) > 1) return NULL;
        NSMutableOrderedSet<NSString *> *lines = [NSMutableOrderedSet orderedSet];
        unsigned visited = 0;
        collect_labels(window,lines,0,&visited,CFAbsoluteTimeGetCurrent()+1);
        if (!bruhs_front_snapshot_target(&current) || current.windowID != target.windowID) return NULL;
        return strdup([lines.array componentsJoinedByString:@"\n"].UTF8String);
    }
}

// Feedback is nonactivating, ignores clicks and respects Reduce Motion.
void bruhs_snapshot_feedback(SnapshotTarget target, bool flash, bool animate, const uint8_t *bytes, size_t count) {
    @autoreleasepool {
        NSScreen *primary = NSScreen.screens.firstObject;
        if (!primary) return;
        NSRect source = NSMakeRect(target.x, NSMaxY(primary.frame)-target.y-target.height,target.width,target.height);
        NSWindow *destination = NSApp.mainWindow ?: NSApp.windows.firstObject;
        if (target.windowID == 0) source = destination.frame;
        if (NSIsEmptyRect(source)) return;
        if (flash) {
            NSPanel *cue = [[NSPanel alloc] initWithContentRect:source styleMask:NSWindowStyleMaskBorderless|NSWindowStyleMaskNonactivatingPanel backing:NSBackingStoreBuffered defer:NO];
            cue.opaque = NO;
            cue.backgroundColor = NSColor.clearColor;
            cue.level = NSFloatingWindowLevel;
            cue.ignoresMouseEvents = YES;
            cue.contentView.wantsLayer = YES;
            cue.contentView.layer.borderWidth = 3;
            cue.contentView.layer.cornerRadius = 9;
            cue.contentView.layer.borderColor = [NSColor colorWithRed:.98 green:.61 blue:.36 alpha:.8].CGColor;
            [cue orderFrontRegardless];
            dispatch_after(dispatch_time(DISPATCH_TIME_NOW,180*NSEC_PER_MSEC),dispatch_get_main_queue(),^{ [cue orderOut:nil]; });
        }
        if (!animate || !destination || NSWorkspace.sharedWorkspace.accessibilityDisplayShouldReduceMotion || !bytes || !count) return;
        NSImage *img = [[NSImage alloc] initWithData:[NSData dataWithBytes:bytes length:count]];
        if (!img || img.size.width < 1) return;
        CGFloat w = 135, h = MIN(120,MAX(35,w * img.size.height / img.size.width));
        NSRect start = NSMakeRect(NSMidX(source)-w/2,NSMidY(source)-h/2,w,h);
        NSPanel *thumb = [[NSPanel alloc] initWithContentRect:start styleMask:NSWindowStyleMaskBorderless|NSWindowStyleMaskNonactivatingPanel backing:NSBackingStoreBuffered defer:NO];
        thumb.level = NSFloatingWindowLevel;
        thumb.opaque = NO;
        thumb.backgroundColor = NSColor.clearColor;
        thumb.ignoresMouseEvents = YES;
        NSImageView *view = [NSImageView imageViewWithImage:img];
        view.imageScaling = NSImageScaleProportionallyUpOrDown;
        thumb.contentView = view;
        [thumb orderFrontRegardless];
        NSRect dest = destination.frame;
        NSRect end = NSMakeRect(NSMidX(dest)-w/4,NSMinY(dest)+65,w/2,h/2);
        [NSAnimationContext runAnimationGroup:^(NSAnimationContext *ctx) {
            ctx.duration = .3;
            [[thumb animator] setFrame:end display:YES];
            [[thumb animator] setAlphaValue:0];
        } completionHandler:^{ [thumb orderOut:nil]; }];
    }
}
