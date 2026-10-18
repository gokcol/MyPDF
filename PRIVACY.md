# Privacy Policy for MyPDF (OG)

Effective date: 18 October 2026

This policy covers MyPDF (OG), the Microsoft Store edition of MyPDF by
Orhan Gökçöl, and the MyPDF desktop application distributed through the project's
GitHub releases. It describes the behavior in version 1.2.0.

## Information processed on your device

MyPDF is a desktop PDF editor. It accesses the documents, images, SVG artwork,
form data and certificate files you choose to open or import. These files may
contain personal information. The app processes their contents on your device
to display, search, edit, annotate, convert, organize, print and sign documents.
Supported text recognition also runs on the device. MyPDF does not upload these
files or their contents to the developer or to a MyPDF server.

No MyPDF account is required. The app has no advertising, usage analytics,
tracking SDKs or automatic crash-report uploads. The developer does not receive
your document contents, usage history or signing keys through the app, and does
not sell or share that information for advertising.

## Local storage and recovery

To provide its features, MyPDF stores settings, recent-file and favorite
shortcuts, file paths, reading positions, custom stamps and signature artwork,
saved signature appearances, public certificate information, trust preferences
and references to signing identities. It also saves recovery copies of applied
unsaved document changes, including document content and source-path metadata.
Temporary local files may be used while processing or printing documents.

The default application-data locations are:

- Windows: `%LOCALAPPDATA%\MyPDF`
- macOS: `~/Library/Application Support/MyPDF`

A configured `MYPDF_DATA_DIR` can change this location. Files you save or export
remain in the locations you choose. MyPDF does not provide cloud storage or
automatic cloud synchronization. If you use a cloud-synced folder, shared drive,
backup service or network printer, that service may handle the files according
to your settings and its own policies.

Settings, artwork and recovery files are not separately encrypted by MyPDF.
Their protection depends on operating-system access controls and any device
encryption you enable. Recovery copies can contain sensitive document content.

## On-device translation

Translation runs on the device with no cloud fallback. On supported Macs, MyPDF
uses Apple's installed translation models. Apple's translation service may
collect language and usage metrics, but not original or translated text. MyPDF
requires a separate, explicit language-download action before a missing pair
can be used; document text is not included in that download request.

On Windows, MyPDF uses a local CTranslate2 runtime. When you choose to download
a language pack, MyPDF contacts the pack's public distribution host (currently
Argos Open Tech's argos-net.com service). That host may receive ordinary
connection information such as your IP address and the requested model file,
but no document text. Downloads are verified before use. Inference uses local
files and does not make network requests.

Language choices and favorites, model files and model verification metadata are
stored locally. Source and translated text are held in memory and are not saved in a separate
translation history. Translation itself does not change the PDF. If you choose
Attach translation as note, the result becomes a document annotation and is
included in normal PDF saves and recovery copies. Closing the translation panel
or changing the document/page clears the displayed text. Copying a result to
the system clipboard is an explicit action. Downloaded Windows model packs can
be removed from the app's translation-models directory while MyPDF is closed;
Apple's downloaded models are managed by the operating system.

## Certificates and digital signatures

Creating a digital identity uses the name and optional details you enter, such
as an email address. Signing uses the certificate and signing identity you
select. Private keys are managed by the operating-system certificate store,
a configured hardware provider, or a password-protected PKCS#12 file, depending
on the identity you use. Identity-file passphrases and private keys are not saved
in MyPDF's settings or identity catalogue.

Signed PDFs contain public certificate information and may contain the signer
name, signing time, reason, location and signature artwork. Anyone receiving
the signed PDF may be able to read that information. Signature verification
uses local information; MyPDF does not contact an online timestamp or
certificate-revocation service.

## External services and information you choose to share

MyPDF can open a document's web or email link in your browser or mail application
after confirmation. That destination's policies then apply. Copying content uses
the system clipboard, which may be accessible to other software or synchronized
by your operating-system settings.

The SVG prompt helper creates text locally. It does not send prompts or documents
to an AI provider. If you copy a prompt into an external AI service yourself,
that service handles the information under its own policies.

Microsoft Store, GitHub, operating-system services and third-party certificate
providers may process information independently under their own policies. MyPDF
does not control their distribution, account, diagnostic or support systems.

## Retention and your controls

You can clear recent-file shortcuts, remove favorites and saved artwork, and
restore or discard recovery copies in the app. Clearing a shortcut does not
delete its PDF. Saving or explicitly discarding a document clears its associated
recovery copy; abandoned recovery copies remain available for your decision.
Other saved preferences and library items remain until changed or removed.

Uninstalling MyPDF preserves application data so settings and custom stamps are
not lost. To remove all MyPDF application data, close the app and delete its
application-data folder, keeping a backup of anything you want to retain. Saved
documents and exported certificates must be managed separately. Certificates
and private keys in Keychain, the Windows certificate store or a hardware token
must be managed through that system or provider.

## Contact and policy changes

For privacy questions, contact the maintainer through
[the MyPDF issue tracker](https://github.com/gokcol/MyPDF/issues).
GitHub issues are public; do not include confidential documents, passwords or
private keys. Information you voluntarily provide there is used to address
your request and is also subject to GitHub's policies and retention controls.

This policy will be updated when the app's data practices change. The effective
date and applicable app version will be shown on this page.
