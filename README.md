# MyPDF — native Rust desktop PDF editor

Version 0.95.1 adds a Windows installer and fixes text wrapping in converted documents; 0.95 was the first public release: a signed and notarized Mac app for Apple Silicon with reading, annotation, text and image editing, forms (design, fill, detect, layouts, *My details*), Compare Documents, OCR, watermarks, passwords, Zen mode and digital signatures. Download it from [Releases](https://github.com/gokcol/MyPDF/releases), along with a Windows preview built on GitHub. macOS first, portable Rust core, no WebViews.

> MyPDF was built with the help of Claude, Anthropic's AI assistant, to make everyday business paperwork easier. It is provided as is, without warranty of any kind: keep copies of important documents and use it at your own risk.

## Screenshots

Taken from the running app on macOS, with made-up sample documents (`cargo run --release --example demo_docs -- <folder>` creates them).

**Compare Documents:** two versions side by side, with changed (amber), removed (red) and added (green) words marked and listed.

![Compare Documents showing two versions of a service agreement side by side](docs/screenshots/compare.png)

**Fill with my details:** your saved name, company, email and phone go into the empty fields that ask for them, after a look at what goes where.

![The Fill with my details dialog over a supplier registration form](docs/screenshots/my-details.png)

**Forms, in the dark theme:** the filled form with its fields listed in tab order on the right.

![A filled supplier registration form in Forms mode, dark theme](docs/screenshots/forms-dark.png)

**Annotate:** highlight, underline and strike out text; every mark is listed on the left and its colour, opacity and stroke are on the right.

![A quarterly report with a highlight, an underline and a strike-through](docs/screenshots/annotate.png)

**Home:** open or drop files, start a common task, and pick up recent documents.

![The Home screen with tools and recent documents](docs/screenshots/home.png)

- **Home:** Open/drop PDFs, create a blank PDF, launch common tools, and browse local Recents and Favorites with page previews.
- **Edit:** type directly on blank or existing pages; click existing text to edit it in place; drag a rectangular selection to reflow several lines together; replace, reposition, resize and delete existing page-level images.
- **Forms:** build forms with text, check box, radio, dropdown, list box, button, image, date and signature fields; detect fields on paper-style forms; fill, clear, flatten, set tab order, and import or export data as FDF, XFDF or CSV.
- **Export:** PNG/JPEG ↔ PDF; PDF ↔ Markdown, text and Word `.docx` using text-oriented conversion.
- **Everyday:** print, an outline sidebar, clickable links, password-protected PDFs, reducing file size, recognizing the text of scans (OCR), and watermarks.
- **SVG:** Annotate → Saved → SVG code; paste code, preview, save and place. Select placed artwork to move, resize, rotate, recolor or edit its SVG source.
- **Blank PDF:** File → New Blank PDF (⌘N/Ctrl+N). **Blank page:** Pages → Blank Page.

Every edit is undoable. **Save a Copy** preserves the original and refuses to overwrite an existing file. Word/Markdown conversion does not reconstruct complex layouts, and existing-text editing has font/object compatibility limits. See [milestone 6 results](MILESTONE6_RESULTS.md) and [milestone 5 compatibility limits](MILESTONE5_RESULTS.md).


A native Rust desktop PDF viewer and annotation/signing editor, macOS first and portable to Windows. The interface uses egui/eframe and winit, with Metal on macOS and DirectX 12 on Windows. It contains no WebView, HTML interface, browser renderer, or JavaScript application runtime.

Milestone 1 adds document tabs, a continuous page canvas, lazy thumbnails, Unicode text search with positioned highlights, page navigation, fit modes and zoom, native Open/Save dialogs, drag and drop, and macOS menus. Each tab retains its page, scroll position, zoom and search query during the session. Parsing, indexing, search and PDF rasterization run on background threads.

Milestone 2 adds a digital identity manager, personal certificate creation, authenticated `.p12`/`.pfx` import and encrypted export, public-certificate inspection, native macOS Keychain identity selection, visible/invisible approval signatures, and offline validation. Signature appearance supports a page rectangle, identity name, reason, location, and reusable artwork.

Milestone 3 adds highlights, underlines, strikeouts, pen strokes, notes, Unicode text boxes, shapes, lines/arrows, symbols, stamps, drawn/typed visual signatures, and a local PNG/JPEG/SVG asset library. Select, move, resize, rotate, style, duplicate or delete your annotations; Undo/Redo tracks document edits. Saved annotations use standard PDF subtypes and appearance streams.

Milestone 4 adds a thumbnail page organizer with drag reordering, multi-selection and page ranges, blank/PDF insertion, deletion, extraction, multi-file merging, splitting every N pages, clockwise/counterclockwise rotation, and crop margins with a live preview and reset. Page changes and annotations share one Undo/Redo history. Rendering and text search refresh after each page transaction.

The redesigned interface follows PDF Expert’s compact toolbar and panel layout: dark document tabs, a grouped annotation ribbon, and a fixed right-hand library. **Draw → Arrows / Symbols / Saved** and **Annotate → Stamps / Saved** opens searchable panels containing 33 built-in vector items, including solid directional arrows, double/elbow arrows, chevrons, colored checks and crosses, stars, warnings, information symbols and stamps. Select an item and click the page, or drag the item from the panel straight onto the page (a faint preview shows where it lands). **Select** reveals the inspector. Drag any of the four round corner handles to resize: symbols, stamps, drawings and imported artwork keep their proportions, and shapes do too with Shift. The pointer is an open hand over a movable mark and a closed hand while moving it. Your existing saved artwork remains available.

**Save a Copy** writes the current edited snapshot to a fresh filename, or preserves the exact opened bytes when there are no applied changes. Existing files are never replaced, including concurrent destinations. Unsaved changes prompt on close. Signed/certified PDFs open protected; **Edit anyway…** in the signature bar unlocks them, and every save then appends your edits after the signed bytes (see *Signed PDFs* below). New certificate signatures also append revisions while preserving earlier bytes. Page tools reject encrypted PDFs. Form fields stay with their pages: reordering moves them, deleting a page removes its fields, and inserted or pasted pages bring theirs (a clashing name gets " (2)"). Cropping hides content; it does not redact it. Existing-text editing, form filling and local conversions are available in the Edit, Forms and Export panels.

In **Pages**, click a thumbnail, use ⌘/Ctrl-click or Shift-click for multiple pages, or enter a range such as `1, 3-5`. Drag to the left/right half of a thumbnail to insert the selection before/after it, or use **Move before page**. **Insert** accepts a blank page or PDF files at a chosen position; **Merge** appends PDFs. **Copy pages** (⌘C) copies the selected pages, with their annotations. Switch to any other open PDF and choose **Paste pages** (⌘V) to insert them after its selected page (or at the end); the pasted pages become the selection, and Undo removes them. You can copy from a signed or read-only PDF too; the copies are not signed, so signature fields are left behind. **Extract** saves selected pages. **Split** exports the whole document into a fresh folder, using the interval in the right panel. **Crop** previews displayed-page margins in points and applies them to the selected pages.

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

The Mac bundle is `dist/MyPDF.app`. The bundle script itself installs nothing. The bundle declares that it opens PDFs with the *Alternate* rank, so once it is copied to Applications, MyPDF appears in Finder's **Open With** menu while the default PDF app stays the same. `scripts/release-macos.sh` makes the release: it builds for Apple Silicon, signs with a Developer ID, notarizes and staples the app, and wraps it in a signed, notarized DMG (its header says which settings it needs). On Windows, GitHub Actions lints and tests MyPDF on every push (Recognize Text, Print and Keychain tests are macOS-only), and the hand-run **Windows build** workflow builds `MyPDF.exe` (with its icon and version details), packs it into `MyPDF-<version>-windows-x64.zip`, builds the `MyPDF-<version>-Setup.exe` installer with Inno Setup (`packaging/windows/MyPDF.iss`), and can attach both to a release. The installer needs no administrator rights: it installs for the current user (or for everyone, if chosen), adds a Start menu entry and an uninstaller, and, when its "Open with" option is ticked, lists MyPDF in the Open With menu for PDF files; uninstalling removes that entry again. The Windows app is not yet code-signed, so Windows SmartScreen asks before the first launch; the 0.95 preview has been tried by hand on Windows 11 in Parallels Desktop and works.

Open with the toolbar, **File → Open**, **⌘O / Ctrl+O**, drag and drop, launch paths, **Open With → MyPDF** in Finder or Explorer, or by dropping PDFs on the Dock icon. Files sent to a MyPDF that is already open become tabs in its window. On macOS they arrive as the system's open-documents event. On Windows, the installer adds MyPDF to the Open With list for .pdf when you tick that option (it never changes the default app); the app itself does not change Windows settings. A second MyPDF started with a file hands the file to the open one through a small request in the data folder, then quits. Use **⌘F / Ctrl+F** for search; Enter and Shift+Enter navigate results while the search field is focused. Use the zoom menu, +/−, **⌘0 / Ctrl+0** for 100%, and Page Up/Page Down for page navigation. Ctrl+Tab switches tabs. **⌘S / Ctrl+S** opens Save a Copy; choose a fresh filename. macOS also supports ⌘Shift+S.

The app runs viewing, annotation export, signing and validation locally. It stores local recovery snapshots of applied unsaved document changes; Home stores recent-file shortcuts locally; search queries and undo history are not stored across launches. Reusable visual assets and public identity metadata/file references are saved in `~/Library/Application Support/MyPDF` on macOS and `%LOCALAPPDATA%/MyPDF` on Windows; `MYPDF_DATA_DIR` overrides this for isolated testing. Passphrases and private keys are not saved in those catalogues. Use **Annotate** for markup and stamps, **Draw** for pen/shapes/arrows/symbols, **Edit** for PDF content, and **Saved** for reusable artwork, and **Sign → Digital Signature → Digital IDs** for certificate identities. Visual signatures and certificate signatures are distinct features.

See [MILESTONE1_RESULTS.md](MILESTONE1_RESULTS.md), [MILESTONE2_RESULTS.md](MILESTONE2_RESULTS.md) [MILESTONE3_RESULTS.md](MILESTONE3_RESULTS.md) and [MILESTONE4_RESULTS.md](MILESTONE4_RESULTS.md), [MILESTONE5_RESULTS.md](MILESTONE5_RESULTS.md), and [MILESTONE6_RESULTS.md](MILESTONE6_RESULTS.md) for verification and limits. The implementation sequence is in [ROADMAP.md](ROADMAP.md).

## Workspace, new text and recovery

**Home** is always available in the tab row or with **⇧⌘H**. It includes Open Files, New Blank PDF, tool shortcuts, searchable Recents and Favorites, and first-page previews. Returning Home keeps open tabs and pending text. The local history stores up to 50 recents and 100 favorites; clearing/removing shortcuts does not delete PDFs. Preview generation is asynchronous and memory-bounded (files above 64 MB use a document placeholder).

The top row switches **Read / Annotate / Edit / Draw / Forms / Sign / Pages / Export**. A context ribbon underneath contains the tools for that mode. **Sign ▾** in the main ribbon offers **Signature** (draw/type a visual signature) and **Digital Signature**, which first asks you to drag a box (or click) where the signature goes, then opens certificate signing (.p12/.pfx or macOS Keychain) with that area filled in. Esc cancels. This is the one Sign menu; the Annotate and Draw ribbons hold only their own tools. Zoom, page navigation, Undo/Redo and recovery status share the footer. At narrow window widths, mode labels collapse to accessible icon buttons with tooltips.

Select a drawing by clicking it; click empty page space or the gray surround to clear its selection. Lines and arrows are selected near their strokes, rather than throughout an empty bounding rectangle. A click on existing artwork selects it even with a drawing tool armed; an in-progress polyline/polygon keeps receiving vertices until finished or canceled. The inspector stays in place to avoid shifting the page when selection clears.

**Select several marks** in Annotate or Draw: with the Select arrow, drag across empty page space to draw a rectangle, or choose **Lasso select** and draw a freeform loop. Marks whose center falls inside are selected (the same rule as Edit → Select area). Shift-click adds or removes a mark; Shift-drag adds to the selection; ⌘A/Ctrl+A selects every mark on the current page; Esc clears it. Drag inside the selection to move it, or drag a corner handle to scale it proportionally (rotated marks keep their angle; text boxes scale their font). Arrow keys nudge by 1 pt (Shift: 10 pt), and Delete/⌫ removes the selection. The inspector sets color, opacity, stroke and style for all of them, aligns their edges or centers, and duplicates or deletes them. Each move, scale, alignment, nudge or deletion is one Undo step, and a selection stays on one page.

Drawing properties update immediately: color, fill, opacity, stroke, rotation, position and size need no Apply or Reset. A continuous slider or numeric drag is one Undo step. The **Style** samples offer Solid, Dashed, Dotted, Dash-dot and Wave for native paths, shapes, arrows, pen strokes and vector symbols. Arrowheads remain solid; filled shapes retain a matching outline. Uploaded SVG/image artwork retains its own stroke structure, with its supported color/opacity/geometry properties applied live. With nothing selected, the inspector sets stroke defaults for new drawings. Style and geometry survive Save a Copy and reopening.

**My signatures** (Sign ribbon, Sign menu, or the first tab of the Signatures window) is where everyday signing is set up. A *signature* joins a digital ID with how it looks on the page; you can keep several, such as *Work*, *Initials* or *Approval*. Each one has a name, the digital ID it **signs with**, what it **shows** (your name, with an optional different display name, the date and time, and a note such as “Approved”, which also becomes the signature's reason), an optional **image** (draw or type it, use a picture, or pick any library image) and one of four **backgrounds**: *None* (the page shows through), *White*, *Soft tint* or *Framed*. A live preview shows the result, and every card shows a thumbnail. **Use for Quick Sign** stars the signature Quick Sign uses; **Duplicate** starts a new one from it, and **Delete…** removes only the signature's look after asking: the digital ID and signed documents are untouched. If you have no digital ID yet, **Create my digital ID…** asks only for your name (and optionally an email), creates a personal ID, keeps it in Keychain (or the Windows certificate store) with no password to remember, can mark it trusted on this computer, and attaches it to the signature. Signatures are remembered across launches; the single appearance and default certificate of earlier versions become your first signature.

**Digital IDs** lists the IDs MyPDF can use, grouped by where they live: *On this Mac (Keychain)*, *Smart cards and USB tokens* (any card or token macOS recognises through its driver), *Files* (.p12/.pfx) and *Certificates for checking signatures*. Each shows a plain status: *Ready · valid until …*, *Ready · expires soon* (within 30 days), *Expired* or *Not available right now*, and the one Quick Sign uses carries a *Quick Sign* pill. Selecting an ID offers **Use in a new signature**, **Share certificate…** (the public .cer for people who check your signatures), **Back up…** for files, trust controls (**Trust in MyPDF**, **Trust on this Mac** or **Stop trusting**) and the certificate details. **More options** keeps creating a passphrase-protected .p12 file, adding a public certificate, and choosing another keychain. MyPDF discovers Keychain and token identities in the background and refreshes them when the window opens; missing identities are marked unavailable instead of substituting another certificate. The **Trusted** tab lists the certificates MyPDF trusts, kept between sessions in `trusted_certificates.json` in the data folder. Adobe Acrobat keeps its own list, so people who receive your PDFs trust your certificate in Acrobat themselves. PKCS#12 references remember only the file, alias and certificate, and still require the passphrase when signing. A Windows Current User Personal-store adapter is implemented and its Win32 API types checked; Windows execution and packaging remain deferred.

**Quick Sign** (Sign ribbon, Sign ▾ menu, or Sign → Quick Sign) signs in one gesture with your starred signature: drag a box on the page, or click to place a default-size box. Keychain and token identities sign at once; a .p12/.pfx identity asks only for its passphrase. The signed copy is saved next to the original as *Name-signed.pdf* (then *Name-signed 2.pdf*, …), never replacing a file, and opens in a new tab. Unsaved new documents ask for a filename. Without a usable signature, Quick Sign opens My signatures and says what is missing instead of guessing. **Sign → Digital Signature** first asks for the box on the page, then opens the **Sign** tab: pick any saved signature from its cards, check the preview of the real box, and optionally open **More options** for the reason, location, field name, page and exact rectangle.

**Appearance:** View → Appearance, or the half-circle button at the right of the tab bar, switches between **Match System**, **Light** and **Dark**. Match System follows macOS as it changes; the choice is remembered in `interface.json` in the data folder. Document pages, thumbnails and artwork previews stay white paper in both themes.

**Copy and paste.**
- **Read:** drag across page text to select it, ⌘A selects the page's text, and ⌘C copies it.
- **Annotate and Draw:** ⌘C, ⌘X and ⌘V copy, cut and paste the selected marks onto the current page of any open PDF. Repeated pastes on the same page step aside by 12 pt.
- **Other apps:** ⌘V also pastes their content. A copied image (for example a screenshot) becomes an image mark, and copied text becomes a text box.
- **Edit:** a pasted image is added as real page content, and pasted text opens **Add text** with the words in place. **Add image…** in the Edit ribbon (or **Edit PDF → Add Image…**) places a PNG or JPEG on the current page; Edit → Images then moves, resizes or replaces it.

Toggle the left and right panels independently using the two top-right panel icons, **View**, or **⇧⌘L / ⇧⌘R** (Ctrl+Shift+L/R on other platforms). Closing a panel retains its state. **View → Accessible Page Text** exposes the current page’s extracted text in a selectable native text control.

**Edit → Add text** works directly on blank and populated editable pages. Click the page and type into the on-page text box. The right panel shows a live sample, installed font family and style, size, bold/italic, left/center/right/justified alignment and color. Drag the top handle to move the box; side/corner handles resize it. Click elsewhere on the page, press **⌘Return / Ctrl+Return**, or choose **Done** to commit selectable PDF text and continue adding blocks. Save/Close first applies pending text; invalid or overflowing text stays open with an error. Escape/Cancel discards the uncommitted block. Text in any installed script works: characters the chosen font lacks fall back per character to an installed font that has them (Arial Unicode, Hiragino, Apple SD Gothic Neo, Noto, …), text is shaped with HarfBuzz rules (Arabic joining, Indic and Thai clusters, ligatures), right-to-left runs are laid out in visual order, and only the used glyphs are embedded as font subsets. Complex scripts also carry `/ActualText`, so copy and search return the logical text. Emoji and characters without any installed outline glyph are refused with a message. Existing content stays intact; Undo and Save a Copy work as usual. Text placement respects page crop and rotation. The initial font catalogue offers installed Arial, Times New Roman, Courier New, Georgia and Verdana faces on macOS, with platform alternatives when available. Mixed formatting within one block is not provided. The on-page typing preview does not join Arabic letters or reorder right-to-left text; the committed PDF text is shaped and ordered correctly.

**Edit → Edit text**: click visible page-level PDF text or a MyPDF text box to open its editor directly on the page. Done, an outside click, or ⌘Return commits; Escape/Cancel restores the original. The editor draws a dashed outline with side handles, like PDF Expert, rather than a solid box. Individual PDF runs keep their own font when it can show every character. When it cannot (most PDFs embed only a subset of the glyphs they use, so typing “ö”, “ş”, Cyrillic or CJK into them used to fail), the run is re-set in the closest installed family with the Unicode pipeline above, and the status bar names the substitute font. Scanned text and text inside Form XObjects are not editable yet.

**Edit → Select area**: drag a rectangle around several lines (with Edit text, dragging across empty page space does the same). Text objects whose centers fall within it highlight as you drag; releasing opens one multiline text area. Choose a font, size and color in the inspector, type on the page, and drag the corner to resize. The selection becomes one block with uniform formatting; it does not retain mixed fonts/colors. Applying replaces the selected page text and/or MyPDF text boxes, preserving other content, with a single Undo step. Empty text removes the selection. Overflow keeps the draft open. If unselected text shares positioning with a selected run, include that complete line; the app rejects edits that would shift nearby text. Selection is limited to 2,000 objects/32 KB per page. While the block is open, only the selected words are hidden; table lines, boxes and backgrounds around them stay visible. Clicking outside an untouched selection closes it without rewriting anything. This is content editing; to remove content for good, use **Redact**.

**Redact** (Edit ribbon, or **Edit PDF → Redact**) removes content for good:
- **Marking:** drag over text, images or anything else to mark it (click a mark to unmark it), or use **Find and mark** to mark every match of a word, name or number.
- **Apply Redactions…:** after a confirmation, removes exactly the characters under the marks, including invisible OCR text. It also replaces the image pixels under them, deletes overlapping annotations and form widgets, and draws black or white boxes. The rest of each line stays where it was, and the page remains searchable.
- **Checks:** every result is verified by rendering. If something can't be removed precisely (for example text inside a form object, or an unusual font encoding), MyPDF refuses and suggests **Flatten marked pages to images** instead.
- **Saving:** Save a Copy writes a fresh file without the removed objects, and Undo works until you save.
- **Signed PDFs** can't be redacted, because their signed revision would still hold the original.
- **Limits:** vector drawings under a box are covered rather than removed, and metadata and bookmarks are not changed.

**Forms** builds and fills PDF forms, like PDF Expert's form editor:

- **Add fields:** the toolbar has a tool for each kind: *Text field*, *Check box*, *Radio button*, *Dropdown*, *List box*, *Button*, *Image*, *Date* and *Signature*. Choose one and drag on the page, or click to place it at its standard size. Fields are named as Acrobat names them (*Text1*, *Check Box1*, …). With a radio button selected, the radio tool (or **Add a button**) adds another choice to the same group.
- **Arrange:** click a field to select it, drag it to move, drag a corner handle to resize, Delete/⌫ removes it, and Esc deselects. The whole field, with its value, moves with the pointer, and the page stays on screen while it is redrawn. Each change is one Undo step.
- **Fill:** the right panel shows the selected field's **Value**. Type text, pick a date (written in the field's format, with a **Today** button), choose from a dropdown or list, tick a check box, pick a radio choice, or **Choose a picture…** for an image field. **Sign here** on an empty signature field signs it in place with your starred signature from *My Signatures* and saves a signed copy.
- **Properties:** name, tooltip, required and read-only; for text, several lines, a character limit and default text; for dates, the format (*dd.mm.yyyy*, *mm/dd/yyyy*, *yyyy-mm-dd*, *d mmmm yyyy*, …); for check boxes and radio buttons, the export value, default state and mark (check, cross, circle or square); for dropdowns and list boxes, the choices, their order, a default and whether people may type their own value; for buttons, the caption and what clicking does (clear the form, print, or open a web page). Text size, alignment, text, border and fill colors complete the look. **Apply changes** applies them together.
- **Layouts** places whole structures at once: choose one, adjust it in a small dialog with a live picture, then drag its area on the page (or click for the standard size). A **Table** has columns with a title, a kind (text, number, check box, date, or dropdown with its choices) and a relative width, any number of rows, an optional header row and grid lines; each cell is a field named after its column and row (*Item 1*, *Quantity 1*, …). **Labeled fields** puts a label and a field on each line (a label with *date* gets a date field, *signature* a signature field). **Checklist** puts a check box before each item, **Choice group** a question with radio-button answers side by side or one below another, and **Signature block** a name, date and signature field for each signer. Lines and labels become part of the page; the fields work like any other, and one Undo removes the whole layout.
- **Detect fields** finds where a paper-style form expects answers: blanks of underscores or dots, lines to write on, empty boxes and text areas, and box characters such as ☐. Each field is named after its label, and labels such as *Date*, *Tarih* or *Signature*, *İmza* make date and signature fields.
- **Tab order** sorts each page's fields by rows or by columns. **Show tab order numbers** labels each field with its position.
- **Form data:** **Export data…** saves the values as FDF (Acrobat's format), XFDF or CSV (field names in the first row; spreadsheets open it directly). **Import data…** fills the form from any of them, matching fields by name, accepting choice labels and *yes/true/1* for check boxes, and reports names the form doesn't have and values a field refused.
- **My details** keeps your name, email, phone, company, job title, address, country and website, entered once and stored on this computer only (`my_details.json` in the data folder). **Fill with my details…** finds the empty fields that ask for them, by field name or tooltip in English or Turkish (*Name*, *Adı Soyadı*, *E-posta*, *Cep Telefonu*, *Şirket*, …), plus date fields called *Date* or *Tarih*, which get today's date. It shows what goes where, with a tick to leave any out, then fills them as one Undo step. Fields about someone else (*Father's name*, *Anne Adı*, *Emergency contact*) and other dates (*Date of birth*, *Start date*) are left alone. First and last names come from the full name when not given separately. The first time, it asks for your details.
- **Clear** returns every field to its default; **Flatten** draws the fields into the page and removes them, keeping empty signature fields so the PDF can still be signed there. Both ask first.

These are also in **Edit PDF → Form**. Fields are standard AcroForm fields with their appearances drawn in, so Acrobat, Preview and browsers show them as designed. Date fields carry Acrobat's date formatting and image fields Acrobat's picture chooser; MyPDF reads them but never runs scripts. Fields work on landscape and upside-down pages too, including portrait pages turned with a rotation setting: their appearance turns with the page so it reads upright. XFA forms are not supported.

**Flatten PDF…** (Export panel, or **Edit PDF → Flatten PDF…**) turns every page into an image at 150, 200 or 300 dpi. Annotations, pasted images and form fields are burned in, and document scripts and form definitions are removed, so nothing can be edited or selected any more. Page sizes and rotation are kept, and the flattened PDF can still be **digitally signed**. Already-signed PDFs are not flattened, since that would break their signatures; flatten first, then sign.

**Print… (⌘P)** — File menu or the printer button next to Save — opens the standard macOS print panel with the document as it looks now, including annotations and form values. Large pages shrink to fit the paper.

The left panel's tabs are icons: **Pages**, **Outline**, **Search** and **Marks**, named when you point at them.

**Outline** is a sidebar tab next to Pages: the document's headings (bookmarks), indented by level with their page numbers. Click one to go there; the section you are reading is highlighted. Documents without an outline say so.

**Links** in a PDF work in Read: a link to another page goes there, and a web or email link asks before opening your browser or mail app, showing the address. In the other modes, ⌘-click follows a link, so tools keep working over it. Scripts and links to files are never followed.

**Password-protected PDFs** ask for their password when opened (a wrong one is simply asked again); a **Protected** label shows in the footer, and every copy you save keeps the password. **File → Protect with Password…** saves a copy that opens only with a password you choose (AES-256, the strongest PDF protection; anyone with the password can print, copy and edit). **File → Remove Password…** saves a copy that opens without one; the original keeps its password. Signed PDFs are not protected here, since that would break their signatures. Protected documents are never written to autosave, so their content is not stored unprotected.

**Reduce File Size…** — File menu or the Export panel — offers *High quality* (pictures at about 200 dpi, for printing), *Balanced* (about 150 dpi, for screens and email) and *Smallest file* (about 100 dpi). Photos and scans are scaled down and stored as JPEG only where that makes them clearly smaller; text, drawings and transparency masks are untouched, uncompressed data is compressed and unused objects are dropped. The status bar shows the size before and after, and Undo restores the original.

**Recognize Text…** (Edit PDF menu, the Export panel, or the button under an empty search) makes scanned pages searchable. MyPDF asks first, then reads each page that has no text of its own with the text recognition built into macOS, on this Mac; nothing is uploaded. It prefers Turkish and the main European languages. The words become an invisible layer over the picture, so search, selection and copy work while the page looks the same; the status bar counts pages as they are read. Pages that already have text are left alone. Recognition needs macOS.

**Compare Documents…** (File menu, or the side-by-side pages button next to Print) shows two versions of a document side by side. Choose the older and newer version from the open documents or a file; the open document starts as the newer one. Words that were changed are marked amber on both, removed words red on the older version, and added words green on the newer one, with a thin mark where text was taken out or put in. The list on the right counts and shows every change with its page. Click a change, a mark or **Previous / Next** (↑ ↓) to bring both versions to it. **Swap sides** compares the other way round, and **Done** returns to the document. Only text is compared, line by line and then word by word; scans need **Recognize Text** first.

**Zen mode** (⇧⌘F, the sunrise button at the right of the toolbar, or **View → Zen Mode**) is for reading: the window goes full screen and only the pages remain. The page number appears briefly when it changes; the arrow keys and Space turn pages, and links still work. Move the pointer to the top for a small bar with the way out, or press Esc or ⇧⌘F. Your mode and window size come back as they were.

**Watermark…** (Edit ribbon next to Header & Footer, or Edit PDF menu) puts text such as *CONFIDENTIAL*, *DRAFT* or *COPY*, or a PNG/JPEG picture, across the middle of each page. Choose the colour, a size that fits the page or a fixed one, the opacity, the angle (45°, 0°, 90°, −45° or any other), in front of or behind the page, and all pages or a range; the page preview shows the result. Text watermarks use any script and stay searchable. **Remove watermarks** in the same dialog takes off every watermark MyPDF added, even after later edits, leaving headers and footers alone.

**Header & Footer** (Edit ribbon, or **Edit PDF → Header and Footer** in the menu bar) adds real, selectable text to the page margins:
- **Add Page Numbers…** offers the formats *1*, *Page 1*, *Page 1 of n*, *1 of n*, *1/n* and *- 1 -*, with a start number.
- **Add Bates Numbers…** combines a prefix, a zero-padded number (start and digits) and a suffix.
- **Add Text…** takes any text. A clock menu offers recently used text, and **Field** inserts the page number, page count, date or file name.

For every kind, you choose all pages or a range, the position (top or bottom; left, center or right), margins in millimetres, and the font, style, size and color. The preview shows the result on any page. Text in any script works, and stamps are marked as pagination artifacts. **Remove Page Numbers…** and **Remove All Text** remove only what MyPDF added, even after later edits; Undo restores them. Recent text and each kind's last layout are remembered in `header_footer.json` in the data folder.

Autosave queues a private recovery copy after 5 seconds without a committed change, or after 30 seconds of continued edits. Serialization and disk I/O run on one background worker. The footer distinguishes pending autosave from a completed recovery copy. Originals are never overwritten. These copies are separate from **Save a Copy** and do not mark a document as saved.

On launch, **Document recovery** offers abandoned copies for restoration or explicit discard. Close the window to decide later; **View → Document Recovery** reopens it. Restored documents are unsaved tabs. Saving or explicitly discarding a tab clears its recovery copy. A copy claimed by another running instance is excluded. Storage uses atomic same-directory replacement, file synchronization, SHA-256 validation and restrictive macOS file permissions; corrupt records remain on disk with an explanation.

**Signed PDFs.** A signed document shows a signature bar under the ribbon, with the signer and the overall result. Click a signature on the page, the bar's **Signature panel** button, or the footer's *Signed PDF* pill to open the **Signatures** panel. Each signature lists as *Rev. N: Signed by …* with an Acrobat-style verdict (**VALID**, validity **UNKNOWN**, or **INVALID**) explained in three points (integrity since signing, signer identity, and the source of the signing time), plus details: signing time, key algorithm, signer, issuer, certificate validity dates, and the field and page (click to jump). The actions are **Add to trusted certificates** (kept between sessions), **Show certificate**, **Export certificate…** (.cer), **View signed version** (opens exactly the bytes that were signed, in a new tab), and **Remove signature…**.

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
- **Visible signatures in Acrobat:** appearances use Acrobat's layered structure (`/FRM` with `/n0` and `/n2`), because Acrobat cannot validate a visible signature whose appearance draws an image directly ("Expected a dict object"). Incremental updates repeat the previous trailer's `/Info` and `/ID`, and signature widgets name their page. This was checked in Adobe Acrobat on a real document.
- **Identity trust:** a signer is trusted when its certificate is in your MyPDF trusted certificates, or when the operating system trusts its chain. On macOS, that means the system roots plus any certificate you marked “Always Trust” in Keychain; on Windows, the certificate stores. New self-signed IDs are untrusted until you choose otherwise, and integrity is reported separately from trust. Validation is offline: no timestamp or revocation service is contacted.
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
| `src/redact.rs`, `src/desktop/redaction.rs` | Glyph-level redaction with render verification, image pixel redaction, flatten; the Redact tool and Flatten dialog |
| `src/header_footer.rs`, `src/desktop/header_footer.rs` | Page numbers, Bates numbers and margin text: placement, marking, removal; the dialog |
| `src/unicode_text.rs` | Font fallback, shaping, bidi, subsetting and ToUnicode for new and edited text |
| `src/proof.rs` | Reproducible evidence and independent renderer/crypto checks |
| `tests/digital.rs`, `tests/annotations.rs` | Certificate profiles/export, rotated signatures, tampering, annotation round-trips and asset persistence |
| `tests/core.rs` | Negative cases, Unicode, shared streams, incremental revisions, save safety |

`artifacts/`, `dist/`, signing identities, and build caches are excluded from version control. Proof identities are synthetic and use the publicly known test passphrase `MyPDF-milestone0-synthetic-only`; they must never be used as personal identities. The milestone 2 proof uses the equally public test passphrase `synthetic-milestone23-password-only`.

## Dependency findings

The PKCS#12 adapter uses `p12-keystore` for modern AES-encrypted PKCS#12 and authenticated imports. It does not use underskrift's legacy PKCS#12 loader. ECDSA uses RustCrypto's prehash signing interface so a digest is signed exactly once; it does not use the candidate library's ECDSA software signer.

`vendor/underskrift` is crates.io release 0.1.4, upstream commit `17bca3c4ac49af475772678f1e9ccd35bfaaa3af`, with three narrowly scoped patches: include the claimed local signing time in PDF `/M`, and calculate trailer `/Size` from object numbers instead of counting rewritten objects, and encode Unicode signature metadata as UTF-16BE PDF strings. These are documented in `THIRD_PARTY_NOTICES.md`. Optional SVT/OpenSSL features are disabled. The `tsp` feature is enabled because upstream's verification module requires it to compile, but no timestamp service is configured.

## Code signing policy

Windows releases are to be signed through SignPath's free program for open-source projects (applied for; until it is approved, Windows builds are unsigned). Once approved: free code signing provided by [SignPath.io](https://signpath.io), certificate by [SignPath Foundation](https://signpath.org). The Mac app is signed with the maintainer's Apple Developer ID and notarized by Apple.

- **Committers and reviewers:** [gokcol](https://github.com/gokcol)
- **Approvers:** [gokcol](https://github.com/gokcol)

Every Windows release is built from this repository by the GitHub Actions workflow in `.github/workflows/windows-build.yml`, and each signing request is approved by hand.

**Privacy:** This program will not transfer any information to other networked systems unless specifically requested by the user or the person installing or operating it.

## License

MyPDF is licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your option. The vendored `underskrift` signing library keeps its own BSD-2-Clause license; see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
