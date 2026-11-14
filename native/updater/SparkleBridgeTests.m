#import "SparkleBridge.m"
#include <assert.h>

int main(void) {
    @autoreleasepool {
        MYPDFUpdateDelegate *d = [MYPDFUpdateDelegate new];
        __block int installed = 0;
        assert([d updater:nil shouldPostponeRelaunchForUpdate:nil untilInvokingBlock:^{ installed++; }]);
        assert(installed == 0);
        assert(mypdf_updater_take_quit_request());
        assert(!mypdf_updater_take_quit_request());
        // Cancelling MyPDF's save/quit prompt leaves the install postponed.
        assert(installed == 0);
        // A subsequent explicit update action requests save/quit again.
        assert(mypdf_updater_check(NULL, true) == NULL);
        assert(!mypdf_updater_take_quit_request());
        assert(mypdf_updater_check(NULL, false) == NULL);
        assert(mypdf_updater_take_quit_request());
        assert(mypdf_updater_resume_installation());
        assert(installed == 1);
        assert(!mypdf_updater_resume_installation());
        assert(installed == 1);
    }
}
