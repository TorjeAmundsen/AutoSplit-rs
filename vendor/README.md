# Vendored crates

## i-slint-renderer-software

Slint 1.18.1's CPU renderer, which draws the window, with two changes in `fonts/vectorfont.rs`, both
marked `AutoSplit:`:

- Glyphs are rendered with the font's hinting (`.hint(true)` on the swash scaler), which keeps
  stems on whole pixels like Qt and Windows do.
- `SUBPIXEL_BIN_COUNT` is 1 instead of 4, so glyphs start on whole pixels. With 4, a letter is
  drawn at quarter-pixel offsets and looks blurred differently depending on where it is.

Without these, text looks fuzzy next to AutoSplit's. Slint has no setting for either.

The workspace's `Cargo.toml` uses it through `[patch.crates-io]`. When upgrading Slint, copy the new
version's `i-slint-renderer-software` here from `~/.cargo/registry/src/*/` and make the same two
changes, or drop the patch if Slint has made them options by then.

It's licensed under Slint's licenses, in `LICENSES/`; this project uses it under GPL-3.0-only.
