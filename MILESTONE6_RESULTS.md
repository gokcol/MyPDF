# Milestone 6 — macOS recovery, interaction and UI polish

Implemented in MyPDF 0.6.0 on 13 October 2026. Native Rust/egui desktop application; no WebViews. Windows build and execution are deferred at the user's request.

## Delivered

- Drawing inspector updates immediately without Apply/Reset, coalesces continuous gestures into one Undo entry, and assigns fresh revisions for autosave/in-flight saves. Color pickers suppress non-user RGB round-trip drift. Solid/dashed/dotted/dash-dot/wave geometry is shared by the canvas and vector PDF appearance export; legacy styles default to solid. Filled shape borders and shared vector silhouette edges are handled explicitly.
- Click drawing artwork to select it, including with drawing tools armed; click blank page or gray canvas to deselect. Stroke-aware hit testing avoids selecting empty corners of line/arrow bounds. Pending vertex paths keep their multi-click semantics. An unselected inspector preserves layout and exposes defaults for new drawings.
- Certificate default preference persists by source and SHA-256, with legacy catalogue migration and safe concurrent-write detection. macOS Keychain discovery runs at startup and when signing opens, independently of other signing work. Missing/expired defaults never fall back to a different identity. Default UI is available in both identity and signing panels; credentials are not persisted. Windows Personal-store discovery/non-exporting CNG/CryptoAPI signing is implemented but not runtime-validated.
- Selection outlines and resize handles use the same live gesture geometry as drawings. They track every drag/resize frame, including rotated marks, page-edge clamping, non-default zoom and dragging a different object, without creating extra undo steps.
- Sign dropdown in the main ribbon and Annotate/Draw ribbons, with Signature and Digital Signature options opening the respective visual-signature and certificate-signing workflows.

- Home workspace with Open/drop PDFs, six tool cards including New Blank PDF, searchable Recents and Favorites, persistent local history and asynchronous first-page previews. Home preserves tabs/drafts and has a ⇧⌘H shortcut. History writes merge concurrent app changes under a file lock; clearing shortcuts never removes documents.
- Existing-text editing now opens a text area directly on the PDF, including MyPDF text-box annotations. Exact operator geometry distinguishes repeated identical phrases and supports page crop/rotation. MyPDF-created embedded fonts extend their character mapping when required.
- Rectangular text selection in Edit → Select area: live selection highlighting, visual line ordering, a multiline on-page editor, font/size/color controls and a resize handle. Selected page runs and/or managed text boxes become one uniformly formatted PDF block; original strings are removed from text-showing operators. Commit/Save/Close share the safe worker transaction, with one Undo step and retained drafts on failure.

- Separate left/right panel toggles, labeled vector icons, compact mode tabs and contextual Annotate/Draw ribbons. Navigation, zoom and undo live in the compact footer.
- Refined annotation inspector: color palette/custom color, compact percentage and stroke controls, stroke presets, expandable position/size/rotation, icon actions and reusable artwork saving.
- Mac-style save/discard/recovery confirmations: rounded card, soft shadow, stacked full-width actions, blue primary action, Return default and Escape cancel. Tool windows share rounded styling. File dialogs remain native macOS Open/Save pickers.
- Pinch zoom over the document, clamped to 10–600%, anchored to the PDF point under the pointer. Panels/UI do not magnify. Existing rasters scale immediately; a sharper render is requested after 120 ms of gesture quiet to avoid queuing every intermediate scale.
- Polyline, polygon and double-arrow tools. Click adds vertices; double-click near the last vertex or Return commits; Backspace removes the last point and Escape cancels. Paths are standard PDF PolyLine/Polygon/Line annotations with appearance streams. Selection, movement, resizing, rotation, styling, undo and save/reopen use the existing annotation pipeline.
- Stamp/library management: create from SVG code or imported SVG/PNG/JPEG; rename; replace artwork; edit saved SVG by stable ID; duplicate; remove and undo the last removal. Custom stamps appear before built-ins in Stamps, and all custom items remain in Saved. Existing placed marks retain their independent content. Updates are atomic and reject concurrent library changes rather than overwrite another process's work.
- External LLM workflow: describe a stamp, copy a tailored prompt, execute it outside MyPDF, paste the response, extract/validate SVG, preview/edit, then save. No model connection, API credentials or network requests were added.
- Add actual selectable PDF text on empty or populated editable pages. Type directly on the page with a live sample and font-family/style, size, bold/italic, alignment and color controls. Selected installed TrueType fonts are embedded. Supports move/resize handles and numeric geometry. Clicking elsewhere, Done or Command/Control-Return commits the text. Save/Close also commit pending text, preserving invalid drafts on failure; unfinished paths cannot silently pass through Save/Close.
- Recovery copies after 5 seconds of quiet committed changes or 30 seconds of continuous edits. One background worker serializes snapshots and synchronizes atomically replaced files. Originals are never overwritten, and autosave does not mark a document saved.
- Startup recovery browser; restore into a new unsaved tab; explicit discard; live-instance locks; checksummed records; restrictive storage permissions; cleanup after successful user save/discard. Failed/corrupt records remain available for diagnosis.
- Performance: cached system SVG fonts, background/debounced SVG previews, bounded artwork/page texture caches, background blank-document loading, pruning superseded PDF appearance objects.
- Accessibility: labeled icon buttons/toggles, focus indication, keyboard panel shortcuts, page-number label, keyboard-accessible numeric controls and View → Accessible Page Text.

## Verification

`cargo test --locked --offline --all-targets`: **94 passed**, 0 failed, 3 manual tests ignored. `cargo clippy --locked --offline --all-targets -- -D warnings` and formatting check passed. Release macOS bundle built and its ad-hoc signature verified.

Tests additionally exercise Home layout at 720/1200 pixels, persistent favorites and recents, concurrent history updates, new-text placement/save/reopen, clickable repeated phrases and text boxes, native egui field focus, rectangular pointer selection and resizing, cancellation, Unicode multiline replacement, mixed text/annotation replacement, one-step undo, original-file preservation, and rejection of stale/overflowing/shared-position selections.

Tests include actual subprocess termination/recovery, exclusive live-session ownership, checksum rejection, original-file preservation, autosave revision tracking, text on blank/existing/rotated/cropped pages, selected-font embedding/alignment, on-page typing and commit, pending-text close behavior, real pointer-driven text movement/resizing, polyline/polygon creation and cancellation, double-arrow creation, undo, PDF save/reopen, standard annotation dictionaries, stamp replacement/legacy-library compatibility, unsafe SVG rejection, zoom anchoring and unchanged UI scale, confirmation keyboard behavior and panel accessibility/layout.

Native checks during this milestone verified new blank text insertion and Save, recovery restoration into an unsaved tab, and accessible page text. The final isolated preview app launched successfully with the updated controls; existing MyPDF and QA editing sessions were preserved. Physical trackpad pinch has not been manually exercised through the native automation tool (it has no pinch operation); the macOS winit PinchGesture → egui Zoom path was inspected and the actual document event loop tested with Zoom events.

Visual QA rendered the actual egui layouts for the inspector, SVG prompt workflow and save confirmation; all were inspected. Independent Poppler rendering verified the saved drawing appearances. Earlier saved new-text and autosave PDFs were independently extracted with pypdf and matched byte-for-byte.

The updated Area Review macOS app launched and exposed the new Edit controls. Native focus and in-place typing were subsequently verified with the test window raised. Concurrent activity interrupted the Save dialog check; the window and original source were left intact. The actual egui pointer/keyboard event tests verified in-place selection, typing, resize, cancellation, commit and save/reopen. Independent Poppler rendering and text extraction of the saved multiline block confirmed the replacement and unchanged neighboring text.

Home and in-place text evidence is in `artifacts/home-proof/`: `home.png`, `homeempty.png`, `inlinetext.png`, `blocktext.png`, `block-proof/original.pdf`, `block-proof/edited.pdf`, `block-proof/edited.png`, `block-proof/edited.txt`, `verified-sha256.txt`, and the final test/clippy/build logs.

The drawing/signing follow-up evidence is in `artifacts/interaction-proof/`: current all-target test, clippy and signed bundle build logs. Its regression tests inspect the emitted drawing, border and handle on intermediate pointer frames, verify one undo step per gesture, and exercise both Sign menu choices from both ribbons, immediate property changes across object types, single-gesture Undo, selection/deselection and unchanged document revisions on clicks. The stroke matrix (`stroke-styles.pdf` / `.png`) was independently rendered by Poppler and inspected; `visuals/inspector.png` shows the actual egui controls with style samples and no Apply/Reset. Certificate tests cover default persistence, legacy migration, missing-store identities, duplicate discovery, public-certificate exclusion and unrelated worker state.

The final main Mac bundle was relaunched after the user quit it. Native inspection confirmed the five style controls without Apply/Reset and automatic discovery of four Keychain identities. The user resumed editing and signing in that window, so further automation stopped without changing their session. Final click/deselect and drag-frame behavior is verified by the actual egui event-loop regression tests rather than an uninterrupted native click-through.

`windows-api-types.log` records a successful compile-only Win32 adapter type check against the installed Windows target. The harness uses stub portable identity/trait definitions and does not test cryptographic execution. A full cross-target dependency check stopped at ring's missing Windows C headers on this Mac (`windows-provider-check.log`); no Windows package was built.

Earlier milestone evidence is in `artifacts/milestone6-proof/`:

- `tests-final.log`, `clippy-final.log`, `build-final.log`
- `visuals/inspector.png`, `visuals/prompt.png`, `visuals/confirmation.png`, `visuals/addtext.png`
- `draw-shapes.pdf`, `draw-shapes.png`
- `native-new-text.pdf`, `native-new-text.png`, `native-autosave.pdf`, `independent-verification.json`

The main updated application is `dist/MyPDF.app`. `MyPDF QA.app` and `MyPDF Preview.app` under the proof directory use separate profiles; leave their user-created edits intact and save documents before closing them.

## Practical limits

Recovery preserves the latest completed committed snapshot, not unapplied typing, pending SVG edits or undo history. Copies contain PDF content and path metadata and are not separately encrypted. Limits are 256 MB per PDF and 2 GB per recovery session. A crash before the first snapshot, disk failure or export delay can leave newer changes unrecovered.

Pinch anchoring necessarily clamps at scroll edges. Polygon editing currently moves/resizes the whole shape rather than exposing individual vertex handles. Imported raster stamps keep their original colors; SVGs support tint overrides. SVG source is limited to 256 KB and static supported vector content. LLM output must be reviewed externally and in the native preview before placement.

Area selection operates on whole page-level text objects and MyPDF text boxes. A grouped block uses one installed font/size/color and does not preserve mixed formatting. Scanned text, Form XObject text and arbitrary external FreeText annotations are outside this editor. Selecting only part of a shared-position line may require expanding the selection; unselected text geometry is verified before a block commit. No OCR or redaction is provided. Home keeps path/date/favorite metadata locally; thumbnail rasters stay in memory.

Safe Save a Copy still requires a new destination filename. Signed/certified documents retain ordinary-edit restrictions. Existing-text compatibility, form, conversion and signing limitations from earlier milestones still apply. Accessibility labels/text access are improved; this is not a claim of complete VoiceOver conformance. The bundle is ad-hoc signed for local use; Developer ID signing/notarization remains release work.
