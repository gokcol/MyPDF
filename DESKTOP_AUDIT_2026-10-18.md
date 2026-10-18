# MyPDF desktop audit and improvement report

**Date:** 18 October 2026
**Baseline:** v1.11.1, build 120, commit `14e0254b97ced16266bb3b3e623e3742423edd5d`
**Environment:** Apple Silicon Mac, macOS 27.0.1 (26A434)
**Scope:** Desktop functionality, interface consistency, memory, process lifecycle, and test quality. Report first; no application code changes, installation, commit, or release performed during this audit.

## Assessment

The core editing workflows are working in the tested cases. All **263 non-ignored automated tests passed across the initial run and isolated reruns**. Formatting and strict Clippy checks passed. The native walkthrough exercised real editing, saving, forms, stamps, page operations, comparison, printing cancellation, and navigation using synthetic documents.

The most useful next work is a focused consistency and reliability pass: compact context menus, clearer save semantics, predictable inspectors and dialogs, cancellable long operations, and stronger visual regression tests. I did not reproduce data loss or a sustained memory leak in the bounded workload. One native Open dialog became unresponsive; a later deliberate retry succeeded. This remains an investigation item, not a confirmed general Open failure.

The original installed application, user PDFs, settings, and custom stamp library were not modified. The installed app still reports 1.11.0/build 119. Native tests ran against the 1.11.1/build 120 release binary in an isolated copy named **MyPDF Desktop Audit**, with a separate data directory and synthetic files; this was not an installation upgrade. The two process exits the user confirmed were manual are excluded from crash findings.

## Recommended order

| Order | Change | Reason | Size estimate |
|---|---|---|---|
| 1 | Compact context menus and consistent shortcut alignment | Directly reproduced and reported by the user; frequent interaction | Small |
| 2 | Clarify first Save, Save As, and Save a Copy | A successful save should leave an unmistakable document identity | Medium |
| 3 | Fix stale instructions and empty/error inspector states | Removes misleading guidance without changing PDF behavior | Small |
| 4 | Standardize modal sizing, scrolling, and persistent action footers | Makes important actions reachable on smaller windows | Medium |
| 5 | Investigate native dialog hang and add OCR cancellation/progress | Reliability and responsiveness | Medium–large |
| 6 | Improve tab overflow, form editing consistency, and accessibility | Reduces friction during longer sessions | Medium |
| 7 | Expand memory accounting and repair visual test isolation | Prevents future regressions as features grow | Medium |

Sizes are relative engineering estimates, not delivery promises. The native-dialog investigation should begin alongside the first UI changes.

## Detailed findings

### 1. Drawing context menu is unnecessarily wide

**Status: confirmed UI issue. Priority: P2.**

Right-clicking a drawing produced a menu approximately 375 logical points wide at the tested window size. Icons and shortcuts are present, but the large gap between labels and shortcuts makes the menu feel oversized. It also consumes excessive space near page edges.

The row sizing in `src/desktop/ui.rs` uses `max(ui.available_width().min(360.))`, allowing available popup width to drive expansion. `src/desktop/context_menu.rs` supplies the menu layout. This is a concrete place to investigate, rather than shrinking every menu with an arbitrary fixed width.

**Change:** measure the longest label and shortcut once per menu, reserve consistent icon and shortcut columns, and choose a compact width, normally around 260–300 logical points. Allow longer localized labels to expand appropriately. Keep submenus aligned and clamp the complete popup to the viewport. Long menus should scroll instead of extending beyond the screen.

**Acceptance:** drawing, image, selected-text, and blank-page menus remain readable in both themes; shortcut columns align; no clipping near any screen edge; keyboard navigation and submenu opening still work.

### 2. First-save identity and Save As semantics are confusing

**Status: confirmed behavior; product improvement, not observed data loss. Priority: P2.**

A new blank document was edited and saved successfully as `Desktop Audit.pdf`, but its open tab remained `Untitled-1.pdf`. The File menu exposes both **Save a Copy…** and **Save As…**, and both currently dispatch the same `save` action. The save completion path remembers the destination and marks the session saved without adopting that path as the tab identity.

Saving copies preserves source files intentionally. That is useful, but a newly created document has no meaningful original to preserve, and the two menu names imply different behavior.

**Change:** the first save of a new document should adopt the chosen name/path. Define Save As as switching the active document to the destination; retain Save a Copy as an explicit export that keeps the current identity. Keep existing safeguards against accidental source replacement and concurrent changes. Make status text say which file was written.

**Acceptance:** save a blank document, reopen it, edit again, save again, close and restore the workspace. Tab name, suggested destination, recent-file entry, recovery metadata, and dirty state must agree. Cancelling or failing a save must preserve edits and keep the document open.

### 3. Tool instructions and inspector states can be misleading

**Status: confirmed. Priority: P2.**

After leaving Redact, the status area still said to drag over content to redact, even though Edit text was active. With no selection, Annotate can show a **Drawing** inspector. A malformed PDF correctly displays an error and disables most editing actions, but still shows **1 / 0** in page navigation and tells the user to click text on a page.

**Change:** derive instructional text from the current tool and document state. Separate persistent tool guidance from transient success/error messages. Invalid documents should show a focused error state with Open another file, Retry, and Close actions; hide the page counter and irrelevant inspectors. Empty pages should offer Add text prominently.

**Acceptance:** changing tools, tabs, or document validity cannot leave instructions from a previous state. No page counter displays an impossible value.

### 4. Dialogs need a shared layout contract

**Status: observed usability inconsistency. Priority: P2.**

The SVG editor can push Save/Place and Cancel below the initially visible area. Its introductory/prompt content competes with code and preview space. Multiple pages per sheet is tall and dense, with a relatively large preview area and much smaller preset/margin controls. Dialogs and small utility windows use different spacing, button styles, and dismissal patterns.

The dark-mode unsaved-changes confirmation was visually coherent, and Escape correctly cancelled closing. That design can serve as a reference for consistent hierarchy without making every utility dialog equally large.

**Change:** use a shared dialog shell with a bounded scrollable body and a persistent footer. Keep the title, primary action, and Cancel visible at the minimum supported window height. Collapse advanced sections; use a balanced code/preview split for SVG. Standardize control heights, horizontal alignment, focus rings, and spacing.

**Acceptance:** check normal and small window sizes, long filenames, expanded advanced options, both themes, Return/Escape behavior, and keyboard focus. No essential action should require discovering an invisible footer.

### 5. Native Open dialog: investigate an intermittent hang

**Status: one observed hang; not reproduced on a later controlled retry. Priority: P1 investigation.**

During a Go to Folder interaction in the native Open dialog, the isolated app stopped responding to path edits and cancellation and consumed approximately one CPU core. A sample captured the main thread inside `NSSavePanel`/`NSApplication` modal-loop work. The isolated process was terminated after evidence capture. A later test entered a full path step by step and opened the correct PDF successfully. Native Print opened and cancelled successfully too.

This evidence does **not** establish whether MyPDF, the OS dialog, or the automation timing caused the hang. Clipboard paste also failed to replace the selected path on a later attempt while ordinary typing worked, which warrants checking native menu/edit-command integration.

**Change:** build a focused reproduction test for repeated Open/Cancel, Go to Folder, paste, keyboard shortcuts, cloud-provider sidebars, and rapid input. Inspect `menu::with_file_dialog` and the synchronous `rfd` picker calls. Consider asynchronous native panels if the nested modal event loop is implicated; do not replace the native dialog purely on this single observation.

**Acceptance:** repeated opening, typing/pasting paths, cancelling, and reopening must leave normal CPU usage and working shortcuts. No user document should need to be closed to recover.

### 6. OCR needs more honest progress and cancellation

**Status: performance observation and code-inspection finding. Priority: P2.**

The isolated UI OCR test passed in **61.15 seconds**; the integration OCR test passed in **75.07 seconds**. An earlier UI rerun timed out at its 120-second deadline during concurrent preview generation. These are debug test timings including fixture/rendering work, not a release-app OCR benchmark, and do not prove the recognition engine itself takes the whole duration.

The UI currently reports page-level progress and promises that each page takes a few seconds. The progress model contains counters but no user cancellation signal once recognition starts.

**Change:** provide Cancel while work is running, preserve the original until successful completion, show the phase and elapsed time, and avoid a fixed speed promise. Distinguish rendering a scan from recognizing text and applying the searchable layer. Keep long jobs out of the interactive rendering path. Separate slow platform OCR tests from ordinary unit tests, with a realistic deadline and diagnostics.

**Acceptance:** cancellation leaves the source unchanged and releases workers; switching tabs remains responsive; failed or cancelled work cannot partially replace the document.

### 7. Tab overflow needs a visible navigation affordance

**Status: observed at eight tabs. Priority: P2.**

With several long filenames, later tabs and the add/open affordance become difficult to discover in the horizontal strip. The tabs still exist, but horizontal overflow is not obvious.

**Change:** keep the add button visible, add a tab-list menu and subtle overflow controls, and scroll the selected tab into view. Show full names in tooltips and preserve meaningful filename suffixes when truncating.

**Acceptance:** all tabs can be reached with mouse and keyboard at a narrow window width; reopening a closed tab makes it visible immediately.

### 8. Inspector behavior and terminology should be consistent

**Status: observed UI differences. Priority: P2/P3.**

Drawing appearance changes are live, whereas form values and some property changes use Apply buttons. This can be valid for structural operations, but the UI does not consistently communicate which edits are pending. The main ribbon says **Export**, while the panel says **Convert**. Some small field/action controls are considerably shorter than adjacent buttons.

**Change:** live-update appearance settings with one undo step per gesture. Use explicit Apply only where validation or structural changes justify it, clearly mark pending edits, and handle navigation away consistently. Standardize inspector headings, close controls, sections, control sizes, and Export/Convert vocabulary. Keep advanced properties collapsed by default.

**Acceptance:** no apparently edited value silently disappears on selection change; undo granularity remains useful; all drawing and image inspectors follow the same conventions.

### 9. Accessibility needs a dedicated acceptance pass

**Status: partial coverage; no accessibility certification claimed. Priority: P2.**

Many buttons and text controls are exposed in the accessibility tree, including editable PDF text runs. In some native walkthrough steps, accessibility activation of a text field did not focus it while a coordinate click did. This may be an automation limitation and is not yet a confirmed VoiceOver defect.

**Change:** audit keyboard-only navigation and VoiceOver for modal focus trapping/restoration, canvas objects, selected state, icon-only buttons, slider values, and menu shortcuts. Make object names and operations available without requiring pointer precision. Check contrast numerically, not only by eye.

**Acceptance:** complete Open → select/edit → save → close without a mouse; every interactive control has a useful label and focus indication; modal focus returns to its invoking control.

### 10. Memory accounting should include image history

**Status: code-inspection risk, not a reproduced leak. Priority: P2.**

The application already bounds raster caching, queues, and undo history. The PDF revision budget counts unique PDF source buffers, but does not include all annotation image payloads or decoded/GPU representations. Live annotation history is capped by step count, so repeated edits of distinct large images could retain substantially more memory than the PDF-byte budget suggests.

**Change:** account for unique image buffers as well as PDF revisions, enforce a combined history/cache budget, and evict expensive derived data first. Add a repeatable high-resolution image-edit stress test with undo/redo and tab closure. Do not remove useful undo steps simply because shared buffers are counted multiple times.

**Acceptance:** memory stabilizes under repeated image/background-removal/blur operations and returns toward baseline after documents close. Report physical footprint, not only RSS.

### 11. Visual tests currently allow state to leak between scenes

**Status: confirmed test-harness defect. Priority: P1 for trustworthy visual QA.**

The all-scenes preview test reuses one Viewer and resets only some state. After the Remove background scene, later screenshots retained that modal; several filenames therefore did not show the interface they claimed to test. The test itself could still produce a passing result.

**Change:** construct a fresh app/context per scene or explicitly reset all transient state, assert the expected panel/modal is present and unrelated dialogs are absent, and add image comparisons with reviewed baselines. Keep separate tests for actual transitions so isolation does not hide transition bugs.

For this audit, affected scenes were regenerated as separate test processes with separate data directories. FormOrder and FormDrag also depend on FormDesign creating the fixture: FormDrag failed with a missing-fixture unwrap when run alone, then passed when run in the required three-scene sequence. This is another harness dependency, not evidence that dragging a real form field crashes the app. The initial contaminated images are not used as evidence of application UI defects.

### 12. Smaller refinements and unresolved checks

- **Certificate chooser:** automatic Keychain enumeration worked, but code-signing and document-signing identities appear together. Group or explain purposes and make the preferred document identity obvious; do not silently choose an unsuitable certificate.
- **Form placement:** two automated drag attempts did not create a field, while clicking created a default field. This needs an independent reproduction; it is not yet classified as a functional failure. Existing-field filling worked and automated form creation/layout tests passed.
- **Redaction:** the UI explicitly warns that vector content may be covered rather than removed and that other document information remains. Keep that distinction prominent and consider offering a safer flattening path when content cannot be removed reliably. No secure-erasure guarantee is inferred from a black rectangle.
- **Comparison:** the live comparison correctly showed two changed phrases, one added clause, and one removed clause. Keep its “text only” limitation visible; image/layout comparison would be a separate feature.

## Test results and coverage

| Check | Result | Notes |
|---|---|---|
| `cargo test --locked --no-fail-fast` | 260 passed, 3 initially failed, 4 ignored | Initial failures involved macOS system trust/OCR APIs under the execution sandbox |
| System-root trust rerun outside sandbox | Passed | Initial Security framework error was not reproduced |
| OCR integration rerun outside sandbox | Passed, 75.07 s | Searchable output verified by the test |
| OCR UI rerun, isolated from preview generation | Passed, 61.15 s | Earlier concurrent rerun timed out |
| Aggregate non-ignored tests | **263 passed across runs** | Not one entirely green original invocation |
| `cargo fmt --all -- --check` | Passed | No formatting changes made |
| `cargo clippy --locked --all-targets -- -D warnings` | Passed | No warnings |
| Manual visual harness | Reviewed with isolation workaround | Four ignored tests are manual proof/preview tests, not four functional failures |

Automated coverage includes annotation persistence, SVG and stroke styles, transforms, selected-text formatting, Unicode and text blocks, existing PDF image edits, image effects/transparency, region copying, page operations, forms and form data, Markdown/Word/image conversions, n-up, headers/footers/watermarks, signatures using test certificates, protected/signed-document guards, safe saving, recovery, redaction, search/viewer behavior, comparison, and workflow shortcuts.

Native interaction checks included Home/New blank; in-place typing and selected-substring styling; Save to a new PDF; live drawing styles and transforms; dragging and deselection; page insertion/rotation/crop/undo; Read navigation; search across 60 pages; facing pages and panel toggles; stamp SVG import and placement; form filling; signature/certificate managers; export and n-up controls; document comparison; native Print cancellation; malformed-file handling; unsaved-close cancellation; and normal application exit. These are representative end-to-end checks, not exhaustive testing of every possible PDF or every control combination.

### Interface coverage inventory

The review covers the harness’s 66 named screen states, using the uncontaminated initial captures, isolated reruns, and the explicit form setup sequence. Screenshots were inspected as contact sheets, with full-size inspection of selected findings. This supplements the live interaction checks; a screenshot proves layout visibility, not that every action in that screen works.

| Area | Screen states reviewed |
|---|---|
| Home and reading | Home, empty Home, viewer, outline, Zen, workspace, tool search, history |
| Editing | Existing text run, text block, new text, images, region copy, image effects, image crop, headers/footers, watermark |
| Drawing and assets | Draw, inspector, arrows, symbols, standard stamps, SVG Studio, group, lasso, object list, repeat on pages |
| Context menus | Drawing, image, text, blank page |
| Pages and export | Organizer, crop, export/conversion, multiple pages per sheet, flatten, reduce file size |
| Forms | Empty form state, field design, tab order, layout/table, field drag, personal details, fill details |
| Signing | Image signature, background removal, digital identities, signature library/editor, new identity, signing, quick-sign prompt, quick-sign ribbon, trusted certificates, signed-document validation |
| Safety and comparison | Unsaved confirmation, password prompt, protection, external-link prompt, redaction, OCR confirmation, comparison chooser and results |

Useful visual evidence: [drawing context menu](artifacts/desktop-audit/isolated-previews/contextdrawing.png), [SVG Studio](artifacts/desktop-audit/isolated-previews/prompt.png), [sheet layout](artifacts/desktop-audit/isolated-previews/sheetlayout.png), [form drag](artifacts/desktop-audit/form-sequence/formdrag.png), and [image inspector](artifacts/desktop-audit/isolated-previews/imageeffects.png).

## Memory and OS process results

The stress workspace contained eight tabs, including multiple copies of a roughly 45 MiB, 60-page PDF. After closing them, three additional cycles reopened and closed eight tabs each.

| Stage | Measured physical footprint |
|---|---:|
| Eight-tab workspace | 858.4 MB |
| Peak during the workload | Approximately 1.1 GB |
| Immediately after closing all tabs | 157.5 MB |
| After three further open/close cycles | 157.8 MB |

Idle CPU returned to **0%** at the measured quiescent point, with no child processes left behind. The isolated app later exited normally via Command-Q; its PID and child-process queries were empty. RSS alone was misleading because macOS had compressed/swapped memory, so the figures above come from `vmmap` physical-footprint snapshots.

**Conclusion:** no sustained leak or runaway idle process was demonstrated in this workload. This is not a long-duration soak test or proof against all leaks. High-resolution image-history churn, GPU allocations, very large/corrupt document collections, prolonged OCR, external storage disconnects, and sleep/wake remain useful follow-up stress scenarios.

## Coverage limits

- This is a fresh **macOS desktop** audit. Windows native interaction, installation, and memory behavior were not tested here. Earlier release CI is not a substitute for that work.
- iPad and pen hardware behavior are outside this desktop report.
- Recovery files and a recovery offer were observed; automated recovery tests passed. Restoring and inspecting that particular native-session recovery entry was not completed.
- No user private key was used to sign a document. Cryptographic tests used test identities; Keychain enumeration was inspected separately.
- Print dialog opening/cancellation passed; no physical print job was sent.
- VoiceOver, all display scales, every small-window configuration, and every dark-theme dialog have not received exhaustive checks.
- No automatic fix, installation, push, or release is implied by this report.

## Evidence

Audit artifacts are under [`artifacts/desktop-audit`](artifacts/desktop-audit). They are local diagnostic outputs, not application source changes.

- [Original test run](artifacts/desktop-audit/tests.log)
- [Trust rerun](artifacts/desktop-audit/trust-unsandboxed.log)
- [OCR integration rerun](artifacts/desktop-audit/ocr-isolated.log)
- [OCR isolated UI rerun](artifacts/desktop-audit/ocr-ui-isolated.log)
- [Earlier OCR timeout](artifacts/desktop-audit/ocr-unsandboxed.log)
- [Formatting check](artifacts/desktop-audit/fmt.log) and [Clippy](artifacts/desktop-audit/clippy.log)
- [Eight-tab memory](artifacts/desktop-audit/memory-8tabs.txt), [post-close memory](artifacts/desktop-audit/memory-closed-immediate.txt), and [post-cycle memory](artifacts/desktop-audit/memory-after-cycles.txt)
- [Sample from the unresponsive native file dialog](artifacts/desktop-audit/file-dialog-sample.txt)
- [Independent interface previews](artifacts/desktop-audit/isolated-previews)

The time-series CSV includes later interactive work under its last stage label; it should not be interpreted as an uninterrupted idle benchmark. The named footprint snapshots above define the reported stress stages.

---

## Implementation follow-up: 1.11.2, build 121

The sections above record the original audit. Following approval, the changes below were implemented and tested. The installation and verification results are recorded at the end of this follow-up.

### Changes made

| Audit finding | Implementation |
|---|---|
| 1. Context menus | Menus measure their labels, icons, and shortcuts instead of expanding to the available panel width. The reviewed drawing menu is about 230 logical points wide. Shortcut columns remain aligned. |
| 2. Save semantics | First Save and Save As adopt the destination name and path, including workspace persistence. Save a Copy retains the original identity and pending edits. Cancellation, newer edits during saving, deferred text edits, and duplicate save requests are guarded. Existing safeguards against overwriting source files remain. |
| 3. Guidance and invalid documents | Instructions follow the active tool. Empty Edit pages offer Add text; empty Annotate has an appropriate heading. Invalid documents show Open another file, Retry, and Close without impossible page counters or irrelevant inspectors. |
| 4. Dialog consistency | SVG Studio, artwork management, page layout, signature managers, identity creation, and form layout use bounded scrolling bodies and persistent action footers. Control heights and spacing are standardized. |
| 5. Native dialogs | Nested native dialogs now restore the Edit menu only when the outermost dialog exits. Repeated Open/Cancel, Go to Folder typing, Save/Cancel, and Print/Cancel completed without the previous hang. The cause of that one original hang remains unconfirmed. |
| 6. OCR | Added cancellation, phase/page progress, and elapsed time. Cancellation reaches the native recognition request, cancels on shutdown, and prevents a late result from replacing the document. Rendering stops at a safe phase boundary. Removed the fixed-speed promise. |
| 7. Tab overflow | Added a visible tab chooser and persistent Open button. Selected tabs scroll into view; truncation preserves filename endings and tooltips expose full names. |
| 8. Inspector consistency | Form drafts survive selection and tab changes, with visible Review/Discard actions and guards before Save/Close. Export terminology is consistent. Existing live drawing appearance updates remain live. |
| 9. Accessibility | Form controls expose field name, type, page, and selected state, with visible keyboard focus. Arrow keys move fields by one point, or ten with Shift. Native menu command integration and keyboard save/close paths were exercised. This is an improvement pass, not a VoiceOver certification. |
| 10. Memory | History accounting includes unique image and PDF buffers. History is trimmed under per-document and global budgets, image/raster caches are bounded, and image textures clear when all tabs close. Current document contents are never discarded to satisfy a cache budget. |
| 11. Visual tests | Every scene creates a fresh Viewer and egui context. Form scenes create their own fixtures. State assertions now reject captures that show the wrong modal or panel. Reviewed light/dark drawing-menu baselines and optional pixel comparison were added. |
| 12. Smaller fixes | Fixed rapid form drags whose final movement and release arrive in one frame. Certificate identities are grouped by purpose without silently selecting an unsuitable identity. The safer flattening option and limitations of covering vector artwork are more prominent in Redact. |

A concurrent image-effects cache insertion race also surfaced during regression testing: simultaneous cache misses could publish duplicate entries for the same image. Publication now rechecks the key under the cache lock, with a regression test using eight workers.

### Regression results

- **292 non-ignored functional tests passed:** 290 in the main run, followed by two isolated macOS OCR tests. Four manual proof/preview tests are excluded from that count.
- Strict Clippy across all targets and formatting checks passed.
- **67 isolated interface scenes** passed their content/state checks. Representative context-menu, SVG Studio, and sheet-layout captures were reviewed in both themes.
- The new pixel comparison initially exposed popup animation and scroll-position timing in the harness. Explicit virtual time and a settled page position fixed the capture timing without relaxing the comparison threshold. Light and dark drawing-menu comparisons then each passed twice with **0.0000% changed pixels**; the PNGs were byte-identical. A six-scene isolation regression also passed after that harness change. [Baseline evidence](artifacts/desktop-improvements/baseline-determinism).
- Native interaction checks covered opening/cancelling repeatedly, Go to Folder, on-page text entry on a blank PDF, first Save and resulting tab identity, Save As cancellation, native Print cancellation, malformed-file Retry/Close, tab navigation, and normal application exit. Saved synthetic text was also verified from the resulting PDF.
- The two isolated OCR tests completed in 68.74 and 76.83 seconds. These are debug end-to-end test timings, not claims about release recognition speed.

Evidence: [functional suite](artifacts/desktop-improvements/tests-final.log), [OCR UI](artifacts/desktop-improvements/tests-ocr-ui.log), [OCR integration](artifacts/desktop-improvements/tests-ocr-integration.log), [Clippy](artifacts/desktop-improvements/clippy-final.log), and [test counts](artifacts/desktop-improvements/test-summary.json).

### Memory and process follow-up

The additional stress test performed 18 replacements of real 2048 × 960 images with background removal and blur. With a deliberately small 48 MiB history budget, retained data settled at about 42–47.5 MB and four to five history states. Undo/redo remained exact, and all source-image references were released after the document closed.

Physical footprint was 2 MB before that isolated workload, 282.2 MB after churn, and 47.4 MB after close; peak footprint was 329.8 MB. The remaining footprint includes the warm bounded blur cache and runtime allocations. A separate native review session measured 215.1 MB, peaking at 244.0 MB, and returned to 0% idle CPU. Its process disappeared after normal Command-Q.

The production budgets are 384 MiB per-document history, 512 MiB total history, 128 MiB raster cache, and 64 MiB image textures. The 704 MiB managed aggregate is **not an OS memory ceiling**: current documents, PDF decoders, driver allocations, and other bounded caches can require additional memory. These bounded tests found no sustained leak; they do not replace a long-duration soak test.

Evidence: [image-history workload](artifacts/desktop-improvements/image-history-stress.log), [native footprint](artifacts/desktop-improvements/native-footprint.txt), and [idle process measurement](artifacts/desktop-improvements/native-idle-process.txt).

### Remaining verification limits

- The earlier native Open hang did not recur in this pass. Automated Command+A/Paste in native file-panel text fields remained inconsistent, while menu-based Select All and ordinary typing worked. Read-only inspection found correctly wired native selectors and no attached duplicate Edit menu. A physical keyboard check is still needed before attributing this to MyPDF or adding an event-handling workaround.
- Full VoiceOver navigation, cloud-storage provider dialogs, every display scale, and extended sleep/wake testing remain unverified. No Windows or iPad acceptance result is implied by this macOS update.
- Tests used synthetic documents and test signing identities; no user PDF or private signing key was used for document-signing tests.

### Build and installation completed

**MyPDF 1.11.2, build 121** is installed at `/Applications/MyPDF.app`. Both the application and `dist/release/MyPDF-1.11.2.dmg` are Developer ID signed, accepted by Apple's notarization service, and stapled. macOS Gatekeeper accepted the installed application, and its notarization ticket validated successfully.

The installer backed up the previous application and user-data folder, replaced only the app bundle, and verified that **all 10 existing user-data files had identical SHA-256 hashes before and after installation**. This includes the existing settings and artwork library. The installed binary's hash also matches the verified release binary. Launching the installed app restored the previous workspace and dark appearance; About MyPDF displayed **1.11.2 / 121**. No document edit was made during this launch check.

- [Installation record](artifacts/desktop-improvements/installation-result.json) and [verification log](artifacts/desktop-improvements/install.log)
- [Signing/notarization log](artifacts/desktop-improvements/release-notarization.log)
- Local rollback backup: `artifacts/desktop-improvements/install-backup` (private directory containing the old app and user-data backup)

The source changes remain in the working tree. No commit, push, or GitHub release was performed as part of this improvement/install request.
