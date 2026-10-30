# MyPDF — current handover

Updated 31 October 2026 (Europe/Istanbul). Project root: `/Users/gokcol/SWPROJECTS/Utility/MyPDF/`.

## MyPDF 2.1.8 / build 197

**GitHub release (latest):** [v2.1.8](https://github.com/gokcol/MyPDF/releases/tag/v2.1.8), publication time **31 October 2026 at 01:33:12 (Europe/Istanbul)**. **Installed Mac:** **2.1.8/build 197**, running at `/Applications/MyPDF.app`; launch verified 31 October 2026 at 01:26 (Europe/Istanbul). The app is Developer ID signed, Apple notarized and Gatekeeper accepted. Its local offline ticket is not stapled; the published DMG and bundled app are stapled and verified, as is the local recovery ZIP. See the installation distinction below.

The user's final requested window-drag behavior is the **standard arrow throughout hover, press, movement and release**, including the logo, MyPDF text and eligible empty header area. No grab/hand cursor is used for window movement. The branding text remains nonselectable; movement threshold, native release cleanup and tab/button gesture ownership remain. Document-tab reordering keeps its existing separate cursor behavior.

| Item | Verified state |
| --- | --- |
| Release source | [`efcb1c83fe682b10199e9b50e1e096f15062867e`](https://github.com/gokcol/MyPDF/commit/efcb1c83fe682b10199e9b50e1e096f15062867e), committed and pushed; tag `v2.1.8` |
| Versions | App **2.1.8**, Mac **197**, Windows/Store package **2.1.8.0** |
| Source snapshot | `artifacts/release-2.1.8-build197/source`; **444 tracked files** verified against the release commit; unrelated dirty/private files excluded |
| Mac | arm64; Developer ID team `HXDYTBC6ZT`; app and DMG notarization accepted, published DMG/bundled app and local recovery ZIP signatures and staples verified |
| Mac executable SHA256 | `f39d27d81dfe0cc32381c34845924cf43d6271820b03315a5bc3dc9aa1dd70ac` |
| Windows | Installer, portable ZIP and unsigned Store-format MSIX built at the same source; versions, x64 app/runtime, licenses, checksums and unchanged Store identity verified |
| Public inventory | Ten assets verified: Mac DMG/checksum, Windows installer/checksum, portable ZIP/checksum, Store MSIX/checksum/metadata and `SHA256SUMS.txt`; final filenames, sizes and GitHub digests match `release-assets.json` and `public-release-verification.json` |

Downloads: [Mac DMG](https://github.com/gokcol/MyPDF/releases/download/v2.1.8/MyPDF-2.1.8.dmg), [Windows installer](https://github.com/gokcol/MyPDF/releases/download/v2.1.8/MyPDF-2.1.8-Setup.exe), [Windows portable ZIP](https://github.com/gokcol/MyPDF/releases/download/v2.1.8/MyPDF-2.1.8-windows-x64.zip). Direct Windows packages and the Store-format MSIX are unsigned. Publishing the MSIX on GitHub is not a Partner Center submission.

The source commit remains pinned independently of the follow-up documentation commit. Record the completion handover's pushed commit in `release-status.json`.

## Changes and validation

- Read mode supports single-finger touchscreen panning with bounded flick inertia. Mouse text selection and Shift-click extension remain. Two-finger pan/pinch retains its content anchor; taps follow links. Outside contacts, release/cancel, view/mode changes, keyboard navigation and page jumps interrupt or clear navigation appropriately.
- Navigation suppresses accidental selections/marks; repeated layout passes apply movement once. Protected annotations already in the raster are not painted twice, and active inline text remains visible while panning. Stationary touch erasing waits for deliberate movement or tap release so a second finger can initiate navigation without erasing ink. Mouse erasing remains immediate.
- Reading-state persistence waits for 600 ms of quiet, with a ten-second checkpoint and retry after failure. Workspace writes wait during navigation; normal approved close/exit flushes pending state. Cache candidates are sorted only when over budget, and extraction decodes each glyph outline once.
- Window movement now uses the standard arrow as requested after the user's build196 test. Logo, text and empty-strip gestures share threshold and ownership rules. Fullscreen/popup suppression and native macOS pointer-release cleanup remain.
- Rich clipboard formatting, verified ligature recovery, duplicate-paint suppression, image copying, mouse/Shift-click selection and the search-clear X remain. Ligature recovery requires a matching locally available font; no private PDF or extra font was bundled.

Final local checks: formatting, Clippy with warnings denied, **4 window-drag tests and 12 tab tests passed (16 total, zero failures)**. Full Rust suites passed on both platforms in exact-source CI; do not substitute earlier builds' local test counts. The local installer passed 22 synthetic transaction/signature/preparation scenarios, plus three placeholder-rejection checks.

| Exact-source check | Result |
| --- | --- |
| [macOS/Windows Rust checks 41137006543](https://github.com/gokcol/MyPDF/actions/runs/41137006543) | Both platforms passed formatting, Clippy and full Rust tests; Mac cleanup/packaging/registration checks passed |
| [Windows installer/portable 41137054906](https://github.com/gokcol/MyPDF/actions/runs/41137054906) | Success; packages and checksum sidecars verified |
| [Store-format MSIX 41137058113](https://github.com/gokcol/MyPDF/actions/runs/41137058113) | Success; unsigned package, identity, metadata and checksum verified |
| [Test MSIX 41137006410](https://github.com/gokcol/MyPDF/actions/runs/41137006410) | Success, including temporary installation/uninstallation; no test package published |

The reported Windows 11 laptop has an Intel i7 and 16 GB RAM. **Physical Windows touchscreen/pen operation, native cursor appearance and end-to-end scrolling speed remain unverified on that laptop. Visible Mac cursor behavior has not been manually verified.** Headless tests and CI do not establish either native cursor appearance or physical performance. Earlier optimized Mac extraction benchmarks measured 25.13% and 30.12% lower median CPU time for two fonts with equivalent output; these are backend measurements, not Windows frame-rate results (`artifacts/windows-touch-performance/extraction-benchmark.json`).

## Installation, notarization and rollback

Current evidence is under `artifacts/mac-install-2.1.8-build197/`: `installation-report.json`, `post-launch-workspace-assessment.json` and `installed-notarization-verification.json`. Normal shutdown, full private backup and unchanged prelaunch user-data bytes/metadata passed. The running installed executable matches the release hash; strict signature and native-window checks passed. No user data was overwritten.

The baseline contained **1 document** and verification found **2 documents**. The original tab and reading view, original relative order, **8 critical settings/artwork/identity/style files** and macOS preferences were preserved; one additional document was open. **Do not claim full workspace equality:** `home`, `tabs`, `active` and `groups` changed, and `strict_workspace_equality` is false. Use this build197 assessment rather than earlier installations' counts.

Installation initially used the authorized signed local build; later Apple acceptance was verified on the same executable. `codesign --verify --deep --strict` passed and Gatekeeper reported `accepted / Notarized Developer ID`. Sealed app files match the stapled release. macOS blocked attaching the offline ticket under `/Applications` (stapler error **73**, exclusive ticket copy **EPERM**, no file created), so installed **`app_stapled=false`**. Published app/DMG and local recovery ZIP verification has **`app_stapled=true` / `dmg_stapled=true`**. Do not describe the installed bundle as stapled or republish its local-only ZIP. The notarization profile and automatic-review issues were resolved using evidence that the same submitted software comes from the public source; there is **no current public notarization blocker**. Historical rejection/profile reports are superseded, not instructions to retry build196.

**Current rollback:** **2.1.7/build 196**, Developer ID signed **but not notarized**, retained at `artifacts/mac-install-2.1.8-build197/previous-installed.retired/` with original `Contents` intact. Its executable SHA256 is `2fe56fabd1c0074a7f5df2e2091f897eef41cec55d76246b1ced4cfd4aef8715`; original bytes/metadata were verified. Full private backup: `artifacts/mac-install-2.1.8-build197/user-data-at-install/`.

For an authorized rollback, quit normally, verify both process names are stopped, preserve the current app and follow the installation report to restore the intact retained bundle. Do not overwrite current user data from an old backup. Preserve settings, custom artwork, identities/Keychain material, styles, preferences and recovery state; never force quit or discard unsaved work.

The separate **generated build197 source app** is archived at `artifacts/release-2.1.8-build197/mac/MyPDF.app.retired-6d32c66d7169/ArchivedContents`, with verified recovery ZIP `mac/MyPDF.app.backup-6d32c66d7169.zip` and `source-app-archive.json`. The ZIP roundtrip retained a valid staple; exact generated registrations were removed. This archive's `ArchivedContents` restoration procedure differs from the installed predecessor's intact `Contents`. Archiving did not change the installed app or user data.

## Evidence and historical records

Paths are relative to the project root. Ignored artifacts are local and absent from fresh clones; later verified receipts supersede historical status fields. Preparation receipts alone are not completion evidence.

- **Current 2.1.8/build197:** `artifacts/release-2.1.8-build197/` holds source/local/CI verification, Windows/Store provenance, `mac/verification.json`, Apple acceptance receipts, source-app archival, release inventory/publication verification and checksums. Installation evidence is in `artifacts/mac-install-2.1.8-build197/`.
- **Previous 2.1.7/build196:** [v2.1.7](https://github.com/gokcol/MyPDF/releases/tag/v2.1.7), source `f266ab84db9d2bd82acba8136d40894b15066df7`; Windows-only public release and signed, unnotarized local Mac installation. Preserve `artifacts/release-2.1.7-build196/` and `artifacts/mac-install-2.1.7-build196/`. Its installation restored one document/eight files; its intact rollback is notarized **build195**, SHA256 `f216910e12447b9080d6ad6af4c21750adb8786b23810a14514e4f0340a5ec50`. Build196's hand-cursor Mac publication plan is obsolete; do not resume it.
- **Historical local 2.1.7/build195, superseded before publication:** release/install directories `artifacts/release-2.1.7-build195/` and `artifacts/mac-install-2.1.7-build195/`; signed/notarized/stapled, one document/eight files preserved. Its intact rollback is notarized **build194**, SHA256 `7345598f1ba5caa66771cd1a8b53b873e0e2ceec2e824c49d712c42cc2a150b0`. Its generated source archive separately uses `ArchivedContents` and a recovery ZIP.
- **Historical build195 CI race:** `rust-checks-attempt1.json`, `rust-checks.json`, `macos-ci-job.log` in that release directory and `artifacts/windows-touch-performance/validation/recovery-ci-diagnosis.json` preserve the immediate recovery-scan failure and successful same-source rerun. Sixteen unchanged recovery-suite repetitions passed; a lock/child-spawn reproducer supported transient inherited lock retention. The original job did not record its exact lock error; no recovery source change or evidence of data loss was found. These are historical, not current release gates.
- **Previous complete 2.1.6/build194:** [v2.1.6](https://github.com/gokcol/MyPDF/releases/tag/v2.1.6), source `48c1a888dff2a8e56321fbdfe36e726546647ce1`; release/install directories `artifacts/release-2.1.6-build194/` and `artifacts/mac-install-2.1.6-build194/`. Ten assets and notarized Mac verified; twelve documents/eight files preserved. Its intact rollback is signed, unnotarized **build193**, SHA256 `cbe00d473ad71ae22aca52ad2adf4905cee14be9ef66aaad28c2fd2362853edc`.
- **Earlier receipts:** local build193 text/image/ligature Word QA and seven-document/eight-file restoration are in `artifacts/mac-install-2.1.5-build193/`; these are Mac results, not new Windows QA. Public [v2.1.5/build192](https://github.com/gokcol/MyPDF/releases/tag/v2.1.5) receipts are in `artifacts/release-2.1.5-build192/`. Local search-clear build191 evidence is in `artifacts/mac-install-2.1.4-build191/` (source `597a7fe641b54fab9cd3d1ea5c33d47c3bdd8e4d`, handover `189195f`). Public [v2.1.4/build190](https://github.com/gokcol/MyPDF/releases/tag/v2.1.4) receipts are in `artifacts/release-2.1.4-build190/`; its older build189 rollback uses the distinct ZIP/`ArchivedContents` procedure in `artifacts/mac-install-2.1.4-build190/installation-report.json`. Preserve these reports and their original rollback distinctions.

## Store and workspace boundaries

Store product **MyPDF (OG)**, `9MXR0WLW42XT`: [Partner Center](https://partner.microsoft.com/en-us/dashboard/products/9MXR0WLW42XT/overview). Last submitted package: **2.1.4.0**, submission `1152921505702158150`, last verified **In certification on 25 October 2026**. No current certification/publication status is claimed. Prior 2.1.3 certification was canceled. No new Partner Center submission or recurring monitor was requested/performed; GitHub publication of 2.1.8.0 does not alter this history.

Preserve unrelated modified `vendor/underskrift/tests/fixtures/sample.pdf`, untracked `.mimosa/` and `scripts/__pycache__/`. Keep private PDFs, fonts, Word QA files, artifacts, backups and credentials out of Git. No user document paths belong in public handover/release notes.

Older handover recovery: `artifacts/release-2.1.4-build190/session-history/HANDOVER-before-refresh.md` and `artifacts/session-handover-2026-10-24/`; old workspace paths there are historical. Do not apply old patches. Keep Finder registration cleanup targeted: no global Launch Services reset, archived-bundle reactivation, xattr stripping or blind reruns of completed installers. Preserve the pre-move Swift cache at `target/release/mypdf-swift-module-cache-before-root-move-20261030/`.

User preferences remain a compact UI, preserved modes/settings, no unsolicited layout changes or competitor comparisons, and no repeated permission requests for authorized work. No new chat or recurring automation was requested.
