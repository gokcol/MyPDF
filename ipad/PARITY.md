# Desktop parity requirement

The intended iPad product has the desktop editor's functionality. The first installed iPad build is a device/input proof, not an accepted reduced product. Do not advertise or release it as equivalent to MyPDF desktop.

## Delivery gates

| Area | Current iPad preview | Required before feature-complete release | Shared Rust implementation |
| --- | --- | --- | --- |
| Read / documents | Files import, local copies, blank PDF, search, native zoom and paging | Tabs, thumbnail/outline panels, reading preferences, compare, accessible reading | `viewer`, `navigation`, `compare`, `home` |
| Edit | No existing-content editing UI | In-place new/existing text, area text editing, formatting/position, image replace/crop/transform, links, redaction | `content`, `text_block`, `text_geometry`, `rich_text`, `redact` |
| Annotate / Draw | Pencil/finger ink, images, move/resize/delete through Objects | Full shape/markup tools, paths/arrows, multiselect, live inspector, rotation/flip, full pen fidelity, direct page selection | `annotations`, `stroke`, `svg` |
| Stamps / assets | Reusable drawn signature and image import | Standard/custom library, SVG import/source edit/prompt workflow, recolor, folders/favorites and portable library import/export | `assets`, `svg` |
| Sign | Visual ink/image signatures | PKCS#12 import/create, identity defaults, iPad Keychain adapter, certificate validation/trust, signature appearances and signed-document workflows | `identity`, `signing`, `signature_edit`, `trust` |
| Forms | Reader displays existing PDF forms | Reliable edit/save, field creation and layout, detection, profiles and data import/export | `forms`, `form_design`, `form_detect`, `form_layout`, `form_profile`, `form_data` |
| Pages | Native thumbnail organizer; reorder, blank/PDF insertion, delete, extract, merge, rotate and crop through shared Rust commands | Split, headers/footers, watermark, N-up layouts, device interaction and accessibility validation | `pages`, `header_footer`, `watermark`, `nup` |
| Export | Save PDF copy to Files | All desktop image/text/MD/DOCX conversions, compression, password protection and native printing | `convert`, `compress`, `security` |
| Recovery / performance | Applied edits persist as new files; local library; session undo | Persisted recovery/undo policy, storage pruning, background memory limits, large/scanned document tests and accessibility audit | `recovery`, `annotations::Session` |
| OCR | Not exposed | Native Vision integration and desktop-equivalent invisible text layer | `ocr` plus iOS adapter |

Keep PDF operations in Rust. Audit reuse of the existing editor UI/command layer before expanding the limited SwiftUI shell into separate implementations of every desktop feature. Platform adapters must cover Files/security-scoped resources, clipboard, keyboard/IME, Apple Pencil and touch, fonts, sharing/printing, background lifecycle and iOS Keychain. iPad controls can adapt to touch, but the feature set and document behavior must be the same.

## Acceptance checks

Use the same synthetic PDFs on Mac, Windows and iPad. Each editing feature must save, reopen in another reader, remain editable where applicable, preserve unrelated PDF content, support undo/recovery, and retain source files. Test at portrait/landscape sizes, with keyboard/finger/Pencil, accessibility sizes, mixed page sizes/rotations, existing annotations and certificate-signed files. Device build success and engine unit tests alone do not establish UI parity.

## Current device result

Build 1 passed two native device tests but hands-on testing exposed that SwiftUI's automatic List-row button behavior could invoke both Edit and Delete. Build 2 uses separate borderless actions, requires delete confirmation, stops a failed editor preview from showing an endless spinner, and recovers retained drawing revisions into separate library entries. On 17 October 2026, the user confirmed on the physical iPad that a recovered drawing remains visible in Objects → Edit and moves with its selection border. This verifies that correction; the feature-parity delivery gates above remain open.

## Next parity increment

Build 3 adds a native page organizer backed by `pages::apply` and `pages::extract`, including safe fresh-file output and the same signed-document guards as desktop. It is development work, not a declaration of full parity. On 17 October 2026, all three native tests passed on the connected iPad, including a bridge regression that exercises drawing preservation through insertion, rotation, reordering, cropping, extraction and merging, plus failure without overwriting an existing file. The build was launched on the device; hands-on testing of the new page controls remains open.

The desktop UI currently lives in the application binary and is coupled to desktop menus, file dialogs and window lifecycle. The next increment reuses its Rust document operations through explicit bridge commands and adds native iPad touch controls; it does not duplicate PDF mutation logic in Swift. Existing-content editing and the shared stamp library are the next gaps after page organization. All delivery gates above still apply.
