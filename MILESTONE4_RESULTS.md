# Milestone 4 — native page organizer and interface redesign

Version 0.4.0, verified on macOS on 13 October 2026. The app remains native Rust/egui with Metal rendering and native file dialogs. No WebViews or browser runtime were introduced. Windows-compatible core/UI code remains in place; Windows execution has not been performed on this Mac.

## Delivered

- MyPDF uses dark document tabs, compact grouped controls, a light annotation ribbon, fixed right-hand asset panels, and a page overview. The Mac titlebar integrates with the tab strip while retaining native window controls.
- 33 rebuilt vector assets: 15 arrows/chevrons, 14 symbols, and 4 stamps. Panels support searching, artwork import, saved signatures, and repeated placement. Filled vector artwork exports as a compound path so external readers do not show internal triangulation seams. Imported/personal assets remain untouched.
- Reorder pages by dragging or a numeric destination; multi-select with ⌘/Ctrl/Shift or explicit ranges.
- Insert a blank page matching the displayed current page size, or insert pages from one or more PDFs at a chosen position. Merge appends PDFs in file-picker order.
- Delete selected pages, retaining at least one. Extract selected pages to a new PDF. Split the complete document every N pages into a fresh output directory.
- Rotate clockwise/counterclockwise. Crop with four displayed-page margins, a shaded live preview, and reset to MediaBox. Existing PDF content is retained; crop is not redaction.
- Page edits and annotations share a revision-based Undo/Redo history. Managed artwork remains editable after page reordering, importing, rotation and crop. Source snapshots are shared; large history is bounded.
- Parsing and page operations run on background workers. Completion is tied to the originating tab and revision; a stale operation cannot replace newer edits. Search and raster caches reload after page changes or page Undo/Redo.
- Safe saving retains the existing no-overwrite publication boundary. Extraction leaves the current document unchanged. Split output is staged and published to a newly created directory; existing paths are preserved.

## Verification

45 regular tests pass: 6 desktop/event-loop tests, 9 annotation/asset tests, 11 core/signing tests, 3 digital-identity/signature tests, 6 page-operation tests, and 10 viewer tests. The two ignored visual/proof tests were run separately. All-target Clippy with warnings denied and formatting checks pass.

Page tests cover inherited resources/rotation, original content, bookmark targets after reorder, imported duplicate annotation IDs, extraction/deletion, invalid ranges and crop geometry, all four rotations, combined history, protection of signed/certified/encrypted/form PDFs, and no-overwrite saving. A desktop regression reproduces a press/move/release delivered in one frame: page dragging now handles this native event pattern, reloads search, supports Undo/Redo, and survives save/reopen.

Live Mac UI verification used only synthetic PDFs:

1. Dragged page 1 to the end; rotated and cropped the first resulting page.
2. Inserted and deleted a blank page; confirmed Undo restored four pages and Redo returned to three.
3. Merged a three-page PDF, producing six pages.
4. Placed the rebuilt blue arrow and gold star from their panels. Resized the arrow from 180 × 94.74 points to 236.48 × 126.35 points. The star’s changed rotation/opacity were retained when saving.
5. Saved to `artifacts/milestone4-proof/native-organized.pdf`, extracted the selected first page, and split into six one-page PDFs.
6. Reopened the saved PDF. All four managed annotations were listed; the new arrow remained editable with its saved dimensions.

Independent pypdf strict parsing confirmed the six-page order `2, 3, 1, 3, 2, 3`, first-page rotation 180°, CropBox `[18, 18, 582, 382]`, two vector assets, all split outputs, and unchanged source SHA-256. Poppler rendering was visually inspected; the initial triangulation seam finding was corrected and covered by a compound-fill regression.

Final release check: additional edits made during live testing were preserved in `native-organized-copy.pdf` before restarting. The final build was launched and its integrated Mac titlebar inspected. `native-final.pdf` preserves all six annotations, their style/geometry, all page contents and crop/rotation settings, and uses one compound fill for each vector asset. Poppler confirms the blue arrow and gold star now render without internal seams. See `final-verification.json` and `native-final-page1.png` beside the PDFs.


Evidence:

- `artifacts/milestone4-proof/independent-verification.json`: core operation outputs, order, boxes, rotations, annotation counts and source hashes.
- `artifacts/milestone4-proof/native-verification.json`: native UI output checks, extracted/split results and saved artwork dimensions.
- `artifacts/milestone4-ui-final/`: actual egui layout renders of the viewer, arrows, symbols, stamps, page overview and crop panel.
- `dist/MyPDF.app`: release build, version 0.4.0, ad-hoc signed; distribution notarization remains later work.

## Compatibility boundaries

Page tools accept unsigned, unencrypted PDFs up to 256 MB and 10,000 pages. Signed/certified documents remain read-only for ordinary edits. PDFs with interactive AcroForm/widget fields are explicitly rejected by page tools to avoid breaking their field hierarchy; form-aware structural editing is later work.

Existing page contents, inherited resources, page boxes, rotation, and annotation appearances are retained. Reorder preserves existing bookmark page targets; navigation to removed pages is cleared. Imported PDFs contribute pages and annotations, not their document-level bookmarks/catalog structures. Extraction/splitting creates page-only documents and does not copy source catalog actions or document-level trees. Custom page labels become sequential after structural page edits. Tagged-PDF reading order/accessibility structure rebuilding and a broad compatibility corpus are not yet validated.

Crop is reversible visibility adjustment. Foreign-reader annotations remain preserved but are not editable through MyPDF’s managed-annotation inspector. Windows build/live UI execution remains an explicit gate, as do installer signing/notarization, form-aware page editing, existing-text editing UI, conversions, and recovery/performance work from later milestones.

## Reproduce

```sh
cargo test --locked --offline --all-targets
cargo clippy --locked --offline --all-targets -- -D warnings
cargo fmt --all -- --check
MYPDF_PAGE_PROOF_DIR="$PWD/artifacts/page-proof-new" cargo test --locked --offline --test pages page_operations_visual_proof -- --ignored
MYPDF_PREVIEW_DIR="$PWD/artifacts/ui-preview-new" cargo test --locked --offline --bin mypdf native_visual_preview -- --ignored
sh scripts/bundle-macos.sh
codesign --verify --deep --strict dist/MyPDF.app
```

Use this checkout’s cached dependencies with `CARGO_HOME="$PWD/.cache/cargo"` when needed. Visual/proof tests write only to their explicitly selected artifact directories.
