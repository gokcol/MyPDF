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
| Pages | Viewer navigation only | Insert/reorder/delete/extract/merge/split/rotate/crop, headers/footers, watermark and N-up layouts | `pages`, `header_footer`, `watermark`, `nup` |
| Export | Save PDF copy to Files | All desktop image/text/MD/DOCX conversions, compression, password protection and native printing | `convert`, `compress`, `security` |
| Recovery / performance | Applied edits persist as new files; local library; session undo | Persisted recovery/undo policy, storage pruning, background memory limits, large/scanned document tests and accessibility audit | `recovery`, `annotations::Session` |
| OCR | Not exposed | Native Vision integration and desktop-equivalent invisible text layer | `ocr` plus iOS adapter |

Keep PDF operations in Rust. Audit reuse of the existing editor UI/command layer before expanding the limited SwiftUI shell into separate implementations of every desktop feature. Platform adapters must cover Files/security-scoped resources, clipboard, keyboard/IME, Apple Pencil and touch, fonts, sharing/printing, background lifecycle and iOS Keychain. iPad controls can adapt to touch, but the feature set and document behavior must be the same.

## Acceptance checks

Use the same synthetic PDFs on Mac, Windows and iPad. Each editing feature must save, reopen in another reader, remain editable where applicable, preserve unrelated PDF content, support undo/recovery, and retain source files. Test at portrait/landscape sizes, with keyboard/finger/Pencil, accessibility sizes, mixed page sizes/rotations, existing annotations and certificate-signed files. Device build success and engine unit tests alone do not establish UI parity.

## Current device result

Build 1 passed two native device tests but hands-on testing exposed that SwiftUI's automatic List-row button behavior could invoke both Edit and Delete. Build 2 uses separate borderless actions, requires delete confirmation, stops a failed editor preview from showing an endless spinner, and recovers retained drawing revisions into separate library entries. Physical editing validation is still required after installing that correction.
