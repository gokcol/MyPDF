# Milestone 3 — annotations and reusable assets

Implemented on 12 October 2026 in the native Rust desktop application.

## Delivered

- Highlight, underline and strikeout using positioned text glyphs, with rectangular fallback for image-only areas.
- Freehand ink, popup-style notes, Unicode free-text boxes, rectangles, ellipses, lines and arrows.
- Check, cross, star, circle and arrow symbols; approved/reviewed/draft/date stamps and editable custom stamp text.
- Selection, drag movement, lower-right resize handle, inspector controls for position, dimensions, rotation, color, fill/background, opacity, line width and text size; duplicate/delete and per-document undo/redo. Inspector changes can be applied/reset, and pending changes are applied before Save, Sign or a close check.
- Drawn or typed visual signatures and reusable selections in a local asset library. PNG/JPEG artwork import and Rust SVG rasterization. Artwork preserves aspect when placed from a click or in a digital signature appearance; a dragged rectangle supplies an explicit placement size.
- Page thumbnails include managed annotations. The Marks sidebar lists your annotations by page. Standard PDF appearance streams let other readers display saved graphics/text without needing MyPDF metadata.
- Immutable source snapshots, fresh output files, atomic catalogue/library publication, visible dirty state and save/discard/keep-working close decisions. Signed/certified PDFs remain read-only for ordinary annotations.

## Verified evidence

Seven annotation integration checks cover standard appearance streams, Unicode text, cropped/rotated pages, preserved original text/content, save/reopen without duplicate annotations, deletion, history/save revisions, signed/certified guards, indirect annotation arrays, foreign-note preservation, reader-modified annotation preservation, persistent assets/concurrent writers, and bounded SVG import with external references rejected. The headless canvas tests drive actual egui pointer events to create, move and resize a mark, checks undo/redo, applies pending inspector properties, rejects invalid style values, blocks signed edits and checks the dirty-close guard, exercises drawing in the real scrolling viewer, and checks fast drags with movement/release in one frame and fully batched gestures. A separate signature-pad check retains both endpoints in batched input.

`artifacts/milestone3-proof-003/annotations.pdf` was rendered with Hayro and independent Poppler. Both Poppler page images were visually inspected: Unicode text, an approved stamp rotated by −8°, a checkmark and a cropped/rotated-page text box render correctly. Original raster/vector content remains visible and unchanged. The complete suite has 36 passing checks, and formatting/Clippy pass.

Live macOS UI checks exercised symbol placement, selection and inspector controls, Undo/Redo, typed-signature asset creation and placement, safe Save a Copy, reopening four editable marks, and the unsaved-close guard. Saving with an unapplied Unicode inspector draft preserved the final text `İstanbul - GUI Test`; independent extraction and Poppler rendering confirmed it. A fast native drag moved a rotated stamp correctly after the canvas was fixed to capture press/release events arriving within one frame. The selected rotated bounding-box resize handle is now hit-tested even when it lies outside the rotated shape. The automated canvas and signature-pad tests cover these batched gestures and retained stroke endpoints.

Native annotation-save and signing evidence is recorded under `artifacts/milestone23-ui/`; `gui-annotations.png` and `gui-signed.png` were visually inspected. All changes were made to synthetic fixtures or fresh copies.

The final optimized 0.3.0 bundle was rebuilt after the rotated-handle fix and passes strict code-signature verification. The Mac locked again before the final live handwriting-pad check and restart of that rebuilt bundle. The stroke-input fix is covered by the passing automated signature-pad check; this last live check remains pending an unlocked desktop.

## Storage and limits

Assets are stored locally in `assets.json` under the MyPDF application-data directory, guarded by atomic publication, an OS file lock, symlink rejection and a source fingerprint to detect other writers. The library supports up to 200 assets/64 MB; each imported file is limited to 8 MB and 4,096 × 4,096 decoded pixels, normalized to at most 2,048 pixels on its longest side. UI image textures are limited to eight 1,024-pixel previews. SVG scripts are not executed, and external file/network image references are rejected.

Up to 5,000 managed annotations per document and 100 history snapshots are supported. Geometry/text/style checks reject malformed data and text that cannot fit in a saved appearance. Image bytes are shared across history snapshots. Existing annotations from other apps remain visible and are preserved, but are not editable in this milestone. The editable round-trip model uses standard annotations plus MyPDF metadata; reader-modified geometry/appearance streams or unsupported round-trip metadata are kept as foreign annotations to avoid restoring stale edits. Importing them into the editable model requires a later gate.

This adds text boxes as annotations. Existing PDF-content text editing remains available in the milestone 0 harness and is scheduled for desktop milestone 5. OCR, rich paragraph editing, annotation flattening, page management and conversions are subsequent work. Visual signatures do not by themselves provide certificate integrity.

Windows build/live verification, a broader PDF corpus, long-session performance/accessibility checks and recovery remain later gates.

## Native UI polish and built-in artwork

The follow-up adds vector toolbar icons, rounded controls, blue selection accents, visible hover/focus states, and a searchable preview-card asset library. Its Arrows, Symbols and Stamps collections include 24 editable vector assets; Saved retains the existing local catalogue. Built-ins do not modify that catalogue or require importing files. Click placement preserves aspect ratio and uses compact initial sizes for symbols and directional arrows.

An additional integration check exports and reopens all 24 assets as editable standard PDF annotations while preserving source text. The desktop gesture/event-loop checks continue to pass. A manual, ignored visual-QA harness renders the actual egui layout to PNG without a window server; previews are saved under `artifacts/ui-polish-preview/`. This provides layout inspection while the Mac is locked, and does not replace live native interaction testing.

The follow-up suite passes all 37 regular tests (the manual visual harness is excluded from that count). Native viewer, arrow, symbol and stamp previews were inspected for legibility, placement, selected states and clipping. The gallery scrolls for larger collections and smaller windows. The final optimized app bundle is ad-hoc signed; strict signature verification passes. Live checking was initially delayed by a locked Mac and completed on 13 October as recorded below.

### Live native asset checks — 13 October 2026

The packaged app was checked on the unlocked Mac. A right arrow and gold star were selected from the built-in gallery, placed on a separate synthetic fixture and resized by dragging each lower-right handle. The arrow changed from 180 × 64.29 to 248.62 × 111.65 display points; the star changed from 72 × 72 to 133.98 × 119.10 points.

The native Save dialog created `artifacts/native-assets-check-20261013/arrows-symbols-resized.pdf`. Reopening that copy showed both entries in Marks as editable annotations, and each inspector retained the resized dimensions. Independent inspection confirms two standard `/Line` and `/Stamp` annotations with appearance streams, unchanged original page content streams and an unchanged source file. Poppler-rendered page 1 was visually inspected and agrees with the native viewer. Evidence is in that directory's `verification.json` and `poppler-page1.png`. No application code fixes were required for this check.
