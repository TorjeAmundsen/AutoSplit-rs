# AutoSplit rewrite

A from-scratch Rust rewrite of [AutoSplit](https://github.com/Toufool/AutoSplit), the image comparison based auto splitter for speedrunning. Goals:

- Same behavior as AutoSplit, including the similarity values it computes.
- Existing split images, settings profiles and the LiveSplit integration keep working.
- Windows, Linux and macOS.
- Low overhead, so it can run next to OBS on a weak machine.

## Status

Milestone 1 is done: the platform independent core and a headless CLI. There's no GUI, screen capture or hotkey handling yet.

| Area | State |
| --- | --- |
| Split image filename conventions (`(0.95)`, `[10]`, `#500#`, `@3@`, `^1^`, `{dbp}`, `start_auto_splitter`, `reset`) | Done |
| Image loading, masks and the adaptive resize for transparent images | Done, PNG only |
| L2 norm, histograms and pHash comparisons | Done, bit-exact with AutoSplit |
| OCR split files (tesseract) | Not supported for now. Folders containing one can't start |
| Split image folder validation, with AutoSplit's error messages | Done |
| Settings profiles (`.toml`) | Done, reads and writes AutoSplit's files |
| Split state machine (delays, pauses, below flag, dummies, loops, reset image, start image, skip and undo) | Done |
| `--auto-controlled` protocol (LiveSplit integration) | Parsing and output done, needs the app around it |
| Screen and video device capture | Milestone 2 |
| Global hotkeys and sending key presses | Milestone 2 |
| GUI (Slint), region and window selection, screenshots | Milestone 3 |
| LiveSplit integration component (Windows only) | Later |

## Compatibility

The test suite compares this code against values produced by AutoSplit's own Python modules running on OpenCV 4.11.0.86, the version AutoSplit's lockfile pins. Covered:

- Every comparison method on real masked split images (made with AutoMask) and synthetic ones, against several capture sizes, including captures with alpha 0 like BitBlt produces. These go through the same code path the runner uses. Results are equal to the last bit.
- Intermediate steps: nearest, area and linear-exact resizes, grayscale conversion and 400 pHash hashes.
- Filename parsing: thresholds, pauses, delays, loops, methods and flags, including edge cases like `inf`, `1e2`, `.5`, `+1`, unclosed delimiters and conflicting flags.
- Profiles written with tomli_w, the writer AutoSplit uses. Profiles saved here load back in Python with the same values and key order.

### Deliberate differences from AutoSplit

Parity is strict where users depend on it: similarity values, the file formats, the LiveSplit protocol and how a run plays out. Implementation details of the Python version aren't copied.

Split images and filenames:

- Only PNG split images are supported. Other formats get a load error saying so.
- OCR split files (`.txt`) aren't supported yet. A folder with one gets a load error and can't start.
- Grayscale and 16-bit PNGs load. AutoSplit crashes on them.
- A fully transparent split image is a load error, since there's nothing to compare. AutoSplit compares it as an opaque image.
- A start or reset image that can't be used makes the folder invalid, like a broken split image. AutoSplit lets the folder run, and that image never matches.
- `reset` and `start_auto_splitter` only count as whole words, not next to other letters: `preset_menu.png` is a normal split image. AutoSplit matches them anywhere in the name.
- Values need their closing delimiter: `x_(0.9` has no threshold. AutoSplit reads up to the end of the name.
- Numbers in filenames are read with Rust's number parsing, which is stricter than Python's: spaces around the value (`( 0.95 )`), underscores (`#1_000#`) and digits from other scripts (`٣`) aren't accepted, so the default is used.
- Filename values that are ignored get a warning when the folder loads: values that aren't numbers, values out of range (like `(1.5)` or `@0@`), unknown comparison methods (`^5^`, which never matches) and invalid or conflicting flags (`{x}`, `{dp}`). The values are still ignored like in AutoSplit.

Runs:

- Skip split and undo split go by what's possible, not by button state: undo needs a previous split and skip a later one. AutoSplit uses its buttons' enabled state, which is sometimes stale (undo is blocked during the pause right after the first split).
- Next image (or the skip hotkey) does nothing on the last split. In AutoSplit it ends the run, while LiveSplit stays on the last split.
- Skipping a split during the start image's pause keeps the skipped-to split. AutoSplit sends the skip to the timer, then jumps back to the first split anyway.
- Starting with the split hotkey during the start image's delay drops the pending start. AutoSplit sends it once the delay ends, which the timer reads as a split.
- When LiveSplit controls the timer, its skips and undos aren't printed back to it. AutoSplit prints them, and LiveSplit ignores them.
- During pauses and delays the reset image is checked at its FPS limit. AutoSplit checks it every millisecond, which costs a lot of CPU and catches no extra frames.
- The reset image's highest similarity is cleared for every run. Without a start image, AutoSplit keeps the highest value across runs.
- The start image is checked every 1/FPS seconds exactly. AutoSplit uses a timer of whole milliseconds, so it checks slightly more often (62.5 times a second at 60 FPS).
- An FPS limit of 0 is treated as 1. AutoSplit divides by zero.

Settings and display:

- Keys missing from a profile get the settings window's defaults (threshold 0.90). AutoSplit uses a second set of defaults for those (threshold 0.95).
- Profile values of the wrong type, like `fps_limit = 60.0`, make the profile invalid. AutoSplit uses them as they are.
- Error messages are reworded and name the file or value involved.
- Similarities and thresholds are shown with 3 decimals, truncated. AutoSplit shows 2, and its truncation sometimes comes out one step low (0.29 shows as 0.28).
- Countdowns say "1.0 second" but "0.5 seconds". AutoSplit uses "second" for anything up to 1.

Image math:

- pHash computes the DCT in f64 where OpenCV uses f32. A hash bit can differ when a coefficient is within float noise of the mean. None of the test inputs hit this.

## Performance

Cost of one comparison, including the resize from a 1920x1080 capture. Median of 3 runs on one core of an Intel Core Ultra 7 155H. AutoSplit is its own Python code on OpenCV 4.11, limited to one thread.

These are rough numbers. They were measured in a container on a shared VM while other work ran on the same machine, so only the comparison between columns means much, not the absolute times. They'll be re-measured on a dedicated machine with nothing else running.

| Split image | Method | This rewrite | AutoSplit |
| --- | --- | --- | --- |
| Opaque (compared at 320x240) | L2 norm | 87 µs | 50 µs |
| Opaque | Histograms | 72 µs | 210 µs |
| Opaque | pHash | 70 µs | 170 µs |
| Masked, large visible area (compared at 887x499) | L2 norm | 77 µs | 406 µs |
| Masked, large visible area | Histograms | 65 µs | 646 µs |
| Masked, large visible area | pHash | 446 µs | 2201 µs |
| Masked, small visible area (real AutoMask reset image) | L2 norm | 4 µs | 123 µs |
| Masked, small visible area | Histograms | 4.5 µs | 212 µs |
| Masked, small visible area | pHash | 160 µs | 822 µs |

Opaque L2 is the one case where OpenCV is faster: its SSE2/AVX2 code computes the squared differences quicker than what LLVM generates from safe Rust. For masked images only the visible pixels are resized and compared. Nothing is allocated per frame, the split image's histogram and hash are computed once at load, and a split image and the reset image of the same size share one resize per frame. Run `autosplit bench` to measure on your machine.

## Building

Needs a recent stable Rust toolchain.

```sh
cargo build --release
cargo test --workspace
```

The tests need no network or Python.

### Release builds

`build.ps1` (PowerShell 7) builds release zips into `build/`, named `AutoSplit-v{version}-{target}.zip`:

```sh
./build.ps1                  # pick a target from a menu
./build.ps1 linux-arm64      # one or more targets
./build.ps1 --all            # every target this machine can build
./build.ps1 --debug ...      # keep debug symbols
```

Targets are `windows-x64`, `windows-arm64`, `linux-x64`, `linux-arm64` and `macos-universal` (one binary for Apple Silicon and Intel Macs). Each target builds natively when it matches this machine's OS. Other targets are cross-compiled:

| Building on | Linux targets | Windows targets | macOS |
| --- | --- | --- | --- |
| Windows | [cargo-zigbuild](https://github.com/rust-cross/cargo-zigbuild), needs [Zig](https://ziglang.org/download) or `pip install ziglang` | native | not possible |
| Linux | native, or cargo-zigbuild for the other architecture | [cargo-xwin](https://github.com/rust-cross/cargo-xwin), needs clang and lld | not possible |
| macOS | cargo-zigbuild | cargo-xwin | native |

The script installs the Rust targets and the cargo tools it needs. macOS builds need a Mac, because Apple's SDK may only be used there. Cross-compiled Linux binaries need glibc 2.35 or newer (Ubuntu 22.04). Windows ARM64 builds on Windows need the MSVC ARM64 build tools from the Visual Studio installer.

### CI

- `.github/workflows/validate.yml` runs on pull requests and pushes to `main`: formatting and clippy, then the tests and a release build for every target, natively on Windows, Linux and macOS, on x64 and ARM64. The x64 half of the macOS build is tested under Rosetta 2.
- `.github/workflows/release.yml` runs on `v*` tags. It checks that the tag matches the version in `Cargo.toml`, runs the validate workflow, and creates a draft GitHub release with the zips. Tags with a `-` (like `v0.2.0-beta`) become prereleases. Linux release builds run on Ubuntu 22.04, so they work on 22.04 and newer.

## CLI

```sh
# Validate a profile's split image folder and show how each image is read
autosplit check settings.toml

# Similarity of a split image against a captured frame, with every method
autosplit compare "split_(0.95).png" frame.png

# Run the split state machine over a folder of recorded PNG frames
autosplit replay settings.toml frames/ --fps 60 [--auto-controlled]

# Time comparisons
autosplit bench [split images...] --capture 1920x1080
```

## Layout

- `crates/autosplit-core`: everything that doesn't depend on the OS. `runner.rs` is the split state machine. It never blocks: the host calls `tick` with the latest frame when `next_wakeup` is due, and acts on the effects it returns.
- `crates/autosplit-cli`: the `autosplit` command.
- `tests/fixtures`: split images, captures and profiles. `real/` holds split images made with AutoMask.
- `tests/golden`: expected values from AutoSplit and OpenCV.
- `tools/golden/generate.py`: regenerates the fixtures and golden values. Instructions are at the top of the file. Only rerun it when you mean to change the baseline.

## License

GPL-3.0-only, like AutoSplit. Parts of the image processing are ports of OpenCV, see `THIRD_PARTY_NOTICES.md`.
