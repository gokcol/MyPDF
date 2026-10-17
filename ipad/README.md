# MyPDF for iPad — development preview

This is a standalone iPad application. SwiftUI, PDFKit and PencilKit provide native iPad controls; PDF changes use the shared Rust engine through a small C/JSON bridge. There is no web view and no connection to a running desktop app.

Implemented in the first device build:

- Local document library, blank A4 PDF, Files import, text search, native pinch zoom and page navigation.
- Pencil/finger drawing and drawn signatures saved as editable PDF ink objects. Finger drawing is an explicit toggle; Pencil-only input uses PencilKit's input handling.
- Reusable drawn signature, PNG/JPEG signature or image import, white-background removal with a preview and adjustable tolerance.
- Objects list, live move/resize with the selection border attached, deletion, undo/redo, and export a PDF copy to Files.
- Every applied edit writes a new working revision before updating the local library. Sources opened through Files are copied and never edited in place. Library entries survive app termination. Undo history is currently session-only; working revisions are retained locally.

## Limits of this preview

This is the first iPad build, not desktop feature parity. Existing PDF text/image editing, forms, page management, certificate signing, custom stamp synchronization and desktop conversions are not exposed yet. Password-protected PDFs must be unlocked elsewhere. Signed/certified PDFs remain read-only unless previously explicitly unlocked by MyPDF desktop.

PencilKit captures the live drawing. Export currently stores centerlines with a uniform stroke width and one color per drawing, rather than reproducing pressure/tilt and mixed brush effects. Keep one pen color and width per drawing for the closest match. Add separate drawing objects for different styles. The iPad object editor moves/resizes unrotated MyPDF ink and image objects; other annotation types remain in the PDF but are not editable there. Background removal targets white/solid paper, not arbitrary photographic subjects.

The iPad library is separate from the Mac's settings and stamps. Share exported PDFs through Files/AirDrop/iCloud Drive. The application has no account, analytics or app-controlled network transfer. If you choose a cloud location in Files, that service controls synchronization.

## Build and install

Requires full Xcode, XcodeGen (`brew install xcodegen`), Rust's `aarch64-apple-ios` target, an Apple Development signing identity, and a paired iPad with Developer Mode enabled. Run:

```sh
rustup target add aarch64-apple-ios
MYPDF_IOS_TEAM=YOUR_TEAM_ID sh ipad/build-device.sh
xcrun devicectl list devices
xcrun devicectl device install app --device YOUR_DEVICE_ID ipad/build/Build/Products/Debug-iphoneos/MyPDFiPad.app
xcrun devicectl device process launch --device YOUR_DEVICE_ID org.mypdf.ipad.dev
```

`ipad/project.yml` is the source of truth; XcodeGen recreates the ignored Xcode project. The Rust library must use `IPHONEOS_DEPLOYMENT_TARGET=17.0`, including its C dependencies. The build script sets this. Override `DEVELOPMENT_TEAM` for your own account; no credentials or provisioning profiles are committed.

Run device tests with the iPad unlocked:

```sh
xcodebuild test -project ipad/MyPDFiPad.xcodeproj -scheme MyPDFiPad \
  -destination 'id=YOUR_DEVICE_ID' -derivedDataPath ipad/build \
  DEVELOPMENT_TEAM=YOUR_TEAM_ID -allowProvisioningUpdates
```

On 17 October 2026, the app was development-signed, installed and launched on an iPad Air 13-inch (M2). Both native tests passed on the device: Pencil canvas coordinate stability through resizing, and Rust blank PDF → ink → move → transparent image → saved PDF roundtrip, including refusing to overwrite an existing output. Physical Pencil feel and visual layout require hands-on testing; automated tests do not establish those.
