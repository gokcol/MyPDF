# Microsoft Store packaging

MyPDF stays a native Rust desktop app. This package adds a Windows application
identity, Start menu artwork, PDF **Open with** registration, and Store-managed
installation/updates. It targets Windows 10 version 2004 (build 19041) or later,
and Windows 11, on x64. It does not change the default PDF application.

The MSIX pipeline is separate from the existing Inno Setup/portable downloads.
Its unsigned output is for **Partner Center submission**, not direct installation
from GitHub. Microsoft signs the Store-distributed package after certification.
The GitHub EXE installer remains unsigned.

## Reserve the product identity

Create a Windows developer account in Microsoft Partner Center, reserve the app
name, then open **Product management → Product identity**. Copy these values
exactly into GitHub repository **Settings → Secrets and variables → Actions → Variables**:

| Repository variable | Partner Center value |
| --- | --- |
| `MSIX_IDENTITY_NAME` | Package/Identity/Name |
| `MSIX_PUBLISHER` | Package/Identity/Publisher (the complete `CN=…` value) |
| `MSIX_PUBLISHER_DISPLAY_NAME` | Package/Properties/PublisherDisplayName |
| `MSIX_DISPLAY_NAME` | Exact reserved app name, currently `MyPDF (OG)` |

These are public identity values, not credentials. Do not generate a replacement
publisher or use the test identity for a Store submission. The workflow passes
`MSIX_DISPLAY_NAME` to the script's `-DisplayName` argument; without that optional
variable, the default is `MyPDF`. This repository's Store product is reserved as
**MyPDF (OG)**, so its Store workflow uses that exact display name. The Mac app
and direct Windows installer retain the name MyPDF.

## Build

On GitHub, run **Actions → Microsoft Store MSIX → Run workflow**:

- `ref`: the release tag or full commit SHA to package.
- `mode`: `store` after configuring the three variables; `test` otherwise.
- `package_version`: usually blank, which uses `version.txt` in this directory.
  For MyPDF **2.1.0**, the internal MSIX version is **2.1.0.0**, which
  sorts above the previous 2.0.3 package **2.0.3.0** and preserves ordinary
  upgrades. Public app versions and MSIX package versions are maintained
  separately. Increase `version.txt` for each Store release.
  An explicit override must be at least the checked-in package version.
  The fourth field is reserved by Microsoft and must remain zero.

On a Windows machine with Rust and the Windows SDK:

```powershell
# Clearly marked validation package; not a Store submission.
./scripts/package-msix.ps1 -TestIdentity

# Use the exact values copied from your own Partner Center product.
./scripts/package-msix.ps1 -IdentityName $identityName `
  -Publisher $publisher -PublisherDisplayName $publisherDisplayName
```

The script builds the desktop executable and icon generator, produces DPI-aware
artwork and a resource index with MakePri, then uses MakeAppx with semantic
validation enabled. It unpacks the result and verifies the executable and identity
round trip. Outputs in `dist/msix` are the `.msix`, SHA-256 sidecar, and JSON build
metadata (identity, public `appVersion`, internal package `version`, source commit
and Windows SDK version). An existing
package is never overwritten. `-SkipBuild` is available when both binaries have
already been built with `cargo build --locked --release --bin mypdf --example msix_assets`.

CI also checks missing/placeholder identities, invalid versions, XML escaping,
and output collisions. For test builds it signs a **temporary copy** with a one-day
self-signed certificate, installs/uninstalls that package on the Windows runner,
checks that an existing data file survives, and removes the certificate. That
signed copy and private key are never uploaded. These checks do not replace a
manual packaged-app review or Microsoft certification.

## Settings, stamps, and capability declarations

MyPDF keeps its existing `%LOCALAPPDATA%\MyPDF` settings, artwork, certificate
references, recovery files and single-instance mailbox. On Windows 11 the
manifest excludes only this directory from MSIX file virtualization. Windows 10
uses the broader filesystem opt-out required on that OS. Registry virtualization
stays enabled. User PDFs remain in the locations the user chooses. Uninstalling
the Store package preserves the shared MyPDF data directory.

The Store submission must explain these restricted capabilities:

- `runFullTrust`: the existing native Win32/Rust PDF editor uses native file
  dialogs, GPU rendering and Windows certificate APIs at the current user's
  privilege level. It does not request administrator elevation.
- `unvirtualizedResources`: preserve the user's stamp library/settings/recovery
  files and share the file-open mailbox with the existing portable/Inno version.
  Removing the Store app must not delete this user-created content.

Microsoft reviews these declarations; packaging success is not Store approval.
The package does not bundle a certificate, user data, or a WebView runtime.

## Before submission

1. Build in `store` mode and check the artifact's identity and version JSON.
2. Test the packaged app on Windows: launch from Start, open PDFs with spaces and
   Unicode in their names, open another PDF while running, save a new copy, use
   custom stamps and certificate signing, and verify upgrade/uninstall behavior.
   Check coexistence with the installer version and that existing user data stays
   visible. Use an isolated test machine/profile, not your only copy of user data.
3. Run the Windows App Certification Kit on the final package; review its report.
4. In Partner Center, create the submission and upload the `.msix` (an accepted
   upload format). Complete the listing, screenshots, privacy policy, age ratings,
   pricing/markets and the capability explanations above. Submit for certification
   only after those details are ready.

No workflow creates a developer account, purchases a service, publishes a Store
listing, or uploads to Partner Center automatically. Until the real identity is
configured and certification succeeds, this is packaging support, not a Store release.

## Microsoft references

- [Manual desktop packaging and MakePri](https://learn.microsoft.com/en-us/windows/msix/desktop/desktop-to-uwp-manual-conversion)
- [MakeAppx and semantic validation](https://learn.microsoft.com/en-us/windows/msix/package/create-app-package-with-makeappx-tool)
- [Virtualization and shared AppData](https://learn.microsoft.com/en-us/windows/msix/desktop/flexible-virtualization)
- [Package version rules](https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msix/app-package-requirements)
- [Accepted submission formats](https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msix/upload-app-packages)
- [Store signing and developer account requirements](https://learn.microsoft.com/en-us/windows/apps/publish/faq/get-started-with-the-microsoft-store)
