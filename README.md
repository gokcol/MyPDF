# MyPDF — native Rust desktop PDF editor

Version 0.6.0 adds a Home workspace, local autosave and crash recovery, icon-led mode tabs and tool ribbons, independent panel toggles, and on-page text editing with rectangular block selection. macOS first, portable Rust core, no WebViews. Windows builds are deferred by request.

- **Home:** Open/drop PDFs, create a blank PDF, launch common tools, and browse local Recents and Favorites with page previews.
- **Edit:** type directly on blank or existing pages; click existing text to edit it in place; drag a rectangular selection to reflow several lines together; replace, reposition, resize and delete existing page-level images.
- **Forms:** fill text, checkboxes, radio groups and single-select choices, with saved appearances.
- **Export:** PNG/JPEG ↔ PDF; PDF ↔ Markdown, text and Word `.docx` using text-oriented conversion.
- **SVG:** Annotate → Saved → SVG code; paste code, preview, save and place. Select placed artwork to move, resize, rotate, recolor or edit its SVG source.
- **Blank PDF:** File → New Blank PDF (⌘N/Ctrl+N). **Blank page:** Pages → Blank Page.

Every edit is undoable. **Save a Copy** preserves the original and refuses to overwrite an existing file. Word/Markdown conversion does not reconstruct complex layouts, and existing-text editing has font/object compatibility limits. See [milestone 6 results](MILESTONE6_RESULTS.md) and [milestone 5 compatibility limits](MILESTONE5_RESULTS.md).


A native Rust desktop PDF viewer and annotation/signing editor, macOS first and portable to Windows. The interface uses egui/eframe and winit, with Metal on macOS and DirectX 12 on Windows. It contains no WebView, HTML interface, browser renderer, or JavaScript application runtime.

Milestone 1 adds document tabs, a continuous page canvas, lazy thumbnails, Unicode text search with positioned highlights, page navigation, fit modes and zoom, native Open/Save dialogs, drag and drop, and macOS menus. Each tab retains its page, scroll position, zoom and search query during the session. Parsing, indexing, search and PDF rasterization run on background threads.

Milestone 2 adds a digital identity manager, personal certificate creation, authenticated `.p12`/`.pfx` import and encrypted export, public-certificate inspection, native macOS Keychain identity selection, visible/invisible approval signatures, and offline validation. Signature appearance supports a page rectangle, identity name, reason, location, and reusable artwork.

Milestone 3 adds highlights, underlines, strikeouts, pen strokes, notes, Unicode text boxes, shapes, lines/arrows, symbols, stamps, drawn/typed visual signatures, and a local PNG/JPEG/SVG asset library. Select, move, resize, rotate, style, duplicate or delete your annotations; Undo/Redo tracks document edits. Saved annotations use standard PDF subtypes and appearance streams.

Milestone 4 adds a thumbnail page organizer with drag reordering, multi-selection and page ranges, blank/PDF insertion, deletion, extraction, multi-file merging, splitting every N pages, clockwise/counterclockwise rotation, and crop margins with a live preview and reset. Page changes and annotations share one Undo/Redo history. Rendering and text search refresh after each page transaction.

The redesigned interface follows PDF Expert’s compact toolbar and panel layout: dark document tabs, a grouped annotation ribbon, and a fixed right-hand library. **Draw → Arrows / Symbols / Saved** and **Annotate → Stamps / Saved** opens searchable panels containing 33 built-in vector items, including solid directional arrows, double/elbow arrows, chevrons, colored checks and crosses, stars, warnings, information symbols and stamps. Select an item, then click or drag on a page to place it. **Select** reveals the inspector; drag the corner handle to resize or use its dimensions and color controls. Your existing saved artwork remains available.

**Save a Copy** writes the current edited snapshot to a fresh filename, or preserves the exact opened bytes when there are no applied changes. Existing files are never replaced, including concurrent destinations. Unsaved changes prompt on close. Signed/certified PDFs open protected; **Edit anyway…** in the signature bar unlocks them, and every save then appends your edits after the signed bytes (see *Signed PDFs* below). New certificate signatures also append revisions while preserving earlier bytes. Page tools also reject encrypted PDFs and interactive forms. Cropping hides content; it does not redact it. Existing-text editing, form filling and local conversions are available in the Edit, Forms and Export panels.

In **Pages**, click a thumbnail, use ⌘/Ctrl-click or Shift-click for multiple pages, or enter a range such as `1, 3-5`. Drag to the left/right half of a thumbnail to insert the selection before/after it, or use **Move before page**. **Insert** accepts a blank page or PDF files at a chosen position; **Merge** appends PDFs. **Extract** saves selected pages. **Split** exports the whole document into a fresh folder, using the interval in the right panel. **Crop** previews displayed-page margins in points and applies them to the selected pages.

## Run the desktop viewer

```sh
# Development run with an isolated profile in target/dev-profile (see --help):
sh scripts/dev.sh
sh scripts/dev.sh --release ~/Documents/report.pdf
# Headless UI screenshots into target/ui-preview:
sh scripts/dev.sh --preview=Home,Confirmation

cargo run --locked --bin mypdf
# Optional PDF paths at launch:
cargo run --locked --bin mypdf -- /path/to/first.pdf /path/to/second.pdf

# Create a locally ad-hoc-signed Mac app bundle:
sh scripts/bundle-macos.sh
# Faster development package:
sh scripts/bundle-macos.sh --debug
```

The Mac bundle is `dist/MyPDF.app`. It stays inside this project; no installation or file-association changes are made. Distribution signing and notarization are separate release work. Windows build work is paused at the user’s request. The portable core and existing Windows CI configuration remain; Windows execution is not verified.

Open with the toolbar, **File → Open**, **⌘O / Ctrl+O**, drag and drop, or launch paths. Use **⌘F / Ctrl+F** for search; Enter and Shift+Enter navigate results while the search field is focused. Use the zoom menu, +/−, **⌘0 / Ctrl+0** for 100%, and Page Up/Page Down for page navigation. Ctrl+Tab switches tabs. **⌘S / Ctrl+S** opens Save a Copy; choose a fresh filename. macOS also supports ⌘Shift+S.

The app runs viewing, annotation export, signing and validation locally. It stores local recovery snapshots of applied unsaved document changes; Home stores recent-file shortcuts locally; search queries and undo history are not stored across launches. Reusable visual assets and public identity metadata/file references are saved in `~/Library/Application Support/MyPDF` on macOS and `%LOCALAPPDATA%/MyPDF` on Windows; `MYPDF_DATA_DIR` overrides this for isolated testing. Passphrases and private keys are not saved in those catalogues. Use **Annotate** for markup and stamps, **Draw** for pen/shapes/arrows/symbols, **Edit** for PDF content, and **Saved** for reusable artwork, and **Sign → Digital Signature → Digital IDs** for certificate identities. Visual signatures and certificate signatures are distinct features.

See [MILESTONE1_RESULTS.md](MILESTONE1_RESULTS.md), [MILESTONE2_RESULTS.md](MILESTONE2_RESULTS.md) [MILESTONE3_RESULTS.md](MILESTONE3_RESULTS.md) and [MILESTONE4_RESULTS.md](MILESTONE4_RESULTS.md), [MILESTONE5_RESULTS.md](MILESTONE5_RESULTS.md), and [MILESTONE6_RESULTS.md](MILESTONE6_RESULTS.md) for verification and limits. The implementation sequence is in [ROADMAP.md](ROADMAP.md).

## Workspace, new text and recovery

**Home** is always available in the tab row or with **⇧⌘H**. It includes Open Files, New Blank PDF, tool shortcuts, searchable Recents and Favorites, and first-page previews. Returning Home keeps open tabs and pending text. The local history stores up to 50 recents and 100 favorites; clearing/removing shortcuts does not delete PDFs. Preview generation is asynchronous and memory-bounded (files above 64 MB use a document placeholder).

The top row switches **Read / Annotate / Edit / Draw / Forms / Sign / Pages / Export**. A context ribbon underneath contains the tools for that mode. **Sign ▾** in the main ribbon offers **Signature** (draw/type a visual signature) and **Digital Signature**, which first asks you to drag a box (or click) where the signature goes, then opens certificate signing (.p12/.pfx or macOS Keychain) with that area filled in. Esc cancels. The Annotate/Draw ribbon uses the same Sign menu. Zoom, page navigation, Undo/Redo and recovery status share the footer. At narrow window widths, mode labels collapse to accessible icon buttons with tooltips.

Select a drawing by clicking it; click empty page space or the gray surround to clear its selection. Lines and arrows are selected near their strokes, rather than throughout an empty bounding rectangle. A click on existing artwork selects it even with a drawing tool armed; an in-progress polyline/polygon keeps receiving vertices until finished or canceled. The inspector stays in place to avoid shifting the page when selection clears.

**Select several marks** in Annotate or Draw: with the Select arrow, drag across empty page space to draw a rectangle, or choose **Lasso select** and draw a freeform loop. Marks whose center falls inside are selected (the same rule as Edit → Select area). Shift-click adds or removes a mark; Shift-drag adds to the selection; ⌘A/Ctrl+A selects every mark on the current page; Esc clears it. Drag inside the selection to move it, or drag a corner handle to scale it proportionally (rotated marks keep their angle; text boxes scale their font). Arrow keys nudge by 1 pt (Shift: 10 pt), and Delete/⌫ removes the selection. The inspector sets color, opacity, stroke and style for all of them, aligns their edges or centers, and duplicates or deletes them. Each move, scale, alignment, nudge or deletion is one Undo step, and a selection stays on one page.

Drawing properties update immediately: color, fill, opacity, stroke, rotation, position and size need no Apply or Reset. A continuous slider or numeric drag is one Undo step. The **Style** samples offer Solid, Dashed, Dotted, Dash-dot and Wave for native paths, shapes, arrows, pen strokes and vector symbols. Arrowheads remain solid; filled shapes retain a matching outline. Uploaded SVG/image artwork retains its own stroke structure, with its supported color/opacity/geometry properties applied live. With nothing selected, the inspector sets stroke defaults for new drawings. Style and geometry survive Save a Copy and reopening.

**Sign → Digital Signature** discovers signing identities from macOS Keychain automatically in the background, and refreshes them when the signing window reopens. Select an identity and choose **Use as default certificate** in either Sign document or Digital IDs. The default is remembered across launches and preselected for subsequent documents; **Clear default** removes the preference. Missing or expired identities are marked unavailable instead of substituting another certificate. PKCS#12 defaults remember only the file/alias/certificate reference, and still require the passphrase when signing. Store discovery reads public certificate metadata; it does not sign, export private keys or change trust settings. A Windows Current User Personal-store adapter is implemented and its Win32 API types checked; Windows execution and packaging remain deferred.

**Quick Sign** (Sign ribbon, Sign ▾ menu, or Sign → Quick Sign) signs in one gesture with your default certificate: drag a box on the page, or click to place a default-size box. Keychain identities sign at once; a .p12/.pfx identity asks only for its passphrase. The signed copy is saved next to the original as *Name-signed.pdf* (then *Name-signed 2.pdf*, …), never replacing a file, and opens in a new tab. Unsaved new documents ask for a filename. Without a default certificate, Quick Sign opens Digital IDs to choose one instead of guessing. **Sign with options…** keeps the full form (field name, reason, location, exact rectangle).

**Signature appearance** (Sign ribbon, Sign menu, or the Appearance tab in Digital IDs) chooses what Quick Sign and Sign with options draw: *Image + details* (your image beside “Digitally signed by”, the name and date), *Image only* (just your image, such as a handwritten signature, on a transparent background, in a box as small as 24 × 12 points), or *Details only*. **Draw or type…** and **Import image…** (PNG, JPEG or SVG) add a new image and select it; any saved library artwork can be picked too. The choice is remembered across launches with the default certificate reference.

**Appearance:** View → Appearance, or the half-circle button at the right of the tab bar, switches between **Match System**, **Light** and **Dark**. Match System follows macOS as it changes; the choice is remembered in `interface.json` in the data folder. Document pages, thumbnails and artwork previews stay white paper in both themes.

Toggle the left and right panels independently using the two top-right panel icons, **View**, or **⇧⌘L / ⇧⌘R** (Ctrl+Shift+L/R on other platforms). Closing a panel retains its state. **View → Accessible Page Text** exposes the current page’s extracted text in a selectable native text control.

**Edit → Add text** works directly on blank and populated editable pages. Click the page and type into the on-page text box. The right panel shows a live sample, installed font family and style, size, bold/italic, left/center/right/justified alignment and color. Drag the top handle to move the box; side/corner handles resize it. Click elsewhere on the page, press **⌘Return / Ctrl+Return**, or choose **Done** to commit selectable PDF text and continue adding blocks. Save/Close first applies pending text; invalid or overflowing text stays open with an error. Escape/Cancel discards the uncommitted block. Text in any installed script works: characters the chosen font lacks fall back per character to an installed font that has them (Arial Unicode, Hiragino, Apple SD Gothic Neo, Noto, …), text is shaped with HarfBuzz rules (Arabic joining, Indic and Thai clusters, ligatures), right-to-left runs are laid out in visual order, and only the used glyphs are embedded as font subsets. Complex scripts also carry `/ActualText`, so copy and search return the logical text. Emoji and characters without any installed outline glyph are refused with a message. Existing content stays intact; Undo and Save a Copy work as usual. Text placement respects page crop and rotation. The initial font catalogue offers installed Arial, Times New Roman, Courier New, Georgia and Verdana faces on macOS, with platform alternatives when available. Mixed formatting within one block is not provided. The on-page typing preview does not join Arabic letters or reorder right-to-left text; the committed PDF text is shaped and ordered correctly.

**Edit → Edit text**: click visible page-level PDF text or a MyPDF text box to open its editor directly on the page. Done, an outside click, or ⌘Return commits; Escape/Cancel restores the original. The editor draws a dashed outline with side handles, like PDF Expert, rather than a solid box. Individual PDF runs keep their own font when it can show every character. When it cannot (most PDFs embed only a subset of the glyphs they use, so typing “ö”, “ş”, Cyrillic or CJK into them used to fail), the run is re-set in the closest installed family with the Unicode pipeline above, and the status bar names the substitute font. Scanned text and text inside Form XObjects are not editable yet.

**Edit → Select area**: drag a rectangle around several lines (with Edit text, dragging across empty page space does the same). Text objects whose centers fall within it highlight as you drag; releasing opens one multiline text area. Choose a font, size and color in the inspector, type on the page, and drag the corner to resize. The selection becomes one block with uniform formatting; it does not retain mixed fonts/colors. Applying replaces the selected page text and/or MyPDF text boxes, preserving other content, with a single Undo step. Empty text removes the selection. Overflow keeps the draft open. If unselected text shares positioning with a selected run, include that complete line; the app rejects edits that would shift nearby text. Selection is limited to 2,000 objects/32 KB per page. While the block is open, only the selected words are hidden; table lines, boxes and backgrounds around them stay visible. Clicking outside an untouched selection closes it without rewriting anything. This is content editing, not a redaction tool.

**Header & Footer** (Edit ribbon, or **Edit PDF → Header and Footer** in the menu bar) adds real, selectable text to the page margins:
- **Add Page Numbers…** offers the formats *1*, *Page 1*, *Page 1 of n*, *1 of n*, *1/n* and *- 1 -*, with a start number.
- **Add Bates Numbers…** combines a prefix, a zero-padded number (start and digits) and a suffix.
- **Add Text…** takes any text. A clock menu offers recently used text, and **Field** inserts the page number, page count, date or file name.

For every kind, you choose all pages or a range, the position (top or bottom; left, center or right), margins in millimetres, and the font, style, size and color. The preview shows the result on any page. Text in any script works, and stamps are marked as pagination artifacts. **Remove Page Numbers…** and **Remove All Text** remove only what MyPDF added, even after later edits; Undo restores them. Recent text and each kind's last layout are remembered in `header_footer.json` in the data folder.

Autosave queues a private recovery copy after 5 seconds without a committed change, or after 30 seconds of continued edits. Serialization and disk I/O run on one background worker. The footer distinguishes pending autosave from a completed recovery copy. Originals are never overwritten. These copies are separate from **Save a Copy** and do not mark a document as saved.

On launch, **Document recovery** offers abandoned copies for restoration or explicit discard. Close the window to decide later; **View → Document Recovery** reopens it. Restored documents are unsaved tabs. Saving or explicitly discarding a tab clears its recovery copy. A copy claimed by another running instance is excluded. Storage uses atomic same-directory replacement, file synchronization, SHA-256 validation and restrictive macOS file permissions; corrupt records remain on disk with an explanation.

**Signed PDFs.** A signed document shows a signature bar under the ribbon, with the signer and the overall result. Click a signature on the page, the bar's **Signature panel** button, or the footer's *Signed PDF* pill to open the **Signatures** panel. Each signature lists as *Rev. N: Signed by …* with an Acrobat-style verdict (**VALID**, validity **UNKNOWN**, or **INVALID**) explained in three points (integrity since signing, signer identity, and the source of the signing time), plus details: signing time, key algorithm, signer, issuer, certificate validity dates, and the field and page (click to jump). The actions are **Trust for this session** (no system trust settings change), **Show certificate**, **Export certificate…** (.cer), **View signed version** (opens exactly the bytes that were signed, in a new tab), and **Remove signature…**.

Signed files are protected by default because rewriting a file breaks its signatures' byte ranges. **Edit anyway…** unlocks normal editing after a confirmation. Saving then appends your changes as an incremental update after the signed bytes, so each signature still verifies for the revision it signed, and readers report that the document changed after signing. **Remove signature…** clears the signature from its field, which can then be signed again. Other signatures stay valid, the removal can be undone, and the original file stays unchanged until you Save a Copy. Signing an unlocked document appends both your edits and the new signature. Recovery copies of an unlocked signed document are written the same incremental way.

Recovery lives under `~/Library/Application Support/MyPDF/recovery` (or `MYPDF_DATA_DIR/recovery`). Copies contain PDF content and source path metadata, and are not encrypted separately. Limits: 256 MB per PDF, 2 GB per running session. Recovery preserves the latest completed snapshot, not unsaved typing inside unapplied inspectors, SVG Studio or signing dialogs. A crash before the first snapshot, storage failure, or a long-running export can leave changes newer than the last copy unrecovered. Undo history is not restored.

## Drawing, stamps and Mac dialogs

Pinch over the document to zoom in or out (10–600%). Zoom keeps the PDF point under the pointer anchored; the interface stays the same size. During the gesture, the existing page image scales immediately and a sharper render follows after a short pause. Pinching over a panel does not zoom the document.

**Draw** includes Pen, Polyline, Polygon, Rectangle, Ellipse, Line, Arrow and Double arrow. For polylines and polygons, click to add vertices, then double-click near the last vertex or press Return to finish. Backspace removes the last vertex; Esc cancels. Saved paths are standard PDF PolyLine/Polygon/Line annotations with appearance streams and remain movable, resizable, rotatable and editable in MyPDF. Return commits one undoable change. Finish or cancel a pending shape before saving/closing.

The property panel has a color palette, custom colors, opacity, stroke presets and a collapsible **Position & size** section. SVG color can be overridden without discarding its editable source. Raster artwork keeps its original colors. All artwork supports movement, scaling, rotation and opacity.

**Annotate → Stamps** (or any asset panel’s Stamps tab) contains built-in stamps and your own stamps. Add SVG code directly or import SVG/PNG/JPEG. Use a saved card’s pencil button to rename it, change its collection, edit SVG source or replace its artwork. Duplicate and Remove controls sit below each saved card; **Undo removal** restores the most recently removed item during that session. Updates affect future placements; marks already in documents keep their own content. The Saved collection includes all custom assets. Old libraries load without migration or loss.

**Create with prompt…** opens SVG Studio’s prompt builder. Describe the stamp and choose **Copy prompt**, run the prompt externally in your chosen LLM, and paste its response into SVG source. **Use SVG from pasted answer** strips surrounding prose/Markdown and validates the SVG. Review the preview, adjust the code/name, and save to the Stamps collection. MyPDF makes no LLM requests and stores no API credentials. Static vector SVGs up to 256 KB are supported; unsupported active/external content produces an error.

Confirmation dialogs use rounded cards and full-width actions, with a blue primary action. Return chooses the safe default; Esc cancels. Closing a changed PDF offers **Save a Copy…**, **Don’t Save**, or **Cancel**. Open and Save pickers remain native macOS system dialogs. Save and Close apply pending new text first; a failed text edit keeps the document open.

## Milestone 0: PDF editing and digital-signature proof

The harness proves PDF rendering, actual existing-text replacement, self-signed certificate creation, PKCS#12 (.p12/.pfx) signing, and signing through an opaque macOS Keychain key handle. Generated PDFs are checked with independent Poppler and OpenSSL tools.

The milestone 0 run passed 30 proof checks and 11 integration tests. Final PKCS#12, PFX and Keychain outputs were also inspected in Acrobat, which accepted their signed content and reported the expected unknown trust for self-signed IDs. See [MILESTONE0_RESULTS.md](MILESTONE0_RESULTS.md) for the observed limits and the signing-format restriction.

## Build and reproduce the proof

Requirements: Rust 1.93+, a TrueType font with Turkish characters, and Poppler/OpenSSL command-line tools for the independent proof. The application commands use Rust libraries; external tools are used only for verification.

```sh
cargo build --locked
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo run --locked --bin mypdf-milestone0 -- prove --out artifacts/proof-001 --keychain
cargo run --locked --example annotation_proof -- artifacts/annotations-001
cargo run --locked --example signing_proof -- artifacts/signing-001 --keychain
```

Use a fresh output directory on every proof run. Outputs never overwrite existing files. On macOS the fixture uses the installed Arial font; use `--font /path/to/font.ttf` to select another font. The font is embedded in generated test PDFs and is not included in this source tree.

The Keychain proof creates a separate temporary test store, imports a generated identity, rediscovers it, signs using `SecKeyCreateSignature`, then deletes the store. It snapshots, restores, and compares the Keychain search list. It does not change trusted roots or use your personal signing identity. Run without `--keychain` on other platforms.

## Commands

```sh
# Create, inspect, and render the synthetic PDF.
cargo run --bin mypdf-milestone0 -- fixture artifacts/example.pdf
cargo run --bin mypdf-milestone0 -- text artifacts/example.pdf --page 1
cargo run --bin mypdf-milestone0 -- render artifacts/example.pdf artifacts/example-render

# Replace one complete existing text run in the page content.
cargo run --bin mypdf-milestone0 -- edit artifacts/example.pdf artifacts/example-edited.pdf \
  --find 'Invoice total: 1250.00 EUR' --replace 'Invoice total: 1450.00 EUR'

# Create a personal self-signed RSA identity; passphrase is prompted securely.
cargo run --bin mypdf-milestone0 -- create-id artifacts/personal.p12 --name 'Your Name'
# Add --ecdsa to generate a P-256 identity instead.

# Prepare an UNSIGNED copy for the currently verified PDF signing format.
cargo run --bin mypdf-milestone0 -- prepare-signing input.pdf artifacts/prepared.pdf

# Sign with a .p12 or .pfx identity; passphrase is prompted securely.
cargo run --bin mypdf-milestone0 -- sign-p12 artifacts/prepared.pdf artifacts/signed.pdf \
  --identity artifacts/personal.p12 --field Signature1

# Inspect offline integrity and trust separately.
cargo run --bin mypdf-milestone0 -- verify artifacts/signed.pdf

# macOS: inspect identity metadata, then select a signing identity explicitly.
cargo run --bin mypdf-milestone0 -- keychain-list
cargo run --bin mypdf-milestone0 -- sign-keychain artifacts/prepared.pdf artifacts/keychain-signed.pdf \
  --sha256 CERTIFICATE_SHA256_FROM_LIST --field KeychainSignature
```

For automation, identity commands accept `--password-file` instead of a prompt. Passwords are never accepted as command-line arguments. Protect such files yourself. `--alias` selects an identity in a PKCS#12 file containing multiple identities. `--keychain` selects a specific macOS store. Private-key export is not used by the Keychain signer.

## Verification boundaries

- **Rendering:** Hayro 0.8 handles the generated vector/image/Unicode/cropped/rotated corpus. Comparisons use Poppler at the same resolution and CropBox. This is a small compatibility corpus, not a universal-fidelity claim.
- **Text editing:** replaces one complete page-level `Tj`, `TJ`, `'`, or `"` run. The run keeps its font when the encoding round-trips and its embedded program has every glyph. Otherwise the run is re-set with a matched installed family and per-character fallback (subset CIDFontType2/CIDFontType0 with ToUnicode). It does not reflow paragraphs or descend into Form XObjects. `TJ` replacement normalizes its internal kerning. A fresh content stream isolates other pages sharing the original stream.
- **Signing:** RSA/SHA-256 and P-256/SHA-256 are exercised independently. P-384/SHA-384 is implemented but awaits corpus testing. Output uses `ETSI.CAdES.detached` and signing-certificate binding. Full ETSI profile conformance has not been independently audited.
- **Adobe compatibility:** the candidate writer fails Acrobat byte-range validation when appending a traditional xref update to a PDF using xref streams. The app therefore accepts traditional xref inputs for signing, and provides explicit preparation of an unsigned copy. It rejects existing signed stream inputs rather than rewriting their bytes. Both signature mathematics and Adobe's document-structure validation must pass before expanding this boundary.
- **Identity trust:** self-signed IDs are untrusted by default. Verification distinguishes integrity from explicit trust. No timestamp/revocation service is contacted by this proof.
- **Keychain:** tested using a generated software identity in an isolated store. The adapter currently embeds the leaf certificate only. Complete intermediate-chain discovery, protected hardware keys and provider-specific tokens remain validation gates. The new Windows Personal-store adapter supports CNG and legacy RSA providers in source; native Windows signing has not been executed here.
- **Saving:** edits and exports publish a complete new file; existing paths are preserved. Signed documents stay protected until explicitly unlocked. After that, saves are incremental updates that are checked to keep the signed bytes and every still-applied signature valid. Preparation still rejects signed documents. Additional signatures preserve prior bytes through incremental updates.
- **Runtime:** source code is Rust, with native Apple APIs for Keychain access. Certificate generation/TLS dependencies include ring's compiled cryptographic backend. No Python, Java, browser, or WebView runtime is used.

## Source layout

| File | Responsibility |
|---|---|
| `src/bin/mypdf.rs`, `src/desktop/` | Native desktop viewer, per-tab state, canvas, controls, menus |
| `src/viewer.rs` | Portable background loading, rasterization, glyph search, exact save copies |
| `tests/viewer.rs` | Viewer worker, search geometry, Unicode, save races, signed copies |
| `src/document.rs` | Fixtures, rendering, exact text runs, encoding-safe replacement, unsigned preparation |
| `src/annotations.rs`, `src/pdf_font.rs` | Standard annotation geometry, appearance streams, embedded Unicode text and history |
| `src/assets.rs` | Bounded SVG/raster imports and atomic local asset storage |
| `src/desktop/digital.rs`, `src/desktop/editing.rs` | Native identity, signing, validation, annotation and asset UI |
| `src/identity.rs` | Personal IDs, modern encrypted PKCS#12, key/certificate binding, digest signing |
| `src/keychain.rs` | Native identity discovery and signing; isolated test-store lifecycle |
| `src/signing.rs` | Revision-preserving signing, offline evidence, detached CMS extraction |
| `src/signature_edit.rs`, `src/desktop/signatures.rs` | Signed-PDF unlock, incremental saves, signature fields, signed revisions and removal; the signature bar and details panel |
| `src/header_footer.rs`, `src/desktop/header_footer.rs` | Page numbers, Bates numbers and margin text: placement, marking, removal; the dialog |
| `src/unicode_text.rs` | Font fallback, shaping, bidi, subsetting and ToUnicode for new and edited text |
| `src/proof.rs` | Reproducible evidence and independent renderer/crypto checks |
| `tests/digital.rs`, `tests/annotations.rs` | Certificate profiles/export, rotated signatures, tampering, annotation round-trips and asset persistence |
| `tests/core.rs` | Negative cases, Unicode, shared streams, incremental revisions, save safety |

`artifacts/`, `dist/`, signing identities, and build caches are excluded from version control. Proof identities are synthetic and use the publicly known test passphrase `MyPDF-milestone0-synthetic-only`; they must never be used as personal identities. The milestone 2 proof uses the equally public test passphrase `synthetic-milestone23-password-only`.

## Dependency findings

The PKCS#12 adapter uses `p12-keystore` for modern AES-encrypted PKCS#12 and authenticated imports. It does not use underskrift's legacy PKCS#12 loader. ECDSA uses RustCrypto's prehash signing interface so a digest is signed exactly once; it does not use the candidate library's ECDSA software signer.

`vendor/underskrift` is crates.io release 0.1.4, upstream commit `17bca3c4ac49af475772678f1e9ccd35bfaaa3af`, with three narrowly scoped patches: include the claimed local signing time in PDF `/M`, and calculate trailer `/Size` from object numbers instead of counting rewritten objects, and encode Unicode signature metadata as UTF-16BE PDF strings. These are documented in `THIRD_PARTY_NOTICES.md`. Optional SVT/OpenSSL features are disabled. The `tsp` feature is enabled because upstream's verification module requires it to compile, but no timestamp service is configured.
