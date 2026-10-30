# MyPDF — current handover

Updated 31 October 2026 at 00:53 (Europe/Istanbul). Project root: `/Users/gokcol/SWPROJECTS/Utility/MyPDF/`.

## MyPDF 2.1.7 / build 196 — release in progress

The user added consistent window-drag cursors and dragging from the MyPDF logo text before publication. These changes are being folded into the still-unpublished v2.1.7 draft, with Mac build 196 and Windows version 2.1.7.0. Final release source and package checks will be recorded after this follow-up is committed. Local final checks passed: formatting, Clippy with warnings denied, and the full Rust suite (704 passes, 0 failures, 7 ignored). Four new window-drag regressions and twelve existing tab tests passed; independent review found no blockers. Receipt: `artifacts/release-2.1.7-build196/local-validation.json`. Build 195 packages are local/draft evidence only and must not be presented as the final release.

The Mac currently runs **2.1.7/build 195**, signed/notarized/stapled and installed successfully. Its fresh shutdown baseline was one open document; one reopened with the same reading position, workspace, all eight critical settings files and macOS preferences. Receipt: `artifacts/mac-install-2.1.7-build195/installation-report.json`. Build 194 is retained intact in that installation's rollback directory. Build 196 must preserve this current state again.

Build 195 source was `8247f33d278593811f56c4701ac1dd7ed94821b9`. Windows Rust checks and all three Windows package jobs passed. The first macOS CI attempt failed an unchanged recovery test during immediate lock reacquisition; the same-source macOS rerun passed, with Windows also passing. Isolated lock/process-spawn reproduction found temporary lock retention, and sixteen repetitions of the unchanged recovery suite passed. The exact original cause remains an inference; see `artifacts/windows-touch-performance/validation/recovery-ci-diagnosis.json` and `artifacts/release-2.1.7-build195/rust-checks-attempt1.json`. Final build 196 must pass its own checks.

New release/install evidence belongs in `artifacts/release-2.1.7-build196/` and `artifacts/mac-install-2.1.7-build196/`. Public latest remains 2.1.6 until the final release is verified and published.

## Earlier build 195 implementation and validation

The Windows 11 touchscreen report is being addressed with one-finger Read-mode scrolling, two-finger navigation, deferred navigation persistence, conditional cache trimming and single-pass glyph-outline decoding. The reported laptop has an Intel i7 and 16 GB RAM. Physical Windows touchscreen/pen behavior and end-to-end scrolling performance still require confirmation on that device.

Local validation: `cargo fmt --all -- --check`, `cargo clippy --locked --all-targets -- -D warnings` and `cargo test --locked --no-fail-fast` passed: 700 passes, 0 failures, 7 ignored. Fifteen touch regressions include link taps versus scrolling, pinch anchoring, cancellation, outside contacts, repeated layout passes, protected/draft annotation painting and staggered two-finger eraser navigation. Existing Shift-click and mouse eraser tests pass. Reading-state tests cover idle saving, retry/checkpoint timing, temporary source unavailability and normal-close flush. Independent code review found no release blockers.

Scrolling now defers reading-state writes until 600 ms of quiet with a ten-second checkpoint and retry on failed writes; workspace writes also wait during navigation, and approved close/exit bypasses debounce. Cache candidates are sorted only over budget. Text indexing decodes each glyph outline once. Synthetic optimized text-indexing benchmarks on the Mac host measured 25.13% and 30.12% lower median CPU time for two fonts with identical extracted text/style/geometry; these are not Windows UI speed measurements. Evidence: `artifacts/windows-touch-performance/extraction-benchmark.json` and `validation/final-validation.json`.

Version metadata is 2.1.7, Mac build 195, Windows/Store 2.1.7.0. Source validation, release packaging and the authorized Mac update are in progress; 2.1.6/build 194 remains the installed/public baseline until the new reports verify completion. New evidence belongs in `artifacts/windows-touch-performance/`, `artifacts/release-2.1.7-build195/` and `artifacts/mac-install-2.1.7-build195/`. Do not mistake the previous release results below for validation of 2.1.7.

## Previous completed release: MyPDF 2.1.6 / build 194

**GitHub release:** Published and marked latest on 30 October 2026 at 20:01 Europe/Istanbul — [v2.1.6](https://github.com/gokcol/MyPDF/releases/tag/v2.1.6). **Installed Mac:** **2.1.6/build 194**, installed and running; launch verified at 19:55 Europe/Istanbul at `/Applications/MyPDF.app`.

The user authorized committing/pushing the clipboard and selection fixes, publishing the next GitHub release including Windows, and installing the updated Mac app while preserving settings and open documents. Completion: source commit/push, Mac and Windows GitHub release, and local Mac installation with preservation checks are complete. No new Microsoft Partner Center submission or recurring monitor was requested or performed.

| Item | Release / verified completion state |
| --- | --- |
| Release source | [`48c1a888dff2a8e56321fbdfe36e726546647ce1`](https://github.com/gokcol/MyPDF/commit/48c1a888dff2a8e56321fbdfe36e726546647ce1) — Release MyPDF 2.1.6 with rich clipboard copying and Shift-click selection |
| Source push and tag | Source pushed to `main`; GitHub tag `v2.1.6` verified at the exact release commit |
| Versions | App **2.1.6**, Mac **194**, Windows/Store package **2.1.6.0** |
| Source snapshot | `artifacts/release-2.1.6-build194/source`; all **443 tracked files** match the release commit; private artifacts and unrelated dirty fixture excluded |
| Mac package | arm64; Developer ID team `HXDYTBC6ZT`; App and DMG signed; both Apple notarizations Accepted; both stapled; signature, Gatekeeper, mounted-DMG contents and local recovery-ZIP roundtrip checks passed |
| Mac binary SHA256 | `7345598f1ba5caa66771cd1a8b53b873e0e2ceec2e824c49d712c42cc2a150b0` |
| Windows packages | Installer, portable ZIP and unsigned Store-format MSIX built at the release source; x64 PE version/runtime/licenses, package hashes and unchanged Store identity verified |
| Public assets | All **10** public asset filenames, sizes and GitHub SHA256 digests matched `release-assets.json`; release marked latest |
| Completion handover | Recorded by the follow-up documentation commit on `main`; its hash is saved locally in `release-status.json` after push; documentation commits do not change the pinned release source |

Downloads: [Mac Apple Silicon](https://github.com/gokcol/MyPDF/releases/download/v2.1.6/MyPDF-2.1.6.dmg), [Windows installer](https://github.com/gokcol/MyPDF/releases/download/v2.1.6/MyPDF-2.1.6-Setup.exe), [Windows portable ZIP](https://github.com/gokcol/MyPDF/releases/download/v2.1.6/MyPDF-2.1.6-windows-x64.zip). The unsigned Store-format MSIX is for Store ingestion; the direct Windows downloads are also unsigned. Do not describe GitHub MSIX publication as a Partner Center submission or certification.

## Changes and validation

- Editable rich text copying retains font family, size, bold/italic style, color, relative indentation and line spacing, with a plain-text alternative. Overlapping faux-bold and fill/stroke paint passes copy once; genuine repeated text remains.
- Damaged ligatures are recovered only when exact embedded glyph outlines and advances match a shaped ligature in a locally available font with the same identity. Matching-font availability remains a requirement; ordinary brackets are preserved. No private PDF or newly bundled font was added.
- Standard text selection: drag-select, release the mouse, then hold Shift and click to extend from the original anchor, forward or backward. Shift-drag and clicks over highlights retain that anchor. Selection stays within one page and tab.
- Embedded images and copied regions use native image clipboard data. Successful copy clears stale queued text writes; failed image cut preserves the artwork and selection.
- The earlier search-clear X, saved artwork sizes, page-selection/context improvements and existing settings behavior remain.

The Mac Word end-to-end QA recorded with build 193 produced five nonblank paragraphs without duplicates, Aptos rounded by Word to 13 pt, two bold lines, and correct “efforts” text. The supplied two-page PDF recovered ten malformed `ff` mappings without changing its bytes. The embedded table pasted into Word at **798 × 350 pixels** with exactly matching RGBA pixels. The prior clipboard was restored and existing Word documents were untouched. Receipts: `artifacts/mac-install-2.1.5-build193/validation/{word-text-verification,word-image-verification,ligature-verification,extraction-verification,image-extraction-verification}.json`. This was Mac functional QA; Windows CI/package checks have the scope stated in their reports.

Release-source CI must correspond to commit `48c1a888dff2a8e56321fbdfe36e726546647ce1`:

| Check | Result |
| --- | --- |
| [macOS and Windows Rust checks 41093250461](https://github.com/gokcol/MyPDF/actions/runs/41093250461) | Success on macOS and Windows: formatting, Clippy with warnings denied, and all Rust tests; Mac cleanup/packaging/registration checks also passed |
| [Windows installer/portable build 41093339989](https://github.com/gokcol/MyPDF/actions/runs/41093339989) | Success; installer and portable ZIP plus both checksum sidecars published |
| [Store-format MSIX build 41093344476](https://github.com/gokcol/MyPDF/actions/runs/41093344476) | Success; Store-format package, metadata and checksum published on GitHub only |
| [Test MSIX install/uninstall 41093250469](https://github.com/gokcol/MyPDF/actions/runs/41093250469) | Success, including temporary test-package installation/uninstallation; no test package was published |

Local release validation: Final source formatting and diff checks passed. Native arm64 release build, Developer ID signature, app/DMG notarization, stapling, Gatekeeper, disk-image integrity and app archive roundtrip checks passed. Final release-source test/Clippy results are the successful two-platform CI run above. Earlier build 193 validation and its limits remain in that installation's handover/validation receipts; do not substitute those counts for the final release-source results. Nine synthetic installer scenarios passed, including rollback and rejection of failed notarization/Gatekeeper checks; see `artifacts/mac-install-2.1.6-build194/preparation-report.json`.

## Installation and rollback

Current installation evidence: `artifacts/mac-install-2.1.6-build194/installation-report.json`. Normal shutdown, complete private backup and exact prelaunch data/metadata checks: passed; user data was not overwritten. Running installed binary/signature/notarization verification: passed; installed executable hash matches the notarized release app and a native app window was verified.

Documents before/after restart: **12 / 12**. Document order, reading positions and workspace state: unchanged. Critical settings/artwork/signature/style files and macOS preferences: all eight critical files and macOS preference bytes unchanged. The user had opened additional documents since the earlier build 193 restart; the twelve-document count comes from this installation report.

**Build 194 rollback target: 2.1.5/build 193**, at `artifacts/mac-install-2.1.6-build194/previous-installed.retired/`; original `Contents` and all bytes/metadata: verified and preserved intact. Its binary SHA256 is `cbe00d473ad71ae22aca52ad2adf4905cee14be9ef66aaad28c2fd2362853edc`. It is Developer ID signed but was not notarized. The build 194 private data backup is `artifacts/mac-install-2.1.6-build194/user-data-at-install/`.

For an authorized rollback, quit normally, verify MyPDF is stopped, preserve the current app, and restore the retained bundle directly to `/Applications/MyPDF.app`; follow the current report's signature/hash and targeted registration checks. Do not rename its internal `Contents` or overwrite current user data from the backup. Preserve settings, custom artwork, identities/Keychain material, styles, preferences and recovery state. Never force quit or discard unsaved work.

## Evidence and historical records

All paths below are relative to `/Users/gokcol/SWPROJECTS/Utility/MyPDF/`. Later verified reports supersede historical status fields.

- **Release 2.1.6/build 194:** `artifacts/release-2.1.6-build194/`. Source: `source-verification.json`; package/signing: `mac-build-report.json` and Mac notarization receipts; Windows: workflow/artifact verification reports; publication: `release-status.json`, `release-assets.json`, `SHA256SUMS.txt`, `github-release.json`, and `public-release-verification.json`. Final release-source app archival/unregistration: complete, recorded in `source-app-archive.json`; the generated source app is retained at `mac/MyPDF.app.retired-eb25653ec107/ArchivedContents` with a verified recovery ZIP, and its source/retired/temporary registrations are absent. This archival did not change the installed app or user data. Preserve the original release assets and source snapshot.
- **Historical local 2.1.5/build 193:** `artifacts/mac-install-2.1.5-build193/installation-report.json` and `source-state.json`. Its launch restored seven documents and eight critical settings files; it was signed but not notarized. Its retained build 191 remains at `previous-installed.retired/` with `Contents` intact, hash `9c22a4150b027f7d3ae717859ce7ddfe3bdeace3111bc10b2488fc452bf020a7`.
- **Previous public [v2.1.5/build 192](https://github.com/gokcol/MyPDF/releases/tag/v2.1.5):** release source [`88499a4effec96785a5d2a26b95294a99abe6d9b`](https://github.com/gokcol/MyPDF/commit/88499a4effec96785a5d2a26b95294a99abe6d9b); `artifacts/release-2.1.5-build192/{release-status,public-release-verification,source-app-archive}.json`. Mac app/DMG were notarized and stapled; Windows 2.1.5.0 assets remain historical. Its generated app archive uses `ArchivedContents`; follow that report only for that archive.
- **Historical local 2.1.4/build 191:** `artifacts/mac-install-2.1.4-build191/installation-report.json`. Search-clear implementation was pushed as [`597a7fe641b54fab9cd3d1ea5c33d47c3bdd8e4d`](https://github.com/gokcol/MyPDF/commit/597a7fe641b54fab9cd3d1ea5c33d47c3bdd8e4d); handover as [`189195f`](https://github.com/gokcol/MyPDF/commit/189195f). Retained build 190 is at that directory's `previous-installed.retired/`, with `Contents` intact and hash `edd634ce986b3901534bf869e9fa0f40240a6c05dae135eb472cbf68ab2fb8fa`. The first failed archival attempt and successful recovery are historical reports there.
- **Earlier public [v2.1.4/build 190](https://github.com/gokcol/MyPDF/releases/tag/v2.1.4):** `artifacts/release-2.1.4-build190/release-status.json` and its validation, public-release and Store-submission reports. Older build 189 rollback is described by `artifacts/mac-install-2.1.4-build190/installation-report.json`; its ZIP/`ArchivedContents` recovery is separate from the intact local rollback bundles above.

## Store and workspace boundaries

Store product **MyPDF (OG)**, `9MXR0WLW42XT`: [Partner Center](https://partner.microsoft.com/en-us/dashboard/products/9MXR0WLW42XT/overview). The last submitted package is **2.1.4.0**, submission `1152921505702158150`, last verified **In certification** on 25 October 2026. No present certification/publication status is claimed. Prior 2.1.3 certification was canceled; do not repeat or alter submissions without a new request. The 2.1.6.0 MSIX publication does not change this history.

Preserve unrelated modified `vendor/underskrift/tests/fixtures/sample.pdf`, untracked `.mimosa/`, and `scripts/__pycache__/`. They were excluded from the release source. Keep private source PDFs, fonts, Word QA files, artifacts, backups and credentials out of Git. At completion of 2.1.6, its source/release metadata and handover were pushed to `main`; current 2.1.7 work is described above.

Older handover recovery: `artifacts/release-2.1.4-build190/session-history/HANDOVER-before-refresh.md` and `artifacts/session-handover-2026-10-24/`; old workspace paths there are historical. Do not apply old patches over the current checkout. Ignored local artifacts are absent from a fresh clone.

Keep Finder registration cleanup targeted; do not reset all Launch Services, reactivate archived bundles, strip xattrs or rerun old installers blindly. The host release Swift cache was regenerated after the project move; its older cache remains at `target/release/mypdf-swift-module-cache-before-root-move-20261030/`. User preferences: compact UI, preserved modes/settings, no unsolicited layout changes or competitor comparisons, and no repeated permission requests for already authorized work. No new chat or recurring monitor was requested.
