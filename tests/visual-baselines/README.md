# Reviewed desktop baselines

These drawing context-menu captures were reviewed on Apple Silicon macOS with MyPDF 1.11.2 (build 121). They cover the compact menu, icon/shortcut columns and page/inspector arrangement in light and dark appearance. They are synthetic documents, not user files.

The harness waits for document/inspector work, explicitly returns the context scene to the top of page 1, and advances virtual time past popup and scrollbar animations before capturing. This prevents variable loading-frame counts from changing the reading anchor or recording a partially transparent menu. The captures use the real UI and its normal animations, with no production styling overrides.

Run from the repository root with an isolated profile (use `dark` for the other appearance):

```sh
MYPDF_DATA_DIR="$PWD/artifacts/visual-check/profile" \
MYPDF_PREVIEW_DIR="$PWD/artifacts/visual-check/light" \
MYPDF_PREVIEW_THEME=light \
MYPDF_PREVIEW_ONLY=ContextDrawing \
MYPDF_PREVIEW_BASELINE="$PWD/tests/visual-baselines/light" \
cargo test --locked --bin mypdf native_visual_preview -- --ignored --nocapture
```

The check compares decoded pixels; it tolerates small rasterization differences and fails if more than 0.5% of pixels differ by over 16 channel values. Fonts and OS rendering can affect results. Inspect any mismatch before deciding whether it is a regression or an intentional platform/layout change. Baselines must never be updated automatically merely to make a failing test pass.

All 67 scene setups also run independently with content/state assertions. Set `MYPDF_PREVIEW_VALIDATE_ONLY=1` to validate those setups without expensive screenshot rasterization; do not set a baseline directory in that mode.
