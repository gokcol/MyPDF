# MyPDF local patch

Upstream: hayro-interpret 0.8.0, licensed under MIT or Apache-2.0 (original license files retained).

`StandardFontBlob::name_to_glyph` prefers a known glyph name's Unicode cmap mapping for OpenType substitute fonts, then falls back to the POST table for custom names. Some installed Times New Roman fonts repeat the POST name N or assign it to a different glyph; name-only lookup could render a dotted I instead of N. This also handles valid uniXXXX/uXXXXX glyph names through the interpreter's existing glyph-name decoder.

Embedded font handling and CFF/Symbol/Dingbats lookup are unchanged. The regression is in `tests/render_font_fallback.rs`.
