// Sparkle is loaded only from the signed app bundle. Keep the bridge usable in
// developer/test binaries that do not bundle the framework.
#import <AppKit/AppKit.h>

// Public Sparkle 2 API declarations; no link-time dependency on Sparkle.
@interface MYPDFUpdaterAPI : NSObject
- (BOOL)startUpdater:(NSError **)error;
- (void)checkForUpdates;
- (void)checkForUpdatesInBackground;
@property BOOL automaticallyChecksForUpdates;
@end
@interface MYPDFControllerAPI : NSObject
- (instancetype)initWithStartingUpdater:(BOOL)start updaterDelegate:(id)delegate userDriverDelegate:(id)driver;
@property(readonly) MYPDFUpdaterAPI *updater;
@end

static MYPDFControllerAPI *controller;
static NSObject *delegate;
static NSString *lastError;
static void (^resumeInstallation)(void);
static BOOL quitRequested;
static void (*wakeUI)(void);

@interface MYPDFUpdateDelegate : NSObject
@end
@implementation MYPDFUpdateDelegate
- (BOOL)updater:(id)updater shouldPostponeRelaunchForUpdate:(id)item untilInvokingBlock:(void (^)(void))handler {
    // Route *every* immediate relaunch through MyPDF's normal save/quit flow.
    // Sparkle's install-on-quit path already waits for the app to terminate.
    resumeInstallation = [handler copy];
    quitRequested = YES;
    if (wakeUI) wakeUI();
    return YES;
}
@end

const char *mypdf_updater_check(void (*wake)(void), bool background) {
    NSCAssert([NSThread isMainThread], @"Updater must run on the UI thread");
    wakeUI = wake;
    if (resumeInstallation) {
        if (!background) { quitRequested = YES; if (wakeUI) wakeUI(); }
        return NULL;
    }
    if (!controller) {
        NSString *path = [[[NSBundle mainBundle] privateFrameworksPath] stringByAppendingPathComponent:@"Sparkle.framework"];
        NSError *error = nil;
        NSBundle *framework = path ? [NSBundle bundleWithPath:path] : nil;
        if (!framework || ![framework loadAndReturnError:&error]) {
            lastError = error.localizedDescription ?: @"The updater is missing. Install a complete MyPDF app bundle.";
            return lastError.UTF8String;
        }
        Class cls = NSClassFromString(@"SPUStandardUpdaterController");
        if (!cls) return "The bundled updater could not be loaded.";
        delegate = [MYPDFUpdateDelegate new];
        controller = [[cls alloc] initWithStartingUpdater:NO updaterDelegate:delegate userDriverDelegate:nil];
        // MyPDF's opt-in scheduler performs the GitHub discovery check. Avoid
        // two independent schedulers or prompting on an unpublished feed.
        controller.updater.automaticallyChecksForUpdates = NO;
        if (![controller.updater startUpdater:&error]) {
            lastError = error.localizedDescription;
            controller = nil;
            return lastError.UTF8String ?: "The updater could not start.";
        }
    }
    if (background) [controller.updater checkForUpdatesInBackground];
    else [controller.updater checkForUpdates];
    return NULL;
}

bool mypdf_updater_take_quit_request(void) {
    BOOL requested = quitRequested;
    quitRequested = NO;
    return requested;
}

bool mypdf_updater_resume_installation(void) {
    if (!resumeInstallation) return false;
    void (^handler)(void) = resumeInstallation;
    resumeInstallation = nil;
    quitRequested = NO;
    handler();
    return true;
}
