# Milestone 1 — native desktop viewer

Built on 12 October 2026 with Rust 1.93.1 on Apple Silicon macOS. Source, the locked dependency graph, tests, a Mac packaging script and a macOS/Windows CI workflow are included.

## Delivered behavior

- Native desktop window: egui/eframe 0.33.3, winit and wgpu. Metal is enabled explicitly for macOS and DirectX 12 for Windows. No WebView or HTML interface is involved.
- Up to 16 document tabs. Opening a canonical path already in the session activates its tab. Page, zoom, scroll state, query and search results belong to each document.
- Continuous scrolling over mixed page sizes, rotated pages and CropBoxes. Only visible pages and a small surrounding area request rasters.
- Lazy, virtualized thumbnails with selected-page borders and accessible navigation labels.
- Page entry, previous/next navigation, Page Up/Page Down, Home/End, fit width, fit page, and 10–600% interactive zoom. Raster resolution follows display density and is independently limited.
- Search reads the renderer's Unicode glyphs and transforms, including embedded fonts and page rotation/crop. It normalizes whitespace, searches without case sensitivity using Unicode lowercase mapping, provides snippets, and highlights the matching glyphs. Enter/Shift+Enter and previous/next controls navigate matches.
- Native multiple-file Open and Save dialogs through rfd; macOS File/Edit/View/Window menus through muda. Toolbar and keyboard shortcuts are shared with Windows. Drag and drop and command-line paths also open documents.
- Save a Copy writes the original document bytes to a same-directory temporary file, synchronizes the file, then publishes to an unused path with `persist_noclobber`. Existing paths, destination symlinks and a concurrently created destination are preserved. Canceling a dialog does not write a file. Copies do not reserialize PDFs, so digital signatures retain their signed bytes.
- Invalid PDFs fail in their own tabs. Other documents remain available. The UI never reports a signature as valid or trusted merely because its field is present.

## Threading and resource limits

Each tab has a document thread which owns Hayro's parser and non-Send caches, plus one cancellable search thread. Render requests have a bounded eight-item queue; document results have a bounded two-item queue. Search requests and responses are also bounded. PDF work does not borrow native UI objects. Closing a tab cancels its workers; any operation already in progress finishes before its thread exits.

Search is debounced by 150 ms, and obsolete results are discarded by generation. A search snapshot shares immutable page indexes rather than copying the text and geometry. The full app raster texture cache uses a 128 MiB eviction budget. A single raster is limited to 12 million pixels and an 8192-pixel dimension; higher display zoom may therefore upscale a capped raster.

Inputs are limited to 256 MiB, 10,000 pages and finite supported page dimensions. Search indexes stop at approximately 64 MiB per document, plus at most one page, and cap each page at 100,000 glyphs / 2 million text bytes. The search panel identifies a partial index. Results cap at 1,000 matches per page and 10,000 per document; the sidebar shows at most 1,000 result cards.

## Automated verification

`cargo test --locked --offline`: **22 passed**, consisting of the previous 11 core checks, 10 viewer checks and one headless desktop event-loop check:

1. Unicode search geometry on both an ordinary page and its cropped, rotated counterpart; glyph bounds remain within the displayed page and rotate as expected.
2. Lowercase expansion maps matches back to the original UTF-8 spans.
3. Phrase search crosses line breaks and normalized whitespace.
4. Background search returns matches on the correct pages and identifies stale generations.
5. Background loading, indexing and rendering work without a window; output has the expected RGBA dimensions and opaque white background.
6. Invalid PDF input yields a recoverable error.
7. Saving a signed PDF preserves every byte; signature mathematics and document digest still verify.
8. Existing files are preserved, exactly one competing save wins, and temporary files are cleaned up.
9. A destination symlink cannot replace its target.
10. Large-page raster requests are bounded.

The headless desktop check drives real egui input frames and the background workers. It checks initial raster delivery, 100% zoom, text input into search, Enter/Shift+Enter navigation, Ctrl+Tab state preservation, canonical-path deduplication and invalid-file recovery. It caught the fact that a single-line text field can surrender focus on Enter; navigation now handles that frame and restores search focus for repeated navigation.

Formatting and Clippy with warnings denied are checked as part of the build workflow.

The optimized Apple Silicon Mac bundle is approximately 17 MiB, includes a procedural application icon, and passed `codesign --verify --deep --strict` after local ad-hoc signing. Apple's icon packager required execution outside the tool sandbox; packaging succeeded with that permission.

## Mac UI verification

The first app launch exposed a missing native wgpu backend feature. Adding the Metal/DirectX features fixed the launch, and the Mac window opened successfully. The Open shortcut displayed the real macOS file picker and opened the synthetic fixture. The displayed vector/image content and Turkish text were inspected. Searching `sözleşme` returned two page results, with the first on-page highlight visually aligned to its text.

After the Mac was unlocked, the remaining live checks were completed in the packaged app:

- Opening the synthetic fixture through the native picker enabled the viewer controls and displayed both thumbnails.
- Clicking the second thumbnail selected page 2, updated the page entry/status and displayed its cropped, rotated content.
- The native ⌘0 shortcut selected 100% zoom; the + button increased it to 120%. Fit Page selected the fit mode and recomputed its scale to 80% in the test window.
- Searching `sözleşme` returned the two page results. Enter advanced to the rotated page with the highlight aligned to its text; Shift+Enter returned to the first result and kept search focus.
- Ctrl+Tab switched between `original.pdf` and `signed-twice.pdf`, restoring the original document's Fit Page zoom, query and selected result. The signed document displayed the signature-presence badge and its own initial zoom/query state.
- ⌘S opened the macOS Save dialog with a fresh suggested filename and the explicit new-filename instruction. Canceling produced no output. Completing the dialog saved `artifacts/milestone1-ui/signed-twice-copy.pdf` and displayed the successful-save status.
- `cmp` passed, and source/copy SHA-256 both equal `bf34aec8de6983b851b8abb15011bbfa0669b54bf478be5310324d6a154ed73b`. Offline verification of the copy reports both `P12Signature` and `SecondSignature` with valid cryptography, matching digest, valid byte range and valid certificate binding. Their synthetic certificates remain untrusted, as expected. Evidence is saved in `artifacts/milestone1-ui/signature-verification.json`.
- The saved copy was reopened through the native picker and rendered with both thumbnails and its signature-presence badge.

Live inspection also found that a global text-color override reduced contrast on green selected controls and that the zoom popup remained open after selection. Both were fixed: selected controls now use the selection's white foreground, and a zoom choice explicitly closes the popup. The headless desktop test, formatting and Clippy passed after these changes. The optimized bundle was rebuilt, its code signature verified, and both fixes visually confirmed after restarting it.

The app was left open with the verified signed copy in Fit Page mode. Native drag-and-drop and a broader PDF compatibility corpus have not been exercised in this live pass; the checks above cover the previously pending Mac acceptance items. Windows execution remains a separate gate.

## Remaining boundaries

- This milestone is a viewer. Text editing, annotation tools, signatures/certificates UI, page management and conversions are subsequent UI work. The milestone 0 command-line proof remains available.
- Search requires a text layer and useful Unicode mappings. OCR and complete reading-order reconstruction are not implemented. Unicode lowercase matching is not full locale-aware case folding; accent-insensitive matching is not claimed.
- Password prompts, encrypted-document support, printing, Finder document associations, persistent sessions and recent files are deferred.
- Save requires a new filename; in-place replacement is intentionally not offered in this read-only milestone. An externally changed source is not automatically reloaded: the immutable opened snapshot is what gets copied.
- A signature-presence badge does not perform trust or integrity validation. The existing signing core supplies explicit verification outside this viewer UI.
- Hayro remains a developing renderer. The synthetic corpus is verified; arbitrary PDF fidelity and malformed-document hardening require a broader corpus.
- Mac packaging uses local ad-hoc signing. Developer ID signing, notarization, release-license auditing, Windows packaging and a live Windows check remain release gates. The Windows CI workflow has not been executed in this local folder.
