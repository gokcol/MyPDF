# MyPDF calico logo

![Calico cat editing a PDF](MyPDF-calico-v2.png)

Calico revision of the original white-cat concept: white, orange and charcoal patches, with the PDF, pencil and blue-violet tile retained. Created using the built-in image-generation tool; transparent PNG master is `MyPDF-calico-v2.png`. The original concept is preserved in `MyPDF-cat-v1.png`.

## Platform use

This is the shared app-icon master for MyPDF 1.2.0. `src/desktop/icon.rs` embeds it for native window icons, in-app branding, macOS packaging, Microsoft Store tiles and Store listing logos. Run `cargo run --locked --release --example platform_icons` to refresh the Windows multi-size ICO and opaque iPad icon; use `store_listing_assets` and `msix_assets` for their respective PNG exports.

The same exporter refreshes `MyPDF-calico-128.png` and `MyPDF-calico-256.png`. The application decodes these prepared sizes at runtime so opening a window never resamples the full-resolution master on the UI thread.

## Edit prompt

Use case: precise-object-edit.
Edit target: the supplied MyPDF cat app icon.
Change ONLY the cat's coat coloration to a lovely recognizable calico: a predominantly white cat with distinct rich orange and charcoal-black patches. Give it asymmetrical orange and black patches across the ears and upper head, with some matching patches on its curled tail; retain a white muzzle, chest and mostly white paws. Natural calico patch boundaries, not stripes. Keep both eyes clearly visible and preserve the exact sweet expression, pose, proportions, face shape and rendering style.
Preserve the pencil, PDF sheet, exact "PDF" lettering, editing strokes, blue-violet rounded-square tile, lighting, composition, dimensions, and transparent exterior. Do not add or remove objects, do not change the design or layout. Preserve true alpha transparency outside the artwork.
