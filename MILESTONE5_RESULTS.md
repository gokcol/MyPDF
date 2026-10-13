# Milestone 5 — native content editing, forms, conversions and SVG assets

MyPDF 0.5.0 remains a native Rust desktop application using egui/eframe and WGPU. No WebView, browser runtime, remote conversion service, or office automation process is used. macOS is the tested platform; Windows runtime validation remains a milestone 6 gate.

## Features

- **Edit → Text:** choose a page-level text-showing run, edit its actual string, and Apply. The selected operator is addressed directly, including duplicate text occurrences. Existing font/CMap encoding must round-trip every replacement character. Changes regenerate the page content stream and refresh rendering and search.
- **Edit → Images:** select a page-level raster image in the panel or on the page; replace it with PNG/JPEG, change its position and size in points, or delete that occurrence. Shared page content, image resources and inherited resource dictionaries are isolated. Transparency is embedded as a soft mask. Rotated/cropped display coordinates are handled by affine transforms.
- **Forms:** fill standard AcroForm text fields, multiline fields, checkboxes, radio groups and single-selection choices. Values and appearances update together; fields with multiple widgets update every visible widget. Button export names come from the document. Required indicators, maximum-length constraints and read-only metadata are exposed and respected. Values that cannot fit are rejected before commit.
- **Convert:** PNG/JPEG to PDF (one page per image, 144 DPI); selected PDF pages to PNG/JPEG (72–300 DPI); PDF text to TXT, Markdown or editable Word `.docx`; UTF-8 text, basic Markdown and text-oriented `.docx` to a fresh A4 PDF with an embedded Unicode font.
- **SVG Studio:** Annotate → Saved → SVG code opens a native code editor with live preview. Paste or edit SVG, name it and save it to the local library. SVG file import also preserves source. Place, drag, resize, rotate and recolor a placed SVG; open Edit SVG Code from its inspector. Saved PDF appearances retain vector paths and the editable source is retained as MyPDF annotation metadata.
- **Blank documents:** File → New Blank PDF (⌘N/Ctrl+N), welcome screen, or Convert → New Blank PDF. New documents are marked unsaved. Pages → Blank Page inserts a page immediately after the selected page; Pages → Insert also provides a configurable blank page size and position.

## Save and history boundaries

Content, form, annotation and page edits share one undo/redo history. Workers operate on immutable snapshots; a stale result is rejected if the document revision changed. Saving and closing wait for an in-flight content/page operation. Save a Copy writes through a same-directory temporary file and refuses to replace existing output. New conversions use fresh files; image exports stage all pages before publishing a fresh folder. Sources remain unchanged.

Ordinary editing remains disabled for signed, certified or encrypted PDFs. Certificate signing uses the existing separate incremental-signing workflow. Conversion is read-only and does not imply that a signature transfers to a converted file.

## Compatibility limits

- Text editing is one complete page-level PDF text run at a time. It preserves the existing font and text positioning operators; it does not perform paragraph reflow. Newline replacements and unavailable glyphs are rejected. Text inside nested Form XObjects, outlined text, scanned text and OCR are outside this milestone.
- Image editing covers page-level Image XObject invocations. Inline images, images nested inside Form XObjects and editing existing clipping paths are unsupported. Replacement retains the original placement/aspect unless changed in the inspector.
- Form filling preserves the interactive field tree. It does not author new fields or flatten forms. XFA, signature/push-button controls, scripted validation/calculation, password/file-select/comb/rich-text, multi-select and rotated widgets are read-only or rejected. Text widget appearances use a plain white background and an embedded font. Page restructuring of interactive forms remains blocked.
- Text conversions use PDF reading-order extraction, not visual layout reconstruction. Columns, typography, tables, headers/footers, annotations and field values are not reconstructed in Word/Markdown. Scans without text return an OCR explanation. Markdown import supports headings and plain paragraphs; inline Markdown syntax is retained. Word import reads body paragraphs and table text, rejects drawings/embedded objects, and ignores complex layout. Legacy `.doc` is not supported.
- SVG assets support static vectors and outlined text, including paths, gradients, transforms and clipping supported by usvg/svg2pdf. Scripts, HTML, external resources, embedded raster images, CSS style blocks, filters and document entities are rejected. Inline fill/stroke styles are supported. Recolor produces a single color while preserving `none` paints. Native previews are cached rasters; the saved PDF artwork remains vector.
- File, image dimensions, text size, recursion and object counts are bounded. Unusual PDFs still require a broader corpus gate before general release.

## Verification

Automated coverage includes selected duplicate text, stale text detection, existing-font rejection, image replacement/shared-resource isolation, transformed positioning on a rotated page, deletion, field hierarchy and multi-widget appearances, choice validation, radio state exclusivity, read-only/XFA guards, vector-only SVG output, recoloring, SVG input rejection, Markdown/Word text round trips, PNG/JPEG export, no-overwrite output, new blank PDF and page insertion, and the desktop edit/undo/reopen event loop. Earlier signing, annotation, page-management and viewer tests remain in the suite.

Proof files and logs are in `artifacts/milestone5-proof`. Poppler renders and pypdf inspect the outputs independently; python-docx reopens the generated Word document. The shared second page retains its original text, all filled widget values and appearances agree, and SVG output has no Image XObjects.


### Final macOS verification

- 54 automated tests passed; three manual proof/preview tests are ignored in the regular suite. Clippy with warnings denied and formatting checks passed. The packaged bundle passed `codesign --verify --deep --strict`.
- Native UI: selected existing invoice text, replaced 1250.00 with 1650.00, replaced the image and resized its width from 80 to 130 points, and saved `native-edited.pdf`. The other page retains its original shared content.
- Native UI: filled two linked Name widgets, the Accept checkbox, Priority radio group and Country choice, then saved `native-filled.pdf`. pypdf and Poppler confirmed matching values and appearances.
- Native UI: created a blank PDF with ⌘N; pasted an orange SVG; saved it to the library; dragged, resized, recolored and edited its source; inserted a blank page; saved `native-blank-svg.pdf`. Its stored rectangle is approximately 306.41 × 171.88 points, source edits and tint survive reopening, and the saved appearance remains vector.
- Native UI: exported `native-export.docx` and imported it as `native-word-to-pdf.pdf`. python-docx and pypdf independently verified the edited text. The macOS Word Save dialog issue was resolved by using a normal filename dialog and enforcing the chosen output extension in the application.
- `native-verification.json` and `independent-verification.json` record logical checks; rendered proof PNGs confirm the visible outputs. Existing user assets were preserved; the native SVG proof asset was added to Saved.
