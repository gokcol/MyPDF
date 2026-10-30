# MyPDF — current handover

Updated 30 October 2026 (Europe/Istanbul). Workspace: `/Users/gokcol/SWPROJECTS/Utility/MyPDF/`.

## Published GitHub release — 30 October 2026

**[MyPDF v2.1.5](https://github.com/gokcol/MyPDF/releases/tag/v2.1.5) is published and marked latest**, verified at 10:57 Europe/Istanbul. The tag points to release-preparation commit [`88499a4effec96785a5d2a26b95294a99abe6d9b`](https://github.com/gokcol/MyPDF/commit/88499a4effec96785a5d2a26b95294a99abe6d9b). It includes the search-clear button on Mac and Windows. Mac release build **192** is Developer ID signed, notarized and stapled. Windows installer, portable ZIP and Store-format MSIX **2.1.5.0** are available. All ten public assets were verified by filename, size and GitHub SHA-256 digest against the local release manifest.

The initial updated handover was committed and pushed as [`189195f`](https://github.com/gokcol/MyPDF/commit/189195f), followed by the release-preparation commit above. This handover now records the completed publication; documentation commits after the release source do not change the pinned release tag.

Authoritative release records are under `artifacts/release-2.1.5-build192/`: `release-status.json`, `github-release.json` and `public-release-verification.json`. The installed Mac remains **2.1.4/build 191**, with settings and open documents preserved. The new MSIX was published to GitHub only; no new Microsoft Partner Center submission, local installation or recurring monitor was performed.

## Installed build and completed source update — 30 October 2026

At the user's request, the app was rebuilt and installed locally as **2.1.4 build 191** with the one-click X inside the document search field. It appears when the query contains text, clears the query and matches, and restores field focus. Nine existing search tests and formatting passed; the optimized app was rebuilt and Developer ID signed with the existing identity. The installed build 191 package is **not notarized and was not uploaded as a GitHub release asset**. The historical 2.1.4/build 190 release and recorded Store submission are unchanged; the newer GitHub release is 2.1.5/build 192 as described above.

The search-clear control and Mac build number were **committed and pushed to GitHub `main`** as [`597a7fe641b54fab9cd3d1ea5c33d47c3bdd8e4d`](https://github.com/gokcol/MyPDF/commit/597a7fe641b54fab9cd3d1ea5c33d47c3bdd8e4d) — `Add one-click search clearing and bump Mac build to 191`. The commit contains exactly `src/desktop/mod.rs` and `packaging/Info.plist`; it is the implementation commit, followed by handover and release-preparation commits. No release tag, release assets or Store submission were changed by that push.

Current installation evidence: `artifacts/mac-install-2.1.4-build191/installation-report.json`. `/Applications/MyPDF.app` is installed as build 191 and launch was verified; its signed binary SHA256 is `9c22a4150b027f7d3ae717859ce7ddfe3bdeace3111bc10b2488fc452bf020a7`. All six documents present at normal shutdown were restored with their reading positions. Additional documents opened after restart were left in place. Eight critical settings/artwork/signature/style files and macOS preference bytes were verified unchanged.

The full verified data backup is private at `artifacts/mac-install-2.1.4-build191/user-data-at-install/`; do not display, commit or upload its contents. The original signed/notarized build 190 is preserved with all bytes and metadata at `artifacts/mac-install-2.1.4-build191/previous-installed.retired/`. **Its original `Contents` directory is intact**: macOS prevented changing the old app's internals. It was explicitly unregistered; only the installed replacement was registered. Follow the current installation report's rollback steps without overwriting current user data. The first unsuccessful archive attempt is recorded in `installation-attempt1.json`; the current `installation-report.json` records successful recovery and installation.

The project move required regenerating the release Swift compiler cache. Its old cache is retained at `target/release/mypdf-swift-module-cache-before-root-move-20261030/`. The search-clear control and build 191 metadata have no remaining uncommitted changes. Preserve the unrelated modified PDF fixture and other local files listed below. The new GitHub release is published; the installed build 191 remains in place.

## Current task and authority

The user requested the search-clear button, rebuilding/installing the updated Mac app with settings and open documents preserved, and committing/pushing the source and build 191 changes to GitHub. The build 191 implementation, local installation and source push are complete. The user subsequently authorized committing/pushing `HANDOVER.md` and the next GitHub release, including Windows. The handover is tracked and its initial update was pushed as `189195f`; GitHub release v2.1.5 is complete.

The earlier public MyPDF 2.1.4 release remains build 190: GitHub publication and Microsoft Store submission were completed on 25 October. Both published Mac app and DMG were signed, notarized and stapled. Store submission `1152921505702158150` was **In certification** at the last verification on 25 October; its present certification/publication state has not been rechecked. GitHub v2.1.5 is now published; no monitoring automation, further Store submission or local installation was requested.

Use these records for their respective scopes:

- **Current published release:** `/Users/gokcol/SWPROJECTS/Utility/MyPDF/artifacts/release-2.1.5-build192/release-status.json`.
- **Current Mac installation:** `/Users/gokcol/SWPROJECTS/Utility/MyPDF/artifacts/mac-install-2.1.4-build191/installation-report.json`.
- **Published build 190 release:** `/Users/gokcol/SWPROJECTS/Utility/MyPDF/artifacts/release-2.1.4-build190/release-status.json`. Its installed-build fields describe the earlier build 190 installation and are superseded by the build 191 report.

This handover is a point-in-time record. Later status/verification reports take precedence. Historical reports may retain earlier values: for example, build 190's `mac-build-report.json` says `notarized: false` because it predates the accepted app/DMG notarization reports. Build 191 is a separate local package and is not notarized.

## Versions and distribution

| Item | Current verified or explicitly recorded state |
| --- | --- |
| Search-clear implementation commit (pushed) | `597a7fe641b54fab9cd3d1ea5c33d47c3bdd8e4d` — Add one-click search clearing and bump Mac build to 191 |
| Current published release source | `88499a4effec96785a5d2a26b95294a99abe6d9b` — Prepare MyPDF 2.1.5 release for Mac and Windows |
| Current release tag | `v2.1.5`, verified to point to `88499a4effec96785a5d2a26b95294a99abe6d9b` |
| Current clean release snapshot | `/Users/gokcol/SWPROJECTS/Utility/MyPDF/artifacts/release-2.1.5-build192/source`; all 440 tracked files matched the commit |
| Repository/release versions | App 2.1.5; Mac build 192; Windows/Store package 2.1.5.0; installed Mac remains 2.1.4/build 191 |
| GitHub 2.1.5 | [Published release](https://github.com/gokcol/MyPDF/releases/tag/v2.1.5), latest; all ten assets verified; published 30 October at 10:57 Europe/Istanbul |
| Previous GitHub 2.1.4 | [Published release](https://github.com/gokcol/MyPDF/releases/tag/v2.1.4), all ten assets verified; published 25 October at 23:28 Europe/Istanbul |
| Earlier GitHub release | [v2.1.3](https://github.com/gokcol/MyPDF/releases/tag/v2.1.3), Mac build 187 |
| Current published Mac release (build 192) | Developer ID signed, app and DMG notarization **Accepted**, stapled; signature, Gatekeeper and DMG integrity verified |
| Installed Mac | `/Applications/MyPDF.app`, **2.1.4 build 191**, Developer ID signed, signature/hash verified and launched; not notarized; original build 190 retained for rollback |
| New GitHub Store-format asset | `MyPDF-2.1.5.0-store-x64.msix`, x64, unchanged Store identity/publisher; verified and published on GitHub, not submitted to Partner Center |
| Previously submitted Store package | `MyPDF-2.1.4.0-store-x64.msix`, verified x64, unchanged Store identity/publisher; uploaded, validated, saved as the only package and submitted |
| Store product | MyPDF (OG), `9MXR0WLW42XT` |
| Last recorded Store submission | `1152921505702158150`, **In certification**; submission complete, pre-processing in progress, automatic publication after passing certification; verified 25 October at 23:33 Europe/Istanbul |

The Store MSIX is intentionally unsigned for Store ingestion/Microsoft signing; do not misdescribe it as an independently signed consumer package. The separate MSIX install/uninstall CI test passed with its test package.

## Completion and external review

The requested build 191 implementation/installation and v2.1.5 GitHub release are complete. Microsoft certification for 2.1.4.0 was pending at the last recorded check on 25 October; no current status is claimed. At that check, the 2.1.3 certification had been canceled, 2.1.4.0 was the only submitted package, and final Submit for certification had succeeded. Do not repeat or change that submission without a new request. No monitor has been requested.

For the historical 2.1.4.0 Store submission, the What's New text and reviewer notes were saved, reopened and verified; existing restricted-capability explanations and automatic-publication settings were preserved. The generated build 192 app was archived and unregistered after packaging; its retained original has `ArchivedContents`, and a verified ZIP is also available. Use `artifacts/release-2.1.5-build192/source-app-archive.json` for recovery. This is separate from the installed build 191 rollback directory, whose build 190 `Contents` must remain intact.

Partner Center: [product overview](https://partner.microsoft.com/en-us/dashboard/products/9MXR0WLW42XT/overview).

Do not inspect unrelated browser pages, conversations or user data. If another private page becomes foreground, stop Store UI interaction and wait for the intended page. No monitoring automation has been requested.

## Included improvements

Committed on GitHub `main`, installed locally in build 191 and included in published v2.1.5: a clear X inside the right edge of the document search field when text is present. One click clears the query and matches, cancels stale search results through the existing invalidation path, and returns keyboard focus to the field. Match-case and whole-word settings remain unchanged.

The earlier refinements included in the published 2.1.4/build 190 release remain:

- Stable annotation text layout and inline editing at reduced zoom; smoother filled shapes, mirrored text and image artwork.
- Each stamp/checkmark/saved ink signature remembers its explicitly chosen size in PDF points, across launches, while fitting smaller pages proportionally.
- Thumbnail/page-sorter multiselection and context actions: copy/cut/paste before or after, duplicate, rotate, extract and delete; cross-document page copying retains annotations.
- Page shortcuts respect thumbnail focus and return to normal text/artwork behavior after a document click. Pending text/table/form edits block page changes, copies and exports so drafts are not silently omitted.
- Sidebar thumbnail navigation preserves Back history; right-click selection does not move the reading position.
- The duplicate toolbar **Find** icon was removed. Sidebar **Search**, Edit → Find and Command/Ctrl+F remain.

Existing Turkish/nonembedded-font fixes and durable Finder Open With cleanup are retained. Installed-font fallback still depends on available fonts and valid PDF character mappings; do not claim universal language support.

## Evidence and validation

Build 191 evidence is under `artifacts/mac-install-2.1.4-build191/`: `candidate-report.json` pins its signed binary, and `installation-report.json` confirms installation, launch, preservation checks and rollback. Nine existing search tests passed, formatting and diff checks passed, and the optimized bundle built successfully. These are the build 191 local checks; the separate release-source GitHub checks are recorded below.

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

Current installation evidence is `artifacts/mac-install-2.1.4-build191/installation-report.json`. Installed build 191's binary SHA256 is `9c22a4150b027f7d3ae717859ce7ddfe3bdeace3111bc10b2488fc452bf020a7`. Its app identifier remains `dev.mypdf.desktop`, signed by the existing Developer ID team `HXDYTBC6ZT`.

MyPDF was quit normally and confirmed stopped before replacement. The complete user-data/preferences backup was verified and user data was unchanged during replacement. All six documents open at shutdown and their reading positions were restored. Verification found a seventh document subsequently opened; it was left in place. Eight critical settings/artwork/signature/style files and macOS preference bytes remained identical. The installed executable and running app path were verified; the clear-search feature marker is present. Native About/UI verification from build 190 must not be represented as a new build 191 check.

**Current rollback is build 190:** `artifacts/mac-install-2.1.4-build191/previous-installed.retired/`, with its original `Contents` directory intact and all bytes/metadata verified. Its binary SHA256 is `edd634ce986b3901534bf869e9fa0f40240a6c05dae135eb472cbf68ab2fb8fa`. After normal shutdown and preserving the current app, move this retained directory directly to `/Applications/MyPDF.app`; follow the current report for signature/hash and registration checks. Do not apply an `ArchivedContents` rename to this build 190 backup, and do not overwrite current user data from the data backup.

The build 190 installation report under `artifacts/mac-install-2.1.4-build190/` is historical evidence. Its earlier build 189 rollback remains available as `previous-installed.zip` and an inert `previous-installed.retired` with `ArchivedContents`; build 189 binary SHA256 is `50152687e0be89525f75c00a620059e26c2ea8b330606e69589aa00195ff1596`. Use that older report only when deliberately recovering the older build.

Preserve settings, custom artwork, digital identities/Keychain material, styles, recovery state and preferences across any future installation. Do not read, commit or upload user-data backups or credentials. Quit normally and verify the app is stopped before an authorized replacement; never force quit or discard unsaved work. The release profile/validation profile are isolated from the user's application data.

Keep Finder cleanup targeted. Build 189's `open-with-after-install.json` verified only the installed MyPDF registration and no removal of unrelated registrations. Review/debug bundles omit PDF handlers, archived apps retain verified recoverable originals, and prior cleanup deliberately avoided a broad Launch Services reset. Do not reactivate all archived apps, strip xattrs, empty Trash, or rerun old installers blindly.

## Working tree and historical recovery

The search-clear update and Mac build 191 are committed and pushed as `597a7fe641b54fab9cd3d1ea5c33d47c3bdd8e4d`. The initial handover update was pushed as `189195f`; release preparation was pushed as `88499a4effec96785a5d2a26b95294a99abe6d9b`, which is pinned by published tag `v2.1.5`. Later documentation commits record completion without changing the release source. Remaining unrelated checkout state must be preserved:

- `vendor/underskrift/tests/fixtures/sample.pdf` is a pre-existing modified fixture; excluded from the release commit/snapshot. Do not reset or include it in release work.
- `.mimosa/` is unrelated and untracked; leave it alone.
- `scripts/__pycache__/` is generated/untracked; do not stage it.
- `HANDOVER.md` is now explicitly authorized for commit and publication. Keep the actual private artifacts, user-data backups and credentials out of Git.

The previous handover is preserved at:

`/Users/gokcol/SWPROJECTS/Utility/MyPDF/artifacts/release-2.1.4-build190/session-history/HANDOVER-before-refresh.md`

The older checkpoint is under `artifacts/session-handover-2026-10-24/`. Its `/Users/gokcol/Works/MyPDF` paths describe the old workspace. Treat it as historical recovery evidence; do not apply its patch over the current release checkout. Ignored artifacts are local and will not appear in a fresh clone.

User preferences: keep the UI compact, preserve modes/settings, avoid unsolicited layout changes and competitor comparisons, and avoid repeated permission questions for work already authorized. No new chat or recurring monitor is requested. The v2.1.5 GitHub release is complete as described above.
