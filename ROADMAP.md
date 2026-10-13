# MyPDF implementation milestones

The implementation follows the original plan agreed in this chat. The application remains native Rust, macOS first, with a portable Windows core and no WebViews.

| Milestone | Scope | Status |
|---|---|---|
| 0 | PDF rendering and real text-replacement proof; certificate creation, PKCS#12 and native Keychain signing | Completed with documented compatibility limits |
| 1 | Native viewer, tabs, thumbnails, navigation, search, zoom and safe file copies | Completed on macOS; Windows execution gate remains |
| 2 | Digital identity manager; personal IDs; PKCS#12 import/export; Keychain identity selection; signature appearance and validation | Completed on macOS with documented offline-signing limits; Windows execution gate remains |
| 3 | Annotation tools; handwritten signatures; icons, symbols and stamps; reusable asset library; undo/redo | Completed on macOS; Windows execution and broader corpus gates remain |
| 4 | Reorder, insert, delete, extract, merge, split, rotate and crop pages; PDF Expert-inspired panels and artwork | Completed on macOS with documented form/catalog limits; Windows execution gate remains |
| 5 | Existing-text/image editing, forms, local image/text/Markdown/Word conversions, editable SVG assets and blank documents | Completed on macOS with documented compatibility limits; Windows execution gate remains |
| 6 | Recovery, autosave, performance, accessibility and native UI polish | Implemented and verified on macOS; Windows build deferred by request; public distribution signing/notarization remains separate release work |

Milestones 2 and 3 retain the proven save boundaries: fresh output paths, incremental certificate signatures preserving previous signed bytes, and ordinary editing disabled for signed/certified PDFs. Annotations carry standard PDF appearance streams for other readers. Unsupported signing inputs and annotation structures receive explicit explanations; no silent signature removal or trust-root changes are allowed. See MILESTONE2_RESULTS.md and MILESTONE3_RESULTS.md for evidence and remaining limits.
