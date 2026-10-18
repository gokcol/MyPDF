# Native offline translation on Windows

MyPDF uses a native CTranslate2 + SentencePiece library on Windows. Rust owns
job cancellation, the language catalog, validated model installation, and UI
results. The C ABI receives UTF-8 text in memory and returns UTF-8 text. It has no
network or file-output API. No Python runtime, local HTTP server, web view, or
cloud translation provider is shipped.

macOS continues to use Apple's installed-model Translation framework. The same
CTranslate2 bridge can be compiled on Mac solely for regression tests.

## Build and package

On Windows x64, install Visual Studio 2022 C++ tools, Git, and CMake 3.31.6:

```powershell
./scripts/build-translation-windows.ps1
./scripts/test-translation-runtime.ps1
```

The output is `dist/translation/windows-x64/`: one `mypdf_ct2.dll`, its checksum,
and license notices. CTranslate2, SentencePiece, and the CPU RUY backend are
statically linked, including the MSVC runtime. CUDA, MKL, oneDNN and Python are
not required. Include the entire output in a `translation/` folder beside
`MyPDF.exe`; ZIP, Inno Setup and MSIX packaging all validate this runtime.

Upstream source commits are pinned in `CMakeLists.txt`:

- CTranslate2 4.6.0: `617405f4b050e994e829d527da6caa0e0030afe7` (MIT).
- SentencePiece 0.2.0: `17d7580d6407802f85855d2cc9190634e2c95624` (Apache 2.0).
- CTranslate2's recursive submodules are pinned by that commit.

CMake 4 is not the tested toolchain. CMake/Ninja/Python used by CI are build tools,
not application runtime dependencies.

## Language packs

The reviewed `languages.json` catalog initially provides English in both
directions with Turkish, German, French and Spanish. These are independent
pairs; MyPDF does not silently pivot through another language. The catalog is
bundled with the app and is never automatically replaced from a remote index.

Selecting **Download** is the only translation operation that accesses the
network. It performs an HTTPS GET for the chosen public model URL, with no
redirects, credentials, document data, or text in the request. The download is
bounded by its pinned byte size and SHA-256. An interrupted or invalid archive
cannot replace an installed language pack.

Installation extracts only an explicit allowlist of native model files,
SentencePiece tokenizer, metadata and original README. It does not extract or
execute Stanza, Python, DLLs or archive paths outside the staging directory.
Installed files are hash-checked before inference. Models live in
`%LOCALAPPDATA%/MyPDF/translation-models`; they are not put inside PDFs, settings,
custom-stamp data, or application binaries. Translation results are memory-only
unless the user explicitly copies or saves them through the UI.

The Argos package index supplies the download URLs:
https://github.com/argosopentech/argospm-index

The Argos maintainer confirms the model binaries use MIT/CC0:
https://github.com/argosopentech/argos-translate/issues/533

The MIT text is included in `licenses/Argos-Translate.txt`. Each installed model
retains its original README attribution. Embedded Stanza components are omitted.

To approve additional packs, inspect the official archive's metadata, tokenizer,
model format and licensing; calculate its exact size and SHA-256; add one entry
per supported direction; and run a native synthetic-text translation test. Do
not add an arbitrary unverified URL, assume one direction implies the reverse,
or interpret missing offline support as permission to use a cloud service.

## Checks

```sh
cargo test --lib translation::ctranslate
./scripts/build-translation-native.sh
MYPDF_CT2_LIBRARY="$PWD/dist/translation/native/libmypdf_ct2.dylib" \
MYPDF_CT2_TEST_PACKS="$PWD/artifacts/translation-ct2/downloads" \
cargo test --locked --config 'profile.dev.package.sha2.opt-level=2' \
  --lib translation::ctranslate::tests::native_translation_smoke_all_required_pairs \
  -- --ignored --nocapture

# Explicitly download and verify one approved pack using the real downloader:
MYPDF_CT2_DOWNLOAD_TEST=1 cargo test --locked \
  --config 'profile.dev.package.sha2.opt-level=2' \
  --lib translation::ctranslate::tests::explicit_language_download_verifies_and_installs_the_approved_pack \
  -- --ignored
```

`MYPDF_CT2_LIBRARY` is accepted only in Rust tests; production only loads the
absolute library path inside its own installation. The Windows ABI check verifies
PE x64 architecture, checksum, dependency loading, and the five C exports.
The optimized SHA-256 setting keeps integrity checks fast in these debug tests;
release builds are already optimized.

Inference uses one model/job at a time, at most four CPU workers, and bounded
Unicode-safe segments. Cancellation prevents late results from appearing and
stops native generation at a token boundary. Download cancellation aborts a
stalled network wait within the worker's 100 ms polling interval; independent
15-second connect/read timeouts also bound network stalls. Incomplete or output-limited
translations are reported as errors rather than silently truncated.
