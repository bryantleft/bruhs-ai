// Recognize only the pixels the user selected and accepted in the system picker.
#import <Foundation/Foundation.h>
#import <Vision/Vision.h>

char *bruhs_snapshot_visible_text(const char *path) {
    @autoreleasepool {
        if (@available(macOS 10.15, *)) {
            NSURL *url = [NSURL fileURLWithPath:[NSString stringWithUTF8String:path]];
            VNImageRequestHandler *handler = [[VNImageRequestHandler alloc] initWithURL:url options:@{}];
            VNRecognizeTextRequest *request = [[VNRecognizeTextRequest alloc] init];
            request.recognitionLevel = VNRequestTextRecognitionLevelAccurate;
            if (![handler performRequests:@[request] error:nil]) return NULL;
            NSMutableArray<NSString *> *lines = [NSMutableArray array];
            for (VNRecognizedTextObservation *observation in request.results) {
                NSString *line = [observation topCandidates:1].firstObject.string;
                if (line.length) [lines addObject:line];
            }
            return strdup([lines componentsJoinedByString:@"\n"].UTF8String);
        }
        return NULL;
    }
}

void bruhs_free_snapshot_text(char *value) { free(value); }
