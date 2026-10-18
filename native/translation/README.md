# Apple translation bridge

MyPDF's macOS translation uses Apple's Translation framework through a small
Swift C-ABI bridge. The viewer keeps its macOS 12 minimum; this feature requires
macOS 26 or later, and building it requires the Xcode 26 or later SDK. Older SDKs
produce a viewer with an explicit unavailable message. Translation frameworks
are weak-linked and referenced only behind runtime availability checks.

## Privacy boundary

- `Languages` and `Check` ask Apple's local service for supported languages and
  pair installation status. They do not request model downloads.
- `Download` contains only source and target language identifiers. It opens a
  native SwiftUI window, where Apple's `prepareTranslation()` may ask the user
  to download language models. The download path rejects document text.
- `Translate` requires an installed language pair. It immediately creates
  `TranslationSession(installedSource:target:)` and refuses to proceed unless
  `canRequestDownloads` is false. It never uses the download window's session.
  The cancellable session exists before the first service await; missing models
  produce a safe `notInstalled` error instead of requesting a download.
- There is no HTTP client, cloud fallback, translation history file, or document
  logging in the bridge. Source and translated text travel in memory.
- Errors are mapped to fixed messages; raw OS error descriptions are not copied
  into MyPDF because they could include private content or internal details.
- Downloading models is a separate user action and may use the network. This is
  not a system-wide network firewall; Apple's service manages its own metadata.

Each operation streams JSON events through a C callback. Rust owns the event
channel and discards late callbacks after cancellation. Cancelling or dropping a
job is nonblocking. Apple receives both task cancellation and session.cancel().
The native download window closes on completion or cancellation.

Page requests are split at whitespace into bounded chunks for inference, then
returned at the original page index. This is plain-text translation, not PDF
layout replacement.

## Native smoke checks

On a logged-in macOS 26+ machine:

```
sh native/translation/smoke.sh
sh native/translation/smoke.sh --translate
```

The first command queries supported languages and Turkish/German to English
installation status. The second also translates four fixed synthetic sentences
in both directions. Neither command requests a language download. Missing models
cause translation to fail with a clear message.

The read-only check is also available as an ignored Rust test:

```
cargo test --lib translation::tests::apple_language_catalog_and_pair_availability -- --ignored --nocapture
```

The normal Rust tests check payload separation, input limits, Unicode event
transport, and cancellation dropping late native results. The native checks need
access to the macOS user session; a restrictive process sandbox can prevent
Apple's local XPC services from answering.
