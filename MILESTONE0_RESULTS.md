# Milestone 0 results — 12 October 2026

Milestone 0 is proven within the boundaries below. The implementation is a Rust CLI harness; the full desktop UI has not been started.

## Requested capabilities

| Capability | Result | Evidence |
|---|---|---|
| PDF rendering | Pass | Hayro PNGs compared with Poppler; vector art, image, Unicode, crop and rotation preserved |
| Real existing-text editing | Pass, bounded text runs | Latin and Turkish replacements change content operators; independent text extraction sees the replacement and no original text on the edited page |
| Certificate creation | Pass | Rust-generated self-signed RSA/P-256 IDs; protected PKCS#12; independent OpenSSL X.509 inspection |
| `.p12` signing | Pass | RSA output accepted by Poppler, OpenSSL and Acrobat |
| `.pfx` signing | Pass | ECDSA P-256 output accepted by Poppler, OpenSSL and Acrobat |
| macOS Keychain signing | Pass | Identity imported into an isolated Keychain, rediscovered, and signed through its native key handle; accepted by independent verifiers and Acrobat |
| Additional signatures | Pass for generated corpus | Two signatures verify independently; original bytes retained; Acrobat accepts the earlier signature after a Keychain signature is appended |

## Automated results

- 30/30 reproducible proof checks passed in `artifacts/milestone0-accepted/report.json`.
- 11/11 integration tests passed with `cargo test`.
- `cargo clippy --all-targets -- -D warnings` and formatting checks passed.
- Generated signing identities never touch the user's login keychain or trusted roots. The temporary store is deleted and the original search list is restored and compared.
- Negative checks cover wrong passwords, changed PKCS#12 MACs, certificate-only stores, expired/inappropriate certificates, tampering, duplicate text matches, unencodable glyphs, signed-document edits and existing-file preservation.

## Adobe observations

The installed Acrobat was used to open the final `signed-p12.pdf`, `signed-pfx.pdf`, `signed-keychain.pdf`, and `signed-p12-then-keychain.pdf` under `artifacts/milestone0-accepted`. Activating the signature field opened **Signature Validation Status**.

For each tested field, Acrobat reported that the document had not been modified since signing, while the signer identity was unknown because the test certificate was not trusted. This is the expected result for these self-signed identities. No certificate was added to Acrobat's trust list.

The initial candidate output failed Acrobat with **“The signature byte range is invalid”**, although Poppler and OpenSSL verified the signature mathematics. Controlled variants isolated this to the candidate writer's handling of cross-reference-stream inputs. A traditional-xref version of the same content was accepted.

The harness therefore saves edited/prepared unsigned PDFs with traditional cross-reference tables and rejects stream inputs at the signing boundary. `prepare-signing` explicitly creates an unsigned copy in the verified format. It cannot rewrite signed PDFs. This restriction is covered by an integration test and must remain until a broader writer implementation passes Adobe checks.

## Limits established by this milestone

Text editing supports one complete page-level text run and its existing font/CMap. Arbitrary paragraph reflow, Form XObject text, missing subset glyphs, and font substitution remain outside the proof. `TJ` replacement normalizes internal kerning.

Signing has no trusted timestamp or revocation lookup. RSA and P-256 are proven; P-384 is implemented but awaits independent testing. Keychain certificate-chain discovery currently includes the leaf only. Hardware identities and Windows certificate stores are not yet proven.

The corpus is synthetic and small. General PDF fidelity, large-file performance, complete ETSI profile validation, and packaging remain later gates. Source is Rust with normal native macOS APIs and ring's compiled crypto backend; there is no Python, Java, browser or WebView runtime.

See `README.md` for reproduction commands, module layout, and dependency findings. Raw independent outputs and generated PDF/PNG files are retained under `artifacts/milestone0-accepted`.
