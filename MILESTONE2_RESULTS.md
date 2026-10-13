# Milestone 2 — digital identities and signing

Implemented on 12 October 2026 in Rust 0.3.0. The UI uses the existing native eframe window, native macOS menus and OS file dialogs. No WebView is used.

## Delivered

- Digital identity manager with a persistent catalogue of public metadata and references to protected identity files or opaque Keychain keys. The catalogue never includes passphrases/private keys; passphrase drafts use zeroizing memory. Atomic writes and an OS file lock reject concurrent catalogue replacement.
- Personal digital IDs with name, optional organization/email, 1–3,650 days of validity, RSA-2048 or ECDSA P-256, signing key usages, and modern encrypted PKCS#12 storage. New/exported files require a fresh destination and matching passphrases of at least 12 bytes. These are self-signed IDs, not CA-issued trust credentials.
- Strict authenticated PKCS#12/PFX import, identity alias discovery, wrong-password feedback, certificate eligibility, and protected re-export with a new passphrase. Public DER/PEM certificate inspection/export. A public certificate alone cannot sign.
- macOS Keychain discovery and explicit certificate selection, optionally from a selected store. Native signing keeps the private key in Security.framework. File/key references are checked against the selected certificate fingerprint before signing.
- Visible/invisible approval signatures with a new field name, page and displayed-page rectangle, Unicode name/reason/location and optional reusable artwork. Drawing the rectangle, numeric placement and appearance preview are provided. Cropped pages and all four standard page rotations are covered.
- Offline validation separates signature mathematics, signed-content digest, byte-range integrity, certificate binding, certificate validity, trust and current-file coverage. Explicit trust uses a selected public certificate for this verification session only; trusted roots are not changed. Earlier signatures can be valid for their signed revision while later bytes exist.
- Signing exports current annotations first, appends the signature, verifies output, publishes a fresh file and opens that signed copy in a new viewer tab. A stale document selection is rejected. Pending inspector changes are applied before signing.

## Verified evidence

The complete automated suite contains 36 passing checks: 11 existing core tests, 10 viewer tests, 7 annotation tests, 3 digital-signature tests and 5 headless desktop tests. Formatting and all-target Clippy with warnings denied pass. The new checks cover certificate subject/SAN/validity, wrong passwords, encrypted backup re-import, public-certificate parsing, field-name collisions, all four page rotations, incremental multiple signatures, trust separation, tampering, and a catalogue containing public data only.

`artifacts/milestone2-proof-003/` contains four annotated/rotated signed PDFs, a two-signature PDF, a tampered copy, a native Keychain output, synthetic PFX/certificate files, detached CMS/content and JSON evidence. Poppler `pdfsig` accepts the rotated, multiple-signature and Keychain outputs and detects the altered digest. Independent OpenSSL CMS verification with binary detached content succeeds for all four rotated outputs. Poppler-rendered appearances were visually inspected for Unicode text, artwork and upright placement on rotated pages.

The Keychain proof creates a separate synthetic store, imports/discovers/selects the generated identity through the same adapter, signs via an opaque key handle, deletes that store and verifies restoration of the original search list. The tool sandbox initially blocked search-list access; the isolated proof succeeded with the required execution permission. No personal signing identity or trust root was changed.

Live macOS UI checks created a synthetic P-256 identity, imported a PFX with wrong-password feedback and a successful retry, exported its public certificate, previewed a visible signature, signed an annotated document and reopened the new signed tab. The validation panel passed mathematics, digest, byte ranges and certificate binding; default trust remained unknown, and selecting the matching public certificate enabled trust for that session. The report also identifies the actual CMS signer certificate. Native text fields support Select All and Paste through the macOS Edit menu while OS file dialogs retain their native editing commands.

The actual desktop output, `artifacts/milestone3-proof-003/annotations-signed.pdf`, independently passes Poppler and OpenSSL CMS verification. Its unsigned source bytes and all four annotations are preserved. Evidence and visually inspected Poppler output are in `artifacts/milestone23-ui/independent-native-verification.json` and `gui-signed.png`. These identities and signatures are synthetic test data.

## Boundaries

This is offline approval signing. Online revocation, trusted timestamping, certification/DocMDP policy, hardware tokens, full intermediate-chain discovery, Windows certificate stores and independently audited PAdES profile conformance remain later work. Reported timestamp values are not independently trusted here. Self-signed IDs remain unknown/untrusted by default.

The proven writer accepts traditional xref inputs. An unsigned xref-stream input requires the clearly labeled prepared-copy option; an already signed stream input is rejected. Signed/certified/encrypted inputs cannot be rewritten for ordinary annotations. Unsupported inputs and authorization errors preserve their source and existing destination files.

Windows portability is implemented and a Windows CI job is present, but compilation and live Windows execution have not been performed on this Mac. Final distribution license audit, Developer ID signing and notarization remain release work.
