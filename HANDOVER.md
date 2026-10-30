# MyPDF — current handover

Updated 30 October 2026 (Europe/Istanbul). Workspace: `/Users/gokcol/SWPROJECTS/Utility/MyPDF/`.

## GitHub release in progress — 30 October 2026

The user authorized committing/pushing the clipboard and selection fixes, publishing the next GitHub release, and updating the local Mac installation. This source prepares **2.1.6, Mac build 194, Windows/Store version 2.1.6.0**. Publication and the build 194 installation are not yet complete. The existing installed build 193 and current public v2.1.5/build 192 records below remain authoritative until superseded by verified release/install reports.

Release work is under `artifacts/release-2.1.6-build194/`. Package only the committed source snapshot; exclude private PDF/Word test material, fonts, user-data backups, the unrelated modified fixture and untracked local folders. No new Microsoft Store submission is authorized by this GitHub release request.

## Current local build — 30 October 2026

**MyPDF 2.1.5/build 193 is installed and running at `/Applications/MyPDF.app`.** It fixes rich text copying to Word, repeated faux-bold text, verified recovery of damaged `ff` ligature mappings, standard Shift-click text selection after releasing the mouse, and image clipboard preservation. The app is Developer ID signed by the existing team `HXDYTBC6ZT`, signature/hash verified, and **not notarized**. Its binary SHA256 is `cbe00d473ad71ae22aca52ad2adf4905cee14be9ef66aaad28c2fd2362853edc`.

Launch was verified at 18:01 Europe/Istanbul. All **seven open documents**, their order and reading positions were restored. Workspace state, eight critical settings/artwork/signature/style files, and macOS preferences were verified unchanged. Normal shutdown and a complete verified private backup preceded replacement.

The build 193 fixes and this handover are included in the 2.1.6 release preparation, based on `01261168d996319b73e9e8ae30d8e81f11e98c31`. Build 193 remains installed while the 2.1.6/build 194 packages are prepared. Public v2.1.5/build 192 remains the latest verified release until publication completes.

Current local evidence is `artifacts/mac-install-2.1.5-build193/`: `candidate-report.json`, `source-state.json`, `installation-report.json`, and `validation/`. The original installed **2.1.4/build 191** is retained at `previous-installed.retired/` with its original `Contents` intact; the private data backup is `user-data-at-install/`.

## Published GitHub release — 30 October 2026

**[MyPDF v2.1.5](https://github.com/gokcol/MyPDF/releases/tag/v2.1.5) is published and marked latest**, verified at 10:57 Europe/Istanbul. The tag points to release-preparation commit [`88499a4effec96785a5d2a26b95294a99abe6d9b`](https://github.com/gokcol/MyPDF/commit/88499a4effec96785a5d2a26b95294a99abe6d9b). It includes the search-clear button on Mac and Windows. Mac release build **192** is Developer ID signed, notarized and stapled. Windows installer, portable ZIP and Store-format MSIX **2.1.5.0** are available. All ten public assets were verified by filename, size and GitHub SHA-256 digest against the local release manifest.

The initial updated handover was committed and pushed as [`189195f`](https://github.com/gokcol/MyPDF/commit/189195f), followed by the release-preparation commit above. This handover now records the completed publication; documentation commits after the release source do not change the pinned release tag.

Authoritative public release records are under `artifacts/release-2.1.5-build192/`: `release-status.json`, `github-release.json` and `public-release-verification.json`. Those records describe the published build 192 artifacts; the later local build 193 installation is recorded separately above. The 2.1.5.0 MSIX was published to GitHub only; no new Microsoft Partner Center submission or recurring monitor was performed.

## Historical build 191 installation and source push — 30 October 2026

Build **2.1.4/191** introduced the one-click X inside the search field. Nine search tests and formatting passed, and the optimized app was Developer ID signed and installed locally without notarization. Its source and Mac build number were committed and pushed to GitHub `main` as [`597a7fe641b54fab9cd3d1ea5c33d47c3bdd8e4d`](https://github.com/gokcol/MyPDF/commit/597a7fe641b54fab9cd3d1ea5c33d47c3bdd8e4d) — `Add one-click search clearing and bump Mac build to 191`. That commit changed `src/desktop/mod.rs` and `packaging/Info.plist`; later handover and release-preparation commits followed.

Historical installation evidence remains at `artifacts/mac-install-2.1.4-build191/installation-report.json`. Its signed binary SHA256 is `9c22a4150b027f7d3ae717859ce7ddfe3bdeace3111bc10b2488fc452bf020a7`. That earlier restart restored six documents and their reading positions; additional documents opened afterward were retained. Eight critical settings files and macOS preferences were verified unchanged. **Build 193 now replaces this installation.**

The older private backup remains at `artifacts/mac-install-2.1.4-build191/user-data-at-install/`. The signed/notarized build 190 original remains at `artifacts/mac-install-2.1.4-build191/previous-installed.retired/`, with `Contents` intact; macOS prevented changing its internals. `installation-attempt1.json` records the unsuccessful initial archive attempt, and that directory's `installation-report.json` records recovery and the successful build 191 installation. These are historical recovery records, not build 193's current rollback instructions.

The project move required regenerating the host release Swift compiler cache. Its old cache is retained at `target/release/mypdf-swift-module-cache-before-root-move-20261030/`. Build 193 used the native host packaging path (`sh scripts/bundle-macos.sh --output ...`) rather than the separate explicit-target cache. Preserve unrelated modified fixtures and local files listed below.

## Current task and authority

The current request is to fix PDF text copying/format preservation, image copying, and standard Shift-click selection: drag-select text, release the mouse, then hold Shift and click another point. These fixes are implemented, tested and installed locally as build 193 while preserving settings and open documents. The user subsequently authorized their commit, push, GitHub release and a new local Mac installation; see release preparation above.

Earlier requests for the search-clear button, build 191 installation, source/handover pushes and the next GitHub release including Windows are complete. The original handover update was pushed as `189195f`; public GitHub release v2.1.5 remains the completed build 192 release.

The earlier public MyPDF 2.1.4 release remains build 190: GitHub publication and Microsoft Store submission were completed on 25 October. Both published Mac app and DMG were signed, notarized and stapled. Store submission `1152921505702158150` was **In certification** at the last verification on 25 October; its present certification/publication state has not been rechecked. GitHub v2.1.5 is published. No monitoring automation or further Store submission was requested; the subsequent local build 193 update is separate from that public release.

Use these records for their respective scopes:

- **Current published release:** `/Users/gokcol/SWPROJECTS/Utility/MyPDF/artifacts/release-2.1.5-build192/release-status.json`.
- **Current Mac installation:** `/Users/gokcol/SWPROJECTS/Utility/MyPDF/artifacts/mac-install-2.1.5-build193/installation-report.json`.
- **Previous Mac installation:** `/Users/gokcol/SWPROJECTS/Utility/MyPDF/artifacts/mac-install-2.1.4-build191/installation-report.json` (historical).
- **Published build 190 release:** `/Users/gokcol/SWPROJECTS/Utility/MyPDF/artifacts/release-2.1.4-build190/release-status.json`. Its installed-build fields describe the earlier build 190 installation; current installed state is recorded in the build 193 report.

This handover is a point-in-time record. Later status/verification reports take precedence. Historical reports may retain earlier values: for example, build 190's `mac-build-report.json` says `notarized: false` because it predates the accepted app/DMG notarization reports. Local builds 191 and 193 are separate from the public packages and are not notarized.

## Versions and distribution

| Item | Current verified or explicitly recorded state |
| --- | --- |
| Search-clear implementation commit (pushed) | `597a7fe641b54fab9cd3d1ea5c33d47c3bdd8e4d` — Add one-click search clearing and bump Mac build to 191 |
| Current published release source | `88499a4effec96785a5d2a26b95294a99abe6d9b` — Prepare MyPDF 2.1.5 release for Mac and Windows |
| Current release tag | `v2.1.5`, verified to point to `88499a4effec96785a5d2a26b95294a99abe6d9b` |
| Current clean release snapshot | `/Users/gokcol/SWPROJECTS/Utility/MyPDF/artifacts/release-2.1.5-build192/source`; all 440 tracked files matched the commit |
| Repository/local versions | Repository prepares app 2.1.6/Mac 194/Windows 2.1.6.0; installed Mac is 2.1.5/193; latest verified public release is 2.1.5/Mac 192 |
| GitHub 2.1.5 | [Published release](https://github.com/gokcol/MyPDF/releases/tag/v2.1.5), latest; all ten assets verified; published 30 October at 10:57 Europe/Istanbul |
| Previous GitHub 2.1.4 | [Published release](https://github.com/gokcol/MyPDF/releases/tag/v2.1.4), all ten assets verified; published 25 October at 23:28 Europe/Istanbul |
| Earlier GitHub release | [v2.1.3](https://github.com/gokcol/MyPDF/releases/tag/v2.1.3), Mac build 187 |
| Current published Mac release (build 192) | Developer ID signed, app and DMG notarization **Accepted**, stapled; signature, Gatekeeper and DMG integrity verified |
| Installed Mac | `/Applications/MyPDF.app`, **2.1.5 build 193**, Developer ID signed, signature/hash and running executable verified; not notarized; original build 191 retained intact for rollback |
| New GitHub Store-format asset | `MyPDF-2.1.5.0-store-x64.msix`, x64, unchanged Store identity/publisher; verified and published on GitHub, not submitted to Partner Center |
| Previously submitted Store package | `MyPDF-2.1.4.0-store-x64.msix`, verified x64, unchanged Store identity/publisher; uploaded, validated, saved as the only package and submitted |
| Store product | MyPDF (OG), `9MXR0WLW42XT` |
| Last recorded Store submission | `1152921505702158150`, **In certification**; submission complete, pre-processing in progress, automatic publication after passing certification; verified 25 October at 23:33 Europe/Istanbul |

The Store MSIX is intentionally unsigned for Store ingestion/Microsoft signing; do not misdescribe it as an independently signed consumer package. The separate MSIX install/uninstall CI test passed with its test package.

## Completion and external review

The local build 193 fixes, installation and preservation checks are complete; the authorized 2.1.6 release and build 194 installation are in progress. Earlier build 191 work and the public v2.1.5/build 192 release are complete. Microsoft certification for 2.1.4.0 was pending at the last recorded check on 25 October; no current status is claimed. At that check, the 2.1.3 certification had been canceled, 2.1.4.0 was the only submitted package, and final Submit for certification had succeeded. Do not repeat or change that submission without a new request. No monitor has been requested.

For the historical 2.1.4.0 Store submission, the What's New text and reviewer notes were saved, reopened and verified; existing restricted-capability explanations and automatic-publication settings were preserved. The generated build 192 app was archived and unregistered after packaging; its retained original has `ArchivedContents`, and a verified ZIP is also available. Use `artifacts/release-2.1.5-build192/source-app-archive.json` for recovery. This is separate from both local installation rollback directories: build 193 retains build 191, and build 191 retains build 190. Both of those original `Contents` directories must remain intact.

Partner Center: [product overview](https://partner.microsoft.com/en-us/dashboard/products/9MXR0WLW42XT/overview).

Do not inspect unrelated browser pages, conversations or user data. If another private page becomes foreground, stop Store UI interaction and wait for the intended page. No monitoring automation has been requested.

## Included improvements

Local build 193 adds:

- Editable rich text plus a plain-text clipboard alternative, retaining the PDF's font family, size, bold/italic style, color, relative indentation and line spacing. Page selections and inline text copying use the same corrected text/style metadata.
- Duplicate paint-pass detection: overlapping faux-bold/fill-and-stroke glyphs copy once, while genuinely repeated text remains. Font metadata is bounded before per-glyph storage, and clipboard markup is escaped.
- Damaged ligature recovery only when the embedded glyph's exact outline and advance match a shaped ligature in a locally available font with the same exact identity. The supplied two-page PDF recovered ten malformed `ff` mappings; ordinary brackets remain unchanged. This depends on the matching font being available. No private PDF or newly bundled font was added, and the source PDF was not modified.
- Shift-click after releasing a drag selection extends from its original anchor, forward or backward, including clicks over a highlight annotation. Shift-drag uses the same anchor. Selection remains within one page and tab.
- Native image clipboard output for embedded images and copied regions. Successful image copy removes stale queued text writes that would replace the bitmap at frame end. Image alpha/pixels and editable internal paste are covered by regression tests; a failed image cut preserves the selection and original artwork.

The search-clear feature was committed on GitHub `main`, introduced in local build 191, included in published v2.1.5/build 192, and retained in build 193: a clear X inside the right edge of the document search field when text is present. One click clears the query and matches, cancels stale search results through the existing invalidation path, and returns keyboard focus to the field. Match-case and whole-word settings remain unchanged.

The earlier refinements included in the published 2.1.4/build 190 release remain:

- Stable annotation text layout and inline editing at reduced zoom; smoother filled shapes, mirrored text and image artwork.
- Each stamp/checkmark/saved ink signature remembers its explicitly chosen size in PDF points, across launches, while fitting smaller pages proportionally.
- Thumbnail/page-sorter multiselection and context actions: copy/cut/paste before or after, duplicate, rotate, extract and delete; cross-document page copying retains annotations.
- Page shortcuts respect thumbnail focus and return to normal text/artwork behavior after a document click. Pending text/table/form edits block page changes, copies and exports so drafts are not silently omitted.
- Sidebar thumbnail navigation preserves Back history; right-click selection does not move the reading position.
- The duplicate toolbar **Find** icon was removed. Sidebar **Search**, Edit → Find and Command/Ctrl+F remain.

Existing Turkish/nonembedded-font fixes and durable Finder Open With cleanup are retained. Installed-font fallback still depends on available fonts and valid PDF character mappings; do not claim universal language support.

## Evidence and validation

Current **local build 193** evidence is under `artifacts/mac-install-2.1.5-build193/`:

- `source-state.json` records the base commit and SHA256 of each changed source/build file. `candidate-report.json` pins the signed arm64 binary and records Word text/image paste and Shift-click checks. `installation-report.json` confirms the installed/running build, original bundle preservation, private data backup, seven restored documents and settings checks.
- `validation/tests.log`: full test run **675 passed, 0 failed, 7 ignored** before the final ligature/context refinements. Final affected tests passed afterward: ligature recovery 2, `viewer_copy` 7, viewer 10, pointer interaction 4, clipboard 9, selection 6, context 41 and image-region 3. The four clipboard HTML tests also passed. The focused clipboard/selection/context/image logs are retained in `validation/`.
- `validation/clippy.log`: final `cargo clippy --locked --all-targets -- -D warnings` passed. Formatting and diff checks passed. The optimized native arm64 bundle built successfully. Five synthetic installer success/failure scenarios verified data preservation and intact bundle rollback; see `preparation-report.json`.
- `validation/word-text-verification.json`, `extraction-verification.json` and `ligature-verification.json`: normal paste into a scratch Microsoft Word document produced five nonblank paragraphs without duplicates, Aptos at Word's 13-point rounding of the 12.80195-point PDF size, and two bold lines. Both selected occurrences of “efforts” were correct; no malformed `e[orts` remained. Recovery used matching font/glyph evidence rather than spelling guesses.
- `validation/word-image-verification.json` and `image-extraction-verification.json`: the embedded table image pasted into Word at **798 × 350 pixels**, with RGBA pixels exactly matching the extracted image. The original clipboard was restored; existing Word documents were untouched.

These are local Mac checks for build 193. No new Windows build/CI run or public release was performed for these fixes. Historical build 191 evidence remains under `artifacts/mac-install-2.1.4-build191/`; its nine search tests and earlier installation checks do not describe build 193. The separate public release-source checks are recorded below.

Current **v2.1.5/build 192 release** evidence is under `/Users/gokcol/SWPROJECTS/Utility/MyPDF/artifacts/release-2.1.5-build192/`:

- `source-verification.json`: all 440 tracked files match release commit `88499a4effec96785a5d2a26b95294a99abe6d9b`; the unrelated dirty PDF fixture is excluded.
- `validation-logs/github-checks.json`: Rust desktop checks [41021430254](https://github.com/gokcol/MyPDF/actions/runs/41021430254) succeeded on macOS and Windows, including formatting, Clippy and tests; macOS packaging/retention checks passed.
- `windows/workflow-verification.json`: Windows build [41021588662](https://github.com/gokcol/MyPDF/actions/runs/41021588662), Store MSIX build [41021591941](https://github.com/gokcol/MyPDF/actions/runs/41021591941), and test MSIX [41021430463](https://github.com/gokcol/MyPDF/actions/runs/41021430463) all succeeded at the exact release source. The test MSIX installation/uninstallation passed; that test package was not published.
- `windows/verification.json`, `windows/artifact-summary.json`, `store/verification.json`: package checksums, x64 runtime, version, search-clear feature markers, and unchanged Store identity/publisher verified. This is static artifact verification plus the CI test-identity MSIX install/uninstall, not a consumer installer/launch test or Store certification.
- `mac/verification.json`, `mac-build-report.json`: release version 2.1.5/build 192, signing, notarization, stapling, Gatekeeper and DMG integrity checks passed. App notarization request `7b724290-9a50-4f2b-a7a4-a2cc38e1bcd9` and DMG request `45b36705-9ff4-46d3-8fd5-362360856ddd` are Accepted.
- `release-assets.json`, `SHA256SUMS.txt`, `github-upload-verification.json`: manifest and checksum verification for all ten public assets. The four primary downloads are the Mac DMG, Windows installer, Windows portable ZIP and Store-format MSIX.
- `github-release.json`, `public-release-verification.json`: public/latest release, correct tag/source, all ten public asset names, sizes and SHA-256 digests verified. Draft receipts remain historical intermediate records.
- `source-app-archive.json`: verified ZIP round trip and retained original at `mac/MyPDF-build192.retired/ArchivedContents`; generated app paths unregistered without changing the installed app or user data.

Current release SHA256 values:

| Artifact | SHA256 |
| --- | --- |
| Mac DMG | `967c8732d02f767ebc47f2acad9946935e57c8dc4641a2d2c138089b25c90f42` |
| Windows installer | `9416ff18bec17ef699b7f7d069fbdf3c7b52aea72ea54c9571079e6a99708566` |
| Windows portable ZIP | `c07272f7fcfe466173c518928bf05c217bdc70774bd3ac31fbc021fa6a2a0c4e` |
| Store-format MSIX | `ad40865bf7b2ce4a97e730cb7543050dc06da6fe4d6002e1399e6d2b95f5c576` |
| Mac app binary | `e26d0b4063ba73df5bff8914afa54758a7dcf7b5adeac2c65e734892521bc8bf` |

The following historical evidence belongs to the **published build 190 release**, rather than v2.1.5, under `/Users/gokcol/SWPROJECTS/Utility/MyPDF/artifacts/release-2.1.4-build190/`:

- `release-status.json`: authoritative progress and publication flags.
- `source-verification.json`: clean source commit and tracked-file equality.
- `validation-logs/validation-summary.json`: **653 Rust tests passed, 7 ignored**, formatting and Clippy passed; **55 Python checks passed**. Three macOS system-service tests initially failed inside the sandbox and passed on exact elevated reruns using the isolated validation profile.
- `workflow-status.json` / `workflow-provenance.json`: successful Windows build `40229617459`, Store package build `40229619719`, macOS+Windows checks `40229617103`, MSIX install/uninstall `40229617166`, all for the release commit.
- `windows-artifact-verification.json`, `windows/verification.json`, `store/verification.json`: hashes, versions, runtime files, retained feature markers, and unchanged Store identity.
- `mac/app-notarization.json`: Accepted, request `955d90c5-0e8b-4863-81ea-21c9d892905a`.
- `mac/dmg-notarization.json`: Accepted, request `1cce97cf-3154-46c4-aed3-6e73ad8f737c`.
- `github-release.json`, `public-release-verification.json`, `github-upload-verification.json`: completed public release and ten-asset verification. `github-draft*.json` files are historical intermediate records.
- `store-submission-verification.json`: validated/saved 2.1.4.0 package, reopened listing text, successful submission and actual certification state.
- `source-app-archive.json`: release-source app archival/recovery evidence.
- `release-notes.md`, `store-whats-new.txt`, `store-certification-notes.txt`: prepared release/listing text.

Published build 190 Store MSIX SHA256: `104c4a0508215bb98573f09155c01c4a2aa218038f9d452d8133518cd145641f`.

Published build 190 Mac binary SHA256: `edd634ce986b3901534bf869e9fa0f40240a6c05dae135eb472cbf68ab2fb8fa`.

## Installed build and preservation

Current installation evidence is `artifacts/mac-install-2.1.5-build193/installation-report.json`. Installed **2.1.5/build 193** has binary SHA256 `cbe00d473ad71ae22aca52ad2adf4905cee14be9ef66aaad28c2fd2362853edc`, app identifier `dev.mypdf.desktop`, and the existing Developer ID team `HXDYTBC6ZT`. It is signed but not notarized.

MyPDF was quit normally and confirmed stopped before replacement. The complete user-data/preferences backup was verified, and exact data/metadata snapshots were unchanged during replacement. After launch, the running installed executable was verified; all seven documents, their order and reading positions were restored. Workspace state, eight critical settings/artwork/signature/style files and macOS preferences remained unchanged. The build 193 private backup is `artifacts/mac-install-2.1.5-build193/user-data-at-install/`.

**Current rollback is 2.1.4/build 191:** `artifacts/mac-install-2.1.5-build193/previous-installed.retired/`, with its original `Contents` directory intact and all bytes/metadata verified. Its binary SHA256 is `9c22a4150b027f7d3ae717859ce7ddfe3bdeace3111bc10b2488fc452bf020a7`. After normal shutdown and preserving the current app, move this retained directory directly to `/Applications/MyPDF.app`; follow the build 193 report's signature/hash and registration checks. Do not rename `Contents` internally or overwrite current user data from the backup.

The earlier build 191 installation retains build 190 at `artifacts/mac-install-2.1.4-build191/previous-installed.retired/`, also with `Contents` intact, binary SHA256 `edd634ce986b3901534bf869e9fa0f40240a6c05dae135eb472cbf68ab2fb8fa`. That is historical recovery material, not the current rollback target.

The build 190 installation report under `artifacts/mac-install-2.1.4-build190/` is historical evidence. Its earlier build 189 rollback remains available as `previous-installed.zip` and an inert `previous-installed.retired` with `ArchivedContents`; build 189 binary SHA256 is `50152687e0be89525f75c00a620059e26c2ea8b330606e69589aa00195ff1596`. Use that older report only when deliberately recovering the older build.

Preserve settings, custom artwork, digital identities/Keychain material, styles, recovery state and preferences across any future installation. Do not read, commit or upload user-data backups or credentials. Quit normally and verify the app is stopped before an authorized replacement; never force quit or discard unsaved work. The release profile/validation profile are isolated from the user's application data.

Keep Finder cleanup targeted. Build 189's `open-with-after-install.json` verified only the installed MyPDF registration and no removal of unrelated registrations. Review/debug bundles omit PDF handlers, archived apps retain verified recoverable originals, and prior cleanup deliberately avoided a broad Launch Services reset. Do not reactivate all archived apps, strip xattrs, empty Trash, or rerun old installers blindly.

## Working tree and historical recovery

The clipboard/selection implementation being committed for 2.1.6 includes `packaging/Info.plist`, clipboard/selection/context/viewer/font changes, new `src/clipboard_text.rs`, new `src/text_recovery.rs`, new `tests/viewer_copy.rs`, the necessary vendored hayro font APIs, and this handover. `artifacts/mac-install-2.1.5-build193/source-state.json` pins the source used for the installed binary against base commit `01261168d996319b73e9e8ae30d8e81f11e98c31`. Local artifacts and private QA documents stay out of Git.

The earlier search-clear update and Mac build 191 were pushed as `597a7fe641b54fab9cd3d1ea5c33d47c3bdd8e4d`; the initial handover as `189195f`; and release preparation as `88499a4effec96785a5d2a26b95294a99abe6d9b`, still pinned by public tag `v2.1.5`. Later documentation commits do not change that release source. Preserve unrelated checkout state:

- `vendor/underskrift/tests/fixtures/sample.pdf` is a pre-existing modified fixture; excluded from the release commit/snapshot. Do not reset or include it in release work.
- `.mimosa/` is unrelated and untracked; leave it alone.
- `scripts/__pycache__/` is generated/untracked; do not stage it.
- The user authorized committing and publishing this updated handover with the fixes and 2.1.6 release. Keep actual private artifacts, user-data backups, source documents, QA Word files and credentials out of Git.

The previous handover is preserved at:

`/Users/gokcol/SWPROJECTS/Utility/MyPDF/artifacts/release-2.1.4-build190/session-history/HANDOVER-before-refresh.md`

The older checkpoint is under `artifacts/session-handover-2026-10-24/`. Its `/Users/gokcol/Works/MyPDF` paths describe the old workspace. Treat it as historical recovery evidence; do not apply its patch over the current release checkout. Ignored artifacts are local and will not appear in a fresh clone.

User preferences: keep the UI compact, preserve modes/settings, avoid unsolicited layout changes and competitor comparisons, and avoid repeated permission questions for work already authorized. No new chat or recurring monitor is requested. The user has now authorized the 2.1.6 GitHub release and local build 194 installation. Build 193 is still installed while that work proceeds.
