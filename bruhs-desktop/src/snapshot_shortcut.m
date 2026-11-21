// Register exactly one requested chord with the system event dispatcher.
#import <Carbon/Carbon.h>
#import <Foundation/Foundation.h>

typedef struct {
    EventHandlerRef handler;
    EventHotKeyRef shortcut;
    EventHotKeyID id;
    void (*notify)(void *);
    void *context;
} SnapshotShortcut;

static OSStatus pressed(EventHandlerCallRef next, EventRef event, void *context) {
    (void)next;
    SnapshotShortcut *s = context;
    EventHotKeyID incoming = {0};
    if (GetEventParameter(event, kEventParamDirectObject, typeEventHotKeyID, NULL,
                          sizeof(incoming), NULL, &incoming) != noErr ||
        incoming.signature != s->id.signature || incoming.id != s->id.id)
        return eventNotHandledErr;
    s->notify(s->context);
    return noErr;
}

static int key_code(NSString *key) {
    NSDictionary *special = @{
        @"space":@(kVK_Space), @"tab":@(kVK_Tab), @"backspace":@(kVK_Delete),
        @"delete":@(kVK_ForwardDelete), @"enter":@(kVK_Return), @"escape":@(kVK_Escape),
        @"left":@(kVK_LeftArrow), @"right":@(kVK_RightArrow), @"up":@(kVK_UpArrow), @"down":@(kVK_DownArrow),
        @"pageup":@(kVK_PageUp), @"pagedown":@(kVK_PageDown), @"home":@(kVK_Home), @"end":@(kVK_End),
        @"f1":@(kVK_F1), @"f2":@(kVK_F2), @"f3":@(kVK_F3), @"f4":@(kVK_F4), @"f5":@(kVK_F5),
        @"f6":@(kVK_F6), @"f7":@(kVK_F7), @"f8":@(kVK_F8), @"f9":@(kVK_F9), @"f10":@(kVK_F10),
        @"f11":@(kVK_F11), @"f12":@(kVK_F12), @"f13":@(kVK_F13), @"f14":@(kVK_F14), @"f15":@(kVK_F15),
        @"f16":@(kVK_F16), @"f17":@(kVK_F17), @"f18":@(kVK_F18), @"f19":@(kVK_F19), @"f20":@(kVK_F20)
    };
    if (special[key]) return [special[key] intValue];
    if (key.length != 1) return -1;
    TISInputSourceRef source = TISCopyCurrentASCIICapableKeyboardLayoutInputSource();
    if (!source) return -1;
    CFDataRef data = TISGetInputSourceProperty(source, kTISPropertyUnicodeKeyLayoutData);
    int result = -1;
    for (UInt16 code = 0; data && code < 128; code++) {
        UInt32 state = 0;
        UniChar output[8];
        UniCharCount count = 0;
        OSStatus status = UCKeyTranslate((const UCKeyboardLayout *)CFDataGetBytePtr(data), code,
            kUCKeyActionDisplay, 0, LMGetKbdType(), kUCKeyTranslateNoDeadKeysMask,
            &state, 8, &count, output);
        if (status == noErr && count == 1 && output[0] == [key characterAtIndex:0]) {
            result = code;
            break;
        }
    }
    CFRelease(source);
    return result;
}

void bruhs_release_snapshot_shortcut(void *handle) {
    SnapshotShortcut *s = handle;
    if (s->shortcut) UnregisterEventHotKey(s->shortcut);
    if (s->handler) RemoveEventHandler(s->handler);
    free(s);
}

void *bruhs_register_snapshot_shortcut(const char *key, unsigned int flags,
                                        void (*notify)(void *), void *context) {
    @autoreleasepool {
        int code = key_code([NSString stringWithUTF8String:key]);
        if (code < 0) return NULL;
        SnapshotShortcut *s = calloc(1, sizeof(*s));
        if (!s) return NULL;
        static UInt32 nextID = 1; // registrations and callbacks run on the main thread
        s->id = (EventHotKeyID){0x42525353, nextID++};
        s->notify = notify;
        s->context = context;
        UInt32 mods = (flags & 1 ? cmdKey : 0) | (flags & 2 ? controlKey : 0) |
                      (flags & 4 ? optionKey : 0) | (flags & 8 ? shiftKey : 0);
        EventTypeSpec type = {kEventClassKeyboard, kEventHotKeyPressed};
        EventTargetRef target = GetApplicationEventTarget();
        if (InstallEventHandler(target, pressed, 1, &type, s, &s->handler) != noErr ||
            RegisterEventHotKey(code, mods, s->id, target, 0, &s->shortcut) != noErr) {
            bruhs_release_snapshot_shortcut(s);
            return NULL;
        }
        return s;
    }
}
